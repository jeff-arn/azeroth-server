//! `Lock.dbc` rows (`LockEntry`, fork
//! `DBCStructure.h:456-463`, fmt `DBCfmt.h:61`
//! `"niiiiiiiiiiiiiiiiiiiiiiiixxxxxxxx"`, store
//! `sLockStore` `DBCStores.cpp:69,229`): id plus
//! eight `(Type, Index, Skill)` triples; the eight
//! trailing `Action` words are padding the producer
//! skips, so produced rows carry 25 fields (0-24).
//!
//! Built by the mangosd binary from generic DBC
//! rows, like [`crate::chat_channels::ChatChannelDbc`].
//! DBC-only: no DB or save-shape touch.

use std::collections::HashMap;

/// `MAX_LOCK_CASE` (`DBCStructure.h:450`).
pub const MAX_LOCK_CASE: usize = 8;

/// `LockKeyType` (`SharedDefines.h:818-823`).
pub mod lock_key {
    pub const NONE: u32 = 0;
    pub const ITEM: u32 = 1;
    pub const SKILL: u32 = 2;
}

/// One `LockEntry` subset (`DBCStructure.h:456-463`;
/// `Action[8]` omitted, matching the producer skip).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LockEntry {
    pub id: u32,
    pub lock_type: [u32; MAX_LOCK_CASE],
    pub index: [u32; MAX_LOCK_CASE],
    pub skill: [u32; MAX_LOCK_CASE],
}

fn word(row: &[u32], idx: usize) -> u32 {
    row.get(idx).copied().unwrap_or(0)
}

impl LockEntry {
    /// Build from one produced `Lock.dbc` row
    /// (fields 0-24: id, `Type[8]`, `Index[8]`,
    /// `Skill[8]`), assembled by the mangosd binary
    /// from the generic DBC row.
    #[must_use]
    pub fn from_produced_row(row: &[u32]) -> LockEntry {
        let mut entry = LockEntry {
            id: word(row, 0),
            ..LockEntry::default()
        };
        for i in 0..MAX_LOCK_CASE {
            entry.lock_type[i] = word(row, 1 + i);
            entry.index[i] = word(row, 9 + i);
            entry.skill[i] = word(row, 17 + i);
        }
        entry
    }
}

/// Loaded `Lock.dbc` (`sLockStore`).
#[derive(Clone, Debug, Default)]
pub struct LockStore {
    entries: HashMap<u32, LockEntry>,
}

impl LockStore {
    /// Builds the store and its id index.
    #[must_use]
    pub fn from_entries(entries: Vec<LockEntry>) -> Self {
        let entries = entries.into_iter().map(|e| (e.id, e)).collect();
        Self { entries }
    }

    /// `sLockStore.LookupEntry(lockId)`.
    #[must_use]
    pub fn get(&self, lock_id: u32) -> Option<LockEntry> {
        self.entries.get(&lock_id).copied()
    }

    /// Id set for template validation
    /// (`CheckGameObjectTemplate` lock arms).
    #[must_use]
    pub fn ids(&self) -> std::collections::HashSet<u32> {
        self.entries.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Vec<u32> {
        (36..61).collect()
    }

    #[test]
    fn produced_row_maps_triples() {
        let entry = LockEntry::from_produced_row(&row());
        assert_eq!(entry.id, 36);
        assert_eq!(entry.lock_type, [37, 38, 39, 40, 41, 42, 43, 44]);
        assert_eq!(entry.index, [45, 46, 47, 48, 49, 50, 51, 52]);
        assert_eq!(entry.skill, [53, 54, 55, 56, 57, 58, 59, 60]);
    }

    #[test]
    fn lookup_by_lock_id() {
        let store = LockStore::from_entries(vec![LockEntry {
            id: 36,
            ..LockEntry::default()
        }]);
        assert_eq!(store.get(36).map(|e| e.id), Some(36));
        assert_eq!(store.get(1), None);
    }

    #[test]
    fn short_row_zero_fills() {
        let entry = LockEntry::from_produced_row(&[7]);
        assert_eq!(entry.id, 7);
        assert_eq!(entry.lock_type, [0; MAX_LOCK_CASE]);
    }
}
