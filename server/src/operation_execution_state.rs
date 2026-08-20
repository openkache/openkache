//! Worker-owned operation registrations bound to API module state.
//!
//! API modules install opaque state into opcode slots during worker startup.
//! Freeze validates those slots against registrations, then request dispatch
//! borrows them without a capability lookup, allocation, or reference-count
//! bump.

use std::any::Any;
use std::sync::Arc;

use super::operation_contract::OperationId;
use super::operation_registration::ServerOperationRegistration;
use crate::operation_contract::OperationWireSpec;

/// Configuration selected once at bind time for operations outside stable v1.
///
/// The generated operation descriptor remains the source of truth for the
/// exact revision string. Keeping the gate in the worker-owned runtime makes
/// header admission and operation execution observe the same immutable choice.
#[derive(Clone, Debug, Default)]
pub(super) struct ExperimentalApiGate {
    pub(super) enabled: bool,
    pub(super) revision: Option<Arc<str>>,
}

impl ExperimentalApiGate {
    pub(super) fn new(enabled: bool, revision: Option<String>) -> Self {
        Self {
            enabled,
            revision: revision.map(Arc::<str>::from),
        }
    }

    pub(super) fn admits(&self, wire: OperationWireSpec) -> bool {
        wire.enabled(self.enabled, self.revision.as_deref())
    }
}

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
    states: [Option<Arc<ErasedOperationState>>; OperationId::COUNT],
}

impl OperationStateInstaller {
    pub(super) const fn new() -> Self {
        Self {
            states: [const { None }; OperationId::COUNT],
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

    pub(super) fn freeze_with_gate(
        mut self,
        registrations: impl IntoIterator<Item = &'static ServerOperationRegistration>,
        experimental_api: ExperimentalApiGate,
    ) -> Result<OperationRuntime, &'static str> {
        let mut operations = [const { None }; OperationId::COUNT];
        for registration in registrations {
            let index = registration.operation_id.index();
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
        Ok(OperationRuntime {
            operations,
            experimental_api,
        })
    }
}

pub(super) struct OperationStateBindings<'a> {
    states: &'a mut [Option<Arc<ErasedOperationState>>; OperationId::COUNT],
    registrations: &'static [ServerOperationRegistration],
}

impl OperationStateBindings<'_> {
    /// Installs one state owner without allocating or cloning it.
    pub(super) fn bind<T>(
        &mut self,
        operation_id: OperationId,
        state: Arc<T>,
    ) -> Result<(), &'static str>
    where
        T: Any + Send + Sync,
    {
        if !self
            .registrations
            .iter()
            .any(|registration| registration.operation_id == operation_id)
        {
            return Err("API module bound state outside its operations");
        }
        let slot = &mut self.states[operation_id.index()];
        if slot.is_some() {
            return Err("operation state was bound more than once");
        }
        *slot = Some(state);
        Ok(())
    }
}

pub(super) struct OperationRuntime {
    operations: [Option<BoundOperation>; OperationId::COUNT],
    experimental_api: ExperimentalApiGate,
}

impl OperationRuntime {
    pub(super) fn registration(
        &self,
        operation_id: OperationId,
    ) -> Option<&'static ServerOperationRegistration> {
        self.operations[operation_id.index()]
            .as_ref()
            .map(|operation| operation.registration)
    }

    pub(super) fn operation(
        &self,
        operation_id: OperationId,
    ) -> Option<(&'static ServerOperationRegistration, OperationStateRef<'_>)> {
        self.operations[operation_id.index()]
            .as_ref()
            .map(|operation| (operation.registration, operation.state()))
    }

    /// Returns whether the generated operation is assigned to this worker's
    /// data-plane under the immutable experimental gate.
    pub(super) fn admits_wire(&self, wire: OperationWireSpec) -> bool {
        self.experimental_api.admits(wire)
    }
}
