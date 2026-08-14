//! Server-owned operation handlers.
//!
//! The protocol and client infrastructure only decode a request according to
//! its generated wire contract. This module is the server's decision point:
//! it receives a borrowed operation context and calls the concrete behavior
//! selected by the server. Adding a new operation therefore does not add an
//! operation-name branch to the transport, framing, or client infrastructure.

use std::any::Any;

use openkache_protocol::{Opcode, OwnedRange, decode_planned_fields};
use smallvec::SmallVec;

pub(super) use super::operation_authorization::{
    AuthorizationContext, AuthorizationFn, authorization_administrator, authorization_allowed,
    authorization_none,
};
use super::operation_capabilities::CapabilityCatalog;
pub(super) use super::operation_fields::OperationFieldEnvelope;
use crate::operation_contract as contract;
use crate::protocol::ServerRequest;

pub(super) type RequestDecoder = fn(ServerRequest) -> OperationInputView;

const INLINE_OPERATION_FIELDS: usize = 8;

/// Owns the bytes behind a generic request view.
///
/// Compact compatibility projections store their decoded values directly in
/// [`OperationFieldStorage`]. Generic requests instead keep either their
/// standalone body or the original frame, so one view can borrow fields
/// without maintaining two ownership paths.
enum GenericPayload {
    Owned(Vec<u8>),
    Frame {
        frame: Vec<u8>,
        payload_start: usize,
        payload_end: usize,
    },
    Projection(OwnedFieldProjection),
}

impl GenericPayload {
    fn payload(&self) -> Option<&[u8]> {
        match self {
            Self::Owned(payload) => Some(payload),
            Self::Frame {
                frame,
                payload_start,
                payload_end,
            } => frame.get(*payload_start..*payload_end),
            Self::Projection(_) => None,
        }
    }

    fn owner(&self) -> Option<&[u8]> {
        match self {
            Self::Frame { frame, .. } => Some(frame),
            Self::Projection(projection) => projection.as_slice(),
            Self::Owned(_) => None,
        }
    }
}

/// Owns one buffer behind fields populated by an external projection.
///
/// Numeric ranges are relative to the owner's visible bytes. Ownership can be
/// transferred once without copying or exposing wire framing to handlers.
struct OwnedFieldProjection {
    owner: Option<OwnedRange>,
}

impl OwnedFieldProjection {
    fn new(owner: OwnedRange) -> Self {
        Self { owner: Some(owner) }
    }

    fn as_slice(&self) -> Option<&[u8]> {
        self.owner.as_ref().map(OwnedRange::as_slice)
    }

    fn take(&mut self, start: usize, end: usize) -> Option<OwnedRange> {
        let owner = self.owner.as_ref()?;
        if start > end || end > owner.len() {
            return None;
        }
        let owner = self.owner.take().expect("owner was present");
        let (buffer, owner_range) = owner.into_parts();
        let (Some(absolute_start), Some(absolute_end)) = (
            owner_range.start.checked_add(start),
            owner_range.start.checked_add(end),
        ) else {
            self.owner = OwnedRange::new(buffer, owner_range);
            return None;
        };
        OwnedRange::new(buffer, absolute_start..absolute_end)
    }
}

/// Context passed from the protocol server to one concrete handler.
///
/// The context deliberately contains storage primitives and decoded request
/// fields, rather than exposing transport or frame details to API handlers.
pub(super) struct OperationInputView {
    pub(super) opcode: Opcode,
    plan: &'static [contract::OperationFieldPlan],
    fields: SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]>,
    /// Generic field-sequence or opaque bytes. The offset table keeps fields
    /// borrowed from this one allocation.
    generic_payload: Option<GenericPayload>,
    generic_offsets: SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]>,
    generic_error: Option<&'static str>,
}

/// Decodes an empty, opaque, or ordered request without inspecting any
/// protocol-v1 semantic fields. Compact requests use the API-owned v1 adapter
/// registered alongside their behavior.
pub(super) fn decode_request(request: ServerRequest) -> OperationInputView {
    match request.into_payload_frame() {
        Ok((opcode, frame, start, end)) => {
            OperationInputView::from_owned_frame(opcode, frame, start, end)
        }
        Err(request) => {
            let (opcode, value) = request.into_generic_parts();
            OperationInputView::from_parts(opcode, value, std::iter::empty())
        }
    }
}

/// One generated plan entry and its decoded semantic value.
///
/// Records are ordered exactly like the Smithy request plan. Optional fields
/// remain present with `None`, so a server operation can distinguish a missing
/// optional member from a zero/empty value without an operation-specific
/// context union.
pub(super) struct OperationFieldRecord {
    pub(super) plan: &'static contract::OperationFieldPlan,
    pub(super) value: Option<OperationFieldStorage>,
}

/// Storage owned or borrowed by one operation-field record.
///
/// The generic view carries only bytes. Application payloads stay owned so
/// dispatch can move the existing request allocation without copying;
/// compatibility adapters may use static bytes for canonical tokens.
pub(super) enum OperationFieldStorage {
    OwnedBytes(Vec<u8>),
    InlineBytes { bytes: [u8; 8], len: u8 },
    FrameRange { start: usize, end: usize },
    OwnerRange { start: usize, end: usize },
    StaticBytes(&'static [u8]),
}

/// A borrowed generic field value. All semantic interpretation happens
/// through a generated codec or an API-owned binding.
pub(super) type OperationFieldValue<'a> = &'a [u8];

impl OperationInputView {
    /// Returns the generated operation identity carried by this view.
    pub(super) const fn opcode(&self) -> Opcode {
        self.opcode
    }

    /// Builds a generated field view from a generic request payload.
    ///
    /// Compatibility projections use [`Self::from_populated_parts`] instead,
    /// so this constructor never needs to identify a compatibility route.
    pub(super) fn from_parts<I>(opcode: Opcode, value: Vec<u8>, fields: I) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OperationFieldRecord>>,
    {
        // Keep this fallback on the same generated shape validator as the
        // zero-copy frame path. Besides avoiding two interpretations of the
        // contract, this makes an accidentally non-empty `empty` request
        // fail consistently instead of silently discarding its payload.
        Self::from_generic_payload(
            opcode,
            fields.into_iter().collect(),
            GenericPayload::Owned(value),
        )
    }

    /// Builds a view from fields populated by an external projection.
    ///
    /// The projection owns all framing-specific decoding and supplies only
    /// generated field records. Keeping this constructor separate makes the
    /// generic payload path independent from protocol-v1 route metadata.
    pub(super) fn from_populated_parts<I>(opcode: Opcode, fields: I) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OperationFieldRecord>>,
    {
        let mut fields: SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]> =
            fields.into_iter().collect();
        let plan = contract::spec(opcode).request.fields;
        let generic_error = (plan.len() > contract::MAX_OPERATION_REQUEST_FIELDS)
            .then_some("generated operation request field bound is stale");
        if fields.len() < plan.len() {
            fields.resize_with(plan.len(), || None);
        }
        OperationInputView {
            opcode,
            plan,
            fields,
            generic_payload: None,
            generic_offsets: sentinel_offsets(plan.len()),
            generic_error,
        }
    }

    /// Builds populated numeric fields over one operation-neutral owner.
    ///
    /// Field ranges are relative to the owner's visible bytes. Adapters remain
    /// responsible for decoding framing and populating generated field slots.
    pub(super) fn from_populated_projection<I>(
        opcode: Opcode,
        owner: OwnedRange,
        fields: I,
    ) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OperationFieldRecord>>,
    {
        let mut input = Self::from_populated_parts(opcode, fields);
        input.generic_payload = Some(GenericPayload::Projection(OwnedFieldProjection::new(owner)));
        input
    }

    /// Builds a generic view directly over the server's owned frame.
    ///
    /// The frame remains owned by the view while field offsets borrow from it,
    /// so ordered and opaque requests do not allocate or shift their payload.
    pub(super) fn from_owned_frame(
        opcode: Opcode,
        frame: Vec<u8>,
        payload_start: usize,
        payload_end: usize,
    ) -> OperationInputView {
        Self::from_generic_payload(
            opcode,
            empty_field_records(contract::spec(opcode).request.fields.len()),
            GenericPayload::Frame {
                frame,
                payload_start,
                payload_end,
            },
        )
    }

    fn from_generic_payload(
        opcode: Opcode,
        mut fields: SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]>,
        payload_owner: GenericPayload,
    ) -> OperationInputView {
        let contract = contract::spec(opcode);
        let plan = contract.request.fields;
        if fields.len() < plan.len() {
            fields.resize_with(plan.len(), || None);
        }
        let mut generic_offsets = sentinel_offsets(plan.len());
        let mut generic_error = None;
        let mut generic_payload = None;
        let payload_is_valid = payload_owner.payload().is_some();
        match contract.request.framing {
            contract::OperationLayoutFraming::OrderedFields
            | contract::OperationLayoutFraming::FieldSequence => {
                for (index, field_plan) in plan.iter().enumerate().take(fields.len()) {
                    fields[index] = Some(OperationFieldRecord {
                        plan: field_plan,
                        value: None,
                    });
                }
                if plan.len() > generic_offsets.len() {
                    generic_error = Some("generated operation request field bound is stale");
                } else if let Some(payload) = payload_owner.payload() {
                    let decoded = decode_planned_fields(
                        payload,
                        plan,
                        contract.request.layout,
                        &mut generic_offsets[..],
                    );
                    if decoded.is_err() {
                        generic_error = Some("operation field sequence is malformed");
                    }
                } else {
                    generic_error = Some("generic request payload range is invalid");
                }
                generic_payload = Some(payload_owner);
            }
            contract::OperationLayoutFraming::Opaque => {
                if plan.len() != 1 {
                    generic_error = Some(if payload_is_valid {
                        "opaque operation requires one modeled field"
                    } else {
                        "generic request payload range is invalid"
                    });
                    generic_payload = Some(payload_owner);
                } else if let Some(field_plan) = plan.first() {
                    if !payload_is_valid {
                        generic_error = Some("generic request payload range is invalid");
                    }
                    match payload_owner {
                        GenericPayload::Owned(value) => {
                            fields[0] = Some(OperationFieldRecord {
                                plan: field_plan,
                                value: Some(OperationFieldStorage::OwnedBytes(value)),
                            });
                        }
                        GenericPayload::Frame {
                            frame,
                            payload_start,
                            payload_end,
                        } => {
                            fields[0] = Some(OperationFieldRecord {
                                plan: field_plan,
                                value: Some(OperationFieldStorage::FrameRange {
                                    start: payload_start,
                                    end: payload_end,
                                }),
                            });
                            generic_payload = Some(GenericPayload::Frame {
                                frame,
                                payload_start,
                                payload_end,
                            });
                        }
                        GenericPayload::Projection(projection) => {
                            generic_error =
                                Some("populated projection requires the projection constructor");
                            generic_payload = Some(GenericPayload::Projection(projection));
                        }
                    }
                }
            }
            contract::OperationLayoutFraming::OptionalValues => {
                generic_error = Some("optional-value framing is response-only");
                generic_payload = Some(payload_owner);
            }
            contract::OperationLayoutFraming::Empty => {
                if !payload_is_valid {
                    generic_error = Some("generic request payload range is invalid");
                } else if payload_owner
                    .payload()
                    .is_some_and(|payload| !payload.is_empty())
                {
                    generic_error = Some("empty operation request contains a payload");
                }
            }
        }
        Self {
            opcode,
            plan,
            fields,
            generic_payload,
            generic_offsets,
            generic_error,
        }
    }

    /// Validates fields populated by an API-owned framing adapter.
    ///
    /// The generic constructor validates ordered and opaque payloads from
    /// their generated byte envelopes. Compact protocol projections invoke
    /// this separate hook after populating their records, keeping the
    /// protocol-v1 framing decision out of the generic constructor.
    pub(super) fn validate_populated_fields(&mut self) {
        if self.plan.len() > self.fields.len() {
            self.generic_error = Some("generated operation request field bound is stale");
            return;
        }
        for (index, field) in self.plan.iter().enumerate() {
            let mut parent = field.parent_index;
            let mut hops = 0;
            while parent != usize::MAX {
                if parent >= index || parent >= self.plan.len() || hops >= self.plan.len() {
                    self.generic_error =
                        Some("generated operation field parent metadata is invalid");
                    return;
                }
                parent = self.plan[parent].parent_index;
                hops += 1;
            }
        }
        let plan = self.plan;
        let presence = Self::populated_presence(&self.fields, plan);
        if plan
            .iter()
            .enumerate()
            .any(|(index, field)| field.required && !presence[index])
        {
            self.generic_error = Some("required operation request field is missing");
        }
    }

    fn populated_presence(
        fields: &[Option<OperationFieldRecord>],
        plan: &'static [contract::OperationFieldPlan],
    ) -> SmallVec<[bool; INLINE_OPERATION_FIELDS]> {
        let mut presence = SmallVec::<[bool; INLINE_OPERATION_FIELDS]>::new();
        presence.resize(plan.len(), false);
        for (index, field) in plan.iter().enumerate() {
            if fields[index]
                .as_ref()
                .is_some_and(|record| record.value.is_some())
            {
                presence[index] = true;
                let mut parent = field.parent_index;
                let mut hops = 0;
                while parent != usize::MAX {
                    if parent >= plan.len() || hops >= plan.len() {
                        break;
                    }
                    presence[parent] = true;
                    parent = plan[parent].parent_index;
                    hops += 1;
                }
            }
        }
        presence
    }

    /// Returns whether the generated generic field sequence decoded cleanly.
    pub(super) fn is_valid(&self) -> bool {
        self.generic_error.is_none()
    }

    /// Validates modeled field codecs after the frame shape is decoded.
    ///
    /// This is deliberately a second boundary from `is_valid`: the latter
    /// checks only field-sequence cardinality, requiredness, and byte
    /// boundaries, while this method applies domain codec validation declared
    /// by Smithy. Keeping the phases separate lets a future codec expose a
    /// typed builder without teaching the frame parser about its semantics.
    pub(super) fn validate_codecs(&self) -> Result<(), &'static [u8]> {
        for (index, plan) in self.plan.iter().enumerate() {
            let Some(value) = self.field_at_index(index) else {
                continue;
            };
            OperationFieldEnvelope::from_plan(plan, value).validate()?;
        }
        Ok(())
    }

    /// Returns the number of modeled fields in the generated request plan.
    ///
    /// Generic behavior only needs this cardinality for shape-level checks;
    /// the generated plan itself remains an implementation detail of the
    /// input decoder and codec envelope.
    pub(super) const fn field_count(&self) -> usize {
        self.plan.len()
    }

    /// Returns the decoded ordered records, including missing optional fields.
    #[allow(dead_code)]
    pub(super) fn records(&self) -> &[Option<OperationFieldRecord>] {
        &self.fields[..self.plan.len()]
    }

    /// Returns the generated plan entry at a numeric slot.
    fn field_plan_at_index(&self, index: usize) -> Option<&'static contract::OperationFieldPlan> {
        self.fields.get(index)?.as_ref().map(|field| field.plan)
    }

    /// Returns one ordered field by its generated numeric index.
    pub(super) fn field_at_index(&self, index: usize) -> Option<OperationFieldValue<'_>> {
        if index >= self.plan.len() {
            return None;
        }
        if let Some(payload) = self
            .generic_payload
            .as_ref()
            .and_then(GenericPayload::payload)
        {
            let (start, end) = *self.generic_offsets.get(index)?;
            if start != usize::MAX {
                return payload.get(start..end);
            }
        }
        let frame = self
            .generic_payload
            .as_ref()
            .and_then(GenericPayload::owner);
        self.fields
            .get(index)?
            .as_ref()?
            .value
            .as_ref()
            .and_then(|value| value.as_value(frame))
    }

    /// Returns one present field together with its generated codec metadata.
    ///
    /// Compact compatibility scalar projections remain available through
    /// [`Self::field_at_index`]. This envelope is the stable generic boundary
    /// for new API-owned handlers and is populated for both generic field
    /// sequences and opaque compact values.
    pub(super) fn encoded_field_at_index(
        &self,
        index: usize,
    ) -> Option<OperationFieldEnvelope<'_>> {
        let plan = self.field_plan_at_index(index)?;
        self.field_at_index(index)
            .map(|bytes| OperationFieldEnvelope::from_plan(plan, bytes))
    }

    /// Returns a required descriptor-backed field at a generated numeric slot.
    pub(super) fn required_encoded_field_at_index(
        &self,
        index: usize,
        message: &'static [u8],
    ) -> Result<OperationFieldEnvelope<'_>, &'static [u8]> {
        self.encoded_field_at_index(index).ok_or(message)
    }

    /// Returns one borrowed byte field by generated numeric field index.
    pub(super) fn bytes_at_index(&self, index: usize) -> Option<&[u8]> {
        self.field_at_index(index)
    }

    /// Moves an owned payload and its logical range out of a generated field.
    ///
    /// Generic opaque requests can return the complete request frame together
    /// with a payload range. Callers that can retain a borrowed range until
    /// completion should use this method to avoid a prefix-removing memmove.
    pub(super) fn take_owned_bytes_range_at_index(
        &mut self,
        index: usize,
    ) -> Option<OwnedRange> {
        let value = self.fields.get_mut(index)?.as_mut()?.value.take()?;
        match value {
            OperationFieldStorage::OwnedBytes(value) => {
                Some(OwnedRange::whole(value))
            }
            OperationFieldStorage::FrameRange { start, end } => {
                let Some(owner) = self.generic_payload.take() else {
                    Option::as_mut(&mut self.fields[index])
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::FrameRange { start, end });
                    return None;
                };
                let GenericPayload::Frame {
                    frame,
                    payload_start: frame_start,
                    payload_end: frame_end,
                } = owner
                else {
                    self.generic_payload = Some(owner);
                    return None;
                };
                if frame_start > start || end > frame_end || start > end {
                    self.generic_payload = Some(GenericPayload::Frame {
                        frame,
                        payload_start: frame_start,
                        payload_end: frame_end,
                    });
                    Option::as_mut(&mut self.fields[index])
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::FrameRange { start, end });
                    return None;
                }
                OwnedRange::new(frame, start..end)
            }
            OperationFieldStorage::OwnerRange { start, end } => {
                let Some(payload) = self.generic_payload.take() else {
                    self.fields[index]
                        .as_mut()
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::OwnerRange { start, end });
                    return None;
                };
                let GenericPayload::Projection(mut projection) = payload else {
                    self.generic_payload = Some(payload);
                    self.fields[index]
                        .as_mut()
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::OwnerRange { start, end });
                    return None;
                };
                let range = projection.take(start, end);
                if range.is_none() {
                    self.generic_payload = Some(GenericPayload::Projection(projection));
                    self.fields[index]
                        .as_mut()
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::OwnerRange { start, end });
                }
                range
            }
            other => {
                self.fields
                    .get_mut(index)
                    .and_then(Option::as_mut)
                    .expect("field record remains present")
                    .value = Some(other);
                None
            }
        }
    }

    /// Moves an owned payload out of a generated numeric field index without
    /// allocating a second payload buffer.
    pub(super) fn take_owned_bytes_at_index(&mut self, index: usize) -> Option<Vec<u8>> {
        let owned = self.take_owned_bytes_range_at_index(index)?;
        let (mut frame, range) = owned.into_parts();
        let start = range.start;
        let end = range.end;
        if start == 0 && end == frame.len() {
            return Some(frame);
        }
        // Reuse the request-frame allocation for an opaque pass-through body.
        // The copy remains only for callers that require a standalone Vec;
        // range-aware consumers can avoid it with
        // `take_owned_bytes_range_at_index`.
        let payload_len = end - start;
        frame.copy_within(start..end, 0);
        frame.truncate(payload_len);
        Some(frame)
    }

    /// Moves the one modeled byte field and its logical range out of a
    /// generic single-field request.
    ///
    /// This is the allocation-preserving counterpart to
    /// [`Self::take_single_field_bytes`]. It lets a generic storage binding
    /// retain the original frame range without selecting an API role name.
    pub(super) fn take_single_field_bytes_range(&mut self) -> Option<OwnedRange> {
        if self.plan.len() != 1 {
            return None;
        }
        self.take_owned_bytes_range_at_index(0)
    }

    /// Moves or copies the one modeled byte field owned by a generic binding.
    ///
    /// Opaque requests can move their frame allocation directly. Ordered
    /// fields are borrowed from the generic payload and copied only at this
    /// explicit owned API boundary, so the transport never needs an operation
    /// specific framing branch.
    #[allow(dead_code)]
    pub(super) fn take_single_field_bytes(&mut self) -> Option<Vec<u8>> {
        if self.plan.len() != 1 {
            return None;
        }
        self.take_owned_bytes_at_index(0)
            .or_else(|| self.bytes_at_index(0).map(ToOwned::to_owned))
    }
}

fn empty_field_records(
    field_count: usize,
) -> SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]> {
    let mut fields = SmallVec::with_capacity(field_count);
    fields.resize_with(field_count, || None);
    fields
}

fn sentinel_offsets(field_count: usize) -> SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]> {
    let mut offsets = SmallVec::with_capacity(field_count);
    offsets.resize(field_count, (usize::MAX, usize::MAX));
    offsets
}

impl OperationFieldStorage {
    pub(super) fn inline<const N: usize>(value: [u8; N]) -> Self {
        assert!(N <= 8, "inline operation fields hold at most eight bytes");
        let mut bytes = [0; 8];
        bytes[..N].copy_from_slice(&value);
        Self::InlineBytes {
            bytes,
            len: N as u8,
        }
    }

    fn as_value<'a>(&'a self, frame: Option<&'a [u8]>) -> Option<&'a [u8]> {
        match self {
            Self::OwnedBytes(value) => Some(value),
            Self::InlineBytes { bytes, len } => Some(&bytes[..usize::from(*len)]),
            Self::StaticBytes(value) => Some(value),
            Self::FrameRange { start, end } => frame.and_then(|frame| frame.get(*start..*end)),
            Self::OwnerRange { start, end } => frame.and_then(|frame| frame.get(*start..*end)),
        }
    }
}

pub(super) struct OperationContext<'a> {
    pub(super) capabilities: &'a dyn CapabilityCatalog,
    pub(super) input: OperationInputView,
}

impl<'a> OperationContext<'a> {
    /// Looks up an API-owned dependency without exposing type erasure to a
    /// behavior binding.
    #[allow(dead_code)]
    pub(super) fn capability<T: Any>(
        &self,
        key: super::operation_api::CapabilityKey<T>,
    ) -> Option<&'a T> {
        super::operation_api::downcast_capability(self.capabilities, key)
    }
}

/// Verifies that every modeled opcode has a server-owned execution path.
///
/// This runs during server bind rather than allowing an omitted behavior to
/// reach a panic or an accidental fallback response.
pub(super) fn validate_handler_registry() -> Result<(), &'static str> {
    super::operation_api::validate_registry()?;
    super::operation_codecs::validate_contract_codecs()?;
    for entry in contract::operation_registry() {
        let opcode = entry.opcode;
        if super::operation_api::server_operation(opcode).is_some() {
            continue;
        }
        return Err("modeled operation has no registered server handler");
    }
    Ok(())
}
