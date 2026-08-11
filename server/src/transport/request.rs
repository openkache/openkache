//! Backend-independent request admission and frame reception.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_util::FutureExt;
use openkache_protocol::{OwnedRange, RequestFrameHeader};

use super::TransportError;
use crate::network_runtime;
use crate::protocol::{FrameLayoutProvider, RequestFrame as ProtocolRequestFrame};

/// Failure while receiving a request frame.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StreamReadError {
    #[error("request read timed out")]
    Timeout,
    #[error("request exceeds the protocol limit")]
    TooLarge,
    #[error(transparent)]
    Protocol(#[from] openkache_protocol::ProtocolError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Request bytes paired with the server-wide memory-budget reservation they consume.
pub(crate) struct RequestFrame {
    pub(crate) prefix: Vec<u8>,
    pub(crate) payload: OwnedRange,
    /// Whether the transport had already delivered bytes beyond this frame.
    ///
    /// QUIC stream reads may coalesce multiple client writes. If the backend
    /// exposes those trailing bytes, the lane must be retired after the
    /// current response because the peer violated request/response lockstep.
    pub(crate) has_trailing_bytes: bool,
    _permit: RequestBudgetPermit,
}

impl RequestFrame {
    pub(super) fn with_trailing_bytes(
        prefix: Vec<u8>,
        payload: OwnedRange,
        permit: RequestBudgetPermit,
        has_trailing_bytes: bool,
    ) -> Self {
        Self {
            prefix,
            payload,
            has_trailing_bytes,
            _permit: permit,
        }
    }
}

/// Accumulates one demand-sized request body without copying readable chunks.
///
/// A backend that can fill caller-owned memory (for example, Quiche's
/// `stream_recv`) can retain this buffer between readiness notifications. The
/// completed range is then transferred to the backend-independent request
/// reader as one owned allocation. Backends that only return owned chunks keep
/// using the generic coalescing fallback below.
#[allow(dead_code)]
pub(crate) struct RequestReadBuffer {
    bytes: Vec<u8>,
    filled: usize,
}

#[allow(dead_code)]
impl RequestReadBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            bytes: vec![0; capacity],
            filled: 0,
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.filled
    }

    pub(crate) fn remaining_slice(&mut self) -> &mut [u8] {
        &mut self.bytes[self.filled..]
    }

    pub(crate) fn record_read(&mut self, read: usize) -> bool {
        debug_assert!(read <= self.remaining());
        self.filled += read;
        self.filled == self.bytes.len()
    }

    pub(crate) fn into_bytes(mut self) -> Vec<u8> {
        self.bytes.truncate(self.filled);
        self.bytes
    }
}

/// Byte-weighted memory budget shared by every connection and network worker.
#[derive(Clone)]
pub(crate) struct RequestBudget {
    inner: Arc<Mutex<RequestBudgetState>>,
}

struct RequestBudgetState {
    capacity: usize,
    used: usize,
    next_waiter_id: u64,
    waiters: VecDeque<RequestBudgetWaiter>,
}

struct RequestBudgetWaiter {
    id: u64,
    bytes: usize,
    waker: Waker,
}

pub(crate) struct RequestBudgetPermit {
    inner: Arc<Mutex<RequestBudgetState>>,
    bytes: usize,
}

struct RequestBudgetAcquire {
    inner: Arc<Mutex<RequestBudgetState>>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl RequestBudget {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RequestBudgetState {
                capacity,
                used: 0,
                next_waiter_id: 0,
                waiters: VecDeque::new(),
            })),
        }
    }

    pub(crate) async fn acquire(
        &self,
        bytes: usize,
        timeout: Duration,
    ) -> Result<RequestBudgetPermit, StreamReadError> {
        network_runtime::timeout(timeout, self.acquire_without_timeout(bytes))
            .await
            .map_err(|_| StreamReadError::Timeout)?
    }

    pub(super) async fn acquire_without_timeout(
        &self,
        bytes: usize,
    ) -> Result<RequestBudgetPermit, StreamReadError> {
        if bytes == 0 {
            return Ok(RequestBudgetPermit {
                inner: Arc::clone(&self.inner),
                bytes: 0,
            });
        }
        if bytes
            > self
                .inner
                .lock()
                .expect("request budget lock poisoned")
                .capacity
        {
            return Err(StreamReadError::TooLarge);
        }
        Ok(RequestBudgetAcquire {
            inner: Arc::clone(&self.inner),
            bytes,
            waiter_id: None,
        }
        .await)
    }
}

impl Future for RequestBudgetAcquire {
    type Output = RequestBudgetPermit;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = Arc::clone(&self.inner);
        let bytes = self.bytes;
        let mut state = inner.lock().expect("request budget lock poisoned");
        let available = state.capacity - state.used;
        let can_acquire = match self.waiter_id {
            Some(waiter_id) => state
                .waiters
                .front()
                .is_some_and(|waiter| waiter.id == waiter_id && bytes <= available),
            None => state.waiters.is_empty() && bytes <= available,
        };
        if can_acquire {
            if self.waiter_id.take().is_some() {
                state.waiters.pop_front();
            }
            state.used += bytes;
            let next = state
                .waiters
                .front()
                .filter(|waiter| waiter.bytes <= state.capacity - state.used)
                .map(|waiter| waiter.waker.clone());
            drop(state);
            if let Some(waker) = next {
                waker.wake();
            }
            return Poll::Ready(RequestBudgetPermit {
                inner: Arc::clone(&inner),
                bytes,
            });
        }

        if let Some(waiter_id) = self.waiter_id
            && let Some(waiter) = state.waiters.iter_mut().find(|waiter| waiter.id == waiter_id)
        {
            if !waiter.waker.will_wake(context.waker()) {
                waiter.waker.clone_from(context.waker());
            }
            return Poll::Pending;
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state
            .next_waiter_id
            .checked_add(1)
            .expect("request budget waiter identifier overflowed");
        state.waiters.push_back(RequestBudgetWaiter {
            id: waiter_id,
            bytes,
            waker: context.waker().clone(),
        });
        drop(state);
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for RequestBudgetAcquire {
    fn drop(&mut self) {
        if let Some(waiter_id) = self.waiter_id {
            let next = {
                let mut state = self.inner.lock().expect("request budget lock poisoned");
                let removed_front = state.waiters.front().is_some_and(|waiter| waiter.id == waiter_id);
                if let Some(position) = state
                    .waiters
                    .iter()
                    .position(|waiter| waiter.id == waiter_id)
                {
                    state.waiters.remove(position);
                }
                removed_front
                    .then(|| state.waiters.front())
                    .flatten()
                    .filter(|waiter| waiter.bytes <= state.capacity - state.used)
                    .map(|waiter| waiter.waker.clone())
            };
            if let Some(waker) = next {
                waker.wake();
            }
        }
    }
}

impl Drop for RequestBudgetPermit {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let next = {
            let mut state = self.inner.lock().expect("request budget lock poisoned");
            state.used = state
                .used
                .checked_sub(self.bytes)
                .expect("released request bytes must be reserved");
            state
                .waiters
                .front()
                .filter(|waiter| waiter.bytes <= state.capacity - state.used)
                .map(|waiter| waiter.waker.clone())
        };
        if let Some(waker) = next {
            waker.wake();
        }
    }
}

pub(crate) trait RequestByteStream {
    fn read_chunk(
        &mut self,
        capacity: usize,
        backend: &'static str,
    ) -> impl Future<Output = Result<Option<OwnedRange>, TransportError>>;

    fn has_readable_byte(
        &mut self,
        backend: &'static str,
    ) -> impl Future<Output = Result<bool, TransportError>>;

    /// Polls once for a trailing byte without waiting for future network
    /// input. Backends with a synchronous driver query can override this
    /// method while retaining the same request state machine.
    fn try_has_readable_byte(
        &mut self,
        backend: &'static str,
    ) -> impl Future<Output = Result<bool, TransportError>> {
        async move {
            self.has_readable_byte(backend)
                .now_or_never()
                .transpose()
                .map(|readable| readable.unwrap_or(false))
        }
    }
}

pub(crate) async fn read_buffered_request<S: RequestByteStream>(
    stream: &mut S,
    backend: &'static str,
    maximum: usize,
    maximum_value: usize,
    timeout: Duration,
    budget: &RequestBudget,
    frame_layout_provider: &dyn FrameLayoutProvider,
) -> Result<RequestFrame, StreamReadError> {
    let (prefix, payload, permit) = network_runtime::timeout(timeout, async {
        let (prefix, header) =
            read_request_prefix(stream, backend, maximum, frame_layout_provider).await?;
        if header.value_len() > maximum_value {
            return Err(StreamReadError::TooLarge);
        }
        let frame_len = header.frame_len()?;
        if frame_len > maximum {
            return Err(StreamReadError::TooLarge);
        }
        let permit = budget
            .acquire_without_timeout(header.value_len())
            .await?;
        let payload = read_request_payload(stream, backend, header.value_len()).await?;
        validate_request_frame_length(&prefix, &payload, frame_len)?;
        Ok::<_, StreamReadError>((prefix, payload, permit))
    })
    .await
    .map_err(|_| StreamReadError::Timeout)??;

    let has_trailing_bytes = stream
        .try_has_readable_byte(backend)
        .await
        .map_err(StreamReadError::Transport)?;
    Ok(RequestFrame::with_trailing_bytes(
        prefix,
        payload,
        permit,
        has_trailing_bytes,
    ))
}

async fn read_request_prefix<S: RequestByteStream>(
    stream: &mut S,
    backend: &'static str,
    maximum: usize,
    frame_layout_provider: &dyn FrameLayoutProvider,
) -> Result<(Vec<u8>, RequestFrameHeader), StreamReadError> {
    let mut prefix = Vec::new();
    loop {
        let needed =
            ProtocolRequestFrame::header_bytes_needed_with(&prefix, frame_layout_provider)?;
        if needed == 0 {
            let header = ProtocolRequestFrame::decode_header_with(&prefix, frame_layout_provider)?
                .ok_or_else(|| {
                    StreamReadError::Transport(TransportError::backend(
                        backend,
                        "stream header read",
                        "completed request metadata did not produce a header",
                    ))
                })?;
            return Ok((prefix, header));
        }
        if prefix
            .len()
            .checked_add(needed)
            .is_none_or(|header_len| header_len > maximum)
        {
            return Err(StreamReadError::TooLarge);
        }
        let next = stream
            .read_chunk(needed, backend)
            .await
            .map_err(StreamReadError::Transport)?;
        let Some(next) = next else {
            return Err(StreamReadError::Transport(TransportError::backend(
                backend,
                "stream header read",
                "stream ended before a request frame header completed",
            )));
        };
        if next.len() == 0 {
            return Err(StreamReadError::Transport(TransportError::backend(
                backend,
                "stream header read",
                "request reader returned an empty chunk",
            )));
        }
        if next.len() > needed {
            return Err(StreamReadError::Transport(TransportError::backend(
                backend,
                "stream header read",
                "request reader returned bytes beyond the requested capacity",
            )));
        }
        prefix.extend_from_slice(next.as_slice());
    }
}

async fn read_request_payload<S: RequestByteStream>(
    stream: &mut S,
    backend: &'static str,
    value_len: usize,
) -> Result<OwnedRange, StreamReadError> {
    if value_len == 0 {
        return Ok(OwnedRange::whole(Vec::new()));
    }
    let first = stream
        .read_chunk(value_len, backend)
        .await
        .map_err(StreamReadError::Transport)?
        .ok_or_else(|| {
            StreamReadError::Transport(TransportError::backend(
                backend,
                "stream body read",
                "stream ended before request body completed",
            ))
        })?;
    if first.len() == 0 {
        return Err(StreamReadError::Transport(TransportError::backend(
            backend,
            "stream body read",
            "request reader returned an empty chunk",
        )));
    }
    if first.len() > value_len {
        return Err(StreamReadError::Transport(TransportError::backend(
            backend,
            "stream body read",
            "request reader returned bytes beyond the requested capacity",
        )));
    }
    if first.len() == value_len {
        return Ok(first);
    }

    let mut coalesced = Vec::new();
    coalesced
        .try_reserve(value_len)
        .map_err(|error| {
            StreamReadError::Transport(TransportError::backend(
                backend,
                "request buffer reserve",
                error,
            ))
        })?;
    coalesced.extend_from_slice(first.as_slice());
    while coalesced.len() < value_len {
        let remaining = value_len - coalesced.len();
        let next = stream
            .read_chunk(remaining, backend)
            .await
            .map_err(StreamReadError::Transport)?
            .ok_or_else(|| {
                StreamReadError::Transport(TransportError::backend(
                    backend,
                    "stream body read",
                    "stream ended before request body completed",
                ))
            })?;
        if next.len() == 0 {
            return Err(StreamReadError::Transport(TransportError::backend(
                backend,
                "stream body read",
                "request reader returned an empty chunk",
            )));
        }
        if next.len() > remaining {
            return Err(StreamReadError::Transport(TransportError::backend(
                backend,
                "stream body read",
                "request reader returned bytes beyond the requested capacity",
            )));
        }
        coalesced.extend_from_slice(next.as_slice());
    }
    Ok(OwnedRange::whole(coalesced))
}

fn validate_request_frame_length(
    prefix: &[u8],
    payload: &OwnedRange,
    expected: usize,
) -> Result<(), StreamReadError> {
    let actual = prefix
        .len()
        .checked_add(payload.len())
        .ok_or(StreamReadError::TooLarge)?;
    if actual == expected {
        return Ok(());
    }
    Err(StreamReadError::Protocol(
        openkache_protocol::ProtocolError::FrameLength { expected, actual },
    ))
}
