//! Registration table for the operation-neutral example APIs.
//!
//! The sibling binding module owns field decoding, behavior, and capabilities.
//! This module contains only the composition slice that selects handlers and
//! their generic lifecycle policy.

use openkache_protocol::Opcode;

use super::operation_api::{ApiModule, RegistrationBuilder};
use super::operation_generic_handlers::{
    acknowledge_handler_async, dense_handler_async, echo_handler_async,
    multi_resource_mutation_handler, page_handler_async, ping_handler_async,
    prepare_multi_resource_mutation, reverse_handler_async, square_array_handler_async,
    storage_read_handler,
};
use super::operation_generic_resources::install_capabilities;

pub(super) const API: ApiModule = ApiModule::new(&[
    RegistrationBuilder::generic(Opcode::Ping, ping_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(Opcode::ExperimentalEcho, echo_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(Opcode::ExperimentalReverse, reverse_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(Opcode::SquareArray, square_array_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(
        Opcode::ExperimentalAcknowledge,
        acknowledge_handler_async,
    )
    .read_only()
    .build(),
    RegistrationBuilder::generic(Opcode::ExperimentalDense, dense_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(Opcode::ExperimentalStorageRead, storage_read_handler)
        .read_only()
        .build(),
    RegistrationBuilder::generic(Opcode::ExperimentalPage, page_handler_async)
        .read_only()
        .build(),
    RegistrationBuilder::generic(
        Opcode::ExperimentalMultiResourceMutation,
        multi_resource_mutation_handler,
    )
    .prepare(prepare_multi_resource_mutation)
    .mutation()
    .build(),
])
.with_capabilities(install_capabilities);
