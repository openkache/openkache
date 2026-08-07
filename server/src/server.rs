//! QUIC server backed by the sharded SSD-first cache runtime.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use futures_util::lock::Mutex as AsyncMutex;
use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{Opcode, Status};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::channel::{self, AsyncReceiver, Sender};
use crate::contract::{MAX_REQUEST_FRAME_BYTES, NAMESPACE_NAME_MAX_BYTES};
use crate::network_runtime;
use crate::observability::{
    NetworkShard, NetworkWorkerId, ObservabilityService, ObservabilityState, Operation,
};
use crate::platform::StorageDeviceKind;
use crate::protocol::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, ItemId, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, Request, Response, SetOptions,
};
use crate::transport::{
    Connection as TransportConnection, Endpoint as TransportEndpoint,
    Incoming as TransportIncoming, ReceiveStream, RequestBudget, SendStream, ServerEndpoint,
    ServerTlsConfig, StreamReadError, TransportError,
};
use crate::{
    AppConfig, KvError, NetworkConfig, NetworkWorkerCache, QuicBackend, SetOutcome,
    ThreadedKvkache, TlsConfig,
};
#[allow(unused_imports)]
pub(crate) use crate::{contract, protocol};

#[path = "operation_codecs.rs"]
mod operation_codecs;
#[path = "operation_extensions.rs"]
mod operation_extensions;
#[path = "operation_handlers.rs"]
mod operation_handlers;

pub(crate) type NetworkWorkerCompletion = (usize, std::result::Result<(), String>);

const NAMESPACE_METADATA_MAGIC: &[u8; 8] = b"OKNSPACE";
const NAMESPACE_METADATA_VERSION: u32 = 2;
const NAMESPACE_METADATA_LEGACY_VERSION: u32 = 1;
const NAMESPACE_METADATA_MAX_ENTRIES: u64 = 1_000_000;
const NAMESPACE_METADATA_MAX_ITEMS_PER_ENTRY: u64 = 1_000_000_000;
const NAMESPACE_METADATA_MAX_DIRTY_WORKERS: u64 = 1_000_000;
static NEXT_NAMESPACE_METADATA_TEMP: AtomicU64 = AtomicU64::new(0);

pub(crate) struct NetworkWorkerHandle {
    stop: Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct NetworkRolePlacement {
    worker_index: usize,
    cpu_id: usize,
    thread_name: String,
    entries: u32,
    event_interval: usize,
    stop: Sender<()>,
}

impl NetworkRolePlacement {
    pub(crate) fn new(
        worker_index: usize,
        cpu_id: usize,
        thread_name: String,
        entries: u32,
        event_interval: usize,
        stop: Sender<()>,
    ) -> Self {
        Self {
            worker_index,
            cpu_id,
            thread_name,
            entries,
            event_interval,
            stop,
        }
    }
}

pub(crate) struct NetworkWorkerReporter {
    worker_id: usize,
    started: Option<Sender<std::result::Result<(), String>>>,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkWorkerReporter {
    pub(crate) fn new(
        worker_id: usize,
        started: Sender<std::result::Result<(), String>>,
        finished: Sender<(usize, std::result::Result<(), String>)>,
    ) -> Self {
        Self {
            worker_id,
            started: Some(started),
            finished: Some(finished),
        }
    }

    pub(crate) fn startup_failed(mut self, message: String) {
        if let Some(started) = self.started.take() {
            let _ = started.send(Err(message));
        }
    }

    pub(crate) fn started(&mut self) -> bool {
        self.started
            .take()
            .is_some_and(|started| started.send(Ok(())).is_ok())
    }

    fn take_completion_sender(&mut self) -> Sender<NetworkWorkerCompletion> {
        self.finished
            .take()
            .expect("network worker completion sender is available at launch")
    }
}

impl Drop for NetworkWorkerReporter {
    fn drop(&mut self) {
        if let Some(started) = self.started.take() {
            let failure = if std::thread::panicking() {
                format!("network worker {} panicked during startup", self.worker_id)
            } else {
                format!(
                    "network worker {} exited without reporting startup",
                    self.worker_id
                )
            };
            let _ = started.send(Err(failure));
        }
    }
}

pub(crate) struct NetworkTaskReporter {
    worker_id: usize,
    finished: Option<Sender<NetworkWorkerCompletion>>,
}

impl NetworkTaskReporter {
    pub(crate) fn new(worker_id: usize, finished: Sender<NetworkWorkerCompletion>) -> Self {
        Self {
            worker_id,
            finished: Some(finished),
        }
    }

    fn finish(mut self, result: std::result::Result<(), String>) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send((self.worker_id, result));
        }
    }
}

impl Drop for NetworkTaskReporter {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let failure = if std::thread::panicking() {
                "panicked"
            } else {
                "exited without reporting completion"
            };
            let _ = finished.send((self.worker_id, Err(failure.into())));
        }
    }
}

async fn run_network_role_task<F, Fut>(
    task_reporter: NetworkTaskReporter,
    reporter: NetworkWorkerReporter,
    role: F,
) where
    F: FnOnce(NetworkWorkerReporter) -> Fut,
    Fut: Future<Output = Option<std::result::Result<(), String>>>,
{
    let result = role(reporter).await.unwrap_or(Ok(()));
    task_reporter.finish(result);
}

pub(crate) fn launch_network_role<F, Fut>(
    cache: &ThreadedKvkache,
    placement: NetworkRolePlacement,
    mut reporter: NetworkWorkerReporter,
    role: F,
) -> Result<NetworkWorkerHandle>
where
    F: FnOnce(NetworkWorkerReporter) -> Fut + Send + 'static,
    Fut: Future<Output = Option<std::result::Result<(), String>>> + 'static,
{
    let NetworkRolePlacement {
        worker_index,
        cpu_id,
        thread_name,
        entries,
        event_interval,
        stop,
    } = placement;
    let worker_id = reporter.worker_id;
    let finished = reporter.take_completion_sender();
    if cache.can_run_on_storage_cpu(cpu_id) {
        let attached = cache.run_on_storage_cpu(cpu_id, move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
            network_runtime::spawn_detached(run_network_role_task(task_reporter, reporter, role));
        })?;
        if !attached {
            return Err(ServerError::NetworkWorker(format!(
                "storage runtime on CPU {cpu_id} rejected its prepared network role"
            )));
        }
        return Ok(NetworkWorkerHandle { stop, thread: None });
    }

    let thread = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let task_reporter = NetworkTaskReporter::new(worker_id, finished);
            if let Err(error) = network_runtime::run(
                network_runtime::RuntimeConfig {
                    entries,
                    event_interval,
                    cpu_id: Some(cpu_id),
                    worker_index: Some(worker_index),
                },
                run_network_role_task(task_reporter, reporter, role),
            ) {
                eprintln!("network worker {worker_id} runtime failed: {error}");
            }
        })?;
    Ok(NetworkWorkerHandle {
        stop,
        thread: Some(thread),
    })
}

enum AccessPolicy {
    InsecureDevelopment,
    MutualTls {
        admin_client_certificates: Vec<CertificateDer<'static>>,
    },
}

#[derive(Clone)]
struct NamespaceEntry {
    descriptor: NamespaceDescriptor,
    name: Vec<u8>,
    items: HashSet<ItemId>,
    dirty_workers: HashSet<usize>,
    operation_lock: Arc<AsyncMutex<()>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SetReservation {
    inserted_item: bool,
    inserted_worker: bool,
}

// Namespace names and IDs are server-wide. Authentication and any future
// owner/ACL mapping are separate authorization concerns and do not participate
// in registry lookup.
struct NamespaceRegistry {
    /// The next never-before-issued ID. `None` means the u64 ID space is exhausted.
    next_id: Option<u64>,
    by_id: HashMap<u64, NamespaceEntry>,
    by_name: HashMap<Vec<u8>, u64>,
    metadata_path: Option<std::path::PathBuf>,
    persistent: bool,
    lifecycle_lock: Arc<AsyncMutex<()>>,
}

impl NamespaceRegistry {
    fn load(directory: &Path, existing_storage: bool) -> std::io::Result<Self> {
        let metadata_path = match crate::storage_backend::namespace_persistence(directory) {
            crate::storage_backend::NamespacePersistence::Durable(path) => path,
            crate::storage_backend::NamespacePersistence::Ephemeral => {
                return Ok(Self::ephemeral(directory));
            }
        };
        let mut registry = Self {
            next_id: Some(1),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            metadata_path: Some(metadata_path),
            persistent: true,
            lifecycle_lock: Arc::new(AsyncMutex::new(())),
        };
        let mut file = match std::fs::File::open(
            registry
                .metadata_path
                .as_ref()
                .expect("durable namespace registry has metadata path"),
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if existing_storage {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        "namespace metadata is missing for existing storage",
                    ));
                }
                return Ok(registry);
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        registry.decode_metadata(&bytes)?;
        Ok(registry)
    }

    fn ephemeral(_directory: &Path) -> Self {
        Self {
            next_id: Some(1),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            metadata_path: None,
            persistent: false,
            lifecycle_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    fn lifecycle_lock(&self) -> Arc<AsyncMutex<()>> {
        Arc::clone(&self.lifecycle_lock)
    }

    fn open(
        &mut self,
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> std::result::Result<(Status, NamespaceDescriptor), Status> {
        if let Some(namespace_id) = self.by_name.get(&name).copied() {
            let Some(entry) = self.by_id.get(&namespace_id) else {
                return Err(Status::InternalError);
            };
            return Ok((Status::Ok, entry.descriptor));
        }
        if !create_if_missing {
            return Err(Status::NamespaceNotFound);
        }
        let policy = policy.ok_or(Status::InvalidRequest)?;
        let previous_next_id = self.next_id;
        let namespace_id = self.allocate_id()?;
        let descriptor = NamespaceDescriptor {
            namespace_id,
            revision: 1,
            policy,
        };
        self.by_name.insert(name.clone(), namespace_id);
        self.by_id.insert(
            namespace_id,
            NamespaceEntry {
                descriptor,
                name: name.clone(),
                items: HashSet::new(),
                dirty_workers: HashSet::new(),
                operation_lock: Arc::new(AsyncMutex::new(())),
            },
        );
        if self.persist().is_err() {
            self.by_id.remove(&namespace_id);
            self.by_name.remove(&name);
            self.next_id = previous_next_id;
            return Err(Status::InternalError);
        }
        Ok((Status::Created, descriptor))
    }

    fn allocate_id(&mut self) -> std::result::Result<u64, Status> {
        let namespace_id = self.next_id.ok_or(Status::InternalError)?;
        self.next_id = if namespace_id == u64::MAX {
            None
        } else {
            Some(namespace_id + 1)
        };
        Ok(namespace_id)
    }

    fn operation_lock(&self, namespace_id: u64) -> Option<Arc<AsyncMutex<()>>> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| Arc::clone(&entry.operation_lock))
    }

    fn descriptor(&self, namespace_id: u64) -> Option<NamespaceDescriptor> {
        self.by_id.get(&namespace_id).map(|entry| entry.descriptor)
    }

    fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| entry.descriptor.policy)
    }

    fn update(
        &mut self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> std::result::Result<NamespaceDescriptor, Status> {
        let (previous, descriptor) = {
            let entry = self
                .by_id
                .get_mut(&namespace_id)
                .ok_or(Status::NamespaceNotFound)?;
            if entry.descriptor.revision != expected_revision {
                return Err(Status::Conflict);
            }
            let previous = entry.descriptor;
            entry.descriptor.revision = entry
                .descriptor
                .revision
                .checked_add(1)
                .ok_or(Status::InternalError)?;
            entry.descriptor.policy = policy;
            (previous, entry.descriptor)
        };
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.descriptor = previous;
            }
            return Err(Status::InternalError);
        }
        Ok(descriptor)
    }

    fn delete(
        &mut self,
        namespace_id: u64,
        expected_revision: u64,
    ) -> std::result::Result<(), Status> {
        let entry = self
            .by_id
            .get(&namespace_id)
            .ok_or(Status::NamespaceNotFound)?;
        if entry.descriptor.revision != expected_revision {
            return Err(Status::Conflict);
        }
        if !entry.items.is_empty() {
            return Err(Status::NamespaceNotEmpty);
        }
        let Some(entry) = self.by_id.remove(&namespace_id) else {
            return Err(Status::NamespaceNotFound);
        };
        self.by_name.remove(&entry.name);
        if self.persist().is_err() {
            self.by_name.insert(entry.name.clone(), namespace_id);
            self.by_id.insert(namespace_id, entry);
            return Err(Status::InternalError);
        }
        Ok(())
    }

    /// Records an item before a SET is dispatched to storage.
    ///
    /// Persisting the conservative "possibly present" state first means a
    /// crash between storage mutation and metadata update cannot make
    /// `IfEmpty` deletion incorrectly report an empty namespace.
    fn reserve_item(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        worker: usize,
    ) -> std::result::Result<SetReservation, Status> {
        let reservation = self
            .by_id
            .get_mut(&namespace_id)
            .ok_or(Status::NamespaceNotFound)
            .map(|entry| SetReservation {
                inserted_item: entry.items.insert(item_id),
                inserted_worker: entry.dirty_workers.insert(worker),
            })?;
        if !reservation.inserted_item && !reservation.inserted_worker {
            return Ok(reservation);
        }
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                if reservation.inserted_item {
                    entry.items.remove(&item_id);
                }
                if reservation.inserted_worker {
                    entry.dirty_workers.remove(&worker);
                }
            }
            return Err(Status::InternalError);
        }
        Ok(reservation)
    }

    /// Reverses a SET reservation when storage reports a definitive
    /// no-mutation result.
    ///
    /// The namespace operation lock prevents another operation from changing
    /// the same entry between reservation and rollback. If metadata
    /// persistence fails, the conservative reservation is restored and the
    /// caller must treat the result as ambiguous.
    fn rollback_set_reservation(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        worker: usize,
        reservation: SetReservation,
    ) -> std::result::Result<(), Status> {
        if !reservation.inserted_item && !reservation.inserted_worker {
            return Ok(());
        }
        {
            let Some(entry) = self.by_id.get_mut(&namespace_id) else {
                return Err(Status::NamespaceNotFound);
            };
            if reservation.inserted_item {
                entry.items.remove(&item_id);
            }
            if reservation.inserted_worker {
                entry.dirty_workers.remove(&worker);
            }
        }
        if self.persist().is_err() {
            if reservation.inserted_item {
                if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                    entry.items.insert(item_id);
                }
            }
            if reservation.inserted_worker {
                if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                    entry.dirty_workers.insert(worker);
                }
            }
            return Err(Status::InternalError);
        }
        Ok(())
    }

    /// Records a worker before a DELETE is dispatched to storage.
    ///
    /// The marker is intentionally conservative: a DELETE that finds no item
    /// still leaves the worker dirty until the next successful `SYNC`.
    fn reserve_worker(
        &mut self,
        namespace_id: u64,
        worker: usize,
    ) -> std::result::Result<(), Status> {
        let inserted = self
            .by_id
            .get_mut(&namespace_id)
            .ok_or(Status::NamespaceNotFound)?
            .dirty_workers
            .insert(worker);
        if !inserted {
            return Ok(());
        }
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.dirty_workers.remove(&worker);
            }
            return Err(Status::InternalError);
        }
        Ok(())
    }

    fn mark_delete(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        deleted: bool,
    ) -> std::result::Result<(), Status> {
        let removed = deleted
            && self
                .by_id
                .get_mut(&namespace_id)
                .is_some_and(|entry| entry.items.remove(&item_id));
        if removed && self.persist().is_err() {
            // Keeping the item in memory is conservative when persistence
            // fails; the caller closes the lane because the mutation outcome
            // can no longer be represented reliably.
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.items.insert(item_id);
            }
            return Err(Status::InternalError);
        }
        Ok(())
    }

    fn tracked_items(&self, namespace_id: u64) -> Option<Vec<ItemId>> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| entry.items.iter().copied().collect())
    }

    fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<usize>> {
        self.by_id.get(&namespace_id).map(|entry| {
            let mut workers = entry.dirty_workers.iter().copied().collect::<Vec<_>>();
            workers.sort_unstable();
            workers
        })
    }

    fn mark_workers_clean(&mut self, namespace_id: u64) -> std::result::Result<(), Status> {
        let previous = {
            let entry = self
                .by_id
                .get_mut(&namespace_id)
                .ok_or(Status::NamespaceNotFound)?;
            let previous = entry.dirty_workers.clone();
            entry.dirty_workers.clear();
            previous
        };
        if previous.is_empty() {
            return Ok(());
        }
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.dirty_workers = previous;
            }
            return Err(Status::InternalError);
        }
        Ok(())
    }

    fn prune_item(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> std::result::Result<(), Status> {
        let Some(entry) = self.by_id.get_mut(&namespace_id) else {
            return Err(Status::NamespaceNotFound);
        };
        if !entry.items.remove(&item_id) {
            return Ok(());
        }
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.items.insert(item_id);
            }
            return Err(Status::InternalError);
        }
        Ok(())
    }

    fn persist(&self) -> std::io::Result<()> {
        if !self.persistent {
            return Ok(());
        }
        let mut entries = self.by_id.values().collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.descriptor.namespace_id);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(NAMESPACE_METADATA_MAGIC);
        bytes.extend_from_slice(&NAMESPACE_METADATA_VERSION.to_be_bytes());
        bytes.extend_from_slice(&self.next_id.unwrap_or(0).to_be_bytes());
        bytes.extend_from_slice(&(entries.len() as u64).to_be_bytes());
        for entry in entries {
            let name_len = u16::try_from(entry.name.len()).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidData, "namespace name is too long")
            })?;
            bytes.extend_from_slice(&entry.descriptor.namespace_id.to_be_bytes());
            bytes.extend_from_slice(&entry.descriptor.revision.to_be_bytes());
            bytes.extend_from_slice(&name_len.to_be_bytes());
            bytes.extend_from_slice(&entry.name);
            let policy =
                entry.descriptor.policy.encode().map_err(|error| {
                    std::io::Error::new(ErrorKind::InvalidData, error.to_string())
                })?;
            let policy_len = u8::try_from(policy.len())
                .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "policy is too long"))?;
            bytes.push(policy_len);
            bytes.extend_from_slice(&policy);
            bytes.extend_from_slice(&(entry.items.len() as u64).to_be_bytes());
            let mut items = entry.items.iter().copied().collect::<Vec<_>>();
            items.sort_unstable();
            for item_id in items {
                bytes.extend_from_slice(item_id.as_bytes());
            }
            bytes.extend_from_slice(&(entry.dirty_workers.len() as u64).to_be_bytes());
            let mut dirty_workers = entry.dirty_workers.iter().copied().collect::<Vec<_>>();
            dirty_workers.sort_unstable();
            for worker in dirty_workers {
                bytes.extend_from_slice(
                    &u64::try_from(worker)
                        .map_err(|_| {
                            std::io::Error::new(
                                ErrorKind::InvalidData,
                                "storage worker ID does not fit metadata",
                            )
                        })?
                        .to_be_bytes(),
                );
            }
        }

        let metadata_path = self
            .metadata_path
            .as_ref()
            .expect("persistent namespace registry has metadata path");
        let sequence = NEXT_NAMESPACE_METADATA_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary_path = self
            .metadata_path
            .as_ref()
            .expect("persistent namespace registry has metadata path")
            .with_extension(format!("tmp-{}-{sequence}", std::process::id()));
        let write_result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary_path, metadata_path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(&temporary_path);
        }
        write_result
    }

    fn decode_metadata(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut cursor = MetadataCursor::new(bytes);
        if cursor.take(NAMESPACE_METADATA_MAGIC.len())? != NAMESPACE_METADATA_MAGIC {
            return Err(cursor.invalid("namespace metadata magic is invalid"));
        }
        let metadata_version = cursor.u32()?;
        if metadata_version != NAMESPACE_METADATA_VERSION
            && metadata_version != NAMESPACE_METADATA_LEGACY_VERSION
        {
            return Err(cursor.invalid("namespace metadata version is unsupported"));
        }
        let next_id = cursor.u64()?;
        self.next_id = (next_id != 0).then_some(next_id);
        let entry_count = cursor.u64()?;
        if entry_count > NAMESPACE_METADATA_MAX_ENTRIES {
            return Err(cursor.invalid("namespace metadata contains too many namespaces"));
        }
        for _ in 0..entry_count {
            let namespace_id = cursor.u64()?;
            let revision = cursor.u64()?;
            if namespace_id == 0 || revision == 0 {
                return Err(cursor.invalid("namespace metadata contains zero identity"));
            }
            let name_len = usize::from(cursor.u16()?);
            if name_len > NAMESPACE_NAME_MAX_BYTES {
                return Err(cursor.invalid("namespace metadata name is too long"));
            }
            let name = cursor.take(name_len)?.to_vec();
            std::str::from_utf8(&name)
                .map_err(|_| cursor.invalid("namespace metadata name is not UTF-8"))?;
            let policy_len = usize::from(cursor.u8()?);
            let policy_bytes = cursor.take(policy_len)?;
            let (policy, used) = NamespacePolicy::decode(policy_bytes)
                .map_err(|error| cursor.invalid(error.to_string()))?
                .ok_or_else(|| cursor.invalid("namespace metadata policy is truncated"))?;
            if used != policy_len {
                return Err(cursor.invalid("namespace metadata policy has trailing bytes"));
            }
            let item_count = cursor.u64()?;
            if item_count > NAMESPACE_METADATA_MAX_ITEMS_PER_ENTRY
                || item_count > (cursor.remaining() / openkache_protocol::ITEM_ID_BYTES) as u64
            {
                return Err(cursor.invalid("namespace metadata item list is invalid"));
            }
            let mut items = HashSet::with_capacity(item_count as usize);
            for _ in 0..item_count {
                let item_bytes = cursor.take(openkache_protocol::ITEM_ID_BYTES)?;
                let item_id = ItemId::new(item_bytes.try_into().expect("item ID width is fixed"));
                items.insert(item_id);
            }
            let mut dirty_workers = HashSet::new();
            if metadata_version >= NAMESPACE_METADATA_VERSION {
                let dirty_worker_count = cursor.u64()?;
                if dirty_worker_count > NAMESPACE_METADATA_MAX_DIRTY_WORKERS {
                    return Err(cursor.invalid("namespace metadata dirty-worker list is invalid"));
                }
                for _ in 0..dirty_worker_count {
                    let worker = usize::try_from(cursor.u64()?)
                        .map_err(|_| cursor.invalid("namespace metadata worker ID is invalid"))?;
                    if !dirty_workers.insert(worker) {
                        return Err(
                            cursor.invalid("namespace metadata contains duplicate dirty workers")
                        );
                    }
                }
            }
            if self.by_id.contains_key(&namespace_id) || self.by_name.contains_key(&name) {
                return Err(cursor.invalid("namespace metadata contains duplicate identity"));
            }
            self.by_name.insert(name.clone(), namespace_id);
            self.by_id.insert(
                namespace_id,
                NamespaceEntry {
                    descriptor: NamespaceDescriptor {
                        namespace_id,
                        revision,
                        policy,
                    },
                    name,
                    items,
                    dirty_workers,
                    operation_lock: Arc::new(AsyncMutex::new(())),
                },
            );
        }
        if cursor.remaining() != 0 {
            return Err(cursor.invalid("namespace metadata has trailing bytes"));
        }
        let maximum_id = self.by_id.keys().copied().max();
        match (maximum_id, self.next_id) {
            (Some(maximum_id), Some(next_id)) if next_id <= maximum_id => {
                return Err(cursor.invalid("namespace metadata next ID is not monotonic"));
            }
            (Some(_), None) if maximum_id != Some(u64::MAX) => {
                return Err(cursor.invalid("namespace metadata marks IDs exhausted early"));
            }
            _ => {}
        }
        Ok(())
    }
}

struct MetadataCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> MetadataCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> std::io::Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| self.invalid("namespace metadata length overflowed"))?;
        if end > self.bytes.len() {
            return Err(self.invalid("namespace metadata is truncated"));
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> std::io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> std::io::Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("u16 width is fixed"),
        ))
    }

    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("u32 width is fixed"),
        ))
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("u64 width is fixed"),
        ))
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn invalid(&self, message: impl Into<String>) -> std::io::Error {
        std::io::Error::new(ErrorKind::InvalidData, message.into())
    }
}

impl AccessPolicy {
    fn permits_administration(&self, peer_certificate: Option<&CertificateDer<'_>>) -> bool {
        match self {
            Self::InsecureDevelopment => true,
            Self::MutualTls {
                admin_client_certificates,
            } => peer_certificate.is_some_and(|peer| {
                admin_client_certificates
                    .iter()
                    .any(|administrator| administrator.as_ref() == peer.as_ref())
            }),
        }
    }
}

fn load_production_tls(config: &TlsConfig) -> Result<(ServerTlsConfig, AccessPolicy)> {
    let certificate_chain = load_certificates(
        config
            .certificate_chain
            .as_deref()
            .expect("validated production TLS certificate path"),
    )?;
    let private_key = load_private_key(
        config
            .private_key
            .as_deref()
            .expect("validated production TLS private key path"),
    )?;
    let client_ca = load_certificates(
        config
            .client_ca
            .as_deref()
            .expect("validated production TLS client CA path"),
    )?;
    let mut admin_client_certificates = Vec::with_capacity(config.admin_client_certificates.len());
    for path in &config.admin_client_certificates {
        let certificate = load_certificates(path)?
            .into_iter()
            .next()
            .expect("certificate loader rejects empty files");
        admin_client_certificates.push(certificate);
    }
    Ok((
        ServerTlsConfig {
            certificate_chain,
            private_key,
            client_ca,
        },
        AccessPolicy::MutualTls {
            admin_client_certificates,
        },
    ))
}

fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let bytes = std::fs::read(path).map_err(|error| ServerError::TlsIdentity {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let certificates = if bytes.starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(&bytes)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| ServerError::TlsIdentity {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?
    } else {
        vec![CertificateDer::from(bytes)]
    };
    if certificates.is_empty() {
        return Err(ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: "no certificates found".into(),
        });
    }
    Ok(certificates)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let bytes = std::fs::read(path).map_err(|error| ServerError::TlsIdentity {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if bytes.starts_with(b"-----BEGIN") {
        PrivateKeyDer::from_pem_slice(&bytes).map_err(|error| ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
    } else {
        PrivateKeyDer::try_from(bytes).map_err(|message| ServerError::TlsIdentity {
            path: path.to_path_buf(),
            message: message.into(),
        })
    }
}

/// Bound reuse-port sockets and the sharded SSD-backed cache they serve.
pub struct KacheServer {
    sockets: Vec<std::net::UdpSocket>,
    local_addr: SocketAddr,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    network: NetworkConfig,
    request_timeout: Duration,
    max_item_bytes: usize,
    observability: ObservabilityService,
}

impl KacheServer {
    /// Binds a server with an explicit SSD cache configuration.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing bound sockets, configured TLS identity, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when production TLS is missing or invalid, configuration validation or
    /// socket binding fails, or cache startup fails.
    pub async fn bind_with_config(address: SocketAddr, config: AppConfig) -> Result<Self> {
        config.validate()?;
        if !config.tls.is_configured() {
            return Err(ServerError::ProductionTlsRequired(address));
        }
        let (tls, access_policy) = load_production_tls(&config.tls)?;
        Self::bind_with_security(address, config, tls, access_policy).await
    }

    /// Binds with a generated certificate and no peer authentication for development only.
    ///
    /// This mode grants every connected peer administrative access and must not be used for
    /// production deployments.
    ///
    /// # Arguments
    ///
    /// * `address` - UDP address on which the QUIC endpoint listens.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing bound sockets, an ephemeral certificate, and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when production TLS is also configured, configuration validation,
    /// certificate generation, socket binding, or cache startup fails.
    pub async fn bind_insecure_for_development(
        address: SocketAddr,
        config: AppConfig,
    ) -> Result<Self> {
        if config.tls.is_configured() {
            return Err(ServerError::ConflictingSecurityModes);
        }
        config.validate()?;
        let mut subject_alt_names = vec!["localhost".to_string()];
        if !address.ip().is_unspecified() {
            subject_alt_names.push(address.ip().to_string());
        }
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(subject_alt_names)?;
        let tls = ServerTlsConfig {
            certificate_chain: vec![cert.into()],
            private_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                signing_key.serialize_der(),
            )),
            client_ca: Vec::new(),
        };
        Self::bind_with_security(address, config, tls, AccessPolicy::InsecureDevelopment).await
    }

    async fn bind_with_security(
        address: SocketAddr,
        config: AppConfig,
        tls: ServerTlsConfig,
        access_policy: AccessPolicy,
    ) -> Result<Self> {
        if let Err(message) = operation_handlers::validate_handler_registry() {
            return Err(ServerError::NetworkWorker(message.into()));
        }
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let max_item_bytes = config.storage.max_item_size_mib * 1024 * 1024;
        let network = config.network.clone();
        let storage_directory = config.storage.directory.clone();
        let observability = ObservabilityService::new(
            network.worker_count,
            config.runtime.thread_count,
            &config.observability,
        )?;
        let observability_state = observability.state();
        let existing_storage = crate::storage_backend::existing_storage(&config);
        let quic_backend = config.quic.selected_backend()?;
        ServerEndpoint::validate_backend(quic_backend)?;
        let mut cache = ThreadedKvkache::start_validated_for_server_with_observability(
            config,
            observability_state,
        )?;
        let namespaces = match NamespaceRegistry::load(&storage_directory, existing_storage) {
            Ok(registry) => registry,
            Err(error) => {
                cache.shutdown()?;
                return Err(ServerError::NamespaceMetadata(error.to_string()));
            }
        };
        if let Err(error) = namespaces.persist() {
            cache.shutdown()?;
            return Err(ServerError::NamespaceMetadata(error.to_string()));
        }
        let sockets = match bind_reuse_port_sockets(address, network.worker_count) {
            Ok(sockets) => sockets,
            Err(error) => {
                cache.shutdown()?;
                return Err(error.into());
            }
        };
        let local_addr = sockets[0].local_addr()?;
        let cache = Arc::new(cache);
        Ok(Self {
            sockets,
            local_addr,
            quic_backend,
            tls: Arc::new(tls),
            access_policy: Arc::new(access_policy),
            cache,
            namespaces: Arc::new(Mutex::new(namespaces)),
            network,
            request_timeout,
            max_item_bytes,
            observability,
        })
    }

    /// Returns the UDP address selected by the operating system.
    ///
    /// # Returns
    ///
    /// The bound local address shared by all reuse-port sockets.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored socket address cannot be reported.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Returns the bound observability management address, when enabled.
    ///
    /// # Returns
    ///
    /// The address bound for the management listener, or `None` when
    /// observability metrics are disabled.
    pub fn metrics_addr(&self) -> Option<SocketAddr> {
        self.observability.metrics_addr()
    }

    /// Returns the conservative classification of the files opened by the
    /// storage workers during bind.
    ///
    /// # Returns
    ///
    /// The aggregate classification of every data and large-value file opened
    /// by the storage workers.
    pub fn storage_device_kind(&self) -> StorageDeviceKind {
        self.cache.storage_device_kind()
    }

    /// Returns the leaf certificate clients must trust directly or through its issuing CA.
    ///
    /// # Returns
    ///
    /// The configured or generated leaf certificate encoded as DER bytes.
    pub fn certificate_der(&self) -> &[u8] {
        self.tls
            .certificate_chain
            .first()
            .expect("validated TLS certificate chain")
            .as_ref()
    }

    /// Accepts connections until `shutdown` resolves, then flushes all cache workers.
    ///
    /// # Arguments
    ///
    /// * `shutdown` - Future whose completion initiates graceful server shutdown.
    ///
    /// # Returns
    ///
    /// `Ok(())` after active connections close and all cache workers flush and stop.
    ///
    /// # Errors
    ///
    /// Returns an error when a network worker fails or cache shutdown fails.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> Result<()> {
        let Self {
            sockets,
            quic_backend,
            tls,
            access_policy,
            cache,
            namespaces,
            network,
            request_timeout,
            max_item_bytes,
            observability: observability_service,
            ..
        } = self;
        let observability = observability_service.state();
        let (started_tx, started_rx) =
            channel::bounded::<std::result::Result<(), String>>(network.worker_count);
        let (finished_tx, finished_rx) =
            channel::bounded_sync_async::<NetworkWorkerCompletion>(network.worker_count);
        let mut workers = Vec::with_capacity(network.worker_count);
        let mut launch_error = None;
        let request_budget = RequestBudget::new(network.max_inflight_value_mib * 1024 * 1024);

        for (worker_id, socket) in sockets.into_iter().enumerate() {
            let (stop_tx, stop_rx) = channel::bounded_sync_async(1);
            let started_tx = started_tx.clone();
            let finished_tx = finished_tx.clone();
            let worker_tls = Arc::clone(&tls);
            let worker_access_policy = Arc::clone(&access_policy);
            let worker_cache = Arc::clone(&cache);
            let cpu_id = network.cpu_ids[worker_id];
            let entries = network.io_uring_entries_per_worker;
            let event_interval = network.event_interval;
            let limits = NetworkWorkerLimits {
                worker_id,
                request_timeout,
                max_stream_lanes: network.max_stream_lanes_per_connection,
                request_budget: request_budget.clone(),
                max_item_bytes,
                namespaces: Arc::clone(&namespaces),
                observability: Arc::clone(&observability),
            };
            let reporter = NetworkWorkerReporter::new(worker_id, started_tx, finished_tx);
            let role = QuicNetworkRole {
                worker_id,
                cpu_id,
                socket,
                quic_backend,
                tls: worker_tls,
                access_policy: worker_access_policy,
                cache: worker_cache,
                limits,
                stop: stop_rx,
            };
            match launch_network_role(
                &cache,
                NetworkRolePlacement::new(
                    worker_id,
                    cpu_id,
                    format!("openkache-network-{worker_id}"),
                    entries,
                    event_interval,
                    stop_tx,
                ),
                reporter,
                move |reporter| run_quic_role(role, reporter),
            ) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    launch_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        drop(started_tx);
        drop(finished_tx);

        let mut startup_error = launch_error;
        for _ in 0..network.worker_count {
            match started_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(message)) => {
                    startup_error.get_or_insert(message);
                }
                Err(_) => {
                    startup_error
                        .get_or_insert_with(|| "network worker startup channel closed".into());
                    break;
                }
            }
        }
        if let Some(message) = startup_error {
            observability.set_failed();
            let remaining_completions = workers.len();
            shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
                .await?;
            return Err(ServerError::NetworkWorker(message));
        }

        let metrics_handle = observability_service.start();
        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async_network().fuse();
        pin_mut!(shutdown, worker_finished);
        let (worker_failure, completed_workers, failed_worker) = select! {
            () = shutdown => (None, 0, None),
            result = worker_finished => match result {
                Ok((worker_id, Ok(()))) => (
                    Some(format!("network worker {worker_id} exited unexpectedly")),
                    1,
                    Some(worker_id),
                ),
                Ok((worker_id, Err(message))) => (
                    Some(format!("network worker {worker_id} failed: {message}")),
                    1,
                    Some(worker_id),
                ),
                Err(_) => (Some("network worker completion channel closed".into()), 1, None),
            },
        };
        let remaining_completions = workers.len().saturating_sub(completed_workers);
        if let Some(worker_id) = failed_worker {
            observability.network_worker_failed(worker_id);
        } else if worker_failure.is_some() {
            observability.set_failed();
        } else {
            observability.set_draining();
        }
        metrics_handle.stop();
        shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
            .await?;
        match worker_failure {
            Some(message) => Err(ServerError::NetworkWorker(message)),
            None => Ok(()),
        }
    }
}

struct QuicNetworkRole {
    worker_id: usize,
    cpu_id: usize,
    socket: std::net::UdpSocket,
    quic_backend: QuicBackend,
    tls: Arc<ServerTlsConfig>,
    access_policy: Arc<AccessPolicy>,
    cache: Arc<ThreadedKvkache>,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
}

async fn run_quic_role(
    role: QuicNetworkRole,
    mut reporter: NetworkWorkerReporter,
) -> Option<std::result::Result<(), String>> {
    let QuicNetworkRole {
        worker_id,
        cpu_id,
        socket,
        quic_backend,
        tls,
        access_policy,
        cache,
        limits,
        stop,
    } = role;
    let endpoint =
        match ServerEndpoint::bind(quic_backend, socket, tls, limits.max_stream_lanes).await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                limits.observability.network_worker_failed(worker_id);
                reporter.startup_failed(error.to_string());
                return None;
            }
        };
    if let Some(error) =
        crate::platform::cpu_assignment_error(&format!("network worker {worker_id}"), cpu_id)
    {
        limits.observability.network_worker_failed(worker_id);
        reporter.startup_failed(error);
        return None;
    }
    if !reporter.started() {
        return None;
    }
    limits.observability.network_worker_started(worker_id);
    Some(
        run_selected_endpoint(endpoint, &cache, &access_policy, limits, stop)
            .await
            .map_err(|error| error.to_string()),
    )
}

fn bind_reuse_port_sockets(
    address: SocketAddr,
    worker_count: usize,
) -> std::io::Result<Vec<std::net::UdpSocket>> {
    let mut sockets = Vec::with_capacity(worker_count);
    let mut bind_address = address;
    for worker_id in 0..worker_count {
        let socket = Socket::new(
            Domain::for_address(bind_address),
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&SockAddr::from(bind_address))?;
        let socket = std::net::UdpSocket::from(socket);
        if worker_id == 0 {
            bind_address = socket.local_addr()?;
        }
        sockets.push(socket);
    }
    Ok(sockets)
}

pub(crate) async fn shutdown_network_workers_and_cache(
    workers: Vec<NetworkWorkerHandle>,
    finished: &AsyncReceiver<NetworkWorkerCompletion>,
    remaining_completions: usize,
    cache: Arc<ThreadedKvkache>,
) -> Result<()> {
    for worker in &workers {
        let _ = worker.stop.send(());
    }
    let mut network_failure = None;
    for _ in 0..remaining_completions {
        match finished.recv_async_network().await {
            Ok((_worker_id, Ok(()))) => {}
            Ok((worker_id, Err(message))) => {
                network_failure.get_or_insert_with(|| {
                    format!("network worker {worker_id} failed during shutdown: {message}")
                });
            }
            Err(_) => {
                network_failure
                    .get_or_insert_with(|| "network worker completion channel closed".into());
                break;
            }
        }
    }
    let join_result = join_network_workers(workers);
    let cache_result = shutdown_cache(cache);
    if let Some(message) = network_failure {
        return Err(ServerError::NetworkWorker(message));
    }
    join_result?;
    cache_result
}

fn join_network_workers(workers: Vec<NetworkWorkerHandle>) -> Result<()> {
    let threads = workers
        .into_iter()
        .filter_map(|worker| worker.thread)
        .collect();
    join_network_threads(threads)
}

pub(crate) fn join_network_threads(threads: Vec<std::thread::JoinHandle<()>>) -> Result<()> {
    let mut panicked_worker = None;
    for thread in threads {
        let worker = thread.thread().clone();
        if thread.join().is_err() && panicked_worker.is_none() {
            panicked_worker = Some(worker.name().unwrap_or("network worker").to_owned());
        }
    }
    if let Some(name) = panicked_worker {
        return Err(ServerError::NetworkWorker(format!("{name} panicked")));
    }
    Ok(())
}

fn shutdown_cache(cache: Arc<ThreadedKvkache>) -> Result<()> {
    let mut cache = Arc::try_unwrap(cache)
        .map_err(|_| ServerError::NetworkWorker("network cache handle leaked".into()))?;
    cache.shutdown()?;
    Ok(())
}

#[derive(Clone)]
struct NetworkWorkerLimits {
    worker_id: usize,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
    observability: Arc<ObservabilityState>,
}

async fn run_selected_endpoint(
    endpoint: ServerEndpoint,
    cache: &ThreadedKvkache,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    match endpoint {
        #[cfg(feature = "quic-quinn")]
        ServerEndpoint::Quinn(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
        #[cfg(feature = "quic-noq")]
        ServerEndpoint::Noq(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
        #[cfg(feature = "quic-quiche")]
        ServerEndpoint::Quiche(endpoint) => {
            run_network_worker(endpoint, cache, access_policy, limits, stop).await
        }
    }
}

async fn run_network_worker<E: TransportEndpoint>(
    endpoint: E,
    cache: &ThreadedKvkache,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    let NetworkWorkerLimits {
        worker_id,
        request_timeout,
        max_stream_lanes,
        request_budget,
        max_item_bytes,
        namespaces,
        observability,
    } = limits;
    let network_shard = observability.network_shard(NetworkWorkerId(worker_id));
    let cache = NetworkWorkerCache::new(cache, network_shard.worker_id());
    let mut connections = FuturesUnordered::new();
    loop {
        if connections.is_empty() {
            let incoming = endpoint.wait_incoming().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, &cache, network_shard, access_policy, request_timeout,
                        max_stream_lanes, request_budget.clone(), max_item_bytes,
                        Arc::clone(&namespaces),
                    ));
                }
                _ = stopping => break,
            }
        } else {
            let incoming = endpoint.wait_incoming().fuse();
            let completed = connections.next().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, completed, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, &cache, network_shard, access_policy, request_timeout,
                        max_stream_lanes, request_budget.clone(), max_item_bytes,
                        Arc::clone(&namespaces),
                    ));
                }
                _ = completed => {}
                _ = stopping => break,
            }
        }
    }
    endpoint.close(b"server shutting down");
    while connections.next().await.is_some() {}
    endpoint.shutdown().await
}

/// Completes one QUIC handshake and serves the accepted connection.
async fn serve_incoming<I: TransportIncoming>(
    incoming: I,
    cache: &NetworkWorkerCache<'_>,
    network_shard: NetworkShard<'_>,
    access_policy: &AccessPolicy,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
) {
    match incoming.connect().await {
        Ok(mut connection) => {
            network_shard.connection_started();
            network_shard.handshake_succeeded();
            let peer_certificate = connection.take_peer_certificate();
            let administrator = access_policy.permits_administration(peer_certificate.as_ref());
            serve_connection(
                connection,
                cache,
                network_shard,
                administrator,
                request_timeout,
                max_stream_lanes,
                request_budget,
                max_item_bytes,
                namespaces,
            )
            .await;
            network_shard.connection_finished();
        }
        Err(_) => network_shard.handshake_failed(),
    }
}

/// Multiplexes bounded reusable request lanes for one QUIC connection.
async fn serve_connection<C: TransportConnection>(
    connection: C,
    cache: &NetworkWorkerCache<'_>,
    network_shard: NetworkShard<'_>,
    administrator: bool,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
) {
    let mut streams = FuturesUnordered::new();
    loop {
        if streams.len() >= max_stream_lanes {
            let _ = streams.next().await;
            continue;
        }
        if streams.is_empty() {
            match connection.accept_bi().await {
                Ok((send, receive)) => {
                    network_shard.stream_started();
                    streams.push(serve_stream(
                        send,
                        receive,
                        cache,
                        network_shard,
                        administrator,
                        request_timeout,
                        request_budget.clone(),
                        max_item_bytes,
                        Arc::clone(&namespaces),
                    ));
                }
                Err(_) => break,
            }
        } else {
            let incoming = connection.accept_bi().fuse();
            let completed = streams.next().fuse();
            pin_mut!(incoming, completed);
            select! {
                incoming = incoming => match incoming {
                    Ok((send, receive)) => {
                        network_shard.stream_started();
                        streams.push(serve_stream(
                            send,
                            receive,
                            cache,
                            network_shard,
                            administrator,
                            request_timeout,
                            request_budget.clone(),
                            max_item_bytes,
                            Arc::clone(&namespaces),
                        ));
                    }
                    Err(_) => break,
                },
                _ = completed => {}
            }
        }
    }
    while streams.next().await.is_some() {}
}

/// Reuses one QUIC stream as a sequential request lane until either peer closes it.
async fn serve_stream<S: SendStream, R: ReceiveStream>(
    mut send: S,
    mut receive: R,
    cache: &NetworkWorkerCache<'_>,
    network_shard: NetworkShard<'_>,
    administrator: bool,
    request_timeout: Duration,
    request_budget: RequestBudget,
    max_item_bytes: usize,
    namespaces: Arc<Mutex<NamespaceRegistry>>,
) {
    let _stream_guard = ActiveStream { network_shard };
    loop {
        let mut frame = match receive
            .read_request(
                MAX_REQUEST_FRAME_BYTES,
                max_item_bytes,
                request_timeout,
                &request_budget,
            )
            .await
        {
            Ok(frame) => frame,
            Err(StreamReadError::Timeout) => {
                network_shard.request_read_timeout();
                if !write_response(
                    &mut send,
                    response_bytes(Status::Timeout, b"request read timed out"),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::TooLarge) => {
                network_shard.protocol_error();
                if !write_response(
                    &mut send,
                    response_bytes(Status::TooLarge, b"request exceeds the protocol limit"),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::Protocol(error)) => {
                network_shard.protocol_error();
                if !write_response(
                    &mut send,
                    wire_protocol_error_response(error),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::Transport(_)) => break,
        };
        let request_bytes = std::mem::take(&mut frame.bytes);
        let mut terminal_after_response = frame.has_trailing_bytes;
        let response_result = match Request::decode_owned(request_bytes) {
            Ok(request) => {
                let operation = Operation::from_opcode(request.opcode);
                let request_started = std::time::Instant::now();
                let may_mutate = request_may_mutate(&request);
                let response_permit = if let Some(response_budget_bytes) =
                    response_budget_bytes(request.opcode, max_item_bytes)
                {
                    match request_budget
                        .acquire(response_budget_bytes, request_timeout)
                        .await
                    {
                        Ok(permit) => Some(permit),
                        Err(StreamReadError::Timeout) => {
                            let response = response_bytes(
                                Status::Timeout,
                                b"response memory budget timed out",
                            );
                            network_shard.record_request(
                                operation,
                                Status::Timeout,
                                request_started.elapsed(),
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                network_shard.response_write_failure();
                                break;
                            }
                            continue;
                        }
                        Err(_) => {
                            let response = response_bytes(
                                Status::Overloaded,
                                b"response exceeds the server memory budget",
                            );
                            network_shard.record_request(
                                operation,
                                Status::Overloaded,
                                request_started.elapsed(),
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                network_shard.response_write_failure();
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    None
                };
                match network_runtime::timeout(
                    request_timeout,
                    execute_request(
                        cache,
                        request,
                        administrator,
                        namespaces.as_ref(),
                        network_shard.state(),
                    ),
                )
                .await
                {
                    Ok(Some(response)) => {
                        network_shard.record_request(
                            operation,
                            response.status,
                            request_started.elapsed(),
                        );
                        (response, response_permit)
                    }
                    Ok(None) => {
                        // A mutating storage failure may have crossed its
                        // linearization point. Do not send an error response
                        // that would falsely guarantee that no mutation took
                        // effect.
                        network_shard.abandoned_request();
                        return;
                    }
                    Err(_) if may_mutate => {
                        // The worker request may already have crossed its mutation
                        // linearization point when this wait expires. An error response
                        // would falsely guarantee that it did not take effect.
                        network_shard.abandoned_request();
                        return;
                    }
                    Err(_) => {
                        network_shard.record_request(
                            operation,
                            Status::Timeout,
                            request_started.elapsed(),
                        );
                        (
                            response_bytes(Status::Timeout, b"request execution timed out"),
                            response_permit,
                        )
                    }
                }
            }
            Err(error) => {
                network_shard.protocol_error();
                terminal_after_response = true;
                (protocol_error_response(error), None)
            }
        };
        if !write_response(&mut send, response_result.0, request_timeout).await {
            network_shard.response_write_failure();
            break;
        }
        if terminal_after_response {
            break;
        }
    }
}

struct ActiveStream<'a> {
    network_shard: NetworkShard<'a>,
}

impl Drop for ActiveStream<'_> {
    fn drop(&mut self) {
        self.network_shard.stream_finished();
    }
}

async fn write_response<S: SendStream>(
    send: &mut S,
    response: Response,
    request_timeout: Duration,
) -> bool {
    let Ok(frame) = response.into_encoded() else {
        return false;
    };
    send.write_response(frame, request_timeout).await.is_ok()
}

fn request_may_mutate(request: &Request) -> bool {
    matches!(
        crate::contract::operation_contract(request.opcode).effect,
        crate::contract::OperationEffect::Mutation | crate::contract::OperationEffect::Barrier
    )
}

fn response_budget_bytes(opcode: Opcode, max_item_bytes: usize) -> Option<usize> {
    let contract = crate::contract::operation_contract(opcode);
    let response_field_count = crate::contract::operation_response_field_count(opcode);
    match contract.response_kind {
        "application_value" => Some(contract.response_payload_bound.min(max_item_bytes)),
        "composite" | "value" => {
            if response_field_count > 1 {
                openkache_protocol::optional_values_max_encoded_len(
                    response_field_count,
                    max_item_bytes,
                )
            } else {
                Some(contract.response_payload_bound.min(max_item_bytes))
            }
        }
        "stats_json" | "namespace_descriptor" => {
            Some(contract.response_payload_bound.min(max_item_bytes))
        }
        _ => None,
    }
}

/// Dispatches a decoded protocol request to the SSD-backed worker runtime.
async fn execute_request(
    cache: &NetworkWorkerCache<'_>,
    request: Request,
    administrator: bool,
    namespaces: &Mutex<NamespaceRegistry>,
    observability: &ObservabilityState,
) -> Option<Response> {
    let Request {
        opcode,
        namespace_id,
        item_ids,
        set_options,
        value,
        namespace_name,
        namespace_policy,
        expected_revision,
        create_if_missing,
    } = request;
    if operation_handlers::is_immediate(opcode) {
        return Some(operation_handlers::immediate_response(opcode, value));
    }
    // Namespace open and delete are identity operations. Serialize them with
    // one lifecycle lock so an open cannot observe a descriptor while delete
    // is concurrently removing it (or vice versa).
    let lifecycle_lock = if matches!(
        crate::contract::operation_contract(opcode).request_kind,
        "namespace_open" | "namespace_delete"
    ) {
        match namespaces.lock() {
            Ok(registry) => Some(registry.lifecycle_lock()),
            Err(_) => {
                return Some(response_bytes(
                    Status::InternalError,
                    b"namespace metadata is unavailable",
                ));
            }
        }
    } else {
        None
    };
    let _lifecycle_guard = if let Some(lifecycle_lock) = lifecycle_lock.as_ref() {
        Some(lifecycle_lock.lock().await)
    } else {
        None
    };
    // Serialize lifecycle and data-plane operations for one namespace while
    // allowing unrelated namespaces to proceed concurrently. The registry
    // mutex only protects map metadata; this async lock covers the cache
    // operation and its corresponding item-tracking update.
    let namespace_lock = if matches!(
        crate::contract::operation_contract(opcode).request_kind,
        "item" | "set" | "namespace" | "namespace_update_policy" | "namespace_delete"
    ) {
        let namespace_id = namespace_id.expect("namespace-scoped requests have a validated ID");
        let operation_lock = namespaces
            .lock()
            .ok()
            .and_then(|registry| registry.operation_lock(namespace_id));
        let Some(operation_lock) = operation_lock else {
            return Some(response_bytes(
                Status::NamespaceNotFound,
                b"namespace does not exist",
            ));
        };
        Some(operation_lock)
    } else {
        None
    };
    let _namespace_guard = if let Some(operation_lock) = namespace_lock.as_ref() {
        Some(operation_lock.lock().await)
    } else {
        None
    };
    // A request may have captured an operation lock immediately before a
    // concurrent namespace delete removed its registry entry. Re-check after
    // waiting for that lock so the stale request cannot access the new
    // namespace that might later reuse the same name.
    if let Some(namespace_id) = namespace_id
        && namespace_lock.is_some()
        && !namespace_exists(namespaces, namespace_id)
    {
        return Some(response_bytes(
            Status::NamespaceNotFound,
            b"namespace does not exist",
        ));
    }
    let input = operation_handlers::OperationInputView {
        opcode,
        namespace_id,
        item_ids: &item_ids,
        value,
        namespace_name: namespace_name.as_deref(),
        namespace_policy,
        expected_revision,
        create_if_missing,
        set_options,
    };
    operation_handlers::execute(operation_handlers::OperationContext {
        cache,
        opcode,
        input,
        administrator,
        namespaces,
        observability,
    })
    .await
}

fn namespace_exists(namespaces: &Mutex<NamespaceRegistry>, namespace_id: u64) -> bool {
    namespaces
        .lock()
        .ok()
        .and_then(|registry| registry.descriptor(namespace_id))
        .is_some()
}

fn descriptor_payload(descriptor: NamespaceDescriptor) -> Vec<u8> {
    descriptor
        .encode()
        .expect("validated namespace policy remains encodable")
}

fn resolve_set_options(
    policy: NamespacePolicy,
    options: SetOptions,
) -> std::result::Result<SetOptions, Status> {
    if options.expiration_mode != ExpirationMode::Inherit
        && policy.expiration_override == OverridePolicy::Disallowed
    {
        return Err(Status::PolicyConflict);
    }
    if options.eviction_mode != EvictionMode::Inherit
        && policy.eviction_override == OverridePolicy::Disallowed
    {
        return Err(Status::PolicyConflict);
    }
    let (expiration_mode, ttl_ms) = match options.expiration_mode {
        ExpirationMode::Inherit => match policy.default_expiration {
            ExpirationDefault::NoExpiry => (ExpirationMode::NoExpiry, None),
            ExpirationDefault::FixedTtl { ttl_ms } => (ExpirationMode::ExplicitTtl, Some(ttl_ms)),
        },
        ExpirationMode::NoExpiry => (ExpirationMode::NoExpiry, None),
        ExpirationMode::ExplicitTtl => (ExpirationMode::ExplicitTtl, options.ttl_ms),
    };
    let eviction_mode = match options.eviction_mode {
        EvictionMode::Inherit => match policy.default_eviction {
            EvictionDefault::Evictable => EvictionMode::Evictable,
            EvictionDefault::EvictionProtected => EvictionMode::EvictionProtected,
        },
        selected => selected,
    };
    Ok(SetOptions::with_policies(
        options.condition,
        expiration_mode,
        ttl_ms,
        eviction_mode,
    ))
}

/// Maps cache failures to stable protocol statuses and messages.
fn cache_error_response(error: KvError) -> Response {
    let status = match error {
        KvError::Timeout(_) => Status::Timeout,
        KvError::NoCapacity => Status::NoCapacity,
        KvError::TableFull | KvError::CapacityExhausted { .. } => Status::Overloaded,
        KvError::ItemTooLarge { .. } | KvError::BlobSegmentFull { .. } => Status::TooLarge,
        KvError::InvalidRequest(_) => Status::InvalidRequest,
        KvError::Io(_) | KvError::InvalidConfig(_) | KvError::Worker(_) | KvError::Usage(_) => {
            Status::InternalError
        }
    };
    response_display(status, error)
}

/// Returns an error response only when the storage failure is known to happen
/// before a mutation can be applied.
///
/// A timeout, I/O failure, worker failure, or internal usage failure may be
/// reported after the storage worker crossed its mutation linearization point.
/// Returning a protocol error for those cases would violate the v1 guarantee
/// that every mutating error response means "no mutation".
fn mutation_cache_error_response(opcode: Opcode, error: KvError) -> Option<Response> {
    let safe_before_mutation = matches!(
        &error,
        KvError::InvalidRequest(_)
            | KvError::TableFull
            | KvError::ItemTooLarge { .. }
            | KvError::BlobSegmentFull { .. }
            | KvError::CapacityExhausted { .. }
            | KvError::NoCapacity
    );
    if !safe_before_mutation {
        return None;
    }

    // `NoCapacity` precisely describes SET admission blocked by protected
    // items. DELETE can encounter the same storage condition while driving
    // background work, but the v1 response contract reserves `NoCapacity` for
    // SET; use the applicable generic overload status for other mutations.
    Some(
        if matches!(error, KvError::NoCapacity) && opcode != Opcode::Set {
            response_display(Status::Overloaded, error)
        } else {
            cache_error_response(error)
        },
    )
}

/// Maps framing and validation failures to stable protocol statuses.
fn protocol_error_response(error: crate::protocol::ProtocolError) -> Response {
    let status = match error {
        crate::protocol::ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        crate::protocol::ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

fn wire_protocol_error_response(error: openkache_protocol::ProtocolError) -> Response {
    let status = match error {
        openkache_protocol::ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        openkache_protocol::ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

fn response_display(status: Status, value: impl std::fmt::Display) -> Response {
    let mut payload = String::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES + openkache_protocol::MAX_VARUINT_BYTES + 64,
    );
    write!(payload, "{value}").expect("writing to a String cannot fail");
    response(status, payload.into_bytes())
}

/// Constructs a protocol response whose payload is known to fit protocol limits.
fn response(status: Status, payload: Vec<u8>) -> Response {
    Response::new(status, payload).expect("server responses stay within protocol limits")
}

fn response_bytes(status: Status, payload: &[u8]) -> Response {
    let mut owned = Vec::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES
            + openkache_protocol::MAX_VARUINT_BYTES
            + payload.len(),
    );
    owned.extend_from_slice(payload);
    response(status, owned)
}

/// Errors produced while configuring or running the QUIC server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("cache failed: {0}")]
    Cache(#[from] KvError),
    #[error(
        "production TLS and client authentication are required to bind {0}; configure [tls] or explicitly select insecure development mode"
    )]
    ProductionTlsRequired(SocketAddr),
    #[error("production TLS cannot be combined with insecure development mode")]
    ConflictingSecurityModes,
    #[error("plaintext RESP is restricted to a loopback address, not {0}")]
    PlaintextRespRequiresLoopback(SocketAddr),
    #[error("certificate generation failed: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("TLS identity file {path} is invalid: {message}")]
    TlsIdentity {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("namespace metadata is invalid or unavailable: {0}")]
    NamespaceMetadata(String),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("QUIC transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("network worker failed: {0}")]
    NetworkWorker(String),
}

/// Convenience result type for server lifecycle operations.
pub type Result<T> = std::result::Result<T, ServerError>;
