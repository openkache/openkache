//! Storage-owned keyed actions.
//!
//! The action enum is deliberately scoped to this adapter. The shared
//! scheduler and worker only consume the [`KeyedWorkPort`] contract, so adding
//! a different API action does not require changing generic runtime code.

use crate::observability::Operation;
use crate::store::{CompletedKeyedJob, KeyedJob, KeyedOperation, KeyedVisibleState};
use crate::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration, StorageWriteOptions,
    StoredItemValue,
};
use crate::{Kvkache, StorageKey};

use super::super::scheduler::ScheduledTask;
use super::super::worker_contract::{
    CollapsedKeyedWork, DeferredResponse, KeyedWorkMetadata, KeyedWorkPort, PreparedKeyedWork,
};
use super::super::{WorkerResponse, WorkerResponseSender};
use super::{CollapsedBatch, completion, lifecycle};

pub type VisibleState = KeyedVisibleState;
pub type PreparedJob = KeyedJob;
pub type CompletedJob = CompletedKeyedJob;

/// CAS-only control envelope.
///
/// One typed box keeps the bounded scheduler command at its existing size.
/// Both value owners move into it without copying payload bytes; GET, SET, and
/// DELETE retain their allocation-free command construction.
pub(in crate::runtime) struct CompareExchangeInput {
    expected: Option<StoredItemValue>,
    replacement: Option<StoredItemValue>,
}

#[derive(Clone, Copy)]
pub(in crate::runtime) struct WriteMetadata {
    pub(super) condition: StorageWriteCondition,
    pub(super) expiration: StorageWriteExpiration,
    pub(super) eviction: StorageWriteEviction,
    pub(super) operation: Operation,
}

impl WriteMetadata {
    pub(super) const fn new(options: StorageWriteOptions, operation: Operation) -> Self {
        Self {
            condition: options.condition,
            expiration: options.expiration,
            eviction: options.eviction,
            operation,
        }
    }

    pub(super) const fn options(self) -> StorageWriteOptions {
        StorageWriteOptions {
            condition: self.condition,
            expiration: self.expiration,
            eviction: self.eviction,
        }
    }
}

/// One keyed storage action routed through a worker lane.
pub(in crate::runtime) enum Command {
    Get {
        operation: Operation,
        response: WorkerResponseSender,
    },
    Set {
        value: StoredItemValue,
        metadata: WriteMetadata,
        response: WorkerResponseSender,
    },
    Delete {
        operation: Operation,
        response: WorkerResponseSender,
    },
    CompareExchange {
        input: Box<CompareExchangeInput>,
        metadata: WriteMetadata,
        response: WorkerResponseSender,
    },
}

pub(in crate::runtime) fn get(operation: Operation, response: WorkerResponseSender) -> Command {
    Command::Get {
        operation,
        response,
    }
}

pub(in crate::runtime) fn set(
    operation: Operation,
    value: StoredItemValue,
    options: StorageWriteOptions,
    response: WorkerResponseSender,
) -> Command {
    Command::Set {
        value,
        metadata: WriteMetadata::new(options, operation),
        response,
    }
}

pub(in crate::runtime) fn delete(operation: Operation, response: WorkerResponseSender) -> Command {
    Command::Delete {
        operation,
        response,
    }
}

pub(in crate::runtime) fn compare_exchange(
    operation: Operation,
    expected: Option<StoredItemValue>,
    replacement: Option<StoredItemValue>,
    options: StorageWriteOptions,
    response: WorkerResponseSender,
) -> Command {
    Command::CompareExchange {
        input: Box::new(CompareExchangeInput {
            expected,
            replacement,
        }),
        metadata: WriteMetadata::new(options, operation),
        response,
    }
}

impl Command {
    fn metadata(&self, cache: &Kvkache) -> KeyedWorkMetadata {
        let (operation, collapsible) = match self {
            Self::Get { operation, .. } => (*operation, true),
            Self::Set {
                value, metadata, ..
            } => {
                let collapsible = metadata.options() == StorageWriteOptions::default()
                    && cache.can_collapse_set(value);
                (metadata.operation, collapsible)
            }
            Self::Delete { operation, .. } => (*operation, true),
            Self::CompareExchange { metadata, .. } => (metadata.operation, false),
        };
        KeyedWorkMetadata {
            operation,
            collapsible,
        }
    }

    fn prepare(
        self,
        cache: &mut Kvkache,
        storage_key: StorageKey,
    ) -> PreparedKeyedWork<WorkerResponse, PreparedJob> {
        let (operation, response) = match self {
            Self::Get { response, .. } => (KeyedOperation::Get, response),
            Self::Set {
                value,
                metadata,
                response,
                ..
            } => (
                KeyedOperation::Set {
                    value,
                    options: metadata.options(),
                },
                response,
            ),
            Self::Delete { response, .. } => (KeyedOperation::Delete, response),
            Self::CompareExchange {
                input,
                metadata,
                response,
            } => {
                let CompareExchangeInput {
                    expected,
                    replacement,
                } = *input;
                (
                    KeyedOperation::CompareExchange {
                        expected,
                        replacement,
                        options: metadata.options(),
                    },
                    response,
                )
            }
        };
        PreparedKeyedWork {
            response,
            job: cache.prepare_keyed(storage_key, operation),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CollapseGroup {
    Storage,
}

impl ScheduledTask for Command {
    type CollapseGroup = CollapseGroup;

    fn collapse_group(&self) -> Self::CollapseGroup {
        CollapseGroup::Storage
    }
}

impl KeyedWorkPort<Kvkache, StorageKey> for Command {
    type Response = WorkerResponse;
    type PreparedJob = PreparedJob;
    type CompletedJob = CompletedJob;
    type VisibleState = VisibleState;
    type Lifecycle = lifecycle::Lifecycle;

    fn metadata(&self, lifecycle: &Kvkache) -> KeyedWorkMetadata {
        Command::metadata(self, lifecycle)
    }

    fn prepare(
        self,
        lifecycle: &mut Kvkache,
        storage_key: StorageKey,
    ) -> PreparedKeyedWork<Self::Response, Self::PreparedJob> {
        Command::prepare(self, lifecycle, storage_key)
    }

    fn run(job: Self::PreparedJob) -> impl std::future::Future<Output = Self::CompletedJob> {
        job.run()
    }

    fn collapse(
        lifecycle: &mut Kvkache,
        storage_key: StorageKey,
        base: Self::VisibleState,
        commands: impl ExactSizeIterator<Item = Self>,
        defer: impl FnMut(DeferredResponse<Self::Response>) -> usize,
    ) -> CollapsedKeyedWork<Self::PreparedJob, Self::VisibleState> {
        CollapsedBatch::reduce(base, commands, defer).into_work(lifecycle, storage_key)
    }

    fn finish(
        lifecycle: &mut Kvkache,
        job: Self::CompletedJob,
        include_visible_state: bool,
    ) -> super::super::worker_contract::FinishedKeyedWork<Self::Response, Self::VisibleState> {
        completion::finish(lifecycle, job, include_visible_state)
    }
}
