//! Operation-neutral request frame delimiting.
//!
//! A request layout is generated from the wire contract and supplied by the
//! protocol adapter.  This module only consumes byte steps; it does not know
//! whether a fixed field is a namespace, item ID, policy flag, or any other
//! domain value.

use crate::{MAX_VALUE_BYTES, OPCODE_BYTES, Opcode, OwnedRange, ProtocolError, Result};
use smallvec::SmallVec;

const MAX_REQUEST_WIRE_FIELDS: usize = 256;
const INLINE_REQUEST_WIRE_FIELDS: usize = 8;

/// One generated field selector inside a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestPackedField {
    /// Generated request-field index.
    pub field: usize,
    /// Bits owned by this field.
    pub mask: u8,
}

/// One operation-neutral byte-consumption step in a request layout.
///
/// The conditional steps are intentionally expressed in terms of byte
/// offsets, masks, and lengths.  They do not assign semantic names to the
/// selected fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestFrameStep {
    /// Consume a fixed number of bytes.
    Fixed { bytes: usize },
    /// Treat the next fixed-width bytes as the body without an outer length.
    ///
    /// This is the generic fixed-body counterpart to [`ValueLength`].  It is
    /// selected from the generated shape plan for dense required tuples, so a
    /// future API can use compact fixed framing without a protocol-specific
    /// parser branch. It MUST be the final step in a layout.
    FixedBody { bytes: usize },
    /// Consume one canonical `vu128` value and treat its value as the opaque
    /// body length.
    ValueLength,
    /// Consume one canonical `vu128` scalar that belongs to request metadata.
    VarUInt,
    /// Consume and validate one generated packed byte.
    Packed { fields: &'static [RequestPackedField] },
    /// Consume exact generated constant bytes.
    Bytes { expected: &'static [u8] },
    /// Consume nested steps when a previously decoded packed field matches.
    Conditional {
        field: usize,
        expected: u8,
        steps: &'static [RequestFrameStep],
    },
    /// Consume one byte length followed by that many bytes.
    ByteLength,
}

/// Generated request metadata used only to delimit one protocol frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrameLayout {
    /// Ordered byte-consumption steps for this operation.
    pub steps: &'static [RequestFrameStep],
}

/// One canonical field value encoded in a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestWirePackedValue {
    pub value: &'static [u8],
    pub bits: u8,
}

/// One modeled field encoded in a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestWirePackedField {
    pub field: usize,
    pub mask: u8,
    pub values: &'static [RequestWirePackedValue],
}

/// One declarative request field projection step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestWireStep {
    FixedField {
        field: usize,
        bytes: usize,
    },
    Packed {
        fields: &'static [RequestWirePackedField],
        reserved_mask: u8,
        constant_bits: u8,
    },
    ByteLengthField {
        field: usize,
    },
    VarUIntField {
        field: usize,
    },
    Conditional {
        field: usize,
        equals: &'static [u8],
        steps: &'static [RequestWireStep],
    },
    Bytes {
        expected: &'static [u8],
    },
    TrailingField {
        field: usize,
    },
}

/// Generated exact request plan for one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestWirePlan {
    pub field_count: usize,
    pub steps: &'static [RequestWireStep],
}

/// Decodes one generated compact request without operation-family knowledge.
///
/// The returned field table stores the common case inline for up to eight
/// fields; larger generated plans spill to the same `SmallVec` allocation
/// without changing the field ownership contract.
pub fn decode_request_wire_fields(
    prefix: OwnedRange,
    payload: OwnedRange,
    plan: RequestWirePlan,
) -> Result<SmallVec<[Option<OwnedRange>; 8]>> {
    let trailing_len = payload.len();
    decode_request_wire_fields_inner(prefix, Some(payload), trailing_len, plan)
}

/// Decodes exact request metadata before its trailing field is retained.
///
/// Header-only callers provide the already delimited trailing length. The
/// trailing field remains absent from the result; every prefix-backed field
/// is decoded and validated through the same plan as a complete request.
///
/// Field metadata uses the same inline-capacity table as
/// [`decode_request_wire_fields`].
pub fn decode_request_wire_prefix_fields(
    prefix: OwnedRange,
    trailing_len: usize,
    plan: RequestWirePlan,
) -> Result<SmallVec<[Option<OwnedRange>; 8]>> {
    decode_request_wire_fields_inner(prefix, None, trailing_len, plan)
}

fn decode_request_wire_fields_inner(
    prefix: OwnedRange,
    payload: Option<OwnedRange>,
    trailing_len: usize,
    plan: RequestWirePlan,
) -> Result<SmallVec<[Option<OwnedRange>; 8]>> {
    if plan.field_count > MAX_REQUEST_WIRE_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "request wire plan exceeds the generated field bound",
        ));
    }
    let mut fields =
        SmallVec::<[Option<OwnedRange>; INLINE_REQUEST_WIRE_FIELDS]>::with_capacity(
            plan.field_count,
        );
    fields.resize(plan.field_count, None);
    let mut cursor = OPCODE_BYTES;
    let mut has_trailing_field = false;
    decode_wire_steps(
        &prefix,
        payload.as_ref(),
        trailing_len,
        &mut cursor,
        plan.steps,
        &mut fields,
        true,
        &mut has_trailing_field,
    )?;
    if cursor != prefix.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "request wire plan did not consume the complete prefix",
        ));
    }
    if payload.as_ref().is_some_and(|payload| !payload.is_empty()) && !has_trailing_field {
        return Err(ProtocolError::InvalidFieldSequence(
            "request wire plan does not declare a trailing field",
        ));
    }
    Ok(fields)
}

/// Encodes canonical generated field values through one exact request plan.
pub fn encode_request_wire_fields(
    opcode: Opcode,
    fields: &[Option<&[u8]>],
    plan: RequestWirePlan,
) -> Result<Vec<u8>> {
    let mut output = encode_request_wire_prefix(opcode, fields, plan)?;
    if let Some(value) = trailing_wire_value(fields, plan.steps)? {
        output.extend_from_slice(value);
    }
    Ok(output)
}

/// Encodes only generated request metadata, leaving a trailing application
/// field independently owned for vectored transport writes.
pub fn encode_request_wire_prefix(
    opcode: Opcode,
    fields: &[Option<&[u8]>],
    plan: RequestWirePlan,
) -> Result<Vec<u8>> {
    if fields.len() != plan.field_count || plan.field_count > MAX_REQUEST_WIRE_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "request wire values do not match the generated field plan",
        ));
    }
    let mut output = vec![opcode as u8];
    encode_wire_steps(fields, plan.steps, &mut output, true)?;
    Ok(output)
}

fn encode_wire_steps(
    fields: &[Option<&[u8]>],
    steps: &[RequestWireStep],
    output: &mut Vec<u8>,
    allow_trailing: bool,
) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        match *step {
            RequestWireStep::FixedField { field, bytes } => {
                let value = required_wire_value(fields, field)?;
                if value.len() != bytes {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "fixed request field has the wrong width",
                    ));
                }
                output.extend_from_slice(value);
            }
            RequestWireStep::Packed {
                fields: packed_fields,
                reserved_mask,
                constant_bits,
            } => {
                let mut byte = constant_bits;
                for packed in packed_fields {
                    let value = required_wire_value(fields, packed.field)?;
                    let mapping = packed
                        .values
                        .iter()
                        .find(|mapping| mapping.value == value)
                        .ok_or(ProtocolError::InvalidFieldSequence(
                            "request field has no generated packed mapping",
                        ))?;
                    byte |= mapping.bits;
                }
                if byte & reserved_mask != 0 {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "generated request packed byte sets reserved bits",
                    ));
                }
                output.push(byte);
            }
            RequestWireStep::ByteLengthField { field } => {
                let value = required_wire_value(fields, field)?;
                let length = u8::try_from(value.len()).map_err(|_| {
                    ProtocolError::InvalidFieldSequence(
                        "byte-length request field exceeds 255 bytes",
                    )
                })?;
                output.push(length);
                output.extend_from_slice(value);
            }
            RequestWireStep::VarUIntField { field } => {
                let value: [u8; 8] = required_wire_value(fields, field)?
                    .try_into()
                    .map_err(|_| {
                        ProtocolError::InvalidFieldSequence(
                            "varuint request field must contain one big-endian u64",
                        )
                    })?;
                let (encoded, length) = crate::encode_varuint(u64::from_be_bytes(value));
                output.extend_from_slice(&encoded[..length]);
            }
            RequestWireStep::Conditional {
                field,
                equals,
                steps,
            } => {
                if fields.get(field).and_then(|value| *value) == Some(equals) {
                    encode_wire_steps(fields, steps, output, false)?;
                }
            }
            RequestWireStep::Bytes { expected } => output.extend_from_slice(expected),
            RequestWireStep::TrailingField { field } => {
                if !allow_trailing || index + 1 != steps.len() {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "trailing request field must be the final top-level step",
                    ));
                }
                let value = required_wire_value(fields, field)?;
                validate_value_length(value.len())?;
                let (encoded, length) = crate::encode_varuint(value.len() as u64);
                output.extend_from_slice(&encoded[..length]);
            }
        }
    }
    Ok(())
}

fn trailing_wire_value<'a>(
    fields: &'a [Option<&'a [u8]>],
    steps: &[RequestWireStep],
) -> Result<Option<&'a [u8]>> {
    let mut trailing = None;
    for step in steps {
        match *step {
            RequestWireStep::TrailingField { field } => {
                if trailing.replace(required_wire_value(fields, field)?).is_some() {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "request wire plan contains multiple trailing fields",
                    ));
                }
            }
            RequestWireStep::Conditional { steps, .. } => {
                if trailing_wire_value(fields, steps)?.is_some() {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "request wire plan nests a trailing field",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(trailing)
}

fn required_wire_value<'a>(
    fields: &'a [Option<&'a [u8]>],
    field: usize,
) -> Result<&'a [u8]> {
    fields
        .get(field)
        .and_then(|value| *value)
        .ok_or(ProtocolError::InvalidFieldSequence(
            "request wire plan requires a missing field",
        ))
}

fn decode_wire_steps(
    prefix: &OwnedRange,
    payload: Option<&OwnedRange>,
    trailing_len: usize,
    cursor: &mut usize,
    steps: &[RequestWireStep],
    fields: &mut [Option<OwnedRange>],
    allow_trailing: bool,
    has_trailing_field: &mut bool,
) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        match *step {
            RequestWireStep::FixedField { field, bytes } => {
                let end = cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                store_wire_field(
                    fields,
                    field,
                    prefix.clone().slice(*cursor..end).ok_or(
                        ProtocolError::InvalidFieldSequence(
                            "fixed request field exceeds the retained prefix",
                        ),
                    )?,
                )?;
                *cursor = end;
            }
            RequestWireStep::Packed {
                fields: packed_fields,
                reserved_mask,
                constant_bits,
            } => {
                let byte = *prefix.as_slice().get(*cursor).ok_or(
                    ProtocolError::InvalidFieldSequence(
                        "packed request field exceeds the retained prefix",
                    ),
                )?;
                if byte & reserved_mask != 0 || byte & constant_bits != constant_bits {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "request packed byte violates the generated bit contract",
                    ));
                }
                for packed in packed_fields {
                    let selected = byte & packed.mask;
                    let mapping = packed
                        .values
                        .iter()
                        .find(|mapping| mapping.bits == selected)
                        .ok_or(ProtocolError::InvalidFieldSequence(
                            "request packed field contains an unknown bit pattern",
                        ))?;
                    store_wire_field(
                        fields,
                        packed.field,
                        OwnedRange::from_static(mapping.value),
                    )?;
                }
                *cursor = cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestWireStep::ByteLengthField { field } => {
                let length = usize::from(*prefix.as_slice().get(*cursor).ok_or(
                    ProtocolError::InvalidFieldSequence(
                        "byte-length request field is missing its length",
                    ),
                )?);
                let start = cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let end = start
                    .checked_add(length)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                store_wire_field(
                    fields,
                    field,
                    prefix.clone().slice(start..end).ok_or(
                        ProtocolError::InvalidFieldSequence(
                            "byte-length request field exceeds the retained prefix",
                        ),
                    )?,
                )?;
                *cursor = end;
            }
            RequestWireStep::VarUIntField { field } => {
                let (value, encoded_len) = crate::decode_varuint(
                    prefix.as_slice().get(*cursor..).unwrap_or_default(),
                    "request wire integer",
                )?
                .ok_or(ProtocolError::InvalidFieldSequence(
                    "request wire integer is incomplete",
                ))?;
                store_wire_field(
                    fields,
                    field,
                    OwnedRange::whole(value.to_be_bytes().to_vec()),
                )?;
                *cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestWireStep::Conditional {
                field,
                equals,
                steps,
            } => {
                let selected = fields.get(field).and_then(Option::as_ref).ok_or(
                    ProtocolError::InvalidFieldSequence(
                        "request condition references an undecoded field",
                    ),
                )?;
                if selected.as_slice() == equals {
                    decode_wire_steps(
                        prefix,
                        payload,
                        trailing_len,
                        cursor,
                        steps,
                        fields,
                        false,
                        has_trailing_field,
                    )?;
                }
            }
            RequestWireStep::Bytes { expected } => {
                let end = cursor
                    .checked_add(expected.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.as_slice().get(*cursor..end) != Some(expected) {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "request constant bytes do not match the generated plan",
                    ));
                }
                *cursor = end;
            }
            RequestWireStep::TrailingField { field } => {
                if !allow_trailing || index + 1 != steps.len() || *has_trailing_field {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "trailing request field must be the final top-level step",
                    ));
                }
                let (length, encoded_len) = crate::decode_varuint(
                    prefix.as_slice().get(*cursor..).unwrap_or_default(),
                    "request trailing field length",
                )?
                .ok_or(ProtocolError::InvalidFieldSequence(
                    "request trailing field length is incomplete",
                ))?;
                let length =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                validate_value_length(length)?;
                if length != trailing_len {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "request trailing field length does not match its payload",
                    ));
                }
                *cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                *has_trailing_field = true;
                if let Some(payload) = payload {
                    store_wire_field(fields, field, payload.clone())?;
                }
            }
        }
    }
    Ok(())
}

fn store_wire_field(
    fields: &mut [Option<OwnedRange>],
    field: usize,
    value: OwnedRange,
) -> Result<()> {
    let slot = fields.get_mut(field).ok_or(ProtocolError::InvalidFieldSequence(
        "request wire field exceeds the generated plan",
    ))?;
    if slot.replace(value).is_some() {
        return Err(ProtocolError::InvalidFieldSequence(
            "request wire plan assigns a field more than once",
        ));
    }
    Ok(())
}

/// Header metadata required to delimit one opaque request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrameHeader {
    opcode: Opcode,
    encoded_len: usize,
    value_len: usize,
}

impl RequestFrameHeader {
    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of bytes before the opaque body.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the body length carried by a value-bearing layout.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns the complete frame length.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.value_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A complete request viewed as an opaque operation call.
///
/// The parser owns only frame delimiting.  A generated client or a server
/// adapter may inspect [`body`](Self::body) after it has selected the
/// operation's modeled request shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpaqueRequestFrame<'a> {
    opcode: Opcode,
    frame: &'a [u8],
    body_offset: usize,
}

impl<'a> OpaqueRequestFrame<'a> {
    /// Decodes request metadata using the generated layout supplied by the
    /// caller.
    pub fn decode_header(
        prefix: &[u8],
        layout: RequestFrameLayout,
    ) -> Result<Option<RequestFrameHeader>> {
        decode_request_frame_header(prefix, layout)
    }

    /// Returns the exact number of additional metadata bytes that can be read
    /// without consuming the opaque body.
    pub fn header_bytes_needed(prefix: &[u8], layout: RequestFrameLayout) -> Result<usize> {
        request_frame_header_bytes_needed(prefix, layout)
    }

    /// Reports the complete frame length once enough metadata is available.
    pub fn frame_len(prefix: &[u8], layout: RequestFrameLayout) -> Result<Option<usize>> {
        Self::decode_header(prefix, layout)?
            .map(RequestFrameHeader::frame_len)
            .transpose()
    }

    /// Decodes one complete request without interpreting its operation body.
    pub fn decode(frame: &'a [u8], layout: RequestFrameLayout) -> Result<Self> {
        let header = Self::decode_header(frame, layout)?.ok_or(ProtocolError::FrameTooShort {
            expected: OPCODE_BYTES,
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
            opcode: header.opcode,
            frame,
            // `encoded_len` includes every generated prefix step, including
            // a variable-length body prefix. Keeping the offset from the
            // decoded header prevents callers from accidentally exposing a
            // length varuint as part of the opaque operation body.
            body_offset: header.encoded_len,
        })
    }

    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the opaque operation body after the opcode.
    pub fn body(self) -> &'a [u8] {
        &self.frame[self.body_offset..]
    }

    /// Returns the original complete encoded frame.
    pub const fn encoded(self) -> &'a [u8] {
        self.frame
    }
}

/// Decodes one request header using only generated byte-consumption metadata.
pub fn decode_request_frame_header(
    prefix: &[u8],
    layout: RequestFrameLayout,
) -> Result<Option<RequestFrameHeader>> {
    match decode_request_frame_header_progress(prefix, layout)? {
        DecodeProgress::Complete(header) => Ok(Some(header)),
        DecodeProgress::Incomplete { .. } => Ok(None),
    }
}

/// Returns the exact number of additional metadata bytes required to advance
/// request-header decoding.
///
/// A zero result means the header is complete. The returned count never
/// includes bytes belonging to a [`RequestFrameStep::FixedBody`] or
/// [`RequestFrameStep::ValueLength`] body, so a transport can safely use it as
/// its next read bound before body admission.
pub fn request_frame_header_bytes_needed(
    prefix: &[u8],
    layout: RequestFrameLayout,
) -> Result<usize> {
    match decode_request_frame_header_progress(prefix, layout)? {
        DecodeProgress::Complete(_) => Ok(0),
        DecodeProgress::Incomplete { additional } => Ok(additional),
    }
}

enum DecodeProgress<T> {
    Complete(T),
    Incomplete { additional: usize },
}

fn decode_request_frame_header_progress(
    prefix: &[u8],
    layout: RequestFrameLayout,
) -> Result<DecodeProgress<RequestFrameHeader>> {
    let Some(&opcode_byte) = prefix.first() else {
        return Ok(DecodeProgress::Incomplete { additional: 1 });
    };
    let opcode = Opcode::try_from(opcode_byte)?;
    let mut cursor: usize = 0;
    let mut value_len = 0;
    let mut selectors = [None; MAX_REQUEST_WIRE_FIELDS];
    for (step_index, step) in layout.steps.iter().enumerate() {
        match *step {
            RequestFrameStep::Fixed { bytes } => {
                let end = cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                cursor = end;
            }
            RequestFrameStep::FixedBody { bytes } => {
                if step_index + 1 != layout.steps.len() {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "fixed-body frame step must be last",
                    ));
                }
                validate_value_length(bytes)?;
                value_len = bytes;
            }
            RequestFrameStep::ValueLength => {
                let (length, encoded_len) =
                    match decode_varuint_progress(&prefix[cursor..], "request value length")? {
                        DecodeProgress::Complete(decoded) => decoded,
                        DecodeProgress::Incomplete { additional } => {
                            return Ok(DecodeProgress::Incomplete { additional });
                        }
                    };
                value_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                validate_value_length(value_len)?;
                cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::VarUInt => {
                let encoded_len =
                    match decode_varuint_progress(&prefix[cursor..], "request integer")? {
                        DecodeProgress::Complete((_, encoded_len)) => encoded_len,
                        DecodeProgress::Incomplete { additional } => {
                            return Ok(DecodeProgress::Incomplete { additional });
                        }
                    };
                cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::Packed { fields } => {
                match decode_packed_byte(prefix, &mut cursor, fields, &mut selectors)? {
                    DecodeProgress::Complete(()) => {}
                    DecodeProgress::Incomplete { additional } => {
                        return Ok(DecodeProgress::Incomplete { additional });
                    }
                }
            }
            RequestFrameStep::Bytes { expected } => {
                let end = cursor
                    .checked_add(expected.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                cursor = end;
            }
            RequestFrameStep::Conditional {
                field,
                expected,
                steps,
            } => {
                let selected = selectors.get(field).copied().flatten().ok_or(
                    ProtocolError::InvalidFieldSequence(
                        "request condition references an undecoded packed field",
                    ),
                )?;
                if selected == expected {
                    match decode_nested_steps(prefix, &mut cursor, steps, &mut selectors)? {
                        DecodeProgress::Complete(()) => {}
                        DecodeProgress::Incomplete { additional } => {
                            return Ok(DecodeProgress::Incomplete { additional });
                        }
                    }
                }
            }
            RequestFrameStep::ByteLength => {
                let Some(&length) = prefix.get(cursor) else {
                    return Ok(DecodeProgress::Incomplete { additional: 1 });
                };
                let length = usize::from(length);
                let end = cursor
                    .checked_add(1)
                    .and_then(|end| end.checked_add(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                cursor = end;
            }
        }
    }
    Ok(DecodeProgress::Complete(RequestFrameHeader {
        opcode,
        encoded_len: cursor,
        value_len,
    }))
}

fn decode_nested_steps(
    prefix: &[u8],
    cursor: &mut usize,
    steps: &[RequestFrameStep],
    selectors: &mut [Option<u8>; MAX_REQUEST_WIRE_FIELDS],
) -> Result<DecodeProgress<()>> {
    for step in steps {
        match *step {
            RequestFrameStep::Fixed { bytes } => {
                let end = cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                *cursor = end;
            }
            RequestFrameStep::VarUInt => {
                let encoded_len =
                    match decode_varuint_progress(&prefix[*cursor..], "request integer")? {
                        DecodeProgress::Complete((_, encoded_len)) => encoded_len,
                        DecodeProgress::Incomplete { additional } => {
                            return Ok(DecodeProgress::Incomplete { additional });
                        }
                    };
                *cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::Packed { fields } => match decode_packed_byte(
                prefix,
                cursor,
                fields,
                selectors,
            )? {
                DecodeProgress::Complete(()) => {}
                DecodeProgress::Incomplete { additional } => {
                    return Ok(DecodeProgress::Incomplete { additional });
                }
            },
            RequestFrameStep::Bytes { expected } => {
                let end = cursor
                    .checked_add(expected.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                *cursor = end;
            }
            RequestFrameStep::ByteLength => {
                let Some(&length) = prefix.get(*cursor) else {
                    return Ok(DecodeProgress::Incomplete { additional: 1 });
                };
                let end = cursor
                    .checked_add(1)
                    .and_then(|end| end.checked_add(usize::from(length)))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(DecodeProgress::Incomplete {
                        additional: end - prefix.len(),
                    });
                }
                *cursor = end;
            }
            RequestFrameStep::Conditional {
                field,
                expected,
                steps,
            } => {
                let selected = selectors.get(field).copied().flatten().ok_or(
                    ProtocolError::InvalidFieldSequence(
                        "request condition references an undecoded packed field",
                    ),
                )?;
                if selected == expected {
                    match decode_nested_steps(prefix, cursor, steps, selectors)? {
                        DecodeProgress::Complete(()) => {}
                        DecodeProgress::Incomplete { additional } => {
                            return Ok(DecodeProgress::Incomplete { additional });
                        }
                    }
                }
            }
            RequestFrameStep::FixedBody { .. }
            | RequestFrameStep::ValueLength => {
                return Err(ProtocolError::InvalidFieldSequence(
                    "request wire plan nests an invalid framing step",
                ));
            }
        }
    }
    Ok(DecodeProgress::Complete(()))
}

fn decode_packed_byte(
    prefix: &[u8],
    cursor: &mut usize,
    fields: &[RequestPackedField],
    selectors: &mut [Option<u8>; MAX_REQUEST_WIRE_FIELDS],
) -> Result<DecodeProgress<()>> {
    let Some(&byte) = prefix.get(*cursor) else {
        return Ok(DecodeProgress::Incomplete { additional: 1 });
    };
    for field in fields {
        let selected = byte & field.mask;
        let slot = selectors.get_mut(field.field).ok_or(
            ProtocolError::InvalidFieldSequence(
                "request packed field exceeds the generated field bound",
            ),
        )?;
        *slot = Some(selected);
    }
    *cursor = cursor
        .checked_add(1)
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    Ok(DecodeProgress::Complete(()))
}

fn decode_varuint_progress(
    input: &[u8],
    context: &'static str,
) -> Result<DecodeProgress<(u64, usize)>> {
    let Some(&first) = input.first() else {
        return Ok(DecodeProgress::Incomplete { additional: 1 });
    };
    let encoded_len = vu128::encoded_len(first);
    if encoded_len > crate::MAX_VARUINT_BYTES {
        return Err(ProtocolError::VaruintOverflow { context });
    }
    if input.len() < encoded_len {
        return Ok(DecodeProgress::Incomplete {
            additional: encoded_len - input.len(),
        });
    }
    let decoded = crate::decode_varuint(input, context)?
        .expect("a complete canonical varuint prefix must decode");
    Ok(DecodeProgress::Complete(decoded))
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
