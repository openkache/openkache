//! Operation-neutral request frame delimiting.
//!
//! An API supplies a request layout to this module. This module only consumes
//! byte steps; it does not know whether a fixed field is a namespace, item ID,
//! policy flag, or any other domain value.

use crate::{MAX_VALUE_BYTES, OPCODE_BYTES, Opcode, ProtocolError, Result};

pub(super) const MAX_REQUEST_FRAME_STATE_SLOTS: usize = 8;
const EMPTY_BYTE_LENGTH: u16 = u16::MAX;
const NO_REQUEST_FIELD: u32 = u32::MAX;

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
    /// Consume one byte length followed by that many bytes.
    ByteLength,
    /// Consume one byte length followed by a projected modeled field.
    ByteLengthField { field: usize },
    /// Consume one modeled field's byte length and retain it for a later body
    /// step.
    ByteLengthPrefix { slot: usize, field: usize },
    /// Consume the body declared by a preceding byte-length prefix.
    ByteLengthBody { slot: usize },
    /// Consume and project the modeled field declared by a preceding
    /// byte-length prefix.
    ByteLengthBodyField { slot: usize, field: usize },
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
    encoded_len: usize,
    body_len: usize,
    opcode: Opcode,
    request_id: u64,
    body_field: u32,
}

impl RequestFrameHeader {
    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the client-selected correlation token carried by this frame.
    pub const fn request_id(self) -> u64 {
        self.request_id
    }

    /// Returns the number of bytes before the opaque body.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the opaque body length carried by the layout.
    pub const fn body_len(self) -> usize {
        self.body_len
    }

    /// Returns the numeric modeled field represented by the opaque body.
    ///
    /// Generic opaque and fixed bodies are not associated with a modeled
    /// field and return `None`.
    pub const fn body_field(self) -> Option<usize> {
        if self.body_field == NO_REQUEST_FIELD {
            None
        } else {
            Some(self.body_field as usize)
        }
    }

    /// Returns the complete frame length.
    pub fn frame_len(self) -> Result<usize> {
        self.encoded_len
            .checked_add(self.body_len)
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
    request_id: u64,
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

    /// Returns the exact additional prefix bytes needed to complete the next
    /// unresolved header step.
    ///
    /// Zero means that the header is complete. Once the first byte of a
    /// canonical `vu128` is available, the returned count includes all of its
    /// remaining bytes instead of advancing one byte at a time. The count
    /// never includes an opaque terminal body declared by the header.
    pub fn header_bytes_needed(prefix: &[u8], layout: RequestFrameLayout) -> Result<usize> {
        match decode_request_frame_metadata_progress::<false>(prefix, layout, &mut [])? {
            DecodeProgress::Complete(_) => Ok(0),
            DecodeProgress::Need(additional) => Ok(additional),
        }
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
            request_id: header.request_id,
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

    /// Returns the client-selected correlation token carried by this frame.
    pub const fn request_id(self) -> u64 {
        self.request_id
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
}

/// Projects modeled fields available after decoding only the request header.
///
/// Header-resident fixed, packed, and integer fields use the same canonical
/// projections as complete-frame decoding. A terminal body remains
/// [`RequestFieldProjection::Missing`]; callers can inspect its declared
/// length and numeric field through [`RequestFrameHeader`].
///
/// The output is reset to [`RequestFieldProjection::Missing`] when the header
/// is incomplete or malformed.
pub fn project_request_frame_header(
    prefix: &[u8],
    layout: RequestFrameLayout,
    fields: &mut [RequestFieldProjection],
) -> Result<Option<RequestFrameHeader>> {
    fields.fill(RequestFieldProjection::Missing);
    let result = (|| {
        if fields.len() < layout.field_count {
            return Err(ProtocolError::InvalidFieldSequence(
                "request field projection output is too short",
            ));
        }
        let header = decode_request_frame_metadata::<true>(
            prefix,
            layout,
            &mut fields[..layout.field_count],
        )?;
        if header
            .and_then(RequestFrameHeader::body_field)
            .is_some_and(|field| field >= layout.field_count)
        {
            return Err(ProtocolError::InvalidFieldSequence(
                "request body field projection is out of range",
            ));
        }
        Ok(header)
    })();
    if !matches!(result, Ok(Some(_))) {
        fields.fill(RequestFieldProjection::Missing);
    }
    result
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
        let header = project_request_frame_header(frame, layout, fields)?.ok_or(
            ProtocolError::FrameTooShort {
                expected: OPCODE_BYTES,
                actual: frame.len(),
            },
        )?;
        let expected = header.frame_len()?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        if let Some(body_field) = header.body_field() {
            set_request_field_projection(
                &mut fields[..layout.field_count],
                body_field,
                RequestFieldProjection::Borrowed {
                    start: header.encoded_len,
                    end: expected,
                },
            )?;
        }
        Ok(header)
    })();
    if result.is_err() {
        fields.fill(RequestFieldProjection::Missing);
    }
    result
}

fn decode_request_frame_metadata<const PROJECT_FIELDS: bool>(
    prefix: &[u8],
    layout: RequestFrameLayout,
    projections: &mut [RequestFieldProjection],
) -> Result<Option<RequestFrameHeader>> {
    decode_request_frame_metadata_progress::<PROJECT_FIELDS>(prefix, layout, projections).map(
        |progress| match progress {
            DecodeProgress::Complete(header) => Some(header),
            DecodeProgress::Need(_) => None,
        },
    )
}

fn decode_request_frame_metadata_progress<const PROJECT_FIELDS: bool>(
    prefix: &[u8],
    layout: RequestFrameLayout,
    projections: &mut [RequestFieldProjection],
) -> Result<DecodeProgress<RequestFrameHeader>> {
    if prefix.len() < OPCODE_BYTES {
        return Ok(DecodeProgress::Need(OPCODE_BYTES - prefix.len()));
    }
    let opcode_byte = prefix[0];
    let opcode = Opcode::try_from(opcode_byte)?;
    let Some((request_id, request_id_len)) =
        crate::decode_varuint(prefix.get(OPCODE_BYTES..).unwrap_or_default(), "request ID")?
    else {
        let end = incomplete_varuint_end(prefix, OPCODE_BYTES)?;
        return Ok(DecodeProgress::Need(end.checked_sub(prefix.len()).ok_or(
            ProtocolError::InvalidFieldSequence("incomplete request ID did not advance"),
        )?));
    };
    let mut state = RequestFrameDecodeState::new(
        OPCODE_BYTES
            .checked_add(request_id_len)
            .ok_or(ProtocolError::FrameLengthOverflow)?,
    );
    let mut conditional_selectors = [false; MAX_REQUEST_FRAME_STATE_SLOTS];
    collect_conditional_selectors(layout.steps, &mut conditional_selectors)?;
    if let DecodeProgress::Need(additional) = decode_request_frame_steps::<PROJECT_FIELDS>(
        prefix,
        layout.steps,
        &mut state,
        projections,
        &conditional_selectors,
    )?
    {
        return Ok(DecodeProgress::Need(additional));
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
    Ok(DecodeProgress::Complete(RequestFrameHeader {
        encoded_len: state.cursor,
        body_len: state.body_len,
        opcode,
        request_id,
        body_field: state.body_field,
    }))
}

enum DecodeProgress<T> {
    Complete(T),
    Need(usize),
}

struct RequestFrameDecodeState {
    cursor: usize,
    body_len: usize,
    body_field: u32,
    body_length_seen: bool,
    terminal_body: bool,
    packed_values: [u8; MAX_REQUEST_FRAME_STATE_SLOTS],
    packed_present: u8,
    byte_lengths: [u16; MAX_REQUEST_FRAME_STATE_SLOTS],
}

impl RequestFrameDecodeState {
    const fn new(cursor: usize) -> Self {
        Self {
            cursor,
            body_len: 0,
            body_field: NO_REQUEST_FIELD,
            body_length_seen: false,
            terminal_body: false,
            packed_values: [0; MAX_REQUEST_FRAME_STATE_SLOTS],
            packed_present: 0,
            byte_lengths: [EMPTY_BYTE_LENGTH; MAX_REQUEST_FRAME_STATE_SLOTS],
        }
    }

    fn set_body_length(
        &mut self,
        body_len: usize,
        terminal: bool,
        field: Option<usize>,
    ) -> Result<()> {
        if self.body_length_seen {
            return Err(ProtocolError::InvalidFieldSequence(
                "request layout declares more than one body",
            ));
        }
        validate_body_length(body_len)?;
        self.body_len = body_len;
        self.body_field = match field {
            Some(field) => u32::try_from(field).map_err(|_| {
                ProtocolError::InvalidFieldSequence(
                    "request body field index exceeds header storage",
                )
            })?,
            None => NO_REQUEST_FIELD,
        };
        if self.body_field == NO_REQUEST_FIELD && field.is_some() {
            return Err(ProtocolError::InvalidFieldSequence(
                "request body field index exceeds header storage",
            ));
        }
        self.body_length_seen = true;
        self.terminal_body = terminal;
        Ok(())
    }
}

fn decode_request_frame_steps<const PROJECT_FIELDS: bool>(
    prefix: &[u8],
    steps: &[RequestFrameStep],
    state: &mut RequestFrameDecodeState,
    projections: &mut [RequestFieldProjection],
    conditional_selectors: &[bool; MAX_REQUEST_FRAME_STATE_SLOTS],
) -> Result<DecodeProgress<()>> {
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
                    return incomplete(prefix.len(), end);
                }
                state.cursor = end;
            }
            RequestFrameStep::FixedField { field, bytes } => {
                let start = state.cursor;
                let end = start
                    .checked_add(bytes)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return incomplete(prefix.len(), end);
                }
                project_request_field::<PROJECT_FIELDS>(
                    projections,
                    field,
                    RequestFieldProjection::Borrowed { start, end },
                )?;
                state.cursor = end;
            }
            RequestFrameStep::FixedBody { bytes } => {
                state.set_body_length(bytes, true, None)?;
            }
            RequestFrameStep::ValueLength => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request body length",
                )?
                else {
                    let end = incomplete_varuint_end(prefix, state.cursor)?;
                    return incomplete(prefix.len(), end);
                };
                let body_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_body_length(body_len, true, None)?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::ValueLengthPrefix => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request body length",
                )?
                else {
                    let end = incomplete_varuint_end(prefix, state.cursor)?;
                    return incomplete(prefix.len(), end);
                };
                let body_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_body_length(body_len, false, None)?;
                state.cursor = state
                    .cursor
                    .checked_add(encoded_len)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
            }
            RequestFrameStep::TrailingField { field }
            | RequestFrameStep::ValueLengthPrefixField { field } => {
                let Some((length, encoded_len)) = crate::decode_varuint(
                    prefix.get(state.cursor..).unwrap_or_default(),
                    "request body length",
                )?
                else {
                    let end = incomplete_varuint_end(prefix, state.cursor)?;
                    return incomplete(prefix.len(), end);
                };
                let body_len =
                    usize::try_from(length).map_err(|_| ProtocolError::FrameLengthOverflow)?;
                state.set_body_length(
                    body_len,
                    matches!(*step, RequestFrameStep::TrailingField { .. }),
                    Some(field),
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
                    let end = incomplete_varuint_end(prefix, state.cursor)?;
                    return incomplete(prefix.len(), end);
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
                    let end = incomplete_varuint_end(prefix, state.cursor)?;
                    return incomplete(prefix.len(), end);
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
                    let end = offset
                        .checked_add(1)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                    return incomplete(prefix.len(), end);
                };
                // Reserved and constant-bit violations do not affect frame
                // shape. Keep delimiting the complete frame and project a
                // raw packed-byte marker so the operation codec reports
                // `InvalidRequest` after the boundary is preserved.
                let packed_bits_invalid =
                    byte & reserved_mask != 0 || byte & constant_bits != constant_bits;
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
                    let Some(value) = field.values.iter().find(|value| value.bits == bits) else {
                        // An unknown selector is terminal only when a later
                        // conditional uses it to decide whether another
                        // field exists. For all other packed fields the
                        // complete frame remains delimited and semantic
                        // validation rejects the raw marker.
                        if conditional_selectors[field.slot] {
                            return Err(ProtocolError::InvalidRequestPackedBits { offset });
                        }
                        project_request_field::<PROJECT_FIELDS>(
                            projections,
                            field.field,
                            RequestFieldProjection::Borrowed {
                                start: offset,
                                end: offset
                                    .checked_add(1)
                                    .ok_or(ProtocolError::FrameLengthOverflow)?,
                            },
                        )?;
                        continue;
                    };
                    if packed_bits_invalid {
                        project_request_field::<PROJECT_FIELDS>(
                            projections,
                            field.field,
                            RequestFieldProjection::Borrowed {
                                start: offset,
                                end: offset
                                    .checked_add(1)
                                    .ok_or(ProtocolError::FrameLengthOverflow)?,
                            },
                        )?;
                    } else {
                        project_request_field::<PROJECT_FIELDS>(
                            projections,
                            field.field,
                            RequestFieldProjection::Static(value.bytes),
                        )?;
                    }
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
                    && let DecodeProgress::Need(additional) = decode_request_frame_steps::<
                        PROJECT_FIELDS,
                    >(
                        prefix,
                        steps,
                        state,
                        projections,
                        conditional_selectors,
                    )?
                {
                    return Ok(DecodeProgress::Need(additional));
                }
            }
            RequestFrameStep::Constant { bytes } => {
                let end = state
                    .cursor
                    .checked_add(bytes.len())
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let Some(actual) = prefix.get(state.cursor..end) else {
                    return incomplete(prefix.len(), end);
                };
                if actual != bytes {
                    return Err(ProtocolError::RequestConstantMismatch {
                        offset: state.cursor,
                    });
                }
                state.cursor = end;
            }
            RequestFrameStep::ByteLength => {
                let Some(&length) = prefix.get(state.cursor) else {
                    let end = state
                        .cursor
                        .checked_add(1)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                    return incomplete(prefix.len(), end);
                };
                let length = usize::from(length);
                let end = state
                    .cursor
                    .checked_add(1)
                    .and_then(|end| end.checked_add(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return incomplete(prefix.len(), end);
                }
                state.cursor = end;
            }
            RequestFrameStep::ByteLengthField { field } => {
                let Some(&length) = prefix.get(state.cursor) else {
                    let end = state
                        .cursor
                        .checked_add(1)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                    return incomplete(prefix.len(), end);
                };
                let start = state
                    .cursor
                    .checked_add(1)
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                let end = start
                    .checked_add(usize::from(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return incomplete(prefix.len(), end);
                }
                project_request_field::<PROJECT_FIELDS>(
                    projections,
                    field,
                    RequestFieldProjection::Borrowed { start, end },
                )?;
                state.cursor = end;
            }
            RequestFrameStep::ByteLengthPrefix { slot, .. } => {
                let Some(&length) = prefix.get(state.cursor) else {
                    let end = state
                        .cursor
                        .checked_add(1)
                        .ok_or(ProtocolError::FrameLengthOverflow)?;
                    return incomplete(prefix.len(), end);
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
                    return incomplete(prefix.len(), end);
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
        }
    }
    Ok(DecodeProgress::Complete(()))
}

fn collect_conditional_selectors(
    steps: &[RequestFrameStep],
    selectors: &mut [bool; MAX_REQUEST_FRAME_STATE_SLOTS],
) -> Result<()> {
    for step in steps {
        if let RequestFrameStep::Conditional {
            selector, steps, ..
        } = *step
        {
            let selected = selectors.get_mut(selector).ok_or(
                ProtocolError::InvalidFieldSequence(
                    "conditional step references an unavailable packed selector",
                ),
            )?;
            *selected = true;
            collect_conditional_selectors(steps, selectors)?;
        }
    }
    Ok(())
}

fn incomplete(available: usize, required: usize) -> Result<DecodeProgress<()>> {
    let additional = required
        .checked_sub(available)
        .filter(|additional| *additional > 0)
        .ok_or(ProtocolError::InvalidFieldSequence(
            "incomplete request header did not advance",
        ))?;
    Ok(DecodeProgress::Need(additional))
}

fn incomplete_varuint_end(prefix: &[u8], cursor: usize) -> Result<usize> {
    let encoded_len = prefix
        .get(cursor)
        .map_or(1, |first| vu128::encoded_len(*first));
    cursor
        .checked_add(encoded_len)
        .ok_or(ProtocolError::FrameLengthOverflow)
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

fn validate_body_length(body_len: usize) -> Result<()> {
    if body_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: body_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}
