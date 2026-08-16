//! Operation registration rows and const-friendly builders.

use std::any::Any;

use openkache_protocol::Opcode;

use super::operation_authorization::{AuthorizationFn, authorization_none};
use super::operation_execution_state::{StateValidator, no_operation_state, typed_operation_state};
use super::operation_preparation::{HeaderAdmissionFn, PrepareFn, prepare_none};
use super::operation_registry::OperationHandler;

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
    pub(super) state_validator: StateValidator,
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
                authorization: authorization_none,
                policy: ServerOperationPolicy::READ_ONLY,
                state_validator: no_operation_state,
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

    /// Declares the concrete worker-local state required by this operation.
    ///
    /// State is validated once during worker construction. Request callbacks
    /// only borrow the already-resolved value from the dense opcode slot.
    pub(super) const fn state<T: Any + Send + Sync>(mut self) -> Self {
        self.registration.state_validator = typed_operation_state::<T>;
        self
    }

    /// Finalizes the immutable registration stored in an API module.
    pub(super) const fn build(self) -> ServerOperationRegistration {
        self.registration
    }
}
