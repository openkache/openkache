//! Worker-owned operation registrations bound to API module state.
//!
//! API modules create opaque state once during worker startup. The runtime
//! retains that state beside each dense registration so request dispatch can
//! borrow it without a capability lookup, allocation, or reference-count bump.

use std::any::Any;
use std::sync::Arc;

use openkache_protocol::Opcode;

use super::operation_api::ServerOperationRegistration;
use super::operation_capabilities::CapabilityCatalog;

pub(super) type ErasedOperationState = dyn Any + Send + Sync;
pub(super) type StateValidator = fn(Option<&ErasedOperationState>) -> bool;

pub(super) fn no_operation_state(state: Option<&ErasedOperationState>) -> bool {
    state.is_none()
}

pub(super) fn typed_operation_state<T: Any + Send + Sync>(
    state: Option<&ErasedOperationState>,
) -> bool {
    state.is_some_and(|state| state.is::<T>())
}

#[derive(Clone)]
pub(super) struct ModuleState {
    value: Arc<ErasedOperationState>,
}

impl ModuleState {
    pub(super) fn new<T>(value: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            value: Arc::new(value),
        }
    }

    fn as_ref(&self) -> &ErasedOperationState {
        self.value.as_ref()
    }

    pub(super) fn is<T: Any>(&self) -> bool {
        self.value.is::<T>()
    }
}

#[derive(Clone, Copy)]
pub(super) struct OperationStateRef<'a> {
    value: Option<&'a (dyn Any + Send + Sync)>,
}

impl<'a> OperationStateRef<'a> {
    pub(super) fn get<T: Any>(&self) -> Option<&'a T> {
        self.value?.downcast_ref()
    }
}

struct BoundOperation {
    registration: &'static ServerOperationRegistration,
    state: Option<ModuleState>,
}

impl BoundOperation {
    fn state(&self) -> OperationStateRef<'_> {
        OperationStateRef {
            value: self.state.as_ref().map(ModuleState::as_ref),
        }
    }
}

pub(super) struct OperationRuntimeBuilder {
    operations: [Option<BoundOperation>; Opcode::COUNT],
}

impl OperationRuntimeBuilder {
    pub(super) const fn new() -> Self {
        Self {
            operations: [const { None }; Opcode::COUNT],
        }
    }

    pub(super) fn bind(
        &mut self,
        registrations: &'static [ServerOperationRegistration],
        state: Option<ModuleState>,
    ) -> Result<(), &'static str> {
        let mut seen = [false; Opcode::COUNT];
        for registration in registrations {
            if !(registration.state_validator)(state.as_ref().map(ModuleState::as_ref)) {
                return Err("operation state is missing or has the wrong type");
            }
            let index = registration.opcode.index();
            if seen[index] || self.operations[index].is_some() {
                return Err("operation runtime contains a duplicate registration");
            }
            seen[index] = true;
        }
        for registration in registrations {
            let slot = &mut self.operations[registration.opcode.index()];
            *slot = Some(BoundOperation {
                registration,
                state: state.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        capabilities: Arc<dyn CapabilityCatalog>,
    ) -> OperationRuntime {
        OperationRuntime {
            operations: self.operations,
            capabilities,
        }
    }
}

pub(super) struct OperationRuntime {
    operations: [Option<BoundOperation>; Opcode::COUNT],
    capabilities: Arc<dyn CapabilityCatalog>,
}

impl OperationRuntime {
    pub(super) fn registration(
        &self,
        opcode: Opcode,
    ) -> Option<&'static ServerOperationRegistration> {
        self.operations[opcode.index()]
            .as_ref()
            .map(|operation| operation.registration)
    }

    pub(super) fn operation(
        &self,
        opcode: Opcode,
    ) -> Option<(&'static ServerOperationRegistration, OperationStateRef<'_>)> {
        self.operations[opcode.index()]
            .as_ref()
            .map(|operation| (operation.registration, operation.state()))
    }

    pub(super) fn capabilities(&self) -> &dyn CapabilityCatalog {
        self.capabilities.as_ref()
    }
}
