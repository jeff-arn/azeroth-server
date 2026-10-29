//! Alterac Valley rules (`BattleGroundAV.cpp`, Tier
//! 3 Slice 3 step 5). Reinforcement score (600 points,
//! displayed but the win condition is removed in the
//! fork), fifteen grave/tower nodes, two mines,
//! boss/captain kill flow, the Nostalrius challenge
//! counters and the AV quest counters (their script
//! consumers live in `battleground_alterac.cpp`,
//! ported in the content half of this step).

use mangos_entities::creature::SpawnRng;
use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::battlegrounds::WorldSafeLoc;

use crate::battlegrounds::{
    BG_EVENT_DOOR, BG_EVENT_NONE_VAL, BgKind, bg_status, score_type, spawn_mode, team_index,
};
use crate::bg_ab::lang as ab_lang;
use crate::bg_ws::BG_EVENT_GHOST_GATE;
use crate::map::{Map, MapEvent};

/// Reputation values (non-holiday / holiday).
pub(crate) const REP_BOSS: u32 = 350;
pub(crate) const REP_BOSS_HOLIDAY: u32 = 525;
pub(crate) const REP_CAPTAIN: u32 = 125;
pub(crate) const REP_CAPTAIN_HOLIDAY: u32 = 185;
pub(crate) const REP_COMMANDER: u32 = 12;
pub(crate) const REP_COMMANDER_HOLIDAY: u32 = 18;
pub(crate) const REP_TOWER: u32 = 12;
pub(crate) const REP_TOWER_HOLIDAY: u32 = 18;
pub(crate) const REP_SURVIVING_TOWER: u32 = 12;
pub(crate) const REP_SURVIVING_TOWER_HOLIDAY: u32 = 18;
pub(crate) const REP_SURVIVING_CAPTAIN: u32 = 125;
pub(crate) const REP_SURVIVING_CAPTAIN_HOLIDAY: u32 = 175;
pub(crate) const REP_OWNED_GRAVE: u32 = 12;
pub(crate) const REP_OWNED_GRAVE_HOLIDAY: u32 = 18;
pub(crate) const REP_OWNED_MINE: u32 = 24;
pub(crate) const REP_OWNED_MINE_HOLIDAY: u32 = 36;

/// `BG_AV_BOSS_KILL_QUEST_SPELL`.
pub(crate) const SPELL_BOSS_KILL_QUEST: u32 = 23658;
/// `BG_AV_CAPTIME` (5 minutes).
pub(crate) const AV_CAPTIME: u32 = 300_000;
/// `BG_AV_SNOWFALL_FIRSTCAP`.
pub(crate) const AV_SNOWFALL_FIRSTCAP: u32 = 300_000;
/// `BG_AV_SCORE_INITIAL_POINTS`.
pub(crate) const AV_SCORE_INITIAL_POINTS: i32 = 600;
/// `BG_AV_SCORE_NEAR_LOSE`.
pub(crate) const AV_SCORE_NEAR_LOSE: i32 = 120;

/// Kill honor values (`GetBonusHonorFromKill` args).
pub(crate) const KILL_BOSS: u32 = 6;
pub(crate) const KILL_CAPTAIN: u32 = 3;
pub(crate) const KILL_COMMANDER: u32 = 1;
pub(crate) const KILL_TOWER: u32 = 2;
pub(crate) const KILL_SURVIVING_TOWER: u32 = 3;
pub(crate) const KILL_SURVIVING_CAPTAIN: u32 = 3;
pub(crate) const KILL_SURVIVING_GRAVE: u32 = 1;
pub(crate) const KILL_SURVIVING_MINE: u32 = 1;

/// Resource losses (`BG_AV_RES_*`).
pub(crate) const RES_CAPTAIN: i32 = 100;
pub(crate) const RES_TOWER: i32 = 75;

/// `BG_AV_FACTION_A` / `BG_AV_FACTION_H`.
pub(crate) const FACTION_ALLIANCE: u32 = 730;
pub(crate) const FACTION_HORDE: u32 = 729;

/// `BG_AV_MINE_TICK_TIMER` / `BG_AV_MINE_RECLAIM_TIMER`.
pub(crate) const MINE_TICK_TIMER: i32 = 45_000;
pub(crate) const MINE_RECLAIM_TIMER: u32 = 1_200_000;

/// Node indexes (`enum BG_AV_Nodes`).
pub(crate) const NODE_FIRSTAID_STATION: u8 = 0;
pub(crate) const NODE_STORMPIKE_GRAVE: u8 = 1;
pub(crate) const NODE_STONEHEART_GRAVE: u8 = 2;
pub(crate) const NODE_SNOWFALL_GRAVE: u8 = 3;
pub(crate) const NODE_ICEBLOOD_GRAVE: u8 = 4;
pub(crate) const NODE_FROSTWOLF_GRAVE: u8 = 5;
pub(crate) const NODE_FROSTWOLF_HUT: u8 = 6;
pub(crate) const NODE_DUNBALDAR_SOUTH: u8 = 7;
pub(crate) const NODE_DUNBALDAR_NORTH: u8 = 8;
pub(crate) const NODE_ICEWING_BUNKER: u8 = 9;
pub(crate) const NODE_STONEHEART_BUNKER: u8 = 10;
pub(crate) const NODE_ICEBLOOD_TOWER: u8 = 11;
pub(crate) const NODE_TOWER_POINT: u8 = 12;
pub(crate) const NODE_FROSTWOLF_ETOWER: u8 = 13;
pub(crate) const NODE_FROSTWOLF_WTOWER: u8 = 14;

/// `BG_AV_NODES_MAX` / `BG_AV_TOWERS_MAX` /
/// `BG_AV_MAX_STATES` / `BG_AV_MAX_MINES`.
pub(crate) const AV_NODES_MAX: usize = 15;
pub(crate) const AV_TOWERS_MAX: u8 = 23;
pub(crate) const AV_MAX_STATES: u8 = 2;
pub(crate) const AV_MAX_MINES: usize = 2;
pub(crate) const MINE_NORTH: u8 = 0;
pub(crate) const MINE_SOUTH: u8 = 1;

/// `enum BG_AV_States`.
pub(crate) const POINT_ASSAULTED: u8 = 0;
pub(crate) const POINT_CONTROLLED: u8 = 1;

/// `BG_AV_TEAM_NEUTRAL`.
pub(crate) const AV_TEAM_NEUTRAL: u8 = 2;

/// Event ids (`enum BG_AV_Events`, engine-used subset).
pub(crate) const EV_MINE_BOSSES: u8 = 46;
pub(crate) const EV_MINE_BOSSES_NORTH: u8 = 46;
pub(crate) const EV_MINE_BOSSES_SOUTH: u8 = 47;
pub(crate) const EV_CAPTAIN_A: u8 = 48;
pub(crate) const EV_CAPTAIN_H: u8 = 49;
pub(crate) const EV_MINE_EVENT: u8 = 50;
pub(crate) const EV_COMMANDER_A_MORTIMER: u8 = 52;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const EV_COMMANDER_A_RANDOLPH: u8 = 53;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const EV_COMMANDER_A_DUFFY: u8 = 54;
pub(crate) const EV_COMMANDER_A_KARLPHILIPS: u8 = 55;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const EV_COMMANDER_H_DARDOSH: u8 = 56;
pub(crate) const EV_COMMANDER_H_LOUISPHILIPS: u8 = 57;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const EV_COMMANDER_H_MULFORT: u8 = 58;
pub(crate) const EV_COMMANDER_H_MALGOR: u8 = 59;
pub(crate) const EV_HERALD: u8 = 60;
pub(crate) const EV_BOSS_A: u8 = 61;
pub(crate) const EV_BOSS_H: u8 = 62;
pub(crate) const EV_CAPTAIN_DEAD_A: u8 = 63;
pub(crate) const EV_CAPTAIN_DEAD_H: u8 = 64;
pub(crate) const EV_SNIVVLE: u8 = 65;
pub(crate) const EV_EXPLOSIVES_EXPERT_A: u8 = 66;
pub(crate) const EV_EXPLOSIVES_EXPERT_H: u8 = 67;
pub(crate) const EV_LIEUTENANT_A: u8 = 68;
pub(crate) const EV_LIEUTENANT_H: u8 = 69;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const EV_LANDMINES_ALLIANCE: u8 = 101;

/// `BG_AV_GraveyardIds` (nodes 0..6, main A 7,
/// main H 8).
pub(crate) const AV_GRAVEYARD_IDS: [u32; 9] = [751, 689, 729, 169, 749, 690, 750, 611, 610];

/// `BG_AV_MineWorldStates[mine][team]`
/// (alliance/horde/neutral control).
pub(crate) const AV_MINE_WORLD_STATES: [[u32; 3]; 2] = [[1358, 1359, 1360], [1355, 1356, 1357]];

/// `BG_AV_NodeWorldStates[node][GetWorldStateType]`.
#[allow(clippy::unreadable_literal)]
pub(crate) const AV_NODE_WORLD_STATES: [[u32; 4]; 15] = [
    [1326, 1325, 1328, 1327],
    [1335, 1333, 1336, 1334],
    [1304, 1302, 1303, 1301],
    [1343, 1341, 1344, 1342],
    [1348, 1346, 1349, 1347],
    [1339, 1337, 1340, 1338],
    [1331, 1329, 1332, 1330],
    [1375, 1361, 1378, 1370],
    [1374, 1362, 1379, 1371],
    [1376, 1363, 1380, 1372],
    [1377, 1364, 1381, 1373],
    [1390, 1368, 1395, 1385],
    [1389, 1367, 1394, 1384],
    [1388, 1366, 1393, 1383],
    [1387, 1365, 1392, 1382],
];

/// Score display world states.
pub(crate) const WS_ALLIANCE_SCORE: u32 = 3127;
pub(crate) const WS_HORDE_SCORE: u32 = 3128;
pub(crate) const WS_SHOW_H_SCORE: u32 = 3133;
pub(crate) const WS_SHOW_A_SCORE: u32 = 3134;
/// `AV_SNOWFALL_N`.
pub(crate) const WS_SNOWFALL_NEUTRAL: u32 = 1966;

/// `enum BG_AV_Sounds`.
pub(crate) mod av_sound {
    pub const ALLIANCE_ASSAULTS: u32 = 8212;
    pub const HORDE_ASSAULTS: u32 = 8174;
    pub const ALLIANCE_GOOD: u32 = 8173;
    pub const HORDE_GOOD: u32 = 8213;
    pub const BOTH_TOWER_DEFEND: u32 = 8192;
}

/// `CHAT_MSG_MONSTER_SAY` / `_YELL`.
pub(crate) const CHAT_MSG_MONSTER_SAY: u8 = 0x0B;
pub(crate) const CHAT_MSG_MONSTER_YELL: u8 = 0x0D;

/// `LANG_BG_AV_*` `mangos_string` ids.
pub(crate) mod lang {
    pub const TOWER_TAKEN: i32 = 759;
    pub const TOWER_ASSAULTED: i32 = 760;
    pub const TOWER_DEFENDED: i32 = 761;
    pub const GRAVE_TAKEN: i32 = 762;
    pub const GRAVE_DEFENDED: i32 = 763;
    pub const GRAVE_ASSAULTED: i32 = 764;
    pub const MINE_TAKEN: i32 = 765;
    pub const MINE_NORTH: i32 = 766;
    pub const MINE_SOUTH: i32 = 767;
    pub const NODE_GRAVE_STORM_AID: i32 = 768;
    pub const NODE_TOWER_DUN_S: i32 = 769;
    pub const NODE_TOWER_DUN_N: i32 = 770;
    pub const NODE_GRAVE_STORMPIKE: i32 = 771;
    pub const NODE_TOWER_ICEWING: i32 = 772;
    pub const NODE_GRAVE_STONE: i32 = 773;
    pub const NODE_TOWER_STONE: i32 = 774;
    pub const NODE_GRAVE_SNOW: i32 = 775;
    pub const NODE_TOWER_ICE: i32 = 776;
    pub const NODE_GRAVE_ICE: i32 = 777;
    pub const NODE_TOWER_POINT: i32 = 778;
    pub const NODE_GRAVE_FROST: i32 = 779;
    pub const NODE_TOWER_FROST_E: i32 = 780;
    pub const NODE_TOWER_FROST_W: i32 = 781;
    pub const NODE_GRAVE_FROST_HUT: i32 = 782;
    pub const H_GENERAL_DEAD: i32 = 789;
    pub const A_GENERAL_DEAD: i32 = 790;
    /// Herald/captain yells hardcoded in `Update`.
    pub const SNIVVLE_YELL: i32 = 791;
    pub const CAPTAIN_H_YELL: i32 = 792;
    pub const CAPTAIN_A_YELL: i32 = 793;
}

/// Challenge indexes (`enum BG_AV_CHALLENGE`).
pub(crate) mod challenge {
    pub const SOLDIER_AIR_ASSAULT: usize = 0;
    pub const LIEUTENANT_AIR_ASSAULT: usize = 1;
    pub const COMMANDER_AIR_ASSAULT: usize = 2;
    pub const HIDE_CAVALRY_ASSAULT: usize = 3;
    pub const TAMED_CAVALRY_ASSAULT: usize = 4;
    pub const IRONDEEP_GROUND_ASSAULT: usize = 5;
    pub const COLDTOOTH_GROUND_ASSAULT: usize = 6;
    pub const BLOOD_WORLDBOSS_ASSAULT: usize = 7;
    pub const NB_CHALLENGES: usize = 8;
}

/// Assault categories (`enum BG_AV_ASSAULT`, incl.
/// the fork's unused `BG_AV_WRONG_VALUE` slot so
/// `NB_ASSAULTS` is 10).
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) mod assault {
    pub const AIR_BEACON_SOLDIER: usize = 0;
    pub const AIR_BEACON_LIEUTENANT: usize = 1;
    pub const AIR_BEACON_COMMANDER: usize = 2;
    pub const AIR_GLOBAL_SOLDIER: usize = 3;
    pub const AIR_GLOBAL_LIEUTENANT: usize = 4;
    pub const AIR_GLOBAL_COMMANDER: usize = 5;
    pub const CAVALRY_ASSAULT: usize = 6;
    pub const GROUND_ASSAULT: usize = 7;
    pub const WORLDBOSS_ASSAULT: usize = 8;
    pub const NB_ASSAULTS: usize = 10;
}

/// Supply/tamed event bases.
pub(crate) const EV_100_SUPPLIES: u8 = 80;
pub(crate) const EV_200_SUPPLIES: u8 = 82;
pub(crate) const EV_300_SUPPLIES: u8 = 84;
pub(crate) const EV_400_SUPPLIES: u8 = 86;
pub(crate) const EV_05_TAMED: u8 = 90;
pub(crate) const EV_10_TAMED: u8 = 92;
pub(crate) const EV_15_TAMED: u8 = 94;
pub(crate) const EV_20_TAMED: u8 = 96;

/// `REP_NEUTRAL` / `REP_HONORED` / `REP_REVERED`.
pub(crate) const REP_NEUTRAL: u32 = 3;
pub(crate) const REP_HONORED: u32 = 5;
pub(crate) const REP_REVERED: u32 = 6;

/// Shredder charms (`CheckSpellCast`).
pub(crate) const SPELL_SUMMON_SHREDDER_H2: u32 = 21544;
pub(crate) const SPELL_SUMMON_SHREDDER_A2: u32 = 21565;
pub(crate) const NPC_SHREDDER_A2: u32 = 13416;
pub(crate) const NPC_SHREDDER_H2: u32 = 13378;
pub(crate) const NPC_LANDMINES_LAYER_A2: u32 = 13356;
pub(crate) const NPC_LANDMINES_LAYER_H2: u32 = 13357;
pub(crate) const NPC_LANDMINES_EXPERT_A2: u32 = 13598;
pub(crate) const NPC_LANDMINES_EXPERT_H2: u32 = 13597;

/// `BG_AV_OBJECTID_MINE_N` / `_S`.
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const MINE_N_OBJECT: u32 = 178_785;
#[allow(dead_code)] // consumed by battleground_alterac content
pub(crate) const MINE_S_OBJECT: u32 = 178_784;

/// Captain buffs cast by the update loop.
pub(crate) const SPELL_CAPTAIN_BUFF_HORDE: u32 = 22751;
pub(crate) const SPELL_CAPTAIN_BUFF_ALLIANCE: u32 = 23693;

/// `BG_AV_NodeInfos`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AvNodeInfo {
    pub total_owner: u8,
    pub owner: u8,
    pub prev_owner: u8,
    pub prev_other_owner: u8,
    pub state: u8,
    pub prev_state: u8,
    pub timer: u32,
    pub tower: bool,
}

/// `BattleGroundAV` members.
#[derive(Clone, Debug, Default)]
pub struct AvState {
    /// `m_teamQuestStatus` (9 quest counters/team).
    pub team_quest_status: [[u32; 9]; 2],
    /// `m_nodes`.
    pub nodes: [AvNodeInfo; 15],
    /// `m_mineOwner` / `m_minePrevOwner`.
    pub mine_owner: [u8; 2],
    pub mine_prev_owner: [u8; 2],
    /// `m_mineTimer` (i32).
    pub mine_timer: [i32; 2],
    /// `m_mineReclaimTimer`.
    pub mine_reclaim_timer: [u32; 2],
    /// `m_isInformedNearLose`.
    pub informed_near_lose: [bool; 2],
    /// reputation knobs (0 = Reset not yet applied).
    pub rep_tower_destruction: u32,
    pub rep_commander: u32,
    pub rep_captain: u32,
    pub rep_boss: u32,
    pub rep_owned_grave: u32,
    pub rep_owned_mine: u32,
    pub rep_survive_captain: u32,
    pub rep_survive_tower: u32,
    /// `m_shredderOwners`.
    pub shredder_owners: [ObjectGuid; 2],
    /// Challenge state (consumed by
    /// `battleground_alterac` scripts).
    pub snivvle_done: bool,
    pub snivvle_timer: u32,
    pub buff_timer_horde: u32,
    pub buff_timer_alliance: u32,
    pub reinforcement_level: [u32; 2],
    pub challenge_status: [[u32; challenge::NB_CHALLENGES]; 2],
    pub challenge_goals: [[u32; challenge::NB_CHALLENGES]; 2],
    pub challenge_min_reputation_needed: [u32; assault::NB_ASSAULTS],
    pub challenge_timer_start: [u32; assault::NB_ASSAULTS],
    pub challenge_player_go_status: [[bool; assault::NB_ASSAULTS]; 2],
}

impl Map {
    fn av(&self) -> Option<&AvState> {
        match self.bg.as_ref().map(|b| &b.kind) {
            Some(BgKind::Av(av)) => Some(av),
            _ => None,
        }
    }

    fn av_mut(&mut self) -> Option<&mut AvState> {
        match self.bg.as_mut().map(|b| &mut b.kind) {
            Some(BgKind::Av(av)) => Some(av),
            _ => None,
        }
    }

    fn av_in_progress(&self) -> bool {
        self.bg
            .as_ref()
            .is_some_and(|b| b.status == bg_status::IN_PROGRESS)
    }

    /// `GetWorldStateType`.
    fn av_world_state_type(state: u8, team_idx: u8) -> usize {
        team_idx as usize * AV_MAX_STATES as usize + state as usize
    }

    /// `IsGrave` / `IsTower`.
    fn av_is_grave(node: u8) -> bool {
        (NODE_FIRSTAID_STATION..=NODE_FROSTWOLF_HUT).contains(&node)
    }
    fn av_is_tower(node: u8) -> bool {
        (NODE_DUNBALDAR_SOUTH..=NODE_FROSTWOLF_WTOWER).contains(&node)
    }

    /// `GetNodeName`.
    fn av_node_name(node: u8) -> i32 {
        match node {
            NODE_FIRSTAID_STATION => lang::NODE_GRAVE_STORM_AID,
            NODE_DUNBALDAR_SOUTH => lang::NODE_TOWER_DUN_S,
            NODE_DUNBALDAR_NORTH => lang::NODE_TOWER_DUN_N,
            NODE_STORMPIKE_GRAVE => lang::NODE_GRAVE_STORMPIKE,
            NODE_ICEWING_BUNKER => lang::NODE_TOWER_ICEWING,
            NODE_STONEHEART_GRAVE => lang::NODE_GRAVE_STONE,
            NODE_STONEHEART_BUNKER => lang::NODE_TOWER_STONE,
            NODE_SNOWFALL_GRAVE => lang::NODE_GRAVE_SNOW,
            NODE_ICEBLOOD_TOWER => lang::NODE_TOWER_ICE,
            NODE_ICEBLOOD_GRAVE => lang::NODE_GRAVE_ICE,
            NODE_TOWER_POINT => lang::NODE_TOWER_POINT,
            NODE_FROSTWOLF_GRAVE => lang::NODE_GRAVE_FROST,
            NODE_FROSTWOLF_ETOWER => lang::NODE_TOWER_FROST_E,
            NODE_FROSTWOLF_WTOWER => lang::NODE_TOWER_FROST_W,
            NODE_FROSTWOLF_HUT => lang::NODE_GRAVE_FROST_HUT,
            _ => 0,
        }
    }

    /// `SendYellToAll(entry, LANG_UNIVERSAL, source)`.
    fn av_send_yell_to_all(&mut self, entry: i32, source: Option<ObjectGuid>) {
        self.events.push(MapEvent::BgLangMessage2 {
            entry,
            msg_type: CHAT_MSG_MONSTER_YELL,
            source,
            arg_ids: [0, 0],
        });
    }

    /// `SendYell2ToAll` (herald node/faction args).
    fn av_send_yell2_to_all(
        &mut self,
        entry: i32,
        source: Option<ObjectGuid>,
        arg1: i32,
        arg2: i32,
    ) {
        self.events.push(MapEvent::BgLangMessage2 {
            entry,
            msg_type: CHAT_MSG_MONSTER_YELL,
            source,
            arg_ids: [arg1, arg2],
        });
    }

    /// Raw-text MonsterSay/Yell (`MonsterSay`/
    /// `MonsterYell`/`PMonsterSay`): fork sends in
    /// listen range of the creature; the BG message
    /// harness broadcasts map-wide (recorded).
    fn av_monster_text(&mut self, giver: ObjectGuid, text: &str, yell: bool) {
        if !giver.is_creature() {
            return;
        }
        let msg_type = if yell {
            CHAT_MSG_MONSTER_YELL
        } else {
            CHAT_MSG_MONSTER_SAY
        };
        let packet = self.build_monster_chat(msg_type, text, 0, giver, None);
        self.bg_send_to_all(&packet);
    }

    fn av_herald_guid(&self) -> Option<ObjectGuid> {
        self.bg_single_creature(EV_HERALD, 0)
    }

    /// `uint32(GetBonusHonorFromKill(k) * GetHonorModifier())`.
    fn av_bonus_honor_scaled(&self, kills: u32) -> u32 {
        let Some(bg) = self.bg.as_ref() else {
            return 0;
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let v = (bg.bonus_honor_from_kill(kills) as f32 * bg.honor_modifier()) as u32;
        v
    }

    /// `GetBonusHonorFromKill(k)` (unmodified fork
    /// calls: captains, commanders, surviving
    /// captain).
    fn av_bonus_honor(&self, kills: u32) -> u32 {
        self.bg
            .as_ref()
            .map_or(0, |bg| bg.bonus_honor_from_kill(kills))
    }

    /// Defender upgrade tier from a team's armor
    /// resources (PopulateNode/PopulateMineNode).
    fn av_defender_type(resources: u32, team_idx: u8) -> u32 {
        if team_idx == AV_TEAM_NEUTRAL || resources < 500 {
            0
        } else if resources < 1000 {
            1
        } else if resources < 1500 {
            2
        } else {
            3
        }
    }

    fn av_team_resources(&self, team_idx: u8) -> u32 {
        if team_idx == AV_TEAM_NEUTRAL {
            0
        } else {
            self.av()
                .map_or(0, |av| av.team_quest_status[team_idx as usize][0])
        }
    }

    /// `InitNode`.
    fn av_init_node(&mut self, node: u8, team_idx: u8, tower: bool) {
        let grave = Self::av_is_grave(node);
        if let Some(av) = self.av_mut() {
            let n = &mut av.nodes[node as usize];
            n.total_owner = team_idx;
            n.owner = team_idx;
            n.prev_owner = team_idx;
            n.prev_other_owner = team_idx;
            n.state = POINT_CONTROLLED;
            n.prev_state = POINT_CONTROLLED;
            n.timer = 0;
            n.tower = tower;
        }
        if let Some(bg) = self.bg.as_mut() {
            bg.active_events
                .insert(node, team_idx * AV_MAX_STATES + POINT_CONTROLLED);
            if grave {
                bg.active_events
                    .insert(node + AV_NODES_MAX as u8, team_idx * 4);
            }
            if tower {
                bg.active_events
                    .insert(node + AV_NODES_MAX as u8, team_idx * 2 + 1);
                bg.active_events.insert(node + AV_TOWERS_MAX, team_idx * 4);
            }
        }
    }

    /// `initializeChallengeInvocationGoals`.
    fn av_init_challenge_goals(&mut self) {
        let (buff_h, buff_a) = (
            120_000 + self.rng.urand(0, 4) * 60_000,
            120_000 + self.rng.urand(0, 4) * 60_000,
        );
        // Bench patch is 1.12 >= WOW_PATCH_106.
        let cavalry_rep = REP_HONORED;
        if let Some(av) = self.av_mut() {
            av.snivvle_done = false;
            av.snivvle_timer = 0;
            av.buff_timer_horde = buff_h;
            av.buff_timer_alliance = buff_a;
            av.challenge_status = [[0; challenge::NB_CHALLENGES]; 2];
            av.reinforcement_level = [0; 2]; // AV_NPC_BASIC
            for i in 0..2 {
                av.challenge_goals[i][challenge::SOLDIER_AIR_ASSAULT] = 90;
                av.challenge_goals[i][challenge::LIEUTENANT_AIR_ASSAULT] = 60;
                av.challenge_goals[i][challenge::COMMANDER_AIR_ASSAULT] = 30;
                av.challenge_goals[i][challenge::HIDE_CAVALRY_ASSAULT] = 25;
                av.challenge_goals[i][challenge::TAMED_CAVALRY_ASSAULT] = 25;
                av.challenge_goals[i][challenge::BLOOD_WORLDBOSS_ASSAULT] = 200;
                av.challenge_player_go_status[i] = [false; assault::NB_ASSAULTS];
            }
            av.challenge_goals[0][challenge::IRONDEEP_GROUND_ASSAULT] = 280;
            av.challenge_goals[0][challenge::COLDTOOTH_GROUND_ASSAULT] = 70;
            av.challenge_goals[1][challenge::IRONDEEP_GROUND_ASSAULT] = 70;
            av.challenge_goals[1][challenge::COLDTOOTH_GROUND_ASSAULT] = 280;
            av.challenge_min_reputation_needed = [
                REP_REVERED,
                REP_REVERED,
                REP_REVERED,
                REP_REVERED,
                REP_REVERED,
                REP_REVERED,
                cavalry_rep,
                REP_HONORED,
                REP_NEUTRAL,
                REP_NEUTRAL,
            ];
            av.challenge_timer_start = [
                1_800_000,
                1_800_000,
                1_800_000,
                172_800_000,
                172_800_000,
                172_800_000,
                1_800_000,
                1_800_000,
                172_800_000,
                0,
            ];
        }
    }

    /// Constructor: start messages are seeded by
    /// `BgState::new` for type 1.
    #[allow(clippy::unused_self)]
    pub(crate) fn av_construct(&mut self) {}

    /// `SetupBattleGround` (empty fork body) +
    /// `Reset` tail (once guard on `rep_boss`).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn av_setup(&mut self) -> bool {
        if self.av().is_some_and(|av| av.rep_boss != 0) {
            return true;
        }
        // UNMODELED: `IsBgWeekend` (see bg_ws.rs).
        let is_weekend = false;
        if let Some(av) = self.av_mut() {
            av.rep_tower_destruction = if is_weekend {
                REP_TOWER_HOLIDAY
            } else {
                REP_TOWER
            };
            av.rep_commander = if is_weekend {
                REP_COMMANDER_HOLIDAY
            } else {
                REP_COMMANDER
            };
            av.rep_captain = if is_weekend {
                REP_CAPTAIN_HOLIDAY
            } else {
                REP_CAPTAIN
            };
            av.rep_boss = if is_weekend {
                REP_BOSS_HOLIDAY
            } else {
                REP_BOSS
            };
            av.rep_owned_grave = if is_weekend {
                REP_OWNED_GRAVE_HOLIDAY
            } else {
                REP_OWNED_GRAVE
            };
            av.rep_survive_captain = if is_weekend {
                REP_SURVIVING_CAPTAIN_HOLIDAY
            } else {
                REP_SURVIVING_CAPTAIN
            };
            av.rep_survive_tower = if is_weekend {
                REP_SURVIVING_TOWER_HOLIDAY
            } else {
                REP_SURVIVING_TOWER
            };
            av.rep_owned_mine = if is_weekend {
                REP_OWNED_MINE_HOLIDAY
            } else {
                REP_OWNED_MINE
            };
            av.team_quest_status = [[0; 9]; 2];
            av.informed_near_lose = [false; 2];
            av.snivvle_done = false;
            av.snivvle_timer = 0;
            av.shredder_owners = [ObjectGuid::EMPTY; 2];
            for i in 0..AV_MAX_MINES {
                av.mine_owner[i] = AV_TEAM_NEUTRAL;
                av.mine_prev_owner[i] = AV_TEAM_NEUTRAL;
                av.mine_timer[i] = MINE_TICK_TIMER;
                av.mine_reclaim_timer[i] = MINE_RECLAIM_TIMER;
            }
        }
        if let Some(bg) = self.bg.as_mut() {
            bg.team_scores = [AV_SCORE_INITIAL_POINTS; 2];
            // BG_TEAMS_COUNT loop: captain-dead per
            // team (i over the two real teams; index 2
            // slot of the fork loop has no match id).
            bg.active_events
                .insert(EV_CAPTAIN_DEAD_A, BG_EVENT_NONE_VAL);
            bg.active_events
                .insert(EV_CAPTAIN_DEAD_H, BG_EVENT_NONE_VAL);
            for i in 0..AV_MAX_MINES {
                bg.active_events
                    .insert(EV_MINE_BOSSES + i as u8, AV_TEAM_NEUTRAL);
                bg.active_events
                    .insert(EV_MINE_EVENT + i as u8, AV_TEAM_NEUTRAL);
            }
            for ev in [
                EV_CAPTAIN_A,
                EV_CAPTAIN_H,
                EV_HERALD,
                EV_SNIVVLE,
                EV_BOSS_A,
                EV_BOSS_H,
                EV_EXPLOSIVES_EXPERT_A,
                EV_EXPLOSIVES_EXPERT_H,
                EV_LIEUTENANT_A,
                EV_LIEUTENANT_H,
                BG_EVENT_GHOST_GATE,
            ] {
                bg.active_events.insert(ev, 0);
            }
            bg.active_events.insert(100, 0); // BG_AV_LANDMINES_HORDE
            bg.active_events.insert(101, 0); // BG_AV_LANDMINES_ALLIANCE
            for i in NODE_DUNBALDAR_SOUTH..=NODE_FROSTWOLF_WTOWER {
                bg.active_events
                    .insert(EV_COMMANDER_A_MORTIMER + i - NODE_DUNBALDAR_SOUTH, 0);
            }
        }
        for i in NODE_FIRSTAID_STATION..=NODE_STONEHEART_GRAVE {
            self.av_init_node(i, 0, false);
        }
        for i in NODE_DUNBALDAR_SOUTH..=NODE_STONEHEART_BUNKER {
            self.av_init_node(i, 0, true);
        }
        for i in NODE_ICEBLOOD_GRAVE..=NODE_FROSTWOLF_HUT {
            self.av_init_node(i, 1, false);
        }
        for i in NODE_ICEBLOOD_TOWER..=NODE_FROSTWOLF_WTOWER {
            self.av_init_node(i, 1, true);
        }
        self.av_init_node(NODE_SNOWFALL_GRAVE, AV_TEAM_NEUTRAL, false);
        self.av_init_challenge_goals();
        true
    }

    /// `UpdateScore`.
    #[allow(clippy::cast_sign_loss)]
    pub(crate) fn av_update_score_team(&mut self, team_idx: u8, points: i32) {
        let t = team_idx as usize;
        let Some(bg) = self.bg.as_mut() else { return };
        bg.team_scores[t] += points;
        if points < 0 {
            // Ivina <Nostalrius>: removed message and
            // winning condition.
            if bg.team_scores[t] < 1 {
                bg.team_scores[t] = 0;
            } else if let BgKind::Av(av) = &mut bg.kind {
                if !av.informed_near_lose[t] && bg.team_scores[t] < AV_SCORE_NEAR_LOSE {
                    av.informed_near_lose[t] = true;
                }
            }
        }
        let score = bg.team_scores[t].max(0) as u32;
        let field = if t == 1 {
            WS_HORDE_SCORE
        } else {
            WS_ALLIANCE_SCORE
        };
        self.bg_update_world_state(field, score);
    }

    /// `ResetTamedEvent`.
    pub(crate) fn av_reset_tamed_event(&mut self, team_idx: u8) {
        for base in [EV_05_TAMED, EV_10_TAMED, EV_15_TAMED, EV_20_TAMED] {
            self.bg_spawn_event(base + team_idx, 2, true, false, 0);
        }
    }

    /// `Update` (kind stage; `bg_update_base` runs
    /// after, mirroring `BattleGround::Update(diff)`
    /// at the tail).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn av_update(&mut self, diff_ms: u32) {
        let snivvle_fire = {
            let Some(av) = self.av_mut() else { return };
            av.snivvle_timer = av.snivvle_timer.saturating_add(diff_ms);
            if av.snivvle_timer >= 70_000 && !av.snivvle_done {
                av.snivvle_done = true;
                true
            } else {
                false
            }
        };
        if snivvle_fire {
            let snivvle = self.bg_single_creature(EV_SNIVVLE, 0);
            self.av_send_yell_to_all(lang::SNIVVLE_YELL, snivvle);
        }
        if !self.av_in_progress() {
            return;
        }
        // Captain buffs (roll before the timer borrow).
        let (roll_h, roll_a) = (
            120_000 + self.rng.urand(0, 4) * 60_000,
            120_000 + self.rng.urand(0, 4) * 60_000,
        );
        for (is_horde, roll) in [(true, roll_h), (false, roll_a)] {
            let fire = {
                let Some(av) = self.av_mut() else { return };
                let timer = if is_horde {
                    &mut av.buff_timer_horde
                } else {
                    &mut av.buff_timer_alliance
                };
                if *timer < diff_ms {
                    *timer = roll;
                    true
                } else {
                    *timer -= diff_ms;
                    false
                }
            };
            if fire {
                let (dead_ev, captain_ev, spell, team, yell) = if is_horde {
                    (
                        EV_CAPTAIN_DEAD_H,
                        EV_CAPTAIN_H,
                        SPELL_CAPTAIN_BUFF_HORDE,
                        Team::Horde,
                        lang::CAPTAIN_H_YELL,
                    )
                } else {
                    (
                        EV_CAPTAIN_DEAD_A,
                        EV_CAPTAIN_A,
                        SPELL_CAPTAIN_BUFF_ALLIANCE,
                        Team::Alliance,
                        lang::CAPTAIN_A_YELL,
                    )
                };
                if !self.bg_is_active_event(dead_ev, 0) {
                    self.bg_cast_spell_on_team(spell, team);
                    let captain = self.bg_single_creature(captain_ev, 0);
                    self.av_send_yell_to_all(yell, captain);
                }
            }
        }
        // Starting-event-3 supply/tamed spawns (dead in
        // practice: IN_PROGRESS implies event 3 fired;
        // ported as written).
        let spike_due = self.bg.as_ref().is_some_and(|b| {
            b.start_delay_time_ms <= 30_000
                && b.events & crate::battlegrounds::BG_STARTING_EVENT_3 == 0
        });
        if spike_due {
            for base in [
                EV_100_SUPPLIES,
                EV_200_SUPPLIES,
                EV_300_SUPPLIES,
                EV_400_SUPPLIES,
            ] {
                self.bg_spawn_event(base, 2, true, false, 0);
                self.bg_spawn_event(base + 1, 2, true, false, 0);
            }
            for base in [EV_05_TAMED, EV_10_TAMED, EV_15_TAMED, EV_20_TAMED] {
                self.bg_spawn_event(base, 2, true, false, 0);
                self.bg_spawn_event(base + 1, 2, true, false, 0);
            }
            self.bg_spawn_event(15, 0, false, true, 0);
            self.bg_spawn_event(28, 0, false, true, 0);
        }
        // mines
        for mine in 0..AV_MAX_MINES {
            let (owner, tick, reclaim) = {
                let Some(av) = self.av_mut() else { return };
                if av.mine_owner[mine] == AV_TEAM_NEUTRAL {
                    continue;
                }
                let diff = i32::try_from(diff_ms).unwrap_or(i32::MAX);
                av.mine_timer[mine] -= diff;
                let tick = if av.mine_timer[mine] <= 0 {
                    av.mine_timer[mine] = MINE_TICK_TIMER;
                    true
                } else {
                    false
                };
                let reclaim = if av.mine_reclaim_timer[mine] > diff_ms {
                    av.mine_reclaim_timer[mine] -= diff_ms;
                    false
                } else {
                    true
                };
                (av.mine_owner[mine], tick, reclaim)
            };
            if tick {
                self.av_update_score_team(owner, 1);
            }
            if reclaim {
                self.av_change_mine_owner(mine as u8, AV_TEAM_NEUTRAL);
            }
        }
        // node timers
        for node in 0..AV_NODES_MAX {
            let (state, timer) = {
                let Some(av) = self.av() else { return };
                (av.nodes[node].state, av.nodes[node].timer)
            };
            if state != POINT_ASSAULTED {
                continue;
            }
            if timer > diff_ms {
                if let Some(av) = self.av_mut() {
                    av.nodes[node].timer -= diff_ms;
                }
            } else {
                if node as u8 == NODE_STONEHEART_BUNKER {
                    self.bg_spawn_event(25, 1, true, true, 0);
                }
                self.av_event_player_destroyed_point(node as u8);
            }
        }
    }

    /// `StartingEventCloseDoors` (log only in the fork).
    #[allow(clippy::unused_self)]
    pub(crate) fn av_starting_close_doors(&mut self) {}

    /// `StartingEventOpenDoors`.
    pub(crate) fn av_starting_open_doors(&mut self) {
        self.bg_open_door_event(BG_EVENT_DOOR, 0);
        self.bg_spawn_event(BG_EVENT_GHOST_GATE, 0, false, true, 0);
    }

    /// `AddPlayer` per-kind arm (flat `BgScore`).
    pub(crate) fn av_add_player_pre(&mut self, guid: ObjectGuid) {
        if let Some(bg) = self.bg.as_mut() {
            bg.scores.entry(guid).or_default();
        }
    }

    /// `RemovePlayer` (empty in the fork).
    #[allow(clippy::unused_self)]
    pub(crate) fn av_remove_player(&mut self, _guid: ObjectGuid) {}

    /// `OnEventStateChanged` (no fork override).
    #[allow(clippy::unused_self)]
    pub(crate) fn av_on_event_state_changed(&mut self, _e1: u8, _e2: u8, _activated: bool) {}

    /// `SendMineWorldStates`.
    fn av_send_mine_world_states(&mut self, mine: u8) {
        let Some(av) = self.av() else { return };
        let m = mine as usize;
        let (owner, prev) = (av.mine_owner[m], av.mine_prev_owner[m]);
        self.bg_update_world_state(AV_MINE_WORLD_STATES[m][owner as usize], 1);
        if owner != prev {
            self.bg_update_world_state(AV_MINE_WORLD_STATES[m][prev as usize], 0);
        }
    }

    /// `ChangeMineOwner`.
    fn av_change_mine_owner(&mut self, mine: u8, team_idx: u8) {
        let same = {
            let Some(av) = self.av_mut() else { return };
            av.mine_timer[mine as usize] = MINE_TICK_TIMER;
            av.mine_owner[mine as usize] == team_idx
        };
        if same {
            return;
        }
        if let Some(av) = self.av_mut() {
            av.mine_prev_owner[mine as usize] = av.mine_owner[mine as usize];
            av.mine_owner[mine as usize] = team_idx;
        }
        self.av_send_mine_world_states(mine);
        self.bg_spawn_event(EV_MINE_EVENT + mine, team_idx, true, false, 0);
        self.bg_spawn_event(EV_MINE_BOSSES + mine, team_idx, true, false, 0);
        if team_idx != AV_TEAM_NEUTRAL {
            self.bg_play_sound_to_all(if team_idx == 0 {
                av_sound::ALLIANCE_GOOD
            } else {
                av_sound::HORDE_GOOD
            });
            if let Some(av) = self.av_mut() {
                av.mine_reclaim_timer[mine as usize] = MINE_RECLAIM_TIMER;
            }
            let herald = self.av_herald_guid();
            self.av_send_yell2_to_all(
                lang::MINE_TAKEN,
                herald,
                if team_idx == 0 {
                    ab_lang::BG_ALLY
                } else {
                    ab_lang::BG_HORDE
                },
                if mine == MINE_NORTH {
                    lang::MINE_NORTH
                } else {
                    lang::MINE_SOUTH
                },
            );
        }
        if team_idx == 0 {
            self.av_complete_quest_for_all(7122);
        } else if team_idx == 1 {
            self.av_complete_quest_for_all(7124);
        }
    }

    /// `CompleteQuestForAll` — fork iterates every
    /// player on the map regardless of team. UNMODELED:
    /// `FullQuestComplete` quest-slot counter writes;
    /// status + quest-complete packet match the
    /// `complete_quest` helper contract.
    pub(crate) fn av_complete_quest_for_all(&mut self, quest_id: u32) {
        let targets: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .map(|bg| bg.players.keys().copied().collect())
            .unwrap_or_default();
        for guid in targets {
            let done = self
                .player_mut(guid)
                .is_some_and(|p| p.quest_status(quest_id) == 3 && p.complete_quest(quest_id));
            if done {
                let mut writer = PacketWriter::new();
                writer.write_u32_le(quest_id);
                let mut packet = WorldPacket::new(op::SMSG_QUESTUPDATE_COMPLETE);
                packet.take_writer(writer);
                self.events.push(MapEvent::Packet { to: guid, packet });
            }
        }
    }

    /// `UpdateNodeWorldState`.
    fn av_update_node_world_state(&mut self, node: u8) {
        let Some(cur) = self.av().map(|av| av.nodes[node as usize]) else {
            return;
        };
        let n = node as usize;
        let t = Self::av_world_state_type(cur.state, cur.owner);
        if t < 4 {
            self.bg_update_world_state(AV_NODE_WORLD_STATES[n][t], 1);
        }
        if cur.prev_owner == AV_TEAM_NEUTRAL {
            self.bg_update_world_state(WS_SNOWFALL_NEUTRAL, 0);
        } else {
            let pt = Self::av_world_state_type(cur.prev_state, cur.prev_owner);
            self.bg_update_world_state(AV_NODE_WORLD_STATES[n][pt], 0);
        }
    }

    /// `PopulateMineNode`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    fn av_populate_mine_node(&mut self, mine: u8, team_idx: u8, old_upgrade_advance: u32) {
        let actual_owner = self
            .av()
            .map_or(AV_TEAM_NEUTRAL, |av| av.mine_owner[mine as usize]);
        let mine_defender =
            Self::av_defender_type(self.av_team_resources(actual_owner), actual_owner);
        let old_defender = Self::av_defender_type(old_upgrade_advance, team_idx);
        if team_idx == actual_owner {
            let event2 = (actual_owner as u32 + mine_defender * 3) as u8;
            self.bg_set_spawn_event_mode(EV_MINE_EVENT + mine, event2, spawn_mode::RESPAWN_FORCED);
            self.bg_spawn_event(EV_MINE_EVENT + mine, event2, true, true, 0);
        } else {
            let event2 = (actual_owner as u32 + old_defender * 3) as u8;
            self.bg_set_spawn_event_mode(
                AV_NODES_MAX as u8 + mine,
                event2,
                spawn_mode::RESPAWN_STOP,
            );
        }
    }

    /// `PopulateNode`.
    fn av_populate_node(&mut self, node: u8) {
        let Some(cur) = self.av().map(|av| av.nodes[node as usize]) else {
            return;
        };
        let new_idx = cur.owner;
        let old_idx = cur.prev_other_owner;
        let type_new = Self::av_defender_type(self.av_team_resources(new_idx), new_idx);
        let type_old = Self::av_defender_type(self.av_team_resources(old_idx), old_idx);
        let mut delay = 0u32;
        let nodes_base = AV_NODES_MAX as u8 + node;
        if Self::av_is_grave(node) {
            if cur.state == POINT_CONTROLLED {
                let event2 = (new_idx as u32 * 4 + type_new) as u8;
                self.bg_set_spawn_event_mode(nodes_base, event2, spawn_mode::RESPAWN_FORCED);
                self.bg_spawn_event(nodes_base, event2, true, true, 0);
                delay = 5;
            } else {
                let event2 = (old_idx as u32 * 4 + type_old) as u8;
                self.bg_set_spawn_event_mode(nodes_base, event2, spawn_mode::RESPAWN_STOP);
                delay = 1;
            }
        }
        if Self::av_is_tower(node) && new_idx != AV_TEAM_NEUTRAL {
            if cur.state == POINT_CONTROLLED {
                let base2 = (new_idx as u32 * AV_MAX_STATES as u32 + 1) as u8;
                self.bg_set_spawn_event_mode(nodes_base, base2, spawn_mode::RESPAWN_FORCED);
                self.bg_spawn_event(nodes_base, base2, true, true, 0);
                let tower2 = (new_idx as u32 * 4 + type_new) as u8;
                self.bg_set_spawn_event_mode(
                    AV_TOWERS_MAX + node,
                    tower2,
                    spawn_mode::RESPAWN_FORCED,
                );
                self.bg_spawn_event(AV_TOWERS_MAX + node, tower2, true, true, 0);
                delay = 5;
            } else {
                let base2 = (old_idx as u32 * AV_MAX_STATES as u32 + 1) as u8;
                self.bg_set_spawn_event_mode(nodes_base, base2, spawn_mode::RESPAWN_STOP);
                let tower2 = (old_idx as u32 * 4 + type_old) as u8;
                self.bg_set_spawn_event_mode(
                    AV_TOWERS_MAX + node,
                    tower2,
                    spawn_mode::RESPAWN_STOP,
                );
                delay = 1;
            }
        }
        self.bg_spawn_event(node, new_idx * AV_MAX_STATES + cur.state, true, true, delay);
    }

    /// `AssaultNode`.
    fn av_assault_node(&mut self, node: u8, team_idx: u8) {
        if let Some(av) = self.av_mut() {
            let n = &mut av.nodes[node as usize];
            n.timer = if n.prev_owner == AV_TEAM_NEUTRAL {
                AV_SNOWFALL_FIRSTCAP
            } else {
                AV_CAPTIME
            };
            n.prev_owner = n.owner;
            n.prev_other_owner = n.owner;
            n.owner = team_idx;
            n.prev_state = n.state;
            n.state = POINT_ASSAULTED;
        }
    }

    /// `DefendNode`.
    fn av_defend_node(&mut self, node: u8, team_idx: u8) {
        if let Some(av) = self.av_mut() {
            let n = &mut av.nodes[node as usize];
            n.prev_owner = n.owner;
            n.prev_other_owner = n.owner;
            n.owner = team_idx;
            n.prev_state = n.state;
            n.state = POINT_CONTROLLED;
            n.timer = 0;
        }
    }

    /// `DestroyNode`.
    fn av_destroy_node(&mut self, node: u8) {
        if let Some(av) = self.av_mut() {
            let n = &mut av.nodes[node as usize];
            n.total_owner = n.owner;
            n.prev_owner = n.owner;
            n.prev_state = n.state;
            n.state = POINT_CONTROLLED;
            n.timer = 0;
        }
    }

    /// `EventPlayerDestroyedPoint`.
    fn av_event_player_destroyed_point(&mut self, node: u8) {
        let Some(cur) = self.av().map(|av| av.nodes[node as usize]) else {
            return;
        };
        let owner_idx = cur.owner;
        let owner_team = if owner_idx == 0 {
            Team::Alliance
        } else {
            Team::Horde
        };
        self.av_destroy_node(node);
        self.av_populate_node(node);
        self.av_update_node_world_state(node);
        if Self::av_is_tower(node) {
            let other = 1 - owner_idx;
            self.av_update_score_team(other, -RES_TOWER);
            let faction = if owner_team == Team::Alliance {
                FACTION_ALLIANCE
            } else {
                FACTION_HORDE
            };
            let rep = self.av().map_or(0, |av| av.rep_tower_destruction);
            self.bg_reward_reputation_to_team(faction, i32::try_from(rep).unwrap_or(0), owner_team);
            let honor = self.av_bonus_honor(KILL_TOWER);
            self.bg_reward_honor_to_team(honor, owner_team);
            let herald = self.av_herald_guid();
            self.av_send_yell2_to_all(
                lang::TOWER_TAKEN,
                herald,
                Self::av_node_name(node),
                if owner_team == Team::Alliance {
                    ab_lang::BG_ALLY
                } else {
                    ab_lang::BG_HORDE
                },
            );
        } else {
            let herald = self.av_herald_guid();
            self.av_send_yell2_to_all(
                lang::GRAVE_TAKEN,
                herald,
                Self::av_node_name(node),
                if owner_team == Team::Alliance {
                    ab_lang::BG_ALLY
                } else {
                    ab_lang::BG_HORDE
                },
            );
        }
        if Self::av_is_tower(node) {
            if owner_team == Team::Alliance {
                self.av_complete_quest_for_all(7102);
            }
            if owner_team == Team::Horde {
                self.av_complete_quest_for_all(7101);
            }
        }
        if Self::av_is_grave(node) {
            if owner_team == Team::Alliance {
                self.av_complete_quest_for_all(7081);
            }
            if owner_team == Team::Horde {
                self.av_complete_quest_for_all(7082);
            }
        }
    }

    /// `EventPlayerClickedOnFlag` + assault/defend
    /// splits (fork routes on the GO's own event2).
    pub(crate) fn av_event_clicked_on_flag(&mut self, source: ObjectGuid, go: ObjectGuid) {
        if !self.av_in_progress() {
            return;
        }
        let (event1, event2) = self
            .stores
            .battle_event_indexes
            .gameobject_indexes(go.counter())
            .first()
            .map_or((BG_EVENT_NONE_VAL, 0), |&(e1, e2)| (e1, e2));
        if event1 as usize >= AV_NODES_MAX {
            return;
        }
        let node = event1;
        match event2 % AV_MAX_STATES {
            POINT_CONTROLLED => self.av_event_player_assaults_point(source, node),
            POINT_ASSAULTED => self.av_event_player_defends_point(source, node),
            _ => {}
        }
    }

    /// `EventPlayerAssaultsPoint`.
    fn av_event_player_assaults_point(&mut self, player: ObjectGuid, node: u8) {
        let team = self.player(player).map_or(Team::None, |p| p.team);
        let team_idx = team_index(team) as u8;
        let cur = self
            .av()
            .map(|av| av.nodes[node as usize])
            .unwrap_or_default();
        if cur.owner == team_idx || team_idx == cur.total_owner {
            return;
        }
        self.av_assault_node(node, team_idx);
        self.av_update_node_world_state(node);
        self.av_populate_node(node);
        let herald = self.av_herald_guid();
        let faction = if team_idx == 0 {
            ab_lang::BG_ALLY
        } else {
            ab_lang::BG_HORDE
        };
        if Self::av_is_tower(node) {
            self.av_send_yell2_to_all(
                lang::TOWER_ASSAULTED,
                herald,
                Self::av_node_name(node),
                faction,
            );
            self.bg_update_player_score(player, score_type::TOWERS_ASSAULTED, 1);
        } else {
            self.av_send_yell2_to_all(
                lang::GRAVE_ASSAULTED,
                herald,
                Self::av_node_name(node),
                faction,
            );
            self.bg_update_player_score(player, score_type::GRAVEYARDS_ASSAULTED, 1);
        }
        self.bg_play_sound_to_all(if team_idx == 0 {
            av_sound::ALLIANCE_ASSAULTS
        } else {
            av_sound::HORDE_ASSAULTS
        });
    }

    /// `EventPlayerDefendsPoint`.
    fn av_event_player_defends_point(&mut self, player: ObjectGuid, node: u8) {
        let team = self.player(player).map_or(Team::None, |p| p.team);
        let team_idx = team_index(team) as u8;
        let cur = self
            .av()
            .map(|av| av.nodes[node as usize])
            .unwrap_or_default();
        if cur.owner == team_idx || cur.state != POINT_ASSAULTED {
            return;
        }
        if cur.total_owner == AV_TEAM_NEUTRAL {
            self.av_event_player_assaults_point(player, node);
            return;
        }
        if cur.prev_owner != team_idx {
            return;
        }
        self.av_defend_node(node, team_idx);
        self.av_populate_node(node);
        self.av_update_node_world_state(node);
        let herald = self.av_herald_guid();
        let faction = if team_idx == 0 {
            ab_lang::BG_ALLY
        } else {
            ab_lang::BG_HORDE
        };
        if Self::av_is_tower(node) {
            self.av_send_yell2_to_all(
                lang::TOWER_DEFENDED,
                herald,
                Self::av_node_name(node),
                faction,
            );
            self.bg_update_player_score(player, score_type::TOWERS_DEFENDED, 1);
            self.bg_play_sound_to_all(av_sound::BOTH_TOWER_DEFEND);
        } else {
            self.av_send_yell2_to_all(
                lang::GRAVE_DEFENDED,
                herald,
                Self::av_node_name(node),
                faction,
            );
            self.bg_update_player_score(player, score_type::GRAVEYARDS_DEFENDED, 1);
            self.bg_play_sound_to_all(if team_idx == 0 {
                av_sound::ALLIANCE_GOOD
            } else {
                av_sound::HORDE_GOOD
            });
        }
    }

    /// `FillInitialWorldStates`.
    pub fn av_fill_initial_world_states(&mut self, player: ObjectGuid) -> Option<WorldPacket> {
        let av = self.av()?;
        let nodes = av.nodes;
        let (mine_owner, mine_prev) = (av.mine_owner, av.mine_prev_owner);
        let (score_a, score_h) = self.bg.as_ref().map_or((0, 0), |b| {
            (b.team_scores[0].max(0), b.team_scores[1].max(0))
        });
        let in_progress = self.av_in_progress();
        let zone_id = self
            .zone_and_area_id(
                self.player(player)
                    .map_or(Position::default(), |p| p.unit.world.position()),
            )
            .0;
        let mut pairs: Vec<(u32, u32)> = crate::map::DEFAULT_WORLD_STATES
            .iter()
            .map(|&(s, v)| (s, v))
            .collect();
        for (n, node) in nodes.iter().enumerate() {
            for j in 0..AV_MAX_STATES {
                let ok = (node.state == j) as u32;
                pairs.push((
                    AV_NODE_WORLD_STATES[n][Self::av_world_state_type(j, 0)],
                    u32::from(node.owner == 0) * ok,
                ));
                pairs.push((
                    AV_NODE_WORLD_STATES[n][Self::av_world_state_type(j, 1)],
                    u32::from(node.owner == 1) * ok,
                ));
            }
        }
        if nodes[NODE_SNOWFALL_GRAVE as usize].owner == AV_TEAM_NEUTRAL {
            pairs.push((WS_SNOWFALL_NEUTRAL, 1));
        }
        pairs.push((WS_ALLIANCE_SCORE, score_a as u32));
        pairs.push((WS_HORDE_SCORE, score_h as u32));
        let show = u32::from(in_progress);
        pairs.push((WS_SHOW_A_SCORE, show));
        pairs.push((WS_SHOW_H_SCORE, show));
        for m in 0..AV_MAX_MINES {
            pairs.push((AV_MINE_WORLD_STATES[m][mine_owner[m] as usize], 1));
            if mine_owner[m] != mine_prev[m] {
                pairs.push((AV_MINE_WORLD_STATES[m][mine_prev[m] as usize], 0));
            }
        }
        let mut writer = PacketWriter::new();
        writer.write_u32_le(self.map_id);
        writer.write_u32_le(zone_id);
        writer.write_u16_le(pairs.len() as u16 + 1);
        for (state, value) in pairs {
            writer.write_u32_le(state);
            writer.write_u32_le(value);
        }
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let mut packet = WorldPacket::new(op::SMSG_INIT_WORLD_STATES);
        packet.take_writer(writer);
        Some(packet)
    }

    /// `GetClosestGraveYard` (GM arm folded into the
    /// in-progress check: no cheap GM read on Map).
    #[must_use]
    pub fn av_closest_graveyard(&mut self, player: ObjectGuid) -> Option<WorldSafeLoc> {
        let team = self.player(player).map_or(Team::None, |p| p.team);
        if !self.av_in_progress() {
            let id = if team == Team::Alliance { 611 } else { 610 };
            return self.stores.safe_locs.get(id);
        }
        let t = team_index(team);
        let pos = self
            .player(player)
            .map_or(Position::default(), |p| p.unit.world.position());
        let mut graveyard = self.stores.safe_locs.get(AV_GRAVEYARD_IDS[t + 7]);
        let mut min_dist = 9_999_999.0f32;
        if let Some(g) = graveyard {
            min_dist = (g.x - pos.x) * (g.x - pos.x) + (g.y - pos.y) * (g.y - pos.y);
        }
        for (node, &graveyard_id) in AV_GRAVEYARD_IDS[..=usize::from(NODE_FROSTWOLF_HUT)]
            .iter()
            .enumerate()
        {
            let owned = self.av().is_some_and(|av| {
                av.nodes[node].owner == t as u8 && av.nodes[node].state == POINT_CONTROLLED
            });
            if !owned {
                continue;
            }
            let Some(entry) = self.stores.safe_locs.get(graveyard_id) else {
                continue;
            };
            let dist =
                (entry.x - pos.x) * (entry.x - pos.x) + (entry.y - pos.y) * (entry.y - pos.y);
            if min_dist > dist {
                min_dist = dist;
                graveyard = Some(entry);
            }
        }
        graveyard
    }

    /// `HandleAreaTrigger` (build > 1.6.1 arms).
    pub(crate) fn av_handle_area_trigger(&mut self, source: ObjectGuid, trigger_id: u32) -> bool {
        match trigger_id {
            2608 => {
                if self.player(source).map(|p| p.team) == Some(Team::Alliance) {
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            2606 => {
                if self.player(source).map(|p| p.team) == Some(Team::Horde) {
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            3326..=3331 => true,
            _ => false,
        }
    }

    /// `UpdatePlayerScore` kind arm.
    pub(crate) fn av_update_score(
        &mut self,
        source: ObjectGuid,
        score_type: u32,
        value: u32,
    ) -> bool {
        let Some(bg) = self.bg.as_mut() else {
            return false;
        };
        let Some(score) = bg.scores.get_mut(&source) else {
            return false;
        };
        match score_type {
            score_type::GRAVEYARDS_ASSAULTED => score.graveyards_assaulted += value,
            score_type::GRAVEYARDS_DEFENDED => score.graveyards_defended += value,
            score_type::TOWERS_ASSAULTED => score.towers_assaulted += value,
            score_type::TOWERS_DEFENDED => score.towers_defended += value,
            score_type::SECONDARY_OBJECTIVES => score.secondary_objectives += value,
            _ => return false,
        }
        true
    }

    /// `EndBattleGround` AV reward stage (pre).
    pub(crate) fn av_end_battleground_rewards(&mut self, winner: Team) {
        let mut towers_survived = [0u32; 2];
        let mut graves_owned = [0u32; 2];
        let mut mines_owned = [0u32; 2];
        if let Some(av) = self.av() {
            for i in NODE_DUNBALDAR_SOUTH..=NODE_STONEHEART_BUNKER {
                let n = av.nodes[i as usize];
                if n.state == POINT_CONTROLLED && n.total_owner == 0 {
                    towers_survived[0] += 1;
                }
            }
            for i in NODE_ICEBLOOD_TOWER..=NODE_FROSTWOLF_WTOWER {
                let n = av.nodes[i as usize];
                if n.state == POINT_CONTROLLED && n.total_owner == 1 {
                    towers_survived[1] += 1;
                }
            }
            // fork counts controlled nodes of every
            // kind except firstaid/hut as "graves".
            for (i, n) in av.nodes.iter().enumerate() {
                if n.state == POINT_CONTROLLED
                    && n.owner != AV_TEAM_NEUTRAL
                    && i != NODE_FROSTWOLF_HUT as usize
                    && i != NODE_FIRSTAID_STATION as usize
                {
                    graves_owned[n.owner as usize] += 1;
                }
            }
            for m in 0..AV_MAX_MINES {
                if av.mine_owner[m] != AV_TEAM_NEUTRAL {
                    mines_owned[av.mine_owner[m] as usize] += 1;
                }
            }
        }
        for (t, team) in [(0usize, Team::Alliance), (1usize, Team::Horde)] {
            let faction = if t == 0 {
                FACTION_ALLIANCE
            } else {
                FACTION_HORDE
            };
            if towers_survived[t] > 0 {
                let rep = towers_survived[t] * self.av().map_or(0, |av| av.rep_survive_tower);
                self.bg_reward_reputation_to_team(faction, i32::try_from(rep).unwrap_or(0), team);
                let honor = self.av_bonus_honor_scaled(towers_survived[t] * KILL_SURVIVING_TOWER);
                self.bg_reward_honor_to_team(honor, team);
            }
            // bench patch 1.12 >= WOW_PATCH_107
            if graves_owned[t] > 0 {
                let rep = graves_owned[t] * self.av().map_or(0, |av| av.rep_owned_grave);
                self.bg_reward_reputation_to_team(faction, i32::try_from(rep).unwrap_or(0), team);
                let honor = self.av_bonus_honor_scaled(graves_owned[t] * KILL_SURVIVING_GRAVE);
                self.bg_reward_honor_to_team(honor, team);
            }
            if mines_owned[t] > 0 {
                let rep = mines_owned[t] * self.av().map_or(0, |av| av.rep_owned_mine);
                self.bg_reward_reputation_to_team(faction, i32::try_from(rep).unwrap_or(0), team);
                let honor = self.av_bonus_honor_scaled(mines_owned[t] * KILL_SURVIVING_MINE);
                self.bg_reward_honor_to_team(honor, team);
            }
            let dead_ev = if t == 0 {
                EV_CAPTAIN_DEAD_A
            } else {
                EV_CAPTAIN_DEAD_H
            };
            if !self.bg_is_active_event(dead_ev, 0) {
                let rep = self.av().map_or(0, |av| av.rep_survive_captain);
                self.bg_reward_reputation_to_team(faction, i32::try_from(rep).unwrap_or(0), team);
                let honor = self.av_bonus_honor(KILL_SURVIVING_CAPTAIN);
                self.bg_reward_honor_to_team(honor, team);
            }
        }
        // UNMODELED: `IsBgWeekend` flat 1584/396 honor.
        let _ = winner;
    }

    /// `HandleKillPlayer` (base runs just before in
    /// `bg_handle_kill_player`): -1 reinforcement
    /// unless the victim has Spirit of Redemption
    /// (aura 27827).
    pub fn av_on_kill_player(&mut self, victim: ObjectGuid, _killer: Option<ObjectGuid>) {
        if !self.av_in_progress() {
            return;
        }
        if !self.player_has_spirit_of_redemption(victim) {
            let t = self.player(victim).map(|p| team_index(p.team) as u8);
            if let Some(t) = t {
                self.av_update_score_team(t, -1);
            }
        }
    }

    /// `HandleKillUnit` (event-index driven).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn av_on_kill_unit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        if !self.av_in_progress() {
            return;
        }
        let Some(entry) = self.object_entry(victim) else {
            return;
        };
        match entry {
            NPC_LANDMINES_LAYER_A2 | NPC_LANDMINES_LAYER_H2 => {
                let ev = if entry == NPC_LANDMINES_LAYER_A2 {
                    101u8
                } else {
                    100u8
                };
                // fork sets active event = 1 here
                // ("Cf script_name go_av_landmineAI").
                if let Some(bg) = self.bg.as_mut() {
                    bg.active_events.insert(ev, 1);
                }
                return;
            }
            NPC_LANDMINES_EXPERT_A2 | NPC_LANDMINES_EXPERT_H2 => {
                let ev = if entry == NPC_LANDMINES_EXPERT_A2 {
                    101u8
                } else {
                    100u8
                };
                // UNMODELED: fork AddObjectToRemoveList
                // over the event's gameobjects (no GO
                // remove-list runtime yet).
                let _ = ev;
                return;
            }
            _ => {}
        }
        if killer.is_none() {
            return;
        }
        let (event1, _event2) = self
            .stores
            .battle_event_indexes
            .creature_indexes(victim.counter())
            .first()
            .map_or((BG_EVENT_NONE_VAL, 0), |&(e1, e2)| (e1, e2));
        if event1 == BG_EVENT_NONE_VAL {
            return;
        }
        let killer_team = killer.and_then(|k| self.player(k).map(|p| p.team));
        match event1 {
            EV_BOSS_A => {
                self.bg_cast_spell_on_team(SPELL_BOSS_KILL_QUEST, Team::Horde);
                let rep = self.av().map_or(0, |av| av.rep_boss);
                self.bg_reward_reputation_to_team(
                    FACTION_HORDE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Horde,
                );
                let honor = self.av_bonus_honor_scaled(KILL_BOSS);
                self.bg_reward_honor_to_team(honor, Team::Horde);
                let herald = self.av_herald_guid();
                self.av_send_yell_to_all(lang::A_GENERAL_DEAD, herald);
                self.bg_end_battleground(Team::Horde);
            }
            EV_BOSS_H => {
                self.bg_cast_spell_on_team(SPELL_BOSS_KILL_QUEST, Team::Alliance);
                let rep = self.av().map_or(0, |av| av.rep_boss);
                self.bg_reward_reputation_to_team(
                    FACTION_ALLIANCE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Alliance,
                );
                let honor = self.av_bonus_honor_scaled(KILL_BOSS);
                self.bg_reward_honor_to_team(honor, Team::Alliance);
                let herald = self.av_herald_guid();
                self.av_send_yell_to_all(lang::H_GENERAL_DEAD, herald);
                self.bg_end_battleground(Team::Alliance);
            }
            EV_CAPTAIN_A => {
                if self.bg_is_active_event(EV_CAPTAIN_DEAD_A, 0) {
                    return;
                }
                let rep = self.av().map_or(0, |av| av.rep_captain);
                self.bg_reward_reputation_to_team(
                    FACTION_HORDE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Horde,
                );
                let honor = self.av_bonus_honor(KILL_CAPTAIN);
                self.bg_reward_honor_to_team(honor, Team::Horde);
                self.av_update_score_team(0, -RES_CAPTAIN);
                self.bg_spawn_event(EV_CAPTAIN_DEAD_A, 0, true, true, 0);
            }
            EV_CAPTAIN_H => {
                if self.bg_is_active_event(EV_CAPTAIN_DEAD_H, 0) {
                    return;
                }
                let rep = self.av().map_or(0, |av| av.rep_captain);
                self.bg_reward_reputation_to_team(
                    FACTION_ALLIANCE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Alliance,
                );
                let honor = self.av_bonus_honor(KILL_CAPTAIN);
                self.bg_reward_honor_to_team(honor, Team::Alliance);
                self.av_update_score_team(1, -RES_CAPTAIN);
                self.bg_spawn_event(EV_CAPTAIN_DEAD_H, 0, true, true, 0);
            }
            EV_COMMANDER_A_MORTIMER..=EV_COMMANDER_H_MALGOR => {
                let alliance_side = event1 <= EV_COMMANDER_A_KARLPHILIPS;
                let (gain_team, faction) = if alliance_side {
                    (Team::Horde, FACTION_HORDE)
                } else {
                    (Team::Alliance, FACTION_ALLIANCE)
                };
                let rep = self.av().map_or(0, |av| av.rep_commander);
                self.bg_reward_reputation_to_team(
                    faction,
                    i32::try_from(rep).unwrap_or(0),
                    gain_team,
                );
                let honor = self.av_bonus_honor(KILL_COMMANDER);
                self.bg_reward_honor_to_team(honor, gain_team);
                self.bg_set_spawn_event_mode(event1, 0, spawn_mode::RESPAWN_STOP);
                if event1 == EV_COMMANDER_A_KARLPHILIPS {
                    self.av_complete_quest_for_all(7281);
                }
                if event1 == EV_COMMANDER_H_LOUISPHILIPS {
                    self.av_complete_quest_for_all(7282);
                }
            }
            EV_MINE_BOSSES_NORTH | EV_MINE_BOSSES_SOUTH => {
                let mine = if event1 == EV_MINE_BOSSES_NORTH {
                    MINE_NORTH
                } else {
                    MINE_SOUTH
                };
                match killer_team {
                    Some(Team::Alliance) => {
                        self.av_change_mine_owner(mine, 0);
                        self.av_complete_quest_for_all(7122);
                    }
                    Some(Team::Horde) => {
                        self.av_change_mine_owner(mine, 1);
                        self.av_complete_quest_for_all(7124);
                    }
                    _ => self.av_change_mine_owner(mine, AV_TEAM_NEUTRAL),
                }
            }
            EV_EXPLOSIVES_EXPERT_A => {
                self.av_complete_quest_for_all(7367);
                self.bg_set_spawn_event_mode(EV_EXPLOSIVES_EXPERT_A, 0, spawn_mode::RESPAWN_STOP);
            }
            EV_EXPLOSIVES_EXPERT_H => {
                self.av_complete_quest_for_all(7368);
                self.bg_set_spawn_event_mode(EV_EXPLOSIVES_EXPERT_H, 0, spawn_mode::RESPAWN_STOP);
            }
            EV_LIEUTENANT_A => {
                let rep = self.av().map_or(0, |av| av.rep_commander);
                self.bg_reward_reputation_to_team(
                    FACTION_HORDE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Horde,
                );
                let honor = self.av_bonus_honor(KILL_COMMANDER);
                self.bg_reward_honor_to_team(honor, Team::Horde);
            }
            EV_LIEUTENANT_H => {
                let rep = self.av().map_or(0, |av| av.rep_commander);
                self.bg_reward_reputation_to_team(
                    FACTION_ALLIANCE,
                    i32::try_from(rep).unwrap_or(0),
                    Team::Alliance,
                );
                let honor = self.av_bonus_honor(KILL_COMMANDER);
                self.bg_reward_honor_to_team(honor, Team::Alliance);
            }
            _ => {}
        }
    }

    /// `GetActualArmorRessources`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_armor_resources(&self, team_idx: u8) -> u32 {
        self.av_team_resources(team_idx)
    }

    /// `PlayerCanDoMineQuest` (GO-entry flavor; the
    /// (mine, team) form is inlined by callers).
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_player_can_do_mine_quest(&self, go_entry: u32, team: Team) -> bool {
        let t = team_index(team) as u8;
        if go_entry == MINE_N_OBJECT {
            return self
                .av()
                .is_some_and(|av| av.mine_owner[MINE_NORTH as usize] == t);
        }
        if go_entry == MINE_S_OBJECT {
            return self
                .av()
                .is_some_and(|av| av.mine_owner[MINE_SOUTH as usize] == t);
        }
        true
    }

    /// `getReinforcementLevelGroundUnit`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_reinforcement_level(&self, faction_id: u8) -> u32 {
        self.av()
            .map_or(0, |av| av.reinforcement_level[faction_id as usize])
    }

    /// `setReinforcementLevelGroundUnit`
    /// (`AV_NPC_BASIC..CHAMPION` = 0..3).
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_set_reinforcement_level(&mut self, faction_id: u8, resources: u32) {
        let level = if resources < 500 {
            0
        } else if resources < 1000 {
            1
        } else if resources < 1500 {
            2
        } else {
            3
        };
        if let Some(av) = self.av_mut() {
            av.reinforcement_level[faction_id as usize] = level;
        }
    }

    /// Challenge counter accessors (fork asserts are
    /// no-ops on out-of-range).
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_challenge_counter(&self, faction_id: u8, challenge_type: usize) -> u32 {
        self.av()
            .and_then(|av| av.challenge_status.get(faction_id as usize))
            .and_then(|row| row.get(challenge_type).copied())
            .unwrap_or(0)
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_add_challenge_counter(
        &mut self,
        faction_id: u8,
        challenge_type: usize,
        effort: u32,
    ) {
        if let Some(av) = self.av_mut()
            && let Some(cell) = av
                .challenge_status
                .get_mut(faction_id as usize)
                .and_then(|row| row.get_mut(challenge_type))
        {
            *cell += effort;
        }
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_challenge_goals(&self, faction_id: u8, challenge_type: usize) -> u32 {
        self.av()
            .and_then(|av| av.challenge_goals.get(faction_id as usize))
            .and_then(|row| row.get(challenge_type).copied())
            .unwrap_or(0)
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_min_reputation_needed(&self, category: usize) -> u32 {
        self.av()
            .and_then(|av| av.challenge_min_reputation_needed.get(category).copied())
            .unwrap_or(0)
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_challenge_timer_needed(&self, category: usize) -> u32 {
        self.av()
            .and_then(|av| av.challenge_timer_start.get(category).copied())
            .unwrap_or(0)
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_set_player_go_status(&mut self, faction_id: u8, category: usize, value: bool) {
        if let Some(av) = self.av_mut()
            && let Some(cell) = av
                .challenge_player_go_status
                .get_mut(faction_id as usize)
                .and_then(|row| row.get_mut(category))
        {
            *cell = value;
        }
    }
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_player_go_status(&self, faction_id: u8, category: usize) -> bool {
        self.av()
            .and_then(|av| av.challenge_player_go_status.get(faction_id as usize))
            .and_then(|row| row.get(category).copied())
            .unwrap_or(false)
    }

    /// `resetAerialChallengeInvocation`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_reset_aerial_challenge(&mut self, faction_id: u8, aerial_id: usize) {
        let idx = match aerial_id {
            assault::AIR_BEACON_SOLDIER => Some(challenge::SOLDIER_AIR_ASSAULT),
            assault::AIR_BEACON_LIEUTENANT => Some(challenge::LIEUTENANT_AIR_ASSAULT),
            assault::AIR_BEACON_COMMANDER => Some(challenge::COMMANDER_AIR_ASSAULT),
            _ => None,
        };
        if let (Some(idx), Some(av)) = (idx, self.av_mut()) {
            av.challenge_status[faction_id as usize][idx] = 0;
        }
    }
    /// `resetGroundChallengeInvocation`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_reset_ground_challenge(&mut self, faction_id: u8) {
        if let Some(av) = self.av_mut() {
            av.challenge_status[faction_id as usize][challenge::IRONDEEP_GROUND_ASSAULT] = 0;
            av.challenge_status[faction_id as usize][challenge::COLDTOOTH_GROUND_ASSAULT] = 0;
        }
    }
    /// `resetCavalryChallengeInvocation`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_reset_cavalry_challenge(&mut self, faction_id: u8) {
        if let Some(av) = self.av_mut() {
            av.challenge_status[faction_id as usize][challenge::HIDE_CAVALRY_ASSAULT] = 0;
            av.challenge_status[faction_id as usize][challenge::TAMED_CAVALRY_ASSAULT] = 0;
        }
        self.av_reset_tamed_event(faction_id);
    }
    /// `resetWorldBossChallengeInvocation`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_reset_world_boss_challenge(&mut self, faction_id: u8) {
        if let Some(av) = self.av_mut() {
            av.challenge_status[faction_id as usize][challenge::BLOOD_WORLDBOSS_ASSAULT] = 0;
        }
    }
    /// `isAerialChallengeInvocationReady`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_aerial_challenge_ready(&self, faction_id: u8, aerial_id: usize) -> bool {
        let idx = match aerial_id {
            assault::AIR_BEACON_SOLDIER | assault::AIR_GLOBAL_SOLDIER => {
                challenge::SOLDIER_AIR_ASSAULT
            }
            assault::AIR_BEACON_LIEUTENANT | assault::AIR_GLOBAL_LIEUTENANT => {
                challenge::LIEUTENANT_AIR_ASSAULT
            }
            assault::AIR_BEACON_COMMANDER | assault::AIR_GLOBAL_COMMANDER => {
                challenge::COMMANDER_AIR_ASSAULT
            }
            _ => return false,
        };
        self.av_challenge_counter(faction_id, idx) >= self.av_challenge_goals(faction_id, idx)
    }
    /// `isCavalryChallengeInvocationReady`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_cavalry_challenge_ready(&self, faction_id: u8) -> bool {
        self.av_challenge_counter(faction_id, challenge::HIDE_CAVALRY_ASSAULT)
            >= self.av_challenge_goals(faction_id, challenge::HIDE_CAVALRY_ASSAULT)
            && self.av_challenge_counter(faction_id, challenge::TAMED_CAVALRY_ASSAULT)
                >= self.av_challenge_goals(faction_id, challenge::TAMED_CAVALRY_ASSAULT)
    }
    /// `isGroundChallengeInvocationReady`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_ground_challenge_ready(&self, faction_id: u8) -> bool {
        self.av_challenge_counter(faction_id, challenge::IRONDEEP_GROUND_ASSAULT)
            >= self.av_challenge_goals(faction_id, challenge::IRONDEEP_GROUND_ASSAULT)
            || self.av_challenge_counter(faction_id, challenge::COLDTOOTH_GROUND_ASSAULT)
                >= self.av_challenge_goals(faction_id, challenge::COLDTOOTH_GROUND_ASSAULT)
    }
    /// `isWorldBossChallengeInvocationReady`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub(crate) fn av_world_boss_challenge_ready(&self, faction_id: u8) -> bool {
        self.av_challenge_counter(faction_id, challenge::BLOOD_WORLDBOSS_ASSAULT)
            >= self.av_challenge_goals(faction_id, challenge::BLOOD_WORLDBOSS_ASSAULT)
    }
}

impl Map {
    /// `CheckSpellCast` (shredder charms): fails with
    /// `SPELL_UNAVAILABLE` while the recorded team
    /// owner still charms a shredder; otherwise
    /// stamps the caster as owner. Returns false =
    /// block the cast.
    pub(crate) fn av_check_spell_cast(&mut self, caster: ObjectGuid, spell_id: u32) -> bool {
        let team_idx = match spell_id {
            SPELL_SUMMON_SHREDDER_A2 => 0,
            SPELL_SUMMON_SHREDDER_H2 => 1,
            _ => return true,
        };
        if !caster.is_player() {
            return true;
        }
        let owner = self
            .av()
            .map_or(ObjectGuid::EMPTY, |av| av.shredder_owners[team_idx]);
        if !owner.is_empty()
            && let Some(pet) = self.pet_of(owner)
            && matches!(
                self.object_entry(pet),
                Some(NPC_SHREDDER_H2 | NPC_SHREDDER_A2)
            )
        {
            return false;
        }
        if let Some(av) = self.av_mut() {
            av.shredder_owners[team_idx] = caster;
        }
        true
    }

    /// `UpgradeArmor`.
    #[allow(dead_code)] // consumed by battleground_alterac content
    pub fn av_upgrade_armor(&mut self, quest_giver: ObjectGuid, player: ObjectGuid) {
        let Some(team) = self.player(player).map(|p| p.team) else {
            return;
        };
        let faction_id = team_index(team) as u8;
        let resources_now = self.av_team_resources(faction_id);
        let level_now = self.av_reinforcement_level(faction_id);
        let resources = if resources_now >= 500 && level_now == 0 {
            500
        } else if resources_now >= 1000 && level_now == 1 {
            1000
        } else if resources_now >= 1500 && level_now == 2 {
            1500
        } else {
            0
        };
        self.av_set_reinforcement_level(faction_id, resources);
        if resources % 500 == 0 && resources_now != 0 && quest_giver.is_creature() {
            let name = self
                .player(player)
                .map_or(String::new(), |p| p.unit.world.name.clone());
            self.av_monster_text(
                quest_giver,
                &format!("Thanks for the supplies, {name}"),
                false,
            );
            let reward = match resources {
                500 => Some((28418, "Seasoned units are entering the battle!")),
                1000 => Some((28419, "Veteran units are entering the battle!")),
                1500 => Some((28420, "Champion units are entering the battle!")),
                _ => None,
            };
            if let Some((spell, yell)) = reward {
                self.bg_cast_spell_on_team(spell, team);
                self.av_monster_text(quest_giver, yell, true);
            }
        }
        if matches!(resources_now, 500 | 1000 | 1500) {
            for node in 0..AV_NODES_MAX {
                let owned = self.av().is_some_and(|av| {
                    av.nodes[node].owner == faction_id && av.nodes[node].state == POINT_CONTROLLED
                });
                if owned {
                    self.av_populate_node(node as u8);
                }
            }
        }
    }

    /// `HandleQuestComplete` (actor-wired from
    /// `Player::RewardQuest` equivalent; creature
    /// quest enders only).
    #[allow(clippy::too_many_lines)]
    pub fn av_handle_quest_complete(
        &mut self,
        quest_giver: ObjectGuid,
        quest_id: u32,
        player: ObjectGuid,
    ) {
        if !self.av_in_progress() {
            return;
        }
        let Some(team) = self.player(player).map(|p| p.team) else {
            return;
        };
        let t = team_index(team);
        let reputation: u32;
        match quest_id {
            7223 | 6781 | 7224 | 6741 => {
                let total = {
                    let Some(av) = self.av_mut() else { return };
                    av.team_quest_status[t][0] += 20;
                    av.team_quest_status[t][0]
                };
                if total % 100 == 0 && total % 500 != 0 {
                    self.av_monster_text(
                        quest_giver,
                        "Great! Let's keep those supplies coming, people!",
                        false,
                    );
                }
                match total % 500 {
                    100 => self.bg_spawn_event(EV_100_SUPPLIES + t as u8, 0, true, false, 0),
                    200 => self.bg_spawn_event(EV_200_SUPPLIES + t as u8, 0, true, false, 0),
                    300 => self.bg_spawn_event(EV_300_SUPPLIES + t as u8, 0, true, true, 0),
                    400 => self.bg_spawn_event(EV_400_SUPPLIES + t as u8, 0, true, true, 0),
                    0 if total != 0 => {
                        for base in [
                            EV_100_SUPPLIES,
                            EV_200_SUPPLIES,
                            EV_300_SUPPLIES,
                            EV_400_SUPPLIES,
                        ] {
                            self.bg_spawn_event(base + t as u8, 2, true, false, 0);
                        }
                    }
                    _ if total == 20 => {
                        for base in [
                            EV_100_SUPPLIES,
                            EV_200_SUPPLIES,
                            EV_300_SUPPLIES,
                            EV_400_SUPPLIES,
                        ] {
                            self.bg_spawn_event(base + t as u8, 2, true, false, 0);
                        }
                    }
                    _ => {}
                }
                reputation = 1;
            }
            6942 | 6825 => {
                let now = self.av_bump_quest_status(t, 1, 1);
                reputation = 1;
                if now == 90 {
                    let text = if t == 0 {
                        "Soldiers of Stormpike, come to my aid! The beacon must be planted."
                    } else {
                        "Soldiers of the Horde, come to my aid! The beacon must be planted."
                    };
                    self.av_monster_text(quest_giver, text, true);
                }
            }
            6941 | 6826 => {
                let now = self.av_bump_quest_status(t, 2, 1);
                reputation = 2;
                if now == 60 {
                    let text = if t == 0 {
                        "Soldiers of Stormpike, come to my aid! The beacon must be planted."
                    } else {
                        "Soldiers of the Horde, come to my aid! The beacon must be planted."
                    };
                    self.av_monster_text(quest_giver, text, true);
                }
            }
            6943 | 6827 => {
                let now = self.av_bump_quest_status(t, 3, 1);
                reputation = 5;
                if now == 30 {
                    let text = if t == 0 {
                        "Soldiers of Stormpike, come to my aid! The beacon must be planted."
                    } else {
                        "Soldiers of the Horde, come to my aid! The beacon must be planted."
                    };
                    self.av_monster_text(quest_giver, text, true);
                }
            }
            7386 | 7385 | 6881 | 6801 => {
                let bonus = if matches!(quest_id, 7386 | 7385) {
                    4
                } else {
                    0
                };
                let now = self.av_bump_quest_status(t, 4, bonus + 1);
                reputation = bonus + 1;
                if now == 200 {
                    let text = if t == 0 {
                        "Soldiers of Stormpike, aid and protect us! The Forest Lord has granted us his protection. The portal must now be opened!"
                    } else {
                        "Soldiers of Frostwolf, come to my aid! The Ice Lord has granted us his protection. He's accepted the offering! The time has come to unleash him upon the Stormpike Army!"
                    };
                    self.av_monster_text(quest_giver, text, true);
                }
            }
            5892 | 5893 => {
                self.av_bump_quest_status(t, 5, 1);
                reputation = 2;
            }
            6982 | 6985 => {
                self.av_bump_quest_status(t, 6, 1);
                reputation = 3;
            }
            7026 | 7002 => {
                self.av_bump_quest_status(t, 7, 1);
                reputation = 1;
            }
            7027 | 7001 => {
                let now = self.av_bump_quest_status(t, 8, 1);
                reputation = 1;
                if now % 25 == 0 && now != 0 {
                    self.av_monster_text(quest_giver, "The stables are filled up!", true);
                }
                if now % 5 == 0 {
                    let name = self
                        .player(player)
                        .map_or(String::new(), |p| p.unit.world.name.clone());
                    self.av_monster_text(
                        quest_giver,
                        &format!("Thanks for the supplies, {name}"),
                        false,
                    );
                }
                match now % 25 {
                    5 => self.bg_spawn_event(EV_05_TAMED + t as u8, 0, true, false, 0),
                    10 => self.bg_spawn_event(EV_10_TAMED + t as u8, 0, true, false, 0),
                    15 => self.bg_spawn_event(EV_15_TAMED + t as u8, 0, true, true, 0),
                    20 => self.bg_spawn_event(EV_20_TAMED + t as u8, 0, true, true, 0),
                    _ => {
                        if now == 1 {
                            self.av_reset_tamed_event(t as u8);
                        }
                    }
                }
            }
            7402 | 7428 | 7364 | 7424 | 7365 | 7425 | 7426 | 7366 | 7367 | 7368 | 7401 | 7427
            | 7361 | 7421 | 7422 | 7362 | 7423 | 7363 => {
                reputation = 1;
            }
            _ => return,
        }
        if reputation != 0 {
            let faction = if team == Team::Alliance {
                FACTION_ALLIANCE
            } else {
                FACTION_HORDE
            };
            self.bg_reward_reputation_to_team(
                faction,
                i32::try_from(reputation).unwrap_or(0),
                team,
            );
        }
    }

    fn av_bump_quest_status(&mut self, team: usize, slot: usize, by: u32) -> u32 {
        let Some(av) = self.av_mut() else { return 0 };
        av.team_quest_status[team][slot] += by;
        av.team_quest_status[team][slot]
    }
}
