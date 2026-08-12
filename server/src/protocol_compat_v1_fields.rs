//! Field lookup helpers for the generated compact-v1 request plan.
//!
//! These helpers expose only generated indexes and borrowed ranges.  They do
//! not know which API owns a role; semantic interpretation remains in the
//! compatibility facade.

use openkache_protocol::{Opcode, OwnedRange};

use super::super::{ProtocolError, Result};
use super::contract;

pub(crate) fn field_count(opcode: Opcode, role: &str) -> usize {
    contract::spec(opcode)
        .request
        .fields
        .iter()
        .filter(|field| field.role == role)
        .count()
}

fn field_index(opcode: Opcode, role: &str, occurrence: usize) -> Option<usize> {
    contract::spec(opcode)
        .request
        .fields
        .iter()
        .filter(|field| field.role == role)
        .nth(occurrence)
        .map(|field| field.index)
}

pub(crate) fn field_bytes<'a>(
    opcode: Opcode,
    fields: &'a [Option<OwnedRange>],
    role: &str,
    occurrence: usize,
) -> Option<&'a [u8]> {
    field_index(opcode, role, occurrence)
        .and_then(|index| fields.get(index))
        .and_then(Option::as_ref)
        .map(OwnedRange::as_slice)
}

pub(crate) fn field_values<'a>(
    opcode: Opcode,
    fields: &'a [Option<OwnedRange>],
    role: &str,
) -> Vec<&'a [u8]> {
    (0..field_count(opcode, role))
        .filter_map(|occurrence| field_bytes(opcode, fields, role, occurrence))
        .collect()
}

pub(crate) fn field_u64(
    opcode: Opcode,
    fields: &[Option<OwnedRange>],
    role: &str,
    occurrence: usize,
) -> Result<Option<u64>> {
    let Some(value) = field_bytes(opcode, fields, role, occurrence) else {
        return Ok(None);
    };
    let bytes: [u8; 8] = value.try_into().map_err(|_| {
        ProtocolError::InvalidFieldSequence("compact integer field has the wrong width")
    })?;
    Ok(Some(u64::from_be_bytes(bytes)))
}
