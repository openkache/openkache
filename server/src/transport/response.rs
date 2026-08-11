//! Backend-independent response segment preparation.

use compio::buf::{BufResult, IoVectoredBuf};
use compio::io::{AsyncWrite, AsyncWriteExt};
use openkache_protocol::{ResponseParts, ResponseSegment};

const MAX_VECTORED_SEGMENTS: usize = 64;

struct ResponseWriteSegments {
    segments: smallvec::SmallVec<[ResponseSegment; 8]>,
    start: usize,
    end: usize,
}

impl ResponseWriteSegments {
    fn new(parts: ResponseParts) -> Self {
        let segments = parts.into_segments();
        let end = segments.len().min(MAX_VECTORED_SEGMENTS);
        Self {
            segments,
            start: 0,
            end,
        }
    }

    fn is_complete(&self) -> bool {
        self.start == self.segments.len()
    }

    fn advance(&mut self) {
        self.start = self.end;
        self.end = self
            .start
            .saturating_add(MAX_VECTORED_SEGMENTS)
            .min(self.segments.len());
    }
}

impl IoVectoredBuf for ResponseWriteSegments {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.segments[self.start..self.end]
            .iter()
            .map(ResponseSegment::as_slice)
    }
}

/// Writes every response segment through backend-safe bounded iovec windows.
///
/// The complete segment collection remains owned until all writes finish.
/// Large lists and maps therefore retain their payload allocations instead of
/// being copied into one coalesced buffer merely to respect an iovec limit.
pub(super) async fn write_response_segments(
    writer: &mut impl AsyncWrite,
    parts: ResponseParts,
) -> std::io::Result<()> {
    let mut response = ResponseWriteSegments::new(parts);
    while !response.is_complete() {
        let BufResult(result, returned) = writer.write_vectored_all(response).await;
        response = returned;
        result?;
        response.advance();
    }
    Ok(())
}
