//! API-owned admission and resource-preparation primitives.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::lock::Mutex as AsyncMutex;
use smallvec::SmallVec;

use super::operation_contract::OperationStatus;
use super::operation_execution_state::OperationStateRef;
use super::operation_handlers::OperationInputView;

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
    body_len: usize,
    body_field: Option<usize>,
    prefix: &'a [u8],
}

impl<'a> OperationHeaderView<'a> {
    pub(super) const fn new(body_len: usize, body_field: Option<usize>, prefix: &'a [u8]) -> Self {
        Self {
            body_len,
            body_field,
            prefix,
        }
    }

    /// Returns the complete declared opaque-body length.
    pub(super) const fn body_len(&self) -> usize {
        self.body_len
    }

    /// Returns the declared body length when it represents this modeled field.
    pub(super) fn declared_body_len(&self, field: usize) -> Option<usize> {
        (self.body_field == Some(field)).then_some(self.body_len())
    }

    /// Returns the exact bytes available before the opaque body.
    #[allow(dead_code)]
    pub(super) const fn prefix(&self) -> &'a [u8] {
        self.prefix
    }
}

#[derive(Clone, Copy)]
pub(super) struct HeaderAdmissionContext<'a> {
    pub(super) state: OperationStateRef<'a>,
}

impl<'a> HeaderAdmissionContext<'a> {
    /// Borrows the state initialized by this operation's API module.
    pub(super) fn state<T: Any>(&self) -> Option<&'a T> {
        self.state.get()
    }
}

pub(super) type HeaderAdmissionFn = for<'a> fn(
    &OperationHeaderView<'a>,
    HeaderAdmissionContext<'a>,
) -> std::result::Result<(), HeaderAdmissionError>;

/// Dependencies exposed to an API-owned preparation boundary.
///
/// Preparation is intentionally narrower than behavior execution. An API
/// binding can resolve opaque resources and reservations without depending on
/// the concrete server, cache implementation, or transport context. The
/// composition root supplies opaque module state.
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
    /// State initialized once for the operation's API module.
    pub(super) state: OperationStateRef<'a>,
}

impl<'a> PrepareContext<'a> {
    /// Borrows the state initialized by this operation's API module.
    pub(super) fn state<T: Any>(&self) -> Option<&'a T> {
        self.state.get()
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

    /// Creates a lifecycle lock that is always active.
    ///
    /// Namespace creation is serialized by the registry's global lifecycle
    /// mutex rather than by an individual namespace entry, so there is no
    /// liveness flag to check after acquisition.
    pub(super) fn unconditional(lock: Arc<AsyncMutex<()>>) -> Self {
        Self {
            lock,
            active: None,
            inactive_error: PrepareError::resource_unavailable(
                OperationStatus::InternalError,
                b"namespace metadata is unavailable",
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
