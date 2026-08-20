//! TLS 1.3 over TCP, one request lane per connection.
//!
//! This module deliberately stops at the provider-neutral connection
//! boundary. Runtime adapters feed complete TLS records into
//! [`OneLaneConnection::receive_record`] and write records returned by
//! [`OneLaneConnection::take_records`]. No adapter may expose a plaintext
//! socket or bypass the bounded record/frame checks below.

use std::collections::VecDeque;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use super::tls::{self, ConformanceError};
use crate::protocol::RequestFrame as ProtocolRequestFrame;

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
    Protocol(#[from] openkache_protocol::ProtocolError),
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

impl OneLaneConnection {
    /// Creates a server-side TLS connection using a strict profile config.
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
    pub(crate) fn wants_read(&self) -> bool {
        self.tls.wants_read()
            && !self.read_paused
            && !matches!(self.state, LaneState::Closed | LaneState::Unclean)
    }

    /// Returns whether TLS has bytes the adapter must send.
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
        validate_record(record)?;
        let mut input = Cursor::new(record);
        let read = self.tls.read_tls(&mut input)?;
        if read != record.len() {
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
                Err(error) => return Err(TcpTransportError::Io(error)),
            };
            if read == 0 {
                break;
            }
            if read > buffered_limit.saturating_sub(self.plaintext.len()) {
                return Err(TcpTransportError::FrameTooLarge);
            }
            self.plaintext
                .try_reserve(read)
                .map_err(|_| TcpTransportError::FrameTooLarge)?;
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
                let additional = ProtocolRequestFrame::header_bytes_needed(&self.plaintext)?;
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
        if response.len() > self.max_frame {
            return Err(TcpTransportError::FrameTooLarge);
        }
        self.tls.writer().write_all(response)?;
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
    }

    fn next_event(&mut self) -> Result<ReceiveEvent, TcpTransportError> {
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
                openkache_protocol::ProtocolError::FrameTooShort {
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
