//! Typed worker identifiers and borrow-only telemetry shard handles.

use std::time::Duration;

use crate::openkache_protocol::Status;

use super::service::{ObservabilityState, Operation};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub(crate) struct NetworkWorkerId(pub(crate) usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub(crate) struct StorageWorkerId(pub(crate) usize);

impl NetworkWorkerId {
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

impl StorageWorkerId {
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// Borrow-only access to one network worker's telemetry shard.
///
/// A worker creates this handle once and passes it by reference to connection
/// and stream futures. It does not clone the process-wide observability `Arc`
/// on every accepted connection or stream.
#[derive(Clone, Copy)]
pub(crate) struct NetworkShard<'a> {
    state: &'a ObservabilityState,
    worker: NetworkWorkerId,
}

impl<'a> NetworkShard<'a> {
    pub(crate) fn new(state: &'a ObservabilityState, worker: NetworkWorkerId) -> Self {
        Self { state, worker }
    }

    pub(crate) const fn worker_id(self) -> NetworkWorkerId {
        self.worker
    }

    pub(crate) fn connection_started(self) {
        self.state.connection_started_on(self.worker.index());
    }

    pub(crate) fn connection_finished(self) {
        self.state.connection_finished_on(self.worker.index());
    }

    pub(crate) fn handshake_succeeded(self) {
        self.state.handshake_succeeded_on(self.worker.index());
    }

    pub(crate) fn handshake_failed(self) {
        self.state.handshake_failed_on(self.worker.index());
    }

    pub(crate) fn stream_started(self) {
        self.state.stream_started_on(self.worker.index());
    }

    pub(crate) fn stream_finished(self) {
        self.state.stream_finished_on(self.worker.index());
    }

    pub(crate) fn protocol_error(self) {
        self.state.protocol_error_on(self.worker.index());
    }

    pub(crate) fn request_read_timeout(self) {
        self.state.request_read_timeout_on(self.worker.index());
    }

    pub(crate) fn response_write_failure(self) {
        self.state.response_write_failure_on(self.worker.index());
    }

    pub(crate) fn abandoned_request(self) {
        self.state.abandoned_request_on(self.worker.index());
    }

    pub(crate) fn record_request(self, operation: Operation, status: Status, elapsed: Duration) {
        self.state
            .record_request_on(self.worker.index(), operation, status, elapsed);
    }
}

/// Borrow-only access to one storage worker's telemetry shard.
#[derive(Clone, Copy)]
pub(crate) struct StorageShard<'a> {
    state: &'a ObservabilityState,
    worker: StorageWorkerId,
}

impl<'a> StorageShard<'a> {
    pub(crate) fn new(state: &'a ObservabilityState, worker: StorageWorkerId) -> Self {
        Self { state, worker }
    }

    pub(crate) fn record_operation(self, operation: Operation, elapsed: Duration) {
        self.state
            .record_storage_operation(self.worker.index(), operation, elapsed);
    }
}
