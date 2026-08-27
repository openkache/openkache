//! Operation-neutral request frame encoding.

use smallvec::SmallVec;

use crate::protocol::request::{
    MAX_REQUEST_FRAME_STATE_SLOTS, RequestFrameLayout, RequestFrameStep,
};
use crate::protocol::segments::RequestPrefix;
use crate::protocol::{
    MAX_OPERATION_REQUEST_FIELDS, MAX_REQUEST_FRAME_BYTES, OPCODE_BYTES, Opcode, OwnedRequestFrame,
    ProtocolError, Result, WireSegment,
};

const NO_REQUEST_FIELD: usize = usize::MAX;

/// Encodes canonical numeric fields through one generated request layout.
///
/// Field positions are the generated numeric indexes in the layout. Fixed and
/// scalar fields use their canonical codec bytes; retained byte owners are
/// moved into the returned frame only after the complete active plan passes
/// validation.
///
/// # Arguments
///
/// * `opcode` - Operation discriminator written as the first frame byte.
/// * `layout` - Generated operation-neutral compact request layout.
/// * `fields` - Canonical field owners in generated numeric-index order.
///
/// # Returns
///
/// An ordered frame that keeps common prefixes inline and retains large field
/// owners without copying their bytes.
///
/// # Errors
///
/// Returns an error when field cardinality, presence, width, packed mapping,
/// scalar encoding, or the generated layout is invalid, or when the complete
/// frame exceeds its generated bound.
pub fn encode_request_frame(
    opcode: Opcode,
    layout: RequestFrameLayout,
    fields: impl IntoIterator<Item = Option<WireSegment>>,
) -> Result<OwnedRequestFrame> {
    encode_request_frame_with_id(0, opcode, layout, fields)
}

/// Encodes a request with an explicit canonical request ID.
///
/// The request ID is emitted immediately after the fixed-width opcode and is
/// retained by the server for response correlation. The legacy
/// [`encode_request_frame`] entry point uses the valid zero token.
pub fn encode_request_frame_with_id(
    request_id: u64,
    opcode: Opcode,
    layout: RequestFrameLayout,
    fields: impl IntoIterator<Item = Option<WireSegment>>,
) -> Result<OwnedRequestFrame> {
    if layout.field_count > MAX_OPERATION_REQUEST_FIELDS {
        return Err(invalid_layout(
            "request field count exceeds the generated bound",
        ));
    }
    let mut fields: SmallVec<[Option<WireSegment>; 8]> = fields
        .into_iter()
        .take(layout.field_count.saturating_add(1))
        .collect();
    if fields.len() != layout.field_count {
        return Err(invalid_layout(
            "request fields do not match the generated layout",
        ));
    }

    let mut plan = EncodePlan::new(layout.field_count, request_id);
    plan.visit(layout.steps, &fields)?;
    plan.finish(&fields)?;

    let mut output = FrameOutput::new(plan.prefix_len);
    output.append_inline(&[opcode as u8])?;
    let (request_id_bytes, request_id_len) = crate::protocol::encode_varuint(request_id);
    output.append_inline(&request_id_bytes[..request_id_len])?;
    for piece in plan.pieces {
        match piece {
            FramePiece::Static(bytes) => output.append_inline(bytes)?,
            FramePiece::Encoded { bytes, len } => {
                output.append_inline(&bytes[..usize::from(len)])?;
            }
            FramePiece::Field(field) => {
                output.append_field(
                    fields[field]
                        .take()
                        .expect("request field passed preflight"),
                )?;
            }
        }
    }
    let frame = output.finish()?;
    debug_assert_eq!(frame.len(), plan.encoded_len);
    Ok(frame)
}

enum FramePiece {
    Static(&'static [u8]),
    Encoded {
        bytes: [u8; crate::protocol::MAX_VARUINT_BYTES],
        len: u8,
    },
    Field(usize),
}

struct EncodePlan {
    encoded_len: usize,
    prefix_len: usize,
    prefix_open: bool,
    pieces: SmallVec<[FramePiece; 8]>,
    used_fields: SmallVec<[bool; 8]>,
    selectors: Selectors,
    byte_length_fields: [usize; MAX_REQUEST_FRAME_STATE_SLOTS],
    value_field: usize,
    terminal_body: bool,
}

impl EncodePlan {
    fn new(field_count: usize, request_id: u64) -> Self {
        let request_id_len = crate::protocol::encode_varuint(request_id).1;
        Self {
            encoded_len: OPCODE_BYTES + request_id_len,
            prefix_len: OPCODE_BYTES + request_id_len,
            prefix_open: true,
            pieces: SmallVec::new(),
            used_fields: SmallVec::from_elem(false, field_count),
            selectors: Selectors::new(),
            byte_length_fields: [NO_REQUEST_FIELD; MAX_REQUEST_FRAME_STATE_SLOTS],
            value_field: NO_REQUEST_FIELD,
            terminal_body: false,
        }
    }

    fn visit(&mut self, steps: &[RequestFrameStep], fields: &[Option<WireSegment>]) -> Result<()> {
        for step in steps {
            if self.terminal_body {
                return Err(invalid_layout("request body must be the final frame step"));
            }
            match *step {
                RequestFrameStep::FixedField { field, bytes } => {
                    let value = self.use_field(fields, field)?;
                    if value.len() != bytes {
                        return Err(invalid_layout("fixed request field has the wrong width"));
                    }
                    self.push_field(field, value)?;
                }
                RequestFrameStep::Packed {
                    fields: packed_fields,
                    reserved_mask,
                    constant_bits,
                } => {
                    if constant_bits & reserved_mask != 0 {
                        return Err(invalid_layout("packed constant bits overlap reserved bits"));
                    }
                    let mut assigned_mask = reserved_mask | constant_bits;
                    let mut encoded = constant_bits;
                    for packed in packed_fields {
                        if assigned_mask & packed.mask != 0 {
                            return Err(invalid_layout("packed request field masks overlap"));
                        }
                        assigned_mask |= packed.mask;
                        let value = self.use_field(fields, packed.field)?;
                        let mut selected = None;
                        for (index, candidate) in packed.values.iter().enumerate() {
                            if candidate.bits & !packed.mask != 0 {
                                return Err(invalid_layout(
                                    "packed request value exceeds its field mask",
                                ));
                            }
                            if packed.values[..index]
                                .iter()
                                .any(|previous| previous.bits == candidate.bits)
                            {
                                return Err(invalid_layout(
                                    "packed request field has duplicate wire bits",
                                ));
                            }
                            if packed.values[..index]
                                .iter()
                                .any(|previous| previous.bytes == candidate.bytes)
                            {
                                return Err(invalid_layout(
                                    "packed request field has duplicate canonical bytes",
                                ));
                            }
                            if candidate.bytes == value.as_slice() {
                                selected = Some(candidate);
                            }
                        }
                        let selected = selected.ok_or_else(|| {
                            invalid_layout("packed request field has no canonical mapping")
                        })?;
                        encoded |= selected.bits;
                        self.selectors.assign(packed.slot, selected.bits)?;
                    }
                    self.push_encoded(&[encoded])?;
                }
                RequestFrameStep::Conditional {
                    selector,
                    expected,
                    steps,
                } => {
                    if self.selectors.get(selector)? == expected {
                        self.visit(steps, fields)?;
                    }
                }
                RequestFrameStep::Constant { bytes } => {
                    self.add_inline_len(bytes.len())?;
                    self.pieces.push(FramePiece::Static(bytes));
                }
                RequestFrameStep::ByteLengthField { field } => {
                    let value = self.use_field(fields, field)?;
                    if value.len() > usize::from(u8::MAX) {
                        return Err(invalid_layout(
                            "byte-length request field exceeds 255 bytes",
                        ));
                    }
                    self.push_encoded(&[u8::try_from(value.len()).expect("length was validated")])?;
                    self.push_field(field, value)?;
                }
                RequestFrameStep::ByteLengthPrefix { slot, field } => {
                    let value = self.use_field(fields, field)?;
                    if value.len() > usize::from(u8::MAX) {
                        return Err(invalid_layout(
                            "byte-length request field exceeds 255 bytes",
                        ));
                    }
                    let stored = self
                        .byte_length_fields
                        .get_mut(slot)
                        .ok_or_else(|| invalid_layout("byte-length slot is out of range"))?;
                    if *stored != NO_REQUEST_FIELD {
                        return Err(invalid_layout(
                            "byte-length slot is assigned more than once",
                        ));
                    }
                    *stored = field;
                    self.push_encoded(&[u8::try_from(value.len()).expect("length was validated")])?;
                }
                RequestFrameStep::ByteLengthBodyField { slot, field } => {
                    let stored = self
                        .byte_length_fields
                        .get_mut(slot)
                        .ok_or_else(|| invalid_layout("byte-length slot is out of range"))?;
                    if *stored != field {
                        return Err(invalid_layout(
                            "byte-length body does not match its prefix field",
                        ));
                    }
                    *stored = NO_REQUEST_FIELD;
                    self.push_field(field, self.field(fields, field)?)?;
                }
                RequestFrameStep::VarUIntField { field } => {
                    let value = self.use_field(fields, field)?;
                    let (encoded, len) = crate::protocol::encode_varuint(canonical_u64(value)?);
                    self.push_encoded(&encoded[..len])?;
                }
                RequestFrameStep::TrailingField { field }
                | RequestFrameStep::ValueLengthPrefixField { field } => {
                    if self.value_field != NO_REQUEST_FIELD {
                        return Err(invalid_layout(
                            "request layout declares more than one value body",
                        ));
                    }
                    let value = self.use_field(fields, field)?;
                    validate_value_length(value.len())?;
                    self.value_field = field;
                    let (encoded, len) = crate::protocol::encode_varuint(value.len() as u64);
                    self.push_encoded(&encoded[..len])?;
                    self.terminal_body = matches!(*step, RequestFrameStep::TrailingField { .. });
                }
                RequestFrameStep::Fixed { .. }
                | RequestFrameStep::FixedBody { .. }
                | RequestFrameStep::ValueLength
                | RequestFrameStep::ValueLengthPrefix
                | RequestFrameStep::VarUInt
                | RequestFrameStep::ByteLength
                | RequestFrameStep::ByteLengthBody { .. } => {
                    return Err(invalid_layout(
                        "request encoder requires field-addressable dynamic steps",
                    ));
                }
            }
        }
        Ok(())
    }

    fn use_field<'a>(
        &mut self,
        fields: &'a [Option<WireSegment>],
        field: usize,
    ) -> Result<&'a WireSegment> {
        let used = self
            .used_fields
            .get_mut(field)
            .ok_or_else(|| invalid_layout("request field index is out of range"))?;
        if *used {
            return Err(invalid_layout("request field is encoded more than once"));
        }
        let value = fields[field]
            .as_ref()
            .ok_or_else(|| invalid_layout("active request field is missing"))?;
        *used = true;
        Ok(value)
    }

    fn field<'a>(
        &self,
        fields: &'a [Option<WireSegment>],
        field: usize,
    ) -> Result<&'a WireSegment> {
        fields
            .get(field)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid_layout("request field index is out of range"))
    }

    fn push_field(&mut self, field: usize, value: &WireSegment) -> Result<()> {
        self.add_len(value.len())?;
        if self.prefix_open && !value.is_empty() {
            match value {
                WireSegment::Inline(_) => {
                    self.add_prefix_len(value.len())?;
                }
                WireSegment::Owned(_) | WireSegment::External(_) => {
                    self.prefix_open = false;
                }
            }
        }
        self.pieces.push(FramePiece::Field(field));
        Ok(())
    }

    fn push_encoded(&mut self, value: &[u8]) -> Result<()> {
        let mut bytes = [0; crate::protocol::MAX_VARUINT_BYTES];
        bytes[..value.len()].copy_from_slice(value);
        self.add_inline_len(value.len())?;
        self.pieces.push(FramePiece::Encoded {
            bytes,
            len: u8::try_from(value.len()).expect("encoded request primitive length fits in u8"),
        });
        Ok(())
    }

    fn add_inline_len(&mut self, bytes: usize) -> Result<()> {
        self.add_len(bytes)?;
        if self.prefix_open {
            self.add_prefix_len(bytes)?;
        }
        Ok(())
    }

    fn add_len(&mut self, bytes: usize) -> Result<()> {
        self.encoded_len = self
            .encoded_len
            .checked_add(bytes)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        Ok(())
    }

    fn add_prefix_len(&mut self, bytes: usize) -> Result<()> {
        self.prefix_len = self
            .prefix_len
            .checked_add(bytes)
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        Ok(())
    }

    fn finish(&mut self, fields: &[Option<WireSegment>]) -> Result<()> {
        if self
            .byte_length_fields
            .iter()
            .any(|field| *field != NO_REQUEST_FIELD)
        {
            return Err(invalid_layout(
                "byte-length prefix has no matching body step",
            ));
        }
        if fields
            .iter()
            .zip(&self.used_fields)
            .any(|(field, used)| field.is_some() && !used)
        {
            return Err(invalid_layout(
                "request contains a field outside the active layout",
            ));
        }
        if self.value_field != NO_REQUEST_FIELD {
            self.push_field(self.value_field, self.field(fields, self.value_field)?)?;
        }
        if self.encoded_len > MAX_REQUEST_FRAME_BYTES {
            return Err(ProtocolError::ValueTooLarge {
                size: self.encoded_len,
                maximum: MAX_REQUEST_FRAME_BYTES,
            });
        }
        Ok(())
    }
}

struct Selectors {
    values: [u8; MAX_REQUEST_FRAME_STATE_SLOTS],
    present: u8,
}

impl Selectors {
    const fn new() -> Self {
        Self {
            values: [0; MAX_REQUEST_FRAME_STATE_SLOTS],
            present: 0,
        }
    }

    fn assign(&mut self, slot: usize, value: u8) -> Result<()> {
        if slot >= MAX_REQUEST_FRAME_STATE_SLOTS || self.present & (1u8 << slot) != 0 {
            return Err(invalid_layout(
                "packed selector slot is invalid or assigned more than once",
            ));
        }
        self.values[slot] = value;
        self.present |= 1u8 << slot;
        Ok(())
    }

    fn get(&self, slot: usize) -> Result<u8> {
        if slot >= MAX_REQUEST_FRAME_STATE_SLOTS || self.present & (1u8 << slot) == 0 {
            return Err(invalid_layout(
                "conditional references an unavailable packed selector",
            ));
        }
        Ok(self.values[slot])
    }
}

struct FrameOutput {
    prefix: RequestPrefix,
    suffix: SmallVec<[WireSegment; 2]>,
    inline_suffix: SmallVec<[u8; 32]>,
    retained_owner_seen: bool,
}

impl FrameOutput {
    fn new(prefix_len: usize) -> Self {
        Self {
            prefix: RequestPrefix::with_capacity(prefix_len),
            suffix: SmallVec::new(),
            inline_suffix: SmallVec::new(),
            retained_owner_seen: false,
        }
    }

    fn append_inline(&mut self, bytes: &[u8]) -> Result<()> {
        if self.retained_owner_seen {
            self.inline_suffix.extend_from_slice(bytes);
            Ok(())
        } else {
            self.prefix.try_extend_from_slice(bytes)
        }
    }

    fn append_field(&mut self, field: WireSegment) -> Result<()> {
        if field.is_empty() {
            return Ok(());
        }
        match field {
            WireSegment::Inline(bytes) => self.append_inline(&bytes),
            retained @ (WireSegment::Owned(_) | WireSegment::External(_)) => {
                self.flush_inline_suffix();
                self.retained_owner_seen = true;
                self.suffix.push(retained);
                Ok(())
            }
        }
    }

    fn flush_inline_suffix(&mut self) {
        if !self.inline_suffix.is_empty() {
            self.suffix
                .push(WireSegment::Inline(std::mem::take(&mut self.inline_suffix)));
        }
    }

    fn finish(mut self) -> Result<OwnedRequestFrame> {
        self.flush_inline_suffix();
        OwnedRequestFrame::from_parts(self.prefix, self.suffix)
    }
}

fn canonical_u64(value: &WireSegment) -> Result<u64> {
    let bytes: [u8; 8] = value
        .as_slice()
        .try_into()
        .map_err(|_| invalid_layout("varuint request field must be canonical u64 bytes"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > crate::protocol::MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: crate::protocol::MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

const fn invalid_layout(message: &'static str) -> ProtocolError {
    ProtocolError::InvalidFieldSequence(message)
}
