//! Keyed storage commands and their worker lifecycle.
//!
//! This module owns storage semantics shared by every API adapter. Adapters
//! construct commands and project the neutral storage response; the scheduler
//! remains unaware of both API families and storage actions.

mod action;
mod completion;
mod lifecycle;
mod reducer;

use crate::types::StoredItemValue;
use crate::{KvError, SetOutcome};

use super::WorkerResponse;
pub(in crate::runtime) use crate::store::KeyedOutcome as Response;
pub(in crate::runtime) use action::{
    Command, compare_exchange, delete, get, set,
};
pub(super) use action::{CompletedJob, PreparedJob, VisibleState};
pub(in crate::runtime) use reducer::CollapsedBatch;

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

pub(in crate::runtime) fn compare_exchange_response(
    response: WorkerResponse,
    operation: &'static str,
) -> crate::Result<bool> {
    match response {
        WorkerResponse::Data(Response::CompareExchange(changed)) => Ok(changed),
        response => Err(KvError::Worker(format!(
            "unexpected {operation} response: {response:?}"
        ))),
    }
}
