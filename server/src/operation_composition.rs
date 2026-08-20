//! Dense operation composition assembled from API-owned modules.

use super::operation_capabilities::CapabilityCatalog;
use super::operation_contract::OperationId;
use super::operation_execution_state::{
    ExperimentalApiGate, OperationRuntime, OperationStateBindings, OperationStateInstaller,
};
use super::operation_registration::ServerOperationRegistration;

/// A self-contained API module contribution.
///
/// The module owns behavior registrations and optional worker state
/// initialization. Runtime frame projection is generated independently of API
/// modules.
#[derive(Clone, Copy)]
pub(super) struct ApiModule {
    operations: &'static [ServerOperationRegistration],
    state_installer: Option<ModuleStateInstaller>,
}

pub(super) type ModuleStateInstaller =
    fn(&mut OperationStateBindings<'_>, &dyn CapabilityCatalog) -> Result<(), &'static str>;

impl ApiModule {
    pub(super) const fn new(operations: &'static [ServerOperationRegistration]) -> Self {
        Self {
            operations,
            state_installer: None,
        }
    }

    /// Installs worker-owned state into exact operation slots.
    pub(super) const fn install_operation_state(mut self, install: ModuleStateInstaller) -> Self {
        self.state_installer = Some(install);
        self
    }

    pub(super) const fn operations(self) -> &'static [ServerOperationRegistration] {
        self.operations
    }

    fn install_state_into(
        self,
        states: &mut OperationStateBindings<'_>,
        bootstrap: &dyn CapabilityCatalog,
    ) -> Result<(), &'static str> {
        if let Some(install) = self.state_installer {
            install(states, bootstrap)?;
        }
        Ok(())
    }
}

/// Server catalogs assembled together from API-owned module contributions.
///
/// Registering one module installs its behavior and optional worker state.
/// Generated metadata remains the sole runtime frame-admission contract.
pub(super) struct ServerComposition {
    operations: OperationCatalog,
    modules: [Option<ApiModule>; OperationId::COUNT],
    module_count: usize,
}

impl ServerComposition {
    pub(super) const fn new() -> Self {
        Self {
            operations: OperationCatalog::new(),
            modules: [None; OperationId::COUNT],
            module_count: 0,
        }
    }

    pub(super) const fn register_module(mut self, module: ApiModule) -> Self {
        if self.module_count == self.modules.len() {
            panic!("too many API modules");
        }
        self.operations = self.operations.register_module(module);
        self.modules[self.module_count] = Some(module);
        self.module_count += 1;
        self
    }

    pub(super) fn initialize_modules_with_gate(
        &'static self,
        bootstrap: &dyn CapabilityCatalog,
        experimental_api: ExperimentalApiGate,
    ) -> Result<OperationRuntime, &'static str> {
        let mut states = OperationStateInstaller::new();
        let mut index = 0;
        while index < self.module_count {
            let module = self.modules[index].expect("registered API module is missing");
            let mut bindings = states.for_module(module.operations());
            module.install_state_into(&mut bindings, bootstrap)?;
            index += 1;
        }
        states.freeze_with_gate(self.operations(), experimental_api)
    }

    pub(super) fn operation(
        &'static self,
        operation_id: OperationId,
    ) -> Option<&'static ServerOperationRegistration> {
        self.operations.get(operation_id)
    }

    pub(super) fn operations(
        &'static self,
    ) -> impl Iterator<Item = &'static ServerOperationRegistration> {
        self.operations.iter()
    }
}

/// Dense operation catalog assembled by the composition root.
///
/// API modules contribute sparse registration slices; this catalog performs
/// the one-time dense opcode placement used by the request hot path. The
/// executor never knows which module supplied an entry.
pub(super) struct OperationCatalog {
    entries: [Option<ServerOperationRegistration>; OperationId::COUNT],
}

impl OperationCatalog {
    const fn new() -> Self {
        Self {
            entries: [None; OperationId::COUNT],
        }
    }

    const fn register(mut self, registrations: &[ServerOperationRegistration]) -> Self {
        let mut index = 0;
        while index < registrations.len() {
            let registration = registrations[index];
            let slot = registration.operation_id.index();
            if self.entries[slot].is_some() {
                panic!("duplicate operation registration");
            }
            self.entries[slot] = Some(registration);
            index += 1;
        }
        self
    }

    const fn register_module(self, module: ApiModule) -> Self {
        self.register(module.operations())
    }

    fn get(
        &'static self,
        operation_id: OperationId,
    ) -> Option<&'static ServerOperationRegistration> {
        self.entries[operation_id.index()].as_ref()
    }

    fn iter(&'static self) -> impl Iterator<Item = &'static ServerOperationRegistration> {
        self.entries.iter().filter_map(Option::as_ref)
    }
}
