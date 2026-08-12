//! Client request projection registry.
//!
//! The request executor exposes one neutral body/field boundary. This small
//! registry is the composition point that selects the generic projection or
//! the historical protocol-v1 compatibility projection for an opcode. Keeping
//! the table outside `protocol.rs` prevents the semantic request facade from
//! becoming a second wire-family dispatcher.

use super::{Opcode, Request, compat_v1, generic};

type ContractRequestFn =
    fn(Opcode, Option<u64>, &[u8], Vec<u8>, crate::SetOptions) -> crate::Result<Request>;
type UnaryRequestFn = fn(Opcode, Vec<u8>) -> crate::Result<Request>;
type FieldsRequestFn = fn(Opcode, Vec<Option<Vec<u8>>>) -> crate::Result<Request>;

#[derive(Clone, Copy)]
struct ClientRequestAdapter {
    accepts: fn(Opcode) -> bool,
    request_from_contract: ContractRequestFn,
    request_from_unary: UnaryRequestFn,
    request_from_fields: FieldsRequestFn,
}

fn accepts_any_operation(_operation: Opcode) -> bool {
    true
}

fn reject_compatibility_unary(_operation: Opcode, _body: Vec<u8>) -> crate::Result<Request> {
    Err(crate::Error::configuration(
        "operation",
        "generic unary requests cannot use the compact protocol-v1 adapter",
    ))
}

fn reject_compatibility_fields(
    _operation: Opcode,
    _fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    Err(crate::Error::configuration(
        "operation",
        "protocol-v1 compatibility operations require their typed adapter",
    ))
}

fn generic_request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: crate::SetOptions,
) -> crate::Result<Request> {
    if namespace_id.is_some() || !item_id.is_empty() {
        return Err(crate::Error::configuration(
            "operation",
            "generic requests cannot carry namespace or item identity",
        ));
    }
    if set_options != crate::SetOptions::new() {
        return Err(crate::Error::configuration(
            "set_options",
            "generic requests cannot carry mutation options",
        ));
    }
    generic::request_from_contract_body(operation, value)
}

const GENERIC_REQUEST_ADAPTER: ClientRequestAdapter = ClientRequestAdapter {
    accepts: accepts_any_operation,
    request_from_contract: generic_request_from_contract,
    request_from_unary: generic::request_from_unary,
    request_from_fields: generic::request_from_fields,
};

const COMPATIBILITY_REQUEST_ADAPTER: ClientRequestAdapter = ClientRequestAdapter {
    accepts: compat_v1::is_compatibility_operation,
    request_from_contract: compat_v1::request_from_contract,
    request_from_unary: reject_compatibility_unary,
    request_from_fields: reject_compatibility_fields,
};

const REQUEST_ADAPTERS: &[ClientRequestAdapter] =
    &[COMPATIBILITY_REQUEST_ADAPTER, GENERIC_REQUEST_ADAPTER];

fn request_adapter(operation: Opcode) -> &'static ClientRequestAdapter {
    REQUEST_ADAPTERS
        .iter()
        .find(|adapter| (adapter.accepts)(operation))
        .expect("client request adapter registry must have a generic fallback")
}

/// Builds a request through the generated contract's selected projection.
pub(crate) fn request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: crate::SetOptions,
) -> crate::Result<Request> {
    (request_adapter(operation).request_from_contract)(
        operation,
        namespace_id,
        item_id,
        value,
        set_options,
    )
}

/// Builds a request for a generic unary operation from its encoded body.
pub(crate) fn request_from_unary(operation: Opcode, body: Vec<u8>) -> crate::Result<Request> {
    (request_adapter(operation).request_from_unary)(operation, body)
}

/// Builds an ordered-field request from generated field values.
pub(crate) fn request_from_fields(
    operation: Opcode,
    fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    (request_adapter(operation).request_from_fields)(operation, fields)
}
