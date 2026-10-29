//! Arathi Basin rules (`BattleGroundAB.cpp`, Tier 3
//! Slice 3 step 4). Five-node resource race: banner
//! assault/contest/occupy flow, resource ticks,
//! node world states, nearest-occupied graveyard.

use mangos_entities::creature::SpawnRng;
use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::battlegrounds::WorldSafeLoc;

use crate::battlegrounds::{
    BG_EVENT_DOOR, BG_EVENT_NONE_VAL, BgKind, RESPAWN_NEVER_MS, bg_status, score_type, team_index,
};
use crate::bg_ws::{BG_EVENT_GHOST_GATE, CHAT_MSG_BG_SYSTEM_ALLIANCE, CHAT_MSG_BG_SYSTEM_HORDE};
use crate::map::{Map, MapEvent};

/// `BG_AB_NODES_MAX`.
pub(crate) const AB_NODES_MAX: usize = 5;
/// `BG_AB_FLAG_CAPTURING_TIME`.
pub(crate) const AB_FLAG_CAPTURING_TIME: u32 = 60_000;
/// `BG_AB_WARNING_NEAR_VICTORY_SCORE`.
pub(crate) const AB_WARNING_NEAR_VICTORY_SCORE: i32 = 1800;
/// `BG_AB_MAX_TEAM_SCORE`.
pub(crate) const AB_MAX_TEAM_SCORE: i32 = 2000;

/// Node indexes (`enum BG_AB_Nodes`).
pub(crate) const NODE_STABLES: u8 = 0;
pub(crate) const NODE_BLACKSMITH: u8 = 1;
pub(crate) const NODE_FARM: u8 = 2;
pub(crate) const NODE_LUMBER_MILL: u8 = 3;
pub(crate) const NODE_GOLD_MINE: u8 = 4;

/// `enum BG_AB_NodeStatus`.
pub(crate) mod node_status {
    pub const NEUTRAL: u8 = 0;
    pub const TYPE_CONTESTED: u8 = 1;
    pub const ALLY_CONTESTED: u8 = 1;
    pub const HORDE_CONTESTED: u8 = 2;
    pub const TYPE_OCCUPIED: u8 = 3;
    pub const ALLY_OCCUPIED: u8 = 3;
    pub const HORDE_OCCUPIED: u8 = 4;
}

/// `enum BG_AB_WorldStates`.
pub(crate) const WS_OCCUPIED_BASES_HORDE: u32 = 1778;
pub(crate) const WS_OCCUPIED_BASES_ALLY: u32 = 1779;
pub(crate) const WS_RESOURCES_ALLY: u32 = 1776;
pub(crate) const WS_RESOURCES_HORDE: u32 = 1777;
pub(crate) const WS_RESOURCES_MAX: u32 = 1780;
pub(crate) const WS_RESOURCES_WARNING: u32 = 1955;

/// `BG_AB_OP_NODESTATES`.
pub(crate) const AB_NODE_STATES: [u32; 5] = [1767, 1782, 1772, 1792, 1787];
/// `BG_AB_OP_NODEICONS`.
pub(crate) const AB_NODE_ICONS: [u32; 5] = [1842, 1846, 1845, 1844, 1843];
/// Shared `plusArray` mapping of `_SendNodeUpdate` /
/// `FillInitialWorldStates`.
pub(crate) const AB_PLUS_ARRAY: [u32; 5] = [0, 2, 3, 0, 1];

/// `enum BG_AB_Sounds`.
pub(crate) mod ab_sound {
    pub const NODE_CLAIMED: u32 = 8192;
    pub const NODE_CAPTURED_ALLIANCE: u32 = 8173;
    pub const NODE_CAPTURED_HORDE: u32 = 8213;
    pub const NODE_ASSAULTED_ALLIANCE: u32 = 8212;
    pub const NODE_ASSAULTED_HORDE: u32 = 8174;
    pub const NEAR_VICTORY_ALLIANCE: u32 = 8457;
    pub const NEAR_VICTORY_HORDE: u32 = 8456;
}

/// `BCT_BG_AB_*` broadcast-text ids.
pub(crate) mod bct {
    pub const A_NEAR_VICTORY: i32 = 10598;
    pub const H_NEAR_VICTORY: i32 = 10599;
}

/// `LANG_BG_*` `mangos_string` ids.
pub(crate) mod lang {
    pub const BG_ALLY: i32 = 650;
    pub const BG_HORDE: i32 = 651;
    pub const NODE_STABLES: i32 = 652;
    pub const NODE_BLACKSMITH: i32 = 653;
    pub const NODE_FARM: i32 = 654;
    pub const NODE_LUMBER_MILL: i32 = 655;
    pub const NODE_GOLD_MINE: i32 = 656;
    pub const NODE_TAKEN: i32 = 657;
    pub const NODE_DEFENDED: i32 = 658;
    pub const NODE_ASSAULTED: i32 = 659;
    pub const NODE_CLAIMED: i32 = 660;
}

/// `enum BG_AB_NodeOccupiedCredit`.
pub(crate) const AB_NODE_CREDITS: [u32; 5] = [15001, 15002, 15003, 15004, 15005];

/// `BG_AB_TickIntervals` / `BG_AB_TickPoints` (index =
/// occupied nodes).
pub(crate) const AB_TICK_INTERVALS: [u32; 6] = [0, 12_000, 9_000, 6_000, 3_000, 1_000];
pub(crate) const AB_TICK_POINTS: [i32; 6] = [0, 10, 10, 10, 10, 30];

/// `BG_AB_PerTickHonor` / `BG_AB_WinMatchHonor`
/// (same table).
pub(crate) const AB_PER_TICK_HONOR: [u32; 5] = [41, 68, 113, 189, 198];

/// `BG_AB_GraveyardIds` (nodes 0..4, ally start 5,
/// horde start 6).
pub(crate) const AB_GRAVEYARD_IDS: [u32; 7] = [895, 894, 893, 897, 896, 898, 899];
/// Entrance graveyards before start (Alliance 890,
/// Horde 889).
pub(crate) const AB_ENTRANCE_GY_ALLIANCE: u32 = 890;
pub(crate) const AB_ENTRANCE_GY_HORDE: u32 = 889;

/// `SPELL_AB_QUEST_REWARD_4_BASES` / `_5_BASES`.
pub(crate) const SPELL_REWARD_4_BASES: u32 = 24061;
pub(crate) const SPELL_REWARD_5_BASES: u32 = 24064;

/// `enum BG_AB_ObjectType` first/limit.
pub(crate) const OBJECT_SPEEDBUFF_STABLES: usize = 1;
pub(crate) const OBJECT_MAX: usize = 16;

/// `BG_AB_BuffPositions` (x, y, z, o per node,
/// Nostalrius retail 5.4.8 positions).
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
pub(crate) const AB_BUFF_POSITIONS: [[f32; 4]; 5] = [
    [1185.566, 1184.629, -56.36329, 2.303831],
    [989.939026, 1008.75, -42.60327, 0.8203033],
    [818.0089, 842.3543, -56.54062, 3.176533],
    [808.8463, 1185.417, 11.92161, 5.619962],
    [1147.091, 816.8362, -98.39896, 6.056293],
];

/// `BattleGroundAB` members.
#[derive(Clone, Debug, Default)]
pub struct AbState {
    /// `m_nodes`.
    pub nodes: [u8; 5],
    /// `m_prevNodes`.
    pub prev_nodes: [u8; 5],
    /// `m_nodeTimers`.
    pub node_timers: [u32; 5],
    /// `m_lastTick`.
    pub last_tick: [u32; 2],
    /// `m_honorScoreTics`.
    pub honor_score_tics: [i32; 2],
    /// `m_reputationScoreTics`.
    pub reputation_score_tics: [i32; 2],
    /// `m_isInformedNearVictory`.
    pub informed_near_victory: bool,
    /// `m_honorTics` (0 = Reset not yet applied).
    pub honor_tics: u32,
    /// `m_reputationTics`.
    pub reputation_tics: u32,
}

impl Map {
    fn ab(&self) -> Option<&AbState> {
        match self.bg.as_ref().map(|b| &b.kind) {
            Some(BgKind::Ab(ab)) => Some(ab),
            _ => None,
        }
    }

    fn ab_mut(&mut self) -> Option<&mut AbState> {
        match self.bg.as_mut().map(|b| &mut b.kind) {
            Some(BgKind::Ab(ab)) => Some(ab),
            _ => None,
        }
    }

    fn ab_in_progress(&self) -> bool {
        self.bg
            .as_ref()
            .is_some_and(|b| b.status == bg_status::IN_PROGRESS)
    }

    /// `_GetNodeNameId` (fork asserts on bad node;
    /// 0 here).
    fn ab_node_name_id(node: u8) -> i32 {
        match node {
            NODE_STABLES => lang::NODE_STABLES,
            NODE_BLACKSMITH => lang::NODE_BLACKSMITH,
            NODE_FARM => lang::NODE_FARM,
            NODE_LUMBER_MILL => lang::NODE_LUMBER_MILL,
            NODE_GOLD_MINE => lang::NODE_GOLD_MINE,
            _ => 0,
        }
    }

    /// `SendMessage2ToAll` with LANG args: map side
    /// emits the lang ids, the actor formats
    /// (`BgLangMessage2`); arg2 0 = none.
    fn ab_send_message2(
        &mut self,
        entry: i32,
        msg_type: u8,
        source: Option<ObjectGuid>,
        arg1: i32,
        arg2: i32,
    ) {
        self.events.push(MapEvent::BgLangMessage2 {
            entry,
            msg_type,
            source,
            arg_ids: [arg1, arg2],
        });
    }

    /// `_SendNodeUpdate`.
    fn ab_send_node_update(&mut self, node: u8) {
        let Some(ab) = self.ab() else { return };
        let (prev, now) = (ab.prev_nodes[node as usize], ab.nodes[node as usize]);
        let nodes = ab.nodes;
        if prev != 0 {
            self.bg_update_world_state(
                AB_NODE_STATES[node as usize] + AB_PLUS_ARRAY[prev as usize],
                0,
            );
        } else {
            self.bg_update_world_state(AB_NODE_ICONS[node as usize], 0);
        }
        self.bg_update_world_state(
            AB_NODE_STATES[node as usize] + AB_PLUS_ARRAY[now as usize],
            1,
        );
        let (mut ally, mut horde) = (0u32, 0u32);
        for state in nodes {
            if state == node_status::ALLY_OCCUPIED {
                ally += 1;
            } else if state == node_status::HORDE_OCCUPIED {
                horde += 1;
            }
        }
        self.bg_update_world_state(WS_OCCUPIED_BASES_ALLY, ally);
        self.bg_update_world_state(WS_OCCUPIED_BASES_HORDE, horde);
    }

    /// `_CreateBanner`.
    fn ab_create_banner(&mut self, node: u8, node_type: u8, team_index: u8) {
        let delay = match node_type {
            node_status::TYPE_CONTESTED => 1,
            node_status::TYPE_OCCUPIED => 5,
            _ => 0,
        };
        let event2 = if node_type == node_status::NEUTRAL {
            node_type
        } else {
            node_type + team_index
        };
        self.bg_spawn_event(node, event2, true, true, delay);
    }

    /// `_NodeOccupied` (quest-reward spells at 4/5
    /// bases).
    fn ab_node_occupied(&mut self, team: Team) {
        let Some(ab) = self.ab() else { return };
        let want = team_index(team) as u8 + node_status::TYPE_OCCUPIED;
        let captured = ab
            .nodes
            .iter()
            .zip(ab.node_timers.iter())
            .filter(|&(state, timer)| *state == want && *timer == 0)
            .count();
        if captured >= 5 {
            self.bg_cast_spell_on_team(SPELL_REWARD_5_BASES, team);
        }
        if captured >= 4 {
            self.bg_cast_spell_on_team(SPELL_REWARD_4_BASES, team);
        }
    }

    /// Constructor: `m_buffChange = true` +
    /// `m_bgObjects.resize(16)` + start messages.
    /// Buff-change swaps run through
    /// `HandleTriggerBuff` (trap GO runtime
    /// unmodeled, recorded in the class doc above);
    /// start messages are seeded by `BgState::new`.
    pub(crate) fn ab_construct(&mut self) {
        if let Some(bg) = self.bg.as_mut() {
            bg.buff_change = true;
            bg.bg_objects.resize(OBJECT_MAX, ObjectGuid::EMPTY);
        }
    }

    /// `SetupBattleGround` + `Reset` tail (once
    /// guard on `m_honorTics`).
    pub(crate) fn ab_setup(&mut self) -> bool {
        if self.ab().is_some_and(|ab| ab.honor_tics != 0) {
            return true;
        }
        // buff trap objects (dynamic creation
        // unmodeled: bg_add_object returns false and
        // the fork only logs the failure).
        for (i, buf) in AB_BUFF_POSITIONS.iter().enumerate() {
            let pos = Position::new(buf[0], buf[1], buf[2], buf[3]);
            let rotation = (0.0, 0.0, (buf[3] / 2.0).sin(), (buf[3] / 2.0).cos());
            let entries = [
                crate::battlegrounds::BG_OBJECTID_SPEEDBUFF_ENTRY,
                crate::battlegrounds::BG_OBJECTID_REGENBUFF_ENTRY,
                crate::battlegrounds::BG_OBJECTID_BERSERKERBUFF_ENTRY,
            ];
            for (j, entry) in entries.into_iter().enumerate() {
                let _ =
                    self.bg_add_object(OBJECT_SPEEDBUFF_STABLES + 3 * i + j, entry, pos, rotation);
            }
        }
        if let Some(bg) = self.bg.as_mut() {
            bg.team_scores = [0, 0];
            for i in 0..AB_NODES_MAX {
                bg.active_events.insert(i as u8, node_status::NEUTRAL);
            }
            bg.active_events.insert(BG_EVENT_GHOST_GATE, 0);
        }
        // UNMODELED: `IsBgWeekend` (see bg_ws.rs);
        // post-1.10 intervals on the bench patch.
        if let Some(ab) = self.ab_mut() {
            ab.nodes = [0; 5];
            ab.prev_nodes = [0; 5];
            ab.node_timers = [0; 5];
            ab.last_tick = [0; 2];
            ab.honor_score_tics = [0; 2];
            ab.reputation_score_tics = [0; 2];
            ab.informed_near_victory = false;
            ab.honor_tics = 330;
            ab.reputation_tics = 200;
        }
        true
    }

    /// `StartingEventCloseDoors`.
    pub(crate) fn ab_starting_close_doors(&mut self) {
        let guids: Vec<ObjectGuid> = (OBJECT_SPEEDBUFF_STABLES..OBJECT_MAX)
            .filter_map(|i| self.bg.as_ref().and_then(|b| b.bg_objects.get(i).copied()))
            .collect();
        for guid in guids {
            self.bg_spawn_object(guid, RESPAWN_NEVER_MS);
        }
    }

    /// `StartingEventOpenDoors`.
    pub(crate) fn ab_starting_open_doors(&mut self) {
        for i in 0..AB_NODES_MAX {
            let buff = self.rng.urand(0, 2) as usize;
            let slot = OBJECT_SPEEDBUFF_STABLES + buff + i * 3;
            if let Some(guid) = self
                .bg
                .as_ref()
                .and_then(|b| b.bg_objects.get(slot).copied())
            {
                // RESPAWN_IMMEDIATELY = 0.
                self.bg_spawn_object(guid, 0);
            }
        }
        self.bg_open_door_event(BG_EVENT_DOOR, 0);
        self.bg_spawn_event(BG_EVENT_GHOST_GATE, 0, false, true, 0);
    }

    /// `AddPlayer` per-kind arm (flat `BgScore`,
    /// see `ws_add_player_pre`).
    pub(crate) fn ab_add_player_pre(&mut self, guid: ObjectGuid) {
        if let Some(bg) = self.bg.as_mut() {
            bg.scores.entry(guid).or_default();
        }
    }

    /// `SendInitWorldStates` BG portion +
    /// `FillInitialWorldStates`.
    pub fn ab_fill_initial_world_states(&mut self, player: ObjectGuid) -> Option<WorldPacket> {
        let nodes = self.ab()?.nodes;
        let (score_a, score_h) = self.bg.as_ref().map_or((0, 0), |b| {
            (b.team_scores[0].max(0), b.team_scores[1].max(0))
        });
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
        for node in 0..AB_NODES_MAX {
            pairs.push((AB_NODE_ICONS[node], u32::from(nodes[node] == 0)));
        }
        for node in 0..AB_NODES_MAX {
            for (i, &plus) in AB_PLUS_ARRAY.iter().enumerate().skip(1) {
                pairs.push((
                    AB_NODE_STATES[node] + plus,
                    u32::from(nodes[node] == i as u8),
                ));
            }
        }
        #[allow(clippy::naive_bytecount)]
        let ally = u32::try_from(
            nodes
                .iter()
                .filter(|&&s| s == node_status::ALLY_OCCUPIED)
                .count(),
        )
        .unwrap_or(0);
        #[allow(clippy::naive_bytecount)]
        let horde = u32::try_from(
            nodes
                .iter()
                .filter(|&&s| s == node_status::HORDE_OCCUPIED)
                .count(),
        )
        .unwrap_or(0);
        pairs.push((WS_OCCUPIED_BASES_ALLY, ally));
        pairs.push((WS_OCCUPIED_BASES_HORDE, horde));
        pairs.push((WS_RESOURCES_MAX, AB_MAX_TEAM_SCORE as u32));
        pairs.push((WS_RESOURCES_WARNING, AB_WARNING_NEAR_VICTORY_SCORE as u32));
        pairs.push((WS_RESOURCES_ALLY, score_a as u32));
        pairs.push((WS_RESOURCES_HORDE, score_h as u32));
        pairs.push((0x745, 0x2));
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

    /// `Update` (node timers, resource ticks,
    /// win test; base update runs after).
    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    pub(crate) fn ab_update(&mut self, diff_ms: u32) {
        if !self.ab_in_progress() {
            return;
        }
        let mut team_points = [0usize; 2];
        for node in 0..AB_NODES_MAX {
            let expire = {
                let Some(ab) = self.ab_mut() else { return };
                if ab.node_timers[node] != 0 {
                    if ab.node_timers[node] > diff_ms {
                        ab.node_timers[node] -= diff_ms;
                        None
                    } else {
                        ab.node_timers[node] = 0;
                        // Change from contested to occupied!
                        let team_index = ab.nodes[node] - 1;
                        ab.prev_nodes[node] = ab.nodes[node];
                        ab.nodes[node] += 2;
                        Some(team_index)
                    }
                } else {
                    None
                }
            };
            if let Some(team_index) = expire {
                self.ab_create_banner(node as u8, node_status::TYPE_OCCUPIED, team_index);
                self.ab_send_node_update(node as u8);
                let team = if team_index == 0 {
                    Team::Alliance
                } else {
                    Team::Horde
                };
                self.ab_node_occupied(team);
                if team_index == 0 {
                    self.ab_send_message2(
                        lang::NODE_TAKEN,
                        CHAT_MSG_BG_SYSTEM_ALLIANCE,
                        None,
                        lang::BG_ALLY,
                        Self::ab_node_name_id(node as u8),
                    );
                    self.bg_play_sound_to_all(ab_sound::NODE_CAPTURED_ALLIANCE);
                } else {
                    self.ab_send_message2(
                        lang::NODE_TAKEN,
                        CHAT_MSG_BG_SYSTEM_HORDE,
                        None,
                        lang::BG_HORDE,
                        Self::ab_node_name_id(node as u8),
                    );
                    self.bg_play_sound_to_all(ab_sound::NODE_CAPTURED_HORDE);
                }
            }
            if let Some(ab) = self.ab() {
                for (team, points) in team_points.iter_mut().enumerate() {
                    if ab.nodes[node] == team as u8 + node_status::TYPE_OCCUPIED {
                        *points += 1;
                    }
                }
            }
        }
        let bracket = self.bg.as_ref().map_or(0, |b| b.bracket_id) as usize;
        for team in [Team::Alliance, Team::Horde] {
            let t = team_index(team);
            let points = team_points[t];
            if points == 0 {
                continue;
            }
            let tick_fired = {
                let Some(ab) = self.ab_mut() else { return };
                ab.last_tick[t] += diff_ms;
                if ab.last_tick[t] > AB_TICK_INTERVALS[points] {
                    ab.last_tick[t] -= AB_TICK_INTERVALS[points];
                    true
                } else {
                    false
                }
            };
            if !tick_fired {
                continue;
            }
            let (new_score, rep_fired, honor_fired, near) = {
                let Some(bg) = self.bg.as_mut() else { return };
                bg.team_scores[t] += AB_TICK_POINTS[points];
                let new_score = bg.team_scores[t];
                let score = &mut bg.kind;
                let BgKind::Ab(ab) = score else { return };
                ab.honor_score_tics[t] += AB_TICK_POINTS[points];
                ab.reputation_score_tics[t] += AB_TICK_POINTS[points];
                let rep = ab.reputation_score_tics[t] >= ab.reputation_tics as i32;
                if rep {
                    ab.reputation_score_tics[t] -= ab.reputation_tics as i32;
                }
                let honor = ab.honor_score_tics[t] >= ab.honor_tics as i32;
                if honor {
                    ab.honor_score_tics[t] -= ab.honor_tics as i32;
                }
                let near = !ab.informed_near_victory && new_score > AB_WARNING_NEAR_VICTORY_SCORE;
                if near {
                    ab.informed_near_victory = true;
                }
                (new_score, rep, honor, near)
            };
            if rep_fired {
                if team == Team::Alliance {
                    self.bg_reward_reputation_to_team(509, 10, Team::Alliance);
                } else {
                    self.bg_reward_reputation_to_team(510, 10, Team::Horde);
                }
            }
            if honor_fired {
                let honor = AB_PER_TICK_HONOR.get(bracket).copied().unwrap_or(0);
                self.bg_reward_honor_to_team(honor, team);
            }
            if near {
                if team == Team::Alliance {
                    self.bg_send_message_to_all(bct::A_NEAR_VICTORY, None);
                    self.bg_play_sound_to_all(ab_sound::NEAR_VICTORY_ALLIANCE);
                } else {
                    self.bg_send_message_to_all(bct::H_NEAR_VICTORY, None);
                    self.bg_play_sound_to_all(ab_sound::NEAR_VICTORY_HORDE);
                }
            }
            let capped = new_score.min(AB_MAX_TEAM_SCORE);
            if let Some(bg) = self.bg.as_mut() {
                bg.team_scores[t] = capped;
            }
            let field = if team == Team::Alliance {
                WS_RESOURCES_ALLY
            } else {
                WS_RESOURCES_HORDE
            };
            self.bg_update_world_state(field, capped.max(0) as u32);
        }
        let (score_a, score_h) = self
            .bg
            .as_ref()
            .map_or((0, 0), |b| (b.team_scores[0], b.team_scores[1]));
        if score_a >= AB_MAX_TEAM_SCORE {
            self.bg_end_battleground(Team::Alliance);
        }
        if score_h >= AB_MAX_TEAM_SCORE {
            self.bg_end_battleground(Team::Horde);
        }
    }

    /// `EndBattleGround` AB reward stage (fork
    /// rewards the win honor twice on weekends).
    pub(crate) fn ab_end_battleground_rewards(&mut self, winner: Team) {
        // UNMODELED: `IsBgWeekend` (see bg_ws.rs).
        let is_weekend = false;
        let bracket = self.bg.as_ref().map_or(0, |b| b.bracket_id) as usize;
        let honor = AB_PER_TICK_HONOR.get(bracket).copied().unwrap_or(0);
        if winner == Team::Alliance {
            if is_weekend {
                self.bg_reward_honor_to_team(honor, Team::Alliance);
            }
            self.bg_reward_honor_to_team(honor, Team::Alliance);
        }
        if winner == Team::Horde {
            if is_weekend {
                self.bg_reward_honor_to_team(honor, Team::Horde);
            }
            self.bg_reward_honor_to_team(honor, Team::Horde);
        }
    }

    /// `UpdatePlayerScore` kind arm.
    pub(crate) fn ab_update_score(
        &mut self,
        source: ObjectGuid,
        score_type: u32,
        value: u32,
    ) -> bool {
        match score_type {
            score_type::BASES_ASSAULTED => {
                if let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    score.bases_assaulted += value;
                }
                true
            }
            score_type::BASES_DEFENDED => {
                if let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    score.bases_defended += value;
                }
                true
            }
            _ => false,
        }
    }

    /// `EventPlayerClickedOnFlag`.
    #[allow(clippy::too_many_lines, clippy::needless_late_init)]
    pub(crate) fn ab_event_clicked_on_flag(&mut self, source: ObjectGuid, go: ObjectGuid) {
        if !self.ab_in_progress() {
            return;
        }
        let event = self
            .stores
            .battle_event_indexes
            .gameobject_indexes(go.counter())
            .first()
            .map_or(BG_EVENT_NONE_VAL, |&(e1, _)| e1);
        if event as usize >= AB_NODES_MAX {
            return;
        }
        let node = event;
        let team = self.player(source).map_or(Team::None, |p| p.team);
        let team_idx = team_index(team) as u8;
        let Some((node_state, prev_state)) = self
            .ab()
            .map(|ab| (ab.nodes[node as usize], ab.prev_nodes[node as usize]))
        else {
            return;
        };
        // Check if player really could use this banner, not cheated
        if !(node_state == 0 || team_idx == node_state % 2) {
            return;
        }
        self.quest_kill_credit_one(source, ObjectGuid::EMPTY, AB_NODE_CREDITS[node as usize]);
        let sound;
        if node_state == node_status::NEUTRAL {
            self.bg_update_player_score(source, score_type::BASES_ASSAULTED, 1);
            if let Some(ab) = self.ab_mut() {
                ab.prev_nodes[node as usize] = ab.nodes[node as usize];
                ab.nodes[node as usize] = team_idx + 1;
                ab.node_timers[node as usize] = AB_FLAG_CAPTURING_TIME;
            }
            self.ab_create_banner(node, node_status::TYPE_CONTESTED, team_idx);
            self.ab_send_node_update(node);
            if team_idx == 0 {
                self.ab_send_message2(
                    lang::NODE_CLAIMED,
                    CHAT_MSG_BG_SYSTEM_ALLIANCE,
                    Some(source),
                    Self::ab_node_name_id(node),
                    lang::BG_ALLY,
                );
            } else {
                self.ab_send_message2(
                    lang::NODE_CLAIMED,
                    CHAT_MSG_BG_SYSTEM_HORDE,
                    Some(source),
                    Self::ab_node_name_id(node),
                    lang::BG_HORDE,
                );
            }
            sound = ab_sound::NODE_CLAIMED;
        } else if node_state == node_status::ALLY_CONTESTED
            || node_state == node_status::HORDE_CONTESTED
        {
            if prev_state < node_status::TYPE_OCCUPIED {
                self.bg_update_player_score(source, score_type::BASES_ASSAULTED, 1);
                if let Some(ab) = self.ab_mut() {
                    ab.prev_nodes[node as usize] = ab.nodes[node as usize];
                    ab.nodes[node as usize] = team_idx + node_status::TYPE_CONTESTED;
                    ab.node_timers[node as usize] = AB_FLAG_CAPTURING_TIME;
                }
                self.ab_create_banner(node, node_status::TYPE_CONTESTED, team_idx);
                self.ab_send_node_update(node);
                let msg_type = if team_idx == 0 {
                    CHAT_MSG_BG_SYSTEM_ALLIANCE
                } else {
                    CHAT_MSG_BG_SYSTEM_HORDE
                };
                self.ab_send_message2(
                    lang::NODE_ASSAULTED,
                    msg_type,
                    Some(source),
                    Self::ab_node_name_id(node),
                    0,
                );
            } else {
                self.bg_update_player_score(source, score_type::BASES_DEFENDED, 1);
                if let Some(ab) = self.ab_mut() {
                    ab.prev_nodes[node as usize] = ab.nodes[node as usize];
                    ab.nodes[node as usize] = team_idx + node_status::TYPE_OCCUPIED;
                    ab.node_timers[node as usize] = 0;
                }
                self.ab_create_banner(node, node_status::TYPE_OCCUPIED, team_idx);
                self.ab_send_node_update(node);
                self.ab_node_occupied(team);
                let msg_type = if team_idx == 0 {
                    CHAT_MSG_BG_SYSTEM_ALLIANCE
                } else {
                    CHAT_MSG_BG_SYSTEM_HORDE
                };
                self.ab_send_message2(
                    lang::NODE_DEFENDED,
                    msg_type,
                    Some(source),
                    Self::ab_node_name_id(node),
                    0,
                );
            }
            sound = if team_idx == 0 {
                ab_sound::NODE_ASSAULTED_ALLIANCE
            } else {
                ab_sound::NODE_ASSAULTED_HORDE
            };
        } else {
            self.bg_update_player_score(source, score_type::BASES_ASSAULTED, 1);
            if let Some(ab) = self.ab_mut() {
                ab.prev_nodes[node as usize] = ab.nodes[node as usize];
                ab.nodes[node as usize] = team_idx + node_status::TYPE_CONTESTED;
                ab.node_timers[node as usize] = AB_FLAG_CAPTURING_TIME;
            }
            self.ab_create_banner(node, node_status::TYPE_CONTESTED, team_idx);
            self.ab_send_node_update(node);
            let msg_type = if team_idx == 0 {
                CHAT_MSG_BG_SYSTEM_ALLIANCE
            } else {
                CHAT_MSG_BG_SYSTEM_HORDE
            };
            self.ab_send_message2(
                lang::NODE_ASSAULTED,
                msg_type,
                Some(source),
                Self::ab_node_name_id(node),
                0,
            );
            sound = if team_idx == 0 {
                ab_sound::NODE_ASSAULTED_ALLIANCE
            } else {
                ab_sound::NODE_ASSAULTED_HORDE
            };
        }
        let now = self.ab().map_or(0, |ab| ab.nodes[node as usize]);
        if now >= node_status::TYPE_OCCUPIED {
            if team_idx == 0 {
                self.ab_send_message2(
                    lang::NODE_TAKEN,
                    CHAT_MSG_BG_SYSTEM_ALLIANCE,
                    None,
                    lang::BG_ALLY,
                    Self::ab_node_name_id(node),
                );
            } else {
                self.ab_send_message2(
                    lang::NODE_TAKEN,
                    CHAT_MSG_BG_SYSTEM_HORDE,
                    None,
                    lang::BG_HORDE,
                    Self::ab_node_name_id(node),
                );
            }
        }
        self.bg_play_sound_to_all(sound);
    }

    /// `GetClosestGraveYard`.
    #[must_use]
    pub fn ab_closest_graveyard(&mut self, player: ObjectGuid) -> Option<WorldSafeLoc> {
        let team = self.player(player).map_or(Team::None, |p| p.team);
        // UNMODELED: the fork's `!player->IsGameMaster()`
        // arm of the pre-start entrance gate (no cheap
        // GM read here); revives route as non-GM.
        if !self.ab_in_progress() {
            let id = if team == Team::Alliance {
                AB_ENTRANCE_GY_ALLIANCE
            } else {
                AB_ENTRANCE_GY_HORDE
            };
            return self.stores.safe_locs.get(id);
        }
        let t = team_index(team);
        let want = t as u8 + node_status::TYPE_OCCUPIED;
        let nodes: Vec<usize> = self
            .ab()
            .map(|ab| {
                ab.nodes
                    .iter()
                    .enumerate()
                    .filter(|&(_, s)| *s == want)
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();
        let mut graveyard: Option<WorldSafeLoc> = None;
        let mut min_dist = 999_999.0f32;
        let pos = self
            .player(player)
            .map_or(Position::default(), |p| p.unit.world.position());
        for node in nodes {
            let Some(entry) = self.stores.safe_locs.get(AB_GRAVEYARD_IDS[node]) else {
                continue;
            };
            let dist =
                (entry.x - pos.x) * (entry.x - pos.x) + (entry.y - pos.y) * (entry.y - pos.y);
            if min_dist > dist {
                min_dist = dist;
                graveyard = Some(entry);
            }
        }
        // If not, place ghost on starting location.
        graveyard.or_else(|| self.stores.safe_locs.get(AB_GRAVEYARD_IDS[t + 5]))
    }

    /// `RemovePlayer` (empty in the fork).
    #[allow(clippy::unused_self)]
    pub(crate) fn ab_remove_player(&mut self, _guid: ObjectGuid) {}

    /// `HandleKillPlayer` / `HandleKillUnit` (no AB
    /// overrides).
    #[allow(clippy::unused_self)]
    pub(crate) fn ab_on_kill_player(&mut self, _victim: ObjectGuid, _killer: Option<ObjectGuid>) {}
    #[allow(clippy::unused_self)]
    pub(crate) fn ab_on_kill_unit(&mut self, _victim: ObjectGuid, _killer: Option<ObjectGuid>) {}

    /// `HandleAreaTrigger`.
    pub(crate) fn ab_handle_area_trigger(&mut self, source: ObjectGuid, trigger_id: u32) -> bool {
        match trigger_id {
            // Arathi Basin Alliance Exit.
            3948 => {
                if self.player(source).map(|p| p.team) == Some(Team::Alliance) {
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            // Arathi Basin Horde Exit.
            3949 => {
                if self.player(source).map(|p| p.team) == Some(Team::Horde) {
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            // Node triggers + unknowns: fork falls
            // through to false (warning commented out).
            _ => false,
        }
    }
}
