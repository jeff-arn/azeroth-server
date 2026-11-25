//! `GameObject` AI (GO scripts): per-script state, `GoAiState`
//! dispatch, and the `Map` bodies for gameobject scripts.
//! Shared content-script dispatch and encounter constants stay
//! in the parent `content` module.

#[allow(clippy::wildcard_imports)]
use super::*;

// `go_firework_rocket` has no new consts: `EVENT_ROCKET_DESPAWN` (id 1) is
// the single `EventMap` event, reduced to the timer field below.

/// `go_firework_rocket` state: `EVENT_ROCKET_DESPAWN` countdown
/// (`None` = disarmed). The fork ctor arms it at `Seconds(0)`, so creation
/// arms `Some(0)` (no RNG involved, unlike the firecracker).
pub(crate) struct FireworkRocketGoAi {
    pub(crate) despawn_timer_ms: Option<u32>,
}

impl FireworkRocketGoAi {
    pub(crate) fn new() -> Self {
        Self {
            despawn_timer_ms: None,
        }
    }
}

const DREADSTEED_GO_WHEEL: u32 = 179_672;
// `GOBJ_CANDLE` (cpp:12), read by `go_ai_pedestal_generate_glyph_and_node_guids`.
const DREADSTEED_GO_CANDLE: u32 = 179_673;
// `GOBJ_BELL` (cpp:13), read by `go_ai_pedestal_generate_glyph_and_node_guids`.
const DREADSTEED_GO_BELL: u32 = 179_674;
// `GOBJ_FEL_FIRE` (cpp:14), also `GOBJ_DREADSTEED_PORTAL` (cpp:15, same
// entry 179_681): flame-ring summon + portal lookups.
const DREADSTEED_GO_FEL_FIRE: u32 = 179_681;
// `GOBJ_RITUAL_CIRCLE` (cpp:16), read by second-part/success/fail arms.
const DREADSTEED_GO_RITUAL_CIRCLE: u32 = 179_668;

// `GOBJ_RUNE_TYPE_1/2/3` (cpp:20-22), read by
// `go_ai_pedestal_generate_glyph_and_node_guids`.
const DREADSTEED_GO_RUNE_TYPE_1: u32 = 179_669;
const DREADSTEED_GO_RUNE_TYPE_2: u32 = 179_670;
const DREADSTEED_GO_RUNE_TYPE_3: u32 = 179_671;
// `NPC_J_EEVEE` (cpp:34), read by `go_ai_pedestal_event_start`.
const DREADSTEED_NPC_J_EEVEE: u32 = 14_500;
// `NPC_XOROTHIAN_IMP` (cpp:35), read by wave spawns + dismiss sweep.
const DREADSTEED_NPC_XOROTHIAN_IMP: u32 = 14_482;
// `NPC_DREAD_GUARD` (cpp:36), read by wave spawns + dismiss sweep.
const DREADSTEED_NPC_DREAD_GUARD: u32 = 14_483;
// `NPC_XOROTHIAN_DREADSTEED` (cpp:37), read by phase-4 wave spawn.
const DREADSTEED_NPC_XOROTHIAN_DREADSTEED: u32 = 14_502;
// `NPC_LORD_HEL_NURATH` (cpp:38), read by phase-4 wave spawn.
const DREADSTEED_NPC_LORD_HEL_NURATH: u32 = 14_506;
// `SPELL_CANDLE_AURA` (cpp:43), read by `go_ai_ritual_node_update`.
const DREADSTEED_SPELL_CANDLE_AURA: u32 = 23_226;
// `SPELL_WHEEL_AURA` / `SPELL_BELL_AURA` (cpp:41-42), read by
// `GoAiState::for_script` (dropped by the lane, restored at integration).
const DREADSTEED_SPELL_WHEEL_AURA: u32 = 23_120;
const DREADSTEED_SPELL_BELL_AURA: u32 = 23_117;
// `SAY_HEL_NURATH` (cpp:45), read by phase-4 wave spawn.
const DREADSTEED_SAY_HEL_NURATH: i32 = 9_727;
// `SAY_DEMON_DESPAWN` (cpp:46), read by `go_ai_pedestal_dismiss_demons`.
const DREADSTEED_SAY_DEMON_DESPAWN: i32 = 9_768;

// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`); kind 1.
const DREADSTEED_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;

/// `go_ritual_nodeAI` state (cpp 731-742): single-event `EventMap`
/// reduction (`timer` + `refreshTime`); `spell` selects the aura branch.
pub(crate) struct RitualNodeAi {
    timer_ms: u32,
    refresh_ms: u32,
    spell: u32,
}

impl RitualNodeAi {
    pub(crate) fn new(refresh_ms: u32, spell: u32) -> Self {
        // Fork ctor inits `timer` to 0 (cpp 733-738).
        Self {
            timer_ms: 0,
            refresh_ms,
            spell,
        }
    }
}

/// `go_pedestal_of_immol_tharAI` state (cpp 109-122 + ctor/reset cpp
/// 63-83). `nodes`/`guidGlyphTab`/`guidFlameTab` become guid arrays plus a
/// live `node_up` set; `spawnPoints` keeps x/y/z only (`m_fO`/`m_wait`
/// never used, cpp 401); `miniTimer` (cpp 121, "not used yet") is dropped.
pub(crate) struct PedestalOfImmolTharAi {
    node_guids: [ObjectGuid; 3],
    node_up: [bool; 3],
    glyph_guids: [ObjectGuid; 9],
    flame_guids: [ObjectGuid; 18],
    ritual_circle: ObjectGuid,
    gobj_timer_ms: u32,
    gobj_step: u32,
    wave_timer_ms: u32,
    wave_step: u32,
    node_timer_ms: u32,
    node_nb: u32,
    node_time_tracker_ms: u32,
    spawn_points: [[f32; 3]; 18],
    event_phase: u8,
}

impl PedestalOfImmolTharAi {
    pub(crate) fn new() -> Self {
        // Fork `reset()` (cpp 70-83) minus the ctor RNG rolls
        // (`nodeTimer`/`nodeTimeTracker`, cpp 80-81): kept 0 here, the
        // integrator rolls them in `go_ai_create` (see unresolved).
        Self {
            node_guids: [ObjectGuid::EMPTY; 3],
            node_up: [false; 3],
            glyph_guids: [ObjectGuid::EMPTY; 9],
            flame_guids: [ObjectGuid::EMPTY; 18],
            ritual_circle: ObjectGuid::EMPTY,
            gobj_timer_ms: 9_000,
            gobj_step: 0,
            wave_timer_ms: 0,
            wave_step: 0,
            node_timer_ms: 0,
            node_nb: 0,
            node_time_tracker_ms: 0,
            spawn_points: [[0.0; 3]; 18],
            event_phase: 0,
        }
    }
}

// ---- `go_urok_challenge` (`boss_urok.cpp:1-19`) ----
/// Urok massacrer add (`boss_urok.cpp:9`).
const UROK_NPC_MASSACRER: u32 = 10_601;
/// Urok mage add (`boss_urok.cpp:10`).
const UROK_NPC_MAGE: u32 = 10_602;
/// Urok Doomhowl boss (`boss_urok.cpp:11`).
const UROK_NPC_DOOMHOWL: u32 = 10_584;
/// Right-click nuke on the nearest ogre (`boss_urok.cpp:142-148`).
const UROK_SPELL_KILL_ADD: u32 = 16_452;
/// Spawn order after the first three deaths (`boss_urok.cpp:164`).
const UROK_RUNE_ORDER: [u8; 10] = [0, 2, 3, 1, 4, 5, 1, 2, 3, 5];

// ---- `go_darrowshire_trigger` (`eastern_plaguelands.cpp:919-965`) ----
/// Attacker spawn 0..2 + bloodletter anchor (`eastern_plaguelands.cpp:903-918`).
const DAR_EVENT: [[f32; 4]; 8] = [
    [1_500.04, -3_662.67, 82.832, 3.708_05],
    [1_506.17, -3_686.72, 82.876_9, 5.759_45],
    [1_512.81, -3_724.64, 87.120_99, 1.641_64],
    [1_537.6, -3_677.1, 88.7, 3.148_84],
    [1_484.68, -3_668.74, 80.695_3, 0.236_567],
    [1_493.53, -3_695.01, 80.134_7, 0.264_055],
    [1_505.28, -3_718.83, 83.343, 1.369_11],
    [1_446.8, -3_694.27, 76.596_6, 0.401_503],
];
/// Self-dup guard entry (`eastern_plaguelands.cpp:1177-1184`).
const DAR_GO_TRIGGER: u32 = 177_526;
const DAR_NPC_MARAUDING_CORPSE: u32 = 10_951;
const DAR_NPC_MARAUDING_SKELETON: u32 = 10_952;
const DAR_NPC_SERVANT_OF_HORGUS: u32 = 10_953;
const DAR_NPC_BLOODLETTER: u32 = 10_954;
const DAR_NPC_HORGUS: u32 = 10_946;
const DAR_NPC_MARDUK: u32 = 10_939;
const DAR_NPC_REDPATH_CORRUPTED: u32 = 10_938;
const DAR_NPC_BETRAYER: u32 = 10_947;
const DAR_NPC_DEFENDER: u32 = 10_948;
const DAR_NPC_DISCIPLE: u32 = 10_949;
const DAR_NPC_MILITIA: u32 = 10_950;
const DAR_NPC_DAVIL_LIGHTFIRE: u32 = 10_944;
const DAR_NPC_CAPTAIN_REDPATH: u32 = 10_937;
const DAR_NPC_JOSEPH_REDPATH: u32 = 10_936;
const DAR_NPC_DAVIL_CROKFORD: u32 = 10_945;
/// Crokford hut spawn (`eastern_plaguelands.cpp:1160`).
const DAR_CROKFORD_POS: [f32; 4] = [1_465.43, -3_678.48, 78.081_6, 0.040_217_6];
const DAR_SAY_HORGUS_DIED: i32 = 7_368;
const DAR_SAY_LIGHTFIRE_DIED: i32 = 7_366;
const DAR_SAY_REDPATH_DIED: i32 = 7_369;
const DAR_SAY_SCOURGE_DEFEATED: i32 = 7_407;
const DAR_SAY_MILITIA: [i32; 8] = [7_347, 7_348, 7_349, 7_350, 7_351, 7_352, 7_353, 7_354];
const DAR_SAY_DEFENDER_YELL: i32 = 7_358;
const DAR_SAY_LIGHTFIRE_YELL: i32 = 7_343;
const DAR_SAY_DAVIL_YELL: i32 = 7_346;
const DAR_SAY_HORGUS_YELL: i32 = 7_344;
const DAR_SAY_DAVIL_DESPAWN: i32 = 7_227;
const DAR_SAY_REDPATH_YELL: i32 = 7_355;
const DAR_SAY_REDPATH_CORRUPTED: i32 = 7_357;
const DAR_SAY_MARDUK_YELL: i32 = 7_471;

// ---- `go_dusty_rug` (`hillsbrad_foothills.cpp:44-50`) ----
/// Captured farmer herded to the keg (`hillsbrad_foothills.cpp:46`).
const RUG_NPC_CAPTURED_FARMER: u32 = 2_284;
/// Keg the farmers walk to (`hillsbrad_foothills.cpp:48`).
const RUG_GO_TAINTED_KEG: u32 = 1_729;

/// `go_urok_challengeAI` (`boss_urok.cpp:27-31`): `_actived`, `_step`,
/// `_timer`, `_spellTimer`, `_runes[6]`, `nbDeadUnderlings`,
/// `guidCurrentUnderlings[3]`. Ctor leaves `_runes` /
/// `guidCurrentUnderlings` uninitialized (cpp 29-31); ported as `None`.
pub(crate) struct UrokChallengeGoAi {
    actived: bool,
    step: u32,
    timer_ms: u32,
    spell_timer_ms: u32,
    runes: [Option<ObjectGuid>; 6],
    nb_dead_underlings: u8,
    current_underlings: [Option<ObjectGuid>; 3],
}

impl UrokChallengeGoAi {
    fn new() -> Self {
        Self {
            actived: true,
            step: 0,
            timer_ms: 0,
            spell_timer_ms: 0,
            runes: [None; 6],
            nb_dead_underlings: 0,
            current_underlings: [None; 3],
        }
    }
}

/// `go_darrowshire_triggerAI` (`eastern_plaguelands.cpp:966-988`): cleanup /
/// init flags, phase + 7 mob timers (`EventMap` reduction: timer 0 = corpse /
/// skeleton waves, 1 = defenders, 2 = servants, 3 = disciples, 4 =
/// bloodletters, 5 = militia, 6 = patrol), defender faction, summon list,
/// named-creature guids. Ctor `m_defenderFaction = 113` is overwritten by
/// `Reset()` (`cpp 968` vs `cpp 989-991`), so `new()` keeps the post-`Reset`
/// value; `SetRespawnTime(15 * MINUTE)` / `SetCreatureSummonLimit(200)` (cpp
/// 972-973) have no helper (see unresolved).
pub(crate) struct DarrowshireTriggerGoAi {
    cleanup_done: bool,
    initialized: bool,
    phase_step: u32,
    phase_timer_ms: u32,
    mob_timer_ms: [u32; 7],
    defender_faction: u32,
    summoned: Vec<ObjectGuid>,
    marduk: Option<ObjectGuid>,
    redpath: Option<ObjectGuid>,
    redpath_corrupted: Option<ObjectGuid>,
    davil: Option<ObjectGuid>,
    horgus: Option<ObjectGuid>,
}

impl DarrowshireTriggerGoAi {
    fn new() -> Self {
        Self {
            cleanup_done: false,
            initialized: false,
            phase_step: 0,
            phase_timer_ms: 6_000,
            mob_timer_ms: [15_000, 17_000, 0, 0, 0, 0, 0],
            defender_faction: 0,
            summoned: Vec::new(),
            marduk: None,
            redpath: None,
            redpath_corrupted: None,
            davil: None,
            horgus: None,
        }
    }
}

/// `go_helcular_s_graveAI` (`hillsbrad_foothills.cpp:13-27`): stored Helcular
/// guid (ctor `guid_helcular = 0`, cpp 15).
pub(crate) struct HelcularsGraveGoAi {
    helcular: Option<ObjectGuid>,
}

impl HelcularsGraveGoAi {
    fn new() -> Self {
        Self { helcular: None }
    }
}

/// `go_dusty_rugAI` (`hillsbrad_foothills.cpp:52-61`): `timer`, farmer list,
/// `step` (0 = idle).
pub(crate) struct DustyRugGoAi {
    timer_ms: u32,
    farmers: Vec<ObjectGuid>,
    step: u8,
}

impl DustyRugGoAi {
    fn new() -> Self {
        Self {
            timer_ms: 0,
            farmers: Vec::new(),
            step: 0,
        }
    }
}

/// `go_lards_picnic_basketAI` (`hinterlands.cpp:222-230`): `timer`, `state`
/// (ported as `in_use`).
pub(crate) struct LardsPicnicBasketGoAi {
    pub(crate) timer_ms: u32,
    pub(crate) in_use: bool,
}

impl LardsPicnicBasketGoAi {
    fn new() -> Self {
        Self {
            timer_ms: 0,
            in_use: false,
        }
    }
}

/// Enraged Foulweald ambusher (`ashenvale.cpp:404`).
const FTM_NPC_ENRAGED_FOULWEALD: u32 = 12_921;
/// Chief Murgut finale spawn (`ashenvale.cpp:405`).
const FTM_NPC_CHIEF_MURGUT: u32 = 12_918;
/// Karang's banner, deleted on `EventEnded` (`ashenvale.cpp:407`).
const FTM_GO_KARANG_S_BANNER: u32 = 178_205;
/// `foulwealdSpawnCoords` (`ashenvale.cpp:418`).
const FTM_SPAWN_COORDS: [[f32; 3]; 4] = [
    [2_237.48, -1_524.45, 89.782_7],
    [2_202.16, -1_544.48, 87.796],
    [2_235.44, -1_578.43, 86.494_4],
    [2_260.9, -1_547.91, 89.173_3],
];
/// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`).
const FTM_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;
/// `MOVE_PATHFINDING` (`MotionMaster.h:71`); same bit as `motion.rs`.
const FTM_MOVE_PATHFINDING: u32 = 0x001;

// ---- `go_ghost_magnet` (`desolace.cpp:285-297`) ----
/// Magrami spectre add (`desolace.cpp:288`).
const GM_NPC_MAGRAMI_SPECTRE: u32 = 11_560;
/// Aura visual checked/summoned by the ctor (`desolace.cpp:289`).
const GM_GO_GHOST_MAGNET_AURA: u32 = 177_749;
/// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`).
const GM_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;

/// `go_foulweald_totem_moundAI` (`ashenvale.cpp:422-434`):
/// `guidCurrentEnragedFoulweald[2]`, `eventPhase` (0 nothing, 1 repop, 2
/// wait, 3 Murgut, 4 done), `phaseTimer`. Ctor runs `reset()` (cpp 424);
/// the guid pair is left uninitialized there (cpp 432), ported as `None`.
pub(crate) struct FoulwealdTotemMoundGoAi {
    current_enraged: [Option<ObjectGuid>; 2],
    event_phase: u8,
    phase_timer_ms: u32,
}

impl FoulwealdTotemMoundGoAi {
    fn new() -> Self {
        Self {
            current_enraged: [None; 2],
            event_phase: 0,
            phase_timer_ms: 170_000,
        }
    }
}

/// `go_bay_of_stormsAI` (`azshara.cpp:196-205`): `m_animId`, `m_playAnimTimer`.
pub(crate) struct BayOfStormsGoAi {
    anim_id: u32,
    play_anim_timer_ms: u32,
}

impl BayOfStormsGoAi {
    fn new() -> Self {
        Self {
            anim_id: 0,
            play_anim_timer_ms: 1_000,
        }
    }
}

const CPL_CLEANSED_PAIRS: [(u32, u32); 29] = [
    (164_885, 164_881),
    (173_324, 173_325),
    (174_608, 174_609),
    (174_684, 174_685),
    (164_886, 164_882),
    (171_939, 171_940),
    (171_942, 171_943),
    (174_594, 174_612),
    (174_595, 174_613),
    (174_596, 174_614),
    (174_598, 174_615),
    (174_712, 174_714),
    (174_713, 174_715),
    (164_888, 164_883),
    (173_284, 174_622),
    (174_605, 174_623),
    (174_606, 174_624),
    (174_607, 174_625),
    (174_686, 174_687),
    (164_887, 164_884),
    (173_327, 173_326),
    (174_599, 174_616),
    (174_600, 174_617),
    (174_601, 174_618),
    (174_602, 174_619),
    (174_603, 174_620),
    (174_604, 174_621),
    (174_708, 174_710),
    (174_709, 174_711),
];

// `boss_ossirian.cpp:51-57` (fork weakness array; note the header
// `SPELL_ARCANE_WEAKNESS = 25171` differs — the cpp array's 25181 wins).
const OSS_SPELL_WEAKNESS: [u32; 5] = [25_177, 25_178, 25_180, 25_183, 25_181];
// `ruins_of_ahnqiraj.h:66` (`CRYSTAL_TRIGGER`).
const OSS_CRYSTAL_TRIGGER: u32 = 15_590;
// `TEMPSUMMON_TIMED_DESPAWN` (`ObjectDefines.h:63`).
const OSS_TEMPSUMMON_TIMED_DESPAWN: u32 = 3;
// Trigger despawn (`boss_ossirian.cpp:364`).
const OSS_TRIGGER_DESPAWN_MS: u32 = 8_000;

/// `go_corrupted_plantAI` state (`felwood.cpp:487-530`).
pub(crate) struct CorruptedPlantGoAi {
    cleansed_entry: u32,
    cleansed_guid: ObjectGuid,
}

impl CorruptedPlantGoAi {
    pub(crate) fn new() -> Self {
        Self {
            // Ctor (cpp 491-499) resolves this from the GO entry (see
            // `corrupted_plant_cleansed_for`); 0 = no pair matched.
            cleansed_entry: 0,
            cleansed_guid: ObjectGuid::EMPTY,
        }
    }
}

/// `go_inconspicuous_landmarkAI` state (`tanaris.cpp:200-233`).
/// `state` (cpp 208) renamed: false = usual, true = in use.
pub(crate) struct InconspicuousLandmarkGoAi {
    pub(crate) rearm_timer_ms: u32,
    pub(crate) in_use: bool,
}

impl InconspicuousLandmarkGoAi {
    pub(crate) fn new() -> Self {
        Self {
            rearm_timer_ms: 0,
            in_use: false,
        }
    }
}

// `InvasionData` zones (`elemental_invasions.cpp:33`).
#[allow(dead_code)] // fork enum index; fire arm folds into wildcard
const WE_EVENT_IND_FIRE: u8 = 0;
const WE_EVENT_IND_AIR: u8 = 1;
const WE_EVENT_IND_EARTH: u8 = 2;
const WE_EVENT_IND_WATER: u8 = 3;
const WE_RIFT_ZONE_FIRE: u32 = 490;
const WE_RIFT_ZONE_AIR: u32 = 1_377;
const WE_RIFT_ZONE_EARTH: u32 = 16;
const WE_RIFT_ZONE_WATER: u32 = 618;

// `InvasionData` invader entries (`elemental_invasions.cpp:8-21`).
const WE_RIFT_INVADER_FIRE: u32 = 14_460;
const WE_RIFT_INVADER_AIR: u32 = 14_455;
const WE_RIFT_INVADER_EARTH: u32 = 14_462;
const WE_RIFT_INVADER_WATER: u32 = 14_458;

// `MIN_RIFT_SPAWN` (cpp 35), `MAX_RIFT_SPAWN` (cpp 36).
const WE_RIFT_MIN_SPAWN: u32 = 3;
const WE_RIFT_MAX_SPAWN: u32 = 6;

// `m_uiTimer` reload, `70 * IN_MILLISECONDS` (cpp 91).
const WE_RIFT_UPDATE_INTERVAL_MS: u32 = 70_000;

// `HOUR * IN_MILLISECONDS` (`HOUR` = 3_600s, `Common.h:128`): ctor
// `m_uiIncreaseTimer` (cpp 45) and summon lifetime (cpp 57).
const WE_RIFT_HOUR_MS: u32 = 3_600_000;

// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`; same kind
// value as `STRAT_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN`, cpp 57).
const WE_RIFT_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;

/// `elemental_invasion_riftAI` state (cpp 48-51). `event_index` selects the
/// `InvasionData` row (cpp 33); `zone` is that row's zone. `spawn_stage`
/// inits to 1, the `GetSavedVariable(varStage, 1)` default (cpp 102); stage
/// persistence needs a saved-variable helper (see unresolved).
pub(crate) struct ElementalInvasionRiftGoAi {
    pub(crate) event_index: u8,
    pub(crate) zone: u32,
    pub(crate) timer_ms: u32,
    pub(crate) increase_timer_ms: u32,
    pub(crate) spawn_stage: u32,
    pub(crate) invaders: [Option<ObjectGuid>; 6],
}

impl ElementalInvasionRiftGoAi {
    pub(crate) fn new(event_index: u8) -> Self {
        // Ctor member inits (cpp 42-46).
        let zone = match event_index {
            WE_EVENT_IND_AIR => WE_RIFT_ZONE_AIR,
            WE_EVENT_IND_EARTH => WE_RIFT_ZONE_EARTH,
            WE_EVENT_IND_WATER => WE_RIFT_ZONE_WATER,
            _ => WE_RIFT_ZONE_FIRE,
        };
        Self {
            event_index,
            zone,
            timer_ms: 500,
            increase_timer_ms: WE_RIFT_HOUR_MS,
            spawn_stage: 1,
            invaders: [None; 6],
        }
    }
}

// `go_cheer_speakerAI` (`fireworks_show.cpp:114-239`): `EVENT_CHEER`/`EVENT_FIRE`
// (cpp 55-56) reduced to two single-shot timer fields (`None` = unscheduled).
// `m_started`/`m_big` (cpp 118-119) are set by wall-clock + game-event arms
// (cpp 149-186, unmodeled; see unresolved) and read by the timer arms below.
pub(crate) struct CheerSpeakerGoAi {
    pub(crate) started: bool,
    pub(crate) big: bool,
    pub(crate) cheer_timer_ms: Option<u32>,
    pub(crate) fire_timer_ms: Option<u32>,
}

impl CheerSpeakerGoAi {
    pub(crate) fn new() -> Self {
        // Ctor member inits (cpp 116-120).
        Self {
            started: false,
            big: false,
            cheer_timer_ms: None,
            fire_timer_ms: None,
        }
    }
}

// `FireworksMisc` sounds (cpp 50-54).
const CHEER_SOUND_CHEER_1: u32 = 8_574;
const CHEER_SOUND_CHEER_2: u32 = 8_573;
const CHEER_SOUND_CHEER_3: u32 = 8_572;
const CHEER_SOUND_CHEER_4: u32 = 8_571;

// `fireworkIds` (cpp 110).
const CHEER_FIREWORK_IDS: [u32; 23] = [
    180_703, 180_704, 180_707, 180_708, 180_720, 180_721, 180_722, 180_723, 180_724, 180_725,
    180_726, 180_727, 180_728, 180_729, 180_730, 180_731, 180_736, 180_737, 180_738, 180_739,
    180_740, 180_741, 180_733,
];

// `fireworkBigIds` (cpp 112).
const CHEER_FIREWORK_BIG_IDS: [u32; 12] = [
    180_707, 180_708, 180_722, 180_723, 180_725, 180_726, 180_729, 180_731, 180_737, 180_739,
    180_741, 180_733,
];

/// Per-gameobject `GameObjectAI` script state
/// (`GOGetAI` scripts). One variant per ported
/// script; created at GO materialization from the
/// template `script_name`, ticked from
/// `Map::go_ai_update`.
#[allow(clippy::large_enum_variant)] // one state per scripted game object
pub(crate) enum GoAiState {
    /// `go_containment_coffer`: 20s despawn countdown.
    ContainmentCoffer {
        despawn_timer_ms: u32,
    },
    /// `go_lunar_festival_firecracker`: armed despawn
    /// countdown (single-event `EventMap` reduction;
    /// `None` = unscheduled).
    LunarFestivalFirecracker {
        despawn_timer_ms: Option<u32>,
    },
    /// `go_defias_gunpowder`: stateless (only
    /// `SummonedMovementInform`).
    DefiasGunpowder,
    /// `go_fire_of_akumai`: stateless (only `OnUse`).
    FireOfAkumai,
    FireworkRocket(FireworkRocketGoAi),
    CellDoor,
    SupplyCrate,
    ViewingRoomDoor,
    /// `go_shallow_grave`: fork `GetUseCount` gate.
    ShallowGrave {
        use_count: u32,
    },
    /// `go_ritual_wheel/candle/bell` (shared node AI;
    /// the spell field selects the aura).
    RitualNode(RitualNodeAi),
    PedestalOfImmolThar(PedestalOfImmolTharAi),
    ArathiCannonFire,
    UrokChallenge(UrokChallengeGoAi),
    DarrowshireTrigger(DarrowshireTriggerGoAi),
    HelcularsGrave(#[allow(dead_code)] HelcularsGraveGoAi),
    DustyRug(DustyRugGoAi),
    LardsPicnicBasket(LardsPicnicBasketGoAi),
    Transpolyporter,
    HerodLever,
    FoulwealdTotemMound(FoulwealdTotemMoundGoAi),
    BayOfStorms(BayOfStormsGoAi),
    GhostMagnet(GhostMagnetGoAi),
    UnforgedSeal(UnforgedSealGoAi),
    ForgedSeal(ForgedSealGoAi),
    CorruptedPlant(CorruptedPlantGoAi),
    OssirianCrystal,
    InconspicuousLandmark(InconspicuousLandmarkGoAi),
    ElementalInvasionRift(ElementalInvasionRiftGoAi),
    CheerSpeaker(CheerSpeakerGoAi),
    ScourgeCircle,
    ScourgeNecropolis,
    /// `go_av_beacon`: 60 s invocation, then the war
    /// rider spawns and the beacon deletes.
    AvBeacon {
        alliance: bool,
        invocation_timer_ms: u32,
    },
    /// `go_av_landmine`: stateless (`OnUse` only; the
    /// fork `UpdateAI` respawn-delay arm runs while
    /// despawned, which this host does not tick).
    AvLandmine,
}

impl GoAiState {
    /// `ScriptMgr::GetGameObjectAI` by script name;
    /// `None` while the named script is unported.
    /// Constructor rolls that need the map RNG run
    /// in `go_ai_create` instead.
    pub(crate) fn for_script(name: &str) -> Option<Self> {
        match name {
            "go_av_beacon" => Some(Self::AvBeacon {
                alliance: false,
                invocation_timer_ms: 60_000,
            }),
            "go_av_landmine" => Some(Self::AvLandmine),
            "go_containment_coffer" => Some(Self::ContainmentCoffer {
                despawn_timer_ms: 20_000,
            }),
            "go_lunar_festival_firecracker" => Some(Self::LunarFestivalFirecracker {
                despawn_timer_ms: None,
            }),
            "go_defias_gunpowder" => Some(Self::DefiasGunpowder),
            "go_fire_of_akumai" => Some(Self::FireOfAkumai),
            "go_firework_rocket" => Some(Self::FireworkRocket(FireworkRocketGoAi::new())),
            "go_cell_door" => Some(Self::CellDoor),
            "go_supply_crate" => Some(Self::SupplyCrate),
            "go_viewing_room_door" => Some(Self::ViewingRoomDoor),
            "go_shallow_grave" => Some(Self::ShallowGrave { use_count: 0 }),
            "go_ritual_wheel" => Some(Self::RitualNode(RitualNodeAi::new(
                5_000,
                DREADSTEED_SPELL_WHEEL_AURA,
            ))),
            "go_ritual_candle" => Some(Self::RitualNode(RitualNodeAi::new(
                5_000,
                DREADSTEED_SPELL_CANDLE_AURA,
            ))),
            "go_ritual_bell" => Some(Self::RitualNode(RitualNodeAi::new(
                5_000,
                DREADSTEED_SPELL_BELL_AURA,
            ))),
            "go_pedestal_of_immol_thar" => {
                Some(Self::PedestalOfImmolThar(PedestalOfImmolTharAi::new()))
            }
            "go_arathi_cannon_fire" => Some(Self::ArathiCannonFire),
            "go_urok_challenge" => Some(Self::UrokChallenge(UrokChallengeGoAi::new())),
            "go_darrowshire_trigger" => {
                Some(Self::DarrowshireTrigger(DarrowshireTriggerGoAi::new()))
            }
            "go_helcular_s_grave" => Some(Self::HelcularsGrave(HelcularsGraveGoAi::new())),
            "go_dusty_rug" => Some(Self::DustyRug(DustyRugGoAi::new())),
            "go_lards_picnic_basket" => Some(Self::LardsPicnicBasket(LardsPicnicBasketGoAi::new())),
            "go_transpolyporter" => Some(Self::Transpolyporter),
            "go_herod_lever" => Some(Self::HerodLever),
            "go_foulweald_totem_mound" => {
                Some(Self::FoulwealdTotemMound(FoulwealdTotemMoundGoAi::new()))
            }
            "go_bay_of_storms" => Some(Self::BayOfStorms(BayOfStormsGoAi::new())),
            "go_ghost_magnet" => Some(Self::GhostMagnet(GhostMagnetGoAi::new())),
            "go_unforged_seal" => Some(Self::UnforgedSeal(UnforgedSealGoAi::new())),
            "go_forged_seal" => Some(Self::ForgedSeal(ForgedSealGoAi::new())),
            "go_corrupted_plant" => Some(Self::CorruptedPlant(CorruptedPlantGoAi::new())),
            "ossirian_crystal" => Some(Self::OssirianCrystal),
            "go_inconspicuous_landmark" => {
                Some(Self::InconspicuousLandmark(InconspicuousLandmarkGoAi::new()))
            }
            "go_elemental_invasion_rift_fire" => Some(Self::ElementalInvasionRift(
                ElementalInvasionRiftGoAi::new(0),
            )),
            "go_elemental_invasion_rift_air" => Some(Self::ElementalInvasionRift(
                ElementalInvasionRiftGoAi::new(1),
            )),
            "go_elemental_invasion_rift_earth" => Some(Self::ElementalInvasionRift(
                ElementalInvasionRiftGoAi::new(2),
            )),
            "go_elemental_invasion_rift_water" => Some(Self::ElementalInvasionRift(
                ElementalInvasionRiftGoAi::new(3),
            )),
            "go_cheer_speaker" => Some(Self::CheerSpeaker(CheerSpeakerGoAi::new())),
            "scourge_invasion_go_circle" => Some(Self::ScourgeCircle),
            "scourge_invasion_go_necropolis" => Some(Self::ScourgeNecropolis),
            _ => None,
        }
    }
}

impl Map {
    // ------------------------------------------------ gameobject scripts

    /// `GameObject::Create` AI binding
    /// (`sScriptMgr.GetGameObjectAI` by template
    /// `script_name`).
    pub(crate) fn go_ai_create(&mut self, guid: ObjectGuid) {
        let Some((script_name, entry)) = self
            .gameobjects
            .get(&guid)
            .map(|go| (go.info.script_name.clone(), go.info.id))
        else {
            return;
        };
        let Some(mut state) = GoAiState::for_script(&script_name) else {
            return;
        };
        // Fork constructor arm: the two static
        // firecracker entries schedule their despawn
        // at creation (`urand(30, 60)` seconds).
        if let GoAiState::LunarFestivalFirecracker { despawn_timer_ms } = &mut state
            && (entry == 180_763 || entry == 180_764)
        {
            *despawn_timer_ms = Some(self.spell_rng().irand(30_000, 60_000) as u32);
        }
        // `AV_BeaconInvocationObjectAI` ctor: entry picks
        // the side, `GAMEOBJECT_LEVEL` is zeroed and the
        // GO faction set (83 alliance / 84 horde). The
        // fork `SetOwnerGuid(ObjectGuid())` has no owner
        // field on Rust GOs yet (dynamic-GO runtime
        // remainder).
        if let GoAiState::AvBeacon { alliance, .. } = &mut state {
            *alliance = matches!(entry, 178_724..=178_726);
            let _ = self.object_set_field(guid, u32::from(offsets::GAMEOBJECT_LEVEL), 0);
            let _ = self.object_set_field(
                guid,
                u32::from(offsets::GAMEOBJECT_FACTION),
                if *alliance { 83 } else { 84 },
            );
        }
        // Fork ctor RNG rolls the state `new()` cannot
        // run (pedestal `reset()` node timers, cpp 80-81).
        if let GoAiState::PedestalOfImmolThar(s) = &mut state {
            s.node_timer_ms = self.spell_rng().irand(15_000, 30_000) as u32;
            s.node_time_tracker_ms = 390_000u32.wrapping_sub(s.node_timer_ms);
        }
        // Cleansed-plant pairing is keyed by GO entry
        // at creation (`GetCleansedEntry`).
        if let GoAiState::CorruptedPlant(s) = &mut state {
            s.cleansed_entry = Self::corrupted_plant_cleansed_for(entry);
        }
        // Ghost-magnet ctor aura scan.
        if let GoAiState::GhostMagnet(s) = &mut state {
            self.go_ai_ghost_magnet_on_create(guid, s);
        }
        self.go_ai.insert(guid, state);
        // Scourge ctor docks (GO-caster cast / active
        // state: documented no-ops, see the fns) and
        // the Darrowshire ctor `Reset()` defender scan.
        match script_name.as_str() {
            "scourge_invasion_go_circle" => self.go_ai_scourge_invasion_go_circle_create(guid),
            "scourge_invasion_go_necropolis" => {
                self.go_ai_scourge_invasion_go_necropolis_create(guid);
            }
            "go_darrowshire_trigger" => {
                if let Some(mut state) = self.go_ai.remove(&guid) {
                    if let GoAiState::DarrowshireTrigger(s) = &mut state {
                        self.go_ai_darrowshire_trigger_reset(guid, s);
                    }
                    if self.gameobjects.contains_key(&guid) {
                        self.go_ai.insert(guid, state);
                    }
                }
            }
            _ => {}
        }
    }

    /// `GameObjectAI::UpdateAI` tick for every GO
    /// with script state (fork: `GameObject::Update`).
    /// GOs present in the map are spawned by
    /// definition, so the fork `isSpawned()` guards
    /// are implicit.
    pub(crate) fn go_ai_update(&mut self, diff_ms: u32) {
        if self.go_ai.is_empty() {
            return;
        }
        let guids: Vec<ObjectGuid> = self.go_ai.keys().copied().collect();
        for guid in guids {
            // State is taken out so update bodies may
            // use the full `Map` API (summons, despawns);
            // it is restored only if the GO still exists.
            let Some(mut state) = self.go_ai.remove(&guid) else {
                continue;
            };
            let mut expired = false;
            match &mut state {
                GoAiState::AvBeacon {
                    alliance,
                    invocation_timer_ms,
                } => {
                    // `AV_BeaconInvocationObjectAI::UpdateAI`:
                    // at 60 s the war rider (13178 horde /
                    // 13161 alliance) is summoned 30 yd up
                    // and the beacon deletes.
                    if *invocation_timer_ms < diff_ms {
                        if let Some(pos) = self.object_position(guid) {
                            self.summon_creature(
                                if *alliance { 13161 } else { 13178 },
                                Position::new(pos.x, pos.y, pos.z + 30.0, pos.o),
                                guid,
                                5, // TEMPSUMMON_CORPSE_DESPAWN
                                10_000,
                            );
                        }
                        expired = true;
                    } else {
                        *invocation_timer_ms -= diff_ms;
                    }
                }
                GoAiState::ContainmentCoffer { despawn_timer_ms } => {
                    if *despawn_timer_ms == 0 {
                    } else if *despawn_timer_ms <= diff_ms {
                        *despawn_timer_ms = 0;
                        expired = true;
                    } else {
                        *despawn_timer_ms -= diff_ms;
                    }
                }
                GoAiState::LunarFestivalFirecracker { despawn_timer_ms } => {
                    match despawn_timer_ms {
                        Some(timer) if *timer <= diff_ms => {
                            *despawn_timer_ms = None;
                            expired = true;
                        }
                        Some(timer) => *timer -= diff_ms,
                        None => {}
                    }
                }
                GoAiState::FireworkRocket(s) => {
                    self.go_ai_firework_rocket_update(guid, s, diff_ms);
                }
                GoAiState::RitualNode(s) => self.go_ai_ritual_node_update(guid, s, diff_ms),
                GoAiState::PedestalOfImmolThar(s) => {
                    self.go_ai_pedestal_of_immol_thar_update(guid, s, diff_ms);
                }
                GoAiState::UrokChallenge(s) => {
                    self.go_ai_urok_challenge_update(guid, s, diff_ms);
                }
                GoAiState::DarrowshireTrigger(s) => {
                    self.go_ai_darrowshire_trigger_update(guid, s, diff_ms);
                }
                GoAiState::DustyRug(s) => self.go_ai_dusty_rug_update(guid, s, diff_ms),
                GoAiState::LardsPicnicBasket(s) => {
                    self.go_ai_lards_picnic_basket_update(guid, s, diff_ms);
                }
                GoAiState::FoulwealdTotemMound(s) => {
                    self.go_ai_foulweald_totem_mound_update(guid, s, diff_ms);
                }
                GoAiState::BayOfStorms(s) => self.go_ai_bay_of_storms_update(guid, s, diff_ms),
                GoAiState::GhostMagnet(s) => {
                    self.go_ai_ghost_magnet_update(guid, s, diff_ms);
                }
                GoAiState::UnforgedSeal(s) => {
                    self.go_ai_unforged_seal_update(guid, s, diff_ms);
                }
                GoAiState::ForgedSeal(s) => self.go_ai_forged_seal_update(guid, s, diff_ms),
                GoAiState::CorruptedPlant(s) => {
                    self.go_ai_corrupted_plant_update(guid, s, diff_ms);
                }
                GoAiState::InconspicuousLandmark(s) => {
                    self.go_ai_inconspicuous_landmark_update(guid, s, diff_ms);
                }
                GoAiState::ElementalInvasionRift(s) => {
                    // Fork `IsActiveEvent` head gate
                    // (event ids 68-71 by element index).
                    if self.game_event_active(68 + u32::from(s.event_index)) {
                        self.go_ai_elemental_invasion_rift_update(guid, s, diff_ms);
                    }
                }
                GoAiState::CheerSpeaker(s) => {
                    self.go_ai_cheer_speaker_update(guid, s, diff_ms);
                }
                _ => {}
            }
            if expired {
                // `Despawn()`: DB-respawn cycling of
                // removed GOs is unmodeled (recorded).
                self.despawn_gameobject_now(guid);
            } else if self.gameobjects.contains_key(&guid) {
                self.go_ai.insert(guid, state);
            }
        }
    }

    /// `GameObjectAI::OnUse` dispatch (fork:
    /// `GameObject::Use` head; the trap-trigger gate
    /// shares it). True = handled, default use
    /// skipped.
    pub(crate) fn go_ai_on_use(&mut self, user: ObjectGuid, go_guid: ObjectGuid) -> bool {
        let Some(mut state) = self.go_ai.remove(&go_guid) else {
            return false;
        };
        let handled = match &mut state {
            GoAiState::AvBeacon { .. } => {
                // `AV_BeaconInvocationObjectAI::OnUse`:
                // a completed channel deletes the beacon.
                self.despawn_gameobject_now(go_guid);
                true
            }
            GoAiState::AvLandmine => {
                // `go_av_landmineAI::OnUse`: friends are
                // ignored (handled); a hostile use
                // despawns the mine and the default trap
                // cast proceeds. Hostility is the fork GO
                // faction reaction; for the two landmine
                // entries that is entry-team (179325
                // alliance / 179324 horde) vs the user
                // (creature users' faction reaction is
                // unmodeled — treated hostile).
                let entry = self.gameobjects.get(&go_guid).map_or(0, |g| g.info.id);
                let hostile = self.player(user).is_none_or(|p| {
                    (entry == 179_325 && p.team == mangos_entities::unit::Team::Horde)
                        || (entry == 179_324 && p.team == mangos_entities::unit::Team::Alliance)
                });
                if hostile {
                    self.despawn_gameobject_now(go_guid);
                    false
                } else {
                    true
                }
            }
            GoAiState::FireOfAkumai => self.go_ai_fire_of_akumai_on_use(go_guid),
            GoAiState::LunarFestivalFirecracker { despawn_timer_ms } => {
                // `Seconds(urand(0, 2))`.
                *despawn_timer_ms = Some(self.spell_rng().irand(0, 2) as u32 * 1_000);
                true
            }
            GoAiState::CellDoor => self.go_ai_cell_door_on_use(go_guid, user),
            GoAiState::SupplyCrate => self.go_ai_supply_crate_on_use(go_guid, user),
            GoAiState::ViewingRoomDoor => self.go_ai_viewing_room_door_on_use(go_guid, user),
            GoAiState::ArathiCannonFire => self.go_ai_arathi_cannon_fire_on_use(go_guid, user),
            GoAiState::UrokChallenge(s) => self.go_ai_urok_challenge_on_use(go_guid, s, user),
            GoAiState::Transpolyporter => self.go_ai_transpolyporter_on_use(go_guid, user),
            GoAiState::HerodLever => self.go_ai_herod_lever_on_use(go_guid, user),
            GoAiState::UnforgedSeal(_) => self.go_ai_unforged_seal_on_use(go_guid, user),
            GoAiState::OssirianCrystal => self.go_ai_ossirian_crystal_on_use(go_guid, user),
            _ => false,
        };
        if self.gameobjects.contains_key(&go_guid) {
            self.go_ai.insert(go_guid, state);
        }
        handled
    }

    /// `go_fire_of_akumaiAI::OnUse` (BFD instance cpp).
    fn go_ai_fire_of_akumai_on_use(&mut self, go_guid: ObjectGuid) -> bool {
        if self.blackfathom_deeps().is_none() {
            return false;
        }
        if self.instance_get_data(BFD_TYPE_KELRIS) != Some(DONE) {
            return false;
        }
        let _ = self.object_set_field(
            go_guid,
            u32::from(offsets::GAMEOBJECT_STATE),
            GO_STATE_ACTIVE,
        );
        let flags = self.object_get_field(go_guid, offsets::GAMEOBJECT_FLAGS);
        let _ = self.object_set_field(
            go_guid,
            u32::from(offsets::GAMEOBJECT_FLAGS),
            flags | BFD_GO_FLAG_NO_INTERACT,
        );
        self.instance_set_data(BFD_TYPE_SHRINE, IN_PROGRESS);
        true
    }

    /// `GameObjectAI::SummonedMovementInform`
    /// dispatch (the summoner is a gameobject with AI
    /// state; caller already filtered to
    /// `POINT_MOTION_TYPE` arrivals).
    pub(crate) fn go_ai_summoned_movement_inform(&mut self, summoned: ObjectGuid, point_id: u32) {
        let Some(go_guid) = self.summons.get(&summoned).map(|st| st.summoner) else {
            return;
        };
        if let Some(GoAiState::DefiasGunpowder) = self.go_ai.get(&go_guid) {
            match point_id {
                0 => {
                    let dest = Position::new(-115.263_67, -617.396_1, 13.579_387, 6.182_347);
                    self.creature_move_point(summoned, 1, dest, 0, 0.0, 6.182_347);
                }
                1 => {
                    if let Some(c) = self.creatures.get_mut(&summoned) {
                        c.home_position =
                            Position::new(-115.263_67, -617.396_1, 13.579_387, 6.182_347);
                    }
                }
                _ => {}
            }
            return;
        }
        if matches!(
            self.go_ai.get(&go_guid),
            Some(GoAiState::DarrowshireTrigger(_))
        ) && let Some(mut state) = self.go_ai.remove(&go_guid)
        {
            if let GoAiState::DarrowshireTrigger(s) = &mut state {
                self.go_ai_darrowshire_trigger_movement_inform(go_guid, s, summoned, point_id);
            }
            if self.gameobjects.contains_key(&go_guid) {
                self.go_ai.insert(go_guid, state);
            }
        }
    }

    /// `GameObjectAI::JustSummoned(Creature*)`
    /// dispatch (summoner is a gameobject with AI
    /// state; hooked at the `summon_creature` tail).
    pub(crate) fn go_ai_just_summoned(&mut self, summoned: ObjectGuid) {
        let Some(go_guid) = self.summons.get(&summoned).map(|st| st.summoner) else {
            return;
        };
        if matches!(
            self.go_ai.get(&go_guid),
            Some(GoAiState::DarrowshireTrigger(_))
        ) && let Some(mut state) = self.go_ai.remove(&go_guid)
        {
            if let GoAiState::DarrowshireTrigger(s) = &mut state {
                self.go_ai_darrowshire_trigger_just_summoned(go_guid, s, summoned);
            }
            if self.gameobjects.contains_key(&go_guid) {
                self.go_ai.insert(go_guid, state);
            }
        }
    }

    /// `GameObjectAI::SummonedCreatureJustDied`
    /// dispatch. Also carries the fork creature-script
    /// death notifications aimed at GO summons of the
    /// same event (Urok underlings, Foulweald enraged,
    /// Magrami spectres — the spectre call is
    /// corpse-timer delayed in the fork; the corpse
    /// AI is unported, so it fires at death here).
    pub(crate) fn go_ai_summoned_creature_just_died(&mut self, victim: ObjectGuid) {
        let Some(go_guid) = self.summons.get(&victim).map(|st| st.summoner) else {
            return;
        };
        let tracked = matches!(
            self.go_ai.get(&go_guid),
            Some(
                GoAiState::DarrowshireTrigger(_)
                    | GoAiState::UrokChallenge(_)
                    | GoAiState::FoulwealdTotemMound(_)
                    | GoAiState::GhostMagnet(_)
            )
        );
        if !tracked {
            return;
        }
        let Some(mut state) = self.go_ai.remove(&go_guid) else {
            return;
        };
        match &mut state {
            GoAiState::DarrowshireTrigger(s) => {
                self.go_ai_darrowshire_trigger_died(go_guid, s, victim);
            }
            GoAiState::UrokChallenge(s) => {
                self.go_ai_urok_challenge_underling_died(go_guid, s, victim);
            }
            GoAiState::FoulwealdTotemMound(s) => {
                self.go_ai_foulweald_totem_mound_enraged_died(go_guid, s, victim);
            }
            GoAiState::GhostMagnet(s) => {
                self.go_ai_ghost_magnet_magrami_died(go_guid, s, victim);
            }
            _ => {}
        }
        if self.gameobjects.contains_key(&go_guid) {
            self.go_ai.insert(go_guid, state);
        }
    }

    /// `go_firework_rocketAI::UpdateAI` (cpp 340-359): single-shot
    /// despawn. Fork `isSpawned()` guard is implicit (map GOs are spawned).
    fn go_ai_firework_rocket_update(
        &mut self,
        go: ObjectGuid,
        state: &mut FireworkRocketGoAi,
        diff_ms: u32,
    ) {
        match state.despawn_timer_ms {
            Some(timer) if timer <= diff_ms => {
                state.despawn_timer_ms = None;
                // `Despawn()` (cpp 352): DB-respawn cycling unmodeled.
                self.despawn_gameobject_now(go);
            }
            Some(timer) => {
                state.despawn_timer_ms = Some(timer - diff_ms);
            }
            None => {}
        }
    }

    /// `go_cell_doorAI::OnUse` (blackrock_depths.cpp:2255-2269): marks
    /// the jail door entry opened; Crest yells at the CREST door.
    fn go_ai_cell_door_on_use(&mut self, go: ObjectGuid, user: ObjectGuid) -> bool {
        let _ = user;
        if self.blackrock_depths().is_none() {
            return true;
        }
        // `SetData(me->GetEntry(), true)` (cpp 2262): jail-door entries
        // double as instance data fields (`brd_set_data` door arms treat
        // nonzero as opened).
        if let Some(entry) = self.object_entry(go) {
            self.instance_set_data(entry, 1);
        }
        // `GetClosestCreatureWithEntry(me, NPC_CREST, 50.0f)` (cpp 2263).
        let crest = self.object_position(go).and_then(|pos| {
            self.creatures
                .values()
                .filter(|c| c.info.entry == BRD_NPC_CREST)
                .map(|c| {
                    (
                        c.unit.guid(),
                        crate::visibility::dist3d(c.unit.world.position(), pos),
                    )
                })
                .filter(|(_, d)| *d <= 50.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g)
        });
        if self.object_entry(go) == Some(BRD_GO_JAIL_DOOR_CREST) {
            if let Some(temp) = crest {
                self.do_script_text(CELL_SAY_CREST_KILLER, temp, None, -1);
            }
        }
        false
    }

    /// `go_supply_crateAI::OnUse` (stratholme.cpp:602-630): summon
    /// `urand(1, 4)` plagued critters of one `urand(0, 2)` kind at
    /// jittered points near the user, then fall through to default use.
    /// Stateless: the fork struct carries no member state.
    #[allow(clippy::cast_precision_loss)] // one fork override port
    fn go_ai_supply_crate_on_use(&mut self, go: ObjectGuid, user: ObjectGuid) -> bool {
        let _ = go;
        // C++ `maxplagued = urand(1, 4)`; `switch (urand(0, 2))`.
        let count = self.spell_rng().irand(1, 4);
        let entry = match self.spell_rng().irand(0, 2) {
            0 => STRAT_NPC_PLAGUED_RAT,    // case 0: Plagued Rat (10441)
            1 => STRAT_NPC_PLAGUED_INSECT, // case 1: Plagued Insect (10461)
            _ => STRAT_NPC_PLAGUED_MAGGOT, // case 2: Plagued Maggot (10536)
        };
        // C++ `float(urand(0, 2))` offsets; orientation 1.0;
        // `HOUR * IN_MILLISECONDS` = 3_600_000 ms.
        if let Some(at) = self.object_position(user) {
            for _ in 0..count {
                let jx = self.spell_rng().irand(0, 2) as f32;
                let jy = self.spell_rng().irand(0, 2) as f32;
                self.summon_creature(
                    entry,
                    Position::new(at.x + jx, at.y + jy, at.z, 1.0),
                    user,
                    STRAT_TEMPSUMMON_DEAD_DESPAWN,
                    3_600_000,
                );
            }
        }
        // C++ `me->SetLootState(GO_JUST_DEACTIVATED)`: no GO loot-state
        // helper exists (recorded below).
        false
    }

    /// `go_viewing_room_door::OnUse` (`instance_scholomance.cpp`): persist
    /// the door state; the default use always continues.
    fn go_ai_viewing_room_door_on_use(&mut self, _go: ObjectGuid, _user: ObjectGuid) -> bool {
        if self.scholomance().is_none() {
            return false;
        }
        self.instance_set_data(SCH_TYPE_VIEWING_ROOM_DOOR, DONE);
        false
    }

    /// `go_ritual_nodeAI::UpdateAI` (cpp 743-770): while the node is active,
    /// re-arm the aura tick; the aura cast itself needs a gameobject caster
    /// (see unresolved). Serves the wheel, candle and bell variants.
    fn go_ai_ritual_node_update(&mut self, go: ObjectGuid, state: &mut RitualNodeAi, diff_ms: u32) {
        if self.go_state(go) != GO_STATE_ACTIVE {
            state.timer_ms = 0;
            return;
        }
        if state.timer_ms < diff_ms {
            // Fork `GetInstanceData` null arm (cpp 749): nothing casts, the
            // timer still re-arms below.
            if self.dire_maul().is_some() {
                // `GetGameObject(GetData64(GO_RITUAL_CANDLE_AURA))` (cpp 751).
                if let Some(aura) = self.dm_get_data64(DM_GO_RITUAL_CANDLE_AURA) {
                    if self.gameobjects.contains_key(&aura) {
                        // `GetPlayer(GetData64(DATA_DREADSTEED_RITUAL_PLAYER))`
                        // (cpp 753).
                        if let Some(target) = self.dm_get_data64(DM_DATA_DREADSTEED_RITUAL_PLAYER) {
                            if state.spell == DREADSTEED_SPELL_CANDLE_AURA {
                                // Fork `candleAura->CastSpell(target, spell,
                                // true)` (cpp 756).
                            } else {
                                // Fork `me->CastSpell(target, spell, true)`
                                // (cpp 758).
                            }
                            // Both arms: cast BY a gameobject has no
                            // GO-caster helper (NEEDS-MAP-HELPER); the target
                            // resolution above is transcribed, the cast is not.
                            let _ = (aura, target);
                        }
                    }
                }
            }
            state.timer_ms = state.refresh_ms;
        } else {
            state.timer_ms -= diff_ms;
        }
    }

    /// `NodeUpped` (cpp 692-703): mark the hellooed node up. Runs against
    /// the suggested `GoAiState::PedestalOfImmolThar` variant (integration
    /// adds it; see unresolved).
    pub(crate) fn go_ai_pedestal_node_upped(&mut self, pedestal: ObjectGuid, node: ObjectGuid) {
        let Some(GoAiState::PedestalOfImmolThar(state)) = self.go_ai.get_mut(&pedestal) else {
            return;
        };
        for i in 0..3usize {
            if state.node_guids[i] == node {
                state.node_up[i] = true;
                break;
            }
        }
    }

    /// `GenerateGlyphAndNodeGuids` (cpp 84-108): resolve the nine rune GOs
    /// by entry cycle and the three node GOs + ritual circle by proximity.
    fn go_ai_pedestal_generate_glyph_and_node_guids(
        &mut self,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
    ) {
        // Fork builds `ObjectGuid(HIGHGUID_GAMEOBJECT, entry, counter)` per
        // DB guid (cpp 90) and looks each up; the fork has no guid-low
        // constructor, so resolve by entry (first unused match), recorded in
        // unresolved. `GOBJ_RUNE_TYPE_3 - (i % 3)` (cpp 90) cycles 3/2/1.
        let rune_entries = [
            DREADSTEED_GO_RUNE_TYPE_3,
            DREADSTEED_GO_RUNE_TYPE_2,
            DREADSTEED_GO_RUNE_TYPE_1,
        ];
        for i in 0..state.glyph_guids.len() {
            let entry = rune_entries[i % 3];
            let found = self
                .gameobjects
                .iter()
                .filter(|(_, g)| g.info.id == entry)
                .map(|(guid, _)| *guid)
                .find(|guid| !state.glyph_guids.contains(guid));
            if let Some(found) = found {
                state.glyph_guids[i] = found;
            }
            // Fork `sLog` when a rune is missing (cpp 94): no log helper.
        }
        // `FindNearestGameObject(GOBJ_WHEEL + i, 30.0)` (cpp 98): wheel,
        // candle, bell in entry order.
        let node_entries = [
            DREADSTEED_GO_WHEEL,
            DREADSTEED_GO_CANDLE,
            DREADSTEED_GO_BELL,
        ];
        if let Some(go_pos) = self.object_position(go) {
            for (i, slot) in state.node_guids.iter_mut().enumerate() {
                let entry = node_entries[i];
                *slot = self
                    .gameobjects
                    .iter()
                    .filter(|(_, g)| g.info.id == entry)
                    .map(|(guid, other)| {
                        (
                            *guid,
                            crate::visibility::dist3d(other.world.position(), go_pos),
                        )
                    })
                    .filter(|(_, d)| *d <= 30.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map_or(ObjectGuid::EMPTY, |(guid, _)| guid);
                state.node_up[i] = false;
                // Fork `sLog` when a node is missing (cpp 104): no log helper.
            }
            // `FindNearestGameObject(GOBJ_RITUAL_CIRCLE, 30.0)` (cpp 106).
            state.ritual_circle = self
                .gameobjects
                .iter()
                .filter(|(_, g)| g.info.id == DREADSTEED_GO_RITUAL_CIRCLE)
                .map(|(guid, other)| {
                    (
                        *guid,
                        crate::visibility::dist3d(other.world.position(), go_pos),
                    )
                })
                .filter(|(_, d)| *d <= 30.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(ObjectGuid::EMPTY, |(guid, _)| guid);
        }
    }

    /// `EventStart` (cpp 124-141): latch the ritual player, resolve guids,
    /// summon Jeevee. Called from `ProcessEventId_event_dreadsteed_ritual_start`
    /// (cpp 707-715), which has no dispatcher (see unresolved).
    #[allow(dead_code)] // wired by `ProcessEventId` (step 6), unwired today
    pub(crate) fn go_ai_pedestal_event_start(
        &mut self,
        player: ObjectGuid,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
    ) -> bool {
        if state.event_phase != 0 {
            return false;
        }
        // `m_pInstance->SetData64(DATA_DREADSTEED_RITUAL_PLAYER, ...)` (cpp 129).
        self.instance_set_data64(DM_DATA_DREADSTEED_RITUAL_PLAYER, player);
        self.go_ai_pedestal_generate_glyph_and_node_guids(go, state);
        state.event_phase = 1;
        // `SummonCreature(NPC_J_EEVEE, 0,0,0,0, TIMED_OR_DEAD_DESPAWN, 420000)`
        // (cpp 132).
        if self
            .summon_creature(
                DREADSTEED_NPC_J_EEVEE,
                Position::new(0.0, 0.0, 0.0, 0.0),
                go,
                DREADSTEED_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                420_000,
            )
            .is_some()
        {
            // `jeeveeAI->ShoutFreedom()` + `SetPlayerGuid` (cpp 134-138): no
            // creature-AI cross-call helper (NEEDS-MAP-HELPER, see unresolved).
        }
        true
    }

    /// `EventSecondPartStart` (cpp 142-162): open phase 4. Called from
    /// `ProcessEventId_event_dreadsteed_ritual_second_part` (cpp 779-789),
    /// which has no dispatcher (see unresolved).
    #[allow(dead_code)] // wired by `ProcessEventId` (step 6), unwired today
    pub(crate) fn go_ai_pedestal_event_second_part_start(
        &mut self,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
    ) {
        if state.event_phase != 3 {
            return;
        }
        state.event_phase = 4;
        state.wave_timer_ms = 8_000;
        state.wave_step = 0;
        let circle = state.ritual_circle;
        if !circle.is_empty() && self.gameobjects.contains_key(&circle) {
            let _ =
                self.object_set_field(circle, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_READY);
            // `SetSpawnedByDefault(false)` + `SetLootState(GO_READY)` +
            // `SetRespawnTime(1)` (cpp 152-154): unmodeled.
        }
        // `FindNearestGameObject(GOBJ_DREADSTEED_PORTAL, 10.0)` (cpp 156):
        // `SetSpawnedByDefault(true)` + `Refresh()` unmodeled.
        if let Some(go_pos) = self.object_position(go) {
            let portal: Option<ObjectGuid> = self
                .gameobjects
                .iter()
                .filter(|(_, g)| g.info.id == DREADSTEED_GO_FEL_FIRE)
                .map(|(guid, other)| {
                    (
                        *guid,
                        crate::visibility::dist3d(other.world.position(), go_pos),
                    )
                })
                .filter(|(_, d)| *d <= 10.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(guid, _)| guid);
            let _ = portal;
        }
        state.gobj_timer_ms = 10_000;
    }

    /// `PhaseTwoEndedSuccess` (cpp 163-192): light the circle, despawn the
    /// flame ring, dismiss the wave demons, advance to phase 3.
    fn go_ai_pedestal_phase_two_ended_success(
        &mut self,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
    ) {
        let circle = state.ritual_circle;
        if !circle.is_empty() && self.gameobjects.contains_key(&circle) {
            let _ = self.object_set_field(
                circle,
                u32::from(offsets::GAMEOBJECT_STATE),
                GO_STATE_ACTIVE,
            );
        }
        for &flame in &state.flame_guids {
            if !flame.is_empty() && self.gameobjects.contains_key(&flame) {
                // `Despawn()` (cpp 173): DB-respawn cycling unmodeled.
                self.despawn_gameobject_now(flame);
            }
        }
        state.event_phase += 1;
        self.go_ai_pedestal_dismiss_demons(go);
    }

    /// Demon dismiss sweep shared by `PhaseTwoEndedSuccess` (cpp 177-191)
    /// and `EventEndedFail` (cpp 224-238): yell on the living, then despawn.
    fn go_ai_pedestal_dismiss_demons(&mut self, go: ObjectGuid) {
        if let Some(go_pos) = self.object_position(go) {
            for entry in [DREADSTEED_NPC_XOROTHIAN_IMP, DREADSTEED_NPC_DREAD_GUARD] {
                let demons: Vec<ObjectGuid> = self
                    .creatures
                    .values()
                    .filter(|c| c.info.entry == entry)
                    .filter(|c| crate::visibility::dist3d(c.unit.world.position(), go_pos) <= 30.0)
                    .map(|c| c.unit.guid())
                    .collect();
                for demon in demons {
                    if self
                        .creatures
                        .get(&demon)
                        .is_some_and(|c| c.unit.is_alive())
                    {
                        self.do_script_text(DREADSTEED_SAY_DEMON_DESPAWN, demon, None, -1);
                    }
                    // `DisappearAndDie` (cpp 183/190): no direct helper;
                    // `despawn_creature_now(.., 0)` matches the fork
                    // `DespawnOrUnsummon` immediate arm.
                    self.despawn_creature_now(demon, 0);
                }
            }
        }
    }

    /// `EventEndedFail` (cpp 193-240): despawn flames, hide glyphs/circle/
    /// nodes, dismiss demons, then `reset()`.
    fn go_ai_pedestal_event_ended_fail(
        &mut self,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
    ) {
        for &flame in &state.flame_guids {
            if !flame.is_empty() && self.gameobjects.contains_key(&flame) {
                self.despawn_gameobject_now(flame);
            }
        }
        for &glyph in &state.glyph_guids {
            if !glyph.is_empty() && self.gameobjects.contains_key(&glyph) {
                // `SetSpawnedByDefault(false)` + `Refresh()` (cpp 204-209):
                // unmodeled.
            }
        }
        let circle = state.ritual_circle;
        if !circle.is_empty() && self.gameobjects.contains_key(&circle) {
            let _ =
                self.object_set_field(circle, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_READY);
            // `SetSpawnedByDefault(false)` + `Refresh()` (cpp 211-214):
            // unmodeled.
        }
        for i in 0..3usize {
            let node = state.node_guids[i];
            if !node.is_empty() && self.gameobjects.contains_key(&node) {
                // `SetSpawnedByDefault(false)` + `Refresh()` (cpp 216-223):
                // unmodeled.
            }
            state.node_up[i] = false;
        }
        self.go_ai_pedestal_dismiss_demons(go);
        // `reset()` (cpp 70-83): literal fields restored; the RNG rolls
        // (`nodeTimer`/`nodeTimeTracker`, cpp 80-81) run here like the fork.
        state.ritual_circle = ObjectGuid::EMPTY;
        state.event_phase = 0;
        state.gobj_timer_ms = 9_000;
        state.gobj_step = 0;
        state.wave_timer_ms = 0;
        state.wave_step = 0;
        state.node_nb = 0;
        state.node_timer_ms = self.spell_rng().irand(15_000, 30_000) as u32;
        state.node_time_tracker_ms = 390_000u32.wrapping_sub(state.node_timer_ms);
    }

    /// Node raise shared by `gobjNextStep` phase-1 steps (cpp 250-289):
    /// spawn-default the node, activate it, flag it in use.
    fn go_ai_pedestal_raise_node(&mut self, state: &mut PedestalOfImmolTharAi, node: usize) {
        if let Some(&guid) = state.node_guids.get(node) {
            if !guid.is_empty() && self.gameobjects.contains_key(&guid) {
                // `SetSpawnedByDefault(true)` + `Refresh()` (cpp 254-255):
                // unmodeled.
                let _ = self.object_set_field(
                    guid,
                    u32::from(offsets::GAMEOBJECT_STATE),
                    GO_STATE_ACTIVE,
                );
                let flags = self.object_get_field(guid, offsets::GAMEOBJECT_FLAGS);
                let _ = self.object_set_field(
                    guid,
                    u32::from(offsets::GAMEOBJECT_FLAGS),
                    flags | DREADSTEED_GO_FLAG_IN_USE,
                );
                if let Some(up) = state.node_up.get_mut(node) {
                    *up = true;
                }
            }
        }
    }

    /// `gobjNextStep` (cpp 241-370): phase-1 node raises, phase-2
    /// flame-ring/glyph sequence, phase-4 portal teardown.
    fn go_ai_pedestal_gobj_next_step(&mut self, go: ObjectGuid, state: &mut PedestalOfImmolTharAi) {
        match state.event_phase {
            1 => match state.gobj_step {
                // Spawn Bell (cpp 250-259).
                0 => {
                    self.go_ai_pedestal_raise_node(state, 2);
                    state.gobj_timer_ms = 5_000;
                    state.gobj_step += 1;
                }
                // Spawn Wheel (cpp 263-273).
                1 => {
                    self.go_ai_pedestal_raise_node(state, 0);
                    state.gobj_timer_ms = 6_000;
                    state.gobj_step += 1;
                }
                // Spawn Candle (cpp 276-286).
                2 => {
                    self.go_ai_pedestal_raise_node(state, 1);
                    state.gobj_timer_ms = 7_000;
                    state.gobj_step += 1;
                }
                _ => {
                    state.gobj_timer_ms = 0;
                    state.event_phase += 1;
                    state.gobj_step = 0;
                }
            },
            2 => match state.gobj_step {
                0 => {
                    // `SummonGameObject(GOBJ_FEL_FIRE, ...)` x18 ring at
                    // radius 56.7 (cpp 301-310): no temp-GO-summon helper
                    // (unmodeled); `flame_guids` stays empty and the despawn
                    // loops are vacuous.
                    let circle = state.ritual_circle;
                    if !circle.is_empty() && self.gameobjects.contains_key(&circle) {
                        let _ = self.object_set_field(
                            circle,
                            u32::from(offsets::GAMEOBJECT_STATE),
                            GO_STATE_READY,
                        );
                        // `SetSpawnedByDefault(true)` + `Refresh()` (cpp
                        // 313-315): unmodeled.
                    }
                    state.gobj_timer_ms = 45_000;
                    state.gobj_step += 1;
                }
                1..=8 => {
                    // Glyphs 0-7 (cpp 320-335): spawn-default + refresh
                    // unmodeled; the slot lookup below keeps the guid live.
                    let idx = (state.gobj_step as usize).wrapping_sub(1);
                    if let Some(&glyph) = state.glyph_guids.get(idx) {
                        if !glyph.is_empty() && self.gameobjects.contains_key(&glyph) {
                            // Fork `SetSpawnedByDefault(true)` + `Refresh()`:
                            // no spawned-default/refresh helper (unmodeled).
                        }
                    }
                    state.gobj_timer_ms = 45_000;
                    state.gobj_step += 1;
                }
                9 | 16 => {
                    // Last glyph (cpp 336-345): spawn-default + refresh +
                    // `SendGameObjectCustomAnim` unmodeled.
                    if let Some(&glyph) = state.glyph_guids.get(8) {
                        if !glyph.is_empty() && self.gameobjects.contains_key(&glyph) {
                            // Fork custom anim: no equivalent (unresolved).
                        }
                    }
                    state.gobj_timer_ms = 0;
                    state.gobj_step += 1;
                }
                _ => {
                    // Wait for the three nodes to come back up (cpp 347-358).
                    let mut up: i32 = 0;
                    for &u in &state.node_up {
                        if u {
                            up += 1;
                        }
                    }
                    if up == 3 {
                        self.go_ai_pedestal_phase_two_ended_success(go, state);
                    }
                    state.gobj_timer_ms = 20_000;
                }
            },
            // Teardown the dreadsteed portal (cpp 361-368): spawn-default +
            // refresh unmodeled.
            _ => {
                if let Some(go_pos) = self.object_position(go) {
                    let portal: Option<ObjectGuid> = self
                        .gameobjects
                        .iter()
                        .filter(|(_, g)| g.info.id == DREADSTEED_GO_FEL_FIRE)
                        .map(|(guid, other)| {
                            (
                                *guid,
                                crate::visibility::dist3d(other.world.position(), go_pos),
                            )
                        })
                        .filter(|(_, d)| *d <= 10.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(guid, _)| guid);
                    let _ = portal;
                }
                state.gobj_step += 1;
            }
        }
    }

    /// `SummonImp` (cpp 371-385): random stored spawn point, walk home.
    fn go_ai_pedestal_summon_imp(&mut self, go: ObjectGuid, state: &mut PedestalOfImmolTharAi) {
        // `urand(0, 17)` (cpp 373), bounds-guarded slot read.
        let i = (self.spell_rng().irand(0, 17) as usize).min(17);
        let [x, y, z] = state.spawn_points[i];
        if let Some(home) = self.object_position(go) {
            if let Some(crea) = self.summon_creature(
                DREADSTEED_NPC_XOROTHIAN_IMP,
                Position::new(x, y, z, 0.0),
                go,
                DREADSTEED_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                600_000,
            ) {
                // `SetFacingToObject(me)` (cpp 376): no facing helper
                // (NEEDS-MAP-HELPER).
                self.set_walk(crea, false);
                if let Some(c) = self.creatures.get_mut(&crea) {
                    c.home_position = Position::new(home.x, home.y, home.z, 0.0);
                }
                // `Clear/Initialize/MovePoint(1, home)` (cpp 382-384): motion
                // reset unmodeled; the point generator below matches the
                // `home_position` precedent in content.rs.
                self.creature_move_point(
                    crea,
                    1,
                    Position::new(home.x, home.y, home.z, 0.0),
                    0,
                    0.0,
                    0.0,
                );
            }
        }
    }

    /// `SummonGuard` (cpp 386-400): same as `SummonImp` with the guard entry.
    fn go_ai_pedestal_summon_guard(&mut self, go: ObjectGuid, state: &mut PedestalOfImmolTharAi) {
        let i = (self.spell_rng().irand(0, 17) as usize).min(17);
        let [x, y, z] = state.spawn_points[i];
        if let Some(home) = self.object_position(go) {
            if let Some(crea) = self.summon_creature(
                DREADSTEED_NPC_DREAD_GUARD,
                Position::new(x, y, z, 0.0),
                go,
                DREADSTEED_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                600_000,
            ) {
                // `SetFacingToObject(me)` (cpp 391): no facing helper
                // (NEEDS-MAP-HELPER).
                self.set_walk(crea, false);
                if let Some(c) = self.creatures.get_mut(&crea) {
                    c.home_position = Position::new(home.x, home.y, home.z, 0.0);
                }
                // `Clear/Initialize/MovePoint(1, home)` (cpp 397-399): motion
                // reset unmodeled; see `go_ai_pedestal_summon_imp`.
                self.creature_move_point(
                    crea,
                    1,
                    Position::new(home.x, home.y, home.z, 0.0),
                    0,
                    0.0,
                    0.0,
                );
            }
        }
    }

    /// `WaveSpawn` (cpp 402-593): phase-2 imp/guard waves, phase-4
    /// dreadsteed + Lord Hel'nurath.
    #[allow(clippy::cast_possible_truncation)] // f64 trig to f32 ring coords
    #[allow(clippy::too_many_lines)] // fork switch transcription
    fn go_ai_pedestal_wave_spawn(&mut self, go: ObjectGuid, state: &mut PedestalOfImmolTharAi) {
        if state.event_phase == 2 {
            match state.wave_step {
                0 => {
                    // Fork `x += 63.0 * cos` / `y += ...` (cpp 413-417);
                    // a successful summon re-reads the GO position
                    // (cpp 421), resetting the accumulation per point.
                    if let Some(base) = self.object_position(go) {
                        let mut x = base.x;
                        let mut y = base.y;
                        for i in 1..=18u32 {
                            let angle = f64::from(i) * 2.0 * std::f64::consts::PI / 18.0 + 0.2;
                            x += (63.0 * angle.cos()) as f32;
                            y += (63.0 * angle.sin()) as f32;
                            state.spawn_points[(i - 1) as usize] = [x, y, -28.0];
                            if let Some(crea) = self.summon_creature(
                                DREADSTEED_NPC_XOROTHIAN_IMP,
                                Position::new(x, y, -28.0, 0.0),
                                go,
                                DREADSTEED_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                                600_000,
                            ) {
                                // `SetFacingToObject(me)` (cpp 423): no
                                // facing helper (NEEDS-MAP-HELPER).
                                x = base.x;
                                y = base.y;
                                if let Some(c) = self.creatures.get_mut(&crea) {
                                    c.home_position =
                                        Position::new(base.x, base.y, base.z, angle as f32);
                                }
                                self.creature_move_point(
                                    crea,
                                    1,
                                    Position::new(base.x, base.y, base.z, 0.0),
                                    0,
                                    0.0,
                                    0.0,
                                );
                            }
                        }
                    }
                    state.wave_timer_ms = 60_000;
                    state.wave_step += 1;
                }
                1 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 11_000;
                    state.wave_step += 1;
                }
                2 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 9_000;
                    state.wave_step += 1;
                }
                3 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 10_000;
                    state.wave_step += 1;
                }
                4 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 28_000;
                    state.wave_step += 1;
                }
                5 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 20_000;
                    state.wave_step += 1;
                }
                6 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 8_000;
                    state.wave_step += 1;
                }
                7 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 19_000;
                    state.wave_step += 1;
                }
                8 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 12_000;
                    state.wave_step += 1;
                }
                9 => {
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 20_000;
                    state.wave_step += 1;
                }
                10 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 23_000;
                    state.wave_step += 1;
                }
                11 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 5_000;
                    state.wave_step += 1;
                }
                12 => {
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 25_000;
                    state.wave_step += 1;
                }
                14 => {
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 39_000;
                    state.wave_step += 1;
                }
                15 | 21 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 4_000;
                    state.wave_step += 1;
                }
                17 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 20_000;
                    state.wave_step += 1;
                }
                18 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 12_000;
                    state.wave_step += 1;
                }
                19 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 50_000;
                    state.wave_step += 1;
                }
                20 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    self.go_ai_pedestal_summon_guard(go, state);
                    state.wave_timer_ms = 20_000;
                    state.wave_step += 1;
                }
                22 => {
                    self.go_ai_pedestal_summon_imp(go, state);
                    state.wave_timer_ms = 8_000;
                    state.wave_step += 1;
                }
                // Terminal arm (cpp 568-570): timer 0, no step advance.
                _ => {
                    state.wave_timer_ms = 0;
                }
            }
        } else if state.event_phase == 4 {
            match state.wave_step {
                // Pop the dreadsteed (cpp 577-583).
                0 => {
                    if let Some(at) = self.object_position(go) {
                        let _ = self.summon_creature(
                            DREADSTEED_NPC_XOROTHIAN_DREADSTEED,
                            Position::new(at.x, at.y, at.z, 0.0),
                            go,
                            DM_TEMPSUMMON_DEAD_DESPAWN,
                            0,
                        );
                    }
                    state.wave_timer_ms = 10_000;
                    state.wave_step += 1;
                }
                // Pop Lord Hel'nurath with yell (cpp 585-589). Later
                // steps have no fork case (no default arm).
                1 => {
                    if let Some(at) = self.object_position(go) {
                        if let Some(crea) = self.summon_creature(
                            DREADSTEED_NPC_LORD_HEL_NURATH,
                            Position::new(at.x, at.y, at.z, 0.0),
                            go,
                            DM_TEMPSUMMON_DEAD_DESPAWN,
                            0,
                        ) {
                            self.do_script_text(DREADSTEED_SAY_HEL_NURATH, crea, None, -1);
                        }
                    }
                    state.wave_timer_ms = 210_000;
                    state.wave_step += 1;
                }
                _ => {}
            }
        }
    }

    /// `BreakNode` (cpp 594-666): knock one live node down; fewer than two
    /// live nodes fails the event.
    fn go_ai_pedestal_break_node(&mut self, go: ObjectGuid, state: &mut PedestalOfImmolTharAi) {
        let mut up: i32 = 0;
        for &u in &state.node_up {
            if u {
                up += 1;
            }
        }
        if up < 2 {
            self.go_ai_pedestal_event_ended_fail(go, state);
            return;
        }
        // `urand(0, nbOkNodes - 1)` (cpp 607), then map to a live slot when
        // exactly two are up (cpp 608-623).
        let mut node_to_break = self.spell_rng().irand(0, up - 1);
        if up == 2 {
            let mut count = 0;
            for i in 0..3i32 {
                if state.node_up[i as usize] {
                    if node_to_break == count {
                        node_to_break = i;
                        break;
                    }
                    count += 1;
                }
            }
        }
        let node_to_break = (node_to_break as usize).min(2);
        match state.node_nb {
            0..=3 => {
                state.node_timer_ms = self.spell_rng().irand(35_000, 45_000) as u32;
                state.node_time_tracker_ms =
                    state.node_time_tracker_ms.wrapping_sub(state.node_timer_ms);
                state.node_up[node_to_break] = false;
                let guid = state.node_guids[node_to_break];
                if !guid.is_empty() && self.gameobjects.contains_key(&guid) {
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_STATE),
                        GO_STATE_READY,
                    );
                    let flags = self.object_get_field(guid, offsets::GAMEOBJECT_FLAGS);
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_FLAGS),
                        flags & !DREADSTEED_GO_FLAG_IN_USE,
                    );
                }
            }
            4..=7 => {
                // `(nodeTimeTracker > (7 - nodeNb) * 41000 + 71000) && urand(0, 1)`
                // (cpp 644).
                let threshold = (7u32.wrapping_sub(state.node_nb)) * 41_000 + 71_000;
                if state.node_time_tracker_ms > threshold && self.spell_rng().irand(0, 1) != 0 {
                    state.node_timer_ms = 70_000;
                } else {
                    state.node_timer_ms = 41_000;
                }
                state.node_time_tracker_ms =
                    state.node_time_tracker_ms.wrapping_sub(state.node_timer_ms);
                state.node_up[node_to_break] = false;
                let guid = state.node_guids[node_to_break];
                if !guid.is_empty() && self.gameobjects.contains_key(&guid) {
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_STATE),
                        GO_STATE_READY,
                    );
                    let flags = self.object_get_field(guid, offsets::GAMEOBJECT_FLAGS);
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_FLAGS),
                        flags & !DREADSTEED_GO_FLAG_IN_USE,
                    );
                }
            }
            _ => {
                state.node_timer_ms = 41_000;
                state.node_up[node_to_break] = false;
                let guid = state.node_guids[node_to_break];
                if !guid.is_empty() && self.gameobjects.contains_key(&guid) {
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_STATE),
                        GO_STATE_READY,
                    );
                    let flags = self.object_get_field(guid, offsets::GAMEOBJECT_FLAGS);
                    let _ = self.object_set_field(
                        guid,
                        u32::from(offsets::GAMEOBJECT_FLAGS),
                        flags & !DREADSTEED_GO_FLAG_IN_USE,
                    );
                }
            }
        }
        state.node_nb += 1;
    }

    /// `go_pedestal_of_immol_tharAI::UpdateAI` (cpp 667-690): gobj steps in
    /// phases 1/2/4, wave spawns in 2/4, node breaking in 2.
    fn go_ai_pedestal_of_immol_thar_update(
        &mut self,
        go: ObjectGuid,
        state: &mut PedestalOfImmolTharAi,
        diff_ms: u32,
    ) {
        if state.event_phase == 1 || state.event_phase == 2 || state.event_phase == 4 {
            if state.gobj_timer_ms < diff_ms {
                self.go_ai_pedestal_gobj_next_step(go, state);
            } else {
                state.gobj_timer_ms -= diff_ms;
            }
        }
        if state.event_phase == 2 || state.event_phase == 4 {
            if state.wave_timer_ms < diff_ms {
                self.go_ai_pedestal_wave_spawn(go, state);
            } else {
                state.wave_timer_ms -= diff_ms;
            }
        }
        if state.event_phase == 2 {
            if state.node_timer_ms < diff_ms {
                self.go_ai_pedestal_break_node(go, state);
            } else {
                state.node_timer_ms -= diff_ms;
            }
        }
    }

    // ------------------------------------------------ go scripts: ek misc a

    /// `go_arathi_cannon_fireAI::OnUse` (`arathi_highlands.cpp:429-433`):
    /// disables the trap (always handled). Stateless.
    #[allow(clippy::unused_self)]
    fn go_ai_arathi_cannon_fire_on_use(&mut self, _go: ObjectGuid, _user: ObjectGuid) -> bool {
        true
    }

    /// Nearest live ogre (massacrer/mage) within 20 of the mound
    /// (`go_urok_challengeAI::NearestOgre`, `boss_urok.cpp:120-138`).
    fn urok_nearest_ogre(&self, go: ObjectGuid) -> Option<ObjectGuid> {
        let go_pos = self.object_position(go)?;
        self.creatures
            .values()
            .filter(|c| {
                (c.info.entry == UROK_NPC_MASSACRER || c.info.entry == UROK_NPC_MAGE)
                    && c.unit.is_alive()
            })
            .map(|c| {
                (
                    c.unit.guid(),
                    crate::visibility::dist3d(c.unit.world.position(), go_pos),
                )
            })
            .filter(|(_, d)| *d <= 20.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// `go_urok_challengeAI::DespawnRunes` (`boss_urok.cpp:110-115`).
    fn urok_despawn_runes(&mut self, state: &mut UrokChallengeGoAi) {
        for slot in &mut state.runes {
            if let Some(rune) = slot.take() {
                // `AddObjectToRemoveList`: DB-respawn cycling unmodeled.
                self.despawn_gameobject_now(rune);
            }
        }
    }

    /// `go_urok_challengeAI::SpawnAtRune` (`boss_urok.cpp:65-93`): summon at
    /// the stored rune GO. `SetRespawnDelay` (cpp 71), `SendSpellGo` visuals
    /// (cpp 74/79), `DefineGoChallenge` underling link (cpp 78) and the
    /// attack-everyone loop (cpp 81-86) have no helper (see unresolved);
    /// `current_underlings[i]` is still recorded for `UrokUnderlingDied`.
    fn urok_spawn_at_rune(
        &mut self,
        go: ObjectGuid,
        state: &mut UrokChallengeGoAi,
        rune: usize,
        entry: u32,
        slot: usize,
    ) {
        // Fork `ASSERT` index guard.
        if rune >= state.runes.len() || slot >= state.current_underlings.len() {
            return;
        }
        let Some(rune_go) = state.runes[rune] else {
            return;
        };
        let Some(at) = self.object_position(rune_go) else {
            return;
        };
        let at = Position::new(at.x, at.y, at.z, 0.0);
        if let Some(summoned) = self.summon_creature(
            entry,
            at,
            go,
            TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT,
            400_000,
        ) {
            // NEEDS-MAP-HELPER: `SetRespawnDelay(7 * DAY)` (cpp 71),
            // `SendSpellGo` visuals (cpp 74/79), `DefineGoChallenge` link
            // (cpp 78), attack-all-players loop (cpp 81-86).
            if entry != UROK_NPC_DOOMHOWL {
                state.current_underlings[slot] = Some(summoned);
            }
        }
    }

    /// `go_urok_challengeAI::SpawnBoss` (`boss_urok.cpp:95-99`).
    fn urok_spawn_boss(&mut self, go: ObjectGuid, state: &mut UrokChallengeGoAi) {
        self.urok_spawn_at_rune(go, state, 5, UROK_NPC_DOOMHOWL, 0);
        self.urok_despawn_runes(state);
    }

    /// `go_urok_challengeAI::UpdateAI` (`boss_urok.cpp:37-63`). Step 0 spawns
    /// the six summon circles via `SummonGameObject` (cpp 44-49):
    /// NEEDS-MAP-HELPER, only the step/timer advance is transcribed.
    fn go_ai_urok_challenge_update(
        &mut self,
        go: ObjectGuid,
        state: &mut UrokChallengeGoAi,
        diff_ms: u32,
    ) {
        if state.spell_timer_ms < diff_ms {
            state.spell_timer_ms = 0;
        } else {
            state.spell_timer_ms -= diff_ms;
        }
        if !state.actived {
            return;
        }
        if state.timer_ms < diff_ms {
            match state.step {
                0 => {
                    // `SpawnRune` x6 (cpp 53-58, Tier 4
                    // B0-5).
                    const GO_SUMMON_CIRCLE: u32 = 175_571;
                    const RUNE_POS: [(f32, f32, f32, f32); 6] = [
                        (-13.7275, -384.816, 48.9746, 3.68265),
                        (-27.8804, -385.891, 48.5067, 3.7001),
                        (-24.8824, -369.619, 49.7059, 3.4034),
                        (-12.3689, -376.475, 49.335, 5.044),
                        (-34.5437, -370.233, 50.3396, 5.35816),
                        (-47.9095, -369.089, 51.5425, 1.37881),
                    ];
                    for (i, (x, y, z, o)) in RUNE_POS.into_iter().enumerate() {
                        state.runes[i] = self.summon_gameobject(
                            go,
                            GO_SUMMON_CIRCLE,
                            Position::new(x, y, z, o),
                            [0.0; 4],
                            0,
                            true,
                        );
                    }
                    state.timer_ms = 3_000;
                }
                1 => {
                    self.urok_spawn_at_rune(go, state, 0, UROK_NPC_MAGE, 0);
                    self.urok_spawn_at_rune(go, state, 2, UROK_NPC_MASSACRER, 1);
                    self.urok_spawn_at_rune(go, state, 3, UROK_NPC_MASSACRER, 2);
                    state.timer_ms = 10_000;
                }
                _ => {}
            }
            state.step += 1;
        } else {
            state.timer_ms -= diff_ms;
        }
    }

    /// `go_urok_challengeAI::OnUse` (`boss_urok.cpp:140-153`): while active,
    /// fire the 30s-cooldown kill spell at the nearest ogre; always handled.
    fn go_ai_urok_challenge_on_use(
        &mut self,
        go: ObjectGuid,
        state: &mut UrokChallengeGoAi,
        user: ObjectGuid,
    ) -> bool {
        if state.actived
            && state.spell_timer_ms == 0
            && let Some(target) = self.urok_nearest_ogre(go)
        {
            state.spell_timer_ms = 30_000;
            // `user->CastSpell(target, SPELL_KILL_UROK_ADD, false)` (cpp 146).
            let _ = self.cast_spell(
                user,
                UROK_SPELL_KILL_ADD,
                CastTarget::Unit(target),
                CastOptions {
                    triggered: false,
                    skip_known: true,
                    ..Default::default()
                },
            );
        }
        true
    }

    /// `go_urok_challengeAI::EventBannerDestroyed` (`boss_urok.cpp:155-161`):
    /// called from `event_banner_destroyed` / underling `BannerDestroyed`
    /// (cpp 227-231, 300-318); no dispatcher exists yet (see unresolved).
    #[allow(dead_code)] // wired by the banner DB event (step 6), unwired today
    pub(crate) fn go_ai_urok_challenge_banner_destroyed(
        &mut self,
        go: ObjectGuid,
        state: &mut UrokChallengeGoAi,
    ) {
        if !state.actived {
            return;
        }
        state.actived = false;
        self.urok_despawn_runes(state);
        // `me->Despawn()`: DB-respawn cycling unmodeled.
        self.despawn_gameobject_now(go);
    }

    /// `go_urok_challengeAI::UrokUnderlingDied` (`boss_urok.cpp:164-186`):
    /// called from `urokUnderlingAI::JustDied` (cpp 199-207); no dispatcher
    /// exists yet (see unresolved). `runeOrder[nbDeadUnderlings + 2]` is
    /// bounds-safe (`nb < 8` => index <= 9).
    fn go_ai_urok_challenge_underling_died(
        &mut self,
        go: ObjectGuid,
        state: &mut UrokChallengeGoAi,
        creature: ObjectGuid,
    ) {
        if !state.actived {
            return;
        }
        for i in 0..3 {
            if Some(creature) == state.current_underlings[i] {
                state.nb_dead_underlings += 1;
                let nb = state.nb_dead_underlings;
                if nb < 8 {
                    let entry = if self.spell_rng().irand(0, 1) != 0 {
                        UROK_NPC_MAGE
                    } else {
                        UROK_NPC_MASSACRER
                    };
                    let rune = UROK_RUNE_ORDER[(nb + 2) as usize] as usize;
                    self.urok_spawn_at_rune(go, state, rune, entry, i);
                } else if nb == 8 {
                    self.urok_spawn_boss(go, state);
                }
            }
        }
    }

    // ------------------------------------------------ `go_darrowshire_trigger`

    /// Nearest live defender within 100 (say-speaker lookup,
    /// `eastern_plaguelands.cpp:1158` pattern).
    fn darrow_nearest_defender(&self, go: ObjectGuid) -> Option<ObjectGuid> {
        let go_pos = self.object_position(go)?;
        self.creatures
            .values()
            .filter(|c| c.info.entry == DAR_NPC_DEFENDER && c.unit.is_alive())
            .map(|c| {
                (
                    c.unit.guid(),
                    crate::visibility::dist3d(c.unit.world.position(), go_pos),
                )
            })
            .filter(|(_, d)| *d <= 100.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// `me->SummonCreature` + summon-list track (`eastern_plaguelands.cpp`
    /// `JustSummoned` push, cpp 1067).
    fn darrow_summon(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        entry: u32,
        at: Position,
        despawn_ms: u32,
    ) -> Option<ObjectGuid> {
        let summoned = self.summon_creature(
            entry,
            at,
            go,
            BRD_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            despawn_ms,
        )?;
        state.summoned.push(summoned);
        Some(summoned)
    }

    /// `GetRandomPoint(anchor, radius)` scatter via `random_point`
    /// (`eastern_plaguelands.cpp:1189` pattern); falls back to the anchor.
    fn darrow_scatter(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        entry: u32,
        anchor: usize,
        radius: f32,
        despawn_ms: u32,
    ) {
        let a = DAR_EVENT[anchor];
        let center = Position::new(a[0], a[1], a[2], a[3]);
        let at = self.random_point(center, radius).unwrap_or(center);
        self.darrow_summon(go, state, entry, at, despawn_ms);
    }

    /// `go_darrowshire_triggerAI::Reset` (`eastern_plaguelands.cpp:989-1024`).
    /// The defender-faction scan (quest `QUEST_BATTLE_DARROWSHIRE` status,
    /// team, GM, area, 20-range gates, cpp 993-1015) needs session-layer
    /// helpers (see unresolved); the reachable timer/list init is kept.
    #[allow(clippy::unused_self)]
    fn go_ai_darrowshire_trigger_reset(
        &mut self,
        _go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
    ) {
        // NEEDS-MAP-HELPER: faction scan (`GetQuestStatus`, `GetTeam`,
        // `IsGameMaster`, `GetAreaId`, cpp 991-1018).
        state.defender_faction = 0;
        state.phase_step = 0;
        state.phase_timer_ms = 6_000;
        state.mob_timer_ms = [15_000, 17_000, 0, 0, 0, 0, 0];
        state.summoned.clear();
    }

    /// `go_darrowshire_triggerAI::DespawnGuid` (`eastern_plaguelands.cpp:1033-1038`).
    fn darrow_despawn_guid(&mut self, slot: &mut Option<ObjectGuid>) {
        if let Some(creature) = slot.take() {
            // `ForcedDespawn()` immediate arm.
            self.despawn_creature_now(creature, u32::MAX);
        }
    }

    /// `go_darrowshire_triggerAI::DespawnAll` (`eastern_plaguelands.cpp:1040-1061`).
    /// Per-creature `ForcedDespawn(5000)` (cpp 1051) has no timed arm; the
    /// immediate despawn is used (see unresolved). `DeleteLater` (cpp 1060):
    /// DB-respawn cycling unmodeled.
    fn go_ai_darrowshire_trigger_despawn_all(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
    ) {
        state.cleanup_done = true;
        state.mob_timer_ms = [0; 7];
        state.phase_timer_ms = 0;
        let guids = std::mem::take(&mut state.summoned);
        for guid in guids {
            // Joseph / Crokford survive the cleanup (cpp 1049-1050).
            let exempt = self.creatures.get(&guid).is_some_and(|c| {
                c.info.entry == DAR_NPC_JOSEPH_REDPATH || c.info.entry == DAR_NPC_DAVIL_CROKFORD
            });
            if !exempt && self.creatures.get(&guid).is_some_and(|c| c.unit.is_alive()) {
                self.despawn_creature_now(guid, u32::MAX);
            }
        }
        self.darrow_despawn_guid(&mut state.marduk);
        self.darrow_despawn_guid(&mut state.redpath);
        self.darrow_despawn_guid(&mut state.redpath_corrupted);
        self.darrow_despawn_guid(&mut state.davil);
        self.darrow_despawn_guid(&mut state.horgus);
        // `DespawnNearCreaturesByEntry(NPC_DARROWSHIRE_BETRAYER, 150.0f)` (cpp 1059).
        if let Some(go_pos) = self.object_position(go) {
            let betrayers: Vec<ObjectGuid> = self
                .creatures
                .values()
                .filter(|c| {
                    c.info.entry == DAR_NPC_BETRAYER
                        && crate::visibility::dist3d(c.unit.world.position(), go_pos) <= 150.0
                })
                .map(|c| c.unit.guid())
                .collect();
            for guid in betrayers {
                self.despawn_creature_now(guid, u32::MAX);
            }
        }
        self.despawn_gameobject_now(go);
    }

    /// `go_darrowshire_triggerAI::OnRemoveFromWorld` (`eastern_plaguelands.cpp:1026-1031`).
    /// No dispatcher exists yet (see unresolved).
    pub(crate) fn go_ai_darrowshire_trigger_on_remove(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
    ) {
        if state.cleanup_done || !state.initialized {
            return;
        }
        self.go_ai_darrowshire_trigger_despawn_all(go, state);
    }

    /// Set `UNIT_FIELD_FACTIONTEMPLATE` (`eastern_plaguelands.cpp:1074/1094` pattern).
    fn darrow_set_faction(&mut self, creature: ObjectGuid, faction: u32) {
        let _ = self.object_set_field(
            creature,
            u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
            faction,
        );
    }

    /// Waypoint leg for the patrol trio / bloodletter
    /// (`eastern_plaguelands.cpp:1079-1101` pattern).
    fn darrow_move_leg(&mut self, creature: ObjectGuid, id: u32, anchor: usize, speed: f32) {
        let a = DAR_EVENT[anchor];
        self.creature_move_point(
            creature,
            id,
            Position::new(a[0], a[1], a[2], a[3]),
            0,
            speed,
            a[3],
        );
    }

    /// `go_darrowshire_triggerAI::JustSummoned` (`eastern_plaguelands.cpp:1063-1104`).
    /// No dispatcher exists yet (see unresolved). `MoveRandom` (cpp 1077)
    /// maps to `creature_move_random`; `SetHomePosition` (cpp 1082/1097) is
    /// a direct field write. `ForcedDespawn(12000)` for Marduk (cpp 1101)
    /// has no timed arm (see unresolved).
    fn go_ai_darrowshire_trigger_just_summoned(
        &mut self,
        _go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        summoned: ObjectGuid,
    ) {
        let Some(entry) = self.creatures.get(&summoned).map(|c| c.info.entry) else {
            return;
        };
        match entry {
            DAR_NPC_DEFENDER | DAR_NPC_DISCIPLE | DAR_NPC_MILITIA => {
                let faction = state.defender_faction;
                self.darrow_set_faction(summoned, faction);
                self.creature_move_random(summoned, true, 0.0, 0);
            }
            DAR_NPC_MARAUDING_CORPSE | DAR_NPC_MARAUDING_SKELETON | DAR_NPC_SERVANT_OF_HORGUS => {
                self.creature_move_random(summoned, true, 0.0, 0);
            }
            DAR_NPC_BLOODLETTER => {
                self.set_walk(summoned, true);
                let a = DAR_EVENT[5];
                if let Some(c) = self.creatures.get_mut(&summoned) {
                    c.home_position = Position::new(a[0], a[1], a[2], a[3]);
                }
                self.darrow_move_leg(summoned, 0, 5, 5.0);
            }
            DAR_NPC_DAVIL_LIGHTFIRE | DAR_NPC_CAPTAIN_REDPATH => {
                let faction = state.defender_faction;
                self.darrow_set_faction(summoned, faction);
                self.set_walk(summoned, false);
                let a = DAR_EVENT[4];
                if let Some(c) = self.creatures.get_mut(&summoned) {
                    c.home_position = Position::new(a[0], a[1], a[2], a[3]);
                }
                self.darrow_move_leg(summoned, 2, 4, 5.0);
            }
            DAR_NPC_MARDUK => {
                let flags = self.object_get_field(summoned, offsets::UNIT_FIELD_FLAGS);
                let _ = self.object_set_field(
                    summoned,
                    u32::from(offsets::UNIT_FIELD_FLAGS),
                    flags | ULD_UNIT_FLAG_NOT_SELECTABLE | ULD_UNIT_FLAG_IMMUNE_TO_NPC,
                );
                // NEEDS-MAP-HELPER: `ForcedDespawn(12000)` timed arm (cpp 1101).
            }
            _ => {}
        }
    }

    /// `go_darrowshire_triggerAI::SummonedMovementInform`
    /// (`eastern_plaguelands.cpp:1106-1151`). Caller filters to
    /// `POINT_MOTION_TYPE`. No dispatcher exists yet (see unresolved).
    fn go_ai_darrowshire_trigger_movement_inform(
        &mut self,
        _go: ObjectGuid,
        _state: &mut DarrowshireTriggerGoAi,
        summoned: ObjectGuid,
        point_id: u32,
    ) {
        let Some(entry) = self.creatures.get(&summoned).map(|c| c.info.entry) else {
            return;
        };
        match entry {
            DAR_NPC_DEFENDER if point_id == 0 => {
                self.creature_move_random(summoned, true, 0.0, 0);
            }
            DAR_NPC_DAVIL_LIGHTFIRE | DAR_NPC_CAPTAIN_REDPATH | DAR_NPC_BLOODLETTER => {
                self.set_walk(summoned, true);
                match point_id {
                    0 => self.darrow_move_leg(summoned, 1, 7, 5.0),
                    1 => self.darrow_move_leg(summoned, 2, 4, 5.0),
                    2 => self.darrow_move_leg(summoned, 3, 6, 5.0),
                    3 => self.darrow_move_leg(summoned, 0, 5, 5.0),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// `go_darrowshire_triggerAI::SummonedCreatureJustDied`
    /// (`eastern_plaguelands.cpp:1153-1195`). No dispatcher exists yet (see
    /// unresolved).
    fn go_ai_darrowshire_trigger_died(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        summoned: ObjectGuid,
    ) {
        let Some(entry) = self.creatures.get(&summoned).map(|c| c.info.entry) else {
            return;
        };
        match entry {
            DAR_NPC_HORGUS => {
                if let Some(speaker) = self.darrow_nearest_defender(go) {
                    self.do_script_text(DAR_SAY_HORGUS_DIED, speaker, None, -1);
                }
                state.phase_step = 3;
                state.phase_timer_ms = 8_000;
            }
            DAR_NPC_DAVIL_LIGHTFIRE => {
                if state.phase_step < 3 {
                    if let Some(speaker) = self.darrow_nearest_defender(go) {
                        self.do_script_text(DAR_SAY_LIGHTFIRE_DIED, speaker, None, -1);
                    }
                    self.go_ai_darrowshire_trigger_despawn_all(go, state);
                }
            }
            DAR_NPC_CAPTAIN_REDPATH => {
                if state.phase_step < 5 {
                    if let Some(speaker) = self.darrow_nearest_defender(go) {
                        self.do_script_text(DAR_SAY_REDPATH_DIED, speaker, None, -1);
                    }
                    self.go_ai_darrowshire_trigger_despawn_all(go, state);
                }
            }
            DAR_NPC_REDPATH_CORRUPTED => {
                if let Some(speaker) = self.darrow_nearest_defender(go) {
                    self.do_script_text(DAR_SAY_SCOURGE_DEFEATED, speaker, None, -1);
                }
                let a = DAR_EVENT[7];
                self.darrow_summon(
                    go,
                    state,
                    DAR_NPC_JOSEPH_REDPATH,
                    Position::new(a[0], a[1], a[2], 0.0),
                    600_000,
                );
                self.darrow_summon(
                    go,
                    state,
                    DAR_NPC_DAVIL_CROKFORD,
                    Position::new(
                        DAR_CROKFORD_POS[0],
                        DAR_CROKFORD_POS[1],
                        DAR_CROKFORD_POS[2],
                        DAR_CROKFORD_POS[3],
                    ),
                    120_000,
                );
                self.go_ai_darrowshire_trigger_despawn_all(go, state);
            }
            _ => {}
        }
    }

    /// Mob-timer arms of `UpdateAI` (`eastern_plaguelands.cpp:1199-1338`).
    /// Fired when `mob_timer_ms[i]` expires; re-arms the timer.
    #[allow(clippy::too_many_lines)] // fork switch transcription
    fn darrow_mob_tick(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        index: usize,
    ) {
        match index {
            0 => {
                for j in 0..3 {
                    let amount = self.spell_rng().irand(1, 2);
                    for _ in 0..amount {
                        let entry = if self.spell_rng().irand(0, 1) != 0 {
                            DAR_NPC_MARAUDING_CORPSE
                        } else {
                            DAR_NPC_MARAUDING_SKELETON
                        };
                        self.darrow_scatter(go, state, entry, j, 5.0, 120_000);
                    }
                }
                state.mob_timer_ms[0] = 25_000;
            }
            1 => {
                for j in 4..7 {
                    self.darrow_scatter(go, state, DAR_NPC_DEFENDER, j, 5.0, 120_000);
                }
                state.mob_timer_ms[1] = 45_000;
            }
            2 => {
                if state.phase_step != 2 {
                    state.mob_timer_ms[2] = 0;
                    return;
                }
                for j in 0..3 {
                    let amount = self.spell_rng().irand(1, 2);
                    for _ in 0..amount {
                        self.darrow_scatter(go, state, DAR_NPC_SERVANT_OF_HORGUS, j, 5.0, 120_000);
                    }
                }
                state.mob_timer_ms[2] = 35_000;
            }
            3 => {
                if state.phase_step <= 2 {
                    state.mob_timer_ms[3] = 0;
                    return;
                }
                for j in 4..7 {
                    self.darrow_scatter(go, state, DAR_NPC_DISCIPLE, j, 5.0, 120_000);
                }
                state.mob_timer_ms[3] = 45_000;
            }
            4 => {
                for _ in 0..3 {
                    self.darrow_scatter(go, state, DAR_NPC_BLOODLETTER, 3, 5.0, 120_000);
                }
                state.mob_timer_ms[4] = 35_000;
            }
            5 => {
                if state.phase_step <= 4 {
                    state.mob_timer_ms[5] = 0;
                    return;
                }
                let mut yelled = false;
                for a in DAR_EVENT.iter().skip(4).take(3) {
                    let center = Position::new(a[0], a[1], a[2], a[3]);
                    let at = self.random_point(center, 6.0).unwrap_or(center);
                    if let Some(militia) =
                        self.darrow_summon(go, state, DAR_NPC_MILITIA, at, 120_000)
                        && !yelled
                    {
                        let say = DAR_SAY_MILITIA[self.spell_rng().irand(0, 7) as usize];
                        self.do_script_text(say, militia, None, -1);
                        yelled = true;
                    }
                }
                state.mob_timer_ms[5] = 45_000;
            }
            6 => {
                // Patrol: idle bloodletters / Davil / Redpath walk the
                // triangle (`eastern_plaguelands.cpp:1300-1331`).
                let guids = state.summoned.clone();
                for guid in guids {
                    let Some(entry) = self.creatures.get(&guid).map(|c| c.info.entry) else {
                        continue;
                    };
                    if entry != DAR_NPC_BLOODLETTER
                        && entry != DAR_NPC_DAVIL_LIGHTFIRE
                        && entry != DAR_NPC_CAPTAIN_REDPATH
                    {
                        continue;
                    }
                    let idle = self.creatures.get(&guid).is_some_and(|c| c.unit.is_alive())
                        && !self.ai_in_combat(guid)
                        && self.motion_type_of(guid) != crate::motion::POINT_MOTION_TYPE;
                    if !idle {
                        continue;
                    }
                    let anchor = match self.spell_rng().irand(0, 3) {
                        0 => 5,
                        1 => 7,
                        2 => 4,
                        _ => 6,
                    };
                    let point = match anchor {
                        5 => 0,
                        7 => 1,
                        4 => 2,
                        _ => 3,
                    };
                    self.darrow_move_leg(guid, point, anchor, 5.0);
                }
                state.mob_timer_ms[6] = 5_000;
            }
            _ => {}
        }
    }

    /// Phase-timer arms of `UpdateAI` (`eastern_plaguelands.cpp:1340-1499`).
    #[allow(clippy::too_many_lines)] // fork switch transcription
    fn darrow_phase_tick(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        _diff_ms: u32,
    ) {
        match state.phase_step {
            0 => {
                let a = DAR_EVENT[7];
                if let Some(defender) = self.darrow_summon(
                    go,
                    state,
                    DAR_NPC_DEFENDER,
                    Position::new(a[0], a[1], a[2], 0.0),
                    120_000,
                ) {
                    self.do_script_text(DAR_SAY_DEFENDER_YELL, defender, None, -1);
                    self.set_walk(defender, false);
                    let h = DAR_EVENT[4];
                    if let Some(c) = self.creatures.get_mut(&defender) {
                        c.home_position = Position::new(h[0], h[1], h[2], h[3]);
                    }
                    self.darrow_move_leg(defender, 0, 4, 3.0);
                    state.phase_timer_ms = self.spell_rng().irand(120_000, 180_000) as u32;
                    state.phase_step = 1;
                }
            }
            1 => {
                let a = DAR_EVENT[7];
                if let Some(davil) = self.darrow_summon(
                    go,
                    state,
                    DAR_NPC_DAVIL_LIGHTFIRE,
                    Position::new(a[0], a[1], a[2], 0.0),
                    120_000,
                ) {
                    self.do_script_text(DAR_SAY_LIGHTFIRE_YELL, davil, None, -1);
                    state.davil = Some(davil);
                    state.phase_timer_ms = 60_000;
                    state.mob_timer_ms[2] = 4_000;
                    state.mob_timer_ms[3] = 6_000;
                    state.mob_timer_ms[6] = 10_000;
                    state.phase_step = 2;
                }
            }
            2 => {
                let Some(davil_go) = state.davil else { return };
                let Some(davil_pos) = self.unit_position(davil_go) else {
                    return;
                };
                if state
                    .horgus
                    .is_some_and(|h| self.creatures.contains_key(&h))
                {
                    self.do_script_text(DAR_SAY_DAVIL_YELL, davil_go, None, -1);
                    state.phase_timer_ms = 0;
                    return;
                }
                let at = self.random_point(davil_pos, 6.0).unwrap_or(davil_pos);
                if let Some(horgus) = self.darrow_summon(go, state, DAR_NPC_HORGUS, at, 120_000) {
                    self.attack_start(horgus, davil_go);
                    state.horgus = Some(horgus);
                    self.do_script_text(DAR_SAY_HORGUS_YELL, horgus, None, -1);
                    state.phase_timer_ms = 3_000;
                }
            }
            3 => {
                if let Some(davil) = state.davil
                    && self.creatures.contains_key(&davil)
                {
                    // `ForcedDespawn(2000)`: no timed arm; immediate used.
                    self.despawn_creature_now(davil, u32::MAX);
                    self.do_script_text(DAR_SAY_DAVIL_DESPAWN, davil, None, -1);
                    state.phase_timer_ms = 10_000;
                    return;
                }
                let a = DAR_EVENT[7];
                if let Some(redpath) = self.darrow_summon(
                    go,
                    state,
                    DAR_NPC_CAPTAIN_REDPATH,
                    Position::new(a[0], a[1], a[2], 0.0),
                    120_000,
                ) {
                    self.do_script_text(DAR_SAY_REDPATH_YELL, redpath, None, -1);
                    state.redpath = Some(redpath);
                    state.phase_timer_ms = self.spell_rng().irand(300_000, 350_000) as u32;
                    state.phase_step = 4;
                    state.mob_timer_ms[4] = 4_000;
                    state.mob_timer_ms[5] = 6_000;
                }
            }
            4 => {
                if let Some(marduk) = state.marduk
                    && self.creatures.contains_key(&marduk)
                {
                    if let Some(redpath) = state.redpath
                        && self.creatures.contains_key(&redpath)
                    {
                        state.phase_step = 5;
                        state.phase_timer_ms = 0;
                        // `DealDamage(redpath, health, ..., DIRECT_DAMAGE,
                        // NORMAL, ...)` (cpp 1463).
                        let hp = self.unit_value_u32(redpath, offsets::UNIT_FIELD_HEALTH);
                        self.deal_damage_school(marduk, redpath, hp, false, 0);
                        if let Some(redpath_pos) = self.unit_position(redpath) {
                            if let Some(corrupted) = self.darrow_summon(
                                go,
                                state,
                                DAR_NPC_REDPATH_CORRUPTED,
                                redpath_pos,
                                120_000,
                            ) {
                                self.do_script_text(DAR_SAY_REDPATH_CORRUPTED, corrupted, None, -1);
                                state.redpath_corrupted = Some(corrupted);
                            }
                        }
                    }
                    return;
                }
                if let Some(redpath) = state.redpath
                    && let Some(redpath_pos) = self.unit_position(redpath)
                {
                    let at = self.random_point(redpath_pos, 10.0).unwrap_or(redpath_pos);
                    if let Some(marduk) = self.darrow_summon(go, state, DAR_NPC_MARDUK, at, 120_000)
                    {
                        self.do_script_text(DAR_SAY_MARDUK_YELL, marduk, None, -1);
                        state.marduk = Some(marduk);
                        state.phase_timer_ms = 5_000;
                    }
                }
            }
            _ => {}
        }
    }

    /// `go_darrowshire_triggerAI::UpdateAI` (`eastern_plaguelands.cpp:1197-1505`).
    /// Init self-dup guard (cpp 1199-1208): another trigger GO within 100
    /// despawns this one (`AddObjectToRemoveList`, cpp 1204).
    fn go_ai_darrowshire_trigger_update(
        &mut self,
        go: ObjectGuid,
        state: &mut DarrowshireTriggerGoAi,
        diff_ms: u32,
    ) {
        if !state.initialized {
            let dup = self.object_position(go).is_some_and(|p| {
                self.gameobjects
                    .values()
                    .filter(|g| g.info.id == DAR_GO_TRIGGER)
                    .filter(|g| crate::visibility::dist3d(g.world.position(), p) <= 100.0)
                    .count()
                    > 1
            });
            if dup {
                self.despawn_gameobject_now(go);
                return;
            }
            state.initialized = true;
        }
        for i in 0..7 {
            if state.mob_timer_ms[i] == 0 {
                continue;
            }
            if state.mob_timer_ms[i] <= diff_ms {
                self.darrow_mob_tick(go, state, i);
            } else {
                state.mob_timer_ms[i] -= diff_ms;
            }
        }
        if state.phase_timer_ms == 0 {
            return;
        }
        if state.phase_timer_ms <= diff_ms {
            self.darrow_phase_tick(go, state, diff_ms);
        } else {
            state.phase_timer_ms -= diff_ms;
        }
    }

    // ------------------------------------------------ `go_helcular_s_grave`

    /// `go_helcular_s_graveAI::CheckHelcularSpawned`
    /// (`hillsbrad_foothills.cpp:19-24`). `pQuestRewardedGO` body (cpp 31-42)
    /// is step-5 owned (see unresolved).
    #[allow(dead_code)] // quest-reward caller lands in step 5
    fn go_ai_helcular_s_grave_has_helcular(&self, state: &HelcularsGraveGoAi) -> bool {
        state
            .helcular
            .is_some_and(|h| self.creatures.contains_key(&h))
    }

    /// `go_helcular_s_graveAI::SetHelcularGuid` (`hillsbrad_foothills.cpp:26`).
    /// No dispatcher exists yet (see unresolved).
    #[allow(dead_code)] // quest-reward caller lands in step 5
    #[allow(clippy::unused_self)]
    fn go_ai_helcular_s_grave_set(&mut self, state: &mut HelcularsGraveGoAi, creature: ObjectGuid) {
        state.helcular = Some(creature);
    }

    // ------------------------------------------------ `go_dusty_rug`

    /// Nearest tainted keg within 10 (`hillsbrad_foothills.cpp:74/98/135` pattern).
    fn rug_nearest_keg(&self, go: ObjectGuid) -> Option<(ObjectGuid, Position)> {
        let go_pos = self.object_position(go)?;
        self.gameobjects
            .iter()
            .filter(|(_, g)| g.info.id == RUG_GO_TAINTED_KEG)
            .map(|(guid, g)| {
                (
                    *guid,
                    g.world.position(),
                    crate::visibility::dist3d(g.world.position(), go_pos),
                )
            })
            .filter(|(_, _, d)| *d <= 10.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(guid, pos, _)| (guid, pos))
    }

    /// `go_dusty_rugAI::UpdateAI` (`hillsbrad_foothills.cpp:63-127`).
    /// `GetContactPoint` (cpp 83) has no helper: farmers walk to the keg
    /// position (see unresolved). `SummonGameObject` smoke (cpp 102) and
    /// `SetRespawnTime`/`Refresh` (cpp 137-138, via `StartEvent`) have no
    /// helper (see unresolved); the summoner-side kill uses `deal_damage`
    /// for the lethal `DealDamage` (cpp 110).
    fn go_ai_dusty_rug_update(&mut self, go: ObjectGuid, state: &mut DustyRugGoAi, diff_ms: u32) {
        if state.step == 0 {
            return;
        }
        // `if (timer < uiDiff)` arm (cpp 67); `timer == 0` fires too (the
        // step-4 reset arm, cpp 119-121, relies on it).
        if state.timer_ms != 0 && state.timer_ms >= diff_ms {
            state.timer_ms -= diff_ms;
            return;
        }
        match state.step {
            1 => {
                if self.rug_nearest_keg(go).is_some() {
                    // `GetCreatureListWithEntryInGrid(farmers, 30.0f)` (cpp 77).
                    if let Some(go_pos) = self.object_position(go) {
                        let farmers: Vec<ObjectGuid> = self
                            .creatures
                            .values()
                            .filter(|c| {
                                c.info.entry == RUG_NPC_CAPTURED_FARMER
                                    && c.unit.is_alive()
                                    && crate::visibility::dist3d(c.unit.world.position(), go_pos)
                                        <= 30.0
                            })
                            .map(|c| c.unit.guid())
                            .collect();
                        for farmer in farmers {
                            state.farmers.push(farmer);
                            // NEEDS-MAP-HELPER: `GetContactPoint` offset
                            // (cpp 83); keg position used directly.
                            if let Some((_, keg_pos)) = self.rug_nearest_keg(go) {
                                self.creature_move_point(farmer, 1, keg_pos, 0, 0.0, keg_pos.o);
                            }
                        }
                    }
                }
                state.timer_ms = 4_500;
                state.step += 1;
            }
            2 => {
                if let Some(front) = state.farmers.first().copied()
                    && self.creatures.contains_key(&front)
                {
                    // `SetStandState(KNEEL)` (cpp 93).
                    self.unit_set_byte(front, offsets::UNIT_FIELD_BYTES_1, 0, STAND_STATE_KNEEL);
                }
                state.timer_ms = 2_000;
                state.step += 1;
            }
            3 => {
                // NEEDS-MAP-HELPER: `SummonGameObject(GO_TAINTED_KEG_SMOKE,
                // ..., 120)` (cpp 102).
                if let Some(front) = state.farmers.first().copied()
                    && self.creatures.contains_key(&front)
                {
                    self.unit_set_byte(front, offsets::UNIT_FIELD_BYTES_1, 0, STAND_STATE_STAND);
                }
                let farmers = std::mem::take(&mut state.farmers);
                for farmer in farmers {
                    if self.creatures.contains_key(&farmer) {
                        let hp = self.unit_value_u32(farmer, offsets::UNIT_FIELD_HEALTH);
                        self.deal_damage(farmer, farmer, hp, false);
                    }
                }
                state.timer_ms = 20_000;
                state.step += 1;
            }
            _ => {
                state.timer_ms = 0;
                state.step = 0;
            }
        }
    }

    /// `go_dusty_rugAI::StartEvent` (`hillsbrad_foothills.cpp:129-140`):
    /// called from `pQuestRewardedGO` (cpp 145-150, step-5 owned); no
    /// dispatcher exists yet (see unresolved).
    #[allow(dead_code)] // quest-reward caller lands in step 5
    fn go_ai_dusty_rug_start_event(&mut self, go: ObjectGuid, state: &mut DustyRugGoAi) {
        if state.step != 0 {
            return;
        }
        state.step = 1;
        state.timer_ms = 2_000;
        // NEEDS-MAP-HELPER: `SetRespawnTime(120)` + `Refresh` (cpp 137-138).
        let _ = self.rug_nearest_keg(go);
    }

    // ------------------------------------------------ `go_lards_picnic_basket`

    /// `go_lards_picnic_basketAI::UpdateAI` (`hinterlands.cpp:232-245`):
    /// lockout expiry re-arms the basket.
    fn go_ai_lards_picnic_basket_update(
        &mut self,
        go: ObjectGuid,
        state: &mut LardsPicnicBasketGoAi,
        diff_ms: u32,
    ) {
        if !state.in_use {
            return;
        }
        if state.timer_ms < diff_ms {
            state.in_use = false;
            // `SetGoState(GO_STATE_READY)` (cpp 239).
            let _ = self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_READY);
            // `RemoveFlag(GAMEOBJECT_FLAGS, GO_FLAG_IN_USE)` (cpp 240).
            let flags = self.object_get_field(go, offsets::GAMEOBJECT_FLAGS);
            let _ = self.object_set_field(
                go,
                u32::from(offsets::GAMEOBJECT_FLAGS),
                flags & !MC_GO_FLAG_IN_USE,
            );
        } else {
            state.timer_ms -= diff_ms;
        }
    }

    // ------------------------------------------------ go scripts: ek misc b

    /// `go_transpolyporterAI::OnUse` (`stranglethorn_vale.cpp:132-141`).
    /// No `pGOHello` body exists (registration commented out, cpp 350-351).
    /// False = player carries the item, default use proceeds.
    fn go_ai_transpolyporter_on_use(&mut self, _go: ObjectGuid, user: ObjectGuid) -> bool {
        let has_item = self
            .players
            .get(&user)
            .is_some_and(|e| e.player.quest_item_count(TP_REQUIRED_ITEM) >= 1);
        !has_item
    }

    /// `go_herod_leverAI::OnUse` (`boss_herod.cpp:357-373`).
    fn go_ai_herod_lever_on_use(&mut self, go: ObjectGuid, _user: ObjectGuid) -> bool {
        // `FindNearestGameObject(GO_HEROD_DOOR, 40.0f)` via the
        // `mc_update_rune` pattern.
        let nearest = self.object_position(go).and_then(|go_pos| {
            self.gameobjects
                .iter()
                .filter(|(_, g)| g.info.id == HL_GO_HEROD_DOOR)
                .map(|(g, other)| {
                    (
                        *g,
                        crate::visibility::dist3d(other.world.position(), go_pos),
                    )
                })
                .filter(|(_, d)| *d <= 40.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g)
        });
        if let Some(door) = nearest {
            // No loot-state accessor on `Map` (GNO precedent): the
            // `GO_READY || GO_JUST_DEACTIVATED` gate is approximated by
            // `go_state`; `ResetDoorOrButton` is a `GO_STATE_READY` write.
            if self.go_state(door) == GO_STATE_ACTIVE {
                let _ = self.object_set_field(
                    door,
                    u32::from(offsets::GAMEOBJECT_STATE),
                    GO_STATE_READY,
                );
            } else {
                self.use_door(door);
            }
        }
        true
    }

    // ------------------------------------------------ go scripts: kalimdor a

    /// Summon one enraged Foulweald + walk it to the mound
    /// (`ashenvale.cpp:445-455` / `479-488` shared body).
    /// `SetRespawnDelay` (cpp 452/486) and the `DefineFoulwealdMound`
    /// cross-AI link (cpp 453/487) have no helper (see unresolved);
    /// the slot is still recorded for `EnragedFoulwealdJustDied`.
    /// Fork `ASSERT` index guards on both indices.
    fn ftm_summon_enraged(
        &mut self,
        go: ObjectGuid,
        state: &mut FoulwealdTotemMoundGoAi,
        slot: usize,
        coords: usize,
    ) {
        if slot >= state.current_enraged.len() {
            return;
        }
        let Some(c) = FTM_SPAWN_COORDS.get(coords) else {
            return;
        };
        let at = Position::new(c[0], c[1], c[2], 0.0);
        if let Some(summoned) = self.summon_creature(
            FTM_NPC_ENRAGED_FOULWEALD,
            at,
            go,
            FTM_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            420_000,
        ) {
            // NEEDS-MAP-HELPER: `SetRespawnDelay(425)` (cpp 452),
            // `DefineFoulwealdMound` link into `npc_enraged_foulwealdAI`
            // (cpp 453, `SetMoundGuid`).
            state.current_enraged[slot] = Some(summoned);
            if let Some(go_pos) = self.object_position(go) {
                let dest = Position::new(go_pos.x, go_pos.y, go_pos.z, 0.0);
                // `MovePoint(1, x, y, z, MOVE_PATHFINDING)` (cpp 450);
                // `-10.0` keeps facing.
                self.creature_move_point(summoned, 1, dest, FTM_MOVE_PATHFINDING, 0.0, -10.0);
                if let Some(creature) = self.creatures.get_mut(&summoned) {
                    creature.home_position = Position::new(go_pos.x, go_pos.y, go_pos.z, 0.0);
                }
            }
        }
    }

    /// `go_foulweald_totem_moundAI::EventStart` (`ashenvale.cpp:436-458`),
    /// driven by `ProcessEventId_event_king_of_the_foulweald` (cpp 629-637).
    /// No process-event dispatcher exists yet (see unresolved).
    #[allow(dead_code)] // wired by `ProcessEventId` (step 6), unwired today
    pub(crate) fn go_ai_foulweald_totem_mound_event_start(
        &mut self,
        go: ObjectGuid,
        state: &mut FoulwealdTotemMoundGoAi,
        _player: ObjectGuid,
    ) -> bool {
        if state.event_phase != 0 {
            return false;
        }
        state.event_phase = 1;
        // `for (int i = 0; i < 2; i++)` over `foulwealdSpawnCoords[i]` (cpp 443).
        for i in 0..2 {
            self.ftm_summon_enraged(go, state, i, i);
        }
        true
    }

    /// `go_foulweald_totem_moundAI::EventEnded` (`ashenvale.cpp:460-465`):
    /// delete the nearest banner within 10, then `reset()`.
    #[allow(dead_code)] // wired by the creature spell-hit side (step 3), unwired today
    fn go_ai_foulweald_totem_mound_event_end(
        &mut self,
        go: ObjectGuid,
        state: &mut FoulwealdTotemMoundGoAi,
    ) {
        // `FindNearestGameObject(GO_KARANG_S_BANNER, 10)` (cpp 462).
        if let Some(go_pos) = self.object_position(go) {
            let nearest = self
                .gameobjects
                .iter()
                .filter(|(_, g)| g.info.id == FTM_GO_KARANG_S_BANNER)
                .map(|(g, other)| {
                    (
                        *g,
                        crate::visibility::dist3d(other.world.position(), go_pos),
                    )
                })
                .filter(|(_, d)| *d <= 10.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g);
            if let Some(banner) = nearest {
                // `AddObjectToRemoveList`: DB-respawn cycling unmodeled.
                self.despawn_gameobject_now(banner);
            }
        }
        // `reset()` (cpp 426-430).
        state.event_phase = 0;
        state.phase_timer_ms = 170_000;
    }

    /// `go_foulweald_totem_moundAI::EnragedFoulwealdJustDied`
    /// (`ashenvale.cpp:467-491`), called from
    /// `npc_enraged_foulwealdAI::JustDied` (cpp 547-555). No creature-to-GO
    /// AI dispatcher exists yet (see unresolved).
    fn go_ai_foulweald_totem_mound_enraged_died(
        &mut self,
        go: ObjectGuid,
        state: &mut FoulwealdTotemMoundGoAi,
        dead: ObjectGuid,
    ) {
        if state.event_phase != 1 {
            return;
        }
        for slot in 0..state.current_enraged.len() {
            if state.current_enraged[slot] == Some(dead) {
                // `pos = urand(0, 3)` (cpp 478).
                let pos = self.spell_rng().irand(0, 3) as usize;
                self.ftm_summon_enraged(go, state, slot, pos);
            }
        }
    }

    /// `go_foulweald_totem_moundAI::UpdateAI` (`ashenvale.cpp:493-529`).
    fn go_ai_foulweald_totem_mound_update(
        &mut self,
        go: ObjectGuid,
        state: &mut FoulwealdTotemMoundGoAi,
        diff_ms: u32,
    ) {
        if state.event_phase == 0 || state.event_phase > 4 {
            return;
        }
        if state.phase_timer_ms < diff_ms {
            state.event_phase += 1;
            match state.event_phase {
                2 => {
                    state.phase_timer_ms = 10_000;
                }
                3 => {
                    let c = FTM_SPAWN_COORDS[3];
                    let at = Position::new(c[0], c[1], c[2], 0.0);
                    if let Some(murgut) = self.summon_creature(
                        FTM_NPC_CHIEF_MURGUT,
                        at,
                        go,
                        FTM_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                        120_000,
                    ) {
                        // NEEDS-MAP-HELPER: `SetRespawnDelay(125)` (cpp 513).
                        if let Some(go_pos) = self.object_position(go) {
                            let dest = Position::new(go_pos.x, go_pos.y, go_pos.z, 0.0);
                            self.creature_move_point(
                                murgut,
                                1,
                                dest,
                                FTM_MOVE_PATHFINDING,
                                0.0,
                                -10.0,
                            );
                            if let Some(creature) = self.creatures.get_mut(&murgut) {
                                creature.home_position =
                                    Position::new(go_pos.x, go_pos.y, go_pos.z, 0.0);
                            }
                        }
                        // `FindNearestGameObject(GO_KARANG_S_BANNER, 10)`
                        // anchor (cpp 514-516) for `SummonGameObject(178_207
                        // = GO_KARANG_LIGHT, banner pos, 120s)` (cpp 517):
                        // NEEDS-MAP-HELPER, no GO-summon helper on `Map`.
                        let _banner_pos: Option<Position> =
                            self.object_position(go).and_then(|p| {
                                self.gameobjects
                                    .iter()
                                    .filter(|(_, g)| g.info.id == FTM_GO_KARANG_S_BANNER)
                                    .map(|(_, other)| other.world.position())
                                    .map(|bp| (bp, crate::visibility::dist3d(bp, p)))
                                    .filter(|(_, d)| *d <= 10.0)
                                    .min_by(|a, b| a.1.total_cmp(&b.1))
                                    .map(|(bp, _)| bp)
                            });
                    }
                    state.phase_timer_ms = 120_000;
                }
                4 => {
                    self.go_ai_foulweald_totem_mound_event_end(go, state);
                }
                _ => {}
            }
        } else {
            state.phase_timer_ms -= diff_ms;
        }
    }

    /// `go_bay_of_stormsAI::UpdateAI` (`azshara.cpp:207-222`): cycle the
    /// 0..2 custom anim on a 3-8s roll. Stateless apart from the two fields.
    fn go_ai_bay_of_storms_update(
        &mut self,
        _go: ObjectGuid,
        state: &mut BayOfStormsGoAi,
        diff_ms: u32,
    ) {
        if state.play_anim_timer_ms < diff_ms {
            // `m_playAnimTimer = urand(3000, 8000)` (cpp 211).
            state.play_anim_timer_ms = self.spell_rng().irand(3_000, 8_000) as u32;
            // NEEDS-MAP-HELPER: `SendGameObjectCustomAnim(m_animId)`
            // (cpp 212); no custom-anim helper on `Map`.
            if state.anim_id >= 2 {
                state.anim_id = 0;
            } else {
                state.anim_id += 1;
            }
        } else {
            state.play_anim_timer_ms -= diff_ms;
        }
    }

    /// `go_ghost_magnetAI` ctor scan (`desolace.cpp:303-326`): a spawned
    /// aura GO within 30yd disables spectre spawns, else the aura visual is
    /// summoned (120s). GOs present on the map are spawned by definition, so
    /// the fork `isSpawned()` guard is implicit. Wire into `go_ai_create`
    /// (firecracker precedent); NEEDS-MAP-HELPER for the `SummonGameObject`
    /// arm (cpp 324).
    fn go_ai_ghost_magnet_on_create(&mut self, go: ObjectGuid, state: &mut GhostMagnetGoAi) {
        let aura_nearby = self.object_position(go).is_some_and(|p| {
            self.gameobjects
                .values()
                .filter(|g| g.info.id == GM_GO_GHOST_MAGNET_AURA)
                .any(|g| crate::visibility::dist3d(g.world.position(), p) <= 30.0)
        });
        // NEEDS-MAP-HELPER: `SummonGameObject(GO_GHOST_MAGNET_AURA, me pos,
        // 120s)` when no aura is nearby (cpp 321-325).
        if aura_nearby {
            state.spawning = false;
        }
    }

    /// `go_ghost_magnetAI::spawnSpetre` (`desolace.cpp:356-364`).
    fn go_ai_ghost_magnet_spawn_spectre(&mut self, go: ObjectGuid) {
        let Some(go_pos) = self.object_position(go) else {
            return;
        };
        // `GetRandomPoint(mx, my, mz, 40, ...)` (cpp 361).
        let center = Position::new(go_pos.x, go_pos.y, go_pos.z, 0.0);
        let Some(at) = self.random_point(center, 40.0) else {
            return;
        };
        let at = Position::new(at.x, at.y, at.z, 0.0);
        // NEEDS-MAP-HELPER: `DefineMagramiMagnet` cross-AI link (cpp 363):
        // contact-point home + `MovePoint(2, ...)` live in
        // `npc_magrami_spetreAI::SetMagnetGuid` (cpp 453-463).
        let _spectre = self.summon_creature(
            GM_NPC_MAGRAMI_SPECTRE,
            at,
            go,
            GM_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            120_000,
        );
    }

    /// `go_ghost_magnetAI::UpdateAI` (`desolace.cpp:332-354`): 120s spawn
    /// window, up to 8 spectres on a 3-8s roll.
    fn go_ai_ghost_magnet_update(
        &mut self,
        go: ObjectGuid,
        state: &mut GhostMagnetGoAi,
        diff_ms: u32,
    ) {
        if !state.spawning {
            return;
        }
        if state.big_timer_ms < diff_ms {
            state.spawning = false;
        } else {
            state.big_timer_ms -= diff_ms;
        }
        if state.nb_to_spawn > 0 {
            if state.timer_ms < diff_ms {
                self.go_ai_ghost_magnet_spawn_spectre(go);
                // `timer = urand(3000, 8000)` (cpp 347).
                state.timer_ms = self.spell_rng().irand(3_000, 8_000) as u32;
                state.nb_to_spawn -= 1;
            } else {
                state.timer_ms -= diff_ms;
            }
        }
    }

    /// `go_ghost_magnetAI::MagramiSpectreDied` (`desolace.cpp:365-369`),
    /// called from `npc_magrami_spetreAI::UpdateAI_corpse` (cpp 437-451).
    /// No creature-to-GO AI dispatcher exists yet (see unresolved).
    fn go_ai_ghost_magnet_magrami_died(
        &mut self,
        go: ObjectGuid,
        state: &mut GhostMagnetGoAi,
        _spectre: ObjectGuid,
    ) {
        if state.spawning {
            self.go_ai_ghost_magnet_spawn_spectre(go);
        }
    }

    /// `go_unforged_sealAI::OnUse` (`dustwallow_marsh.cpp:679-684`):
    /// delete the seal. Always handled.
    fn go_ai_unforged_seal_on_use(&mut self, go: ObjectGuid, _user: ObjectGuid) -> bool {
        // `me->Delete()` (cpp 681): DB-respawn cycling unmodeled.
        self.despawn_gameobject_now(go);
        true
    }

    /// `go_unforged_sealAI::UpdateAI` (`dustwallow_marsh.cpp:686-692`):
    /// 3-minute despawn countdown.
    fn go_ai_unforged_seal_update(
        &mut self,
        go: ObjectGuid,
        state: &mut UnforgedSealGoAi,
        diff_ms: u32,
    ) {
        if state.despawn_timer_ms < diff_ms {
            // `me->Delete()` (cpp 689).
            self.despawn_gameobject_now(go);
        } else {
            state.despawn_timer_ms -= diff_ms;
        }
    }

    /// `go_forged_sealAI::UpdateAI` (`dustwallow_marsh.cpp:707-713`):
    /// 3-minute despawn countdown (no `OnUse` override).
    fn go_ai_forged_seal_update(
        &mut self,
        go: ObjectGuid,
        state: &mut ForgedSealGoAi,
        diff_ms: u32,
    ) {
        if state.despawn_timer_ms < diff_ms {
            // `me->Delete()` (cpp 710).
            self.despawn_gameobject_now(go);
        } else {
            state.despawn_timer_ms -= diff_ms;
        }
    }

    // ------------------------------------------------ go scripts: kalimdor b

    /// Create-time entry fixup for `go_corrupted_plantAI` (ctor,
    /// `felwood.cpp:492-499`): resolve the cleansed entry for a GO
    /// entry. Called from `go_ai_create` once the state exists.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-3 remainder)
    fn corrupted_plant_cleansed_for(entry: u32) -> u32 {
        for (corrupted, cleansed) in CPL_CLEANSED_PAIRS {
            if entry == corrupted || entry == cleansed {
                return cleansed;
            }
        }
        0
    }

    /// `go_corrupted_plantAI::UpdateAI` (`felwood.cpp:506-515`).
    /// `isSpawned()` is implicit for present GOs; the cleansed guid is
    /// cleared even when the cleansed GO is already gone (cpp 513).
    fn go_ai_corrupted_plant_update(
        &mut self,
        _go: ObjectGuid,
        state: &mut CorruptedPlantGoAi,
        _diff_ms: u32,
    ) {
        if state.cleansed_guid == ObjectGuid::EMPTY {
            return;
        }
        // `AddObjectToRemoveList`: DB-respawn cycling of removed GOs is
        // unmodeled (recorded).
        let cleansed = state.cleansed_guid;
        self.despawn_gameobject_now(cleansed);
        state.cleansed_guid = ObjectGuid::EMPTY;
    }

    /// `ossirian_crystalAI::OnUse` (`boss_ossirian.cpp:335-371`).
    /// Stateless (empty ctor, no members). False = fall through to the
    /// default use; true = already used / encounter not started.
    fn go_ai_ossirian_crystal_on_use(&mut self, go: ObjectGuid, _user: ObjectGuid) -> bool {
        if self.ruins_of_ahnqiraj().is_none() {
            // `sLog` error arm (cpp 339-343): no instance.
            return false;
        }
        let Some(go_pos) = self.object_position(go) else {
            return false;
        };
        // Already used: trigger nearby (cpp 346-347).
        let used = self.creatures.values().any(|c| {
            c.info.entry == OSS_CRYSTAL_TRIGGER
                && crate::visibility::dist3d(c.unit.world.position(), go_pos) <= 5.0
        });
        if used {
            return true;
        }
        // NEEDS-MAP-HELPER: `pInstance->SpawnNewCrystals(me->GetObjectGuid())`
        // (cpp 350; `ruins_of_ahnqiraj.h:134`, unported — same gap as the
        // `RuinsOfAhnqirajState` notes).
        // Closest Ossirian within 300 (cpp 352-358; the `GetData64`
        // guid in the log arm has no Rust read).
        let ossirian = self
            .creatures
            .iter()
            .filter(|(_, c)| c.info.entry == AQ20_NPC_OSSIRIAN)
            .map(|(g, c)| {
                (
                    *g,
                    crate::visibility::dist3d(c.unit.world.position(), go_pos),
                )
            })
            .filter(|(_, d)| *d <= 300.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g);
        let Some(ossirian) = ossirian else {
            // `sLog` error arm (cpp 354-358): no Ossirian found.
            return false;
        };
        // Encounter not started (cpp 361-362; `SelectHostileTarget`'s
        // acquire side effect is not mirrored).
        if self.unit_victim(ossirian).is_none() {
            return true;
        }
        if let Some(trigger) = self.summon_creature(
            OSS_CRYSTAL_TRIGGER,
            go_pos,
            go,
            OSS_TEMPSUMMON_TIMED_DESPAWN,
            OSS_TRIGGER_DESPAWN_MS,
        ) {
            // `SpellWeakness[urand(0, 4)]` (cpp 367); clamped, so the
            // index is always in range.
            let roll = self.spell_rng().irand(0, 4);
            let idx = (roll.clamp(0, 4)) as usize;
            let spell = OSS_SPELL_WEAKNESS[idx];
            let _ = self.cast_spell(
                trigger,
                spell,
                CastTarget::Unit(ossirian),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..Default::default()
                },
            );
        }
        false
    }

    /// `go_inconspicuous_landmarkAI::UpdateAI` (`tanaris.cpp:210-223`):
    /// 10-minute rearm countdown, then reopen + clear in-use.
    fn go_ai_inconspicuous_landmark_update(
        &mut self,
        go: ObjectGuid,
        state: &mut InconspicuousLandmarkGoAi,
        diff_ms: u32,
    ) {
        if !state.in_use {
            return;
        }
        if state.rearm_timer_ms < diff_ms {
            state.in_use = false;
            let _ = self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_READY);
            let flags = self.object_get_field(go, offsets::GAMEOBJECT_FLAGS);
            let _ = self.object_set_field(
                go,
                u32::from(offsets::GAMEOBJECT_FLAGS),
                flags & !ILM_GO_FLAG_IN_USE,
            );
        } else {
            state.rearm_timer_ms -= diff_ms;
        }
    }

    // ------------------------------------------------ go scripts: world events

    /// `elemental_invasion_riftAI::UpdateAI` head (cpp 83-91, 121-123):
    /// the free-running `m_uiIncreaseTimer` countdown and the 70s
    /// `m_uiTimer` reload. Covers all four rift scripts
    /// (`GetAI_go_elemental_invasion_rift_fire/water/earth/air`, cpp 127-133,
    /// single shared class with `EVENT_IND_*` index 0/1/2/3).
    fn go_ai_elemental_invasion_rift_update(
        &mut self,
        go: ObjectGuid,
        state: &mut ElementalInvasionRiftGoAi,
        diff_ms: u32,
    ) {
        // `m_uiIncreaseTimer` (cpp 85-86; the fork never zeroes it, it only
        // stops subtracting once below `uiDiff`).
        if state.increase_timer_ms > diff_ms {
            state.increase_timer_ms -= diff_ms;
        }
        // `m_uiTimer` (cpp 88-91, 121-123): 500ms first tick, then 70s.
        if state.timer_ms >= diff_ms {
            state.timer_ms -= diff_ms;
            return;
        }
        state.timer_ms = WE_RIFT_UPDATE_INTERVAL_MS;
        // `IsActiveEvent(InvasionData[event].event)` (cpp 94-95): game events
        // are unmodeled on `Map` (see unresolved). The tick below is the
        // event-active path; integration must gate this call on the event
        // system before wiring it into `go_ai_update`.
        self.go_ai_elemental_invasion_rift_tick(go, state);
    }

    /// Rift tick: the event-active arm of `UpdateAI` (cpp 97-120).
    fn go_ai_elemental_invasion_rift_tick(
        &mut self,
        go: ObjectGuid,
        state: &mut ElementalInvasionRiftGoAi,
    ) {
        // `me->GetZoneId() != InvasionData[...].zone` (cpp 97-99).
        let zone = self
            .object_position(go)
            .map(|pos| self.zone_and_area_id(pos).0);
        if zone != Some(state.zone) {
            return;
        }
        // `GetSavedVariable(varDeadInvaders)/GetSavedVariable(varStage)` and
        // the stage-up/`SetSavedVariable` block (cpp 101-111): no
        // saved-variable accessor on `Map` (NEEDS-MAP-HELPER, see
        // unresolved). `spawn_stage` stays at its default 1, so the count
        // below is the stage-1 minimum.
        let spawn_count = (WE_RIFT_MIN_SPAWN + state.spawn_stage.saturating_sub(1))
            .min(WE_RIFT_MAX_SPAWN) as usize;
        // `GetCreature(m_uiInvadersGuid[i])` respawn loop (cpp 116-120),
        // two-phase for borrow order.
        let count = spawn_count.min(state.invaders.len());
        let mut missing = [false; WE_RIFT_MAX_SPAWN as usize];
        for (index, slot) in state.invaders.iter().enumerate().take(count) {
            if slot.is_none_or(|guid| !self.creatures.contains_key(&guid)) {
                missing[index] = true;
            }
        }
        for (index, respawn) in missing.iter().enumerate().take(count) {
            if *respawn {
                self.go_ai_elemental_invasion_rift_spawn(go, state, index);
            }
        }
    }

    /// `elemental_invasion_riftAI::DoSpawn` (cpp 53-81).
    fn go_ai_elemental_invasion_rift_spawn(
        &mut self,
        go: ObjectGuid,
        state: &mut ElementalInvasionRiftGoAi,
        index: usize,
    ) -> bool {
        // `InvasionData[m_uiEventIndex].invader` (cpp 33, 57).
        let entry = match state.event_index {
            WE_EVENT_IND_AIR => WE_RIFT_INVADER_AIR,
            WE_EVENT_IND_EARTH => WE_RIFT_INVADER_EARTH,
            WE_EVENT_IND_WATER => WE_RIFT_INVADER_WATER,
            _ => WE_RIFT_INVADER_FIRE,
        };
        // `me->GetPosition` + `SummonCreature(invader, x, y, z, 0,
        // TEMPSUMMON_TIMED_OR_DEAD_DESPAWN, HOUR * IN_MILLISECONDS)` (cpp 55-57).
        let Some(at) = self.object_position(go) else {
            return false;
        };
        let Some(invader) = self.summon_creature(
            entry,
            at,
            go,
            WE_RIFT_TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            WE_RIFT_HOUR_MS,
        ) else {
            return false;
        };
        // `GetWalkRandomPosition` spread (cpp 63-67): no walk-position helper
        // on `Map` (NEEDS-MAP-HELPER, see unresolved); the summon holds at
        // the rift position.
        // `SetHomePosition` (cpp 70): direct field write
        // (defias-gunpowder precedent).
        if let Some(c) = self.creatures.get_mut(&invader) {
            c.home_position = at;
        }
        // `SetWanderDistance/SetDefaultMovementType/MoveRandom` (cpp 71-77):
        // no wander/motion-master helpers (see unresolved).
        // `m_uiInvadersGuid[index] = ...` (cpp 79), bounds-guarded.
        let Some(slot) = state.invaders.get_mut(index) else {
            return false;
        };
        *slot = Some(invader);
        true
    }

    /// `go_cheer_speakerAI::UpdateAI` timer arms (cpp 140-142, 188-232).
    /// The wall-clock start/stop/big triggers (cpp 144-186) need localtime
    /// plus the game-event system (see unresolved); this transcribes the
    /// `EVENT_CHEER`/`EVENT_FIRE` handlers those triggers arm.
    fn go_ai_cheer_speaker_update(
        &mut self,
        go: ObjectGuid,
        state: &mut CheerSpeakerGoAi,
        diff_ms: u32,
    ) {
        // `m_events.Update(diff)` + `ExecuteEvent` (cpp 142, 188).
        let cheer_due = match state.cheer_timer_ms {
            Some(timer) if timer <= diff_ms => {
                state.cheer_timer_ms = None;
                true
            }
            Some(timer) => {
                state.cheer_timer_ms = Some(timer - diff_ms);
                false
            }
            None => false,
        };
        let fire_due = match state.fire_timer_ms {
            Some(timer) if timer <= diff_ms => {
                state.fire_timer_ms = None;
                true
            }
            Some(timer) => {
                state.fire_timer_ms = Some(timer - diff_ms);
                false
            }
            None => false,
        };
        if cheer_due {
            // `EVENT_CHEER` (cpp 192-196): `me->PlayDistanceSound(CheerPicker())`.
            let sound = self.go_ai_cheer_speaker_cheer_pick();
            self.play_direct_sound(go, sound, None);
        }
        if fire_due {
            // `EVENT_FIRE` (cpp 197-228): the zone-table pick plus
            // `SummonGameObject(entry, rndpos..., 1)` with instant
            // `SetRespawnTime(0)/Delete` (cpp 199-222). There is no
            // gameobject-summon helper on `Map` (NEEDS-MAP-HELPER, see
            // unresolved); the entry pick below is the transcribable head.
            let _firework_entry = if state.big {
                self.go_ai_cheer_speaker_fireworks_big_pick()
            } else {
                self.go_ai_cheer_speaker_fireworks_pick()
            };
            // Re-arm while the show runs (cpp 224-225):
            // `ScheduleEvent(EVENT_FIRE, Seconds(1), Seconds(2))`.
            if state.started {
                state.fire_timer_ms = Some(self.spell_rng().irand(1_000, 2_000) as u32);
            }
        }
    }

    /// `CheerPicker` (cpp 122-134): `urand(0, 3)` over the four sounds.
    fn go_ai_cheer_speaker_cheer_pick(&mut self) -> u32 {
        match self.spell_rng().irand(0, 3) {
            0 => CHEER_SOUND_CHEER_1,
            1 => CHEER_SOUND_CHEER_2,
            2 => CHEER_SOUND_CHEER_3,
            _ => CHEER_SOUND_CHEER_4,
        }
    }

    /// `FireworksPicker` (cpp 136): `SelectRandomContainerElement(fireworkIds)`.
    fn go_ai_cheer_speaker_fireworks_pick(&mut self) -> u32 {
        let bound = 22;
        let index = self.spell_rng().irand(0, bound) as usize;
        CHEER_FIREWORK_IDS
            .get(index)
            .copied()
            .unwrap_or(CHEER_FIREWORK_IDS[0])
    }

    /// `FireworksBIGOnlyPicker` (cpp 138).
    fn go_ai_cheer_speaker_fireworks_big_pick(&mut self) -> u32 {
        let bound = 11;
        let index = self.spell_rng().irand(0, bound) as usize;
        CHEER_FIREWORK_BIG_IDS
            .get(index)
            .copied()
            .unwrap_or(CHEER_FIREWORK_BIG_IDS[0])
    }

    /// `GoCircle` ctor (`scourge_invasion.cpp:259`): `me->CastSpell(me,
    /// SPELL_CREATE_CRYSTAL, true)`. A cast by the gameobject itself has no
    /// GO-caster helper on `Map` (NEEDS-MAP-HELPER, see unresolved); this is
    /// the `go_ai_create` docking point.
    #[allow(clippy::unused_self)]
    fn go_ai_scourge_invasion_go_circle_create(&mut self, go: ObjectGuid) {
        let _ = go;
    }

    /// `GoNecropolis` ctor (`scourge_invasion.cpp:270-274`):
    /// `SetActiveObjectState(true)` + `SetVisibilityModifier(3000.0f)`.
    /// Neither has a `Map` equivalent (NEEDS-MAP-HELPER, see unresolved);
    /// this is the `go_ai_create` docking point.
    #[allow(clippy::unused_self)]
    fn go_ai_scourge_invasion_go_necropolis_create(&mut self, go: ObjectGuid) {
        let _ = go;
    }
}
