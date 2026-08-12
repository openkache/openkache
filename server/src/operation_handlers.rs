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

enum InputFields {
    /// A projection that already owns one range per generated field.
    Exact(SmallVec<[Option<OwnedRange>; INLINE_OPERATION_FIELDS]>),
    /// A generic request that keeps one allocation and borrows logical ranges
    /// from it until a handler explicitly moves one out.
    Generic {
        payload: OwnedRange,
        offsets: SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]>,
    },
}

/// Context passed from the protocol server to one concrete handler.
///
/// The context deliberately contains storage primitives and decoded request
/// fields, rather than exposing transport or frame details to API handlers.
pub(super) struct OperationInputView {
    pub(super) opcode: Opcode,
    plan: &'static [contract::OperationFieldPlan],
    fields: InputFields,
    generic_error: Option<&'static str>,
}

/// Decodes an empty, opaque, or ordered request from the common owned
/// transport envelope. Compact requests select their API-owned projection
/// through registration instead of changing this envelope.
pub(super) fn decode_request(request: ServerRequest) -> OperationInputView {
    let opcode = request.opcode();
    if contract::spec(opcode).generic_request_framing().is_some()
        && let Some(plan) = contract::request_wire_plan(opcode)
    {
        let (_, prefix, payload) = request.into_wire_parts();
        return match openkache_protocol::decode_request_wire_fields(
            OwnedRange::whole(prefix),
            payload,
            plan,
        ) {
            Ok(fields) => OperationInputView::from_wire_fields(opcode, fields),
            Err(_) => OperationInputView::invalid(
                opcode,
                "request does not match the generated wire plan",
            ),
        };
    }
    let (opcode, payload) = request.into_payload();
    OperationInputView::from_owned_payload(opcode, payload)
}

/// A borrowed generic field value. All semantic interpretation happens
/// through a generated codec or an API-owned binding.
pub(super) type OperationFieldValue<'a> = &'a [u8];

impl OperationInputView {
    /// Builds a fail-closed view when an API-owned projection rejects bytes
    /// retained by the common transport envelope.
    pub(super) fn invalid(opcode: Opcode, message: &'static str) -> OperationInputView {
        let plan = contract::spec(opcode).request.fields;
        OperationInputView {
            opcode,
            plan,
            fields: InputFields::Generic {
                payload: OwnedRange::whole(Vec::new()),
                offsets: sentinel_offsets(plan.len()),
            },
            generic_error: Some(message),
        }
    }

    /// Builds a field view from one generated compact request plan.
    pub(super) fn from_wire_fields<I>(
        opcode: Opcode,
        values: I,
    ) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OwnedRange>>,
    {
        let plan = contract::spec(opcode).request.fields;
        if plan.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
            return Self::invalid(opcode, "request wire field count is invalid");
        }
        let mut values = values.into_iter();
        let mut fields =
            SmallVec::<[Option<OwnedRange>; INLINE_OPERATION_FIELDS]>::with_capacity(plan.len());
        for _ in plan {
            let Some(value) = values.next() else {
                return Self::invalid(opcode, "request wire field count is invalid");
            };
            fields.push(value);
        }
        if values.next().is_some() {
            return Self::invalid(opcode, "request wire field count is invalid");
        }
        let mut input = Self {
            opcode,
            plan,
            fields: InputFields::Exact(fields),
            generic_error: None,
        };
        input.validate_populated_fields();
        input
    }

    /// Returns the generated operation identity carried by this view.
    pub(super) const fn opcode(&self) -> Opcode {
        self.opcode
    }

    /// Builds a generic view over an independently owned request payload.
    pub(super) fn from_owned_payload(opcode: Opcode, payload: OwnedRange) -> OperationInputView {
        Self::from_generic_payload(opcode, payload)
    }

    fn from_generic_payload(opcode: Opcode, payload: OwnedRange) -> OperationInputView {
        let contract = contract::spec(opcode);
        let plan = contract.request.fields;
        let mut offsets = sentinel_offsets(plan.len());
        let mut generic_error = None;
        match contract.generic_request_framing() {
            Some(contract::OperationRequestFraming::OrderedFields) => {
                if plan.len() > offsets.len() {
                    generic_error = Some("generated operation request field bound is stale");
                } else if decode_planned_fields(
                    payload.as_slice(),
                    plan,
                    contract.request.layout,
                    &mut offsets[..],
                )
                .is_err()
                {
                    generic_error = Some("operation field sequence is malformed");
                }
            }
            Some(contract::OperationRequestFraming::Opaque) => {
                if plan.len() != 1 {
                    generic_error = Some("opaque operation requires one modeled field");
                } else if plan.first().is_some() {
                    offsets[0] = (0, payload.len());
                }
            }
            Some(contract::OperationRequestFraming::Empty) => {
                if !payload.is_empty() {
                    generic_error = Some("empty operation request contains a payload");
                }
            }
            None => generic_error = Some("operation request framing is not generic"),
        }
        Self {
            opcode,
            plan,
            fields: InputFields::Generic { payload, offsets },
            generic_error,
        }
    }

    /// Validates fields populated by an API-owned framing adapter.
    ///
    /// The generic constructor validates ordered and opaque payloads from
    /// their generated byte envelopes. Exact-plan projections invoke this
    /// hook after populating their fields, keeping framing decisions out of
    /// the generic field validator.
    pub(super) fn validate_populated_fields(&mut self) {
        let InputFields::Exact(fields) = &self.fields else {
            self.generic_error = Some("operation fields were not populated");
            return;
        };
        if self.plan.len() > fields.len() {
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
        let presence = Self::populated_presence(fields, plan);
        if plan
            .iter()
            .enumerate()
            .any(|(index, field)| field.required && !presence[index])
        {
            self.generic_error = Some("required operation request field is missing");
        }
    }

    fn populated_presence(
        fields: &[Option<OwnedRange>],
        plan: &'static [contract::OperationFieldPlan],
    ) -> SmallVec<[bool; INLINE_OPERATION_FIELDS]> {
        let mut presence = SmallVec::<[bool; INLINE_OPERATION_FIELDS]>::new();
        presence.resize(plan.len(), false);
        for (index, field) in plan.iter().enumerate() {
            if fields[index].is_some() {
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

    /// Returns the generated plan entry at a numeric slot.
    fn field_plan_at_index(&self, index: usize) -> Option<&'static contract::OperationFieldPlan> {
        self.plan.get(index)
    }

    /// Returns one ordered field by its generated numeric index.
    pub(super) fn field_at_index(&self, index: usize) -> Option<OperationFieldValue<'_>> {
        if index >= self.plan.len() {
            return None;
        }
        match &self.fields {
            InputFields::Generic { payload, offsets } => {
                let (start, end) = *offsets.get(index)?;
                if start != usize::MAX {
                    payload.as_slice().get(start..end)
                } else {
                    None
                }
            }
            InputFields::Exact(fields) => fields.get(index)?.as_ref().map(OwnedRange::as_slice),
        }
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
        match &mut self.fields {
            InputFields::Exact(fields) => fields.get_mut(index).and_then(Option::take),
            InputFields::Generic { payload, offsets } => {
                let (start, end) = *offsets.get(index)?;
                if start == usize::MAX || start > end || end > payload.len() {
                    return None;
                }
                // A multi-field generic request keeps the shared `Bytes`
                // allocation alive so each extracted field can move a
                // zero-copy range. The single-field path transfers the owner
                // directly, avoiding an unnecessary reference-count
                // increment for the common opaque case.
                let value = if self.plan.len() == 1 {
                    if end > payload.len() {
                        return None;
                    }
                    let payload = std::mem::replace(payload, OwnedRange::whole(Vec::new()));
                    // The bounds check above makes this infallible. Keeping
                    // the checked constructor here still makes the ownership
                    // transfer fail closed if the range representation ever
                    // changes independently of the generated decoder.
                    payload.slice(start..end)?
                } else {
                    payload.clone().slice(start..end)?
                };
                // Mark a shared generic range as consumed. The backing
                // allocation remains available for sibling fields, but the
                // same logical field must not be moved twice if a binding
                // retries extraction.
                if self.plan.len() > 1 {
                    offsets[index] = (usize::MAX, usize::MAX);
                }
                Some(value)
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
    /// [`Self::take_owned_bytes_at_index`]. It lets a generic storage binding
    /// retain the original frame range without selecting an API role name.
    pub(super) fn take_single_field_bytes_range(&mut self) -> Option<OwnedRange> {
        if self.plan.len() != 1 || self.field_at_index(0).is_none() {
            return None;
        }
        self.take_owned_bytes_range_at_index(0)
    }
}

fn sentinel_offsets(field_count: usize) -> SmallVec<[(usize, usize); INLINE_OPERATION_FIELDS]> {
    let mut offsets = SmallVec::with_capacity(field_count);
    offsets.resize(field_count, (usize::MAX, usize::MAX));
    offsets
}

pub(super) struct OperationContext<'a> {
    pub(super) capabilities: &'a dyn CapabilityCatalog,
    pub(super) input: OperationInputView,
}

impl<'a> OperationContext<'a> {
    /// Looks up an API-owned dependency without exposing type erasure to a
    /// behavior binding.
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
