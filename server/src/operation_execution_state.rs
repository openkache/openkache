//! Worker-owned operation registrations bound to API module state.
//!
//! API modules install opaque state into opcode slots during worker startup.
//! Freeze validates those slots against registrations, then request dispatch
//! borrows them without a capability lookup, allocation, or reference-count
//! bump.

use std::any::Any;
use std::sync::Arc;

use openkache_protocol::Opcode;

use super::operation_registration::ServerOperationRegistration;

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
    state: Option<Arc<ErasedOperationState>>,
}

impl BoundOperation {
    fn state(&self) -> OperationStateRef<'_> {
        OperationStateRef {
            value: self.state.as_deref(),
        }
    }
}

pub(super) struct OperationStateInstaller {
    states: [Option<Arc<ErasedOperationState>>; Opcode::COUNT],
}

impl OperationStateInstaller {
    pub(super) const fn new() -> Self {
        Self {
            states: [const { None }; Opcode::COUNT],
        }
    }

    /// Restricts one API module to the operation slots it registered.
    pub(super) fn for_module(
        &mut self,
        registrations: &'static [ServerOperationRegistration],
    ) -> OperationStateBindings<'_> {
        OperationStateBindings {
            states: &mut self.states,
            registrations,
        }
    }

    /// Validates all registrations and state slots before publishing runtime.
    pub(super) fn freeze(
        mut self,
        registrations: impl IntoIterator<Item = &'static ServerOperationRegistration>,
    ) -> Result<OperationRuntime, &'static str> {
        let mut operations = [const { None }; Opcode::COUNT];
        for registration in registrations {
            let index = registration.opcode.index();
            if operations[index].is_some() {
                return Err("operation runtime contains a duplicate registration");
            }
            let state = self.states[index].take();
            if !(registration.state_validator)(state.as_deref()) {
                if state.is_none() {
                    return Err("operation state is missing");
                }
                if (registration.state_validator)(None) {
                    return Err("stateless operation received state");
                }
                return Err("operation state has the wrong type");
            }
            operations[index] = Some(BoundOperation {
                registration,
                state,
            });
        }
        if self.states.iter().any(Option::is_some) {
            return Err("operation state was bound for an unregistered opcode");
        }
        Ok(OperationRuntime { operations })
    }
}

pub(super) struct OperationStateBindings<'a> {
    states: &'a mut [Option<Arc<ErasedOperationState>>; Opcode::COUNT],
    registrations: &'static [ServerOperationRegistration],
}

impl OperationStateBindings<'_> {
    /// Installs one state owner without allocating or cloning it.
    pub(super) fn bind<T>(
        &mut self,
        opcode: Opcode,
        state: Arc<T>,
    ) -> Result<(), &'static str>
    where
        T: Any + Send + Sync,
    {
        if !self
            .registrations
            .iter()
            .any(|registration| registration.opcode == opcode)
        {
            return Err("API module bound state outside its operations");
        }
        let slot = &mut self.states[opcode.index()];
        if slot.is_some() {
            return Err("operation state was bound more than once");
        }
        *slot = Some(state);
        Ok(())
    }
}

pub(super) struct OperationRuntime {
    operations: [Option<BoundOperation>; Opcode::COUNT],
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

}
