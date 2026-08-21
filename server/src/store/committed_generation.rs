//! Format-neutral metadata for one generation accepted by storage.

use super::{GenerationLocation, LargeValueLocation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationIntegrity {
    pub(crate) segment_checksum: u32,
    pub(crate) blob_checksum: u32,
    pub(crate) large_value_checksum: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CommittedGenerationState {
    pub(crate) sequence: u64,
    pub(crate) location: GenerationLocation,
    pub(crate) large_value_location: Option<LargeValueLocation>,
}
