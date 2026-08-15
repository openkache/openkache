//! Type-erased composition primitives for server and API capabilities.
//!
//! Startup composition exposes caller capabilities and borrowed server ports
//! to API module initializers without retaining a request-path service locator.

use std::any::Any;
use std::sync::Arc;

use super::operation_api::{CapabilityKey, capability_id};

/// Type-erased dependencies supplied by the server composition root.
///
/// Catalog values must outlive the catalog that borrows or owns them. API
/// bindings should pair stable keys with their concrete types through the
/// server's typed capability helpers.
pub trait CapabilityCatalog: Send + Sync {
    /// Returns the dependency registered under one stable composition key.
    fn get(&self, key: &'static str) -> Option<&(dyn Any + Send + Sync)>;

    /// Reports whether any dependency uses one stable numeric identity.
    ///
    /// Composition uses this independently of a diagnostic name so duplicate
    /// keys and hash collisions cannot be resolved by overlay precedence.
    fn contains_id(&self, id: u64) -> bool;

    /// Returns a dependency using its stable numeric identity and diagnostic
    /// name. Implementations that only support the compatibility string method
    /// retain compatibility through this default.
    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get(name).filter(|_| capability_id(name) == id)
    }
}

/// One borrowed capability exposed through [`CapabilityList`].
#[derive(Clone, Copy)]
pub struct CapabilityEntry<'a> {
    /// Stable key chosen by the API module.
    pub key: &'static str,
    id: u64,
    /// Type-erased value retained by the composition root.
    pub value: &'a (dyn Any + Send + Sync),
}

impl<'a> CapabilityEntry<'a> {
    /// Creates an entry whose value type is checked against its capability key.
    ///
    /// # Arguments
    ///
    /// * `key` - Typed identity owned by the capability consumer.
    /// * `value` - Capability value that remains valid for the catalog lifetime.
    ///
    /// # Returns
    ///
    /// A borrowed, type-erased entry that retains the key's stable identity.
    pub fn new<T>(key: CapabilityKey<T>, value: &'a T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self::erased(key.name(), value)
    }

    pub(crate) const fn erased(
        key: &'static str,
        value: &'a (dyn Any + Send + Sync),
    ) -> Self {
        Self {
            key,
            id: capability_id(key),
            value,
        }
    }

}

/// Allocation-free capability catalog for a composition root.
///
/// The list is intentionally small and immutable for its borrowed lifetime. It
/// can be stack-scoped to one composition call; server-lifetime values belong
/// in [`CapabilityRegistry`]. A future API can add one entry without changing
/// the dispatcher or transport adapter.
pub struct CapabilityList<'a> {
    /// Immutable entries searched by their stable key.
    pub entries: &'a [CapabilityEntry<'a>],
    base: Option<&'a dyn CapabilityCatalog>,
}

impl<'a> CapabilityList<'a> {
    /// Creates an immutable capability catalog from borrowed entries.
    ///
    /// # Arguments
    ///
    /// * `entries` - Entries whose values outlive the catalog.
    pub const fn new(entries: &'a [CapabilityEntry<'a>]) -> Self {
        Self {
            entries,
            base: None,
        }
    }

    /// Creates an immutable overlay over one borrowed base catalog.
    ///
    /// The base and entries remain borrowed, so composition does not allocate
    /// or clone shared ownership. An identity present in both catalogs is
    /// ambiguous and cannot be resolved by overlay precedence.
    ///
    /// # Arguments
    ///
    /// * `base` - Existing catalog borrowed for the overlay lifetime.
    /// * `entries` - Additional borrowed entries.
    ///
    /// # Returns
    ///
    /// An allocation-free catalog that searches both sources.
    pub const fn overlay(
        base: &'a dyn CapabilityCatalog,
        entries: &'a [CapabilityEntry<'a>],
    ) -> Self {
        Self {
            entries,
            base: Some(base),
        }
    }
}

impl CapabilityCatalog for CapabilityList<'_> {
    fn get(&self, key: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get_by_id(capability_id(key), key)
    }

    fn contains_id(&self, id: u64) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
            || self.base.is_some_and(|base| base.contains_id(id))
    }

    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        let mut found = None;
        for entry in self.entries.iter().filter(|entry| entry.id == id) {
            if entry.key != name || found.is_some() {
                return None;
            }
            found = Some(entry.value);
        }
        let Some(base) = self.base else {
            return found;
        };
        if found.is_some() && base.contains_id(id) {
            return None;
        }
        found.or_else(|| base.get_by_id(id, name))
    }
}

/// Empty capability catalog used when no API-owned dependencies are installed.
#[derive(Clone, Copy, Default)]
pub struct EmptyCapabilityCatalog;

impl CapabilityCatalog for EmptyCapabilityCatalog {
    fn get(&self, _key: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        None
    }

    fn contains_id(&self, _id: u64) -> bool {
        false
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
}

impl CapabilityRegistry {
    /// Creates an empty registry.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Registers one API-owned capability.
    ///
    /// The typed key keeps the downcast type next to the registration site;
    /// API module initializers consume the erased value during worker startup.
    ///
    /// # Panics
    ///
    /// Panics if the key duplicates or collides with any existing entry.
    pub fn insert<T>(&mut self, key: CapabilityKey<T>, value: T)
    where
        T: Any + Send + Sync,
    {
        if let Some((_, name, _)) = self.entries.iter().find(|(id, _, _)| *id == key.id()) {
            if *name == key.name() {
                panic!("duplicate capability key");
            }
            panic!("capability key hash collision");
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

    fn contains_id(&self, id: u64) -> bool {
        self.entries.iter().any(|(entry_id, _, _)| *entry_id == id)
    }

    fn get_by_id(&self, id: u64, name: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.entries
            .binary_search_by_key(&id, |(entry_id, _, _)| *entry_id)
            .ok()
            .filter(|index| self.entries[*index].1 == name)
            .map(|index| self.entries[index].2.as_ref())
    }
}
