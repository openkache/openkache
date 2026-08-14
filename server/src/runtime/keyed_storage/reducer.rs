//! Reduction of contiguous keyed storage actions.

use crate::types::StorageWriteOptions;
use crate::store::{KeyedOperation, KeyedVisibleState};
use crate::{Kvkache, SetOutcome, StorageKey};
use crate::observability::Operation;

use super::super::worker_contract::CollapsedKeyedWork;
use super::super::{DeferredWorkerResponse, WorkerResponse};
use super::{Command, PreparedJob, Response, VisibleState};

/// Responses and visible states produced by one collapsed lane prefix.
pub(in crate::runtime) struct CollapsedBatch {
    pub(in crate::runtime) mutation: Option<CollapsedMutation>,
    pub(in crate::runtime) responses: Vec<DeferredWorkerResponse>,
    pub(in crate::runtime) success_state: VisibleState,
    pub(in crate::runtime) failure_state: VisibleState,
}

pub(in crate::runtime) struct CollapsedMutation {
    pub(in crate::runtime) operation: KeyedOperation,
    pub(in crate::runtime) response_index: usize,
}

impl CollapsedBatch {
    pub(in crate::runtime) fn reduce(base: KeyedVisibleState, commands: Vec<Command>) -> Self {
        let base_present = matches!(base, KeyedVisibleState::Present(_));
        let mut current = base.clone();
        let mut responses = Vec::with_capacity(commands.len());
        let mut last_mutation = None;

        for command in commands {
            let response_index = responses.len();
            let (response, value) = match command {
                Command::Task { .. } => {
                    unreachable!("exclusive storage tasks are never collapsed")
                }
                Command::Get { response, .. } => {
                    let value = match &current {
                        KeyedVisibleState::Missing => Response::Value(None),
                        KeyedVisibleState::Present(value) => Response::Value(Some(value.clone())),
                    };
                    (response, value)
                }
                Command::Set {
                    value,
                    options,
                    response,
                } => {
                    debug_assert_eq!(options, StorageWriteOptions::default());
                    let outcome = match current {
                        KeyedVisibleState::Missing => SetOutcome::Created,
                        KeyedVisibleState::Present(_) => SetOutcome::Replaced,
                    };
                    current = KeyedVisibleState::Present(value);
                    last_mutation = Some(response_index);
                    (response, Response::Set(outcome))
                }
                Command::Delete { response } => {
                    let deleted = matches!(current, KeyedVisibleState::Present(_));
                    current = KeyedVisibleState::Missing;
                    last_mutation = Some(response_index);
                    (response, Response::Deleted(deleted))
                }
            };
            responses.push(DeferredWorkerResponse {
                sender: response,
                value: WorkerResponse::Data(value),
            });
        }

        let mutation = last_mutation.and_then(|response_index| {
            let operation = match &current {
                KeyedVisibleState::Present(value) => KeyedOperation::Set {
                    value: value.clone(),
                    options: StorageWriteOptions::default(),
                },
                KeyedVisibleState::Missing if base_present => KeyedOperation::Delete,
                KeyedVisibleState::Missing => return None,
            };
            Some(CollapsedMutation {
                operation,
                response_index,
            })
        });

        Self {
            mutation,
            responses,
            success_state: current,
            failure_state: base,
        }
    }

    pub(super) fn into_work(
        self,
        cache: &mut Kvkache,
        storage_key: StorageKey,
    ) -> CollapsedKeyedWork<WorkerResponse, PreparedJob, VisibleState> {
        let Some(mutation) = self.mutation else {
            return CollapsedKeyedWork::Complete(self.responses);
        };
        let telemetry = match &mutation.operation {
            KeyedOperation::Get => Operation::storage_get(),
            KeyedOperation::Set { .. } => Operation::storage_set(),
            KeyedOperation::Delete => Operation::storage_delete(),
        };
        let job = cache.prepare_keyed(storage_key, mutation.operation);
        CollapsedKeyedWork::Prepared {
            operation: telemetry,
            job,
            responses: self.responses,
            mutation_response_index: mutation.response_index,
            success_state: self.success_state,
            failure_state: self.failure_state,
        }
    }
}
