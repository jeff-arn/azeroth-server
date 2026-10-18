//! Faction templates and the initial reputation state
//! projection (`ObjectMgr::LoadFactions`,
//! `ReputationMgr::Initialize`, `ReputationMgr::LoadFromDB`,
//! and `SendInitialReputations`).

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_db::character_load::ReputationRow;
use mysql::prelude::Queryable;

/// Client build whose `faction` rows are selected
/// (`SUPPORTED_CLIENT_BUILD`).
const SUPPORTED_CLIENT_BUILD: u32 = 5875;

/// `FACTION_FLAG_*` (`ReputationMgr.h`).
const FACTION_FLAG_VISIBLE: u32 = 0x01;
const FACTION_FLAG_AT_WAR: u32 = 0x02;
const FACTION_FLAG_HIDDEN: u32 = 0x04;
const FACTION_FLAG_INVISIBLE_FORCED: u32 = 0x08;
const FACTION_FLAG_PEACE_FORCED: u32 = 0x10;
const FACTION_FLAG_INACTIVE: u32 = 0x20;

/// Stored `character_reputation.flags` bits
/// (`ReputationMgr::LoadFromDB`).
const FACTION_DB_VISIBLE: i32 = 0x01;
const FACTION_DB_ATWAR: i32 = 0x02;
const FACTION_DB_INACTIVE: i32 = 0x04;

/// `SMSG_INITIALIZE_FACTIONS` always carries 64 slots.
pub const INITIAL_FACTION_SLOTS: usize = 64;

/// The `faction` fields reputation initialization consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactionEntry {
    pub id: u32,
    pub reputation_index: i32,
    pub race_masks: [u32; 4],
    pub class_masks: [u32; 4],
    pub base_values: [i32; 4],
    pub flags: [u32; 4],
}

impl FactionEntry {
    /// `FactionEntry::GetIndexFitTo`: first slot matching
    /// race and (empty or matching) class masks.
    #[must_use]
    pub fn index_fit_to(&self, race_mask: u32, class_mask: u32) -> Option<usize> {
        (0..4).find(|&index| {
            self.race_masks[index] & race_mask != 0
                && (self.class_masks[index] == 0 || self.class_masks[index] & class_mask != 0)
        })
    }
}

/// Loaded `faction` rows by entry and reputation-list index.
#[derive(Clone, Debug, Default)]
pub struct FactionStore {
    by_id: HashMap<u32, FactionEntry>,
    by_reputation_index: HashMap<i32, FactionEntry>,
}

impl FactionStore {
    #[must_use]
    pub fn get(&self, faction_id: u32) -> Option<FactionEntry> {
        self.by_id.get(&faction_id).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    fn from_entries(entries: impl IntoIterator<Item = FactionEntry>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_reputation_index = HashMap::new();
        for entry in entries {
            by_id.insert(entry.id, entry);
            if entry.reputation_index >= 0 {
                by_reputation_index.insert(entry.reputation_index, entry);
            }
        }
        Self {
            by_id,
            by_reputation_index,
        }
    }

    /// `ReputationMgr::Initialize` plus `LoadFromDB`, reduced
    /// to the 64 `(flags, standing)` pairs on the wire.
    /// Standings are the stored deltas, exactly as
    /// `SendInitialReputations` sends `FactionState.Standing`.
    #[must_use]
    pub fn initial_slots(
        &self,
        race: u8,
        class: u8,
        rows: &[ReputationRow],
    ) -> [(u8, i32); INITIAL_FACTION_SLOTS] {
        let race_mask = race_mask(race);
        let class_mask = class_mask(class);
        let mut states: HashMap<u32, (u32, i32)> = HashMap::new();

        for entry in self.by_id.values() {
            if entry.reputation_index >= 0 {
                states.insert(entry.id, (default_flags(entry, race_mask, class_mask), 0));
            }
        }

        for row in rows {
            let Some(entry) = self.by_id.get(&row.faction).copied() else {
                continue;
            };
            let Some(state) = states.get_mut(&entry.id) else {
                continue;
            };
            state.1 = row.standing;

            if row.flags & FACTION_DB_VISIBLE != 0 && state.0 & FACTION_FLAG_INVISIBLE_FORCED == 0 {
                state.0 |= FACTION_FLAG_VISIBLE;
                state.0 &= !FACTION_FLAG_HIDDEN;
            }
            if row.flags & FACTION_DB_INACTIVE != 0 {
                state.0 |= FACTION_FLAG_INACTIVE;
            }
            if row.flags & FACTION_DB_ATWAR != 0 {
                set_at_war(state, true);
            } else if row.flags & FACTION_DB_VISIBLE != 0 {
                set_at_war(state, false);
            }

            let base = entry
                .index_fit_to(race_mask, class_mask)
                .map_or(0, |index| entry.base_values[index]);
            if base.saturating_add(state.1) < -3000 {
                set_at_war(state, true);
            }
        }

        let mut slots = [(0u8, 0i32); INITIAL_FACTION_SLOTS];
        for entry in self.by_reputation_index.values() {
            let Ok(index) = usize::try_from(entry.reputation_index) else {
                continue;
            };
            if index >= INITIAL_FACTION_SLOTS {
                continue;
            }
            let (flags, standing) = states.get(&entry.id).copied().unwrap_or((0, 0));
            slots[index] = (flags as u8, standing);
        }
        slots
    }
}

fn race_mask(race: u8) -> u32 {
    if race == 0 {
        0
    } else {
        1u32.checked_shl(u32::from(race - 1)).unwrap_or(0)
    }
}

fn class_mask(class: u8) -> u32 {
    if class == 0 {
        0
    } else {
        1u32.checked_shl(u32::from(class - 1)).unwrap_or(0)
    }
}

fn default_flags(entry: &FactionEntry, race_mask: u32, class_mask: u32) -> u32 {
    entry
        .index_fit_to(race_mask, class_mask)
        .map_or(0, |index| entry.flags[index])
}

fn set_at_war(state: &mut (u32, i32), on: bool) {
    if state.0 & FACTION_FLAG_PEACE_FORCED != 0 {
        return;
    }
    if on {
        state.0 |= FACTION_FLAG_AT_WAR;
    } else {
        state.0 &= !FACTION_FLAG_AT_WAR;
    }
}

/// Loads `ObjectMgr::LoadFactions` for build 5875.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_factions(conn: &mut mysql::Conn) -> Result<FactionStore, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `reputation_list_id`, \
         `base_rep_race_mask1`, `base_rep_race_mask2`, `base_rep_race_mask3`, \
         `base_rep_race_mask4`, `base_rep_class_mask1`, `base_rep_class_mask2`, \
         `base_rep_class_mask3`, `base_rep_class_mask4`, `base_rep_value1`, \
         `base_rep_value2`, `base_rep_value3`, `base_rep_value4`, \
         `reputation_flags1`, `reputation_flags2`, `reputation_flags3`, `reputation_flags4` \
         FROM `faction` t1 \
         WHERE `build` = (SELECT max(`build`) FROM `faction` t2 \
                          WHERE t1.`id` = t2.`id` AND `build` <= ?)",
        (SUPPORTED_CLIENT_BUILD,),
    )?;
    Ok(FactionStore::from_entries(rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        FactionEntry {
            id: f.u32(0),
            reputation_index: f.i32(1),
            race_masks: [f.u32(2), f.u32(3), f.u32(4), f.u32(5)],
            class_masks: [f.u32(6), f.u32(7), f.u32(8), f.u32(9)],
            base_values: [f.i32(10), f.i32(11), f.i32(12), f.i32(13)],
            flags: [f.u32(14), f.u32(15), f.u32(16), f.u32(17)],
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, reputation_index: i32, flags: u32, base: i32) -> FactionEntry {
        FactionEntry {
            id,
            reputation_index,
            race_masks: [0xFFFF_FFFF, 0, 0, 0],
            class_masks: [0, 0, 0, 0],
            base_values: [base, 0, 0, 0],
            flags: [flags, 0, 0, 0],
        }
    }

    #[test]
    fn initial_slots_apply_defaults_and_saved_flags() {
        let store = FactionStore::from_entries([
            entry(72, 6, 0, 0),
            entry(47, 8, FACTION_FLAG_PEACE_FORCED, -5000),
            entry(999, -1, FACTION_FLAG_VISIBLE, 0),
        ]);
        let rows = [
            ReputationRow {
                faction: 72,
                standing: 125,
                flags: FACTION_DB_VISIBLE | FACTION_DB_INACTIVE,
            },
            ReputationRow {
                faction: 47,
                standing: 0,
                flags: FACTION_DB_VISIBLE | FACTION_DB_ATWAR,
            },
        ];

        let slots = store.initial_slots(1, 1, &rows);
        assert_eq!(
            slots[6],
            ((FACTION_FLAG_VISIBLE | FACTION_FLAG_INACTIVE) as u8, 125)
        );
        assert_eq!(
            slots[8].0,
            (FACTION_FLAG_PEACE_FORCED | FACTION_FLAG_VISIBLE) as u8
        );
        assert_eq!(slots[8].1, 0);
    }

    #[test]
    fn initial_slots_force_war_at_hostile_threshold() {
        let store = FactionStore::from_entries([entry(72, 0, FACTION_FLAG_VISIBLE, -2900)]);
        let rows = [ReputationRow {
            faction: 72,
            standing: -101,
            flags: FACTION_DB_VISIBLE,
        }];
        let slots = store.initial_slots(2, 1, &rows);
        assert_eq!(
            slots[0],
            ((FACTION_FLAG_VISIBLE | FACTION_FLAG_AT_WAR) as u8, -101)
        );
    }
}
