//! Escort and follower creature AI (Tier 4 B0-6):
//! `npc_escortAI` (`ScriptedEscortAI.{h,cpp}`) and
//! `FollowerAI` (`ScriptedFollowerAI.{h,cpp}`), keyed
//! by creature guid in `Map::escorts` (the fork state
//! lives on the per-creature AI object; ours lives in
//! the side map like `go_runtime`).
//!
//! Script hooks ride the content dispatchers
//! `creature_script_escort_waypoint_reached/_start`,
//! `creature_script_just_started_escort`,
//! `creature_script_escort_update` (both classes'
//! `UpdateEscortAI`/`UpdateFollowerAI` override point)
//! and `creature_script_escort_failed`
//! (`FollowerAI::OnEscortFailed`).
//!
//! UNMODELED: `AssistPlayerInCombat` LOS assist (the
//! engine-wide aggro scan approximates it), the
//! default `UpdateEscortAI` passive-aggro pull
//! (`SelectNearestHostileUnitInAggroRange` on the
//! escort; normal creature aggro already targets the
//! escort), `CanAssistPlayers` flag screen, exact
//! `IsWithinDistInMap` size-adjusted distance (plain
//! 3D distance here), `DisappearAndDie` corpse delay
//! arm (rides `despawn_creature_now`), escort state
//! surviving creature respawn (fork `JustRespawned`
//! clears it; our entry is dropped on despawn/death,
//! same observable), `SetDelayBeforeTheFirstWaypoint`
//! per-script values (ctor 2500 used; scripts calling
//! the setter record at their site), the fork
//! Point-generator resume after combat (our tick
//! re-issues the waypoint move, so `WaypointStart`
//! can refire where the fork silently resumes).

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;

use crate::map::Map;
use crate::motion::{MOVE_PATHFINDING, MOVE_RUN_MODE, MOVE_WALK_MODE};
use crate::pets::{PET_FOLLOW_ANGLE, PET_FOLLOW_DIST};

/// `STATE_ESCORT_ESCORTING` (`ScriptedEscortAI.h:34`).
pub const STATE_ESCORT_ESCORTING: u32 = 0x001;
/// `STATE_ESCORT_RETURNING`.
pub const STATE_ESCORT_RETURNING: u32 = 0x002;
/// `STATE_ESCORT_PAUSED`.
pub const STATE_ESCORT_PAUSED: u32 = 0x004;

/// `STATE_FOLLOW_INPROGRESS` (`ScriptedFollowerAI.h`).
pub const STATE_FOLLOW_INPROGRESS: u32 = 0x001;
/// `STATE_FOLLOW_RETURNING`.
pub const STATE_FOLLOW_RETURNING: u32 = 0x002;
/// `STATE_FOLLOW_PAUSED`.
pub const STATE_FOLLOW_PAUSED: u32 = 0x004;
/// `STATE_FOLLOW_COMPLETE`.
pub const STATE_FOLLOW_COMPLETE: u32 = 0x008;
/// `STATE_FOLLOW_POSTEVENT`.
pub const STATE_FOLLOW_POSTEVENT: u32 = 0x020;

/// `POINT_LAST_POINT` (`ScriptedEscortAI.cpp:25`):
/// combat-start return point id.
pub const POINT_LAST_POINT: u32 = 0xFF_FFFF;
/// `POINT_HOME`: return-to-spawn point id.
pub const POINT_HOME: u32 = 0xFF_FFFE;

/// `DEFAULT_MAX_PLAYER_DISTANCE`
/// (`ScriptedEscortAI.cpp:20`; follower
/// `MAX_PLAYER_DISTANCE` is the same value).
const MAX_PLAYER_DISTANCE: f32 = 100.0;

/// `UNIT_NPC_FLAG_QUESTGIVER`.
const UNIT_NPC_FLAG_QUESTGIVER: u32 = 0x0000_0002;

/// One filled escort waypoint (`Escort_Waypoint`).
#[derive(Clone, Copy, Debug)]
pub struct EscortWaypoint {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub wait_time_ms: u32,
}

/// `npc_escortAI` member state.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // fork member mirror
pub struct EscortData {
    pub player_guid: ObjectGuid,
    pub quest_id: u32,
    pub waypoints: Vec<EscortWaypoint>,
    pub current_idx: usize,
    pub state: u32,
    pub wp_wait_timer: u32,
    pub player_check_timer: u32,
    pub is_running: bool,
    pub can_instant_respawn: bool,
    pub can_return_to_start: bool,
    pub pathfinding_between: bool,
    pub max_player_distance: f32,
    pub combat_start: Position,
    pub combat_start_o: f32,
}

/// `FollowerAI` member state.
#[derive(Clone, Debug)]
pub struct FollowerData {
    pub leader_guid: ObjectGuid,
    pub quest_id: u32,
    pub follow_distance: f32,
    pub state: u32,
    pub update_timer: u32,
}

/// Which of the two fork AI classes a creature runs.
#[derive(Clone, Debug)]
pub enum EscortKind {
    Escort(EscortData),
    Follower(FollowerData),
}

impl Map {
    /// Plain 3D distance between two units (fork
    /// `GetDistance` / `IsWithinDistInMap` reduced;
    /// see module UNMODELED note).
    fn escort_unit_distance(&self, a: ObjectGuid, b: ObjectGuid) -> Option<f32> {
        let pa = self.unit_position(a)?;
        let pb = self.unit_position(b)?;
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        let dz = pa.z - pb.z;
        Some((dx * dx + dy * dy + dz * dz).sqrt())
    }

    /// Members of `player`'s group on this map, or
    /// just the player when ungrouped (`Group` member
    /// iteration reduced to the shared `group_id`).
    pub(crate) fn escort_group_members(&self, player: ObjectGuid) -> Vec<ObjectGuid> {
        let group_id = self.players.get(&player).and_then(|e| e.player.group_id);
        match group_id {
            None => vec![player],
            Some(group_id) => self
                .players
                .iter()
                .filter(|(_, e)| e.player.group_id == Some(group_id))
                .map(|(guid, _)| *guid)
                .collect(),
        }
    }

    /// `Player::SetEscortingGuid`.
    fn escort_set_player_guid(&mut self, player: ObjectGuid, creature: ObjectGuid) {
        if let Some(entry) = self.players.get_mut(&player) {
            entry.player.escorting_guid = creature;
        }
    }

    /// `GetPlayerForEscort`: the escort player when
    /// present on this map (no alive screen in fork).
    pub(crate) fn escort_player(&self, creature: ObjectGuid) -> Option<ObjectGuid> {
        let Some(EscortKind::Escort(data)) = self.escorts.get(&creature) else {
            return None;
        };
        self.players
            .contains_key(&data.player_guid)
            .then_some(data.player_guid)
    }

    /// `HasEscortState` / `HasFollowState`.
    pub(crate) fn escort_has_state(&self, creature: ObjectGuid, flag: u32) -> bool {
        match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => d.state & flag != 0,
            Some(EscortKind::Follower(d)) => d.state & flag != 0,
            None => false,
        }
    }

    fn escort_in_combat(&self, creature: ObjectGuid) -> bool {
        self.ai.get(&creature).is_some_and(|ai| ai.in_combat)
    }

    fn escort_creature_alive(&self, creature: ObjectGuid) -> bool {
        self.creatures
            .get(&creature)
            .is_some_and(|c| c.unit.is_alive())
    }

    /// `npc_escortAI::Start` (`ScriptedEscortAI.cpp:
    /// 405`): fill waypoints from `script_waypoint`,
    /// clear NPC flags, seed the first-waypoint delay
    /// (ctor `m_uiDelayBeforeTheFirstWaypoint` 2500),
    /// mark the player's escorting guid, then the
    /// `JustStartedEscort` script hook. `quest_id` 0
    /// is the fork's null quest.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn escort_start(
        &mut self,
        creature: ObjectGuid,
        run: bool,
        player: Option<ObjectGuid>,
        quest_id: u32,
        instant_respawn: bool,
        can_loop_path: bool,
    ) {
        if self.escort_in_combat(creature)
            || self.escort_has_state(creature, STATE_ESCORT_ESCORTING)
        {
            return;
        }
        let Some(entry) = self.creatures.get(&creature).map(|c| c.original_entry) else {
            return;
        };
        let waypoints: Vec<EscortWaypoint> = self
            .stores
            .script_waypoints
            .list(entry)
            .map(|rows| {
                rows.iter()
                    .map(|r| EscortWaypoint {
                        id: r.point_id,
                        x: r.x,
                        y: r.y,
                        z: r.z,
                        wait_time_ms: r.wait_time_ms,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if waypoints.is_empty() {
            // Fork logs a DB error and does not start.
            return;
        }
        let start = self.unit_position(creature).unwrap_or_default();
        self.escorts.insert(
            creature,
            EscortKind::Escort(EscortData {
                player_guid: player.unwrap_or(ObjectGuid::EMPTY),
                quest_id,
                waypoints,
                current_idx: 0,
                state: STATE_ESCORT_ESCORTING,
                wp_wait_timer: 2500,
                player_check_timer: 1000,
                is_running: run,
                can_instant_respawn: instant_respawn,
                can_return_to_start: can_loop_path,
                pathfinding_between: true,
                max_player_distance: MAX_PLAYER_DISTANCE,
                combat_start: start,
                combat_start_o: start.o,
            }),
        );
        // `SetUInt32Value(UNIT_NPC_FLAGS, NONE)`.
        let _ = self.object_set_field(creature, u32::from(offsets::UNIT_NPC_FLAGS), 0);
        self.set_walk(creature, !run);
        if let Some(player) = player {
            self.escort_set_player_guid(player, creature);
        }
        self.creature_script_just_started_escort(creature);
    }

    /// `npc_escortAI::Stop`: clear the player's
    /// escorting guid and drop ESCORTING/PAUSED (the
    /// side-map entry is removed; fork keeps the AI
    /// object with state NONE).
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn escort_stop(&mut self, creature: ObjectGuid) {
        if let Some(player) = self.escort_player(creature) {
            self.escort_set_player_guid(player, ObjectGuid::EMPTY);
        }
        self.escorts.remove(&creature);
    }

    /// `SetEscortPaused`.
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn escort_set_paused(&mut self, creature: ObjectGuid, paused: bool) {
        if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
            if d.state & STATE_ESCORT_ESCORTING == 0 {
                return;
            }
            if paused {
                d.state |= STATE_ESCORT_PAUSED;
            } else {
                d.state &= !STATE_ESCORT_PAUSED;
            }
        }
    }

    /// `SetRun`.
    pub(crate) fn escort_set_run(&mut self, creature: ObjectGuid, run: bool) {
        let changed = match self.escorts.get_mut(&creature) {
            Some(EscortKind::Escort(d)) => {
                let changed = d.is_running != run;
                d.is_running = run;
                changed
            }
            _ => false,
        };
        if changed {
            self.set_walk(creature, !run);
        }
    }

    /// `SetCurrentWaypoint`.
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn escort_set_current_waypoint(&mut self, creature: ObjectGuid, idx: usize) {
        if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature)
            && idx < d.waypoints.len()
        {
            d.current_idx = idx;
        }
    }

    /// `GetCurrentWaypoint` (waypoint id, 0 when not
    /// escorting).
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn escort_current_waypoint(&self, creature: ObjectGuid) -> u32 {
        match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => d.waypoints.get(d.current_idx).map_or(0, |wp| wp.id),
            _ => 0,
        }
    }

    /// `FollowerAI::StartFollow`
    /// (`ScriptedFollowerAI.cpp:292`). `faction` 0
    /// keeps the template faction; `quest_id` 0 is
    /// the fork's null quest.
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn follower_start(
        &mut self,
        creature: ObjectGuid,
        leader: ObjectGuid,
        faction: u32,
        quest_id: u32,
        follow_distance: f32,
    ) {
        let has_victim = self.current_victim(creature).is_some();
        if has_victim || self.escort_has_state(creature, STATE_FOLLOW_INPROGRESS) {
            return;
        }
        let dist = if follow_distance > 0.0 {
            follow_distance
        } else {
            PET_FOLLOW_DIST
        };
        self.escorts.insert(
            creature,
            EscortKind::Follower(FollowerData {
                leader_guid: leader,
                quest_id,
                follow_distance: dist,
                state: STATE_FOLLOW_INPROGRESS,
                update_timer: 300,
            }),
        );
        if faction != 0 {
            // `SetFactionTemplateId`.
            let _ = self.object_set_field(
                creature,
                u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
                faction,
            );
        }
        let _ = self.object_set_field(creature, u32::from(offsets::UNIT_NPC_FLAGS), 0);
        self.creature_move_follow(creature, leader, dist, PET_FOLLOW_ANGLE);
    }

    /// `SetFollowPaused` (`ScriptedFollowerAI.cpp:
    /// 378`).
    pub(crate) fn follower_set_paused(&mut self, creature: ObjectGuid, paused: bool) {
        let state = match self.escorts.get(&creature) {
            Some(EscortKind::Follower(d)) => d.state,
            _ => return,
        };
        if state & STATE_FOLLOW_INPROGRESS == 0 || state & STATE_FOLLOW_COMPLETE != 0 {
            return;
        }
        if paused {
            if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
                d.state |= STATE_FOLLOW_PAUSED;
            }
            if self.follow_target_of(creature).is_some() {
                self.creature_move_idle(creature);
            }
        } else {
            let dist = match self.escorts.get_mut(&creature) {
                Some(EscortKind::Follower(d)) => {
                    d.state &= !STATE_FOLLOW_PAUSED;
                    d.follow_distance
                }
                _ => return,
            };
            if let Some(leader) = self.follower_leader(creature) {
                self.creature_move_follow(creature, leader, dist, PET_FOLLOW_ANGLE);
            }
        }
    }

    /// `SetFollowComplete` (`ScriptedFollowerAI.cpp:
    /// 358`).
    #[allow(dead_code)] // escort script ports call these as they land
    pub(crate) fn follower_set_complete(&mut self, creature: ObjectGuid, with_end_event: bool) {
        if self.follow_target_of(creature).is_some() {
            self.creature_move_idle(creature);
        }
        if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
            if with_end_event {
                d.state |= STATE_FOLLOW_POSTEVENT;
            } else {
                d.state &= !STATE_FOLLOW_POSTEVENT;
            }
            d.state |= STATE_FOLLOW_COMPLETE;
        }
    }

    /// `GetLeaderForFollower`: alive leader on this
    /// map, else the first alive in-range group
    /// member becomes the new leader, else none.
    pub(crate) fn follower_leader(&mut self, creature: ObjectGuid) -> Option<ObjectGuid> {
        let leader = match self.escorts.get(&creature) {
            Some(EscortKind::Follower(d)) => d.leader_guid,
            _ => return None,
        };
        let alive = |g: ObjectGuid| {
            self.players
                .get(&g)
                .is_some_and(|e| e.player.unit.is_alive())
        };
        if alive(leader) {
            return Some(leader);
        }
        if self.players.contains_key(&leader) {
            for member in self.escort_group_members(leader) {
                if alive(member)
                    && self
                        .escort_unit_distance(creature, member)
                        .is_some_and(|d| d <= MAX_PLAYER_DISTANCE)
                {
                    if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
                        d.leader_guid = member;
                    }
                    return Some(member);
                }
            }
        }
        None
    }

    /// `IsPlayerOrGroupInRange`
    /// (`ScriptedEscortAI.cpp:161`).
    fn escort_player_or_group_in_range(
        &self,
        creature: ObjectGuid,
        player: ObjectGuid,
        max_dist: f32,
    ) -> bool {
        if !self.players.contains_key(&player) {
            return false;
        }
        self.escort_group_members(player).into_iter().any(|member| {
            self.escort_unit_distance(creature, member)
                .is_some_and(|d| d <= max_dist)
        })
    }

    /// `Creature::DisappearAndDie` reduced to the
    /// engine despawn: instant fork `Respawn()` is
    /// delay 0, otherwise the spawn's respawn delay
    /// (as on creature death, `spells.rs` Kill arm).
    fn escort_disappear(&mut self, creature: ObjectGuid, instant_respawn: bool) {
        let delay = if instant_respawn {
            0
        } else {
            self.creatures.get(&creature).map_or(0, |c| c.respawn_delay)
        };
        self.escorts.remove(&creature);
        self.despawn_creature_now(creature, delay);
    }

    /// End-of-waypoints arm of `npc_escortAI::
    /// UpdateAI` (also `ResetEscort` shape with
    /// `clear_player` false).
    fn escort_finish(&mut self, creature: ObjectGuid) {
        let (can_return, instant) = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => (d.can_return_to_start, d.can_instant_respawn),
            _ => return,
        };
        if can_return {
            let home = self
                .creatures
                .get(&creature)
                .map_or_else(Position::default, |c| c.home_position);
            self.creature_move_point(creature, POINT_HOME, home, MOVE_PATHFINDING, 0.0, 0.0);
            if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                d.wp_wait_timer = 0;
            }
            return;
        }
        let flags = self.object_get_field(creature, offsets::UNIT_NPC_FLAGS);
        let _ = self.object_set_field(
            creature,
            u32::from(offsets::UNIT_NPC_FLAGS),
            flags | UNIT_NPC_FLAG_QUESTGIVER,
        );
        if let Some(player) = self.escort_player(creature) {
            self.escort_set_player_guid(player, ObjectGuid::EMPTY);
        }
        self.escort_disappear(creature, instant);
    }

    /// `npc_escortAI::JustDied` (also the fork's
    /// synthetic failure call from the range check):
    /// group quest fail + clear escorting guid.
    fn escort_just_died(&mut self, creature: ObjectGuid) {
        let (player, quest_id) = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d))
                if d.state & STATE_ESCORT_ESCORTING != 0
                    && d.player_guid != ObjectGuid::EMPTY
                    && d.quest_id != 0 =>
            {
                (d.player_guid, d.quest_id)
            }
            _ => return,
        };
        if self.players.contains_key(&player) {
            self.group_event_fail_happens(player, quest_id);
            self.escort_set_player_guid(player, ObjectGuid::EMPTY);
        }
    }

    /// `ResetEscort` (`ScriptedEscortAI.cpp:272`).
    fn escort_reset_escort(&mut self, creature: ObjectGuid) {
        let instant = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => d.can_instant_respawn,
            _ => return,
        };
        let flags = self.object_get_field(creature, offsets::UNIT_NPC_FLAGS);
        let _ = self.object_set_field(
            creature,
            u32::from(offsets::UNIT_NPC_FLAGS),
            flags | UNIT_NPC_FLAG_QUESTGIVER,
        );
        self.escort_disappear(creature, instant);
    }

    /// Per-tick escort/follower update (`UpdateAI`
    /// of both classes), called from `Map::tick`.
    pub(crate) fn update_escorts(&mut self, diff_ms: u32) {
        let guids: Vec<ObjectGuid> = self.escorts.keys().copied().collect();
        for guid in guids {
            if !self.escort_creature_alive(guid) {
                continue;
            }
            match self.escorts.get(&guid) {
                Some(EscortKind::Escort(_)) => self.escort_tick_one(guid, diff_ms),
                Some(EscortKind::Follower(_)) => self.follower_tick_one(guid, diff_ms),
                None => {}
            }
        }
    }

    fn escort_tick_one(&mut self, creature: ObjectGuid, diff_ms: u32) {
        let in_combat = self.escort_in_combat(creature);
        let snapshot = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => (
                d.state,
                d.wp_wait_timer,
                d.current_idx,
                d.player_guid,
                d.max_player_distance,
            ),
            _ => return,
        };
        let (state, timer, idx, player, max_dist) = snapshot;
        // Waypoint updating.
        if state & STATE_ESCORT_ESCORTING != 0
            && !in_combat
            && timer > 0
            && state & STATE_ESCORT_RETURNING == 0
        {
            if timer <= diff_ms {
                let wp = match self.escorts.get(&creature) {
                    Some(EscortKind::Escort(d)) => d.waypoints.get(idx).copied(),
                    _ => return,
                };
                match wp {
                    None => {
                        self.escort_finish(creature);
                        return;
                    }
                    Some(wp) => {
                        if state & STATE_ESCORT_PAUSED == 0 {
                            let (options, run) = match self.escorts.get(&creature) {
                                Some(EscortKind::Escort(d)) => {
                                    let mut options = if d.pathfinding_between {
                                        MOVE_PATHFINDING
                                    } else {
                                        0
                                    };
                                    options |= if d.is_running {
                                        MOVE_RUN_MODE
                                    } else {
                                        MOVE_WALK_MODE
                                    };
                                    (options, d.is_running)
                                }
                                _ => return,
                            };
                            let _ = run;
                            self.creature_move_point(
                                creature,
                                wp.id,
                                Position {
                                    x: wp.x,
                                    y: wp.y,
                                    z: wp.z,
                                    o: 0.0,
                                },
                                options,
                                0.0,
                                0.0,
                            );
                            self.creature_script_escort_waypoint_start(creature, wp.id);
                            if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                                d.wp_wait_timer = 0;
                            } else {
                                return;
                            }
                        }
                    }
                }
            } else if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                d.wp_wait_timer = timer - diff_ms;
            }
        }
        // Player/group range check.
        let state = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => d.state,
            _ => return,
        };
        if state & STATE_ESCORT_ESCORTING != 0
            && player != ObjectGuid::EMPTY
            && max_dist > 0.0
            && !self.escort_in_combat(creature)
            && state & STATE_ESCORT_RETURNING == 0
        {
            let check = match self.escorts.get(&creature) {
                Some(EscortKind::Escort(d)) => d.player_check_timer,
                _ => return,
            };
            if check < diff_ms {
                if !self.escort_player_or_group_in_range(creature, player, max_dist) {
                    self.escort_just_died(creature);
                    self.escort_reset_escort(creature);
                    return;
                }
                if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                    d.player_check_timer = 1000;
                }
            } else if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                d.player_check_timer = check - diff_ms;
            }
        }
        // `UpdateEscortAI` override point.
        self.creature_script_escort_update(creature, diff_ms);
    }

    fn follower_tick_one(&mut self, creature: ObjectGuid, diff_ms: u32) {
        let (state, timer, quest_id, follow_dist) = match self.escorts.get(&creature) {
            Some(EscortKind::Follower(d)) => {
                (d.state, d.update_timer, d.quest_id, d.follow_distance)
            }
            _ => return,
        };
        if state & STATE_FOLLOW_INPROGRESS != 0 && self.current_victim(creature).is_none() {
            if timer < diff_ms {
                if state & STATE_FOLLOW_COMPLETE != 0 && state & STATE_FOLLOW_POSTEVENT == 0 {
                    self.escort_disappear(creature, false);
                    return;
                }
                let mut abort = true;
                if let Some(leader) = self.follower_leader(creature) {
                    let state = match self.escorts.get(&creature) {
                        Some(EscortKind::Follower(d)) => d.state,
                        _ => return,
                    };
                    if state & STATE_FOLLOW_RETURNING != 0 {
                        if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
                            d.state &= !STATE_FOLLOW_RETURNING;
                        }
                        self.creature_move_follow(creature, leader, follow_dist, PET_FOLLOW_ANGLE);
                        return;
                    }
                    if self.escort_player_or_group_in_range(creature, leader, MAX_PLAYER_DISTANCE) {
                        abort = false;
                    }
                    if !abort
                        && quest_id != 0
                        && state & STATE_FOLLOW_COMPLETE == 0
                        && self
                            .players
                            .get(&leader)
                            .is_some_and(|e| e.player.quest_status(quest_id) != 3)
                    {
                        abort = true;
                    }
                }
                if abort {
                    self.follower_set_paused(creature, false);
                    self.creature_script_escort_failed(creature, false);
                    self.escort_disappear(creature, false);
                    return;
                }
                if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
                    d.update_timer = 300;
                }
            } else if let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature) {
                d.update_timer = timer - diff_ms;
            }
        }
        // `UpdateFollowerAI` override point.
        self.creature_script_escort_update(creature, diff_ms);
    }

    /// `MovementInform(POINT_MOTION_TYPE, id)` of
    /// both classes, from `process_movement_informs`
    /// before the script inform dispatcher.
    pub(crate) fn escort_movement_inform(&mut self, creature: ObjectGuid, point_id: u32) {
        match self.escorts.get(&creature) {
            Some(EscortKind::Escort(_)) => self.escort_movement_inform_escort(creature, point_id),
            Some(EscortKind::Follower(_)) => {
                self.escort_movement_inform_follower(creature, point_id);
            }
            None => {}
        }
    }

    fn escort_movement_inform_escort(&mut self, creature: ObjectGuid, point_id: u32) {
        let (state, idx) = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => (d.state, d.current_idx),
            _ => return,
        };
        if state & STATE_ESCORT_ESCORTING == 0 {
            return;
        }
        if point_id == POINT_LAST_POINT {
            let running = match self.escorts.get_mut(&creature) {
                Some(EscortKind::Escort(d)) => {
                    d.state &= !STATE_ESCORT_RETURNING;
                    if d.wp_wait_timer == 0 {
                        d.wp_wait_timer = 1;
                    }
                    d.is_running
                }
                _ => return,
            };
            self.set_walk(creature, !running);
            return;
        }
        if point_id == POINT_HOME {
            if state & STATE_ESCORT_RETURNING != 0 {
                return;
            }
            if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                d.current_idx = 0;
                d.wp_wait_timer = 1;
            }
            return;
        }
        if state & STATE_ESCORT_RETURNING != 0 {
            return;
        }
        let wp = match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) => d.waypoints.get(idx).copied(),
            _ => return,
        };
        let Some(wp) = wp else {
            return;
        };
        let pos = self.unit_position(creature).unwrap_or_default();
        let dx = pos.x - wp.x;
        let dy = pos.y - wp.y;
        let dz = pos.z - wp.z;
        if (dx * dx + dy * dy + dz * dz).sqrt() > 10.0 {
            if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                d.wp_wait_timer = 1;
            }
            return;
        }
        if wp.id != point_id {
            return;
        }
        self.creature_script_escort_waypoint_reached(creature, wp.id);
        if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
            d.wp_wait_timer = wp.wait_time_ms + 1;
            d.current_idx = idx + 1;
        }
    }

    fn escort_movement_inform_follower(&mut self, creature: ObjectGuid, point_id: u32) {
        /// `POINT_COMBAT_START`
        /// (`ScriptedFollowerAI.cpp:22`).
        const POINT_COMBAT_START: u32 = 0xFF_FFFF;
        let state = match self.escorts.get(&creature) {
            Some(EscortKind::Follower(d)) => d.state,
            _ => return,
        };
        if state & STATE_FOLLOW_INPROGRESS == 0 || point_id != POINT_COMBAT_START {
            return;
        }
        if self.follower_leader(creature).is_some() {
            if state & STATE_FOLLOW_PAUSED == 0
                && let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature)
            {
                d.state |= STATE_FOLLOW_RETURNING;
            }
        } else {
            // Fork: self-damage to death, teleport
            // home, forced despawn ("ugly fix");
            // reduced to death + despawn (the death
            // itself raises the quest-fail hook).
            self.follower_set_paused(creature, false);
            self.kill_unit(creature, None);
            self.escort_disappear(creature, false);
        }
    }

    /// `npc_escortAI::EnterCombat` position record,
    /// from the fresh-combat arm of `enter_combat`.
    pub(crate) fn escort_on_enter_combat(&mut self, creature: ObjectGuid) {
        let pos = self.unit_position(creature).unwrap_or_default();
        if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature)
            && d.state & STATE_ESCORT_RETURNING == 0
        {
            d.combat_start = pos;
            d.combat_start_o = pos.o;
        }
    }

    /// Evade arm of both classes. Returns true when
    /// the escort/follower return replaced the
    /// walk home.
    pub(crate) fn escort_on_evade(&mut self, creature: ObjectGuid) -> bool {
        match self.escorts.get(&creature) {
            Some(EscortKind::Escort(d)) if d.state & STATE_ESCORT_ESCORTING != 0 => {
                // `ReturnToCombatStartPosition`
                // (`ScriptedEscortAI.cpp:494`); the
                // generic evade already cleared motion,
                // so the fork's generator screen is met.
                let (start, start_o) = (d.combat_start, d.combat_start_o);
                let pos = self.unit_position(creature).unwrap_or_default();
                let dx = pos.x - start.x;
                let dy = pos.y - start.y;
                if (dx * dx + dy * dy).sqrt() > 1000.0 {
                    // Fork resets the record and informs
                    // synthetically (with its fPosY typo
                    // unobservable through our Position).
                    if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                        d.combat_start = pos;
                        d.state |= STATE_ESCORT_RETURNING;
                    }
                    self.escort_movement_inform_escort(creature, POINT_LAST_POINT);
                    return true;
                }
                if let Some(EscortKind::Escort(d)) = self.escorts.get_mut(&creature) {
                    d.state |= STATE_ESCORT_RETURNING;
                }
                self.creature_move_point(
                    creature,
                    POINT_LAST_POINT,
                    start,
                    MOVE_RUN_MODE | MOVE_PATHFINDING,
                    0.0,
                    start_o,
                );
                true
            }
            Some(EscortKind::Follower(_))
                if self.escort_has_state(creature, STATE_FOLLOW_INPROGRESS) =>
            {
                if !self.escort_has_state(creature, STATE_FOLLOW_PAUSED)
                    && let Some(EscortKind::Follower(d)) = self.escorts.get_mut(&creature)
                {
                    d.state |= STATE_FOLLOW_RETURNING;
                }
                true
            }
            _ => false,
        }
    }

    /// `JustDied` of both classes, from `kill_unit`
    /// beside the script just-died dispatcher.
    pub(crate) fn escort_on_just_died(&mut self, creature: ObjectGuid) {
        match self.escorts.get(&creature) {
            Some(EscortKind::Escort(_)) => {
                self.escort_just_died(creature);
                self.escorts.remove(&creature);
            }
            Some(EscortKind::Follower(d)) => {
                let (state, quest_id) = (d.state, d.quest_id);
                if state & STATE_FOLLOW_INPROGRESS != 0 && quest_id != 0 {
                    if let Some(leader) = self.follower_leader(creature) {
                        let members = self.escort_group_members(leader);
                        for member in members {
                            if self
                                .players
                                .get(&member)
                                .is_some_and(|e| e.player.quest_status(quest_id) == 3)
                            {
                                self.fail_quest(member, quest_id);
                            }
                        }
                        self.creature_script_escort_failed(creature, true);
                    }
                }
                self.escorts.remove(&creature);
            }
            None => {}
        }
    }
}
