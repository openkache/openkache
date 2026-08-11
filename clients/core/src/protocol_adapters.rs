//! Client construction facade over generated request plans.
//!
//! Historical typed calls use their public convenience adapter. Generic
//! field calls, including exact-plan operations, use the common generated
//! field constructor.

use super::{Opcode, Request, compat_v1, generic};

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

/// Builds a request through the generated contract's selected projection.
pub(crate) fn request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: crate::SetOptions,
) -> crate::Result<Request> {
    if compat_v1::is_compatibility_operation(operation) {
        compat_v1::request_from_contract(operation, namespace_id, item_id, value, set_options)
    } else {
        generic_request_from_contract(operation, namespace_id, item_id, value, set_options)
    }
}

/// Builds a request for a generic unary operation from its encoded body.
pub(crate) fn request_from_unary(operation: Opcode, body: Vec<u8>) -> crate::Result<Request> {
    if compat_v1::is_compatibility_operation(operation) {
        return Err(crate::Error::configuration(
            "operation",
            "generic unary requests cannot use an exact typed request plan",
        ));
    }
    generic::request_from_unary(operation, body)
}

/// Builds an ordered-field request from generated field values.
pub(crate) fn request_from_fields(
    operation: Opcode,
    fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    generic::request_from_fields(operation, fields)
}
