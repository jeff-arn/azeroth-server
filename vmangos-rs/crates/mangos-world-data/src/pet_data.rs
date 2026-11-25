//! Pet static stores: `pet_levelstats`
//! (`ObjectMgr::LoadPetLevelInfo`), `petcreateinfo_spell`
//! (`ObjectMgr::LoadPetCreateSpells`), and
//! `pet_name_generation` (`ObjectMgr::LoadPetNames`).
//! Validation mirrors C++: levelstats rows for unknown
//! creature ids skip (unless the id exists in another
//! patch band — the fork's `IsExistingCreatureId`
//! nuance degrades to a plain template lookup here,
//! logged identically), levels outside
//! `1..=PLAYER_STRONG_MAX_LEVEL` skip; create-spell
//! rows with a set `pet_spell_list_id`, unknown
//! spells, or no spells at all skip.

use std::collections::{HashMap, HashSet};

use mangos_db::DbError;
use mysql::prelude::Queryable;

use crate::load_report::LoadReport;

/// `PLAYER_STRONG_MAX_LEVEL`.
const STRONG_MAX_LEVEL: u32 = 255;

/// One `pet_levelstats` row (`PetLevelInfo`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PetLevelInfo {
    pub health: u16,
    pub mana: u16,
    pub armor: u16,
    pub dmg_min: f32,
    pub dmg_max: f32,
    /// Strength, agility, stamina, intellect, spirit.
    pub stats: [u16; 5],
}

/// `(creature entry, level) -> PetLevelInfo`
/// (`m_PetInfoMap` flattened; hunter pets read entry 1).
pub type PetLevelStats = HashMap<(u32, u32), PetLevelInfo>;

/// Entry -> 4 create spell ids (`m_PetCreateSpellMap`).
pub type PetCreateSpells = HashMap<u32, [u32; 4]>;

/// Entry -> (first halves, second halves)
/// (`m_PetHalfNameMap0/1`).
pub type PetNameParts = HashMap<u32, PetNameHalves>;

/// The two half-name lists for one entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PetNameHalves {
    pub first: Vec<String>,
    pub second: Vec<String>,
}

/// All pet static data loaded at boot.
#[derive(Clone, Debug, Default)]
pub struct PetStores {
    pub level_stats: PetLevelStats,
    pub create_spells: PetCreateSpells,
    pub name_parts: PetNameParts,
}

impl PetStores {
    /// `ObjectMgr::GetPetLevelInfo(creatureId, level)`:
    /// `None` when the fork's zero-filled array slot
    /// would report no data (fork returns the slot and
    /// callers test individual fields; `None` here
    /// drives the same fallbacks).
    #[must_use]
    pub fn level_info(&self, entry: u32, level: u32) -> Option<&PetLevelInfo> {
        self.level_stats.get(&(entry, level))
    }

    /// `ObjectMgr::GeneratePetName(entry)` minus the
    /// family-name fallback (needs the creature
    /// template + DBC family names; the caller applies
    /// that when this returns `None`). `pick` is
    /// `urand(0, len-1)`.
    #[must_use]
    pub fn generated_name(
        &self,
        entry: u32,
        pick: &mut dyn FnMut(u32, u32) -> u32,
    ) -> Option<String> {
        let halves = self.name_parts.get(&entry)?;
        if halves.first.is_empty() || halves.second.is_empty() {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let a = pick(0, halves.first.len() as u32 - 1) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let b = pick(0, halves.second.len() as u32 - 1) as usize;
        Some(format!("{}{}", halves.first[a], halves.second[b]))
    }
}

/// Load the three pet stores. `wow_patch` selects
/// `petcreateinfo_spell` rows (`patch BETWEEN
/// patch_min AND patch_max`); `max_player_level` is
/// the fork's `CONFIG_UINT32_MAX_PLAYER_LEVEL` used
/// to log-but-count levelstats rows above it.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_pet_stores<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    creature_entries: &HashSet<u32, S>,
    pet_spell_list_ids: &dyn Fn(u32) -> u32,
    spell_ids: &HashSet<u32, S>,
    wow_patch: u32,
    max_player_level: u32,
    report: &mut LoadReport,
) -> Result<PetStores, DbError> {
    let level_stats = load_level_stats(conn, creature_entries, max_player_level, report)?;
    let create_spells = load_create_spells(
        conn,
        creature_entries,
        pet_spell_list_ids,
        spell_ids,
        wow_patch,
        report,
    )?;
    let name_parts = load_name_parts(conn)?;
    Ok(PetStores {
        level_stats,
        create_spells,
        name_parts,
    })
}

fn load_level_stats<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    creature_entries: &HashSet<u32, S>,
    max_player_level: u32,
    report: &mut LoadReport,
) -> Result<PetLevelStats, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query(
            "SELECT `entry`, `level`, `health`, `mana`, `armor`, `dmg_min`, \
             `dmg_max`, `strength`, `agility`, `stamina`, `intellect`, \
             `spirit` FROM `pet_levelstats`",
        )
        .map_err(DbError::from)?;
    let mut out: PetLevelStats = HashMap::new();
    if rows.is_empty() {
        report.error("Error loading `pet_levelstats` table or empty table.".to_string());
        return Ok(out);
    }
    for row in rows {
        let entry: u32 = row.get(0).unwrap_or_default();
        if !creature_entries.contains(&entry) {
            report.error(format!(
                "Wrong creature id {entry} in `pet_levelstats` table, ignoring."
            ));
            continue;
        }
        let level: u32 = row.get(1).unwrap_or_default();
        if level > max_player_level {
            if level > STRONG_MAX_LEVEL {
                report.error(format!(
                    "Wrong (> {STRONG_MAX_LEVEL}) level {level} in \
                     `pet_levelstats` table, ignoring."
                ));
            }
            continue;
        }
        if level < 1 {
            report.error(format!(
                "Wrong (<1) level {level} in `pet_levelstats` table, ignoring."
            ));
            continue;
        }
        out.insert(
            (entry, level),
            PetLevelInfo {
                health: row.get(2).unwrap_or_default(),
                mana: row.get(3).unwrap_or_default(),
                armor: row.get(4).unwrap_or_default(),
                dmg_min: row.get(5).unwrap_or_default(),
                dmg_max: row.get(6).unwrap_or_default(),
                stats: [
                    row.get(7).unwrap_or_default(),
                    row.get(8).unwrap_or_default(),
                    row.get(9).unwrap_or_default(),
                    row.get(10).unwrap_or_default(),
                    row.get(11).unwrap_or_default(),
                ],
            },
        );
    }
    Ok(out)
}

fn load_create_spells<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    creature_entries: &HashSet<u32, S>,
    pet_spell_list_ids: &dyn Fn(u32) -> u32,
    spell_ids: &HashSet<u32, S>,
    wow_patch: u32,
    report: &mut LoadReport,
) -> Result<PetCreateSpells, DbError> {
    let rows: Vec<mysql::Row> = conn
        .exec(
            "SELECT `entry`, `spell1`, `spell2`, `spell3`, `spell4` \
             FROM `petcreateinfo_spell` WHERE ? BETWEEN `patch_min` AND `patch_max`",
            (wow_patch,),
        )
        .map_err(DbError::from)?;
    let mut out: PetCreateSpells = HashMap::new();
    for row in rows {
        let entry: u32 = row.get(0).unwrap_or_default();
        if entry == 0 || !creature_entries.contains(&entry) {
            report.error(format!(
                "Creature id {entry} listed in `petcreateinfo_spell` not exist."
            ));
            continue;
        }
        if pet_spell_list_ids(entry) != 0 {
            report.error(format!(
                "Creature id {entry} listed in `petcreateinfo_spell` have set \
                 `pet_spell_list_id` field and will use its instead, skip."
            ));
            continue;
        }
        let mut spells = [0u32; 4];
        let mut have_spell = false;
        let mut have_spell_db = false;
        for (i, slot) in spells.iter_mut().enumerate() {
            *slot = row.get(1 + i).unwrap_or_default();
            if *slot == 0 {
                continue;
            }
            have_spell_db = true;
            if !spell_ids.contains(slot) {
                report.error(format!(
                    "Spell {slot} listed in `petcreateinfo_spell` does not exist"
                ));
                *slot = 0;
                continue;
            }
            have_spell = true;
        }
        if !have_spell_db {
            report.error(format!(
                "Creature {entry} listed in `petcreateinfo_spell` have only 0 \
                 spell data, why it listed?"
            ));
            continue;
        }
        if !have_spell {
            continue;
        }
        out.insert(entry, spells);
    }
    Ok(out)
}

fn load_name_parts(conn: &mut mysql::Conn) -> Result<PetNameParts, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query("SELECT `word`, `entry`, `half` FROM `pet_name_generation`")
        .map_err(DbError::from)?;
    let mut out: PetNameParts = HashMap::new();
    for row in rows {
        let word: String = row.get(0).unwrap_or_default();
        let entry: u32 = row.get(1).unwrap_or_default();
        let half: bool = row.get(2).unwrap_or_default();
        let halves = out.entry(entry).or_default();
        if half {
            halves.second.push(word);
        } else {
            halves.first.push(word);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_name_joins_halves() {
        let mut stores = PetStores::default();
        stores.name_parts.insert(
            416,
            PetNameHalves {
                first: vec!["Az".to_string(), "Bel".to_string()],
                second: vec!["gorg".to_string()],
            },
        );
        let mut pick = |lo: u32, _hi: u32| lo;
        assert_eq!(
            stores.generated_name(416, &mut pick).as_deref(),
            Some("Azgorg")
        );
        assert_eq!(stores.generated_name(999, &mut pick), None);
    }

    #[test]
    fn level_info_lookup() {
        let mut stores = PetStores::default();
        stores.level_stats.insert(
            (416, 19),
            PetLevelInfo {
                health: 136,
                mana: 369,
                armor: 268,
                ..Default::default()
            },
        );
        assert_eq!(stores.level_info(416, 19).map(|i| i.health), Some(136));
        assert_eq!(stores.level_info(416, 20), None);
    }
}
