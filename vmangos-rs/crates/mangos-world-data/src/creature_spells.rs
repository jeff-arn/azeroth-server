//! `creature_spells` spell-list templates
//! (`ObjectMgr::LoadCreatureSpells`): one wide row per
//! list entry with 8 spell slots; delays are stored in
//! seconds in the DB and converted to milliseconds.
//! Validation mirrors C++: unknown spell ids (against
//! the loaded spell store) skip the slot, invalid
//! probability clamps to 100, inverted delay ranges
//! skip the slot.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use mangos_db::DbError;
use mysql::prelude::Queryable;

use crate::load_report::LoadReport;

/// `CREATURE_SPELLS_MAX_SPELLS`.
pub const MAX_CREATURE_SPELLS: usize = 8;
/// `CREATURE_SPELLS_MAX_COLUMNS`.
const MAX_COLUMNS: usize = 11;

/// One spell entry (`CreatureSpellsEntry`).
#[derive(Clone, Debug, PartialEq)]
pub struct CreatureSpellEntry {
    pub spell_id: u32,
    pub probability: u8,
    /// `ScriptTarget` enum value.
    pub cast_target: u8,
    pub target_param1: u32,
    pub target_param2: u32,
    /// `CastFlags` bitmask.
    pub cast_flags: u16,
    pub delay_initial_min: u32,
    pub delay_initial_max: u32,
    pub delay_repeat_min: u32,
    pub delay_repeat_max: u32,
    /// `creature_spells_scripts` id (0 = none; scripts
    /// engine is Slice 6b).
    pub script_id: u32,
}

/// List id -> spell entries (`m_CreatureSpellsMap`).
pub type CreatureSpellsMap = HashMap<u32, Vec<CreatureSpellEntry>>;

/// Load `creature_spells`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_creature_spells<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    spell_ids: &HashSet<u32, S>,
    report: &mut LoadReport,
) -> Result<CreatureSpellsMap, DbError> {
    let mut query = String::from("SELECT `entry`");
    for i in 1..=MAX_CREATURE_SPELLS {
        let _ = write!(
            query,
            ", `spellId_{i}`, `probability_{i}`, `castTarget_{i}`, \
             `targetParam1_{i}`, `targetParam2_{i}`, `castFlags_{i}`, \
             `delayInitialMin_{i}`, `delayInitialMax_{i}`, \
             `delayRepeatMin_{i}`, `delayRepeatMax_{i}`, `scriptId_{i}`"
        );
    }
    query.push_str(" FROM `creature_spells`");
    let rows: Vec<mysql::Row> = conn.query(query).map_err(DbError::from)?;
    let mut out: CreatureSpellsMap = HashMap::new();
    for row in rows {
        let entry: u32 = row.get(0).unwrap_or_default();
        let mut list: Vec<CreatureSpellEntry> = Vec::new();
        for slot in 0..MAX_CREATURE_SPELLS {
            let base = 1 + slot * MAX_COLUMNS;
            let spell_id: u32 = row.get(base).unwrap_or_default();
            if spell_id == 0 {
                continue;
            }
            if !spell_ids.contains(&spell_id) {
                report.error(format!(
                    "Entry {entry} in table `creature_spells` has non-existent spell                      {spell_id} used as spellId_{}, skipping spell.",
                    slot + 1
                ));
                continue;
            }
            let mut probability: u8 = row.get(base + 1).unwrap_or_default();
            if probability == 0 || probability > 100 {
                probability = 100;
            }
            let delay_initial_min: u32 =
                u32::from(row.get::<u16, _>(base + 6).unwrap_or_default()) * 1000;
            let delay_initial_max: u32 =
                u32::from(row.get::<u16, _>(base + 7).unwrap_or_default()) * 1000;
            if delay_initial_min > delay_initial_max {
                report.error(format!(
                    "Entry {entry} in table `creature_spells` has invalid initial timers, skipping spell."
                ));
                continue;
            }
            let delay_repeat_min: u32 =
                u32::from(row.get::<u16, _>(base + 8).unwrap_or_default()) * 1000;
            let delay_repeat_max: u32 =
                u32::from(row.get::<u16, _>(base + 9).unwrap_or_default()) * 1000;
            if delay_repeat_min > delay_repeat_max {
                report.error(format!(
                    "Entry {entry} in table `creature_spells` has invalid repeat timers, skipping spell."
                ));
                continue;
            }
            list.push(CreatureSpellEntry {
                spell_id,
                probability,
                cast_target: row.get(base + 2).unwrap_or_default(),
                target_param1: row.get(base + 3).unwrap_or_default(),
                target_param2: row.get(base + 4).unwrap_or_default(),
                cast_flags: row.get(base + 5).unwrap_or_default(),
                delay_initial_min,
                delay_initial_max,
                delay_repeat_min,
                delay_repeat_max,
                script_id: row.get(base + 10).unwrap_or_default(),
            });
        }
        if !list.is_empty() {
            out.insert(entry, list);
        }
    }
    Ok(out)
}
