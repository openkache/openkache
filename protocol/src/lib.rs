//! Primitive wire values and framing shared by clients and servers.
//!
//! Request bodies remain outside this crate. Version 1 has no common request
//! length field: the generated layout selects operation-specific byte steps.
//! The [`request`] module consumes those steps without assigning them domain
//! meaning, while protocol-v1 adapters own compact request semantics. The
//! shared [`codec`] module provides operation-neutral value validation and
//! container primitives.

macro_rules! wire_enum {
    (
        $(#[$metadata:meta])*
        pub enum $name:ident {
            $($variant:ident = $value:expr),+ $(,)?
        }
        unknown => $unknown:ident
    ) => {
        $(#[$metadata])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub enum $name {
            $($variant = $value),+
        }

        impl TryFrom<u8> for $name {
            type Error = ProtocolError;

            fn try_from(value: u8) -> Result<Self> {
                match value {
                    $(value if value == Self::$variant as u8 => Ok(Self::$variant),)+
                    _ => Err(ProtocolError::$unknown(value)),
                }
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/wire_values.rs"));

/// Canonical operation framing, field, and codec metadata generated from the
/// protocol model. Server and client adapters re-export this artifact instead
/// of rendering their own operation contract copies.
pub mod operation {
    use super::{Opcode, Status};

    include!(concat!(env!("OUT_DIR"), "/operation_contract.rs"));
}

// Keep the crate root limited to generic operation metadata. Client retry/result
// projections and server execution metadata remain available through the
// explicit `operation` module so adapters cannot acquire them through a broad
// root import.
pub use operation::{
    MAX_OPERATION_FIELDS, MAX_OPERATION_REQUEST_FIELDS, OPERATION_CODEC_NAMES,
    OperationFieldLayout, OperationFieldPlan, OperationFramePolicy, OperationLayoutFraming,
    OperationLayoutPlan, OperationRequestFraming, OperationResponseFraming, OperationWireSpec,
    WIRE_CODEC_DESCRIPTORS, WIRE_CODEC_NAMES, WireCodecCardinality, WireCodecDescriptor,
    WireCodecKind, WireCodecLengthEncoding, WireCodecWidth, operation_registry,
    operation_wire_spec, request_fields, response_fields, wire_codec_kind,
};

/// Protocol-v1 compatibility projections.
///
/// Generic operation metadata does not contain these routes. The v1 server and
/// client adapters opt into this module explicitly, keeping the route
/// vocabulary out of shared request/response infrastructure.
pub mod compat_v1;

/// Generic value-shape codecs shared by server and client adapters.
pub mod codec;
/// Generic generated field-layout dispatch.
pub mod layout;
/// Operation-neutral request frame delimiting.
pub mod request;
/// Operation-neutral response framing and owned response buffers.
pub mod response;

pub use layout::{
    DenseFields, decode_layout_fields, decode_planned_fields, encode_dense_fields,
    encode_layout_fields, encode_planned_fields,
};
pub use request::{
    OpaqueRequestFrame, RequestFrameHeader, RequestFrameLayout, RequestFrameStep,
    decode_request_frame_header,
};
pub use response::{
    OwnedRange, OwnedResponseFrame, Response, ResponseFrame, ResponseHeader, ResponseParts,
    ResponseSegment,
};

/// The exact fixed-size item identifier carried by the wire protocol.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId([u8; ITEM_ID_BYTES]);

impl ItemId {
    /// Wraps an exact item ID without interpreting its bytes.
    pub const fn new(bytes: [u8; ITEM_ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the complete item ID bytes.
    pub const fn as_bytes(&self) -> &[u8; ITEM_ID_BYTES] {
        &self.0
    }

    /// Consumes the item ID and returns its bytes.
    pub const fn into_bytes(self) -> [u8; ITEM_ID_BYTES] {
        self.0
    }
}

impl AsRef<[u8]> for ItemId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Status {
    /// Returns whether this status represents a server-side error.
    pub const fn is_error(self) -> bool {
        (self as u8) >= ERROR_STATUS_MINIMUM
    }
}

/// Encodes ordered optional values with one big-endian length per entry.
///
/// The framing is intentionally independent of any operation name. A value
/// length of `u32::MAX` is reserved for `None`; every present value must be
/// strictly smaller. The aggregate payload is bounded by the same wire value
/// ceiling used by response frames.
pub fn encode_optional_values(values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
    let mut encoder = OptionalValuesEncoder::new(values.len());
    for value in values {
        encoder.push(*value)?;
    }
    encoder.finish()
}

/// Returns the encoded length of the fixed-width optional-value layout
/// without allocating the payload.
///
/// This is useful to a transport planner that wants to compare the fixed
/// four-byte length table with a generic field sequence before reserving an
/// output buffer. The calculation validates the same per-value and aggregate
/// limits as [`encode_optional_values`].
pub fn optional_values_encoded_len(values: &[Option<&[u8]>]) -> Result<usize> {
    optional_values_encoded_len_iter(values.iter().map(|value| value.map(<[u8]>::len)))
}

/// Computes an optional-value payload size from lengths alone.
///
/// The fixed-width table is selected explicitly by generated layout metadata;
/// it is never inferred from an operation's semantic name.
pub fn optional_values_encoded_len_from_lengths(lengths: &[Option<usize>]) -> Result<usize> {
    optional_values_encoded_len_iter(lengths.iter().copied())
}

fn optional_values_encoded_len_iter(lengths: impl Iterator<Item = Option<usize>>) -> Result<usize> {
    let mut encoded_len = 0usize;
    let mut field_count = 0usize;
    for length in lengths {
        field_count = field_count
            .checked_add(1)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let value_len = length.unwrap_or(0);
        if value_len >= OPTIONAL_VALUE_MISSING as usize {
            return Err(ProtocolError::ValueTooLarge {
                size: value_len,
                maximum: (OPTIONAL_VALUE_MISSING - 1) as usize,
            });
        }
        encoded_len = encoded_len
            .checked_add(value_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
    }
    encoded_len = encoded_len
        .checked_add(
            field_count
                .checked_mul(OPTIONAL_VALUE_LENGTH_BYTES)
                .ok_or(ProtocolError::FrameLengthOverflow)?,
        )
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    validate_value_length(encoded_len)?;
    Ok(encoded_len)
}

/// Incremental encoder for an ordered optional-value field sequence.
///
/// Server behaviors can append already-decoded domain values directly without
/// first constructing a temporary `Vec<Option<&[u8]>>`.  The encoder owns only
/// the aggregate framing buffer; it does not interpret operation roles,
/// requiredness, or response semantics.
#[derive(Debug)]
pub struct OptionalValuesEncoder {
    payload: Vec<u8>,
    expected_fields: usize,
    written_fields: usize,
}

impl OptionalValuesEncoder {
    /// Creates an encoder for exactly `field_count` ordered fields.
    pub fn new(field_count: usize) -> Self {
        Self {
            payload: Vec::with_capacity(
                field_count
                    .saturating_mul(OPTIONAL_VALUE_LENGTH_BYTES)
                    .min(MAX_VALUE_BYTES),
            ),
            expected_fields: field_count,
            written_fields: 0,
        }
    }

    /// Appends one present field or the canonical missing sentinel.
    pub fn push(&mut self, value: Option<&[u8]>) -> Result<()> {
        if self.written_fields >= self.expected_fields {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value encoder received too many fields",
            ));
        }
        let value_len = value.map_or(0, <[u8]>::len);
        if value_len >= OPTIONAL_VALUE_MISSING as usize {
            return Err(ProtocolError::ValueTooLarge {
                size: value_len,
                maximum: (OPTIONAL_VALUE_MISSING - 1) as usize,
            });
        }
        let next_len = self
            .payload
            .len()
            .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
            .and_then(|length| length.checked_add(value_len))
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        validate_value_length(next_len)?;
        match value {
            None => self
                .payload
                .extend_from_slice(&OPTIONAL_VALUE_MISSING.to_be_bytes()),
            Some(value) => {
                self.payload
                    .extend_from_slice(&(value_len as u32).to_be_bytes());
                self.payload.extend_from_slice(value);
            }
        }
        self.written_fields += 1;
        Ok(())
    }

    /// Completes the sequence after all declared fields have been appended.
    pub fn finish(self) -> Result<Vec<u8>> {
        if self.written_fields != self.expected_fields {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value encoder received too few fields",
            ));
        }
        Ok(self.payload)
    }
}

/// Returns an upper bound for an optional-value payload.
///
/// The bound includes one shared length/sentinel prefix for every field and
/// `max_value_bytes` bytes for each present field. It intentionally does not
/// clamp the result to the aggregate response ceiling: callers use the bound
/// to reserve memory before an operation-specific response is encoded and the
/// encoder still enforces the aggregate ceiling on the actual payload.
pub fn optional_values_max_encoded_len(
    value_count: usize,
    max_value_bytes: usize,
) -> Option<usize> {
    value_count.checked_mul(OPTIONAL_VALUE_LENGTH_BYTES.checked_add(max_value_bytes)?)
}

/// A zero-copy view over the fixed-width optional-value response layout.
///
/// Callers provide bounded offset storage, allowing response adapters to
/// borrow every present value from the original frame without allocating one
/// buffer per field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalValues<'a, 'b> {
    payload: &'a [u8],
    offsets: &'b [(usize, usize)],
    field_count: usize,
}

impl<'a, 'b> OptionalValues<'a, 'b> {
    /// Decodes exactly `value_count` optional values into caller-owned offsets.
    pub fn decode(
        payload: &'a [u8],
        value_count: usize,
        offsets: &'b mut [(usize, usize)],
    ) -> Result<Self> {
        if offsets.len() < value_count {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value offset storage is smaller than the modeled field count",
            ));
        }
        validate_value_length(payload.len())?;
        let minimum = value_count
            .checked_mul(OPTIONAL_VALUE_LENGTH_BYTES)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        if payload.len() < minimum {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value payload is missing an entry length",
            ));
        }
        let mut cursor = 0usize;
        for index in 0..value_count {
            let prefix_end = cursor
                .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            let length = u32::from_be_bytes(
                payload[cursor..prefix_end]
                    .try_into()
                    .expect("optional-value length prefix has fixed width"),
            );
            cursor = prefix_end;
            if length == OPTIONAL_VALUE_MISSING {
                offsets[index] = (usize::MAX, usize::MAX);
                continue;
            }
            let length = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
            validate_value_length(length)?;
            let end = cursor
                .checked_add(length)
                .ok_or(ProtocolError::FrameLengthOverflow)?;
            if end > payload.len() {
                return Err(ProtocolError::InvalidOptionalValues(
                    "optional-value payload entry is truncated",
                ));
            }
            offsets[index] = (cursor, end);
            cursor = end;
        }
        if cursor != payload.len() {
            return Err(ProtocolError::InvalidOptionalValues(
                "optional-value payload contains trailing bytes",
            ));
        }
        Ok(Self {
            payload,
            offsets: &offsets[..value_count],
            field_count: value_count,
        })
    }

    /// Returns the number of modeled values.
    pub const fn len(self) -> usize {
        self.field_count
    }

    /// Returns one present value, preserving present-empty as `Some(&[])`.
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.offsets.get(index)?;
        (start != usize::MAX).then(|| &self.payload[start..end])
    }
}

/// Decodes ordered optional opaque values from the shared response codec.
pub fn decode_optional_values(payload: &[u8], value_count: usize) -> Result<Vec<Option<Vec<u8>>>> {
    validate_value_length(payload.len())?;
    let minimum = value_count
        .checked_mul(OPTIONAL_VALUE_LENGTH_BYTES)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    if payload.len() < minimum {
        return Err(ProtocolError::InvalidOptionalValues(
            "optional-value payload is missing an entry length",
        ));
    }
    let mut cursor = 0usize;
    let mut values = Vec::with_capacity(value_count);
    for _ in 0..value_count {
        let end = cursor
            .checked_add(OPTIONAL_VALUE_LENGTH_BYTES)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let length_bytes: [u8; OPTIONAL_VALUE_LENGTH_BYTES] = payload[cursor..end]
            .try_into()
            .expect("optional-value length has a fixed width");
        cursor = end;
        let length = u32::from_be_bytes(length_bytes);
        if length == OPTIONAL_VALUE_MISSING {
            values.push(None);
            continue;
        }
        let length = usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
        validate_value_length(length)?;
        let end = cursor
            .checked_add(length)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let bytes = payload
            .get(cursor..end)
            .ok_or(ProtocolError::InvalidOptionalValues(
                "optional-value payload entry is truncated",
            ))?;
        values.push(Some(bytes.to_vec()));
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(ProtocolError::InvalidOptionalValues(
            "optional-value payload contains trailing bytes",
        ));
    }
    Ok(values)
}

/// Encodes an ordered operation field sequence.
///
/// The payload starts with a compact presence mask, followed by canonical
/// `vu128` lengths and bytes for every present field before the final present
/// field. The final present field consumes the remaining bytes. The operation
/// plan supplies field order, cardinality, requiredness, and codecs; this
/// primitive only carries bounded opaque field bytes.
pub fn encode_field_sequence(values: &[Option<&[u8]>]) -> Result<Vec<u8>> {
    let mask_bytes = values.len().saturating_add(7) / 8;
    if mask_bytes > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: mask_bytes,
            maximum: MAX_VALUE_BYTES,
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
        validate_value_length(value.len())?;
        payload[mask_start + index / 8] |= 1 << (index % 8);
        let (encoded, encoded_len) = encode_varuint(
            u64::try_from(value.len()).map_err(|_| ProtocolError::FrameLengthOverflow)?,
        );
        if Some(index) != last_present {
            payload.extend_from_slice(&encoded[..encoded_len]);
        }
        payload.extend_from_slice(value);
    }
    validate_value_length(payload.len())?;
    Ok(())
}

/// Returns the encoded length of a generic presence-mask field sequence
/// without allocating the payload.
///
/// The returned value is the exact body size for the supplied fields. Every
/// present field before the final present field pays a canonical `vu128`
/// length prefix; the final present field consumes the remainder. It is
/// deliberately independent of operation names and semantic roles so
/// generated adapters can use it as a shared size-cost primitive.
pub fn field_sequence_encoded_len(values: &[Option<&[u8]>]) -> Result<usize> {
    field_sequence_encoded_len_iter(values.iter().map(|value| value.map(<[u8]>::len)))
}

/// Computes a field-sequence payload size from lengths alone.
///
/// This is the allocation-free cost primitive used by generated size
/// planners. `None` is a missing field; `Some(0)` is a present-empty field.
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
        validate_value_length(length)?;
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
    if mask_bytes > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: mask_bytes,
            maximum: MAX_VALUE_BYTES,
        });
    }
    let total = mask_bytes
        .checked_add(encoded_len)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    validate_value_length(total)?;
    Ok(total)
}

/// A zero-copy view over an ordered operation field sequence.
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
        validate_value_length(payload.len())?;
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
                validate_value_length(length)?;
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
            validate_value_length(length)?;
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

    /// Validates an ordered field sequence and its generated requiredness
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
    /// Requiredness is supplied by generated model metadata rather than
    /// inferred from the route. Optional fields retain their cleared mask bit
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
/// field sequence. This gives batch operations a generic item/value alignment
/// primitive and lets a future CAS request carry expected and replacement
/// values without teaching v1 about either operation name.
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
    validate_value_length(encoded_len)?;
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

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

/// Errors that belong to the common wire boundary.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("frame is too short: expected at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },
    #[error("frame length does not match header: expected {expected} bytes, got {actual}")]
    FrameLength { expected: usize, actual: usize },
    #[error("frame length overflow")]
    FrameLengthOverflow,
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
