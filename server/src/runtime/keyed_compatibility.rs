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

use super::storage_task::StorageTask;
use super::worker::{DeferredWorkerResponse, WorkerResponse, WorkerResponseSender};

/// API-owned result projection for the compatibility keyed operations.
///
/// The worker transports this as one opaque keyed response.  The public
/// runtime facade may project it into its historical GET/SET/DELETE result
/// types, but the scheduler never needs to know those shapes.
#[derive(Debug)]
pub(super) enum KeyedResponse {
    Value(Option<StoredItemValue>),
    Set(SetOutcome),
    Deleted(bool),
}

/// Backend-owned keyed state kept opaque to the generic scheduler.
pub(super) type VisibleState = KeyedVisibleState;
pub(super) type PreparedJob = KeyedJob;
pub(super) type CompletedJob = CompletedKeyedJob;

/// Non-zero-sized identity token for one reducer's compatibility group.
///
/// The scheduler compares the address of this token, so it must not be a
/// zero-sized type: distinct `&'static ()` values are allowed to share an
/// address and would make unrelated reducers appear compatible.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct CollapseGroup(u8);

/// One operation's static scheduler metadata and preparation boundary.
///
/// Function pointers keep the hot request envelope allocation-free.  The
/// scheduler stores the command enum in its existing slab and invokes these
/// callbacks without matching an operation name or protocol opcode.
#[derive(Clone, Copy)]
pub(super) struct KeyedDescriptor {
    pub(super) operation: Operation,
    pub(super) collapsible: fn(&Kvkache, &StorageCommand) -> bool,
    pub(super) prepare: fn(&mut Kvkache, StorageKey, StorageCommand) -> PreparedKeyedCommand,
    pub(super) collapse: fn(KeyedVisibleState, Vec<StorageCommand>) -> CollapsedLaneBatch,
    /// Identity for one collapse reducer. Different API adapters must not be
    /// reduced into the same batch even when both report collapsible work.
    pub(super) collapse_group: &'static CollapseGroup,
    pub(super) exclusive: bool,
}

/// Compatibility-owned keyed data-plane work.
///
/// This enum deliberately lives outside the generic worker scheduler.  Its
/// constructors are the only compatibility-facing part of the runtime
/// envelope; the scheduler interacts with it through [`KeyedDescriptor`].
pub(super) enum StorageCommand {
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
pub(super) type KeyedCommand = StorageCommand;

pub(super) fn storage_task(task: StorageTask, response: WorkerResponseSender) -> KeyedCommand {
    StorageCommand::Custom { task, response }
}

pub(super) fn get(response: WorkerResponseSender) -> KeyedCommand {
    StorageCommand::Get { response }
}

pub(super) fn set(
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

pub(super) fn delete(response: WorkerResponseSender) -> KeyedCommand {
    StorageCommand::Delete { response }
}

impl StorageCommand {
    pub(super) fn descriptor(&self) -> &'static KeyedDescriptor {
        match self {
            Self::Custom { .. } => &CUSTOM_DESCRIPTOR,
            Self::Get { .. } => &GET_DESCRIPTOR,
            Self::Set { .. } => &SET_DESCRIPTOR,
            Self::Delete { .. } => &DELETE_DESCRIPTOR,
        }
    }

    pub(super) fn metadata(&self, cache: &Kvkache) -> KeyedCommandMetadata {
        let descriptor = self.descriptor();
        KeyedCommandMetadata {
            operation: descriptor.operation,
            collapsible: (descriptor.collapsible)(cache, self),
        }
    }

    pub(super) fn prepare(
        self,
        cache: &mut Kvkache,
        storage_key: StorageKey,
    ) -> PreparedKeyedCommand {
        let prepare = self.descriptor().prepare;
        prepare(cache, storage_key, self)
    }

    pub(super) fn is_collapsible(&self, cache: &Kvkache) -> bool {
        self.metadata(cache).collapsible
    }

    pub(super) fn belongs_to_collapse_group(&self, collapse_group: &'static CollapseGroup) -> bool {
        std::ptr::eq(self.descriptor().collapse_group, collapse_group)
    }

    pub(super) fn is_exclusive(&self) -> bool {
        self.descriptor().exclusive
    }

    pub(super) fn take_exclusive(self) -> Option<(StorageTask, WorkerResponseSender)> {
        match self {
            Self::Custom { task, response } => Some((task, response)),
            Self::Get { .. } | Self::Set { .. } | Self::Delete { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct KeyedCommandMetadata {
    pub(super) operation: Operation,
    pub(super) collapsible: bool,
}

pub(super) struct PreparedKeyedCommand {
    pub(super) response: WorkerResponseSender,
    pub(super) job: PreparedJob,
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
    collapse_group: &CUSTOM_COLLAPSE_GROUP,
    exclusive: true,
};

static GET_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Get),
    collapsible: always_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: &COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

static SET_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Set),
    collapsible: set_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: &COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

static DELETE_DESCRIPTOR: KeyedDescriptor = KeyedDescriptor {
    operation: Operation::from_opcode(openkache_protocol::Opcode::Delete),
    collapsible: always_collapsible,
    prepare: prepare_command,
    collapse: reduce_compatibility_batch,
    collapse_group: &COMPATIBILITY_COLLAPSE_GROUP,
    exclusive: false,
};

static COMPATIBILITY_COLLAPSE_GROUP: CollapseGroup = CollapseGroup(1);
static CUSTOM_COLLAPSE_GROUP: CollapseGroup = CollapseGroup(2);

fn no_collapse(_base: KeyedVisibleState, _commands: Vec<StorageCommand>) -> CollapsedLaneBatch {
    unreachable!("non-collapsible keyed work cannot be reduced")
}

/// A response waiting for a collapsed mutation's actual storage result.
pub(super) struct CollapsedLaneBatch {
    pub(super) operation: Option<KeyedOperation>,
    pub(super) responses: Vec<DeferredWorkerResponse>,
    pub(super) mutation_response_index: Option<usize>,
    pub(super) success_state: VisibleState,
    pub(super) failure_state: VisibleState,
}

impl CollapsedLaneBatch {
    pub(super) fn reduce(base: KeyedVisibleState, commands: Vec<StorageCommand>) -> Self {
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
                value: WorkerResponse::Keyed(value),
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

    pub(super) fn has_mutation(&self) -> bool {
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

pub(super) struct PreparedCollapsed {
    pub(super) operation: Operation,
    pub(super) job: PreparedJob,
    pub(super) responses: Vec<DeferredWorkerResponse>,
    pub(super) mutation_response_index: Option<usize>,
    pub(super) success_state: VisibleState,
    pub(super) failure_state: VisibleState,
}

pub(super) fn prepare_collapsed_batch(
    cache: &mut Kvkache,
    storage_key: StorageKey,
    batch: CollapsedLaneBatch,
) -> PreparedCollapsed {
    batch.into_prepared(cache, storage_key)
}

pub(super) fn finish_keyed(
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
pub(super) struct KeyedFinish {
    pub(super) outcome: Result<WorkerResponse>,
    pub(super) visible_state: Option<VisibleState>,
    pub(super) flush_required: bool,
    pub(super) pending: bool,
}

pub(super) fn worker_response_for_outcome(outcome: KeyedOutcome) -> WorkerResponse {
    WorkerResponse::Keyed(match outcome {
        KeyedOutcome::Value(value) => KeyedResponse::Value(value),
        KeyedOutcome::Set(outcome) => KeyedResponse::Set(outcome),
        KeyedOutcome::Deleted(deleted) => KeyedResponse::Deleted(deleted),
    })
}

pub(super) fn pending_response(outcome: KeyedOutcome) -> WorkerResponse {
    worker_response_for_outcome(outcome)
}

pub(super) fn replace_mutation_response(
    response: &mut DeferredWorkerResponse,
    outcome: WorkerResponse,
) {
    response.value = outcome;
}

pub(super) fn value_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<Option<StoredItemValue>> {
    match response {
        WorkerResponse::Keyed(KeyedResponse::Value(value)) => Ok(value),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}

pub(super) fn set_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<SetOutcome> {
    match response {
        WorkerResponse::Keyed(KeyedResponse::Set(outcome)) => Ok(outcome),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}

pub(super) fn delete_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<bool> {
    match response {
        WorkerResponse::Keyed(KeyedResponse::Deleted(deleted)) => Ok(deleted),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}
