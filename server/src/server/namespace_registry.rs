use super::namespace_journal::{JournalEvent, NamespaceJournal};
use super::namespace_metadata;
use super::storage_port::StorageRoute;
use super::{ItemId, NamespaceDescriptor, NamespacePolicy};
use futures_util::lock::Mutex as AsyncMutex;
use openkache_protocol::OwnedRange;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const NAMESPACE_METADATA_MAX_ENTRIES: u64 = 1_000_000;
const NAMESPACE_METADATA_MAX_ITEMS_PER_ENTRY: u64 = 1_000_000_000;
const NAMESPACE_METADATA_MAX_DIRTY_WORKERS: u64 = 1_000_000;
static NEXT_NAMESPACE_METADATA_TEMP: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

fn journal_item_id(item_id: ItemId) -> ([u8; openkache_protocol::ITEM_ID_BYTES], u8) {
    let mut bytes = [0; openkache_protocol::ITEM_ID_BYTES];
    bytes[..item_id.len()].copy_from_slice(item_id.as_bytes());
    (bytes, item_id.len() as u8)
}

#[derive(Clone, Debug)]
struct NamespaceName(Arc<OwnedRange>);

impl NamespaceName {
    fn new(name: OwnedRange) -> Self {
        Self(Arc::new(name))
    }
}

impl AsRef<[u8]> for NamespaceName {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl Borrow<[u8]> for NamespaceName {
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl PartialEq for NamespaceName {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for NamespaceName {}

impl Hash for NamespaceName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

#[derive(Clone)]
struct NamespaceEntry {
    descriptor: NamespaceDescriptor,
    name: NamespaceName,
    items: HashSet<ItemId>,
    dirty_workers: HashSet<StorageRoute>,
    operation_lock: Arc<AsyncMutex<()>>,
    active: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct NamespaceOperationLock {
    lock: Arc<AsyncMutex<()>>,
    active: Arc<AtomicBool>,
}

impl NamespaceOperationLock {
    pub(crate) fn into_parts(self) -> (Arc<AsyncMutex<()>>, Arc<AtomicBool>) {
        (self.lock, self.active)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SetReservation {
    pub(crate) inserted_item: bool,
    pub(crate) inserted_worker: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NamespaceError {
    InvalidRequest,
    NotFound,
    Conflict,
    PolicyConflict,
    NotEmpty,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NamespaceOpenResult {
    Existing,
    Created,
}

// Namespace names and IDs are server-wide. Authentication and any future
// owner/ACL mapping are separate authorization concerns and do not participate
// in registry lookup.
pub(crate) struct NamespaceRegistry {
    /// The next never-before-issued ID. `None` means the u64 ID space is exhausted.
    next_id: Option<u64>,
    by_id: HashMap<u64, NamespaceEntry>,
    by_name: HashMap<NamespaceName, u64>,
    metadata_path: Option<std::path::PathBuf>,
    journal: Option<NamespaceJournal>,
    persistent: bool,
    lifecycle_lock: Arc<AsyncMutex<()>>,
}

impl NamespaceRegistry {
    pub(crate) fn load(directory: &Path, existing_storage: bool) -> std::io::Result<Self> {
        let metadata_path = match super::super::storage_backend::namespace_persistence(directory) {
            super::super::storage_backend::NamespacePersistence::Durable(path) => path,
            super::super::storage_backend::NamespacePersistence::Ephemeral => {
                return Ok(Self::ephemeral(directory));
            }
        };
        let mut registry = Self {
            next_id: Some(1),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            metadata_path: Some(metadata_path),
            journal: None,
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
                let journal_path = registry
                    .metadata_path
                    .as_ref()
                    .expect("durable namespace registry has metadata path")
                    .with_extension("journal");
                registry.journal = Some(NamespaceJournal::start(&journal_path)?);
                return Ok(registry);
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        registry.decode_metadata(&bytes)?;
        let journal_path = registry
            .metadata_path
            .as_ref()
            .expect("durable namespace registry has metadata path")
            .with_extension("journal");
        let journal_events = NamespaceJournal::load_events(&journal_path)?;
        registry.replay_journal(journal_events)?;
        registry.journal = Some(NamespaceJournal::start(&journal_path)?);
        Ok(registry)
    }

    pub(crate) fn ephemeral(_directory: &Path) -> Self {
        Self {
            next_id: Some(1),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            metadata_path: None,
            journal: None,
            persistent: false,
            lifecycle_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub(crate) fn lifecycle_lock(&self) -> Arc<AsyncMutex<()>> {
        Arc::clone(&self.lifecycle_lock)
    }

    pub(crate) fn open(
        &mut self,
        name: impl Into<OwnedRange>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> std::result::Result<(NamespaceOpenResult, NamespaceDescriptor), NamespaceError> {
        let name = name.into();
        if name.len() > namespace_metadata::NAME_MAX_BYTES {
            return Err(NamespaceError::InvalidRequest);
        }
        if let Some(namespace_id) = self.by_name.get(name.as_slice()).copied() {
            let Some(entry) = self.by_id.get(&namespace_id) else {
                return Err(NamespaceError::Internal);
            };
            return Ok((NamespaceOpenResult::Existing, entry.descriptor));
        }
        if !create_if_missing {
            return Err(NamespaceError::NotFound);
        }
        let policy = policy.ok_or(NamespaceError::InvalidRequest)?;
        let name = NamespaceName::new(name);
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
                name,
                items: HashSet::new(),
                dirty_workers: HashSet::new(),
                operation_lock: Arc::new(AsyncMutex::new(())),
                active: Arc::new(AtomicBool::new(true)),
            },
        );
        if self.persist().is_err() {
            let entry = self
                .by_id
                .remove(&namespace_id)
                .expect("new namespace remains registered until persistence completes");
            self.by_name.remove(entry.name.as_ref());
            self.next_id = previous_next_id;
            return Err(NamespaceError::Internal);
        }
        Ok((NamespaceOpenResult::Created, descriptor))
    }

    fn allocate_id(&mut self) -> std::result::Result<u64, NamespaceError> {
        let namespace_id = self.next_id.ok_or(NamespaceError::Internal)?;
        self.next_id = if namespace_id == u64::MAX {
            None
        } else {
            Some(namespace_id + 1)
        };
        Ok(namespace_id)
    }

    pub(crate) fn operation_lock(&self, namespace_id: u64) -> Option<NamespaceOperationLock> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| NamespaceOperationLock {
                lock: Arc::clone(&entry.operation_lock),
                active: Arc::clone(&entry.active),
            })
    }

    pub(crate) fn descriptor(&self, namespace_id: u64) -> Option<NamespaceDescriptor> {
        self.by_id.get(&namespace_id).map(|entry| entry.descriptor)
    }

    pub(crate) fn policy(&self, namespace_id: u64) -> Option<NamespacePolicy> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| entry.descriptor.policy)
    }

    pub(crate) fn update(
        &mut self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> std::result::Result<NamespaceDescriptor, NamespaceError> {
        let (previous, descriptor) = {
            let entry = self
                .by_id
                .get_mut(&namespace_id)
                .ok_or(NamespaceError::NotFound)?;
            if entry.descriptor.revision != expected_revision {
                return Err(NamespaceError::Conflict);
            }
            let previous = entry.descriptor;
            entry.descriptor.revision = entry
                .descriptor
                .revision
                .checked_add(1)
                .ok_or(NamespaceError::Internal)?;
            entry.descriptor.policy = policy;
            (previous, entry.descriptor)
        };
        if self.persist().is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.descriptor = previous;
            }
            return Err(NamespaceError::Internal);
        }
        Ok(descriptor)
    }

    pub(crate) fn delete(
        &mut self,
        namespace_id: u64,
        expected_revision: u64,
    ) -> std::result::Result<(), NamespaceError> {
        let entry = self
            .by_id
            .get(&namespace_id)
            .ok_or(NamespaceError::NotFound)?;
        if entry.descriptor.revision != expected_revision {
            return Err(NamespaceError::Conflict);
        }
        if !entry.items.is_empty() {
            return Err(NamespaceError::NotEmpty);
        }
        let Some(entry) = self.by_id.remove(&namespace_id) else {
            return Err(NamespaceError::NotFound);
        };
        self.by_name.remove(entry.name.as_ref());
        if self.persist().is_err() {
            self.by_name.insert(entry.name.clone(), namespace_id);
            self.by_id.insert(namespace_id, entry);
            return Err(NamespaceError::Internal);
        }
        entry.active.store(false, Ordering::Release);
        Ok(())
    }

    /// Records an item before a SET is dispatched to storage.
    ///
    /// Persisting the conservative "possibly present" state first means a
    /// crash between storage mutation and metadata update cannot make
    /// `IfEmpty` deletion incorrectly report an empty namespace.
    pub(crate) fn reserve_item(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        route: StorageRoute,
    ) -> std::result::Result<SetReservation, NamespaceError> {
        let reservation = self
            .by_id
            .get_mut(&namespace_id)
            .ok_or(NamespaceError::NotFound)
            .map(|entry| SetReservation {
                inserted_item: entry.items.insert(item_id),
                inserted_worker: entry.dirty_workers.insert(route),
            })?;
        if !reservation.inserted_item && !reservation.inserted_worker {
            return Ok(reservation);
        }
        let (item_id_bytes, item_id_len) = journal_item_id(item_id);
        let event = JournalEvent::ReserveItem {
            namespace_id,
            item_id: item_id_bytes,
            item_id_len,
            route: route.persisted(),
            inserted_item: reservation.inserted_item,
            inserted_worker: reservation.inserted_worker,
        };
        if self.append_event(event).is_err() {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                if reservation.inserted_item {
                    entry.items.remove(&item_id);
                }
                if reservation.inserted_worker {
                    entry.dirty_workers.remove(&route);
                }
            }
            return Err(NamespaceError::Internal);
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
    pub(crate) fn rollback_set_reservation(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        route: StorageRoute,
        reservation: SetReservation,
    ) -> std::result::Result<(), NamespaceError> {
        if !reservation.inserted_item && !reservation.inserted_worker {
            return Ok(());
        }
        {
            let Some(entry) = self.by_id.get_mut(&namespace_id) else {
                return Err(NamespaceError::NotFound);
            };
            if reservation.inserted_item {
                entry.items.remove(&item_id);
            }
            if reservation.inserted_worker {
                entry.dirty_workers.remove(&route);
            }
        }
        let (item_id_bytes, item_id_len) = journal_item_id(item_id);
        if self
            .append_event(JournalEvent::RollbackItem {
                namespace_id,
                item_id: item_id_bytes,
                item_id_len,
                route: route.persisted(),
                remove_item: reservation.inserted_item,
                remove_worker: reservation.inserted_worker,
            })
            .is_err()
        {
            if reservation.inserted_item {
                if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                    entry.items.insert(item_id);
                }
            }
            if reservation.inserted_worker {
                if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                    entry.dirty_workers.insert(route);
                }
            }
            return Err(NamespaceError::Internal);
        }
        Ok(())
    }

    /// Records a worker before a DELETE is dispatched to storage.
    ///
    /// The marker is intentionally conservative: a DELETE that finds no item
    /// still leaves the worker dirty until the next successful `SYNC`.
    pub(crate) fn reserve_worker(
        &mut self,
        namespace_id: u64,
        route: StorageRoute,
    ) -> std::result::Result<(), NamespaceError> {
        let inserted = self
            .by_id
            .get_mut(&namespace_id)
            .ok_or(NamespaceError::NotFound)?
            .dirty_workers
            .insert(route);
        if !inserted {
            return Ok(());
        }
        if self
            .append_event(JournalEvent::ReserveWorker {
                namespace_id,
                route: route.persisted(),
            })
            .is_err()
        {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.dirty_workers.remove(&route);
            }
            return Err(NamespaceError::Internal);
        }
        Ok(())
    }

    pub(crate) fn mark_delete(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
        deleted: bool,
    ) -> std::result::Result<(), NamespaceError> {
        let removed = deleted
            && self
                .by_id
                .get_mut(&namespace_id)
                .is_some_and(|entry| entry.items.remove(&item_id));
        if removed
            && {
                let (item_id_bytes, item_id_len) = journal_item_id(item_id);
                self.append_event(JournalEvent::MarkDelete {
                    namespace_id,
                    item_id: item_id_bytes,
                    item_id_len,
                })
                .is_err()
            }
        {
            // Keeping the item in memory is conservative when persistence
            // fails; the caller closes the lane because the mutation outcome
            // can no longer be represented reliably.
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.items.insert(item_id);
            }
            return Err(NamespaceError::Internal);
        }
        Ok(())
    }

    pub(crate) fn tracked_items(&self, namespace_id: u64) -> Option<Vec<ItemId>> {
        self.by_id
            .get(&namespace_id)
            .map(|entry| entry.items.iter().copied().collect())
    }

    pub(crate) fn dirty_workers(&self, namespace_id: u64) -> Option<Vec<StorageRoute>> {
        self.by_id.get(&namespace_id).map(|entry| {
            let mut workers = entry.dirty_workers.iter().copied().collect::<Vec<_>>();
            workers.sort_unstable();
            workers
        })
    }

    pub(crate) fn mark_workers_clean(
        &mut self,
        namespace_id: u64,
    ) -> std::result::Result<(), NamespaceError> {
        let previous = {
            let entry = self
                .by_id
                .get_mut(&namespace_id)
                .ok_or(NamespaceError::NotFound)?;
            let previous = entry.dirty_workers.clone();
            entry.dirty_workers.clear();
            previous
        };
        if previous.is_empty() {
            return Ok(());
        }
        if self
            .append_event(JournalEvent::MarkWorkersClean { namespace_id })
            .is_err()
        {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.dirty_workers = previous;
            }
            return Err(NamespaceError::Internal);
        }
        Ok(())
    }

    pub(crate) fn prune_item(
        &mut self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> std::result::Result<(), NamespaceError> {
        let Some(entry) = self.by_id.get_mut(&namespace_id) else {
            return Err(NamespaceError::NotFound);
        };
        if !entry.items.remove(&item_id) {
            return Ok(());
        }
        let (item_id_bytes, item_id_len) = journal_item_id(item_id);
        if self
            .append_event(JournalEvent::PruneItem {
                namespace_id,
                item_id: item_id_bytes,
                item_id_len,
            })
            .is_err()
        {
            if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                entry.items.insert(item_id);
            }
            return Err(NamespaceError::Internal);
        }
        Ok(())
    }

    fn append_event(&self, event: JournalEvent) -> std::io::Result<()> {
        if !self.persistent {
            return Ok(());
        }
        // The legacy journal record is fixed-width and intentionally remains
        // a compatibility reader for existing 32-byte memberships. Compact
        // the exact variable-length snapshot for short IDs instead of
        // padding them into a different identity.
        let short_item = match event {
            JournalEvent::ReserveItem { item_id_len, .. }
            | JournalEvent::RollbackItem { item_id_len, .. }
            | JournalEvent::MarkDelete { item_id_len, .. }
            | JournalEvent::PruneItem { item_id_len, .. } => {
                // `item_id_len` carries the original wire length; the fixed
                // array is only a journal encoding buffer.
                item_id_len < openkache_protocol::ITEM_ID_BYTES as u8
            }
            JournalEvent::ReserveWorker { .. } | JournalEvent::MarkWorkersClean { .. } => false,
        };
        if short_item {
            return self.persist();
        }
        self.journal
            .as_ref()
            .expect("persistent namespace registry has journal")
            .append(event)
    }

    fn replay_journal(&mut self, events: Vec<JournalEvent>) -> std::io::Result<()> {
        for event in events {
            match event {
                JournalEvent::ReserveItem {
                    namespace_id,
                    item_id,
                    item_id_len,
                    route,
                    inserted_item,
                    inserted_worker,
                } => {
                    let Some(entry) = self.by_id.get_mut(&namespace_id) else {
                        continue;
                    };
                    if inserted_item {
                        let item_id = ItemId::from_slice(&item_id[..usize::from(item_id_len)])
                            .expect("journal item ID length was validated during decode");
                        entry.items.insert(item_id);
                    }
                    if inserted_worker {
                        entry
                            .dirty_workers
                            .insert(StorageRoute::from_persisted(route));
                    }
                }
                JournalEvent::RollbackItem {
                    namespace_id,
                    item_id,
                    item_id_len,
                    route,
                    remove_item,
                    remove_worker,
                } => {
                    let Some(entry) = self.by_id.get_mut(&namespace_id) else {
                        continue;
                    };
                    if remove_item {
                        let item_id = ItemId::from_slice(&item_id[..usize::from(item_id_len)])
                            .expect("journal item ID length was validated during decode");
                        entry.items.remove(&item_id);
                    }
                    if remove_worker {
                        entry
                            .dirty_workers
                            .remove(&StorageRoute::from_persisted(route));
                    }
                }
                JournalEvent::ReserveWorker {
                    namespace_id,
                    route,
                } => {
                    if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                        entry
                            .dirty_workers
                            .insert(StorageRoute::from_persisted(route));
                    }
                }
                JournalEvent::MarkWorkersClean { namespace_id } => {
                    if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                        entry.dirty_workers.clear();
                    }
                }
                JournalEvent::MarkDelete {
                    namespace_id,
                    item_id,
                    item_id_len,
                }
                | JournalEvent::PruneItem {
                    namespace_id,
                    item_id,
                    item_id_len,
                } => {
                    if let Some(entry) = self.by_id.get_mut(&namespace_id) {
                        let item_id = ItemId::from_slice(&item_id[..usize::from(item_id_len)])
                            .expect("journal item ID length was validated during decode");
                        entry.items.remove(&item_id);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn persist(&self) -> std::io::Result<()> {
        if !self.persistent {
            return Ok(());
        }
        let mut entries = self.by_id.values().collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.descriptor.namespace_id);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(namespace_metadata::MAGIC);
        bytes.extend_from_slice(&namespace_metadata::VERSION.to_be_bytes());
        bytes.extend_from_slice(&self.next_id.unwrap_or(0).to_be_bytes());
        bytes.extend_from_slice(&(entries.len() as u64).to_be_bytes());
        for entry in entries {
            let name_len = u16::try_from(entry.name.as_ref().len()).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidData, "namespace name is too long")
            })?;
            bytes.extend_from_slice(&entry.descriptor.namespace_id.to_be_bytes());
            bytes.extend_from_slice(&entry.descriptor.revision.to_be_bytes());
            bytes.extend_from_slice(&name_len.to_be_bytes());
            bytes.extend_from_slice(entry.name.as_ref());
            let policy = namespace_metadata::encode_policy(entry.descriptor.policy)?;
            let policy_len = u8::try_from(policy.len())
                .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "policy is too long"))?;
            bytes.push(policy_len);
            bytes.extend_from_slice(&policy);
            bytes.extend_from_slice(&(entry.items.len() as u64).to_be_bytes());
            let mut items = entry.items.iter().copied().collect::<Vec<_>>();
            items.sort_unstable();
            for item_id in items {
                let item_id_len = u8::try_from(item_id.len()).map_err(|_| {
                    std::io::Error::new(ErrorKind::InvalidData, "item ID is too long")
                })?;
                bytes.push(item_id_len);
                bytes.extend_from_slice(item_id.as_bytes());
            }
            bytes.extend_from_slice(&(entry.dirty_workers.len() as u64).to_be_bytes());
            let mut dirty_workers = entry.dirty_workers.iter().copied().collect::<Vec<_>>();
            dirty_workers.sort_unstable();
            for route in dirty_workers {
                bytes.extend_from_slice(&route.persisted().to_be_bytes());
            }
        }

        if let Some(journal) = &self.journal {
            return journal.compact(bytes);
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
        if cursor.take(namespace_metadata::MAGIC.len())? != namespace_metadata::MAGIC {
            return Err(cursor.invalid("namespace metadata magic is invalid"));
        }
        let metadata_version = cursor.u32()?;
        if metadata_version != namespace_metadata::VERSION
            && metadata_version != namespace_metadata::LEGACY_V3_VERSION
            && metadata_version != namespace_metadata::LEGACY_V2_VERSION
            && metadata_version != namespace_metadata::LEGACY_V1_VERSION
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
            if name_len > namespace_metadata::NAME_MAX_BYTES {
                return Err(cursor.invalid("namespace metadata name is too long"));
            }
            let name = cursor.take(name_len)?.to_vec();
            std::str::from_utf8(&name)
                .map_err(|_| cursor.invalid("namespace metadata name is not UTF-8"))?;
            let policy_len = usize::from(cursor.u8()?);
            let policy_bytes = cursor.take(policy_len)?;
            let (policy, used) = namespace_metadata::decode_policy(metadata_version, policy_bytes)
                .map_err(|error| cursor.invalid(error.to_string()))?
                .ok_or_else(|| cursor.invalid("namespace metadata policy is truncated"))?;
            if used != policy_len {
                return Err(cursor.invalid("namespace metadata policy has trailing bytes"));
            }
            let item_count = cursor.u64()?;
            let minimum_item_bytes = if metadata_version == namespace_metadata::VERSION {
                1
            } else {
                openkache_protocol::ITEM_ID_BYTES
            };
            if item_count > NAMESPACE_METADATA_MAX_ITEMS_PER_ENTRY
                || item_count > (cursor.remaining() / minimum_item_bytes) as u64
            {
                return Err(cursor.invalid("namespace metadata item list is invalid"));
            }
            let mut items = HashSet::with_capacity(item_count as usize);
            for _ in 0..item_count {
                let item_id = if metadata_version == namespace_metadata::VERSION {
                    let item_len = usize::from(cursor.u8()?);
                    let item_bytes = cursor.take(item_len)?;
                    ItemId::from_slice(item_bytes)
                        .map_err(|_| cursor.invalid("namespace metadata item ID is too long"))?
                } else {
                    let item_bytes = cursor.take(openkache_protocol::ITEM_ID_BYTES)?;
                    ItemId::from_slice(item_bytes).expect("legacy metadata item ID width is fixed")
                };
                items.insert(item_id);
            }
            let mut dirty_workers = HashSet::new();
            if metadata_version >= namespace_metadata::LEGACY_V2_VERSION {
                let dirty_worker_count = cursor.u64()?;
                if dirty_worker_count > NAMESPACE_METADATA_MAX_DIRTY_WORKERS {
                    return Err(cursor.invalid("namespace metadata dirty-worker list is invalid"));
                }
                for _ in 0..dirty_worker_count {
                    let route = StorageRoute::from_persisted(cursor.u64()?);
                    if !dirty_workers.insert(route) {
                        return Err(
                            cursor.invalid("namespace metadata contains duplicate dirty workers")
                        );
                    }
                }
            }
            if self.by_id.contains_key(&namespace_id) || self.by_name.contains_key(name.as_slice())
            {
                return Err(cursor.invalid("namespace metadata contains duplicate identity"));
            }
            let name = NamespaceName::new(OwnedRange::whole(name));
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
                    active: Arc::new(AtomicBool::new(true)),
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
