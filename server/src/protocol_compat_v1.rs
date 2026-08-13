//! Historical protocol-v1 request layouts.
//!
//! The public server protocol still exposes semantic request and policy types,
//! but the old namespace/item/SET prefix grammar belongs to this compatibility
//! projection. Generic frame parsing consumes only the generated layout
//! returned by this module.

use openkache_protocol::compat_v1::OperationCompactV1Route;

use super::super::operation_compatibility_contract as contract;
use contract::{
    DELETE_FLAGS_BYTES, DELETE_IF_EMPTY, DELETE_MODE_MASK, DELETE_RESERVED_MASK,
    NAMESPACE_NAME_MAX_BYTES, OPEN_CREATE_IF_MISSING, OPEN_FLAGS_BYTES, OPEN_RESERVED_MASK,
    SET_CONDITION_MASK, SET_CONDITION_RESERVED_BITS, SET_EVICTABLE_BITS, SET_EVICTION_MASK,
    SET_EVICTION_PROTECTED_BITS, SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS, SET_FLAGS_BYTES,
    SET_INHERIT_EVICTION_BITS, SET_INHERIT_EXPIRATION_BITS, SET_NO_EXPIRY_BITS, SET_RESERVED_MASK,
};

use super::{
    ItemId, NamespacePolicy, Opcode, ProtocolError, RequestHeader, Result, SetOptions,
    WireRequestLayout, WireRequestStep, WireResult,
};

#[path = "protocol_compat_v1_policy.rs"]
mod policy;
pub(crate) use policy::decode_namespace_policy;

/// Compact request layouts owned by the protocol-v1 compatibility adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompactV1RequestRoute {
    Item,
    Set,
    Namespace,
    NamespaceOpen,
    NamespaceUpdatePolicy,
    NamespaceDelete,
}

pub(super) struct DecodedRequestMetadata {
    pub(super) namespace_id: Option<u64>,
    pub(super) item_ids: Vec<ItemId>,
    pub(super) set_options: SetOptions,
    pub(super) namespace_name: Option<Vec<u8>>,
    pub(super) namespace_policy: Option<NamespacePolicy>,
    pub(super) expected_revision: Option<u64>,
    pub(super) create_if_missing: bool,
}

/// Resolves the request shape for the public protocol-v1 request adapter.
pub(super) fn compatibility_route(opcode: Opcode) -> Option<CompactV1RequestRoute> {
    openkache_protocol::compat_v1::route_for_opcode(opcode).map(|route| match route {
        OperationCompactV1Route::Item => CompactV1RequestRoute::Item,
        OperationCompactV1Route::Set => CompactV1RequestRoute::Set,
        OperationCompactV1Route::Namespace => CompactV1RequestRoute::Namespace,
        OperationCompactV1Route::NamespaceOpen => CompactV1RequestRoute::NamespaceOpen,
        OperationCompactV1Route::NamespaceUpdatePolicy => {
            CompactV1RequestRoute::NamespaceUpdatePolicy
        }
        OperationCompactV1Route::NamespaceDelete => CompactV1RequestRoute::NamespaceDelete,
    })
}

/// Predicate used by the protocol adapter registry.
pub(super) fn compatibility_route_for_opcode(opcode: Opcode) -> bool {
    compatibility_route(opcode).is_some()
}

/// Returns the protocol-v1 namespace-name limit for compatibility-owned
/// persistence and request validation.
pub(super) const fn namespace_name_max_bytes() -> usize {
    NAMESPACE_NAME_MAX_BYTES
}

/// Returns whether the compact adapter carries an application value.
pub(super) fn has_value_payload(opcode: Opcode) -> bool {
    matches!(
        compatibility_route(opcode),
        Some(CompactV1RequestRoute::Set)
    )
}

/// Encodes a compact protocol-v1 prefix into an already opcode-prefixed frame.
///
/// The generic protocol facade delegates the complete historical grammar here
/// and only handles generated layouts after this function returns `false`.
pub(super) fn encode_request_prefix(
    request: &super::Request,
    output: &mut Vec<u8>,
) -> Result<bool> {
    let Some(route) = compatibility_route(request.opcode) else {
        return Ok(false);
    };
    match route {
        CompactV1RequestRoute::Item | CompactV1RequestRoute::Namespace => {
            put_namespace_id(output, request.namespace_id)?;
            if route == CompactV1RequestRoute::Item {
                for item_id in &request.item_ids {
                    output.push(item_id.len() as u8);
                    output.extend_from_slice(item_id.as_bytes());
                }
            }
        }
        CompactV1RequestRoute::Set => {
            put_namespace_id(output, request.namespace_id)?;
            output.push(request.set_options.flags()?);
            let item_id = request
                .item_ids
                .first()
                .ok_or(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: openkache_protocol::MAX_ITEM_ID_BYTES,
                    expected_value: "any",
                })?;
            output.push(item_id.len() as u8);
            let (encoded, length) = super::encode_varuint(request.value.len() as u64);
            output.extend_from_slice(&encoded[..length]);
            if let Some(ttl_ms) = request.set_options.ttl_ms {
                let (encoded, length) = super::encode_varuint(ttl_ms);
                output.extend_from_slice(&encoded[..length]);
            }
            output.extend_from_slice(item_id.as_bytes());
        }
        CompactV1RequestRoute::NamespaceOpen => {
            output.push(if request.create_if_missing {
                OPEN_CREATE_IF_MISSING
            } else {
                0
            });
            let name =
                request
                    .namespace_name
                    .as_deref()
                    .ok_or(ProtocolError::InvalidNamespaceName(
                        "namespace-open name is missing",
                    ))?;
            output.push(u8::try_from(name.len()).map_err(|_| {
                ProtocolError::InvalidNamespaceName("namespace name exceeds 255 octets")
            })?);
            output.extend_from_slice(name);
            if request.create_if_missing {
                output.extend_from_slice(
                    &request
                        .namespace_policy
                        .ok_or(ProtocolError::MissingNamespacePolicy)?
                        .encode()?,
                );
            }
        }
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            put_namespace_id(output, request.namespace_id)?;
            put_revision(output, request.expected_revision)?;
            output.extend_from_slice(
                &request
                    .namespace_policy
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .encode()?,
            );
        }
        CompactV1RequestRoute::NamespaceDelete => {
            output.push(DELETE_IF_EMPTY);
            put_namespace_id(output, request.namespace_id)?;
            put_revision(output, request.expected_revision)?;
        }
    }
    Ok(true)
}

/// Decodes the header for one opcode already classified as compatibility.
pub(super) fn decode_header(
    prefix: &[u8],
    opcode: Opcode,
    adapter: super::RequestAdapter,
) -> Result<Option<RequestHeader>> {
    let route = compatibility_route(opcode).ok_or(ProtocolError::InvalidFieldSequence(
        "opcode has no protocol-v1 compatibility route",
    ))?;
    match route {
        CompactV1RequestRoute::Item => {
            let item_id_count = contract::operation_field_count(
                opcode,
                contract::OperationFieldDirection::Request,
                contract::OperationFieldRole::ItemId,
            );
            let fixed = openkache_protocol::OPCODE_BYTES
                + openkache_protocol::NAMESPACE_ID_BYTES;
            if prefix.len() < fixed {
                return Ok(None);
            }
            let namespace_id = read_namespace_id(&prefix[openkache_protocol::OPCODE_BYTES..])?;
            let mut cursor = fixed;
            let mut item_id_lengths = [0; 2];
            for item_id_length in item_id_lengths.iter_mut().take(item_id_count) {
                let Some(&length) = prefix.get(cursor) else {
                    return Ok(None);
                };
                if usize::from(length) > openkache_protocol::MAX_ITEM_ID_BYTES {
                    return Err(ProtocolError::InvalidItemIdLength {
                        opcode,
                        expected: openkache_protocol::MAX_ITEM_ID_BYTES,
                        actual: usize::from(length),
                    });
                }
                cursor += 1;
                let end = cursor
                    .checked_add(usize::from(length))
                    .ok_or(ProtocolError::FrameLengthOverflow)?;
                if prefix.len() < end {
                    return Ok(None);
                }
                *item_id_length = length;
                cursor = end;
            }
            Ok(Some(RequestHeader::compatibility(
                adapter,
                opcode,
                cursor,
                0,
                Some(namespace_id),
                Some(fixed),
                item_id_count,
                item_id_lengths,
                SetOptions::NONE,
                false,
            )))
        }
        CompactV1RequestRoute::Namespace => {
            let required =
                openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES;
            if prefix.len() < required {
                return Ok(None);
            }
            let namespace_id = read_namespace_id(&prefix[openkache_protocol::OPCODE_BYTES..])?;
            Ok(Some(RequestHeader::compatibility(
                adapter,
                opcode,
                required,
                0,
                Some(namespace_id),
                None,
                0,
                [0; 2],
                SetOptions::NONE,
                false,
            )))
        }
        CompactV1RequestRoute::Set => decode_set_header(prefix, adapter),
        CompactV1RequestRoute::NamespaceOpen => decode_namespace_open_header(prefix, adapter),
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            decode_namespace_update_header(prefix, adapter)
        }
        CompactV1RequestRoute::NamespaceDelete => decode_namespace_delete_header(prefix, adapter),
    }
}

/// Materializes the semantic request owned by the protocol-v1 projection.
///
/// The compatibility adapter is the only request decoder that turns compact
/// namespace/item/SET bytes into the historical [`Request`] facade. Generic
/// framing uses a different callback and never enters this path.
pub(super) fn decode_request(frame: &[u8], header: RequestHeader) -> Result<super::Request> {
    let metadata = decode_request_metadata(frame, header)?;
    let value = frame[header.encoded_len..].to_vec();
    super::Request::from_decoded_parts(metadata, value, header.opcode)
}

/// Materializes a protocol-v1 request while reusing the frame allocation for
/// the optional SET value.
pub(super) fn decode_owned_request(
    mut frame: Vec<u8>,
    header: RequestHeader,
) -> Result<super::Request> {
    let metadata = decode_request_metadata(&frame, header)?;
    let value = if has_value_payload(header.opcode) {
        frame.copy_within(header.encoded_len.., 0);
        frame.truncate(header.value_len);
        frame
    } else {
        Vec::new()
    };
    super::Request::from_decoded_parts(metadata, value, header.opcode)
}

/// Converts the compatibility projection into the server's adapter-neutral
/// request envelope. The dispatcher does not need to know this route family.
pub(super) fn decode_server_request(
    frame: Vec<u8>,
    header: RequestHeader,
) -> Result<super::ServerRequest> {
    Ok(super::ServerRequest::from_request(decode_owned_request(
        frame, header,
    )?))
}

// These tables are the compatibility adapter's only knowledge of the
// historical namespace/item/SET byte prefixes.
const COMPACT_V1_ITEM_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[
        WireRequestStep::Fixed {
            bytes: openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES,
        },
        WireRequestStep::ByteLengthPrefix { slot: 0 },
        WireRequestStep::ByteLengthBody { slot: 0 },
    ],
};
const COMPACT_V1_ITEM_PAIR_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[
        WireRequestStep::Fixed {
            bytes: openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES,
        },
        WireRequestStep::ByteLengthPrefix { slot: 0 },
        WireRequestStep::ByteLengthBody { slot: 0 },
        WireRequestStep::ByteLengthPrefix { slot: 1 },
        WireRequestStep::ByteLengthBody { slot: 1 },
    ],
};
const COMPACT_V1_SET_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[
        WireRequestStep::Fixed {
            bytes: openkache_protocol::OPCODE_BYTES
                + openkache_protocol::NAMESPACE_ID_BYTES
                + SET_FLAGS_BYTES,
        },
        WireRequestStep::ByteLengthPrefix { slot: 0 },
        WireRequestStep::ValueLengthPrefix,
        WireRequestStep::ConditionalVarUInt {
            selector_offset: openkache_protocol::OPCODE_BYTES
                + openkache_protocol::NAMESPACE_ID_BYTES,
            mask: SET_EXPIRATION_MASK,
            expected: SET_EXPLICIT_TTL_BITS,
        },
        WireRequestStep::ByteLengthBody { slot: 0 },
    ],
};
const COMPACT_V1_NAMESPACE_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[WireRequestStep::Fixed {
        bytes: openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES,
    }],
};
const COMPACT_V1_NAMESPACE_OPEN_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[
        WireRequestStep::Fixed {
            bytes: openkache_protocol::OPCODE_BYTES + contract::OPEN_FLAGS_BYTES,
        },
        WireRequestStep::ByteLength,
        WireRequestStep::ConditionalByteThenVarUInt {
            selector_offset: openkache_protocol::OPCODE_BYTES,
            mask: contract::OPEN_CREATE_IF_MISSING,
            expected: contract::OPEN_CREATE_IF_MISSING,
            prefix_bytes: contract::POLICY_FLAGS_BYTES,
            value_mask: contract::POLICY_DEFAULT_EXPIRATION_MASK,
            value_expected: contract::POLICY_FIXED_TTL,
        },
    ],
};
const COMPACT_V1_NAMESPACE_POLICY_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[
        WireRequestStep::Fixed {
            bytes: openkache_protocol::OPCODE_BYTES
                + openkache_protocol::NAMESPACE_ID_BYTES
                + openkache_protocol::NAMESPACE_REVISION_BYTES,
        },
        WireRequestStep::ByteThenVarUInt {
            prefix_bytes: contract::POLICY_FLAGS_BYTES,
            mask: contract::POLICY_DEFAULT_EXPIRATION_MASK,
            expected: contract::POLICY_FIXED_TTL,
        },
    ],
};
const COMPACT_V1_NAMESPACE_DELETE_LAYOUT: WireRequestLayout = WireRequestLayout {
    steps: &[WireRequestStep::Fixed {
        bytes: openkache_protocol::OPCODE_BYTES
            + contract::DELETE_FLAGS_BYTES
            + openkache_protocol::NAMESPACE_ID_BYTES
            + openkache_protocol::NAMESPACE_REVISION_BYTES,
    }],
};

pub(super) fn request_frame_layout(opcode: Opcode) -> WireResult<WireRequestLayout> {
    let route = compatibility_route(opcode).ok_or_else(|| {
        openkache_protocol::ProtocolError::InvalidFieldSequence(
            "compact operation has no protocol-v1 route",
        )
    })?;
    match route {
        CompactV1RequestRoute::Item => {
            let item_count = contract::operation_field_count(
                opcode,
                contract::OperationFieldDirection::Request,
                contract::OperationFieldRole::ItemId,
            );
            match item_count {
                1 => Ok(COMPACT_V1_ITEM_LAYOUT),
                2 => Ok(COMPACT_V1_ITEM_PAIR_LAYOUT),
                // Protocol-v1 only published one- and two-item compact
                // routes. Future batches must use a generated repeated-field
                // layout instead of silently being parsed as one item.
                _ => Err(openkache_protocol::ProtocolError::InvalidFieldSequence(
                    "protocol-v1 item route supports one or two item IDs",
                )),
            }
        }
        CompactV1RequestRoute::Set => Ok(COMPACT_V1_SET_LAYOUT),
        CompactV1RequestRoute::Namespace => Ok(COMPACT_V1_NAMESPACE_LAYOUT),
        CompactV1RequestRoute::NamespaceOpen => Ok(COMPACT_V1_NAMESPACE_OPEN_LAYOUT),
        CompactV1RequestRoute::NamespaceUpdatePolicy => Ok(COMPACT_V1_NAMESPACE_POLICY_LAYOUT),
        CompactV1RequestRoute::NamespaceDelete => Ok(COMPACT_V1_NAMESPACE_DELETE_LAYOUT),
    }
}

pub(super) fn decode_request_metadata(
    frame: &[u8],
    header: RequestHeader,
) -> Result<DecodedRequestMetadata> {
    let Some(route) = compatibility_route(header.opcode) else {
        return Err(ProtocolError::InvalidFieldSequence(
            "generic operation entered the compatibility metadata adapter",
        ));
    };
    let item_ids = header
        .item_id_start()
        .map(|start| {
            (0..header.item_id_count())
                .map(|index| {
                    let item_start = start + index * openkache_protocol::ITEM_ID_BYTES;
                    ItemId::new(
                        frame[item_start..item_start + openkache_protocol::ITEM_ID_BYTES]
                            .try_into()
                            .expect("validated item ID range"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let namespace_name = if route == CompactV1RequestRoute::NamespaceOpen {
        let name_start = openkache_protocol::OPCODE_BYTES
            + OPEN_FLAGS_BYTES
            + openkache_protocol::NAMESPACE_NAME_LENGTH_BYTES;
        let name_len_offset = openkache_protocol::OPCODE_BYTES + OPEN_FLAGS_BYTES;
        Some(frame[name_start..name_start + usize::from(frame[name_len_offset])].to_vec())
    } else {
        None
    };
    let (namespace_policy, expected_revision, create_if_missing) = match route {
        CompactV1RequestRoute::NamespaceOpen => {
            let flags_offset = openkache_protocol::OPCODE_BYTES;
            let name_len_offset = flags_offset + OPEN_FLAGS_BYTES;
            let name_start = name_len_offset + openkache_protocol::NAMESPACE_NAME_LENGTH_BYTES;
            let create = frame[flags_offset] & OPEN_CREATE_IF_MISSING != 0;
            let policy = if create {
                let start = name_start + usize::from(frame[name_len_offset]);
                Some(
                    decode_namespace_policy(&frame[start..])?
                        .ok_or(ProtocolError::MissingNamespacePolicy)?
                        .0,
                )
            } else {
                None
            };
            (policy, None, create)
        }
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            let revision_start =
                openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES;
            let revision = super::read_u64_be(&frame[revision_start..])?;
            let start = revision_start + openkache_protocol::NAMESPACE_REVISION_BYTES;
            let policy = Some(
                decode_namespace_policy(&frame[start..])?
                    .ok_or(ProtocolError::MissingNamespacePolicy)?
                    .0,
            );
            (policy, Some(revision), false)
        }
        CompactV1RequestRoute::NamespaceDelete => (
            None,
            Some(super::read_u64_be(
                &frame[openkache_protocol::OPCODE_BYTES
                    + DELETE_FLAGS_BYTES
                    + openkache_protocol::NAMESPACE_ID_BYTES..],
            )?),
            false,
        ),
        _ => (None, None, false),
    };
    Ok(DecodedRequestMetadata {
        namespace_id: header.namespace_id(),
        item_ids,
        set_options: header.set_options(),
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
    })
}

pub(super) fn decode_set_header(
    prefix: &[u8],
    adapter: super::RequestAdapter,
) -> Result<Option<RequestHeader>> {
    let fixed = openkache_protocol::OPCODE_BYTES
        + openkache_protocol::NAMESPACE_ID_BYTES
        + SET_FLAGS_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let namespace_id = read_namespace_id(&prefix[openkache_protocol::OPCODE_BYTES..])?;
    let flags_offset = openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES;
    let flags = prefix[flags_offset];
    if flags & SET_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            flags & SET_RESERVED_MASK,
        ));
    }
    if flags & SET_CONDITION_MASK == SET_CONDITION_RESERVED_BITS {
        return Err(ProtocolError::ConflictingSetConditions);
    }
    let has_ttl = matches!(flags & SET_EXPIRATION_MASK, SET_EXPLICIT_TTL_BITS);
    if !matches!(
        flags & SET_EXPIRATION_MASK,
        SET_INHERIT_EXPIRATION_BITS | SET_NO_EXPIRY_BITS | SET_EXPLICIT_TTL_BITS
    ) {
        return Err(ProtocolError::InvalidSetOptions {
            opcode: Opcode::Set,
        });
    }
    if !matches!(
        flags & SET_EVICTION_MASK,
        SET_INHERIT_EVICTION_BITS | SET_EVICTABLE_BITS | SET_EVICTION_PROTECTED_BITS
    ) {
        return Err(ProtocolError::InvalidSetOptions {
            opcode: Opcode::Set,
        });
    }
    let item_id_length_offset = fixed;
    let Some(&item_id_length) = prefix.get(item_id_length_offset) else {
        return Ok(None);
    };
    if usize::from(item_id_length) > openkache_protocol::MAX_ITEM_ID_BYTES {
        return Err(ProtocolError::InvalidItemIdLength {
            opcode: Opcode::Set,
            expected: openkache_protocol::MAX_ITEM_ID_BYTES,
            actual: usize::from(item_id_length),
        });
    }
    let item_id_start = item_id_length_offset + 1;
    let item_id_end = item_id_start
        .checked_add(usize::from(item_id_length))
        .ok_or(ProtocolError::FrameLengthOverflow)?;
    if prefix.len() < item_id_end {
        return Ok(None);
    }
    let mut cursor = item_id_end;
    let ttl_ms = if has_ttl {
        let Some((ttl, length)) = super::decode_varuint(&prefix[cursor..], "SET TTL")? else {
            return Ok(None);
        };
        if ttl == 0 {
            return Err(ProtocolError::InvalidSetTtl);
        }
        cursor += length;
        Some(ttl)
    } else {
        None
    };
    let Some((value_len, value_len_bytes)) =
        super::decode_varuint(&prefix[cursor..], "SET value length")?
    else {
        return Ok(None);
    };
    let value_len = usize::try_from(value_len).map_err(|_| ProtocolError::FrameLengthOverflow)?;
    super::validate_value_length(value_len)?;
    let set_options = SetOptions::decode_set_options(flags, ttl_ms)?;
    Ok(Some(RequestHeader::compatibility(
        adapter,
        Opcode::Set,
        cursor + value_len_bytes,
        value_len,
        Some(namespace_id),
        Some(item_id_start),
        1,
        [item_id_length, 0],
        set_options,
        has_ttl,
    )))
}

pub(super) fn decode_namespace_open_header(
    prefix: &[u8],
    adapter: super::RequestAdapter,
) -> Result<Option<RequestHeader>> {
    let fixed = openkache_protocol::OPCODE_BYTES
        + OPEN_FLAGS_BYTES
        + openkache_protocol::NAMESPACE_NAME_LENGTH_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let flags = prefix[openkache_protocol::OPCODE_BYTES];
    if flags & OPEN_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            flags & OPEN_RESERVED_MASK,
        ));
    }
    let name_len_offset = openkache_protocol::OPCODE_BYTES + OPEN_FLAGS_BYTES;
    let name_start = fixed;
    let name_len = usize::from(prefix[name_len_offset]);
    let name_end = name_start + name_len;
    if prefix.len() < name_end {
        return Ok(None);
    }
    validate_namespace_name(&prefix[name_start..name_end])?;
    let create = flags & OPEN_CREATE_IF_MISSING != 0;
    let encoded_len = if create {
        let Some((_, policy_len)) = decode_namespace_policy(&prefix[name_end..])? else {
            return Ok(None);
        };
        name_end + policy_len
    } else {
        name_end
    };
    Ok(Some(RequestHeader::compatibility(
        adapter,
        Opcode::NamespaceOpen,
        encoded_len,
        0,
        None,
        None,
        0,
        [0; 2],
        SetOptions::NONE,
        false,
    )))
}

pub(super) fn decode_namespace_update_header(
    prefix: &[u8],
    adapter: super::RequestAdapter,
) -> Result<Option<RequestHeader>> {
    let fixed = openkache_protocol::OPCODE_BYTES
        + openkache_protocol::NAMESPACE_ID_BYTES
        + openkache_protocol::NAMESPACE_REVISION_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let namespace_id = read_namespace_id(&prefix[openkache_protocol::OPCODE_BYTES..])?;
    let expected_revision = super::read_u64_be(
        &prefix[openkache_protocol::OPCODE_BYTES + openkache_protocol::NAMESPACE_ID_BYTES..],
    )?;
    if expected_revision == 0 {
        return Err(ProtocolError::InvalidRevision);
    }
    let Some((_, policy_len)) = decode_namespace_policy(&prefix[fixed..])? else {
        return Ok(None);
    };
    Ok(Some(RequestHeader::compatibility(
        adapter,
        Opcode::NamespaceUpdatePolicy,
        fixed + policy_len,
        0,
        Some(namespace_id),
        None,
        0,
        [0; 2],
        SetOptions::NONE,
        false,
    )))
}

pub(super) fn decode_namespace_delete_header(
    prefix: &[u8],
    adapter: super::RequestAdapter,
) -> Result<Option<RequestHeader>> {
    let fixed = openkache_protocol::OPCODE_BYTES
        + DELETE_FLAGS_BYTES
        + openkache_protocol::NAMESPACE_ID_BYTES
        + openkache_protocol::NAMESPACE_REVISION_BYTES;
    if prefix.len() < fixed {
        return Ok(None);
    }
    let flags_offset = openkache_protocol::OPCODE_BYTES;
    if prefix[flags_offset] & DELETE_MODE_MASK != DELETE_IF_EMPTY {
        return Err(ProtocolError::UnknownRequestFlags(prefix[flags_offset]));
    }
    if prefix[flags_offset] & DELETE_RESERVED_MASK != 0 {
        return Err(ProtocolError::UnknownRequestFlags(
            prefix[flags_offset] & DELETE_RESERVED_MASK,
        ));
    }
    let namespace_id = read_namespace_id(&prefix[flags_offset + DELETE_FLAGS_BYTES..])?;
    let expected_revision = super::read_u64_be(
        &prefix[flags_offset + DELETE_FLAGS_BYTES + openkache_protocol::NAMESPACE_ID_BYTES..],
    )?;
    if expected_revision == 0 {
        return Err(ProtocolError::InvalidRevision);
    }
    Ok(Some(RequestHeader::compatibility(
        adapter,
        Opcode::NamespaceDelete,
        fixed,
        0,
        Some(namespace_id),
        None,
        0,
        [0; 2],
        SetOptions::NONE,
        false,
    )))
}

pub(super) fn validate_namespace_name(name: &[u8]) -> Result<()> {
    if name.len() > NAMESPACE_NAME_MAX_BYTES {
        return Err(ProtocolError::InvalidNamespaceName(
            "namespace name exceeds 255 octets",
        ));
    }
    std::str::from_utf8(name)
        .map_err(|_| ProtocolError::InvalidNamespaceName("namespace name is not UTF-8"))?;
    Ok(())
}

pub(super) fn validate_namespace_id(namespace_id: Option<u64>) -> Result<u64> {
    match namespace_id {
        Some(namespace_id @ 1..) => Ok(namespace_id),
        Some(0) => Err(ProtocolError::InvalidNamespaceId),
        None => Err(ProtocolError::MissingNamespaceId),
    }
}

pub(super) fn validate_revision(revision: Option<u64>) -> Result<u64> {
    match revision {
        Some(revision @ 1..) => Ok(revision),
        _ => Err(ProtocolError::InvalidRevision),
    }
}

pub(super) fn put_namespace_id(output: &mut Vec<u8>, namespace_id: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_namespace_id(namespace_id)?.to_be_bytes());
    Ok(())
}

pub(super) fn put_revision(output: &mut Vec<u8>, revision: Option<u64>) -> Result<()> {
    output.extend_from_slice(&validate_revision(revision)?.to_be_bytes());
    Ok(())
}

pub(super) fn read_namespace_id(input: &[u8]) -> Result<u64> {
    let value = super::read_u64_be(input)?;
    if value == 0 {
        return Err(ProtocolError::InvalidNamespaceId);
    }
    Ok(value)
}

pub(super) fn validate_request(request: &super::Request) -> Result<()> {
    super::validate_value_length(request.value.len())?;
    let Some(route) = compatibility_route(request.opcode) else {
        return Err(ProtocolError::InvalidFieldSequence(
            "operation has no protocol-v1 compatibility route",
        ));
    };
    match route {
        CompactV1RequestRoute::Item => {
            validate_namespace_id(request.namespace_id)?;
            let expected_item_count = contract::operation_field_count(
                request.opcode,
                contract::OperationFieldDirection::Request,
                contract::OperationFieldRole::ItemId,
            );
            if request.item_ids.len() != expected_item_count
                || request.set_options != SetOptions::NONE
                || !request.value.is_empty()
                || request.namespace_name.is_some()
                || request.namespace_policy.is_some()
                || request.expected_revision.is_some()
                || request.create_if_missing
            {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: openkache_protocol::ITEM_ID_BYTES * expected_item_count,
                    expected_value: "0",
                });
            }
        }
        CompactV1RequestRoute::Namespace => {
            validate_namespace_id(request.namespace_id)?;
            if !request.item_ids.is_empty()
                || request.set_options != SetOptions::NONE
                || !request.value.is_empty()
                || request.namespace_name.is_some()
                || request.namespace_policy.is_some()
                || request.expected_revision.is_some()
                || request.create_if_missing
            {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: 0,
                    expected_value: "0",
                });
            }
        }
        CompactV1RequestRoute::Set => {
            validate_namespace_id(request.namespace_id)?;
            if request.item_ids.len() != 1 {
                return Err(ProtocolError::InvalidItemIdLength {
                    opcode: request.opcode,
                    expected: openkache_protocol::ITEM_ID_BYTES,
                    actual: 0,
                });
            }
            request.set_options.flags()?;
            if request.namespace_name.is_some()
                || request.namespace_policy.is_some()
                || request.expected_revision.is_some()
                || request.create_if_missing
            {
                return Err(ProtocolError::InvalidSetOptions {
                    opcode: request.opcode,
                });
            }
        }
        CompactV1RequestRoute::NamespaceOpen => {
            let name =
                request
                    .namespace_name
                    .as_deref()
                    .ok_or(ProtocolError::InvalidNamespaceName(
                        "namespace name missing",
                    ))?;
            validate_namespace_name(name)?;
            if request.create_if_missing != request.namespace_policy.is_some() {
                return Err(if request.create_if_missing {
                    ProtocolError::MissingNamespacePolicy
                } else {
                    ProtocolError::UnexpectedNamespacePolicy
                });
            }
            if request.expected_revision.is_some()
                || request.namespace_id.is_some()
                || !request.item_ids.is_empty()
                || request.set_options != SetOptions::NONE
                || !request.value.is_empty()
            {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: 0,
                    expected_value: "0",
                });
            }
            if let Some(policy) = request.namespace_policy {
                policy.encode()?;
            }
        }
        CompactV1RequestRoute::NamespaceUpdatePolicy => {
            validate_namespace_id(request.namespace_id)?;
            validate_revision(request.expected_revision)?;
            request
                .namespace_policy
                .ok_or(ProtocolError::MissingNamespacePolicy)?
                .encode()?;
            if !request.item_ids.is_empty()
                || request.set_options != SetOptions::NONE
                || !request.value.is_empty()
                || request.namespace_name.is_some()
                || request.create_if_missing
            {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: 0,
                    expected_value: "0",
                });
            }
        }
        CompactV1RequestRoute::NamespaceDelete => {
            validate_namespace_id(request.namespace_id)?;
            validate_revision(request.expected_revision)?;
            if !request.item_ids.is_empty()
                || request.set_options != SetOptions::NONE
                || !request.value.is_empty()
                || request.namespace_name.is_some()
                || request.namespace_policy.is_some()
                || request.create_if_missing
            {
                return Err(ProtocolError::InvalidRequestShape {
                    opcode: request.opcode,
                    expected_item_id: 0,
                    expected_value: "0",
                });
            }
        }
    }
    Ok(())
}
