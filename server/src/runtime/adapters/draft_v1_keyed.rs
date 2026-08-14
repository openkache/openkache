//! Compatibility-owned keyed work and its optimized collapse adapter.
//!
//! The worker scheduler only owns ordering, capacity, and completion
//! bookkeeping.  It does not identify protocol operations or inspect their
//! payloads.  This module owns the draft compatibility API's static descriptor
//! boundary, including the GET/SET/DELETE collapse reducer.  A future API can
//! use the generic storage-task path or provide another adapter without adding
//! an operation branch to the scheduler.

use crate::observability::Operation;
use crate::protocol::SetOptions;
use crate::store::{
    CompletedKeyedJob, KeyedFinish as StoreKeyedFinish, KeyedJob, KeyedOperation, KeyedOutcome,
    KeyedVisibleState,
};
use crate::types::StoredItemValue;
use crate::{KvError, Kvkache, SetOutcome, StorageKey};

use super::super::scheduler::ScheduledTask;
use super::super::storage_task::StorageTask;
use super::super::worker::{ExclusiveWorkPort, ExclusiveWorkResult};
use super::super::worker_control::execute_storage_task;
use super::super::{DeferredWorkerResponse, WorkerResponse, WorkerResponseSender};

/// API-owned result projection for the compatibility keyed operations.
///
/// The worker transports this as one opaque keyed response.  The public
/// runtime facade may project it into its historical GET/SET/DELETE result
/// types, but the scheduler never needs to know those shapes.
#[derive(Debug)]
pub(in crate::runtime) enum KeyedResponse {
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
}

/// Backend-owned keyed state kept opaque to the generic scheduler.
pub(in crate::runtime) type VisibleState = KeyedVisibleState;
pub(in crate::runtime) type PreparedJob = KeyedJob;
pub(in crate::runtime) type CompletedJob = CompletedKeyedJob;

/// One operation's static scheduler metadata and preparation boundary.
///
/// Function pointers keep the hot request envelope allocation-free.  The
/// scheduler stores the command enum in its existing slab and invokes these
/// callbacks without matching an operation name or protocol opcode.
#[derive(Clone, Copy)]
pub(in crate::runtime) struct KeyedDescriptor {
    pub(in crate::runtime) operation: Operation,
    pub(in crate::runtime) collapsible: fn(&Kvkache, &StorageCommand) -> bool,
    pub(in crate::runtime) prepare:
        fn(&mut Kvkache, StorageKey, StorageCommand) -> PreparedKeyedCommand,
    pub(in crate::runtime) collapse:
        fn(KeyedVisibleState, Vec<StorageCommand>) -> CollapsedLaneBatch,
    /// Identity for one collapse reducer. Different API adapters must not be
    /// reduced into the same batch even when both report collapsible work.
    pub(in crate::runtime) collapse_group: u8,
    pub(in crate::runtime) exclusive: bool,
}

/// Compatibility-owned keyed data-plane work.
///
/// This enum deliberately lives outside the generic worker scheduler.  Its
/// constructors are the only compatibility-facing part of the runtime
/// envelope; the scheduler interacts with it through [`KeyedDescriptor`].
pub(in crate::runtime) enum StorageCommand {
    /// API-owned task work that must be serialized in its key lane.
    Custom {
        task: StorageTask,
        response: WorkerResponseSender,
    },
    Get {
        response: WorkerResponseSender,
    },
    Set {
        value: StoredItemValue,
        options: SetOptions,
        response: WorkerResponseSender,
    },
    Delete {
        response: WorkerResponseSender,
    },
}

/// Name retained for private scheduler fixtures while the compatibility enum
/// remains owned by this adapter module.
pub(in crate::runtime) type KeyedCommand = StorageCommand;

pub(in crate::runtime) fn storage_task(
    task: StorageTask,
    response: WorkerResponseSender,
) -> KeyedCommand {
    StorageCommand::Custom { task, response }
}

pub(in crate::runtime) fn get(response: WorkerResponseSender) -> KeyedCommand {
    StorageCommand::Get { response }
}

pub(in crate::runtime) fn set(
    value: StoredItemValue,
    options: SetOptions,
    response: WorkerResponseSender,
) -> KeyedCommand {
    StorageCommand::Set {
        value,
        options,
        response,
    }
}

pub(in crate::runtime) fn delete(response: WorkerResponseSender) -> KeyedCommand {
    StorageCommand::Delete { response }
}

impl StorageCommand {
    pub(in crate::runtime) fn descriptor(&self) -> &'static KeyedDescriptor {
        match self {
            Self::Custom { .. } => &CUSTOM_DESCRIPTOR,
            Self::Get { .. } => &GET_DESCRIPTOR,
            Self::Set { .. } => &SET_DESCRIPTOR,
            Self::Delete { .. } => &DELETE_DESCRIPTOR,
        }
    }

    pub(in crate::runtime) fn metadata(&self, cache: &Kvkache) -> KeyedCommandMetadata {
        let descriptor = self.descriptor();
        KeyedCommandMetadata {
            operation: descriptor.operation,
            collapsible: (descriptor.collapsible)(cache, self),
        }
    }

    pub(in crate::runtime) fn prepare(
        self,
        cache: &mut Kvkache,
        storage_key: StorageKey,
    ) -> PreparedKeyedCommand {
        let prepare = self.descriptor().prepare;
        prepare(cache, storage_key, self)
    }

    pub(in crate::runtime) fn is_collapsible(&self, cache: &Kvkache) -> bool {
        self.metadata(cache).collapsible
    }
}

impl ScheduledTask for StorageCommand {
    type CollapseGroup = u8;

    fn collapse_group(&self) -> Self::CollapseGroup {
        self.descriptor().collapse_group
    }

    fn is_exclusive(&self) -> bool {
        self.descriptor().exclusive
    }
}

pub(in crate::runtime) struct ExclusiveStorageTask {
    task: StorageTask,
    response: WorkerResponseSender,
}

impl ExclusiveWorkPort<StorageCommand> for Kvkache {
    type Work = ExclusiveStorageTask;

    fn take_exclusive(command: StorageCommand) -> Option<Self::Work> {
        match command {
            StorageCommand::Custom { task, response } => {
                Some(ExclusiveStorageTask { task, response })
            }
            StorageCommand::Get { .. }
            | StorageCommand::Set { .. }
            | StorageCommand::Delete { .. } => None,
        }
    }

    fn execute_exclusive(
        &mut self,
        work: Self::Work,
    ) -> impl std::future::Future<Output = ExclusiveWorkResult> + '_ {
        async move {
            let ExclusiveStorageTask { task, response } = work;
            if task.metadata().cancellation()
                == super::super::StorageTaskCancellation::CancelIfDisconnected
                && response.is_disconnected()
            {
                return ExclusiveWorkResult::Cancelled;
            }
            let result = execute_storage_task(self, task).await;
            let _ = response.send(Ok(result));
            ExclusiveWorkResult::Completed {
                operation: Operation::unknown(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct KeyedCommandMetadata {
    pub(in crate::runtime) operation: Operation,
    pub(in crate::runtime) collapsible: bool,
}

pub(in crate::runtime) struct PreparedKeyedCommand {
    pub(in crate::runtime) response: WorkerResponseSender,
    pub(in crate::runtime) job: PreparedJob,
}

fn prepare_command(
    cache: &mut Kvkache,
    storage_key: StorageKey,
    command: StorageCommand,
) -> PreparedKeyedCommand {
    let (operation, response) = match command {
        StorageCommand::Custom { .. } => {
            unreachable!("exclusive storage tasks do not enter the keyed job path")
        }
        StorageCommand::Get { response } => (KeyedOperation::Get, response),
        StorageCommand::Set {
            value,
            options,
            response,
        } => (KeyedOperation::Set { value, options }, response),
        StorageCommand::Delete { response } => (KeyedOperation::Delete, response),
    };
    PreparedKeyedCommand {
        response,
        job: cache.prepare_keyed(storage_key, operation),
    }
}

fn never_collapsible(_cache: &Kvkache, _command: &StorageCommand) -> bool {
    false
}

fn always_collapsible(_cache: &Kvkache, _command: &StorageCommand) -> bool {
    true
}

fn set_collapsible(cache: &Kvkache, command: &StorageCommand) -> bool {
    match command {
        StorageCommand::Set { value, options, .. } => {
            *options == SetOptions::NONE && cache.can_collapse_set(value)
        }
        StorageCommand::Custom { .. }
        | StorageCommand::Get { .. }
        | StorageCommand::Delete { .. } => false,
    }
}

static CUSTOM_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::unknown(),
    collapsible: never_collapsible,
    prepare: prepare_command,
    collapse: no_collapse,
    collapse_group: CUSTOM_COLLAPSE_GROUP,
    exclusive: true,
};

static GET_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Get),
    collapsible: always_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

static SET_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Set),
    collapsible: set_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

static DELETE_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Delete),
    collapsible: always_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

const COMPATIBILITY_COLLAPSE_GROUP: u8 = 1;
const CUSTOM_COLLAPSE_GROUP: u8 = 2;

fn no_collapse(_base: KeyedVisibleState, _commands: Vec<StorageCommand>) -> CollapsedLaneBatch {
    unreachable!("non-collapsible keyed work cannot be reduced")
}

/// A response waiting for a collapsed mutation's actual storage result.
pub(in crate::runtime) struct CollapsedLaneBatch {
    pub(in crate::runtime) operation: Option<KeyedOperation>,
    pub(in crate::runtime) responses: Vec<DeferredWorkerResponse>,
    pub(in crate::runtime) mutation_response_index: Option<usize>,
    pub(in crate::runtime) success_state: VisibleState,
    pub(in crate::runtime) failure_state: VisibleState,
}

impl CollapsedLaneBatch {
    pub(in crate::runtime) fn reduce(
        base: KeyedVisibleState,
        commands: Vec<StorageCommand>,
    ) -> Self {
        let base_present = matches!(base, KeyedVisibleState::Present(_));
        let mut current = base.clone();
        let mut responses = Vec::with_capacity(commands.len());
        let mut mutation_response_index = None;
        let mut mutated = false;

        for command in commands {
            let response_index = responses.len();
            let (sender, value) = match command {
                StorageCommand::Custom { .. } => {
                    unreachable!("exclusive storage tasks are never collapsed")
                }
                StorageCommand::Get { response } => {
                    let value = match &current {
                        KeyedVisibleState::Missing => None,
                        KeyedVisibleState::Present(value) => Some(value.clone()),
                    };
                    (response, KeyedResponse::Value(value))
                }
                StorageCommand::Set {
                    value,
                    options,
                    response,
                } => {
                    debug_assert_eq!(options, SetOptions::NONE);
                    let outcome = match current {
                        KeyedVisibleState::Missing => SetOutcome::Created,
                        KeyedVisibleState::Present(_) => SetOutcome::Replaced,
                    };
                    current = KeyedVisibleState::Present(value);
                    mutated = true;
                    mutation_response_index = Some(response_index);
                    (response, KeyedResponse::Set(outcome))
                }
                StorageCommand::Delete { response } => {
                    let deleted = matches!(current, KeyedVisibleState::Present(_));
                    current = KeyedVisibleState::Missing;
                    mutated = true;
                    mutation_response_index = Some(response_index);
                    (response, KeyedResponse::Deleted(deleted))
                }
            };
            responses.push(DeferredWorkerResponse {
                sender,
                value: WorkerResponse::Data(value),
            });
        }

        let operation = if mutated {
            match &current {
                KeyedVisibleState::Present(value) => Some(KeyedOperation::Set {
                    value: value.clone(),
                    options: SetOptions::NONE,
                }),
                KeyedVisibleState::Missing if base_present => Some(KeyedOperation::Delete),
                KeyedVisibleState::Missing => None,
            }
        } else {
            None
        };
        let mutation_response_index = operation
            .as_ref()
            .map(|_| mutation_response_index.expect("collapsed mutation has a response"));

        Self {
            operation,
            responses,
            mutation_response_index,
            success_state: current,
            failure_state: base,
        }
    }

    pub(in crate::runtime) fn has_mutation(&self) -> bool {
        self.operation.is_some()
    }

    fn into_prepared(self, cache: &mut Kvkache, storage_key: StorageKey) -> PreparedCollapsed {
        let operation = self
            .operation
            .expect("collapsed batch contains a storage mutation");
        let telemetry_operation = operation.telemetry_operation();
        let job = cache.prepare_keyed(storage_key, operation);
        PreparedCollapsed {
            operation: telemetry_operation,
            job,
            responses: self.responses,
            mutation_response_index: self.mutation_response_index,
            success_state: self.success_state,
            failure_state: self.failure_state,
        }
    }
}

fn reduce_compatibility_batch(
    base: KeyedVisibleState,
    commands: Vec<StorageCommand>,
) -> CollapsedLaneBatch {
    CollapsedLaneBatch::reduce(base, commands)
}

pub(in crate::runtime) struct PreparedCollapsed {
    pub(in crate::runtime) operation: Operation,
    pub(in crate::runtime) job: PreparedJob,
    pub(in crate::runtime) responses: Vec<DeferredWorkerResponse>,
    pub(in crate::runtime) mutation_response_index: Option<usize>,
    pub(in crate::runtime) success_state: VisibleState,
    pub(in crate::runtime) failure_state: VisibleState,
}

pub(in crate::runtime) fn prepare_collapsed_batch(
    cache: &mut Kvkache,
    storage_key: StorageKey,
    batch: CollapsedLaneBatch,
) -> PreparedCollapsed {
    batch.into_prepared(cache, storage_key)
}

pub(in crate::runtime) fn finish_keyed(
    cache: &mut Kvkache,
    job: CompletedJob,
    include_visible_state: bool,
) -> KeyedFinish {
    let StoreKeyedFinish {
        outcome,
        visible_state,
        flush_required,
        pending,
    } = cache.finish_keyed(job, include_visible_state);
    KeyedFinish {
        outcome: outcome.map(worker_response_for_outcome),
        visible_state,
        flush_required,
        pending,
    }
}

/// Completion projected into the worker's opaque response envelope.
pub(in crate::runtime) struct KeyedFinish {
    pub(in crate::runtime) outcome: crate::Result<WorkerResponse>,
    pub(in crate::runtime) visible_state: Option<VisibleState>,
    pub(in crate::runtime) flush_required: bool,
    pub(in crate::runtime) pending: bool,
}

pub(in crate::runtime) fn worker_response_for_outcome(outcome: KeyedOutcome) -> WorkerResponse {
    WorkerResponse::Data(match outcome {
        KeyedOutcome::Value(value) => KeyedResponse::Value(value),
        KeyedOutcome::Set(outcome) => KeyedResponse::Set(outcome),
        KeyedOutcome::Deleted(deleted) => KeyedResponse::Deleted(deleted),
    })
}

pub(in crate::runtime) fn pending_response(outcome: KeyedOutcome) -> WorkerResponse {
    worker_response_for_outcome(outcome)
}

pub(in crate::runtime) fn value_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<Option<StoredItemValue>> {
    match response {
        WorkerResponse::Data(KeyedResponse::Value(value)) => Ok(value),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}

pub(in crate::runtime) fn set_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<SetOutcome> {
    match response {
        WorkerResponse::Data(KeyedResponse::Set(outcome)) => Ok(outcome),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}

pub(in crate::runtime) fn delete_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<bool> {
    match response {
        WorkerResponse::Data(KeyedResponse::Deleted(deleted)) => Ok(deleted),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}
