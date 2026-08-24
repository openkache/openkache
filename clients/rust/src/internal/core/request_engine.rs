//! Transport-neutral multiplexed request engine.
//!
//! The engine deliberately stops at opaque protocol bytes.  Request framing,
//! response framing, and value codecs remain explicit interfaces owned by the
//! wire and value layers; this module only admits bytes, correlates response
//! IDs, and applies lifecycle policy.

use crate::internal_protocol::{Opcode, OwnedRequestFrame, Response, Status};
use crossfire::{AsyncRx, MAsyncTx};
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt, pin_mut, select};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::internal_core::Operation;

/// Whether a request can have an externally visible effect after transmission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestKind {
    /// A read-only request may be retried when its lane fails.
    ReadOnly,
    /// A mutation is unknown when transmission completes without a response.
    Mutation,
    /// A durability or maintenance barrier is treated like a mutation.
    Barrier,
}

impl RequestKind {
    fn is_mutating(self) -> bool {
        matches!(self, Self::Mutation | Self::Barrier)
    }
}

/// Metadata used to validate and classify one response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestMetadata {
    /// Generated protocol operation.
    pub operation: Operation,
    /// Request side-effect classification.
    pub kind: RequestKind,
    /// Accepted successful statuses for this operation.
    pub success_statuses: &'static [Status],
    /// Accepted definitive server-error statuses for this operation.
    pub error_statuses: &'static [Status],
    /// Maximum complete response frame accepted by this request.
    pub maximum_response_bytes: usize,
}

/// Returns whether a response status is assigned to the stable-v1 operation.
///
/// Generated operation metadata still carries transitional lifecycle and
/// experimental statuses for compatibility adapters. Stable data operations
/// must apply the protocol specification's narrower status applicability
/// before accepting any generated status list.
pub(crate) const fn stable_status_allowed(operation: Operation, status: Status) -> bool {
    match operation {
        Operation::Ping => matches!(
            status,
            Status::Ok
                | Status::InvalidRequest
                | Status::Overloaded
                | Status::Forbidden
                | Status::InternalError
        ),
        Operation::Get => matches!(
            status,
            Status::Ok
                | Status::NotFound
                | Status::InvalidRequest
                | Status::Overloaded
                | Status::Forbidden
                | Status::InternalError
                | Status::NamespaceNotFound
        ),
        Operation::Set => matches!(
            status,
            Status::Created
                | Status::Replaced
                | Status::NotStored
                | Status::InvalidRequest
                | Status::TooLarge
                | Status::Overloaded
                | Status::Forbidden
                | Status::InternalError
                | Status::NoCapacity
                | Status::PolicyConflict
                | Status::NamespaceNotFound
        ),
        Operation::Delete => matches!(
            status,
            Status::Deleted
                | Status::NotFound
                | Status::InvalidRequest
                | Status::Overloaded
                | Status::Forbidden
                | Status::InternalError
                | Status::NamespaceNotFound
        ),
        _ => true,
    }
}

/// An owned request frame that keeps the exact segment bytes sent by a lane.
#[derive(Clone, Debug)]
pub struct RequestBytes(Arc<OwnedRequestFrame>);

impl RequestBytes {
    /// Retains one encoded frame without coalescing its payload segments.
    pub fn new(frame: OwnedRequestFrame) -> Self {
        Self(Arc::new(frame))
    }

    /// Returns the encoded byte count.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the frame has no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the ordered exact bytes for a transport write.
    pub fn segments(&self) -> impl Iterator<Item = &[u8]> {
        self.0.segments()
    }

    /// Returns the request frame owner.
    pub fn as_frame(&self) -> &OwnedRequestFrame {
        &self.0
    }

    /// Decodes the correlation token from the common request prefix.
    ///
    /// The caller supplies the operation layout because the engine does not
    /// interpret operation fields.
    pub fn request_id(
        &self,
        layout: crate::internal_protocol::RequestFrameLayout,
    ) -> Result<u64, EngineError> {
        crate::internal_protocol::OpaqueRequestFrame::decode(
            &self.0.segments().collect::<Vec<_>>().concat(),
            layout,
        )
        .map(|frame| frame.request_id())
        .map_err(|error| EngineError::Protocol(error.to_string()))
    }
}

/// An owned complete response frame.  The status, ID, and payload all borrow
/// the retained original bytes, so adapters can preserve exact wire bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseBytes {
    frame: Vec<u8>,
    status: Status,
    request_id: u64,
    payload_offset: usize,
}

impl ResponseBytes {
    fn decode(frame: Vec<u8>, metadata: RequestMetadata) -> Result<Self, EngineError> {
        if frame.len() > metadata.maximum_response_bytes {
            return Err(EngineError::Protocol(format!(
                "response exceeds {} bytes",
                metadata.maximum_response_bytes
            )));
        }
        let header = Response::decode_header(&frame)
            .map_err(|error| EngineError::Protocol(error.to_string()))?
            .ok_or_else(|| EngineError::Protocol("truncated response header".into()))?;
        let expected = header
            .frame_len()
            .map_err(|error| EngineError::Protocol(error.to_string()))?;
        if expected != frame.len() {
            return Err(EngineError::Protocol(format!(
                "response frame length is {expected}, got {}",
                frame.len()
            )));
        }
        let status = header.status();
        if !stable_status_allowed(metadata.operation, status)
            || (!metadata.success_statuses.contains(&status)
                && !metadata.error_statuses.contains(&status))
        {
            return Err(EngineError::Protocol(format!(
                "status {status:?} is not valid for {}",
                metadata.operation
            )));
        }
        Ok(Self {
            frame,
            status,
            request_id: header.request_id(),
            payload_offset: header.encoded_len(),
        })
    }

    /// Returns the complete exact response frame bytes.
    pub fn encoded(&self) -> &[u8] {
        &self.frame
    }

    /// Returns the response status.
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Returns the echoed request ID.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Returns the opaque response payload without decoding its value format.
    pub fn payload(&self) -> &[u8] {
        &self.frame[self.payload_offset..]
    }

    /// Consumes the owner and returns exact response bytes.
    pub fn into_encoded(self) -> Vec<u8> {
        self.frame
    }
}

/// Transport failure with an explicit transmission boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportError {
    /// Human-readable backend detail.
    pub message: String,
    /// Whether the backend may have written any request bytes.
    pub transmitted: bool,
}

impl TransportError {
    /// Creates a failure known to occur before any request byte was written.
    pub fn before_send(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transmitted: false,
        }
    }

    /// Creates a failure after the write boundary was crossed.
    pub fn after_send(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transmitted: true,
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TransportError {}

/// Engine-level lifecycle and protocol error categories.
#[derive(Debug, thiserror::Error, Clone, Eq, PartialEq)]
pub enum EngineError {
    /// Input/configuration was rejected before a correlation entry was sent.
    #[error("request rejected locally: {0}")]
    Local(String),
    /// The engine was closed before admission.
    #[error("request engine is closed")]
    Closed,
    /// The caller cancelled a request before a response arrived.
    #[error("request {request_id} was cancelled")]
    Cancelled {
        /// Correlation token of the cancelled request.
        request_id: u64,
    },
    /// A lane transport failed before or after transmission.
    #[error("transport failed: {0}")]
    Transport(TransportError),
    /// A response frame was malformed or semantically inapplicable.
    #[error("invalid response: {0}")]
    Protocol(String),
    /// A mutation may have taken effect but no response was confirmed.
    #[error("mutation {operation} has an unknown outcome after transmission: {cause}")]
    UnknownMutation {
        /// Operation whose effect cannot be determined.
        operation: Operation,
        /// Correlation token of the mutation.
        request_id: u64,
        /// Underlying transport or cancellation detail.
        cause: Box<Self>,
    },
    /// The aggregate byte budget cannot admit the request.
    #[error("request requires {requested} bytes but the aggregate budget is {maximum} bytes")]
    BudgetExceeded {
        /// Requested retained bytes.
        requested: usize,
        /// Configured aggregate budget.
        maximum: usize,
    },
}

/// A transport-neutral lane.  Implementations split send/read internally so a
/// writer and response reader can make progress concurrently.
pub trait TransportLane: Send + Sync + 'static {
    /// Writes one complete request while preserving all request segments.
    ///
    /// Implementations MUST serialize concurrent calls. A lane has one
    /// ordered request direction, while the engine may have several writer
    /// futures ready at once.
    fn write_request(
        &self,
        request: RequestBytes,
    ) -> BoxFuture<'static, Result<(), TransportError>>;

    /// Reads one complete response frame, retaining exact bytes.
    fn read_response(&self, maximum: usize) -> BoxFuture<'static, Result<Vec<u8>, TransportError>>;

    /// Closes both directions of the lane.
    fn close(&self);
}

/// Transport profile selected by a connection adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransportKind {
    /// QUIC with one or more client-initiated bidirectional lanes.
    Quic,
    /// TLS 1.3 over TCP with one ordered lane.
    TlsTcp,
}

/// Connection-level interface shared by QUIC and TLS-over-TCP adapters.
///
/// The core never inspects socket, TLS, or runtime types. A connector exposes
/// owned lane handles and the adapter maps its backend's read/write/cancel
/// operations onto [`TransportLane`].
pub trait TransportConnection: Send + Sync + 'static {
    /// Returns the negotiated transport profile.
    fn kind(&self) -> TransportKind;

    /// Returns the bounded lanes owned by this connection.
    fn lanes(&self) -> Vec<Arc<dyn TransportLane>>;

    /// Closes the connection and all lanes.
    fn close(&self);
}

struct BudgetState {
    used: usize,
    next_waiter_id: u64,
    waiters: HashMap<u64, Waker>,
}

/// One aggregate byte budget shared by request, transport, and response work.
pub struct InFlightByteBudget {
    maximum: usize,
    state: Mutex<BudgetState>,
}

impl InFlightByteBudget {
    /// Creates a bounded aggregate budget.
    pub fn new(maximum: usize) -> Result<Arc<Self>, EngineError> {
        if maximum == 0 {
            return Err(EngineError::Local("byte budget must be positive".into()));
        }
        Ok(Arc::new(Self {
            maximum,
            state: Mutex::new(BudgetState {
                used: 0,
                next_waiter_id: 0,
                waiters: HashMap::new(),
            }),
        }))
    }

    /// Returns the configured aggregate byte limit.
    pub const fn maximum(&self) -> usize {
        self.maximum
    }

    /// Reserves bytes before the caller allocates a request body.
    pub fn try_reserve(self: &Arc<Self>, bytes: usize) -> Result<BytePermit, EngineError> {
        let mut state = self.state.lock().expect("byte budget lock is not poisoned");
        if bytes > self.maximum.saturating_sub(state.used) {
            return Err(EngineError::BudgetExceeded {
                requested: bytes,
                maximum: self.maximum,
            });
        }
        state.used += bytes;
        Ok(BytePermit {
            budget: Arc::clone(self),
            bytes,
        })
    }

    fn release(&self, bytes: usize) {
        let waiters = {
            let mut state = self.state.lock().expect("byte budget lock is not poisoned");
            state.used = state.used.saturating_sub(bytes);
            // Keep registrations in place while waking. A wake-up is only a
            // hint: if one waiter consumes the newly available bytes, every
            // other waiter must remain registered for the next release.
            state.waiters.values().cloned().collect::<Vec<_>>()
        };
        for waiter in waiters {
            waiter.wake();
        }
    }

    /// Waits for bytes to become available without allocating a body.
    pub fn reserve(
        self: &Arc<Self>,
        bytes: usize,
    ) -> impl Future<Output = Result<BytePermit, EngineError>> + '_ {
        RequestBudgetAcquire {
            budget: Arc::clone(self),
            bytes,
            waiter_id: None,
        }
    }
}

struct RequestBudgetAcquire {
    budget: Arc<InFlightByteBudget>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl Future for RequestBudgetAcquire {
    type Output = Result<BytePermit, EngineError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let budget = Arc::clone(&self.budget);
        let mut state = budget
            .state
            .lock()
            .expect("byte budget lock is not poisoned");
        if self.bytes <= budget.maximum.saturating_sub(state.used) {
            if let Some(waiter_id) = self.waiter_id.take() {
                state.waiters.remove(&waiter_id);
            }
            state.used += self.bytes;
            drop(state);
            return Poll::Ready(Ok(BytePermit {
                budget,
                bytes: self.bytes,
            }));
        }
        if self.bytes > budget.maximum {
            return Poll::Ready(Err(EngineError::BudgetExceeded {
                requested: self.bytes,
                maximum: budget.maximum,
            }));
        }

        if let Some(waiter_id) = self.waiter_id {
            if let Some(waiter) = state.waiters.get_mut(&waiter_id) {
                if !waiter.will_wake(context.waker()) {
                    waiter.clone_from(context.waker());
                }
            }
            return Poll::Pending;
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state
            .next_waiter_id
            .checked_add(1)
            .expect("byte budget waiter identifier overflowed");
        state.waiters.insert(waiter_id, context.waker().clone());
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for RequestBudgetAcquire {
    fn drop(&mut self) {
        if let Some(waiter_id) = self.waiter_id {
            self.budget
                .state
                .lock()
                .expect("byte budget lock is not poisoned")
                .waiters
                .remove(&waiter_id);
        }
    }
}

/// A retained byte-budget reservation released exactly once on drop.
pub struct BytePermit {
    budget: Arc<InFlightByteBudget>,
    bytes: usize,
}

impl BytePermit {
    /// Returns the reserved byte count.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for BytePermit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

struct PendingEntry {
    lane: usize,
    metadata: RequestMetadata,
    sender: Option<crossfire::oneshot::TxOneshot<Result<ResponseBytes, EngineError>>>,
    _permit: BytePermit,
    transmitted: bool,
    started: bool,
    canceled: bool,
}

struct Registry {
    entries: Mutex<HashMap<u64, PendingEntry>>,
}

impl Registry {
    fn reserve(
        &self,
        id: u64,
        lane: usize,
        metadata: RequestMetadata,
        permit: BytePermit,
    ) -> crossfire::oneshot::RxOneshot<Result<ResponseBytes, EngineError>> {
        let (sender, receiver) = crossfire::oneshot::oneshot();
        self.entries
            .lock()
            .expect("request registry lock is not poisoned")
            .insert(
                id,
                PendingEntry {
                    lane,
                    metadata,
                    sender: Some(sender),
                    _permit: permit,
                    transmitted: false,
                    started: false,
                    canceled: false,
                },
            );
        receiver
    }

    fn mark_transmitted(&self, id: u64) {
        if let Some(entry) = self
            .entries
            .lock()
            .expect("request registry lock is not poisoned")
            .get_mut(&id)
        {
            entry.transmitted = true;
        }
    }

    fn mark_started(&self, id: u64) -> bool {
        if let Some(entry) = self
            .entries
            .lock()
            .expect("request registry lock is not poisoned")
            .get_mut(&id)
        {
            entry.started = true;
            true
        } else {
            false
        }
    }

    fn metadata(&self, id: u64) -> Option<RequestMetadata> {
        self.entries
            .lock()
            .expect("request registry lock is not poisoned")
            .get(&id)
            .map(|entry| entry.metadata)
    }

    fn contains(&self, id: u64) -> bool {
        self.entries
            .lock()
            .expect("request registry lock is not poisoned")
            .contains_key(&id)
    }

    fn started(&self, id: u64) -> bool {
        self.entries
            .lock()
            .expect("request registry lock is not poisoned")
            .get(&id)
            .is_some_and(|entry| entry.started)
    }

    fn complete(&self, id: u64, result: Result<ResponseBytes, EngineError>) {
        if let Some(entry) = self
            .entries
            .lock()
            .expect("request registry lock is not poisoned")
            .remove(&id)
        {
            if let Some(sender) = entry.sender {
                sender.send(result);
            }
        }
    }

    fn cancel(&self, id: u64) {
        let mut entries = self
            .entries
            .lock()
            .expect("request registry lock is not poisoned");
        let Some(entry) = entries.get_mut(&id) else {
            return;
        };
        if entry.canceled {
            return;
        }
        // `started` is set immediately before entering the transport write
        // future.  At that point a backend may already have crossed an
        // unobservable write boundary, even when it later reports
        // `transmitted = false` (for example, if cancellation races writer
        // startup).  Treat both states conservatively for mutations.
        let error = if (entry.transmitted || entry.started) && entry.metadata.kind.is_mutating() {
            EngineError::UnknownMutation {
                operation: entry.metadata.operation,
                request_id: id,
                cause: Box::new(EngineError::Cancelled { request_id: id }),
            }
        } else {
            EngineError::Cancelled { request_id: id }
        };
        entry.canceled = true;
        let remove = !entry.started && !entry.transmitted;
        if let Some(sender) = entry.sender.take() {
            sender.send(Err(error));
        }
        if remove {
            entries.remove(&id);
        }
    }

    fn fail_all(&self, cause: EngineError) {
        let entries = std::mem::take(
            &mut *self
                .entries
                .lock()
                .expect("request registry lock is not poisoned"),
        );
        for (id, entry) in entries {
            if entry.canceled {
                continue;
            }
            let result =
                if (entry.transmitted || entry.started) && entry.metadata.kind.is_mutating() {
                    Err(EngineError::UnknownMutation {
                        operation: entry.metadata.operation,
                        request_id: id,
                        cause: Box::new(cause.clone()),
                    })
                } else {
                    Err(cause.clone())
                };
            if let Some(sender) = entry.sender {
                sender.send(result);
            }
        }
    }

    fn fail_lane(
        &self,
        lane: usize,
        active: &HashSet<u64>,
        cause: EngineError,
        assume_started_transmitted: bool,
    ) {
        let entries = {
            let mut all = self
                .entries
                .lock()
                .expect("request registry lock is not poisoned");
            let ids: Vec<u64> = all
                .iter()
                .filter_map(|(&id, entry)| {
                    (entry.lane == lane || active.contains(&id)).then_some(id)
                })
                .collect();
            ids.into_iter()
                .filter_map(|id| all.remove(&id).map(|entry| (id, entry)))
                .collect::<Vec<_>>()
        };
        for (id, entry) in entries {
            if entry.canceled {
                continue;
            }
            let transmitted = entry.transmitted || (assume_started_transmitted && entry.started);
            let result = if transmitted && entry.metadata.kind.is_mutating() {
                Err(EngineError::UnknownMutation {
                    operation: entry.metadata.operation,
                    request_id: id,
                    cause: Box::new(cause.clone()),
                })
            } else {
                Err(cause.clone())
            };
            if let Some(sender) = entry.sender {
                sender.send(result);
            }
        }
    }
}

/// A request admission reserved before body allocation or transport enqueue.
pub struct RequestAdmission {
    engine: Arc<RequestEngineInner>,
    metadata: RequestMetadata,
    request_id: u64,
    request_bytes: usize,
    permit: Option<BytePermit>,
}

impl RequestAdmission {
    /// Returns the ID that must be encoded into the request frame.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Returns the request metadata captured before encoding.
    pub const fn metadata(&self) -> RequestMetadata {
        self.metadata
    }

    /// Queues an encoded frame for one bounded lane.
    pub fn submit(mut self, frame: OwnedRequestFrame) -> Result<RequestHandle, EngineError> {
        if self.engine.state.load(Ordering::Acquire) != 0 {
            return Err(EngineError::Closed);
        }
        let request = RequestBytes::new(frame);
        let request_id = self.request_id;
        // The request encoder's common header is intentionally opaque here;
        // adapters pass the ID explicitly and this cheap prefix check catches
        // accidental default-token frames before any bytes reach a transport.
        let (opcode, encoded_id) = request_prefix(&request)?;
        let opcode = Opcode::try_from(opcode)
            .map_err(|error| EngineError::Local(format!("request opcode is invalid: {error}")))?;
        let expected_opcode = opcode_for_operation(self.metadata.operation).ok_or_else(|| {
            EngineError::Local(format!(
                "operation {:?} has no request opcode",
                self.metadata.operation
            ))
        })?;
        if opcode != expected_opcode {
            return Err(EngineError::Local(format!(
                "request opcode {opcode:?} does not match metadata operation {:?}",
                self.metadata.operation
            )));
        }
        if encoded_id != request_id {
            return Err(EngineError::Local(format!(
                "request frame ID {encoded_id} does not match reserved ID {request_id}"
            )));
        }
        if request.len() > self.request_bytes {
            return Err(EngineError::BudgetExceeded {
                requested: request.len(),
                maximum: self.engine.budget.maximum(),
            });
        }
        let lane = {
            let start = self.engine.next_lane.fetch_add(1, Ordering::Relaxed);
            let mut selected = None;
            for offset in 0..self.engine.lanes.len() {
                let index = start.wrapping_add(offset) % self.engine.lanes.len();
                if self.engine.lane_states[index].load(Ordering::Acquire) == 0 {
                    selected = Some(index);
                    break;
                }
            }
            selected.ok_or(EngineError::Closed)?
        };
        let receiver = self.engine.registry.reserve(
            request_id,
            lane,
            self.metadata,
            self.permit
                .take()
                .expect("request admission permit is present"),
        );
        self.engine
            .reserved
            .lock()
            .expect("request reservation lock is not poisoned")
            .remove(&request_id);
        let command = LaneCommand {
            id: request_id,
            metadata: self.metadata,
            request,
        };
        if let Err(error) = self.engine.lane_senders[lane].try_send(command) {
            self.engine.registry.complete(
                request_id,
                Err(EngineError::Local(format!(
                    "lane queue rejected request: {error}"
                ))),
            );
            return Err(EngineError::Local("lane queue is full".into()));
        }
        Ok(RequestHandle {
            id: request_id,
            receiver: Some(receiver),
            registry: Arc::clone(&self.engine.registry),
        })
    }
}

impl Drop for RequestAdmission {
    fn drop(&mut self) {
        if self.permit.is_some() {
            self.engine
                .reserved
                .lock()
                .expect("request reservation lock is not poisoned")
                .remove(&self.request_id);
            self.engine.registry.cancel(self.request_id);
        }
    }
}

/// Resolves the wire opcode owned by one generated protocol operation.
///
/// Request-admission metadata is captured before a caller allocates and
/// encodes its frame. Keeping this correspondence check in the engine makes
/// it impossible for an adapter to enqueue bytes under the wrong operation
/// classification.
fn opcode_for_operation(operation: Operation) -> Option<Opcode> {
    match operation {
        Operation::Ping => Some(Opcode::Ping),
        Operation::Get => Some(Opcode::Get),
        Operation::Set => Some(Opcode::Set),
        Operation::Delete => Some(Opcode::Delete),
        Operation::ExperimentalStats => Some(Opcode::ExperimentalStats),
        Operation::ExperimentalSync => Some(Opcode::ExperimentalSync),
        Operation::NamespaceOpen => Some(Opcode::NamespaceOpen),
        Operation::NamespaceUpdatePolicy => Some(Opcode::NamespaceUpdatePolicy),
        Operation::NamespaceDelete => Some(Opcode::NamespaceDelete),
        _ => None,
    }
}

/// Reads the fixed opcode and canonical request ID without allocating the
/// complete (potentially multi-megabyte) request body.
fn request_prefix(request: &RequestBytes) -> Result<(u8, u64), EngineError> {
    let mut prefix = [0_u8; 1 + crate::internal_protocol::MAX_VARUINT_BYTES];
    let mut length = 0;
    for segment in request.segments() {
        let remaining = prefix.len().saturating_sub(length);
        if remaining == 0 {
            break;
        }
        let take = remaining.min(segment.len());
        prefix[length..length + take].copy_from_slice(&segment[..take]);
        length += take;
        if length >= 2 {
            if let Some((request_id, _)) =
                crate::internal_protocol::decode_varuint(&prefix[1..length], "request ID")
                    .map_err(|error| EngineError::Local(error.to_string()))?
            {
                return Ok((prefix[0], request_id));
            }
        }
    }
    if length == 0 {
        return Err(EngineError::Local("request frame is empty".into()));
    }
    if length == 1 {
        return Err(EngineError::Local("request frame has no request ID".into()));
    }
    Err(EngineError::Local(
        "request frame has no complete request ID".into(),
    ))
}

/// A caller-owned request completion. Dropping it cancels the registry entry
/// exactly once and preserves unknown mutation outcome after transmission.
pub struct RequestHandle {
    id: u64,
    receiver: Option<crossfire::oneshot::RxOneshot<Result<ResponseBytes, EngineError>>>,
    registry: Arc<Registry>,
}

impl RequestHandle {
    /// Returns the reserved request ID.
    pub const fn request_id(&self) -> u64 {
        self.id
    }

    /// Waits for the correlated response.
    pub async fn response(mut self) -> Result<ResponseBytes, EngineError> {
        self.receiver
            .take()
            .expect("request response receiver is present")
            .recv_async()
            .await
            .unwrap_or_else(|_| Err(EngineError::Closed))
    }
}

impl Future for RequestHandle {
    type Output = Result<ResponseBytes, EngineError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: polling a pinned oneshot receiver does not move it.
        let this = unsafe { self.get_unchecked_mut() };
        Pin::new(
            this.receiver
                .as_mut()
                .expect("request response receiver is present"),
        )
        .poll(context)
        .map(|result| result.unwrap_or_else(|_| Err(EngineError::Closed)))
    }
}

impl Drop for RequestHandle {
    fn drop(&mut self) {
        self.registry.cancel(self.id);
    }
}

struct LaneCommand {
    id: u64,
    metadata: RequestMetadata,
    request: RequestBytes,
}

struct RequestEngineInner {
    lanes: Vec<Arc<dyn TransportLane>>,
    lane_senders: Vec<MAsyncTx<crossfire::mpsc::Array<LaneCommand>>>,
    lane_receivers: Mutex<Vec<Option<AsyncRx<crossfire::mpsc::Array<LaneCommand>>>>>,
    registry: Arc<Registry>,
    budget: Arc<InFlightByteBudget>,
    reserved: Mutex<HashSet<u64>>,
    next_request_id: AtomicU64,
    next_lane: AtomicUsize,
    state: AtomicU8,
    lane_states: Vec<AtomicU8>,
    running_lanes: AtomicUsize,
    drain_wakers: Mutex<Vec<Waker>>,
}

/// Bounded connection/lane/request engine.
#[derive(Clone)]
pub struct RequestEngine {
    inner: Arc<RequestEngineInner>,
}

impl RequestEngine {
    /// Creates an engine from a transport adapter's owned lanes.
    ///
    /// The adapter retains ownership of the connection and is responsible for
    /// starting one [`RequestEngine::run_lane`] task per returned lane.
    pub fn from_connection(
        connection: &dyn TransportConnection,
        maximum_in_flight_bytes: usize,
        maximum_queued_requests: usize,
    ) -> Result<Self, EngineError> {
        Self::new(
            connection.lanes(),
            maximum_in_flight_bytes,
            maximum_queued_requests,
        )
    }

    /// Creates one engine over one or more transport-neutral lanes.
    pub fn new(
        lanes: Vec<Arc<dyn TransportLane>>,
        maximum_in_flight_bytes: usize,
        maximum_queued_requests: usize,
    ) -> Result<Self, EngineError> {
        if lanes.is_empty() {
            return Err(EngineError::Local("at least one lane is required".into()));
        }
        if maximum_queued_requests == 0 {
            return Err(EngineError::Local(
                "maximum queued requests must be positive".into(),
            ));
        }
        let budget = InFlightByteBudget::new(maximum_in_flight_bytes)?;
        let registry = Arc::new(Registry {
            entries: Mutex::new(HashMap::new()),
        });
        let mut lane_senders = Vec::with_capacity(lanes.len());
        let mut lane_receivers = Vec::with_capacity(lanes.len());
        for _ in &lanes {
            let (sender, receiver) = crossfire::mpsc::bounded_async(maximum_queued_requests);
            lane_senders.push(sender);
            lane_receivers.push(Some(receiver));
        }
        let lane_count = lanes.len();
        let inner = Arc::new(RequestEngineInner {
            lanes,
            lane_senders,
            lane_receivers: Mutex::new(lane_receivers),
            registry,
            budget,
            reserved: Mutex::new(HashSet::new()),
            next_request_id: AtomicU64::new(0),
            next_lane: AtomicUsize::new(0),
            state: AtomicU8::new(0),
            lane_states: (0..lane_count).map(|_| AtomicU8::new(0)).collect(),
            running_lanes: AtomicUsize::new(0),
            drain_wakers: Mutex::new(Vec::new()),
        });
        Ok(Self { inner })
    }

    /// Reserves a correlation entry and aggregate bytes before body allocation.
    ///
    /// `request_bytes` is the maximum retained request-frame allocation. The
    /// engine adds the request's bounded response allowance to that value, so
    /// network/request/response ownership consumes one aggregate permit.
    pub fn admit(
        &self,
        metadata: RequestMetadata,
        request_bytes: usize,
    ) -> Result<RequestAdmission, EngineError> {
        if self.inner.state.load(Ordering::Acquire) != 0 {
            return Err(EngineError::Closed);
        }
        let bytes = request_bytes
            .checked_add(metadata.maximum_response_bytes)
            .ok_or(EngineError::BudgetExceeded {
                requested: usize::MAX,
                maximum: self.inner.budget.maximum(),
            })?;
        let permit = self.inner.budget.try_reserve(bytes)?;
        if self.inner.state.load(Ordering::Acquire) != 0 {
            drop(permit);
            return Err(EngineError::Closed);
        }
        let request_id = loop {
            let candidate = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
            let mut reserved = self
                .inner
                .reserved
                .lock()
                .expect("request reservation lock is not poisoned");
            if reserved.insert(candidate) {
                drop(reserved);
                if self.inner.registry.contains(candidate) {
                    self.inner
                        .reserved
                        .lock()
                        .expect("request reservation lock is not poisoned")
                        .remove(&candidate);
                    continue;
                }
                break candidate;
            }
        };
        // Reserve the ID with a placeholder sender only when submit receives
        // the encoded frame; this keeps local encoding failures pre-send.
        Ok(RequestAdmission {
            engine: Arc::clone(&self.inner),
            metadata,
            request_id,
            request_bytes,
            permit: Some(permit),
        })
    }

    /// Drives one lane's writer and response reader. Start one runner per
    /// lane on the selected asynchronous runtime.
    pub async fn run_lane(&self, index: usize) -> Result<(), EngineError> {
        let lane = self
            .inner
            .lanes
            .get(index)
            .cloned()
            .ok_or_else(|| EngineError::Local("lane index is out of range".into()))?;
        let receiver = self
            .inner
            .lane_receivers
            .lock()
            .expect("lane receiver lock is not poisoned")
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| EngineError::Local("lane runner already started".into()))?;
        self.inner.running_lanes.fetch_add(1, Ordering::AcqRel);
        let _running = LaneRunnerGuard {
            inner: Arc::clone(&self.inner),
        };
        let mut active = HashSet::new();
        let mut write: Option<
            BoxFuture<'static, Result<(u64, RequestMetadata), (u64, EngineError)>>,
        > = None;
        let mut write_queue: VecDeque<LaneCommand> = VecDeque::new();
        let mut reads: FuturesUnordered<BoxFuture<'static, Result<Vec<u8>, EngineError>>> =
            FuturesUnordered::new();
        loop {
            if self.inner.state.load(Ordering::Acquire) != 0 {
                while let Ok(command) = receiver.try_recv() {
                    self.inner
                        .registry
                        .complete(command.id, Err(EngineError::Closed));
                }
                let cause = EngineError::Closed;
                self.inner.registry.fail_lane(index, &active, cause, true);
                self.close_lane(index, &lane, 2);
                return Ok(());
            }
            // A queued request can be canceled before its command reaches the
            // writer. Do not let that stale lane-local ID start a response
            // read, or count it as an outstanding protocol request.
            active.retain(|id| self.inner.registry.metadata(*id).is_some());
            if reads.is_empty() && active.iter().any(|id| self.inner.registry.started(*id)) {
                let lane = Arc::clone(&lane);
                let maximum = active
                    .iter()
                    .filter(|id| self.inner.registry.started(**id))
                    .filter_map(|id| self.inner.registry.metadata(*id))
                    .map(|metadata| metadata.maximum_response_bytes)
                    .max()
                    .unwrap_or(0);
                reads.push(
                    async move {
                        lane.read_response(maximum)
                            .await
                            .map_err(EngineError::Transport)
                    }
                    .boxed(),
                );
            }
            if write.is_none() {
                if let Some(command) = write_queue.pop_front() {
                    let request = command.request;
                    let id = command.id;
                    let metadata = command.metadata;
                    if self.inner.registry.metadata(id).is_none() {
                        active.remove(&id);
                        continue;
                    }
                    let lane = Arc::clone(&lane);
                    let registry = Arc::clone(&self.inner.registry);
                    write = Some(
                        async move {
                            if !registry.mark_started(id) {
                                // Cancellation won the race before the lane
                                // runner crossed its write boundary. The
                                // command was already dequeued, so report a
                                // no-op completion and let the runner retire
                                // its local active marker without failing the
                                // lane.
                                return Ok((id, metadata));
                            }
                            lane.write_request(request)
                                .await
                                .map_err(|error| (id, EngineError::Transport(error)))?;
                            Ok((id, metadata))
                        }
                        .boxed(),
                    );
                }
            }
            let command = receiver.recv().fuse();
            let write_future = async {
                match write.as_mut() {
                    Some(future) => future.await,
                    None => std::future::pending().await,
                }
            }
            .fuse();
            let read_future = async {
                if reads.is_empty() {
                    std::future::pending::<Option<Result<Vec<u8>, EngineError>>>().await
                } else {
                    reads.next().await
                }
            }
            .fuse();
            let state_future = futures_util::future::poll_fn(|context| {
                if self.inner.state.load(Ordering::Acquire) != 0 {
                    Poll::Ready(())
                } else {
                    let mut wakers = self
                        .inner
                        .drain_wakers
                        .lock()
                        .expect("drain waker lock is not poisoned");
                    if !wakers.iter().any(|waker| waker.will_wake(context.waker())) {
                        wakers.push(context.waker().clone());
                    }
                    Poll::Pending
                }
            })
            .fuse();
            pin_mut!(command, write_future, read_future, state_future);
            select! {
                _ = state_future => continue,
                next = command => {
                    match next {
                        Ok(command) => {
                            if self.inner.registry.metadata(command.id).is_some() {
                                active.insert(command.id);
                                write_queue.push_back(command);
                            }
                        }
                        Err(_) => {
                            self.inner.registry.fail_lane(
                                index,
                                &active,
                                EngineError::Closed,
                                true,
                            );
                            self.close_lane(index, &lane, 1);
                            return Ok(());
                        }
                    }
                }
                write_result = write_future => {
                    write = None;
                    match write_result {
                        Ok((id, _metadata)) => {
                            if self.inner.registry.metadata(id).is_some() {
                                self.inner.registry.mark_transmitted(id);
                            } else {
                                active.remove(&id);
                                if active.is_empty() {
                                    // A read future may have been installed
                                    // before this dequeued command observed
                                    // cancellation. With no admitted request
                                    // left, drop that pending read so the
                                    // lane cannot wait forever for a response
                                    // to a command that never crossed the
                                    // write boundary.
                                    reads = FuturesUnordered::new();
                                }
                            }
                        }
                        Err((id, error)) => {
                            if let EngineError::Transport(transport) = &error {
                                if transport.transmitted {
                                    self.inner.registry.mark_transmitted(id);
                                }
                            }
                            self.inner.registry.fail_lane(index, &active, error, false);
                            self.close_lane(index, &lane, 1);
                            return Ok(());
                        }
                    }
                }
                result = read_future => {
                    if let Some(result) = result {
                        match result {
                        Ok(frame) => {
                            let Some(request_id) = Response::decode_header(&frame)
                                .ok()
                                .flatten()
                                .map(|header| header.request_id())
                            else {
                                self.inner.registry.fail_lane(
                                    index,
                                    &active,
                                    EngineError::Protocol("response header is malformed".into()),
                                    true,
                                );
                                self.close_lane(index, &lane, 1);
                                return Ok(());
                            };
                            if !active.contains(&request_id) {
                                self.inner.registry.fail_lane(
                                    index,
                                    &active,
                                    EngineError::Protocol(format!(
                                        "response request ID {request_id} is not outstanding"
                                    )),
                                    true,
                                );
                                self.close_lane(index, &lane, 1);
                                return Ok(());
                            }
                            if !self.inner.registry.started(request_id) {
                                self.inner.registry.fail_lane(
                                    index,
                                    &active,
                                    EngineError::Protocol(format!(
                                        "response request ID {request_id} arrived before request transmission"
                                    )),
                                    true,
                                );
                                self.close_lane(index, &lane, 1);
                                return Ok(());
                            }
                            let metadata = self.inner.registry.metadata(request_id);
                            let Some(metadata) = metadata else {
                                self.inner.registry.fail_lane(index, &active, EngineError::Protocol("response request ID was cancelled or duplicated".into()), false);
                                self.close_lane(index, &lane, 1);
                                return Ok(());
                            };
                            match ResponseBytes::decode(frame, metadata) {
                                Ok(response) => {
                                    active.remove(&request_id);
                                    self.inner.registry.complete(request_id, Ok(response));
                                }
                                Err(error) => {
                                    // A malformed or semantically
                                    // inapplicable response poisons the lane.
                                    // Keep the offending request in `active`
                                    // so all in-flight requests fail together.
                                    self.inner.registry.fail_lane(index, &active, error, true);
                                    self.close_lane(index, &lane, 1);
                                    return Ok(());
                                }
                            }
                        }
                        Err(error) => {
                            self.inner.registry.fail_lane(index, &active, error, true);
                            self.close_lane(index, &lane, 1);
                            return Ok(());
                        }
                    }
                    }
                }
            }
        }
    }

    /// Initiates idempotent shutdown and completes every pending caller once.
    pub fn shutdown(&self) {
        if self
            .inner
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.inner.registry.fail_all(EngineError::Closed);
            for (index, lane) in self.inner.lanes.iter().enumerate() {
                self.close_lane(index, lane, 2);
            }
            let waiters = std::mem::take(
                &mut *self
                    .inner
                    .drain_wakers
                    .lock()
                    .expect("drain waker lock is not poisoned"),
            );
            for waiter in waiters {
                waiter.wake();
            }
        }
    }

    /// Marks the engine closed after shutdown/drain has been requested.
    pub async fn drain(&self) {
        self.shutdown();
        futures_util::future::poll_fn(|context| {
            if self.inner.running_lanes.load(Ordering::Acquire) == 0 {
                self.inner.state.store(2, Ordering::Release);
                return Poll::Ready(());
            }
            self.inner
                .drain_wakers
                .lock()
                .expect("drain waker lock is not poisoned")
                .push(context.waker().clone());
            Poll::Pending
        })
        .await;
    }

    fn close_lane(&self, index: usize, lane: &Arc<dyn TransportLane>, terminal_state: u8) {
        if self.inner.lane_states[index]
            .compare_exchange(0, terminal_state, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            lane.close();
        }
    }
}

struct LaneRunnerGuard {
    inner: Arc<RequestEngineInner>,
}

impl Drop for LaneRunnerGuard {
    fn drop(&mut self) {
        self.inner.running_lanes.fetch_sub(1, Ordering::AcqRel);
        let waiters = std::mem::take(
            &mut *self
                .inner
                .drain_wakers
                .lock()
                .expect("drain waker lock is not poisoned"),
        );
        for waiter in waiters {
            waiter.wake();
        }
    }
}
