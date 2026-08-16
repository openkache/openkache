//! Generic server-owned behavior types.

use std::future::Future;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::ptr;
use std::rc::Rc;
use std::task::{Context, Poll};

use super::operation_contract::OperationStatus;
use super::operation_handlers::OperationContext;
use super::operation_outcome::{OperationError, OperationOutcome};

/// The 512-byte payload plus metadata costs at most about 5.2 MiB per 10,000
/// active streams. Current modeled futures are measured in private tests and
/// must stay within this API-neutral bound.
pub(super) const OPERATION_TASK_BYTES: usize = 512;
pub(super) const OPERATION_TASK_ALIGNMENT: usize = 16;
const TASK_OVERLOADED_MESSAGE: &[u8] = b"operation task is unavailable";

#[repr(C, align(16))]
struct TaskStorageBytes([MaybeUninit<u8>; OPERATION_TASK_BYTES]);

struct TaskVtable {
    poll: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<OperationOutcome>,
    drop: unsafe fn(*mut u8),
}

unsafe fn poll_task<F: Future<Output = OperationOutcome>>(
    pointer: *mut u8,
    context: &mut Context<'_>,
) -> Poll<OperationOutcome> {
    // SAFETY: `OperationTaskStorage::try_start` writes one valid `F` into
    // this aligned slot and publishes its vtable for the value's lifetime.
    unsafe { Pin::new_unchecked(&mut *pointer.cast::<F>()).poll(context) }
}

unsafe fn drop_task<F>(pointer: *mut u8) {
    // SAFETY: the slot is initialized with `F` exactly once before its vtable
    // is published. Its owner removes the vtable before invoking this once.
    unsafe { ptr::drop_in_place(pointer.cast::<F>()) };
}

fn task_vtable<F: Future<Output = OperationOutcome>>() -> TaskVtable {
    TaskVtable {
        poll: poll_task::<F>,
        drop: drop_task::<F>,
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum TaskStartError {
    Occupied,
    LayoutTooLarge,
}

/// One reusable worker/stream-owned slot for a suspended operation.
///
/// The slot is deliberately bounded. A future that does not fit is rejected
/// as overload rather than falling back to a request-path heap allocation.
pub(super) struct OperationTaskStorage {
    bytes: TaskStorageBytes,
    task: Option<TaskVtable>,
    // A forgotten `OperationTask` leaves its potentially non-Send future
    // owned by this storage until `Drop`, so the storage must remain local.
    _not_send: PhantomData<Rc<()>>,
}

impl OperationTaskStorage {
    pub(super) fn new() -> Self {
        Self {
            bytes: TaskStorageBytes([const { MaybeUninit::uninit() }; OPERATION_TASK_BYTES]),
            task: None,
            _not_send: PhantomData,
        }
    }

    pub(super) fn try_start<'a, F>(
        &'a mut self,
        future: F,
    ) -> Result<OperationTask<'a>, TaskStartError>
    where
        F: Future<Output = OperationOutcome> + 'a,
    {
        if self.task.is_some() {
            return Err(TaskStartError::Occupied);
        }
        if std::mem::size_of::<F>() > OPERATION_TASK_BYTES
            || std::mem::align_of::<F>() > OPERATION_TASK_ALIGNMENT
        {
            return Err(TaskStartError::LayoutTooLarge);
        }
        let vtable = task_vtable::<F>();
        let pointer = self.pointer().cast::<F>();
        // SAFETY: size and alignment were checked above. The exclusive slot
        // borrow and empty task state prove there is no live value.
        unsafe { pointer.write(future) };
        self.task = Some(vtable);
        Ok(OperationTask {
            storage: self,
            _not_send: PhantomData,
        })
    }

    fn pointer(&mut self) -> *mut u8 {
        self.bytes.0.as_mut_ptr().cast()
    }
}

impl Drop for OperationTaskStorage {
    fn drop(&mut self) {
        if let Some(vtable) = self.task.take() {
            // A forgotten task leaves ownership with the storage. Taking the
            // vtable first prevents a panicking future destructor from being
            // invoked again.
            unsafe { (vtable.drop)(self.pointer()) };
        }
    }
}

pub(super) struct OperationTask<'a> {
    storage: &'a mut OperationTaskStorage,
    // Keep the erased task boundary local, as the previous boxed future was.
    _not_send: PhantomData<Rc<()>>,
}

impl Future for OperationTask<'_> {
    type Output = OperationOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // `OperationTask` never moves the initialized future after it has
        // been written into the pinned storage slot.
        let task = self.get_mut();
        let vtable = task
            .storage
            .task
            .as_ref()
            .expect("operation task storage lost its initialized future");
        unsafe { (vtable.poll)(task.storage.pointer(), context) }
    }
}

impl Drop for OperationTask<'_> {
    fn drop(&mut self) {
        let vtable = self
            .storage
            .task
            .take()
            .expect("operation task storage lost its initialized future");
        // Taking the vtable first makes the slot logically empty even when
        // the future destructor panics.
        unsafe { (vtable.drop)(self.storage.pointer()) };
    }
}

/// Future returned by an operation binding.
///
/// Immediate API handlers use the inline `Ready` variant and therefore do not
/// allocate. Suspended handlers place their concrete future in one bounded
/// stream-owned slot; `Pending` erases only its poll and drop functions and
/// never falls back to a request-path heap allocation.
pub(super) enum OperationFuture<'a> {
    Ready(Option<OperationOutcome>),
    Pending(OperationTask<'a>),
}

impl<'a> OperationFuture<'a> {
    pub(super) fn ready(outcome: OperationOutcome) -> Self {
        Self::Ready(Some(outcome))
    }

    pub(super) fn pending<F>(storage: &'a mut OperationTaskStorage, future: F) -> Self
    where
        F: Future<Output = OperationOutcome> + 'a,
    {
        match storage.try_start(future) {
            Ok(task) => Self::Pending(task),
            Err(_) => Self::Ready(Some(OperationOutcome::error(OperationError::status(
                OperationStatus::Overloaded,
                TASK_OVERLOADED_MESSAGE,
            )))),
        }
    }
}

impl Future for OperationFuture<'_> {
    type Output = OperationOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Ready(outcome) => Poll::Ready(
                outcome
                    .take()
                    .expect("operation ready future was polled after completion"),
            ),
            Self::Pending(task) => Pin::new(task).poll(context),
        }
    }
}

/// One operation behavior slot.
pub(super) type OperationHandler =
    for<'a> fn(OperationContext<'a>, &'a mut OperationTaskStorage) -> OperationFuture<'a>;
