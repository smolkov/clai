//! All sessions of the server, running and saved, by id and by name. Names are
//! unique across both.

use std::collections::BTreeMap;

use crate::session::SessionHandle;
use crate::session::store::{Meta, Store};

/// A session in the registry: running in the server, or saved on disk.
pub(super) struct Entry {
    pub name: String,
    /// `None` if it could not be saved; it then lives in memory only.
    pub store: Option<Store>,
    pub slot: Slot,
}

pub(super) enum Slot {
    Running(SessionHandle),
    Saved(Box<Meta>),
}

impl Entry {
    pub fn handle(&self) -> Option<&SessionHandle> {
        match &self.slot {
            Slot::Running(handle) => Some(handle),
            Slot::Saved(_) => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Registry {
    entries: BTreeMap<u64, Entry>,
}

impl Registry {
    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.get(&id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.get_mut(&id)
    }

    pub fn by_name(&self, name: &str) -> Option<(u64, &Entry)> {
        self.entries
            .iter()
            .find(|(_, e)| e.name == name)
            .map(|(id, e)| (*id, e))
    }

    pub fn by_name_mut(&mut self, name: &str) -> Option<(u64, &mut Entry)> {
        self.entries
            .iter_mut()
            .find(|(_, e)| e.name == name)
            .map(|(id, e)| (*id, e))
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.by_name(name).is_some()
    }

    /// `name`, else `name-2`, `name-3`, …
    pub fn unique_name(&self, name: &str) -> String {
        if !self.is_taken(name) {
            return name.to_owned();
        }
        (2u64..)
            .map(|i| format!("{name}-{i}"))
            .find(|n| !self.is_taken(n))
            .unwrap_or_else(|| name.to_owned())
    }

    pub fn insert(&mut self, id: u64, entry: Entry) {
        self.entries.insert(id, entry);
    }

    pub fn remove(&mut self, id: u64) -> Option<Entry> {
        self.entries.remove(&id)
    }

    /// The handles of the running sessions.
    pub fn handles(&self) -> Vec<SessionHandle> {
        self.entries
            .values()
            .filter_map(|e| e.handle().cloned())
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, &Entry)> {
        self.entries.iter().map(|(id, e)| (*id, e))
    }

    /// All sessions, running and saved.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn saved(name: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            store: None,
            slot: Slot::Saved(Box::new(Meta::new(
                name.to_owned(),
                PathBuf::from("/ws"),
                "~/ws".to_owned(),
                "a/b".to_owned(),
                0,
            ))),
        }
    }

    fn registry(names: &[&str]) -> Registry {
        let mut registry = Registry::default();
        for (id, name) in (1u64..).zip(names) {
            registry.insert(id, saved(name));
        }
        registry
    }

    #[test]
    fn lookups() {
        let mut registry = registry(&["a", "b"]);
        assert_eq!(registry.by_name("b").map(|(id, _)| id), Some(2));
        assert!(registry.by_name("c").is_none());
        let (id, entry) = registry.by_name_mut("a").unwrap();
        assert_eq!(id, 1);
        entry.name = "c".to_owned();
        assert_eq!(registry.get(1).unwrap().name, "c");
        assert!(registry.by_name("a").is_none());
        assert_eq!(registry.by_name("c").map(|(id, _)| id), Some(1));
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.remove(1).unwrap().name, "c");
        assert!(registry.remove(1).is_none());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn taken_names() {
        let registry = registry(&["a"]);
        assert!(registry.is_taken("a"));
        assert!(!registry.is_taken("b"));
    }

    #[test]
    fn unique_names() {
        let registry = registry(&["a", "a-2", "b"]);
        assert_eq!(registry.unique_name("new"), "new");
        assert_eq!(registry.unique_name("a"), "a-3");
        assert_eq!(registry.unique_name("b"), "b-2");
    }

    #[test]
    fn saved_sessions_have_no_handle() {
        let registry = registry(&["a"]);
        assert!(registry.handles().is_empty());
        assert!(registry.iter().all(|(_, e)| e.handle().is_none()));
    }
}
