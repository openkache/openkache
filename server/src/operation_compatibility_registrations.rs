//! Registration table for the protocol-v1 compatibility API module.
//!
//! Decoding and typed behavior stay in
//! [`super::operation_compatibility_bindings`]. Keeping the composition slice
//! here makes the adapter's registration policy visible without mixing it into
//! field projection code.

use openkache_protocol::Opcode;

use super::operation_api::{ApiModule, RegistrationBuilder};
use super::operation_handlers;

pub(super) const API: ApiModule = ApiModule::new(&[
    RegistrationBuilder::generic(
        Opcode::Get,
        super::operation_compatibility_bindings::get_handler,
    )
        .prepare(super::operation_compatibility_bindings::prepare_namespace)
        .authorize(operation_handlers::authorization_none)
        .read_only()
        .build(),
    RegistrationBuilder::generic(
        Opcode::Set,
        super::operation_compatibility_bindings::set_handler,
    )
        .prepare(super::operation_compatibility_bindings::prepare_namespace)
        .authorize(operation_handlers::authorization_none)
        .mutation()
        .build(),
    RegistrationBuilder::generic(
        Opcode::Delete,
        super::operation_compatibility_bindings::delete_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::generic(
        Opcode::Stats,
        super::operation_compatibility_bindings::stats_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_namespace)
    .authorize(operation_handlers::authorization_administrator)
    .read_only()
    .build(),
    RegistrationBuilder::generic(
        Opcode::Sync,
        super::operation_compatibility_bindings::sync_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_namespace)
    .authorize(operation_handlers::authorization_administrator)
    .mutation()
    .build(),
    RegistrationBuilder::generic(
        Opcode::NamespaceOpen,
        super::operation_compatibility_bindings::namespace_open_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_lifecycle)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::generic(
        Opcode::NamespaceUpdatePolicy,
        super::operation_compatibility_bindings::namespace_update_policy_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::generic(
        Opcode::NamespaceDelete,
        super::operation_compatibility_bindings::namespace_delete_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_lifecycle_and_namespace)
    .authorize(operation_handlers::authorization_none)
    .mutation()
    .build(),
    RegistrationBuilder::generic(
        Opcode::Get2,
        super::operation_compatibility_bindings::get2_handler,
    )
    .prepare(super::operation_compatibility_bindings::prepare_namespace)
    .authorize(operation_handlers::authorization_none)
    .read_only()
    .build(),
])
.with_capabilities(super::operation_compatibility_bindings::install_capabilities);
