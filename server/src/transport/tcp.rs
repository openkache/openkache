//! TLS 1.3 over TCP, one request lane per connection.
//!
//! [`OneLaneConnection`] owns the provider-neutral TLS and framing state
//! machine. [`TlsTcpLane`] adapts it to the selected runtime's TCP stream;
//! no caller may expose a plaintext socket or bypass the bounded record/frame
//! checks below.

use std::collections::VecDeque;
use std::future::Future;
use std::io::{Cursor, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::tls::{self, ConformanceError};
use crate::network_runtime::TcpStream;
use crate::protocol::RequestFrame as ProtocolRequestFrame;
use crate::transport::{
    ReceiveStream, RequestBudget, RequestFrame, RequestRead, SendStream, StreamReadError,
    TransportError,
};
use futures_util::lock::Mutex;
use crate::openkache_protocol::ResponseParts;

/// TLS record payloads are limited to 2^14 bytes by TLS 1.3. The extra
/// allowance covers the record header and AEAD expansion without accepting an
/// unbounded encrypted read from a peer.
pub(crate) const MAX_TLS_RECORD_BYTES: usize = (1 << 14) + 256 + 5;

/// Default aggregate request budget for a TCP lane.
///
/// Requests are returned to the caller as soon as their complete frames are
/// available, so this budget accounts for the request bytes that remain
/// outstanding until the corresponding responses are finished. Callers that
/// have a tighter memory budget should use [`OneLaneConnection::with_limits`].
#[allow(dead_code)]
const DEFAULT_MAX_IN_FLIGHT_BYTES_MULTIPLIER: usize = 64;

/// One-lane connection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaneState {
    /// TLS handshake is still in progress.
    Handshaking,
    /// A request may be received.
    Open,
    /// At least one request has been delivered and its response is pending.
    AwaitingResponse,
    /// The peer sent `close_notify`; finish outstanding responses then close.
    Draining,
    /// A clean TLS close completed.
    Closed,
    /// TCP EOF or a transport error arrived without `close_notify`.
    Unclean,
}

/// Result of feeding one encrypted TLS record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReceiveEvent {
    /// More handshake or application data is required.
    NeedMore,
    /// The TLS handshake completed and the lane is ready.
    Ready,
    /// One complete bounded request frame is available.
    Request(Vec<u8>),
    /// The peer sent `close_notify`; the response direction must be drained.
    PeerCloseNotify,
    /// The aggregate in-flight request budget is exhausted.
    ///
    /// The caller must finish at least one outstanding response before asking
    /// for another event. No additional encrypted bytes should be read while
    /// backpressure is active.
    Backpressure,
}

/// Errors that retire a TCP lane.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TcpTransportError {
    #[error("TLS record is shorter than its five-byte header")]
    RecordTooShort,
    #[error("TLS record exceeds the {MAX_TLS_RECORD_BYTES}-byte limit")]
    RecordTooLarge,
    #[error("TLS record contains trailing bytes")]
    RecordTrailingBytes,
    #[error("TLS handshake did not satisfy the OpenKache profile: {0}")]
    Conformance(#[from] ConformanceError),
    #[error("TLS processing failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("TLS record I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("request frame exceeded the protocol limit")]
    FrameTooLarge,
    #[error("request frame is not valid: {0}")]
    Protocol(#[from] crate::openkache_protocol::ProtocolError),
    #[error("request data arrived after TLS close_notify")]
    PipelinedRequest,
    #[error("TLS close_notify arrived in the middle of a request frame")]
    TruncatedFrame,
    #[error("the TCP peer closed without TLS close_notify")]
    UncleanClose,
    #[error("the lane is already closed")]
    Closed,
    #[error("no response is pending on the lane")]
    NoPendingResponse,
}

/// A bounded TLS 1.3 connection carrying one request lane.
pub(crate) struct OneLaneConnection {
    tls: rustls::ServerConnection,
    state: LaneState,
    plaintext: Vec<u8>,
    max_frame: usize,
    max_in_flight_bytes: usize,
    in_flight_bytes: usize,
    pending_request_sizes: VecDeque<usize>,
    peer_close_seen: bool,
    peer_close_event_emitted: bool,
    read_paused: bool,
}

/// Runtime-owned TLS-over-TCP lane.
///
/// `OneLaneConnection` contains the provider-neutral TLS and frame state
/// machine.  This wrapper adds the selected network runtime's TCP stream and
/// exposes the same receive/send traits used by QUIC request dispatch.  The
/// two halves share one mutex because rustls keeps record reads and writes in
/// one connection object; request dispatch remains sequential on this lane,
/// while complete pipelined frames stay buffered in `OneLaneConnection`.
pub(crate) struct TlsTcpLane {
    inner: Arc<Mutex<TlsTcpLaneInner>>,
}

struct TlsTcpLaneInner {
    stream: TcpStream,
    connection: OneLaneConnection,
}

pub(crate) struct TlsTcpReceiveStream {
    lane: Arc<Mutex<TlsTcpLaneInner>>,
}

pub(crate) struct TlsTcpSendStream {
    lane: Arc<Mutex<TlsTcpLaneInner>>,
}

impl TlsTcpLane {
    pub(crate) fn new(
        stream: TcpStream,
        config: Arc<rustls::ServerConfig>,
        max_frame: usize,
        max_in_flight_bytes: usize,
    ) -> Result<Self, rustls::Error> {
        Ok(Self {
            inner: Arc::new(Mutex::new(TlsTcpLaneInner {
                stream,
                connection: OneLaneConnection::with_limits(
                    config,
                    max_frame,
                    max_in_flight_bytes,
                )?,
            })),
        })
    }

    pub(crate) fn split(&self) -> (TlsTcpSendStream, TlsTcpReceiveStream) {
        (
            TlsTcpSendStream {
                lane: Arc::clone(&self.inner),
            },
            TlsTcpReceiveStream {
                lane: Arc::clone(&self.inner),
            },
        )
    }

    /// Completes the TLS handshake before request admission.
    pub(crate) async fn handshake(&self, timeout: Duration) -> Result<(), TransportError> {
        let result = crate::network_runtime::timeout(timeout, async {
            loop {
                let mut lane = self.inner.lock().await;
                if lane.connection.state() != LaneState::Handshaking {
                    return Ok::<_, TcpTransportError>(());
                }
                let record = read_record(&mut lane.stream).await?;
                let Some(record) = record else {
                    let _ = lane.connection.receive_eof()?;
                    return Err(TcpTransportError::UncleanClose);
                };
                lane.connection.receive_record(&record)?;
                flush_records(&mut lane).await?;
            }
        })
        .await
        .map_err(|_| TransportError::backend("tls-tcp", "handshake", "timed out"))?;
        result.map_err(|error| tcp_error(error, "handshake"))
    }

    pub(crate) async fn peer_certificate(
        &self,
    ) -> Option<rustls::pki_types::CertificateDer<'static>> {
        self.inner
            .lock()
            .await
            .connection
            .peer_certificate()
    }

    /// Sends a close notification and flushes the resulting TLS records.
    pub(crate) async fn close(&self) {
        let mut lane = self.inner.lock().await;
        lane.connection.close();
        let _ = flush_records(&mut lane).await;
    }
}

impl OneLaneConnection {
    fn peer_certificate(&self) -> Option<rustls::pki_types::CertificateDer<'static>> {
        self.tls
            .peer_certificates()
            .and_then(|certificates| certificates.first().cloned())
    }
}

impl ReceiveStream for TlsTcpReceiveStream {
    fn supports_concurrent_read(&self) -> bool {
        false
    }

    fn read_request<T>(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
        progress: &AtomicBool,
        admit: impl FnOnce(
            crate::openkache_protocol::RequestFrameHeader,
            &[u8],
        ) -> Result<(), T>,
    ) -> impl Future<Output = Result<RequestRead<T>, StreamReadError>> {
        let lane = Arc::clone(&self.lane);
        let budget = budget.clone();
        async move {
            crate::network_runtime::timeout(timeout, async {
                let mut admit = Some(admit);
                loop {
                    let event = {
                        let mut state = lane.lock().await;
                        match state
                            .connection
                            .take_event()
                            .map_err(map_stream_error)?
                        {
                            ReceiveEvent::NeedMore | ReceiveEvent::Ready => {
                                let record = read_record(&mut state.stream)
                                    .await
                                    .map_err(|error| tcp_error(error, "read"))?;
                                let Some(record) = record else {
                                    match state.connection.receive_eof() {
                                        Ok(ReceiveEvent::PeerCloseNotify)
                                        | Ok(ReceiveEvent::NeedMore) => {
                                            return Ok(RequestRead::Finished);
                                        }
                                        Ok(ReceiveEvent::Request(_)) => {
                                            return Err(StreamReadError::Transport(
                                                TransportError::backend(
                                                    "tls-tcp",
                                                    "read",
                                                    "TLS close_notify interrupted a request frame",
                                                ),
                                            ));
                                        }
                                        Ok(ReceiveEvent::Backpressure)
                                        | Ok(ReceiveEvent::Ready) => {
                                            return Err(StreamReadError::Transport(
                                                TransportError::backend(
                                                    "tls-tcp",
                                                    "read",
                                                    "stream ended before a request frame",
                                                ),
                                            ));
                                        }
                                        Err(error) => return Err(map_stream_error(error)),
                                    }
                                };
                                let event = state
                                    .connection
                                    .receive_record(&record)
                                    .map_err(map_stream_error)?;
                                flush_records(&mut state)
                                    .await
                                    .map_err(|error| tcp_error(error, "write"))?;
                                event
                            }
                            event => event,
                        }
                    };

                    match event {
                        ReceiveEvent::Request(frame) => {
                            progress.store(true, Ordering::Relaxed);
                            if frame.len() > maximum {
                                return Err(StreamReadError::TooLarge);
                            }
                            let header =
                                ProtocolRequestFrame::decode_header(&frame)?.ok_or(
                                    crate::openkache_protocol::ProtocolError::FrameTooShort {
                                        expected: 1,
                                        actual: frame.len(),
                                    },
                                )?;
                            let frame_len = header.frame_len()?;
                            if frame_len != frame.len() {
                                return Err(StreamReadError::Malformed(
                                    crate::openkache_protocol::ProtocolError::FrameLength {
                                        expected: frame_len,
                                        actual: frame.len(),
                                    },
                                ));
                            }
                            let rejection = admit
                                .take()
                                .expect("request admission callback is called once")(
                                header,
                                &frame[..header.encoded_len()],
                            )
                                .err();
                            let permit = match budget.acquire(header.body_len(), timeout).await {
                                Ok(permit) => permit,
                                Err(StreamReadError::Timeout) => {
                                    return Ok(RequestRead::Overloaded {
                                        header,
                                        timed_out: true,
                                    });
                                }
                                Err(StreamReadError::TooLarge) => {
                                    return Ok(RequestRead::Overloaded {
                                        header,
                                        timed_out: false,
                                    });
                                }
                                Err(error) => return Err(error),
                            };
                            if let Some(rejection) = rejection {
                                drop(permit);
                                return Ok(RequestRead::Rejected { header, rejection });
                            }
                            return Ok(RequestRead::Frame(RequestFrame::new(
                                header, frame, permit,
                            )));
                        }
                        ReceiveEvent::PeerCloseNotify => {
                            return Ok(RequestRead::Finished);
                        }
                        ReceiveEvent::Backpressure => {
                            // The frame is complete and still buffered, but an
                            // earlier response owns the lane's in-flight
                            // window. Let the connection loop finish that
                            // response before asking for another event.
                            return Ok(RequestRead::Backpressured);
                        }
                        ReceiveEvent::NeedMore | ReceiveEvent::Ready => continue,
                    }
                }
            })
            .await
            .map_err(|_| StreamReadError::Timeout)?
        }
    }

    fn stop(&mut self) {
        // TLS-over-TCP has no directional reset primitive. The lane's
        // connection future is dropped by the caller, which closes the
        // underlying socket and retires both halves.
    }
}

impl SendStream for TlsTcpSendStream {
    fn write_response(
        &mut self,
        parts: ResponseParts,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), TransportError>> {
        let lane = Arc::clone(&self.lane);
        async move {
            let result = crate::network_runtime::timeout(timeout, async {
                let mut state = lane.lock().await;
                let segments = parts.into_segments();
                let response_len = segments.iter().try_fold(0_usize, |length, segment| {
                    length.checked_add(segment.as_slice().len())
                });
                if response_len
                    .map(|length| length > crate::openkache_protocol::MAX_RESPONSE_FRAME_BYTES)
                    .unwrap_or(true)
                {
                    state.connection.state = LaneState::Unclean;
                    return Err(TcpTransportError::FrameTooLarge);
                }
                for segment in segments {
                    state.connection.write_response(segment.as_slice())?;
                }
                state.connection.finish_response()?;
                flush_records(&mut state).await?;
                Ok::<_, TcpTransportError>(())
            })
            .await
            .map_err(|_| TransportError::backend("tls-tcp", "write response", "timed out"))?;
            result.map_err(|error| tcp_error(error, "write response"))
        }
    }

    fn finish(&mut self) -> Result<(), TransportError> {
        Ok(())
    }

    fn reset(&mut self) {
        // Dropping the shared lane closes the socket; there is no separate
        // response-direction reset in the TLS-over-TCP profile.
    }
}

async fn read_record(stream: &mut TcpStream) -> Result<Option<Vec<u8>>, TcpTransportError> {
    let mut header = vec![0_u8; 5];
    let read = read_exact(stream, &mut header).await?;
    if read == 0 {
        return Ok(None);
    }
    if read != header.len() {
        return Err(TcpTransportError::RecordTooShort);
    }
    let payload_len = u16::from_be_bytes([header[3], header[4]]) as usize;
    let record_len = payload_len
        .checked_add(5)
        .ok_or(TcpTransportError::RecordTooLarge)?;
    if record_len > MAX_TLS_RECORD_BYTES {
        return Err(TcpTransportError::RecordTooLarge);
    }
    let mut record = header;
    record.resize(record_len, 0);
    if read_exact(stream, &mut record[5..]).await? != payload_len {
        return Err(TcpTransportError::RecordTooShort);
    }
    validate_record(&record)?;
    Ok(Some(record))
}

async fn read_exact(stream: &mut TcpStream, buffer: &mut [u8]) -> Result<usize, TcpTransportError> {
    let mut offset = 0;
    while offset < buffer.len() {
        let input = vec![0_u8; buffer.len() - offset];
        let (read, input) = stream.read(input).await?;
        if read == 0 {
            return Ok(offset);
        }
        buffer[offset..offset + read].copy_from_slice(&input[..read]);
        offset += read;
    }
    Ok(offset)
}

async fn flush_records(state: &mut TlsTcpLaneInner) -> Result<(), TcpTransportError> {
    for record in state.connection.take_records()? {
        state.stream.write_all(record).await?;
    }
    Ok(())
}

fn map_stream_error(error: TcpTransportError) -> StreamReadError {
    match error {
        TcpTransportError::Protocol(error) => StreamReadError::Malformed(error),
        TcpTransportError::FrameTooLarge | TcpTransportError::RecordTooLarge => {
            StreamReadError::TooLarge
        }
        error => StreamReadError::Transport(tcp_error(error, "read")),
    }
}

fn tcp_error(error: impl std::fmt::Display, operation: &'static str) -> TransportError {
    TransportError::backend("tls-tcp", operation, error)
}

impl OneLaneConnection {
    /// Creates a server-side TLS connection using a strict profile config.
    #[allow(dead_code)]
    pub(crate) fn new(
        config: Arc<rustls::ServerConfig>,
        max_frame: usize,
    ) -> Result<Self, rustls::Error> {
        Self::with_limits(
            config,
            max_frame,
            max_frame.saturating_mul(DEFAULT_MAX_IN_FLIGHT_BYTES_MULTIPLIER),
        )
    }

    /// Creates a server-side TLS connection with an explicit aggregate budget.
    ///
    /// `max_frame` bounds every individual request and response. The
    /// in-flight budget bounds the total size of complete request frames that
    /// have been delivered to the request engine but whose responses have not
    /// yet been finished. This keeps pipelined requests finite without
    /// imposing request/response lockstep on the one lane.
    pub(crate) fn with_limits(
        config: Arc<rustls::ServerConfig>,
        max_frame: usize,
        max_in_flight_bytes: usize,
    ) -> Result<Self, rustls::Error> {
        assert!(max_frame > 0, "TCP frame limit must be greater than zero");
        assert!(
            max_in_flight_bytes >= max_frame,
            "TCP in-flight budget must fit at least one complete frame"
        );
        Ok(Self {
            tls: rustls::ServerConnection::new(config)?,
            state: LaneState::Handshaking,
            plaintext: Vec::new(),
            max_frame,
            max_in_flight_bytes,
            in_flight_bytes: 0,
            pending_request_sizes: VecDeque::new(),
            peer_close_seen: false,
            peer_close_event_emitted: false,
            read_paused: false,
        })
    }

    /// Returns the current close/request state.
    pub(crate) const fn state(&self) -> LaneState {
        self.state
    }

    /// Returns whether the adapter should read another encrypted record.
    #[allow(dead_code)]
    pub(crate) fn wants_read(&self) -> bool {
        self.tls.wants_read()
            && !self.read_paused
            && !matches!(self.state, LaneState::Closed | LaneState::Unclean)
    }

    /// Returns whether TLS has bytes the adapter must send.
    #[allow(dead_code)]
    pub(crate) fn wants_write(&self) -> bool {
        self.tls.wants_write()
    }

    /// Feeds one complete TLS record and returns at most one application event.
    ///
    /// A caller must preserve record boundaries. Accepting a larger buffer and
    /// allowing Rustls to discover multiple records would defeat the explicit
    /// per-record bound and make close ordering ambiguous.
    pub(crate) fn receive_record(
        &mut self,
        record: &[u8],
    ) -> Result<ReceiveEvent, TcpTransportError> {
        if matches!(self.state, LaneState::Closed | LaneState::Unclean) {
            return Err(TcpTransportError::Closed);
        }
        if self.read_paused {
            return Ok(ReceiveEvent::Backpressure);
        }
        if self.peer_close_seen {
            self.state = LaneState::Unclean;
            return Err(TcpTransportError::PipelinedRequest);
        }
        if let Err(error) = validate_record(record) {
            self.state = LaneState::Unclean;
            return Err(error);
        }
        let mut input = Cursor::new(record);
        let read = match self.tls.read_tls(&mut input) {
            Ok(read) => read,
            Err(error) => {
                self.state = LaneState::Unclean;
                return Err(error.into());
            }
        };
        if read != record.len() {
            self.state = LaneState::Unclean;
            return Err(TcpTransportError::RecordTrailingBytes);
        }
        let io_state = self.tls.process_new_packets().map_err(|error| {
            self.state = LaneState::Unclean;
            TcpTransportError::Tls(error)
        })?;
        let handshake_completed =
            !self.tls.is_handshaking() && self.state == LaneState::Handshaking;
        if handshake_completed {
            if let Err(error) = tls::validate_negotiated(&self.tls) {
                self.state = LaneState::Unclean;
                return Err(TcpTransportError::Conformance(error));
            }
            self.state = LaneState::Open;
        }

        // `rustls::Reader::read_to_end` waits for EOF and reports
        // `WouldBlock` while a handshake or application record is still
        // in flight. Drain only the plaintext currently available so one
        // bounded TLS record never turns a normal incremental read into an
        // I/O failure.
        let buffered_limit = self.max_in_flight_bytes.saturating_add(self.max_frame);
        loop {
            let mut bytes = [0_u8; 16 * 1024];
            let read = match self.tls.reader().read(&mut bytes) {
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.state = LaneState::Unclean;
                    return Err(TcpTransportError::Io(error));
                }
            };
            if read == 0 {
                break;
            }
            if read > buffered_limit.saturating_sub(self.plaintext.len()) {
                self.state = LaneState::Unclean;
                return Err(TcpTransportError::FrameTooLarge);
            }
            self.plaintext.try_reserve(read).map_err(|_| {
                self.state = LaneState::Unclean;
                TcpTransportError::FrameTooLarge
            })?;
            self.plaintext.extend_from_slice(&bytes[..read]);
        }
        if io_state.peer_has_closed() {
            self.peer_close_seen = true;
            self.state = LaneState::Draining;
        }
        let event = self.next_event()?;
        if handshake_completed && matches!(event, ReceiveEvent::NeedMore) {
            Ok(ReceiveEvent::Ready)
        } else {
            Ok(event)
        }
    }

    /// Reports a TCP EOF. EOF is clean only after TLS `close_notify`.
    pub(crate) fn receive_eof(&mut self) -> Result<ReceiveEvent, TcpTransportError> {
        if matches!(self.state, LaneState::Closed | LaneState::Unclean) {
            return Err(TcpTransportError::Closed);
        }
        if self.peer_close_seen {
            self.peer_close_seen = true;
            self.state = LaneState::Draining;
            if !self.plaintext.is_empty() {
                let additional = match ProtocolRequestFrame::header_bytes_needed(&self.plaintext) {
                    Ok(additional) => additional,
                    Err(error) => {
                        self.state = LaneState::Unclean;
                        return Err(error.into());
                    }
                };
                if additional != 0 {
                    self.state = LaneState::Unclean;
                    return Err(TcpTransportError::TruncatedFrame);
                }
            }
            return self.next_event();
        }
        self.state = LaneState::Unclean;
        Err(TcpTransportError::UncleanClose)
    }

    /// Returns the next application event already buffered by TLS.
    ///
    /// A socket adapter should call this after handling a `Request`,
    /// `PeerCloseNotify`, or `Backpressure` event before reading another
    /// record. This lets one TLS record carry several bounded, pipelined
    /// request frames without allocating an unbounded queue.
    pub(crate) fn take_event(&mut self) -> Result<ReceiveEvent, TcpTransportError> {
        self.next_event()
    }

    /// Marks one request as answered and returns the lane to `Open` when no
    /// other request is outstanding.
    pub(crate) fn finish_response(&mut self) -> Result<(), TcpTransportError> {
        if self.in_flight_bytes == 0 {
            if self.state == LaneState::Draining && self.plaintext.is_empty() {
                self.tls.send_close_notify();
                self.state = LaneState::Closed;
                return Ok(());
            }
            return Err(TcpTransportError::NoPendingResponse);
        }
        let request_size = self
            .pending_request_sizes
            .pop_front()
            .expect("in-flight bytes and request sizes stay in sync");
        self.in_flight_bytes -= request_size;
        self.read_paused = false;
        match self.state {
            LaneState::AwaitingResponse if self.in_flight_bytes == 0 => {
                self.state = LaneState::Open;
                Ok(())
            }
            LaneState::AwaitingResponse | LaneState::Open => Ok(()),
            LaneState::Draining => {
                if self.in_flight_bytes == 0 && self.plaintext.is_empty() {
                    self.tls.send_close_notify();
                    self.state = LaneState::Closed;
                }
                Ok(())
            }
            LaneState::Handshaking => {
                unreachable!("a response cannot be pending before the handshake")
            }
            LaneState::Closed | LaneState::Unclean => Err(TcpTransportError::Closed),
        }
    }

    /// Writes one bounded response into the TLS plaintext stream.
    pub(crate) fn write_response(&mut self, response: &[u8]) -> Result<(), TcpTransportError> {
        if self.in_flight_bytes == 0
            || !matches!(
                self.state,
                LaneState::AwaitingResponse | LaneState::Draining
            )
        {
            return Err(TcpTransportError::Closed);
        }
        if response.len() > crate::openkache_protocol::MAX_RESPONSE_FRAME_BYTES {
            self.state = LaneState::Unclean;
            return Err(TcpTransportError::FrameTooLarge);
        }
        if let Err(error) = self.tls.writer().write_all(response) {
            self.state = LaneState::Unclean;
            return Err(error.into());
        }
        Ok(())
    }

    /// Starts a clean server close and returns TLS records through
    /// [`Self::take_records`].
    pub(crate) fn close(&mut self) {
        if !matches!(self.state, LaneState::Closed | LaneState::Unclean) {
            self.tls.send_close_notify();
            self.state = LaneState::Closed;
        }
    }

    /// Drains all currently buffered TLS records.
    pub(crate) fn take_records(&mut self) -> Result<Vec<Vec<u8>>, TcpTransportError> {
        let result = (|| {
            let mut records = Vec::new();
            while self.tls.wants_write() {
                let mut output = Cursor::new(Vec::new());
                self.tls.write_tls(&mut output)?;
                let output = output.into_inner();
                if output.is_empty() {
                    break;
                }
                records.extend(split_records(&output)?);
            }
            Ok(records)
        })();
        if result.is_err() {
            self.state = LaneState::Unclean;
        }
        result
    }

    fn next_event(&mut self) -> Result<ReceiveEvent, TcpTransportError> {
        let result = self.next_event_inner();
        if result.is_err() {
            self.state = LaneState::Unclean;
        }
        result
    }

    fn next_event_inner(&mut self) -> Result<ReceiveEvent, TcpTransportError> {
        if self.state == LaneState::Handshaking {
            return Ok(ReceiveEvent::NeedMore);
        }
        if !self.plaintext.is_empty() {
            let additional = ProtocolRequestFrame::header_bytes_needed(&self.plaintext)?;
            if additional != 0 {
                if self.peer_close_seen {
                    self.state = LaneState::Unclean;
                    return Err(TcpTransportError::TruncatedFrame);
                }
                if additional > self.max_frame.saturating_sub(self.plaintext.len()) {
                    return Err(TcpTransportError::FrameTooLarge);
                }
                return Ok(ReceiveEvent::NeedMore);
            }
            let header = ProtocolRequestFrame::decode_header(&self.plaintext)?.ok_or(
                crate::openkache_protocol::ProtocolError::FrameTooShort {
                    expected: 1,
                    actual: self.plaintext.len(),
                },
            )?;
            let frame_len = header.frame_len()?;
            if frame_len > self.max_frame {
                return Err(TcpTransportError::FrameTooLarge);
            }
            if self.plaintext.len() < frame_len {
                if self.peer_close_seen {
                    self.state = LaneState::Unclean;
                    return Err(TcpTransportError::TruncatedFrame);
                }
                return Ok(ReceiveEvent::NeedMore);
            }
            if self.in_flight_bytes.saturating_add(frame_len) > self.max_in_flight_bytes {
                self.read_paused = true;
                return Ok(ReceiveEvent::Backpressure);
            }
            let frame = self.plaintext.drain(..frame_len).collect();
            self.in_flight_bytes += frame_len;
            self.pending_request_sizes.push_back(frame_len);
            if !self.peer_close_seen {
                self.state = LaneState::AwaitingResponse;
            }
            return Ok(ReceiveEvent::Request(frame));
        }
        if self.peer_close_seen && !self.peer_close_event_emitted {
            self.peer_close_event_emitted = true;
            return Ok(ReceiveEvent::PeerCloseNotify);
        }
        if self.peer_close_seen && self.in_flight_bytes == 0 {
            // The caller may have observed the close event before finishing
            // the final response. `finish_response` emits the server close
            // notification once the response bytes have been written.
            self.state = LaneState::Draining;
        }
        Ok(ReceiveEvent::NeedMore)
    }
}

fn validate_record(record: &[u8]) -> Result<(), TcpTransportError> {
    if record.len() < 5 {
        return Err(TcpTransportError::RecordTooShort);
    }
    if record.len() > MAX_TLS_RECORD_BYTES {
        return Err(TcpTransportError::RecordTooLarge);
    }
    let payload_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    if payload_len + 5 != record.len() {
        return Err(if payload_len + 5 < record.len() {
            TcpTransportError::RecordTrailingBytes
        } else {
            TcpTransportError::RecordTooShort
        });
    }
    Ok(())
}

/// Splits the bytes emitted by Rustls into complete, bounded TLS records.
///
/// `ServerConnection::write_tls` may drain more than one record in a single
/// call. The TCP adapter must still write records as distinct bounded units so
/// the receive side can retain its one-record validation boundary.
fn split_records(output: &[u8]) -> Result<Vec<Vec<u8>>, TcpTransportError> {
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < output.len() {
        let remaining = &output[offset..];
        if remaining.len() < 5 {
            return Err(TcpTransportError::RecordTooShort);
        }
        let payload_len = u16::from_be_bytes([remaining[3], remaining[4]]) as usize;
        let record_len = payload_len
            .checked_add(5)
            .ok_or(TcpTransportError::RecordTooLarge)?;
        if record_len > MAX_TLS_RECORD_BYTES {
            return Err(TcpTransportError::RecordTooLarge);
        }
        if record_len > remaining.len() {
            return Err(TcpTransportError::RecordTooShort);
        }
        records.push(remaining[..record_len].to_vec());
        offset += record_len;
    }
    Ok(records)
}
