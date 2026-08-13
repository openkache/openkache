//! Primitive wire values and framing shared by clients and servers.
//!
//! Request and response bodies remain outside this crate. API modules choose
//! their own wire layout and use these operation-neutral framing and value
//! primitives to implement it.

include!(concat!(env!("OUT_DIR"), "/wire_values.rs"));

/// Operation-neutral scalar and container value codecs.
pub mod codec;
/// Generic field-layout helpers shared by API-owned codecs.
pub mod layout;
/// Configurable fixed-width optional-value codec.
pub mod optional_values;
/// Operation-neutral request frame delimiting.
pub mod request;
/// Operation-neutral response framing and owned response buffers.
pub mod response;

pub use layout::{
    DenseFields, LayoutValue, encode_dense_fields, encode_field_sequence_segments,
    encode_optional_value_segments,
};
pub use optional_values::{
    OptionalValueCodec, OptionalValues, OptionalValuesEncoder, decode_optional_values,
    encode_optional_values, optional_values_encoded_len, optional_values_encoded_len_from_lengths,
    optional_values_max_encoded_len,
};
pub use request::{
    OpaqueRequestFrame, RequestFrameHeader, RequestFrameLayout, RequestFrameStep,
    decode_request_frame_header,
};
pub use response::{
    OwnedRange, OwnedResponseFrame, Response, ResponseFrame, ResponseHeader, ResponseParts,
    ResponseSegment,
};

/// Encodes an ordered field sequence.
///
/// The payload starts with a compact presence mask, followed by canonical
/// `vu128` lengths and bytes for every present field before the final present
/// field. The final present field consumes the remaining bytes. The caller
/// supplies field order, cardinality, requiredness, and codecs; this primitive
/// only carries bounded opaque field bytes.
pub fn encode_field_sequence(values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
    let mask_bytes = values.len().saturating_add(7) / 8;
    if mask_bytes > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: mask_bytes,
            maximum: MAX_PAYLOAD_BYTES,
        });
    }
    let capacity = field_sequence_encoded_len(values)?;
    let mut payload = Vec::with_capacity(capacity);
    append_field_sequence(values, &mut payload)?;
    debug_assert_eq!(payload.len(), capacity);
    Ok(payload)
}

/// Appends one ordered field sequence to an existing output buffer.
///
/// Keeping the append operation private prevents callers from accidentally
/// treating a nested sequence as a standalone frame while allowing the common
/// encoder and grouped-field encoder to reuse one exact-size output buffer.
fn append_field_sequence(values: &[Option<&[u8]>], payload: &mut Vec<u8>) -> Result<()> {
    let mask_bytes = values.len().saturating_add(7) / 8;
    let mask_start = payload.len();
    payload.resize(
        mask_start
            .checked_add(mask_bytes)
            .ok_or(ProtocolError::FrameLengthOverflow)?,
        0,
    );
    let last_present = values.iter().rposition(Option::is_some);
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        validate_payload_length(value.len())?;
        payload[mask_start + index / 8] |= 1 << (index % 8);
        let (encoded, encoded_len) = encode_varuint(
            u64::try_from(value.len()).map_err(|_| ProtocolError::FrameLengthOverflow)?,
        );
        if Some(index) != last_present {
            payload.extend_from_slice(&encoded[..encoded_len]);
        }
        payload.extend_from_slice(value);
    }
    validate_payload_length(payload.len())?;
    Ok(())
}

/// Returns the encoded length of a generic presence-mask field sequence
/// without allocating the payload.
///
/// The returned value is the exact body size for the supplied fields. Every
/// present field before the final present field pays a canonical `vu128`
/// length prefix; the final present field consumes the remainder. It is
/// deliberately independent of operation names and semantic roles so API
/// modules can use it as a shared size-cost primitive.
pub fn field_sequence_encoded_len(values: &[Option<&[u8]>]) -> Result<usize> {
    field_sequence_encoded_len_iter(values.iter().map(|value| value.map(<[u8]>::len)))
}

/// Computes a field-sequence payload size from lengths alone.
///
/// This is the allocation-free cost primitive used by API size planners.
/// `None` is a missing field; `Some(0)` is a present-empty field.
pub fn field_sequence_encoded_len_from_lengths(lengths: &[Option<usize>]) -> Result<usize> {
    field_sequence_encoded_len_iter(lengths.iter().copied())
}

fn field_sequence_encoded_len_iter(lengths: impl Iterator<Item = Option<usize>>) -> Result<usize> {
    let mut field_count = 0usize;
    let mut encoded_len = 0usize;
    let mut pending_length = None;
    for length in lengths {
        field_count = field_count
            .checked_add(1)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let Some(length) = length else {
            continue;
        };
        validate_payload_length(length)?;
        if let Some(previous_length) = pending_length.replace(length) {
            encoded_len = encoded_len
                .checked_add(
                    encode_varuint(
                        u64::try_from(previous_length)
                            .map_err(|_| ProtocolError::FrameLengthOverflow)?,
                    )
                    .1,
                )
                .and_then(|total| total.checked_add(previous_length))
                .ok_or(ProtocolError::FrameLengthOverflow)?;
        }
    }
    if let Some(last_length) = pending_length {
        encoded_len = encoded_len
            .checked_add(last_length)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    let mask_bytes = field_count.saturating_add(7) / 8;
    if mask_bytes > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: mask_bytes,
            maximum: MAX_PAYLOAD_BYTES,
        });
    }
    let total = mask_bytes
        .checked_add(encoded_len)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    validate_payload_length(total)?;
    Ok(total)
}

/// A zero-copy view over an ordered field sequence.
///
/// The cursor validates the presence mask and every present length/entry
/// boundary once, then returns borrowed field slices without allocating one
/// buffer per field. Missing fields have a cleared mask bit; present-empty
/// fields have a canonical zero length and return `Some(&[])`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldSequence<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
    field_count: usize,
}

impl<'a, 'b> FieldSequence<'a, 'b> {
    fn validate_entries(
        payload: &[u8],
        field_count: usize,
        required: Option<&[bool]>,
        mut offsets: Option<&mut [(usize, usize)]>,
    ) -> Result<()> {
        if let Some(required) = required {
            if required.len() != field_count {
                return Err(ProtocolError::InvalidFieldSequence(
                    "requiredness does not match the modeled field count",
                ));
            }
        }
        if offsets
            .as_ref()
            .is_some_and(|offsets| offsets.len() < field_count)
        {
            return Err(ProtocolError::InvalidFieldSequence(
                "field offset storage is smaller than the modeled field count",
            ));
        }
        let mask_bytes = field_count.saturating_add(7) / 8;
        if payload.len() < mask_bytes {
            return Err(ProtocolError::InvalidFieldSequence(
                "field sequence is missing its presence mask",
            ));
        }
        validate_payload_length(payload.len())?;
        if mask_bytes > 0 && field_count % 8 != 0 {
            let unused = payload[mask_bytes - 1] & !((1 << (field_count % 8)) - 1);
            if unused != 0 {
                return Err(ProtocolError::InvalidFieldSequence(
                    "field sequence presence mask has unused bits set",
                ));
            }
        }
        let last_present = (0..field_count)
            .rev()
            .find(|&index| payload[index / 8] & (1 << (index % 8)) != 0);
        let mut cursor = mask_bytes;
        for index in 0..field_count {
            let present = payload[index / 8] & (1 << (index % 8)) != 0;
            if !present {
                if required.is_some_and(|required| required[index]) {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "required field is missing from the field sequence",
                    ));
                }
                if let Some(offsets) = offsets.as_deref_mut() {
                    offsets[index] = (usize::MAX, usize::MAX);
                }
                continue;
            }
            if Some(index) == last_present {
                let end = payload.len();
                let length = end
                    .checked_sub(cursor)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                validate_payload_length(length)?;
                if let Some(offsets) = offsets.as_deref_mut() {
                    offsets[index] = (cursor, end);
                }
                cursor = end;
                continue;
            }
            let Some((length, encoded_len)) = decode_varuint(
                payload
                    .get(cursor..)
                    .ok_or(ProtocolError::InvalidFieldSequence(
                        "field sequence entry is truncated",
                    ))?,
                "field sequence length",
            )?
            else {
                return Err(ProtocolError::InvalidFieldSequence(
                    "field sequence entry is missing its length",
                ));
            };
            let length = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            cursor = cursor
                .checked_add(encoded_len)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            validate_payload_length(length)?;
            let end = cursor
                .checked_add(length)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if payload.get(cursor..end).is_none() {
                return Err(ProtocolError::InvalidFieldSequence(
                    "field sequence entry is truncated",
                ));
            }
            if let Some(offsets) = offsets.as_deref_mut() {
                offsets[index] = (cursor, end);
            }
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "field sequence contains trailing bytes",
            ));
        }
        Ok(())
    }

    /// Validates an ordered field sequence without allocating offset storage.
    ///
    /// This is the shape-only counterpart to [`Self::decode`]. It is useful at
    /// request-construction and frame-validation boundaries that only need to
    /// reject malformed bytes; a behavior that needs field access can decode
    /// the same payload once into its own bounded offset storage.
    pub fn validate(payload: &[u8], field_count: usize) -> Result<()> {
        Self::validate_entries(payload, field_count, None, None)
    }

    /// Validates an ordered field sequence and caller-supplied requiredness
    /// without allocating offset storage.
    pub fn validate_with_required(payload: &[u8], required: &[bool]) -> Result<()> {
        Self::validate_entries(payload, required.len(), Some(required), None)
    }

    /// Decodes a field sequence and validates its exact field cardinality.
    ///
    /// `offsets` is supplied by the caller so a request handler can keep the
    /// small index array on its stack while the payload remains borrowed.
    pub fn decode(
        payload: &'a [u8],
        field_count: usize,
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        Self::validate_entries(payload, field_count, None, Some(offsets))?;
        Ok(Self {
            payload,
            offsets: &offsets[..field_count],
            field_count,
        })
    }

    /// Decodes a field sequence and rejects missing required fields.
    ///
    /// Requiredness is supplied by the API rather than inferred from a route.
    /// Optional fields retain their cleared mask bit
    /// and remain addressable through [`Self::get`].
    pub fn decode_with_required(
        payload: &'a [u8],
        required: &[bool],
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        Self::validate_entries(payload, required.len(), Some(required), Some(offsets))?;
        Ok(Self {
            payload,
            offsets: &offsets[..required.len()],
            field_count: required.len(),
        })
    }

    /// Returns the number of fields represented by this cursor.
    pub const fn len(self) -> usize {
        self.field_count
    }

    /// Returns one borrowed field, or `None` when the modeled field is absent.
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        if index >= self.field_count {
            return None;
        }
        let (start, end) = *self.offsets.get(index)?;
        if start == usize::MAX {
            None
        } else {
            Some(&self.payload[start..end])
        }
    }
}

/// Encodes aligned groups of fields as nested field sequences.
///
/// The outer sequence preserves group order; each group is itself an ordered
/// field sequence. This gives batch APIs a generic alignment primitive without
/// teaching the transport about any operation name.
pub fn encode_field_groups(groups: &[&[Option<&[u8]>]]) -> Result<Vec<u8>> {
    // Size the complete nested payload before allocating. The previous
    // implementation encoded every group into a temporary Vec, collected a
    // second Vec of references, and then copied those groups into the outer
    // sequence. Batch-shaped APIs are expected to use this primitive on a
    // hot path, so keep one output allocation and append each group directly.
    let outer_mask_bytes = groups.len().saturating_add(7) / 8;
    let mut encoded_len = outer_mask_bytes;
    for (index, group) in groups.iter().enumerate() {
        let group_len = field_sequence_encoded_len(group)?;
        let prefix_len = if index + 1 < groups.len() {
            let group_len_u64 =
                u64::try_from(group_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            encode_varuint(group_len_u64).1
        } else {
            0
        };
        encoded_len = encoded_len
            .checked_add(prefix_len)
            .and_then(|length| length.checked_add(group_len))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    validate_payload_length(encoded_len)?;
    let mut payload = Vec::with_capacity(encoded_len);
    payload.resize(outer_mask_bytes, 0);
    for (index, group) in groups.iter().enumerate() {
        let group_len = field_sequence_encoded_len(group)?;
        payload[index / 8] |= 1 << (index % 8);
        let (encoded, prefix_len) = if index + 1 < groups.len() {
            let group_len_u64 =
                u64::try_from(group_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            encode_varuint(group_len_u64)
        } else {
            ([0; MAX_VARUINT_BYTES], 0)
        };
        payload.extend_from_slice(&encoded[..prefix_len]);
        append_field_sequence(group, &mut payload)?;
    }
    debug_assert_eq!(payload.len(), encoded_len);
    Ok(payload)
}

/// A zero-copy view over the outer groups of a nested field sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldGroups<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
}

impl<'a, 'b> FieldGroups<'a, 'b> {
    /// Decodes and validates the exact number of groups and their field
    /// cardinalities.
    pub fn decode(
        payload: &'a [u8],
        group_widths: &[usize],
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        if offsets.len() < group_widths.len() {
            return Err(ProtocolError::InvalidFieldSequence(
                "group offset storage is smaller than the modeled group count",
            ));
        }
        let outer = FieldSequence::decode(payload, group_widths.len(), offsets)?;
        for (index, width) in group_widths.iter().enumerate() {
            let group = outer.get(index).ok_or(ProtocolError::InvalidFieldSequence(
                "field group is missing",
            ))?;
            validate_field_sequence_payload(group, *width)?;
        }
        Ok(Self {
            payload,
            offsets: &offsets[..group_widths.len()],
        })
    }

    /// Returns the encoded field-sequence payload for one group.
    pub fn group_payload(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        if start == usize::MAX {
            None
        } else {
            Some(&self.payload[start..end])
        }
    }
}

fn validate_field_sequence_payload(payload: &[u8], field_count: usize) -> Result<()> {
    FieldSequence::validate(payload, field_count)
}

/// Encodes one canonical unsigned 64-bit `vu128`.
pub fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    let mut encoded = [0; MAX_VARUINT_BYTES];
    let length = vu128::encode_u64(&mut encoded, value);
    (encoded, length)
}

/// Decodes one canonical unsigned 64-bit `vu128`.
///
/// `Ok(None)` means that the prefix is valid but incomplete. The context is
/// included only in malformed-input diagnostics; it does not assign semantic
/// meaning to the field.
pub fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
    let Some(&first) = input.first() else {
        return Ok(None);
    };
    let encoded_len = vu128::encoded_len(first);
    if encoded_len > MAX_VARUINT_BYTES {
        return Err(ProtocolError::VaruintOverflow { context });
    }
    if input.len() < encoded_len {
        return Ok(None);
    }
    let mut encoded = [0; MAX_VARUINT_BYTES];
    encoded[..encoded_len].copy_from_slice(&input[..encoded_len]);
    let (value, decoded_len) = vu128::decode_u64(&encoded);
    if decoded_len != encoded_len {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    let mut canonical = [0; MAX_VARUINT_BYTES];
    let canonical_len = vu128::encode_u64(&mut canonical, value);
    if canonical_len != encoded_len || canonical[..canonical_len] != input[..encoded_len] {
        return Err(ProtocolError::NonCanonicalVaruint { context });
    }
    Ok(Some((value, encoded_len)))
}

pub(crate) fn validate_payload_length(payload_len: usize) -> Result<()> {
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: payload_len,
            maximum: MAX_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

/// Errors that belong to the common wire boundary.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame is too short: expected at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },
    #[error("frame length does not match header: expected {expected} bytes, got {actual}")]
    FrameLength { expected: usize, actual: usize },
    #[error("frame length overflow")]
    FrameLengthOverflow,
    #[error("invalid frame layout: {0}")]
    InvalidFrameLayout(&'static str),
    #[error("{context} uses a non-canonical vu128 encoding")]
    NonCanonicalVaruint { context: &'static str },
    #[error("{context} exceeds the supported 64-bit vu128 range")]
    VaruintOverflow { context: &'static str },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("invalid optional-value payload: {0}")]
    InvalidOptionalValues(&'static str),
    #[error("invalid operation field sequence: {0}")]
    InvalidFieldSequence(&'static str),
}

/// Convenience result type for common wire operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;
