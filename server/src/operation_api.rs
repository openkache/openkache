//! API registration and preparation primitives.
//!
//! The sibling [`operation_outcome`] and [`operation_registry`] modules own only
//! the transport-neutral operation contract and registry validation. This module
//! owns the registration and resource-preparation boundary; concrete behavior
//! remains in API-owned binding modules.

use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::{Opcode, RequestFrameHeader};
use smallvec::SmallVec;

use super::operation_capabilities::{CapabilityCatalog, CapabilityRegistry};
use super::operation_contract as contract;
use super::operation_contract::OperationStatus;
use super::operation_handlers::{AuthorizationFn, OperationInputView};
use super::operation_registry::OperationHandler;
/// Downcasts one API-owned capability without exposing the catalog's
/// type-erasure details to a binding.
const CAPABILITY_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const CAPABILITY_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Computes the stable numeric identity used by the capability catalog.
///
/// Names remain available for diagnostics and collision checks, while the
/// numeric identity keeps request lookups independent of string allocation.
pub const fn capability_id(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = CAPABILITY_HASH_OFFSET;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(CAPABILITY_HASH_PRIME);
        index += 1;
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityKey<T: Any> {
    name: &'static str,
    id: u64,
    _type: PhantomData<fn() -> T>,
}

impl<T: Any> CapabilityKey<T> {
    /// Creates a stable key owned by one API module.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            id: capability_id(name),
            _type: PhantomData,
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn id(&self) -> u64 {
        self.id
    }
}

pub(super) fn downcast_capability<'a, T: Any>(
    catalog: &'a dyn CapabilityCatalog,
    key: CapabilityKey<T>,
) -> Option<&'a T> {
    catalog.get_by_id(key.id(), key.name())?.downcast_ref::<T>()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationCommitDisposition {
    /// The operation cannot make a durable state change.
    ReadOnly,
    /// The operation may have crossed its commit point when a wait expires.
    MayBeCommitted,
}

/// Server-side commit policy retained as a named alias for API registrations.
///
/// There is only one policy dimension at this boundary. Keeping a wrapper
/// struct would add a second `.commit` hop without carrying any more
/// information.
pub(super) type ServerOperationPolicy = OperationCommitDisposition;

impl OperationCommitDisposition {
    pub(super) const READ_ONLY: Self = Self::ReadOnly;
    pub(super) const MUTATION: Self = Self::MayBeCommitted;
}

/// A preparation failure expressed in API-owned status vocabulary.
///
/// The dispatcher only projects this token through the operation contract; it
/// does not know whether the failed resource was a namespace, tenant, shard,
/// or another API-owned identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PrepareError {
    pub(super) status: OperationStatus,
    pub(super) message: &'static [u8],
}

/// One header-level admission failure expressed in API-owned status vocabulary.
///
/// Admission runs after generic framing has validated the declared request
/// shape but before the transport reserves or reads its opaque body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HeaderAdmissionError {
    pub(super) status: OperationStatus,
    pub(super) message: &'static [u8],
}

impl HeaderAdmissionError {
    pub(super) const fn new(status: OperationStatus, message: &'static [u8]) -> Self {
        Self { status, message }
    }
}

/// Header metadata exposed to an API-owned admission hook.
///
/// Numeric field indexes come from the generated API contract. The generic
/// server does not attach semantic names or storage policy to them.
pub(super) struct OperationHeaderView<'a> {
    header: RequestFrameHeader,
    prefix: &'a [u8],
}

impl<'a> OperationHeaderView<'a> {
    pub(super) const fn new(header: RequestFrameHeader, prefix: &'a [u8]) -> Self {
        Self { header, prefix }
    }

    /// Returns the complete declared opaque-body length.
    pub(super) const fn body_len(&self) -> usize {
        self.header.body_len()
    }

    /// Returns the declared body length when it represents this modeled field.
    pub(super) fn declared_body_len(&self, field: usize) -> Option<usize> {
        (self.header.body_field() == Some(field)).then_some(self.body_len())
    }

    /// Returns the exact bytes available before the opaque body.
    #[allow(dead_code)]
    pub(super) const fn prefix(&self) -> &'a [u8] {
        self.prefix
    }
}

#[derive(Clone, Copy)]
pub(super) struct HeaderAdmissionContext<'a> {
    pub(super) capabilities: &'a dyn CapabilityCatalog,
}

pub(super) type HeaderAdmissionFn = for<'a> fn(
    &OperationHeaderView<'a>,
    HeaderAdmissionContext<'a>,
) -> std::result::Result<(), HeaderAdmissionError>;

/// Dependencies exposed to an API-owned preparation boundary.
///
/// Preparation is intentionally narrower than behavior execution. An API
/// binding can resolve opaque resources and reservations without
/// depending on the concrete server, cache implementation, or transport
/// context. The composition root supplies the compatibility resolver and the
/// opaque capability catalog.
pub(super) type PrepareFn = for<'a> fn(
    &OperationInputView,
    PrepareContext<'a>,
) -> std::result::Result<PreparePlan, PrepareError>;

/// Default preparation for an operation that has no API-owned resources.
///
/// Keeping this in the registration foundation makes a generic operation row
/// describe only its opcode, handler, and commit policy. Resource-aware APIs
/// can still provide their own preparation function without changing the
/// dispatcher or introducing another registration family.
pub(super) fn prepare_none(
    _input: &OperationInputView,
    _context: PrepareContext<'_>,
) -> std::result::Result<PreparePlan, PrepareError> {
    Ok(PreparePlan::none())
}

#[derive(Clone, Copy)]
pub(super) struct PrepareContext<'a> {
    /// API-owned dependencies used to build API-owned resource plans.
    ///
    /// Every binding, including compatibility adapters, obtains its resolver
    /// through this opaque catalog. The dispatcher does not carry a
    /// domain-specific dependency field.
    pub(super) capabilities: &'a dyn CapabilityCatalog,
}

impl<'a> PrepareContext<'a> {
    /// Looks up one API-owned dependency for custom resource preparation.
    pub(super) fn capability<T: Any>(&self, key: CapabilityKey<T>) -> Option<&'a T> {
        downcast_capability(self.capabilities, key)
    }
}

impl PrepareError {
    pub(super) const fn invalid_request(message: &'static [u8]) -> Self {
        Self {
            status: OperationStatus::InvalidRequest,
            message,
        }
    }

    pub(super) const fn resource_unavailable(
        status: OperationStatus,
        message: &'static [u8],
    ) -> Self {
        Self { status, message }
    }
}

/// A resource lock resolved by an API-owned preparation boundary.
///
/// The server executor acquires this opaque handle and checks its liveness
/// after waiting. It never interprets a resource key or reaches into a
/// namespace-specific registry.
#[derive(Clone)]
pub(crate) struct ResourceLock {
    lock: Arc<AsyncMutex<()>>,
    active: Option<Arc<AtomicBool>>,
    inactive_error: PrepareError,
}

impl ResourceLock {
    pub(super) fn new(
        lock: Arc<AsyncMutex<()>>,
        active: Arc<AtomicBool>,
        inactive_error: PrepareError,
    ) -> Self {
        Self {
            lock,
            active: Some(active),
            inactive_error,
        }
    }

    /// Wraps a process-wide resource that has no deletion lifecycle.
    ///
    /// Lifecycle locks and API-owned resources use the same bundle so the
    /// executor has one deterministic acquisition path. The always-active
    /// flag keeps that representation allocation-free at the call site while
    /// preserving the stale-handle check for deletable resources.
    pub(super) fn unconditional(lock: Arc<AsyncMutex<()>>) -> Self {
        Self {
            lock,
            active: None,
            inactive_error: PrepareError::resource_unavailable(
                OperationStatus::InternalError,
                b"prepared resource is no longer available",
            ),
        }
    }

    pub(super) fn lock(&self) -> &Arc<AsyncMutex<()>> {
        &self.lock
    }

    pub(super) fn inactive_error(&self) -> Option<PrepareError> {
        self.active
            .as_ref()
            .filter(|active| !active.load(Ordering::Acquire))
            .map(|_| self.inactive_error)
    }

    fn order_key(&self) -> usize {
        Arc::as_ptr(&self.lock) as usize
    }

    fn same_lock(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.lock, &other.lock)
    }
}

/// Lock requirements computed by one operation's typed preparation boundary.
///
/// The dispatcher does not infer lock identity from field roles. An API-owned
/// preparation hook returns the complete plan once the generated input view has
/// been decoded; adding another preparation shape therefore does not add a
/// branch to request parsing.
#[derive(Default)]
pub(super) struct PreparePlan {
    resources: SmallVec<[ResourceLock; 8]>,
}

impl PreparePlan {
    pub(super) fn none() -> Self {
        Self::default()
    }

    pub(super) fn resource(resource: ResourceLock) -> Self {
        Self::from_resources([resource])
    }

    /// Creates a deterministic lock plan for one or more resources.
    ///
    /// Sorting and deduplicating here gives every API the same deadlock-free
    /// multi-resource preparation boundary without adding resource semantics
    /// to the dispatcher.
    pub(super) fn from_resources<I>(resource_handles: I) -> Self
    where
        I: IntoIterator<Item = ResourceLock>,
    {
        let mut resources: SmallVec<[ResourceLock; 8]> = resource_handles.into_iter().collect();
        resources.sort_unstable_by_key(ResourceLock::order_key);
        resources.dedup_by(|left, right| left.same_lock(right));
        Self { resources }
    }

    pub(super) fn resources(&self) -> &[ResourceLock] {
        &self.resources
    }
}

/// One complete server registration. Keeping behavior and server policy in
/// the same entry prevents a new operation from being added to one table and
/// forgotten in another.
#[derive(Clone, Copy)]
pub(super) struct ServerOperationRegistration {
    pub(super) opcode: Opcode,
    pub(super) handler: OperationHandler,
    pub(super) admit_header: Option<HeaderAdmissionFn>,
    pub(super) prepare: PrepareFn,
    pub(super) authorization: AuthorizationFn,
    pub(super) policy: ServerOperationPolicy,
}

/// Const-friendly registration builder used by API-owned modules.
///
/// The builder keeps the generic defaults in one place while making every
/// non-default concern visible at the API boundary. A new API can therefore
/// register a handler with:
///
/// ```text
/// RegistrationBuilder::new(opcode, handler)
///     .prepare(prepare)
///     .authorize(authorize)
///     .mutation()
///     .build()
/// ```
#[derive(Clone, Copy)]
pub(super) struct RegistrationBuilder {
    registration: ServerOperationRegistration,
}

impl RegistrationBuilder {
    /// Starts one operation registration with safe read-only defaults.
    pub(super) const fn new(opcode: Opcode, handler: OperationHandler) -> Self {
        Self {
            registration: ServerOperationRegistration {
                opcode,
                handler,
                admit_header: None,
                prepare: prepare_none,
                authorization: super::operation_handlers::authorization_none,
                policy: ServerOperationPolicy::READ_ONLY,
            },
        }
    }

    /// Adds an API-owned admission hook over generated request-header fields.
    pub(super) const fn admit_header(mut self, admit: HeaderAdmissionFn) -> Self {
        self.registration.admit_header = Some(admit);
        self
    }

    /// Adds an API-owned resource preparation hook.
    pub(super) const fn prepare(mut self, prepare: PrepareFn) -> Self {
        self.registration.prepare = prepare;
        self
    }

    /// Adds an API-owned connection authorization predicate.
    pub(super) const fn authorize(mut self, authorization: AuthorizationFn) -> Self {
        self.registration.authorization = authorization;
        self
    }

    /// Marks the operation as potentially crossing its mutation point.
    pub(super) const fn mutation(mut self) -> Self {
        self.registration.policy = ServerOperationPolicy::MUTATION;
        self
    }

    /// Retains the default read-only commit policy explicitly.
    pub(super) const fn read_only(mut self) -> Self {
        self.registration.policy = ServerOperationPolicy::READ_ONLY;
        self
    }

    /// Finalizes the immutable registration stored in an API module.
    pub(super) const fn build(self) -> ServerOperationRegistration {
        self.registration
    }
}

/// A self-contained API module contribution.
///
/// The module owns behavior registrations and optional capability
/// installation. Runtime frame projection is generated independently of API
/// modules.
#[derive(Clone, Copy)]
pub(super) struct ApiModule {
    operations: &'static [ServerOperationRegistration],
    install_capabilities: Option<ModuleCapabilityInstaller>,
}

pub(super) type ModuleCapabilityInstaller =
    fn(&mut CapabilityRegistry, &dyn CapabilityCatalog) -> Result<(), &'static str>;

impl ApiModule {
    pub(super) const fn new(operations: &'static [ServerOperationRegistration]) -> Self {
        Self {
            operations,
            install_capabilities: None,
        }
    }

    pub(super) const fn install_capabilities(mut self, install: ModuleCapabilityInstaller) -> Self {
        self.install_capabilities = Some(install);
        self
    }

    pub(super) const fn operations(self) -> &'static [ServerOperationRegistration] {
        self.operations
    }

    fn install(
        self,
        registry: &mut CapabilityRegistry,
        bootstrap: &dyn CapabilityCatalog,
    ) -> Result<(), &'static str> {
        if let Some(install) = self.install_capabilities {
            install(registry, bootstrap)?;
        }
        Ok(())
    }
}

/// Server catalogs assembled together from API-owned module contributions.
///
/// Registering one module installs its behavior and optional capabilities.
/// Generated metadata remains the sole runtime frame-admission contract.
pub(super) struct ServerComposition {
    operations: OperationCatalog,
    modules: [Option<ApiModule>; Opcode::COUNT],
    module_count: usize,
}

impl ServerComposition {
    pub(super) const fn new() -> Self {
        Self {
            operations: OperationCatalog::new(),
            modules: [None; Opcode::COUNT],
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

    pub(super) fn install_capabilities(
        &'static self,
        registry: &mut CapabilityRegistry,
        bootstrap: &dyn CapabilityCatalog,
    ) -> Result<(), &'static str> {
        let mut index = 0;
        while index < self.module_count {
            self.modules[index]
                .expect("registered API module is missing")
                .install(registry, bootstrap)?;
            index += 1;
        }
        Ok(())
    }

    pub(super) fn operation(
        &'static self,
        opcode: Opcode,
    ) -> Option<&'static ServerOperationRegistration> {
        self.operations.get(opcode)
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
    entries: [Option<ServerOperationRegistration>; Opcode::COUNT],
}

impl OperationCatalog {
    const fn new() -> Self {
        Self {
            entries: [None; Opcode::COUNT],
        }
    }

    const fn register(mut self, registrations: &[ServerOperationRegistration]) -> Self {
        let mut index = 0;
        while index < registrations.len() {
            let registration = registrations[index];
            let slot = registration.opcode.index();
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

    fn get(&'static self, opcode: Opcode) -> Option<&'static ServerOperationRegistration> {
        self.entries[opcode.index()].as_ref()
    }

    fn iter(&'static self) -> impl Iterator<Item = &'static ServerOperationRegistration> {
        self.entries.iter().filter_map(Option::as_ref)
    }
}

#[allow(dead_code)]
pub(super) fn handles(opcode: Opcode) -> bool {
    super::operation_registrations::server_operation(opcode).is_some()
}

pub(super) fn server_operation(opcode: Opcode) -> Option<&'static ServerOperationRegistration> {
    super::operation_registrations::server_operation(opcode)
}

/// Validates API-owned behavior entries against generated operation metadata.
pub(super) fn validate_registry() -> Result<(), &'static str> {
    let mut seen = [false; Opcode::COUNT];
    for registration in super::operation_registrations::registered_operations() {
        let index = registration.opcode.index();
        if seen[index] {
            return Err("server operation policy registry contains a duplicate opcode");
        }
        seen[index] = true;
    }
    for entry in contract::operation_registry() {
        let Some(_registration) = server_operation(entry.opcode) else {
            return Err("modeled operation has no server registration");
        };
        let wire = entry.wire;
        if wire.request.fields.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
            return Err("modeled operation request plan exceeds generated bounds");
        }
        if matches!(
            wire.response.framing,
            contract::OperationLayoutFraming::OptionalValues
                | contract::OperationLayoutFraming::FieldSequence
        ) && wire.response.fields.is_empty()
        {
            return Err("ordered response operation has no generated fields");
        }
        // The single handler pointer is the executable behavior boundary.
    }
    Ok(())
}
