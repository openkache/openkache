//! Draft-v1 request construction owned by the Rust client adapter.

use openkache_protocol::{ItemId, Opcode, WireSegment};

use crate::request::{RequestBuilder, RequestParts, RequestRetryPolicy};

use super::{
    NamespacePolicy, Result, SetWireOptions, compat_v1, generated_retry_policy, invalid_shape,
    validate_value_length,
};

/// A validated draft-v1 request owned by the Rust client adapter.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DraftV1Request {
    pub(super) opcode: Opcode,
    pub(super) namespace_id: Option<u64>,
    pub(super) item_ids: Vec<ItemId>,
    pub(super) set_options: SetWireOptions,
    pub(super) value: Vec<u8>,
    pub(super) namespace_name: Option<Vec<u8>>,
    pub(super) namespace_policy: Option<NamespacePolicy>,
    pub(super) expected_revision: Option<u64>,
    pub(super) create_if_missing: bool,
    retry_policy: RequestRetryPolicy,
}

impl DraftV1Request {
    pub(crate) fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(opcode, namespace_id, item_id, SetWireOptions::NONE, value)
    }

    pub(crate) fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetWireOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids: item_id.into_iter().collect(),
            set_options,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
            retry_policy: generated_retry_policy(opcode, false),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_open(
        name: impl AsRef<[u8]>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceOpen,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: Some(name.as_ref().to_vec()),
            namespace_policy: policy,
            expected_revision: None,
            create_if_missing,
            retry_policy: generated_retry_policy(Opcode::NamespaceOpen, create_if_missing),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceUpdatePolicy,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: Some(policy),
            expected_revision: Some(expected_revision),
            create_if_missing: false,
            retry_policy: generated_retry_policy(Opcode::NamespaceUpdatePolicy, false),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceDelete,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetWireOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: Some(expected_revision),
            create_if_missing: false,
            retry_policy: generated_retry_policy(Opcode::NamespaceDelete, false),
        };
        request.validate()?;
        Ok(request)
    }

    fn into_parts(self) -> Result<RequestParts> {
        let prefix = compat_v1::encode_prefix(&self)?
            .ok_or_else(|| invalid_shape(self.opcode, self.item_ids.len(), "draft-v1 request"))?;
        Ok(RequestParts::new([
            WireSegment::owned(prefix),
            WireSegment::owned(self.value),
        ])?)
    }

    fn validate(&self) -> Result<()> {
        validate_value_length(self.value.len())?;
        if compat_v1::validate_request(self)? {
            Ok(())
        } else {
            Err(invalid_shape(
                self.opcode,
                self.item_ids.len(),
                "draft-v1 request",
            ))
        }
    }

    pub(super) fn has_non_empty_fields_except_namespace(&self) -> bool {
        !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || !self.value.is_empty()
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
    }

    pub(super) fn has_non_empty_fields_except_namespace_revision(&self) -> bool {
        !self.item_ids.is_empty()
            || self.set_options != SetWireOptions::NONE
            || !self.value.is_empty()
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.create_if_missing
    }
}

impl RequestBuilder for DraftV1Request {
    fn retry_policy(&self) -> RequestRetryPolicy {
        self.retry_policy
    }

    fn into_parts(self) -> crate::Result<RequestParts> {
        Self::into_parts(self).map_err(crate::Error::protocol)
    }
}
