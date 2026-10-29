//! Battleground templates and world safe locations
//! (`BattleGroundMgr::CreateInitialBattleGrounds`,
//! `sWorldSafeLocsStore`, `ObjectMgr::LoadWorldSafeLocsFacing`).

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// `enum BattleGroundTypeId` (`SharedDefines.h`).
pub mod bg_type {
    pub const NONE: u32 = 0;
    pub const AV: u32 = 1;
    pub const WS: u32 = 2;
    pub const AB: u32 = 3;
}

/// `GetBattleGroundTypeIdByMapId`.
#[must_use]
pub fn bg_type_by_map_id(map_id: u32) -> u32 {
    match map_id {
        30 => bg_type::AV,
        489 => bg_type::WS,
        529 => bg_type::AB,
        _ => bg_type::NONE,
    }
}

/// `GetBattleGrounMapIdByTypeId`.
#[must_use]
pub fn bg_map_id_by_type(bg_type_id: u32) -> u32 {
    match bg_type_id {
        bg_type::AV => 30,
        bg_type::WS => 489,
        bg_type::AB => 529,
        _ => 0,
    }
}

/// One `WorldSafeLocs.dbc` row plus the DB facing
/// (`WorldSafeLocsEntry` + `GetWorldSafeLocFacing`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSafeLoc {
    pub id: u32,
    pub map_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

/// `WorldSafeLocs.dbc` by id, with `world_safe_locs_facing`
/// orientations applied (missing facing reads 0.0, fork
/// `GetWorldSafeLocFacing`).
#[derive(Clone, Debug, Default)]
pub struct SafeLocStore {
    by_id: HashMap<u32, WorldSafeLoc>,
}

impl SafeLocStore {
    /// Builds the store from produced `WorldSafeLocs.dbc`
    /// rows (fmt `nifffxxxxxxxxx`: 0 id, 1 map, 2-4 xyz)
    /// and DB facings.
    #[must_use]
    pub fn from_rows(rows: &[(u32, u32, f32, f32, f32)], facings: &HashMap<u32, f32>) -> Self {
        let by_id = rows
            .iter()
            .map(|&(id, map_id, x, y, z)| {
                (
                    id,
                    WorldSafeLoc {
                        id,
                        map_id,
                        x,
                        y,
                        z,
                        orientation: facings.get(&id).copied().unwrap_or(0.0),
                    },
                )
            })
            .collect();
        Self { by_id }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// `sWorldSafeLocsStore.LookupEntry`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<WorldSafeLoc> {
        self.by_id.get(&id).copied()
    }
}

/// Loads `world_safe_locs_facing`
/// (`ObjectMgr::LoadWorldSafeLocsFacing`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_safe_loc_facings(conn: &mut mysql::Conn) -> Result<HashMap<u32, f32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `orientation` FROM `world_safe_locs_facing`",
        (),
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = mangos_db::row::Fields::from_row(row);
            (f.u32(0), f.f32(1))
        })
        .collect())
}

/// One `battleground_template` row with both team start
/// locations resolved (the inputs of fork
/// `CreateBattleGround`).
#[derive(Clone, Debug, PartialEq)]
pub struct BattlegroundTemplate {
    pub bg_type_id: u32,
    pub min_players_per_team: u32,
    pub max_players_per_team: u32,
    pub min_level: u32,
    pub max_level: u32,
    pub alliance_win_spell: u32,
    pub alliance_lose_spell: u32,
    pub horde_win_spell: u32,
    pub horde_lose_spell: u32,
    pub alliance_start: WorldSafeLoc,
    pub horde_start: WorldSafeLoc,
    pub player_skin_ref_loot_id: u32,
    pub map_id: u32,
    pub name: String,
}

/// Patch-selected `battleground_template` rows by type id.
#[derive(Clone, Debug, Default)]
pub struct BattlegroundTemplates {
    by_type: HashMap<u32, BattlegroundTemplate>,
}

impl BattlegroundTemplates {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_type.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_type.is_empty()
    }

    /// `BattleGroundMgr::GetBattleGroundTemplate` input row.
    #[must_use]
    pub fn get(&self, bg_type_id: u32) -> Option<&BattlegroundTemplate> {
        self.by_type.get(&bg_type_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &BattlegroundTemplate)> {
        self.by_type.iter()
    }
}

/// Loads the progression-selected `battleground_template`
/// rows (`CreateInitialBattleGrounds` query), resolving
/// start locations through `safe_locs` and map names
/// through `map_name` (fork `sMapStorage`). Rows with an
/// unresolvable start location or map are skipped with a
/// report line, exactly as the fork skips them.
///
/// UNMODELED: `ExistsRefLootTemplate` zeroing of
/// `player_loot_id` (no ref-loot existence set on this
/// path yet; value carried verbatim).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_battleground_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    safe_locs: &SafeLocStore,
    map_name: &dyn Fn(u32) -> Option<String>,
    report: &mut crate::load_report::LoadReport,
) -> Result<BattlegroundTemplates, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `min_players_per_team`, `max_players_per_team`, \
         `min_level`, `max_level`, `alliance_win_spell`, `alliance_lose_spell`, \
         `horde_win_spell`, `horde_lose_spell`, `alliance_start_location`, \
         `horde_start_location`, `player_loot_id` \
         FROM `battleground_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `battleground_template` t2 \
                          WHERE t1.`id` = t2.`id` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let mut by_type = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let bg_type_id = f.u32(0);
        let Some(alliance_start) = safe_locs.get(f.u32(9)) else {
            report.error(format!(
                "battleground_template id {bg_type_id}: nonexistent WorldSafeLocs id {} (alliance start)",
                f.u32(9)
            ));
            continue;
        };
        let Some(horde_start) = safe_locs.get(f.u32(10)) else {
            report.error(format!(
                "battleground_template id {bg_type_id}: nonexistent WorldSafeLocs id {} (horde start)",
                f.u32(10)
            ));
            continue;
        };
        let map_id = bg_map_id_by_type(bg_type_id);
        let Some(name) = map_name(map_id) else {
            report.error(format!(
                "battleground_template id {bg_type_id}: nonexistent map id {map_id}"
            ));
            continue;
        };
        by_type.insert(
            bg_type_id,
            BattlegroundTemplate {
                bg_type_id,
                min_players_per_team: f.u32(1),
                max_players_per_team: f.u32(2),
                min_level: f.u32(3),
                max_level: f.u32(4),
                alliance_win_spell: f.u32(5),
                alliance_lose_spell: f.u32(6),
                horde_win_spell: f.u32(7),
                horde_lose_spell: f.u32(8),
                alliance_start,
                horde_start,
                player_skin_ref_loot_id: f.u32(11),
                map_id,
                name,
            },
        );
    }
    Ok(BattlegroundTemplates { by_type })
}

/// `battlemaster_entry` creature entry -> battleground
/// type id (`BattleGroundMgr::LoadBattleMastersEntry` /
/// `GetBattleMasterBG`).
#[derive(Clone, Debug, Default)]
pub struct BattleMasters {
    by_entry: HashMap<u32, u32>,
}

impl BattleMasters {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    /// `GetBattleMasterBG` (missing entry reads
    /// `BATTLEGROUND_TYPE_NONE`).
    #[must_use]
    pub fn bg_type(&self, creature_entry: u32) -> u32 {
        self.by_entry
            .get(&creature_entry)
            .copied()
            .unwrap_or(bg_type::NONE)
    }
}

/// Loads `battlemaster_entry`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_battlemasters(conn: &mut mysql::Conn) -> Result<BattleMasters, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `bg_template` FROM `battlemaster_entry`",
        (),
    )?;
    let by_entry = rows
        .into_iter()
        .map(|row| {
            let f = mangos_db::row::Fields::from_row(row);
            (f.u32(0), f.u32(1))
        })
        .collect();
    Ok(BattleMasters { by_entry })
}

/// `BG_EVENT_NONE` sentinel (`BattleGroundDefines.h`).
pub const BG_EVENT_NONE: u8 = 255;

/// DB spawn -> battleground event indexes
/// (`BattleGroundMgr::LoadBattleEventIndexes`). The fork
/// query full-outer-joins `battleground_events` only to
/// log mismatches; map content comes from the two
/// `*_battleground` tables with `BG_EVENT_NONE` rows
/// skipped, and lookups of unknown guids read the
/// `(255, 255)` sentinel.
#[derive(Clone, Debug, Default)]
pub struct BattleEventIndexes {
    creature: HashMap<u32, Vec<(u8, u8)>>,
    gameobject: HashMap<u32, Vec<(u8, u8)>>,
}

impl BattleEventIndexes {
    #[must_use]
    pub fn len(&self) -> usize {
        self.creature.len() + self.gameobject.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.creature.is_empty() && self.gameobject.is_empty()
    }

    /// `GetCreatureEventIndex` (first index; sentinel when
    /// unregistered).
    #[must_use]
    pub fn creature_index(&self, db_guid: u32) -> (u8, u8) {
        self.creature
            .get(&db_guid)
            .and_then(|v| v.first().copied())
            .unwrap_or((BG_EVENT_NONE, BG_EVENT_NONE))
    }

    /// `GetCreatureEventsVector`.
    #[must_use]
    pub fn creature_indexes(&self, db_guid: u32) -> &[(u8, u8)] {
        self.creature.get(&db_guid).map_or(&[], Vec::as_slice)
    }

    /// `GetGameObjectEventIndex`.
    #[must_use]
    pub fn gameobject_index(&self, db_guid: u32) -> (u8, u8) {
        self.gameobject
            .get(&db_guid)
            .and_then(|v| v.first().copied())
            .unwrap_or((BG_EVENT_NONE, BG_EVENT_NONE))
    }

    /// `GetGameObjectEventsVector`.
    #[must_use]
    pub fn gameobject_indexes(&self, db_guid: u32) -> &[(u8, u8)] {
        self.gameobject.get(&db_guid).map_or(&[], Vec::as_slice)
    }
}

fn collect_event_indexes(rows: Vec<mysql::Row>) -> HashMap<u32, Vec<(u8, u8)>> {
    let mut by_guid: HashMap<u32, Vec<(u8, u8)>> = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (event1, event2) = (f.u8(1), f.u8(2));
        if event1 == BG_EVENT_NONE || event2 == BG_EVENT_NONE {
            continue;
        }
        by_guid.entry(f.u32(0)).or_default().push((event1, event2));
    }
    by_guid
}

/// One `game_graveyard_zone` link (`GraveYardData`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraveyardLink {
    pub safe_loc_id: u32,
    /// Fork `Team` DB value (0 none, 67 horde, 469 alliance).
    pub team: u32,
}

/// `game_graveyard_zone` links by ghost zone
/// (`ObjectMgr::LoadGraveyardZones` / `m_GraveYardMap`),
/// insertion order preserved per zone (fork multimap).
#[derive(Clone, Debug, Default)]
pub struct GraveyardLinks {
    by_zone: HashMap<u32, Vec<GraveyardLink>>,
}

impl GraveyardLinks {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_zone.values().map(Vec::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_zone.is_empty()
    }

    /// Links registered for a zone/area id.
    #[must_use]
    pub fn for_zone(&self, zone_id: u32) -> &[GraveyardLink] {
        self.by_zone.get(&zone_id).map_or(&[], Vec::as_slice)
    }
}

/// Loads `game_graveyard_zone` at the server patch,
/// skipping rows with an unknown safe loc, unknown
/// zone, or non-player team (fork validations), and
/// deduping identical (loc, zone, team) links
/// (`AddGraveYardLink`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_graveyard_links(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    safe_locs: &SafeLocStore,
    zone_known: &dyn Fn(u32) -> bool,
    report: &mut crate::load_report::LoadReport,
) -> Result<GraveyardLinks, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `ghost_zone`, `faction` FROM `game_graveyard_zone` \
         WHERE ? BETWEEN `patch_min` AND `patch_max`",
        (wow_patch,),
    )?;
    let mut by_zone: HashMap<u32, Vec<GraveyardLink>> = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (safe_loc_id, zone_id, team) = (f.u32(0), f.u32(1), f.u32(2));
        if safe_locs.get(safe_loc_id).is_none() {
            report.error(format!(
                "game_graveyard_zone: nonexistent WorldSafeLocs id {safe_loc_id}"
            ));
            continue;
        }
        if !zone_known(zone_id) {
            report.error(format!(
                "game_graveyard_zone: nonexistent zone id {zone_id}"
            ));
            continue;
        }
        if !matches!(team, 0 | 67 | 469) {
            report.error(format!("game_graveyard_zone: non-player faction {team}"));
            continue;
        }
        let links = by_zone.entry(zone_id).or_default();
        let link = GraveyardLink { safe_loc_id, team };
        // `FindGraveYardData(id, zoneId)`: dup key is
        // (safe loc, zone), team ignored.
        if links.iter().any(|l| l.safe_loc_id == safe_loc_id) {
            report.error(format!(
                "game_graveyard_zone: duplicate link loc {safe_loc_id} zone {zone_id}"
            ));
            continue;
        }
        links.push(link);
    }
    Ok(GraveyardLinks { by_zone })
}

/// One `areatrigger_bg_entrance` row
/// (`BattlegroundEntranceTrigger`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BattlegroundEntrance {
    /// Fork `Team` DB value (67 horde, 469 alliance).
    pub team: u32,
    pub bg_type_id: u32,
    pub exit_map_id: u32,
    pub exit_x: f32,
    pub exit_y: f32,
    pub exit_z: f32,
    pub exit_o: f32,
}

/// `areatrigger_bg_entrance` by trigger id
/// (`ObjectMgr::LoadBattlegroundEntranceTriggers`).
#[derive(Clone, Debug, Default)]
pub struct BattlegroundEntrances {
    by_trigger: HashMap<u32, BattlegroundEntrance>,
}

impl BattlegroundEntrances {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_trigger.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_trigger.is_empty()
    }

    /// `GetBattlegroundEntranceTrigger`.
    #[must_use]
    pub fn get(&self, trigger_id: u32) -> Option<BattlegroundEntrance> {
        self.by_trigger.get(&trigger_id).copied()
    }
}

/// Loads `areatrigger_bg_entrance`, skipping rows with
/// an unknown trigger, non-Alliance/Horde team, or
/// out-of-range battleground type (fork validations).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_battleground_entrances(
    conn: &mut mysql::Conn,
    trigger_known: &dyn Fn(u32) -> bool,
    report: &mut crate::load_report::LoadReport,
) -> Result<BattlegroundEntrances, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `team`, `bg_template`, `exit_map`, `exit_position_x`, \
         `exit_position_y`, `exit_position_z`, `exit_orientation` \
         FROM `areatrigger_bg_entrance`",
        (),
    )?;
    let mut by_trigger = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (trigger_id, team, bg_type_id) = (f.u32(0), f.u32(1), f.u32(2));
        if !trigger_known(trigger_id) {
            report.error(format!(
                "areatrigger_bg_entrance: unknown area trigger id {trigger_id}"
            ));
            continue;
        }
        if !matches!(team, 67 | 469) {
            report.error(format!(
                "areatrigger_bg_entrance: team {team} is not Horde or Alliance for trigger {trigger_id}"
            ));
            continue;
        }
        if bg_type_id >= 4 {
            report.error(format!(
                "areatrigger_bg_entrance: nonexistent battleground type {bg_type_id} for trigger {trigger_id}"
            ));
            continue;
        }
        by_trigger.insert(
            trigger_id,
            BattlegroundEntrance {
                team,
                bg_type_id,
                exit_map_id: f.u32(3),
                exit_x: f.f32(4),
                exit_y: f.f32(5),
                exit_z: f.f32(6),
                exit_o: f.f32(7),
            },
        );
    }
    Ok(BattlegroundEntrances { by_trigger })
}

/// Loads `creature_battleground` + `gameobject_battleground`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_battle_event_indexes(conn: &mut mysql::Conn) -> Result<BattleEventIndexes, DbError> {
    let creature_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `event1`, `event2` FROM `creature_battleground`",
        (),
    )?;
    let gameobject_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `event1`, `event2` FROM `gameobject_battleground`",
        (),
    )?;
    Ok(BattleEventIndexes {
        creature: collect_event_indexes(creature_rows),
        gameobject: collect_event_indexes(gameobject_rows),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_map_roundtrip() {
        assert_eq!(bg_type_by_map_id(30), bg_type::AV);
        assert_eq!(bg_type_by_map_id(489), bg_type::WS);
        assert_eq!(bg_type_by_map_id(529), bg_type::AB);
        assert_eq!(bg_type_by_map_id(0), bg_type::NONE);
        assert_eq!(bg_map_id_by_type(bg_type::AV), 30);
        assert_eq!(bg_map_id_by_type(bg_type::NONE), 0);
    }

    #[test]
    fn safe_loc_facing_defaults_zero() {
        let store = SafeLocStore::from_rows(&[(7, 30, 1.0, 2.0, 3.0)], &HashMap::new());
        assert_eq!(
            store.get(7),
            Some(WorldSafeLoc {
                id: 7,
                map_id: 30,
                x: 1.0,
                y: 2.0,
                z: 3.0,
                orientation: 0.0,
            })
        );
        assert_eq!(store.get(8), None);
    }
}
