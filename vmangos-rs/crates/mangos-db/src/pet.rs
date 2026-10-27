//! `character_pet` rows (Tier 1 Slice 2): load at
//! character load, and the `Pet::SavePetToDB` /
//! `Pet::DeleteFromDB` statement sequences. Statement
//! texts match the fork byte-for-byte, including the
//! double space after `` `entry`, `` in the INSERT
//! column list. The fork reads rows through its
//! character-database cache with the same SELECT
//! shape (commented query text in
//! `Pet::LoadPetFromDB`).
//!
//! Pet spells/cooldowns/auras (`pet_spell`,
//! `pet_spell_cooldown`, `pet_aura`) ride in the
//! row's spell list for this slice; the fork's
//! separate `_SaveSpells` tables are written by
//! `save_pet_spells` so the C++ server can read a
//! Rust-saved pet.

use mysql::prelude::Queryable;

use crate::DbError;

/// `PET_SAVE_AS_CURRENT`.
pub const PET_SAVE_AS_CURRENT: u32 = 0;
/// `PET_SAVE_LAST_STABLE_SLOT` (`MAX_PET_STABLES` = 2
/// in this fork's 1.12 build).
pub const PET_SAVE_LAST_STABLE_SLOT: u32 = 2;
/// `PET_SAVE_NOT_IN_SLOT`.
pub const PET_SAVE_NOT_IN_SLOT: u32 = 100;

/// `SUMMON_PET`.
pub const PET_TYPE_SUMMON: u32 = 0;
/// `HUNTER_PET`.
pub const PET_TYPE_HUNTER: u32 = 1;

/// One `character_pet` row (fork `CharacterPetCache`
/// field set, in `LoadPetFromDB` SELECT order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PetRow {
    pub id: u32,
    pub entry: u32,
    pub owner_guid: u32,
    pub display_id: u32,
    pub level: u32,
    pub xp: u32,
    pub react_state: u32,
    pub loyalty_points: i32,
    pub loyalty: u32,
    pub training_points: i32,
    pub slot: u32,
    pub name: String,
    pub renamed: u32,
    pub current_health: u32,
    pub current_mana: u32,
    pub current_happiness: u32,
    /// 10 `(type, action)` pairs, fork string form
    /// parsed (`"t a t a ... "` with trailing space).
    pub action_bar: [(u32, u32); 10],
    /// Up to 4 `(ability, teaching spell)` pairs.
    pub teach_spells: Vec<(u32, u32)>,
    pub save_time: u64,
    pub reset_talents_cost: u32,
    pub reset_talents_time: u64,
    pub created_by_spell: u32,
    pub pet_type: u32,
    /// `pet_spell` rows `(spell, active)`, load-only
    /// companion data (written by `save_pet_spells`).
    pub spells: Vec<(u32, u32)>,
}

impl PetRow {
    /// Serialize the action bar exactly as
    /// `SavePetToDB` does: `"type action "` x10.
    #[must_use]
    pub fn action_bar_data(&self) -> String {
        let mut out = String::new();
        for (kind, action) in self.action_bar {
            out.push_str(&kind.to_string());
            out.push(' ');
            out.push_str(&action.to_string());
            out.push(' ');
        }
        out
    }

    /// Serialize teach spells: 4 pairs, zero-padded,
    /// each `"ability teacher "`.
    #[must_use]
    pub fn teach_spell_data(&self) -> String {
        let mut out = String::new();
        for i in 0..4 {
            let (ability, teacher) = self.teach_spells.get(i).copied().unwrap_or((0, 0));
            out.push_str(&ability.to_string());
            out.push(' ');
            out.push_str(&teacher.to_string());
            out.push(' ');
        }
        out
    }
}

/// Parse the fork action-bar string; broken data
/// yields zeroed slots past the break (the fork
/// refills by defaults later, `LoadPetActionBar`).
#[must_use]
pub fn parse_action_bar(data: &str) -> [(u32, u32); 10] {
    let mut out = [(0u32, 0u32); 10];
    let mut tokens = data.split_whitespace();
    for slot in &mut out {
        let Some(kind) = tokens.next().and_then(|t| t.parse::<u32>().ok()) else {
            break;
        };
        let Some(action) = tokens.next().and_then(|t| t.parse::<u32>().ok()) else {
            break;
        };
        *slot = (kind, action);
    }
    out
}

/// Parse teach-spell pairs (stops at the first zero
/// ability, fork semantics).
#[must_use]
pub fn parse_teach_spells(data: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut tokens = data.split_whitespace();
    for _ in 0..4 {
        let Some(ability) = tokens.next().and_then(|t| t.parse::<u32>().ok()) else {
            break;
        };
        let teacher = tokens
            .next()
            .and_then(|t| t.parse::<u32>().ok())
            .unwrap_or(0);
        if ability == 0 {
            break;
        }
        out.push((ability, teacher));
    }
    out
}

const LOAD_PETS_SQL: &str = "SELECT `id`, `entry`, `owner_guid`, `display_id`, `level`, \
    `xp`, `react_state`, `loyalty_points`, `loyalty`, `training_points`, \
    `slot`, `name`, `renamed`, `current_health`, `current_mana`, \
    `current_happiness`, `action_bar_data`, `teach_spell_data`, `save_time`, \
    `reset_talents_cost`, `reset_talents_time`, `created_by_spell`, `pet_type` \
    FROM `character_pet` WHERE `owner_guid` = ?";

/// Load every pet row for an owner (the fork lazy-
/// loads single rows through its cache; loading the
/// ≤3 rows at character load is behavior-identical
/// for the runtime's selection rules). Also loads
/// each row's `pet_spell` list.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_pets(conn: &mut mysql::Conn, owner_low: u32) -> Result<Vec<PetRow>, DbError> {
    let rows: Vec<mysql::Row> = conn
        .exec(LOAD_PETS_SQL, (owner_low,))
        .map_err(DbError::Driver)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let action_bar_data: String = row.get(16).unwrap_or_default();
        let teach_spell_data: String = row.get(17).unwrap_or_default();
        let id: u32 = row.get(0).unwrap_or_default();
        let spells = load_pet_spells(conn, id)?;
        out.push(PetRow {
            id,
            entry: row.get(1).unwrap_or_default(),
            owner_guid: row.get(2).unwrap_or_default(),
            display_id: row.get(3).unwrap_or_default(),
            level: row.get(4).unwrap_or_default(),
            xp: row.get(5).unwrap_or_default(),
            react_state: row.get(6).unwrap_or_default(),
            loyalty_points: row.get(7).unwrap_or_default(),
            loyalty: row.get(8).unwrap_or_default(),
            training_points: row.get(9).unwrap_or_default(),
            slot: row.get(10).unwrap_or_default(),
            name: row.get(11).unwrap_or_default(),
            renamed: row.get(12).unwrap_or_default(),
            current_health: row.get(13).unwrap_or_default(),
            current_mana: row.get(14).unwrap_or_default(),
            current_happiness: row.get(15).unwrap_or_default(),
            action_bar: parse_action_bar(&action_bar_data),
            teach_spells: parse_teach_spells(&teach_spell_data),
            save_time: row.get(18).unwrap_or_default(),
            reset_talents_cost: row.get(19).unwrap_or_default(),
            reset_talents_time: row.get(20).unwrap_or_default(),
            created_by_spell: row.get(21).unwrap_or_default(),
            pet_type: row.get(22).unwrap_or_default(),
            spells,
        });
    }
    Ok(out)
}

/// `Pet::_LoadSpells` row set (spell, active).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_pet_spells(conn: &mut mysql::Conn, pet_id: u32) -> Result<Vec<(u32, u32)>, DbError> {
    let rows: Vec<mysql::Row> = conn
        .exec(
            "SELECT `spell`, `active` FROM `pet_spell` WHERE `guid` = ?",
            (pet_id,),
        )
        .map_err(DbError::Driver)?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0).unwrap_or_default(), r.get(1).unwrap_or_default()))
        .collect())
}

/// `ObjectMgr::LoadPetNumber` seed: fork starts at 1
/// and `GeneratePetNumber` walks the cache for the
/// next free id; the Rust counter starts at
/// `MAX(id) + 1` which yields the same allocations
/// for a fresh boot.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn next_pet_number_seed(conn: &mut mysql::Conn) -> Result<u32, DbError> {
    let row: Option<mysql::Row> = conn
        .query_first("SELECT COALESCE(MAX(`id`), 0) FROM `character_pet`")
        .map_err(DbError::Driver)?;
    let max_id: u32 = row.and_then(|r| r.get(0)).unwrap_or_default();
    Ok(max_id + 1)
}

/// `Pet::SavePetToDB(mode >= AS_CURRENT)` statement
/// sequence. `mode` is the slot value to write
/// (REAGENTS already collapsed by the caller to
/// `NOT_IN_SLOT`, fork-side). Caller owns the
/// transaction boundary (fork wraps the whole body).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_pet(conn: &mut mysql::Conn, row: &PetRow, mode: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_pet` WHERE `owner_guid` = ? AND `id` = ?",
        (row.owner_guid, row.id),
    )
    .map_err(DbError::Driver)?;
    if mode <= PET_SAVE_LAST_STABLE_SLOT {
        conn.exec_drop(
            "UPDATE `character_pet` SET `slot` = ? WHERE `owner_guid` = ? AND `slot` = ?",
            (PET_SAVE_NOT_IN_SLOT, row.owner_guid, mode),
        )
        .map_err(DbError::Driver)?;
    }
    if row.pet_type == PET_TYPE_HUNTER
        && (mode == PET_SAVE_AS_CURRENT || mode > PET_SAVE_LAST_STABLE_SLOT)
    {
        conn.exec_drop(
            "DELETE FROM `character_pet` WHERE `owner_guid` = ? AND (`slot` = ? OR `slot` > ?)",
            (
                row.owner_guid,
                PET_SAVE_AS_CURRENT,
                PET_SAVE_LAST_STABLE_SLOT,
            ),
        )
        .map_err(DbError::Driver)?;
    }
    conn.exec_drop(
        "INSERT INTO `character_pet` \
         ( `id`, `entry`,  `owner_guid`, `display_id`, `level`, `xp`, \
         `react_state`, `loyalty_points`, `loyalty`, `training_points`, \
         `slot`, `name`, `renamed`, `current_health`, `current_mana`, \
         `current_happiness`, `action_bar_data`, `teach_spell_data`, \
         `save_time`, `reset_talents_cost`, `reset_talents_time`, \
         `created_by_spell`, `pet_type`) \
         VALUES ( ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        mysql::Params::Positional(vec![
            row.id.into(),
            row.entry.into(),
            row.owner_guid.into(),
            row.display_id.into(),
            row.level.into(),
            row.xp.into(),
            row.react_state.into(),
            row.loyalty_points.into(),
            row.loyalty.into(),
            row.training_points.into(),
            mode.into(),
            row.name.clone().into(),
            row.renamed.into(),
            row.current_health.max(1).into(),
            row.current_mana.into(),
            row.current_happiness.into(),
            row.action_bar_data().into(),
            row.teach_spell_data().into(),
            row.save_time.into(),
            row.reset_talents_cost.into(),
            row.reset_talents_time.into(),
            row.created_by_spell.into(),
            row.pet_type.into(),
        ]),
    )
    .map_err(DbError::Driver)?;
    save_pet_spells(conn, row)
}

/// `Pet::_SaveSpells`: rewrite the `pet_spell` rows
/// for this pet (the fork deletes then re-inserts
/// non-removed spells).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_pet_spells(conn: &mut mysql::Conn, row: &PetRow) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `pet_spell` WHERE `guid` = ?", (row.id,))
        .map_err(DbError::Driver)?;
    for (spell, active) in &row.spells {
        conn.exec_drop(
            "INSERT INTO `pet_spell` (`guid`, `spell`, `active`) VALUES (?, ?, ?)",
            (row.id, spell, active),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `Pet::DeleteFromDB` (without aura/cooldown
/// tables, which this runtime does not write yet —
/// the DELETEs are still issued for C++-saved rows).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_pet(conn: &mut mysql::Conn, pet_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `character_pet` WHERE `id` = ?", (pet_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop("DELETE FROM `pet_aura` WHERE `guid` = ?", (pet_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop("DELETE FROM `pet_spell` WHERE `guid` = ?", (pet_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `pet_spell_cooldown` WHERE `guid` = ?",
        (pet_id,),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// The `HandlePetRename` immediate UPDATE
/// (statement shape from the fork; bound instead
/// of escaped-interpolated).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn rename_pet(
    conn: &mut mysql::Conn,
    owner_low: u32,
    pet_id: u32,
    name: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_pet` SET `name` = ?, `renamed` = '1' WHERE `owner_guid` = ? AND `id` = ?",
        (name, owner_low, pet_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// The slot-promotion pair from `LoadPetFromDB`
/// (loaded pet becomes current): demote whoever
/// held slot 0, promote this pet.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn promote_pet_to_current(
    conn: &mut mysql::Conn,
    owner_low: u32,
    pet_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_pet` SET `slot` = ? WHERE `owner_guid` = ? AND `slot` = ? AND `id` <> ?",
        (PET_SAVE_NOT_IN_SLOT, owner_low, PET_SAVE_AS_CURRENT, pet_id),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "UPDATE `character_pet` SET `slot` = ? WHERE `owner_guid` = ? AND `id` = ?",
        (PET_SAVE_AS_CURRENT, owner_low, pet_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_bar_round_trip() {
        let mut row = PetRow::default();
        row.action_bar[0] = (7, 2);
        row.action_bar[9] = (6, 0);
        let data = row.action_bar_data();
        assert_eq!(parse_action_bar(&data), row.action_bar);
        assert!(data.ends_with(' '));
    }

    #[test]
    fn teach_spells_round_trip_and_zero_stop() {
        let row = PetRow {
            teach_spells: vec![(17253, 17101), (24579, 24447)],
            ..Default::default()
        };
        assert_eq!(
            parse_teach_spells(&row.teach_spell_data()),
            row.teach_spells
        );
        assert_eq!(parse_teach_spells("0 0 5 6 "), Vec::new());
    }
}
