//! Compatibility-aware request header accessors.
//!
//! [`RequestFrame`](super::RequestFrame) exposes only operation-neutral frame
//! metadata. This richer header is retained for the historical public
//! [`super::Request`] facade, whose accessors describe namespace/item/SET
//! projections.

use super::{ProtocolError, Result};
use openkache_protocol::Opcode;

/// Metadata decoded only by the compatibility adapter.
///
/// Generic framing never constructs or inspects this representation. Keeping
/// it beside the typed request header prevents compatibility namespace/item
/// vocabulary from leaking into the operation-neutral frame parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompatibilityHeaderMetadata {
    namespace_id: Option<u64>,
    item_id_count: usize,
    item_id_len: usize,
    has_ttl: bool,
}

/// A validated request header for the historical request facade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    opcode: Opcode,
    encoded_len: usize,
    value_len: usize,
    compatibility: Option<CompatibilityHeaderMetadata>,
}

impl RequestHeader {
    pub(super) const fn generic(opcode: Opcode, encoded_len: usize, value_len: usize) -> Self {
        Self {
            opcode,
            encoded_len,
            value_len,
            compatibility: None,
        }
    }

    pub(super) const fn compatibility(
        opcode: Opcode,
        encoded_len: usize,
        value_len: usize,
        namespace_id: Option<u64>,
        item_id_count: usize,
        item_id_len: usize,
        has_ttl: bool,
    ) -> Self {
        Self {
            opcode,
            encoded_len,
            value_len,
            compatibility: Some(CompatibilityHeaderMetadata {
                namespace_id,
                item_id_count,
                item_id_len,
                has_ttl,
            }),
        }
    }

    /// Returns the decoded operation.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of encoded bytes before a SET value.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the total opaque item-ID bytes carried by this request.
    pub const fn item_id_len(self) -> usize {
        match self.compatibility {
            Some(metadata) => metadata.item_id_len,
            None => 0,
        }
    }

    /// Returns the number of item IDs carried by this request.
    pub const fn item_id_count(self) -> usize {
        match self.compatibility {
            Some(metadata) => metadata.item_id_count,
            None => 0,
        }
    }

    /// Returns the opaque SET or application-value length, or zero for other operations.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns the namespace ID carried by this request, when applicable.
    pub const fn namespace_id(self) -> Option<u64> {
        match self.compatibility {
            Some(metadata) => metadata.namespace_id,
            None => None,
        }
    }

    /// Returns whether a TTL varuint follows the SET item ID.
    pub const fn has_ttl(self) -> bool {
        match self.compatibility {
            Some(metadata) => metadata.has_ttl,
            None => false,
        }
    }

    /// Reports the complete frame length once all metadata is available.
    pub fn frame_len(self, _prefix: &[u8]) -> Result<Option<usize>> {
        self.encoded_len
            .checked_add(self.value_len)
            .map(Some)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}
