//! Operation-neutral ownership for ordered wire byte segments.

use std::ops::Range;

use smallvec::{Array, SmallVec};

use crate::{ProtocolError, Result};

// Keep the common framing/payload pair inline without making every frame
// carry a large array of segment owners. Longer plans spill metadata only;
// their payload allocations remain unchanged.
pub(crate) const INLINE_SEGMENTS: usize = 2;

/// Owned wire frame using the common two-segment inline storage.
pub type OwnedFrame = SegmentFrame<[WireSegment; INLINE_SEGMENTS]>;

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

/// An owner for one stable byte sequence.
///
/// Implementations may retain aligned buffers, pooled read leases, shared
/// segments, or another byte owner without exposing that implementation to the
/// consumer. The returned slice must keep the same length while the owner is
/// held by [`StableBytes`].
pub trait StableByteOwner: Send + Sync + 'static {
    /// Returns the complete visible bytes owned by this value.
    fn as_bytes(&self) -> &[u8];
}

/// Backward-compatible name for owners passed to [`WireSegment::external`].
pub use StableByteOwner as WireByteOwner;

/// Type-erased ownership for one stable byte sequence.
///
/// This operation-neutral container can cross application, storage, and
/// transport boundaries without copying its payload or repeating type erasure.
pub struct StableBytes {
    owner: Box<dyn StableByteOwner>,
    len: usize,
}

impl StableBytes {
    /// Erases one byte owner's concrete type.
    pub fn new(owner: impl StableByteOwner) -> Self {
        let len = owner.as_bytes().len();
        Self {
            owner: Box::new(owner),
            len,
        }
    }

    /// Returns the stable visible bytes.
    pub fn as_slice(&self) -> &[u8] {
        let bytes = self.owner.as_bytes();
        assert_eq!(
            bytes.len(),
            self.len,
            "stable byte owner changed its visible length"
        );
        bytes
    }
}

impl std::fmt::Debug for StableBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("StableBytes")
            .field(&self.as_slice())
            .finish()
    }
}

impl PartialEq for StableBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for StableBytes {}

/// Type-erased ownership for one stable external byte sequence.
///
/// Construct this through [`WireSegment::external`].
pub struct ExternalWireBytes(StableBytes);

impl ExternalWireBytes {
    fn new(owner: impl StableByteOwner) -> Self {
        Self(StableBytes::new(owner))
    }

    fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl std::fmt::Debug for ExternalWireBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("External")
            .field(&self.as_slice())
            .finish()
    }
}

impl PartialEq for ExternalWireBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for ExternalWireBytes {}

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
    /// Bytes retaining an operation-neutral external owner.
    External(ExternalWireBytes),
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

    /// Retains an external byte owner without copying its payload.
    pub fn external(owner: impl StableByteOwner) -> Self {
        Self::External(ExternalWireBytes::new(owner))
    }

    /// Returns the visible wire bytes.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_slice(),
            Self::External(bytes) => bytes.as_slice(),
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

impl From<StableBytes> for WireSegment {
    fn from(value: StableBytes) -> Self {
        Self::External(ExternalWireBytes(value))
    }
}

impl From<Vec<u8>> for WireSegment {
    fn from(value: Vec<u8>) -> Self {
        Self::owned(value)
    }
}

/// Ordered owned wire segments with a checked cached byte length.
#[derive(Debug, Eq, PartialEq)]
pub struct SegmentFrame<A>
where
    A: Array<Item = WireSegment>,
{
    segments: SmallVec<A>,
    encoded_len: usize,
}

impl<A> SegmentFrame<A>
where
    A: Array<Item = WireSegment>,
{
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
        let segments: SmallVec<A> = segments.into_iter().map(Into::into).collect();
        Self::from_segments(segments)
    }

    pub(crate) fn from_segments(segments: SmallVec<A>) -> Result<Self> {
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
    pub fn into_segments(self) -> SmallVec<A> {
        self.segments
    }
}

/// Compatibility name for response APIs while ownership becomes shared.
pub use WireSegment as ResponseSegment;
