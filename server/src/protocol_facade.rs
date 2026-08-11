//! Compatibility-only semantic request construction and decoding.
//!
//! Generic operations use [`super::RequestFrame`] and the generated operation
//! field decoder. This module preserves the historical namespace/item/SET
//! convenience API without making generic framing depend on those concepts.

use super::{
    FrameLayoutProvider, ProtocolError, RequestFrame, RequestHeader, Result, ServerRequest,
    SetOptions, compat_v1, generic,
};
use openkache_protocol::{OwnedRange, Opcode, REQUEST_FIXED_BYTES};
use super::{ItemId, NamespacePolicy};

/// A decoded protocol-v1 compatibility request.
///
/// This facade preserves the historical namespace/item/SET convenience fields.
/// Generic operations, including exact plans without an explicit
/// `compatibilityRequestProjection`, must use [`RequestFrame`] and the
/// generated operation field decoder instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub namespace_id: Option<u64>,
    pub item_ids: Vec<ItemId>,
    pub set_options: SetOptions,
    pub value: Vec<u8>,
    pub namespace_name: Option<Vec<u8>>,
    pub namespace_policy: Option<NamespacePolicy>,
    pub expected_revision: Option<u64>,
    pub create_if_missing: bool,
}

impl Request {
    /// Creates a request for a route-less operation using its already encoded
    /// generic body.
    ///
    /// Generic callers do not need to construct the historical namespace/item
    /// facade fields. The operation contract still validates the framing and
    /// generated field shape before the request is returned.
    pub fn new_generic(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        if super::super::contract::request_wire_plan(opcode).is_some() {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "generic operation",
            });
        }
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a route-less request from generated ordered field values.
    ///
    /// Field presence, codecs, and dense/sequence layout are selected from
    /// the operation contract. API callers provide only the modeled field
    /// bytes in plan order; compatibility operations intentionally reject this
    /// constructor and use their typed adapter instead.
    pub fn new_generic_fields(opcode: Opcode, fields: Vec<Option<Vec<u8>>>) -> Result<Self> {
        if super::super::contract::request_wire_plan(opcode).is_some() {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "generic operation",
            });
        }
        let value = generic::encode_fields(opcode, fields)?;
        Self::new_generic(opcode, value)
    }

    /// Creates a request for an operation that has no namespace or item fields.
    pub fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
        if item_id.is_none() {
            return Self::new_generic(opcode, value);
        }
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: item_id.into_iter().collect(),
            set_options: SetOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request with a namespace ID.
    pub fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(opcode, namespace_id, item_id, SetOptions::NONE, value)
    }

    /// Creates a data-plane request with explicit SET options.
    pub fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetOptions,
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
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request carrying one or more exact item IDs.
    pub fn new_scoped_items(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids,
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a SET request with explicit options.
    pub fn new_set(
        namespace_id: u64,
        item_id: ItemId,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(Opcode::Set, namespace_id, Some(item_id), set_options, value)
    }

    /// Creates a namespace-open request. An empty name is a valid name.
    pub fn namespace_open(
        name: impl AsRef<[u8]>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceOpen,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: Some(name.as_ref().to_vec()),
            namespace_policy: policy,
            expected_revision: None,
            create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a namespace-policy update request.
    pub fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceUpdatePolicy,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: Some(policy),
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates an empty-only namespace-delete request.
    pub fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceDelete,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if compat_v1::is_compatibility_operation(self.opcode) {
            self.validate()?;
            return compat_v1::encode_request(self);
        }
        let mut frame = self.encode_prefix()?;
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        if compat_v1::is_compatibility_operation(self.opcode) {
            return self.encode();
        }
        let prefix = self.encode_prefix()?;
        let value_len = self.value.len();
        self.value.reserve(prefix.len());
        self.value.resize(prefix.len() + value_len, 0);
        self.value.copy_within(0..value_len, prefix.len());
        self.value[..prefix.len()].copy_from_slice(&prefix);
        Ok(self.value)
    }

    fn encode_prefix(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::new();
        output.push(self.opcode as u8);
        let encoded = if compat_v1::is_compatibility_operation(self.opcode) {
            compat_v1::encode_request_prefix(self, &mut output)?
        } else {
            generic::encode_request_prefix(self, &mut output)?
        };
        if !encoded {
            return Err(ProtocolError::InvalidFieldSequence(
                "generated request plan did not encode the modeled operation",
            ));
        }
        Ok(output)
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        Self::decode_with(frame, &super::GeneratedFrameLayoutProvider)
    }

    /// Decodes and validates one complete request frame with an explicitly
    /// selected layout provider.
    pub(crate) fn decode_with<P: FrameLayoutProvider + ?Sized>(
        frame: &[u8],
        provider: &P,
    ) -> Result<Self> {
        let header = Self::validated_header_with(frame, provider)?;
        if compat_v1::is_compatibility_operation(header.opcode()) {
            compat_v1::decode_request(frame, header)
        } else {
            generic::decode_request(frame, header)
        }
    }

    /// Decodes a request while reusing the frame allocation for its value.
    pub fn decode_owned(frame: Vec<u8>) -> Result<Self> {
        Self::decode_owned_impl(frame, &super::GeneratedFrameLayoutProvider)
    }

    /// Decodes independently owned request-prefix and payload buffers.
    ///
    /// Network backends use this entry point so a contiguous receive chunk can
    /// remain reference-counted through generic execution. Compatibility
    /// adapters may materialize the payload only at their own semantic
    /// boundary.
    pub(crate) fn decode_received_for_server_with<P: FrameLayoutProvider + ?Sized>(
        prefix: Vec<u8>,
        payload: OwnedRange,
        provider: &P,
    ) -> Result<ServerRequest> {
        let header = RequestFrame::decode_header_with(&prefix, provider)?.ok_or(
            ProtocolError::FrameTooShort {
                expected: REQUEST_FIXED_BYTES,
                actual: prefix.len(),
            },
        )?;
        let actual = prefix
            .len()
            .checked_add(payload.len())
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let expected = header.frame_len()?;
        if prefix.len() != header.encoded_len() || payload.len() != header.value_len() {
            return Err(ProtocolError::FrameLength { expected, actual });
        }
        Ok(ServerRequest::new(header.opcode(), prefix, payload))
    }

    fn decode_owned_impl<P: FrameLayoutProvider + ?Sized>(
        frame: Vec<u8>,
        provider: &P,
    ) -> Result<Self> {
        let header = Self::validated_header_with(&frame, provider)?;
        if compat_v1::is_compatibility_operation(header.opcode()) {
            let decode_owned_request: fn(Vec<u8>, RequestHeader) -> Result<Self> =
                compat_v1::decode_owned_request;
            decode_owned_request(frame, header)
        } else {
            generic::decode_owned_request(frame, header)
        }
    }

    fn validated_header_with<P: FrameLayoutProvider + ?Sized>(
        frame: &[u8],
        provider: &P,
    ) -> Result<RequestHeader> {
        let header =
            Self::decode_header_with(frame, provider)?.ok_or(ProtocolError::FrameTooShort {
                expected: REQUEST_FIXED_BYTES,
                actual: frame.len(),
            })?;
        let expected = header
            .frame_len(&frame)?
            .ok_or(ProtocolError::FrameTooShort {
                expected: header.encoded_len(),
                actual: frame.len(),
            })?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(header)
    }

    /// Decodes a request header when enough metadata bytes are available.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
        Self::decode_header_with(prefix, &super::GeneratedFrameLayoutProvider)
    }

    /// Decodes a request header with an explicitly selected layout provider.
    pub(crate) fn decode_header_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> Result<Option<RequestHeader>> {
        let Some(&opcode_byte) = prefix.first() else {
            return Ok(None);
        };
        let opcode = Opcode::try_from(opcode_byte)?;
        if compat_v1::is_compatibility_operation(opcode) {
            // Compact requests have a semantic adapter-owned header. Let it
            // validate packed policy bits and project compatibility errors
            // before the operation-neutral parser is asked only to delimit a
            // generic frame.
            return compat_v1::decode_header(prefix, opcode);
        }
        if super::super::contract::request_wire_plan(opcode).is_some() {
            return Err(ProtocolError::InvalidFieldSequence(
                "generic exact requests must use RequestFrame or generated operation decoding",
            ));
        }
        let layout = provider.layout_for(opcode)?;
        let Some(frame) = openkache_protocol::OpaqueRequestFrame::decode_header(prefix, layout)?
        else {
            return Ok(None);
        };
        Ok(Some(RequestHeader::generic(
            opcode,
            frame.encoded_len(),
            frame.value_len(),
        )))
    }

    /// Reports the complete request frame length once metadata is available.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::frame_len_with(prefix, &super::GeneratedFrameLayoutProvider)
    }

    /// Reports frame length with an explicitly selected layout provider.
    pub(crate) fn frame_len_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> Result<Option<usize>> {
        Self::decode_header_with(prefix, provider)?
            .map(|header| header.frame_len(prefix))
            .transpose()
            .map(|value| value.flatten())
    }

    fn validate(&self) -> Result<()> {
        if !compat_v1::is_compatibility_operation(self.opcode)
            && super::super::contract::request_wire_plan(self.opcode).is_some()
        {
            return Err(ProtocolError::InvalidFieldSequence(
                "generic exact requests must use RequestFrame or generated operation decoding",
            ));
        }
        if compat_v1::is_compatibility_operation(self.opcode) {
            compat_v1::validate_request(self)
        } else {
            generic::validate_request(self)
        }
    }

    /// Returns the payload view accepted by a generic framing adapter.
    ///
    /// Compatibility-only semantic metadata stays in the v1 projection. The
    /// generic adapter receives this narrow view instead of inspecting the
    /// compatibility facade fields itself.
    pub(super) fn generic_payload(&self) -> Result<&[u8]> {
        if self.namespace_id.is_some()
            || !self.item_ids.is_empty()
            || self.set_options != SetOptions::NONE
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
        {
            return Err(ProtocolError::InvalidRequestShape {
                opcode: self.opcode,
                expected_item_id: 0,
                expected_value: "generic fields",
            });
        }
        Ok(&self.value)
    }

    pub(super) fn from_generic_parts(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        Self::new_generic(opcode, value)
    }

    pub(super) fn from_decoded_parts(
        metadata: compat_v1::DecodedRequestMetadata,
        value: Vec<u8>,
        opcode: Opcode,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: metadata.namespace_id,
            item_ids: metadata.item_ids,
            set_options: metadata.set_options,
            value,
            namespace_name: metadata.namespace_name,
            namespace_policy: metadata.namespace_policy,
            expected_revision: metadata.expected_revision,
            create_if_missing: metadata.create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }
}
