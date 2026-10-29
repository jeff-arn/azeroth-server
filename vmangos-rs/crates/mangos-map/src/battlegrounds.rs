//! Battleground match state and base lifecycle
//! (`BattleGround` base class, `BattleGroundDefines.h`;
//! per-map rules in `BgKind`). Queue/invite/session flow
//! lives in `mangosd::battleground`; this module owns
//! everything that happens on the BG map itself.
//!
//! Fork threading note: `BattleGround::Update` runs on
//! the map updater while the queue runs on world
//! update; Rust keeps both on the actor, with the map
//! side emitting `MapEvent::Bg*` notices for manager
//! bookkeeping (free-slot queue, queue scheduling,
//! player transport, BG deletion).

use std::collections::HashMap;
use std::ops::Not;

use mangos_entities::battleground::queue_type_of;
use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::battlegrounds::{BattlegroundTemplate, WorldSafeLoc};

use crate::map::{Map, MapEvent};

/// `enum BattleGroundStatus`.
pub mod bg_status {
    pub const NONE: u32 = 0;
    pub const WAIT_QUEUE: u32 = 1;
    pub const WAIT_JOIN: u32 = 2;
    pub const IN_PROGRESS: u32 = 3;
    pub const WAIT_LEAVE: u32 = 4;
}

/// `enum BattleGroundWinner`.
pub mod bg_winner {
    pub const HORDE: u32 = 0;
    pub const ALLIANCE: u32 = 1;
    pub const NONE: u32 = 2;
}

/// `enum BattleGroundSounds`.
pub const SOUND_HORDE_WINS: u32 = 8454;
pub const SOUND_ALLIANCE_WINS: u32 = 8455;
pub const SOUND_BG_START: u32 = 3439;

/// `enum BattleGroundTimeIntervals` (subset used here).
pub const TIME_TO_AUTOREMOVE_MS: i32 = 120_000;
/// `enum BattleGroundStartTimeIntervals`.
pub const BG_START_DELAY_2M: i32 = 120_000;
pub const BG_START_DELAY_1M: i32 = 60_000;
pub const BG_START_DELAY_30S: i32 = 30_000;

/// `enum ScoreType`.
pub mod score_type {
    pub const KILLING_BLOWS: u32 = 1;
    pub const DEATHS: u32 = 2;
    pub const HONORABLE_KILLS: u32 = 3;
    pub const BONUS_HONOR: u32 = 4;
    pub const FLAG_CAPTURES: u32 = 7;
    pub const FLAG_RETURNS: u32 = 8;
    pub const BASES_ASSAULTED: u32 = 9;
    pub const BASES_DEFENDED: u32 = 10;
    pub const GRAVEYARDS_ASSAULTED: u32 = 11;
    pub const GRAVEYARDS_DEFENDED: u32 = 12;
    pub const TOWERS_ASSAULTED: u32 = 13;
    pub const TOWERS_DEFENDED: u32 = 14;
    pub const SECONDARY_OBJECTIVES: u32 = 15;
}

/// `BattleGroundPlayer`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BgPlayerInfo {
    pub team: Team,
}

/// `BattleGroundScore` + subclass counters flattened
/// (fork subclasses add per-BG fields; one flat struct
/// keeps the same wire/DB surface).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BgScore {
    pub killing_blows: u32,
    pub deaths: u32,
    pub honorable_kills: u32,
    pub bonus_honor: u32,
    // WS (`BattleGroundWGScore`).
    pub flag_captures: u32,
    pub flag_returns: u32,
    // AB (`BattleGroundABScore`).
    pub bases_assaulted: u32,
    pub bases_defended: u32,
    // AV (`BattleGroundAVScore`).
    pub graveyards_assaulted: u32,
    pub graveyards_defended: u32,
    pub towers_assaulted: u32,
    pub towers_defended: u32,
    pub secondary_objectives: u32,
    pub lieutenant_count: u32,
    pub secondary_npc: u32,
}

/// Inputs of `CreateNewBattleGround` for one fresh match
/// (template fields already resolved by the manager).
#[derive(Clone, Debug)]
pub struct BgInit {
    pub bg_type_id: u32,
    pub bracket_id: u32,
    pub client_instance_id: u32,
    pub name: String,
    pub min_players_per_team: u32,
    pub max_players_per_team: u32,
    pub alliance_win_spell: u32,
    pub alliance_lose_spell: u32,
    pub horde_win_spell: u32,
    pub horde_lose_spell: u32,
    pub alliance_start: WorldSafeLoc,
    pub horde_start: WorldSafeLoc,
    pub player_skin_ref_loot_id: u32,
    pub level_min: u32,
    pub level_max: u32,
}

impl BgInit {
    #[must_use]
    pub fn from_template(
        template: &BattlegroundTemplate,
        bracket_id: u32,
        client_instance_id: u32,
        level_min: u32,
        level_max: u32,
    ) -> Self {
        Self {
            bg_type_id: template.bg_type_id,
            bracket_id,
            client_instance_id,
            name: template.name.clone(),
            min_players_per_team: template.min_players_per_team,
            max_players_per_team: template.max_players_per_team,
            alliance_win_spell: template.alliance_win_spell,
            alliance_lose_spell: template.alliance_lose_spell,
            horde_win_spell: template.horde_win_spell,
            horde_lose_spell: template.horde_lose_spell,
            alliance_start: template.alliance_start,
            horde_start: template.horde_start,
            player_skin_ref_loot_id: template.player_skin_ref_loot_id,
            level_min,
            level_max,
        }
    }
}

/// Per-map rule state (`BattleGroundWS/AB/AV` members).
#[derive(Clone, Debug, Default)]
pub enum BgKind {
    #[default]
    None,
    Ws(crate::bg_ws::WsState),
    Ab(crate::bg_ab::AbState),
    Av(Box<crate::bg_av::AvState>),
}

/// The live match (`BattleGround` instance members).
#[derive(Clone, Debug)]
pub struct BgState {
    pub bg_type_id: u32,
    pub status: u32,
    pub bracket_id: u32,
    pub client_instance_id: u32,
    pub name: String,
    pub winner: u32,
    pub start_time_ms: u32,
    pub end_time_ms: i32,
    pub level_min: u32,
    pub level_max: u32,
    pub min_players_per_team: u32,
    pub max_players_per_team: u32,
    pub alliance_win_spell: u32,
    pub alliance_lose_spell: u32,
    pub horde_win_spell: u32,
    pub horde_lose_spell: u32,
    pub team_start: [WorldSafeLoc; 2],
    pub player_skin_ref_loot_id: u32,
    pub players: HashMap<ObjectGuid, BgPlayerInfo>,
    pub scores: HashMap<ObjectGuid, BgScore>,
    pub players_count: [u32; 2],
    pub invited: [u32; 2],
    /// `m_teamScores`.
    pub team_scores: [i32; 2],
    /// `m_events` starting-event bitmask.
    pub events: u8,
    pub start_delay_time_ms: i32,
    pub start_delay_times: [i32; 4],
    pub start_message_ids: [i32; 4],
    /// `m_buffChange`.
    pub buff_change: bool,
    pub premature_countdown: bool,
    pub premature_countdown_timer_ms: u32,
    /// Cached `m_finalScore` packet body (empty until end).
    pub final_score: Option<WorldPacket>,
    /// `m_activeEvents` (event1 -> event2); door event
    /// (254) is seeded at reset.
    pub active_events: HashMap<u8, u8>,
    /// `m_eventObjects`: (event1,event2) -> spawned
    /// object/creature guids collected at DB load.
    pub event_objects: HashMap<(u8, u8), BgEventObjects>,
    /// `m_bgObjects` indexed by per-BG object type slot.
    pub bg_objects: Vec<ObjectGuid>,
    /// Spawn modes per DB creature spawn guid-low
    /// (`SetSpawnEventMode` targets, checked at spawn).
    pub creature_spawn_modes: HashMap<u32, u8>,
    pub kind: BgKind,
    /// Map creation wall-clock (fork `Map::GetCreateTime`,
    /// seconds) for the empty-BG queue nudge.
    pub create_unix: u64,
}

/// `EventObjects`.
#[derive(Clone, Debug, Default)]
pub struct BgEventObjects {
    pub gameobjects: Vec<ObjectGuid>,
    pub creatures: Vec<ObjectGuid>,
}

/// `BattleGroundCreatureSpawnMode`.
pub mod spawn_mode {
    pub const DESPAWN_FORCED: u8 = 0;
    pub const RESPAWN_STOP: u8 = 1;
    pub const RESPAWN_START: u8 = 2;
    pub const RESPAWN_FORCED: u8 = 3;
}

/// Team index (`GetTeamIndexByTeamId`): 0 alliance,
/// 1 horde; `Team::None` reads horde in the fork helper
/// (callers guard).
#[must_use]
pub fn team_index(team: Team) -> usize {
    match team {
        Team::Alliance => 0,
        _ => 1,
    }
}

/// `GetOtherTeam`.
#[must_use]
pub fn other_team(team: Team) -> Team {
    match team {
        Team::Alliance => Team::Horde,
        Team::Horde => Team::Alliance,
        Team::None => Team::None,
    }
}

impl BgState {
    /// `CreateNewBattleGround` copy + `Reset` +
    /// `SetStatus(WAIT_JOIN)` + bracket/client id.
    #[must_use]
    pub fn new(init: BgInit, create_unix: u64) -> Self {
        let kind = match init.bg_type_id {
            2 => BgKind::Ws(crate::bg_ws::WsState::default()),
            3 => BgKind::Ab(crate::bg_ab::AbState::default()),
            1 => BgKind::Av(Box::default()),
            _ => BgKind::None,
        };
        let mut state = Self {
            bg_type_id: init.bg_type_id,
            status: bg_status::WAIT_QUEUE,
            bracket_id: init.bracket_id,
            client_instance_id: init.client_instance_id,
            name: init.name,
            winner: bg_winner::NONE,
            start_time_ms: 0,
            end_time_ms: 0,
            level_min: init.level_min,
            level_max: init.level_max,
            min_players_per_team: init.min_players_per_team,
            max_players_per_team: init.max_players_per_team,
            alliance_win_spell: init.alliance_win_spell,
            alliance_lose_spell: init.alliance_lose_spell,
            horde_win_spell: init.horde_win_spell,
            horde_lose_spell: init.horde_lose_spell,
            team_start: [init.alliance_start, init.horde_start],
            player_skin_ref_loot_id: init.player_skin_ref_loot_id,
            players: HashMap::new(),
            scores: HashMap::new(),
            players_count: [0; 2],
            invited: [0; 2],
            team_scores: [0; 2],
            events: 0,
            start_delay_time_ms: 0,
            start_delay_times: [BG_START_DELAY_2M, BG_START_DELAY_1M, BG_START_DELAY_30S, 0],
            // Subclass ctor `m_startMessageIds`
            // (`BattleGroundDefines.h` BCT_* texts).
            start_message_ids: match init.bg_type_id {
                1 => [0, 10638, 10639, 10640],
                3 => [0, 10477, 10478, 10479],
                _ => [0, 10015, 10016, 10014],
            },
            buff_change: false,
            premature_countdown: false,
            premature_countdown_timer_ms: 0,
            final_score: None,
            active_events: HashMap::new(),
            event_objects: HashMap::new(),
            bg_objects: Vec::new(),
            creature_spawn_modes: HashMap::new(),
            kind,
            create_unix,
        };
        state.reset_common();
        state.status = bg_status::WAIT_JOIN;
        // Per-kind constructor message ids / delays are
        // applied by `Map::bg_setup_construct` (fork
        // subclass ctors set `m_startMessageIds`).
        state
    }

    /// `BattleGround::Reset` common part.
    pub fn reset_common(&mut self) {
        self.winner = bg_winner::NONE;
        self.status = bg_status::WAIT_QUEUE;
        self.start_time_ms = 0;
        self.end_time_ms = 0;
        self.events = 0;
        // door-event2 is always 0
        self.active_events.insert(254, 0);
        self.invited = [0; 2];
        self.players.clear();
        self.scores.clear();
        self.players_count = [0; 2];
        self.final_score = None;
    }

    /// `GetMaxPlayers`.
    #[must_use]
    pub fn max_players(&self) -> u32 {
        self.max_players_per_team * 2
    }

    /// `GetMinPlayers`.
    #[must_use]
    pub fn min_players(&self) -> u32 {
        self.min_players_per_team * 2
    }

    /// `HasFreeSlots`.
    #[must_use]
    pub fn has_free_slots(&self) -> bool {
        (self.players.len() as u32) < self.max_players()
    }

    /// `GetInvitedCount`.
    #[must_use]
    pub fn invited_count(&self, team: Team) -> u32 {
        self.invited[team_index(team)]
    }

    /// `GetFreeSlotsForTeam` (the live fork arm: the
    /// balancing block above it is commented out).
    #[must_use]
    pub fn free_slots_for_team(&self, team: Team) -> u32 {
        if self.status == bg_status::WAIT_JOIN || self.status == bg_status::IN_PROGRESS {
            let invited = self.invited_count(team);
            if invited < self.max_players_per_team {
                return self.max_players_per_team - invited;
            }
        }
        0
    }

    /// `GetPlayersCountByTeam`.
    #[must_use]
    pub fn players_count_by_team(&self, team: Team) -> u32 {
        self.players_count[team_index(team)]
    }

    /// `GetPlayerTeam`.
    #[must_use]
    pub fn player_team(&self, guid: ObjectGuid) -> Team {
        self.players.get(&guid).map_or(Team::None, |p| p.team)
    }

    /// `IsPlayerInBattleGround`.
    #[must_use]
    pub fn is_player_in(&self, guid: ObjectGuid) -> bool {
        self.players.contains_key(&guid)
    }

    /// `GetTeamStartLoc`.
    #[must_use]
    pub fn team_start_pos(&self, team: Team) -> Position {
        let loc = &self.team_start[team_index(team)];
        Position {
            x: loc.x,
            y: loc.y,
            z: loc.z,
            o: loc.orientation,
        }
    }

    /// `GetBonusHonorFromKill`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn bonus_honor_from_kill(&self, kills: u32) -> u32 {
        kills * (honor_gain(self.level_max, self.level_max, 1) as u32)
    }

    /// `GetHonorModifier`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn honor_modifier(&self) -> f32 {
        let elapsed = self.start_time_ms as f32 / 1000.0 / 3600.0;
        if elapsed < 1.0 {
            60f32.powf(elapsed - 1.0)
        } else {
            1.0
        }
    }

    /// `GetBattlemasterEntry`.
    #[must_use]
    pub fn battlemaster_entry(&self) -> u32 {
        battlemaster_entry_for(self.bg_type_id)
    }

    /// `GetWinnerText` (note the fork AV arm reads
    /// A-wins for `WINNER_NONE` too; callers only use
    /// it when nonzero... fork: `if (int32 winTextId =
    /// GetWinnerText(winner))` — all arms return a text
    /// for any input incl. none, kept verbatim).
    #[must_use]
    pub fn winner_text(&self, winner: Team) -> i32 {
        match self.bg_type_id {
            1 => {
                if winner == Team::Horde {
                    7336
                } else {
                    7335
                }
            }
            2 => {
                if winner == Team::Horde {
                    9842
                } else {
                    9843
                }
            }
            3 => {
                if winner == Team::Horde {
                    10634
                } else {
                    10633
                }
            }
            _ => 0,
        }
    }

    /// `GetHeraldEntry`.
    #[must_use]
    pub fn herald_entry(&self) -> u32 {
        match self.bg_type_id {
            1 => 14848,
            2 => 14645,
            _ => 0,
        }
    }
}

/// `BattleGround::GetBattlemasterEntry` by type.
#[must_use]
pub fn battlemaster_entry_for(bg_type_id: u32) -> u32 {
    match bg_type_id {
        1 => 15972,
        2 => 14623,
        3 => 14879,
        _ => 0,
    }
}

/// `MaNGOS::Honor::GetHonorGain` 3-arg form at patch >= 1.8
/// (expFactor 188.3; totalKills 0, groupSize 1).
#[must_use]
pub fn honor_gain(killer_level: u32, victim_level: u32, victim_rank: u32) -> f32 {
    let diff_factor = crate::xp::base_gain_level_factor(killer_level, victim_level);
    let level_coeff: f64 = match killer_level {
        60.. => 1.0,
        50..=59 => 0.9545,
        40..=49 => 0.5707,
        30..=39 => 0.3434,
        20..=29 => 0.2070,
        _ => 0.1212,
    };
    (level_coeff * (188.3f64 * (0.05331 * f64::from(victim_rank)).exp()) * f64::from(diff_factor))
        as f32
}

/// `HonorMgr::CalculateRank(rankPoints, totalHK).rank`
/// (totalHK unused by the fork body).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn calculate_rank(rank_points: f32) -> u8 {
    if rank_points == 0.0 {
        return 0;
    }
    let positive = rank_points > 0.0;
    let rp = if positive { rank_points } else { -rank_points };
    let rank_count: u32 = if positive { 15 - 2 } else { 4 };
    let first_rank: u32 = if positive { 4 + 1 } else { 1 };
    if rp < 2000.0 {
        return if positive { first_rank as u8 } else { 4 };
    }
    if rp > (rank_count - 1) as f32 * 5000.0 {
        return if positive { 19 - 1 } else { first_rank as u8 };
    }
    let mut rank = (rp / 5000.0) as u32 + first_rank;
    rank = if positive { rank + 1 } else { 4 - rank };
    rank as u8
}

/// Fields `BuildBattleGroundStatusPacket` reads from a BG.
#[derive(Clone, Copy, Debug)]
pub struct BgStatusInfo {
    pub map_id: u32,
    pub bracket_id: u32,
    pub client_instance_id: u32,
}

/// `BattleGroundMgr::BuildBattleGroundStatusPacket`
/// (post-1.8.4 layout: queue slot first).
#[must_use]
pub fn build_battleground_status_packet(
    info: Option<BgStatusInfo>,
    queue_slot: u32,
    status_id: u32,
    time1: u32,
    time2: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    match info {
        None => {
            writer.write_u32_le(queue_slot);
            writer.write_u32_le(0);
        }
        Some(info) if status_id == 0 => {
            // fork: `statusId == 0 || !bg` short form.
            let _ = info;
            writer.write_u32_le(queue_slot);
            writer.write_u32_le(0);
        }
        Some(info) => {
            writer.write_u32_le(queue_slot);
            writer.write_u32_le(info.map_id);
            writer.write_u8(info.bracket_id as u8);
            writer.write_u32_le(info.client_instance_id);
            writer.write_u32_le(status_id);
            match status_id {
                bg_status::WAIT_QUEUE | bg_status::IN_PROGRESS => {
                    writer.write_u32_le(time1);
                    writer.write_u32_le(time2);
                }
                bg_status::WAIT_JOIN => {
                    writer.write_u32_le(time1);
                }
                _ => {}
            }
        }
    }
    let mut packet = WorldPacket::new(op::SMSG_BATTLEFIELD_STATUS);
    packet.take_writer(writer);
    packet
}

/// `BattleGroundMgr::BuildGroupJoinedBattlegroundPacket`.
#[must_use]
pub fn build_group_joined_packet(status: i32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(status as u32);
    let mut packet = WorldPacket::new(op::SMSG_GROUP_JOINED_BATTLEGROUND);
    packet.take_writer(writer);
    packet
}

/// `BattleGroundMgr::BuildPlaySoundPacket`.
#[must_use]
pub fn build_play_sound_packet(sound_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(sound_id);
    let mut packet = WorldPacket::new(op::SMSG_PLAY_SOUND);
    packet.take_writer(writer);
    packet
}

/// `BattleGroundMgr::BuildUpdateWorldStatePacket`.
#[must_use]
pub fn build_update_world_state_packet(field: u32, value: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(field);
    writer.write_u32_le(value);
    let mut packet = WorldPacket::new(op::SMSG_UPDATE_WORLD_STATE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_CLIENT_CONTROL_UPDATE` (`Player::SetClientControl`,
/// post-1.9.4 layout: packed guid + allowMove).
#[must_use]
pub fn build_client_control_packet(target: ObjectGuid, allow_move: bool) -> WorldPacket {
    let mut writer = PacketWriter::new();
    let mut packed_guid = Vec::new();
    target.write_packed(&mut packed_guid);
    writer.write_bytes(&packed_guid);
    writer.write_u8(u8::from(allow_move));
    let mut packet = WorldPacket::new(op::SMSG_CLIENT_CONTROL_UPDATE);
    packet.take_writer(writer);
    packet
}

impl Map {
    /// The BG status-packet projection of this map's BG.
    #[must_use]
    pub fn bg_status_info(&self) -> Option<BgStatusInfo> {
        self.bg.as_ref().map(|bg| BgStatusInfo {
            map_id: self.map_id,
            bracket_id: bg.bracket_id,
            client_instance_id: bg.client_instance_id,
        })
    }

    /// Status packet for one queued/in-BG player
    /// against this map's BG.
    #[must_use]
    pub fn bg_build_status_packet(
        &self,
        queue_slot: u32,
        status_id: u32,
        time1: u32,
        time2: u32,
    ) -> WorldPacket {
        build_battleground_status_packet(self.bg_status_info(), queue_slot, status_id, time1, time2)
    }

    fn bg_queue_slot(&self, player: ObjectGuid) -> u32 {
        let queue_type = self
            .bg
            .as_ref()
            .map_or(0, |bg| queue_type_of(bg.bg_type_id));
        self.player(player)
            .map_or(3, |p| p.battleground.queue_index(queue_type))
    }

    /// `SendPacketToTeam` (sender exclusion when
    /// `self_send` is false).
    pub(crate) fn bg_send_to_team(
        &mut self,
        team: Team,
        packet: &WorldPacket,
        sender: Option<ObjectGuid>,
        self_send: bool,
    ) {
        let targets: Vec<ObjectGuid> = match self.bg.as_ref() {
            Some(bg) => bg
                .players
                .iter()
                .filter(|(_, info)| info.team == team)
                .map(|(guid, _)| *guid)
                .filter(|guid| self_send || Some(*guid) != sender)
                .collect(),
            None => return,
        };
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `SendPacketToAll`.
    pub(crate) fn bg_send_to_all(&mut self, packet: &WorldPacket) {
        let targets: Vec<ObjectGuid> = match self.bg.as_ref() {
            Some(bg) => bg.players.keys().copied().collect(),
            None => return,
        };
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `PlaySoundToAll`.
    pub(crate) fn bg_play_sound_to_all(&mut self, sound_id: u32) {
        let packet = build_play_sound_packet(sound_id);
        self.bg_send_to_all(&packet);
    }

    /// `PlaySoundToTeam`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_play_sound_to_team(&mut self, sound_id: u32, team: Team) {
        let packet = build_play_sound_packet(sound_id);
        self.bg_send_to_team(team, &packet, None, true);
    }

    /// `UpdateWorldState`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_update_world_state(&mut self, field: u32, value: u32) {
        let packet = build_update_world_state_packet(field, value);
        self.bg_send_to_all(&packet);
    }

    /// `UpdateWorldStateForPlayer`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_update_world_state_for(&mut self, field: u32, value: u32, player: ObjectGuid) {
        self.events.push(MapEvent::Packet {
            to: player,
            packet: build_update_world_state_packet(field, value),
        });
    }

    /// `BlockMovement`.
    pub(crate) fn bg_block_movement(&mut self, player: ObjectGuid) {
        self.events.push(MapEvent::Packet {
            to: player,
            packet: build_client_control_packet(player, false),
        });
    }

    /// `SendMessageToAll` — broadcast-text chat to every
    /// BG player (`BattleGroundBroadcastBuilder`:
    /// `GetBroadcastText` + `BuildChatPacket` default arm).
    ///
    /// UNMODELED: per-locale text selection (single
    /// locale store server-wide today).
    pub(crate) fn bg_send_message_to_all(&mut self, entry: i32, source: Option<ObjectGuid>) {
        self.bg_send_message_typed(entry, CHAT_MSG_BG_SYSTEM_NEUTRAL, source);
    }

    pub(crate) fn bg_send_message_typed(
        &mut self,
        entry: i32,
        msg_type: u8,
        source: Option<ObjectGuid>,
    ) {
        if entry == 0 {
            return;
        }
        let Some(text) = self.bg_broadcast_text_of(entry, source) else {
            return;
        };
        let packet = build_bg_chat_packet(msg_type, &text, source);
        self.bg_send_to_all(&packet);
    }

    /// `SendMessage2ToAll` — broadcast text with up to
    /// two nested broadcast-text substitutions
    /// (`BattleGround2ChatBuilder` $N placeholders are
    /// resolved client-side from the arg ids; fork
    /// substitutes `GetBroadcastText(arg)` server-side
    /// into `%s` slots — resolved here the same way).
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_send_message2_to_all(
        &mut self,
        entry: i32,
        msg_type: u8,
        source: Option<ObjectGuid>,
        arg1: i32,
        arg2: i32,
    ) {
        if entry == 0 {
            return;
        }
        let Some(mut text) = self.bg_broadcast_text_of(entry, source) else {
            return;
        };
        for arg in [arg1, arg2] {
            if arg != 0
                && let Some(sub) = self.bg_broadcast_text_of(arg, source)
                && let Some(pos) = text.find("%s")
            {
                text.replace_range(pos..pos + 2, &sub);
            }
        }
        let packet = build_bg_chat_packet(msg_type, &text, source);
        self.bg_send_to_all(&packet);
    }

    fn bg_broadcast_text_of(&self, entry: i32, source: Option<ObjectGuid>) -> Option<String> {
        let bct = self
            .stores
            .scripts
            .broadcast_texts
            .get(&entry.unsigned_abs())?;
        // Gender pick mirrors `script_text_of` (source
        // BYTES_0 byte 2); without a source the male
        // text is used.
        let female = source.is_some_and(|g| {
            (self.object_get_field(
                g,
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_BYTES_0,
            ) >> 16)
                & 0xFF
                == 1
        });
        if female && !bct.female_text.is_empty() {
            Some(bct.female_text.clone())
        } else {
            Some(bct.male_text.clone())
        }
    }

    /// `PSendMessageToAll` (`mangos_string` + one %u arg;
    /// resolved actor-side).
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_psend_lang_to_all(
        &mut self,
        entry: i32,
        msg_type: u32,
        source: Option<ObjectGuid>,
        arg: Option<u32>,
    ) {
        self.events.push(MapEvent::BgLangMessage {
            entry,
            msg_type,
            source,
            arg,
        });
    }

    /// Subclass `Update` + `BattleGround::Update`:
    /// every fork subclass runs its own logic first
    /// and calls the base at the end.
    pub(crate) fn bg_update(&mut self, diff_ms: u32) {
        if self.bg.is_none() {
            return;
        }
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_update(diff_ms),
            Some(3) => self.ab_update(diff_ms),
            Some(1) => self.av_update(diff_ms),
            _ => {}
        }
        self.bg_update_base(diff_ms);
    }

    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    fn bg_update_base(&mut self, diff_ms: u32) {
        let Some(bg) = self.bg.as_ref() else { return };
        if bg.players.is_empty() {
            if bg.invited_count(Team::Horde) == 0 && bg.invited_count(Team::Alliance) == 0 {
                self.events.push(MapEvent::BgDelete);
            } else if bg.status <= bg_status::WAIT_JOIN && bg.create_unix + 120 < self.now_unix {
                let (qt, ty, br) = (queue_type_of(bg.bg_type_id), bg.bg_type_id, bg.bracket_id);
                self.events.push(MapEvent::BgScheduleQueueUpdate {
                    queue_type_id: qt,
                    bg_type_id: ty,
                    bracket_id: br,
                });
            }
            return;
        }

        // Premature-finish balance system (non-AV).
        let premature_ms = self.config.bg_premature_finish_ms;
        let mut end_winner: Option<Team> = None;
        if let Some(bg) = self.bg.as_mut() {
            if bg.bg_type_id != 1
                && bg.status == bg_status::IN_PROGRESS
                && premature_ms != 0
                && (bg.players_count_by_team(Team::Alliance) < bg.min_players_per_team
                    || bg.players_count_by_team(Team::Horde) < bg.min_players_per_team)
            {
                if !bg.premature_countdown {
                    bg.premature_countdown = true;
                    bg.premature_countdown_timer_ms = premature_ms;
                } else if bg.premature_countdown_timer_ms < diff_ms {
                    let winner =
                        if bg.players_count_by_team(Team::Alliance) >= bg.min_players_per_team {
                            Team::Alliance
                        } else if bg.players_count_by_team(Team::Horde) >= bg.min_players_per_team {
                            Team::Horde
                        } else {
                            Team::None
                        };
                    end_winner = Some(winner);
                    bg.premature_countdown = false;
                } else {
                    let newtime = bg.premature_countdown_timer_ms - diff_ms;
                    let old = bg.premature_countdown_timer_ms;
                    if newtime > 60_000 {
                        if newtime / 60_000 != old / 60_000 {
                            self.events.push(MapEvent::BgLangMessage {
                                entry: LANG_BG_PREMATURE_MIN,
                                msg_type: CHAT_MSG_SYSTEM,
                                source: None,
                                arg: Some(old / 60_000),
                            });
                        }
                    } else if newtime / 15_000 != old / 15_000 {
                        self.events.push(MapEvent::BgLangMessage {
                            entry: LANG_BG_PREMATURE_SECS,
                            msg_type: CHAT_MSG_SYSTEM,
                            source: None,
                            arg: Some(old / 1000),
                        });
                    }
                    bg.premature_countdown_timer_ms = newtime;
                }
            } else if bg.premature_countdown {
                bg.premature_countdown = false;
            }
        }
        if let Some(winner) = end_winner {
            self.bg_end_battleground(winner);
        }

        // Starting system.
        self.bg_update_starting(diff_ms);

        // Door despawn after start + 180s.
        let despawn_doors = self.bg.as_ref().is_some_and(|bg| {
            bg.events & BG_DOORS_DESPAWNED == 0
                && bg.status == bg_status::IN_PROGRESS
                && bg.start_time_ms > 180_000
        });
        if despawn_doors {
            self.bg_starting_despawn_doors();
            if let Some(bg) = self.bg.as_mut() {
                bg.events |= BG_DOORS_DESPAWNED;
            }
        }

        // Ending system.
        let eject_all = {
            let Some(bg) = self.bg.as_mut() else { return };
            if bg.status == bg_status::WAIT_LEAVE {
                bg.end_time_ms -= diff_ms as i32;
                if bg.end_time_ms <= 0 {
                    bg.end_time_ms = 0;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if eject_all {
            let guids: Vec<ObjectGuid> = self
                .bg
                .as_ref()
                .map(|bg| bg.players.keys().copied().collect())
                .unwrap_or_default();
            for guid in guids {
                self.bg_remove_player_at_leave(guid, true, true);
            }
        }

        if let Some(bg) = self.bg.as_mut() {
            bg.start_time_ms = bg.start_time_ms.saturating_add(diff_ms);
        }
    }

    #[allow(clippy::cast_possible_wrap)]
    fn bg_update_starting(&mut self, diff_ms: u32) {
        let starting = self
            .bg
            .as_ref()
            .is_some_and(|bg| bg.status == bg_status::WAIT_JOIN && !bg.players.is_empty());
        if !starting {
            return;
        }
        if let Some(bg) = self.bg.as_mut() {
            bg.start_delay_time_ms -= diff_ms as i32;
        }
        let Some(bg) = self.bg.as_ref() else { return };
        if bg.events & BG_STARTING_EVENT_1 == 0 {
            if let Some(bg) = self.bg.as_mut() {
                bg.events |= BG_STARTING_EVENT_1;
            }
            if !self.bg_setup() {
                self.bg_end_now();
                return;
            }
            self.bg_starting_close_doors();
            let (delay, msg) = {
                let bg = self.bg.as_ref().expect("bg");
                (bg.start_delay_times[0], bg.start_message_ids[0])
            };
            if let Some(bg) = self.bg.as_mut() {
                bg.start_delay_time_ms = delay;
            }
            if msg != 0 {
                self.bg_send_message_to_all(msg, None);
            }
        } else if bg.start_delay_time_ms <= bg.start_delay_times[1]
            && bg.events & BG_STARTING_EVENT_2 == 0
        {
            let msg = bg.start_message_ids[1];
            if let Some(bg) = self.bg.as_mut() {
                bg.events |= BG_STARTING_EVENT_2;
            }
            self.bg_send_message_to_all(msg, None);
        } else if bg.start_delay_time_ms <= bg.start_delay_times[2]
            && bg.events & BG_STARTING_EVENT_3 == 0
        {
            let msg = bg.start_message_ids[2];
            if let Some(bg) = self.bg.as_mut() {
                bg.events |= BG_STARTING_EVENT_3;
            }
            self.bg_send_message_to_all(msg, None);
        } else if bg.start_delay_time_ms <= 0 && bg.events & BG_STARTING_EVENT_4 == 0 {
            let msg = bg.start_message_ids[3];
            let announce = {
                let bg = self.bg.as_mut().expect("bg");
                bg.events |= BG_STARTING_EVENT_4;
                bg.status = bg_status::IN_PROGRESS;
                bg.start_delay_time_ms = bg.start_delay_times[3];
                (bg.name.clone(), bg.level_min, bg.level_max)
            };
            self.bg_starting_open_doors();
            self.bg_return_players_to_home_gy();
            self.bg_send_message_to_all(msg, None);
            self.bg_play_sound_to_all(SOUND_BG_START);
            // `CONFIG_BOOL_BATTLEGROUND_QUEUE_ANNOUNCER_START`
            // is read actor-side; the event is emitted
            // unconditionally and gated there.
            self.events.push(MapEvent::BgWorldText {
                text: format!(
                    "\u{1}bgstart\u{1}{}\u{1}{}\u{1}{}",
                    announce.0, announce.1, announce.2
                ),
            });
        }
    }

    /// `SetupBattleGround` dispatch (false -> `EndNow`).
    fn bg_setup(&mut self) -> bool {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_setup(),
            Some(3) => self.ab_setup(),
            Some(1) => self.av_setup(),
            _ => true,
        }
    }

    fn bg_starting_close_doors(&mut self) {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_starting_close_doors(),
            Some(3) => self.ab_starting_close_doors(),
            Some(1) => self.av_starting_close_doors(),
            _ => {}
        }
    }

    fn bg_starting_open_doors(&mut self) {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_starting_open_doors(),
            Some(3) => self.ab_starting_open_doors(),
            Some(1) => self.av_starting_open_doors(),
            _ => {}
        }
    }

    /// `StartingEventDespawnDoors` base.
    fn bg_starting_despawn_doors(&mut self) {
        let active = self
            .bg
            .as_ref()
            .is_some_and(|bg| bg.active_events.get(&BG_EVENT_DOOR) == Some(&0));
        if !active {
            return;
        }
        let doors: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(BG_EVENT_DOOR, 0)))
            .map(|o| o.gameobjects.clone())
            .unwrap_or_default();
        for guid in doors {
            if self.gameobjects.contains_key(&guid) {
                self.despawn_gameobject_now(guid);
            }
        }
    }

    /// `ReturnPlayersToHomeGY`.
    fn bg_return_players_to_home_gy(&mut self) {
        let Some(bg) = self.bg.as_ref() else { return };
        let players: Vec<ObjectGuid> = bg.players.keys().copied().collect();
        for guid in players {
            let Some(player) = self.player(guid) else {
                continue;
            };
            // UNMODELED: GM skip (`IsGameMaster`) — no GM
            // flag read on this path yet.
            let team = player.team;
            let pos = player.unit.world.position();
            let start = self.bg.as_ref().expect("bg").team_start_pos(team);
            let dx = pos.x - start.x;
            let dy = pos.y - start.y;
            let dz = pos.z - start.z;
            if dx * dx + dy * dy + dz * dz >= 100.0 * 100.0 {
                self.bg_repop_at_graveyard(guid);
            }
        }
    }

    /// `Player::RepopAtGraveyard` BG arm: teleport the
    /// (ghost or misplaced living) player to the
    /// closest graveyard safe loc on this map.
    ///
    /// UNMODELED: corpse creation / ghost aura side
    /// effects of the full repop (player death ghost
    /// flow itself is a recorded pre-existing gap).
    pub(crate) fn bg_repop_at_graveyard(&mut self, player: ObjectGuid) {
        let Some(loc) = self.bg_closest_graveyard(player) else {
            return;
        };
        let pos = Position {
            x: loc.x,
            y: loc.y,
            z: loc.z,
            o: loc.orientation,
        };
        if loc.map_id == self.map_id {
            self.teleport_player_in_map(player, pos);
        } else {
            self.events.push(MapEvent::TeleportPlayer {
                player,
                map_id: loc.map_id,
                position: pos,
            });
        }
    }

    /// `BattleGround::GetClosestGraveYard` (base;
    /// AV overrides in its step).
    pub(crate) fn bg_closest_graveyard(&mut self, player: ObjectGuid) -> Option<WorldSafeLoc> {
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 1) {
            return self.av_closest_graveyard(player);
        }
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 2) {
            return self.ws_closest_graveyard(player);
        }
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 3) {
            return self.ab_closest_graveyard(player);
        }
        let p = self.player(player)?;
        let pos = p.unit.world.position();
        let team = p.team;
        let (zone_id, area_id) = self.zone_and_area_id(pos);
        self.closest_graveyard_for_area(area_id, pos, team)
            .or_else(|| {
                if area_id == zone_id {
                    None
                } else {
                    self.closest_graveyard_for_area(zone_id, pos, team)
                }
            })
    }

    /// `ObjectMgr::GetClosestGraveYardForArea`.
    fn closest_graveyard_for_area(
        &self,
        area_or_zone: u32,
        pos: Position,
        team: Team,
    ) -> Option<WorldSafeLoc> {
        let links = self.stores.graveyard_links.for_zone(area_or_zone);
        if links.is_empty() {
            return None;
        }
        let team_db = mangos_entities::battleground::team_db_value(team);
        let temp_entry = self.stores.maps.get(self.map_id);
        let mut near: Option<(f32, WorldSafeLoc)> = None;
        let mut entr: Option<(f32, WorldSafeLoc)> = None;
        let mut far: Option<WorldSafeLoc> = None;
        for link in links {
            let Some(entry) = self.stores.safe_locs.get(link.safe_loc_id) else {
                continue;
            };
            if link.team != 0 && team_db != 0 && link.team != team_db {
                continue;
            }
            if self.map_id == entry.map_id {
                let dist2 = (entry.x - pos.x).powi(2)
                    + (entry.y - pos.y).powi(2)
                    + (entry.z - pos.z).powi(2);
                if near.is_none_or(|(d, _)| dist2 < d) {
                    near = Some((dist2, entry));
                }
            } else {
                let entrance_ok = temp_entry.is_some_and(|t| {
                    t.ghost_entrance_map >= 0
                        && t.ghost_entrance_map as u32 == entry.map_id
                        && (t.ghost_entrance_x != 0.0 || t.ghost_entrance_y != 0.0)
                });
                if !entrance_ok {
                    far = Some(entry);
                    continue;
                }
                let t = temp_entry.expect("entry");
                let dist2 =
                    (entry.x - t.ghost_entrance_x).powi(2) + (entry.y - t.ghost_entrance_y).powi(2);
                if entr.is_none_or(|(d, _)| dist2 < d) {
                    entr = Some((dist2, entry));
                }
            }
        }
        if let Some((_, e)) = near {
            return Some(e);
        }
        if let Some((_, e)) = entr {
            return Some(e);
        }
        far
    }
}

/// `CHAT_MSG_SYSTEM` (0x01) / `CHAT_MSG_BG_SYSTEM_NEUTRAL` (0x52).
pub const CHAT_MSG_SYSTEM: u32 = 0x01;
pub const CHAT_MSG_BG_SYSTEM_NEUTRAL: u8 = 0x52;
/// `LANG_BATTLEGROUND_PREMATURE_FINISH_WARNING` (750/751).
const LANG_BG_PREMATURE_MIN: i32 = 750;
const LANG_BG_PREMATURE_SECS: i32 = 751;
/// `BG_EVENT_DOOR` / starting-event bits.
pub const BG_EVENT_DOOR: u8 = 254;
const BG_STARTING_EVENT_1: u8 = 0x01;
const BG_STARTING_EVENT_2: u8 = 0x02;
pub(crate) const BG_STARTING_EVENT_3: u8 = 0x04;
const BG_STARTING_EVENT_4: u8 = 0x08;
const BG_DOORS_DESPAWNED: u8 = 0x10;

/// `ChatHandler::BuildChatPacket` default arm for the
/// BG system types (sender guid once, `LANG_UNIVERSAL`).
#[must_use]
pub fn build_bg_chat_packet(msg_type: u8, text: &str, source: Option<ObjectGuid>) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(msg_type);
    writer.write_u32_le(0); // LANG_UNIVERSAL
    writer.write_u64_le(source.map_or(0, ObjectGuid::raw));
    writer.write_u32_le(text.len() as u32 + 1);
    writer.write_cstring(text);
    writer.write_u8(0); // CHAT_TAG_NONE
    let mut packet = WorldPacket::new(op::SMSG_MESSAGECHAT);
    packet.take_writer(writer);
    packet
}

/// `SMSG_BATTLEGROUND_PLAYER_JOINED` / `_LEFT` builders.
#[must_use]
pub fn build_bg_player_joined_packet(player: ObjectGuid) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(player.raw());
    let mut packet = WorldPacket::new(op::SMSG_BATTLEGROUND_PLAYER_JOINED);
    packet.take_writer(writer);
    packet
}

#[must_use]
pub fn build_bg_player_left_packet(player: ObjectGuid) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(player.raw());
    let mut packet = WorldPacket::new(op::SMSG_BATTLEGROUND_PLAYER_LEFT);
    packet.take_writer(writer);
    packet
}

/// Power indexes (`Powers`): mana 0, rage 1, energy 3.
const POWER_MANA: usize = 0;
const POWER_RAGE: usize = 1;
const POWER_ENERGY: usize = 3;
/// `UNIT_FLAG_SKINNABLE` (0x04000000).
const UNIT_FLAG_SKINNABLE: u32 = 0x0400_0000;
/// Spirit of Redemption aura spell.
const SPELL_SPIRIT_OF_REDEMPTION: u32 = 27827;
/// `SPELL_AURA_SPIRIT_OF_REDEMPTION` aura type (79)? —
/// fork checks are by aura *type*; `HasAura(27827)` is
/// by spell. Both sites here mirror their fork forms.
const AURA_TYPE_SPIRIT_OF_REDEMPTION: u32 = 79;

impl Map {
    /// `CreateNewBattleGround` + `StartBattleGround`
    /// map side: attach a fresh match to this map.
    /// The actor registers the manager handle and the
    /// free-slot notice is emitted here.
    pub fn init_battleground(&mut self, init: BgInit) {
        let mut state = BgState::new(init, self.now_unix);
        // `StartBattleGround`: start time reset (ctor
        // already 0) + free-slot queue add.
        state.start_time_ms = 0;
        self.bg = Some(Box::new(state));
        self.bg_kind_construct();
        self.events
            .push(MapEvent::BgFreeSlotQueue { present: true });
    }

    fn bg_kind_construct(&mut self) {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_construct(),
            Some(3) => self.ab_construct(),
            Some(1) => self.av_construct(),
            _ => {}
        }
    }

    /// `IncreaseInvitedCount` (manager-driven).
    pub fn bg_increase_invited(&mut self, team: Team) {
        if let Some(bg) = self.bg.as_mut()
            && matches!(team, Team::Alliance | Team::Horde)
        {
            bg.invited[team_index(team)] += 1;
        }
    }

    /// `DecreaseInvitedCount` (fork asserts > 0; Rust
    /// saturates to keep a bad counter from panicking
    /// the map thread).
    pub fn bg_decrease_invited(&mut self, team: Team) {
        if let Some(bg) = self.bg.as_mut()
            && matches!(team, Team::Alliance | Team::Horde)
        {
            let slot = &mut bg.invited[team_index(team)];
            *slot = slot.saturating_sub(1);
        }
    }

    pub(crate) fn player_has_spirit_of_redemption(&self, guid: ObjectGuid) -> bool {
        self.player(guid).is_some_and(|p| {
            p.unit
                .auras
                .iter()
                .any(|a| a.spell_id == SPELL_SPIRIT_OF_REDEMPTION)
        })
    }

    fn player_has_sor_aura_type(&self, guid: ObjectGuid) -> bool {
        self.player(guid).is_some_and(|p| {
            p.unit.auras.iter().any(|a| {
                a.effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == AURA_TYPE_SPIRIT_OF_REDEMPTION)
            })
        })
    }

    /// `Player::ResurrectPlayer(pct, false)` subset +
    /// `SpawnCorpseBones` (UNMODELED: corpse/bones
    /// objects and ghost-form removal — the player
    /// ghost flow is a recorded pre-existing gap;
    /// interrupt/visibility tails ride the normal
    /// value-update pass).
    #[allow(clippy::cast_precision_loss)]
    pub fn bg_resurrect_player(&mut self, guid: ObjectGuid, restore_percent: f32) {
        let Some(p) = self.player_mut(guid) else {
            return;
        };
        p.unit.death_state = mangos_entities::unit::DeathState::Alive;
        if restore_percent > 0.0 {
            let max_hp = p.unit.max_health();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            p.unit.set_health((max_hp as f32 * restore_percent) as u32);
            let max_mana = p.unit.max_power(POWER_MANA);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            p.unit
                .set_power(POWER_MANA, (max_mana as f32 * restore_percent) as u32);
            p.unit.set_power(POWER_RAGE, 0);
            let max_energy = p.unit.max_power(POWER_ENERGY);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            p.unit
                .set_power(POWER_ENERGY, (max_energy as f32 * restore_percent) as u32);
        }
    }

    /// `BattleGround::AddPlayer` base (called from the
    /// worldport-ack completion, fork
    /// `HandleMoveWorldportAckOpcode`).
    pub fn bg_add_player(&mut self, guid: ObjectGuid) {
        if self.bg.is_none() {
            return;
        }
        // Subclass `AddPlayer` pre-arm (score creation
        // + per-kind joins) runs before the base body
        // in the fork subclasses that have one.
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_add_player_pre(guid),
            Some(3) => self.ab_add_player_pre(guid),
            Some(1) => self.av_add_player_pre(guid),
            _ => {}
        }
        let team = self
            .player(guid)
            .map_or(Team::None, |p| p.battleground.bg_team_or(p.team));
        if let Some(bg) = self.bg.as_mut() {
            bg.players.insert(guid, BgPlayerInfo { team });
            bg.players_count[team_index(team)] += 1;
            bg.scores.entry(guid).or_default();
        }
        let packet = build_bg_player_joined_packet(guid);
        self.bg_send_to_team(team, &packet, Some(guid), false);
        self.bg_player_added_check_running(guid);
        self.events
            .push(MapEvent::BgRaidJoin { player: guid, team });
    }

    /// `PlayerAddedToBGCheckIfBGIsRunning`.
    fn bg_player_added_check_running(&mut self, guid: ObjectGuid) {
        let Some(bg) = self.bg.as_ref() else { return };
        if bg.status != bg_status::WAIT_LEAVE {
            return;
        }
        self.bg_block_movement(guid);
        let packet = self.bg_build_pvp_log_data();
        self.events.push(MapEvent::Packet { to: guid, packet });
        let slot = self.bg_queue_slot(guid);
        let (end, start) = self
            .bg
            .as_ref()
            .map_or((0, 0), |b| (b.end_time_ms.max(0) as u32, b.start_time_ms));
        let packet = self.bg_build_status_packet(slot, bg_status::IN_PROGRESS, end, start);
        self.events.push(MapEvent::Packet { to: guid, packet });
    }

    /// `BuildPvpLogDataPacket`.
    #[must_use]
    pub fn bg_build_pvp_log_data(&self) -> WorldPacket {
        let mut writer = PacketWriter::new();
        let Some(bg) = self.bg.as_ref() else {
            writer.write_u8(0);
            writer.write_u32_le(0);
            let mut packet = WorldPacket::new(op::MSG_PVP_LOG_DATA);
            packet.take_writer(writer);
            return packet;
        };
        if bg.status == bg_status::WAIT_LEAVE {
            writer.write_u8(1);
            writer.write_u8(bg.winner as u8);
        } else {
            writer.write_u8(0);
        }
        // Fork iterates the score std::map (guid-ordered);
        // cap 80 (client hard limit).
        let mut rows: Vec<(ObjectGuid, BgScore)> =
            bg.scores.iter().map(|(g, s)| (*g, *s)).collect();
        rows.sort_by_key(|(g, _)| g.raw());
        let count = rows.len().min(80);
        writer.write_u32_le(count as u32);
        for (guid, score) in rows.into_iter().take(count) {
            writer.write_u64_le(guid.raw());
            let rank = self
                .player(guid)
                .map_or(4, |p| u32::from(calculate_rank(p.honor_rank_points)));
            writer.write_u32_le(rank);
            writer.write_u32_le(score.killing_blows);
            writer.write_u32_le(score.honorable_kills);
            writer.write_u32_le(score.deaths);
            writer.write_u32_le(score.bonus_honor);
            match bg.bg_type_id {
                1 => {
                    writer.write_u32_le(7);
                    writer.write_u32_le(score.graveyards_assaulted);
                    writer.write_u32_le(score.graveyards_defended);
                    writer.write_u32_le(score.towers_assaulted);
                    writer.write_u32_le(score.towers_defended);
                    writer.write_u32_le(score.secondary_objectives);
                    writer.write_u32_le(score.lieutenant_count);
                    writer.write_u32_le(score.secondary_npc);
                }
                2 => {
                    writer.write_u32_le(2);
                    writer.write_u32_le(score.flag_captures);
                    writer.write_u32_le(score.flag_returns);
                }
                3 => {
                    writer.write_u32_le(2);
                    writer.write_u32_le(score.bases_assaulted);
                    writer.write_u32_le(score.bases_defended);
                }
                _ => {
                    writer.write_u32_le(0);
                }
            }
        }
        let mut packet = WorldPacket::new(op::MSG_PVP_LOG_DATA);
        packet.take_writer(writer);
        packet
    }

    /// `UpdatePlayerScore` base + per-kind extra types.
    pub fn bg_update_player_score(&mut self, source: ObjectGuid, score_type: u32, value: u32) {
        if self
            .bg
            .as_ref()
            .is_none_or(|bg| !bg.scores.contains_key(&source))
        {
            return;
        }
        // Per-kind types first (fork subclass switch
        // falls through to base for the shared types).
        let handled = match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_update_score(source, score_type, value),
            Some(3) => self.ab_update_score(source, score_type, value),
            Some(1) => self.av_update_score(source, score_type, value),
            _ => false,
        };
        if handled {
            return;
        }
        match score_type {
            score_type::BONUS_HONOR => {
                if self.bg_honor_add(source, f32::from(value as u16))
                    && let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    score.bonus_honor += value;
                }
            }
            score_type::KILLING_BLOWS | score_type::DEATHS | score_type::HONORABLE_KILLS => {
                if let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    match score_type {
                        score_type::KILLING_BLOWS => score.killing_blows += value,
                        score_type::DEATHS => score.deaths += value,
                        _ => score.honorable_kills += value,
                    }
                }
            }
            _ => {}
        }
    }

    /// `HonorMgr::Add(cp, BONUS)` subset: appends a
    /// `character_honor_cp` row (the save diff inserts
    /// it, fork `STATE_NEW`). Returns false at cp 0.
    ///
    /// UNMODELED (Slice 4 remainder, reached here):
    /// `HonorMgr` rank/standing recomputation, daily/
    /// weekly totals and `SMSG_PVP_CREDIT` for bonus
    /// honor (fork sends none for BONUS adds either —
    /// `SendPVPCredit` is kill-side).
    pub(crate) fn bg_honor_add(&mut self, player: ObjectGuid, cp: f32) -> bool {
        if cp == 0.0 {
            return false;
        }
        #[allow(clippy::cast_possible_truncation)]
        let day = (self.now_unix / 86_400) as u32;
        let Some(p) = self.player_mut(player) else {
            return false;
        };
        p.honor_add_bonus(cp, day);
        true
    }

    /// `RewardHonorToTeam`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_reward_honor_to_team(&mut self, honor: u32, team: Team) {
        let targets: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .map(|bg| {
                bg.players
                    .iter()
                    .filter(|(_, i)| i.team == team)
                    .map(|(g, _)| *g)
                    .collect()
            })
            .unwrap_or_default();
        for guid in targets {
            self.bg_update_player_score(guid, score_type::BONUS_HONOR, honor);
        }
    }

    /// `RewardReputationToTeam` (row-delta subset —
    /// fork `ModifyFactionReputation` also refreshes
    /// client fields/packets; no runtime reputation
    /// manager exists yet, same as quest rewards).
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_reward_reputation_to_team(
        &mut self,
        faction_id: u32,
        reputation: i32,
        team: Team,
    ) {
        let targets: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .map(|bg| {
                bg.players
                    .iter()
                    .filter(|(_, i)| i.team == team)
                    .map(|(g, _)| *g)
                    .collect()
            })
            .unwrap_or_default();
        for guid in targets {
            if let Some(p) = self.player_mut(guid) {
                p.modify_reputation_row(faction_id, reputation);
            }
        }
    }

    /// `CastSpellOnTeam`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_cast_spell_on_team(&mut self, spell_id: u32, team: Team) {
        let targets: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .map(|bg| {
                bg.players
                    .iter()
                    .filter(|(_, i)| i.team == team)
                    .map(|(g, _)| *g)
                    .collect()
            })
            .unwrap_or_default();
        for guid in targets {
            self.bg_reward_spell_cast(guid, spell_id);
        }
    }

    /// `RewardSpellCast` (triggered self-cast; bots are
    /// excluded by `RewardMark`'s `IsBot` gate — Rust
    /// bot players ride real sessions on this fork, so
    /// the gate has no distinct runtime population:
    /// recorded).
    pub fn bg_reward_spell_cast(&mut self, player: ObjectGuid, spell_id: u32) {
        if spell_id == 0 || self.stores.spells.get(spell_id).is_none() {
            return;
        }
        let _ = self.cast_spell(
            player,
            spell_id,
            crate::spells::CastTarget::Unit(player),
            crate::spells::CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
    }

    /// `RewardMark`.
    pub(crate) fn bg_reward_mark(&mut self, player: ObjectGuid, winner: bool) {
        let Some(bg) = self.bg.as_ref() else { return };
        let team = self.player(player).map_or(Team::None, |p| p.team);
        let spell = match (team, winner) {
            (Team::Horde, true) => bg.horde_win_spell,
            (Team::Horde, false) => bg.horde_lose_spell,
            (_, true) => bg.alliance_win_spell,
            (_, false) => bg.alliance_lose_spell,
        };
        self.bg_reward_spell_cast(player, spell);
    }

    /// `RewardItem` subset: inventory grant via the
    /// actor item path. Fork-dead code (step-6
    /// census: zero `RewardItem` callers fork-wide;
    /// its `SendRewardMarkByMail` overflow is
    /// likewise unreachable — BG marks arrive via
    /// the win/lose spells' `CREATE_ITEM` effects
    /// 24950-24955 → `DoCreateItem`, whose overflow
    /// drops). Kept as the kind-step grant seam.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_reward_item(&mut self, player: ObjectGuid, item_id: u32, count: u32) {
        if count == 0 || self.stores.items.get(item_id).is_none() {
            return;
        }
        self.events.push(MapEvent::CreateItem {
            player,
            item_id,
            count,
        });
    }

    /// `EndNow`.
    pub(crate) fn bg_end_now(&mut self) {
        self.events
            .push(MapEvent::BgFreeSlotQueue { present: false });
        let final_packet = self.bg_build_pvp_log_data();
        if let Some(bg) = self.bg.as_mut() {
            bg.status = bg_status::WAIT_LEAVE;
            bg.end_time_ms = 0;
            if bg.final_score.is_none() {
                bg.final_score = Some(final_packet);
            }
        }
    }

    /// `EndBattleGround`.
    ///
    /// # Panics
    ///
    /// Panics only if the just-cached final-score
    /// packet vanished mid-call (unreachable: both
    /// reads happen on the same map tick).
    pub fn bg_end_battleground(&mut self, winner: Team) {
        // Fork subclass override reward stage runs
        // before the base ending.
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(1) => self.av_end_battleground_rewards(winner),
            Some(2) => self.ws_end_battleground_rewards(winner),
            Some(3) => self.ab_end_battleground_rewards(winner),
            _ => {}
        }
        self.events
            .push(MapEvent::BgFreeSlotQueue { present: false });
        match winner {
            Team::Alliance => {
                self.bg_play_sound_to_all(SOUND_ALLIANCE_WINS);
                if let Some(bg) = self.bg.as_mut() {
                    bg.winner = bg_winner::ALLIANCE;
                }
            }
            Team::Horde => {
                self.bg_play_sound_to_all(SOUND_HORDE_WINS);
                if let Some(bg) = self.bg.as_mut() {
                    bg.winner = bg_winner::HORDE;
                }
            }
            Team::None => {
                if let Some(bg) = self.bg.as_mut() {
                    bg.winner = bg_winner::NONE;
                }
            }
        }
        if let Some(bg) = self.bg.as_mut() {
            bg.status = bg_status::WAIT_LEAVE;
            bg.end_time_ms = TIME_TO_AUTOREMOVE_MS;
        }
        let final_packet = self.bg_build_pvp_log_data();
        if let Some(bg) = self.bg.as_mut()
            && bg.final_score.is_none()
        {
            bg.final_score = Some(final_packet);
        }
        let final_packet = self
            .bg
            .as_ref()
            .and_then(|b| b.final_score.clone())
            .expect("final score");
        let roster: Vec<(ObjectGuid, Team)> = self
            .bg
            .as_ref()
            .map(|bg| bg.players.iter().map(|(g, i)| (*g, i.team)).collect())
            .unwrap_or_default();
        let start_time = self.bg.as_ref().map_or(0, |b| b.start_time_ms);
        for (guid, team) in roster {
            if self.player(guid).is_none() {
                continue;
            }
            if self.player_has_sor_aura_type(guid) {
                self.remove_spells_causing_aura(guid, mangos_spells::defines::aura::MOD_SHAPESHIFT);
            }
            let alive = self.player(guid).is_some_and(|p| p.unit.is_alive());
            if alive.not() {
                self.bg_resurrect_player(guid, 1.0);
            } else {
                // `CombatStop` + hostile-ref cleanup:
                // attack_stop covers the swing side;
                // UNMODELED: threat-table purge.
                self.attack_stop(guid);
            }
            if team == winner {
                self.bg_reward_mark(guid, true);
            } else if start_time > 10 * 60 * 1000 {
                self.bg_reward_mark(guid, false);
            }
            // `CombatStopWithPets(true)`: pet attack stop
            // rides the pet unsummon on transport;
            // UNMODELED here.
            self.bg_block_movement(guid);
            self.events.push(MapEvent::Packet {
                to: guid,
                packet: final_packet.clone(),
            });
            let slot = self.bg_queue_slot(guid);
            let packet = self.bg_build_status_packet(
                slot,
                bg_status::IN_PROGRESS,
                TIME_TO_AUTOREMOVE_MS as u32,
                start_time,
            );
            self.events.push(MapEvent::Packet { to: guid, packet });
            // UNMODELED: `logs_battleground` insert
            // (`LogsDB.Battlegrounds` = 0 on the bench
            // conf; fork gate).
        }
        let win_text = self.bg.as_ref().map_or(0, |b| b.winner_text(winner));
        if win_text != 0 {
            self.bg_send_message_to_all(win_text, None);
        }
        let invited_left = self.bg.as_ref().is_some_and(|bg| {
            bg.invited_count(Team::Horde) != 0 || bg.invited_count(Team::Alliance) != 0
        });
        if invited_left && let Some(bg) = self.bg.as_ref() {
            self.events.push(MapEvent::BgScheduleQueueUpdate {
                queue_type_id: queue_type_of(bg.bg_type_id),
                bg_type_id: bg.bg_type_id,
                bracket_id: bg.bracket_id,
            });
        }
    }

    /// `RemovePlayerAtLeave` map-side bookkeeping; the
    /// transport itself is emitted for the actor.
    pub fn bg_remove_player_at_leave(
        &mut self,
        guid: ObjectGuid,
        transport: bool,
        send_packet: bool,
    ) {
        let Some(bg) = self.bg.as_ref() else { return };
        let mut team = bg.player_team(guid);
        let participant = bg.players.contains_key(&guid);
        if let Some(bg) = self.bg.as_mut() {
            if bg.players.remove(&guid).is_some() {
                let idx = team_index(team);
                bg.players_count[idx] = bg.players_count[idx].saturating_sub(1);
            }
            bg.scores.remove(&guid);
        }
        if self.player_has_sor_aura_type(guid) {
            self.remove_spells_causing_aura(guid, mangos_spells::defines::aura::MOD_SHAPESHIFT);
        }
        let alive = self.player(guid).is_some_and(|p| p.unit.is_alive());
        if self.player(guid).is_some() && !alive {
            self.bg_resurrect_player(guid, 1.0);
        }
        // BG subclass `RemovePlayer` hook.
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_remove_player(guid),
            Some(3) => self.ab_remove_player(guid),
            Some(1) => self.av_remove_player(guid),
            _ => {}
        }
        if participant {
            let (bg_type_id, bracket_id, status) = self
                .bg
                .as_ref()
                .map_or((0, 0, 0), |b| (b.bg_type_id, b.bracket_id, b.status));
            let queue_type = queue_type_of(bg_type_id);
            if self.player(guid).is_some() {
                if team == Team::None {
                    team = self.player(guid).map_or(Team::None, |p| p.team);
                }
                if send_packet {
                    let slot = self.bg_queue_slot(guid);
                    let packet =
                        build_battleground_status_packet(None, slot, bg_status::NONE, 0, 0);
                    self.events.push(MapEvent::Packet { to: guid, packet });
                }
                if let Some(p) = self.player_mut(guid) {
                    p.battleground.remove_queue_id(queue_type);
                }
            }
            self.events
                .push(MapEvent::BgRaidLeave { player: guid, team });
            self.bg_decrease_invited(team);
            if status < bg_status::WAIT_LEAVE {
                self.events
                    .push(MapEvent::BgFreeSlotQueue { present: true });
                self.events.push(MapEvent::BgScheduleQueueUpdate {
                    queue_type_id: queue_type,
                    bg_type_id,
                    bracket_id,
                });
                let packet = build_bg_player_left_packet(guid);
                self.bg_send_to_team(team, &packet, Some(guid), false);
            }
        }
        if let Some(p) = self.player_mut(guid) {
            p.battleground.set_battleground_id(0, 0);
            p.battleground.set_bg_team(Team::None);
        }
        if transport && self.player(guid).is_some() {
            self.events.push(MapEvent::BgTransportOut { player: guid });
        }
    }

    /// `HandleKillPlayer` base (AV wraps with its
    /// reinforcement decrement in its step).
    pub(crate) fn bg_handle_kill_player(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        if self.bg.is_none() {
            return;
        }
        if let Some(killer) = killer.filter(|k| k.is_player()) {
            let victim_faction = self.player(victim).map(|p| p.unit.faction_template());
            let killer_faction = self.player(killer).map(|p| p.unit.faction_template());
            if victim_faction.is_some() && victim_faction != killer_faction {
                self.bg_update_player_score(killer, score_type::HONORABLE_KILLS, 1);
                self.bg_update_player_score(killer, score_type::KILLING_BLOWS, 1);
                let killer_team = self.player(killer).map_or(Team::None, |p| p.team);
                let victim_pos = self.object_position(victim);
                let mates: Vec<ObjectGuid> = self
                    .bg
                    .as_ref()
                    .map(|bg| bg.players.keys().copied().collect())
                    .unwrap_or_default();
                for guid in mates {
                    if guid == killer {
                        continue;
                    }
                    let Some(p) = self.player(guid) else { continue };
                    if p.team != killer_team {
                        continue;
                    }
                    let in_range = match (self.object_position(guid), victim_pos) {
                        (Some(a), Some(b)) => {
                            let dx = a.x - b.x;
                            let dy = a.y - b.y;
                            dx * dx + dy * dy < 74.0 * 74.0
                        }
                        _ => false,
                    };
                    if in_range {
                        self.bg_update_player_score(guid, score_type::HONORABLE_KILLS, 1);
                    }
                }
            }
        }
        if !self.player_has_spirit_of_redemption(victim) {
            self.bg_update_player_score(victim, score_type::DEATHS, 1);
            if let Some(p) = self.player_mut(victim) {
                let values = p.unit.values();
                let flags =
                    values.get_u32(mangos_protocol::update_field_table::offsets::UNIT_FIELD_FLAGS);
                values.set_u32(
                    mangos_protocol::update_field_table::offsets::UNIT_FIELD_FLAGS,
                    flags | UNIT_FLAG_SKINNABLE,
                );
            }
        }
        // Per-kind kill hooks (AV reinforcement) run in
        // their kind functions via `bg_on_kill_player`.
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(1) => self.av_on_kill_player(victim, killer),
            Some(2) => self.ws_on_kill_player(victim, killer),
            Some(3) => self.ab_on_kill_player(victim, killer),
            _ => {}
        }
    }

    /// `HandleKillUnit` dispatch (creature victims).
    pub(crate) fn bg_handle_kill_unit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(1) => self.av_on_kill_unit(victim, killer),
            Some(2) => self.ws_on_kill_unit(victim, killer),
            Some(3) => self.ab_on_kill_unit(victim, killer),
            _ => {}
        }
    }

    /// `HandleAreaTrigger` dispatch; true suppresses
    /// the generic areatrigger handling (fork: the
    /// session checks the BG first).
    pub fn bg_handle_area_trigger(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_handle_area_trigger(player, trigger_id),
            Some(3) => self.ab_handle_area_trigger(player, trigger_id),
            Some(1) => self.av_handle_area_trigger(player, trigger_id),
            _ => false,
        }
    }

    /// `EventPlayerClickedOnFlag` dispatch (GO use path).
    pub(crate) fn bg_event_clicked_on_flag(&mut self, player: ObjectGuid, go: ObjectGuid) {
        match self.bg.as_ref().map(|b| b.bg_type_id) {
            Some(2) => self.ws_event_clicked_on_flag(player, go),
            Some(3) => self.ab_event_clicked_on_flag(player, go),
            Some(1) => self.av_event_clicked_on_flag(player, go),
            _ => {}
        }
    }

    /// `EventPlayerDroppedFlag` dispatch (flag aura
    /// removal / death paths call this in kind steps).
    #[allow(dead_code)] // consumed by the kind steps
    pub fn bg_event_dropped_flag(&mut self, player: ObjectGuid) {
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 2) {
            self.ws_event_dropped_flag(player);
        }
    }

    /// `OnObjectDBLoad(Creature*)` — register the spawn
    /// against its BG events and despawn it when any
    /// event is inactive.
    pub(crate) fn bg_on_creature_db_load(&mut self, guid: ObjectGuid) {
        if self.bg.is_none() {
            return;
        }
        let indexes: Vec<(u8, u8)> = self
            .stores
            .battle_event_indexes
            .creature_indexes(guid.counter())
            .to_vec();
        if indexes.is_empty() {
            return;
        }
        for (e1, e2) in &indexes {
            if let Some(bg) = self.bg.as_mut() {
                let objects = bg.event_objects.entry((*e1, *e2)).or_default();
                if !objects.creatures.contains(&guid) {
                    objects.creatures.push(guid);
                }
            }
            if !self.bg_is_active_event(*e1, *e2) {
                self.bg_spawn_creature(guid, spawn_mode::DESPAWN_FORCED);
            }
        }
    }

    /// `OnObjectDBLoad(GameObject*)`.
    pub(crate) fn bg_on_gameobject_db_load(&mut self, guid: ObjectGuid) {
        if self.bg.is_none() {
            return;
        }
        let indexes: Vec<(u8, u8)> = self
            .stores
            .battle_event_indexes
            .gameobject_indexes(guid.counter())
            .to_vec();
        if indexes.is_empty() {
            return;
        }
        for (e1, e2) in &indexes {
            if let Some(bg) = self.bg.as_mut() {
                let objects = bg.event_objects.entry((*e1, *e2)).or_default();
                if !objects.gameobjects.contains(&guid) {
                    objects.gameobjects.push(guid);
                }
            }
            if !self.bg_is_active_event(*e1, *e2) {
                self.bg_spawn_object(guid, RESPAWN_NEVER_MS);
            } else if self
                .bg
                .as_ref()
                .is_some_and(|b| b.status >= bg_status::IN_PROGRESS)
                && is_door(*e1, *e2)
            {
                self.bg_door_open(guid);
            }
        }
    }

    /// `IsActiveEvent`.
    pub(crate) fn bg_is_active_event(&self, event1: u8, event2: u8) -> bool {
        self.bg
            .as_ref()
            .and_then(|bg| bg.active_events.get(&event1).copied())
            == Some(event2)
    }

    /// `ActivateEventWithoutSpawn`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_activate_event(&mut self, event1: u8, event2: u8) {
        if let Some(bg) = self.bg.as_mut() {
            bg.active_events.insert(event1, event2);
        }
    }

    /// `GetSingleCreatureGuid`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_single_creature(&self, event1: u8, event2: u8) -> Option<ObjectGuid> {
        self.bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .and_then(|o| o.creatures.first().copied())
    }

    /// `GetSingleGameObjectGuid`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_single_gameobject(&self, event1: u8, event2: u8) -> Option<ObjectGuid> {
        self.bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .and_then(|o| o.gameobjects.first().copied())
    }

    /// `SpawnEvent`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_spawn_event(
        &mut self,
        event1: u8,
        event2: u8,
        spawn: bool,
        forced_despawn: bool,
        delay_ms: u32,
    ) {
        if event2 == BG_EVENT_NONE_VAL {
            return;
        }
        let current = self
            .bg
            .as_ref()
            .and_then(|bg| bg.active_events.get(&event1).copied())
            .unwrap_or(0);
        // fork indexes `m_activeEvents[event1]` (0 when
        // absent) for these compares.
        if (spawn && current == event2) || (!spawn && current != event2) {
            return;
        }
        if spawn {
            self.bg_spawn_event(event1, current, false, forced_despawn, 0);
            if let Some(bg) = self.bg.as_mut() {
                bg.active_events.insert(event1, event2);
            }
        } else if let Some(bg) = self.bg.as_mut() {
            bg.active_events.insert(event1, BG_EVENT_NONE_VAL);
        }
        let creatures: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .map(|o| o.creatures.clone())
            .unwrap_or_default();
        for guid in creatures {
            let indexes = self
                .stores
                .battle_event_indexes
                .creature_indexes(guid.counter())
                .to_vec();
            let mut spawn_this = spawn;
            for (i1, i2) in &indexes {
                if !self.bg_is_active_event(*i1, *i2) {
                    spawn_this = false;
                    break;
                }
            }
            let mode = if spawn_this {
                spawn_mode::RESPAWN_FORCED
            } else if forced_despawn {
                spawn_mode::DESPAWN_FORCED
            } else {
                spawn_mode::RESPAWN_STOP
            };
            self.bg_spawn_creature(guid, mode);
        }
        let objects: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .map(|o| o.gameobjects.clone())
            .unwrap_or_default();
        for guid in objects {
            self.bg_spawn_object(guid, if spawn { delay_ms } else { RESPAWN_NEVER_MS });
        }
        self.bg_on_event_state_changed(event1, event2, spawn);
    }

    /// `SetSpawnEventMode`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_set_spawn_event_mode(&mut self, event1: u8, event2: u8, mode: u8) {
        if event2 == BG_EVENT_NONE_VAL {
            return;
        }
        let is_spawn_mode = mode == spawn_mode::RESPAWN_FORCED;
        let creatures: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .map(|o| o.creatures.clone())
            .unwrap_or_default();
        for guid in creatures {
            let indexes = self
                .stores
                .battle_event_indexes
                .creature_indexes(guid.counter())
                .to_vec();
            let mut spawn_this = true;
            for (i1, i2) in &indexes {
                if !self.bg_is_active_event(*i1, *i2) {
                    spawn_this = false;
                    break;
                }
            }
            if spawn_this == is_spawn_mode {
                self.bg_spawn_creature(guid, mode);
            }
        }
    }

    /// `OnEventStateChanged` dispatch.
    fn bg_on_event_state_changed(&mut self, event1: u8, event2: u8, activated: bool) {
        if self.bg.as_ref().map(|b| b.bg_type_id) == Some(1) {
            self.av_on_event_state_changed(event1, event2, activated);
        }
    }

    /// `SpawnBGCreature`.
    pub(crate) fn bg_spawn_creature(&mut self, guid: ObjectGuid, mode: u8) {
        // Fork returns when the creature object is not
        // on the map (corpse-removed spawns still have
        // objects in C++; in Rust a removed creature is
        // re-materialized by the respawn pass, so a
        // missing object with a pending respawn gets
        // its due time adjusted instead).
        match mode {
            spawn_mode::RESPAWN_FORCED | spawn_mode::RESPAWN_START => {
                if let Some(c) = self.creatures.get_mut(&guid) {
                    c.respawn_delay = 120;
                }
                if let Some(when) = self.respawns_due.get_mut(&guid)
                    && *when > self.now_unix
                {
                    *when = self.now_unix + 1;
                }
                if !self.creatures.contains_key(&guid) && !self.respawns_due.contains_key(&guid) {
                    self.spawn_creature(
                        guid.counter(),
                        &mangos_world_data::spawn_support::RespawnTimes::default(),
                    );
                }
            }
            spawn_mode::DESPAWN_FORCED => {
                if let Some(c) = self.creatures.get_mut(&guid) {
                    c.respawn_delay = 31_536_000;
                }
                if self.creatures.contains_key(&guid) {
                    self.despawn_creature_now(guid, u32::MAX);
                }
                self.respawns_due.remove(&guid);
            }
            _ => {
                // RESPAWN_STOP
                if let Some(c) = self.creatures.get_mut(&guid) {
                    c.respawn_delay = 31_536_000;
                }
                let alive = self.creatures.get(&guid).is_some_and(|c| c.unit.is_alive());
                if self.creatures.contains_key(&guid) && !alive {
                    self.despawn_creature_now(guid, u32::MAX);
                    self.respawns_due.remove(&guid);
                } else if let Some(when) = self.respawns_due.get_mut(&guid)
                    && *when < self.now_unix
                {
                    *when = self.now_unix + 1;
                }
            }
        }
    }

    /// `SpawnBGObject` (`respawnTime` in seconds in the
    /// fork for GO respawn; `delay` here is milliseconds
    /// only in name — fork passes seconds through
    /// `SpawnEvent(..., delay)` call sites; kept as the
    /// fork's raw value).
    ///
    /// Rust representation: object materialized =
    /// spawned, removed = despawned (fork keeps a
    /// deactivated object; wire-equivalent).
    ///
    /// UNMODELED: GO loot-state reset + the 178786-89
    /// 60s respawn-delay special case ride the GO
    /// respawn runtime when it lands.
    pub(crate) fn bg_spawn_object(&mut self, guid: ObjectGuid, respawn_time: u32) {
        if respawn_time != RESPAWN_NEVER_MS {
            if !self.gameobjects.contains_key(&guid) {
                self.spawn_gameobject(
                    guid.counter(),
                    &mangos_world_data::spawn_support::RespawnTimes::default(),
                );
            }
        } else if self.gameobjects.contains_key(&guid) {
            self.despawn_gameobject_now(guid);
        }
    }

    /// `AddObject` — dynamic GO creation by entry at a
    /// fixed position, stored at `m_bgObjects[type]`.
    ///
    /// UNMODELED until kind steps need it: dynamic
    /// (non-DB) gameobject creation is not in the map
    /// runtime yet (no `CreateGameObject(entry)` path);
    /// kind setups record their object layouts against
    /// this seam in their steps.
    #[allow(clippy::unused_self, dead_code)]
    pub(crate) fn bg_add_object(
        &mut self,
        _slot: usize,
        _entry: u32,
        _pos: Position,
        _rotation: (f32, f32, f32, f32),
    ) -> bool {
        false
    }

    /// `DoorOpen` / `DoorClose` (GO state toggle).
    pub(crate) fn bg_door_open(&mut self, guid: ObjectGuid) {
        self.bg_set_door_state(guid, true);
    }

    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_door_close(&mut self, guid: ObjectGuid) {
        self.bg_set_door_state(guid, false);
    }

    fn bg_set_door_state(&mut self, guid: ObjectGuid, open: bool) {
        // `UseDoorOrButton(RESPAWN_NEVER)` toggles
        // `GO_STATE_ACTIVE`(0, open)/`GO_STATE_READY`(1,
        // closed). UNMODELED: loot-state precondition of
        // `DoorClose` (GO_ACTIVATED) — no GO loot-state
        // runtime yet.
        use mangos_protocol::update_field_table::offsets;
        let want = u32::from(!open);
        if self.object_get_field(guid, offsets::GAMEOBJECT_STATE) != want {
            let _ = self.object_set_field(guid, u32::from(offsets::GAMEOBJECT_STATE), want);
        }
    }

    /// `OpenDoorEvent`.
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_open_door_event(&mut self, event1: u8, event2: u8) {
        if !is_door(event1, event2) || !self.bg_is_active_event(event1, event2) {
            return;
        }
        let doors: Vec<ObjectGuid> = self
            .bg
            .as_ref()
            .and_then(|bg| bg.event_objects.get(&(event1, event2)))
            .map(|o| o.gameobjects.clone())
            .unwrap_or_default();
        for guid in doors {
            self.bg_door_open(guid);
        }
    }

    /// `HandleTriggerBuff` — trap GO buff pickup
    /// (speed/regen/berserk). Fork body ports with the
    /// WS step (first consumer); stub keeps the GO-use
    /// seam typed.
    #[allow(clippy::unused_self)]
    #[allow(dead_code)] // consumed by the kind steps
    pub(crate) fn bg_handle_trigger_buff(&mut self, _go: ObjectGuid) {}
}

/// `BG_OBJECTID_SPEEDBUFF_ENTRY`.
pub const BG_OBJECTID_SPEEDBUFF_ENTRY: u32 = 179_871;
/// `BG_OBJECTID_REGENBUFF_ENTRY`.
pub const BG_OBJECTID_REGENBUFF_ENTRY: u32 = 179_904;
/// `BG_OBJECTID_BERSERKERBUFF_ENTRY`.
pub const BG_OBJECTID_BERSERKERBUFF_ENTRY: u32 = 179_905;

/// `BG_EVENT_NONE`.
pub const BG_EVENT_NONE_VAL: u8 = 255;
/// `RESPAWN_NEVER` seconds sentinel as passed through
/// `SpawnBGObject`.
pub const RESPAWN_NEVER_MS: u32 = 31_536_000;

/// `BattleGround::IsDoor`.
#[must_use]
pub fn is_door(event1: u8, event2: u8) -> bool {
    event1 == BG_EVENT_DOOR && event2 == 0
}
