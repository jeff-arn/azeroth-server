//! One simulated map: spawn activation, player visibility,
//! values replication, and movement relay at Slice 3
//! scope. Tick order follows `Map::Update`
//! (`Map.cpp:933-1060`) minus the continent barrier (not
//! reproduced — recorded in `PHASE4-PROGRESS.md`), grid
//! state machine/unload (cells stay loaded), transports,
//! scripts, weather, and adaptive visibility.
//!
//! Session packet pumping happens in the binary's world
//! actor before/around [`Map::tick`]; packets the map
//! produces are drained as [`MapEvent`]s tagged with the
//! receiving player's GUID.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mangos_entities::creature::{Creature, CreatureRates, DisplayResolver, VirtualItemPrototype};
use mangos_entities::gameobject::GameObject;
use mangos_entities::player::Player;
use mangos_entities::unit::DeathState;
use mangos_entities::update::{
    BuildEffects, MovementSection, ObjectView, Viewer, build_create_block, build_values_block,
};
use mangos_guid::ObjectGuid;
use mangos_guid::timer::ShortIntervalTimer;
use mangos_movement::movement_info::{MovementInfo, Position};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_data::{CompressionConfig, UpdateData};
use mangos_protocol::world::WorldPacket;
use mangos_world_data::spawn_support::RespawnTimes;
use mangos_world_data::spawns::{cell_id, compute_cell_pair};

use crate::stores::WorldStores;
use crate::visibility;

/// `def_world_states` (`Player.cpp:8102-8115`): default
/// initial world-state pairs. Event/battleground-specific
/// pairs join with their systems.
const DEFAULT_WORLD_STATES: &[(u32, u32)] = &[
    (0x07AE, 0x01),
    (0x0532, 0x01),
    (0x0531, 0x00),
    (0x0530, 0x00),
    (0x07A6, 0x01),
    (0x07A5, 0x01),
    (0x07A4, 0x03),
    (0x07A7, 0x05),
    (0x07A8, 0x00),
    (0x07A9, 0x00),
    (0x07AA, 0x00),
    (0x07AB, 0x00),
    (0x07AC, 0x00),
    (0x07AD, 0x00),
    (0x08D8, 0x14),
    (0x08D7, 0x01),
    (0x08D6, 0x00),
    (0x08D5, 0x00),
    (0x08D4, 0x01),
    (0x08D3, 0x01),
    (0x0A40, 0xC8),
    (0x09F3, 0x00),
    (0x09F2, 0x00),
    (0x09F1, 0x00),
    (0x09F0, 0x01),
    (0x09EF, 0x01),
    (0x09EE, 0x00),
    (0x09ED, 0x00),
    (0x09EC, 0x00),
    (0x09EB, 0x01),
    (0x09EA, 0x01),
    (0x0A41, 0xC8),
    (0x0BC7, 0x01),
    (0x0BC6, 0x00),
    (0x0C0D, 0x00),
    (0x0C0E, 0x00),
    (0x0BAF, 0x00),
    (0x0BC5, 0x00),
    (0x0BC4, 0x00),
    (0x0BC3, 0x00),
    (0x0BC2, 0x00),
    (0x0BC1, 0x00),
    (0x0BC0, 0x00),
    (0x0BBF, 0x00),
    (0x0BBE, 0x00),
    (0x0BBD, 0x00),
    (0x0BBC, 0x00),
    (0x0BBB, 0x00),
    (0x0BBA, 0x00),
    (0x0BB9, 0x00),
    (0x0BB8, 0x00),
    (0x0BB7, 0x00),
    (0x0BB6, 0x00),
    (0x0BB5, 0x00),
    (0x0BB4, 0x00),
    (0x0BB3, 0x00),
    (0x0BB2, 0x00),
    (0x0BB1, 0x00),
    (0x0BB0, 0x00),
    (0x0C0C, 0x00),
    (0x0C0B, 0x00),
    (0x0A45, 0x00),
    (0x0A44, 0x00),
    (0x0A43, 0x00),
    (0x08DC, 0x01),
    (0x08DB, 0x00),
    (0x08DA, 0x00),
    (0x08D9, 0x01),
    (0x08E2, 0x01),
    (0x08E1, 0x01),
    (0x08E0, 0x00),
    (0x08DF, 0x00),
    (0x079F, 0x00),
    (0x079E, 0x00),
    (0x07C0, 0x00),
    (0x07C1, 0x00),
    (0x07C2, 0x00),
    (0x07C3, 0x00),
    (0x07C4, 0x00),
    (0x07C5, 0x00),
    (0x0A42, 0x0A),
    (0x0A3C, 0x00),
    (0x0A3F, 0x00),
    (0x0A3E, 0x00),
    (0x0A3D, 0x00),
    (0x0A3B, 0x00),
    (0x0A3A, 0x00),
    (0x0A39, 0x00),
    (0x0A38, 0x00),
    (0x0A36, 0x00),
    (0x0A35, 0x00),
    (0x0A37, 0x00),
    (0x0A32, 0x00),
    (0x0A33, 0x00),
    (0x0A34, 0x00),
    (0x0516, 0x00),
    (0x0517, 0x00),
    (0x0703, 0x00),
];

/// `Player::SendInitWorldStates`: map, zone, count
/// (including the terminating zero pair), state/value
/// pairs, terminator.
#[must_use]
pub fn build_init_world_states(map_id: u32, zone_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(map_id);
    writer.write_u32_le(zone_id);
    writer.write_u16_le(DEFAULT_WORLD_STATES.len() as u16 + 1);
    for &(state, value) in DEFAULT_WORLD_STATES {
        writer.write_u32_le(state);
        writer.write_u32_le(value);
    }
    writer.write_u32_le(0);
    writer.write_u32_le(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_INIT_WORLD_STATES);
    packet.take_writer(writer);
    packet
}
/// `Rate.XP.Kill` / `.Kill.Elite` / `.Quest` /
/// `.Explore` (`World.cpp:506-509`, all default 1.0;
/// production runs Kill/Quest/Explore at 1.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XpRates {
    pub kill: f32,
    pub kill_elite: f32,
    pub quest: f32,
    pub explore: f32,
}

impl Default for XpRates {
    fn default() -> Self {
        Self {
            kill: 1.0,
            kill_elite: 1.0,
            quest: 1.0,
            explore: 1.0,
        }
    }
}

/// Map tuning (`mangosd.conf` values with their tree
/// defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapConfig {
    /// `Visibility.Distance.*` for this map kind.
    pub visibility_distance: f32,
    /// Grid activation distance (= visibility on
    /// continents, `Map.cpp:208-213`).
    pub grid_activation_distance: f32,
    /// `Visibility.Distance.Grey.Unit` (1).
    pub grey_unit: f32,
    /// `Visibility.Distance.Grey.Object` (10).
    pub grey_object: f32,
    /// `Visibility.RelocationLowerLimit` squared (100).
    pub relocation_lower_limit_sq: f32,
    /// `MapUpdate.UpdatePlayersDiff` (100 ms).
    pub update_players_diff: u32,
    /// `Rate.Creature.Aggro` (1.0) — ghost corpse sight.
    pub aggro_rate: f32,
    /// `Visibility.ForceActiveObjects` (true).
    pub force_active_objects: bool,
    pub creature_rates: CreatureRates,
    pub compression: CompressionConfig,
    /// `ActivateWeather` (true).
    pub activate_weather: bool,
    /// `Rate.XP.*` kill/quest/explore rates (1.0).
    pub xp_rates: XpRates,
    /// `Spell.ProcDelay` batching window (ms; fork
    /// code default 400).
    pub proc_delay_ms: u32,
    /// `IsPvPRealm()` for the `UpdateZone` rest branch.
    pub pvp_realm: bool,
    /// `ChangeWeatherInterval` in milliseconds (10 min).
    pub change_weather_interval_ms: u32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            visibility_distance: 100.0,
            grid_activation_distance: 100.0,
            grey_unit: 1.0,
            grey_object: 10.0,
            relocation_lower_limit_sq: 100.0,
            update_players_diff: 100,
            aggro_rate: 1.0,
            force_active_objects: true,
            creature_rates: CreatureRates::default(),
            compression: CompressionConfig::default(),
            activate_weather: true,
            xp_rates: XpRates::default(),
            proc_delay_ms: 400,
            pvp_realm: false,
            change_weather_interval_ms: 600_000,
        }
    }
}

/// Opaque map-local player handle = character GUID.
pub type PlayerSlotId = ObjectGuid;

/// Something the map wants the caller to do.
#[derive(Clone, Debug)]
pub enum MapEvent {
    /// Deliver a packet to this player's session.
    Packet { to: ObjectGuid, packet: WorldPacket },
    /// Persist `respawn_time = 0` for a creature spawn
    /// (expired row consumed at spawn).
    ClearCreatureRespawn { guid_low: u32, instance_id: u32 },
    /// Persist `respawn_time = 0` for a gameobject spawn.
    ClearGameObjectRespawn { guid_low: u32, instance_id: u32 },
    /// Persist a future creature respawn time
    /// (`REPLACE INTO creature_respawn`, death in
    /// Slice 5).
    StoreCreatureRespawn {
        guid_low: u32,
        respawn_time: u64,
        map_id: u32,
        instance_id: u32,
    },
    /// A unit died (Slice 5 damage; threat/loot are
    /// Slice 6 consumers).
    UnitDied {
        guid: ObjectGuid,
        killer: Option<ObjectGuid>,
    },
    /// Persist a pet row (`Pet::SavePetToDB` snapshot
    /// taken by the map at the fork's trigger points;
    /// `mode` is the slot value to write).
    SavePet {
        row: Box<mangos_entities::PetRow>,
        mode: u32,
    },
    /// `Pet::DeleteFromDB` (abandoned hunter pet).
    DeletePet { pet_id: u32 },
    /// The `LoadPetFromDB` slot-promotion UPDATE pair.
    PromotePet { owner_low: u32, pet_id: u32 },
    /// The `HandlePetRename` immediate name UPDATE.
    RenamePet {
        owner_low: u32,
        pet_id: u32,
        name: String,
    },
    /// `Spell::DoCreateItem` grant request: store
    /// `count` of `item_id` for `player` (count
    /// already clamped to one stack by the effect;
    /// the actor runs the `CanStoreNewItem` subset,
    /// writes the rows, and sends `SendNewItem`).
    CreateItem {
        player: ObjectGuid,
        item_id: u32,
        count: u32,
    },
    /// Cross-map player transfer request
    /// (`Player::TeleportTo` far arm of
    /// `Spell::EffectTeleportUnits`): the actor runs
    /// its `teleport_player` far-transfer flow.
    TeleportPlayer {
        player: ObjectGuid,
        map_id: u32,
        position: Position,
    },
    /// Knockback launched on a player
    /// (`MovementAnticheat::OnKnockBack` +
    /// `Player::SetLaunched`): the actor marks the
    /// session anticheat launched and seeds the
    /// jump initial speed.
    KnockBack {
        player: ObjectGuid,
        vertical_speed: f32,
    },
    /// Persist one item stack's count
    /// (`DestroyItemCount` consumption; 0 deletes
    /// both rows). Baseline already edited by the map.
    SetItemCount { item_guid: u32, count: u32 },
    /// Persist one item's charges text
    /// (`TakeCastItem` write-back). Baseline already
    /// edited by the map.
    SetItemCharges { item_guid: u32, charges: String },
    /// `AI()->MovementInform(type, id)` — consumed by the
    /// AI slice; emitted by Point/Waypoint generators.
    MovementInform {
        guid: ObjectGuid,
        motion_type: u32,
        point_id: u32,
    },
    /// `AI()->JustReachedHome()` (Home generator arrival).
    JustReachedHome { guid: ObjectGuid },
    /// Chase `_reachTarget`: in melee reach — the C++
    /// `Attack(target, true)` hook (melee is Slice 5/6).
    ChaseReached {
        guid: ObjectGuid,
        target: ObjectGuid,
    },
    /// Waypoint node carried a `script_id`
    /// (`ScriptsStart(sCreatureMovementScripts, ...)`) —
    /// consumed by the script-engine slice.
    WaypointScript {
        guid: ObjectGuid,
        script_id: u32,
        point_id: u32,
    },
}

pub(crate) struct PlayerEntry {
    pub(crate) player: Player,
    pub(crate) visible: HashSet<ObjectGuid>,
    /// `m_lastNotifiedPosition`.
    pub(crate) last_notified: Position,
    /// A full camera re-scan is due (entry or relocation
    /// past the lower limit).
    pub(crate) scan_due: bool,
    /// `m_zoneUpdateId` / `m_areaUpdateId` (0 until the
    /// first zone refresh after map entry).
    pub(crate) zone_id: u32,
    pub(crate) area_id: u32,
}

/// One simulated map instance.
/// One cooldown entry (`CooldownData` subset):
/// expiry on the map ms clock, the item that created
/// it (0 for spell casts), and the permanent hold of
/// `SPELL_ATTR_COOLDOWN_ON_EVENT` spells (never
/// expires, never saved; converted to a timed entry
/// when the aura fades).
#[derive(Clone, Copy)]
pub(crate) struct CooldownEntry {
    pub expiry_ms: u32,
    pub item_id: u32,
    pub permanent: bool,
    /// Effective spell category recorded at add
    /// time (item spell rows can override the
    /// spell's own category); only meaningful on
    /// spell-key entries, where the save snapshot
    /// reads it.
    pub category: u32,
}

impl CooldownEntry {
    pub(crate) fn timed(expiry_ms: u32) -> Self {
        Self {
            expiry_ms,
            item_id: 0,
            permanent: false,
            category: 0,
        }
    }
}

/// One loaded `character_spell_cooldown` row in
/// remaining-time form (`Player::_LoadSpellCooldowns`
/// output), seeded into the map at login.
pub struct LoadedCooldown {
    pub spell: u32,
    pub item_id: u32,
    pub category: u32,
    pub spell_remaining_ms: u32,
    pub category_remaining_ms: u32,
}

pub struct Map {
    #[allow(clippy::struct_field_names)] // mirrors C++ GetId()
    pub(crate) map_id: u32,
    pub(crate) instance_id: u32,
    pub(crate) config: MapConfig,
    pub(crate) stores: Arc<WorldStores>,
    pub(crate) rng: crate::WorldRng,
    pub(crate) players: HashMap<ObjectGuid, PlayerEntry>,
    pub(crate) creatures: HashMap<ObjectGuid, Creature>,
    pub(crate) gameobjects: HashMap<ObjectGuid, GameObject>,
    /// Live dynamic objects (Slice 4b).
    pub(crate) dynobjects: HashMap<ObjectGuid, mangos_entities::dynobject::DynObject>,
    /// Dynobj affected-pass throttle accumulator.
    pub(crate) dynobj_tick_ms: u32,
    /// Runtime cell index for units/GOs (guid sets per
    /// cell id) — visibility queries walk these.
    pub(crate) cells: HashMap<u32, CellObjects>,
    /// DB spawn cells whose objects are instantiated.
    pub(crate) spawn_cells_loaded: HashSet<u32>,
    /// Transport gameobjects by guid (created at map
    /// construction from the transport spawn sets; motion
    /// deferred with the transport system).
    pub(crate) events: Vec<MapEvent>,
    /// Live per-zone weather (`WeatherSystem::m_weathers`).
    pub(crate) zone_weather: HashMap<u32, crate::weather::ZoneWeather>,
    pub(crate) players_timer: ShortIntervalTimer,
    /// Server ms clock stamped into creates/movement.
    pub(crate) now_ms: u32,
    pub(crate) now_unix: u64,
    /// Per-creature movement generators (Slice 4).
    pub(crate) motion: HashMap<ObjectGuid, crate::motion::MotionMaster>,
    /// `splineCounter` (`MoveSplineInit.cpp:63`, starts 1).
    pub(crate) spline_counter: u32,
    /// Ground-height source (`.map` grid files).
    pub(crate) heights: crate::motion::Heights,
    /// Detour navmesh host (`.mmap`/`.mmtile` files).
    pub(crate) navhost: crate::motion::NavHost,
    /// Per-creature AI runtime (Slice 6), created lazily
    /// on first AI update or first damage taken.
    pub(crate) ai: HashMap<ObjectGuid, crate::ai::CreatureAi>,
    /// In-progress casts by caster (Slice 5).
    pub(crate) casts: HashMap<ObjectGuid, crate::spells::ActiveCast>,
    /// Spell + category cooldowns (map ms clock):
    /// (caster, spell id) and (caster, category) share
    /// one map via the high-bit tag on the id.
    pub(crate) cooldowns: HashMap<(ObjectGuid, u32), CooldownEntry>,
    /// Delayed proc checks (`m_pendingProcChecks`,
    /// dealer-side queue flushed when due).
    pub(crate) pending_procs: Vec<crate::procs::PendingProc>,
    /// True while `cast_spell` runs for a proc
    /// handler (marks the cast aura-triggered for
    /// proc-chain gating).
    pub(crate) in_proc_cast: bool,
    /// Dead creatures awaiting respawn: guid -> respawn
    /// unix time (Slice 5 death handling).
    pub(crate) respawns_due: HashMap<ObjectGuid, u64>,
    /// Scheduled generic script steps (`m_scriptSchedule`,
    /// Slice 6b): (due map ms, insertion sequence) ->
    /// step. C++ keys by game-time seconds; the map ms
    /// clock gives the same order with finer grain
    /// (recorded deviation).
    pub(crate) script_schedule:
        std::collections::BTreeMap<(u64, u64), crate::scripts::ScheduledScript>,
    pub(crate) script_seq: u64,
    /// Server variables (`SetSavedVariable` store),
    /// in-memory only — C++ persists them, which would
    /// be a new DB write (recorded; not in the write
    /// register).
    pub(crate) saved_variables: HashMap<u32, u32>,
    /// `InstanceData` field stores (present on instanced
    /// maps only, mirroring `GetInstanceData()` null on
    /// world maps).
    pub(crate) instance_data: HashMap<u32, u32>,
    pub(crate) instance_data64: HashMap<u32, u64>,
    /// Delayed creature despawns (Slice 6b
    /// `DespawnOrUnsummon(msTimeToDespawn, respawn)`):
    /// guid -> (despawn-at map ms, respawn delay sec;
    /// `u32::MAX` = never respawn).
    pub(crate) despawns_due: HashMap<ObjectGuid, (u64, u32)>,
    /// Temporary summons (Slice 7): guid -> lifecycle.
    pub(crate) summons: HashMap<ObjectGuid, SummonState>,
    /// Controlled pets (Tier 1 Slice 2): pet guid ->
    /// charm state.
    pub(crate) pets: HashMap<ObjectGuid, crate::pets::PetState>,
    /// Local counter for summon guids (DB spawn guids
    /// stay well below this band on the bench).
    pub(crate) summon_counter: u32,
    /// Objects with visibility switched off
    /// (`VISIBILITY_OFF` / `SetVisible(false)`): the
    /// Deadmines patrols and the Darkmoon chest.
    pub(crate) hidden_objects: std::collections::HashSet<ObjectGuid>,
    /// Deadmines instance script state (map 36 with a
    /// real instance id; Slice 7 hand-port).
    pub(crate) deadmines: Option<crate::content::DeadminesState>,
}

/// `TempSummon` lifecycle (Slice 7 subset).
pub(crate) struct SummonState {
    pub summoner: ObjectGuid,
    /// `TempSummonType`.
    pub kind: u32,
    /// Absolute map-ms despawn/kill time for the timed
    /// types; `None` for manual/corpse types.
    pub despawn_at_ms: Option<u64>,
    pub despawn_delay_ms: u32,
}

#[derive(Default)]
pub(crate) struct CellObjects {
    pub(crate) players: HashSet<ObjectGuid>,
    pub(crate) creatures: HashSet<ObjectGuid>,
    pub(crate) gameobjects: HashSet<ObjectGuid>,
    pub(crate) dynobjects: HashSet<ObjectGuid>,
}

impl Map {
    #[must_use]
    pub fn new(
        map_id: u32,
        instance_id: u32,
        config: MapConfig,
        stores: Arc<WorldStores>,
        rng: crate::WorldRng,
    ) -> Map {
        let mut players_timer = ShortIntervalTimer::new();
        players_timer.set_interval(config.update_players_diff);
        Map {
            map_id,
            instance_id,
            config,
            stores,
            rng,
            players: HashMap::new(),
            creatures: HashMap::new(),
            gameobjects: HashMap::new(),
            dynobjects: HashMap::new(),
            dynobj_tick_ms: 0,
            cells: HashMap::new(),
            spawn_cells_loaded: HashSet::new(),
            events: Vec::new(),
            zone_weather: HashMap::new(),
            players_timer,
            now_ms: 0,
            now_unix: 0,
            motion: HashMap::new(),
            spline_counter: 1,
            heights: crate::motion::Heights::new(map_id, None),
            navhost: crate::motion::NavHost::new(map_id, None),
            ai: HashMap::new(),
            casts: HashMap::new(),
            cooldowns: HashMap::new(),
            pending_procs: Vec::new(),
            in_proc_cast: false,
            respawns_due: HashMap::new(),
            script_schedule: std::collections::BTreeMap::new(),
            script_seq: 0,
            saved_variables: HashMap::new(),
            instance_data: HashMap::new(),
            instance_data64: HashMap::new(),
            despawns_due: HashMap::new(),
            summons: HashMap::new(),
            pets: HashMap::new(),
            summon_counter: 0x00F0_0000,
            hidden_objects: std::collections::HashSet::new(),
            deadmines: (map_id == 36 && instance_id != 0).then(crate::content::DeadminesState::new),
        }
    }

    /// Points the height source at the extracted `.map`
    /// directory (binary boot; tests may leave it unset,
    /// in which case ground queries fall back to the
    /// caller's z hint).
    pub fn set_maps_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.heights = crate::motion::Heights::new(self.map_id, dir);
    }

    #[must_use]
    pub fn id(&self) -> u32 {
        self.map_id
    }

    #[must_use]
    pub fn instance_id(&self) -> u32 {
        self.instance_id
    }

    /// Persistent-state instance key for respawn rows
    /// (`MapPersistentState::m_instanceid`): dungeon
    /// maps scope rows to this instance; other maps
    /// keep the spawn's instanciated-continent id
    /// (0 when continents are not instanciated).
    pub(crate) fn respawn_instance_key(&self, continent_instance_id: u32) -> u32 {
        if self.stores.maps.is_instance_map(self.map_id) {
            self.instance_id
        } else {
            continent_instance_id
        }
    }

    /// Respawn instance key for a DB creature spawn
    /// guid (death/despawn store paths).
    pub(crate) fn creature_respawn_key(&self, guid_low: u32) -> u32 {
        let continent_id = self
            .stores
            .spawns
            .creature(guid_low)
            .map_or(0, |data| data.instanciated_continent_instance_id);
        self.respawn_instance_key(continent_id)
    }

    #[must_use]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    #[must_use]
    pub fn creature_count(&self) -> usize {
        self.creatures.len()
    }

    #[must_use]
    pub fn gameobject_count(&self) -> usize {
        self.gameobjects.len()
    }

    /// One gameobject by guid (tests/tools).
    #[must_use]
    pub fn gameobject(&self, guid: ObjectGuid) -> Option<&GameObject> {
        self.gameobjects.get(&guid)
    }

    #[must_use]
    pub fn player(&self, guid: ObjectGuid) -> Option<&Player> {
        self.players.get(&guid).map(|e| &e.player)
    }

    pub fn player_mut(&mut self, guid: ObjectGuid) -> Option<&mut Player> {
        self.players.get_mut(&guid).map(|e| &mut e.player)
    }

    pub fn creature_mut(&mut self, guid: ObjectGuid) -> Option<&mut Creature> {
        self.creatures.get_mut(&guid)
    }

    #[must_use]
    pub fn creature(&self, guid: ObjectGuid) -> Option<&Creature> {
        self.creatures.get(&guid)
    }

    /// GUIDs currently visible to a player (tests/tools).
    #[must_use]
    pub fn visible_to(&self, guid: ObjectGuid) -> Option<&HashSet<ObjectGuid>> {
        self.players.get(&guid).map(|e| &e.visible)
    }

    pub fn drain_events(&mut self) -> Vec<MapEvent> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn cell_of(position: Position) -> u32 {
        cell_id(position.x, position.y)
    }

    /// `Map::Add(Player*)` subset: activates spawn cells in
    /// range, registers the player, sends init-self, and
    /// only then runs the first visibility pass
    /// (`Map.cpp:413-426`). Order matters: the local player
    /// create must precede surrounding object creates.
    pub fn add_player(
        &mut self,
        mut player: Player,
        respawns: &RespawnTimes,
        now_unix: u64,
        now_ms: u32,
    ) {
        self.now_unix = now_unix;
        self.now_ms = now_ms;
        let guid = player.unit.guid();
        let position = player.unit.world.position();
        let activated = self.activate_cells_around(position, respawns);
        // Grid-load spawns have already had their own
        // Add-to-world pass before this player is inserted;
        // the newcomer therefore receives CREATE_OBJECT,
        // matching C++ Map::Add(WorldObject*) clearing
        // m_isNewObject after UpdateObjectVisibility.
        for activated_guid in activated {
            self.clear_is_new(activated_guid);
        }
        player.unit.world.object.add_to_world();
        // The player is not new yet for SendInitSelf:
        // C++ Map::Add(Player*) sends init-self before the
        // generic WorldObject add sets m_isNewObject, so
        // the local create uses CREATE_OBJECT.
        let cell = Self::cell_of(position);
        self.cells.entry(cell).or_default().players.insert(guid);
        let entry = PlayerEntry {
            player,
            visible: HashSet::new(),
            last_notified: position,
            scan_due: true,
            zone_id: 0,
            area_id: 0,
        };
        self.players.insert(guid, entry);
        // `SendInitSelf` precedes `UpdateObjectVisibility`
        // in C++ `Map::Add(Player*)`.
        self.build_init_self(guid);
        // The generic add now marks the newcomer new for
        // other viewers, matching Map::Add(WorldObject*).
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.unit.world.object.set_is_new_object(true);
        }
        // The newcomer scans; everyone else re-scans for
        // the newcomer (object-centric path).
        self.scan_player_visibility(guid);
        let others: Vec<ObjectGuid> = self
            .players
            .keys()
            .copied()
            .filter(|g| *g != guid)
            .collect();
        for other in others {
            self.scan_player_visibility(other);
        }
        self.clear_is_new(guid);
        self.update_player_zone(guid, true);
    }

    /// `Map::Remove(Player*)` subset: out-of-range to
    /// viewers, returns the aggregate for logout handling.
    pub fn remove_player(&mut self, guid: ObjectGuid) -> Option<Player> {
        let entry = self.players.remove(&guid)?;
        let cell = Self::cell_of(entry.player.unit.world.position());
        if let Some(c) = self.cells.get_mut(&cell) {
            c.players.remove(&guid);
        }
        let mut player = entry.player;
        player.unit.world.object.remove_from_world();
        // Viewers lose the player; the leaver's own visible
        // set dies with the session.
        let others: Vec<ObjectGuid> = self.players.keys().copied().collect();
        for other in others {
            if self
                .players
                .get(&other)
                .is_some_and(|e| e.visible.contains(&guid))
            {
                let mut data = UpdateData::new();
                data.add_out_of_range_guid(guid);
                self.send_update_data(other, &mut data);
                if let Some(e) = self.players.get_mut(&other) {
                    e.visible.remove(&guid);
                }
            }
        }
        Some(player)
    }

    /// `TerrainInfo::GetAreaFlag` over the `GridMap`
    /// area flags. `VMap` WMO area overrides are deferred
    /// with the rest of the vmap port; a missing grid tile
    /// falls back to the map's linked-area flag
    /// (`AreaEntry::GetFlagByMapId`), as C++ does.
    pub(crate) fn area_flag_at(&mut self, position: Position) -> u16 {
        match self.heights.grid(position.x, position.y) {
            Some(grid) => grid.area(position.x, position.y),
            None => self.stores.areas.flag_by_map_id(self.map_id) as u16,
        }
    }

    /// `WorldObject::GetZoneAndAreaId`.
    pub fn zone_and_area_id(&mut self, position: Position) -> (u32, u32) {
        let area_flag = self.area_flag_at(position);
        self.stores
            .areas
            .zone_and_area_by_flag(area_flag, self.map_id)
    }

    /// `WorldObject::SendMessageToSetInRange` for player
    /// chat: queues `packet` for every player within
    /// `range` (3D) of `origin`, including the origin
    /// player, optionally restricted to the origin's team.
    pub fn broadcast_packet_in_range(
        &mut self,
        origin: ObjectGuid,
        range: f32,
        own_team_only: bool,
        packet: &WorldPacket,
    ) {
        let Some((origin_pos, origin_team)) = self
            .players
            .get(&origin)
            .map(|entry| (entry.player.unit.world.position(), entry.player.team))
        else {
            return;
        };
        let range_sq = range * range;
        let recipients: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, entry)| {
                if own_team_only && entry.player.team != origin_team {
                    return false;
                }
                let pos = entry.player.unit.world.position();
                let dx = pos.x - origin_pos.x;
                let dy = pos.y - origin_pos.y;
                let dz = pos.z - origin_pos.z;
                dx * dx + dy * dy + dz * dz <= range_sq
            })
            .map(|(guid, _)| *guid)
            .collect();
        for recipient in recipients {
            self.events.push(MapEvent::Packet {
                to: recipient,
                packet: packet.clone(),
            });
        }
    }

    /// A player's current zone id (0 when absent or not yet
    /// refreshed), for friend-list `FriendInfo::Area`.
    #[must_use]
    pub fn player_zone_id(&self, guid: ObjectGuid) -> u32 {
        self.players.get(&guid).map_or(0, |entry| entry.zone_id)
    }

    /// Refreshes one player's zone/area and, on a zone
    /// change (or forced first entry), queues
    /// `SMSG_INIT_WORLD_STATES` and the current
    /// The packet half of `Player::UpdateZone` plus its
    /// rest-type branch (`Player.cpp:6792-6806`): a capital
    /// zone that is not PvP-enforced for this player puts
    /// them in `REST_TYPE_IN_CITY`; any other zone change
    /// clears rest unless they are inn-resting (tavern
    /// exit is the areatrigger box check, still unported —
    /// tavern rest therefore survives zone changes exactly
    /// as in the fork, but is not cleared on foot within
    /// the zone). PvP/duel side effects of `UpdateZone`
    /// remain with their own systems.
    pub fn update_player_zone(&mut self, guid: ObjectGuid, force: bool) {
        let Some(position) = self
            .players
            .get(&guid)
            .map(|entry| entry.player.unit.world.position())
        else {
            return;
        };
        let (zone_id, area_id) = self.zone_and_area_id(position);
        let zone_changed = {
            let Some(entry) = self.players.get_mut(&guid) else {
                return;
            };
            let changed = force || entry.zone_id != zone_id;
            entry.zone_id = zone_id;
            entry.area_id = area_id;
            changed
        };
        if !zone_changed {
            return;
        }
        self.update_rest_for_zone(guid, zone_id);

        self.events.push(MapEvent::Packet {
            to: guid,
            packet: build_init_world_states(self.map_id, zone_id),
        });
        if self.config.activate_weather {
            let chances = self.stores.weather_chances.get(zone_id);
            let interval = self.config.change_weather_interval_ms;
            let weather = self
                .zone_weather
                .entry(zone_id)
                .or_insert_with(|| crate::weather::ZoneWeather::new(chances, interval));
            let packet = weather.packet();
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
    }

    /// Tavern areatrigger entry (`HandleAreaTriggerOpcode`
    /// tavern branch): `SetRestType(REST_TYPE_IN_TAVERN)`.
    pub fn set_player_rest_tavern(&mut self, guid: ObjectGuid, trigger_id: u32) {
        let now_ms = u64::from(self.now_ms);
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.set_rest_type(
                mangos_entities::player::RestType::InTavern,
                trigger_id,
                now_ms,
            );
        }
    }

    fn update_rest_for_zone(&mut self, guid: ObjectGuid, zone_id: u32) {
        use mangos_entities::player::RestType;
        use mangos_entities::unit::Team;
        // `AREA_FLAG_CAPITAL` (AreaTable flags).
        const AREA_FLAG_CAPITAL: u32 = 0x100;
        // `AREATEAM_ALLY` / `AREATEAM_HORDE` (`DBCEnums.h:48-50`).
        const AREATEAM_ALLY: u32 = 2;
        const AREATEAM_HORDE: u32 = 4;
        let Some(zone) = self.stores.areas.entry(zone_id).copied() else {
            return;
        };
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let player = &mut entry.player;
        let capital = zone.flags & AREA_FLAG_CAPITAL != 0;
        let pvp_enforced = match zone.team {
            AREATEAM_ALLY => player.team != Team::Alliance && (self.config.pvp_realm || capital),
            AREATEAM_HORDE => player.team != Team::Horde && (self.config.pvp_realm || capital),
            0 => self.config.pvp_realm,
            _ => false,
        };
        let now_ms = u64::from(self.now_ms);
        if capital && !pvp_enforced {
            player.set_rest_type(RestType::InCity, 0, now_ms);
        } else if player.rest_type != RestType::InTavern && player.rest_type != RestType::No {
            player.set_rest_type(RestType::No, 0, now_ms);
        } else if player.rest_type == RestType::No {
            // Fork: `HasFlag(RESTING) && restType != IN_TAVERN`
            // clears a load-restored resting flag whose rest
            // type never engaged (e.g. saved resting
            // outdoors).
            let resting = player
                .unit
                .world
                .object
                .values()
                .get_u32(mangos_protocol::update_field_table::offsets::PLAYER_FLAGS)
                & 0x20
                != 0;
            if resting {
                player.set_rest_type(RestType::No, 0, now_ms);
            }
        }
    }

    /// `WeatherSystem::UpdateWeathers`: regenerated zones
    /// broadcast to their players; a changed zone with no
    /// remaining players drops its weather object.
    fn update_weather(&mut self, diff_ms: u32) {
        if !self.config.activate_weather || self.zone_weather.is_empty() {
            return;
        }
        let zones: Vec<u32> = self.zone_weather.keys().copied().collect();
        for zone_id in zones {
            let packet = {
                let Some(weather) = self.zone_weather.get_mut(&zone_id) else {
                    continue;
                };
                if weather.update(diff_ms, self.now_unix, &mut self.rng) {
                    Some(weather.packet())
                } else {
                    None
                }
            };
            let Some(packet) = packet else {
                continue;
            };
            let recipients: Vec<ObjectGuid> = self
                .players
                .iter()
                .filter(|(_, entry)| entry.zone_id == zone_id)
                .map(|(guid, _)| *guid)
                .collect();
            if recipients.is_empty() {
                self.zone_weather.remove(&zone_id);
                continue;
            }
            for recipient in recipients {
                self.events.push(MapEvent::Packet {
                    to: recipient,
                    packet: packet.clone(),
                });
            }
        }
    }

    /// Cell ids in the activation square around a position
    /// (`Cell::CalculateCellArea`): corner pairs from
    /// `(x-r, y-r)` to `(x+r, y+r)`, clamped to the
    /// 1024-cell axis.
    #[allow(clippy::unused_self)] // grid constants move here with grid state
    fn cell_area(&self, position: Position, radius: f32) -> Vec<u32> {
        let norm = |v: u32| v.min(1023);
        let (x0, y0) = compute_cell_pair(position.x - radius, position.y - radius);
        let (x1, y1) = compute_cell_pair(position.x + radius, position.y + radius);
        let (x0, x1) = (norm(x0.min(x1)), norm(x0.max(x1)));
        let (y0, y1) = (norm(y0.min(y1)), norm(y0.max(y1)));
        let mut out = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                out.push(cy * 1024 + cx);
            }
        }
        out
    }

    /// Instantiates DB spawns for newly entered cells
    /// (`EnsureGridLoaded` data half). Elevator transports
    /// were registered separately by the spawn loader and
    /// are created on first activation too (Slice 3 keeps
    /// them static). Pooled/event spawns: the loader
    /// already withheld pooled GUIDs from the grid, so
    /// cells only contain directly-spawnable objects.
    fn activate_cells_around(
        &mut self,
        position: Position,
        respawns: &RespawnTimes,
    ) -> Vec<ObjectGuid> {
        let mut spawned = Vec::new();
        for cell in self.cell_area(position, self.config.grid_activation_distance) {
            if !self.spawn_cells_loaded.insert(cell) {
                continue;
            }
            let Some(guids) = self.stores.spawns.grid().cell(self.map_id, cell) else {
                continue;
            };
            let creature_guids: Vec<u32> = guids.creatures.iter().copied().collect();
            let go_guids: Vec<u32> = guids.gameobjects.iter().copied().collect();
            for guid_low in creature_guids {
                if let Some(guid) = self.spawn_creature(guid_low, respawns) {
                    spawned.push(guid);
                }
            }
            for guid_low in go_guids {
                if let Some(guid) = self.spawn_gameobject(guid_low, respawns) {
                    spawned.push(guid);
                }
            }
        }
        // Elevator transports for this map ride along on
        // first activation (they are not in spawn cells).
        let elevators: Vec<u32> = self.stores.spawns.elevator_transports(self.map_id).to_vec();
        for guid_low in elevators {
            if let Some(guid) = self.spawn_gameobject(guid_low, respawns) {
                spawned.push(guid);
            }
        }
        spawned
    }

    pub(crate) fn spawn_creature(
        &mut self,
        guid_low: u32,
        respawns: &RespawnTimes,
    ) -> Option<ObjectGuid> {
        let data = self.stores.spawns.creature(guid_low).cloned()?;
        let respawn_time = respawns
            .creature(
                self.respawn_instance_key(data.instanciated_continent_instance_id),
                guid_low,
            )
            .map_or(0, |e| e.respawn_time);
        let addon = self.stores.creature_addons.get(guid_low).cloned();
        self.materialize_creature(guid_low, &data, addon.as_ref(), respawn_time)
    }

    /// Materialize one creature from a spawn row (DB
    /// spawn or a synthetic temp-summon row) and add it
    /// to the world. Returns its guid.
    pub(crate) fn materialize_creature(
        &mut self,
        guid_low: u32,
        data: &mangos_world_data::spawns::CreatureSpawn,
        addon: Option<&mangos_world_data::spawn_support::CreatureDataAddon>,
        respawn_time: u64,
    ) -> Option<ObjectGuid> {
        // ObjectGuid needs the chosen entry; spawn_from_db
        // picks it — compute the key after spawn from the
        // creature's own guid.
        let stores = &self.stores;
        let resolver = DisplayResolver {
            scale_for_display: &|id| stores.scale_for_display(id),
            model_for_display: &|id| stores.model_for_display(id),
        };
        let rates = self.config.creature_rates;
        let now_unix = self.now_unix;
        let item_store = &self.stores.items;
        let items = |id: u32| {
            item_store.get(id).map(|p| VirtualItemPrototype {
                display_id: p.display_id,
                class: p.class,
                subclass: p.subclass,
                material: p.material,
                inventory_type: p.inventory_type,
                sheath: p.sheath,
            })
        };
        let result = Creature::spawn_from_db(
            guid_low,
            data,
            &stores.creature_templates,
            addon,
            &stores.equipment,
            &items,
            &stores.cls,
            &stores.display_addons,
            &resolver,
            &rates,
            respawn_time,
            now_unix,
            false,
            &mut self.rng,
        );
        // Failures are logged at the loader level in C++.
        let Ok(mut creature) = result else {
            return None;
        };
        creature.unit.world.object.set_is_new_object(true);
        creature.unit.world.object.add_to_world();
        if creature.respawn_row_expired {
            self.events.push(MapEvent::ClearCreatureRespawn {
                guid_low,
                instance_id: self.respawn_instance_key(data.instanciated_continent_instance_id),
            });
        }
        let guid = creature.unit.guid();
        // Re-materialization (respawn) replaces a corpse
        // that may sit in a different cell than the spawn
        // point: drop the stale cell entry first.
        if let Some(old) = self.creatures.get(&guid) {
            let old_cell = Self::cell_of(old.unit.world.position());
            if let Some(cell) = self.cells.get_mut(&old_cell) {
                cell.creatures.remove(&guid);
            }
        }
        let cell = Self::cell_of(creature.unit.world.position());
        self.cells.entry(cell).or_default().creatures.insert(guid);
        self.creatures.insert(guid, creature);
        // `MotionMaster::Initialize` at AddToWorld: default
        // generator from the spawn's movement type.
        self.motion_initialize(guid);
        // Spawn/addon auras (`AddAura` in LoadCreatureAddon /
        // template auras), applied now that the spell store
        // exists (Slice 5/6).
        let pending: Vec<u32> = self
            .creatures
            .get(&guid)
            .map(|c| c.pending_auras.clone())
            .unwrap_or_default();
        for spell_id in pending {
            self.apply_aura_by_id(guid, spell_id, guid);
        }
        // EventAI `JustRespawned` (Slice 6b).
        self.eventai_on_spawn(guid);
        // Instance-script `OnCreatureCreate` (Slice 7).
        self.instance_on_creature_create(guid);
        Some(guid)
    }

    /// `WorldObject::SummonCreature` subset (Slice 7):
    /// materialize a temporary creature from a
    /// synthetic spawn row and track its lifecycle.
    pub(crate) fn summon_creature(
        &mut self,
        entry: u32,
        position: Position,
        summoner: ObjectGuid,
        kind: u32,
        despawn_delay_ms: u32,
    ) -> Option<ObjectGuid> {
        self.stores.creature_templates.get(entry)?;
        self.summon_counter += 1;
        let guid_low = self.summon_counter;
        let mut creature_id = [0u32; mangos_world_data::spawns::MAX_CREATURE_IDS_PER_SPAWN];
        creature_id[0] = entry;
        let data = mangos_world_data::spawns::CreatureSpawn {
            creature_id,
            position: mangos_world_data::spawns::WorldLocation {
                // Map ids above u16 do not exist in 1.12 data.
                #[allow(clippy::cast_possible_truncation)]
                map_id: self.map_id as u16,
                x: position.x,
                y: position.y,
                z: position.z,
                o: position.o,
            },
            ..Default::default()
        };
        let guid = self.materialize_creature(guid_low, &data, None, 0)?;
        let despawn_at_ms = match kind {
            // Timed types count from summoning.
            1 | 2 | 3 | 9 | 10 | 11 if despawn_delay_ms > 0 => {
                Some(u64::from(self.now_ms) + u64::from(despawn_delay_ms))
            }
            _ => None,
        };
        self.summons.insert(
            guid,
            SummonState {
                summoner,
                kind,
                despawn_at_ms,
                despawn_delay_ms,
            },
        );
        if summoner.is_creature() {
            self.eventai_process_type(
                summoner,
                mangos_scripts_engine::event_type::SUMMONED_UNIT,
                Some(guid),
            );
        }
        Some(guid)
    }

    /// Temp-summon timers (`TemporarySummon::Update`
    /// subset).
    pub(crate) fn update_summons(&mut self) {
        if self.summons.is_empty() {
            return;
        }
        let now = u64::from(self.now_ms);
        let mut expired: Vec<(ObjectGuid, u32)> = Vec::new();
        for (guid, state) in &self.summons {
            let Some(at) = state.despawn_at_ms else {
                continue;
            };
            if at > now {
                continue;
            }
            // Out-of-combat timed types wait for combat
            // to end (checked every tick).
            if matches!(state.kind, 1..=4) && self.ai_in_combat(*guid) {
                continue;
            }
            expired.push((*guid, state.kind));
        }
        for (guid, kind) in expired {
            if kind == 11 {
                // TIMED_DEATH_AND_DEAD_DESPAWN: dies now.
                if self.unit_is_alive(guid) {
                    self.kill_unit(guid, None);
                }
            } else {
                self.despawn_creature_now(guid, u32::MAX);
            }
        }
    }

    pub(crate) fn spawn_gameobject(
        &mut self,
        guid_low: u32,
        respawns: &RespawnTimes,
    ) -> Option<ObjectGuid> {
        let data = self.stores.spawns.gameobject(guid_low).cloned()?;

        let respawn_time = respawns
            .gameobject(
                self.respawn_instance_key(data.instanciated_continent_instance_id),
                guid_low,
            )
            .map_or(0, |e| e.respawn_time);
        let now_unix = self.now_unix;
        let force_active = self.config.force_active_objects;
        let Ok(mut go) = GameObject::spawn_from_db(
            guid_low,
            &data,
            &self.stores.gameobject_templates,
            respawn_time,
            now_unix,
            false,
            force_active,
            &mut self.rng,
        ) else {
            return None;
        };
        go.world.object.set_is_new_object(true);
        go.world.object.add_to_world();
        if go.respawn_row_expired {
            self.events.push(MapEvent::ClearGameObjectRespawn {
                guid_low,
                instance_id: self.respawn_instance_key(data.instanciated_continent_instance_id),
            });
        }
        let guid = go.world.object.object_guid();
        if self.gameobjects.contains_key(&guid) {
            return None;
        }
        let cell = Self::cell_of(go.world.position());
        self.cells.entry(cell).or_default().gameobjects.insert(guid);
        self.gameobjects.insert(guid, go);
        self.instance_on_object_create(guid);
        Some(guid)
    }

    /// Serialized `UpdateData` -> packet events for one
    /// viewer (`SendObjectUpdates` tail: build + compress +
    /// send, normally via the Broadcaster; Slice 3 builds
    /// per viewer directly).
    pub(crate) fn send_update_data(&mut self, to: ObjectGuid, data: &mut UpdateData) {
        if !data.has_data() {
            return;
        }
        // has_transport: transports exist on the map; the
        // byte only tells the client to expect transport
        // movement blocks. Slice 3 spawns no moving
        // transports, so false matches C++ output until a
        // transport block is written.
        for packet in data.build_packets(false, self.config.compression) {
            self.events.push(MapEvent::Packet { to, packet });
        }
    }

    /// `ObjectView` for a unit (player or creature arm).
    /// The active spline rides along when the unit is
    /// server-moving (`SPLINE_ENABLED` in the info); the
    /// block builder checks the flag itself.
    fn unit_view(
        info: MovementInfo,
        position: Position,
        speeds: [f32; 6],
        is_creature: bool,
        spline: Option<&mangos_movement::MoveSpline>,
    ) -> ObjectView<'_> {
        ObjectView {
            movement: MovementSection::Living {
                info,
                position,
                speeds,
                is_creature,
                spline,
            },
            rewrites: mangos_entities::update::RewriteInputs::default(),
            melee_victim: None,
            transport_path_progress: None,
            is_transport: false,
        }
    }

    /// Viewer context for `viewer` looking at an object.
    fn viewer_for(is_self: bool) -> Viewer {
        Viewer {
            is_self,
            // Default visibility masks everything non-public;
            // health-percent rules resolve inside the
            // builder from values + viewer relation.
            can_see_health: false,
            ..Viewer::default()
        }
    }

    /// Camera-side visibility recomputation
    /// (`Player::UpdateObjectVisibility`): collects the
    /// full visible candidate set around the view point,
    /// then applies creates/destroys.
    fn scan_player_visibility(&mut self, viewer_guid: ObjectGuid) {
        let (view_point, search_radius) = {
            let Some(entry) = self.players.get(&viewer_guid) else {
                return;
            };
            let pos = entry.player.unit.world.position();
            // Grey distances extend the outgoing edge; the
            // search must cover the larger of the two.
            let r = self.config.visibility_distance
                + self.config.grey_unit.max(self.config.grey_object);
            (pos, r)
        };
        let mut candidates: Vec<ObjectGuid> = Vec::new();
        for cell in self.cell_area(view_point, search_radius) {
            let Some(objects) = self.cells.get(&cell) else {
                continue;
            };
            candidates.extend(objects.players.iter().copied());
            candidates.extend(objects.creatures.iter().copied());
            candidates.extend(objects.gameobjects.iter().copied());
            candidates.extend(objects.dynobjects.iter().copied());
        }
        // Objects already visible but outside the cell
        // search (they moved, or the viewer teleported) must
        // still be visited to fall out.
        if let Some(entry) = self.players.get(&viewer_guid) {
            for guid in &entry.visible {
                if !candidates.contains(guid) {
                    candidates.push(*guid);
                }
            }
        }
        let mut data = UpdateData::new();
        let mut effects = BuildEffects::default();
        for target in candidates {
            if target == viewer_guid {
                continue;
            }
            self.update_visibility_of(viewer_guid, target, &mut data, &mut effects);
        }
        self.send_update_data(viewer_guid, &mut data);
        if let Some(entry) = self.players.get_mut(&viewer_guid) {
            entry.last_notified = view_point;
            entry.scan_due = false;
        }
        // Objects that produced their first create are no
        // longer new (C++ clears in SendObjectUpdates/AOI
        // after CreateObject2).
    }

    /// `Player::UpdateVisibilityOf<T>` for one target.
    fn update_visibility_of(
        &mut self,
        viewer_guid: ObjectGuid,
        target: ObjectGuid,
        data: &mut UpdateData,
        effects: &mut BuildEffects,
    ) {
        let in_visible_list = self
            .players
            .get(&viewer_guid)
            .is_some_and(|e| e.visible.contains(&target));
        let visible_now = !self.hidden_objects.contains(&target)
            && self.is_visible(viewer_guid, target, in_visible_list);
        match (in_visible_list, visible_now) {
            (false, true) => {
                self.build_create_for(viewer_guid, target, data, effects);
                if let Some(e) = self.players.get_mut(&viewer_guid) {
                    e.visible.insert(target);
                }
            }
            (true, false) => {
                data.add_out_of_range_guid(target);
                if let Some(e) = self.players.get_mut(&viewer_guid) {
                    e.visible.remove(&target);
                }
            }
            _ => {}
        }
    }

    /// Distance + state visibility for one pair.
    fn is_visible(
        &self,
        viewer_guid: ObjectGuid,
        target: ObjectGuid,
        in_visible_list: bool,
    ) -> bool {
        let Some(viewer) = self.players.get(&viewer_guid).map(|e| &e.player) else {
            return false;
        };
        let view_point = viewer.unit.world.position();
        if let Some(target_player) = self.players.get(&target).map(|e| &e.player) {
            let dist = visibility::dist2d(view_point, target_player.unit.world.position());
            if !visibility::within_unit_visibility(
                dist,
                self.config.visibility_distance,
                self.config.grey_unit,
                in_visible_list,
                0.0,
            ) {
                return false;
            }
            return visibility::unit_visible_for_player(
                viewer,
                None,
                Some(target_player),
                None,
                self.config.aggro_rate,
            );
        }
        if let Some(target_creature) = self.creatures.get(&target) {
            let dist = visibility::dist2d(view_point, target_creature.unit.world.position());
            if !visibility::within_unit_visibility(
                dist,
                self.config.visibility_distance,
                self.config.grey_unit,
                in_visible_list,
                target_creature.visibility_modifier,
            ) {
                return false;
            }
            return target_creature.respawn_time <= self.now_unix
                && visibility::unit_visible_for_player(
                    viewer,
                    None,
                    None,
                    Some(target_creature),
                    self.config.aggro_rate,
                );
        }
        if let Some(target_go) = self.gameobjects.get(&target) {
            let dist = visibility::dist2d(view_point, target_go.world.position());
            return visibility::gameobject_visible(
                target_go,
                dist,
                self.config.visibility_distance,
                self.config.grey_object,
                in_visible_list,
                self.now_unix,
            );
        }
        if let Some(target_obj) = self.dynobjects.get(&target) {
            let dist = visibility::dist2d(view_point, target_obj.world.position());
            // Fork dynobjects use the object visibility
            // band (grey object distance).
            let edge = self.config.visibility_distance
                + if in_visible_list {
                    self.config.grey_object
                } else {
                    0.0
                };
            return dist <= edge;
        }
        false
    }

    fn build_create_for(
        &mut self,
        viewer_guid: ObjectGuid,
        target: ObjectGuid,
        data: &mut UpdateData,
        effects: &mut BuildEffects,
    ) {
        let viewer = Self::viewer_for(false);
        let now_ms = self.now_ms;
        if let Some(p) = self.players.get(&target).map(|e| &e.player) {
            let view = Self::unit_view(
                p.unit.world.movement_info,
                p.unit.world.position(),
                p.unit.speeds(),
                false,
                Some(&p.unit.spline),
            );
            build_create_block(&p.unit.world.object, &view, &viewer, data, now_ms, effects);
            return;
        }
        if let Some(c) = self.creatures.get(&target) {
            let view = Self::unit_view(
                c.unit.world.movement_info,
                c.unit.world.position(),
                c.unit.speeds(),
                true,
                Some(&c.unit.spline),
            );
            build_create_block(&c.unit.world.object, &view, &viewer, data, now_ms, effects);
            return;
        }
        if let Some(obj) = self.dynobjects.get(&target) {
            let view = ObjectView {
                movement: MovementSection::HasPosition(obj.world.position()),
                rewrites: mangos_entities::update::RewriteInputs::default(),
                melee_victim: None,
                transport_path_progress: None,
                is_transport: false,
            };
            build_create_block(&obj.world.object, &view, &viewer, data, now_ms, effects);
            return;
        }
        if let Some(go) = self.gameobjects.get(&target) {
            let _ = viewer_guid;
            let view = ObjectView {
                movement: MovementSection::HasPosition(go.stationary_position),
                rewrites: mangos_entities::update::RewriteInputs::default(),
                melee_victim: None,
                transport_path_progress: None,
                is_transport: go.world.object.update_flag().0
                    & mangos_protocol::update_data::ObjectUpdateFlags::TRANSPORT
                    != 0,
            };
            build_create_block(&go.world.object, &view, &viewer, data, now_ms, effects);
        }
    }

    /// `Player::SendInitSelf` create (own object to own
    /// session). The login flow sends this before the
    /// first tick; transports/owned summons join later.
    pub fn build_init_self(&mut self, guid: ObjectGuid) {
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        let view = Self::unit_view(
            player.unit.world.movement_info,
            player.unit.world.position(),
            player.unit.speeds(),
            false,
            Some(&player.unit.spline),
        );
        let viewer = Self::viewer_for(true);
        let mut data = UpdateData::new();
        let mut effects = BuildEffects::default();
        build_create_block(
            &player.unit.world.object,
            &view,
            &viewer,
            &mut data,
            self.now_ms,
            &mut effects,
        );
        self.send_update_data(guid, &mut data);
    }

    /// Applies a client movement packet (`HandleMovementOpcodes`
    /// relay half): verify, relocate, broadcast to viewers
    /// of the mover, schedule visibility/activation.
    /// Returns false when the mover is unknown or the info
    /// is rejected (anticheat is Phase 4-late; position
    /// validity is the only gate here).
    pub fn apply_movement(
        &mut self,
        guid: ObjectGuid,
        opcode: u16,
        info: MovementInfo,
        respawns: &RespawnTimes,
    ) -> bool {
        self.apply_movement_impl(guid, Some(opcode), info, respawns)
    }

    /// `HandleMoverRelocation` as used by the ACK
    /// handlers: relocate + visibility, no packet relay
    /// (observers get the change packet instead).
    pub fn relocate_from_client(
        &mut self,
        guid: ObjectGuid,
        info: MovementInfo,
        respawns: &RespawnTimes,
    ) -> bool {
        self.apply_movement_impl(guid, None, info, respawns)
    }

    fn apply_movement_impl(
        &mut self,
        guid: ObjectGuid,
        relay_opcode: Option<u16>,
        info: MovementInfo,
        respawns: &RespawnTimes,
    ) -> bool {
        if !mangos_movement::movement_info::is_valid_map_coord(
            info.pos.x, info.pos.y, info.pos.z, info.pos.o,
        ) {
            return false;
        }
        let (old_cell, old_position, new_position) = {
            let Some(entry) = self.players.get(&guid) else {
                return false;
            };
            let position = entry.player.unit.world.position();
            (Self::cell_of(position), position, info.pos)
        };
        // Fork `positionChanged`: exact float compares.
        #[allow(clippy::float_cmp)]
        let position_changed = old_position.x != new_position.x
            || old_position.y != new_position.y
            || old_position.z != new_position.z;
        let new_cell = Self::cell_of(new_position);
        // Moving interrupts a timed cast
        // (`Spell::CheckAllowedToMove` subset — no
        // cast-while-moving attributes in the slice).
        if self.casts.contains_key(&guid)
            && info
                .flags
                .has(mangos_movement::movement_info::MovementFlags::MASK_MOVING)
        {
            self.cancel_cast(guid, true);
        }
        // Grid transition loads new spawn cells.
        self.activate_cells_around(new_position, respawns);
        let relocation_due = {
            let Some(entry) = self.players.get_mut(&guid) else {
                return false;
            };
            entry.player.unit.world.movement_info = info;
            entry.player.unit.world.relocate(new_position);
            visibility::dist_sq(entry.last_notified, new_position)
                >= self.config.relocation_lower_limit_sq
        };
        if old_cell != new_cell {
            if let Some(c) = self.cells.get_mut(&old_cell) {
                c.players.remove(&guid);
            }
            self.cells.entry(new_cell).or_default().players.insert(guid);
        }
        if position_changed {
            // `CheckAreaExploreAndOutdoor` on relocation
            // (discovery arm; fork delays only under a
            // nonzero `Movement.RelocationVmapsCheckDelay`).
            self.check_area_explore(guid);
        }
        if relocation_due {
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.scan_due = true;
            }
            // Object-centric pass (`Unit::OnRelocated` ->
            // watchers' `UpdateVisibilityOf`): players who
            // see the mover re-evaluate it immediately.
            let watchers: Vec<ObjectGuid> = self
                .players
                .iter()
                .filter(|(g, e)| **g != guid && e.visible.contains(&guid))
                .map(|(g, _)| *g)
                .collect();
            for watcher in watchers {
                let mut data = UpdateData::new();
                let mut effects = BuildEffects::default();
                self.update_visibility_of(watcher, guid, &mut data, &mut effects);
                self.send_update_data(watcher, &mut data);
            }
        }
        // Relay: packed mover GUID + MovementInfo under the
        // original opcode, to every player who sees the
        // mover (PlayerBroadcaster listener set). Skipped
        // for ACK relocations.
        let Some(opcode) = relay_opcode else {
            return true;
        };
        let mut guid_bytes = Vec::new();
        guid.write_packed(&mut guid_bytes);
        let mut payload = PacketWriter::new();
        payload.write_bytes(&guid_bytes);
        info.write(&mut payload);
        let packet = WorldPacket::with_body(opcode, payload.into_vec());
        let viewers: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(g, e)| **g != guid && e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        for viewer in viewers {
            self.events.push(MapEvent::Packet {
                to: viewer,
                packet: packet.clone(),
            });
        }
        true
    }

    /// Forced heartbeat after an anticheat reject
    /// (`Unit::SendHeartBeat(true)` + the flag-test
    /// `RemoveUnitMovementFlag`): strip `remove_flags`
    /// from the stored info, restamp, and send
    /// `MSG_MOVE_HEARTBEAT` to the mover and watchers.
    pub fn send_forced_heartbeat(&mut self, guid: ObjectGuid, remove_flags: u32) {
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let info = &mut entry.player.unit.world.movement_info;
        info.flags.remove(remove_flags);
        info.update_time(self.now_ms);
        let info = *info;
        let mut guid_bytes = Vec::new();
        guid.write_packed(&mut guid_bytes);
        let mut payload = PacketWriter::new();
        payload.write_bytes(&guid_bytes);
        info.write(&mut payload);
        let packet = WorldPacket::with_body(
            mangos_protocol::opcode_table::opcode_id::MSG_MOVE_HEARTBEAT,
            payload.into_vec(),
        );
        self.events.push(MapEvent::Packet {
            to: guid,
            packet: packet.clone(),
        });
        let viewers: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(g, e)| **g != guid && e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        for viewer in viewers {
            self.events.push(MapEvent::Packet {
                to: viewer,
                packet: packet.clone(),
            });
        }
    }

    /// Zone + area ids at a position (anticheat
    /// transport-area test; explore resolution path).
    pub fn zone_area_at(&mut self, position: Position) -> (u32, u32) {
        let flag = self.area_flag_at(position);
        if flag == 0xffff {
            return (0, 0);
        }
        self.stores.areas.zone_and_area_by_flag(flag, self.map_id)
    }

    /// Claimed transport exists, is a transport, and
    /// is within 70 yd (fake-transport check).
    #[must_use]
    pub fn claimed_transport_ok(&self, t_guid: ObjectGuid, pos: Position) -> bool {
        let Some(go) = self.gameobject(t_guid) else {
            return false;
        };
        if go.info.go_type != 11 {
            return false;
        }
        let gp = go.world.position();
        let d2 = (gp.x - pos.x).powi(2) + (gp.y - pos.y).powi(2) + (gp.z - pos.z).powi(2);
        d2 < 70.0 * 70.0
    }

    /// One map tick (`Map::Update` subset). `diff_ms` is
    /// real elapsed milliseconds.
    pub fn tick(&mut self, diff_ms: u32, now_ms: u32, now_unix: u64) {
        self.now_ms = now_ms;
        self.now_unix = now_unix;
        self.players_timer.update(diff_ms);
        if self.players_timer.passed() {
            let player_guids: Vec<ObjectGuid> = self.players.keys().copied().collect();
            for guid in player_guids {
                self.update_player_zone(guid, false);
            }
            let accumulated = self.players_timer.current();
            debug_assert!(accumulated >= diff_ms);
            // `UpdatePlayers` -> `Player::Update` with
            // the accumulated gate diff (regen arm).
            self.update_players(accumulated);
            self.players_timer.reset();
            let due: Vec<ObjectGuid> = self
                .players
                .iter()
                .filter(|(_, e)| e.scan_due)
                .map(|(g, _)| *g)
                .collect();
            for guid in due {
                self.scan_player_visibility(guid);
            }
        }
        // Slice 4: spline integration then creature
        // movement generators (C++ runs both inside
        // `Creature::Update` during the grid pass, before
        // `SendObjectUpdates`; the relative order vs the
        // values pass only matters for same-tick creates,
        // which read post-move positions either way here).
        self.update_spline_movement(diff_ms);
        self.update_creature_motion(diff_ms);
        // Slice 6: creature AI (aggro/threat/chase/spell
        // lists) + melee swings.
        self.update_ai(diff_ms);
        // Slice 5: casts finish, auras tick/expire, dead
        // creatures respawn.
        self.update_casts(diff_ms);
        self.process_pending_procs();
        // Slice 6b: due generic script steps
        // (`ScriptsProcess` inside `Map::Update`).
        self.scripts_process();
        self.process_despawns_due();
        self.update_summons();
        self.instance_update(diff_ms);
        self.update_dynobjects(diff_ms);
        self.update_auras(diff_ms);
        self.update_respawns();
        self.update_weather(diff_ms);
        self.send_object_updates();
        // The second players gate runs the same scan for
        // late relocations.
        let due: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, e)| e.scan_due)
            .map(|(g, _)| *g)
            .collect();
        for guid in due {
            self.scan_player_visibility(guid);
        }
    }

    /// `SendObjectUpdates`: values blocks for updated
    /// objects to every viewer that sees them; masks clear
    /// afterwards (`clear_update_mask(false)`). The fork
    /// queues objects through `MarkForClientUpdate`'s
    /// delayed mark; here writers mostly write the raw
    /// `Values` store, so a non-empty mirror diff also
    /// queues the object (same outbound set either way).
    fn send_object_updates(&mut self) {
        fn queued(object: &mangos_entities::object::ObjectBase) -> bool {
            object.is_object_updated() || object.values().has_changes()
        }
        let mut updated: Vec<ObjectGuid> = Vec::new();
        for (guid, entry) in &self.players {
            if queued(&entry.player.unit.world.object) {
                updated.push(*guid);
            }
        }
        for (guid, creature) in &self.creatures {
            if queued(&creature.unit.world.object) {
                updated.push(*guid);
            }
        }
        for (guid, go) in &self.gameobjects {
            if queued(&go.world.object) {
                updated.push(*guid);
            }
        }
        if updated.is_empty() {
            return;
        }
        let viewers: Vec<ObjectGuid> = self.players.keys().copied().collect();
        for viewer_guid in viewers {
            let mut data = UpdateData::new();
            let mut effects = BuildEffects::default();
            for target in &updated {
                let sees = *target == viewer_guid
                    || self
                        .players
                        .get(&viewer_guid)
                        .is_some_and(|e| e.visible.contains(target));
                if !sees {
                    continue;
                }
                let viewer = Self::viewer_for(*target == viewer_guid);
                self.build_values_for(*target, &viewer, &mut data, &mut effects);
            }
            self.send_update_data(viewer_guid, &mut data);
        }
        for guid in &updated {
            self.clear_object_update(*guid);
        }
        // New-object state persists until the first values
        // pass after creation (matches create-then-update
        // ordering closely enough for Slice 3; the flag
        // only changes CREATE_OBJECT vs CREATE_OBJECT2).
        for guid in &updated {
            self.clear_is_new(*guid);
        }
    }

    fn build_values_for(
        &self,
        target: ObjectGuid,
        viewer: &Viewer,
        data: &mut UpdateData,
        effects: &mut BuildEffects,
    ) {
        if let Some(p) = self.players.get(&target).map(|e| &e.player) {
            let view = Self::unit_view(
                p.unit.world.movement_info,
                p.unit.world.position(),
                p.unit.speeds(),
                false,
                Some(&p.unit.spline),
            );
            build_values_block(&p.unit.world.object, &view, viewer, data, effects);
            return;
        }
        if let Some(c) = self.creatures.get(&target) {
            let view = Self::unit_view(
                c.unit.world.movement_info,
                c.unit.world.position(),
                c.unit.speeds(),
                true,
                Some(&c.unit.spline),
            );
            build_values_block(&c.unit.world.object, &view, viewer, data, effects);
            return;
        }
        if let Some(go) = self.gameobjects.get(&target) {
            let view = ObjectView {
                movement: MovementSection::HasPosition(go.stationary_position),
                rewrites: mangos_entities::update::RewriteInputs::default(),
                melee_victim: None,
                transport_path_progress: None,
                is_transport: go.world.object.update_flag().0
                    & mangos_protocol::update_data::ObjectUpdateFlags::TRANSPORT
                    != 0,
            };
            build_values_block(&go.world.object, &view, viewer, data, effects);
        }
    }

    fn clear_object_update(&mut self, guid: ObjectGuid) {
        if let Some(e) = self.players.get_mut(&guid) {
            e.player.unit.world.object.clear_update_mask(false);
        } else if let Some(c) = self.creatures.get_mut(&guid) {
            c.unit.world.object.clear_update_mask(false);
        } else if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.world.object.clear_update_mask(false);
        }
    }

    fn clear_is_new(&mut self, guid: ObjectGuid) {
        if let Some(e) = self.players.get_mut(&guid) {
            e.player.unit.world.object.set_is_new_object(false);
        } else if let Some(c) = self.creatures.get_mut(&guid) {
            c.unit.world.object.set_is_new_object(false);
        } else if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.world.object.set_is_new_object(false);
        }
    }

    /// Death-state helper for tests/tools (logout cleanup
    /// and kill paths use the aggregate directly later).
    #[must_use]
    pub fn player_death_state(&self, guid: ObjectGuid) -> Option<DeathState> {
        self.players.get(&guid).map(|e| e.player.unit.death_state)
    }
}
