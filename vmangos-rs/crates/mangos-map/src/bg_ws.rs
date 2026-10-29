//! Warsong Gulch rules (`BattleGroundWS.cpp`, Tier 3
//! Slice 3 step 3). First-of-kind battleground-map
//! exemplar: flag states, capture/drop/return flow,
//! score counters, world states, fixed graveyards.

use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::world::WorldPacket;

use crate::battlegrounds::{
    BG_EVENT_DOOR, BG_EVENT_NONE_VAL, BgKind, bg_status, other_team, score_type, team_index,
};
use crate::map::Map;

/// `BG_WS_MAX_TEAM_SCORE`.
pub(crate) const WS_MAX_TEAM_SCORE: i32 = 3;
/// `BG_WS_FLAG_RESPAWN_TIME` (23 s).
pub(crate) const WS_FLAG_RESPAWN_TIME: i32 = 23_000;
/// `BG_WS_FLAG_DROP_TIME` (10 s).
pub(crate) const WS_FLAG_DROP_TIME: i32 = 10_000;

/// `enum BG_WS_Sound`.
pub(crate) mod ws_sound {
    pub const FLAG_CAPTURED_ALLIANCE: u32 = 8173;
    pub const FLAG_CAPTURED_HORDE: u32 = 8213;
    pub const FLAG_RETURNED: u32 = 8192;
    pub const HORDE_FLAG_PICKED_UP: u32 = 8212;
    pub const ALLIANCE_FLAG_PICKED_UP: u32 = 8174;
    pub const FLAGS_RESPAWNED: u32 = 8232;
}

/// `enum BG_WS_SpellId`.
pub(crate) mod ws_spell {
    pub const WARSONG_FLAG: u32 = 23333;
    pub const WARSONG_FLAG_DROPPED: u32 = 23334;
    pub const SILVERWING_FLAG: u32 = 23335;
    pub const SILVERWING_FLAG_DROPPED: u32 = 23336;
}

/// `enum BG_WS_WorldStates`.
pub(crate) mod ws_world_state {
    pub const FLAG_TAKEN_ALLIANCE: u32 = 1545;
    pub const FLAG_TAKEN_HORDE: u32 = 1546;
    pub const FLAG_CAPTURES_ALLIANCE: u32 = 1581;
    pub const FLAG_CAPTURES_HORDE: u32 = 1582;
    pub const FLAG_CAPTURES_MAX: u32 = 1601;
    pub const FLAG_STATE_HORDE: u32 = 2338;
    pub const FLAG_STATE_ALLIANCE: u32 = 2339;
}

/// `enum BG_WS_FlagState`.
pub(crate) mod ws_flag_state {
    pub const ON_BASE: u8 = 0;
    pub const WAIT_RESPAWN: u8 = 1;
    pub const ON_PLAYER: u8 = 2;
    pub const ON_GROUND: u8 = 3;
}

/// `enum BG_WS_Events` + `BG_EVENT_GHOST_GATE` (253).
pub(crate) const WS_EVENT_FLAG_A: u8 = 0;
pub(crate) const WS_EVENT_FLAG_H: u8 = 1;
pub(crate) const WS_EVENT_SPIRITGUIDES_SPAWN: u8 = 2;
pub(crate) const BG_EVENT_GHOST_GATE: u8 = 253;

/// Flag gameobject entries (`enum BG_WS_GameObjects`;
// base entries 179830/179831 are matched through the
// DB event indexes in the fork, as here).
pub(crate) const WS_ALLIANCE_FLAG_GROUND_ENTRY: u32 = 179_785;
pub(crate) const WS_HORDE_FLAG_GROUND_ENTRY: u32 = 179_786;

/// Flag-spawn area triggers.
pub(crate) const AT_ALLIANCE_FLAG_SPAWN: u32 = 3646;
pub(crate) const AT_HORDE_FLAG_SPAWN: u32 = 3647;

/// `enum BG_WS_Graveyards` (safe-loc ids).
pub(crate) const WS_GRAVEYARD_FLAGROOM_ALLIANCE: u32 = 769;
pub(crate) const WS_GRAVEYARD_FLAGROOM_HORDE: u32 = 770;
pub(crate) const WS_GRAVEYARD_MAIN_ALLIANCE: u32 = 771;
pub(crate) const WS_GRAVEYARD_MAIN_HORDE: u32 = 772;

/// `BattleGroundWS` members.
#[derive(Clone, Debug, Default)]
pub struct WsState {
    /// `m_flagKeepers`.
    pub flag_keepers: [ObjectGuid; 2],
    /// `m_droppedFlagGuid`.
    pub dropped_flag_guid: [ObjectGuid; 2],
    /// `m_flagState`.
    pub flag_state: [u8; 2],
    /// `m_flagsTimer`.
    pub flags_timer: [i32; 2],
    /// `m_flagsDropTimer`.
    pub flags_drop_timer: [i32; 2],
    /// `m_reputationCapture` (0 = uninitialized: Reset
    /// values are nonzero, so guards can tell).
    pub reputation_capture: u32,
    /// `m_honorWinKills` (member never read in the fork).
    pub honor_win_kills: u32,
    /// `m_honorEndKills` (member never read in the fork).
    pub honor_end_kills: u32,
}

/// `CHAT_MSG_BG_SYSTEM_ALLIANCE` / `_HORDE`.
pub(crate) const CHAT_MSG_BG_SYSTEM_ALLIANCE: u8 = 0x53;
pub(crate) const CHAT_MSG_BG_SYSTEM_HORDE: u8 = 0x54;

/// `BCT_BG_WS_*` broadcast-text ids.
pub(crate) mod bct {
    pub const CAPTURED_HF: i32 = 9801;
    pub const CAPTURED_AF: i32 = 9802;
    pub const F_PLACED: i32 = 9803;
    pub const PICKEDUP_AF: i32 = 9804;
    pub const DROPPED_AF: i32 = 9805;
    pub const DROPPED_HF: i32 = 9806;
    pub const PICKEDUP_HF: i32 = 9807;
    pub const RETURNED_AF: i32 = 9808;
    pub const RETURNED_HF: i32 = 9809;
    pub const ALLIANCE_FLAG_RESPAWNED: i32 = 10022;
    pub const HORDE_FLAG_RESPAWNED: i32 = 10023;
}

/// `BG_WSG_FlagCapturedHonor[GetBracketId()]`.
pub(crate) const WSG_FLAG_CAPTURED_HONOR: [u32; 6] = [48, 82, 136, 226, 378, 396];
/// `BG_WSG_WinMatchHonor[GetBracketId()]`.
pub(crate) const WSG_WIN_MATCH_HONOR: [u32; 6] = [24, 41, 68, 113, 189, 198];
/// `BG_WSG_WinMatchHonorHolidays[GetBracketId()]`.
pub(crate) const WSG_WIN_MATCH_HONOR_HOLIDAYS: [u32; 6] = [48, 82, 136, 226, 378, 396];
/// `BG_WSG_WinMatchHonorBonusCompleteHolidays[GetBracketId()]`.
pub(crate) const WSG_HONOR_BONUS_COMPLETE_HOLIDAYS: [u32; 6] = [72, 123, 204, 339, 567, 594];

impl Map {
    fn ws(&self) -> Option<&WsState> {
        match self.bg.as_ref().map(|b| &b.kind) {
            Some(BgKind::Ws(ws)) => Some(ws),
            _ => None,
        }
    }

    fn ws_mut(&mut self) -> Option<&mut WsState> {
        match self.bg.as_mut().map(|b| &mut b.kind) {
            Some(BgKind::Ws(ws)) => Some(ws),
            _ => None,
        }
    }

    /// `GetTeamScore`.
    fn ws_team_score(&self, team: Team) -> i32 {
        self.bg
            .as_ref()
            .map_or(0, |b| b.team_scores[team_index(team)])
    }

    /// `GetFlagState`.
    fn ws_flag_state(&self, team: Team) -> u8 {
        self.ws().map_or(0, |ws| ws.flag_state[team_index(team)])
    }

    /// `IsAllianceFlagPickedup` / `IsHordeFlagPickedup`.
    fn ws_flag_picked_up(&self, team: Team) -> bool {
        self.ws()
            .is_some_and(|ws| !ws.flag_keepers[team_index(team)].is_empty())
    }

    /// `UpdateFlagState`.
    fn ws_update_flag_state(&mut self, team: Team, value: u32) {
        let field = if team == Team::Alliance {
            ws_world_state::FLAG_STATE_ALLIANCE
        } else {
            ws_world_state::FLAG_STATE_HORDE
        };
        self.bg_update_world_state(field, value);
    }

    /// `UpdateTeamScore`.
    fn ws_update_team_score(&mut self, team: Team) {
        let field = if team == Team::Alliance {
            ws_world_state::FLAG_CAPTURES_ALLIANCE
        } else {
            ws_world_state::FLAG_CAPTURES_HORDE
        };
        let score = self.ws_team_score(team).max(0) as u32;
        self.bg_update_world_state(field, score);
    }

    /// Constructor (fork `m_startMessageIds`
    /// `[0, BCT_BG_WS_START_ONE_MINUTE, BCT_BG_WS_START_HALF_MINUTE, BCT_BG_WS_HAS_BEGUN]` are
    /// seeded by `BgState::new` for type 2 — nothing
    /// left to do).
    #[allow(clippy::unused_self)]
    pub(crate) fn ws_construct(&mut self) {}

    /// `Update` (flag timers; base update runs after,
    /// in `bg_update`).
    #[allow(clippy::cast_possible_wrap)]
    pub(crate) fn ws_update(&mut self, diff_ms: u32) {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return;
        }
        for team in [Team::Alliance, Team::Horde] {
            let idx = team_index(team);
            let state = self.ws_flag_state(team);
            if state == ws_flag_state::WAIT_RESPAWN {
                let fire = {
                    let Some(ws) = self.ws_mut() else { return };
                    ws.flags_timer[idx] -= diff_ms as i32;
                    if ws.flags_timer[idx] < 0 {
                        ws.flags_timer[idx] = 0;
                        true
                    } else {
                        false
                    }
                };
                if fire {
                    self.ws_respawn_flag(team, true);
                }
            }
            if state == ws_flag_state::ON_GROUND {
                let fire = {
                    let Some(ws) = self.ws_mut() else { return };
                    ws.flags_drop_timer[idx] -= diff_ms as i32;
                    if ws.flags_drop_timer[idx] < 0 {
                        ws.flags_drop_timer[idx] = 0;
                        true
                    } else {
                        false
                    }
                };
                if fire {
                    self.ws_respawn_flag_after_drop(team);
                }
            }
        }
    }

    /// `SetupBattleGround` (always true) + `Reset`'s
    /// kind tail folded into the same once-guard:
    /// fork `Reset()` runs at creation, `SetupBattleGround`
    /// at starting-event 1.
    pub(crate) fn ws_setup(&mut self) -> bool {
        if self.ws().is_some_and(|ws| ws.reputation_capture != 0) {
            return true;
        }
        if let Some(bg) = self.bg.as_mut() {
            // spiritguides and flags not spawned at
            // beginning; ghost gates spawned at
            // beginning (active event2 = 0).
            bg.active_events
                .insert(WS_EVENT_SPIRITGUIDES_SPAWN, BG_EVENT_NONE_VAL);
            bg.active_events.insert(WS_EVENT_FLAG_A, BG_EVENT_NONE_VAL);
            bg.active_events.insert(WS_EVENT_FLAG_H, BG_EVENT_NONE_VAL);
            bg.active_events.insert(BG_EVENT_GHOST_GATE, 0);
            bg.team_scores = [0, 0];
        }
        // UNMODELED: `BattleGroundMgr::IsBgWeekend`
        // (`sGameEventMgr.IsActiveHoliday`) — no game
        // event manager (`game_event_active` is a
        // recorded false stub), so the non-weekend
        // values hold.
        let is_weekend = false;
        if let Some(ws) = self.ws_mut() {
            ws.dropped_flag_guid = [ObjectGuid::EMPTY; 2];
            ws.flag_keepers = [ObjectGuid::EMPTY; 2];
            ws.flag_state = [ws_flag_state::ON_BASE; 2];
            ws.reputation_capture = if is_weekend { 45 } else { 35 };
            // [-PROGRESSIVE] < 1.10 accurate-pvp-rewards
            // arm can't fire (bench patch is 1.12 =
            // WOW_PATCH_110+, see `config.bg.wow_patch`).
            ws.honor_win_kills = if is_weekend { 3 } else { 1 };
            ws.honor_end_kills = if is_weekend { 4 } else { 2 };
        }
        true
    }

    /// `StartingEventCloseDoors` (empty in the fork).
    #[allow(clippy::unused_self)]
    pub(crate) fn ws_starting_close_doors(&mut self) {}

    /// `StartingEventOpenDoors`.
    pub(crate) fn ws_starting_open_doors(&mut self) {
        self.bg_spawn_event(WS_EVENT_SPIRITGUIDES_SPAWN, 0, true, true, 0);
        self.bg_spawn_event(WS_EVENT_FLAG_A, 0, true, true, 0);
        self.bg_spawn_event(WS_EVENT_FLAG_H, 0, true, true, 0);
        self.bg_spawn_event(BG_EVENT_GHOST_GATE, 0, false, true, 0);
        // `OpenDoorEvent(BG_EVENT_DOOR)`: the fork
        // helper opens every door-event GO; the Rust
        // spawn machinery already toggles door GO
        // state through `SpawnBGObject`, so only the
        // animated door state flip remains.
        self.bg_open_door_event(BG_EVENT_DOOR, 0);
    }

    /// `AddPlayer` per-kind arm: the flat `BgScore`
    /// covers `BattleGroundWGScore` fields, so only
    /// the entry defaults matter (fork: ctor zeroes).
    pub(crate) fn ws_add_player_pre(&mut self, guid: ObjectGuid) {
        if let Some(bg) = self.bg.as_mut() {
            bg.scores.entry(guid).or_default();
        }
    }

    /// `SendInitWorldStates` BG portion +
    /// `FillInitialWorldStates`
    /// (`Player::SendInitWorldStates` runs on
    /// worldport-ack in `bg_maybe_add_player`).
    pub fn ws_fill_initial_world_states(&mut self, player: ObjectGuid) -> Option<WorldPacket> {
        let flag_states = self.ws()?.flag_state;
        let zone_id = self
            .zone_and_area_id(
                self.player(player)
                    .map_or(Position::default(), |p| p.unit.world.position()),
            )
            .0;
        let mut writer = PacketWriter::new();
        writer.write_u32_le(self.map_id);
        writer.write_u32_le(zone_id);
        let mut pairs: Vec<(u32, u32)> = crate::map::DEFAULT_WORLD_STATES
            .iter()
            .map(|&(s, v)| (s, v))
            .collect();
        pairs.push((
            ws_world_state::FLAG_CAPTURES_ALLIANCE,
            self.ws_team_score(Team::Alliance).max(0) as u32,
        ));
        pairs.push((
            ws_world_state::FLAG_CAPTURES_HORDE,
            self.ws_team_score(Team::Horde).max(0) as u32,
        ));
        pairs.push((
            ws_world_state::FLAG_TAKEN_ALLIANCE,
            taken_value(flag_states[team_index(Team::Alliance)]),
        ));
        pairs.push((
            ws_world_state::FLAG_TAKEN_HORDE,
            taken_value(flag_states[team_index(Team::Horde)]),
        ));
        pairs.push((ws_world_state::FLAG_CAPTURES_MAX, WS_MAX_TEAM_SCORE as u32));
        pairs.push((
            ws_world_state::FLAG_STATE_ALLIANCE,
            if flag_states[team_index(Team::Horde)] == ws_flag_state::ON_PLAYER {
                2
            } else {
                1
            },
        ));
        pairs.push((
            ws_world_state::FLAG_STATE_HORDE,
            if flag_states[team_index(Team::Alliance)] == ws_flag_state::ON_PLAYER {
                2
            } else {
                1
            },
        ));
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

    /// `RespawnFlag`.
    fn ws_respawn_flag(&mut self, team: Team, captured: bool) {
        let (idx, event) = if team == Team::Alliance {
            (team_index(Team::Alliance), WS_EVENT_FLAG_A)
        } else {
            (team_index(Team::Horde), WS_EVENT_FLAG_H)
        };
        if let Some(ws) = self.ws_mut() {
            ws.flag_state[idx] = ws_flag_state::ON_BASE;
        }
        self.bg_spawn_event(event, 0, true, true, 0);
        if captured {
            self.bg_spawn_event(WS_EVENT_FLAG_A, 0, true, true, 0);
            self.bg_spawn_event(WS_EVENT_FLAG_H, 0, true, true, 0);
            self.bg_send_message_to_all(bct::F_PLACED, None);
            self.bg_play_sound_to_all(ws_sound::FLAGS_RESPAWNED);
        }
    }

    /// `RespawnFlagAfterDrop`.
    fn ws_respawn_flag_after_drop(&mut self, team: Team) {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return;
        }
        self.ws_respawn_flag(team, false);
        let taken = if team == Team::Horde {
            ws_world_state::FLAG_TAKEN_HORDE
        } else {
            ws_world_state::FLAG_TAKEN_ALLIANCE
        };
        self.bg_update_world_state(taken, 0);
        if team == Team::Alliance {
            self.bg_send_message_to_all(bct::ALLIANCE_FLAG_RESPAWNED, None);
        } else {
            self.bg_send_message_to_all(bct::HORDE_FLAG_RESPAWNED, None);
        }
        self.bg_play_sound_to_all(ws_sound::FLAGS_RESPAWNED);
        // UNMODELED: fork deletes the dropped-flag GO
        // summoned by spells 23334/23336; the Rust spell
        // engine has no EFFECT_SUMMON_OBJECT arm, so no
        // ground flag object exists to delete (step 6
        // remainder). `ClearDroppedFlagGuid` still runs.
        if let Some(ws) = self.ws_mut() {
            ws.dropped_flag_guid[team_index(team)] = ObjectGuid::EMPTY;
        }
        self.ws_force_flag_area_trigger(team);
    }

    /// `ForceFlagAreaTrigger`.
    fn ws_force_flag_area_trigger(&mut self, team: Team) {
        let keeper = self
            .ws()
            .map(|ws| ws.flag_keepers[team_index(other_team(team))]);
        let Some(keeper) = keeper.filter(|g| !g.is_empty()) else {
            return;
        };
        let trigger_id = if team == Team::Alliance {
            AT_ALLIANCE_FLAG_SPAWN
        } else {
            AT_HORDE_FLAG_SPAWN
        };
        let Some(pos) = self.object_position(keeper) else {
            return;
        };
        let inside = self
            .stores
            .area_triggers
            .get(trigger_id)
            .is_some_and(|t| t.contains(self.map_id, pos.x, pos.y, pos.z, 2.0));
        if !inside {
            return;
        }
        self.ws_handle_area_trigger(keeper, trigger_id);
    }

    /// `EventPlayerCapturedFlag`.
    #[allow(clippy::cast_possible_wrap)]
    fn ws_event_player_captured_flag(&mut self, source: ObjectGuid) {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return;
        }
        let team = self.player(source).map_or(Team::None, |p| p.team);
        let mut winner = Team::None;
        if team == Team::Alliance {
            if !self.ws_flag_picked_up(Team::Horde) {
                return;
            }
            // clear must be before aura remove to
            // prevent 2 events (drop+capture).
            if let Some(ws) = self.ws_mut() {
                ws.flag_keepers[team_index(Team::Horde)] = ObjectGuid::EMPTY;
                ws.flag_state[team_index(Team::Horde)] = ws_flag_state::WAIT_RESPAWN;
            }
            self.remove_auras_due_to_spell(source, ws_spell::WARSONG_FLAG);
            if self.ws_team_score(Team::Alliance) < WS_MAX_TEAM_SCORE
                && let Some(bg) = self.bg.as_mut()
            {
                bg.team_scores[team_index(Team::Alliance)] += 1;
            }
            self.bg_play_sound_to_all(ws_sound::FLAG_CAPTURED_ALLIANCE);
            let rep = self.ws().map_or(0, |ws| ws.reputation_capture);
            self.bg_reward_reputation_to_team(890, rep as i32, Team::Alliance);
            self.bg_update_world_state(ws_world_state::FLAG_TAKEN_HORDE, 0);
        } else {
            if !self.ws_flag_picked_up(Team::Alliance) {
                return;
            }
            if let Some(ws) = self.ws_mut() {
                ws.flag_keepers[team_index(Team::Alliance)] = ObjectGuid::EMPTY;
                ws.flag_state[team_index(Team::Alliance)] = ws_flag_state::WAIT_RESPAWN;
            }
            self.remove_auras_due_to_spell(source, ws_spell::SILVERWING_FLAG);
            if self.ws_team_score(Team::Horde) < WS_MAX_TEAM_SCORE
                && let Some(bg) = self.bg.as_mut()
            {
                bg.team_scores[team_index(Team::Horde)] += 1;
            }
            self.bg_play_sound_to_all(ws_sound::FLAG_CAPTURED_HORDE);
            let rep = self.ws().map_or(0, |ws| ws.reputation_capture);
            self.bg_reward_reputation_to_team(889, rep as i32, Team::Horde);
            self.bg_update_world_state(ws_world_state::FLAG_TAKEN_ALLIANCE, 0);
        }
        let bracket = self.bg.as_ref().map_or(0, |b| b.bracket_id) as usize;
        let honor = WSG_FLAG_CAPTURED_HONOR.get(bracket).copied().unwrap_or(0);
        self.bg_reward_honor_to_team(honor, team);
        self.bg_spawn_event(WS_EVENT_FLAG_A, 0, false, true, 0);
        self.bg_spawn_event(WS_EVENT_FLAG_H, 0, false, true, 0);
        if team == Team::Alliance {
            self.bg_send_message_typed(bct::CAPTURED_HF, CHAT_MSG_BG_SYSTEM_ALLIANCE, Some(source));
        } else {
            self.bg_send_message_typed(bct::CAPTURED_AF, CHAT_MSG_BG_SYSTEM_HORDE, Some(source));
        }
        self.ws_update_flag_state(team, 1);
        self.ws_update_team_score(team);
        self.bg_update_player_score(source, score_type::FLAG_CAPTURES, 1);
        if self.ws_team_score(Team::Alliance) == WS_MAX_TEAM_SCORE {
            winner = Team::Alliance;
        }
        if self.ws_team_score(Team::Horde) == WS_MAX_TEAM_SCORE {
            winner = Team::Horde;
        }
        if winner != Team::None {
            self.bg_update_world_state(ws_world_state::FLAG_STATE_ALLIANCE, 1);
            self.bg_update_world_state(ws_world_state::FLAG_STATE_HORDE, 1);
            self.bg_end_battleground(winner);
        } else if let Some(ws) = self.ws_mut() {
            ws.flags_timer[team_index(other_team(team))] = WS_FLAG_RESPAWN_TIME;
        }
    }

    /// `EndBattleGround` WS reward stage (runs before
    /// the base ending; fork subclass order).
    pub(crate) fn ws_end_battleground_rewards(&mut self, winner: Team) {
        // UNMODELED: `IsBgWeekend` (see `ws_setup`).
        let is_weekend = false;
        let bracket = self.bg.as_ref().map_or(0, |b| b.bracket_id) as usize;
        if is_weekend {
            let honor = WSG_HONOR_BONUS_COMPLETE_HOLIDAYS
                .get(bracket)
                .copied()
                .unwrap_or(0);
            self.bg_reward_honor_to_team(honor, Team::Alliance);
            self.bg_reward_honor_to_team(honor, Team::Horde);
        }
        if winner == Team::Alliance {
            let honor = WSG_WIN_MATCH_HONOR.get(bracket).copied().unwrap_or(0);
            self.bg_reward_honor_to_team(honor, Team::Alliance);
            if is_weekend {
                let honor = WSG_WIN_MATCH_HONOR_HOLIDAYS
                    .get(bracket)
                    .copied()
                    .unwrap_or(0);
                self.bg_reward_honor_to_team(honor, Team::Alliance);
            }
        }
        if winner == Team::Horde {
            let honor = WSG_WIN_MATCH_HONOR.get(bracket).copied().unwrap_or(0);
            self.bg_reward_honor_to_team(honor, Team::Horde);
            if is_weekend {
                let honor = WSG_WIN_MATCH_HONOR_HOLIDAYS
                    .get(bracket)
                    .copied()
                    .unwrap_or(0);
                self.bg_reward_honor_to_team(honor, Team::Horde);
            }
        }
    }

    /// `EventPlayerDroppedFlag`.
    pub(crate) fn ws_event_dropped_flag(&mut self, source: ObjectGuid) {
        let in_progress = self
            .bg
            .as_ref()
            .is_some_and(|b| b.status == bg_status::IN_PROGRESS);
        let team = self.player(source).map_or(Team::None, |p| p.team);
        if !in_progress {
            if team == Team::Alliance {
                if !self.ws_flag_picked_up(Team::Horde) {
                    return;
                }
                if self.ws().map(|ws| ws.flag_keepers[team_index(Team::Horde)]) == Some(source) {
                    if let Some(ws) = self.ws_mut() {
                        ws.flag_keepers[team_index(Team::Horde)] = ObjectGuid::EMPTY;
                    }
                    self.remove_auras_due_to_spell(source, ws_spell::WARSONG_FLAG);
                }
            } else {
                if !self.ws_flag_picked_up(Team::Alliance) {
                    return;
                }
                if self
                    .ws()
                    .map(|ws| ws.flag_keepers[team_index(Team::Alliance)])
                    == Some(source)
                {
                    if let Some(ws) = self.ws_mut() {
                        ws.flag_keepers[team_index(Team::Alliance)] = ObjectGuid::EMPTY;
                    }
                    self.remove_auras_due_to_spell(source, ws_spell::SILVERWING_FLAG);
                }
            }
            return;
        }
        let mut set = false;
        if team == Team::Alliance {
            if !self.ws_flag_picked_up(Team::Horde) {
                return;
            }
            if self.ws().map(|ws| ws.flag_keepers[team_index(Team::Horde)]) == Some(source) {
                if let Some(ws) = self.ws_mut() {
                    ws.flag_keepers[team_index(Team::Horde)] = ObjectGuid::EMPTY;
                    ws.flag_state[team_index(Team::Horde)] = ws_flag_state::ON_GROUND;
                }
                self.remove_auras_due_to_spell(source, ws_spell::WARSONG_FLAG);
                self.bg_reward_spell_cast(source, ws_spell::WARSONG_FLAG_DROPPED);
                set = true;
            }
        } else {
            if !self.ws_flag_picked_up(Team::Alliance) {
                return;
            }
            if self
                .ws()
                .map(|ws| ws.flag_keepers[team_index(Team::Alliance)])
                == Some(source)
            {
                if let Some(ws) = self.ws_mut() {
                    ws.flag_keepers[team_index(Team::Alliance)] = ObjectGuid::EMPTY;
                    ws.flag_state[team_index(Team::Alliance)] = ws_flag_state::ON_GROUND;
                }
                self.remove_auras_due_to_spell(source, ws_spell::SILVERWING_FLAG);
                self.bg_reward_spell_cast(source, ws_spell::SILVERWING_FLAG_DROPPED);
                set = true;
            }
        }
        if set {
            self.ws_update_flag_state(team, 1);
            if team == Team::Alliance {
                self.bg_send_message_typed(bct::DROPPED_HF, CHAT_MSG_BG_SYSTEM_HORDE, Some(source));
                self.bg_update_world_state(ws_world_state::FLAG_TAKEN_HORDE, u32::MAX);
            } else {
                self.bg_send_message_typed(
                    bct::DROPPED_AF,
                    CHAT_MSG_BG_SYSTEM_ALLIANCE,
                    Some(source),
                );
                self.bg_update_world_state(ws_world_state::FLAG_TAKEN_ALLIANCE, u32::MAX);
            }
            if let Some(ws) = self.ws_mut() {
                ws.flags_drop_timer[team_index(other_team(team))] = WS_FLAG_DROP_TIME;
            }
        }
    }

    /// `EventPlayerClickedOnFlag`.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn ws_event_clicked_on_flag(&mut self, source: ObjectGuid, go: ObjectGuid) {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return;
        }
        let team = self.player(source).map_or(Team::None, |p| p.team);
        let event = self
            .stores
            .battle_event_indexes
            .gameobject_indexes(go.counter())
            .first()
            .map_or(BG_EVENT_NONE_VAL, |&(e1, _)| e1);
        let entry = self.object_entry(go).unwrap_or(0);
        let mut message_id = 0;
        let mut msg_type = 0u8;
        // alliance flag picked up from base
        if team == Team::Horde
            && self.ws_flag_state(Team::Alliance) == ws_flag_state::ON_BASE
            && event == WS_EVENT_FLAG_A
        {
            message_id = bct::PICKEDUP_AF;
            msg_type = CHAT_MSG_BG_SYSTEM_HORDE;
            self.bg_play_sound_to_all(ws_sound::ALLIANCE_FLAG_PICKED_UP);
            self.bg_spawn_event(WS_EVENT_FLAG_A, 0, false, true, 0);
            if let Some(ws) = self.ws_mut() {
                ws.flag_keepers[team_index(Team::Alliance)] = source;
                ws.flag_state[team_index(Team::Alliance)] = ws_flag_state::ON_PLAYER;
            }
            self.ws_update_flag_state(Team::Horde, u32::from(ws_flag_state::ON_PLAYER));
            self.bg_update_world_state(ws_world_state::FLAG_TAKEN_ALLIANCE, 1);
            self.bg_reward_spell_cast(source, ws_spell::SILVERWING_FLAG);
        }
        // horde flag picked up from base
        if team == Team::Alliance
            && self.ws_flag_state(Team::Horde) == ws_flag_state::ON_BASE
            && event == WS_EVENT_FLAG_H
        {
            message_id = bct::PICKEDUP_HF;
            msg_type = CHAT_MSG_BG_SYSTEM_ALLIANCE;
            self.bg_play_sound_to_all(ws_sound::HORDE_FLAG_PICKED_UP);
            self.bg_spawn_event(WS_EVENT_FLAG_H, 0, false, true, 0);
            if let Some(ws) = self.ws_mut() {
                ws.flag_keepers[team_index(Team::Horde)] = source;
                ws.flag_state[team_index(Team::Horde)] = ws_flag_state::ON_PLAYER;
            }
            self.ws_update_flag_state(Team::Alliance, u32::from(ws_flag_state::ON_PLAYER));
            self.bg_update_world_state(ws_world_state::FLAG_TAKEN_HORDE, 1);
            self.bg_reward_spell_cast(source, ws_spell::WARSONG_FLAG);
        }
        // Alliance flag on ground (returned or picked up again)
        if self.ws_flag_state(Team::Alliance) == ws_flag_state::ON_GROUND
            && self.within_dist(source, go, 10.0)
            && entry == WS_ALLIANCE_FLAG_GROUND_ENTRY
        {
            if team == Team::Alliance {
                message_id = bct::RETURNED_AF;
                msg_type = CHAT_MSG_BG_SYSTEM_ALLIANCE;
                self.ws_update_flag_state(Team::Horde, u32::from(ws_flag_state::WAIT_RESPAWN));
                self.ws_respawn_flag(Team::Alliance, false);
                self.bg_play_sound_to_all(ws_sound::FLAG_RETURNED);
                self.bg_update_player_score(source, score_type::FLAG_RETURNS, 1);
                self.ws_force_flag_area_trigger(Team::Alliance);
            } else {
                message_id = bct::PICKEDUP_AF;
                msg_type = CHAT_MSG_BG_SYSTEM_HORDE;
                self.bg_play_sound_to_all(ws_sound::ALLIANCE_FLAG_PICKED_UP);
                self.bg_spawn_event(WS_EVENT_FLAG_A, 0, false, true, 0);
                if let Some(ws) = self.ws_mut() {
                    ws.flag_keepers[team_index(Team::Alliance)] = source;
                    ws.flag_state[team_index(Team::Alliance)] = ws_flag_state::ON_PLAYER;
                }
                self.bg_reward_spell_cast(source, ws_spell::SILVERWING_FLAG);
                self.ws_update_flag_state(Team::Horde, u32::from(ws_flag_state::ON_PLAYER));
                self.bg_update_world_state(ws_world_state::FLAG_TAKEN_ALLIANCE, 1);
            }
        }
        // Horde flag on ground (returned or picked up again)
        if self.ws_flag_state(Team::Horde) == ws_flag_state::ON_GROUND
            && self.within_dist(source, go, 10.0)
            && entry == WS_HORDE_FLAG_GROUND_ENTRY
        {
            if team == Team::Horde {
                message_id = bct::RETURNED_HF;
                msg_type = CHAT_MSG_BG_SYSTEM_HORDE;
                self.ws_update_flag_state(Team::Alliance, u32::from(ws_flag_state::WAIT_RESPAWN));
                self.ws_respawn_flag(Team::Horde, false);
                self.bg_play_sound_to_all(ws_sound::FLAG_RETURNED);
                self.bg_update_player_score(source, score_type::FLAG_RETURNS, 1);
                self.ws_force_flag_area_trigger(Team::Horde);
            } else {
                message_id = bct::PICKEDUP_HF;
                msg_type = CHAT_MSG_BG_SYSTEM_ALLIANCE;
                self.bg_play_sound_to_all(ws_sound::HORDE_FLAG_PICKED_UP);
                self.bg_spawn_event(WS_EVENT_FLAG_H, 0, false, true, 0);
                if let Some(ws) = self.ws_mut() {
                    ws.flag_keepers[team_index(Team::Horde)] = source;
                    ws.flag_state[team_index(Team::Horde)] = ws_flag_state::ON_PLAYER;
                }
                self.bg_reward_spell_cast(source, ws_spell::WARSONG_FLAG);
                self.ws_update_flag_state(Team::Alliance, u32::from(ws_flag_state::ON_PLAYER));
                self.bg_update_world_state(ws_world_state::FLAG_TAKEN_HORDE, 1);
            }
        }
        if message_id == 0 {
            return;
        }
        self.bg_send_message_typed(message_id, msg_type, Some(source));
    }

    /// `UpdatePlayerScore` kind arm.
    pub(crate) fn ws_update_score(
        &mut self,
        source: ObjectGuid,
        score_type: u32,
        value: u32,
    ) -> bool {
        match score_type {
            score_type::FLAG_CAPTURES => {
                if let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    score.flag_captures += value;
                }
                true
            }
            score_type::FLAG_RETURNS => {
                if let Some(bg) = self.bg.as_mut()
                    && let Some(score) = bg.scores.get_mut(&source)
                {
                    score.flag_returns += value;
                }
                true
            }
            _ => false,
        }
    }

    /// `HandleKillPlayer` kind arm (base runs just
    /// before in `bg_handle_kill_player`).
    pub(crate) fn ws_on_kill_player(&mut self, victim: ObjectGuid, _killer: Option<ObjectGuid>) {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return;
        }
        self.ws_event_dropped_flag(victim);
    }

    /// `HandleKillUnit` (no WS override).
    #[allow(clippy::unused_self)]
    pub(crate) fn ws_on_kill_unit(&mut self, _victim: ObjectGuid, _killer: Option<ObjectGuid>) {}

    /// `RemovePlayer`.
    pub(crate) fn ws_remove_player(&mut self, guid: ObjectGuid) {
        for team in [Team::Alliance, Team::Horde] {
            let idx = team_index(team);
            if self.ws_flag_picked_up(team)
                && self.ws().map(|ws| ws.flag_keepers[idx]) == Some(guid)
            {
                if self.player(guid).is_none() {
                    if let Some(ws) = self.ws_mut() {
                        ws.flag_keepers[idx] = ObjectGuid::EMPTY;
                    }
                    self.ws_respawn_flag(team, false);
                } else {
                    self.ws_event_dropped_flag(guid);
                }
            }
        }
    }

    /// `GetClosestGraveYard`.
    #[must_use]
    pub fn ws_closest_graveyard(
        &self,
        player: ObjectGuid,
    ) -> Option<mangos_world_data::battlegrounds::WorldSafeLoc> {
        let team = self.player(player).map_or(Team::None, |p| p.team);
        let in_progress = self
            .bg
            .as_ref()
            .is_some_and(|b| b.status == bg_status::IN_PROGRESS);
        let id = if team == Team::Alliance {
            if in_progress {
                WS_GRAVEYARD_MAIN_ALLIANCE
            } else {
                WS_GRAVEYARD_FLAGROOM_ALLIANCE
            }
        } else if in_progress {
            WS_GRAVEYARD_MAIN_HORDE
        } else {
            WS_GRAVEYARD_FLAGROOM_HORDE
        };
        self.stores.safe_locs.get(id)
    }

    /// `HandleAreaTrigger`.
    pub(crate) fn ws_handle_area_trigger(&mut self, source: ObjectGuid, trigger_id: u32) -> bool {
        if self
            .bg
            .as_ref()
            .is_none_or(|b| b.status != bg_status::IN_PROGRESS)
        {
            return false;
        }
        match trigger_id {
            // buff elixir spawns: handled by trap GOs
            // in the fork (the fork comment notes the
            // triggers themselves do nothing); no Rust
            // trap runtime yet — recorded remainder.
            3686 | 3687 | 3706 | 3707 | 3708 | 3709 => true,
            AT_ALLIANCE_FLAG_SPAWN => {
                if self.ws_flag_state(Team::Horde) != 0
                    && self.ws_flag_state(Team::Alliance) == 0
                    && self.ws().map(|ws| ws.flag_keepers[team_index(Team::Horde)]) == Some(source)
                {
                    self.ws_event_player_captured_flag(source);
                }
                true
            }
            AT_HORDE_FLAG_SPAWN => {
                if self.ws_flag_state(Team::Alliance) != 0
                    && self.ws_flag_state(Team::Horde) == 0
                    && self
                        .ws()
                        .map(|ws| ws.flag_keepers[team_index(Team::Alliance)])
                        == Some(source)
                {
                    self.ws_event_player_captured_flag(source);
                }
                true
            }
            // Warsong Gulch - Horde Exit
            3669 => {
                if self.player(source).map(|p| p.team) == Some(Team::Horde) {
                    // `LeaveBattleground` (transport +
                    // deserter per config).
                    self.ws_remove_player(source);
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            // Warsong Gulch - Alliance Exit
            3671 => {
                if self.player(source).map(|p| p.team) == Some(Team::Alliance) {
                    self.ws_remove_player(source);
                    self.bg_remove_player_at_leave(source, true, true);
                    return true;
                }
                false
            }
            // Fork logs "Unhandled AreaTrigger" +
            // SendAreaTriggerMessage; the actor-side AT
            // handler logs the same warning on a false
            // return — recorded message remainder.
            _ => false,
        }
    }
}

impl Map {
    /// Flag carrier guids for `SMSG_BATTLEGROUND_PLAYER_POSITIONS`
    /// (`GetAllianceFlagPickerGuid` /
    /// `GetHordeFlagPickerGuid`).
    #[must_use]
    pub fn ws_flag_carriers_pub(&self) -> (Option<ObjectGuid>, Option<ObjectGuid>) {
        match self.ws() {
            Some(ws) => (
                ws.flag_keepers[team_index(Team::Alliance)].pipe_non_empty(),
                ws.flag_keepers[team_index(Team::Horde)].pipe_non_empty(),
            ),
            None => (None, None),
        }
    }
}

/// `FillInitialWorldState` taken-value mapping.
fn taken_value(flag_state: u8) -> u32 {
    match flag_state {
        ws_flag_state::ON_GROUND => u32::MAX,
        ws_flag_state::ON_PLAYER => 1,
        _ => 0,
    }
}

trait NonEmptyGuid {
    fn pipe_non_empty(self) -> Option<ObjectGuid>;
}

impl NonEmptyGuid for ObjectGuid {
    fn pipe_non_empty(self) -> Option<ObjectGuid> {
        if self.is_empty() { None } else { Some(self) }
    }
}
