//! Game-event data (`GameEventMgr::LoadFromDB`,
//! `GameEventMgr.cpp:164-666`, Tier 4 B0-8):
//! event definitions, spawn guid lists, creature
//! data overrides, event quests and mails, plus
//! the pure scheduling math (`CheckOneGameEvent`,
//! `NextCheck`). Runtime state (active set,
//! timers, apply/unapply) lives in the world
//! actor; spawn gating in `mangos-map`.
//!
//! UNMODELED here: pool registration
//! (`mGameEventSpawnPoolIds` is loaded but the
//! Rust pool manager does not exist yet, so pool
//! spawns stay unspawned exactly as before this
//! slice), hardcoded event implementations
//! (framework dispatch records them in the
//! actor).

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_db::row::Fields;

use crate::load_report::LoadReport;

/// One `GameEventData` row (`game_event`).
#[derive(Clone, Debug, PartialEq)]
pub struct GameEventData {
    pub start: u64,
    pub end: u64,
    /// Occurence in minutes (raw DB value).
    pub occurence_min: u64,
    /// Length in minutes; 0 = invalid (fork skips
    /// the row at load, leaving the default).
    pub length_min: u64,
    pub holiday: u32,
    pub description: String,
    pub disabled: bool,
    pub hardcoded: bool,
    /// Leap days between start and load year
    /// (yearly events only).
    pub leap_days: u8,
}

impl Default for GameEventData {
    /// Fork ctor: `start(1)`, rest zero.
    fn default() -> Self {
        Self {
            start: 1,
            end: 0,
            occurence_min: 0,
            length_min: 0,
            holiday: 0,
            description: String::new(),
            disabled: false,
            hardcoded: false,
            leap_days: 0,
        }
    }
}

impl GameEventData {
    /// `isValid()`: length > 0.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.length_min > 0
    }
}

/// One `game_event_creature_data` override.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameEventCreatureData {
    pub entry: u32,
    pub display: u32,
    pub equipment: i32,
    pub spell_start: u32,
    pub spell_end: u32,
}

/// One `game_event_mail` row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameEventMail {
    pub race_mask: u32,
    pub quest: u32,
    pub mail_template: u32,
    pub sender_entry: u32,
}

/// Loaded game-event tables.
#[derive(Clone, Debug, Default)]
pub struct GameEvents {
    /// Indexed by event entry (sparse holes hold
    /// a default with `occurence_min == 0`).
    events: Vec<GameEventData>,
    creature: HashMap<i16, Vec<u32>>,
    gameobject: HashMap<i16, Vec<u32>>,
    creature_data: HashMap<u16, Vec<(u32, GameEventCreatureData)>>,
    creature_data_by_guid: HashMap<u32, Vec<u16>>,
    quests: HashMap<u16, Vec<u32>>,
    quest_ids: HashSet<u32>,
    mails: HashMap<i16, Vec<GameEventMail>>,
    spawn_pool_ids: HashMap<u16, Vec<u16>>,
}

impl GameEvents {
    /// Event definition by entry; none when the
    /// row does not exist or is disabled-absent
    /// (fork keeps disabled entries loaded but
    /// unstartable — `disabled` distinguishes).
    #[must_use]
    pub fn get(&self, entry: u16) -> Option<&GameEventData> {
        self.events.get(usize::from(entry)).filter(|e| e.is_valid())
    }

    /// Loaded event entries in ascending order.
    pub fn entries(&self) -> impl Iterator<Item = (u16, &GameEventData)> {
        self.events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_valid())
            .map(|(i, e)| {
                #[allow(clippy::cast_possible_truncation)]
                (i as u16, e)
            })
    }

    #[must_use]
    pub fn creature_guids(&self, event: i16) -> &[u32] {
        self.creature.get(&event).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    /// `mGameEventCreatureData[event]` rows.
    pub fn creature_data_for(&self, event: u16) -> &[(u32, GameEventCreatureData)] {
        self.creature_data.get(&event).map_or(&[], |v| v)
    }

    /// The row for one guid under one event
    /// (`GetGameEventDataForCreature`).
    #[must_use]
    pub fn creature_data_entry(&self, event: u16, guid: u32) -> Option<&GameEventCreatureData> {
        self.creature_data_for(event)
            .iter()
            .find(|(g, _)| *g == guid)
            .map(|(_, d)| d)
    }

    pub fn gameobject_guids(&self, event: i16) -> &[u32] {
        self.gameobject.get(&event).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn creature_data(&self, event: u16) -> &[(u32, GameEventCreatureData)] {
        self.creature_data.get(&event).map_or(&[], Vec::as_slice)
    }

    /// Active-event data override for one
    /// creature guid (`GetCreatureUpdateDataFor
    /// ActiveEvent`): first registered active
    /// event wins, fork iteration order.
    #[must_use]
    pub fn creature_data_for_active(
        &self,
        guid_low: u32,
        active: &HashSet<u16>,
    ) -> Option<GameEventCreatureData> {
        let events = self.creature_data_by_guid.get(&guid_low)?;
        for event in events {
            if active.contains(event) {
                return self
                    .creature_data
                    .get(event)?
                    .iter()
                    .find(|(g, _)| *g == guid_low)
                    .map(|(_, d)| *d);
            }
        }
        None
    }

    #[must_use]
    pub fn quests(&self, event: u16) -> &[u32] {
        self.quests.get(&event).map_or(&[], Vec::as_slice)
    }

    /// Whether `quest` is takeable with this active
    /// set: non-event quests always; event quests
    /// only while their event runs (fork removes
    /// them from quest relations otherwise,
    /// `UpdateEventQuests`).
    #[must_use]
    pub fn quest_takeable(&self, quest: u32, active: &std::collections::HashSet<u16>) -> bool {
        if !self.quest_ids.contains(&quest) {
            return true;
        }
        self.quests
            .iter()
            .any(|(event, quests)| active.contains(event) && quests.contains(&quest))
    }

    /// All quests tied to any event (fork marks
    /// them inactive until their event runs).
    #[must_use]
    pub fn quest_ids(&self) -> &HashSet<u32> {
        &self.quest_ids
    }

    #[must_use]
    pub fn mails(&self, event: i16) -> &[GameEventMail] {
        self.mails.get(&event).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn spawn_pool_ids(&self, event: u16) -> &[u16] {
        self.spawn_pool_ids.get(&event).map_or(&[], Vec::as_slice)
    }

    /// Test-fixture constructor.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_fixtures_for_tests(
        events: Vec<(u16, GameEventData)>,
        creature: Vec<(i16, Vec<u32>)>,
        gameobject: Vec<(i16, Vec<u32>)>,
    ) -> Self {
        let max = events
            .iter()
            .map(|(e, _)| usize::from(*e))
            .max()
            .unwrap_or(0);
        let mut vec = vec![GameEventData::default(); max + 1];
        for (entry, data) in events {
            vec[usize::from(entry)] = data;
        }
        Self {
            events: vec,
            creature: creature.into_iter().collect(),
            gameobject: gameobject.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// `CheckOneGameEvent` (`GameEventMgr.cpp:39`):
/// in-window test with the leap-day offset.
#[must_use]
pub fn is_event_active_at(data: &GameEventData, now: u64) -> bool {
    if data.occurence_min == 0 {
        return false;
    }
    data.start <= now
        && now < data.end
        && (now.wrapping_sub(data.start + u64::from(data.leap_days) * 86_400)
            % (data.occurence_min * 60))
            < data.length_min * 60
}

/// `NextCheck(entry)` (`GameEventMgr.cpp:46`).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn next_check_for(data: &GameEventData, now: u64) -> u64 {
    if now > data.end {
        return 86_400;
    }
    if data.start > now {
        return data.start - now;
    }
    if data.occurence_min == 0 {
        return 86_400;
    }
    let elapsed = (now.wrapping_sub(data.start + u64::from(data.leap_days) * 86_400))
        % (data.occurence_min * 60);
    let delay = if elapsed < data.length_min * 60 {
        data.length_min * 60 - elapsed
    } else {
        data.occurence_min * 60 - elapsed
    };
    if data.end < now + delay {
        return data.end - now;
    }
    delay
}

/// `GameEventMgr::Update` next-delay fold over
/// non-hardcoded, enabled events (hardcoded
/// delays are the actor's; fork default 1 day).
#[must_use]
pub fn next_check_delay(events: &GameEvents, now: u64) -> u64 {
    let mut delay = 86_400u64;
    for (_, data) in events.entries() {
        if data.disabled || data.hardcoded {
            continue;
        }
        let d = next_check_for(data, now);
        if d < delay {
            delay = d;
        }
    }
    delay
}

/// UTC year of a unix timestamp (civil math).
fn year_of(unix: u64) -> i64 {
    let days = i64::try_from(unix / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    if mp >= 10 { y + 1 } else { y }
}

fn is_leap_year(year: i64) -> bool {
    year % 400 == 0 || (year % 100 != 0 && year % 4 == 0)
}

/// `LoadFromDB` body. Existence screens use the
/// spawn/quest/template presence probes (fork
/// logs + skips the same rows).
///
/// # Errors
///
/// [`DbError`] on connection failures.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
pub fn load_game_events(
    conn: &mut mysql::Conn,
    now_unix: u64,
    wow_patch: u32,
    creature_spawn_exists: &dyn Fn(u32) -> bool,
    gameobject_spawn_exists: &dyn Fn(u32) -> bool,
    quest_exists: &dyn Fn(u32) -> bool,
    mail_template_exists: &dyn Fn(u32) -> bool,
    creature_template_exists: &dyn Fn(u32) -> bool,
) -> Result<(GameEvents, LoadReport), DbError> {
    let mut report = LoadReport::default();
    let mut out = GameEvents::default();

    // ---- game_event ----
    // `SELECT MAX(entry)` sizing happens implicitly
    // via resize-on-insert below.
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, UNIX_TIMESTAMP(`start_time`), UNIX_TIMESTAMP(`end_time`), \
         `occurence`, `length`, `holiday`, `description`, `hardcoded`, `disabled`, \
         `patch_min`, `patch_max` FROM `game_event`",
        (),
    )?;
    for row in rows {
        let f = Fields::from_row(row);
        let entry = f.u32(0);
        if entry == 0 {
            report.error("`game_event` game event id 0 is reserved, skipped.".to_string());
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        let entry16 = entry as u16;
        if usize::from(entry16) >= out.events.len() {
            out.events
                .resize(usize::from(entry16) + 1, GameEventData::default());
        }
        let length = f.u64(4);
        if length == 0 {
            report.error(format!(
                "`game_event` game event id {entry} has length 0, skipped."
            ));
            continue;
        }
        let (mut patch_min, mut patch_max) = (u32::from(f.u8(9)), u32::from(f.u8(10)));
        if patch_min > patch_max || patch_max > 10 {
            report.error(format!(
                "`game_event` game event id {entry} has invalid patch_min={patch_min}, \
                 patch_max={patch_max}."
            ));
            patch_min = 0;
            patch_max = 10;
        }
        let mut disabled = f.u8(8) != 0;
        if !(wow_patch >= patch_min && wow_patch <= patch_max) {
            disabled = true;
        }
        let start = f.u64(1);
        let occurence = f.u64(3);
        let mut leap_days = 0u8;
        if occurence == 525_600 && length < 525_600 {
            let start_year = year_of(start);
            let current_year = year_of(now_unix);
            if current_year > start_year {
                let mut days = 0i64;
                for y in start_year..current_year {
                    if is_leap_year(y) {
                        days += 1;
                    }
                }
                leap_days = u8::try_from(days).unwrap_or(u8::MAX);
            }
        }
        out.events[usize::from(entry16)] = GameEventData {
            start,
            end: f.u64(2),
            occurence_min: occurence,
            length_min: length,
            holiday: f.u32(5),
            description: f.string(6),
            hardcoded: f.u8(7) != 0,
            disabled,
            leap_days,
        };
    }

    // ---- game_event_creature / gameobject ----
    load_event_guids(
        conn,
        "SELECT `guid`, `event` FROM `game_event_creature`",
        creature_spawn_exists,
        "creature",
        &mut out.creature,
        &mut report,
    )?;
    load_event_guids(
        conn,
        "SELECT `guid`, `event` FROM `game_event_gameobject`",
        gameobject_spawn_exists,
        "gameobject",
        &mut out.gameobject,
        &mut report,
    )?;

    // ---- game_event_creature_data ----
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `event`, `display_id`, `equipment_id`, `entry_id`, \
         `spell_start`, `spell_end` FROM `game_event_creature_data` t1 \
         WHERE `patch`=(SELECT max(`patch`) FROM `game_event_creature_data` t2 \
         WHERE t1.`guid`=t2.`guid` && t1.`event`=t2.`event` && `patch` <= ?)",
        (wow_patch,),
    )?;
    for row in rows {
        let f = Fields::from_row(row);
        let guid = f.u32(0);
        #[allow(clippy::cast_possible_truncation)]
        let event = f.u32(1) as u16;
        if !creature_spawn_exists(guid) {
            report.error(format!(
                "`game_event_creature_data` guid {guid} not found in `creature` table, skipped."
            ));
            continue;
        }
        if out.get(event).is_none() {
            report.error(format!(
                "`game_event_creature_data` event {event} not found, skipped."
            ));
            continue;
        }
        let data = GameEventCreatureData {
            display: f.u32(2),
            equipment: f.i32(3),
            entry: f.u32(4),
            spell_start: f.u32(5),
            spell_end: f.u32(6),
        };
        if data.entry != 0 && !creature_template_exists(data.entry) {
            report.error(format!(
                "`game_event_creature_data` entry {} not found in creature_template, skipped.",
                data.entry
            ));
            continue;
        }
        out.creature_data
            .entry(event)
            .or_default()
            .push((guid, data));
        out.creature_data_by_guid
            .entry(guid)
            .or_default()
            .push(event);
    }

    // ---- game_event_quest ----
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT quest, event FROM game_event_quest WHERE patch_min <= ?",
        (wow_patch,),
    )?;
    for row in rows {
        let f = Fields::from_row(row);
        let quest = f.u32(0);
        #[allow(clippy::cast_possible_truncation)]
        let event = f.u32(1) as u16;
        if out.get(event).is_none() {
            report.error(format!(
                "`game_event_quest` event {event} not found, skipped."
            ));
            continue;
        }
        if !quest_exists(quest) {
            report.error(format!(
                "`game_event_quest` quest {quest} (event {event}) does not exist, skipped."
            ));
            continue;
        }
        out.quests.entry(event).or_default().push(quest);
        out.quest_ids.insert(quest);
    }

    // ---- game_event_mail ----
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `event`, `raceMask`, `quest`, `mailTemplateId`, `senderEntry` \
         FROM `game_event_mail`",
        (),
    )?;
    for row in rows {
        let f = Fields::from_row(row);
        #[allow(clippy::cast_possible_truncation)]
        let event = f.i32(0) as i16;
        #[allow(clippy::cast_possible_truncation)]
        let event_u = event.unsigned_abs();
        if out.get(event_u).is_none() {
            continue;
        }
        let mail = GameEventMail {
            race_mask: f.u32(1),
            quest: f.u32(2),
            mail_template: f.u32(3),
            sender_entry: f.u32(4),
        };
        if !mail_template_exists(mail.mail_template) {
            continue;
        }
        out.mails.entry(event).or_default().push(mail);
    }

    Ok((out, report))
}

fn load_event_guids(
    conn: &mut mysql::Conn,
    sql: &str,
    exists: &dyn Fn(u32) -> bool,
    kind: &str,
    into: &mut HashMap<i16, Vec<u32>>,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> = conn.exec(sql, ())?;
    for row in rows {
        let f = Fields::from_row(row);
        let guid = f.u32(0);
        #[allow(clippy::cast_possible_truncation)]
        let event = f.i32(1) as i16;
        if !exists(guid) {
            report.error(format!(
                "`game_event_{kind}` guid {guid} not found in `{kind}` table, skipped."
            ));
            continue;
        }
        into.entry(event).or_default().push(guid);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn ev(start: u64, end: u64, occ_min: u64, len_min: u64) -> GameEventData {
        GameEventData {
            start,
            end,
            occurence_min: occ_min,
            length_min: len_min,
            description: "x".into(),
            ..GameEventData::default()
        }
    }

    #[test]
    fn check_windows() {
        // Weekly 2h event starting at t=10000, no end
        // bound issue: end far future required (fork
        // CheckOne needs now < end).
        let d = ev(10_000, u64::MAX, 10_080, 120);
        assert!(!is_event_active_at(&d, 9_999));
        assert!(is_event_active_at(&d, 10_000));
        assert!(is_event_active_at(&d, 17_199));
        assert!(!is_event_active_at(&d, 17_200));
        // NextCheck: before start -> start-now.
        assert_eq!(next_check_for(&d, 9_000), 1_000);
        // In window at start -> full length.
        assert_eq!(next_check_for(&d, 10_000), 7_200);
        // After window -> occurence - elapsed.
        assert_eq!(next_check_for(&d, 17_201), 10_080 * 60 - 7_201);
        // Outdated -> 1 day.
        let d2 = ev(10_000, 20_000, 60, 30);
        assert_eq!(next_check_for(&d2, 20_001), 86_400);
        // End clamp: delay shortened to end.
        let d3 = ev(10_000, 10_500, 10_080, 120);
        assert_eq!(next_check_for(&d3, 10_000), 500);
    }
}
