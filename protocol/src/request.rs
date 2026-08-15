//! Operation-neutral request frame delimiting.
//!
//! An API supplies a request layout to this module. This module only consumes
//! byte steps; it does not know whether a fixed field is a namespace, item ID,
//! policy flag, or any other domain value.

use crate::{MAX_VALUE_BYTES, OPCODE_BYTES, Opcode, ProtocolError, Result};

const MAX_REQUEST_FRAME_STATE_SLOTS: usize = 8;
const EMPTY_BYTE_LENGTH: u16 = u16::MAX;
const NO_REQUEST_FIELD: usize = usize::MAX;

/// One canonical modeled field recovered while delimiting a request frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestFieldProjection {
    /// The field was not encoded, including a field in an untaken conditional.
    Missing,
    /// Exact field bytes borrowed by their range in the complete frame.
    Borrowed { start: usize, end: usize },
    /// Canonical fixed-width scalar bytes decoded from a compact wire value.
    Inline([u8; 8]),
    /// Canonical bytes selected from generated static metadata.
    Static(&'static [u8]),
}

/// One canonical value represented by bits in a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFramePackedValue {
    /// The masked wire bits selecting this value.
    pub bits: u8,
    /// Canonical codec bytes for the modeled value.
    pub bytes: &'static [u8],
}

/// One field projected from a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFramePackedField {
    /// Bounded state slot retained for a later conditional step.
    pub slot: usize,
    /// Numeric index in the modeled request field plan.
    pub field: usize,
    /// Bits belonging to this modeled field.
    pub mask: u8,
    /// Complete generated mapping from packed bits to canonical codec bytes.
    pub values: &'static [RequestFramePackedValue],
}

/// One operation-neutral byte-consumption step in a request layout.
///
/// Conditional steps use bounded selector slots, masks, and byte lengths.
/// They do not assign semantic names to the selected fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestFrameStep {
    /// Consume a fixed number of bytes.
    Fixed { bytes: usize },
    /// Consume and project a fixed-width modeled field.
    FixedField { field: usize, bytes: usize },
    /// Treat the next fixed-width bytes as the body without an outer length.
    ///
    /// This is the generic fixed-body counterpart to [`ValueLength`].  It is
    /// selected by an API for a dense required tuple, so a future API can use
    /// compact fixed framing without a protocol-specific parser branch. It
    /// MUST be the final step in a layout.
    FixedBody { bytes: usize },
    /// Consume one canonical `vu128` value and treat its value as the opaque
    /// body length.
    ValueLength,
    /// Consume one canonical `vu128` body length while allowing later metadata
    /// steps before the body.
    ValueLengthPrefix,
    /// Consume one canonical `vu128` length and project the terminal field.
    TrailingField { field: usize },
    /// Consume one canonical `vu128` body length before later metadata and
    /// project the terminal body.
    ValueLengthPrefixField { field: usize },
    /// Consume one canonical `vu128` metadata value.
    VarUInt,
    /// Consume one canonical `vu128` and project its fixed-width scalar bytes.
    VarUIntField { field: usize },
    /// Consume one packed byte and retain modeled selector bits.
    Packed {
        fields: &'static [RequestFramePackedField],
        reserved_mask: u8,
        constant_bits: u8,
    },
    /// Conditionally consume nested steps using a retained packed selector.
    Conditional {
        selector: usize,
        expected: u8,
        steps: &'static [RequestFrameStep],
    },
    /// Consume and validate exact constant bytes.
    Constant { bytes: &'static [u8] },
    /// Consume a conditional canonical `vu128`, selected by a previously
    /// decoded byte.
    ConditionalVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
    },
    /// Consume one byte length followed by that many bytes.
    ByteLength,
    /// Consume one byte length followed by a projected modeled field.
    ByteLengthField { field: usize },
    /// Consume one byte length and retain it for a later body step.
    ByteLengthPrefix { slot: usize },
    /// Consume the body declared by a preceding byte-length prefix.
    ByteLengthBody { slot: usize },
    /// Consume and project the modeled field declared by a preceding
    /// byte-length prefix.
    ByteLengthBodyField { slot: usize, field: usize },
    /// Consume a fixed prefix and, when the leading byte matches, a canonical
    /// `vu128`.
    ByteThenVarUInt {
        prefix_bytes: usize,
        mask: u8,
        expected: u8,
    },
    /// Consume a fixed prefix and conditionally consume a canonical `vu128`
    /// selected by both a selector byte and the prefix's leading byte.
    ConditionalByteThenVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
        prefix_bytes: usize,
        value_mask: u8,
        value_expected: u8,
    },
}

/// API-owned request metadata used only to delimit one protocol frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrameLayout {
    /// Ordered byte-consumption steps after the opcode.
    ///
    /// The opcode is always consumed by the shared parser and MUST NOT be
    /// repeated as a `Fixed` step.
    pub steps: &'static [RequestFrameStep],
    /// Number of modeled request fields represented by generated steps.
    ///
    /// Header-only decoding ignores this bound; projection decoding rejects an
    /// output slice that cannot represent every field, including conditional
    /// fields that are absent in the current frame.
    pub field_count: usize,
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
/// The parser owns only frame delimiting. An API client or server adapter
/// may inspect [`body`](Self::body) after it has selected the operation's
/// modeled request shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpaqueRequestFrame<'a> {
    opcode: Opcode,
    frame: &'a [u8],
    body_offset: usize,
}

impl<'a> OpaqueRequestFrame<'a> {
    /// Decodes request metadata using the layout supplied by the caller.
    pub fn decode_header(
        prefix: &[u8],
        layout: RequestFrameLayout,
    ) -> Result<Option<RequestFrameHeader>> {
        decode_request_frame_header(prefix, layout)
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
            // `encoded_len` includes every prefix step, including a
            // variable-length body prefix. Keeping the offset from the
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

/// Decodes one request header using only caller-supplied byte-consumption
/// metadata.
pub fn decode_request_frame_header(
    prefix: &[u8],
    layout: RequestFrameLayout,
) -> Result<Option<RequestFrameHeader>> {
    decode_request_frame_metadata::<false>(prefix, layout, &mut [])
        .map(|decoded| decoded.map(|decoded| decoded.header))
}

/// Projects modeled fields from one complete request frame.
///
/// Borrowed projections are frame-relative ranges, so this API validates the
/// exact complete frame length before returning them. The output is reset to
/// [`RequestFieldProjection::Missing`] before decoding and after any error.
pub fn project_request_frame(
    frame: &[u8],
    layout: RequestFrameLayout,
    fields: &mut [RequestFieldProjection],
) -> Result<RequestFrameHeader> {
    fields.fill(RequestFieldProjection::Missing);
    let result = (|| {
        if fields.len() < layout.field_count {
            return Err(ProtocolError::InvalidFieldSequence(
                "request field projection output is too short",
            ));
        }
        let decoded = decode_request_frame_metadata::<true>(
            frame,
            layout,
            &mut fields[..layout.field_count],
        )?
        .ok_or(ProtocolError::FrameTooShort {
            expected: OPCODE_BYTES,
            actual: frame.len(),
        })?;
        let expected = decoded.header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        if decoded.value_field != NO_REQUEST_FIELD {
            set_request_field_projection(
                &mut fields[..layout.field_count],
                decoded.value_field,
                RequestFieldProjection::Borrowed {
                    start: decoded.header.encoded_len,
                    end: expected,
                },
            )?;
        }
        Ok(decoded.header)
    })();
    if result.is_err() {
        fields.fill(RequestFieldProjection::Missing);
    }
    result
}

struct DecodedRequestFrame {
    header: RequestFrameHeader,
    value_field: usize,
}

fn decode_request_frame_metadata<const PROJECT_FIELDS: bool>(
    prefix: &[u8],
    layout: RequestFrameLayout,
    projections: &mut [RequestFieldProjection],
) -> Result<Option<DecodedRequestFrame>> {
    if prefix.len() < OPCODE_BYTES {
        return Ok(None);
    }
    let opcode_byte = prefix[0];
    let opcode = Opcode::try_from(opcode_byte)?;
    let mut state = RequestFrameDecodeState::new();
    if decode_request_frame_steps::<PROJECT_FIELDS>(prefix, layout.steps, &mut state, projections)?
        .is_none()
    {
        return Ok(None);
    }
    if state
        .byte_lengths
        .iter()
        .any(|length| *length != EMPTY_BYTE_LENGTH)
    {
        return Err(ProtocolError::InvalidFieldSequence(
            "byte-length prefix has no matching body step",
        ));
    }
    Ok(Some(DecodedRequestFrame {
        header: RequestFrameHeader {
            opcode,
            encoded_len: state.cursor,
            value_len: state.value_len,
        },
        value_field: state.value_field,
    }))
}

struct RequestFrameDecodeState {
    cursor: usize,
    value_len: usize,
    value_field: usize,
    value_length_seen: bool,
    terminal_body: bool,
    packed_values: [u8; MAX_REQUEST_FRAME_STATE_SLOTS],
    packed_present: u8,
    byte_lengths: [u16; MAX_REQUEST_FRAME_STATE_SLOTS],
}

impl RequestFrameDecodeState {
    const fn new() -> Self {
        Self {
            cursor: OPCODE_BYTES,
            value_len: 0,
            value_field: NO_REQUEST_FIELD,
            value_length_seen: false,
            terminal_body: false,
            packed_values: [0; MAX_REQUEST_FRAME_STATE_SLOTS],
            packed_present: 0,
            byte_lengths: [EMPTY_BYTE_LENGTH; MAX_REQUEST_FRAME_STATE_SLOTS],
        }
    }

    fn set_value_length(&mut self, value_len: usize, terminal: bool, field: usize) -> Result<()> {
        if self.value_length_seen {
            return Err(ProtocolError::InvalidFieldSequence(
                "request layout declares more than one value body",
            ));
        }
        validate_value_length(value_len)?;
        self.value_len = value_len;
        self.value_field = field;
        self.value_length_seen = true;
        self.terminal_body = terminal;
        Ok(())
    }
}

fn decode_request_frame_steps<const PROJECT_FIELDS: bool>(
    prefix: &[u8],
    steps: &[RequestFrameStep],
    state: &mut RequestFrameDecodeState,
    projections: &mut [RequestFieldProjection],
) -> Result<Option<()>> {
    for step in steps {
        if state.terminal_body {
            return Err(ProtocolError::InvalidFieldSequence(
                "request body must be the final frame step",
            ));
        }
        match *step {
            RequestFrameStep::Fixed { bytes } => {
                let end = state
                    .cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                state.cursor = end;
            }
            RequestFrameStep::FixedField { field, bytes } => {
                let start = state.cursor;
                let end = start
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                project_request_field::<PROJECT_FIELDS>(
                    projections,
                    field,
                    RequestFieldProjection::Borrowed { start, end },
                )?;
                state.cursor = end;
            }
            RequestFrameStep::FixedBody { bytes } => {
                state.set_value_length(bytes, true, NO_REQUEST_FIELD)?;
                let body_end = state
                    .cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < body_end {
                    return Ok(None);
                }
            }
            RequestFrameStep::ValueLength => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request value length",
                )?
                else {
                    return Ok(None);
                };
                let value_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_value_length(value_len, true, NO_REQUEST_FIELD)?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::ValueLengthPrefix => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request value length",
                )?
                else {
                    return Ok(None);
                };
                let value_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_value_length(value_len, false, NO_REQUEST_FIELD)?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::TrailingField { field }
            | RequestFrameStep::ValueLengthPrefixField { field } => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request value length",
                )?
                else {
                    return Ok(None);
                };
                let value_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_value_length(
                    value_len,
                    matches!(*step, RequestFrameStep::TrailingField { .. }),
                    field,
                )?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::VarUInt => {
                let Some((_, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request integer",
                )?
                else {
                    return Ok(None);
                };
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::VarUIntField { field } => {
                let Some((value, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request integer",
                )?
                else {
                    return Ok(None);
                };
                project_request_field::<PROJECT_FIELDS>(
                    projections,
                    field,
                    RequestFieldProjection::Inline(value.to_be_bytes()),
                )?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::Packed {
                fields,
                reserved_mask,
                constant_bits,
            } => {
                let offset = state.cursor;
                let Some(&byte) = prefix.get(offset) else {
                    return Ok(None);
                };
                if byte & reserved_mask != 0 || byte & constant_bits != constant_bits {
                    return Err(ProtocolError::InvalidRequestPackedBits { offset });
                }
                for field in fields {
                    if field.slot >= MAX_REQUEST_FRAME_STATE_SLOTS {
                        return Err(ProtocolError::InvalidFieldSequence(
                            "packed selector slot is out of range",
                        ));
                    }
                    let bit = 1u8 << field.slot;
                    if state.packed_present & bit != 0 {
                        return Err(ProtocolError::InvalidFieldSequence(
                            "packed selector slot is assigned more than once",
                        ));
                    }
                    state.packed_values[field.slot] = byte & field.mask;
                    state.packed_present |= bit;
                    let bits = byte & field.mask;
                    let value = field
                        .values
                        .iter()
                        .find(|value| value.bits == bits)
                        .ok_or(ProtocolError::InvalidRequestPackedBits { offset })?;
                    project_request_field::<PROJECT_FIELDS>(
                        projections,
                        field.field,
                        RequestFieldProjection::Static(value.bytes),
                    )?;
                }
                state.cursor = state
                    .cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::Conditional {
                selector,
                expected,
                steps,
            } => {
                if selector >= MAX_REQUEST_FRAME_STATE_SLOTS
                    || state.packed_present & (1u8 << selector) == 0
                {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "conditional step references an unavailable packed selector",
                    ));
                }
                if state.packed_values[selector] == expected
                    && decode_request_frame_steps::<PROJECT_FIELDS>(
                        prefix,
                        steps,
                        state,
                        projections,
                    )?
                    .is_none()
                {
                    return Ok(None);
                }
            }
            RequestFrameStep::Constant { bytes } => {
                let end = state
                    .cursor
                    .checked_add(bytes.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let Some(actual) = prefix.get(state.cursor..end) else {
                    return Ok(None);
                };
                if actual != bytes {
                    return Err(ProtocolError::RequestConstantMismatch {
                        offset: state.cursor,
                    });
                }
                state.cursor = end;
            }
            RequestFrameStep::ConditionalVarUInt {
                selector_offset,
                mask,
                expected,
            } => {
                let Some(&selector) = prefix.get(selector_offset) else {
                    return Ok(None);
                };
                if selector & mask == expected {
                    let Some((_, encoded_len)) = crate::decode_varuint(
                        prefix.get(state.cursor..).unwrap_or_default(),
                        "request conditional integer",
                    )?
                    else {
                        return Ok(None);
                    };
                    state.cursor = state
                        .cursor
                        .checked_add(encoded_len)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                }
            }
            RequestFrameStep::ByteLength => {
                let Some(&length) = prefix.get(state.cursor) else {
                    return Ok(None);
                };
                let length = usize::from(length);
                let end = state
                    .cursor
                    .checked_add(1)
                    .and_then(|end| end.checked_add(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                state.cursor = end;
            }
            RequestFrameStep::ByteLengthField { field } => {
                let Some(&length) = prefix.get(state.cursor) else {
                    return Ok(None);
                };
                let start = state
                    .cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let end = start
                    .checked_add(usize::from(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                project_request_field::<PROJECT_FIELDS>(
                    projections,
                    field,
                    RequestFieldProjection::Borrowed { start, end },
                )?;
                state.cursor = end;
            }
            RequestFrameStep::ByteLengthPrefix { slot } => {
                let Some(&length) = prefix.get(state.cursor) else {
                    return Ok(None);
                };
                let stored =
                    state
                        .byte_lengths
                        .get_mut(slot)
                        .ok_or(ProtocolError::InvalidFieldSequence(
                            "byte-length slot is out of range",
                        ))?;
                if *stored != EMPTY_BYTE_LENGTH {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "byte-length slot is assigned more than once",
                    ));
                }
                *stored = u16::from(length);
                state.cursor = state
                    .cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::ByteLengthBody { slot }
            | RequestFrameStep::ByteLengthBodyField { slot, .. } => {
                let stored =
                    state
                        .byte_lengths
                        .get_mut(slot)
                        .ok_or(ProtocolError::InvalidFieldSequence(
                            "byte-length slot is out of range",
                        ))?;
                if *stored == EMPTY_BYTE_LENGTH {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "byte-length body has no preceding prefix",
                    ));
                }
                let length = usize::from(*stored);
                *stored = EMPTY_BYTE_LENGTH;
                let start = state.cursor;
                let end = start
                    .checked_add(length)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                if let RequestFrameStep::ByteLengthBodyField { field, .. } = *step {
                    project_request_field::<PROJECT_FIELDS>(
                        projections,
                        field,
                        RequestFieldProjection::Borrowed { start, end },
                    )?;
                }
                state.cursor = end;
            }
            RequestFrameStep::ByteThenVarUInt {
                prefix_bytes,
                mask,
                expected,
            } => {
                let Some(encoded_len) = decode_byte_then_varuint_len(
                    &prefix[state.cursor..],
                    prefix_bytes,
                    mask,
                    expected,
                )?
                else {
                    return Ok(None);
                };
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::ConditionalByteThenVarUInt {
                selector_offset,
                mask,
                expected,
                prefix_bytes,
                value_mask,
                value_expected,
            } => {
                let Some(&selector) = prefix.get(selector_offset) else {
                    return Ok(None);
                };
                if selector & mask == expected {
                    let Some(encoded_len) = decode_byte_then_varuint_len(
                        &prefix[state.cursor..],
                        prefix_bytes,
                        value_mask,
                        value_expected,
                    )?
                    else {
                        return Ok(None);
                    };
                    state.cursor = state
                        .cursor
                        .checked_add(encoded_len)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                }
            }
        }
    }
    Ok(Some(()))
}

fn project_request_field<const PROJECT_FIELDS: bool>(
    projections: &mut [RequestFieldProjection],
    field: usize,
    projection: RequestFieldProjection,
) -> Result<()> {
    if PROJECT_FIELDS {
        set_request_field_projection(projections, field, projection)
    } else {
        Ok(())
    }
}

fn set_request_field_projection(
    projections: &mut [RequestFieldProjection],
    field: usize,
    projection: RequestFieldProjection,
) -> Result<()> {
    let target = projections
        .get_mut(field)
        .ok_or(ProtocolError::InvalidFieldSequence(
            "request field projection is out of range",
        ))?;
    if *target != RequestFieldProjection::Missing {
        return Err(ProtocolError::InvalidFieldSequence(
            "request field is projected more than once",
        ));
    }
    *target = projection;
    Ok(())
}

fn decode_byte_then_varuint_len(
    input: &[u8],
    prefix_bytes: usize,
    mask: u8,
    expected: u8,
) -> Result<Option<usize>> {
    if input.len() < prefix_bytes {
        return Ok(None);
    }
    let Some(&flags) = input.first() else {
        return Ok(None);
    };
    if flags & mask != expected {
        return Ok(Some(prefix_bytes));
    }
    let Some((_, value_len)) = crate::decode_varuint(
        input.get(prefix_bytes..).unwrap_or_default(),
        "request conditional integer",
    )?
    else {
        return Ok(None);
    };
    Ok(Some(
        prefix_bytes
            .checked_add(value_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?,
    ))
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
