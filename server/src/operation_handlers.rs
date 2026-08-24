//! Server-owned operation handlers.
//!
//! The protocol and client infrastructure only decode a request according to
//! its generated wire contract. This module is the server's decision point:
//! it receives a borrowed operation context and calls the concrete behavior
//! selected by the server. Adding a new operation therefore does not add an
//! operation-name branch to the transport, framing, or client infrastructure.

use std::any::Any;

use crate::openkache_protocol::OwnedRange;
use smallvec::SmallVec;

use super::operation_execution_state::OperationStateRef;
pub(super) use super::operation_fields::OperationFieldEnvelope;
use crate::operation_contract as contract;

const INLINE_OPERATION_FIELDS: usize = contract::MAX_OPERATION_REQUEST_FIELDS;

/// Owns one buffer behind fields populated by the generated projection.
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
        let owner = self.owner.take()?;
        match owner.into_subrange(start..end) {
            Ok(range) => Some(range),
            Err(owner) => {
                self.owner = Some(owner);
                None
            }
        }
    }
}

/// Context passed from the protocol server to one concrete handler.
///
/// The context deliberately contains storage primitives and decoded request
/// fields, rather than exposing transport or frame details to API handlers.
pub(super) struct OperationInputView {
    pub(super) operation_id: contract::OperationId,
    request_id: u64,
    plan: &'static [contract::OperationFieldPlan],
    fields: SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]>,
    projection: Option<OwnedFieldProjection>,
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
/// The generic view carries only borrowed, inline, or static canonical bytes.
pub(super) enum OperationFieldStorage {
    InlineBytes { bytes: [u8; 8], len: u8 },
    OwnerRange { start: usize, end: usize },
    StaticBytes(&'static [u8]),
}

/// A borrowed generic field value. All semantic interpretation happens
/// through a generated codec or an API-owned binding.
pub(super) type OperationFieldValue<'a> = &'a [u8];

impl OperationInputView {
    /// Returns the generated operation identity carried by this view.
    pub(super) const fn operation_id(&self) -> contract::OperationId {
        self.operation_id
    }

    /// Returns the client-selected correlation token carried by this request.
    pub(super) const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Builds a view from generated numeric field records.
    pub(super) fn from_populated_parts<I>(
        operation_id: contract::OperationId,
        fields: I,
    ) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OperationFieldRecord>>,
    {
        let mut fields: SmallVec<[Option<OperationFieldRecord>; INLINE_OPERATION_FIELDS]> =
            fields.into_iter().collect();
        let plan = contract::operation_wire_spec_for_id(operation_id)
            .request
            .fields;
        if fields.len() < plan.len() {
            fields.resize_with(plan.len(), || None);
        }
        OperationInputView {
            operation_id,
            request_id: 0,
            plan,
            fields,
            projection: None,
        }
    }

    /// Builds populated numeric fields over one operation-neutral owner.
    ///
    /// Field ranges are relative to the owner's visible bytes.
    pub(super) fn from_populated_projection<I>(
        operation_id: contract::OperationId,
        request_id: u64,
        owner: OwnedRange,
        fields: I,
    ) -> OperationInputView
    where
        I: IntoIterator<Item = Option<OperationFieldRecord>>,
    {
        let mut input = Self::from_populated_parts(operation_id, fields);
        input.request_id = request_id;
        input.projection = Some(OwnedFieldProjection::new(owner));
        input
    }

    /// Validates fields populated by the generated frame projector.
    pub(super) fn validate_populated_fields(&self) -> Result<(), &'static str> {
        if self.plan.len() > self.fields.len() {
            return Err("generated operation request field bound is stale");
        }
        for (index, field) in self.plan.iter().enumerate() {
            let mut parent = field.parent_index;
            let mut hops = 0;
            while parent != usize::MAX {
                if parent >= index || parent >= self.plan.len() || hops >= self.plan.len() {
                    return Err("generated operation field parent metadata is invalid");
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
            return Err("required operation request field is missing");
        }
        Ok(())
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

    /// Validates modeled field codecs after the frame shape is decoded.
    ///
    /// Frame projection has already checked cardinality, requiredness, and byte
    /// boundaries. This method applies domain codec validation declared by
    /// Smithy without teaching the frame parser about those semantics.
    pub(super) fn validate_codecs(&self) -> Result<(), &'static [u8]> {
        for (index, plan) in self.plan.iter().enumerate() {
            let Some(value) = self.field_at_index(index) else {
                continue;
            };
            OperationFieldEnvelope::from_plan(plan, value).validate()?;
        }
        Ok(())
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
        let frame = self
            .projection
            .as_ref()
            .and_then(OwnedFieldProjection::as_slice);
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

    /// Returns one borrowed byte field by generated numeric field index.
    pub(super) fn bytes_at_index(&self, index: usize) -> Option<&[u8]> {
        self.field_at_index(index)
    }

    /// Moves an owned payload and its logical range out of a generated field.
    ///
    /// Generic opaque requests can return the complete request frame together
    /// with a payload range. Callers that can retain a borrowed range until
    /// completion should use this method to avoid a prefix-removing memmove.
    pub(super) fn take_owned_bytes_range_at_index(&mut self, index: usize) -> Option<OwnedRange> {
        let value = self.fields.get_mut(index)?.as_mut()?.value.take()?;
        match value {
            OperationFieldStorage::OwnerRange { start, end } => {
                let Some(mut projection) = self.projection.take() else {
                    self.fields[index]
                        .as_mut()
                        .expect("field record remains present")
                        .value = Some(OperationFieldStorage::OwnerRange { start, end });
                    return None;
                };
                let range = projection.take(start, end);
                if range.is_none() {
                    self.projection = Some(projection);
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
}

impl OperationFieldStorage {
    fn as_value<'a>(&'a self, frame: Option<&'a [u8]>) -> Option<&'a [u8]> {
        match self {
            Self::InlineBytes { bytes, len } => Some(&bytes[..usize::from(*len)]),
            Self::StaticBytes(value) => Some(value),
            Self::OwnerRange { start, end } => frame.and_then(|frame| frame.get(*start..*end)),
        }
    }
}

pub(super) struct OperationContext<'a> {
    pub(super) state: OperationStateRef<'a>,
    pub(super) input: OperationInputView,
}

impl<'a> OperationContext<'a> {
    /// Borrows this operation's API module state.
    pub(super) fn state<T: Any>(&self) -> Option<&'a T> {
        self.state.get()
    }
}
