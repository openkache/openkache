//! Operation-neutral request frame delimiting.
//!
//! An API supplies a request layout to this module. This module only consumes
//! byte steps; it does not know whether a fixed field is a namespace, item ID,
//! policy flag, or any other domain value.

use crate::{MAX_PAYLOAD_BYTES, ProtocolError, REQUEST_CODE_BYTES, Result};

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
    /// This is the generic fixed-body counterpart to [`PayloadLength`].  It is
    /// selected by an API for a dense required tuple, so a future API can use
    /// compact fixed framing without a protocol-specific parser branch. It
    /// MUST be the final step in a layout.
    FixedBody { bytes: usize },
    /// Consume one canonical `vu128` value and treat its value as the opaque
    /// payload length.
    PayloadLength,
    /// Consume a conditional canonical `vu128`, selected by a previously
    /// decoded byte.
    ConditionalVarUInt {
        selector_offset: usize,
        mask: u8,
        expected: u8,
    },
    /// Consume one byte length followed by that many bytes.
    ByteLength,
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
    /// Ordered byte-consumption steps for this operation.
    pub steps: &'static [RequestFrameStep],
}

/// Header metadata required to delimit one opaque request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrameHeader {
    code: u8,
    encoded_len: usize,
    payload_len: usize,
}

impl RequestFrameHeader {
    /// Returns the opaque request code.
    pub const fn code(self) -> u8 {
        self.code
    }

    /// Returns the number of bytes before the opaque body.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the payload length carried by the selected layout.
    pub const fn payload_len(self) -> usize {
        self.payload_len
    }

    /// Returns the complete frame length.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.payload_len)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A complete request viewed as an opaque operation call.
///
/// The parser owns only frame delimiting. An API client or server adapter
/// may inspect [`payload`](Self::payload) after it has selected the operation's
/// modeled request shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpaqueRequestFrame<'a> {
    code: u8,
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
            expected: REQUEST_CODE_BYTES,
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
            code: header.code,
            frame,
            // `encoded_len` includes every prefix step, including a
            // variable-length body prefix. Keeping the offset from the
            // decoded header prevents callers from accidentally exposing a
            // length varuint as part of the opaque operation body.
            body_offset: header.encoded_len,
        })
    }

    /// Returns the opaque request code.
    pub const fn code(self) -> u8 {
        self.code
    }

    /// Returns the opaque request payload after the code and framing prefix.
    pub fn payload(self) -> &'a [u8] {
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
    let Some(&code_byte) = prefix.first() else {
        return Ok(None);
    };
    if REQUEST_CODE_BYTES != 1 {
        return Err(ProtocolError::InvalidFrameLayout(
            "opaque request codes wider than one byte are not supported by this v1 parser",
        ));
    }
    let code = code_byte;
    let mut cursor: usize = REQUEST_CODE_BYTES;
    let mut payload_len = 0;
    for (step_index, step) in layout.steps.iter().enumerate() {
        match *step {
            RequestFrameStep::Fixed { bytes } => {
                let end = cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                cursor = end;
            }
            RequestFrameStep::FixedBody { bytes } => {
                if step_index + 1 != layout.steps.len() {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "fixed-body frame step must be last",
                    ));
                }
                validate_payload_length(bytes)?;
                payload_len = bytes;
                let body_end = cursor
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < body_end {
                    return Ok(None);
                }
            }
            RequestFrameStep::PayloadLength => {
                if step_index + 1 != layout.steps.len() {
                    return Err(ProtocolError::InvalidFrameLayout(
                        "payload-length frame step must be last",
                    ));
                }
                let Some((length, encoded_len)) =
                    crate::decode_varuint(&prefix[cursor..], "request payload length")?
                else {
                    return Ok(None);
                };
                payload_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                validate_payload_length(payload_len)?;
                cursor = cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
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
                    let Some((_, encoded_len)) =
                        crate::decode_varuint(&prefix[cursor..], "request conditional integer")?
                    else {
                        return Ok(None);
                    };
                    cursor = cursor
                        .checked_add(encoded_len)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                }
            }
            RequestFrameStep::ByteLength => {
                if step_index + 1 != layout.steps.len() {
                    return Err(ProtocolError::InvalidFrameLayout(
                        "byte-length frame step must be last",
                    ));
                }
                let Some(&length) = prefix.get(cursor) else {
                    return Ok(None);
                };
                let length = usize::from(length);
                validate_payload_length(length)?;
                let body_start = cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let end = body_start
                    .checked_add(length)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                cursor = body_start;
                payload_len = length;
            }
            RequestFrameStep::ByteThenVarUInt {
                prefix_bytes,
                mask,
                expected,
            } => {
                let Some(encoded_len) =
                    decode_byte_then_varuint_len(&prefix[cursor..], prefix_bytes, mask, expected)?
                else {
                    return Ok(None);
                };
                cursor = cursor
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
                        &prefix[cursor..],
                        prefix_bytes,
                        value_mask,
                        value_expected,
                    )?
                    else {
                        return Ok(None);
                    };
                    cursor = cursor
                        .checked_add(encoded_len)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                }
            }
        }
    }
    Ok(Some(RequestFrameHeader {
        code,
        encoded_len: cursor,
        payload_len,
    }))
}

fn decode_byte_then_varuint_len(
    input: &[u8],
    prefix_bytes: usize,
    mask: u8,
    expected: u8,
) -> Result<Option<usize>> {
    if prefix_bytes == 0 {
        return Err(ProtocolError::InvalidFrameLayout(
            "conditional byte prefix must contain at least one byte",
        ));
    }
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

fn validate_payload_length(value_len: usize) -> Result<()> {
    if value_len > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_PAYLOAD_BYTES,
        });
    }
    Ok(())
}
