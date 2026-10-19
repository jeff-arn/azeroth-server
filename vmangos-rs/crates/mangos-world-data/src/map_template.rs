//! `map_template` rows (`ObjectMgr::LoadMapTemplate` /
//! `sMapStorage`): map parent/type/linked-zone plus
//! dungeon reset and ghost-entrance data.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// `enum MapTypes` (`DBCEnums.h`).
pub mod map_type {
    pub const COMMON: u32 = 0;
    pub const INSTANCE: u32 = 1;
    pub const RAID: u32 = 2;
    pub const BATTLEGROUND: u32 = 3;
}

/// One `map_template` row (`MapEntry`).
#[derive(Clone, Debug, PartialEq)]
pub struct MapEntry {
    pub id: u32,
    pub parent: u32,
    pub map_type: u32,
    pub linked_zone: u32,
    pub max_players: u32,
    pub reset_delay: u32,
    pub ghost_entrance_map: i32,
    pub ghost_entrance_x: f32,
    pub ghost_entrance_y: f32,
    pub name: String,
    pub script_name: String,
}

impl MapEntry {
    /// `MapEntry::IsDungeon`.
    #[must_use]
    pub fn is_dungeon(&self) -> bool {
        self.map_type == map_type::INSTANCE || self.map_type == map_type::RAID
    }

    /// `MapEntry::IsNonRaidDungeon`.
    #[must_use]
    pub fn is_non_raid_dungeon(&self) -> bool {
        self.map_type == map_type::INSTANCE
    }

    /// `MapEntry::Instanceable`.
    #[must_use]
    pub fn instanceable(&self) -> bool {
        self.is_dungeon() || self.map_type == map_type::BATTLEGROUND
    }

    /// `MapEntry::IsRaid`.
    #[must_use]
    pub fn is_raid(&self) -> bool {
        self.map_type == map_type::RAID
    }

    /// `MapEntry::IsBattleGround`.
    #[must_use]
    pub fn is_battleground(&self) -> bool {
        self.map_type == map_type::BATTLEGROUND
    }

    /// `MapEntry::IsContinent`.
    #[must_use]
    pub fn is_continent(&self) -> bool {
        self.id == 0 || self.id == 1
    }
}

/// All progression-selected `map_template` rows by id
/// (`sMapStorage`).
#[derive(Clone, Debug, Default)]
pub struct MapStore {
    entries: HashMap<u32, MapEntry>,
}

impl MapStore {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `sMapStorage.LookupEntry<MapEntry>`.
    #[must_use]
    pub fn get(&self, map_id: u32) -> Option<&MapEntry> {
        self.entries.get(&map_id)
    }

    #[must_use]
    pub fn known(&self, map_id: u32) -> bool {
        self.entries.contains_key(&map_id)
    }

    /// Dungeon/raid check for a possibly-unknown map
    /// (unknown maps are not dungeons).
    #[must_use]
    pub fn is_instance_map(&self, map_id: u32) -> bool {
        self.get(map_id).is_some_and(MapEntry::instanceable)
    }

    fn from_entries(entries: impl IntoIterator<Item = MapEntry>, reset_rate: f32) -> Self {
        let mut by_id: HashMap<u32, MapEntry> = entries.into_iter().map(|e| (e.id, e)).collect();
        // Fork fixups (`LoadMapTemplate`): invalid or
        // continent parents on dungeons are cleared.
        let parents: Vec<(u32, u32)> = by_id
            .values()
            .filter(|e| e.is_dungeon() && e.parent > 0)
            .map(|e| (e.id, e.parent))
            .collect();
        for (id, parent) in parents {
            let bad = by_id.get(&parent).is_none_or(MapEntry::is_continent);
            if bad {
                if let Some(entry) = by_id.get_mut(&id) {
                    entry.parent = 0;
                }
            }
        }
        for entry in by_id.values_mut() {
            if entry.reset_delay != 0 {
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let scaled = (entry.reset_delay as f32 * reset_rate) as u32;
                entry.reset_delay = scaled.max(1);
            }
        }
        Self { entries: by_id }
    }
}

/// Loads the progression-selected `map_template` rows
/// (`SQLMapLoader::LoadProgressive`) and applies the
/// fork parent/reset fixups.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_map_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    reset_rate: f32,
) -> Result<MapStore, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `parent`, `map_type`, `linked_zone`, `player_limit`, \
         `reset_delay`, `ghost_entrance_map`, `ghost_entrance_x`, \
         `ghost_entrance_y`, `map_name`, `script_name` \
         FROM `map_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `map_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let entries = rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        MapEntry {
            id: f.u32(0),
            parent: f.u32(1),
            map_type: f.u32(2),
            linked_zone: f.u32(3),
            max_players: f.u32(4),
            reset_delay: f.u32(5),
            ghost_entrance_map: f.i32(6),
            ghost_entrance_x: f.f32(7),
            ghost_entrance_y: f.f32(8),
            name: f.string(9),
            script_name: f.string(10),
        }
    });
    Ok(MapStore::from_entries(entries, reset_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, parent: u32, map_type: u32) -> MapEntry {
        MapEntry {
            id,
            parent,
            map_type,
            linked_zone: 0,
            max_players: 0,
            reset_delay: 0,
            ghost_entrance_map: -1,
            ghost_entrance_x: 0.0,
            ghost_entrance_y: 0.0,
            name: String::new(),
            script_name: String::new(),
        }
    }

    #[test]
    fn dungeon_flags_and_parent_fixups() {
        let store = MapStore::from_entries(
            [
                entry(0, 0, map_type::COMMON),
                entry(36, 0, map_type::INSTANCE),
                entry(389, 1, map_type::INSTANCE),
                entry(999, 777, map_type::RAID),
            ],
            1.0,
        );
        assert!(store.get(36).is_some_and(MapEntry::is_dungeon));
        assert!(store.get(36).is_some_and(MapEntry::is_non_raid_dungeon));
        assert!(store.get(999).is_some_and(MapEntry::is_raid));
        assert!(store.is_instance_map(36));
        assert!(!store.is_instance_map(0));
        assert!(!store.is_instance_map(4242));
        // Continent parent (map 1 unknown here too) and
        // missing parent both clear to 0.
        assert_eq!(store.get(389).map(|e| e.parent), Some(0));
        assert_eq!(store.get(999).map(|e| e.parent), Some(0));
    }

    #[test]
    fn reset_delay_scales_with_floor() {
        let mut e = entry(36, 0, map_type::INSTANCE);
        e.reset_delay = 3;
        let store = MapStore::from_entries([e], 0.5);
        assert_eq!(store.get(36).map(|m| m.reset_delay), Some(1));
    }
}
