//! Operation-neutral response framing and owned response buffers.
//!
//! Response-code and payload framing are shared by clients and servers.
//! This module intentionally does not decode operation-specific field
//! semantics; callers pass the borrowed payload to the API codec that owns
//! those semantics.

use std::ops::Range;

use smallvec::SmallVec;

use crate::{
    MIN_VARUINT_BYTES, ProtocolError, RESPONSE_CODE_BYTES, Result, decode_varuint, encode_varuint,
    validate_payload_length,
};

/// An owned buffer with a logical byte range.
///
/// Keeping the range beside its allocation lets request and response paths
/// transfer payload ownership without shifting bytes to offset zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRange {
    buffer: Vec<u8>,
    range: Range<usize>,
}

impl OwnedRange {
    /// Retains a validated logical range of an owned buffer.
    pub fn new(buffer: Vec<u8>, range: Range<usize>) -> Option<Self> {
        (range.start <= range.end && range.end <= buffer.len()).then_some(Self { buffer, range })
    }

    /// Owns one complete buffer.
    pub fn whole(buffer: Vec<u8>) -> Self {
        let end = buffer.len();
        Self {
            buffer,
            range: 0..end,
        }
    }

    /// Returns the visible bytes.
    pub fn as_slice(&self) -> &[u8] {
        self.buffer
            .get(self.range.clone())
            .expect("owned byte range remains within its buffer")
    }

    /// Returns the visible byte count.
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Returns whether the visible range is empty.
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    /// Recovers the buffer and logical range without copying.
    pub fn into_parts(self) -> (Vec<u8>, Range<usize>) {
        (self.buffer, self.range)
    }
}

impl From<Vec<u8>> for OwnedRange {
    fn from(buffer: Vec<u8>) -> Self {
        Self::whole(buffer)
    }
}

impl AsRef<[u8]> for OwnedRange {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Metadata required to delimit one response with an opaque payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    code: u8,
    encoded_len: usize,
    payload_len: usize,
}

impl ResponseHeader {
    /// Returns the decoded response code.
    pub const fn code(self) -> u8 {
        self.code
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
    if RESPONSE_CODE_BYTES != 1 {
        return Err(ProtocolError::InvalidFrameLayout(
            "opaque response codes wider than one byte are not supported by this v1 parser",
        ));
    }
    let Some(&code) = prefix.first() else {
        return Ok(None);
    };
    let Some((payload_len, encoded_len)) = decode_varuint(
        prefix.get(RESPONSE_CODE_BYTES..).unwrap_or_default(),
        "response payload length",
    )?
    else {
        return Ok(None);
    };
    let payload_len =
        usize::try_from(payload_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    validate_payload_length(payload_len)?;
    Ok(Some(ResponseHeader {
        code,
        encoded_len: RESPONSE_CODE_BYTES + encoded_len,
        payload_len,
    }))
}

/// A complete response viewed as an opaque code and payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFrame<'a> {
    code: u8,
    frame: &'a [u8],
    payload_offset: usize,
}

impl<'a> ResponseFrame<'a> {
    /// Decodes one complete response without interpreting its payload.
    pub fn decode(frame: &'a [u8]) -> Result<Self> {
        let header = decode_response_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_CODE_BYTES + MIN_VARUINT_BYTES,
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
            code: header.code,
            frame,
            payload_offset: header.encoded_len,
        })
    }

    /// Returns the opaque response code.
    pub const fn code(self) -> u8 {
        self.code
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
/// over the code/length prefix. Use this type when an API field view can
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
            expected: RESPONSE_CODE_BYTES + MIN_VARUINT_BYTES,
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

    /// Returns the decoded response code.
    pub const fn code(&self) -> u8 {
        self.header.code()
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
    pub code: u8,
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

/// One owned response-body segment.
///
/// Small framing prefixes stay inline while application/storage payloads keep
/// their original allocation and logical range.
#[derive(Debug, Eq, PartialEq)]
pub enum ResponseSegment {
    /// Framing bytes stored inline without a heap allocation.
    Inline(SmallVec<[u8; 32]>),
    /// An ownership-preserving application or storage payload.
    Payload(OwnedRange),
}

impl ResponseSegment {
    /// Returns the visible wire bytes.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline(bytes) => bytes,
            Self::Payload(bytes) => bytes.as_slice(),
        }
    }

    /// Returns the visible byte count.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Returns whether the segment is empty.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Copies small framing bytes into inline storage.
    pub fn inline(bytes: &[u8]) -> Self {
        Self::Inline(SmallVec::from_slice(bytes))
    }
}

impl From<OwnedRange> for ResponseSegment {
    fn from(value: OwnedRange) -> Self {
        Self::Payload(value)
    }
}

impl ResponseParts {
    /// Encodes a response header over ownership-preserving body segments.
    pub fn segmented<I, T>(code: u8, segments: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<ResponseSegment>,
    {
        let segments: SmallVec<[ResponseSegment; 8]> =
            segments.into_iter().map(Into::into).collect();
        let payload_len = segments.iter().try_fold(0usize, |total, segment| {
            total
                .checked_add(segment.len())
                .ok_or(ProtocolError::FrameLengthOverflow)
        })?;
        validate_payload_length(payload_len)?;
        let length = u64::try_from(payload_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let (length, length_bytes) = encode_varuint(length);
        let mut header = SmallVec::new();
        header.push(code);
        header.extend_from_slice(&length[..length_bytes]);
        Ok(Self {
            header,
            payload: Vec::new(),
            segments,
        })
    }

    /// Validates independently received header and payload buffers.
    ///
    /// The payload allocation is retained exactly as supplied. This is the
    /// preferred receive-side constructor for transports that read the small
    /// response header and the body separately.
    pub fn decode(header: Vec<u8>, payload: Vec<u8>) -> Result<Self> {
        let decoded = Response::decode_header(&header)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_CODE_BYTES + MIN_VARUINT_BYTES,
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
            header: header.into_iter().collect(),
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
        let segment_count = self
            .segments
            .len()
            .saturating_add(1)
            .saturating_add(usize::from(!self.payload.is_empty()));
        let mut segments = SmallVec::with_capacity(segment_count);
        segments.push(ResponseSegment::Inline(self.header));
        if !self.payload.is_empty() {
            segments.push(ResponseSegment::Payload(self.payload.into()));
        }
        segments.extend(self.segments);
        segments
    }

    /// Decodes the response code and moves the already-owned payload without copying.
    pub fn into_response(self) -> Result<Response> {
        let decoded =
            Response::decode_header(&self.header)?.ok_or(ProtocolError::FrameTooShort {
                expected: RESPONSE_CODE_BYTES + MIN_VARUINT_BYTES,
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
            code: decoded.code(),
            payload,
        })
    }
}

impl Response {
    /// Creates a response after checking the wire payload ceiling.
    pub fn new(code: u8, payload: Vec<u8>) -> Result<Self> {
        validate_payload_length(payload.len())?;
        Ok(Self { code, payload })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_payload_length(self.payload.len())?;
        let length =
            u64::try_from(self.payload.len()).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let (length, length_bytes) = encode_varuint(length);
        let mut frame = Vec::with_capacity(RESPONSE_CODE_BYTES + length_bytes + self.payload.len());
        frame.push(self.code);
        frame.extend_from_slice(&length[..length_bytes]);
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Consumes this response without copying its payload.
    pub fn into_parts(self) -> Result<ResponseParts> {
        validate_payload_length(self.payload.len())?;
        let length =
            u64::try_from(self.payload.len()).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        let (length, length_bytes) = encode_varuint(length);
        let mut header = SmallVec::new();
        header.push(self.code);
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
            expected: RESPONSE_CODE_BYTES + MIN_VARUINT_BYTES,
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
            code: header.code,
            payload: frame[header.encoded_len..].to_vec(),
        })
    }

    /// Decodes a response while retaining a conventional owned payload.
    pub fn decode_owned(mut frame: Vec<u8>) -> Result<Self> {
        let header = OwnedResponseFrame::decode(frame)?;
        // Reuse the transport-owned frame allocation. The payload still has
        // to move past the code/length prefix, but no second payload Vec is
        // allocated on the client response hot path.
        frame = header.frame;
        frame.copy_within(header.header.encoded_len().., 0);
        frame.truncate(header.header.payload_len());
        Ok(Self {
            code: header.header.code(),
            payload: frame,
        })
    }
}
