//! DB script engine loaders (Slice 6b):
//! `ScriptMgr::LoadScripts` for `creature_ai_scripts`
//! and `generic_scripts`,
//! `CreatureEventAIMgr::LoadCreatureEventAI_Events`,
//! the `conditions` store, `ObjectMgr::LoadBroadcastTexts`
//! and `ScriptMgr::LoadScriptTexts` (enUS content only,
//! as in earlier slices). Structural C++ loader checks
//! are kept (bad event type / unknown creature template
//! skip the event; unknown condition zeroes the id on
//! events and skips the script row; unknown
//! `data_flags` bits skip the script row); the deep
//! per-command `CheckScript*` validations are not
//! replicated — the executor resolves bad references to
//! no-ops with error logs, matching the net effect.

use std::collections::HashSet;

use mangos_db::DbError;
use mangos_scripts_engine::{
    ALL_DB_SCRIPT_FLAGS, BroadcastText, BroadcastTextMap, Condition, ConditionMap, EventAiEvent,
    EventAiEventMap, MAX_ACTIONS, SF_GENERAL_SWAP_FINAL_TARGETS, SF_GENERAL_SWAP_INITIAL_TARGETS,
    ScriptInfo, ScriptMapMap, ScriptText, ScriptTextMap, event_type,
};
use mysql::prelude::Queryable;

use crate::load_report::LoadReport;

/// Load every script-engine store in dependency order
/// (conditions first — script/event rows validate
/// against them).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_script_stores<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    creature_template_ids: &HashSet<u32, S>,
    report: &mut LoadReport,
) -> Result<mangos_scripts_engine::ScriptStores, DbError> {
    let conditions = load_conditions(conn)?;
    let condition_ids: HashSet<u32> = conditions.keys().copied().collect();
    let eventai_events = load_eventai_events(conn, creature_template_ids, &condition_ids, report)?;
    let creature_ai_scripts =
        load_scripts_table(conn, "creature_ai_scripts", &condition_ids, report)?;
    let generic_scripts = load_scripts_table(conn, "generic_scripts", &condition_ids, report)?;
    let creature_movement_scripts =
        load_scripts_table(conn, "creature_movement_scripts", &condition_ids, report)?;
    let creature_spells_scripts =
        load_scripts_table(conn, "creature_spells_scripts", &condition_ids, report)?;
    let broadcast_texts = load_broadcast_texts(conn)?;
    let script_texts = load_script_texts(conn)?;
    Ok(mangos_scripts_engine::ScriptStores {
        eventai_events,
        creature_ai_scripts,
        generic_scripts,
        creature_movement_scripts,
        creature_spells_scripts,
        conditions,
        broadcast_texts,
        script_texts,
    })
}

/// `ScriptMgr::LoadScripts` for one script table.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_scripts_table<S: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    table: &str,
    condition_ids: &HashSet<u32, S>,
    report: &mut LoadReport,
) -> Result<ScriptMapMap, DbError> {
    let query = format!(
        "SELECT `id`, `delay`, `command`, `datalong`, `datalong2`, \
         `datalong3`, `datalong4`, `target_param1`, `target_param2`, \
         `target_type`, `data_flags`, `dataint`, `dataint2`, `dataint3`, \
         `dataint4`, `x`, `y`, `z`, `o`, `condition_id` \
         FROM `{table}` ORDER BY `id`, `delay`, `priority`"
    );
    let rows: Vec<mysql::Row> = conn.query(query).map_err(DbError::from)?;
    let mut out: ScriptMapMap = ScriptMapMap::new();
    for row in rows {
        let info = ScriptInfo {
            id: row.get(0).unwrap_or_default(),
            delay: row.get(1).unwrap_or_default(),
            command: row.get(2).unwrap_or_default(),
            datalong: [
                row.get(3).unwrap_or_default(),
                row.get(4).unwrap_or_default(),
                row.get(5).unwrap_or_default(),
                row.get(6).unwrap_or_default(),
            ],
            target_param1: row.get(7).unwrap_or_default(),
            target_param2: row.get(8).unwrap_or_default(),
            target_type: row.get(9).unwrap_or_default(),
            data_flags: row.get(10).unwrap_or_default(),
            dataint: [
                row.get(11).unwrap_or_default(),
                row.get(12).unwrap_or_default(),
                row.get(13).unwrap_or_default(),
                row.get(14).unwrap_or_default(),
            ],
            x: row.get(15).unwrap_or_default(),
            y: row.get(16).unwrap_or_default(),
            z: row.get(17).unwrap_or_default(),
            o: row.get(18).unwrap_or_default(),
            condition_id: row.get(19).unwrap_or_default(),
        };
        if info.condition_id != 0 && !condition_ids.contains(&info.condition_id) {
            report.error(format!(
                "Table `{table}` has condition = {} for script id {}, \
                 but this condition entry does not exist.",
                info.condition_id, info.id
            ));
            continue;
        }
        if info.data_flags != (info.data_flags & ALL_DB_SCRIPT_FLAGS) {
            report.error(format!(
                "Table `{table}` has unknown script flags (data_flags = {}) \
                 for script id {}",
                info.data_flags, info.id
            ));
            continue;
        }
        if info.target_type == 0
            && (info.data_flags & SF_GENERAL_SWAP_INITIAL_TARGETS != 0)
            && (info.data_flags & SF_GENERAL_SWAP_FINAL_TARGETS != 0)
        {
            report.error(format!(
                "Table `{table}` has nonsensical flag combination \
                 (data_flags = {}) without a buddy for script id {}",
                info.data_flags, info.id
            ));
            continue;
        }
        out.entry(info.id).or_default().push(info);
    }
    Ok(out)
}

/// `CreatureEventAIMgr::LoadCreatureEventAI_Events`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_eventai_events<S: std::hash::BuildHasher, T: std::hash::BuildHasher>(
    conn: &mut mysql::Conn,
    creature_template_ids: &HashSet<u32, S>,
    condition_ids: &HashSet<u32, T>,
    report: &mut LoadReport,
) -> Result<EventAiEventMap, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query(
            "SELECT id, creature_id, condition_id, event_type, \
             event_inverse_phase_mask, event_chance, event_flags, \
             event_param1, event_param2, event_param3, event_param4, \
             action1_script, action2_script, action3_script \
             FROM creature_ai_events",
        )
        .map_err(DbError::from)?;
    let mut out: EventAiEventMap = EventAiEventMap::new();
    for row in rows {
        let event_id: u32 = row.get(0).unwrap_or_default();
        let creature_id: u32 = row.get(1).unwrap_or_default();
        let e_type: u32 = row.get(3).unwrap_or_default();
        if e_type >= u32::from(event_type::END) {
            report.error(format!(
                "CreatureEventAI:  Event {event_id} with wrong type ({e_type}), skipping."
            ));
            continue;
        }
        if !creature_template_ids.contains(&creature_id) {
            report.error(format!(
                "CreatureEventAI:  Event {event_id} has script for non-existing \
                 creature entry ({creature_id}), skipping."
            ));
            continue;
        }
        let mut condition_id: u32 = row.get(2).unwrap_or_default();
        if condition_id != 0 && !condition_ids.contains(&condition_id) {
            report.error(format!(
                "CreatureEventAI: Creature {creature_id} has condition_id \
                 {condition_id} that does not exist in `conditions`, ignoring"
            ));
            condition_id = 0;
        }
        let mut chance: u8 = row.get(5).unwrap_or_default();
        if chance > 100 {
            chance = 100;
        }
        let mut action_scripts = [0u32; MAX_ACTIONS];
        for (i, slot) in action_scripts.iter_mut().enumerate() {
            *slot = row.get(11 + i).unwrap_or_default();
        }
        out.entry(creature_id).or_default().push(EventAiEvent {
            event_id,
            creature_id,
            condition_id,
            event_type: e_type as u16,
            inverse_phase_mask: row.get(4).unwrap_or_default(),
            chance,
            flags: row.get(6).unwrap_or_default(),
            // `event_param*` are signed columns; C++
            // `GetUInt32` wraps negatives (strtoul),
            // mirrored by the byte reinterpretation.
            params: [7, 8, 9, 10].map(|i| {
                let raw: i32 = row.get(i).unwrap_or_default();
                u32::from_ne_bytes(raw.to_ne_bytes())
            }),
            action_scripts,
        });
    }
    Ok(out)
}

/// The `conditions` store (`sConditionStorage` fill).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_conditions(conn: &mut mysql::Conn) -> Result<ConditionMap, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query(
            "SELECT condition_entry, type, value1, value2, value3, value4, flags \
             FROM conditions",
        )
        .map_err(DbError::from)?;
    let mut out: ConditionMap = ConditionMap::new();
    for row in rows {
        let entry: u32 = row.get(0).unwrap_or_default();
        out.insert(
            entry,
            Condition {
                entry,
                cond_type: row.get(1).unwrap_or_default(),
                values: [
                    row.get(2).unwrap_or_default(),
                    row.get(3).unwrap_or_default(),
                    row.get(4).unwrap_or_default(),
                    row.get(5).unwrap_or_default(),
                ],
                flags: row.get(6).unwrap_or_default(),
            },
        );
    }
    Ok(out)
}

/// `ObjectMgr::LoadBroadcastTexts` (enUS).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_broadcast_texts(conn: &mut mysql::Conn) -> Result<BroadcastTextMap, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query(
            "SELECT `entry`, `male_text`, `female_text`, `chat_type`, \
             `sound_id`, `language_id`, `emote_id1`, `emote_id2`, `emote_id3`, \
             `emote_delay1`, `emote_delay2`, `emote_delay3` FROM `broadcast_text`",
        )
        .map_err(DbError::from)?;
    let mut out: BroadcastTextMap = BroadcastTextMap::new();
    for row in rows {
        let entry: u32 = row.get(0).unwrap_or_default();
        out.insert(
            entry,
            BroadcastText {
                entry,
                male_text: row.get(1).unwrap_or_default(),
                female_text: row.get(2).unwrap_or_default(),
                chat_type: row.get(3).unwrap_or_default(),
                sound_id: row.get(4).unwrap_or_default(),
                language_id: row.get(5).unwrap_or_default(),
                emote_ids: [
                    row.get(6).unwrap_or_default(),
                    row.get(7).unwrap_or_default(),
                    row.get(8).unwrap_or_default(),
                ],
                emote_delays: [
                    row.get(9).unwrap_or_default(),
                    row.get(10).unwrap_or_default(),
                    row.get(11).unwrap_or_default(),
                ],
            },
        );
    }
    Ok(out)
}

/// `ScriptMgr::LoadScriptTexts` (negative entries,
/// enUS `content_default`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_script_texts(conn: &mut mysql::Conn) -> Result<ScriptTextMap, DbError> {
    let rows: Vec<mysql::Row> = conn
        .query(
            "SELECT `entry`, `content_default`, `sound`, `type`, `language`, \
             `emote` FROM `script_texts`",
        )
        .map_err(DbError::from)?;
    let mut out: ScriptTextMap = ScriptTextMap::new();
    for row in rows {
        let entry: i32 = row.get(0).unwrap_or_default();
        if entry >= 0 {
            continue;
        }
        out.insert(
            entry,
            ScriptText {
                entry,
                content: row.get(1).unwrap_or_default(),
                sound_id: row.get(2).unwrap_or_default(),
                chat_type: row.get(3).unwrap_or_default(),
                language: row.get(4).unwrap_or_default(),
                emote: row.get(5).unwrap_or_default(),
            },
        );
    }
    Ok(out)
}
