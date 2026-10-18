//! DB script engine data types for the Rust `mangosd`
//! (Phase 4 Slice 6b): the `ScriptInfo` command rows
//! (`creature_ai_scripts`, `generic_scripts`), the
//! `EventAI` event definitions (`creature_ai_events`),
//! the Conditions rows, and the two text sources
//! (`broadcast_text`, `script_texts`). Pure data — the
//! interpreter lives in `mangos-map` (it needs units,
//! visibility and movement), loaders live in
//! `mangos-world-data`.

use std::collections::HashMap;

/// `ScriptCommands` ids (ScriptCommands.h). Only the
/// implemented subset is named; unknown ids are logged
/// and skipped by the executor.
pub mod command {
    pub const TALK: u32 = 0;
    pub const EMOTE: u32 = 1;
    pub const FIELD_SET: u32 = 2;
    pub const MOVE_TO: u32 = 3;
    pub const MODIFY_FLAGS: u32 = 4;
    pub const INTERRUPT_CASTS: u32 = 5;
    pub const TELEPORT_TO: u32 = 6;
    pub const RESPAWN_GAMEOBJECT: u32 = 9;
    pub const TEMP_SUMMON: u32 = 10;
    pub const OPEN_DOOR: u32 = 11;
    pub const CLOSE_DOOR: u32 = 12;
    pub const ACTIVATE_OBJECT: u32 = 13;
    pub const REMOVE_AURA: u32 = 14;
    pub const CAST_SPELL: u32 = 15;
    pub const PLAY_SOUND: u32 = 16;
    pub const DESPAWN_CREATURE: u32 = 18;
    pub const SET_EQUIPMENT: u32 = 19;
    pub const MOVEMENT: u32 = 20;
    pub const SET_ACTIVEOBJECT: u32 = 21;
    pub const SET_FACTION: u32 = 22;
    pub const MORPH_TO_ENTRY_OR_MODEL: u32 = 23;
    pub const MOUNT_TO_ENTRY_OR_MODEL: u32 = 24;
    pub const SET_RUN: u32 = 25;
    pub const ATTACK_START: u32 = 26;
    pub const UPDATE_ENTRY: u32 = 27;
    pub const STAND_STATE: u32 = 28;
    pub const MODIFY_THREAT: u32 = 29;
    pub const TERMINATE_SCRIPT: u32 = 31;
    pub const TERMINATE_CONDITION: u32 = 32;
    pub const ENTER_EVADE_MODE: u32 = 33;
    pub const SET_HOME_POSITION: u32 = 34;
    pub const TURN_TO: u32 = 35;
    pub const SET_INST_DATA: u32 = 37;
    pub const SET_INST_DATA64: u32 = 38;
    pub const START_SCRIPT: u32 = 39;
    pub const REMOVE_OBJECT: u32 = 41;
    pub const SET_MELEE_ATTACK: u32 = 42;
    pub const SET_COMBAT_MOVEMENT: u32 = 43;
    pub const SET_PHASE: u32 = 44;
    pub const SET_PHASE_RANDOM: u32 = 45;
    pub const SET_PHASE_RANGE: u32 = 46;
    pub const FLEE: u32 = 47;
    pub const DEAL_DAMAGE: u32 = 48;
    pub const ZONE_COMBAT_PULSE: u32 = 49;
    pub const CALL_FOR_HELP: u32 = 50;
    pub const SET_SHEATH: u32 = 51;
    pub const INVINCIBILITY: u32 = 52;
    pub const SET_SERVER_VARIABLE: u32 = 54;
    pub const CREATURE_SPELLS: u32 = 55;
    pub const ADD_SPELL_COOLDOWN: u32 = 57;
    pub const REMOVE_SPELL_COOLDOWN: u32 = 58;
    pub const SET_REACT_STATE: u32 = 59;
    pub const START_WAYPOINTS: u32 = 60;
    pub const SET_DEFAULT_MOVEMENT: u32 = 67;
    pub const RESPAWN_CREATURE: u32 = 71;
    pub const ASSIST_UNIT: u32 = 72;
    pub const COMBAT_STOP: u32 = 73;
    pub const ADD_AURA: u32 = 74;
    pub const ADD_THREAT: u32 = 75;
    pub const SET_GO_STATE: u32 = 80;
    pub const DESPAWN_GAMEOBJECT: u32 = 81;
    pub const LOAD_GAMEOBJECT_SPAWN: u32 = 82;
    pub const SET_GOSSIP_MENU: u32 = 84;
    pub const SEND_SCRIPT_EVENT: u32 = 85;
    pub const SET_PVP: u32 = 86;
    pub const RESET_DOOR_OR_BUTTON: u32 = 87;
    pub const SET_COMMAND_STATE: u32 = 88;
    pub const PLAY_CUSTOM_ANIM: u32 = 89;
    pub const LOAD_CREATURE_SPAWN: u32 = 91;
    /// Loader-disabled action (`SCRIPT_COMMAND_DISABLED`).
    pub const DISABLED: u32 = 9999;
}

/// `eDataFlags` (the `data_flags` column, `raw.data[4]`).
pub const SF_GENERAL_SWAP_INITIAL_TARGETS: u32 = 0x01;
pub const SF_GENERAL_SWAP_FINAL_TARGETS: u32 = 0x02;
pub const SF_GENERAL_TARGET_SELF: u32 = 0x04;
pub const SF_GENERAL_ABORT_ON_FAILURE: u32 = 0x08;
pub const SF_GENERAL_SKIP_MISSING_TARGETS: u32 = 0x10;
/// `ALL_DB_SCRIPT_FLAGS` (loader rejects other bits).
pub const ALL_DB_SCRIPT_FLAGS: u32 = SF_GENERAL_SWAP_INITIAL_TARGETS
    | SF_GENERAL_SWAP_FINAL_TARGETS
    | SF_GENERAL_TARGET_SELF
    | SF_GENERAL_ABORT_ON_FAILURE
    | SF_GENERAL_SKIP_MISSING_TARGETS;

/// One DB script step (`ScriptInfo`): the raw union is
/// kept as positional arrays exactly as loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptInfo {
    pub id: u32,
    /// Ordering key in the script map; for scheduled
    /// (generic) scripts also the delay in seconds.
    pub delay: u32,
    pub command: u32,
    pub condition_id: u32,
    /// `datalong..datalong4`.
    pub datalong: [u32; 4],
    /// `data_flags` (`SF_GENERAL_*`).
    pub data_flags: u32,
    /// `dataint..dataint4`.
    pub dataint: [i32; 4],
    /// `ScriptTarget` value.
    pub target_type: u8,
    pub target_param1: u32,
    pub target_param2: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub o: f32,
}

impl ScriptInfo {
    /// `ShouldAbortScript`: abort flag means failures
    /// terminate the running script.
    #[must_use]
    pub fn should_abort(&self) -> bool {
        self.data_flags & SF_GENERAL_ABORT_ON_FAILURE != 0
    }
}

/// Steps of one script id in (delay, priority) order
/// (`ScriptMap` iteration order).
pub type ScriptSteps = Vec<ScriptInfo>;
/// Script id -> steps (`ScriptMapMap`).
pub type ScriptMapMap = HashMap<u32, ScriptSteps>;

/// `EventAI_Type` values (CreatureEventAI.h).
pub mod event_type {
    pub const TIMER_IN_COMBAT: u16 = 0;
    pub const TIMER_OOC: u16 = 1;
    pub const HP: u16 = 2;
    pub const MANA: u16 = 3;
    pub const AGGRO: u16 = 4;
    pub const KILL: u16 = 5;
    pub const DEATH: u16 = 6;
    pub const EVADE: u16 = 7;
    pub const HIT_BY_SPELL: u16 = 8;
    pub const RANGE: u16 = 9;
    pub const OOC_LOS: u16 = 10;
    pub const SPAWNED: u16 = 11;
    pub const TARGET_HP: u16 = 12;
    pub const TARGET_CASTING: u16 = 13;
    pub const FRIENDLY_HP: u16 = 14;
    pub const FRIENDLY_IS_CC: u16 = 15;
    pub const FRIENDLY_MISSING_BUFF: u16 = 16;
    pub const SUMMONED_UNIT: u16 = 17;
    pub const TARGET_MANA: u16 = 18;
    pub const QUEST_ACCEPT: u16 = 19;
    pub const QUEST_COMPLETE: u16 = 20;
    pub const REACHED_HOME: u16 = 21;
    pub const RECEIVE_EMOTE: u16 = 22;
    pub const AURA: u16 = 23;
    pub const TARGET_AURA: u16 = 24;
    pub const SUMMONED_JUST_DIED: u16 = 25;
    pub const SUMMONED_JUST_DESPAWN: u16 = 26;
    pub const MISSING_AURA: u16 = 27;
    pub const TARGET_MISSING_AURA: u16 = 28;
    pub const MOVEMENT_INFORM: u16 = 29;
    pub const LEAVE_COMBAT: u16 = 30;
    pub const SCRIPT: u16 = 31;
    pub const GROUP_MEMBER_DIED: u16 = 32;
    pub const VICTIM_ROOTED: u16 = 33;
    pub const HIT_BY_AURA: u16 = 34;
    pub const STEALTH_ALERT: u16 = 35;
    pub const SPELL_HIT_TARGET: u16 = 36;
    pub const END: u16 = 37;
}

/// `EventFlags`.
pub const EFLAG_REPEATABLE: u8 = 0x01;
pub const EFLAG_RANDOM_ACTION: u8 = 0x02;
pub const EFLAG_NOT_CASTING: u8 = 0x04;
pub const EFLAG_CHECK_RESULT: u8 = 0x08;
pub const EFLAG_DEBUG_ONLY: u8 = 0x10;

/// `MAX_PHASE`.
pub const MAX_PHASE: u32 = 32;
/// `EVENT_UPDATE_TIME`.
pub const EVENT_UPDATE_TIME: u32 = 500;
/// `MAX_ACTIONS`.
pub const MAX_ACTIONS: usize = 3;

/// One `creature_ai_events` row
/// (`CreatureEventAI_Event`, actions kept as script ids
/// resolved against the script store at run time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventAiEvent {
    pub event_id: u32,
    pub creature_id: u32,
    pub condition_id: u32,
    pub event_type: u16,
    pub inverse_phase_mask: u32,
    pub chance: u8,
    pub flags: u8,
    /// `event_param1..4` (raw union; timer values are
    /// milliseconds in this DB).
    pub params: [u32; 4],
    /// `action1_script..action3_script` (0 = none).
    pub action_scripts: [u32; MAX_ACTIONS],
}

/// Creature entry -> its events in load order
/// (`CreatureEventAI_Event_Map`).
pub type EventAiEventMap = HashMap<u32, Vec<EventAiEvent>>;

/// `ConditionType` values used by the evaluator subset.
pub mod condition_type {
    pub const NOT: i32 = -3;
    pub const OR: i32 = -2;
    pub const AND: i32 = -1;
    pub const NONE: i32 = 0;
    pub const AURA: i32 = 1;
    pub const TEAM: i32 = 6;
    pub const QUESTREWARDED: i32 = 8;
    pub const QUESTTAKEN: i32 = 9;
    pub const SAVED_VARIABLE: i32 = 11;
    pub const SOURCE_ENTRY: i32 = 16;
    pub const NEARBY_CREATURE: i32 = 20;
    pub const NEARBY_GAMEOBJECT: i32 = 21;
    pub const QUEST_NONE: i32 = 22;
    pub const WOW_PATCH: i32 = 24;
    pub const IS_PLAYER: i32 = 28;
    pub const MAP_ID: i32 = 33;
    pub const LINE_OF_SIGHT: i32 = 37;
    pub const DISTANCE_TO_TARGET: i32 = 38;
    pub const IS_MOVING: i32 = 39;
    pub const HEALTH_PERCENT: i32 = 41;
    pub const MANA_PERCENT: i32 = 42;
    pub const IS_IN_COMBAT: i32 = 43;
    pub const IS_ALIVE: i32 = 46;
    pub const DB_GUID: i32 = 52;
    pub const LOCAL_TIME: i32 = 53;
    pub const DISTANCE_TO_POSITION: i32 = 54;
}

/// `CONDITION_FLAG_*`.
pub const CONDITION_FLAG_REVERSE_RESULT: u32 = 0x1;
pub const CONDITION_FLAG_SWAP_TARGETS: u32 = 0x2;

/// One `conditions` row (`ConditionEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition {
    pub entry: u32,
    pub cond_type: i32,
    pub values: [i32; 4],
    pub flags: u32,
}

/// Condition id -> row (`sConditionStorage`).
pub type ConditionMap = HashMap<u32, Condition>;

/// One `broadcast_text` row (`BroadcastText`, enUS
/// only — locale joins are out of scope, as in Slice 2).
#[derive(Clone, Debug, PartialEq)]
pub struct BroadcastText {
    pub entry: u32,
    pub male_text: String,
    pub female_text: String,
    pub chat_type: u32,
    pub sound_id: u32,
    pub language_id: u32,
    pub emote_ids: [u32; 3],
    pub emote_delays: [u32; 3],
}

/// Broadcast text id -> row.
pub type BroadcastTextMap = HashMap<u32, BroadcastText>;

/// One `script_texts` row (`StringTextData` + the
/// enUS content from `script_texts.content_default`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptText {
    /// Negative entry id.
    pub entry: i32,
    pub content: String,
    pub sound_id: u32,
    pub chat_type: u32,
    pub language: u32,
    pub emote: u32,
}

/// Negative text id -> row.
pub type ScriptTextMap = HashMap<i32, ScriptText>;

/// `ChatType` (CreatureDefines.h).
pub mod chat_type {
    pub const SAY: u32 = 0;
    pub const YELL: u32 = 1;
    pub const TEXT_EMOTE: u32 = 2;
    pub const BOSS_EMOTE: u32 = 3;
    pub const WHISPER: u32 = 4;
    pub const BOSS_WHISPER: u32 = 5;
    pub const ZONE_YELL: u32 = 6;
    pub const ZONE_EMOTE: u32 = 7;
}

/// The loaded script-engine stores (boot product,
/// shared read-only by every map).
#[derive(Clone, Debug, Default)]
pub struct ScriptStores {
    pub eventai_events: EventAiEventMap,
    pub creature_ai_scripts: ScriptMapMap,
    pub generic_scripts: ScriptMapMap,
    pub creature_movement_scripts: ScriptMapMap,
    pub creature_spells_scripts: ScriptMapMap,
    pub conditions: ConditionMap,
    pub broadcast_texts: BroadcastTextMap,
    pub script_texts: ScriptTextMap,
}

impl ScriptStores {
    /// Total `EventAI` events across creatures.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.eventai_events.values().map(Vec::len).sum()
    }

    /// Total creature AI script steps.
    #[must_use]
    pub fn creature_script_steps(&self) -> usize {
        self.creature_ai_scripts.values().map(Vec::len).sum()
    }

    /// Total generic script steps.
    #[must_use]
    pub fn generic_script_steps(&self) -> usize {
        self.generic_scripts.values().map(Vec::len).sum()
    }
}

/// `ScriptTarget` values (ScriptCommands.h).
pub mod script_target {
    pub const PROVIDED_TARGET: u8 = 0;
    pub const HOSTILE: u8 = 1;
    pub const HOSTILE_SECOND_AGGRO: u8 = 2;
    pub const HOSTILE_LAST_AGGRO: u8 = 3;
    pub const HOSTILE_RANDOM: u8 = 4;
    pub const HOSTILE_RANDOM_NOT_TOP: u8 = 5;
    pub const HOSTILE_NEAREST: u8 = 6;
    pub const HOSTILE_FARTHEST: u8 = 7;
    pub const OWNER_OR_SELF: u8 = 8;
    pub const OWNER: u8 = 9;
    pub const NEAREST_CREATURE_WITH_ENTRY: u8 = 10;
    pub const CREATURE_WITH_GUID: u8 = 11;
    pub const CREATURE_FROM_INSTANCE_DATA: u8 = 12;
    pub const NEAREST_GAMEOBJECT_WITH_ENTRY: u8 = 13;
    pub const GAMEOBJECT_WITH_GUID: u8 = 14;
    pub const GAMEOBJECT_FROM_INSTANCE_DATA: u8 = 15;
    pub const FRIENDLY: u8 = 16;
    pub const FRIENDLY_INJURED: u8 = 17;
    pub const FRIENDLY_INJURED_EXCEPT: u8 = 18;
    pub const FRIENDLY_MISSING_BUFF: u8 = 19;
    pub const FRIENDLY_MISSING_BUFF_EXCEPT: u8 = 20;
    pub const FRIENDLY_CC: u8 = 21;
    pub const MAP_EVENT_SOURCE: u8 = 22;
    pub const MAP_EVENT_TARGET: u8 = 23;
    pub const MAP_EVENT_EXTRA_TARGET: u8 = 24;
    pub const NEAREST_PLAYER: u8 = 25;
    pub const NEAREST_HOSTILE_PLAYER: u8 = 26;
    pub const NEAREST_FRIENDLY_PLAYER: u8 = 27;
    pub const RANDOM_CREATURE_WITH_ENTRY: u8 = 28;
    pub const RANDOM_GAMEOBJECT_WITH_ENTRY: u8 = 29;
}
