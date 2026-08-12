//! Semantic validation for the protocol-v1 compatibility projection.
//!
//! Generic request framing stops at generated field boundaries. This adapter
//! owns the historical namespace/item/SET shape checks that are needed by the
//! public convenience facade, keeping those rules out of the generic parser.

use super::super::{ProtocolError, Result, SetOptions};

pub(crate) fn validate_request(request: &super::super::Request) -> Result<()> {
    super::super::validate_value_length(request.value.len())?;
    let opcode = request.opcode;
    if super::contract::request_wire_plan(opcode).is_none() {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation has no compact request plan",
        ));
    }
    let item_count = super::field_count(opcode, "item_id");
    let value_count = super::field_count(opcode, "value");
    let has_name = super::field_count(opcode, "name") != 0;
    let has_revision = super::field_count(opcode, "expected_revision") != 0;
    let has_policy = super::field_count(opcode, "policy") != 0;
    if item_count != 0 {
        validate_namespace_id(request.namespace_id)?;
        if request.item_ids.len() != item_count
            || request.namespace_name.is_some()
            || request.namespace_policy.is_some()
            || request.expected_revision.is_some()
            || request.create_if_missing
        {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: openkache_protocol::ITEM_ID_BYTES * item_count,
                expected_value: if value_count == 0 { "0" } else { "any" },
            });
        }
        if value_count == 0 {
            if request.set_options != SetOptions::NONE || !request.value.is_empty() {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode,
                    expected_item_id: openkache_protocol::ITEM_ID_BYTES * item_count,
                    expected_value: "0",
                });
            }
        } else {
            request.set_options.flags()?;
        }
        return Ok(());
    }
    if has_name {
        let name = request
            .namespace_name
            .as_deref()
            .ok_or(ProtocolError::InvalidNamespaceName(
                "namespace name missing",
            ))?;
        validate_namespace_name(name)?;
        if request.create_if_missing != request.namespace_policy.is_some()
            || request.namespace_id.is_some()
            || !request.item_ids.is_empty()
            || request.set_options != SetOptions::NONE
            || !request.value.is_empty()
            || request.expected_revision.is_some()
        {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "0",
            });
        }
        if let Some(policy) = request.namespace_policy {
            policy.encode()?;
        }
        return Ok(());
    }
    if has_revision {
        validate_namespace_id(request.namespace_id)?;
        validate_revision(request.expected_revision)?;
        if has_policy {
            request
                .namespace_policy
                .ok_or(ProtocolError::MissingNamespacePolicy)?
                .encode()?;
        }
        if !request.item_ids.is_empty()
            || request.set_options != SetOptions::NONE
            || !request.value.is_empty()
            || request.namespace_name.is_some()
            || request.create_if_missing
        {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "0",
            });
        }
        return Ok(());
    }
    if super::field_count(opcode, "namespace_id") != 0 {
        validate_namespace_id(request.namespace_id)?;
    }
    if !request.item_ids.is_empty()
        || request.set_options != SetOptions::NONE
        || !request.value.is_empty()
        || request.namespace_name.is_some()
        || request.namespace_policy.is_some()
        || request.expected_revision.is_some()
        || request.create_if_missing
    {
        return Err(ProtocolError::InvalidRequestShape {
            opcode,
            expected_item_id: 0,
            expected_value: "0",
        });
    }
    Ok(())
}

pub(crate) fn validate_namespace_name(name: &[u8]) -> Result<()> {
    if name.len() > super::namespace_name_max_bytes() {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name exceeds 255 octets",
        ));
    }
    std::str::from_utf8(name)
        .map_err(|_| ProtocolError::InvalidNamespaceName("namespace name is not UTF-8"))?;
    Ok(())
}

pub(crate) fn validate_namespace_id(namespace_id: Option<u64>) -> Result<u64> {
    match namespace_id {
        Some(value @ 1..) => Ok(value),
        Some(0) => Err(ProtocolError::InvalidNamespaceId),
        None => Err(ProtocolError::MissingNamespaceId),
    }
}

pub(crate) fn validate_revision(revision: Option<u64>) -> Result<u64> {
    match revision {
        Some(value @ 1..) => Ok(value),
        _ => Err(ProtocolError::InvalidRevision),
    }
}
