//! Reduction of contiguous keyed storage actions.

use crate::observability::Operation;
use crate::store::{KeyedOperation, KeyedVisibleState};
use crate::types::StorageWriteOptions;
use crate::{Kvkache, SetOutcome, StorageKey};

use super::super::worker_contract::CollapsedKeyedWork;
use super::super::{DeferredWorkerResponse, WorkerResponse};
use super::{Command, PreparedJob, Response, VisibleState};

/// Responses and visible states produced by one collapsed lane prefix.
pub(in crate::runtime) struct CollapsedBatch {
    pub(in crate::runtime) mutation: Option<CollapsedMutation>,
    pub(in crate::runtime) success_state: VisibleState,
    pub(in crate::runtime) failure_state: VisibleState,
}

pub(in crate::runtime) struct CollapsedMutation {
    pub(in crate::runtime) telemetry: Operation,
    pub(in crate::runtime) operation: KeyedOperation,
    pub(in crate::runtime) response_index: usize,
}

impl CollapsedBatch {
    pub(in crate::runtime) fn reduce(
        base: KeyedVisibleState,
        commands: impl ExactSizeIterator<Item = Command>,
        mut defer: impl FnMut(DeferredWorkerResponse) -> usize,
    ) -> Self {
        let base_present = matches!(base, KeyedVisibleState::Present(_));
        let mut current = base.clone();
        let mut last_mutation = None;

        for command in commands {
            let (response, value, mutation) = match command {
                Command::Get { response, .. } => {
                    let value = match &current {
                        KeyedVisibleState::Missing => Response::Value(None),
                        KeyedVisibleState::Present(value) => Response::Value(Some(value.clone())),
                    };
                    (response, value, None)
                }
                Command::Set {
                    value,
                    metadata,
                    response,
                } => {
                    debug_assert_eq!(metadata.options(), StorageWriteOptions::default());
                    let outcome = match current {
                        KeyedVisibleState::Missing => SetOutcome::Created,
                        KeyedVisibleState::Present(_) => SetOutcome::Replaced,
                    };
                    current = KeyedVisibleState::Present(value);
                    (response, Response::Set(outcome), Some(metadata.operation))
                }
                Command::Delete {
                    operation,
                    response,
                } => {
                    let deleted = matches!(current, KeyedVisibleState::Present(_));
                    current = KeyedVisibleState::Missing;
                    (response, Response::Deleted(deleted), Some(operation))
                }
                Command::CompareExchange { .. } => {
                    unreachable!("non-collapsible compare/exchange reached the storage reducer")
                }
            };
            let response_index = defer(DeferredWorkerResponse {
                sender: response,
                value: WorkerResponse::Data(value),
            });
            if let Some(operation) = mutation {
                last_mutation = Some((response_index, operation));
            }
        }

        let mutation = last_mutation.and_then(|(response_index, telemetry)| {
            let operation = match &current {
                KeyedVisibleState::Present(value) => KeyedOperation::Set {
                    value: value.clone(),
                    options: StorageWriteOptions::default(),
                },
                KeyedVisibleState::Missing if base_present => KeyedOperation::Delete,
                KeyedVisibleState::Missing => return None,
            };
            Some(CollapsedMutation {
                telemetry,
                operation,
                response_index,
            })
        });

        Self {
            mutation,
            success_state: current,
            failure_state: base,
        }
    }

    pub(super) fn into_work(
        self,
        cache: &mut Kvkache,
        storage_key: StorageKey,
    ) -> CollapsedKeyedWork<PreparedJob, VisibleState> {
        let Some(mutation) = self.mutation else {
            return CollapsedKeyedWork::Complete;
        };
        let job = cache.prepare_keyed(storage_key, mutation.operation);
        CollapsedKeyedWork::Prepared {
            operation: mutation.telemetry,
            job,
            mutation_response_index: mutation.response_index,
            success_state: self.success_state,
            failure_state: self.failure_state,
        }
    }
}
