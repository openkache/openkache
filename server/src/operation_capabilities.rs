//! Public composition primitives for API-owned server dependencies.
//!
//! The generic executor only sees this type-erased catalog. Concrete API
//! modules own the keys and downcast values at their binding boundary.

use std::any::Any;
use std::sync::Arc;

use super::operation_api::{CapabilityKey, capability_id};

/// Type-erased API-owned dependencies supplied by the server composition root.
///
/// Implementations should keep values owned for the server lifetime and use
/// stable keys only as an internal registry detail. API bindings should pair a
/// key with their concrete type through the server's typed capability helpers.
pub trait CapabilityCatalog: Send + Sync {
    /// Returns the dependency registered under one stable composition key.
    fn get(&self, key: &'static str) -> Option<&(dyn Any + Send + Sync)>;

    /// Returns a dependency using its stable numeric identity and diagnostic
    /// name. Implementations that only support the compatibility string method
    /// retain compatibility through this default.
    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get(name).filter(|_| capability_id(name) == id)
    }
}

/// One API-owned capability exposed through [`CapabilityList`].
#[derive(Clone, Copy)]
pub struct CapabilityEntry<'a> {
    /// Stable key chosen by the API module.
    pub key: &'static str,
    id: u64,
    /// Type-erased value retained by the composition root.
    pub value: &'a (dyn Any + Send + Sync),
}

impl<'a> CapabilityEntry<'a> {
    /// Creates one key/value entry for a capability list.
    ///
    /// # Arguments
    ///
    /// * `key` - Stable lookup key owned by the API module.
    /// * `value` - Capability value that remains valid for the catalog lifetime.
    pub const fn new(key: &'static str, value: &'a (dyn Any + Send + Sync)) -> Self {
        Self {
            key,
            id: capability_id(key),
            value,
        }
    }
}

/// Allocation-free capability catalog for a composition root.
///
/// The list is intentionally small and immutable for a server lifetime. A
/// future API can add one entry at composition time without changing the
/// compatibility service trait, dispatcher, or transport adapter.
pub struct CapabilityList<'a> {
    /// Immutable entries searched by their stable key.
    pub entries: &'a [CapabilityEntry<'a>],
}

impl<'a> CapabilityList<'a> {
    /// Creates an immutable capability catalog from borrowed entries.
    ///
    /// # Arguments
    ///
    /// * `entries` - Entries whose values outlive the catalog.
    pub const fn new(entries: &'a [CapabilityEntry<'a>]) -> Self {
        Self { entries }
    }
}

impl CapabilityCatalog for CapabilityList<'_> {
    fn get(&self, key: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get_by_id(capability_id(key), key)
    }

    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.entries
            .iter()
            .find(|entry| entry.id == id && entry.key == name)
            .map(|entry| entry.value)
    }
}

/// Empty capability catalog used when no API-owned dependencies are installed.
#[derive(Clone, Copy, Default)]
pub struct EmptyCapabilityCatalog;

impl CapabilityCatalog for EmptyCapabilityCatalog {
    fn get(&self, _key: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        None
    }
}

/// Owned capability catalog for a server composition root.
///
/// API modules can build this registry without leaking borrowed values or
/// teaching the dispatcher about their concrete dependency types. Entries are
/// immutable from the executor's point of view once the catalog is moved into
/// [`KacheServer::with_capabilities`](super::KacheServer::with_capabilities).
pub struct CapabilityRegistry {
    entries: Vec<(u64, &'static str, Arc<dyn Any + Send + Sync>)>,
    base: Option<Arc<dyn CapabilityCatalog>>,
}

impl CapabilityRegistry {
    /// Creates an empty registry.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            base: None,
        }
    }

    /// Starts one immutable worker registry over caller-supplied base entries.
    ///
    /// API contributions are inserted into this single registry before it is
    /// shared. Lookups therefore perform at most one sorted search and one
    /// fallback instead of walking an overlay chain.
    pub(crate) fn overlay(base: Arc<dyn CapabilityCatalog>) -> Self {
        Self {
            entries: Vec::new(),
            base: Some(base),
        }
    }

    /// Registers or replaces one API-owned capability.
    ///
    /// The typed key keeps the downcast type next to the registration site;
    /// the generic dispatcher only stores and looks up the erased value.
    pub fn insert<T>(&mut self, key: CapabilityKey<T>, value: T)
    where
        T: Any + Send + Sync,
    {
        if self
            .entries
            .iter()
            .any(|(id, name, _)| *id == key.id() && *name != key.name())
        {
            panic!("capability key hash collision");
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(_, name, _)| *name == key.name())
        {
            entry.0 = key.id();
            entry.2 = Arc::new(value);
            return;
        }
        self.entries.push((key.id(), key.name(), Arc::new(value)));
        self.entries.sort_unstable_by_key(|(id, _, _)| *id);
    }

    /// Adds a capability and returns the registry for fluent composition.
    pub fn with<T>(mut self, key: CapabilityKey<T>, value: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.insert(key, value);
        self
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityCatalog for CapabilityRegistry {
    fn get(&self, key: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get_by_id(capability_id(key), key)
    }

    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.entries
            .binary_search_by(|(entry_id, entry_name, _)| (*entry_id, *entry_name).cmp(&(id, name)))
            .ok()
            .map(|index| self.entries[index].2.as_ref())
            .or_else(|| self.base.as_ref()?.get_by_id(id, name))
    }
}
