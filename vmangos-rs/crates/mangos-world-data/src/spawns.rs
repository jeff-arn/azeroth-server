//! Spawn-row stores + the static spawn grid index, ported from
//! `ObjectMgr::LoadCreatures` / `LoadGameobjects` /
//! `AddCreatureToGrid` / `AddGameobjectToGrid`
//! (`ObjectMgr.cpp:2275-2651`), `CreatureData`
//! (`CreatureDefines.h:370-412`), `GameObjectData`
//! (`GameObjectDefines.h:909-929`), spawn flags
//! (`ObjectDefines.h:127-134`) and grid math
//! (`Maps/GridDefines.h:38-59,145-167`).
//!
//! Faithful bits that matter:
//!
//! - Rows are inserted into the data maps **before** the map /
//!   state / rotation / coordinate validations, so rows that
//!   fail them stay in the store (never grid-added), exactly as
//!   in C++.
//! - Creature rows outside the current content patch stay in
//!   the store with `SPAWN_FLAG_DISABLED` OR-ed in; gameobject
//!   rows outside the patch are skipped entirely (the C++
//!   asymmetry at `ObjectMgr.cpp:2418-2419` vs `:2524-2525`).
//! - Grid membership requires not-already-present (reload),
//!   in-patch, and event/pool ids all zero. Transport-type GOs
//!   never grid-add; they register as elevator transports.
//! - `instanciated_continent_instance_id` is stored as 0:
//!   `GetContinentInstanceId` returns 0 unless
//!   `ContinentsInstantiate` is on, and the polygon port belongs
//!   to the Slice 3 map manager (reported at load).
//! - Map existence comes from a caller-supplied id set
//!   (`sMapStorage` / `IsValidMAP`); when absent, validation is
//!   skipped and reported, and rows grid-add unconditionally.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;

use crate::creature_template::CreatureTemplates;
use crate::gameobject_template::{GameObjectTemplates, TYPE_TRANSPORT};
use crate::load_report::LoadReport;

/// `MAX_CREATURE_IDS_PER_SPAWN`.
pub const MAX_CREATURE_IDS_PER_SPAWN: usize = 5;

/// `SPAWN_FLAG_ACTIVE` (`ObjectDefines.h:127`).
pub const SPAWN_FLAG_ACTIVE: u32 = 0x01;
/// `SPAWN_FLAG_DISABLED`.
pub const SPAWN_FLAG_DISABLED: u32 = 0x02;
/// `SPAWN_FLAG_RANDOM_RESPAWN_TIME`.
pub const SPAWN_FLAG_RANDOM_RESPAWN_TIME: u32 = 0x04;
/// `SPAWN_FLAG_DYNAMIC_RESPAWN_TIME`.
pub const SPAWN_FLAG_DYNAMIC_RESPAWN_TIME: u32 = 0x08;
/// `SPAWN_FLAG_FORCE_DYNAMIC_ELITE` (creature only).
pub const SPAWN_FLAG_FORCE_DYNAMIC_ELITE: u32 = 0x10;
/// `SPAWN_FLAG_EVADE_OUT_HOME_AREA` (creature only).
pub const SPAWN_FLAG_EVADE_OUT_HOME_AREA: u32 = 0x20;
/// `SPAWN_FLAG_NOT_VISIBLE` (creature only).
pub const SPAWN_FLAG_NOT_VISIBLE: u32 = 0x40;
/// `SPAWN_FLAG_DEAD` (creature only).
pub const SPAWN_FLAG_DEAD: u32 = 0x80;

/// `CREATURE_STATIC_FLAG_NO_AUTOMATIC_REGEN`
/// (`CreatureDefines.h:109`).
const STATIC_FLAG_NO_AUTOMATIC_REGEN: u32 = 0x0000_0400;
/// `CLASS_WARRIOR` (`SharedDefines.h:82`).
const CLASS_WARRIOR: u32 = 1;

/// `MAX_GO_STATE`: `GOState` enum has `READY`=0, `ACTIVE`=1,
/// `ACTIVE_ALTERNATIVE`=2 (+MAX sentinel 3 at 5875).
const MAX_GO_STATE: u32 = 3;

/// `MAX_NUMBER_OF_GRIDS` / `SIZE_OF_GRIDS` (`GridDefines.h:38-41`).
pub const MAX_NUMBER_OF_GRIDS: u32 = 64;
/// `TOTAL_NUMBER_OF_CELLS_PER_MAP` (`GridDefines.h:54`): 64 * 16.
pub const TOTAL_NUMBER_OF_CELLS_PER_MAP: u32 = 1024;

/// `MaNGOS::ComputeCellPair` (`GridDefines.h:150-167`): computed
/// in `f64` like the C++ (comment says "for having same result
/// as MySQL calculations"), then C-style truncation to `int`,
/// then `CoordPair(y, x)` clamped via the constructor (`val < 0`
/// keeps the request in debug asserts only; stores as `uint32`
/// — negative results wrap, as in C++).
///
/// Returns `(x_coord, y_coord)`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // f32->f64 widening is exact; the arithmetic is f64 as in C++
pub fn compute_cell_pair(x: f32, y: f32) -> (u32, u32) {
    // SIZE_OF_GRIDS/16 and its half, as f32 literals like the macros.
    let size_of_grid_cell = f64::from(mangos_movement::movement_info::SIZE_OF_GRIDS / 16.0);
    let center_offset = size_of_grid_cell / 2.0;
    let center = 512.0; // CENTER_GRID_CELL_ID
    let x_val = ((f64::from(x) - center_offset) / size_of_grid_cell + center + 0.5) as i32;
    let y_val = ((f64::from(y) - center_offset) / size_of_grid_cell + center + 0.5) as i32;
    // CoordPair constructor stores into uint32 members.
    (
        u32::from_ne_bytes(x_val.to_ne_bytes()),
        u32::from_ne_bytes(y_val.to_ne_bytes()),
    )
}

/// Flat cell id used by `AddCreatureToGrid` (`ObjectMgr.cpp:2464`):
/// `y * TOTAL_NUMBER_OF_CELLS_PER_MAP + x` with wrapping u32 math
/// for out-of-range inputs, as in C++.
#[must_use]
pub fn cell_id(x: f32, y: f32) -> u32 {
    let (cx, cy) = compute_cell_pair(x, y);
    cy.wrapping_mul(TOTAL_NUMBER_OF_CELLS_PER_MAP)
        .wrapping_add(cx)
}

/// `struct WorldLocation` (`SharedDefines.h:1828-1839`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldLocation {
    pub map_id: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub o: f32,
}

impl WorldLocation {
    #[must_use]
    pub fn position(self) -> Position {
        Position::new(self.x, self.y, self.z, self.o)
    }
}

/// `struct CreatureData` (`CreatureDefines.h:370-412`).
#[derive(Clone, Debug)]
pub struct CreatureSpawn {
    pub creature_id: [u32; MAX_CREATURE_IDS_PER_SPAWN],
    pub position: WorldLocation,
    pub spawntimesecsmin: u32,
    pub spawntimesecsmax: u32,
    pub wander_distance: f32,
    pub health_percent: f32,
    pub mana_percent: f32,
    pub movement_type: u8,
    pub spawn_flags: u32,
    pub visibility_mod: f32,
    /// Non-DB field; 0 until the Slice 3 map manager ports
    /// continent instancing (module docs).
    pub instanciated_continent_instance_id: u32,
}

impl Default for CreatureSpawn {
    fn default() -> Self {
        Self {
            creature_id: [0; MAX_CREATURE_IDS_PER_SPAWN],
            position: WorldLocation::default(),
            spawntimesecsmin: 0,
            spawntimesecsmax: 0,
            wander_distance: 0.0,
            health_percent: 100.0,
            mana_percent: 100.0,
            movement_type: 0,
            spawn_flags: 0,
            visibility_mod: 0.0,
            instanciated_continent_instance_id: 0,
        }
    }
}

impl CreatureSpawn {
    /// `GetObjectGuid` (`CreatureDefines.h:387`) — `HIGHGUID_UNIT`.
    #[must_use]
    pub fn object_guid(&self, low_guid: u32) -> ObjectGuid {
        ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, self.creature_id[0], low_guid)
    }

    /// `GetCreatureIdCount` (`CreatureDefines.h:405-411`).
    #[must_use]
    pub fn creature_id_count(&self) -> usize {
        self.creature_id.iter().take_while(|&&id| id != 0).count()
    }

    /// `HasCreatureId` (`CreatureDefines.h:404`).
    #[must_use]
    pub fn has_creature_id(&self, id: u32) -> bool {
        self.creature_id.contains(&id)
    }

    /// `ChooseCreatureId` (`CreatureDefines.h:389-403`): uniform
    /// pick among the nonzero prefix via caller-supplied
    /// `urand(0, count-1)`; falls back to entry 1 when empty
    /// (unreachable for store-loaded rows, which validate id).
    #[must_use]
    pub fn choose_creature_id(&self, urand: impl FnOnce(u32, u32) -> u32) -> u32 {
        let count = self.creature_id_count();
        if count > 0 {
            return self.creature_id[urand(0, count as u32 - 1) as usize];
        }
        1
    }

    /// `GetRandomRespawnTime` (`CreatureDefines.h:388`).
    #[must_use]
    pub fn random_respawn_time(&self, urand: impl FnOnce(u32, u32) -> u32) -> u32 {
        urand(self.spawntimesecsmin, self.spawntimesecsmax)
    }
}

/// `struct GameObjectData` (`GameObjectDefines.h:913-929`).
#[derive(Clone, Debug, Default)]
pub struct GameObjectSpawn {
    /// Entry in `gameobject_template`.
    pub id: u32,
    pub position: WorldLocation,
    pub rotation0: f32,
    pub rotation1: f32,
    pub rotation2: f32,
    pub rotation3: f32,
    pub spawntimesecsmin: i32,
    pub spawntimesecsmax: i32,
    pub animprogress: u32,
    /// Raw `GOState` value (0 `READY`, 1 `ACTIVE`, 2 `ACTIVE_ALTERNATIVE`).
    pub go_state: u32,
    pub spawn_flags: u32,
    pub visibility_mod: f32,
    /// Non-DB field; 0 (see [`CreatureSpawn`]).
    pub instanciated_continent_instance_id: u32,
}

impl GameObjectSpawn {
    /// `GetRandomRespawnTime` (`GameObjectDefines.h:928`): the
    /// signed fields are cast to `uint32` for the roll.
    #[must_use]
    pub fn random_respawn_time(&self, urand: impl FnOnce(u32, u32) -> u32) -> u32 {
        urand(
            u32::from_ne_bytes(self.spawntimesecsmin.to_ne_bytes()),
            u32::from_ne_bytes(self.spawntimesecsmax.to_ne_bytes()),
        )
    }
}

/// `CellObjectGuids` (`ObjectMgr.h:149-154`): static spawn GUIDs
/// per cell. Corpses join at runtime (player-guid → instance),
/// empty after load.
#[derive(Clone, Debug, Default)]
pub struct CellObjectGuids {
    pub creatures: BTreeSet<u32>,
    pub gameobjects: BTreeSet<u32>,
    pub corpses: BTreeMap<u32, u32>,
}

/// `m_MapObjectGuids` (`ObjectMgr.h:155`): map id → cell id → sets.
#[derive(Clone, Debug, Default)]
pub struct SpawnGrid {
    by_map: HashMap<u32, HashMap<u32, CellObjectGuids>>,
}

impl SpawnGrid {
    /// `AddCreatureToGrid` (`ObjectMgr.cpp:2461-2469`).
    pub fn add_creature(&mut self, guid: u32, position: &WorldLocation) {
        self.cell_mut(position).creatures.insert(guid);
    }

    /// `RemoveCreatureFromGrid` (`ObjectMgr.cpp:2471-2479`).
    pub fn remove_creature(&mut self, guid: u32, position: &WorldLocation) {
        self.cell_mut(position).creatures.remove(&guid);
    }

    /// `AddGameobjectToGrid` (`ObjectMgr.cpp:2633-2641`).
    pub fn add_gameobject(&mut self, guid: u32, position: &WorldLocation) {
        self.cell_mut(position).gameobjects.insert(guid);
    }

    /// `RemoveGameobjectFromGrid` (`ObjectMgr.cpp:2643-2651`).
    pub fn remove_gameobject(&mut self, guid: u32, position: &WorldLocation) {
        self.cell_mut(position).gameobjects.remove(&guid);
    }

    fn cell_mut(&mut self, position: &WorldLocation) -> &mut CellObjectGuids {
        self.by_map
            .entry(u32::from(position.map_id))
            .or_default()
            .entry(cell_id(position.x, position.y))
            .or_default()
    }

    #[must_use]
    pub fn cell(&self, map_id: u32, cell: u32) -> Option<&CellObjectGuids> {
        self.by_map.get(&map_id).and_then(|m| m.get(&cell))
    }

    /// Total grid-registered creature guids across all maps.
    #[must_use]
    pub fn creature_count(&self) -> usize {
        self.by_map
            .values()
            .flat_map(|m| m.values())
            .map(|c| c.creatures.len())
            .sum()
    }

    /// Total grid-registered gameobject guids across all maps.
    #[must_use]
    pub fn gameobject_count(&self) -> usize {
        self.by_map
            .values()
            .flat_map(|m| m.values())
            .map(|c| c.gameobjects.len())
            .sum()
    }
}

/// The ObjectMgr-owned spawn stores (`m_CreatureDataMap`,
/// `m_GameObjectDataMap`, `m_MapObjectGuids`, elevator registry).
#[derive(Clone, Debug, Default)]
pub struct SpawnData {
    creatures: HashMap<u32, CreatureSpawn>,
    gameobjects: HashMap<u32, GameObjectSpawn>,
    grid: SpawnGrid,
    /// `sTransportMgr.AddElevatorTransportForMap` registrations:
    /// map id → transport GO guids, in load order.
    elevator_transports: HashMap<u32, Vec<u32>>,
}

impl SpawnData {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `GetCreatureData(guid)` (`ObjectMgr.h`, map lookup).
    #[must_use]
    pub fn creature(&self, guid: u32) -> Option<&CreatureSpawn> {
        self.creatures.get(&guid)
    }

    /// `GetGameObjectData(guid)`.
    #[must_use]
    pub fn gameobject(&self, guid: u32) -> Option<&GameObjectSpawn> {
        self.gameobjects.get(&guid)
    }

    #[must_use]
    pub fn creature_count(&self) -> usize {
        self.creatures.len()
    }

    #[must_use]
    pub fn gameobject_count(&self) -> usize {
        self.gameobjects.len()
    }

    #[must_use]
    pub fn grid(&self) -> &SpawnGrid {
        &self.grid
    }

    #[must_use]
    pub fn grid_mut(&mut self) -> &mut SpawnGrid {
        &mut self.grid
    }

    #[must_use]
    pub fn elevator_transports(&self, map_id: u32) -> &[u32] {
        self.elevator_transports
            .get(&map_id)
            .map_or(&[], Vec::as_slice)
    }

    pub fn creatures(&self) -> impl Iterator<Item = (&u32, &CreatureSpawn)> {
        self.creatures.iter()
    }

    pub fn gameobjects(&self) -> impl Iterator<Item = (&u32, &GameObjectSpawn)> {
        self.gameobjects.iter()
    }
}

const CREATURE_SELECT: &str = "SELECT `creature`.`guid`, `creature`.`id`, `creature`.`id2`, \
    `creature`.`id3`, `creature`.`id4`, `creature`.`id5`, `map`, `position_x`, `position_y`, \
    `position_z`, `orientation`, `spawntimesecsmin`, `spawntimesecsmax`, `wander_distance`, \
    `health_percent`, `mana_percent`, `movement_type`, `event`, \
    `pool_creature`.`pool_entry`, `pool_creature_template`.`pool_entry`, `spawn_flags`, \
    `visibility_mod`, `creature`.`patch_min`, `creature`.`patch_max` \
    FROM `creature` \
    LEFT OUTER JOIN `game_event_creature` ON `creature`.`guid` = `game_event_creature`.`guid` \
    LEFT OUTER JOIN `pool_creature` ON `creature`.`guid` = `pool_creature`.`guid` \
    LEFT OUTER JOIN `pool_creature_template` ON `creature`.`id` = `pool_creature_template`.`id`";

const GAMEOBJECT_SELECT: &str = "SELECT `gameobject`.`guid`, `gameobject`.`id`, `map`, \
    `position_x`, `position_y`, `position_z`, `orientation`, `rotation0`, `rotation1`, \
    `rotation2`, `rotation3`, `spawntimesecsmin`, `spawntimesecsmax`, `animprogress`, `state`, \
    `event`, `pool_gameobject`.`pool_entry`, `pool_gameobject_template`.`pool_entry`, \
    `spawn_flags`, `visibility_mod`, `gameobject`.`patch_min`, `gameobject`.`patch_max` \
    FROM `gameobject` \
    LEFT OUTER JOIN `game_event_gameobject` ON `gameobject`.`guid` = `game_event_gameobject`.`guid` \
    LEFT OUTER JOIN `pool_gameobject` ON `gameobject`.`guid` = `pool_gameobject`.`guid` \
    LEFT OUTER JOIN `pool_gameobject_template` ON `gameobject`.`id` = `pool_gameobject_template`.`id`";

fn note_spawn_skips(table: &str, map_ids: Option<&HashSet<u32>>, report: &mut LoadReport) {
    if map_ids.is_none() {
        report.error(format!(
            "{table} load: map existence validation skipped (no map id set supplied)"
        ));
    }
    report.error(
        "spawn load: instanciated continent instance ids stored as 0 (continent instancing lands with the Slice 3 map manager)".to_string(),
    );
}

impl SpawnData {
    /// `LoadCreatures(reload)` (`ObjectMgr.cpp:2275-2459`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    #[allow(clippy::too_many_lines)] // one C++ loader body, kept in row order
    pub fn load_creatures(
        &mut self,
        conn: &mut mysql::Conn,
        wow_patch: u32,
        templates: &CreatureTemplates,
        map_ids: Option<&HashSet<u32>>,
        reload: bool,
    ) -> Result<LoadReport, DbError> {
        let rows: Vec<mysql::Row> = conn.exec(CREATURE_SELECT, ())?;
        let mut report = LoadReport::default();
        note_spawn_skips("creature", map_ids, &mut report);

        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            let guid = f.u32(0);
            let first_entry = f.u32(1);
            let mut curhealth = f.f32(14);
            let mut curmana = f.f32(15);
            let spawn_flags = f.u32(20);
            let is_dead = spawn_flags & SPAWN_FLAG_DEAD != 0;
            let mut patch_min = f.u8(22);
            let mut patch_max = f.u8(23);

            if first_entry == 0 {
                report.error(format!(
                    "Table `creature` has creature (GUID: {guid}) with non existing creature entry 0, skipped."
                ));
                continue;
            }
            if patch_min > patch_max || patch_max > 10 {
                report.error(format!(
                    "Table `creature` GUID {guid} (entry {first_entry}) has invalid values patch_min={patch_min}, patch_max={patch_max}."
                ));
                patch_min = 0;
                patch_max = 10;
            }
            let exists_in_patch =
                wow_patch >= u32::from(patch_min) && wow_patch <= u32::from(patch_max);

            let mut skip = false;
            for i in 0..MAX_CREATURE_IDS_PER_SPAWN {
                let entry = f.u32(1 + i);
                if entry == 0 {
                    continue;
                }
                let Some(cinfo) = templates.get(entry) else {
                    if exists_in_patch {
                        report.error(format!(
                            "Table `creature` has creature (GUID: {guid}) with non existing creature entry {entry}, skipped."
                        ));
                    }
                    skip = true;
                    break;
                };
                let no_regen = cinfo.static_flags1 & STATIC_FLAG_NO_AUTOMATIC_REGEN != 0;
                if !no_regen && curhealth < 100.0 && !is_dead {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {first_entry}) with no CREATURE_STATIC_FLAG_NO_AUTOMATIC_REGEN and low current health percent ({curhealth}%)."
                    ));
                    curhealth = 100.0;
                }
                if !no_regen && cinfo.unit_class != CLASS_WARRIOR && curmana < 100.0 {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {first_entry}) with no CREATURE_STATIC_FLAG_NO_AUTOMATIC_REGEN and low current mana percent ({curmana}%)."
                    ));
                    curmana = 100.0;
                }
                if curhealth > 100.0 {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {first_entry}) with more than 100% health."
                    ));
                    curhealth = 100.0;
                }
                if curmana > 100.0 {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {first_entry}) with more than 100% mana."
                    ));
                    curmana = 100.0;
                }
            }
            if skip {
                continue;
            }

            let already_present = reload && self.creatures.contains_key(&guid);
            let mut creature_id = [0u32; MAX_CREATURE_IDS_PER_SPAWN];
            for (i, slot) in creature_id.iter_mut().enumerate() {
                *slot = f.u32(1 + i);
            }
            let mut data = CreatureSpawn {
                creature_id,
                position: WorldLocation {
                    map_id: f.u16(6),
                    x: f.f32(7),
                    y: f.f32(8),
                    z: f.f32(9),
                    o: f.f32(10),
                },
                spawntimesecsmin: f.u32(11),
                spawntimesecsmax: f.u32(12),
                wander_distance: f.f32(13),
                health_percent: curhealth,
                mana_percent: curmana,
                movement_type: f.u8(16),
                spawn_flags,
                visibility_mod: f.f32(21),
                instanciated_continent_instance_id: 0,
            };
            let game_event = f.i32(17) as i16;
            let guid_pool_id = f.i32(18) as i16;
            let entry_pool_id = f.i32(19) as i16;

            // Inserted before the map/existence tail, as in C++:
            // rows failing later checks stay in the store.
            let map_exists =
                map_ids.is_none_or(|set| set.contains(&u32::from(data.position.map_id)));
            if !map_exists {
                report.error(format!(
                    "Table `creature` have creature (GUID: {guid}) that spawned at nonexistent map (Id: {}), skipped.",
                    data.position.map_id
                ));
                self.creatures.insert(guid, data);
                continue;
            }

            if !exists_in_patch {
                data.spawn_flags |= SPAWN_FLAG_DISABLED;
            }
            if data.spawntimesecsmax < data.spawntimesecsmin {
                report.error(format!(
                    "Table `creature` have creature (GUID: {guid} Entry: {}) with `spawntimesecsmax` ({}) value lower than `spawntimesecsmin` ({}), it will be adjusted to {}.",
                    data.creature_id[0], data.spawntimesecsmax, data.spawntimesecsmin, data.spawntimesecsmin
                ));
                data.spawntimesecsmax = data.spawntimesecsmin;
            }
            if data.wander_distance < 0.0 {
                report.error(format!(
                    "Table `creature` have creature (GUID: {guid} Entry: {}) with `wander_distance`< 0, set to 0.",
                    data.creature_id[0]
                ));
                data.wander_distance = 0.0;
            } else if data.movement_type == 1 {
                // RANDOM_MOTION_TYPE
                if data.wander_distance == 0.0 {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {}) with `MovementType`=1 (random movement) but with `wander_distance`=0, replace by idle movement type (0).",
                        data.creature_id[0]
                    ));
                    data.movement_type = 0;
                }
            } else if data.movement_type == 0 {
                // IDLE_MOTION_TYPE
                if data.wander_distance != 0.0 {
                    report.error(format!(
                        "Table `creature` have creature (GUID: {guid} Entry: {}) with `MovementType`=0 (idle) have `wander_distance`<>0, set to 0.",
                        data.creature_id[0]
                    ));
                    data.wander_distance = 0.0;
                }
            }

            self.creatures.insert(guid, data);
            let data = &self.creatures[&guid];
            if !already_present
                && exists_in_patch
                && game_event == 0
                && guid_pool_id == 0
                && entry_pool_id == 0
            {
                let position = data.position;
                self.grid.add_creature(guid, &position);
            }
        }
        Ok(report)
    }

    /// `LoadGameobjects(reload)` (`ObjectMgr.cpp:2481-2631`).
    /// `go_display_ids` gates the display-exists check
    /// (`sGameObjectDisplayInfoStore`); `None` skips it (reported).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    #[allow(clippy::too_many_lines)] // one C++ loader body, kept in row order
    pub fn load_gameobjects(
        &mut self,
        conn: &mut mysql::Conn,
        wow_patch: u32,
        templates: &GameObjectTemplates,
        map_ids: Option<&HashSet<u32>>,
        go_display_ids: Option<&HashSet<u32>>,
        reload: bool,
    ) -> Result<LoadReport, DbError> {
        let rows: Vec<mysql::Row> = conn.exec(GAMEOBJECT_SELECT, ())?;
        let mut report = LoadReport::default();
        note_spawn_skips("gameobject", map_ids, &mut report);
        if go_display_ids.is_none() {
            report.error(
                "gameobject load: display existence validation skipped (no id set supplied)"
                    .to_string(),
            );
        }

        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            let guid = f.u32(0);
            let entry = f.u32(1);
            let mut patch_min = f.u8(20);
            let mut patch_max = f.u8(21);

            if patch_min > patch_max || patch_max > 10 {
                report.error(format!(
                    "Table `gameobject` GUID {guid} (entry {entry}) has invalid values patch_min={patch_min}, patch_max={patch_max}."
                ));
                patch_min = 0;
                patch_max = 10;
            }
            if !(wow_patch >= u32::from(patch_min) && wow_patch <= u32::from(patch_max)) {
                continue;
            }
            let Some(ginfo) = templates.get(entry) else {
                report.error(format!(
                    "Table `gameobject` has gameobject (GUID: {guid}) with non existing gameobject entry {entry}, skipped."
                ));
                continue;
            };
            if ginfo.display_id != 0 {
                if let Some(set) = go_display_ids {
                    if !set.contains(&ginfo.display_id) {
                        report.error(format!(
                            "Gameobject (GUID: {guid} Entry {entry} GoType: {}) have invalid displayId ({}), not loaded.",
                            ginfo.go_type, ginfo.display_id
                        ));
                        continue;
                    }
                }
            }

            let already_present = reload && self.gameobjects.contains_key(&guid);
            let mut data = GameObjectSpawn {
                id: entry,
                position: WorldLocation {
                    map_id: f.u16(2),
                    x: f.f32(3),
                    y: f.f32(4),
                    z: f.f32(5),
                    o: f.f32(6),
                },
                rotation0: f.f32(7),
                rotation1: f.f32(8),
                rotation2: f.f32(9),
                rotation3: f.f32(10),
                spawntimesecsmin: f.i32(11),
                spawntimesecsmax: f.i32(12),
                animprogress: f.u32(13),
                go_state: 0,
                spawn_flags: f.u32(18),
                visibility_mod: f.f32(19),
                instanciated_continent_instance_id: 0,
            };

            // Inserted before the validation tail, as in C++.
            let store = |this: &mut Self, data: GameObjectSpawn| {
                this.gameobjects.insert(guid, data);
            };
            let map_exists =
                map_ids.is_none_or(|set| set.contains(&u32::from(data.position.map_id)));
            if !map_exists {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) that spawned at nonexistent map (Id: {}), skip",
                    data.position.map_id
                ));
                store(self, data);
                continue;
            }
            if data.spawntimesecsmin == 0 && ginfo.is_despawn_at_action() {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) with `spawntimesecs` (0) value, but gameobejct marked as despawnable at action."
                ));
            }
            if data.spawntimesecsmax < data.spawntimesecsmin {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) with `spawntimesecsmax` ({}) value lower than `spawntimesecsmin` ({}), it will be adjusted to {}.",
                    data.spawntimesecsmax as u32, data.spawntimesecsmin as u32, data.spawntimesecsmin as u32
                ));
                data.spawntimesecsmax = data.spawntimesecsmin;
            }
            let go_state = f.u32(14);
            if go_state >= MAX_GO_STATE {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) with invalid `state` ({go_state}) value, skip"
                ));
                store(self, data);
                continue;
            }
            data.go_state = go_state;

            let game_event = f.i32(15) as i16;
            let guid_pool_id = f.i32(16) as i16;
            let entry_pool_id = f.i32(17) as i16;

            let mut bad_rotation: Option<(&str, f32)> = None;
            for (name, value) in [
                ("rotation0", data.rotation0),
                ("rotation1", data.rotation1),
                ("rotation2", data.rotation2),
                ("rotation3", data.rotation3),
            ] {
                if !(-1.0..=1.0).contains(&value) {
                    bad_rotation = Some((name, value));
                    break;
                }
            }
            if let Some((name, value)) = bad_rotation {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) with invalid {name} ({value}) value, skip"
                ));
                store(self, data);
                continue;
            }
            if !mangos_movement::movement_info::is_valid_map_coord(
                data.position.x,
                data.position.y,
                data.position.z,
                data.position.o,
            ) {
                report.error(format!(
                    "Table `gameobject` have gameobject (GUID: {guid} Entry: {entry}) with invalid coordinates, skip"
                ));
                store(self, data);
                continue;
            }

            let position = data.position;
            let is_elevator = ginfo.go_type == TYPE_TRANSPORT;
            store(self, data);
            if is_elevator {
                self.elevator_transports
                    .entry(u32::from(position.map_id))
                    .or_default()
                    .push(guid);
            } else if !already_present && game_event == 0 && guid_pool_id == 0 && entry_pool_id == 0
            {
                self.grid.add_gameobject(guid, &position);
            }
        }
        Ok(report)
    }
}

/// `SELECT entry FROM map_template` helper for the spawn
/// loaders' map-existence validation (`sMapStorage` keys).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_map_ids(conn: &mut mysql::Conn) -> Result<HashSet<u32>, DbError> {
    let ids: Vec<u32> = conn.exec_map("SELECT `entry` FROM `map_template`", (), |(id,)| id)?;
    Ok(ids.into_iter().collect())
}
