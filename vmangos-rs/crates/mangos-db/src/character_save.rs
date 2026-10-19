//! Character save: `Player::SaveToDB` and its `_Save*`
//! bundle (`Player.cpp:16229-16900`), `Item::SaveToDB`
//! (`Item.cpp:253`), `ReputationMgr::SaveToDB`
//! (`ReputationMgr.cpp:448`), `HonorMgr::Save`
//! (`HonorMgr.cpp:633`). Statement shapes and the
//! bundle order mirror the fork; the caller executes
//! the whole bundle inside one DB-actor call, which
//! is the serial lane the fork's thread-local
//! `BeginTransaction(GUIDLow)` provided (the
//! characters schema is `MyISAM`: there is no rollback
//! to lean on, only ordering). The `IsSavingDisabled`
//! bot gate and the save scheduling live caller-side.

use mysql::Value;
use mysql::prelude::Queryable;

use crate::DbError;
use crate::character_load::{
    ActionRow, AuraRow, BgDataRow, ForgottenSkillRow, HonorCpRow, InventoryRow, ItemLootRow,
    QuestStatusRow, ReputationRow, SkillRow, SpellCooldownRow, SpellRow,
};

/// Bind inputs of the 61-parameter
/// `REPLACE INTO characters` (`Player.cpp:16253`).
/// The four text blobs are precomposed by the caller
/// in fork `ostringstream` format (space-separated
/// words with trailing spaces; `''`, never NULL).
/// `logout_time` is `time(nullptr)` sampled by the
/// caller at save start, as in C++.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterSaveRow {
    pub guid: u32,
    pub account: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u32,
    pub xp: u32,
    pub money: u32,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u32,
    pub bank_bag_slots: u32,
    pub character_flags: u32,
    pub map: u32,
    pub instance: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation: f32,
    pub transport_guid: u32,
    pub transport_x: f32,
    pub transport_y: f32,
    pub transport_z: f32,
    pub transport_o: f32,
    pub known_taxi_mask: String,
    pub current_taxi_path: String,
    pub online: u32,
    pub played_time_total: u32,
    pub played_time_level: u32,
    pub rest_bonus: f32,
    pub logout_time: u64,
    pub reset_talents_multiplier: u32,
    pub reset_talents_time: u64,
    pub extra_flags: u32,
    pub stable_slots: u32,
    pub zone: u32,
    pub death_expire_time: u64,
    pub honor_rank_points: f32,
    pub honor_highest_rank: u32,
    pub honor_standing: u32,
    pub honor_last_week_hk: u32,
    pub honor_last_week_cp: f32,
    pub honor_stored_hk: u32,
    pub honor_stored_dk: u32,
    pub watched_faction: i32,
    pub drunk: u16,
    pub health: u32,
    pub power: [u32; 5],
    pub explored_zones: String,
    pub equipment_cache: String,
    pub ammo_id: u32,
    pub action_bars: u32,
    pub world_phase_mask: u32,
    pub create_time: u64,
}

const REPLACE_CHARACTER_SQL: &str = "REPLACE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `xp`, `money`, `skin`, `face`, `hair_style`, `hair_color`, `facial_hair`, `bank_bag_slots`, `character_flags`,`map`, `instance`, `position_x`, `position_y`, `position_z`, `orientation`, `transport_guid`, `transport_x`, `transport_y`, `transport_z`, `transport_o`, `known_taxi_mask`, `current_taxi_path`, `online`, `played_time_total`, `played_time_level`, `rest_bonus`, `logout_time`, `reset_talents_multiplier`, `reset_talents_time`, `extra_flags`, `stable_slots`, `zone`, `death_expire_time`,`honor_rank_points`, `honor_highest_rank`, `honor_standing`, `honor_last_week_hk`, `honor_last_week_cp`, `honor_stored_hk`, `honor_stored_dk`, `watched_faction`, `drunk`, `health`, `power1`, `power2`, `power3`, `power4`, `power5`, `explored_zones`, `equipment_cache`, `ammo_id`, `action_bars`, `world_phase_mask`, `create_time`) VALUES ( ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn finite(v: f32) -> f32 {
    // `finiteAlways`
    if v.is_finite() { v } else { 0.0 }
}

/// The `characters` REPLACE (`Player.cpp:16253`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn replace_character(conn: &mut mysql::Conn, row: &CharacterSaveRow) -> Result<(), DbError> {
    let values: Vec<Value> = vec![
        Value::UInt(u64::from(row.guid)),
        Value::UInt(u64::from(row.account)),
        Value::Bytes(row.name.as_bytes().to_vec()),
        Value::UInt(u64::from(row.race)),
        Value::UInt(u64::from(row.class)),
        Value::UInt(u64::from(row.gender)),
        Value::UInt(u64::from(row.level)),
        Value::UInt(u64::from(row.xp)),
        Value::UInt(u64::from(row.money)),
        Value::UInt(u64::from(row.skin)),
        Value::UInt(u64::from(row.face)),
        Value::UInt(u64::from(row.hair_style)),
        Value::UInt(u64::from(row.hair_color)),
        Value::UInt(u64::from(row.facial_hair)),
        Value::UInt(u64::from(row.bank_bag_slots)),
        Value::UInt(u64::from(row.character_flags)),
        Value::UInt(u64::from(row.map)),
        Value::UInt(u64::from(row.instance)),
        Value::Float(finite(row.position_x)),
        Value::Float(finite(row.position_y)),
        Value::Float(finite(row.position_z)),
        Value::Float(finite(row.orientation)),
        Value::UInt(u64::from(row.transport_guid)),
        Value::Float(finite(row.transport_x)),
        Value::Float(finite(row.transport_y)),
        Value::Float(finite(row.transport_z)),
        Value::Float(finite(row.transport_o)),
        Value::Bytes(row.known_taxi_mask.as_bytes().to_vec()),
        Value::Bytes(row.current_taxi_path.as_bytes().to_vec()),
        Value::UInt(u64::from(row.online)),
        Value::UInt(u64::from(row.played_time_total)),
        Value::UInt(u64::from(row.played_time_level)),
        Value::Float(finite(row.rest_bonus)),
        Value::UInt(row.logout_time),
        Value::UInt(u64::from(row.reset_talents_multiplier)),
        Value::UInt(row.reset_talents_time),
        Value::UInt(u64::from(row.extra_flags)),
        Value::UInt(u64::from(row.stable_slots)),
        Value::UInt(u64::from(row.zone)),
        Value::UInt(row.death_expire_time),
        Value::Float(finite(row.honor_rank_points)),
        Value::UInt(u64::from(row.honor_highest_rank)),
        Value::UInt(u64::from(row.honor_standing)),
        Value::UInt(u64::from(row.honor_last_week_hk)),
        Value::Float(finite(row.honor_last_week_cp)),
        Value::UInt(u64::from(row.honor_stored_hk)),
        Value::UInt(u64::from(row.honor_stored_dk)),
        Value::Int(i64::from(row.watched_faction)),
        Value::UInt(u64::from(row.drunk)),
        Value::UInt(u64::from(row.health)),
        Value::UInt(u64::from(row.power[0])),
        Value::UInt(u64::from(row.power[1])),
        Value::UInt(u64::from(row.power[2])),
        Value::UInt(u64::from(row.power[3])),
        Value::UInt(u64::from(row.power[4])),
        Value::Bytes(row.explored_zones.as_bytes().to_vec()),
        Value::Bytes(row.equipment_cache.as_bytes().to_vec()),
        Value::UInt(u64::from(row.ammo_id)),
        Value::UInt(u64::from(row.action_bars)),
        Value::UInt(u64::from(row.world_phase_mask)),
        Value::UInt(row.create_time),
    ];
    conn.exec_drop(REPLACE_CHARACTER_SQL, mysql::Params::Positional(values))
        .map_err(DbError::Driver)
}

/// `SaveGoldToDB` (`Player.cpp:16445`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_gold(conn: &mut mysql::Conn, guid_low: u32, money: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `characters` SET `money` = ? WHERE `guid` = ?",
        (money, guid_low),
    )
    .map_err(DbError::Driver)
}

/// `SavePositionInDB` (`Player.cpp:16892`): printf-style
/// in C++; the bind form writes the same values
/// (transport zeroed, `current_taxi_path=''`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::too_many_arguments)]
pub fn save_position(
    conn: &mut mysql::Conn,
    guid_low: u32,
    map: u32,
    x: f32,
    y: f32,
    z: f32,
    o: f32,
    zone: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `characters` SET `position_x`=?, `position_y`=?, `position_z`=?, `orientation`=?, `map`=?, `zone`=?, `transport_x`=0, `transport_y`=0, `transport_z`=0, `transport_guid`=0, `current_taxi_path`='' WHERE `guid`=?",
        (finite(x), finite(y), finite(z), finite(o), map, zone, guid_low),
    )
    .map_err(DbError::Driver)
}

/// `_SaveAuras` (`Player.cpp:16456`): delete-all then
/// insert the caller-filtered savable rows
/// (`Player::SaveAura` runs caller-side).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_auras(conn: &mut mysql::Conn, guid_low: u32, auras: &[AuraRow]) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `character_aura` WHERE `guid` = ?", (guid_low,))
        .map_err(DbError::Driver)?;
    for aura in auras {
        conn.exec_drop(
            "INSERT INTO `character_aura` (`guid`, `caster_guid`, `item_guid`, `spell`, `stacks`, `charges`, `base_points0`, `base_points1`, `base_points2`, `periodic_time0`, `periodic_time1`, `periodic_time2`, `max_duration`, `duration`, `effect_index_mask`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            mysql::Params::Positional(vec![
                Value::UInt(u64::from(guid_low)),
                Value::UInt(aura.caster_guid),
                Value::UInt(u64::from(aura.item_guid)),
                Value::UInt(u64::from(aura.spell)),
                Value::Int(i64::from(aura.stacks)),
                Value::Int(i64::from(aura.charges)),
                Value::Float(finite(aura.base_points[0])),
                Value::Float(finite(aura.base_points[1])),
                Value::Float(finite(aura.base_points[2])),
                Value::Int(i64::from(aura.periodic_time[0])),
                Value::Int(i64::from(aura.periodic_time[1])),
                Value::Int(i64::from(aura.periodic_time[2])),
                Value::Int(i64::from(aura.max_duration)),
                Value::Int(i64::from(aura.duration)),
                Value::Int(i64::from(aura.effect_index_mask)),
            ]),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveSpellCooldowns` (`Player.cpp:4042`):
/// delete-all then insert the live non-permanent
/// cooldowns (filtering is caller-side).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_spell_cooldowns(
    conn: &mut mysql::Conn,
    guid_low: u32,
    cooldowns: &[SpellCooldownRow],
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_spell_cooldown` WHERE `guid` = ?",
        (guid_low,),
    )
    .map_err(DbError::Driver)?;
    for cd in cooldowns {
        conn.exec_drop(
            "INSERT INTO `character_spell_cooldown` (`guid`, `spell`, `spell_expire_time`, `category`, `category_expire_time`, `item_id`) VALUES( ?, ?, ?, ?, ?, ?)",
            (
                guid_low,
                cd.spell,
                cd.spell_expire_time,
                cd.category,
                cd.category_expire_time,
                cd.item_id,
            ),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveSpells` (`Player.cpp:16812`): per-spell
/// state machine reduced to caller-computed lists.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_spells(
    conn: &mut mysql::Conn,
    guid_low: u32,
    inserts: &[SpellRow],
    deletes: &[u32],
) -> Result<(), DbError> {
    for spell in deletes {
        conn.exec_drop(
            "DELETE FROM `character_spell` WHERE `guid` = ? and `spell` = ?",
            (guid_low, spell),
        )
        .map_err(DbError::Driver)?;
    }
    for row in inserts {
        conn.exec_drop(
            "INSERT INTO `character_spell` (`guid`, `spell`, `active`, `disabled`) VALUES (?, ?, ?, ?)",
            (guid_low, row.spell, row.active, row.disabled),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveSkills` (`Player.cpp:16745`), including the
/// `character_forgotten_skills` replaces.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_skills(
    conn: &mut mysql::Conn,
    guid_low: u32,
    inserts: &[SkillRow],
    updates: &[SkillRow],
    deletes: &[u32],
    forgotten: &[ForgottenSkillRow],
) -> Result<(), DbError> {
    for skill in deletes {
        conn.exec_drop(
            "DELETE FROM `character_skills` WHERE `guid` = ? AND `skill` = ?",
            (guid_low, skill),
        )
        .map_err(DbError::Driver)?;
    }
    for row in inserts {
        conn.exec_drop(
            "INSERT INTO `character_skills` (`guid`, `skill`, `value`, `max`) VALUES (?, ?, ?, ?)",
            (guid_low, row.skill, row.value, row.max),
        )
        .map_err(DbError::Driver)?;
    }
    for row in updates {
        conn.exec_drop(
            "UPDATE `character_skills` SET `value` = ?, `max` = ? WHERE `guid` = ? AND `skill` = ?",
            (row.value, row.max, guid_low, row.skill),
        )
        .map_err(DbError::Driver)?;
    }
    for row in forgotten {
        conn.exec_drop(
            "REPLACE INTO `character_forgotten_skills` (`guid`, `skill`, `value`) VALUES (?, ?, ?)",
            (guid_low, row.skill, row.value),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveQuestStatus` (`Player.cpp:16677`). Row
/// `timer` values are the *stored* form: the fork
/// writes `m_timer / IN_MILLISECONDS + GetGameTime()`
/// (an absolute expiry), so conversion from the live
/// remaining time is caller-side.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_quest_status(
    conn: &mut mysql::Conn,
    guid_low: u32,
    inserts: &[QuestStatusRow],
    updates: &[QuestStatusRow],
    deletes: &[u32],
) -> Result<(), DbError> {
    for quest in deletes {
        conn.exec_drop(
            "DELETE FROM `character_queststatus` WHERE `guid` = ? AND `quest` = ?",
            (guid_low, quest),
        )
        .map_err(DbError::Driver)?;
    }
    for row in inserts {
        conn.exec_drop(
            "INSERT INTO `character_queststatus` (`guid`, `quest`, `status`, `rewarded`, `explored`, `timer`, `mob_count1`, `mob_count2`, `mob_count3`, `mob_count4`, `item_count1`, `item_count2`, `item_count3`, `item_count4`, `reward_choice`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            mysql::Params::Positional(vec![
                Value::UInt(u64::from(guid_low)),
                Value::UInt(u64::from(row.quest)),
                Value::Int(i64::from(row.status)),
                Value::UInt(u64::from(row.rewarded)),
                Value::UInt(u64::from(row.explored)),
                Value::UInt(row.timer),
                Value::UInt(u64::from(row.mob_count[0])),
                Value::UInt(u64::from(row.mob_count[1])),
                Value::UInt(u64::from(row.mob_count[2])),
                Value::UInt(u64::from(row.mob_count[3])),
                Value::UInt(u64::from(row.item_count[0])),
                Value::UInt(u64::from(row.item_count[1])),
                Value::UInt(u64::from(row.item_count[2])),
                Value::UInt(u64::from(row.item_count[3])),
                Value::Int(i64::from(row.reward_choice)),
            ]),
        )
        .map_err(DbError::Driver)?;
    }
    for row in updates {
        conn.exec_drop(
            "UPDATE `character_queststatus` SET `status` = ?, `rewarded` = ?, `reward_choice` = ?, `explored` = ?, `timer` = ?,`mob_count1` = ?, `mob_count2` = ?, `mob_count3` = ?, `mob_count4` = ?, `item_count1` = ?, `item_count2` = ?, `item_count3` = ?, `item_count4` = ?  WHERE `guid` = ? AND `quest` = ?",
            mysql::Params::Positional(vec![
                Value::Int(i64::from(row.status)),
                Value::UInt(u64::from(row.rewarded)),
                Value::Int(i64::from(row.reward_choice)),
                Value::UInt(u64::from(row.explored)),
                Value::UInt(row.timer),
                Value::UInt(u64::from(row.mob_count[0])),
                Value::UInt(u64::from(row.mob_count[1])),
                Value::UInt(u64::from(row.mob_count[2])),
                Value::UInt(u64::from(row.mob_count[3])),
                Value::UInt(u64::from(row.item_count[0])),
                Value::UInt(u64::from(row.item_count[1])),
                Value::UInt(u64::from(row.item_count[2])),
                Value::UInt(u64::from(row.item_count[3])),
                Value::UInt(u64::from(guid_low)),
                Value::UInt(u64::from(row.quest)),
            ]),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `ReputationMgr::SaveToDB` (`ReputationMgr.cpp:448`):
/// delete+insert for each caller-dirtied faction.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_reputation(
    conn: &mut mysql::Conn,
    guid_low: u32,
    dirty: &[ReputationRow],
) -> Result<(), DbError> {
    for row in dirty {
        conn.exec_drop(
            "DELETE FROM character_reputation WHERE guid = ? AND faction=?",
            (guid_low, row.faction),
        )
        .map_err(DbError::Driver)?;
        conn.exec_drop(
            "INSERT INTO character_reputation (guid,faction,standing,flags) VALUES (?, ?, ?, ?)",
            (guid_low, row.faction, row.standing, row.flags),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `HonorMgr::Save` inserts (`HonorMgr.cpp:646`):
/// printf form in C++ (`%.1f` for cp); the bind form
/// writes the same f32 value (oracle adjudicates the
/// column's rounding).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_honor_cp(
    conn: &mut mysql::Conn,
    guid_low: u32,
    inserts: &[HonorCpRow],
) -> Result<(), DbError> {
    for row in inserts {
        conn.exec_drop(
            "INSERT INTO `character_honor_cp` (`guid`, `victim_type`, `victim_id`, `cp`, `date`, `type`) VALUES (?, ?, ?, ?, ?, ?)",
            (
                guid_low,
                row.victim_type,
                row.victim_id,
                finite(row.cp),
                row.date,
                row.kind,
            ),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `MasterPlayer::SaveActions`
/// (`MasterPlayer.cpp:382`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_actions(
    conn: &mut mysql::Conn,
    guid_low: u32,
    inserts: &[ActionRow],
    updates: &[ActionRow],
    deletes: &[u8],
) -> Result<(), DbError> {
    for row in inserts {
        conn.exec_drop(
            "INSERT INTO `character_action` (`guid`, `button`, `action`, `type`) VALUES (?, ?, ?, ?)",
            (guid_low, row.button, row.action, row.kind),
        )
        .map_err(DbError::Driver)?;
    }
    for row in updates {
        conn.exec_drop(
            "UPDATE `character_action`  SET `action` = ?, `type` = ? WHERE `guid` = ? AND `button` = ?",
            (row.action, row.kind, guid_low, row.button),
        )
        .map_err(DbError::Driver)?;
    }
    for button in deletes {
        conn.exec_drop(
            "DELETE FROM `character_action` WHERE `guid` = ? AND `button` = ?",
            (guid_low, button),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveBGData` (`Player.cpp:20696`): the delete
/// always runs when the caller reports `m_needSave`;
/// the insert only when a row is provided (fork:
/// `bgInstanceID || InBattleGroundQueue()`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_bg_data(
    conn: &mut mysql::Conn,
    guid_low: u32,
    row: Option<&BgDataRow>,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_battleground_data` WHERE `guid` = ?",
        (guid_low,),
    )
    .map_err(DbError::Driver)?;
    if let Some(row) = row {
        conn.exec_drop(
            "INSERT INTO `character_battleground_data` VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                guid_low,
                row.instance_id,
                row.team,
                finite(row.join_x),
                finite(row.join_y),
                finite(row.join_z),
                finite(row.join_o),
                row.join_map,
            ),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `_SaveStats` row (`Player.cpp:16841`).
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterStatsRow {
    pub max_health: u32,
    pub max_power: [u32; 5],
    pub strength: f32,
    pub agility: f32,
    pub stamina: f32,
    pub intellect: f32,
    pub spirit: f32,
    /// armor + 6 school resistances (holy..arcane).
    pub resistance: [i32; 7],
    pub block_chance: f32,
    pub dodge_chance: f32,
    pub parry_chance: f32,
    pub crit_chance: f32,
    pub ranged_crit_chance: f32,
    pub spell_crit_chance: f32,
    pub attack_power: u32,
    pub ranged_attack_power: u32,
    pub spell_damage: u32,
    pub spell_healing: u32,
}

/// `_SaveStats` (`Player.cpp:16841`): delete +
/// insert; gating (logout/config/min level) is
/// caller-side.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_stats(
    conn: &mut mysql::Conn,
    guid_low: u32,
    row: &CharacterStatsRow,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_stats` WHERE `guid` = ?",
        (guid_low,),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "INSERT INTO `character_stats` (`guid`, `max_health`, `max_power1`, `max_power2`, `max_power3`, `max_power4`, `max_power5`, `strength`, `agility`, `stamina`, `intellect`, `spirit`, `armor`, `holy_res`, `fire_res`, `nature_res`, `frost_res`, `shadow_res`, `arcane_res`, `block_chance`, `dodge_chance`, `parry_chance`, `crit_chance`, `ranged_crit_chance`, `spell_crit_chance`, `attack_power`, `ranged_attack_power`, `spell_damage`, `spell_healing`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        mysql::Params::Positional(vec![
            Value::UInt(u64::from(guid_low)),
            Value::UInt(u64::from(row.max_health)),
            Value::UInt(u64::from(row.max_power[0])),
            Value::UInt(u64::from(row.max_power[1])),
            Value::UInt(u64::from(row.max_power[2])),
            Value::UInt(u64::from(row.max_power[3])),
            Value::UInt(u64::from(row.max_power[4])),
            Value::Float(finite(row.strength)),
            Value::Float(finite(row.agility)),
            Value::Float(finite(row.stamina)),
            Value::Float(finite(row.intellect)),
            Value::Float(finite(row.spirit)),
            Value::Int(i64::from(row.resistance[0])),
            Value::Int(i64::from(row.resistance[1])),
            Value::Int(i64::from(row.resistance[2])),
            Value::Int(i64::from(row.resistance[3])),
            Value::Int(i64::from(row.resistance[4])),
            Value::Int(i64::from(row.resistance[5])),
            Value::Int(i64::from(row.resistance[6])),
            Value::Float(finite(row.block_chance)),
            Value::Float(finite(row.dodge_chance)),
            Value::Float(finite(row.parry_chance)),
            Value::Float(finite(row.crit_chance)),
            Value::Float(finite(row.ranged_crit_chance)),
            Value::Float(finite(row.spell_crit_chance)),
            Value::UInt(u64::from(row.attack_power)),
            Value::UInt(u64::from(row.ranged_attack_power)),
            Value::UInt(u64::from(row.spell_damage)),
            Value::UInt(u64::from(row.spell_healing)),
        ]),
    )
    .map_err(DbError::Driver)
}

/// `ItemUpdateState` (`Item.h`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemSaveState {
    Unchanged,
    New,
    Changed,
    Removed,
}

/// `ItemLootUpdateState` reduced to the write shapes
/// (`Item.cpp:313-365`): NEW inserts without the
/// delete, CHANGED deletes then inserts, REMOVED
/// deletes only.
#[derive(Clone, Debug, PartialEq)]
pub enum LootSave {
    Unchanged,
    Removed,
    Insert { gold: u32, rows: Vec<ItemLootRow> },
    Replace { gold: u32, rows: Vec<ItemLootRow> },
}

/// One queued item of `_SaveInventory`
/// (`Player.cpp:16561`): placement + instance rows,
/// the uState, and the `DeleteAllFromDB` inputs
/// (`Item.cpp:468`). `charges`/`enchantments` are
/// the composed text forms (fork recomposition from
/// live fields is caller-side; unmodeled items pass
/// their loaded strings through).
#[derive(Clone, Debug, PartialEq)]
pub struct InventorySaveItem {
    pub row: InventoryRow,
    pub state: ItemSaveState,
    /// `ITEM_DYNFLAG_WRAPPED` snapshot (gift rows).
    pub wrapped: bool,
    pub has_saved_loot: bool,
    pub loot: LootSave,
}

fn save_item_instance(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item: &InventorySaveItem,
) -> Result<(), DbError> {
    let inst = &item.row.item;
    if item.state == ItemSaveState::Changed && item.wrapped {
        conn.exec_drop(
            "UPDATE `character_gifts` SET `guid` = ? WHERE `item_guid` = ?",
            (owner_low, inst.item_guid),
        )
        .map_err(DbError::Driver)?;
    }
    let random_property = u16::from_ne_bytes(inst.random_property_id.to_ne_bytes());
    match item.state {
        ItemSaveState::New => conn.exec_drop(
            "REPLACE INTO `item_instance` (`item_id`, `owner_guid`, `creator_guid`, `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, `random_property_id`, `durability`, `text`, `generated_loot`, `guid`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            mysql::Params::Positional(vec![
                Value::UInt(u64::from(inst.item_id)),
                Value::UInt(u64::from(owner_low)),
                Value::UInt(u64::from(inst.creator_guid)),
                Value::UInt(u64::from(inst.gift_creator_guid)),
                Value::UInt(u64::from(inst.count)),
                Value::Int(i64::from(inst.duration)),
                Value::Bytes(inst.charges.as_bytes().to_vec()),
                Value::UInt(u64::from(inst.flags)),
                Value::Bytes(inst.enchantments.as_bytes().to_vec()),
                Value::UInt(u64::from(random_property)),
                Value::UInt(u64::from(inst.durability)),
                Value::UInt(u64::from(inst.text)),
                Value::UInt(u64::from(inst.generated_loot)),
                Value::UInt(u64::from(inst.item_guid)),
            ]),
        )
        .map_err(DbError::Driver)?,
        ItemSaveState::Changed => conn.exec_drop(
            "UPDATE `item_instance` SET `item_id` = ?, `owner_guid` = ?, `creator_guid` = ?, `gift_creator_guid` = ?, `count` = ?, `duration` = ?, `charges` = ?, `flags` = ?, `enchantments` = ?, `random_property_id` = ?, `durability` = ?, `text` = ?, `generated_loot` = ? WHERE `guid` = ?",
            mysql::Params::Positional(vec![
                Value::UInt(u64::from(inst.item_id)),
                Value::UInt(u64::from(owner_low)),
                Value::UInt(u64::from(inst.creator_guid)),
                Value::UInt(u64::from(inst.gift_creator_guid)),
                Value::UInt(u64::from(inst.count)),
                Value::Int(i64::from(inst.duration)),
                Value::Bytes(inst.charges.as_bytes().to_vec()),
                Value::UInt(u64::from(inst.flags)),
                Value::Bytes(inst.enchantments.as_bytes().to_vec()),
                Value::UInt(u64::from(random_property)),
                Value::UInt(u64::from(inst.durability)),
                Value::UInt(u64::from(inst.text)),
                Value::UInt(u64::from(inst.generated_loot)),
                Value::UInt(u64::from(inst.item_guid)),
            ]),
        )
        .map_err(DbError::Driver)?,
        ItemSaveState::Removed => {
            // `Item::DeleteAllFromDB` (Item.cpp:468); the
            // `character_inventory` delete already ran
            // in the placement switch.
            if inst.text != 0 {
                conn.exec_drop("DELETE FROM `item_text` WHERE `id` = ?", (inst.text,))
                    .map_err(DbError::Driver)?;
            }
            conn.exec_drop(
                "DELETE FROM `item_instance` WHERE `guid` = ?",
                (inst.item_guid,),
            )
            .map_err(DbError::Driver)?;
            if item.wrapped {
                conn.exec_drop(
                    "DELETE FROM `character_gifts` WHERE `item_guid` = ?",
                    (inst.item_guid,),
                )
                .map_err(DbError::Driver)?;
            }
            if item.has_saved_loot {
                conn.exec_drop("DELETE FROM `item_loot` WHERE `guid` = ?", (inst.item_guid,))
                    .map_err(DbError::Driver)?;
            }
            return Ok(());
        }
        ItemSaveState::Unchanged => return Ok(()),
    }
    save_item_loot(conn, owner_low, inst.item_guid, &item.loot)
}

fn save_item_loot(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_guid: u32,
    loot: &LootSave,
) -> Result<(), DbError> {
    match loot {
        LootSave::Unchanged => {}
        LootSave::Removed => {
            conn.exec_drop("DELETE FROM `item_loot` WHERE `guid` = ?", (item_guid,))
                .map_err(DbError::Driver)?;
        }
        LootSave::Insert { gold, rows } | LootSave::Replace { gold, rows } => {
            if matches!(loot, LootSave::Replace { .. }) {
                conn.exec_drop("DELETE FROM `item_loot` WHERE `guid` = ?", (item_guid,))
                    .map_err(DbError::Driver)?;
            }
            if *gold > 0 {
                conn.exec_drop(
                    "INSERT INTO `item_loot` (`guid`, `owner_guid`, `item_id`, `amount`, `property`) VALUES (?, ?, 0, ?, 0)",
                    (item_guid, owner_low, gold),
                )
                .map_err(DbError::Driver)?;
            }
            for loot_row in rows {
                conn.exec_drop(
                    "INSERT INTO `item_loot` (`guid`, `owner_guid`, `item_id`, `amount`, `property`) VALUES (?, ?, ?, ?, ?)",
                    (
                        item_guid,
                        owner_low,
                        loot_row.item_id,
                        loot_row.amount,
                        loot_row.property,
                    ),
                )
                .map_err(DbError::Driver)?;
            }
        }
    }
    Ok(())
}

/// `_SaveInventory` (`Player.cpp:16561`): buyback
/// purge first (those items re-enter as NEW in
/// `items`), then per queued item the placement
/// statement followed by `Item::SaveToDB`. The
/// anti-duplicate slot test and enchantment-duration
/// refresh are caller-side (they read live bags).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_inventory(
    conn: &mut mysql::Conn,
    guid_low: u32,
    buyback_purge: &[u32],
    items: &[InventorySaveItem],
) -> Result<(), DbError> {
    for item_guid in buyback_purge {
        conn.exec_drop(
            "DELETE FROM `character_inventory` WHERE `item_guid` = ?",
            (item_guid,),
        )
        .map_err(DbError::Driver)?;
        conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_guid,))
            .map_err(DbError::Driver)?;
    }
    for item in items {
        let inst = &item.row.item;
        match item.state {
            ItemSaveState::New => conn
                .exec_drop(
                    "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) VALUES (?, ?, ?, ?, ?)",
                    (guid_low, item.row.bag, item.row.slot, inst.item_guid, inst.item_id),
                )
                .map_err(DbError::Driver)?,
            ItemSaveState::Changed => conn
                .exec_drop(
                    "UPDATE `character_inventory` SET `guid` = ?, `bag` = ?, `slot` = ?, `item_id` = ? WHERE `item_guid` = ?",
                    (guid_low, item.row.bag, item.row.slot, inst.item_id, inst.item_guid),
                )
                .map_err(DbError::Driver)?,
            ItemSaveState::Removed => conn
                .exec_drop(
                    "DELETE FROM `character_inventory` WHERE `item_guid` = ?",
                    (inst.item_guid,),
                )
                .map_err(DbError::Driver)?,
            ItemSaveState::Unchanged => {}
        }
        save_item_instance(conn, guid_low, item)?;
    }
    Ok(())
}

/// The full `SaveToDB` bundle minus stats/pet
/// (post-commit in C++). Field grouping mirrors the
/// fork's call order at `Player.cpp:16396-16405`;
/// lists are caller-computed from the live state
/// machines. Tutorials (`SaveTutorialsData`) write
/// to the login DB and stay with the session layer,
/// as in C++.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterSaveBundle {
    pub row: CharacterSaveRow,
    /// Outer `Some` iff fork `m_bgData.m_needSave`.
    pub bg: Option<Option<BgDataRow>>,
    pub buyback_purge: Vec<u32>,
    pub inventory: Vec<InventorySaveItem>,
    pub quest_inserts: Vec<QuestStatusRow>,
    pub quest_updates: Vec<QuestStatusRow>,
    pub quest_deletes: Vec<u32>,
    pub spell_inserts: Vec<SpellRow>,
    pub spell_deletes: Vec<u32>,
    pub cooldowns: Vec<SpellCooldownRow>,
    pub auras: Vec<AuraRow>,
    pub skill_inserts: Vec<SkillRow>,
    pub skill_updates: Vec<SkillRow>,
    pub skill_deletes: Vec<u32>,
    pub forgotten_skills: Vec<ForgottenSkillRow>,
    pub reputation_dirty: Vec<ReputationRow>,
    pub honor_cp_inserts: Vec<HonorCpRow>,
}

/// `Player::SaveToDB` transactional body: REPLACE,
/// `_SaveBGData`, `_SaveInventory`,
/// `_SaveQuestStatus`, `_SaveSpells`,
/// `_SaveSpellCooldowns`, `_SaveAuras`,
/// `_SaveSkills`, reputations, honor. Execute inside
/// one DB-actor call (the fork's serial lane).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_to_db(conn: &mut mysql::Conn, bundle: &CharacterSaveBundle) -> Result<(), DbError> {
    let guid_low = bundle.row.guid;
    replace_character(conn, &bundle.row)?;
    if let Some(bg) = &bundle.bg {
        save_bg_data(conn, guid_low, bg.as_ref())?;
    }
    save_inventory(conn, guid_low, &bundle.buyback_purge, &bundle.inventory)?;
    save_quest_status(
        conn,
        guid_low,
        &bundle.quest_inserts,
        &bundle.quest_updates,
        &bundle.quest_deletes,
    )?;
    save_spells(conn, guid_low, &bundle.spell_inserts, &bundle.spell_deletes)?;
    save_spell_cooldowns(conn, guid_low, &bundle.cooldowns)?;
    save_auras(conn, guid_low, &bundle.auras)?;
    save_skills(
        conn,
        guid_low,
        &bundle.skill_inserts,
        &bundle.skill_updates,
        &bundle.skill_deletes,
        &bundle.forgotten_skills,
    )?;
    save_reputation(conn, guid_low, &bundle.reputation_dirty)?;
    save_honor_cp(conn, guid_low, &bundle.honor_cp_inserts)?;
    Ok(())
}

/// `SaveInventoryAndGoldToDB` (`Player.cpp:16425`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_inventory_and_gold(
    conn: &mut mysql::Conn,
    guid_low: u32,
    money: u32,
    buyback_purge: &[u32],
    items: &[InventorySaveItem],
) -> Result<(), DbError> {
    save_inventory(conn, guid_low, buyback_purge, items)?;
    save_gold(conn, guid_low, money)
}

/// The synthesized-homebind INSERT (`_LoadHomeBind`,
/// `Player.cpp:16010`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_homebind(
    conn: &mut mysql::Conn,
    guid_low: u32,
    homebind: &crate::character_load::HomebindRow,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `character_homebind` (`guid`, `map`, `zone`, `position_x`, `position_y`, `position_z`) VALUES (?, ?, ?, ?, ?, ?)",
        (
            guid_low,
            homebind.map,
            homebind.zone,
            finite(homebind.position_x),
            finite(homebind.position_y),
            finite(homebind.position_z),
        ),
    )
    .map_err(DbError::Driver)
}

/// The `SetHomeBind` UPDATE (`Player.cpp:20781`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_homebind(
    conn: &mut mysql::Conn,
    guid_low: u32,
    homebind: &crate::character_load::HomebindRow,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_homebind` SET `map` = ?, `zone` = ?, `position_x` = ?, `position_y` = ?, `position_z` = ? WHERE `guid` = ?",
        (
            homebind.map,
            homebind.zone,
            finite(homebind.position_x),
            finite(homebind.position_y),
            finite(homebind.position_z),
            guid_low,
        ),
    )
    .map_err(DbError::Driver)
}

// ---------------------------------------------------------------------------
// Row-diff helpers: the fork's per-entry `uState`
// machines expressed over loaded-vs-current row sets
// (the Rust aggregate tracks rows, not C++ state
// enums; the emitted statement sets are identical).

/// `_SaveSpells` diff: removed or changed spells
/// delete; new or changed non-dependent spells
/// insert. `dependent` ids never insert (fork skips
/// dependent spells on the insert arm only).
#[must_use]
pub fn diff_spells(
    loaded: &[SpellRow],
    current: &[SpellRow],
    dependent: &[u32],
) -> (Vec<SpellRow>, Vec<u32>) {
    let mut inserts = Vec::new();
    let mut deletes = Vec::new();
    for cur in current {
        match loaded.iter().find(|l| l.spell == cur.spell) {
            None => {
                if !dependent.contains(&cur.spell) {
                    inserts.push(cur.clone());
                }
            }
            Some(old) if old != cur => {
                deletes.push(cur.spell);
                if !dependent.contains(&cur.spell) {
                    inserts.push(cur.clone());
                }
            }
            Some(_) => {}
        }
    }
    for old in loaded {
        if !current.iter().any(|c| c.spell == old.spell) {
            deletes.push(old.spell);
        }
    }
    (inserts, deletes)
}

/// `_SaveSkills` diff.
#[must_use]
pub fn diff_skills(
    loaded: &[SkillRow],
    current: &[SkillRow],
) -> (Vec<SkillRow>, Vec<SkillRow>, Vec<u32>) {
    let mut inserts = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    for cur in current {
        match loaded.iter().find(|l| l.skill == cur.skill) {
            None => inserts.push(cur.clone()),
            Some(old) if old != cur => updates.push(cur.clone()),
            Some(_) => {}
        }
    }
    for old in loaded {
        if !current.iter().any(|c| c.skill == old.skill) {
            deletes.push(old.skill);
        }
    }
    (inserts, updates, deletes)
}

/// The forgotten-skills filter (`itr.second > 1`).
#[must_use]
pub fn forgotten_to_save(rows: &[ForgottenSkillRow]) -> Vec<ForgottenSkillRow> {
    rows.iter().filter(|r| r.value > 1).cloned().collect()
}

/// `_SaveQuestStatus` diff. Caller converts live
/// remaining timers to the stored absolute form in
/// *both* sets before diffing (unchanged quests
/// must not churn on timer arithmetic).
#[must_use]
pub fn diff_quests(
    loaded: &[QuestStatusRow],
    current: &[QuestStatusRow],
) -> (Vec<QuestStatusRow>, Vec<QuestStatusRow>, Vec<u32>) {
    let mut inserts = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    for cur in current {
        match loaded.iter().find(|l| l.quest == cur.quest) {
            None => inserts.push(cur.clone()),
            Some(old) if old != cur => updates.push(cur.clone()),
            Some(_) => {}
        }
    }
    for old in loaded {
        if !current.iter().any(|c| c.quest == old.quest) {
            deletes.push(old.quest);
        }
    }
    (inserts, updates, deletes)
}

/// `ReputationMgr::SaveToDB` dirty set (fork
/// `needSave`): rows new or changed vs load.
#[must_use]
pub fn diff_reputation(loaded: &[ReputationRow], current: &[ReputationRow]) -> Vec<ReputationRow> {
    current
        .iter()
        .filter(|cur| {
            loaded
                .iter()
                .find(|l| l.faction == cur.faction)
                .is_none_or(|old| old != *cur)
        })
        .cloned()
        .collect()
}

/// `MasterPlayer::SaveActions` diff over button ids.
#[must_use]
pub fn diff_actions(
    loaded: &[ActionRow],
    current: &[ActionRow],
) -> (Vec<ActionRow>, Vec<ActionRow>, Vec<u8>) {
    let mut inserts = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    for cur in current {
        match loaded.iter().find(|l| l.button == cur.button) {
            None => inserts.push(cur.clone()),
            Some(old) if old != cur => updates.push(cur.clone()),
            Some(_) => {}
        }
    }
    for old in loaded {
        if !current.iter().any(|c| c.button == old.button) {
            deletes.push(old.button);
        }
    }
    (inserts, updates, deletes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(id: u32, active: u8) -> SpellRow {
        SpellRow {
            spell: id,
            active,
            disabled: 0,
        }
    }

    #[test]
    fn spell_diff_matches_fork_states() {
        let loaded = vec![spell(1, 1), spell(2, 1), spell(3, 0)];
        let current = vec![spell(1, 1), spell(2, 0), spell(4, 1), spell(5, 1)];
        let (inserts, deletes) = diff_spells(&loaded, &current, &[5]);
        // 2 changed (delete+insert), 3 removed, 4 new,
        // 5 new-but-dependent (no insert, no delete).
        assert_eq!(deletes, vec![2, 3]);
        assert_eq!(inserts, vec![spell(2, 0), spell(4, 1)]);
    }

    #[test]
    fn skill_and_quest_diffs() {
        let loaded = vec![
            SkillRow {
                skill: 43,
                value: 100,
                max: 150,
            },
            SkillRow {
                skill: 44,
                value: 1,
                max: 75,
            },
        ];
        let current = vec![
            SkillRow {
                skill: 43,
                value: 101,
                max: 150,
            },
            SkillRow {
                skill: 45,
                value: 1,
                max: 75,
            },
        ];
        let (ins, upd, del) = diff_skills(&loaded, &current);
        assert_eq!(ins.len(), 1);
        assert_eq!(upd.len(), 1);
        assert_eq!(del, vec![44]);

        let q = |id: u32, timer: u64| QuestStatusRow {
            quest: id,
            status: 1,
            rewarded: 0,
            explored: 0,
            timer,
            mob_count: [0; 4],
            item_count: [0; 4],
            reward_choice: 0,
        };
        let (ins, upd, del) = diff_quests(&[q(1, 0), q(2, 5)], &[q(1, 0), q(2, 9), q(3, 0)]);
        assert_eq!(ins.len(), 1);
        assert_eq!(upd.len(), 1);
        assert!(del.is_empty());
    }

    #[test]
    fn forgotten_filter_and_reputation_dirty() {
        let rows = vec![
            ForgottenSkillRow {
                skill: 43,
                value: 1,
            },
            ForgottenSkillRow {
                skill: 44,
                value: 200,
            },
        ];
        assert_eq!(forgotten_to_save(&rows).len(), 1);
        let loaded = vec![ReputationRow {
            faction: 72,
            standing: 3000,
            flags: 1,
        }];
        let current = vec![
            ReputationRow {
                faction: 72,
                standing: 3000,
                flags: 1,
            },
            ReputationRow {
                faction: 47,
                standing: 100,
                flags: 1,
            },
        ];
        assert_eq!(diff_reputation(&loaded, &current).len(), 1);
    }
}
