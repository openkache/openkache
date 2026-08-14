//! Operation-neutral request frame delimiting.
//!
//! An API supplies a request layout to this module. This module only consumes
//! byte steps; it does not know whether a fixed field is a namespace, item ID,
//! policy flag, or any other domain value.

use crate::{MAX_VALUE_BYTES, OPCODE_BYTES, Opcode, ProtocolError, Result};

const MAX_REQUEST_FRAME_STATE_SLOTS: usize = 8;
const EMPTY_BYTE_LENGTH: u16 = u16::MAX;

/// One field projected from a packed request byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFramePackedField {
    /// Bounded state slot retained for a later conditional step.
    pub slot: usize,
    /// Bits belonging to this modeled field.
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
    /// Consume one canonical `vu128` metadata value.
    VarUInt,
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
    /// Consume one byte length and retain it for a later body step.
    ByteLengthPrefix { slot: usize },
    /// Consume the body declared by a preceding byte-length prefix.
    ByteLengthBody { slot: usize },
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
    /// repeated as a `Fixed` step. Conditional selector offsets are absolute
    /// offsets in the complete frame, including the opcode.
    pub steps: &'static [RequestFrameStep],
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
    if prefix.len() < OPCODE_BYTES {
        return Ok(None);
    }
    let opcode_byte = prefix[0];
    let opcode = Opcode::try_from(opcode_byte)?;
    let mut state = RequestFrameDecodeState::new();
    if decode_request_frame_steps(prefix, layout.steps, &mut state)?.is_none() {
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
    Ok(Some(RequestFrameHeader {
        opcode,
        encoded_len: state.cursor,
        value_len: state.value_len,
    }))
}

struct RequestFrameDecodeState {
    cursor: usize,
    value_len: usize,
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
            value_length_seen: false,
            terminal_body: false,
            packed_values: [0; MAX_REQUEST_FRAME_STATE_SLOTS],
            packed_present: 0,
            byte_lengths: [EMPTY_BYTE_LENGTH; MAX_REQUEST_FRAME_STATE_SLOTS],
        }
    }

    fn set_value_length(&mut self, value_len: usize, terminal: bool) -> Result<()> {
        if self.value_length_seen {
            return Err(ProtocolError::InvalidFieldSequence(
                "request layout declares more than one value body",
            ));
        }
        validate_value_length(value_len)?;
        self.value_len = value_len;
        self.value_length_seen = true;
        self.terminal_body = terminal;
        Ok(())
    }
}

fn decode_request_frame_steps(
    prefix: &[u8],
    steps: &[RequestFrameStep],
    state: &mut RequestFrameDecodeState,
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
            RequestFrameStep::FixedBody { bytes } => {
                state.set_value_length(bytes, true)?;
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
                state.set_value_length(value_len, true)?;
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
                state.set_value_length(value_len, false)?;
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
                    && decode_request_frame_steps(prefix, steps, state)?.is_none()
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
            RequestFrameStep::ByteLengthPrefix { slot } => {
                let Some(&length) = prefix.get(state.cursor) else {
                    return Ok(None);
                };
                let stored = state.byte_lengths.get_mut(slot).ok_or(
                    ProtocolError::InvalidFieldSequence("byte-length slot is out of range"),
                )?;
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
            RequestFrameStep::ByteLengthBody { slot } => {
                let stored = state.byte_lengths.get_mut(slot).ok_or(
                    ProtocolError::InvalidFieldSequence("byte-length slot is out of range"),
                )?;
                if *stored == EMPTY_BYTE_LENGTH {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "byte-length body has no preceding prefix",
                    ));
                }
                let length = usize::from(*stored);
                *stored = EMPTY_BYTE_LENGTH;
                let end = state
                    .cursor
                    .checked_add(length)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                state.cursor = end;
            }
            RequestFrameStep::ByteThenVarUInt {
                prefix_bytes,
                mask,
                expected,
            } => {
                let Some(encoded_len) =
                    decode_byte_then_varuint_len(
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
