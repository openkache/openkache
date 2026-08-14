//! Operation-neutral ownership for ordered wire byte segments.

use std::ops::Range;

use smallvec::SmallVec;

use crate::{ProtocolError, Result};

// Keep the common framing/payload pair inline without making every frame
// carry a large array of segment owners. Longer plans spill metadata only;
// their payload allocations remain unchanged.
const INLINE_SEGMENTS: usize = 2;

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

/// One owned segment in an operation-neutral wire plan.
///
/// Small framing prefixes stay inline while application and storage bytes
/// retain their original allocation and logical range.
#[derive(Debug, Eq, PartialEq)]
pub enum WireSegment {
    /// Framing bytes stored inline without a heap allocation.
    Inline(SmallVec<[u8; 32]>),
    /// Application, storage, or framing bytes retaining their allocation.
    Owned(OwnedRange),
}

impl WireSegment {
    /// Copies bounded framing bytes into inline storage.
    pub fn inline(bytes: &[u8]) -> Self {
        Self::Inline(SmallVec::from_slice(bytes))
    }

    /// Retains owned bytes and their logical range without copying.
    pub fn owned(bytes: impl Into<OwnedRange>) -> Self {
        Self::Owned(bytes.into())
    }

    /// Returns the visible wire bytes.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_slice(),
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
}

impl From<OwnedRange> for WireSegment {
    fn from(value: OwnedRange) -> Self {
        Self::owned(value)
    }
}

impl From<Vec<u8>> for WireSegment {
    fn from(value: Vec<u8>) -> Self {
        Self::owned(value)
    }
}

/// Ordered owned wire segments with a checked cached byte length.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnedFrame {
    segments: SmallVec<[WireSegment; INLINE_SEGMENTS]>,
    encoded_len: usize,
}

impl OwnedFrame {
    /// Builds one frame without coalescing independently owned segments.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::FrameLengthOverflow`] when the combined
    /// segment length cannot be represented by `usize`.
    pub fn new<I, T>(segments: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<WireSegment>,
    {
        let segments: SmallVec<[WireSegment; INLINE_SEGMENTS]> =
            segments.into_iter().map(Into::into).collect();
        let encoded_len = segments.iter().try_fold(0usize, |total, segment| {
            total
                .checked_add(segment.len())
                .ok_or(ProtocolError::FrameLengthOverflow)
        })?;
        Ok(Self {
            segments,
            encoded_len,
        })
    }

    /// Returns the ordered segments without exposing their storage.
    pub fn segments(&self) -> &[WireSegment] {
        &self.segments
    }

    /// Returns the checked complete frame length.
    pub const fn len(&self) -> usize {
        self.encoded_len
    }

    /// Returns whether the frame contains no visible bytes.
    pub const fn is_empty(&self) -> bool {
        self.encoded_len == 0
    }

    /// Recovers the ordered segment owners without copying payload bytes.
    pub fn into_segments(self) -> SmallVec<[WireSegment; INLINE_SEGMENTS]> {
        self.segments
    }
}

/// Compatibility name for response APIs while ownership becomes shared.
pub use WireSegment as ResponseSegment;
