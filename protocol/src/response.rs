//! Operation-neutral response framing and owned response buffers.
//!
//! Response status and payload framing are shared by clients and servers.
//! This module intentionally does not decode operation-specific field
//! semantics; callers can pass the borrowed payload to the generated layout
//! or compatibility adapter that owns those semantics.

use std::ops::Range;

use bytes::Bytes;
use smallvec::SmallVec;

use crate::{
    MIN_VARUINT_BYTES, ProtocolError, RESPONSE_FIXED_BYTES, Result, Status, decode_varuint,
    encode_varuint, validate_value_length,
};

/// An owned buffer with a logical byte range.
///
/// Keeping the range beside its allocation lets request and response paths
/// transfer payload ownership without shifting bytes to offset zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRange {
    buffer: Bytes,
    range: Range<usize>,
}

impl OwnedRange {
    /// Retains a validated logical range of an owned buffer.
    pub fn new(buffer: Vec<u8>, range: Range<usize>) -> Option<Self> {
        Self::from_bytes(Bytes::from(buffer), range)
    }

    /// Owns one complete buffer.
    pub fn whole(buffer: Vec<u8>) -> Self {
        let end = buffer.len();
        Self {
            buffer: Bytes::from(buffer),
            range: 0..end,
        }
    }

    /// Retains a logical range of a transport-neutral shared byte buffer.
    pub fn from_bytes(buffer: Bytes, range: Range<usize>) -> Option<Self> {
        (range.start <= range.end && range.end <= buffer.len())
            .then_some(Self { buffer, range })
    }

    /// Retains one complete transport-neutral shared byte buffer.
    pub fn whole_bytes(buffer: Bytes) -> Self {
        let end = buffer.len();
        Self {
            buffer,
            range: 0..end,
        }
    }

    /// Retains one complete static byte string without allocation.
    pub fn from_static(buffer: &'static [u8]) -> Self {
        Self::whole_bytes(Bytes::from_static(buffer))
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

    /// Narrows this owner to a range relative to its currently visible bytes.
    pub fn slice(self, range: Range<usize>) -> Option<Self> {
        if range.start > range.end || range.end > self.range.len() {
            return None;
        }
        let start = self.range.start.checked_add(range.start)?;
        let end = self.range.start.checked_add(range.end)?;
        Some(Self {
            buffer: self.buffer,
            range: start..end,
        })
    }

    /// Recovers a vector and logical range.
    ///
    /// Vector-backed values retain their original allocation and range.
    /// Erased shared buffers are materialized only at this explicit
    /// vector-only compatibility boundary.
    pub fn into_parts(self) -> (Vec<u8>, Range<usize>) {
        (self.buffer.into(), self.range)
    }

    /// Materializes only the visible bytes as a standalone vector.
    pub fn into_vec(self) -> Vec<u8> {
        let (mut buffer, range) = self.into_parts();
        if range.start == 0 && range.end == buffer.len() {
            return buffer;
        }
        buffer.copy_within(range.clone(), 0);
        buffer.truncate(range.len());
        buffer
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
    status: Status,
    encoded_len: usize,
    payload_len: usize,
}

impl ResponseHeader {
    /// Returns the decoded status.
    pub const fn status(self) -> Status {
        self.status
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
    let Some((payload_len, encoded_len)) = decode_varuint(
        prefix.get(RESPONSE_FIXED_BYTES..).unwrap_or_default(),
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
        encoded_len: RESPONSE_FIXED_BYTES + encoded_len,
        payload_len,
    }))
}

/// A complete response viewed as an opaque status and payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFrame<'a> {
    status: Status,
    frame: &'a [u8],
    payload_offset: usize,
}

impl<'a> ResponseFrame<'a> {
    /// Decodes one complete response without interpreting its payload.
    pub fn decode(frame: &'a [u8]) -> Result<Self> {
        let header = decode_response_header(frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
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
            frame,
            payload_offset: header.encoded_len,
        })
    }

    /// Returns the response status.
    pub const fn status(self) -> Status {
        self.status
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
/// over the status/length prefix. Use this type when a generated field view
/// can retain the received frame for its lifetime.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedResponseFrame {
    header: ResponseHeader,
    frame: Vec<u8>,
}

impl OwnedResponseFrame {
    /// Decodes one complete response while retaining its original allocation.
    pub fn decode(frame: Vec<u8>) -> Result<Self> {
        let header = Response::decode_header(&frame)?.ok_or(ProtocolError::FrameTooShort {
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
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

/// One composite value that retains its owned child payloads.
///
/// The structural encoding is preserved until the transport asks for wire
/// segments. Generated response plans can therefore validate every nested
/// child without coalescing the complete value.
#[derive(Debug, Eq, PartialEq)]
pub struct SegmentedValue {
    encoded_len: usize,
    encoding: SegmentedEncoding,
}

/// One contiguous or already-segmented child of a composite codec value.
#[derive(Debug, Eq, PartialEq)]
pub enum SegmentedPayload {
    /// A single owned payload range.
    Contiguous(OwnedRange),
    /// A nested list, map, union, or future composite value.
    Nested(Box<SegmentedValue>),
}

/// Canonical structural encoding retained by a segmented value.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SegmentedEncoding {
    List(SmallVec<[SegmentedPayload; 8]>),
    Map(SmallVec<[(SegmentedPayload, SegmentedPayload); 4]>),
    Union {
        tag: u8,
        payload: SegmentedPayload,
    },
}

impl SegmentedPayload {
    /// Returns the exact encoded child length.
    pub fn len(&self) -> usize {
        match self {
            Self::Contiguous(value) => value.len(),
            Self::Nested(value) => value.len(),
        }
    }

    /// Returns whether the encoded child is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn append_to(self, segments: &mut SmallVec<[ResponseSegment; 8]>) {
        match self {
            Self::Contiguous(value) => segments.push(ResponseSegment::Payload(value)),
            Self::Nested(value) => value.append_segments(segments),
        }
    }
}

impl From<OwnedRange> for SegmentedPayload {
    fn from(value: OwnedRange) -> Self {
        Self::Contiguous(value)
    }
}

impl From<Vec<u8>> for SegmentedPayload {
    fn from(value: Vec<u8>) -> Self {
        Self::Contiguous(OwnedRange::whole(value))
    }
}

impl From<SegmentedValue> for SegmentedPayload {
    fn from(value: SegmentedValue) -> Self {
        Self::Nested(Box::new(value))
    }
}

impl SegmentedValue {
    pub(crate) fn new(encoded_len: usize, encoding: SegmentedEncoding) -> Result<Self> {
        validate_value_length(encoded_len)?;
        Ok(Self {
            encoded_len,
            encoding,
        })
    }

    /// Returns the generic codec that validated and framed this value.
    pub const fn codec(&self) -> crate::codec::CodecKind {
        match &self.encoding {
            SegmentedEncoding::List(_) => crate::codec::CodecKind::List,
            SegmentedEncoding::Map(_) => crate::codec::CodecKind::Map,
            SegmentedEncoding::Union { .. } => crate::codec::CodecKind::Union,
        }
    }

    pub(crate) const fn encoding(&self) -> &SegmentedEncoding {
        &self.encoding
    }

    /// Returns the exact encoded byte count without coalescing the segments.
    pub const fn len(&self) -> usize {
        self.encoded_len
    }

    /// Returns whether the encoded value is empty.
    pub const fn is_empty(&self) -> bool {
        self.encoded_len == 0
    }

    /// Moves the ordered wire segments into the outer response frame.
    pub fn into_segments(self) -> SmallVec<[ResponseSegment; 8]> {
        let mut segments = SmallVec::new();
        self.append_segments(&mut segments);
        segments
    }

    /// Appends this value directly to an existing response write plan.
    ///
    /// This avoids an intermediate segment collection when a generated field
    /// is already being framed with sibling fields.
    pub fn append_segments(self, segments: &mut SmallVec<[ResponseSegment; 8]>) {
        let encoded_len = self.encoded_len;
        let first_segment = segments.len();
        match self.encoding {
            SegmentedEncoding::List(values) => {
                let (count, count_len) = encode_varuint(values.len() as u64);
                segments.push(ResponseSegment::inline(&count[..count_len]));
                for value in values {
                    append_segmented_payload(segments, value);
                }
            }
            SegmentedEncoding::Map(entries) => {
                let (count, count_len) = encode_varuint(entries.len() as u64);
                segments.push(ResponseSegment::inline(&count[..count_len]));
                for (key, value) in entries {
                    append_segmented_payload(segments, key);
                    append_segmented_payload(segments, value);
                }
            }
            SegmentedEncoding::Union { tag, payload } => {
                let length = u32::try_from(payload.len())
                    .expect("validated segmented union payload length fits u32");
                let mut prefix = SmallVec::<[u8; 32]>::new();
                prefix.push(tag);
                prefix.extend_from_slice(&length.to_be_bytes());
                segments.push(ResponseSegment::Inline(prefix));
                payload.append_to(segments);
            }
        }
        debug_assert_eq!(
            segments[first_segment..]
                .iter()
                .map(ResponseSegment::len)
                .sum::<usize>(),
            encoded_len
        );
    }
}

fn append_segmented_payload(
    segments: &mut SmallVec<[ResponseSegment; 8]>,
    payload: SegmentedPayload,
) {
    let length =
        u32::try_from(payload.len()).expect("validated segmented child payload length fits u32");
    segments.push(ResponseSegment::inline(&length.to_be_bytes()));
    payload.append_to(segments);
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
    pub fn segmented<I, T>(status: Status, segments: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<ResponseSegment>,
    {
        let segments: SmallVec<[ResponseSegment; 8]> =
            segments.into_iter().map(Into::into).collect();
        Self::from_segments(status, segments)
    }

    /// Encodes a response header over an already-owned segment collection.
    ///
    /// Callers that build generic framing segments incrementally can transfer
    /// that collection without allocating and moving it into another buffer.
    pub fn from_segments(
        status: Status,
        segments: SmallVec<[ResponseSegment; 8]>,
    ) -> Result<Self> {
        let payload_len = segments.iter().try_fold(0usize, |total, segment| {
            total
                .checked_add(segment.len())
                .ok_or(ProtocolError::FrameLengthOverflow)
        })?;
        validate_value_length(payload_len)?;
        let (length, length_bytes) = encode_varuint(payload_len as u64);
        let mut header = SmallVec::new();
        header.push(status as u8);
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
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
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
        let mut segments = self.segments;
        segments.reserve(1 + usize::from(!self.payload.is_empty()));
        segments.insert(0, ResponseSegment::Inline(self.header));
        if !self.payload.is_empty() {
            segments.insert(1, ResponseSegment::Payload(self.payload.into()));
        }
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
        if decoded.encoded_len() != self.header.len()
            || decoded.payload_len() != actual_payload_len
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
            payload,
        })
    }
}

impl Response {
    /// Creates a response after checking the wire payload ceiling.
    pub fn new(status: Status, payload: Vec<u8>) -> Result<Self> {
        validate_value_length(payload.len())?;
        Ok(Self { status, payload })
    }

    /// Encodes this response into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_value_length(self.payload.len())?;
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let mut frame =
            Vec::with_capacity(RESPONSE_FIXED_BYTES + length_bytes + self.payload.len());
        frame.push(self.status as u8);
        frame.extend_from_slice(&length[..length_bytes]);
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }

    /// Consumes this response without copying its payload.
    pub fn into_parts(self) -> Result<ResponseParts> {
        validate_value_length(self.payload.len())?;
        let (length, length_bytes) = encode_varuint(self.payload.len() as u64);
        let mut header = SmallVec::new();
        header.push(self.status as u8);
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
            expected: RESPONSE_FIXED_BYTES + MIN_VARUINT_BYTES,
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
            payload: frame,
        })
    }
}
