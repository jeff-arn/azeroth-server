//! `OutdoorPvP` Eastern Plaguelands towers (`OutdoorPvPEP.h/.cpp`:
//! map 0 (`OutdoorPvP_eastern_plaguelands::GetMapId`), buff zones
//! 139/2017/2057, four capture-point sliders in registration order
//! EWT/PWT/CGT/NPT (`OutdoorPvPEP.cpp:935-938`), tower-state bitmasks
//! in `Towers`-enum order EWT/NPT/PWT/CGT (`OutdoorPvPEP.h:119-127`).
//!
//! Map-local only (append-only DB lock): slider values, controls,
//! buff counts and spawned-tower guids live on [`EpState`] on one map-0
//! `Map` and reset on map drop. The fork persists nothing for EP
//! (tower controls/sliders never touch saved variables), so there is
//! no persistence deviation to record.
//!
//! Deviations recorded at the use sites: capture radius/max/min/
//! neutral read from the banner `GameObjectInfo` capture-point words
//! (`GameObjectDefines.h:489-511`) with a neutralfallback when the
//! template row is absent; GO artkit rides the `GAMEOBJECT_ARTKIT`
//! update field plus the existing custom-anim packet shape (fork
//! `SetGoArtKit` + `SendGameObjectCustomAnim`); tower sounds reuse the
//! existing `SMSG_PLAY_SOUND` viewer packet (fork `PlayDirectSound` on
//! the banner); squad formation (`JoinCreatureGroup`) is spawn-only;
//! Spirit of Victory waypoints are spawn + aura (no waypoint engine
//! hook); taxi unlock is spawn-only (normal flight path once the
//! flightmaster lives); graveyard links ride a map-local overlay (the
//! DB link store is read-only shared state).

// Fork-sniffed coordinate/rotation literals (same convention as the
// coordinate-heavy `area_triggers.rs` impl blocks).
#![allow(clippy::unreadable_literal, clippy::excessive_precision)]

use std::collections::{HashMap, HashSet};

use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::aura as aura_type;

use crate::battlegrounds::build_update_world_state_packet;
use crate::map::{Map, MapEvent};
use crate::motion::UNIT_FLAG_TAXI_FLIGHT;
use crate::spells::{CastOptions, CastTarget};

/// Eastern Kingdoms (`OutdoorPvP_eastern_plaguelands::GetMapId`).
pub const EP_MAP_ID: u32 = 0;
/// `EP_BuffZones` (`OutdoorPvPEP.h:210`).
pub const EP_BUFF_ZONES: [u32; 3] = [139, 2017, 2057];
/// `EP_Zone` broadcast zone (`OutdoorPvPEP.h:203`).
pub const EP_ZONE: u32 = 139;
/// `TFV_area` second graveyard-link zone (`OutdoorPvPEP.h:204`).
pub const EP_TFV_AREA: u32 = 2258;
/// `EP_GraveYardId` (`OutdoorPvPEP.h:206`).
pub const EP_GRAVEYARD_ID: u32 = 927;
/// `OUTDOORPVP_OBJECTIVE_UPDATE_INTERVAL` (`ZoneScriptMgr.h:21`).
pub const EP_UPDATE_INTERVAL_MS: u32 = 1000;

/// Towers-enum order (`OutdoorPvPEP.h:119-127`), also the
/// `EP_Controls` index order.
pub const EP_TOWER_EWT: usize = 0;
/// Northpass index in `Towers`/`EP_Controls` order.
pub const EP_TOWER_NPT: usize = 1;
/// Plaguewood index in `Towers`/`EP_Controls` order.
pub const EP_TOWER_PWT: usize = 2;
/// Crown Guard index in `Towers`/`EP_Controls` order.
pub const EP_TOWER_CGT: usize = 3;

/// Capture-point banner entries in tower order
/// (`EP_CapturePoints`, `OutdoorPvPEP.h:218`).
pub const EP_CAPTURE_ENTRIES: [u32; 4] = [182_097, 181_899, 182_098, 182_096];

/// Capture-point banner positions in tower order
/// (`EP_CapturePoints` x/y/z/o).
pub const EP_CAPTURE_POS: [Position; 4] = [
    Position::new(2574.51, -4794.89, 144.704, -1.45003),
    Position::new(3181.08, -4379.36, 174.123, -2.03472),
    Position::new(2962.71, -3042.31, 154.789, 2.08426),
    Position::new(1860.85, -3731.23, 196.716, -2.53214),
];

/// Capture-point banner rotations in tower order (rot0-3).
pub const EP_CAPTURE_ROT: [[f32; 4]; 4] = [
    [-0.097056, 0.095578, -0.656229, 0.742165],
    [-0.065392, 0.119494, -0.842275, 0.521553],
    [-0.074807, -0.113837, 0.855928, 0.49883],
    [0.033967, -0.131914, 0.944741, -0.298177],
];

/// Tower-banner (182106) positions (`EP_TowerFlags`, h:220).
pub const EP_TOWER_FLAG_POS: [Position; 8] = [
    Position::new(1838.42, -3703.56, 167.713, 0.890117),
    Position::new(1877.6, -3716.76, 167.188, 1.74533),
    Position::new(2539.61, -4801.55, 115.766, 2.00713),
    Position::new(2569.6, -4772.93, 115.399, 2.72271),
    Position::new(2975.5, -3060.36, 125.108, 5.23599),
    Position::new(2992.63, -3022.95, 125.593, 3.03684),
    Position::new(3148.17, -4365.51, 145.029, 1.53589),
    Position::new(3188.76, -4358.5, 144.555, 1.97222),
];

/// Tower-banner rotations (rot2/rot3 carry the facing).
pub const EP_TOWER_FLAG_ROT: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.43051, 0.902586],
    [0.0, 0.0, 0.766044, 0.642789],
    [0.0, 0.0, 0.843391, 0.5373],
    [0.0, 0.0, 0.978148, 0.207912],
    [0.0, 0.0, -0.5, 0.866025],
    [0.0, 0.0, 0.998629, 0.0523532],
    [0.0, 0.0, 0.694658, 0.71934],
    [0.0, 0.0, 0.833885, 0.551938],
];

/// Tower-flag pair per tower in tower order (EWT [2,3], NPT [6,7],
/// PWT [4,5], CGT [0,1]; `GobjectSummons` h:169-185).
pub const EP_TOWER_FLAG_SLOTS: [[usize; 2]; 4] = [[2, 3], [6, 7], [4, 5], [0, 1]];
/// Tower-banner entry (`GOBJECT_TOWER_BANNER`, h:108).
pub const EP_TOWER_BANNER_ENTRY: u32 = 182_106;

/// Flag-flare entries `[Alliance, Horde]` per flare pair.
pub const EP_FLARE_ENTRIES: [u32; 2] = [181_852, 181_853];

/// Flag-flare rotations (rot2/rot3 carry the facing).
pub const EP_FLARE_ROT: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.694658, 0.71934],
    [0.0, 0.0, 0.309016, 0.951057],
    [0.0, 0.0, 0.788011, 0.615662],
    [0.0, 0.0, 0.999048, 0.0436193],
    [0.0, 0.0, 0.0871553, 0.996195],
    [0.0, 0.0, 0.976295, 0.216442],
    [0.0, 0.0, -0.446198, 0.894935],
    [0.0, 0.0, 0.833885, 0.551938],
];

/// NPT shrine/aura rotation (`EP_NPT_LordaeronShrine`).
pub const EP_SHRINE_ROT: [f32; 4] = [0.0, 0.0, 0.748956, 0.66262];

/// CGT banner-aura rotation (`EP_CGT_BannerAuraGraveYard`).
pub const EP_BANNER_AURA_ROT: [f32; 4] = [0.0, 0.0, 0.66913, 0.743145];

/// Flag-flare positions (`EP_TowerFlares`, h:222-231).
pub const EP_FLARE_POS: [Position; 8] = [
    Position::new(1855.66, -3725.0, 197.044, 1.53589),
    Position::new(1853.12, -3722.62, 197.406, 0.628317),
    Position::new(2563.26, -4795.15, 145.852, 1.81514),
    Position::new(2565.27, -4797.59, 147.846, 3.05433),
    Position::new(3171.86, -4377.2, 174.898, 0.174532),
    Position::new(3169.76, -4375.13, 175.458, 2.70526),
    Position::new(2971.41, -3038.36, 157.492, 5.35816),
    Position::new(2973.17, -3037.19, 156.443, 1.97222),
];

/// Flare pair per tower in tower order (EWT [2,3], NPT [4,5],
/// PWT [6,7], CGT [0,1]).
pub const EP_FLARE_SLOTS: [[usize; 2]; 4] = [[2, 3], [4, 5], [6, 7], [0, 1]];

/// Tower rewards: buffer NPC entries (`NPC_TOWER_BUFFER_*`, h:90-91).
pub const EP_BUFFER_ENTRY_A: u32 = 17_794;
/// Horde tower buffer entry.
pub const EP_BUFFER_ENTRY_H: u32 = 17_795;

/// Buffer spawn rows (`EP_BufferNPCs`, h:216): entry, teamval, pos.
pub const EP_BUFFER_ROWS: [(u32, u32, Position); 8] = [
    (
        17_794,
        469,
        Position::new(1860.59, -3730.8, 197.854, 2.54818),
    ),
    (
        17_794,
        469,
        Position::new(2574.12, -4795.33, 145.871, 5.11381),
    ),
    (
        17_794,
        469,
        Position::new(2962.6, -3041.96, 155.835, 3.00197),
    ),
    (
        17_794,
        469,
        Position::new(3180.54, -4379.31, 175.275, 3.57792),
    ),
    (
        17_795,
        67,
        Position::new(1860.48, -3731.34, 197.778, 2.42601),
    ),
    (
        17_795,
        67,
        Position::new(2574.0, -4794.79, 145.881, 1.95477),
    ),
    (
        17_795,
        67,
        Position::new(2963.02, -3041.9, 155.965, 4.27606),
    ),
    (
        17_795,
        67,
        Position::new(3180.48, -4379.07, 174.995, 2.74017),
    ),
];

/// Buffer row per tower `[Alliance, Horde]` (`Buffers` h:142-152:
/// CGT 0/4, EWT 1/5, PWT 2/6, NPT 3/7) in tower order.
pub const EP_BUFFER_ROWS_PER_TOWER: [[usize; 2]; 4] = [[1, 5], [3, 7], [2, 6], [0, 4]];

/// EWT squad entries (`NPC_LORDAERON_*`, h:92-95).
pub const EP_COMMANDER_A: u32 = 17_635;
/// EWT soldier entry (Alliance).
pub const EP_SOLDIER_A: u32 = 17_647;
/// EWT commander entry (Horde veteran).
pub const EP_COMMANDER_H: u32 = 17_995;
/// EWT soldier entry (Horde fighter).
pub const EP_SOLDIER_H: u32 = 17_996;

/// EWT squad positions shared by both factions
/// (`EP_EWT_Summons_A/H`, h:237-239).
pub const EP_SQUAD_POS: [Position; 5] = [
    Position::new(2532.85, -4764.92, 103.617, 2.35619),
    Position::new(2533.33, -4769.31, 104.396, 2.37365),
    Position::new(2537.34, -4773.92, 105.941, 2.21657),
    Position::new(2537.77, -4765.94, 104.432, 2.3911),
    Position::new(2542.57, -4770.22, 106.145, 2.42601),
];

/// NPT curing-shrine rows (`EP_NPT_LordaeronShrine`, h:233):
/// (entry, pos) for [shrine A, aura A, shrine H, aura H].
pub const EP_SHRINE_ROWS: [(u32, Position); 4] = [
    (181_682, Position::new(3167.72, -4355.91, 138.785, 1.69297)),
    (180_100, Position::new(3167.72, -4355.91, 138.785, 1.69297)),
    (181_955, Position::new(3167.5, -4356.25, 138.821, 1.69297)),
    (180_101, Position::new(3167.5, -4356.25, 138.821, 1.69297)),
];

/// CGT banner-aura rows (`EP_CGT_BannerAuraGraveYard`, h:235):
/// index 0 Horde, 1 Alliance.
pub const EP_BANNER_AURA_ROWS: [(u32, Position); 2] = [
    (180_422, Position::new(1985.47, -3653.88, 120.172, 1.46608)),
    (180_421, Position::new(1985.47, -3653.88, 120.172, 1.46608)),
];

/// PWT flightmaster (`EP_PWT_FlightMaster`, h:241).
pub const EP_FLIGHTMASTER_ENTRY: u32 = 17_209;
/// PWT flightmaster position.
pub const EP_FLIGHTMASTER_POS: Position = Position::new(2987.5, -3049.11, 120.126, 5.75959);
/// PWT flightmaster faction templates (cpp:916).
pub const EP_FLIGHTMASTER_FACTION: [u32; 2] = [774, 775];

/// CGT Spirit of Victory (`EP_CGT_SpiritOfVictory`, h:243).
pub const EP_SPIRIT_ENTRY: u32 = 18_039;
/// Spirit of Victory position.
pub const EP_SPIRIT_POS: Position = Position::new(1856.58, -3714.72, 194.637, 0.762214);

/// Per-tower world-state ids in `FillInitialWorldStates` order
/// (A, H, A-prog, H-prog, A-cont, H-cont, neutral) in tower order
/// (`WorldStates.h:118-148`).
pub const EP_TOWER_WS: [[u32; 7]; 4] = [
    [2354, 2356, 2357, 2358, 2359, 2360, 2361],
    [2372, 2373, 2364, 2365, 2362, 2363, 2352],
    [2370, 2371, 2368, 2369, 2366, 2367, 2353],
    [2378, 2379, 2376, 2377, 2374, 2375, 2355],
];

/// Tower-state bitmask per WS column (`TowerStates`, h:188-197).
pub const EP_TOWER_WS_BITS: [u32; 7] = [32, 64, 8, 16, 2, 4, 1];

/// `WS_UI_TOWER_COUNT_ALLIANCE` (`WorldStates.h:154`).
pub const EP_WS_COUNT_A: u32 = 2327;
/// `WS_UI_TOWER_COUNT_HORDE` (`WorldStates.h:155`).
pub const EP_WS_COUNT_H: u32 = 2328;
/// `WS_UI_TOWER_SLIDER_DISPLAY` (`WorldStates.h:150`).
pub const EP_WS_SLIDER_DISPLAY: u32 = 2426;
/// `WS_UI_TOWER_SLIDER_POSITION` (`WorldStates.h:151`).
pub const EP_WS_SLIDER_POSITION: u32 = 2427;
/// `WS_UI_TOWER_SLIDER_N` (`WorldStates.h:152`).
pub const EP_WS_SLIDER_N: u32 = 2428;

/// Objective states (`ZoneScript.h:42-50`).
pub const EP_STATE_NEUTRAL: u8 = 0;
/// Alliance-contested objective state.
pub const EP_STATE_A_CONTESTED: u8 = 1;
/// Horde-contested objective state.
pub const EP_STATE_H_CONTESTED: u8 = 2;
/// Alliance-progressing objective state.
pub const EP_STATE_A_PROGRESSING: u8 = 3;
/// Horde-progressing objective state.
pub const EP_STATE_H_PROGRESSING: u8 = 4;
/// Alliance-held objective state.
pub const EP_STATE_A: u8 = 5;
/// Horde-held objective state.
pub const EP_STATE_H: u8 = 6;

/// Tower-state bitmask (`TowerStates`, h:188-197).
pub const EP_TS_NEUTRAL: u32 = 1;
/// Alliance-contested tower bit.
pub const EP_TS_A_CONTESTED: u32 = 2;
/// Horde-contested tower bit.
pub const EP_TS_H_CONTESTED: u32 = 4;
/// Alliance-progressing tower bit.
pub const EP_TS_A_PROGRESSING: u32 = 8;
/// Horde-progressing tower bit.
pub const EP_TS_H_PROGRESSING: u32 = 16;
/// Alliance-held tower bit.
pub const EP_TS_A: u32 = 32;
/// Horde-held tower bit.
pub const EP_TS_H: u32 = 64;

/// Tower capture test (`SPELL_TOWER_CAPTURE_TEST_DND`, h:69).
pub const EP_SPELL_CAPTURE_TEST: u32 = 30_882;
/// Tower capture (`SPELL_TOWER_CAPTURE_DND`, h:70).
pub const EP_SPELL_CAPTURE: u32 = 31_929;
/// Tower kill credit (`SPELL_TXT_TOWOW_TOWER_KILL_CREDIT_DND`, h:68).
pub const EP_SPELL_KILL_CREDIT: u32 = 32_061;
/// Spirit spawn-in aura on the Alliance flightmaster (h:71).
pub const EP_SPELL_SPIRIT_SPAWN_IN: u32 = 17_321;
/// Red big particles on the Horde flightmaster (h:73).
pub const EP_SPELL_SPIRIT_PARTICLES_RED: u32 = 31_309;
/// Super-big particles on the Alliance Spirit of Victory (h:74).
pub const EP_SPELL_VICTORY_A: u32 = 31_954;
/// Red super-big particles on the Horde Spirit of Victory (h:75).
pub const EP_SPELL_VICTORY_H: u32 = 31_951;

/// Echoes of Lordaeron ranks, Alliance (`EP_AllianceBuffs`, h:199).
pub const EP_BUFFS_A: [u32; 4] = [11_413, 11_414, 11_415, 1386];
/// Echoes of Lordaeron ranks, Horde (`EP_HordeBuffs`, h:201).
pub const EP_BUFFS_H: [u32; 4] = [30_880, 30_683, 30_682, 29_520];

/// Tower-taken broadcast texts `[Alliance, Horde]` per tower
/// (EWT 13631/13636, NPT 13630/13635, PWT 13629/13634,
/// CGT 13632/13633; `BroadCastTexts` h:28-35).
pub const EP_BCT_TOWER: [[i32; 2]; 4] = [
    [13631, 13636],
    [13630, 13635],
    [13629, 13634],
    [13632, 13633],
];
/// All-four sweep texts `[Alliance, Horde]` (h:36-37).
pub const EP_BCT_SWEEP: [i32; 2] = [13638, 13637];

/// Victory sounds `[Alliance, Horde]` (8455/8454, h:46-47).
pub const EP_SOUND_VICTORY: [u32; 2] = [8455, 8454];
/// Warning sounds `[Alliance, Horde]` (8332/8333, h:48-49).
pub const EP_SOUND_WARNING: [u32; 2] = [8332, 8333];
/// Flag-captured sounds `[Alliance, Horde]` (8173/8213, h:43-44).
pub const EP_SOUND_CAPTURED: [u32; 2] = [8173, 8213];

/// Banner artkit/animation `[Alliance, Horde, Neutral]`
/// (`Artkit` h:52-57, `Animation` h:59-64).
pub const EP_BANNER_ART: [(u32, u32); 3] = [(2, 1), (1, 0), (21, 2)];

/// Tower quest-doodad entries in tower order
/// (EWT 17690, NPT 17696, PWT 17698, CGT 17689; `NPCs` h:96-99).
pub const EP_DOODAD_ENTRIES: [u32; 4] = [17_690, 17_696, 17_698, 17_689];

/// Capture-point DBC words (`GameObjectDefines.h:489-511`):
/// radius (0), worldState1 (2), worldstate2 (3), neutralPercent (12),
/// worldstate3 (13), minTime (16), maxTime (17).
pub const EP_CAPTURE_GO_TYPE: u32 = 29;

/// Fallback capture params when the banner template row is absent
/// (production rows come from the DB; the fallback keeps the neutral
/// 20% grey band and a 60s solo full-capture).
pub const EP_FALLBACK_RADIUS: f32 = 30.0;
/// Fallback full-shift value.
pub const EP_FALLBACK_MAX_VALUE: f32 = 60.0;
/// Fallback minimum capture time (seconds).
pub const EP_FALLBACK_MIN_TIME: f32 = 60.0;
/// Fallback neutral band percent.
pub const EP_FALLBACK_NEUTRAL_PCT: u32 = 20;

/// Capture pacing read from (or defaulted for) the banner template.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureParams {
    /// Capture radius (yards).
    pub radius: f32,
    /// Full-shift value (`capturePoint.maxTime`).
    pub max_value: f32,
    /// Grey-band half width (`maxValue * neutralPct / 100`).
    pub min_value: f32,
    /// Maximum capture speed (`maxValue / minTime`, minTime 0 -> 60).
    pub max_speed: f32,
    /// Neutral slider percent (`capturePoint.neutralPercent`).
    pub neutral_pct: u32,
}

/// Slider step outcome (`OPvPCapturePoint::Update` return shape,
/// `ZoneScript.cpp:288-430`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SliderOutcome {
    /// State changed: run `ChangeState`, flag `objective_changed`.
    pub state_changed: bool,
    /// Slider moved without a state change: send position.
    pub send_phase: bool,
}

/// One tower's capture state (`OPvPCapturePoint` members for one EP
/// tower plus its `m_TowerState` bitmask).
#[derive(Clone, Debug)]
pub struct EpTower {
    /// Slider value (`m_value`, +Alliance / -Horde).
    pub value: f32,
    /// Slider percent (`m_valuePct`).
    pub value_pct: u32,
    /// Last numeric-superiority diff (`m_factDiff`).
    pub fact_diff: i32,
    /// Controlling team (`m_team`).
    pub team: Team,
    /// Objective state (`m_state`).
    pub state: u8,
    /// Previous objective state (`m_oldState`).
    pub old_state: u8,
    /// Display bitmask (`m_TowerState`).
    pub tower_state: u32,
    /// Active Alliance players in radius (`m_activePlayers[0]`).
    pub active_a: HashSet<ObjectGuid>,
    /// Active Horde players in radius (`m_activePlayers[1]`).
    pub active_h: HashSet<ObjectGuid>,
    /// Current banner artkit/animation intent.
    pub artkit: (u32, u32),
    /// Last tower sound intent.
    pub last_sound: u32,
    /// Live spawned creatures by tower-local slot.
    pub creatures: HashMap<u32, ObjectGuid>,
    /// Live spawned gameobjects by tower-local slot.
    pub objects: HashMap<u32, ObjectGuid>,
}

/// EP runtime state for one map-0 `Map` (`OutdoorPvPEP` members).
#[derive(Clone, Debug)]
pub struct EpState {
    /// Four towers in `Towers`-enum order.
    pub towers: [EpTower; 4],
    /// Tower controls in `Towers`-enum order (`EP_Controls`).
    pub controls: [Team; 4],
    /// Cached Alliance tower count.
    pub allies: u32,
    /// Cached Horde tower count.
    pub hordes: u32,
    /// Set by slider transitions, consumed next tick
    /// (`m_objective_changed`, cpp:942-964).
    pub objective_changed: bool,
    /// Update accumulator (1000ms cadence).
    pub accum_ms: u32,
    /// Crown Guard graveyard-link overlay (`LinkGraveYard`).
    pub graveyard_team: Option<Team>,
    /// Capture-point/banner spawns materialized.
    pub initialized: bool,
    /// Pending tower spawn intents (sync queue).
    pub(crate) pending: Vec<SpawnIntent>,
}

impl Default for EpTower {
    fn default() -> Self {
        Self {
            value: 0.0,
            value_pct: 0,
            fact_diff: 0,
            team: Team::None,
            state: EP_STATE_NEUTRAL,
            old_state: EP_STATE_NEUTRAL,
            tower_state: EP_TS_NEUTRAL,
            active_a: HashSet::new(),
            active_h: HashSet::new(),
            artkit: (21, 2),
            last_sound: 0,
            creatures: HashMap::new(),
            objects: HashMap::new(),
        }
    }
}

impl Default for EpState {
    fn default() -> Self {
        Self {
            towers: [
                EpTower::default(),
                EpTower::default(),
                EpTower::default(),
                EpTower::default(),
            ],
            controls: [Team::None, Team::None, Team::None, Team::None],
            allies: 0,
            hordes: 0,
            objective_changed: false,
            accum_ms: 0,
            graveyard_team: None,
            initialized: false,
            pending: Vec::new(),
        }
    }
}

impl EpState {
    /// `EP_BuffZones` membership (`SetupZoneScript`, cpp:932-933).
    #[must_use]
    pub fn is_ep_zone(zone_id: u32) -> bool {
        EP_BUFF_ZONES.contains(&zone_id)
    }

    /// `FillInitialWorldStates` 33 pairs (cpp:1025-1036): counts,
    /// slider display/position/n, then 7 per tower from the
    /// `m_TowerState` bitmask.
    #[must_use]
    pub fn fill_initial(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::with_capacity(33);
        out.push((EP_WS_COUNT_A, self.allies));
        out.push((EP_WS_COUNT_H, self.hordes));
        out.push((EP_WS_SLIDER_DISPLAY, 0));
        out.push((EP_WS_SLIDER_POSITION, 50));
        out.push((EP_WS_SLIDER_N, 100));
        for (tower, row) in EP_TOWER_WS.iter().enumerate() {
            let bits = self.towers[tower].tower_state;
            for (slot, ws) in row.iter().enumerate() {
                out.push((*ws, u32::from(bits & EP_TOWER_WS_BITS[slot] != 0)));
            }
        }
        out
    }

    /// `SendRemoveWorldStates` zeroes (cpp:1038-1077): same 33
    /// fields, all zero (fork skips them on logout; the caller
    /// cannot see logout, recorded same as SI).
    #[must_use]
    pub fn leave_zeroes(&self) -> Vec<(u32, u32)> {
        self.fill_initial()
            .into_iter()
            .map(|(field, _)| (field, 0))
            .collect()
    }

    /// Echoes rank from towers held: 1..4, else 0 (cpp:972-979,
    /// `BuffTeams` then casts `EP_*Buffs[held - 1]`).
    #[must_use]
    pub fn buff_rank(&self, team: Team) -> u32 {
        let held = self
            .controls
            .iter()
            .filter(|&&c| c == team && team != Team::None)
            .count() as u32;
        if (1..=4).contains(&held) { held } else { 0 }
    }
}

impl EpTower {
    /// Slider step for pruned/scanned counts (`OPvPCapturePoint::Update`
    /// without the radius scan, `ZoneScript.cpp:288-430`): numeric
    /// superiority drives `m_value`, clamped to `maxSpeed * diff` per
    /// tick and `[-maxValue, maxValue]` overall; grey-band challenger
    /// names the contested state; `m_valuePct` is the ceiling percent;
    /// only a state change or a moved slider (or a fact-diff settling
    /// to zero) sends an update.
    pub fn slider_step(
        &mut self,
        diff_ms: u32,
        allies: usize,
        hordes: usize,
        params: CaptureParams,
    ) -> SliderOutcome {
        #[allow(clippy::cast_precision_loss)]
        let diff = diff_ms as f32;
        #[allow(clippy::cast_precision_loss)]
        let fact_diff = (allies as f32 - hordes as f32) * diff / 1000.0;
        let max_value = params.max_value;
        let max_diff = params.max_speed * diff;
        let (fact_diff, challenger) = if fact_diff < 0.0 {
            if self.value <= -max_value {
                return SliderOutcome::default();
            }
            (fact_diff.max(-max_diff), Team::Horde)
        } else if fact_diff > 0.0 {
            if self.value >= max_value {
                return SliderOutcome::default();
            }
            (fact_diff.min(max_diff), Team::Alliance)
        } else {
            (0.0, Team::None)
        };

        let old_pct = self.value_pct;
        let old_fact = self.fact_diff;
        let old_team = self.team;
        self.old_state = self.state;

        self.value += fact_diff;
        if self.value <= -params.min_value {
            if self.value <= -max_value {
                self.value = -max_value;
                self.state = EP_STATE_H;
            } else {
                self.state = EP_STATE_H_PROGRESSING;
            }
            self.team = Team::Horde;
        } else if self.value >= params.min_value {
            if self.value >= max_value {
                self.value = max_value;
                self.state = EP_STATE_A;
            } else {
                self.state = EP_STATE_A_PROGRESSING;
            }
            self.team = Team::Alliance;
        } else if challenger == Team::Alliance {
            self.state = EP_STATE_A_CONTESTED;
            self.team = Team::None;
        } else if challenger == Team::Horde {
            self.state = EP_STATE_H_CONTESTED;
            self.team = Team::None;
        } else {
            self.state = EP_STATE_NEUTRAL;
            self.team = Team::None;
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            self.value_pct = ((self.value + max_value) / (2.0 * max_value) * 100.0).ceil() as u32;
        }
        #[allow(clippy::cast_possible_wrap)]
        {
            self.fact_diff = allies as i32 - hordes as i32;
        }

        if self.old_state != self.state {
            let _ = old_team;
            return SliderOutcome {
                state_changed: true,
                send_phase: false,
            };
        }
        if self.value_pct != old_pct || (old_fact != self.fact_diff && self.fact_diff == 0) {
            return SliderOutcome {
                state_changed: false,
                send_phase: true,
            };
        }
        SliderOutcome::default()
    }
}

/// Tower-local spawn slots (fork per-tower `m_Creatures`/`m_Objects`
/// keys; `NPCSummons` h:154-167 / `GobjectSummons` h:169-185).
pub const EP_SLOT_SQUAD: [u32; 5] = [0, 1, 2, 3, 4];
/// EWT buffer slot (`EASTWALL_TOWER_BUFFER`, h:164).
pub const EP_SLOT_EWT_BUFFER: u32 = 8;
/// EWT banner object slots.
pub const EP_SLOT_EWT_BANNERS: [u32; 2] = [2, 3];
/// EWT flare object slot (`EASTWALL_TOWER_FLAG_FLARE`, h:181).
pub const EP_SLOT_EWT_FLARE: u32 = 10;
/// NPT buffer slot (`NORTHPASS_TOWER_BUFFER`, h:165).
pub const EP_SLOT_NPT_BUFFER: u32 = 9;
/// NPT banner object slots.
pub const EP_SLOT_NPT_BANNERS: [u32; 2] = [6, 7];
/// NPT flare object slot.
pub const EP_SLOT_NPT_FLARE: u32 = 11;
/// NPT shrine object slot.
pub const EP_SLOT_NPT_SHRINE: u32 = 12;
/// NPT banner-aura object slot.
pub const EP_SLOT_NPT_AURA: u32 = 13;
/// PWT flightmaster slot (`PLAGUEWOOD_TOWER_FLIGHTMASTER`, h:161).
pub const EP_SLOT_PWT_FLIGHTMASTER: u32 = 5;
/// PWT buffer slot (`PLAGUEWOOD_TOWER_BUFFER`, h:162).
pub const EP_SLOT_PWT_BUFFER: u32 = 6;
/// PWT banner object slots.
pub const EP_SLOT_PWT_BANNERS: [u32; 2] = [4, 5];
/// PWT flare object slot.
pub const EP_SLOT_PWT_FLARE: u32 = 8;
/// CGT buffer slot (`CROWN_GUARD_TOWER_BUFFER`, h:163).
pub const EP_SLOT_CGT_BUFFER: u32 = 7;
/// CGT Spirit of Victory slot (`CROWN_GUARD_TOWER_SPIRITOFVICTORY`).
pub const EP_SLOT_CGT_SPIRIT: u32 = 10;
/// CGT banner object slots.
pub const EP_SLOT_CGT_BANNERS: [u32; 2] = [0, 1];
/// CGT flare object slot.
pub const EP_SLOT_CGT_FLARE: u32 = 9;
/// CGT banner-aura object slot.
pub const EP_SLOT_CGT_AURA: u32 = 14;
/// Capture-point banner object slot (local; fork tracks it
/// separately in `m_capturePointGUID`).
pub const EP_SLOT_CAPTURE: u32 = 100;

/// Buffer creature slot per tower (`NPCSummons` buffer rows).
pub const EP_BUFFER_SLOT: [u32; 4] = [
    EP_SLOT_EWT_BUFFER,
    EP_SLOT_NPT_BUFFER,
    EP_SLOT_PWT_BUFFER,
    EP_SLOT_CGT_BUFFER,
];

/// `PLAYER_EXTRA_GM_ON` (`AbstractPlayer.h:12`, `area_triggers.rs:21`).
const EP_GM_FLAG: u32 = 0x0001;
/// `PLAYER_FLAGS_PVP_DESIRED` (player.rs:93; `IsPvPDesired`).
const EP_PVP_DESIRED_FLAG: u32 = 0x0000_0200;

/// Pending tower spawn (intent materialized by `ep_sync_spawns`;
/// fork `AddCreature`/`AddObject` spawn immediately — the intent
/// queue only defers until a summoner player exists and lets tests
/// assert the exact spawn set without DB templates).
#[derive(Clone, Debug)]
pub(crate) struct SpawnIntent {
    tower: usize,
    creature: bool,
    slot: u32,
    entry: u32,
    pos: Position,
    rot: [f32; 4],
    /// Nonzero `UNIT_FIELD_FACTIONTEMPLATE` override
    /// (PWT flightmaster 774/775, cpp:916).
    faction_template: u32,
    /// Aura applied after summon (flightmaster spawn-in,
    /// Spirit of Victory particles).
    aura: u32,
    /// `SetSpawnedByDefault` (NPT curing shrine, cpp:473).
    spawned_by_default: bool,
}

fn triggered_self() -> CastOptions {
    CastOptions {
        triggered: true,
        triggered_by_aura: false,
        skip_known: true,
        cast_item: None,
        custom_base_points: [None; 3],
        triggered_by_aura_spell: 0,
        triggered_by_spell: 0,
    }
}

fn dist3(a: Position, b: Position) -> f32 {
    let dx = (f64::from(a.x) - f64::from(b.x)) as f32;
    let dy = (f64::from(a.y) - f64::from(b.y)) as f32;
    let dz = (f64::from(a.z) - f64::from(b.z)) as f32;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

impl Map {
    /// EP gate: Eastern Kingdoms map + `OutdoorPvP.EP.Enable`
    /// (`Register.cpp`, `World.cpp:787`, conf 2747). No
    /// client-patch gate: the tree targets 1.12+ builds
    /// (fork gates `SUPPORTED_CLIENT_BUILD > 1.11.2 &&
    /// wowPatch >= 112`; recorded deviation).
    #[must_use]
    pub(crate) fn ep_enabled(&self) -> bool {
        self.map_id == EP_MAP_ID && self.config.outdoor_pvp_ep_enable
    }

    /// `FillInitialWorldStates` packet for EP zones, else `None`
    /// (caller falls back to SI/default init).
    pub(crate) fn ep_init_world_states(&self, zone_id: u32) -> Option<WorldPacket> {
        if !self.ep_enabled() || !EpState::is_ep_zone(zone_id) {
            return None;
        }
        let mut writer = PacketWriter::new();
        writer.write_u32_le(self.map_id);
        writer.write_u32_le(zone_id);
        let extra = self.outdoor_ep.fill_initial();
        writer.write_u16_le((crate::map::DEFAULT_WORLD_STATES.len() + extra.len()) as u16 + 1);
        for &(state, value) in crate::map::DEFAULT_WORLD_STATES {
            writer.write_u32_le(state);
            writer.write_u32_le(value);
        }
        for &(state, value) in &extra {
            writer.write_u32_le(state);
            writer.write_u32_le(value);
        }
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let mut packet = WorldPacket::new(opcode_id::SMSG_INIT_WORLD_STATES);
        packet.take_writer(writer);
        Some(packet)
    }

    /// Zone-change hook (`OnPlayerEnter/Leave`, cpp:967-997):
    /// leaving EP zones sends the 33 zeroes and strips Echoes;
    /// entering under a held count grants the rank buff.
    pub(crate) fn ep_on_zone_change(&mut self, guid: ObjectGuid, old_zone: u32, new_zone: u32) {
        if !self.ep_enabled() {
            return;
        }
        let was_in = EpState::is_ep_zone(old_zone);
        let is_in = EpState::is_ep_zone(new_zone);
        if was_in && !is_in {
            for (field, _) in self.outdoor_ep.leave_zeroes() {
                let packet = build_update_world_state_packet(field, 0);
                self.events.push(MapEvent::Packet { to: guid, packet });
            }
            let team = self.player_team_of(guid).unwrap_or(Team::None);
            for spell in ep_echoes(team) {
                self.remove_auras_due_to_spell(guid, spell);
            }
        }
        if is_in {
            let team = self.player_team_of(guid).unwrap_or(Team::None);
            let rank = self.outdoor_ep.buff_rank(team);
            if rank > 0 {
                let spell = ep_echoes(team)[(rank - 1) as usize];
                let _ = self.cast_spell(guid, spell, CastTarget::Unit(guid), triggered_self());
            }
        }
    }

    /// `SendUpdateWorldState` fan-out to EP-zone players
    /// (`ZoneScript::SendUpdateWorldState`, ZoneScript.cpp:568).
    fn ep_broadcast_ws(&mut self, field: u32, value: u32) {
        let packet = build_update_world_state_packet(field, value);
        let targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, entry)| EpState::is_ep_zone(entry.zone_id))
            .map(|(guid, _)| *guid)
            .collect();
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `UpdateTowerState` 7-pair broadcast for one tower
    /// (EWT cpp:166-175 et al).
    fn ep_broadcast_tower(&mut self, tower: usize) {
        let bits = self.outdoor_ep.towers[tower].tower_state;
        let row = EP_TOWER_WS[tower];
        let pairs: Vec<(u32, u32)> = row
            .iter()
            .enumerate()
            .map(|(slot, ws)| (*ws, u32::from(bits & EP_TOWER_WS_BITS[slot] != 0)))
            .collect();
        for (field, value) in pairs {
            self.ep_broadcast_ws(field, value);
        }
    }

    /// `SendChangePhase`: slider position to the tower's active
    /// players only (`OPvPCapturePoint::SendUpdateWorldState` over
    /// `m_activePlayers`, ZoneScript.cpp:432-444).
    fn ep_send_phase(&mut self, tower: usize) {
        let value = self.outdoor_ep.towers[tower].value_pct;
        let packet = build_update_world_state_packet(EP_WS_SLIDER_POSITION, value);
        let targets: Vec<ObjectGuid> = self.outdoor_ep.towers[tower]
            .active_a
            .iter()
            .chain(self.outdoor_ep.towers[tower].active_h.iter())
            .copied()
            .collect();
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `Player::IsOutdoorPvPActive` (Player.cpp:6821): alive, not
    /// GM, no stealth/invisibility aura, `PvP`-desired or `PvP` realm,
    /// not on a taxi.
    pub(crate) fn ep_player_active(&self, guid: ObjectGuid) -> bool {
        if !self.unit_is_alive(guid) {
            return false;
        }
        let Some(entry) = self.players.get(&guid) else {
            return false;
        };
        if entry.player.extra_flags & EP_GM_FLAG != 0 {
            return false;
        }
        let shrouded = self
            .aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .any(|e| {
                e.aura_type == aura_type::MOD_STEALTH || e.aura_type == aura_type::MOD_INVISIBILITY
            });
        if shrouded {
            return false;
        }
        let desired = entry
            .player
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::PLAYER_FLAGS, EP_PVP_DESIRED_FLAG);
        if !(desired || self.config.pvp_realm) {
            return false;
        }
        self.unit_value_u32(guid, offsets::UNIT_FIELD_FLAGS) & UNIT_FLAG_TAXI_FLIGHT == 0
    }

    /// Capture pacing for one banner: DBC capture-point words
    /// (`GameObjectDefines.h:489-511`: radius 0, neutralPercent 12,
    /// minTime 16, maxTime 17), else the recorded fallback.
    fn ep_capture_params(&self, tower: usize) -> CaptureParams {
        let entry = EP_CAPTURE_ENTRIES[tower];
        if let Some(info) = self.stores.gameobject_templates.get(entry)
            && info.go_type == EP_CAPTURE_GO_TYPE
        {
            let word = |i: usize| info.data_u32(i);
            let max_time = word(17);
            let min_time = word(16);
            let neutral = word(12);
            let radius_w = word(0);
            let max_value = if max_time == 0 {
                EP_FALLBACK_MAX_VALUE
            } else {
                #[allow(clippy::cast_precision_loss)]
                let v = max_time as f32;
                v
            };
            #[allow(clippy::cast_precision_loss)]
            let divisor = if min_time == 0 { 60.0 } else { min_time as f32 };
            #[allow(clippy::cast_precision_loss)]
            let radius = if radius_w == 0 {
                EP_FALLBACK_RADIUS
            } else {
                radius_w as f32
            };
            #[allow(clippy::cast_precision_loss)]
            let min_value = max_value * neutral as f32 / 100.0;
            return CaptureParams {
                radius,
                max_value,
                min_value,
                max_speed: max_value / divisor,
                neutral_pct: neutral,
            };
        }
        #[allow(clippy::cast_precision_loss)]
        let min_value = EP_FALLBACK_MAX_VALUE * EP_FALLBACK_NEUTRAL_PCT as f32 / 100.0;
        CaptureParams {
            radius: EP_FALLBACK_RADIUS,
            max_value: EP_FALLBACK_MAX_VALUE,
            min_value,
            max_speed: EP_FALLBACK_MAX_VALUE / EP_FALLBACK_MIN_TIME,
            neutral_pct: EP_FALLBACK_NEUTRAL_PCT,
        }
    }

    /// Map tick arm (`OutdoorPvPEP::Update`, cpp:942-965): consume
    /// the previous tick's `objective_changed` (recount, counts,
    /// `BuffTeams`, 4-0 sweep), then run each tower on the 1000ms
    /// cadence (`OutdoorPvP::Update`, ZoneScript.cpp:278-286).
    pub(crate) fn ep_update(&mut self, diff_ms: u32) {
        if !self.ep_enabled() {
            return;
        }
        if !self.outdoor_ep.initialized {
            self.ep_ensure_initialized();
        }
        if self.outdoor_ep.objective_changed {
            self.outdoor_ep.objective_changed = false;
            self.ep_recount_and_buff();
        }
        self.outdoor_ep.accum_ms += diff_ms;
        if self.outdoor_ep.accum_ms < EP_UPDATE_INTERVAL_MS {
            return;
        }
        let step = self.outdoor_ep.accum_ms;
        self.outdoor_ep.accum_ms = 0;
        for tower in 0..4 {
            self.ep_update_tower(tower, step);
        }
    }

    /// One tower's 1000ms pass: prune/scan radius players
    /// (`OPvPCapturePoint::Update` 293-324), then the slider step;
    /// state changes run `ChangeState` and flag
    /// `objective_changed`, else a moved slider sends the phase.
    fn ep_update_tower(&mut self, tower: usize, step_ms: u32) {
        let params = self.ep_capture_params(tower);
        let anchor = EP_CAPTURE_POS[tower];
        // Prune: out of radius or inactive observers leave, with
        // the display reset (`ZoneScript.cpp:295-310` sends
        // worldState1 0 — the shared slider display here).
        let mut pruned = Vec::new();
        for guid in self.outdoor_ep.towers[tower]
            .active_a
            .iter()
            .chain(self.outdoor_ep.towers[tower].active_h.iter())
        {
            let pos = self
                .players
                .get(guid)
                .map(|entry| entry.player.unit.world.position());
            let keep = pos.is_some_and(|p| dist3(p, anchor) <= params.radius)
                && self.ep_player_active(*guid);
            if !keep {
                pruned.push(*guid);
            }
        }
        for guid in pruned {
            self.outdoor_ep.towers[tower].active_a.remove(&guid);
            self.outdoor_ep.towers[tower].active_h.remove(&guid);
            let packet = build_update_world_state_packet(EP_WS_SLIDER_DISPLAY, 0);
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
        // Scan: EP-zone, active, in-radius players join, with the
        // per-player state triple and the slider LAST
        // (`HandlePlayerEnter`, ZoneScript.cpp:36-62).
        let mut joined = Vec::new();
        let zone_players: Vec<(ObjectGuid, Team, Position)> = self
            .players
            .iter()
            .filter(|(_, entry)| EpState::is_ep_zone(entry.zone_id))
            .map(|(guid, entry)| (*guid, entry.player.team, entry.player.unit.world.position()))
            .collect();
        for (guid, team, pos) in zone_players {
            if dist3(pos, anchor) > params.radius {
                continue;
            }
            if !self.ep_player_active(guid) {
                continue;
            }
            let tower_ref = &mut self.outdoor_ep.towers[tower];
            let inserted = match team {
                Team::Alliance => tower_ref.active_a.insert(guid),
                Team::Horde => tower_ref.active_h.insert(guid),
                Team::None => false,
            };
            if inserted {
                joined.push(guid);
            }
        }
        for guid in joined {
            let pct = self.outdoor_ep.towers[tower].value_pct;
            for (field, value) in [
                (EP_WS_SLIDER_DISPLAY, 1),
                (EP_WS_SLIDER_N, params.neutral_pct),
                (EP_WS_SLIDER_POSITION, pct),
            ] {
                let packet = build_update_world_state_packet(field, value);
                self.events.push(MapEvent::Packet { to: guid, packet });
            }
        }
        let allies = self.outdoor_ep.towers[tower].active_a.len();
        let hordes = self.outdoor_ep.towers[tower].active_h.len();
        let outcome = self.outdoor_ep.towers[tower].slider_step(step_ms, allies, hordes, params);
        if outcome.state_changed {
            self.ep_change_state(tower);
            self.outdoor_ep.objective_changed = true;
        } else if outcome.send_phase {
            self.ep_send_phase(tower);
        }
    }

    /// Recount, counts broadcast, `BuffTeams`, 4-0 sweep
    /// (cpp:942-963; the fork re-sends inside the control loop —
    /// one send each here, same client state).
    fn ep_recount_and_buff(&mut self) {
        let mut allies = 0;
        let mut hordes = 0;
        for control in self.outdoor_ep.controls {
            if control == Team::Alliance {
                allies += 1;
            } else if control == Team::Horde {
                hordes += 1;
            }
        }
        self.outdoor_ep.allies = allies;
        self.outdoor_ep.hordes = hordes;
        self.ep_broadcast_ws(EP_WS_COUNT_A, allies);
        self.ep_broadcast_ws(EP_WS_COUNT_H, hordes);
        self.ep_buff_teams();
        if allies == 4 {
            self.events.push(MapEvent::OutdoorZoneText {
                zone_id: EP_ZONE,
                entry: EP_BCT_SWEEP[0],
                defense: true,
            });
        } else if hordes == 4 {
            self.events.push(MapEvent::OutdoorZoneText {
                zone_id: EP_ZONE,
                entry: EP_BCT_SWEEP[1],
                defense: true,
            });
        }
    }

    /// `BuffTeams` (cpp:999-1023): strip all four tiers, cast the
    /// held-count rank.
    fn ep_buff_teams(&mut self) {
        let members: Vec<(ObjectGuid, Team)> = self
            .players
            .iter()
            .filter(|(_, entry)| EpState::is_ep_zone(entry.zone_id))
            .map(|(guid, entry)| (*guid, entry.player.team))
            .collect();
        for (guid, team) in members {
            for spell in ep_echoes(team) {
                self.remove_auras_due_to_spell(guid, spell);
            }
            let rank = self.outdoor_ep.buff_rank(team);
            if rank > 0 {
                let spell = ep_echoes(team)[(rank - 1) as usize];
                let _ = self.cast_spell(guid, spell, CastTarget::Unit(guid), triggered_self());
            }
        }
    }

    /// `ChangeState` per tower (EWT cpp:39-150, NPT cpp:278-390,
    /// CGT cpp:486-604, PWT cpp:721-833): clearing control on
    /// terminal exit, grey-entry vs same-team-progressing arms,
    /// flare/buffer/shrine/flightmaster/squad/graveyard/banner
    /// lifecycle, 30882 tail cast (all but PWT, which casts inline
    /// in its progressing arms), then the 7-pair tower broadcast.
    /// Fork `DelCreature`/`DelObject` remove live spawns; here live
    /// guids despawn and matching intents drop.
    #[allow(clippy::too_many_lines)] // one ChangeState port per tower
    fn ep_change_state(&mut self, tower: usize) {
        let old = self.outdoor_ep.towers[tower].old_state;
        let new = self.outdoor_ep.towers[tower].state;
        if (old == EP_STATE_A || old == EP_STATE_H) && old != new {
            self.outdoor_ep.controls[tower] = Team::None;
        }
        let alliance_side = new == EP_STATE_A || new == EP_STATE_A_PROGRESSING;
        // Prologue deletions (per-tower heads of each ChangeState).
        match tower {
            EP_TOWER_EWT => {
                self.ep_del_creature(tower, EP_SLOT_EWT_BUFFER);
                self.ep_del_object(tower, EP_SLOT_EWT_FLARE);
            }
            EP_TOWER_NPT => {
                self.ep_del_creature(tower, EP_SLOT_NPT_BUFFER);
                self.ep_del_object(tower, EP_SLOT_NPT_SHRINE);
                self.ep_del_object(tower, EP_SLOT_NPT_AURA);
                self.ep_del_object(tower, EP_SLOT_NPT_FLARE);
            }
            EP_TOWER_PWT => {
                self.ep_del_creature(tower, EP_SLOT_PWT_BUFFER);
                self.ep_del_creature(tower, EP_SLOT_PWT_FLIGHTMASTER);
                self.ep_del_object(tower, EP_SLOT_PWT_FLARE);
            }
            _ => {
                self.ep_del_creature(tower, EP_SLOT_CGT_BUFFER);
                self.ep_del_object(tower, EP_SLOT_CGT_AURA);
                self.ep_del_object(tower, EP_SLOT_CGT_FLARE);
                self.ep_unlink_graveyard();
            }
        }
        let side = usize::from(!alliance_side);
        // PWT casts 30882 inline in its grey-entry progressing arms
        // (cpp:772-775, 798-801), i.e. after the buffer exists:
        // defer past the sync below like the other towers' tail.
        let mut pwt_inline_cast = false;
        match new {
            EP_STATE_A | EP_STATE_H => {
                let bit = if alliance_side { EP_TS_A } else { EP_TS_H };
                self.outdoor_ep.towers[tower].tower_state = bit;
                self.ep_play_sound(tower, EP_SOUND_VICTORY[side]);
                let flare_slot = ep_flare_slot(tower);
                let row = EP_FLARE_SLOTS[tower][side];
                self.ep_push_object(
                    tower,
                    flare_slot,
                    EP_FLARE_ENTRIES[side],
                    EP_FLARE_POS[row],
                    EP_FLARE_ROT[row],
                    false,
                );
                match tower {
                    EP_TOWER_NPT => self.ep_summon_shrine(alliance_side),
                    EP_TOWER_PWT => self.ep_summon_flightmaster(alliance_side),
                    EP_TOWER_CGT => {
                        self.ep_link_graveyard(alliance_side);
                        self.ep_summon_banner_aura(alliance_side);
                    }
                    _ => {}
                }
            }
            EP_STATE_A_PROGRESSING | EP_STATE_H_PROGRESSING => {
                let (art, bit) = if alliance_side {
                    (EP_BANNER_ART[0], EP_TS_A_PROGRESSING)
                } else {
                    (EP_BANNER_ART[1], EP_TS_H_PROGRESSING)
                };
                self.ep_set_banner_art(tower, art);
                match tower {
                    EP_TOWER_NPT => self.ep_summon_shrine(alliance_side),
                    EP_TOWER_PWT => self.ep_summon_flightmaster(alliance_side),
                    EP_TOWER_CGT => {
                        self.ep_summon_banner_aura(alliance_side);
                        self.ep_link_graveyard(alliance_side);
                    }
                    _ => {}
                }
                let terminal = if alliance_side {
                    EP_STATE_A
                } else {
                    EP_STATE_H
                };
                if old == terminal {
                    self.outdoor_ep.towers[tower].tower_state = bit;
                    self.ep_play_sound(tower, EP_SOUND_WARNING[side]);
                } else if old <= EP_STATE_H_CONTESTED {
                    self.outdoor_ep.towers[tower].tower_state = bit;
                    self.ep_play_sound(tower, EP_SOUND_CAPTURED[side]);
                    if tower == EP_TOWER_EWT {
                        self.ep_summon_squad(alliance_side);
                    }
                    if tower == EP_TOWER_CGT {
                        self.ep_summon_spirit(alliance_side);
                    }
                    self.ep_add_buffer(tower, alliance_side);
                    let winner = if alliance_side {
                        Team::Alliance
                    } else {
                        Team::Horde
                    };
                    self.outdoor_ep.controls[tower] = winner;
                    if old != new {
                        self.events.push(MapEvent::OutdoorZoneText {
                            zone_id: EP_ZONE,
                            entry: EP_BCT_TOWER[tower][side],
                            defense: true,
                        });
                    }
                    if tower == EP_TOWER_PWT {
                        pwt_inline_cast = true;
                    }
                }
            }
            _ => {
                let bit = match new {
                    EP_STATE_A_CONTESTED => EP_TS_A_CONTESTED,
                    EP_STATE_H_CONTESTED => EP_TS_H_CONTESTED,
                    _ => EP_TS_NEUTRAL,
                };
                self.outdoor_ep.towers[tower].tower_state = bit;
                self.ep_set_banner_art(tower, EP_BANNER_ART[2]);
                if tower == EP_TOWER_EWT {
                    self.ep_remove_squad();
                }
            }
        }
        self.ep_sync_spawns();
        if tower != EP_TOWER_PWT || pwt_inline_cast {
            self.ep_cast_buffer_test(tower);
        }
        self.ep_broadcast_tower(tower);
    }

    /// Grey-entry buffer add (`AddCreature` buffer row, team side).
    fn ep_add_buffer(&mut self, tower: usize, alliance_side: bool) {
        let side = usize::from(!alliance_side);
        let row = EP_BUFFER_ROWS[EP_BUFFER_ROWS_PER_TOWER[tower][side]];
        self.ep_push_creature(
            tower,
            EP_BUFFER_SLOT[tower],
            row.0,
            row.2,
            [0.0, 0.0, 0.0, 1.0],
            0,
            0,
            false,
        );
    }

    /// Buffer tail cast: `CastSpell(30882, self)`
    /// (EWT cpp:144-147, NPT cpp:384-387, CGT cpp:598-601; PWT
    /// casts inline in its progressing arms instead).
    fn ep_cast_buffer_test(&mut self, tower: usize) {
        let slot = EP_BUFFER_SLOT[tower];
        let buffer = self.outdoor_ep.towers[tower].creatures.get(&slot).copied();
        if let Some(buffer) = buffer
            && self.creatures.contains_key(&buffer)
        {
            let _ = self.cast_spell(
                buffer,
                EP_SPELL_CAPTURE_TEST,
                CastTarget::Unit(buffer),
                CastOptions {
                    triggered: false,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
        }
    }

    /// EWT squad summon (`SummonSquadAtEastWallTower`, cpp:235-268):
    /// commander + 4 soldiers/fighters (`asActiveObject`; the fork
    /// `JoinCreatureGroup` formation has no Rust group seam —
    /// spawn-only, recorded deviation).
    fn ep_summon_squad(&mut self, alliance_side: bool) {
        let (commander, soldier) = if alliance_side {
            (EP_COMMANDER_A, EP_SOLDIER_A)
        } else {
            (EP_COMMANDER_H, EP_SOLDIER_H)
        };
        for (i, pos) in EP_SQUAD_POS.iter().enumerate() {
            let entry = if i == 0 { commander } else { soldier };
            self.ep_push_creature(
                EP_TOWER_EWT,
                EP_SLOT_SQUAD[i],
                entry,
                *pos,
                [0.0, 0.0, 0.0, 1.0],
                0,
                0,
                false,
            );
        }
    }

    /// EWT `RemoveSquad` (cpp:227-233).
    fn ep_remove_squad(&mut self) {
        for slot in EP_SLOT_SQUAD {
            self.ep_del_creature(EP_TOWER_EWT, slot);
        }
    }

    /// NPT `SummonCuringShrine` (cpp:467-475): shrine + aura,
    /// shrine `SetSpawnedByDefault(true)`.
    fn ep_summon_shrine(&mut self, alliance_side: bool) {
        let base = usize::from(!alliance_side) * 2;
        self.ep_push_object(
            EP_TOWER_NPT,
            EP_SLOT_NPT_SHRINE,
            EP_SHRINE_ROWS[base].0,
            EP_SHRINE_ROWS[base].1,
            EP_SHRINE_ROT,
            true,
        );
        self.ep_push_object(
            EP_TOWER_NPT,
            EP_SLOT_NPT_AURA,
            EP_SHRINE_ROWS[base + 1].0,
            EP_SHRINE_ROWS[base + 1].1,
            EP_SHRINE_ROT,
            false,
        );
    }

    /// PWT `SummonFlightMaster` (cpp:910-920): faction template +
    /// spawn-in aura (`RemoveAllAuras` first on live resummon).
    fn ep_summon_flightmaster(&mut self, alliance_side: bool) {
        let side = usize::from(!alliance_side);
        let aura = if alliance_side {
            EP_SPELL_SPIRIT_SPAWN_IN
        } else {
            EP_SPELL_SPIRIT_PARTICLES_RED
        };
        self.ep_push_creature(
            EP_TOWER_PWT,
            EP_SLOT_PWT_FLIGHTMASTER,
            EP_FLIGHTMASTER_ENTRY,
            EP_FLIGHTMASTER_POS,
            [0.0, 0.0, 0.0, 1.0],
            EP_FLIGHTMASTER_FACTION[side],
            aura,
            false,
        );
    }

    /// CGT `LinkGraveYard` (cpp:681-687): runtime overlay for
    /// graveyard 927 on zones 139 + 2258 (the DB link store is
    /// read-only shared state).
    fn ep_link_graveyard(&mut self, alliance_side: bool) {
        self.outdoor_ep.graveyard_team = Some(if alliance_side {
            Team::Alliance
        } else {
            Team::Horde
        });
    }

    /// CGT `UnLinkGraveYard` (cpp:689-695).
    fn ep_unlink_graveyard(&mut self) {
        self.outdoor_ep.graveyard_team = None;
    }

    /// CGT `SummonBannerAura` (cpp:697): row 1 is Alliance,
    /// row 0 is Horde (`EP_CGT_BannerAuraGraveYard`).
    fn ep_summon_banner_aura(&mut self, alliance_side: bool) {
        let row = usize::from(alliance_side);
        self.ep_push_object(
            EP_TOWER_CGT,
            EP_SLOT_CGT_AURA,
            EP_BANNER_AURA_ROWS[row].0,
            EP_BANNER_AURA_ROWS[row].1,
            EP_BANNER_AURA_ROT,
            false,
        );
    }

    /// CGT `SummonSpiritOfVictory` (cpp:699-711): spawn-only +
    /// victory aura (the waypoint walk has no engine hook —
    /// spawn + aura fallback, recorded deviation).
    fn ep_summon_spirit(&mut self, alliance_side: bool) {
        let aura = if alliance_side {
            EP_SPELL_VICTORY_A
        } else {
            EP_SPELL_VICTORY_H
        };
        self.ep_push_creature(
            EP_TOWER_CGT,
            EP_SLOT_CGT_SPIRIT,
            EP_SPIRIT_ENTRY,
            EP_SPIRIT_POS,
            [0.0, 0.0, 0.0, 1.0],
            0,
            aura,
            false,
        );
    }

    /// `UpdateBannerArt` (EWT cpp:177-203 et al): record the
    /// intent, write `GAMEOBJECT_ARTKIT` on the live capture +
    /// tower banners, broadcast the custom anim (fork
    /// `SendGameObjectCustomAnim(anim)`; the packet shape matches
    /// the existing Rust custom-anim sender).
    fn ep_set_banner_art(&mut self, tower: usize, art: (u32, u32)) {
        self.outdoor_ep.towers[tower].artkit = art;
        let mut live = Vec::new();
        if let Some(guid) = self.outdoor_ep.towers[tower].objects.get(&EP_SLOT_CAPTURE) {
            live.push(*guid);
        }
        for slot in ep_banner_slots(tower) {
            if let Some(guid) = self.outdoor_ep.towers[tower].objects.get(&slot) {
                live.push(*guid);
            }
        }
        for guid in live {
            let current = self
                .gameobjects
                .get(&guid)
                .map(|go| go.world.object.values().get_u32(offsets::GAMEOBJECT_ARTKIT));
            if current != Some(art.0)
                && let Some(go) = self.gameobjects.get_mut(&guid)
            {
                go.world
                    .object
                    .values_mut()
                    .set_u32(offsets::GAMEOBJECT_ARTKIT, art.0);
            }
            if current != Some(art.0) {
                let mut payload = PacketWriter::new();
                payload.write_u64_le(guid.raw());
                payload.write_u32_le(art.1);
                let packet = WorldPacket::with_body(
                    opcode_id::SMSG_GAMEOBJECT_CUSTOM_ANIM,
                    payload.into_vec(),
                );
                self.broadcast_movement(guid, &packet);
            }
        }
    }

    /// `PlaySound` on the capture banner (cpp:205-214): the
    /// existing viewer `SMSG_PLAY_SOUND` path (fork
    /// `PlayDirectSound` on the banner GO).
    fn ep_play_sound(&mut self, tower: usize, sound: u32) {
        if sound == 0 {
            return;
        }
        self.outdoor_ep.towers[tower].last_sound = sound;
        if let Some(guid) = self.outdoor_ep.towers[tower]
            .objects
            .get(&EP_SLOT_CAPTURE)
            .copied()
            && self.gameobjects.contains_key(&guid)
        {
            self.play_direct_sound(guid, sound, None);
        }
    }

    /// Queue a creature spawn intent, replacing any intent for the
    /// same tower slot (`DelCreature` + `AddCreature` in one
    /// `ChangeState`).
    #[allow(clippy::too_many_arguments)] // one spawn-intent row
    fn ep_push_creature(
        &mut self,
        tower: usize,
        slot: u32,
        entry: u32,
        pos: Position,
        rot: [f32; 4],
        faction_template: u32,
        aura: u32,
        spawned_by_default: bool,
    ) {
        self.outdoor_ep
            .pending
            .retain(|i| !(i.tower == tower && i.creature && i.slot == slot));
        self.outdoor_ep.pending.push(SpawnIntent {
            tower,
            creature: true,
            slot,
            entry,
            pos,
            rot,
            faction_template,
            aura,
            spawned_by_default,
        });
    }

    /// Queue a gameobject spawn intent (same replace rule).
    fn ep_push_object(
        &mut self,
        tower: usize,
        slot: u32,
        entry: u32,
        pos: Position,
        rot: [f32; 4],
        spawned_by_default: bool,
    ) {
        self.outdoor_ep
            .pending
            .retain(|i| !(i.tower == tower && !i.creature && i.slot == slot));
        self.outdoor_ep.pending.push(SpawnIntent {
            tower,
            creature: false,
            slot,
            entry,
            pos,
            rot,
            faction_template: 0,
            aura: 0,
            spawned_by_default,
        });
    }

    /// `DelCreature` equivalent: despawn the live guid and drop
    /// the matching intent (cpp:183-206).
    fn ep_del_creature(&mut self, tower: usize, slot: u32) {
        if let Some(guid) = self.outdoor_ep.towers[tower].creatures.remove(&slot) {
            self.despawn_creature_now(guid, u32::MAX);
        }
        self.outdoor_ep
            .pending
            .retain(|i| !(i.tower == tower && i.creature && i.slot == slot));
    }

    /// `DelObject` equivalent (cpp:208-224).
    fn ep_del_object(&mut self, tower: usize, slot: u32) {
        if let Some(guid) = self.outdoor_ep.towers[tower].objects.remove(&slot) {
            self.despawn_gameobject_now(guid);
        }
        self.outdoor_ep
            .pending
            .retain(|i| !(i.tower == tower && !i.creature && i.slot == slot));
    }

    /// Materialize pending tower spawns (`AddCreature`/`AddObject`
    /// via `SummonCreature`/`SummonGameObject`, ZoneScript.cpp:79-
    /// 135). Missing templates or no summoner player keep the
    /// intent queued, like the SI dust-bag unknown-guid skip.
    fn ep_sync_spawns(&mut self) {
        if self.outdoor_ep.pending.is_empty() {
            return;
        }
        let summoner = self
            .players
            .iter()
            .filter(|(_, entry)| EpState::is_ep_zone(entry.zone_id))
            .map(|(guid, _)| *guid)
            .next()
            .or_else(|| self.players.keys().next().copied());
        let Some(summoner) = summoner else {
            return;
        };
        let intents = std::mem::take(&mut self.outdoor_ep.pending);
        let mut failed = Vec::new();
        for intent in intents {
            if intent.creature {
                // `TEMPSUMMON_MANUAL_DESPAWN` (ObjectDefines.h:68),
                // no despawn timer (`spawntimedelay` 0).
                let live = self.summon_creature(intent.entry, intent.pos, summoner, 8, 0);
                match live {
                    Some(guid) => {
                        if intent.faction_template != 0 {
                            let _ = self.object_set_field(
                                guid,
                                u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
                                intent.faction_template,
                            );
                        }
                        if intent.aura != 0 {
                            self.apply_aura_by_id(guid, intent.aura, guid);
                        }
                        self.outdoor_ep.towers[intent.tower]
                            .creatures
                            .insert(intent.slot, guid);
                    }
                    None => failed.push(intent),
                }
            } else {
                let live = self.summon_gameobject(
                    summoner,
                    intent.entry,
                    intent.pos,
                    intent.rot,
                    0,
                    false,
                );
                match live {
                    Some(guid) => {
                        if intent.spawned_by_default
                            && let Some(go) = self.gameobjects.get_mut(&guid)
                        {
                            go.spawned_by_default = true;
                        }
                        self.outdoor_ep.towers[intent.tower]
                            .objects
                            .insert(intent.slot, guid);
                    }
                    None => failed.push(intent),
                }
            }
        }
        self.outdoor_ep.pending = failed;
    }

    /// First-tick tower setup (`SetupZoneScript` plus ctor
    /// `ChangeState`, cpp:31-37, 930-940): one capture banner and
    /// two tower flags per tower, then the neutral display pass.
    /// Intents that cannot materialize stay queued for the next
    /// tick (no players yet or templates absent).
    fn ep_ensure_initialized(&mut self) {
        self.outdoor_ep.initialized = true;
        for (tower, entry) in EP_CAPTURE_ENTRIES.iter().enumerate() {
            self.ep_push_object(
                tower,
                EP_SLOT_CAPTURE,
                *entry,
                EP_CAPTURE_POS[tower],
                EP_CAPTURE_ROT[tower],
                false,
            );
            for (i, slot) in ep_banner_slots(tower).iter().enumerate() {
                let flag = EP_TOWER_FLAG_SLOTS[tower][i];
                self.ep_push_object(
                    tower,
                    *slot,
                    EP_TOWER_BANNER_ENTRY,
                    EP_TOWER_FLAG_POS[flag],
                    EP_TOWER_FLAG_ROT[flag],
                    false,
                );
            }
        }
        self.ep_sync_spawns();
        for tower in 0..4 {
            self.ep_set_banner_art(tower, EP_BANNER_ART[2]);
            self.ep_broadcast_tower(tower);
        }
    }
}

/// Tower-flag object slots per tower in tower order.
fn ep_banner_slots(tower: usize) -> [u32; 2] {
    match tower {
        EP_TOWER_EWT => EP_SLOT_EWT_BANNERS,
        EP_TOWER_NPT => EP_SLOT_NPT_BANNERS,
        EP_TOWER_PWT => EP_SLOT_PWT_BANNERS,
        _ => EP_SLOT_CGT_BANNERS,
    }
}

/// Flare object slot per tower.
fn ep_flare_slot(tower: usize) -> u32 {
    match tower {
        EP_TOWER_EWT => EP_SLOT_EWT_FLARE,
        EP_TOWER_NPT => EP_SLOT_NPT_FLARE,
        EP_TOWER_PWT => EP_SLOT_PWT_FLARE,
        _ => EP_SLOT_CGT_FLARE,
    }
}

/// Echoes of Lordaeron tiers for one team (`EP_AllianceBuffs` /
/// `EP_HordeBuffs`, h:199-201).
fn ep_echoes(team: Team) -> [u32; 4] {
    match team {
        Team::Alliance => EP_BUFFS_A,
        _ => EP_BUFFS_H,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> CaptureParams {
        CaptureParams {
            radius: EP_FALLBACK_RADIUS,
            max_value: 60.0,
            min_value: 12.0,
            max_speed: 1.0,
            neutral_pct: 20,
        }
    }

    #[test]
    fn ep_zones_match_fork_buff_zones() {
        assert!(EpState::is_ep_zone(139));
        assert!(EpState::is_ep_zone(2017));
        assert!(EpState::is_ep_zone(2057));
        assert!(!EpState::is_ep_zone(1377));
        assert!(!EpState::is_ep_zone(0));
    }

    #[test]
    fn fill_initial_carries_33_fork_pairs() {
        let state = EpState::default();
        let pairs = state.fill_initial();
        assert_eq!(pairs.len(), 33);
        assert!(pairs.contains(&(EP_WS_COUNT_A, 0)));
        assert!(pairs.contains(&(EP_WS_COUNT_H, 0)));
        assert!(pairs.contains(&(EP_WS_SLIDER_DISPLAY, 0)));
        assert!(pairs.contains(&(EP_WS_SLIDER_POSITION, 50)));
        assert!(pairs.contains(&(EP_WS_SLIDER_N, 100)));
        for (tower, row) in EP_TOWER_WS.iter().enumerate() {
            let mut set = 0;
            for (slot, ws) in row.iter().enumerate() {
                let expect = u32::from(slot == 6);
                assert!(pairs.contains(&(*ws, expect)), "tower {tower} slot {slot}");
                set += expect;
            }
            assert_eq!(set, 1, "exactly neutral for tower {tower}");
        }
    }

    #[test]
    fn slider_solo_alliance_captures_to_max() {
        let mut tower = EpTower::default();
        // Grey band exit at +12 needs 12 ticks at +1/s.
        let mut changed = false;
        for _ in 0..12 {
            let out = tower.slider_step(1000, 1, 0, params());
            changed |= out.state_changed;
        }
        assert!(changed, "grey -> progressing must flag");
        assert_eq!(tower.state, EP_STATE_A_PROGRESSING);
        assert_eq!(tower.team, Team::Alliance);
        assert_eq!(
            tower.tower_state, EP_TS_NEUTRAL,
            "display set by ChangeState, not slider"
        );
        // Full capture at +60.
        for _ in 0..48 {
            tower.slider_step(1000, 1, 0, params());
        }
        assert_eq!(tower.state, EP_STATE_A);
        assert!((tower.value - 60.0).abs() < f32::EPSILON);
        assert_eq!(tower.value_pct, 100);
        // Holding at max stays quiet.
        let out = tower.slider_step(1000, 1, 0, params());
        assert!(!out.state_changed);
        assert!(!out.send_phase);
    }

    #[test]
    fn slider_equal_forces_stall_in_grey() {
        let mut tower = EpTower::default();
        for _ in 0..30 {
            let out = tower.slider_step(1000, 1, 1, params());
            assert!(!out.state_changed);
        }
        assert_eq!(tower.state, EP_STATE_NEUTRAL);
        assert!(tower.value.abs() < f32::EPSILON);
    }

    #[test]
    fn slider_challenger_names_grey_contested() {
        let mut tower = EpTower::default();
        tower.slider_step(1000, 1, 0, params());
        assert_eq!(tower.state, EP_STATE_A_CONTESTED);
        let mut tower = EpTower::default();
        tower.slider_step(1000, 0, 2, params());
        assert_eq!(tower.state, EP_STATE_H_CONTESTED);
    }

    #[test]
    fn buff_rank_follows_towers_held() {
        let mut state = EpState::default();
        assert_eq!(state.buff_rank(Team::Alliance), 0);
        state.controls[EP_TOWER_EWT] = Team::Alliance;
        state.controls[EP_TOWER_NPT] = Team::Alliance;
        state.controls[EP_TOWER_CGT] = Team::Horde;
        assert_eq!(state.buff_rank(Team::Alliance), 2);
        assert_eq!(state.buff_rank(Team::Horde), 1);
        assert_eq!(state.buff_rank(Team::None), 0);
    }

    #[test]
    fn leave_zeroes_clear_all_33() {
        let state = EpState::default();
        let zeroes = state.leave_zeroes();
        assert_eq!(zeroes.len(), 33);
        assert!(zeroes.iter().all(|&(_, v)| v == 0));
    }

    // ---------------- Map-level integration (synthetic stores) ----

    use std::sync::Arc;

    use mangos_db::character_load::{CharacterLoad, CharacterRow};
    use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
    use mangos_spells::defines::{effect, target as tgt};
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::battlegrounds::SafeLocStore;
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::gameobject_template::{GameObjectInfo, GameObjectTemplates};
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };

    use crate::WorldRng;
    use crate::map::MapConfig;
    use crate::stores::WorldStores;

    fn test_infos() -> PlayerInfos {
        PlayerInfos::from_maps_for_tests(
            60,
            HashMap::from([(
                (1, 1),
                PlayerCreateInfo {
                    map_id: 0,
                    area_id: 14,
                    position_x: 100.0,
                    position_y: 200.0,
                    position_z: 10.0,
                    orientation: 0.0,
                    display_id_m: 51,
                    display_id_f: 52,
                },
            )]),
            HashMap::from([(
                1,
                vec![
                    PlayerClassLevelInfo {
                        basehealth: 100,
                        basemana: 100
                    };
                    60
                ],
            )]),
            HashMap::from([((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
            vec![400; 61],
        )
    }

    fn test_load(guid: u32) -> CharacterLoad {
        CharacterLoad {
            row: CharacterRow {
                guid,
                account: 42,
                name: format!("Ep{guid}"),
                race: 1,
                class: 1,
                gender: 0,
                level: 60,
                xp: 0,
                money: 0,
                skin: 1,
                face: 2,
                hair_style: 3,
                hair_color: 4,
                facial_hair: 5,
                bank_bag_slots: 0,
                character_flags: 0,
                position_x: 100.0,
                position_y: 200.0,
                position_z: 10.0,
                map: 0,
                orientation: 0.0,
                known_taxi_mask: String::new(),
                played_time_total: 60,
                played_time_level: 60,
                rest_bonus: 0.0,
                logout_time: 0,
                reset_talents_multiplier: 0,
                reset_talents_time: 0,
                transport_guid: 0,
                transport_x: 0.0,
                transport_y: 0.0,
                transport_z: 0.0,
                transport_o: 0.0,
                extra_flags: 0,
                stable_slots: 0,
                death_expire_time: 0,
                current_taxi_path: String::new(),
                honor_rank_points: 0.0,
                honor_highest_rank: 0,
                honor_standing: 0,
                honor_last_week_hk: 0,
                honor_last_week_cp: 0.0,
                honor_stored_hk: 0,
                honor_stored_dk: 0,
                watched_faction: -1,
                drunk: 0,
                health: 100,
                power1: 100,
                power2: 0,
                power3: 0,
                power4: 0,
                power5: 0,
                explored_zones: String::new(),
                ammo_id: 0,
                action_bars: 0,
                world_phase_mask: 0,
                create_time: 0,
                instance: 0,
                zone: EP_ZONE,
            },
            group_id: None,
            bound_instances: vec![],
            auras: vec![],
            spells: vec![],
            quest_status: vec![],
            honor_cp: vec![],
            reputation: vec![],
            inventory: vec![],
            item_loot: vec![],
            actions: vec![],
            social: vec![],
            homebind: None,
            spell_cooldowns: vec![],
            guild: None,
            bg_data: None,
            account_data: vec![],
            skills: vec![],
            mails: vec![],
            mailed_items: vec![],
            forgotten_skills: vec![],
            pets: Vec::new(),
        }
    }

    /// Aura fixture (triggered casts land a holder).
    fn aura_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            effects: [
                SpellEffectData {
                    effect: effect::APPLY_AURA,
                    implicit_target_a: tgt::UNIT,
                    apply_aura_name: aura_type::DUMMY,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        }
    }

    /// Script-effect fixture (routes to `effect_script_effect`).
    fn script_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            effects: [
                SpellEffectData {
                    effect: effect::SCRIPT_EFFECT,
                    implicit_target_a: tgt::UNIT,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        }
    }

    /// Script-effect + aura-marker fixture: routes the custom arm
    /// and leaves an observable holder (test-only observability;
    /// the fork 31929 carries no aura).
    fn script_aura_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            effects: [
                SpellEffectData {
                    effect: effect::SCRIPT_EFFECT,
                    implicit_target_a: tgt::UNIT,
                    ..Default::default()
                },
                SpellEffectData {
                    effect: effect::APPLY_AURA,
                    implicit_target_a: tgt::UNIT,
                    apply_aura_name: aura_type::DUMMY,
                    ..Default::default()
                },
                SpellEffectData::default(),
            ],
            ..Default::default()
        }
    }

    fn plain_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            ..Default::default()
        }
    }

    fn go_template(id: u32, go_type: u32, data: [i32; 24]) -> GameObjectInfo {
        GameObjectInfo {
            id,
            go_type,
            data,
            ..Default::default()
        }
    }

    #[allow(clippy::cast_possible_wrap)] // world-state ids < i32::MAX
    fn capture_data() -> [i32; 24] {
        let mut data = [0i32; 24];
        data[0] = 30;
        data[2] = EP_WS_SLIDER_DISPLAY as i32;
        data[3] = EP_WS_SLIDER_POSITION as i32;
        data[12] = 20;
        data[13] = EP_WS_SLIDER_N as i32;
        data[16] = 60;
        data[17] = 60;
        data
    }

    /// Map 0 with EP spells, capture/banner/flare templates,
    /// buffer + squad + flightmaster + spirit templates and
    /// graveyard 927. `pvp_realm` stands in for the PvP-desired
    /// flag in the `IsOutdoorPvPActive` gate.
    fn ep_map() -> Map {
        let mut stores = WorldStores::empty();
        for id in EP_BUFFS_A
            .iter()
            .chain(EP_BUFFS_H.iter())
            .chain([EP_SPELL_VICTORY_A, EP_SPELL_VICTORY_H].iter())
            .chain([EP_SPELL_SPIRIT_SPAWN_IN, EP_SPELL_SPIRIT_PARTICLES_RED].iter())
        {
            stores.spells.spells.insert(*id, aura_spell(*id));
        }
        stores
            .spells
            .spells
            .insert(EP_SPELL_CAPTURE_TEST, script_spell(EP_SPELL_CAPTURE_TEST));
        stores
            .spells
            .spells
            .insert(EP_SPELL_CAPTURE, script_aura_spell(EP_SPELL_CAPTURE));
        stores
            .spells
            .spells
            .insert(EP_SPELL_KILL_CREDIT, plain_spell(EP_SPELL_KILL_CREDIT));
        let gos: Vec<GameObjectInfo> = EP_CAPTURE_ENTRIES
            .iter()
            .map(|id| go_template(*id, EP_CAPTURE_GO_TYPE, capture_data()))
            .chain(
                [
                    EP_TOWER_BANNER_ENTRY,
                    EP_FLARE_ENTRIES[0],
                    EP_FLARE_ENTRIES[1],
                    EP_SHRINE_ROWS[0].0,
                    EP_SHRINE_ROWS[1].0,
                    EP_BANNER_AURA_ROWS[0].0,
                    EP_BANNER_AURA_ROWS[1].0,
                ]
                .iter()
                .map(|id| go_template(*id, 5, [0; 24])),
            )
            .collect();
        stores.gameobject_templates = GameObjectTemplates::from_entries_for_tests(gos);
        let creatures: Vec<CreatureInfo> = [
            EP_BUFFER_ENTRY_A,
            EP_BUFFER_ENTRY_H,
            EP_COMMANDER_A,
            EP_SOLDIER_A,
            EP_FLIGHTMASTER_ENTRY,
            EP_SPIRIT_ENTRY,
        ]
        .iter()
        .map(|entry| CreatureInfo {
            entry: *entry,
            display_id: [51, 0, 0, 0],
            display_probability: [1, 0, 0, 0],
            display_total_probability: 1,
            ..CreatureInfo::default()
        })
        .collect();
        stores.creature_templates = CreatureTemplates::from_entries_for_tests(creatures);
        stores.display_addons =
            mangos_world_data::spawn_support::DisplayInfoAddons::from_creature_addons_for_tests(
                vec![mangos_world_data::spawn_support::CreatureDisplayInfoAddon {
                    display_id: 51,
                    speed_walk: 2.5,
                    speed_run: 7.0,
                    ..Default::default()
                }],
            );
        stores.safe_locs =
            SafeLocStore::from_rows(&[(EP_GRAVEYARD_ID, 0, 1.0, 2.0, 3.0)], &HashMap::new());
        let mut map = Map::new(
            EP_MAP_ID,
            0,
            MapConfig::default(),
            Arc::new(stores),
            WorldRng::from_seed(7),
        );
        map.config.pvp_realm = true;
        map
    }

    /// Materialized level-60 player in EP zone 139 at `pos`.
    fn add_ep_player(map: &mut Map, guid_low: u32, team: Team, pos: Position) -> ObjectGuid {
        let infos = test_infos();
        let inputs = MaterializeInputs {
            infos: &infos,
            race: None,
            power_type: 1,
            display_scale: 1.0,
            model: None,
            saved_map_known: true,
            saved_map_is_instance: false,
            max_money: MAX_MONEY_AMOUNT,
            now_unix: 1_800_000_000,
            rest_rates: RestRates::default(),
            skills: None,
            item_prototypes: None,
            quest_templates: None,
            taxi: None,
        };
        let mut player = Player::materialize(test_load(guid_low), &inputs).expect("player");
        player.team = team;
        let guid = player.unit.guid();
        player.unit.world.relocate(pos);
        map.players.insert(
            guid,
            crate::map::PlayerEntry {
                player,
                visible: HashSet::new(),
                last_notified: pos,
                scan_due: false,
                zone_id: EP_ZONE,
                area_id: 0,
            },
        );
        guid
    }

    fn has_aura(map: &Map, guid: ObjectGuid, spell: u32) -> bool {
        map.aura_holders(guid).iter().any(|h| h.spell_id == spell)
    }

    /// (field, value) of every `SMSG_UPDATE_WORLD_STATE` in
    /// drained events, in order.
    fn update_states(map: &mut Map) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for event in map.drain_events() {
            if let MapEvent::Packet { packet, .. } = event
                && packet.opcode()
                    == mangos_protocol::opcode_table::opcode_id::SMSG_UPDATE_WORLD_STATE
            {
                let mut reader = packet.reader();
                out.push((
                    reader.read_u32_le().expect("field"),
                    reader.read_u32_le().expect("value"),
                ));
            }
        }
        out
    }

    fn zone_texts(map: &mut Map) -> Vec<(u32, i32)> {
        map.drain_events()
            .into_iter()
            .filter_map(|event| match event {
                MapEvent::OutdoorZoneText { zone_id, entry, .. } => Some((zone_id, entry)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn slider_loop_captures_ewt_over_ticks() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_EWT];
        add_ep_player(&mut map, 11, Team::Alliance, anchor);
        for _ in 0..12 {
            map.ep_update(1000);
        }
        assert_eq!(
            map.outdoor_ep.towers[EP_TOWER_EWT].state,
            EP_STATE_A_PROGRESSING
        );
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_EWT].team, Team::Alliance);
        assert_eq!(map.outdoor_ep.controls[EP_TOWER_EWT], Team::Alliance);
        assert_eq!(
            map.outdoor_ep.towers[EP_TOWER_EWT].tower_state,
            EP_TS_A_PROGRESSING
        );
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_EWT].artkit, EP_BANNER_ART[0]);
        assert_eq!(
            map.outdoor_ep.towers[EP_TOWER_EWT].last_sound,
            EP_SOUND_CAPTURED[0]
        );
        // Buffer + squad materialized from fixture templates.
        assert!(
            map.outdoor_ep.towers[EP_TOWER_EWT]
                .creatures
                .contains_key(&EP_SLOT_EWT_BUFFER)
        );
        for slot in EP_SLOT_SQUAD {
            assert!(
                map.outdoor_ep.towers[EP_TOWER_EWT]
                    .creatures
                    .contains_key(&slot),
                "squad slot {slot}"
            );
        }
        // Grey-entry BCT went to zone 139.
        assert!(zone_texts(&mut map).contains(&(EP_ZONE, EP_BCT_TOWER[EP_TOWER_EWT][0])));
        for _ in 0..48 {
            map.ep_update(1000);
        }
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_EWT].state, EP_STATE_A);
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_EWT].tower_state, EP_TS_A);
        assert!(
            map.outdoor_ep.towers[EP_TOWER_EWT]
                .objects
                .contains_key(&EP_SLOT_EWT_FLARE)
        );
        assert_eq!(
            map.outdoor_ep.towers[EP_TOWER_EWT].last_sound,
            EP_SOUND_VICTORY[0]
        );
    }

    #[test]
    fn objective_changed_recards_counts_and_buffs() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_EWT];
        let player = add_ep_player(&mut map, 11, Team::Alliance, anchor);
        for _ in 0..12 {
            map.ep_update(1000);
        }
        assert!(map.outdoor_ep.objective_changed);
        map.ep_update(0);
        assert!(!map.outdoor_ep.objective_changed);
        assert_eq!(map.outdoor_ep.allies, 1);
        assert_eq!(map.outdoor_ep.hordes, 0);
        let states = update_states(&mut map);
        assert!(states.contains(&(EP_WS_COUNT_A, 1)));
        assert!(states.contains(&(EP_WS_COUNT_H, 0)));
        assert!(has_aura(&map, player, EP_BUFFS_A[0]));
    }

    #[test]
    fn contested_tower_clears_control_and_squad() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_EWT];
        add_ep_player(&mut map, 11, Team::Alliance, anchor);
        for _ in 0..60 {
            map.ep_update(1000);
        }
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_EWT].state, EP_STATE_A);
        assert_eq!(map.outdoor_ep.controls[EP_TOWER_EWT], Team::Alliance);
        // Horde pressure drags it back through contested.
        add_ep_player(&mut map, 12, Team::Horde, anchor);
        let horde = add_ep_player(&mut map, 13, Team::Horde, anchor);
        let _ = horde;
        for _ in 0..120 {
            map.ep_update(1000);
            if map.outdoor_ep.towers[EP_TOWER_EWT].state == EP_STATE_H_CONTESTED
                || map.outdoor_ep.towers[EP_TOWER_EWT].state == EP_STATE_NEUTRAL
            {
                break;
            }
        }
        let state = map.outdoor_ep.towers[EP_TOWER_EWT].state;
        assert!(
            state == EP_STATE_H_CONTESTED || state == EP_STATE_NEUTRAL,
            "back to grey, got {state}"
        );
    }

    #[test]
    fn enter_grants_rank_leave_strips_and_zeroes() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_EWT];
        map.outdoor_ep.controls[EP_TOWER_EWT] = Team::Alliance;
        map.outdoor_ep.controls[EP_TOWER_NPT] = Team::Alliance;
        let player = add_ep_player(&mut map, 11, Team::Alliance, anchor);
        map.ep_on_zone_change(player, 0, EP_ZONE);
        assert!(has_aura(&map, player, EP_BUFFS_A[1]));
        map.ep_on_zone_change(player, EP_ZONE, 0);
        for spell in EP_BUFFS_A {
            assert!(!has_aura(&map, player, spell));
        }
        let states = update_states(&mut map);
        assert_eq!(states.len(), 33);
        assert!(states.iter().all(|&(_, v)| v == 0));
    }

    #[test]
    fn capture_test_cast_hits_friendly_active_only() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_EWT];
        let ally = add_ep_player(&mut map, 11, Team::Alliance, anchor);
        let enemy = add_ep_player(&mut map, 12, Team::Horde, anchor);
        let far = add_ep_player(
            &mut map,
            13,
            Team::Alliance,
            Position::new(anchor.x + 500.0, anchor.y, anchor.z, 0.0),
        );
        // Buffer at the anchor (summoned directly for the chain).
        let buffer = map
            .summon_creature(EP_BUFFER_ENTRY_A, anchor, ally, 8, 0)
            .expect("buffer");
        let _ = map.cast_spell(
            buffer,
            EP_SPELL_CAPTURE_TEST,
            crate::spells::CastTarget::Unit(buffer),
            crate::spells::CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert!(has_aura(&map, ally, EP_SPELL_CAPTURE));
        assert!(!has_aura(&map, enemy, EP_SPELL_CAPTURE));
        assert!(!has_aura(&map, far, EP_SPELL_CAPTURE));
        // Non-buffer caster: no chain.
        let other = map
            .summon_creature(EP_COMMANDER_A, anchor, ally, 8, 0)
            .expect("other");
        let _ = map.cast_spell(
            other,
            EP_SPELL_CAPTURE_TEST,
            crate::spells::CastTarget::Unit(other),
            crate::spells::CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert!(!has_aura(&map, far, EP_SPELL_CAPTURE));
    }

    #[test]
    fn per_tower_rewards_match_fork_lifecycle() {
        let mut map = ep_map();
        // NPT grey entry: curing shrine + banner aura, Alliance side.
        let npt = add_ep_player(&mut map, 21, Team::Alliance, EP_CAPTURE_POS[EP_TOWER_NPT]);
        for _ in 0..12 {
            map.ep_update(1000);
        }
        assert_eq!(map.outdoor_ep.controls[EP_TOWER_NPT], Team::Alliance);
        assert!(
            map.outdoor_ep.towers[EP_TOWER_NPT]
                .objects
                .contains_key(&EP_SLOT_NPT_SHRINE)
        );
        assert!(
            map.outdoor_ep.towers[EP_TOWER_NPT]
                .objects
                .contains_key(&EP_SLOT_NPT_AURA)
        );
        let _ = npt;
        // PWT full capture: flightmaster with faction + spawn-in aura.
        let pwt = add_ep_player(&mut map, 22, Team::Alliance, EP_CAPTURE_POS[EP_TOWER_PWT]);
        for _ in 0..60 {
            map.ep_update(1000);
        }
        assert_eq!(map.outdoor_ep.towers[EP_TOWER_PWT].state, EP_STATE_A);
        let fm = map.outdoor_ep.towers[EP_TOWER_PWT]
            .creatures
            .get(&EP_SLOT_PWT_FLIGHTMASTER)
            .copied()
            .expect("flightmaster");
        assert_eq!(
            map.unit_value_u32(fm, offsets::UNIT_FIELD_FACTIONTEMPLATE),
            774
        );
        assert!(has_aura(&map, fm, EP_SPELL_SPIRIT_SPAWN_IN));
        let _ = pwt;
        // CGT grey entry, Horde: banner aura + spirit + graveyard link.
        let cgt = add_ep_player(&mut map, 23, Team::Horde, EP_CAPTURE_POS[EP_TOWER_CGT]);
        for _ in 0..24 {
            map.ep_update(1000);
            if map.outdoor_ep.controls[EP_TOWER_CGT] == Team::Horde {
                break;
            }
        }
        assert_eq!(map.outdoor_ep.controls[EP_TOWER_CGT], Team::Horde);
        assert!(
            map.outdoor_ep.towers[EP_TOWER_CGT]
                .objects
                .contains_key(&EP_SLOT_CGT_AURA)
        );
        let spirit = map.outdoor_ep.towers[EP_TOWER_CGT]
            .creatures
            .get(&EP_SLOT_CGT_SPIRIT)
            .copied()
            .expect("spirit");
        assert!(has_aura(&map, spirit, EP_SPELL_VICTORY_H));
        assert_eq!(map.outdoor_ep.graveyard_team, Some(Team::Horde));
        let _ = cgt;
    }

    #[test]
    fn crown_guard_links_graveyard_927() {
        let mut map = ep_map();
        let anchor = EP_CAPTURE_POS[EP_TOWER_CGT];
        let player = add_ep_player(&mut map, 11, Team::Alliance, anchor);
        // Grey entry on CGT links the graveyard for Alliance.
        for _ in 0..12 {
            map.ep_update(1000);
        }
        // CGT needs Alliance presence at its own anchor; the EWT
        // player does not count there.
        let _ = player;
        let ally = add_ep_player(&mut map, 12, Team::Alliance, anchor);
        for _ in 0..24 {
            map.ep_update(1000);
        }
        assert_eq!(map.outdoor_ep.controls[EP_TOWER_CGT], Team::Alliance);
        assert_eq!(map.outdoor_ep.graveyard_team, Some(Team::Alliance));
        let gy = map
            .graveyard_for(anchor, Team::Alliance, EP_ZONE, 0)
            .expect("linked gy");
        assert_eq!(gy.id, EP_GRAVEYARD_ID);
        // Horde ghosts do not use it.
        assert!(
            map.graveyard_for(anchor, Team::Horde, EP_ZONE, 0)
                .is_none_or(|g| g.id != EP_GRAVEYARD_ID)
        );
        let _ = ally;
    }
}
