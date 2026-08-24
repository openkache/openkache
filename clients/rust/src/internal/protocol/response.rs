//! Operation-neutral response framing and owned response buffers.
//!
//! Response status and payload framing are shared by clients and servers.
//! This module intentionally does not decode operation-specific field
//! semantics; callers pass the borrowed payload to the API codec that owns
//! those semantics.

use smallvec::SmallVec;

use crate::internal_protocol::{
    MAX_VARUINT_BYTES, MIN_VARUINT_BYTES, OperationLayoutPlan, ProtocolError, RESPONSE_FIXED_BYTES,
    ResponseSegment, Result, SegmentFrame, Status, WireSegment, decode_varuint, encode_varuint,
    validate_value_length,
};

const _: () = assert!(RESPONSE_FIXED_BYTES + MAX_VARUINT_BYTES + MAX_VARUINT_BYTES <= 32);

/// Metadata required to delimit one response with an opaque payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    status: Status,
    request_id: u64,
    encoded_len: usize,
    payload_len: usize,
}

impl ResponseHeader {
    /// Returns the decoded status.
    pub const fn status(self) -> Status {
        self.status
    }

    /// Returns the echoed request correlation token.
    pub const fn request_id(self) -> u64 {
        self.request_id
    }

    /// Returns the number of bytes before the opaque payload.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the payload length.
    pub const fn payload_len(self) -> usize {
        self.payload_len
    }

    /// Returns the complete response frame length.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

pub(crate) fn decode_response_header(prefix: &[u8]) -> Result<Option<ResponseHeader>> {
    let Some(&status_byte) = prefix.first() else {
        return Ok(None);
    };
    let status = Status::try_from(status_byte)?;
    let Some((request_id, request_id_len)) = decode_varuint(
        prefix.get(RESPONSE_FIXED_BYTES..).unwrap_or_default(),
        "response request ID",
    )?
    else {
        return Ok(None);
    };
    let payload_offset = RESPONSE_FIXED_BYTES
        .checked_add(request_id_len)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    let Some((payload_len, payload_len_bytes)) = decode_varuint(
        prefix.get(payload_offset..).unwrap_or_default(),
        "response payload length",
    )?
    else {
        return Ok(None);
    };
    let payload_len =
        usize::try_from(payload_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_value_length(payload_len)?;
    Ok(Some(ResponseHeader {
        status,
        request_id,
        encoded_len: payload_offset
            .checked_add(payload_len_bytes)
            .ok_or(ProtocolError::FrameLengthOverflow)?,
        payload_len,
    }))
}

/// A complete response viewed as an opaque status and payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFrame<'a> {
    status: Status,
    request_id: u64,
    frame: &'a [u8],
    payload_offset: usize,
}

impl<'a> ResponseFrame<'a> {
    /// Decodes one complete response without interpreting its payload.
    pub fn decode(frame: &'a [u8]) -> Result<Self> {
        let header = decode_response_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + 2 * MIN_VARUINT_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            status: header.status,
            request_id: header.request_id,
            frame,
            payload_offset: header.encoded_len,
        })
    }

    /// Returns the response status.
    pub const fn status(self) -> Status {
        self.status
    }

    /// Returns the echoed request correlation token.
    pub const fn request_id(self) -> u64 {
        self.request_id
    }

    /// Returns the opaque response payload.
    pub fn payload(self) -> &'a [u8] {
        &self.frame[self.payload_offset..]
    }

    /// Returns the original complete encoded frame.
    pub const fn encoded(self) -> &'a [u8] {
        self.frame
    }
}

/// An owned response frame with a borrowed payload view.
///
/// The frame allocation stays intact while callers inspect the payload. This
/// is the allocation-free counterpart to [`Response::decode_owned`], whose
/// conventional `Response { payload: Vec<u8> }` shape must move the payload
/// over the status/length prefix. Use this type when an API field view can
/// retain the received frame for its lifetime.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedResponseFrame {
    header: ResponseHeader,
    frame: Vec<u8>,
}

impl OwnedResponseFrame {
    /// Decodes one complete response while retaining its original allocation.
    pub fn decode(frame: Vec<u8>) -> Result<Self> {
        let header = Response::decode_header(&frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + 2 * MIN_VARUINT_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self { header, frame })
    }

    /// Returns the decoded status.
    pub const fn status(&self) -> Status {
        self.header.status()
    }

    /// Returns the payload without allocating or copying.
    pub fn payload(&self) -> &[u8] {
        &self.frame[self.header.encoded_len()..]
    }

    /// Returns the byte offset at which the payload starts.
    pub const fn payload_offset(&self) -> usize {
        self.header.encoded_len()
    }

    /// Returns the complete encoded frame.
    pub fn encoded(&self) -> &[u8] {
        &self.frame
    }

    /// Consumes the view and returns the original encoded allocation.
    pub fn into_encoded(self) -> Vec<u8> {
        self.frame
    }
}

/// Generic response frame encoder/decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    /// Correlation token echoed from the corresponding request.
    pub request_id: u64,
    pub payload: Vec<u8>,
}

/// Owned response pieces ready for a transport write.
///
/// Keeping the header separate from the already-owned payload avoids copying
/// storage/application bytes into a second complete-frame allocation.
#[derive(Debug, Eq, PartialEq)]
pub struct ResponseParts {
    pub header: SmallVec<[u8; 32]>,
    pub payload: Vec<u8>,
    /// Additional ownership-preserving payload segments in wire order.
    pub segments: SmallVec<[ResponseSegment; 8]>,
}

/// Inline-owned response header bytes assembled by a streaming transport.
///
/// The compact response header fits in inline storage, so reading its status
/// and canonical length prefix does not require a heap allocation. Moving
/// this owner into [`ResponseParts`] also avoids copying into another header
/// buffer after the frame boundary is known.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct ResponseHeaderBytes {
    bytes: SmallVec<[u8; 32]>,
}

impl ResponseHeaderBytes {
    /// Creates an empty response header owner.
    pub const fn new() -> Self {
        Self {
            bytes: SmallVec::new_const(),
        }
    }

    /// Appends one byte received from the transport.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::FrameLength`] if the bytes exceed the largest
    /// compact response header accepted by this protocol.
    pub fn push(&mut self, byte: u8) -> Result<()> {
        let maximum = RESPONSE_FIXED_BYTES + MAX_VARUINT_BYTES + MAX_VARUINT_BYTES;
        if self.bytes.len() >= maximum {
            return Err(ProtocolError::FrameLength {
                expected: maximum,
                actual: self.bytes.len() + 1,
            });
        }
        self.bytes.push(byte);
        Ok(())
    }

    /// Returns the received header prefix.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the received header byte count.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether no header byte has been received.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl ResponseParts {
    /// Replaces the echoed request ID while preserving all payload owners.
    pub fn with_request_id(mut self, request_id: u64) -> Result<Self> {
        let status = *self.header.first().ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES,
            actual: 0,
        })?;
        let payload_len = self
            .payload
            .len()
            .checked_add(self.segments.iter().try_fold(0usize, |total, segment| {
                total
                    .checked_add(segment.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)
            })?)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let (id_bytes, id_len) = encode_varuint(request_id);
        let (length_bytes, length_len) = encode_varuint(payload_len as u64);
        let mut header = SmallVec::new();
        header.push(status);
        header.extend_from_slice(&id_bytes[..id_len]);
        header.extend_from_slice(&length_bytes[..length_len]);
        self.header = header;
        Ok(self)
    }

    /// Encodes a generated field plan into an ownership-preserving response.
    ///
    /// This is the response projection boundary for every planned operation.
    /// Its inline metadata capacity and canonical layout codecs are protocol
    /// details; servers supply only status, owners, and generated layout.
    ///
    /// # Errors
    ///
    /// Returns an error when values do not match the generated plan, the
    /// layout codec rejects a value, or the complete payload exceeds protocol
    /// limits.
    pub fn planned_fields<I, T>(
        status: Status,
        values: I,
        plan: OperationLayoutPlan,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = Option<T>>,
        T: Into<WireSegment>,
    {
        let frame = crate::internal_protocol::layout::encode_planned_field_segments_in::<
            [WireSegment; 8],
            _,
            _,
        >(
            values,
            plan.fields,
            plan.layout,
            plan.optional_value_codec.as_ref(),
        )?;
        Self::from_frame(status, 0, frame)
    }

    /// Encodes a response header over ownership-preserving body segments.
    pub fn segmented<I, T>(status: Status, segments: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<ResponseSegment>,
    {
        Self::from_frame(status, 0, SegmentFrame::<[WireSegment; 8]>::new(segments)?)
    }

    /// Encodes a response header over an existing owned body frame.
    ///
    /// The frame's checked length and segment storage are consumed directly;
    /// payload owners are neither copied nor collected into another buffer.
    pub(crate) fn from_frame(
        status: Status,
        request_id: u64,
        frame: SegmentFrame<[WireSegment; 8]>,
    ) -> Result<Self> {
        let payload_len = frame.len();
        validate_value_length(payload_len)?;
        let (length, length_bytes) = encode_varuint(payload_len as u64);
        let mut header = SmallVec::new();
        header.push(status as u8);
        let (request_id_bytes, request_id_len) = encode_varuint(request_id);
        header.extend_from_slice(&request_id_bytes[..request_id_len]);
        header.extend_from_slice(&length[..length_bytes]);
        Ok(Self {
            header,
            payload: Vec::new(),
            segments: frame.into_segments(),
        })
    }

    /// Validates independently received header and payload buffers.
    ///
    /// The payload allocation is retained exactly as supplied. This is the
    /// preferred receive-side constructor for transports that read the small
    /// response header and the body separately.
    pub fn decode(header: ResponseHeaderBytes, payload: Vec<u8>) -> Result<Self> {
        let decoded =
            Response::decode_header(header.as_slice())?.ok_or(ProtocolError::FrameTooShort {
                expected: RESPONSE_FIXED_BYTES + 2 * MIN_VARUINT_BYTES,
                actual: header.len(),
            })?;
        if decoded.encoded_len() != header.len() {
            return Err(ProtocolError::FrameLength {
                expected: decoded.encoded_len(),
                actual: header.len(),
            });
        }
        if decoded.payload_len() != payload.len() {
            let expected = decoded
                .encoded_len()
                .checked_add(decoded.payload_len())
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            let actual = header
                .len()
                .checked_add(payload.len())
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            return Err(ProtocolError::FrameLength { expected, actual });
        }
        Ok(Self {
            header: header.bytes,
            payload,
            segments: SmallVec::new(),
        })
    }

    /// Consumes the response into one ordered write plan.
    ///
    /// The response header stays inline, while an existing payload allocation
    /// and additional application/storage segments retain ownership. A
    /// transport can therefore submit or advance one segment sequence without
    /// maintaining separate header, payload, and segmented-body paths.
    pub fn into_segments(self) -> SmallVec<[ResponseSegment; 8]> {
        let mut segments = self.segments;
        segments.reserve(1 + usize::from(!self.payload.is_empty()));
        if !self.payload.is_empty() {
            segments.insert(0, ResponseSegment::Owned(self.payload.into()));
        }
        segments.insert(0, ResponseSegment::Inline(self.header));
        segments
    }

    /// Decodes the status and moves the already-owned payload without copying.
    pub fn into_response(self) -> Result<Response> {
        let decoded =
            Response::decode_header(&self.header)?.ok_or(ProtocolError::FrameTooShort {
                expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
                actual: self.header.len(),
            })?;
        let segment_len = self.segments.iter().try_fold(0usize, |total, segment| {
            total
                .checked_add(segment.len())
                .ok_or(ProtocolError::FrameLengthOverflow)
        })?;
        let actual_payload_len = self
            .payload
            .len()
            .checked_add(segment_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if decoded.encoded_len() != self.header.len() || decoded.payload_len() != actual_payload_len
        {
            let expected = decoded
                .encoded_len()
                .checked_add(decoded.payload_len())
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            let actual = self
                .header
                .len()
                .checked_add(actual_payload_len)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            return Err(ProtocolError::FrameLength { expected, actual });
        }
        let mut payload = self.payload;
        if !self.segments.is_empty() {
            payload.reserve(segment_len);
            for segment in self.segments {
                payload.extend_from_slice(segment.as_slice());
            }
        }
        Ok(Response {
            status: decoded.status(),
            request_id: decoded.request_id(),
            payload,
        })
    }
}

impl Response {
    /// Creates a response after checking the wire payload ceiling.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        Self::new_with_id(status, 0, payload)
    }

    /// Creates a response with an explicit echoed request ID.
    pub fn new_with_id(status: Status, request_id: u64, payload: Vec<u8>) -> Result<Self> {
        validate_value_length(payload.len())?;
        Ok(Self {
            status,
            request_id,
            payload,
        })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_value_length(self.payload.len())?;
        let (request_id, request_id_bytes) = encode_varuint(self.request_id);
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let mut frame = Vec::with_capacity(
            RESPONSE_FIXED_BYTES + request_id_bytes + length_bytes + self.payload.len(),
        );
        frame.push(self.status as u8);
        frame.extend_from_slice(&request_id[..request_id_bytes]);
        frame.extend_from_slice(&length[..length_bytes]);
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Consumes this response without copying its payload.
    pub fn into_parts(self) -> Result<ResponseParts> {
        validate_value_length(self.payload.len())?;
        let (request_id, request_id_bytes) = encode_varuint(self.request_id);
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let mut header = SmallVec::new();
        header.push(self.status as u8);
        header.extend_from_slice(&request_id[..request_id_bytes]);
        header.extend_from_slice(&length[..length_bytes]);
        Ok(ResponseParts {
            header,
            payload: self.payload,
            segments: SmallVec::new(),
        })
    }

    /// Consumes and encodes this response.
    pub fn into_encoded(self) -> Result<Vec<u8>> {
        let parts = self.into_parts()?;
        let mut frame = Vec::with_capacity(parts.header.len() + parts.payload.len());
        frame.extend_from_slice(&parts.header);
        frame.extend_from_slice(&parts.payload);
        Ok(frame)
    }

    /// Decodes a response header when enough bytes are available.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<ResponseHeader>> {
        decode_response_header(prefix)
    }

    /// Reports the complete response frame length once the header is available.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::decode_header(prefix)?
            .map(ResponseHeader::frame_len)
            .transpose()
    }

    /// Decodes and validates one complete response frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let header = Self::decode_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + 2 * MIN_VARUINT_BYTES,
            actual: frame.len(),
        })?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(Self {
            status: header.status,
            request_id: header.request_id,
            payload: frame[header.encoded_len..].to_vec(),
        })
    }

    /// Decodes a response while retaining a conventional owned payload.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let header = OwnedResponseFrame::decode(frame)?;
        // Reuse the transport-owned frame allocation. The payload still has
        // to move past the status/length prefix, but no second payload Vec is
        // allocated on the client response hot path.
        frame = header.frame;
        frame.copy_within(header.header.encoded_len().., 0);
        frame.truncate(header.header.payload_len());
        Ok(Self {
            status: header.header.status(),
            request_id: header.header.request_id(),
            payload: frame,
        })
    }
}
