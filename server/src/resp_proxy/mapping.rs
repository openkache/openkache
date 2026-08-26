use std::borrow::Cow;
use std::io;

use openkache_protocol::compat_v1::{
    POLICY_EVICTION_OVERRIDE, POLICY_EXPIRATION_OVERRIDE, POLICY_NO_EXPIRY,
};
use openkache_protocol::request_fields::{op_delete, op_get, op_set};
use openkache_protocol::{
    MAX_OPERATION_REQUEST_FIELDS, Opcode, RequestFieldProjection, Response, Status,
    project_request_frame, wire_request_layout,
};

use super::resp_backend::RespBackend;

const GATE0_NAMESPACE_ID: u64 = 1;
const GATE0_NAMESPACE_REVISION: u64 = 1;

pub(super) async fn dispatch(frame: &[u8], backend: &mut RespBackend) -> io::Result<Response> {
    let opcode = Opcode::try_from(
        frame
            .first()
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty native request"))?,
    )
    .map_err(io::Error::other)?;
    let layout = wire_request_layout(opcode);
    let mut fields = [RequestFieldProjection::Missing; MAX_OPERATION_REQUEST_FIELDS];
    let header = project_request_frame(frame, layout, &mut fields).map_err(io::Error::other)?;
    let request_id = header.request_id();

    match opcode {
        Opcode::Ping => response(Status::Ok, request_id, b"PONG".to_vec()),
        Opcode::Get => {
            require_gate0_namespace(frame, fields[op_get::NAMESPACE_ID])?;
            let key = field_bytes(frame, fields[op_get::ITEM_ID])?;
            match backend.get(&key).await? {
                Some(value) => response(Status::Ok, request_id, value),
                None => response(Status::NotFound, request_id, Vec::new()),
            }
        }
        Opcode::Set => {
            require_gate0_namespace(frame, fields[op_set::NAMESPACE_ID])?;
            require_default_set_options(&fields)?;
            let key = field_bytes(frame, fields[op_set::ITEM_ID])?;
            let value = field_bytes(frame, fields[op_set::VALUE])?;
            let existed = backend.get(&key).await?.is_some();
            backend.set(&key, &value).await?;
            response(
                if existed {
                    Status::Replaced
                } else {
                    Status::Created
                },
                request_id,
                Vec::new(),
            )
        }
        Opcode::Delete => {
            require_gate0_namespace(frame, fields[op_delete::NAMESPACE_ID])?;
            let key = field_bytes(frame, fields[op_delete::ITEM_ID])?;
            let existed = backend.delete(&key).await?;
            response(
                if existed {
                    Status::Deleted
                } else {
                    Status::NotFound
                },
                request_id,
                Vec::new(),
            )
        }
        Opcode::NamespaceOpen => response(Status::Ok, request_id, synthetic_namespace_descriptor()),
        Opcode::ExperimentalStats
        | Opcode::ExperimentalSync
        | Opcode::NamespaceUpdatePolicy
        | Opcode::NamespaceDelete => response(
            Status::UnsupportedOpcode,
            request_id,
            b"operation is unavailable in the RESP-backed prototype".to_vec(),
        ),
    }
}

fn response(status: Status, request_id: u64, payload: Vec<u8>) -> io::Result<Response> {
    Response::new_with_id(status, request_id, payload).map_err(io::Error::other)
}

fn require_gate0_namespace(frame: &[u8], projection: RequestFieldProjection) -> io::Result<()> {
    let namespace = field_bytes(frame, projection)?;
    if namespace.as_ref() == GATE0_NAMESPACE_ID.to_be_bytes() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "only Gate 0 namespace ID 1 is supported",
        ))
    }
}

fn require_default_set_options(
    fields: &[RequestFieldProjection; MAX_OPERATION_REQUEST_FIELDS],
) -> io::Result<()> {
    let valid = projection_equals(fields[op_set::CONDITION], b"any")
        && projection_equals(fields[op_set::EXPIRATION_MODE], b"inherit")
        && projection_equals(fields[op_set::EVICTION_MODE], b"inherit")
        && matches!(
            fields[op_set::TTL_MILLISECONDS],
            RequestFieldProjection::Missing
        );
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the RESP-backed prototype supports only unconditional SET without TTL overrides",
        ))
    }
}

fn projection_equals(projection: RequestFieldProjection, expected: &[u8]) -> bool {
    match projection {
        RequestFieldProjection::Static(bytes) => bytes == expected,
        RequestFieldProjection::Inline(bytes) => bytes.as_slice() == expected,
        RequestFieldProjection::Missing | RequestFieldProjection::Borrowed { .. } => false,
    }
}

fn field_bytes<'a>(
    frame: &'a [u8],
    projection: RequestFieldProjection,
) -> io::Result<Cow<'a, [u8]>> {
    match projection {
        RequestFieldProjection::Borrowed { start, end } => frame
            .get(start..end)
            .map(Cow::Borrowed)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid field range")),
        RequestFieldProjection::Inline(bytes) => Ok(Cow::Owned(bytes.to_vec())),
        RequestFieldProjection::Static(bytes) => Ok(Cow::Borrowed(bytes)),
        RequestFieldProjection::Missing => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "required native request field is missing",
        )),
    }
}

fn synthetic_namespace_descriptor() -> Vec<u8> {
    let policy = POLICY_NO_EXPIRY | POLICY_EXPIRATION_OVERRIDE | POLICY_EVICTION_OVERRIDE;
    let mut descriptor = Vec::with_capacity(17);
    descriptor.extend_from_slice(&GATE0_NAMESPACE_ID.to_be_bytes());
    descriptor.extend_from_slice(&GATE0_NAMESPACE_REVISION.to_be_bytes());
    descriptor.push(policy);
    descriptor
}
