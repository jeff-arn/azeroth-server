//! Server-side movement (Slice 4): spline launch/integration
//! (`MoveSplineInit::Launch`, `Unit::UpdateSplineMovement`,
//! `Unit::StopMoving`), ground heights from `.map` files, and
//! the `MotionMaster` generator stack subset — Idle, Point,
//! Random, Waypoint, Home (Chase joins in the same slice's
//! second half).
//!
//! Recorded deviations at this stage (see
//! `PHASE4-PROGRESS.md`): pathfinding is straight-line with
//! ground-height clamping (Detour navmesh wiring lands with
//! Chase); `GetHeight` is GridMap-only (no vmap height);
//! walk/run toggle side packets ride with the speed/ACK
//! work; waypoint `path_id` special sub-paths and Cyclic
//! closed-loop splines are treated as plain repeating
//! waypoint movement.

use std::collections::HashMap;
use std::path::PathBuf;

use mangos_assets::gridmap::{GridMap, INVALID_HEIGHT, gridmap_filename};
use mangos_detour::{NavMesh, NavMeshQuery, nav_terrain};
use mangos_entities::unit::{
    MOVE_RUN, MOVE_WALK, UNIT_STATE_ALL_DYN_STATES, UNIT_STATE_CAN_NOT_MOVE, UNIT_STATE_CHASE,
    UNIT_STATE_CHASE_MOVE, UNIT_STATE_DISTRACTED, UNIT_STATE_MOVING, UNIT_STATE_ROAMING,
    UNIT_STATE_ROAMING_MOVE, UNIT_STATE_RUNNING, UnitBase,
};
use mangos_guid::ObjectGuid;
use mangos_guid::timer::ShortTimeTracker;
use mangos_movement::movement_info::{MovementFlags, Position, is_valid_map_coord};
use mangos_movement::spline::Vec3;
use mangos_movement::{FacingInfo, MoveSplineFlag, MoveSplineInitArgs, packet_builder};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as opcodes;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::waypoints::WaypointPath;

use crate::map::{Map, MapEvent};

/// `MovementGeneratorType` values used here
/// (`MotionMaster.h:36-59`).
pub const IDLE_MOTION_TYPE: u32 = 0;
pub const RANDOM_MOTION_TYPE: u32 = 1;
pub const WAYPOINT_MOTION_TYPE: u32 = 2;
pub const CYCLIC_MOTION_TYPE: u32 = 3;
pub const CHASE_MOTION_TYPE: u32 = 6;
pub const HOME_MOTION_TYPE: u32 = 7;
pub const POINT_MOTION_TYPE: u32 = 9;

/// C++ `uint32` millisecond diffs flow into `int32` timers
/// with the wrapping cast; diffs are frame times, so the
/// wrap case never occurs in practice.
#[allow(clippy::cast_possible_wrap)]
const fn ms_i32(ms: u32) -> i32 {
    ms as i32
}

/// `BASE_MELEERANGE_OFFSET` (`UnitDefines.h:70`).
const BASE_MELEE_RANGE_OFFSET: f32 = 1.333_333_4;
/// `ATTACK_DISTANCE` (`ObjectDefines.h:25`).
const ATTACK_DISTANCE: f32 = 5.0;

/// `moveTypeToOpcode[type][0]` at 1.12.1 (spline forms).
const SPLINE_SPEED_OPCODES: [u16; 6] = [
    opcodes::SMSG_SPLINE_SET_WALK_SPEED,
    opcodes::SMSG_SPLINE_SET_RUN_SPEED,
    opcodes::SMSG_SPLINE_SET_RUN_BACK_SPEED,
    opcodes::SMSG_SPLINE_SET_SWIM_SPEED,
    opcodes::SMSG_SPLINE_SET_SWIM_BACK_SPEED,
    opcodes::SMSG_SPLINE_SET_TURN_RATE,
];
/// `moveTypeToOpcode[type][1]` (force-to-controller forms).
const FORCE_SPEED_OPCODES: [u16; 6] = [
    opcodes::SMSG_FORCE_WALK_SPEED_CHANGE,
    opcodes::SMSG_FORCE_RUN_SPEED_CHANGE,
    opcodes::SMSG_FORCE_RUN_BACK_SPEED_CHANGE,
    opcodes::SMSG_FORCE_SWIM_SPEED_CHANGE,
    opcodes::SMSG_FORCE_SWIM_BACK_SPEED_CHANGE,
    opcodes::SMSG_FORCE_TURN_RATE_CHANGE,
];
/// `moveTypeToOpcode[type][2]` (observer move-set forms).
const MOVE_SET_SPEED_OPCODES: [u16; 6] = [
    opcodes::MSG_MOVE_SET_WALK_SPEED,
    opcodes::MSG_MOVE_SET_RUN_SPEED,
    opcodes::MSG_MOVE_SET_RUN_BACK_SPEED,
    opcodes::MSG_MOVE_SET_SWIM_SPEED,
    opcodes::MSG_MOVE_SET_SWIM_BACK_SPEED,
    opcodes::MSG_MOVE_SET_TURN_RATE,
];

/// (apply, remove) opcode pair.
type OpcodePair = (u16, u16);

/// Flag-change opcode triples per movement flag:
/// (change type, controller pair, observer pair, spline
/// pair).
fn flag_change_tables(flag: u32) -> Option<(u8, OpcodePair, OpcodePair, OpcodePair)> {
    use mangos_entities::unit::{
        MOVEMENT_CHANGE_FEATHER_FALL, MOVEMENT_CHANGE_ROOT, MOVEMENT_CHANGE_SET_HOVER,
        MOVEMENT_CHANGE_WATER_WALK,
    };
    Some(match flag {
        MovementFlags::ROOT => (
            MOVEMENT_CHANGE_ROOT,
            (
                opcodes::SMSG_FORCE_MOVE_ROOT,
                opcodes::SMSG_FORCE_MOVE_UNROOT,
            ),
            (opcodes::MSG_MOVE_ROOT, opcodes::MSG_MOVE_UNROOT),
            (
                opcodes::SMSG_SPLINE_MOVE_ROOT,
                opcodes::SMSG_SPLINE_MOVE_UNROOT,
            ),
        ),
        MovementFlags::WATER_WALKING => (
            MOVEMENT_CHANGE_WATER_WALK,
            (opcodes::SMSG_MOVE_WATER_WALK, opcodes::SMSG_MOVE_LAND_WALK),
            // C++ observer form is MSG_MOVE_WATER_WALK for
            // both apply and remove (kept as-is).
            (opcodes::MSG_MOVE_WATER_WALK, opcodes::MSG_MOVE_WATER_WALK),
            (
                opcodes::SMSG_SPLINE_MOVE_WATER_WALK,
                opcodes::SMSG_SPLINE_MOVE_LAND_WALK,
            ),
        ),
        MovementFlags::HOVER => (
            MOVEMENT_CHANGE_SET_HOVER,
            (opcodes::SMSG_MOVE_SET_HOVER, opcodes::SMSG_MOVE_UNSET_HOVER),
            (opcodes::MSG_MOVE_HOVER, opcodes::MSG_MOVE_HOVER),
            (
                opcodes::SMSG_SPLINE_MOVE_SET_HOVER,
                opcodes::SMSG_SPLINE_MOVE_UNSET_HOVER,
            ),
        ),
        MovementFlags::SAFE_FALL => (
            MOVEMENT_CHANGE_FEATHER_FALL,
            (
                opcodes::SMSG_MOVE_FEATHER_FALL,
                opcodes::SMSG_MOVE_NORMAL_FALL,
            ),
            (
                opcodes::MSG_MOVE_FEATHER_FALL,
                opcodes::MSG_MOVE_FEATHER_FALL,
            ),
            (
                opcodes::SMSG_SPLINE_MOVE_FEATHER_FALL,
                opcodes::SMSG_SPLINE_MOVE_NORMAL_FALL,
            ),
        ),
        _ => return None,
    })
}

/// `MoveOptions` (`MotionMaster.h:71-79`, subset).
pub const MOVE_PATHFINDING: u32 = 0x001;
pub const MOVE_WALK_MODE: u32 = 0x002;
pub const MOVE_RUN_MODE: u32 = 0x004;
pub const MOVE_EXCLUDE_STEEP_SLOPES: u32 = 0x080;

// ---------------------------------------------------------------------------
// Ground heights

/// Lazily-loaded `.map` height grids plus the
/// vmap host for one map id (`TerrainInfo` +
/// `VMapManager2` halves of `Map::GetHeight`).
pub struct Heights {
    map_id: u32,
    dir: Option<PathBuf>,
    grids: HashMap<(i32, i32), Option<GridMap>>,
    vmaps: crate::vmaps::VmapHost,
}

impl Heights {
    #[must_use]
    pub fn new(map_id: u32, dir: Option<PathBuf>) -> Heights {
        Heights {
            map_id,
            dir,
            grids: HashMap::new(),
            vmaps: crate::vmaps::VmapHost::new(map_id),
        }
    }

    pub fn set_vmaps_dir(&mut self, dir: Option<PathBuf>, enable_los: bool, enable_height: bool) {
        self.vmaps.set_dir(dir, enable_los, enable_height);
    }

    pub(crate) fn vmaps(&mut self) -> &mut crate::vmaps::VmapHost {
        &mut self.vmaps
    }

    /// Tile pair for a world position
    /// (`TerrainInfo::GetGrid`, `GridMap.cpp:1107-1110` —
    /// deliberately reversed: grid x derives from world
    /// y). File names print `(map, ty, tx)`
    /// (`LoadMapAndVMap` / `MMapManager::loadMap`).
    #[allow(clippy::cast_possible_truncation, clippy::excessive_precision)] // C++ constants
    pub(crate) fn tile_pair(x: f32, y: f32) -> (i32, i32) {
        const SIZE_OF_GRIDS: f32 = 533.333_33;
        let tx = (32.0 - y / SIZE_OF_GRIDS) as i32;
        let ty = (32.0 - x / SIZE_OF_GRIDS) as i32;
        (tx, ty)
    }

    pub(crate) fn grid(&mut self, x: f32, y: f32) -> Option<&GridMap> {
        let key = Self::tile_pair(x, y);
        if !self.grids.contains_key(&key) {
            let loaded = self.dir.as_ref().and_then(|dir| {
                // Filenames print (map, ty, tx).
                #[allow(clippy::cast_sign_loss)]
                let path = dir.join(gridmap_filename(self.map_id, key.1 as u32, key.0 as u32));
                let bytes = std::fs::read(path).ok()?;
                GridMap::parse(&bytes).ok()
            });
            self.grids.insert(key, loaded);
        }
        self.grids.get(&key).and_then(Option::as_ref)
    }

    /// `Map::GetHeight(x, y, zHint)` `GridMap` subset: the
    /// grid height when the tile is loaded and the height
    /// is not the invalid sentinel, else the hint.
    pub fn ground_z(&mut self, x: f32, y: f32, z_hint: f32) -> f32 {
        let map_height = self.grid(x, y).map(|grid| grid.height(x, y));
        if self.vmaps.height_enabled() {
            // TerrainInfo::GetHeightStatic (GridMap.cpp:806)
            // transcription. The dynamic-tree max of
            // Map::GetHeight is not modeled (recorded).
            use mangos_assets::gridmap::INVALID_HEIGHT_VALUE;
            /// `DEFAULT_HEIGHT_SEARCH`.
            const DEFAULT_HEIGHT_SEARCH: f32 = 10.0;
            let mut map_h = INVALID_HEIGHT_VALUE;
            if let Some(h) = map_height {
                map_h = h;
            }
            let z2 = z_hint + 2.0;
            let mut search = DEFAULT_HEIGHT_SEARCH;
            if map_h > INVALID_HEIGHT && z2 - map_h > search {
                search = z2 - map_h + 1.0;
            }
            let mut vmap_h = self.vmaps.vmap_height(x, y, z2, search);
            if vmap_h <= INVALID_HEIGHT {
                vmap_h = self.vmaps.vmap_height(x, y, z2, 10_000.0);
            }
            if vmap_h <= INVALID_HEIGHT && map_h > z2 && (z2 - map_h).abs() > 30.0 {
                vmap_h = self.vmaps.vmap_height(x, y, z2, -search);
            }
            if vmap_h <= INVALID_HEIGHT && map_h > INVALID_HEIGHT && z2 < map_h {
                vmap_h = self
                    .vmaps
                    .vmap_height(x, y, map_h + 2.0, DEFAULT_HEIGHT_SEARCH);
            }
            let out = if vmap_h > INVALID_HEIGHT {
                if map_h > INVALID_HEIGHT && z_hint >= map_h && vmap_h <= map_h {
                    map_h
                } else {
                    vmap_h
                }
            } else {
                map_h
            };
            // Callers here expect a usable z; the fork
            // returns the sentinel and lets its callers
            // cope (UpdateAllowedPositionZ clamps via
            // GetHeight's VALID_HEIGHT check upstream).
            return if out > INVALID_HEIGHT { out } else { z_hint };
        }
        match map_height {
            Some(h) => {
                // No vmap height calc: a grid height more
                // than DEFAULT_HEIGHT_SEARCH (10) above the
                // hint is treated as not applicable and the
                // hint (navmesh/spawn Z) stands — dungeons
                // sit under outdoor terrain. (The fork
                // without vmap height is documented broken;
                // this keeps vmap-less worlds usable.)
                if h > INVALID_HEIGHT && h <= z_hint + 10.0 {
                    h
                } else {
                    z_hint
                }
            }
            None => z_hint,
        }
    }
}

// ---------------------------------------------------------------------------
// Navmesh pathfinding (PathInfo subset)

/// Outcome class of a path query (`PathType` subset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathKind {
    /// `PATHFIND_NORMAL` (end poly reached).
    Normal,
    /// `PATHFIND_INCOMPLETE` (partial corridor).
    Incomplete,
    /// `PATHFIND_NOPATH` (points are the shortcut).
    NoPath,
    /// `PATHFIND_NORMAL | PATHFIND_NOT_USING_PATH`
    /// (no mesh/tiles: straight shortcut).
    Shortcut,
}

pub(crate) struct PathResult {
    pub points: Vec<Position>,
    pub kind: PathKind,
}

/// Per-map Detour host: the `.mmap` params mesh plus
/// lazily loaded `.mmtile` tiles (3x3 grids around query
/// points, the lazy analogue of grid-driven tile loads).
pub struct NavHost {
    map_id: u32,
    dir: Option<PathBuf>,
    mesh: Option<NavMesh>,
    attempted: std::collections::HashSet<(i32, i32)>,
}

impl NavHost {
    #[must_use]
    pub fn new(map_id: u32, dir: Option<PathBuf>) -> NavHost {
        let mesh = dir.as_ref().and_then(|dir| {
            let bytes = std::fs::read(dir.join(format!("{map_id:03}.mmap"))).ok()?;
            NavMesh::from_mmap_params(&bytes)
        });
        NavHost {
            map_id,
            dir,
            mesh,
            attempted: std::collections::HashSet::new(),
        }
    }

    fn ensure_tiles(&mut self, x: f32, y: f32) {
        let (Some(dir), Some(mesh)) = (self.dir.clone(), self.mesh.as_mut()) else {
            return;
        };
        let (gx, gy) = Heights::tile_pair(x, y);
        for tx in (gx - 1).max(0)..=(gx + 1).min(63) {
            for ty in (gy - 1).max(0)..=(gy + 1).min(63) {
                if !self.attempted.insert((tx, ty)) {
                    continue;
                }
                // Filenames print (map, ty, tx).
                let path = dir.join(format!("{:03}{:02}{:02}.mmtile", self.map_id, ty, tx));
                if let Ok(bytes) = std::fs::read(&path) {
                    let _ = mesh.load_tile_file(&bytes);
                }
            }
        }
    }

    fn has_tile(&mut self, pos: Position) -> bool {
        self.ensure_tiles(pos.x, pos.y);
        self.mesh.as_ref().is_some_and(|mesh| {
            mesh.has_tile_at_position(Vec3 {
                x: pos.x,
                y: pos.y,
                z: pos.z,
            })
        })
    }
}

impl Map {
    /// Points the navmesh host at the extracted `mmaps`
    /// directory (binary boot).
    pub fn set_mmaps_dir(&mut self, dir: Option<PathBuf>) {
        self.navhost = NavHost::new(self.map_id, dir);
    }

    /// Whether a navmesh tile covers `pos` (loads the
    /// surrounding tiles on first use). Test/ops probe
    /// for the lazy tile loader.
    pub fn navmesh_active_at(&mut self, pos: Position) -> bool {
        self.navhost.has_tile(pos)
    }

    fn shortcut(&mut self, from: Position, to: Position, kind: PathKind) -> PathResult {
        PathResult {
            points: self.build_path_straight(from, to),
            kind,
        }
    }

    /// `PathInfo::calculate` + `BuildPolyPath` +
    /// `BuildPointPath` subset: no poly-path reuse across
    /// calls (C++ callers construct a fresh `PathFinder`
    /// per launch too), straight-path point extraction
    /// instead of `findSmoothPath`, creature filter
    /// (walk + swim liquids), no swim/fly shortcut arms,
    /// no transports. Heights come from the `GridMap`
    /// subset of `UpdateAllowedPositionZ`.
    #[allow(clippy::too_many_lines)] // one PathInfo::calculate port
    pub(crate) fn find_path_points(
        &mut self,
        from: Position,
        to: Position,
        force_dest: bool,
        exclude_steep: bool,
    ) -> PathResult {
        enum NavOutcome {
            Shortcut,
            NoPath,
            Points(Vec<Vec3>, PathKind),
        }
        let outcome = {
            if self.navhost.mesh.is_none()
                || !self.navhost.has_tile(from)
                || !self.navhost.has_tile(to)
            {
                NavOutcome::Shortcut
            } else if let Some(mesh) = self.navhost.mesh.as_ref() {
                let include = nav_terrain::GROUND
                    | nav_terrain::WATER
                    | nav_terrain::MAGMA
                    | nav_terrain::SLIME;
                let exclude = if exclude_steep {
                    nav_terrain::STEEP_SLOPES
                } else {
                    0
                };
                let start = Vec3 {
                    x: from.x,
                    y: from.y,
                    z: from.z,
                };
                let mut end = Vec3 {
                    x: to.x,
                    y: to.y,
                    z: to.z,
                };
                match NavMeshQuery::new(mesh, mangos_detour::DEFAULT_MAX_NODES) {
                    None => NavOutcome::Shortcut,
                    Some(query) => {
                        match (
                            query.find_walk_poly(start, include, exclude, 10.0),
                            query.find_walk_poly(end, include, exclude, 10.0),
                        ) {
                            (Some((start_poly, _, dist_start)), Some((end_poly, _, dist_end))) => {
                                let mut kind = PathKind::Normal;
                                if dist_start > 7.0 || dist_end > 7.0 {
                                    let (_, boundary) =
                                        query.closest_point_on_poly_boundary(end_poly, end);
                                    end = boundary;
                                    kind = PathKind::Incomplete;
                                }
                                let (status, polys) = query.find_path(
                                    start_poly,
                                    end_poly,
                                    start,
                                    end,
                                    include,
                                    exclude,
                                    mangos_detour::MAX_PATH_LENGTH,
                                );
                                if status.failed() || polys.is_empty() {
                                    NavOutcome::NoPath
                                } else {
                                    if polys.last() != Some(&end_poly) {
                                        kind = PathKind::Incomplete;
                                    }
                                    let (status, raw) =
                                        query.find_straight_path(start, end, &polys, 256);
                                    if status.failed() || raw.len() < 2 {
                                        NavOutcome::NoPath
                                    } else {
                                        NavOutcome::Points(raw, kind)
                                    }
                                }
                            }
                            _ => NavOutcome::NoPath,
                        }
                    }
                }
            } else {
                NavOutcome::Shortcut
            }
        };

        let (raw_points, kind) = match outcome {
            NavOutcome::Shortcut => return self.shortcut(from, to, PathKind::Shortcut),
            NavOutcome::NoPath => return self.shortcut(from, to, PathKind::NoPath),
            NavOutcome::Points(raw, kind) => (raw, kind),
        };
        let mut points: Vec<Position> = raw_points
            .iter()
            .map(|p| Position {
                x: p.x,
                y: p.y,
                z: self.ground_z(p.x, p.y, p.z),
                o: 0.0,
            })
            .collect();
        // Force-destination tail (BuildPointPath): keep a
        // partial subpath that got most of the way, else
        // fall back to the shortcut.
        if force_dest {
            let actual_end = *points.last().expect(">=2 points");
            let near = dist3d(actual_end, to) <= 1.0;
            if kind != PathKind::Normal || !near {
                let partial = dist3d(actual_end, to).powi(2) < 0.3 * dist3d(from, to).powi(2);
                if partial {
                    if let Some(last) = points.last_mut() {
                        *last = to;
                    }
                } else {
                    return self.shortcut(from, to, kind);
                }
            }
        }
        PathResult { points, kind }
    }

    /// Two-point straight path with ground clamping (the
    /// pre-navmesh behaviour; also `BuildShortcut`).
    pub(crate) fn build_path_straight(&mut self, from: Position, to: Position) -> Vec<Position> {
        let z = self.ground_z(to.x, to.y, to.z);
        vec![
            from,
            Position {
                x: to.x,
                y: to.y,
                z,
                o: to.o,
            },
        ]
    }

    /// Path for a spline launch: navmesh when
    /// `pathfinding` is set (shortcut points on failure,
    /// exactly like C++ `Move(&path)` consuming a
    /// shortcut), straight line otherwise.
    pub(crate) fn build_path(
        &mut self,
        from: Position,
        to: Position,
        pathfinding: bool,
        exclude_steep: bool,
    ) -> Vec<Position> {
        if !pathfinding {
            return self.build_path_straight(from, to);
        }
        self.find_path_points(from, to, false, exclude_steep).points
    }
}

// ---------------------------------------------------------------------------
// Spline launch

/// One `MoveSplineInit` configuration (`Launch` inputs).
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // mirrors the MoveSplineInit setters
pub struct SplineLaunch {
    /// Path points; element 0 is replaced with the unit's
    /// real position at launch, as in C++.
    pub path: Vec<Position>,
    pub walk: bool,
    pub flying: bool,
    pub cyclic: bool,
    pub falling: bool,
    pub facing: Option<FacingInfo>,
    /// 0 = pick from movement flags (`SelectSpeedType`).
    pub velocity: f32,
    /// `SetFirstPointId` (packet chunking starts here for
    /// the launch packet; C++ default firstPoint is 1).
    pub first_point_id: i32,
    /// `SetStop()` — emits the stop form of `MONSTER_MOVE`.
    pub done: bool,
}

impl Default for SplineLaunch {
    fn default() -> Self {
        SplineLaunch {
            path: Vec::new(),
            walk: false,
            flying: false,
            cyclic: false,
            falling: false,
            facing: None,
            velocity: 0.0,
            first_point_id: 1,
            done: false,
        }
    }
}

impl Map {
    /// Read access to any unit on the map.
    pub(crate) fn unit_base(&self, guid: ObjectGuid) -> Option<&UnitBase> {
        if let Some(entry) = self.players.get(&guid) {
            return Some(&entry.player.unit);
        }
        self.creatures.get(&guid).map(|c| &c.unit)
    }

    /// Mutable access to any unit on the map.
    pub(crate) fn unit_base_mut(&mut self, guid: ObjectGuid) -> Option<&mut UnitBase> {
        if let Some(entry) = self.players.get_mut(&guid) {
            return Some(&mut entry.player.unit);
        }
        self.creatures.get_mut(&guid).map(|c| &mut c.unit)
    }

    pub(crate) fn unit_position(&self, guid: ObjectGuid) -> Option<Position> {
        if let Some(entry) = self.players.get(&guid) {
            return Some(entry.player.unit.world.position());
        }
        self.creatures.get(&guid).map(|c| c.unit.world.position())
    }

    /// `SendMovementMessageToSet(packet, self)`: every
    /// player who sees `guid`, plus `guid` itself when it
    /// is a player (the C++ `self` arm).
    pub(crate) fn broadcast_movement(&mut self, guid: ObjectGuid, packet: &WorldPacket) {
        let mut targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(g, e)| **g == guid || e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        targets.sort_by_key(|g| g.raw());
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// Same-map teleport for host-driven placement
    /// (bot director relocation): cell-index move
    /// only; session-less bots need no client
    /// teleport packet train.
    pub fn teleport_unit_within_map(&mut self, guid: ObjectGuid, position: Position) {
        self.relocate_unit(guid, position);
    }

    /// Player near teleport (`Player::ExecuteTeleportNear`
    /// plus `SendTeleportToController`/`ToObservers`):
    /// relocate now, push the pending TELEPORT change,
    /// `MSG_MOVE_TELEPORT_ACK` (packed guid + counter +
    /// server-side movement info at the destination) to
    /// the controller and `MSG_MOVE_TELEPORT` (no
    /// counter) to observers.
    /// Creature near teleport (`Unit::NearTeleportTo`
    /// creature arm): `SendTeleportToObservers`
    /// (`MSG_MOVE_TELEPORT` with server-side movement
    /// info at the destination) before **and** after
    /// `CreatureRelocation`, exactly as the fork
    /// sends it. Motion-master Interrupt/Reset
    /// collapses to a stop here (recorded).
    pub fn teleport_creature_in_map(&mut self, guid: ObjectGuid, position: Position) {
        self.stop_moving(guid);
        let packet = {
            let now_ms = self.now_ms;
            let Some(mut info) = self.unit_base_mut(guid).map(|unit| {
                let mut info = unit.world.movement_info;
                info.set_as_server_side(now_ms);
                info.change_position(position);
                info
            }) else {
                return;
            };
            info.change_position(position);
            let mut payload = PacketWriter::new();
            payload.write_bytes(&Self::packed_guid_bytes(guid));
            info.write(&mut payload);
            WorldPacket::with_body(
                mangos_protocol::opcode_table::opcode_id::MSG_MOVE_TELEPORT,
                payload.into_vec(),
            )
        };
        self.broadcast_movement(guid, &packet);
        self.relocate_unit(guid, position);
        self.broadcast_movement(guid, &packet);
    }

    pub fn teleport_player_in_map(&mut self, guid: ObjectGuid, position: Position) {
        self.relocate_unit(guid, position);
        let now_ms = self.now_ms;
        let Some((counter, mut info)) = self.unit_base_mut(guid).map(|unit| {
            let counter = unit.movement_counter_and_inc();
            unit.pending_movement_changes
                .push_back(mangos_entities::unit::PendingMovementChange {
                    movement_counter: counter,
                    change_type: mangos_entities::unit::MOVEMENT_CHANGE_TELEPORT,
                    new_value: 0.0,
                    apply: false,
                    knockback: None,
                });
            let mut info = unit.world.movement_info;
            info.set_as_server_side(now_ms);
            info.change_position(position);
            unit.world.movement_info = info;
            (counter, info)
        }) else {
            return;
        };
        info.change_position(position);
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(guid));
        payload.write_u32_le(counter);
        info.write(&mut payload);
        let packet = WorldPacket::with_body(
            mangos_protocol::opcode_table::opcode_id::MSG_MOVE_TELEPORT_ACK,
            payload.into_vec(),
        );
        self.events.push(MapEvent::Packet { to: guid, packet });
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(guid));
        info.write(&mut payload);
        let packet = WorldPacket::with_body(
            mangos_protocol::opcode_table::opcode_id::MSG_MOVE_TELEPORT,
            payload.into_vec(),
        );
        self.broadcast_movement_except(guid, &packet, guid);
    }

    /// Relocates a unit, moving its cell index entry.
    pub(crate) fn relocate_unit(&mut self, guid: ObjectGuid, position: Position) {
        let Some(old) = self.unit_position(guid) else {
            return;
        };
        let old_cell = Self::cell_of(old);
        let new_cell = Self::cell_of(position);
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.world.relocate(position);
            unit.world.movement_info.pos = position;
        }
        if old_cell != new_cell {
            let is_player = guid.is_player();
            if let Some(cell) = self.cells.get_mut(&old_cell) {
                if is_player {
                    cell.players.remove(&guid);
                } else {
                    cell.creatures.remove(&guid);
                }
            }
            let cell = self.cells.entry(new_cell).or_default();
            if is_player {
                cell.players.insert(guid);
            } else {
                cell.creatures.insert(guid);
            }
            // Watchers re-evaluate at the next scan; mark
            // every player's scan due when a mover crosses
            // cells (cheap approximation of the C++
            // relocation notification at this scale).
            for entry in self.players.values_mut() {
                entry.scan_due = true;
            }
        }
        // Server-side player relocation shares the fork
        // `Player::SetPosition` discovery check.
        #[allow(clippy::float_cmp)]
        if guid.is_player() && (old.x != position.x || old.y != position.y || old.z != position.z) {
            self.check_area_explore(guid);
        }
    }

    /// `Map::GetHeight` (`GridMap` + vmap static
    /// tree; dynamic-tree max not modeled).
    pub fn ground_z(&mut self, x: f32, y: f32, z_hint: f32) -> f32 {
        self.heights.ground_z(x, y, z_hint)
    }

    /// `Map::IsWithinLOS` static half
    /// (`VMapManager2::isInLineOfSight`).
    #[allow(clippy::too_many_arguments)]
    pub fn is_in_line_of_sight(
        &mut self,
        x1: f32,
        y1: f32,
        z1: f32,
        x2: f32,
        y2: f32,
        z2: f32,
        ignore_m2: bool,
    ) -> bool {
        self.heights
            .vmaps()
            .is_in_line_of_sight(x1, y1, z1, x2, y2, z2, ignore_m2)
    }

    /// `Map::GetLosHitPosition` static half
    /// (dynamic gameobject tree not modeled):
    /// trimmed destination on hit.
    #[allow(clippy::too_many_arguments)]
    pub fn get_los_hit_position(
        &mut self,
        x1: f32,
        y1: f32,
        z1: f32,
        x2: f32,
        y2: f32,
        z2: f32,
        modify_dist: f32,
    ) -> Option<(f32, f32, f32)> {
        self.heights
            .vmaps()
            .get_object_hit_pos(x1, y1, z1, x2, y2, z2, modify_dist)
    }

    /// `MoveSplineInit::Launch` (`MoveSplineInit.cpp:65-191`,
    /// transport arms excluded). Returns the spline
    /// duration in ms (0 when the unit is unknown).
    pub fn launch_spline(&mut self, guid: ObjectGuid, launch: SplineLaunch) -> i32 {
        let spline_id = self.spline_counter;
        self.spline_counter = self.spline_counter.wrapping_add(1);
        let Some(unit) = self.unit_base_mut(guid) else {
            return 0;
        };

        // Real position: mid-spline launches start from the
        // evaluated spline position.
        let real = if unit.spline.finalized() {
            let p = unit.world.position();
            Vec3 {
                x: p.x,
                y: p.y,
                z: p.z,
            }
        } else {
            unit.spline.compute_position().pos
        };

        let (done, first_point_id) = (launch.done, launch.first_point_id);
        let (args, move_flags) = Self::prepare_launch(unit, launch, real, spline_id);

        let is_player = guid.is_player();
        if is_player {
            unit.pending_spline_done = true;
        }
        unit.world.movement_info.client_time = 0;
        unit.world.movement_info.flags = move_flags;
        unit.spline.initialize(&args);

        let packet = Self::monster_move_packet(unit, guid, real, spline_id, done, first_point_id);
        let duration = unit.spline.duration();
        self.broadcast_movement(guid, &packet);
        duration
    }

    /// Builds the spline init args + adjusted movement
    /// flags for a launch (the `Launch` pre-packet half:
    /// first-vertex correction, flag mixing, velocity
    /// selection).
    fn prepare_launch(
        unit: &UnitBase,
        launch: SplineLaunch,
        real: Vec3,
        spline_id: u32,
    ) -> (MoveSplineInitArgs, MovementFlags) {
        let mut path = launch.path;
        if path.is_empty() {
            // Launch calls MoveTo(real_position), which
            // sizes the path to 2 (Validate requires > 1).
            let here = Position {
                x: real.x,
                y: real.y,
                z: real.z,
                o: 0.0,
            };
            path.push(here);
            path.push(here);
        } else if path.len() == 1 {
            path.push(path[0]);
        }
        path[0] = Position {
            x: real.x,
            y: real.y,
            z: real.z,
            o: path[0].o,
        };

        let mut move_flags = unit.world.movement_info.flags;
        let mut flags = MoveSplineFlag::default();
        if launch.done {
            flags.0 |= MoveSplineFlag::DONE;
            move_flags.remove(MovementFlags::SPLINE_ENABLED | MovementFlags::MASK_MOVING);
        } else {
            move_flags.add(MovementFlags::SPLINE_ENABLED | MovementFlags::FORWARD);
            if launch.walk {
                move_flags.add(MovementFlags::WALK_MODE);
            } else {
                move_flags.remove(MovementFlags::WALK_MODE);
                flags.0 |= MoveSplineFlag::RUNMODE;
            }
            if launch.flying {
                flags.0 |= MoveSplineFlag::FLYING;
            }
            if launch.cyclic {
                flags.0 |= MoveSplineFlag::CYCLIC;
            }
            if launch.falling {
                flags.0 |= MoveSplineFlag::FALLING;
            }
        }
        // No transports yet: ONTRANSPORT always cleared
        // (the `newTransport == nullptr` arm).
        move_flags.remove(MovementFlags::ON_TRANSPORT);

        let velocity = if launch.velocity == 0.0 {
            if move_flags.has(MovementFlags::WALK_MODE) {
                unit.speed(MOVE_WALK)
            } else {
                unit.speed(MOVE_RUN)
            }
        } else {
            launch.velocity
        };

        let mut args = MoveSplineInitArgs {
            path: path
                .iter()
                .map(|p| Vec3 {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                })
                .collect(),
            flags,
            velocity,
            spline_id,
            ..MoveSplineInitArgs::default()
        };
        if let Some(facing) = launch.facing {
            args.facing = facing;
            match facing {
                FacingInfo::Angle(_) => args.flags.enable_facing_angle(),
                FacingInfo::Target(_) => args.flags.enable_facing_target(),
                FacingInfo::Point(_) => args.flags.enable_facing_point(),
            }
        }
        (args, move_flags)
    }

    /// The `SMSG_MONSTER_MOVE` body for a launch: packed
    /// GUID + stop form or the monster-move chunk.
    fn monster_move_packet(
        unit: &mut UnitBase,
        guid: ObjectGuid,
        real: Vec3,
        spline_id: u32,
        done: bool,
        first_point_id: i32,
    ) -> WorldPacket {
        let mut guid_bytes = Vec::new();
        guid.write_packed(&mut guid_bytes);
        let mut payload = PacketWriter::new();
        payload.write_bytes(&guid_bytes);
        if done {
            payload.write_f32_le(real.x);
            payload.write_f32_le(real.y);
            payload.write_f32_le(real.z);
            payload.write_u32_le(spline_id);
            payload.write_u8(1); // MonsterMoveStop
        } else {
            let last = packet_builder::write_monster_move(
                &mut payload,
                &unit.spline,
                first_point_id,
                packet_builder::DEFAULT_MAX_POINTS_PER_PACKET,
            );
            unit.spline.set_last_point_sent(last);
        }
        WorldPacket::with_body(opcodes::SMSG_MONSTER_MOVE, payload.into_vec())
    }

    /// `Unit::StopMoving` (`Unit.cpp:9136-9162`, non-force
    /// arm; uninterruptible splines do not exist yet).
    pub fn stop_moving(&mut self, guid: ObjectGuid) {
        let Some(unit) = self.unit_base_mut(guid) else {
            return;
        };
        unit.clear_unit_state(UNIT_STATE_MOVING);
        unit.world
            .movement_info
            .flags
            .remove(MovementFlags::MASK_MOVING);
        if unit.spline.finalized() {
            return;
        }
        let _ = self.launch_spline(
            guid,
            SplineLaunch {
                done: true,
                ..SplineLaunch::default()
            },
        );
        if let Some(unit) = self.unit_base_mut(guid) {
            Self::disable_spline(unit);
        }
    }

    /// `Unit::DisableSpline` (`Unit.cpp:10743-10750`).
    pub(crate) fn disable_spline(unit: &mut UnitBase) {
        unit.world
            .movement_info
            .flags
            .remove(MovementFlags::SPLINE_ENABLED | MovementFlags::FORWARD);
        unit.world.movement_info.client_time = 0;
        unit.spline.interrupt();
    }

    /// `Unit::UpdateSplineMovement` for every unit
    /// (`Unit.cpp:10674-10730`, transport arm excluded).
    pub(crate) fn update_spline_movement(&mut self, diff_ms: u32) {
        let mut guids: Vec<ObjectGuid> = self.creatures.keys().copied().collect();
        guids.extend(self.players.keys().copied());
        guids.sort_by_key(|g| g.raw());
        for guid in guids {
            let mut continuation: Option<WorldPacket> = None;
            let mut new_position: Option<Position> = None;
            if let Some(unit) = self.unit_base_mut(guid) {
                if unit.spline.finalized() {
                    continue;
                }
                unit.spline.update_state(ms_i32(diff_ms));
                if unit.spline.finalized() {
                    Self::disable_spline(unit);
                    if unit.pending_spline_done && !guid.is_player() {
                        unit.pending_spline_done = false;
                    }
                } else if !unit.spline.is_cyclic()
                    && unit.spline.last_point_sent() >= 0
                    && unit.spline.last_point_sent() < unit.spline.current_path_idx() + 3
                {
                    let mut guid_bytes = Vec::new();
                    guid.write_packed(&mut guid_bytes);
                    let mut payload = PacketWriter::new();
                    payload.write_bytes(&guid_bytes);
                    let next = unit.spline.last_point_sent() + 1;
                    let last = packet_builder::write_monster_move(
                        &mut payload,
                        &unit.spline,
                        next,
                        packet_builder::DEFAULT_MAX_POINTS_PER_PACKET,
                    );
                    unit.spline.set_last_point_sent(last);
                    continuation = Some(WorldPacket::with_body(
                        opcodes::SMSG_MONSTER_MOVE,
                        payload.into_vec(),
                    ));
                }
                let loc = unit.spline.compute_position();
                if is_valid_map_coord(loc.pos.x, loc.pos.y, loc.pos.z, loc.orientation) {
                    new_position = Some(Position {
                        x: loc.pos.x,
                        y: loc.pos.y,
                        z: loc.pos.z,
                        o: loc.orientation,
                    });
                }
            }
            if let Some(packet) = continuation {
                self.broadcast_movement(guid, &packet);
            }
            if let Some(position) = new_position {
                self.relocate_unit(guid, position);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MotionMaster + generators

/// One movement generator (`MovementGenerator` subset as an
/// enum; the C++ vtable becomes match dispatch).
#[derive(Clone, Debug)]
enum Generator {
    Idle,
    Point {
        id: u32,
        dest: Position,
        options: u32,
        speed: f32,
        final_orientation: f32,
        recalculate_speed: bool,
    },
    Random {
        start: Position,
        wander_distance: f32,
        next_move: ShortTimeTracker,
        wander_steps: u32,
        expire_time: u32,
    },
    Waypoint {
        /// Point ids in ascending order (path snapshot from
        /// the stores at initialization).
        nodes: Vec<(u32, mangos_world_data::waypoints::WaypointNode)>,
        current_node: u32,
        last_reached: u32,
        arrival_done: bool,
        is_wandering: bool,
        repeating: bool,
        next_move: ShortTimeTracker,
    },
    Home {
        arrived: bool,
    },
    Chase {
        target: ObjectGuid,
        offset: f32,
        angle: f32,
        check_timer: ShortTimeTracker,
        last_target_pos: Position,
        recalculate: bool,
        target_reached: bool,
        /// `m_bReachable`.
        reachable: bool,
    },
}

impl Generator {
    fn motion_type(&self) -> u32 {
        match self {
            Generator::Idle => IDLE_MOTION_TYPE,
            Generator::Point { .. } => POINT_MOTION_TYPE,
            Generator::Random { .. } => RANDOM_MOTION_TYPE,
            Generator::Waypoint { .. } => WAYPOINT_MOTION_TYPE,
            Generator::Home { .. } => HOME_MOTION_TYPE,
            Generator::Chase { .. } => CHASE_MOTION_TYPE,
        }
    }

    /// `Initialize` — may launch a spline.
    fn initialize(&mut self, map: &mut Map, guid: ObjectGuid) {
        match self {
            Generator::Idle => {}
            Generator::Point {
                dest,
                options,
                speed,
                final_orientation,
                ..
            } => {
                let (dest, options, speed, o) = (*dest, *options, *speed, *final_orientation);
                point_initialize(map, guid, dest, options, speed, o);
            }
            Generator::Random { next_move, .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.add_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
                next_move.reset(1000);
            }
            Generator::Waypoint { .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.add_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
                // InitializeWaypointPath ends with StartMove.
                self.waypoint_start_move(map, guid);
            }
            Generator::Home { arrived } => {
                *arrived = false;
                home_set_target(map, guid);
            }
            Generator::Chase { recalculate, .. } => {
                // SetWalk(false) + states + async recalc
                // (run inline here).
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.world
                        .movement_info
                        .flags
                        .remove(MovementFlags::WALK_MODE);
                    unit.add_unit_state(UNIT_STATE_CHASE | UNIT_STATE_CHASE_MOVE);
                }
                *recalculate = true;
                self.chase_set_target_location(map, guid);
            }
        }
    }

    /// `Interrupt`.
    fn interrupt(&mut self, map: &mut Map, guid: ObjectGuid) {
        match self {
            Generator::Idle | Generator::Home { .. } => {}
            Generator::Point { .. } | Generator::Random { .. } | Generator::Waypoint { .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.clear_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
            }
            Generator::Chase { .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.clear_unit_state(UNIT_STATE_CHASE | UNIT_STATE_CHASE_MOVE);
                }
            }
        }
    }

    /// `Finalize` — runs when the generator leaves the
    /// stack for good.
    fn finalize(&mut self, map: &mut Map, guid: ObjectGuid) {
        match self {
            Generator::Idle => {}
            Generator::Point { id, .. } => {
                let id = *id;
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.clear_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
                map.events.push(MapEvent::MovementInform {
                    guid,
                    motion_type: POINT_MOTION_TYPE,
                    point_id: id,
                });
            }
            Generator::Random { .. } | Generator::Waypoint { .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.clear_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
            }
            Generator::Home { arrived } => {
                if *arrived {
                    map.events.push(MapEvent::JustReachedHome { guid });
                }
            }
            Generator::Chase { .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.clear_unit_state(UNIT_STATE_CHASE | UNIT_STATE_CHASE_MOVE);
                }
            }
        }
    }

    /// `Reset` — the generator surfaces again after an
    /// expire above it.
    fn reset(&mut self, map: &mut Map, guid: ObjectGuid) {
        match self {
            Generator::Idle | Generator::Home { .. } => {}
            Generator::Point { .. } => {
                map.stop_moving_if_moving(guid);
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.add_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
            }
            Generator::Random { .. } | Generator::Chase { .. } => self.initialize(map, guid),
            Generator::Waypoint { is_wandering, .. } => {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.add_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
                }
                if *is_wandering {
                    // Wander return-to-node move is the
                    // MonsterMoveWithSpeed arm; approximated
                    // by a fresh StartMove (recorded).
                    *is_wandering = false;
                }
                self.waypoint_start_move(map, guid);
            }
        }
    }

    /// `Update(diff) -> bool` (false = expire self). The
    /// C++ `UpdateAsync` halves run inline after the sync
    /// half (single-threaded map).
    fn update(&mut self, map: &mut Map, guid: ObjectGuid, diff: u32) -> bool {
        match self {
            Generator::Idle => true,
            Generator::Point {
                dest,
                options,
                speed,
                final_orientation,
                recalculate_speed,
                ..
            } => {
                let (can_not_move, finalized) = match map.unit_base_mut(guid) {
                    Some(unit) => {
                        if unit.has_unit_state(UNIT_STATE_CAN_NOT_MOVE) {
                            unit.clear_unit_state(UNIT_STATE_ROAMING_MOVE);
                            return true;
                        }
                        unit.add_unit_state(UNIT_STATE_ROAMING_MOVE);
                        (false, unit.spline.finalized())
                    }
                    None => return false,
                };
                let _ = can_not_move;
                if !finalized && *recalculate_speed {
                    *recalculate_speed = false;
                    let (dest, options, speed, o) = (*dest, *options, *speed, *final_orientation);
                    point_initialize(map, guid, dest, options, speed, o);
                }
                !map.unit_base_mut(guid)
                    .is_some_and(|u| u.spline.finalized())
            }
            Generator::Random { expire_time, .. } => {
                if *expire_time != 0 {
                    if *expire_time <= diff {
                        return false;
                    }
                    *expire_time -= diff;
                }
                self.random_update_async(map, guid, diff);
                true
            }
            Generator::Waypoint { .. } => self.waypoint_update(map, guid, diff),
            Generator::Chase { .. } => self.chase_update(map, guid, diff),
            Generator::Home { arrived } => {
                *arrived = map
                    .unit_base_mut(guid)
                    .is_some_and(|u| u.spline.finalized());
                !*arrived
            }
        }
    }

    /// `GetResetPosition` for the Home generator.
    fn reset_position(&self, map: &Map, guid: ObjectGuid) -> Option<Position> {
        match self {
            Generator::Random {
                start,
                wander_distance,
                ..
            } => {
                let pos = map.unit_position(guid)?;
                let dx = pos.x - start.x;
                let dy = pos.y - start.y;
                if dx * dx + dy * dy <= wander_distance * wander_distance {
                    Some(pos)
                } else {
                    Some(*start)
                }
            }
            Generator::Waypoint {
                nodes,
                last_reached,
                ..
            } => {
                if *last_reached == 0 {
                    return None;
                }
                nodes
                    .iter()
                    .find(|(id, _)| id == last_reached)
                    .map(|(_, n)| Position {
                        x: n.x,
                        y: n.y,
                        z: n.z,
                        o: n.orientation,
                    })
            }
            _ => None,
        }
    }

    // -- Random ------------------------------------------------

    fn random_update_async(&mut self, map: &mut Map, guid: ObjectGuid, diff: u32) {
        let Generator::Random {
            start,
            wander_distance,
            next_move,
            ..
        } = self
        else {
            return;
        };
        let (blocked, finalized, alive) = match map.unit_base_mut(guid) {
            Some(unit) => (
                unit.has_unit_state(UNIT_STATE_CAN_NOT_MOVE | UNIT_STATE_DISTRACTED),
                unit.spline.finalized(),
                unit.is_alive(),
            ),
            None => return,
        };
        if !alive {
            return;
        }
        if blocked {
            next_move.reset(0);
            if let Some(unit) = map.unit_base_mut(guid) {
                unit.clear_unit_state(UNIT_STATE_ROAMING_MOVE);
            }
            return;
        }
        if !finalized {
            return;
        }
        next_move.update(ms_i32(diff));
        if !next_move.passed() {
            return;
        }
        // _setRandomLocation (ground arm).
        let start = *start;
        let wander = *wander_distance;
        let Some(dest) = map.random_point(start, wander) else {
            return;
        };
        if let Some(unit) = map.unit_base_mut(guid) {
            unit.add_unit_state(UNIT_STATE_ROAMING_MOVE);
        }
        let from = map.unit_position(guid).unwrap_or(start);
        // MoveTo(MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES)
        let path = map.build_path(from, dest, true, true);
        let walk = !map.creature_always_runs(guid);
        map.launch_spline(
            guid,
            SplineLaunch {
                path,
                walk,
                ..SplineLaunch::default()
            },
        );
        let Generator::Random {
            next_move,
            wander_steps,
            wander_distance,
            ..
        } = self
        else {
            return;
        };
        if *wander_steps != 0 {
            *wander_steps -= 1;
            next_move.reset(50);
        } else {
            let pause = i32::try_from(map.rng_urand(4, 10) * 1000).unwrap_or(4000);
            next_move.reset(pause);
            *wander_steps = map.rng_urand(0, if *wander_distance <= 1.0 { 2 } else { 8 });
        }
    }

    // -- Waypoint ----------------------------------------------

    fn waypoint_node(&self, point: u32) -> Option<mangos_world_data::waypoints::WaypointNode> {
        let Generator::Waypoint { nodes, .. } = self else {
            return None;
        };
        nodes.iter().find(|(id, _)| *id == point).map(|(_, n)| *n)
    }

    fn waypoint_start_move(&mut self, map: &mut Map, guid: ObjectGuid) {
        let Generator::Waypoint {
            nodes,
            current_node,
            arrival_done,
            repeating,
            next_move,
            ..
        } = self
        else {
            return;
        };
        if nodes.is_empty() || !next_move.passed() {
            return;
        }
        let mut idx = nodes
            .iter()
            .position(|(id, _)| *id == *current_node)
            .unwrap_or(0);
        if *arrival_done {
            idx += 1;
            if idx >= nodes.len() {
                if !*repeating {
                    // Set home to the last node and
                    // re-initialize motion (falls to the
                    // default generator).
                    let last = nodes[idx - 1].1;
                    if let Some(creature) = map.creatures.get_mut(&guid) {
                        creature.home_position = Position {
                            x: last.x,
                            y: last.y,
                            z: last.z,
                            o: last.orientation,
                        };
                    }
                    map.motion_initialize(guid);
                    return;
                }
                idx = 0;
            }
            *current_node = nodes[idx].0;
        }
        *arrival_done = false;
        let node = nodes[idx].1;
        if let Some(unit) = map.unit_base_mut(guid) {
            unit.add_unit_state(UNIT_STATE_ROAMING_MOVE);
        }
        let from = map.unit_position(guid).unwrap_or(Position {
            x: node.x,
            y: node.y,
            z: node.z,
            o: 0.0,
        });
        let dest = Position {
            x: node.x,
            y: node.y,
            z: node.z,
            o: node.orientation,
        };
        // Special sub-paths (`path_id`) ride as a plain
        // move for now (recorded deviation).
        let path = map.build_path(from, dest, true, false);
        #[allow(clippy::float_cmp)] // C++ compares the 100.0 sentinel exactly
        let facing = if node.orientation != 100.0 && node.delay != 0 {
            Some(FacingInfo::Angle(node.orientation))
        } else {
            None
        };
        let walk = !map
            .unit_base_mut(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_RUNNING));
        map.launch_spline(
            guid,
            SplineLaunch {
                path,
                walk,
                facing,
                ..SplineLaunch::default()
            },
        );
    }

    fn waypoint_on_arrived(&mut self, map: &mut Map, guid: ObjectGuid) -> bool {
        let Generator::Waypoint {
            current_node,
            last_reached,
            arrival_done,
            is_wandering,
            next_move,
            ..
        } = self
        else {
            return false;
        };
        if *is_wandering {
            *is_wandering = false;
            next_move.reset(1000);
            return false;
        }
        *last_reached = *current_node;
        if *arrival_done {
            return true;
        }
        if let Some(unit) = map.unit_base_mut(guid) {
            unit.clear_unit_state(UNIT_STATE_ROAMING_MOVE);
        }
        *arrival_done = true;
        let point = *current_node;
        let Some(node) = self.waypoint_node(point) else {
            return true;
        };
        if node.script_id != 0 {
            map.events.push(MapEvent::WaypointScript {
                guid,
                script_id: node.script_id,
                point_id: point,
            });
        }
        map.events.push(MapEvent::MovementInform {
            guid,
            motion_type: WAYPOINT_MOTION_TYPE,
            point_id: point,
        });
        if node.delay != 0 {
            if node.wander_distance != 0.0 {
                if let Generator::Waypoint { is_wandering, .. } = self {
                    *is_wandering = true;
                }
                map.creature_move_random(guid, true, node.wander_distance, node.delay);
            } else if let Generator::Waypoint { next_move, .. } = self {
                next_move.reset(ms_i32(node.delay));
            }
            return false;
        }
        true
    }

    fn waypoint_update(&mut self, map: &mut Map, guid: ObjectGuid, diff: u32) -> bool {
        let Generator::Waypoint {
            nodes, next_move, ..
        } = self
        else {
            return false;
        };
        let (blocked, stopped, finalized) = match map.unit_base_mut(guid) {
            Some(unit) => (
                unit.has_unit_state(UNIT_STATE_CAN_NOT_MOVE | UNIT_STATE_DISTRACTED),
                unit.is_stopped(),
                unit.spline.finalized(),
            ),
            None => return false,
        };
        if blocked || nodes.is_empty() {
            if let Some(unit) = map.unit_base_mut(guid) {
                unit.clear_unit_state(UNIT_STATE_ROAMING_MOVE);
            }
            return true;
        }
        if !next_move.passed() {
            // Stopped(): wait out the node delay.
            next_move.update(ms_i32(diff));
            if next_move.passed() {
                self.waypoint_start_move(map, guid);
            }
        } else if stopped && !finalized {
            // Externally stopped mid-spline: C++ waits
            // STOP_TIME_FOR_PLAYER before resuming.
            if let Generator::Waypoint { next_move, .. } = self {
                next_move.reset(30_000);
            }
        } else if finalized && self.waypoint_on_arrived(map, guid) {
            self.waypoint_start_move(map, guid);
        }
        true
    }

    // -- Chase -------------------------------------------------
    //
    // Subset of `ChaseMovementGenerator`
    // (`TargetedMovementGenerator.cpp:84-418`): no
    // spreading/back-movement, no movement extrapolation,
    // LOS always true until vmaps land. `_lostTarget`
    // (victim mismatch) has no victim system yet — target
    // existence/aliveness is the loss condition; the AI
    // slice expires chase on victim change itself.

    fn chase_set_target_location(&mut self, map: &mut Map, guid: ObjectGuid) {
        let Generator::Chase {
            target,
            offset,
            angle,
            last_target_pos,
            recalculate,
            ..
        } = self
        else {
            return;
        };
        let (target, offset, angle) = (*target, *offset, *angle);
        let Some(target_pos) = map.unit_position(target) else {
            return;
        };
        if map
            .unit_base_mut(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_CAN_NOT_MOVE))
        {
            return;
        }
        *last_target_pos = target_pos;
        let owner_pos = map.unit_position(guid).unwrap_or(target_pos);

        let dest = if offset == 0.0 {
            if map.can_reach_melee(guid, target) {
                // In reach (+LOS): stay.
                *recalculate = false;
                return;
            }
            // GetRandomAttackPoint subset: from the
            // target toward the owner at reach-0.5.
            let attack_angle = (owner_pos.y - target_pos.y).atan2(owner_pos.x - target_pos.x);
            let dist = map.reach_between(target, guid) - 0.5;
            let actual = dist3d(owner_pos, target_pos);
            if dist > actual {
                owner_pos
            } else {
                Position {
                    x: target_pos.x + dist * attack_angle.cos(),
                    y: target_pos.y + dist * attack_angle.sin(),
                    z: target_pos.z,
                    o: attack_angle,
                }
            }
        } else {
            // GetNearPointAroundPosition: offset +
            // searcher bounding radius at target facing +
            // angle.
            let radius = offset + map.bounding_radius_of(guid);
            let abs_angle = target_pos.o + angle;
            Position {
                x: target_pos.x + radius * abs_angle.cos(),
                y: target_pos.y + radius * abs_angle.sin(),
                z: target_pos.z,
                o: abs_angle,
            }
        };

        let result = map.find_path_points(owner_pos, dest, false, false);
        let reachable = matches!(result.kind, PathKind::Normal | PathKind::Shortcut);
        if let Generator::Chase {
            recalculate,
            target_reached,
            reachable: stored,
            ..
        } = self
        {
            *stored = reachable;
            *recalculate = false;
            if result.kind == PathKind::NoPath {
                // PATHFIND_NOPATH: C++ returns without
                // launching (m_bReachable = false).
                return;
            }
            *target_reached = false;
        }
        map.launch_spline(
            guid,
            SplineLaunch {
                path: result.points,
                walk: false,
                ..SplineLaunch::default()
            },
        );
    }

    fn chase_update(&mut self, map: &mut Map, guid: ObjectGuid, diff: u32) -> bool {
        let Generator::Chase {
            target,
            offset,
            check_timer,
            last_target_pos,
            recalculate,
            ..
        } = self
        else {
            return false;
        };
        let (target, offset) = (*target, *offset);
        let Some(target_pos) = map.unit_position(target) else {
            return false;
        };
        let (can_not_move, finalized) = match map.unit_base_mut(guid) {
            Some(unit) => (
                unit.has_unit_state(UNIT_STATE_CAN_NOT_MOVE),
                unit.spline.finalized(),
            ),
            None => return false,
        };
        if can_not_move {
            if let Some(unit) = map.unit_base_mut(guid) {
                unit.clear_unit_state(UNIT_STATE_CHASE_MOVE);
            }
            return true;
        }

        check_timer.update(ms_i32(diff));
        if check_timer.passed() {
            check_timer.reset(100);
            if !finalized && offset == 0.0 && map.within_dist(guid, target, 0.0) {
                if let Some(unit) = map.unit_base_mut(guid) {
                    unit.spline.interrupt();
                }
            } else {
                let mut target_moved = target_pos != *last_target_pos;
                if !target_moved && finalized {
                    let allowed = map.reach_between(guid, target);
                    target_moved = !map.within_dist(guid, target, allowed);
                }
                if target_moved {
                    *recalculate = true;
                }
            }
        }
        if *recalculate {
            self.chase_set_target_location(map, guid);
        }

        let finalized = map
            .unit_base_mut(guid)
            .is_some_and(|u| u.spline.finalized());
        if finalized {
            // Face the target (SetInFront orientation;
            // the facing packet rides the next movement).
            if let Some(owner_pos) = map.unit_position(guid) {
                let facing = (target_pos.y - owner_pos.y).atan2(target_pos.x - owner_pos.x);
                if (owner_pos.o - facing).abs() > 0.01 {
                    map.relocate_unit(
                        guid,
                        Position {
                            o: facing,
                            ..owner_pos
                        },
                    );
                }
            }
            map.events.push(MapEvent::MovementInform {
                guid,
                motion_type: CHASE_MOTION_TYPE,
                point_id: target.counter(),
            });
            if let Generator::Chase { target_reached, .. } = self
                && !*target_reached
            {
                *target_reached = true;
                if map.can_reach_melee(guid, target) {
                    map.events.push(MapEvent::ChaseReached { guid, target });
                }
            }
        }
        true
    }
}

/// 3D distance between two positions.
fn dist3d(a: Position, b: Position) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// `PointMovementGenerator::Initialize` shared body.
fn point_initialize(
    map: &mut Map,
    guid: ObjectGuid,
    dest: Position,
    options: u32,
    speed: f32,
    final_orientation: f32,
) {
    map.stop_moving_if_moving(guid);
    if let Some(unit) = map.unit_base_mut(guid) {
        unit.add_unit_state(UNIT_STATE_ROAMING | UNIT_STATE_ROAMING_MOVE);
    }
    let from = map.unit_position(guid).unwrap_or(dest);
    let path = map.build_path(
        from,
        dest,
        options & MOVE_PATHFINDING != 0,
        options & MOVE_EXCLUDE_STEEP_SLOPES != 0,
    );
    let walk = if options & MOVE_WALK_MODE != 0 {
        true
    } else if options & MOVE_RUN_MODE != 0 {
        false
    } else {
        // MoveSplineInit mixes existing walk state in.
        map.unit_base_mut(guid)
            .is_some_and(|u| u.world.movement_info.flags.has(MovementFlags::WALK_MODE))
    };
    let facing = (final_orientation > -7.0).then_some(FacingInfo::Angle(final_orientation));
    map.launch_spline(
        guid,
        SplineLaunch {
            path,
            walk,
            velocity: if speed > 0.0 { speed } else { 0.0 },
            facing,
            ..SplineLaunch::default()
        },
    );
}

/// `HomeMovementGenerator::_setTargetLocation` (sessile
/// and aura-state arms excluded; teleport fallback kept).
fn home_set_target(map: &mut Map, guid: ObjectGuid) {
    if map
        .unit_base_mut(guid)
        .is_some_and(|u| u.has_unit_state(UNIT_STATE_CAN_NOT_MOVE))
    {
        return;
    }
    // Reset position from the generator underneath, else
    // the respawn coord with facing.
    let (dest, set_facing) = {
        let from_stack = map
            .motion
            .get(&guid)
            .and_then(|mm| mm.stack.iter().rev().nth(1))
            .and_then(|g| g.reset_position(map, guid));
        if let Some(pos) = from_stack {
            (pos, false)
        } else {
            let home = map
                .creatures
                .get(&guid)
                .map(|c| c.home_position)
                .unwrap_or_default();
            (home, true)
        }
    };
    let from = map.unit_position(guid).unwrap_or(dest);
    // `path.calculate(x, y, z, true)`; spline only on
    // PATHFIND_NORMAL, NearTeleportTo otherwise. The
    // no-mesh shortcut counts as NORMAL|NOT_USING_PATH
    // in C++ and launches.
    let result = map.find_path_points(from, dest, true, false);
    if matches!(result.kind, PathKind::Normal | PathKind::Shortcut) {
        map.launch_spline(
            guid,
            SplineLaunch {
                path: result.points,
                walk: false,
                facing: set_facing.then_some(FacingInfo::Angle(dest.o)),
                ..SplineLaunch::default()
            },
        );
    } else {
        map.relocate_unit(guid, dest);
        if let Some(unit) = map.unit_base_mut(guid) {
            unit.spline.interrupt();
        }
    }
    if let Some(unit) = map.unit_base_mut(guid) {
        unit.clear_unit_state(UNIT_STATE_ALL_DYN_STATES);
    }
}

/// The per-unit generator stack (`MotionMaster` subset).
pub struct MotionMaster {
    stack: Vec<Generator>,
}

impl MotionMaster {
    fn new(bottom: Generator) -> MotionMaster {
        MotionMaster {
            stack: vec![bottom],
        }
    }

    /// `GetCurrentMovementGeneratorType`.
    #[must_use]
    pub fn current_type(&self) -> u32 {
        self.stack
            .last()
            .map_or(IDLE_MOTION_TYPE, Generator::motion_type)
    }

    /// `UpdateMotion` (`MotionMaster.cpp:173-209`, without
    /// the deferred-delete list — generators are values).
    fn update_motion(&mut self, map: &mut Map, guid: ObjectGuid, diff: u32) {
        if map
            .unit_base_mut(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_CAN_NOT_MOVE))
        {
            return;
        }
        let Some(mut top) = self.stack.pop() else {
            return;
        };
        let alive = top.update(map, guid, diff);
        if alive {
            self.stack.push(top);
            return;
        }
        // Expire: finalize the old top, then reset whatever
        // surfaces (unless Finalize pushed something — our
        // finalizers never push).
        top.finalize(map, guid);
        if self.stack.is_empty() {
            // Rebuilt by the caller via motion_initialize
            // semantics: keep an Idle bottom so the stack
            // invariant (size >= 1) holds.
            self.stack.push(Generator::Idle);
        } else if let Some(mut new_top) = self.stack.pop() {
            new_top.reset(map, guid);
            self.stack.push(new_top);
        }
    }

    /// `Mutate` (`MotionMaster.cpp:672-699`): CHASE/HOME
    /// tops expire instead of interrupting (DISTANCING
    /// exception has no generator yet).
    fn mutate(&mut self, map: &mut Map, guid: ObjectGuid, mut generator: Generator) {
        if let Some(mut top) = self.stack.pop() {
            match top.motion_type() {
                HOME_MOTION_TYPE | CHASE_MOTION_TYPE if !self.stack.is_empty() => {
                    top.finalize(map, guid);
                }
                _ => {
                    top.interrupt(map, guid);
                    self.stack.push(top);
                }
            }
        }
        generator.initialize(map, guid);
        self.stack.push(generator);
    }

    /// `Clear(false)`: pop to the bottom generator and
    /// reset it (finalizers run after the reset in C++;
    /// ours only clear states, so order is unobservable
    /// here beyond `MovementInform` events, which we emit
    /// on finalize as C++ does).
    pub(crate) fn clear(&mut self, map: &mut Map, guid: ObjectGuid) {
        let mut popped = Vec::new();
        while self.stack.len() > 1 {
            if let Some(generator) = self.stack.pop() {
                popped.push(generator);
            }
        }
        if let Some(mut bottom) = self.stack.pop() {
            bottom.reset(map, guid);
            self.stack.push(bottom);
        }
        for mut generator in popped {
            generator.finalize(map, guid);
        }
    }
}

impl Map {
    fn rng_urand(&mut self, min: u32, max: u32) -> u32 {
        use mangos_entities::creature::SpawnRng;
        self.rng.urand(min, max)
    }

    pub(crate) fn creature_always_runs(&self, guid: ObjectGuid) -> bool {
        self.creatures.get(&guid).is_some_and(|c| c.default_running)
    }

    /// `!IsStopped()` guard used by Point initialize/reset.
    pub(crate) fn stop_moving_if_moving(&mut self, guid: ObjectGuid) {
        let moving = self.unit_base_mut(guid).is_some_and(|u| !u.is_stopped());
        if moving {
            self.stop_moving(guid);
        }
    }

    /// `WorldObject::GetRandomPoint` ground arm via
    /// `Map::GetWalkRandomPosition`'s non-navmesh fallback
    /// (angle/radius around `center`, z lifted to ground).
    pub(crate) fn random_point(&mut self, center: Position, distance: f32) -> Option<Position> {
        if distance < 0.1 {
            return Some(center);
        }
        let radius = distance * self.rng.rand_norm();
        let angle = self.rng.frand(0.0, std::f32::consts::TAU);
        let x = center.x + radius * angle.cos();
        let y = center.y + radius * angle.sin();
        if !is_valid_map_coord(x, y, center.z, 0.0) {
            return None;
        }
        let ground = self.ground_z(x, y, center.z);
        // C++: `if (vmapH > z) z = vmapH` with z starting
        // at the center height.
        Some(Position {
            x,
            y,
            z: ground.max(center.z),
            o: angle,
        })
    }

    /// Sets a unit's target field (`Unit::SetTarget`
    /// field write; selection parity for bots).
    pub fn bot_set_target(&mut self, guid: ObjectGuid, target: ObjectGuid) {
        {
            let Some(unit) = self.unit_base_mut(guid) else {
                return;
            };
            unit.values().set_guid(
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET,
                target,
            );
        }
    }

    /// Installs an idle motion master for a player
    /// (bot locomotion, Phase 5 Slice D). Human
    /// players are client-driven and never call this.
    pub fn motion_initialize_player(&mut self, guid: ObjectGuid) {
        if self.players.contains_key(&guid) {
            self.motion.insert(guid, MotionMaster::new(Generator::Idle));
        }
    }

    /// `MotionMaster::Initialize` for a creature: stop,
    /// clear, install the factory default from
    /// `default_movement_type`.
    pub fn motion_initialize(&mut self, guid: ObjectGuid) {
        let Some(creature) = self.creatures.get(&guid) else {
            return;
        };
        if !creature.unit.is_alive() {
            self.motion.insert(guid, MotionMaster::new(Generator::Idle));
            return;
        }
        let motion_type = creature.default_movement_type;
        let home = creature.home_position;
        let wander = creature.wander_distance;
        let default_running = creature.default_running;
        let guid_low = guid.counter();
        let entry = creature.original_entry;
        let mut bottom = match motion_type {
            RANDOM_MOTION_TYPE => Generator::Random {
                start: home,
                wander_distance: if wander > 0.0 { wander } else { 5.0 },
                next_move: ShortTimeTracker::new(1000),
                wander_steps: 0,
                expire_time: 0,
            },
            WAYPOINT_MOTION_TYPE | CYCLIC_MOTION_TYPE => {
                let nodes: Vec<(u32, mangos_world_data::waypoints::WaypointNode)> = self
                    .stores
                    .waypoints
                    .path_for(guid_low, entry)
                    .map(|p: &WaypointPath| p.iter().map(|(id, n)| (*id, *n)).collect())
                    .unwrap_or_default();
                let first = nodes.first().map_or(0, |(id, _)| *id);
                Generator::Waypoint {
                    nodes,
                    current_node: first,
                    last_reached: 0,
                    arrival_done: false,
                    is_wandering: false,
                    repeating: true,
                    next_move: ShortTimeTracker::new(0),
                }
            }
            _ => Generator::Idle,
        };
        // UNIT_STATE_RUNNING default from flags_extra.
        if default_running {
            if let Some(unit) = self.unit_base_mut(guid) {
                unit.add_unit_state(UNIT_STATE_RUNNING);
            }
        }
        let mut master = MotionMaster::new(Generator::Idle);
        master.stack.clear();
        bottom.initialize(self, guid);
        master.stack.push(bottom);
        self.motion.insert(guid, master);
    }

    /// Per-tick creature motion driver
    /// (`MotionMaster::UpdateMotion` inside
    /// `Creature::Update`).
    pub(crate) fn update_creature_motion(&mut self, diff_ms: u32) {
        let mut guids: Vec<ObjectGuid> = self.motion.keys().copied().collect();
        guids.sort_by_key(|g| g.raw());
        for guid in guids {
            // Bot players (Phase 5 Slice D) carry a
            // motion master too; human players never
            // get one inserted, so this stays a no-op
            // for them.
            let alive = self.creatures.get(&guid).is_some_and(|c| c.unit.is_alive())
                || self
                    .players
                    .get(&guid)
                    .is_some_and(|entry| entry.player.unit.is_alive());
            if !alive {
                continue;
            }
            let Some(mut master) = self.motion.remove(&guid) else {
                continue;
            };
            master.update_motion(self, guid, diff_ms);
            // A generator may have re-initialized motion
            // (waypoint path end): keep the fresh stack.
            self.motion.entry(guid).or_insert(master);
        }
    }

    /// `GetCombatReach()` from the values mirror.
    pub(crate) fn combat_reach_of(&self, guid: ObjectGuid) -> f32 {
        self.unit_value_f32(guid, offsets::UNIT_FIELD_COMBATREACH)
    }

    /// `GetObjectBoundingRadius()` from the values mirror.
    pub(crate) fn bounding_radius_of(&self, guid: ObjectGuid) -> f32 {
        self.unit_value_f32(guid, offsets::UNIT_FIELD_BOUNDINGRADIUS)
    }

    fn unit_value_f32(&self, guid: ObjectGuid, index: u16) -> f32 {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.world.object.values().get_f32(index);
        }
        self.creatures
            .get(&guid)
            .map_or(0.0, |c| c.unit.world.object.values().get_f32(index))
    }

    /// `GetCombatReachToTarget(victim, false, 0, true)`
    /// (no melee leeway): sum of reaches + base offset,
    /// floored at `ATTACK_DISTANCE`.
    pub(crate) fn reach_between(&self, a: ObjectGuid, b: ObjectGuid) -> f32 {
        let reach = self.combat_reach_of(a) + self.combat_reach_of(b) + BASE_MELEE_RANGE_OFFSET;
        if reach < ATTACK_DISTANCE {
            ATTACK_DISTANCE
        } else {
            reach
        }
    }

    /// `IsWithinDist(other, dist)` with size factor: 3D
    /// distance minus both bounding radii <= dist.
    pub(crate) fn within_dist(&self, a: ObjectGuid, b: ObjectGuid, dist: f32) -> bool {
        let (Some(pa), Some(pb)) = (self.unit_position(a), self.unit_position(b)) else {
            return false;
        };
        let size = self.bounding_radius_of(a) + self.bounding_radius_of(b);
        dist3d(pa, pb) - size <= dist
    }

    /// `CanReachWithMeleeAutoAttack(victim)` (reach only;
    /// the LOS/arc parts are melee-slice concerns).
    pub(crate) fn can_reach_melee(&self, a: ObjectGuid, b: ObjectGuid) -> bool {
        self.within_dist(a, b, self.reach_between(a, b))
    }

    /// `MotionMaster::MoveChase` (creature arm: interrupt
    /// the current spline first).
    pub fn creature_move_chase(
        &mut self,
        guid: ObjectGuid,
        target: ObjectGuid,
        offset: f32,
        angle: f32,
    ) {
        if self.unit_position(target).is_none() {
            return;
        }
        self.stop_moving_if_moving(guid);
        let last = self.unit_position(target).unwrap_or_default();
        self.mutate_motion(
            guid,
            Generator::Chase {
                target,
                offset,
                angle,
                check_timer: ShortTimeTracker::new(0),
                last_target_pos: last,
                recalculate: true,
                target_reached: false,
                reachable: true,
            },
        );
    }

    /// `MotionMaster::MovePoint` (creature arm).
    pub fn creature_move_point(
        &mut self,
        guid: ObjectGuid,
        id: u32,
        dest: Position,
        options: u32,
        speed: f32,
        final_orientation: f32,
    ) {
        self.mutate_motion(
            guid,
            Generator::Point {
                id,
                dest,
                options,
                speed,
                final_orientation,
                recalculate_speed: false,
            },
        );
    }

    /// `MotionMaster::MoveRandom`.
    pub fn creature_move_random(
        &mut self,
        guid: ObjectGuid,
        use_current_position: bool,
        wander_distance: f32,
        expire_time: u32,
    ) {
        let Some(creature) = self.creatures.get(&guid) else {
            return;
        };
        let start = if use_current_position {
            creature.unit.world.position()
        } else {
            creature.home_position
        };
        let wander = if wander_distance > 0.0 {
            wander_distance
        } else if creature.wander_distance > 0.0 {
            creature.wander_distance
        } else {
            5.0
        };
        self.mutate_motion(
            guid,
            Generator::Random {
                start,
                wander_distance: wander,
                next_move: ShortTimeTracker::new(1000),
                wander_steps: 0,
                expire_time,
            },
        );
    }

    /// `MotionMaster::MoveIdle` (script MOVEMENT arm).
    pub fn creature_move_idle(&mut self, guid: ObjectGuid) {
        self.stop_moving_if_moving(guid);
        self.mutate_motion(guid, Generator::Idle);
    }

    /// `MotionMaster::MoveWaypoint` over the spawn's
    /// loaded path (script `START_WAYPOINTS` / MOVEMENT
    /// arms; wpSource/overwrite variants are recorded
    /// deviations).
    pub fn creature_move_waypoint(&mut self, guid: ObjectGuid, start_point: u32, repeating: bool) {
        let Some(creature) = self.creatures.get(&guid) else {
            return;
        };
        let guid_low = guid.counter();
        let entry = creature.original_entry;
        let nodes: Vec<(u32, mangos_world_data::waypoints::WaypointNode)> = self
            .stores
            .waypoints
            .path_for(guid_low, entry)
            .map(|p: &WaypointPath| p.iter().map(|(id, n)| (*id, *n)).collect())
            .unwrap_or_default();
        if nodes.is_empty() {
            return;
        }
        let current_node = if nodes.iter().any(|(id, _)| *id == start_point) {
            start_point
        } else {
            nodes.first().map_or(0, |(id, _)| *id)
        };
        self.mutate_motion(
            guid,
            Generator::Waypoint {
                nodes,
                current_node,
                last_reached: 0,
                arrival_done: false,
                is_wandering: false,
                repeating,
                next_move: ShortTimeTracker::new(0),
            },
        );
    }

    /// `MotionMaster::MoveTargetedHome` (plain creature
    /// arm: clear, then Home on top of the default).
    pub fn creature_move_home(&mut self, guid: ObjectGuid) {
        let Some(mut master) = self.motion.remove(&guid) else {
            return;
        };
        master.clear(self, guid);
        master.mutate(self, guid, Generator::Home { arrived: false });
        self.motion.entry(guid).or_insert(master);
    }

    fn mutate_motion(&mut self, guid: ObjectGuid, generator: Generator) {
        let Some(mut master) = self.motion.remove(&guid) else {
            return;
        };
        master.mutate(self, guid, generator);
        self.motion.entry(guid).or_insert(master);
    }

    // -- Speed / movement-flag changes (MovementPacketSender) --

    /// `SendMovementMessageToSet(data, true, except)`.
    pub(crate) fn broadcast_movement_except(
        &mut self,
        guid: ObjectGuid,
        packet: &WorldPacket,
        except: ObjectGuid,
    ) {
        let mut targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(g, e)| **g != except && (**g == guid || e.visible.contains(&guid)))
            .map(|(g, _)| *g)
            .collect();
        targets.sort_by_key(|g| g.raw());
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    pub(crate) fn packed_guid_bytes(guid: ObjectGuid) -> Vec<u8> {
        let mut bytes = Vec::new();
        guid.write_packed(&mut bytes);
        bytes
    }

    /// `Unit::SetSpeedRate` (`Unit.cpp:7270-7288`):
    /// player mover -> pending change + FORCE packet to
    /// the controller only; server mover -> real rate +
    /// spline-set packet to the set. `UpdateSpeed` (the
    /// aura-modifier computation feeding this) lands with
    /// auras in Slice 5; callers pass the final rate.
    pub fn set_speed_rate(&mut self, guid: ObjectGuid, move_type: usize, rate: f32) {
        let rate = if rate < 0.0 { 0.0 } else { rate };
        let change_type = mangos_entities::unit::speed_change_type(move_type);
        let Some(unit) = self.unit_base_mut(guid) else {
            return;
        };
        #[allow(clippy::float_cmp)] // C++ compares the stored rate exactly
        if unit.speed_rate[move_type] == rate
            && !unit.has_pending_movement_change(Some(change_type))
        {
            return;
        }
        if guid.is_player() {
            let counter = unit.movement_counter_and_inc();
            let flat = rate * mangos_entities::unit::BASE_MOVE_SPEED[move_type];
            unit.pending_movement_changes
                .push_back(mangos_entities::unit::PendingMovementChange {
                    movement_counter: counter,
                    change_type,
                    new_value: flat,
                    apply: false,
                    knockback: None,
                });
            let mut payload = PacketWriter::new();
            payload.write_bytes(&Self::packed_guid_bytes(guid));
            payload.write_u32_le(counter);
            payload.write_f32_le(flat);
            let packet = WorldPacket::with_body(FORCE_SPEED_OPCODES[move_type], payload.into_vec());
            self.events.push(MapEvent::Packet { to: guid, packet });
        } else {
            unit.speed_rate[move_type] = rate;
            let flat = rate * mangos_entities::unit::BASE_MOVE_SPEED[move_type];
            let mut payload = PacketWriter::new();
            payload.write_bytes(&Self::packed_guid_bytes(guid));
            payload.write_f32_le(flat);
            let packet =
                WorldPacket::with_body(SPLINE_SPEED_OPCODES[move_type], payload.into_vec());
            self.broadcast_movement(guid, &packet);
            // PropagateSpeedChange: active splines keep
            // their baked velocity in this slice (C++
            // generators re-init on next point; recorded).
        }
    }

    /// `SetSpeedRateReal`: store without sending (ACK
    /// application + login).
    pub fn set_speed_rate_real(&mut self, guid: ObjectGuid, move_type: usize, rate: f32) {
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.speed_rate[move_type] = if rate < 0.0 { 0.0 } else { rate };
        }
    }

    /// `SendSpeedChangeToObservers` (after a valid ACK):
    /// finalized spline -> `MSG_MOVE_SET_*_SPEED` with the
    /// movement info; else the spline-set form. Both go
    /// to the set except the mover.
    pub fn send_speed_change_to_observers(
        &mut self,
        guid: ObjectGuid,
        move_type: usize,
        flat_speed: f32,
    ) {
        let now_ms = self.now_ms;
        let Some((finalized, info)) = self.unit_base_mut(guid).map(|u| {
            (u.spline.finalized(), {
                let mut info = u.world.movement_info;
                info.set_as_server_side(now_ms);
                u.world.movement_info = info;
                info
            })
        }) else {
            return;
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(guid));
        let opcode = if finalized {
            info.write(&mut payload);
            payload.write_f32_le(flat_speed);
            MOVE_SET_SPEED_OPCODES[move_type]
        } else {
            payload.write_f32_le(flat_speed);
            SPLINE_SPEED_OPCODES[move_type]
        };
        let packet = WorldPacket::with_body(opcode, payload.into_vec());
        self.broadcast_movement_except(guid, &packet, guid);
    }

    /// `MovementPacketSender::SendKnockBackToObservers`:
    /// `MSG_MOVE_KNOCK_BACK` (packed guid + stored
    /// movement info + jump fields) to the set
    /// except the mover. Runs after the ack
    /// relocation, so the stored info is the ack's.
    pub fn send_knock_back_to_observers(&mut self, guid: ObjectGuid) {
        let now_ms = self.now_ms;
        let Some(info) = self.unit_base_mut(guid).map(|u| {
            let mut info = u.world.movement_info;
            info.set_as_server_side(now_ms);
            u.world.movement_info = info;
            info
        }) else {
            return;
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(guid));
        info.write(&mut payload);
        payload.write_f32_le(info.jump.cos_angle);
        payload.write_f32_le(info.jump.sin_angle);
        payload.write_f32_le(info.jump.xy_speed);
        payload.write_f32_le(info.jump.z_speed);
        let packet = WorldPacket::with_body(
            mangos_protocol::opcode_table::opcode_id::MSG_MOVE_KNOCK_BACK,
            payload.into_vec(),
        );
        self.broadcast_movement_except(guid, &packet, guid);
    }

    /// `Unit::SetWalk(enable, as_default=false)` +
    /// `SendToggleRunWalkToAll`: flag flip + spline
    /// run/walk-mode packet to the set.
    pub fn set_walk(&mut self, guid: ObjectGuid, enable: bool) {
        let Some(unit) = self.unit_base_mut(guid) else {
            return;
        };
        if enable == unit.world.movement_info.flags.has(MovementFlags::WALK_MODE) {
            return;
        }
        if enable {
            unit.world.movement_info.flags.add(MovementFlags::WALK_MODE);
        } else {
            unit.world
                .movement_info
                .flags
                .remove(MovementFlags::WALK_MODE);
        }
        let opcode = if enable {
            opcodes::SMSG_SPLINE_MOVE_SET_WALK_MODE
        } else {
            opcodes::SMSG_SPLINE_MOVE_SET_RUN_MODE
        };
        let packet = WorldPacket::with_body(opcode, Self::packed_guid_bytes(guid));
        self.broadcast_movement(guid, &packet);
    }

    /// Movement-flag change (`SetRooted`/`SetWaterWalking`/
    /// `SetHover`/`SetFeatherFall` send halves): player
    /// mover -> pending + FORCE/MOVE packet to controller;
    /// server mover -> real flag + `SPLINE_MOVE` packet to
    /// the set.
    pub fn set_movement_flag(&mut self, guid: ObjectGuid, flag: u32, apply: bool) {
        let Some((change_type, controller_opcodes, observer_opcode, spline_opcodes)) =
            flag_change_tables(flag)
        else {
            return;
        };
        if guid.is_player() {
            let Some(unit) = self.unit_base_mut(guid) else {
                return;
            };
            let counter = unit.movement_counter_and_inc();
            unit.pending_movement_changes
                .push_back(mangos_entities::unit::PendingMovementChange {
                    movement_counter: counter,
                    change_type,
                    new_value: 0.0,
                    apply,
                    knockback: None,
                });
            let mut payload = PacketWriter::new();
            payload.write_bytes(&Self::packed_guid_bytes(guid));
            payload.write_u32_le(counter);
            let opcode = if apply {
                controller_opcodes.0
            } else {
                controller_opcodes.1
            };
            let packet = WorldPacket::with_body(opcode, payload.into_vec());
            self.events.push(MapEvent::Packet { to: guid, packet });
        } else {
            self.set_movement_flag_real(guid, flag, apply);
            let opcode = if apply {
                spline_opcodes.0
            } else {
                spline_opcodes.1
            };
            let packet = WorldPacket::with_body(opcode, Self::packed_guid_bytes(guid));
            self.broadcast_movement(guid, &packet);
        }
        let _ = observer_opcode; // used by the ACK path below
    }

    /// The `...Real` flag application (also ROOT's unit
    /// state).
    pub fn set_movement_flag_real(&mut self, guid: ObjectGuid, flag: u32, apply: bool) {
        if let Some(unit) = self.unit_base_mut(guid) {
            if apply {
                unit.world.movement_info.flags.add(flag);
            } else {
                unit.world.movement_info.flags.remove(flag);
            }
            if flag == MovementFlags::ROOT {
                if apply {
                    unit.add_unit_state(mangos_entities::unit::UNIT_STATE_ROOT);
                } else {
                    unit.clear_unit_state(mangos_entities::unit::UNIT_STATE_ROOT);
                }
            }
        }
        if apply && flag == MovementFlags::ROOT {
            // Rooted units stop (the spline is interrupted
            // by the root aura path in C++; StopMoving is
            // the visible equivalent here).
            self.stop_moving_if_moving(guid);
        }
    }

    /// `SendMovementFlagChangeToObservers` (after a valid
    /// flag/root ACK).
    pub fn send_flag_change_to_observers(&mut self, guid: ObjectGuid, flag: u32, apply: bool) {
        let Some((_, _, observer_opcodes, _)) = flag_change_tables(flag) else {
            return;
        };
        let now_ms = self.now_ms;
        let Some(info) = self.unit_base_mut(guid).map(|u| {
            let mut info = u.world.movement_info;
            info.set_as_server_side(now_ms);
            u.world.movement_info = info;
            info
        }) else {
            return;
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(guid));
        info.write(&mut payload);
        let opcode = if apply {
            observer_opcodes.0
        } else {
            observer_opcodes.1
        };
        let packet = WorldPacket::with_body(opcode, payload.into_vec());
        self.broadcast_movement_except(guid, &packet, guid);
    }

    /// Current chase target, when the top generator is
    /// a chase (AI victim tracking).
    #[must_use]
    pub fn chase_target_of(&self, guid: ObjectGuid) -> Option<ObjectGuid> {
        match self.motion.get(&guid).and_then(|m| m.stack.last()) {
            Some(Generator::Chase { target, .. }) => Some(*target),
            _ => None,
        }
    }

    /// Current generator type for a creature (tests/AI).
    #[must_use]
    pub fn motion_type_of(&self, guid: ObjectGuid) -> u32 {
        self.motion
            .get(&guid)
            .map_or(IDLE_MOTION_TYPE, MotionMaster::current_type)
    }
}
