//! Keyed storage commands and their worker lifecycle.
//!
//! This module owns storage semantics shared by every API adapter. Adapters
//! construct commands and project the neutral storage response; the scheduler
//! remains unaware of both API families and storage actions.

mod completion;
mod lifecycle;
mod reducer;

use crate::observability::Operation;
use crate::store::{CompletedKeyedJob, KeyedJob, KeyedOperation, KeyedVisibleState};
use crate::types::{
    StorageWriteCondition, StorageWriteEviction, StorageWriteExpiration, StorageWriteOptions,
    StoredItemValue,
};
use crate::{KvError, Kvkache, SetOutcome, StorageKey};

use super::scheduler::ScheduledTask;
use super::worker_contract::{
    CollapsedKeyedWork, KeyedWorkMetadata, KeyedWorkPort, PreparedKeyedWork,
};
use super::{WorkerResponse, WorkerResponseSender};

pub(in crate::runtime) use crate::store::KeyedOutcome as Response;
pub(in crate::runtime) use reducer::CollapsedBatch;

pub(super) type VisibleState = KeyedVisibleState;
pub(super) type PreparedJob = KeyedJob;
pub(super) type CompletedJob = CompletedKeyedJob;

#[derive(Clone, Copy)]
pub(in crate::runtime) struct WriteMetadata {
    condition: StorageWriteCondition,
    expiration: StorageWriteExpiration,
    eviction: StorageWriteEviction,
    operation: Operation,
}

impl WriteMetadata {
    const fn new(options: StorageWriteOptions, operation: Operation) -> Self {
        Self {
            condition: options.condition,
            expiration: options.expiration,
            eviction: options.eviction,
            operation,
        }
    }

    const fn options(self) -> StorageWriteOptions {
        StorageWriteOptions {
            condition: self.condition,
            expiration: self.expiration,
            eviction: self.eviction,
        }
    }
}

/// One keyed storage command routed through a worker lane.
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

impl Command {
    fn metadata(&self, cache: &Kvkache) -> KeyedWorkMetadata {
        let (operation, collapsible) = match self {
            Self::Get { operation, .. } => (*operation, true),
            Self::Set {
                value,
                metadata,
                ..
            } => {
                let collapsible = metadata.options() == StorageWriteOptions::default()
                    && cache.can_collapse_set(value);
                (metadata.operation, collapsible)
            }
            Self::Delete { operation, .. } => (*operation, true),
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
        };
        PreparedKeyedWork {
            response,
            job: cache.prepare_keyed(storage_key, operation),
        }
    }
}

pub(in crate::runtime) fn value_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<Option<StoredItemValue>> {
    match response {
        WorkerResponse::Data(Response::Value(value)) => Ok(value),
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
        WorkerResponse::Data(Response::Set(outcome)) => Ok(outcome),
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
        WorkerResponse::Data(Response::Deleted(deleted)) => Ok(deleted),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CollapseGroup {
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
        defer: impl FnMut(super::worker_contract::DeferredResponse<Self::Response>) -> usize,
    ) -> CollapsedKeyedWork<Self::PreparedJob, Self::VisibleState> {
        CollapsedBatch::reduce(base, commands, defer).into_work(lifecycle, storage_key)
    }

    fn finish(
        lifecycle: &mut Kvkache,
        job: Self::CompletedJob,
        include_visible_state: bool,
    ) -> super::worker_contract::FinishedKeyedWork<Self::Response, Self::VisibleState> {
        completion::finish(lifecycle, job, include_visible_state)
    }

}
