//! Format-neutral metadata for one generation accepted by storage.

use super::{GenerationLocation, LargeValueLocation};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CommittedGenerationState {
    pub(crate) sequence: u64,
    pub(crate) location: GenerationLocation,
    pub(crate) large_value_location: Option<LargeValueLocation>,
}
