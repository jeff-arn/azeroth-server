//! Player create/level info: `LoadPlayerInfo` minus the
//! create-item/action sub-loaders (those feed character
//! creation, not login), `ObjectMgr.cpp:4475-5060`. The
//! create-spell sub-loader is included because
//! `Player::LearnDefaultSpells` also runs at login load.
//!
//! Covers `playercreateinfo` (home position + display ids from
//! `ChrRaces`), `player_classlevelstats` (base health/mana per
//! class/level), `player_levelstats` (base stats per
//! race/class/level), and `player_xp_for_level`. The C++ exit(1)
//! integrity failures become [`LoadReport`] errors,
//! as with the CLS loader; gap filling copies the previous
//! level exactly like C++.
//!
//! Race/class validity and display ids come from the DBCs;
//! the caller supplies them via [`RaceClassDbc`] (built from
//! `ChrRaces.dbc`/`ChrClasses.dbc` produced rows). Without it,
//! rows load unchecked and display ids stay 0 — which also
//! makes [`PlayerInfos::level_info`] return `None`, matching
//! the C++ `displayId == 0` guard.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::load_report::LoadReport;

/// `MAX_RACES` (`SharedDefines.h:70`): races are `1..MAX_RACES`.
pub const MAX_RACES: u32 = 9;
/// `MAX_CLASSES` (`SharedDefines.h:96`).
pub const MAX_CLASSES: u32 = 12;
/// `PLAYER_STRONG_MAX_LEVEL` (`DBCEnums.h:38`).
const PLAYER_STRONG_MAX_LEVEL: u32 = 255;
/// `MAX_STATS`.
pub const MAX_STATS: usize = 5;

/// `RACEMASK_ALL_PLAYABLE` bit test (`SharedDefines.h:72`):
/// races 1-8.
#[must_use]
pub fn race_playable(race: u32) -> bool {
    (1..=8).contains(&race)
}

/// `CLASSMASK_ALL_PLAYABLE` bit test (`SharedDefines.h:98`).
#[must_use]
pub fn class_playable(class: u32) -> bool {
    matches!(class, 1 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 11)
}

/// The ChrRaces/ChrClasses fields the loaders consume.
#[derive(Clone, Debug, Default)]
pub struct RaceClassDbc {
    /// race id -> (`FactionID`, `model_m`, `model_f`, `TeamID`)
    /// (`DBCStructure.h:143-166`, produced fields 2,3,4,5 of
    /// fmt `niixiixxiiiiixixissssssssxxxx`).
    pub races: HashMap<u32, RaceDbc>,
    /// Class ids present in `ChrClasses.dbc`.
    pub classes: HashSet<u32>,
    /// Class id -> `powerType` (`ChrClassesEntry`, produced
    /// field 1 of fmt `nxxixssssssssxxix`).
    pub class_power: HashMap<u32, u32>,
}

/// One `ChrRacesEntry` subset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RaceDbc {
    pub faction_id: u32,
    pub model_m: u32,
    pub model_f: u32,
    /// `TeamID`: 7 = Alliance, 1 = Horde.
    pub team_id: u32,
}

/// `PlayerInfo` position/display half (`Player.h` create info).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerCreateInfo {
    pub map_id: u32,
    pub area_id: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation: f32,
    pub display_id_m: u32,
    pub display_id_f: u32,
}

/// `struct PlayerClassLevelInfo` (`Player.h:178-182`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlayerClassLevelInfo {
    pub basehealth: u16,
    pub basemana: u16,
}

/// `struct PlayerLevelInfo` (`Player.h:189-192`): str, agi,
/// sta, int, spi.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlayerLevelInfo {
    pub stats: [u8; MAX_STATS],
}

/// All `LoadPlayerInfo` products used at login.
#[derive(Clone, Debug, Default)]
pub struct PlayerInfos {
    max_player_level: u32,
    create: HashMap<(u32, u32), PlayerCreateInfo>,
    create_spells: HashMap<(u32, u32), Vec<u32>>,
    class_level: HashMap<u32, Vec<PlayerClassLevelInfo>>,
    level_stats: HashMap<(u32, u32), Vec<PlayerLevelInfo>>,
    xp_per_level: Vec<u32>,
}

impl PlayerInfos {
    /// Test constructor: assembles stores from literal rows
    /// (used by socket-level tests with fake worlds).
    #[doc(hidden)]
    #[must_use]
    pub fn from_maps_for_tests(
        max_player_level: u32,
        create: HashMap<(u32, u32), PlayerCreateInfo>,
        class_level: HashMap<u32, Vec<PlayerClassLevelInfo>>,
        level_stats: HashMap<(u32, u32), Vec<PlayerLevelInfo>>,
        xp_per_level: Vec<u32>,
    ) -> PlayerInfos {
        PlayerInfos {
            max_player_level,
            create,
            create_spells: HashMap::new(),
            class_level,
            level_stats,
            xp_per_level,
        }
    }

    #[must_use]
    pub fn max_player_level(&self) -> u32 {
        self.max_player_level
    }

    /// Number of `(race, class)` create definitions.
    #[must_use]
    pub fn create_count(&self) -> usize {
        self.create.len()
    }

    #[must_use]
    pub fn create_info(&self, race: u32, class: u32) -> Option<&PlayerCreateInfo> {
        self.create.get(&(race, class))
    }

    /// `PlayerInfo::spell` for the race/class pair:
    /// `playercreateinfo_spell` rows at build 5875, in
    /// load order. Empty when the pair has no defaults.
    #[must_use]
    pub fn default_spells(&self, race: u32, class: u32) -> &[u32] {
        self.create_spells
            .get(&(race, class))
            .map_or(&[], Vec::as_slice)
    }

    /// `GetPlayerClassLevelInfo`: `None` for level 0 or class
    /// out of range (C++ leaves the out-param untouched);
    /// level clamps to `MaxPlayerLevel`.
    #[must_use]
    pub fn class_level_info(&self, class: u32, level: u32) -> Option<PlayerClassLevelInfo> {
        if level < 1 || class >= MAX_CLASSES {
            return None;
        }
        let table = self.class_level.get(&class)?;
        let level = level.min(self.max_player_level);
        table.get((level - 1) as usize).copied()
    }

    /// `GetPlayerLevelInfo`: `None` when the pair has no
    /// create info (C++ checks display ids); above
    /// `MaxPlayerLevel` the per-class growth table applies
    /// (`BuildPlayerLevelInfo`).
    #[must_use]
    pub fn level_info(&self, race: u32, class: u32, level: u32) -> Option<PlayerLevelInfo> {
        if level < 1 || race >= MAX_RACES || class >= MAX_CLASSES {
            return None;
        }
        let create = self.create.get(&(race, class))?;
        if create.display_id_m == 0 || create.display_id_f == 0 {
            return None;
        }
        let table = self.level_stats.get(&(race, class))?;
        if level <= self.max_player_level {
            table.get((level - 1) as usize).copied()
        } else {
            let mut info = *table.get((self.max_player_level - 1) as usize)?;
            build_level_info_above(&mut info, class, self.max_player_level, level);
            Some(info)
        }
    }

    /// `GetXPForLevel` (`ObjectMgr.cpp:8132-8138`): clamps to
    /// the last index; 0 when the table is empty.
    #[must_use]
    pub fn xp_for_level(&self, level: u32) -> u32 {
        if self.xp_per_level.is_empty() {
            return 0;
        }
        let idx = (level as usize).min(self.xp_per_level.len() - 1);
        self.xp_per_level[idx]
    }
}

/// `BuildPlayerLevelInfo` (`ObjectMgr.cpp:5058-5136`):
/// stats at `MaxPlayerLevel` grown per class rules.
fn build_level_info_above(info: &mut PlayerLevelInfo, class: u32, max_level: u32, level: u32) {
    for lvl in (max_level - 1)..level {
        let gain = growth_at(class, lvl);
        for (stat, add) in info.stats.iter_mut().zip(gain) {
            *stat = stat.wrapping_add(add);
        }
    }
}

/// One `lvl` iteration of the growth switch, transcribed
/// exactly (increments on `[str, agi, sta, int, spi]`, the
/// `enum Stats` order). The C++ shapes repeat: `lvl > a ? x :
/// (lvl > b ? y : 0)` and parity gates.
fn growth_at(class: u32, lvl: u32) -> [u8; MAX_STATS] {
    let odd = lvl % 2 == 1;
    let step = |a: u32, x: u8, b: u32, y: u8| {
        if lvl > a {
            x
        } else if lvl > b {
            y
        } else {
            0
        }
    };
    let gt = |a: u32| u8::from(lvl > a);
    let odd_gt = |a: u32| u8::from(lvl > a && odd);
    let even_gt = |a: u32| u8::from(lvl > a && !odd);
    let cap_or = |cap: u32, v: u8, a: u32, odd_parity: bool| {
        if lvl > cap {
            v
        } else if odd_parity {
            odd_gt(a)
        } else {
            even_gt(a)
        }
    };
    match class {
        1 => [
            // warrior
            step(23, 2, 1, 1),
            cap_or(36, 1, 6, true),
            step(23, 2, 1, 1),
            even_gt(9),
            even_gt(9),
        ],
        2 => [
            // paladin
            gt(3),
            cap_or(38, 1, 7, false),
            step(33, 2, 1, 1),
            odd_gt(6),
            gt(7),
        ],
        3 => [
            // hunter
            gt(4),
            step(33, 2, 1, 1),
            gt(4),
            odd_gt(8),
            cap_or(38, 1, 9, false),
        ],
        4 => [
            // rogue
            gt(5),
            step(16, 2, 1, 1),
            gt(4),
            even_gt(8),
            cap_or(38, 1, 9, false),
        ],
        5 => [
            // priest
            even_gt(9),
            cap_or(38, 1, 8, true),
            gt(5),
            step(22, 2, 1, 1),
            gt(3),
        ],
        7 => [
            // shaman
            cap_or(34, 1, 6, true),
            even_gt(7),
            gt(4),
            gt(5),
            gt(4),
        ],
        8 => [
            // mage
            even_gt(9),
            even_gt(9),
            gt(5),
            step(24, 2, 1, 1),
            step(33, 2, 2, 1),
        ],
        9 => [
            // warlock
            even_gt(9),
            even_gt(9),
            step(38, 2, 3, 1),
            step(33, 2, 2, 1),
            step(38, 2, 3, 1),
        ],
        11 => [
            // druid
            cap_or(38, 2, 6, true),
            cap_or(38, 2, 8, true),
            step(32, 2, 4, 1),
            step(38, 3, 4, 1),
            step(38, 3, 5, 1),
        ],
        _ => [0; MAX_STATS],
    }
}

/// Loads everything in [`PlayerInfos`]. `max_player_level` is
/// the `MaxPlayerLevel` config (default 60). `instanceable_maps`
/// gates the create-info home-position check.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)] // default-hashers sets like the other loaders
pub fn load_player_infos(
    conn: &mut mysql::Conn,
    max_player_level: u32,
    dbc: Option<&RaceClassDbc>,
    map_ids: Option<&HashSet<u32>>,
    instanceable_maps: Option<&HashSet<u32>>,
) -> Result<(PlayerInfos, LoadReport), DbError> {
    let mut report = LoadReport::default();
    if dbc.is_none() {
        report.error(
            "player info load: ChrRaces/ChrClasses validation skipped (no DBC data supplied)"
                .to_string(),
        );
    }
    let mut infos = PlayerInfos {
        max_player_level,
        xp_per_level: vec![0; (max_player_level + 1) as usize],
        ..PlayerInfos::default()
    };
    load_create_info(
        conn,
        dbc,
        map_ids,
        instanceable_maps,
        &mut infos,
        &mut report,
    )?;
    load_create_spells(conn, dbc, &mut infos, &mut report)?;
    load_class_level_stats(conn, max_player_level, dbc, &mut infos, &mut report)?;
    load_level_stats(conn, max_player_level, dbc, &mut infos, &mut report)?;
    load_xp_for_level(conn, max_player_level, &mut infos, &mut report)?;
    Ok((infos, report))
}

fn race_ok(dbc: Option<&RaceClassDbc>, race: u32) -> bool {
    race_playable(race) && dbc.is_none_or(|d| d.races.contains_key(&race))
}

fn class_ok(dbc: Option<&RaceClassDbc>, class: u32) -> bool {
    class_playable(class) && dbc.is_none_or(|d| d.classes.contains(&class))
}

fn load_create_info(
    conn: &mut mysql::Conn,
    dbc: Option<&RaceClassDbc>,
    map_ids: Option<&HashSet<u32>>,
    instanceable_maps: Option<&HashSet<u32>>,
    infos: &mut PlayerInfos,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `race`, `class`, `map`, `zone`, `position_x`, `position_y`, `position_z`, \
         `orientation` FROM `playercreateinfo`",
    )?;
    if rows.is_empty() {
        report.error("Error loading `playercreateinfo` table or empty table.".to_string());
    }
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (race, class, map_id) = (f.u32(0), f.u32(1), f.u32(2));
        if !race_ok(dbc, race) {
            report.error(format!(
                "Wrong race {race} in `playercreateinfo` table, ignoring."
            ));
            continue;
        }
        if !class_ok(dbc, class) {
            report.error(format!(
                "Wrong class {class} in `playercreateinfo` table, ignoring."
            ));
            continue;
        }
        let (pos_x, pos_y, pos_z, orient) = (f.f32(4), f.f32(5), f.f32(6), f.f32(7));
        let map_known = map_ids.is_none_or(|m| m.contains(&map_id));
        if !map_known
            || !mangos_movement::movement_info::is_valid_map_coord(pos_x, pos_y, pos_z, orient)
        {
            report.error(format!(
                "Wrong home position for class {class} race {race} pair in `playercreateinfo` table, ignoring."
            ));
            continue;
        }
        if instanceable_maps.is_some_and(|m| m.contains(&map_id)) {
            report.error(format!(
                "Home position in instanceable map for class {class} race {race} pair in `playercreateinfo` table, ignoring."
            ));
            continue;
        }
        let (display_id_m, display_id_f) = dbc
            .and_then(|d| d.races.get(&race))
            .map_or((0, 0), |r| (r.model_m, r.model_f));
        infos.create.insert(
            (race, class),
            PlayerCreateInfo {
                map_id,
                area_id: f.u32(3),
                position_x: pos_x,
                position_y: pos_y,
                position_z: pos_z,
                orientation: orient,
                display_id_m,
                display_id_f,
            },
        );
    }
    Ok(())
}

fn load_create_spells(
    conn: &mut mysql::Conn,
    dbc: Option<&RaceClassDbc>,
    infos: &mut PlayerInfos,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `race`, `class`, `spell` FROM `playercreateinfo_spell` \
         WHERE ? BETWEEN `build_min` AND `build_max`",
        (5875u32,),
    )?;
    if rows.is_empty() {
        report.error("Error loading `playercreateinfo_spell` table or empty table.".to_string());
    }
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (race, class, spell_id) = (f.u32(0), f.u32(1), f.u32(2));
        if !race_ok(dbc, race) {
            report.error(format!(
                "Wrong race {race} in `playercreateinfo_spell` table, ignoring."
            ));
            continue;
        }
        if !class_ok(dbc, class) {
            report.error(format!(
                "Wrong class {class} in `playercreateinfo_spell` table, ignoring."
            ));
            continue;
        }
        infos
            .create_spells
            .entry((race, class))
            .or_default()
            .push(spell_id);
    }
    Ok(())
}

fn load_class_level_stats(
    conn: &mut mysql::Conn,
    max_player_level: u32,
    dbc: Option<&RaceClassDbc>,
    infos: &mut PlayerInfos,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> =
        conn.query("SELECT `class`, `level`, `basehp`, `basemana` FROM `player_classlevelstats`")?;
    if rows.is_empty() {
        report.error("Error loading `player_classlevelstats` table or empty table.".to_string());
    }
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (class, level) = (f.u32(0), f.u32(1));
        if class >= MAX_CLASSES {
            report.error(format!(
                "Wrong class {class} in `player_classlevelstats` table, ignoring."
            ));
            continue;
        }
        if level == 0 {
            report.error("Wrong level 0 in `player_classlevelstats` table, ignoring.".to_string());
            continue;
        }
        if level > max_player_level {
            if level > PLAYER_STRONG_MAX_LEVEL {
                report.error(format!(
                    "Wrong (> {PLAYER_STRONG_MAX_LEVEL}) level {level} in `player_classlevelstats` table, ignoring."
                ));
            }
            continue;
        }
        let table = infos
            .class_level
            .entry(class)
            .or_insert_with(|| vec![PlayerClassLevelInfo::default(); max_player_level as usize]);
        table[(level - 1) as usize] = PlayerClassLevelInfo {
            basehealth: f.u16(2),
            basemana: f.u16(3),
        };
    }
    // Fill gaps + integrity (existing DBC classes only).
    for class in 0..MAX_CLASSES {
        let exists = dbc.is_none_or(|d| d.classes.contains(&class));
        if !exists {
            continue;
        }
        match infos.class_level.get_mut(&class) {
            Some(table) if !table.is_empty() && table[0].basehealth != 0 => {
                for level in 1..max_player_level as usize {
                    if table[level].basehealth == 0 {
                        report.error(format!(
                            "Class {class} Level {} does not have health/mana data. Using stats data of level {level}.",
                            level + 1
                        ));
                        table[level] = table[level - 1];
                    }
                }
            }
            _ => {
                if dbc.is_some() {
                    report.error(format!(
                        "Class {class} Level 1 does not have health/mana data!"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn load_level_stats(
    conn: &mut mysql::Conn,
    max_player_level: u32,
    dbc: Option<&RaceClassDbc>,
    infos: &mut PlayerInfos,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `race`, `class`, `level`, `str`, `agi`, `sta`, `inte`, `spi` \
         FROM `player_levelstats`",
    )?;
    if rows.is_empty() {
        report.error("Error loading `player_levelstats` table or empty table.".to_string());
    }
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (race, class, level) = (f.u32(0), f.u32(1), f.u32(2));
        if !race_ok(dbc, race) {
            report.error(format!(
                "Wrong race {race} in `player_levelstats` table, ignoring."
            ));
            continue;
        }
        if !class_ok(dbc, class) {
            report.error(format!(
                "Wrong class {class} in `player_levelstats` table, ignoring."
            ));
            continue;
        }
        // C++ indexes [level-1] unchecked: level 0 rows would
        // be UB there; skip them loudly instead.
        if level == 0 || level > max_player_level {
            if level > PLAYER_STRONG_MAX_LEVEL {
                report.error(format!(
                    "Wrong (> {PLAYER_STRONG_MAX_LEVEL}) level {level} in `player_levelstats` table, ignoring."
                ));
            } else if level == 0 {
                report.error("Wrong level 0 in `player_levelstats` table, ignoring.".to_string());
            }
            continue;
        }
        let table = infos
            .level_stats
            .entry((race, class))
            .or_insert_with(|| vec![PlayerLevelInfo::default(); max_player_level as usize]);
        table[(level - 1) as usize] = PlayerLevelInfo {
            stats: [f.u8(3), f.u8(4), f.u8(5), f.u8(6), f.u8(7)],
        };
    }
    for race in 1..MAX_RACES {
        if !race_ok(dbc, race) {
            continue;
        }
        for class in 1..MAX_CLASSES {
            if !class_ok(dbc, class) {
                continue;
            }
            // Skip combinations without create info (C++
            // displayId guard).
            let has_create = infos
                .create
                .get(&(race, class))
                .is_some_and(|c| c.display_id_m != 0 && c.display_id_f != 0);
            if !has_create {
                continue;
            }
            match infos.level_stats.get_mut(&(race, class)) {
                Some(table) if !table.is_empty() && table[0].stats[0] != 0 => {
                    for level in 1..max_player_level as usize {
                        if table[level].stats[0] == 0 {
                            report.error(format!(
                                "Race {race} Class {class} Level {} does not have stats data. Using stats data of level {level}.",
                                level + 1
                            ));
                            table[level] = table[level - 1];
                        }
                    }
                }
                _ => report.error(format!(
                    "Race {race} Class {class} Level 1 does not have stats data!"
                )),
            }
        }
    }
    Ok(())
}

fn load_xp_for_level(
    conn: &mut mysql::Conn,
    max_player_level: u32,
    infos: &mut PlayerInfos,
    report: &mut LoadReport,
) -> Result<(), DbError> {
    let rows: Vec<mysql::Row> =
        conn.query("SELECT `lvl`, `xp_for_next_level` FROM `player_xp_for_level`")?;
    if rows.is_empty() {
        report.error("Error loading `player_xp_for_level` table or empty table.".to_string());
    }
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (level, xp) = (f.u32(0), f.u32(1));
        if level > max_player_level {
            if level > PLAYER_STRONG_MAX_LEVEL {
                report.error(format!(
                    "Wrong (> {PLAYER_STRONG_MAX_LEVEL}) level {level} in `player_xp_for_level` table, ignoring."
                ));
            }
            continue;
        }
        infos.xp_per_level[level as usize] = xp;
    }
    for level in 1..max_player_level as usize {
        if infos.xp_per_level[level] == 0 {
            report.error(format!(
                "Level {} does not have XP for level data. Using data of level [{level}] + 100.",
                level + 1
            ));
            infos.xp_per_level[level] = infos.xp_per_level[level - 1] + 100;
        }
    }
    Ok(())
}
