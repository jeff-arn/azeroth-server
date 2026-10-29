//! Battleground manager + queues + session handlers
//! (`BattleGroundMgr`, `BattleGroundQueue`,
//! `Handlers/BattleGroundHandler.cpp`; Tier 3 Slice 3).
//! Match simulation lives in `mangos_map::battlegrounds`;
//! this module owns everything session-side: queues,
//! invites, port-in/out, BG raid groups, login/logout.

use std::collections::{HashMap, HashSet, VecDeque};

use mangos_entities::battleground::{
    MAX_BATTLEGROUND_BRACKETS, MAX_BATTLEGROUND_QUEUES, bg_type_of_queue, bracket_id_from_level,
    max_level_for_bracket, min_level_for_bracket, queue_type_of,
};
use mangos_entities::player::Player;
use mangos_guid::ObjectGuid;
use mangos_log::Level;
use mangos_map::battlegrounds::{
    BgInit, BgStatusInfo, bg_status, build_battleground_status_packet, build_group_joined_packet,
};
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::battlegrounds::{bg_map_id_by_type, bg_type_by_map_id};

#[allow(clippy::wildcard_imports)]
use super::*;

/// `INVITE_ACCEPT_WAIT_TIME`.
const INVITE_ACCEPT_WAIT_TIME_MS: u32 = 80_000;
/// `INVITATION_REMIND_TIME`.
const INVITATION_REMIND_TIME_MS: u32 = 40_000;
/// `OFFLINE_BG_QUEUE_TIME`.
const OFFLINE_BG_QUEUE_TIME_MS: u32 = 60_000;
/// `COUNT_OF_PLAYERS_TO_AVERAGE_WAIT_TIME`.
const COUNT_AVERAGE: usize = 10;
/// Deserter debuff spell.
const SPELL_DESERTER: u32 = 26013;
// Queue-store indexes (`enum BattleGroundQueueGroupTypes`).
const Q_PREMADE_ALLIANCE: usize = 0;
#[allow(dead_code)] // fork lane index, reached via team_queue_index arithmetic
const Q_PREMADE_HORDE: usize = 1;
/// `UNIT_NPC_FLAG_BATTLEMASTER`.
const NPC_FLAG_BATTLEMASTER: u32 = 0x0000_0400;
const Q_NORMAL_ALLIANCE: usize = 2;
const Q_NORMAL_HORDE: usize = 3;
const QUEUE_GROUP_TYPES: usize = 4;
// `GroupJoinBattlegroundResult` (`BattleGroundDefines.h`).
const BG_JOIN_ERR_OK: u8 = 0;
const BG_JOIN_ERR_OFFLINE_MEMBER: u8 = 1;
const BG_JOIN_ERR_GROUP_TOO_MANY: u8 = 2;
const BG_JOIN_ERR_MIXED_FACTION: u8 = 3;
// const BG_JOIN_ERR_MIXED_LEVELS: u8 = 4; (unused by CanJoin)
const BG_JOIN_ERR_MEMBER_IN_QUEUE: u8 = 6;
const BG_JOIN_ERR_GROUP_DESERTER: u8 = 7;
const BG_JOIN_ERR_ALL_QUEUES_USED: u8 = 8;
const BG_JOIN_ERR_GROUP_NOT_ENOUGH: u8 = 9;
// `BattleGroundGroupJoinStatus`
const BG_GROUPJOIN_DESERTERS: i32 = -2;
const BG_GROUPJOIN_FAILED: i32 = -1;
// Language ids.
const LANG_YOUR_BG_LEVEL_REQ_ERROR: i32 = 715;
const LANG_BG_QUEUE_ANNOUNCE_SELF: i32 = 711;
const LANG_BG_QUEUE_ANNOUNCE_WORLD: i32 = 712;
const LANG_BG_STARTED_ANNOUNCE_WORLD: i32 = 717;
const LANG_BG_GROUP_OFFLINE_MEMBER: i32 = 727;
const LANG_BG_GROUP_TOO_LARGE: i32 = 720;
const LANG_BG_GROUP_MIXED_FACTION: i32 = 728;
const LANG_BG_GROUP_MIXED_LEVELS: i32 = 729;
const LANG_BG_GROUP_MEMBER_ALREADY_IN_QUEUE: i32 = 730;
const LANG_BG_GROUP_MEMBER_DESERTER: i32 = 731;
const LANG_BG_GROUP_MEMBER_NO_FREE_QUEUE_SLOTS: i32 = 732;

/// `PlayerQueueInfo` (online marker is per queue entry
/// in the fork; here stored on the queued-player row).
#[derive(Clone, Copy, Debug, Default)]
struct PlayerQueueInfo {
    online: bool,
    last_online_ms: u32,
}

/// `GroupQueueInfo`.
#[derive(Clone, Debug)]
struct GroupQueueInfo {
    #[allow(dead_code)] // fork id mirrored; map key restates it
    id: u64,
    bg_type_id: u32,
    bracket_id: u32,
    team: Team,
    invited_to_instance: u32,
    join_time_ms: u32,
    remove_invite_time_ms: u32,
    desired_client_instance_id: u32,
    players: Vec<u32>,
}

/// One `BattleGroundQueue` (per queue type).
#[derive(Default)]
struct BgQueue {
    /// [bracket][group type] -> queued group ids.
    queued_groups: [[VecDeque<u64>; QUEUE_GROUP_TYPES]; MAX_BATTLEGROUND_BRACKETS as usize],
    infos: HashMap<u64, GroupQueueInfo>,
    queued_players: HashMap<u32, (u64, PlayerQueueInfo)>,
    wait_times: [[[u32; COUNT_AVERAGE]; MAX_BATTLEGROUND_BRACKETS as usize]; 2],
    wait_sums: [[u32; MAX_BATTLEGROUND_BRACKETS as usize]; 2],
    wait_last: [[usize; MAX_BATTLEGROUND_BRACKETS as usize]; 2],
}

/// `SelectionPool`.
#[derive(Default)]
struct SelectionPool {
    selected: Vec<u64>,
    player_count: u32,
}

impl SelectionPool {
    fn init(&mut self) {
        self.selected.clear();
        self.player_count = 0;
    }
}

/// One live match handle (manager projection of the
/// map-side `BgState`).
#[derive(Clone, Debug)]
pub struct BgHandle {
    pub map_key: MapKey,
    pub bg_type_id: u32,
    pub bracket_id: u32,
    pub client_instance_id: u32,
    /// BG raid group keys by team index (0 alliance).
    pub raid_keys: [Option<u64>; 2],
}

/// Deadline kinds replacing fork `BasicEvent`s.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BgTimerKind {
    InviteRemind,
    InviteRemove,
}

#[derive(Clone, Copy, Debug)]
struct BgTimer {
    when_ms: u32,
    kind: BgTimerKind,
    guid_low: u32,
    bg_instance: u32,
    bg_type_id: u32,
    bg_queue_type_id: u32,
    remove_time_ms: u32,
}

/// `BattleGroundMgr` runtime state.
#[derive(Default)]
pub struct BgManager {
    /// Indexed by queue type id (0 unused, 1..=3).
    queues: [BgQueue; 4],
    pools: [SelectionPool; 2],
    /// Live matches by instance id.
    pub live: HashMap<u32, BgHandle>,
    /// `m_bgFreeSlotQueue[type]`, front = most recent.
    free_slots: [VecDeque<u32>; 4],
    /// `m_clientBattleGroundIds[type][bracket]`.
    client_ids: [[HashSet<u32>; MAX_BATTLEGROUND_BRACKETS as usize]; 4],
    scheduled: Vec<(u32, u32, u32)>,
    timers: Vec<BgTimer>,
    /// `m_testing` (`.debug bg` toggle).
    pub testing: bool,
    next_group_info_id: u64,
}

impl BgManager {
    fn queue(&mut self, queue_type_id: u32) -> &mut BgQueue {
        &mut self.queues[(queue_type_id as usize) % 4]
    }
    fn queue_ro(&self, queue_type_id: u32) -> &BgQueue {
        &self.queues[(queue_type_id as usize) % 4]
    }
}

fn team_queue_index(team: Team, premade: bool) -> usize {
    let mut index = if premade {
        Q_PREMADE_ALLIANCE
    } else {
        Q_NORMAL_ALLIANCE
    };
    if team == Team::Horde {
        index += 1;
    }
    index
}

impl World {
    // ------------------------------------------------ accessors

    pub(crate) fn bg_player(&self, guid_low: u32) -> Option<&Player> {
        let key = self.player_map.get(&guid_low).copied()?;
        self.maps.get(&key)?.player(player_guid(guid_low))
    }

    fn bg_send(&mut self, guid_low: u32, packet: WorldPacket) {
        if let Some(id) = self.player_session.get(&guid_low).copied() {
            self.reply_packet(id, packet);
        }
    }

    fn bg_template_min_max(&self, bg_type_id: u32) -> Option<(u32, u32, u32, u32)> {
        let t = self.deps.world_stores.battlegrounds.get(bg_type_id)?;
        Some((
            t.min_players_per_team,
            t.max_players_per_team,
            t.min_level,
            t.max_level,
        ))
    }

    /// `Player::GetBattleGroundBracketIdFromLevel(type)`
    /// for an online player (0 when not applicable; the
    /// fork reads the template min level).
    fn bg_bracket_of(&self, guid_low: u32, bg_type_id: u32) -> Option<u32> {
        let level = self.bg_player(guid_low)?.unit.level();
        let t = self.deps.world_stores.battlegrounds.get(bg_type_id)?;
        bracket_id_from_level(t.min_level, level)
    }

    fn bg_can_join(&self, guid_low: u32) -> bool {
        // `CanJoinToBattleground`: no Deserter aura.
        self.bg_player(guid_low)
            .is_some_and(|p| !p.unit.auras.iter().any(|a| a.spell_id == SPELL_DESERTER))
    }

    fn bg_template_status_info(bg_type_id: u32) -> BgStatusInfo {
        // Fork uses the template BG object (instance 0,
        // bracket 0 after template Reset).
        BgStatusInfo {
            map_id: bg_map_id_by_type(bg_type_id),
            bracket_id: 0,
            client_instance_id: 0,
        }
    }

    fn bg_status_info(&self, instance_id: u32) -> Option<BgStatusInfo> {
        let handle = self.bg_manager.live.get(&instance_id)?;
        Some(BgStatusInfo {
            map_id: handle.map_key.map_id,
            bracket_id: handle.bracket_id,
            client_instance_id: handle.client_instance_id,
        })
    }

    /// `GetBattleGround(instanceId, typeId)`.
    fn bg_live(&self, instance_id: u32, bg_type_id: u32) -> Option<&BgHandle> {
        self.bg_manager
            .live
            .get(&instance_id)
            .filter(|h| h.bg_type_id == bg_type_id)
    }

    /// `GetBattleGroundThroughClientInstance`.
    fn bg_through_client(&self, client_id: u32, bg_type_id: u32) -> Option<&BgHandle> {
        self.bg_manager
            .live
            .values()
            .find(|h| h.bg_type_id == bg_type_id && h.client_instance_id == client_id)
    }

    // ------------------------------------------------ map bootstrap

    fn bg_map_config(&self) -> MapConfig {
        MapConfig {
            pvp_realm: matches!(self.config.game_type, 1 | 8 | 16),
            xp_rates: mangos_map::map::XpRates {
                kill: self.config.rate_xp_kill,
                kill_elite: self.config.rate_xp_kill_elite,
                quest: self.config.rate_xp_quest,
                explore: self.config.rate_xp_explore,
            },
            activate_weather: self.config.activate_weather,
            change_weather_interval_ms: self.config.change_weather_interval_ms,
            proc_delay_ms: self.config.spell_proc_delay_ms,
            bg_premature_finish_ms: self.config.battleground.premature_finish_timer_ms,
            ..MapConfig::default()
        }
    }

    pub(crate) fn ensure_map(&mut self, map_key: MapKey) -> &mut Map {
        let map_id = map_key.map_id;
        let maps_dir = self.deps.maps_dir.clone();
        let navmesh_dir = self.deps.mmaps_dir.clone();
        let vmaps_dir = self.deps.vmaps_dir.clone();
        let (vmap_los, vmap_height) = (self.config.vmap_enable_los, self.config.vmap_enable_height);
        let map_config = self.bg_map_config();
        self.maps.entry(map_key).or_insert_with(|| {
            let mut map = Map::new(
                map_id,
                map_key.instance_id,
                map_config,
                Arc::clone(&self.deps.world_stores),
                WorldRng::from_entropy(),
            );
            map.set_maps_dir(maps_dir);
            map.set_mmaps_dir(navmesh_dir);
            map.set_vmaps_dir(vmaps_dir, vmap_los, vmap_height);
            map
        })
    }

    /// `CreateNewBattleGround`.
    fn bg_create_new(
        &mut self,
        bg_type_id: u32,
        bracket_id: u32,
        level_min: u32,
        level_max: u32,
    ) -> Option<u32> {
        let template = self
            .deps
            .world_stores
            .battlegrounds
            .get(bg_type_id)?
            .clone();
        let map_id = template.map_id;
        // instance id from the shared allocator (fork:
        // `sMapMgr.GenerateInstanceId`).
        let instance_id = self.instances.allocate(map_id);
        // client-visible id: lowest unused for the bracket.
        let mut client_id = 1u32;
        let used = &self.bg_manager.client_ids[bg_type_id as usize][bracket_id as usize];
        while used.contains(&client_id) {
            client_id += 1;
        }
        self.bg_manager.client_ids[bg_type_id as usize][bracket_id as usize].insert(client_id);
        let key = MapKey {
            map_id,
            instance_id,
        };
        let map = self.ensure_map(key);
        map.init_battleground(BgInit::from_template(
            &template, bracket_id, client_id, level_min, level_max,
        ));
        self.bg_manager.live.insert(
            instance_id,
            BgHandle {
                map_key: key,
                bg_type_id,
                bracket_id,
                client_instance_id: client_id,
                raid_keys: [None, None],
            },
        );
        self.log.out(
            Level::Detail,
            &format!("battleground created: type {bg_type_id} instance {instance_id} client {client_id} bracket {bracket_id}"),
        );
        Some(instance_id)
    }

    // ------------------------------------------------ free slots / delete

    pub(crate) fn bg_free_slot_queue_set(&mut self, key: MapKey, present: bool) {
        let Some(handle) = self
            .bg_manager
            .live
            .values()
            .find(|h| h.map_key == key)
            .cloned()
        else {
            return;
        };
        let queue = &mut self.bg_manager.free_slots[handle.bg_type_id as usize];
        queue.retain(|id| *id != key.instance_id);
        if present {
            queue.push_front(key.instance_id);
        }
    }

    /// Fork `delete this` for an empty uninvited BG:
    /// drop the map + handle, release the client id.
    pub(crate) fn bg_delete_map(&mut self, key: MapKey) {
        let Some(handle) = self.bg_manager.live.remove(&key.instance_id) else {
            return;
        };
        self.bg_manager.client_ids[handle.bg_type_id as usize][handle.bracket_id as usize]
            .remove(&handle.client_instance_id);
        self.bg_manager.free_slots[handle.bg_type_id as usize].retain(|id| *id != key.instance_id);
        // Disband raids (fork `~BattleGround` deletes
        // the groups).
        for team_idx in 0..2 {
            if let Some(raid_key) = handle.raid_keys[team_idx] {
                let statuses = self.statuses_for_group(raid_key, &[]);
                let view = |low: u32| statuses.get(&low).copied();
                let fx = self
                    .groups
                    .get_mut(&raid_key)
                    .map(|g| g.disband(true, &view))
                    .unwrap_or_default();
                self.destroy_group(raid_key);
                self.apply_group_fx(fx);
            }
        }
        // Timer cleanup for invites to the dead BG.
        self.bg_manager
            .timers
            .retain(|t| t.bg_instance != key.instance_id);
        let empty = self.maps.get(&key).is_some_and(|m| m.player_count() == 0);
        if empty {
            self.maps.remove(&key);
            self.instances.release(key.instance_id);
        }
    }

    pub(crate) fn bg_schedule_queue_update(
        &mut self,
        queue_type_id: u32,
        bg_type_id: u32,
        bracket_id: u32,
    ) {
        self.bg_manager
            .scheduled
            .push((queue_type_id, bg_type_id, bracket_id));
    }

    // ------------------------------------------------ queue core

    #[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
    fn bg_queue_add_group(
        &mut self,
        leader_low: u32,
        group_key: Option<u64>,
        bg_type_id: u32,
        bracket_id: u32,
        is_premade: bool,
        desired_client_instance: u32,
        excluded: &[u32],
        leader_name: &str,
    ) -> u64 {
        let queue_type_id = queue_type_of(bg_type_id);
        let team = self.bg_player(leader_low).map_or(Team::None, |p| p.team);
        let now = self.server_ms();
        self.bg_manager.next_group_info_id += 1;
        let ginfo_id = self.bg_manager.next_group_info_id;
        let mut ginfo = GroupQueueInfo {
            id: ginfo_id,
            bg_type_id,
            bracket_id,
            team,
            invited_to_instance: 0,
            join_time_ms: now,
            remove_invite_time_ms: 0,
            desired_client_instance_id: desired_client_instance,
            players: Vec::new(),
        };
        let index = team_queue_index(team, is_premade);
        let members: Vec<u32> = match group_key {
            Some(key) => self
                .groups
                .get(&key)
                .map(|g| g.members().iter().map(|m| m.guid_low).collect())
                .unwrap_or_default(),
            None => vec![leader_low],
        };
        let group_count = group_key.map_or(1, |_| members.len());
        let group_limit = self.config.battleground.group_queue_limit as usize;
        for member in members {
            if self.bg_player(member).is_none() {
                continue;
            }
            if excluded.contains(&member) {
                continue;
            }
            if group_key.is_some() && group_count > group_limit {
                self.bg_queue_add_group(
                    member,
                    None,
                    bg_type_id,
                    bracket_id,
                    false,
                    desired_client_instance,
                    &[],
                    leader_name,
                );
                self.bg_sys_message(
                    member,
                    &format!(
                        "Group queue limit is set to {group_limit}. You have been queued solo."
                    ),
                );
            } else {
                ginfo.players.push(member);
            }
        }
        if ginfo.players.is_empty() {
            return ginfo_id;
        }
        let queue = self.bg_manager.queue(queue_type_id);
        for member in &ginfo.players {
            queue.queued_players.insert(
                *member,
                (
                    ginfo_id,
                    PlayerQueueInfo {
                        online: true,
                        last_online_ms: 0,
                    },
                ),
            );
        }
        queue.queued_groups[bracket_id as usize][index].push_back(ginfo_id);
        // Queue announcer (non-premade only).
        if !is_premade && self.config.battleground.queue_announcer_join != 0 {
            self.bg_queue_announce(bg_type_id, bracket_id, leader_low);
        }
        self.bg_manager
            .queue(queue_type_id)
            .infos
            .insert(ginfo_id, ginfo);
        ginfo_id
    }

    fn bg_queue_announce(&mut self, bg_type_id: u32, bracket_id: u32, leader_low: u32) {
        let Some(template) = self
            .deps
            .world_stores
            .battlegrounds
            .get(bg_type_id)
            .cloned()
        else {
            return;
        };
        let queue_type_id = queue_type_of(bg_type_id);
        let count = |mgr: &Self, index: usize| -> u32 {
            mgr.bg_manager.queue_ro(queue_type_id).queued_groups[bracket_id as usize][index]
                .iter()
                .filter_map(|id| mgr.bg_manager.queue_ro(queue_type_id).infos.get(id))
                .filter(|g| g.invited_to_instance == 0)
                .map(|g| g.players.len() as u32)
                .sum()
        };
        let q_alliance = count(self, Q_NORMAL_ALLIANCE);
        let q_horde = count(self, Q_NORMAL_HORDE);
        let min_players = template.min_players_per_team;
        let args: [&str; 0] = [];
        let _ = args;
        let q_min = min_level_for_bracket(bg_type_id, template.min_level, bracket_id);
        let q_max = max_level_for_bracket(bg_type_id, template.min_level, bracket_id) - 1;
        let text = self.bg_format_lang(
            if self.config.battleground.queue_announcer_join == 1 {
                LANG_BG_QUEUE_ANNOUNCE_SELF
            } else {
                LANG_BG_QUEUE_ANNOUNCE_WORLD
            },
            &[
                template.name.as_str(),
                &q_min.to_string(),
                &q_max.to_string(),
                &q_alliance.to_string(),
                &min_players.saturating_sub(q_alliance).to_string(),
                &q_horde.to_string(),
                &min_players.saturating_sub(q_horde).to_string(),
            ],
        );
        if let Some(text) = text {
            if self.config.battleground.queue_announcer_join == 1 {
                self.bg_sys_message(leader_low, &text);
            } else {
                self.bg_world_sys_message(&text, CHAT_MSG_BG_SYSTEM_NEUTRAL_BYTE);
            }
        }
    }

    /// `RemovePlayer` from one queue.
    fn bg_queue_remove_player(
        &mut self,
        queue_type_id: u32,
        guid_low: u32,
        decrease_invited: bool,
    ) {
        let Some((ginfo_id, _)) = self
            .bg_manager
            .queue_ro(queue_type_id)
            .queued_players
            .get(&guid_low)
            .copied()
        else {
            return;
        };
        let (team, invited_instance, bg_type_id, bracket_id) = {
            let q = self.bg_manager.queue(queue_type_id);
            let Some(ginfo) = q.infos.get_mut(&ginfo_id) else {
                q.queued_players.remove(&guid_low);
                return;
            };
            ginfo.players.retain(|p| *p != guid_low);
            (
                ginfo.team,
                ginfo.invited_to_instance,
                ginfo.bg_type_id,
                ginfo.bracket_id,
            )
        };
        if decrease_invited
            && invited_instance != 0
            && let Some(handle) = self.bg_manager.live.get(&invited_instance).cloned()
            && let Some(map) = self.maps.get_mut(&handle.map_key)
        {
            map.bg_decrease_invited(team);
        }
        let q = self.bg_manager.queue(queue_type_id);
        q.queued_players.remove(&guid_low);
        let empty = q.infos.get(&ginfo_id).is_some_and(|g| g.players.is_empty());
        if empty {
            q.infos.remove(&ginfo_id);
            for groups in &mut q.queued_groups[bracket_id as usize] {
                groups.retain(|id| *id != ginfo_id);
            }
            let _ = bg_type_id;
        }
    }

    fn bg_group_info(&self, queue_type_id: u32, guid_low: u32) -> Option<GroupQueueInfo> {
        let q = self.bg_manager.queue_ro(queue_type_id);
        let (id, _) = q.queued_players.get(&guid_low)?;
        q.infos.get(id).cloned()
    }

    fn bg_is_player_invited(
        &self,
        queue_type_id: u32,
        guid_low: u32,
        instance: u32,
        remove_time_ms: u32,
    ) -> bool {
        self.bg_group_info(queue_type_id, guid_low)
            .is_some_and(|g| {
                g.invited_to_instance == instance && g.remove_invite_time_ms == remove_time_ms
            })
    }

    fn bg_invite_update_wait_time(&mut self, queue_type_id: u32, ginfo: &GroupQueueInfo) {
        let now = self.server_ms();
        let time_in_queue = now.wrapping_sub(ginfo.join_time_ms);
        let team_index = usize::from(ginfo.team == Team::Horde);
        let bracket = ginfo.bracket_id as usize;
        let q = self.bg_manager.queue(queue_type_id);
        let last = q.wait_last[team_index][bracket];
        q.wait_sums[team_index][bracket] -= q.wait_times[team_index][bracket][last];
        q.wait_times[team_index][bracket][last] = time_in_queue;
        q.wait_sums[team_index][bracket] += time_in_queue;
        q.wait_last[team_index][bracket] = (last + 1) % COUNT_AVERAGE;
    }

    fn bg_average_wait_time(&self, queue_type_id: u32, ginfo: &GroupQueueInfo) -> u32 {
        let team_index = usize::from(ginfo.team == Team::Horde);
        let bracket = ginfo.bracket_id as usize;
        let q = self.bg_manager.queue_ro(queue_type_id);
        if q.wait_times[team_index][bracket][COUNT_AVERAGE - 1] != 0 {
            q.wait_sums[team_index][bracket] / COUNT_AVERAGE as u32
        } else {
            0
        }
    }

    /// `InviteGroupToBG`.
    fn bg_invite_group(&mut self, queue_type_id: u32, ginfo_id: u64, instance: u32, side: Team) {
        let now = self.server_ms();
        let (players, bg_type_id, bracket_id) = {
            let q = self.bg_manager.queue(queue_type_id);
            let Some(ginfo) = q.infos.get_mut(&ginfo_id) else {
                return;
            };
            if side != Team::None {
                ginfo.team = side;
            }
            if ginfo.invited_to_instance != 0 {
                return;
            }
            ginfo.invited_to_instance = instance;
            ginfo.remove_invite_time_ms = now.wrapping_add(INVITE_ACCEPT_WAIT_TIME_MS);
            (ginfo.players.clone(), ginfo.bg_type_id, ginfo.bracket_id)
        };
        let Some(handle) = self.bg_manager.live.get(&instance).cloned() else {
            return;
        };
        let ginfo_snapshot = self
            .bg_manager
            .queue_ro(queue_type_id)
            .infos
            .get(&ginfo_id)
            .cloned()
            .expect("ginfo");
        for member in players {
            if let Some(map) = self.maps.get_mut(&handle.map_key) {
                map.bg_increase_invited(ginfo_snapshot.team);
            }
            let Some(session_id) = self.player_session.get(&member).copied() else {
                continue;
            };
            let _ = session_id;
            self.bg_invite_update_wait_time(queue_type_id, &ginfo_snapshot);
            let slot = {
                let Some(p) = self.bg_player_mut_bg(member) else {
                    continue;
                };
                p.battleground.set_invite(queue_type_id, instance);
                p.battleground.queue_index(queue_type_id)
            };
            self.bg_manager.timers.push(BgTimer {
                when_ms: now.wrapping_add(INVITATION_REMIND_TIME_MS),
                kind: BgTimerKind::InviteRemind,
                guid_low: member,
                bg_instance: instance,
                bg_type_id,
                bg_queue_type_id: queue_type_id,
                remove_time_ms: ginfo_snapshot.remove_invite_time_ms,
            });
            self.bg_manager.timers.push(BgTimer {
                when_ms: now.wrapping_add(INVITE_ACCEPT_WAIT_TIME_MS),
                kind: BgTimerKind::InviteRemove,
                guid_low: member,
                bg_instance: instance,
                bg_type_id,
                bg_queue_type_id: queue_type_id,
                remove_time_ms: ginfo_snapshot.remove_invite_time_ms,
            });
            let packet = build_battleground_status_packet(
                Some(BgStatusInfo {
                    map_id: handle.map_key.map_id,
                    bracket_id,
                    client_instance_id: handle.client_instance_id,
                }),
                slot,
                bg_status::WAIT_JOIN,
                INVITE_ACCEPT_WAIT_TIME_MS,
                0,
            );
            self.bg_send(member, packet);
        }
    }

    /// Mutable player access through the owning map
    /// (queue bookkeeping lives on `Player::battleground`).
    fn bg_player_mut_bg(&mut self, guid_low: u32) -> Option<&mut Player> {
        let key = self.player_map.get(&guid_low).copied()?;
        self.maps.get_mut(&key)?.player_mut(player_guid(guid_low))
    }
}

/// `CHAT_MSG_BG_SYSTEM_NEUTRAL`.
const CHAT_MSG_BG_SYSTEM_NEUTRAL_BYTE: u8 = 0x52;
/// `CHAT_MSG_SYSTEM`.
const CHAT_MSG_SYSTEM_BYTE: u8 = 0x01;

impl World {
    // ------------------------------------------------ text helpers

    /// `GetMangosString` + printf-style %s/%u sequencing
    /// (the BG announce strings mix both; args arrive
    /// pre-stringified in fork call order).
    fn bg_format_lang(&self, entry: i32, args: &[&str]) -> Option<String> {
        let format = self.strings.get(entry)?.text(0).to_string();
        let mut out = String::new();
        let mut rest = format.as_str();
        let mut next = 0usize;
        while let Some(pos) = rest.find('%') {
            out.push_str(&rest[..pos]);
            let spec = rest[pos + 1..].chars().next().unwrap_or(' ');
            match spec {
                's' | 'u' | 'd' | 'i' => {
                    if let Some(arg) = args.get(next) {
                        out.push_str(arg);
                        next += 1;
                    }
                    rest = &rest[pos + 2..];
                }
                '%' => {
                    out.push('%');
                    rest = &rest[pos + 2..];
                }
                _ => {
                    out.push('%');
                    rest = &rest[pos + 1..];
                }
            }
        }
        out.push_str(rest);
        Some(out)
    }

    fn bg_sys_message(&mut self, guid_low: u32, text: &str) {
        let packet =
            mangos_map::battlegrounds::build_bg_chat_packet(CHAT_MSG_SYSTEM_BYTE, text, None);
        self.bg_send(guid_low, packet);
    }

    /// `SendWorldText` (all online players).
    fn bg_world_sys_message(&mut self, text: &str, msg_type: u8) {
        let packet = mangos_map::battlegrounds::build_bg_chat_packet(msg_type, text, None);
        let targets: Vec<u32> = self.player_session.keys().copied().collect();
        for guid_low in targets {
            self.bg_send(guid_low, packet.clone());
        }
    }

    pub(crate) fn bg_send_lang_message(
        &mut self,
        key: MapKey,
        entry: i32,
        msg_type: u32,
        _source: Option<ObjectGuid>,
        arg: Option<u32>,
    ) {
        let arg_str = arg.map(|a| a.to_string());
        let args: Vec<&str> = arg_str.iter().map(String::as_str).collect();
        let Some(text) = self.bg_format_lang(entry, &args) else {
            return;
        };
        let packet = mangos_map::battlegrounds::build_bg_chat_packet(msg_type as u8, &text, None);
        let targets: Vec<u32> = self
            .player_map
            .iter()
            .filter(|(_, k)| **k == key)
            .map(|(g, _)| *g)
            .collect();
        for guid_low in targets {
            self.bg_send(guid_low, packet.clone());
        }
    }

    /// `SendMessage2ToAll`: resolve `mangos_string`
    /// entry with up to two nested `mangos_string`
    /// arg ids substituted into the %s slots
    /// (`BattleGround2ChatBuilder`).
    pub(crate) fn bg_send_lang2_message(
        &mut self,
        key: MapKey,
        entry: i32,
        msg_type: u8,
        source: Option<ObjectGuid>,
        arg_ids: [i32; 2],
    ) {
        let subs: Vec<String> = arg_ids
            .into_iter()
            .filter(|&id| id != 0)
            .filter_map(|id| self.strings.get(id).map(|e| e.text(0).to_string()))
            .collect();
        let args: Vec<&str> = subs.iter().map(String::as_str).collect();
        let Some(text) = self.bg_format_lang(entry, &args) else {
            return;
        };
        let packet = mangos_map::battlegrounds::build_bg_chat_packet(msg_type, &text, source);
        let targets: Vec<u32> = self
            .player_map
            .iter()
            .filter(|(_, k)| **k == key)
            .map(|(g, _)| *g)
            .collect();
        for guid_low in targets {
            self.bg_send(guid_low, packet.clone());
        }
    }

    pub(crate) fn bg_send_world_text(&mut self, text: &str) {
        if let Some(rest) = text.strip_prefix("\u{1}bgstart\u{1}") {
            if !self.config.battleground.queue_announcer_start {
                return;
            }
            let parts: Vec<&str> = rest.split('\u{1}').collect();
            if parts.len() == 3
                && let Some(formatted) = self.bg_format_lang(LANG_BG_STARTED_ANNOUNCE_WORLD, &parts)
            {
                self.bg_world_sys_message(&formatted, CHAT_MSG_BG_SYSTEM_NEUTRAL_BYTE);
            }
            return;
        }
        self.bg_world_sys_message(text, CHAT_MSG_SYSTEM_BYTE);
    }

    // ------------------------------------------------ raids

    fn bg_group_statuses(&self, key: u64, extra: u32) -> HashMap<u32, u8> {
        let mut lows: Vec<u32> = self
            .groups
            .get(&key)
            .map(|g| g.members().iter().map(|m| m.guid_low).collect())
            .unwrap_or_default();
        if extra != 0 && !lows.contains(&extra) {
            lows.push(extra);
        }
        self.group_statuses(&lows)
    }

    /// `AddOrSetPlayerToCorrectBgGroup`.
    pub(crate) fn bg_raid_join(&mut self, key: MapKey, guid_low: u32, team: Team) {
        let team_idx = usize::from(team == Team::Horde);
        let Some(instance_id) = self
            .bg_manager
            .live
            .values()
            .find(|h| h.map_key == key)
            .map(|h| h.map_key.instance_id)
        else {
            return;
        };
        let name = self
            .player_name_race_of(guid_low)
            .map(|(n, _)| n)
            .unwrap_or_default();
        let raid_key = self.bg_manager.live[&instance_id].raid_keys[team_idx];
        match raid_key {
            None => {
                // Fork `GenerateGroupId` consumption for
                // the transient raid id (never persisted).
                self.next_group_db_id = self.next_group_db_id.saturating_add(1);
                let new_key = self.alloc_group_key();
                self.groups
                    .insert(new_key, crate::group::Group::new_bg_raid(guid_low, name));
                self.bg_manager
                    .live
                    .get_mut(&instance_id)
                    .expect("handle")
                    .raid_keys[team_idx] = Some(new_key);
                self.bg_set_battle_ground_raid(guid_low, new_key);
            }
            Some(raid_key) => {
                let is_member = self
                    .groups
                    .get(&raid_key)
                    .is_some_and(|g| g.is_member(guid_low));
                if is_member {
                    let subgroup = self
                        .groups
                        .get(&raid_key)
                        .map_or(0, |g| g.member_subgroup(guid_low));
                    self.bg_set_battle_ground_raid_with_subgroup(guid_low, raid_key, subgroup);
                } else {
                    let statuses = self.bg_group_statuses(raid_key, guid_low);
                    let view = |low: u32| statuses.get(&low).copied();
                    let fx = self
                        .groups
                        .get_mut(&raid_key)
                        .and_then(|g| g.add_member(guid_low, &name, &view))
                        .unwrap_or_default();
                    self.apply_group_fx(fx);
                }
                let _ = name;
            }
        }
        self.sync_entity_group(guid_low);
    }

    /// `Player::SetBattleGroundRaid` (original group is
    /// the player's current group at port-in).
    fn bg_set_battle_ground_raid(&mut self, guid_low: u32, raid_key: u64) {
        let subgroup = self
            .groups
            .get(&raid_key)
            .map_or(0, |g| g.member_subgroup(guid_low));
        self.bg_set_battle_ground_raid_with_subgroup(guid_low, raid_key, subgroup);
    }

    fn bg_set_battle_ground_raid_with_subgroup(
        &mut self,
        guid_low: u32,
        raid_key: u64,
        _subgroup: u8,
    ) {
        if let Some(current) = self.player_group.get(&guid_low).copied()
            && current != raid_key
        {
            let current_subgroup = self
                .groups
                .get(&current)
                .map_or(0, |g| g.member_subgroup(guid_low));
            self.player_original_group
                .insert(guid_low, (current, current_subgroup));
        }
        self.player_group.insert(guid_low, raid_key);
        self.refresh_leader_flag(guid_low);
    }

    /// Raid removal arm of `RemovePlayerAtLeave` +
    /// `RemoveFromBattleGroundRaid`.
    pub(crate) fn bg_raid_leave(&mut self, key: MapKey, guid_low: u32, team: Team) {
        let team_idx = usize::from(team == Team::Horde);
        let instance_id = key.instance_id;
        if let Some(handle) = self.bg_manager.live.get(&instance_id).cloned()
            && let Some(raid_key) = handle.raid_keys[team_idx]
        {
            let statuses = self.bg_group_statuses(raid_key, 0);
            let view = |low: u32| statuses.get(&low).copied();
            let (fx, _count, disbanded) = self
                .groups
                .get_mut(&raid_key)
                .map(|g| g.remove_member(guid_low, 0, &view))
                .unwrap_or_default();
            if disbanded && let Some(h) = self.bg_manager.live.get_mut(&instance_id) {
                h.raid_keys[team_idx] = None;
            }
            if disbanded {
                self.destroy_group(raid_key);
            }
            self.apply_group_fx(fx);
        }
        // Restore the original group if it still exists
        // and still carries the player as a member.
        match self.player_original_group.remove(&guid_low) {
            Some((original_key, _subgroup))
                if self
                    .groups
                    .get(&original_key)
                    .is_some_and(|g| g.is_member(guid_low)) =>
            {
                self.player_group.insert(guid_low, original_key);
            }
            _ => {
                // Only drop the mapping when it pointed
                // at the raid just left.
                if let Some(current) = self.player_group.get(&guid_low).copied()
                    && self
                        .groups
                        .get(&current)
                        .is_none_or(|g| !g.is_member(guid_low))
                {
                    self.player_group.remove(&guid_low);
                }
            }
        }
        self.refresh_leader_flag(guid_low);
        self.sync_entity_group(guid_low);
    }

    // ------------------------------------------------ enter / leave

    /// Ack-side / login-side `BattleGround::AddPlayer`.
    pub(crate) fn bg_maybe_add_player(&mut self, key: MapKey, guid_low: u32) {
        let belongs = self.maps.get(&key).is_some_and(|m| {
            m.bg.is_some()
                && m.player(player_guid(guid_low)).is_some_and(|p| {
                    p.battleground.bg_instance_id == key.instance_id
                        && !m
                            .bg
                            .as_ref()
                            .expect("bg")
                            .is_player_in(player_guid(guid_low))
                })
        });
        if belongs {
            if let Some(map) = self.maps.get_mut(&key) {
                map.bg_add_player(player_guid(guid_low));
            }
            self.drain_map_events();
        }
    }

    pub(crate) fn bg_cast_deserter(&mut self, guid_low: u32) {
        let Some(&key) = self.player_map.get(&guid_low) else {
            return;
        };
        if let Some(map) = self.maps.get_mut(&key) {
            map.bg_reward_spell_cast(player_guid(guid_low), SPELL_DESERTER);
        }
        self.drain_map_events();
    }

    /// Entry-point position (`TeleportToBGEntryPoint`:
    /// homebind when the join pos is all zero).
    fn bg_entry_point(player: &Player) -> (u32, Position) {
        let (map_id, x, y, z, o) = player.battleground.join_pos;
        if x == 0.0 && y == 0.0 && z == 0.0 {
            let hb = crate::character_save::homebind_row(player);
            (
                hb.map,
                Position {
                    x: hb.position_x,
                    y: hb.position_y,
                    z: hb.position_z,
                    o: 0.0,
                },
            )
        } else {
            (map_id, Position { x, y, z, o })
        }
    }

    /// `Player::LeaveBattleground(teleport)`.
    pub(crate) fn leave_battleground(&mut self, guid_low: u32, teleport: bool) {
        let Some(player) = self.bg_player(guid_low) else {
            return;
        };
        if !player.battleground.in_battleground() {
            return;
        }
        let instance_id = player.battleground.bg_instance_id;
        let Some(handle) = self.bg_manager.live.get(&instance_id).cloned() else {
            return;
        };
        let status = self
            .maps
            .get(&handle.map_key)
            .and_then(|m| m.bg.as_ref())
            .map(|bg| bg.status);
        let Some(status) = status else { return };
        let cast_deserter =
            status != bg_status::WAIT_LEAVE && self.config.battleground.cast_deserter;
        // `RemoveAurasDueToSpell(2584)` (Waiting to
        // Resurrect); runs inside `RemovePlayerAtLeave`
        // on the fork too via BG subclass paths — here
        // it rides this wrapper like the fork's.
        if let Some(map) = self.maps.get_mut(&handle.map_key) {
            map.remove_auras_due_to_spell(player_guid(guid_low), 2584);
            map.bg_remove_player_at_leave(player_guid(guid_low), false, true);
        }
        self.drain_map_events();
        if teleport {
            let (dest_map, dest_pos) = self
                .bg_player(guid_low)
                .map_or((0, Position::default()), Self::bg_entry_point);
            if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                let (dest_key, _granted) = self.resolve_dest_key(guid_low, dest_map);
                self.teleport_player_to_key(session_id, dest_key, dest_pos, false, cast_deserter);
            }
        } else if cast_deserter {
            self.bg_cast_deserter(guid_low);
        }
    }

    /// Map-eject transport (`WAIT_LEAVE` timeout / BG
    /// unload): fork `TeleportToBGEntryPoint`.
    pub(crate) fn bg_transport_out(&mut self, guid_low: u32) {
        let Some(player) = self.bg_player(guid_low) else {
            return;
        };
        let (dest_map, dest_pos) = Self::bg_entry_point(player);
        if let Some(session_id) = self.player_session.get(&guid_low).copied() {
            let (dest_key, _granted) = self.resolve_dest_key(guid_low, dest_map);
            self.teleport_player_to_key(session_id, dest_key, dest_pos, false, false);
        }
    }

    /// Login relocation when the saved battleground is
    /// gone (fork `BattleGroundMap` unload semantics +
    /// `TeleportToBGEntryPoint`).
    pub(crate) fn bg_relocate_login(&mut self, player: &mut Player) {
        if bg_type_by_map_id(player.unit.world.map_id) == 0 {
            return;
        }
        let bg_instance = player.battleground.bg_instance_id;
        let live = bg_instance != 0
            && self
                .bg_manager
                .live
                .get(&bg_instance)
                .is_some_and(|h| h.map_key.map_id == player.unit.world.map_id);
        if live {
            return;
        }
        let (map_id, pos) = Self::bg_entry_point(player);
        player.battleground.set_battleground_id(0, 0);
        player.battleground.set_bg_team(Team::None);
        player.unit.world.map_id = map_id;
        player.unit.world.relocate(pos);
    }

    /// `BattleGroundMgr::PlayerLoggedOut` + in-BG
    /// `LeaveBattleground(false)` (logout order).
    pub(crate) fn bg_player_logged_out(&mut self, guid_low: u32) {
        if self
            .bg_player(guid_low)
            .is_some_and(|p| p.battleground.in_battleground())
        {
            self.leave_battleground(guid_low, false);
        }
        let now = self.server_ms();
        for qt in 1..=3u32 {
            let slot_type = self.bg_player(guid_low).and_then(|p| {
                p.battleground
                    .queue_slots
                    .iter()
                    .find(|s| s.queue_type_id == qt)
                    .map(|_| qt)
            });
            let queued = self
                .bg_manager
                .queue_ro(qt)
                .queued_players
                .contains_key(&guid_low);
            if slot_type.is_some() || queued {
                if let Some((_id, info)) =
                    self.bg_manager.queue(qt).queued_players.get_mut(&guid_low)
                {
                    info.online = false;
                    info.last_online_ms = now;
                }
                let ginfo = self.bg_group_info(qt, guid_low);
                if let Some(ginfo) = ginfo {
                    self.bg_schedule_queue_update(qt, ginfo.bg_type_id, ginfo.bracket_id);
                }
            }
        }
    }

    /// `BattleGroundMgr::PlayerLoggedIn`.
    pub(crate) fn bg_player_logged_in(&mut self, _session: SessionId, guid_low: u32) {
        let now = self.server_ms();
        for qt in 1..=3u32 {
            let queued = {
                let q = self.bg_manager.queue(qt);
                if let Some((_id, info)) = q.queued_players.get_mut(&guid_low) {
                    info.online = true;
                    info.last_online_ms = 0;
                    true
                } else {
                    false
                }
            };
            if !queued {
                continue;
            }
            let Some(ginfo) = self.bg_group_info(qt, guid_low) else {
                continue;
            };
            let bracket_now = self
                .bg_bracket_of(guid_low, ginfo.bg_type_id)
                .unwrap_or(ginfo.bracket_id);
            let avg = self.bg_average_wait_time_for(qt, ginfo.team, bracket_now);
            let slot = {
                let Some(p) = self.bg_player_mut_bg(guid_low) else {
                    continue;
                };
                p.battleground.add_queue_id(qt)
            };
            let info_pkt = Some(Self::bg_template_status_info(ginfo.bg_type_id));
            let packet = build_battleground_status_packet(
                info_pkt,
                slot,
                bg_status::WAIT_QUEUE,
                avg,
                now.wrapping_sub(ginfo.join_time_ms),
            );
            self.bg_send(guid_low, packet);
            if ginfo.invited_to_instance != 0 {
                if let Some(p) = self.bg_player_mut_bg(guid_low) {
                    p.battleground.set_invite(qt, ginfo.invited_to_instance);
                }
                let offset = if now > ginfo.remove_invite_time_ms {
                    1
                } else {
                    ginfo.remove_invite_time_ms - now
                };
                self.bg_manager.timers.push(BgTimer {
                    when_ms: now.wrapping_add(offset),
                    kind: BgTimerKind::InviteRemove,
                    guid_low,
                    bg_instance: ginfo.invited_to_instance,
                    bg_type_id: ginfo.bg_type_id,
                    bg_queue_type_id: qt,
                    remove_time_ms: ginfo.remove_invite_time_ms,
                });
            }
        }
    }

    fn bg_average_wait_time_for(&self, queue_type_id: u32, team: Team, bracket: u32) -> u32 {
        let team_index = usize::from(team == Team::Horde);
        let q = self.bg_manager.queue_ro(queue_type_id);
        if q.wait_times[team_index][bracket as usize][COUNT_AVERAGE - 1] != 0 {
            q.wait_sums[team_index][bracket as usize] / COUNT_AVERAGE as u32
        } else {
            0
        }
    }

    // ------------------------------------------------ join flow

    /// `Group::CanJoinBattleGroundQueue` + the solo
    /// handler gates, returning (error, excluded).
    fn bg_can_join_queue(
        &self,
        leader_low: u32,
        group_key: Option<u64>,
        bg_type_id: u32,
        queue_type_id: u32,
        min_player_count: u32,
        max_player_count: u32,
    ) -> (u8, Vec<u32>) {
        let mut excluded: Vec<u32> = Vec::new();
        let queues_count = self.config.battleground.queues_count;
        let Some(leader) = self.bg_player(leader_low) else {
            return (BG_JOIN_ERR_OFFLINE_MEMBER, excluded);
        };
        let leader_team = leader.team;
        let leader_bracket = self.bg_bracket_of(leader_low, bg_type_id);
        // Fork solo path (no group): the member loop
        // runs over the (empty) group and returns OK —
        // solo gates live in the handler.
        let Some(group_key) = group_key else {
            return (BG_JOIN_ERR_OK, excluded);
        };
        let Some(group) = self.groups.get(&group_key) else {
            return (BG_JOIN_ERR_OK, excluded);
        };
        let member_count = group.members_count();
        let mut members_to_queue_count = 0u32;
        let members: Vec<u32> = group.members().iter().map(|m| m.guid_low).collect();
        for member in &members {
            let Some(member_player) = self.bg_player(*member) else {
                return (BG_JOIN_ERR_OFFLINE_MEMBER, excluded);
            };
            if member_player.team != leader_team {
                return (BG_JOIN_ERR_MIXED_FACTION, excluded);
            }
            let member_bracket = self.bg_bracket_of(*member, bg_type_id);
            let skip = member_bracket.is_none()
                || leader_bracket.is_none()
                || member_bracket != leader_bracket
                || member_player.battleground.queue_index(queue_type_id) < MAX_BATTLEGROUND_QUEUES
                || member_player.battleground.in_battleground()
                || member_player
                    .battleground
                    .is_invited_for_queue_type(queue_type_id)
                || !member_player.battleground.has_free_queue_id(queues_count);
            if skip {
                excluded.push(*member);
            } else {
                members_to_queue_count += 1;
            }
        }
        if members_to_queue_count == 0 && member_count != 0 {
            return (BG_JOIN_ERR_GROUP_NOT_ENOUGH, excluded);
        }
        let mut i = 0usize;
        while i < members.len() {
            let member = members[i];
            if excluded.contains(&member) {
                i += 1;
                continue;
            }
            if !self.bg_can_join(member) {
                return (BG_JOIN_ERR_GROUP_DESERTER, excluded);
            }
            if members_to_queue_count > max_player_count {
                excluded.push(member);
                members_to_queue_count -= 1;
            }
            i += 1;
        }
        if members_to_queue_count < min_player_count && member_count != 0 {
            return (BG_JOIN_ERR_GROUP_TOO_MANY, excluded);
        }
        (BG_JOIN_ERR_OK, excluded)
    }

    fn bg_send_group_join_error(&mut self, leader_low: u32, error: u8) {
        let msg = match error {
            BG_JOIN_ERR_OFFLINE_MEMBER => LANG_BG_GROUP_OFFLINE_MEMBER,
            BG_JOIN_ERR_GROUP_TOO_MANY | BG_JOIN_ERR_GROUP_NOT_ENOUGH => LANG_BG_GROUP_TOO_LARGE,
            BG_JOIN_ERR_MIXED_FACTION => LANG_BG_GROUP_MIXED_FACTION,
            BG_JOIN_ERR_MEMBER_IN_QUEUE => LANG_BG_GROUP_MEMBER_ALREADY_IN_QUEUE,
            4 => LANG_BG_GROUP_MIXED_LEVELS,
            BG_JOIN_ERR_GROUP_DESERTER => {
                let packet = build_group_joined_packet(BG_GROUPJOIN_DESERTERS);
                self.bg_send(leader_low, packet);
                let packet = build_group_joined_packet(BG_GROUPJOIN_FAILED);
                self.bg_send(leader_low, packet);
                LANG_BG_GROUP_MEMBER_DESERTER
            }
            BG_JOIN_ERR_ALL_QUEUES_USED => LANG_BG_GROUP_MEMBER_NO_FREE_QUEUE_SLOTS,
            _ => return,
        };
        if error != BG_JOIN_ERR_GROUP_DESERTER
            && error != BG_JOIN_ERR_GROUP_TOO_MANY
            && error != BG_JOIN_ERR_GROUP_NOT_ENOUGH
        {
            let packet = build_group_joined_packet(BG_GROUPJOIN_FAILED);
            self.bg_send(leader_low, packet);
        }
        if let Some(text) = self.bg_format_lang(msg, &[]) {
            self.bg_sys_message(leader_low, &text);
        }
    }
}

impl World {
    // ------------------------------------------------ selection pools

    /// `SelectionPool::AddGroup`.
    fn pool_add_group(
        &mut self,
        queue_type_id: u32,
        team_index: usize,
        ginfo_id: u64,
        desired_count: u32,
        bg_client_instance_id: u32,
    ) -> bool {
        let (players, invited, desired) = {
            let Some(g) = self.bg_manager.queue_ro(queue_type_id).infos.get(&ginfo_id) else {
                return false;
            };
            (
                g.players.len() as u32,
                g.invited_to_instance != 0,
                g.desired_client_instance_id,
            )
        };
        let pool = &mut self.bg_manager.pools[team_index];
        if !invited
            && (desired == 0 || desired == bg_client_instance_id)
            && desired_count >= pool.player_count + players
        {
            pool.selected.push(ginfo_id);
            pool.player_count += players;
            return true;
        }
        pool.player_count < desired_count
    }

    /// `SelectionPool::KickGroup` (returns fork bool:
    /// false when the kicked group was <= size+1).
    #[allow(clippy::cast_possible_wrap)]
    fn pool_kick_group(&mut self, queue_type_id: u32, team_index: usize, size: u32) -> bool {
        let sizes: Vec<u32> = self.bg_manager.pools[team_index]
            .selected
            .iter()
            .map(|id| {
                self.bg_manager
                    .queue_ro(queue_type_id)
                    .infos
                    .get(id)
                    .map_or(0, |g| g.players.len() as u32)
            })
            .collect();
        let pool = &mut self.bg_manager.pools[team_index];
        if pool.player_count == 0 || pool.selected.is_empty() {
            return true;
        }
        let mut kick = 0usize;
        let mut found = false;
        for (i, sz) in sizes.iter().enumerate() {
            if i32::abs(*sz as i32 - size as i32) <= 1 {
                kick = i;
                found = true;
            } else if !found && *sz >= sizes[kick] {
                kick = i;
            }
        }
        let kicked_size = sizes[kick];
        pool.selected.remove(kick);
        pool.player_count -= kicked_size;
        kicked_size > size + 1
    }

    /// `FillPlayersToBg`.
    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    fn bg_fill_players(&mut self, queue_type_id: u32, instance: u32, bracket_id: u32) {
        let (ally_free, horde_free, client_id) = {
            let Some(handle) = self.bg_manager.live.get(&instance).cloned() else {
                return;
            };
            let Some(map) = self.maps.get(&handle.map_key) else {
                return;
            };
            let Some(bg) = map.bg.as_ref() else { return };
            (
                bg.free_slots_for_team(Team::Alliance) as i32,
                bg.free_slots_for_team(Team::Horde) as i32,
                handle.client_instance_id,
            )
        };
        let ally_groups: Vec<u64> = self.bg_manager.queue_ro(queue_type_id).queued_groups
            [bracket_id as usize][Q_NORMAL_ALLIANCE]
            .iter()
            .copied()
            .collect();
        let horde_groups: Vec<u64> = self.bg_manager.queue_ro(queue_type_id).queued_groups
            [bracket_id as usize][Q_NORMAL_HORDE]
            .iter()
            .copied()
            .collect();
        let mut ali_index = 0usize;
        while ali_index < ally_groups.len()
            && self.pool_add_group(
                queue_type_id,
                0,
                ally_groups[ali_index],
                ally_free.max(0) as u32,
                client_id,
            )
        {
            ali_index += 1;
        }
        let mut horde_index = 0usize;
        while horde_index < horde_groups.len()
            && self.pool_add_group(
                queue_type_id,
                1,
                horde_groups[horde_index],
                horde_free.max(0) as u32,
                client_id,
            )
        {
            horde_index += 1;
        }
        if self.config.battleground.invitation_type == 0 {
            return;
        }
        // Balancing loop (invitation type 1).
        let mut diff_ali = ally_free - self.bg_manager.pools[0].player_count as i32;
        let mut diff_horde = horde_free - self.bg_manager.pools[1].player_count as i32;
        while i32::abs(diff_ali - diff_horde) > 1
            && (self.bg_manager.pools[0].player_count > 0
                || self.bg_manager.pools[1].player_count > 0)
        {
            if diff_ali < diff_horde {
                if self.pool_kick_group(queue_type_id, 0, (diff_horde - diff_ali) as u32) {
                    while ali_index < ally_groups.len()
                        && self.pool_add_group(
                            queue_type_id,
                            0,
                            ally_groups[ali_index],
                            if ally_free >= diff_horde {
                                (ally_free - diff_horde) as u32
                            } else {
                                0
                            },
                            client_id,
                        )
                    {
                        ali_index += 1;
                    }
                }
                if self.bg_manager.pools[0].player_count == 0 {
                    if ally_free <= diff_horde + 1 {
                        break;
                    }
                    self.pool_kick_group(queue_type_id, 1, (diff_horde - diff_ali) as u32);
                }
            } else {
                if self.pool_kick_group(queue_type_id, 1, (diff_ali - diff_horde) as u32) {
                    while horde_index < horde_groups.len()
                        && self.pool_add_group(
                            queue_type_id,
                            1,
                            horde_groups[horde_index],
                            if horde_free >= diff_ali {
                                (horde_free - diff_ali) as u32
                            } else {
                                0
                            },
                            client_id,
                        )
                    {
                        horde_index += 1;
                    }
                }
                if self.bg_manager.pools[1].player_count == 0 {
                    if horde_free <= diff_ali + 1 {
                        break;
                    }
                    self.pool_kick_group(queue_type_id, 0, (diff_ali - diff_horde) as u32);
                }
            }
            diff_ali = ally_free - self.bg_manager.pools[0].player_count as i32;
            diff_horde = horde_free - self.bg_manager.pools[1].player_count as i32;
        }
    }

    /// `CheckPremadeMatch`.
    fn bg_check_premade_match(
        &mut self,
        queue_type_id: u32,
        bracket_id: u32,
        min_per_team: u32,
        max_per_team: u32,
    ) -> bool {
        let min_group_size = self.config.battleground.premade_min_group_size;
        for queue_kind in 0..2usize {
            for team in 0..2usize {
                let index = 2 * queue_kind + team;
                let groups: Vec<u64> = self.bg_manager.queue_ro(queue_type_id).queued_groups
                    [bracket_id as usize][index]
                    .iter()
                    .copied()
                    .collect();
                for gid in groups {
                    let eligible = self
                        .bg_manager
                        .queue_ro(queue_type_id)
                        .infos
                        .get(&gid)
                        .is_some_and(|g| {
                            g.invited_to_instance == 0 && g.players.len() as u32 >= min_group_size
                        });
                    if eligible {
                        self.pool_add_group(queue_type_id, team, gid, max_per_team, 0);
                        if self.bg_manager.pools[team].player_count >= min_per_team {
                            break;
                        }
                    }
                }
            }
        }
        if self.bg_manager.testing
            && (self.bg_manager.pools[0].player_count > 0
                || self.bg_manager.pools[1].player_count > 0)
        {
            return true;
        }
        if self.bg_manager.pools[0].player_count >= min_per_team
            && self.bg_manager.pools[1].player_count >= min_per_team
        {
            return true;
        }
        // Move stale/undersized premades to the normal
        // queue (front entry only, per team).
        let now = self.server_ms();
        let time_before = now.wrapping_sub(self.config.battleground.premade_wait_for_match_ms);
        for team in 0..2usize {
            let move_id = {
                let q = self.bg_manager.queue_ro(queue_type_id);
                q.queued_groups[bracket_id as usize][team]
                    .front()
                    .copied()
                    .filter(|id| {
                        q.infos.get(id).is_some_and(|g| {
                            g.invited_to_instance == 0
                                && (g.join_time_ms < time_before
                                    || (g.players.len() as u32) < min_per_team)
                        })
                    })
            };
            if let Some(gid) = move_id {
                let q = self.bg_manager.queue(queue_type_id);
                q.queued_groups[bracket_id as usize][team].pop_front();
                q.queued_groups[bracket_id as usize][team + 2].push_back(gid);
            }
        }
        false
    }

    /// `CheckNormalMatch`.
    #[allow(clippy::cast_possible_wrap)]
    fn bg_check_normal_match(
        &mut self,
        queue_type_id: u32,
        bracket_id: u32,
        min_players: u32,
        max_players: u32,
    ) -> bool {
        let mut next_index = [0usize; 2];
        for (team, next_index_slot) in next_index.iter_mut().enumerate() {
            let groups: Vec<u64> = self.bg_manager.queue_ro(queue_type_id).queued_groups
                [bracket_id as usize][Q_NORMAL_ALLIANCE + team]
                .iter()
                .copied()
                .collect();
            let mut i = 0usize;
            while i < groups.len() {
                let invited = self
                    .bg_manager
                    .queue_ro(queue_type_id)
                    .infos
                    .get(&groups[i])
                    .is_some_and(|g| g.invited_to_instance != 0);
                if !invited {
                    self.pool_add_group(queue_type_id, team, groups[i], max_players, 0);
                    if self.bg_manager.pools[team].player_count >= min_players {
                        break;
                    }
                }
                i += 1;
            }
            *next_index_slot = i + usize::from(i < groups.len());
            // store where the fork iterator stopped (+1
            // applied at balancing start).
            if i >= groups.len() {
                *next_index_slot = i;
            }
        }
        let mut j = 0usize;
        if self.bg_manager.pools[1].player_count < self.bg_manager.pools[0].player_count {
            j = 1;
        }
        if self.config.battleground.invitation_type != 0
            && self.bg_manager.pools[1].player_count >= min_players
            && self.bg_manager.pools[0].player_count >= min_players
        {
            let groups: Vec<u64> = self.bg_manager.queue_ro(queue_type_id).queued_groups
                [bracket_id as usize][Q_NORMAL_ALLIANCE + j]
                .iter()
                .copied()
                .collect();
            let other_count = self.bg_manager.pools[(j + 1) % 2].player_count;
            let mut i = next_index[j];
            while i < groups.len() {
                let invited = self
                    .bg_manager
                    .queue_ro(queue_type_id)
                    .infos
                    .get(&groups[i])
                    .is_some_and(|g| g.invited_to_instance != 0);
                if !invited && !self.pool_add_group(queue_type_id, j, groups[i], other_count, 0) {
                    break;
                }
                i += 1;
            }
            let diff = (self.bg_manager.pools[1].player_count as i32
                - self.bg_manager.pools[0].player_count as i32)
                .abs();
            if diff > 2 {
                return false;
            }
        }
        if self.bg_manager.testing
            && (self.bg_manager.pools[0].player_count > 0
                || self.bg_manager.pools[1].player_count > 0)
        {
            return true;
        }
        self.bg_manager.pools[0].player_count >= min_players
            && self.bg_manager.pools[1].player_count >= min_players
    }

    /// `RemoveOfflinePlayer` (runs on every queue of
    /// this type inside `Update` in the fork — one
    /// `m_queuedPlayers` per queue type).
    fn bg_remove_offline_players(&mut self, queue_type_id: u32) {
        let now = self.server_ms();
        let candidates: Vec<(u32, u64, PlayerQueueInfo)> = self
            .bg_manager
            .queue_ro(queue_type_id)
            .queued_players
            .iter()
            .map(|(g, (id, info))| (*g, *id, *info))
            .collect();
        for (guid_low, ginfo_id, info) in candidates {
            let mut remove = false;
            if !info.online && now.wrapping_sub(info.last_online_ms) > OFFLINE_BG_QUEUE_TIME_MS {
                remove = true;
            } else if let Some(ginfo) = self
                .bg_manager
                .queue_ro(queue_type_id)
                .infos
                .get(&ginfo_id)
                .cloned()
                && ginfo.invited_to_instance != 0
            {
                let ended = self
                    .bg_live(ginfo.invited_to_instance, ginfo.bg_type_id)
                    .is_some_and(|h| {
                        self.maps
                            .get(&h.map_key)
                            .and_then(|m| m.bg.as_ref())
                            .is_some_and(|bg| bg.status == bg_status::WAIT_LEAVE)
                    });
                if ended {
                    if info.online {
                        let slot = self
                            .bg_player(guid_low)
                            .map(|p| p.battleground.queue_index(queue_type_id));
                        if let Some(slot) = slot.filter(|s| *s < MAX_BATTLEGROUND_QUEUES) {
                            if let Some(p) = self.bg_player_mut_bg(guid_low) {
                                p.battleground.remove_queue_id(queue_type_id);
                            }
                            let info_pkt = self.bg_status_info(ginfo.invited_to_instance);
                            let packet = build_battleground_status_packet(
                                info_pkt,
                                slot,
                                bg_status::NONE,
                                0,
                                0,
                            );
                            self.bg_send(guid_low, packet);
                        }
                    }
                    remove = true;
                }
            }
            if remove {
                self.bg_manager
                    .queue(queue_type_id)
                    .queued_players
                    .remove(&guid_low);
            }
        }
    }

    fn bg_has_players_in_queue(&self, queue_type_id: u32, bracket_id: u32) -> bool {
        self.bg_manager.queue_ro(queue_type_id).queued_groups[bracket_id as usize]
            .iter()
            .any(|q| !q.is_empty())
    }

    /// `CheckFreeSlots`.
    fn bg_check_free_slots(&mut self, queue_type_id: u32, bg_type_id: u32, bracket_id: u32) {
        if self.config.battleground.randomize_queues {
            // UNMODELED: chrono-seeded std::shuffle of
            // the normal queues (nondeterministic in the
            // fork; order-only effect).
        }
        let mut full: Vec<u32> = Vec::new();
        let instances: Vec<u32> = self.bg_manager.free_slots[bg_type_id as usize]
            .iter()
            .copied()
            .collect();
        for instance in instances {
            let Some(handle) = self.bg_manager.live.get(&instance).cloned() else {
                continue;
            };
            if handle.bg_type_id != bg_type_id || handle.bracket_id != bracket_id {
                continue;
            }
            let (status, has_free) = {
                let Some(map) = self.maps.get(&handle.map_key) else {
                    continue;
                };
                let Some(bg) = map.bg.as_ref() else { continue };
                (bg.status, bg.has_free_slots())
            };
            if status <= bg_status::WAIT_QUEUE || status >= bg_status::WAIT_LEAVE {
                continue;
            }
            if !has_free {
                full.push(instance);
                continue;
            }
            self.bg_manager.pools[0].init();
            self.bg_manager.pools[1].init();
            self.bg_fill_players(queue_type_id, instance, bracket_id);
            for team in 0..2usize {
                let selected: Vec<u64> = self.bg_manager.pools[team].selected.clone();
                for gid in selected {
                    let team_val = if team == 0 {
                        Team::Alliance
                    } else {
                        Team::Horde
                    };
                    self.bg_invite_group(queue_type_id, gid, instance, team_val);
                }
            }
            let has_free_after = self
                .maps
                .get(&handle.map_key)
                .and_then(|m| m.bg.as_ref())
                .is_some_and(|bg| bg.has_free_slots());
            if !has_free_after {
                full.push(instance);
            }
        }
        for instance in full {
            self.bg_manager.free_slots[bg_type_id as usize].retain(|id| *id != instance);
        }
    }

    /// `CheckCreateNewBg`.
    fn bg_check_create_new(
        &mut self,
        queue_type_id: u32,
        bg_type_id: u32,
        bracket_id: u32,
    ) -> bool {
        let Some((template_min, template_max, template_min_level, _)) =
            self.bg_template_min_max(bg_type_id)
        else {
            return false;
        };
        let mut min_per_team = template_min;
        let mut max_per_team = template_max;
        if self.bg_manager.testing {
            min_per_team = 1;
        }
        if bg_type_id == 1
            && self.config.battleground.av_min_players_in_queue != 0
            && !self.bg_manager.testing
        {
            let need = self.config.battleground.av_min_players_in_queue;
            for team in 0..2usize {
                let count: u32 = self.bg_manager.queue_ro(queue_type_id).queued_groups
                    [bracket_id as usize][Q_NORMAL_ALLIANCE + team]
                    .iter()
                    .filter(|id| {
                        self.bg_manager
                            .queue_ro(queue_type_id)
                            .infos
                            .get(id)
                            .is_some_and(|g| g.invited_to_instance == 0)
                    })
                    .count() as u32;
                if count < need {
                    return false;
                }
            }
            // UNMODELED: AV queue shuffle (order-only).
        }
        if bg_type_id == 1
            && self.config.battleground.av_init_max_players != 0
            && !self.bg_manager.testing
        {
            let configured = self.config.battleground.av_init_max_players;
            if configured > min_per_team && configured < max_per_team {
                max_per_team = configured;
            }
        }
        let q_min = min_level_for_bracket(bg_type_id, template_min_level, bracket_id);
        let q_max = max_level_for_bracket(bg_type_id, template_min_level, bracket_id) - 1;

        self.bg_manager.pools[0].init();
        self.bg_manager.pools[1].init();
        if self.bg_check_premade_match(queue_type_id, bracket_id, min_per_team, max_per_team) {
            if let Some(instance) = self.bg_create_new(bg_type_id, bracket_id, q_min, q_max) {
                self.bg_invite_selected(queue_type_id, instance);
                return true;
            }
            return false;
        }
        self.bg_manager.pools[0].init();
        self.bg_manager.pools[1].init();
        if self.bg_check_normal_match(queue_type_id, bracket_id, min_per_team, max_per_team) {
            if let Some(instance) = self.bg_create_new(bg_type_id, bracket_id, q_min, q_max) {
                self.bg_invite_selected(queue_type_id, instance);
                return true;
            }
            return false;
        }
        false
    }

    fn bg_invite_selected(&mut self, queue_type_id: u32, instance: u32) {
        for team in 0..2usize {
            let selected: Vec<u64> = self.bg_manager.pools[team].selected.clone();
            for gid in selected {
                let team_val = self
                    .bg_manager
                    .queue_ro(queue_type_id)
                    .infos
                    .get(&gid)
                    .map_or(Team::None, |g| g.team);
                self.bg_invite_group(queue_type_id, gid, instance, team_val);
            }
        }
    }

    /// `BattleGroundQueue::Update`.
    fn bg_queue_update(&mut self, queue_type_id: u32, bg_type_id: u32, bracket_id: u32) {
        self.bg_remove_offline_players(queue_type_id);
        if !self.bg_has_players_in_queue(queue_type_id, bracket_id) {
            return;
        }
        self.bg_check_free_slots(queue_type_id, bg_type_id, bracket_id);
        if self.bg_check_create_new(queue_type_id, bg_type_id, bracket_id) {
            self.bg_check_free_slots(queue_type_id, bg_type_id, bracket_id);
        }
    }

    /// `BattleGroundMgr::Update` (scheduler drain +
    /// timer events; BG object updates ride map ticks).
    pub(crate) fn bg_update_manager(&mut self) {
        let scheduled: Vec<(u32, u32, u32)> = std::mem::take(&mut self.bg_manager.scheduled);
        let mut seen: HashSet<(u32, u32, u32)> = HashSet::new();
        for (qt, ty, br) in scheduled {
            if seen.insert((qt, ty, br)) {
                self.bg_queue_update(qt, ty, br);
            }
        }
        let now = self.server_ms();
        let due: Vec<BgTimer> = self
            .bg_manager
            .timers
            .iter()
            .filter(|t| {
                // mstime diff >= 0 (wrap-safe, fork
                // `getMSTimeDiff(when, now)` semantics).
                now.wrapping_sub(t.when_ms) < 0x8000_0000
            })
            .copied()
            .collect();
        self.bg_manager
            .timers
            .retain(|t| now.wrapping_sub(t.when_ms) >= 0x8000_0000);
        for timer in due {
            match timer.kind {
                BgTimerKind::InviteRemind => self.bg_timer_remind(timer),
                BgTimerKind::InviteRemove => self.bg_timer_remove(timer),
            }
        }
    }

    /// `BgQueueInviteEvent::Execute`.
    fn bg_timer_remind(&mut self, timer: BgTimer) {
        if !self.player_session.contains_key(&timer.guid_low) {
            return;
        }
        let Some(handle) = self.bg_live(timer.bg_instance, timer.bg_type_id).cloned() else {
            return;
        };
        let Some(slot) = self
            .bg_player(timer.guid_low)
            .map(|p| p.battleground.queue_index(timer.bg_queue_type_id))
        else {
            return;
        };
        if slot >= MAX_BATTLEGROUND_QUEUES {
            return;
        }
        if self.bg_is_player_invited(
            timer.bg_queue_type_id,
            timer.guid_low,
            timer.bg_instance,
            timer.remove_time_ms,
        ) {
            let packet = build_battleground_status_packet(
                Some(BgStatusInfo {
                    map_id: handle.map_key.map_id,
                    bracket_id: handle.bracket_id,
                    client_instance_id: handle.client_instance_id,
                }),
                slot,
                bg_status::WAIT_JOIN,
                INVITE_ACCEPT_WAIT_TIME_MS - INVITATION_REMIND_TIME_MS,
                0,
            );
            self.bg_send(timer.guid_low, packet);
        }
    }

    /// `BGQueueRemoveEvent::Execute`.
    fn bg_timer_remove(&mut self, timer: BgTimer) {
        if !self.player_session.contains_key(&timer.guid_low) {
            return;
        }
        let handle = self.bg_live(timer.bg_instance, timer.bg_type_id).cloned();
        let Some(slot) = self
            .bg_player(timer.guid_low)
            .map(|p| p.battleground.queue_index(timer.bg_queue_type_id))
        else {
            return;
        };
        if slot >= MAX_BATTLEGROUND_QUEUES {
            return;
        }
        if !self.bg_is_player_invited(
            timer.bg_queue_type_id,
            timer.guid_low,
            timer.bg_instance,
            timer.remove_time_ms,
        ) {
            return;
        }
        if let Some(p) = self.bg_player_mut_bg(timer.guid_low) {
            p.battleground.remove_queue_id(timer.bg_queue_type_id);
        }
        self.bg_queue_remove_player(timer.bg_queue_type_id, timer.guid_low, true);
        let status = handle.as_ref().and_then(|h| {
            self.maps
                .get(&h.map_key)
                .and_then(|m| m.bg.as_ref())
                .map(|bg| bg.status)
        });
        if let Some(h) = &handle
            && status != Some(bg_status::WAIT_LEAVE)
        {
            self.bg_schedule_queue_update(timer.bg_queue_type_id, timer.bg_type_id, h.bracket_id);
        }
        let info = handle.as_ref().map(|h| BgStatusInfo {
            map_id: h.map_key.map_id,
            bracket_id: h.bracket_id,
            client_instance_id: h.client_instance_id,
        });
        let packet = build_battleground_status_packet(info, slot, bg_status::NONE, 0, 0);
        self.bg_send(timer.guid_low, packet);
    }
}

impl World {
    // ------------------------------------------------ handlers

    /// `SendBattlegGroundList`.
    pub(crate) fn bg_send_list(
        &mut self,
        guid_low: u32,
        battlemaster: ObjectGuid,
        bg_type_id: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(battlemaster.raw());
        writer.write_u32_le(bg_type_id);
        let mut instances: Vec<u32> = self
            .bg_manager
            .live
            .values()
            .filter(|h| h.bg_type_id == bg_type_id)
            .filter(|h| {
                self.maps
                    .get(&h.map_key)
                    .and_then(|m| m.bg.as_ref())
                    .is_some_and(|bg| bg.status != bg_status::WAIT_LEAVE)
            })
            .map(|h| h.client_instance_id)
            .collect();
        instances.sort_unstable();
        writer.write_u8(0);
        writer.write_u8(instances.len() as u8);
        for id in instances {
            writer.write_u32_le(id);
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_BATTLEFIELD_LIST);
        packet.take_writer(writer);
        self.bg_send(guid_low, packet);
    }

    /// `HandleBattlemasterHelloOpcode`.
    pub(crate) fn on_bg_battlemaster_hello(
        &mut self,
        _id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
    ) {
        let Ok(npc_guid) = packet.reader().read_u64_le().map(ObjectGuid::from_wire_raw) else {
            return;
        };
        let Some(&key) = self.player_map.get(&actor_low) else {
            return;
        };
        // `GetNPCIfCanInteractWith(guid, UNIT_NPC_FLAG_BATTLEMASTER)`
        // subset: creature present, battlemaster flag,
        // within interact distance.
        // UNMODELED: alive/faction react gates of the
        // generic can-interact helper.
        let bg_type_id = {
            let Some(map) = self.maps.get(&key) else {
                return;
            };
            let Some(creature) = map.creature(npc_guid) else {
                self.send_gossip_close_for(actor_low);
                return;
            };
            if creature.info.npc_flags & NPC_FLAG_BATTLEMASTER == 0 {
                self.send_gossip_close_for(actor_low);
                return;
            }
            let in_range = match (
                map.object_position(player_guid(actor_low)),
                map.object_position(npc_guid),
            ) {
                (Some(a), Some(b)) => {
                    let dx = a.x - b.x;
                    let dy = a.y - b.y;
                    let dz = a.z - b.z;
                    dx * dx + dy * dy + dz * dz <= 100.0
                }
                _ => false,
            };
            if !in_range {
                self.send_gossip_close_for(actor_low);
                return;
            }
            self.deps
                .world_stores
                .battlemasters
                .bg_type(creature.info.entry)
        };
        if bg_type_id == 0 {
            self.send_gossip_close_for(actor_low);
            return;
        }
        self.bg_send_list(actor_low, npc_guid, bg_type_id);
    }

    fn send_gossip_close_for(&mut self, guid_low: u32) {
        let packet = WorldPacket::new(opcode_id::SMSG_GOSSIP_COMPLETE);
        self.bg_send(guid_low, packet);
    }

    /// Shared body of `HandleBattlemasterJoinOpcode` /
    /// `HandleBattlefieldJoinOpcode`
    /// (`AddPlayerToBgQueue` + group path).
    #[allow(clippy::too_many_lines)]
    fn bg_join_queue(
        &mut self,
        actor_low: u32,
        bg_type_id: u32,
        desired_client_instance: u32,
        as_group: bool,
    ) {
        // UNMODELED gates with no Rust systems yet (same
        // recorded gaps as elsewhere in the actor): taxi
        // flight, duel, trial restrictions.
        let (in_bg, alive, has_free_slot, team, level) = {
            let Some(p) = self.bg_player(actor_low) else {
                return;
            };
            (
                p.battleground.in_battleground(),
                p.unit.is_alive(),
                p.battleground
                    .has_free_queue_id(self.config.battleground.queues_count),
                p.team,
                p.unit.level(),
            )
        };
        if in_bg || !alive {
            return;
        }
        if !has_free_slot {
            return;
        }
        let queue_type_id = queue_type_of(bg_type_id);
        if queue_type_id == 0 {
            return;
        }
        let Some(template) = self
            .deps
            .world_stores
            .battlegrounds
            .get(bg_type_id)
            .cloned()
        else {
            return;
        };
        let Some(bracket_id) = bracket_id_from_level(template.min_level, level) else {
            // `LANG_YOUR_BG_LEVEL_REQ_ERROR` system message.
            if let Some(text) = self.bg_format_lang(
                LANG_YOUR_BG_LEVEL_REQ_ERROR,
                &[
                    &template.min_level.to_string(),
                    &template.max_level.to_string(),
                ],
            ) {
                self.bg_sys_message(actor_low, &text);
            }
            return;
        };
        let invited = self
            .bg_player(actor_low)
            .is_some_and(|p| p.battleground.is_invited_for_queue_type(queue_type_id));
        if invited {
            return;
        }
        // Group determination (fork: asGroup only counts
        // with a real group led by the player).
        let group_key = if as_group {
            self.player_group.get(&actor_low).copied().filter(|key| {
                self.groups
                    .get(key)
                    .is_some_and(|g| g.leader_low() == actor_low)
            })
        } else {
            None
        };
        let (error, mut excluded) = self.bg_can_join_queue(
            actor_low,
            group_key,
            bg_type_id,
            queue_type_id,
            template.min_players_per_team,
            template.max_players_per_team,
        );
        if error != BG_JOIN_ERR_OK {
            self.bg_send_group_join_error(actor_low, error);
            return;
        }
        // Entry point snapshot (only outside BGs — in_bg
        // already returned).
        {
            let pos = self
                .bg_player(actor_low)
                .map(|p| (p.unit.world.map_id, p.unit.world.position()));
            if let Some((map_id, pos)) = pos
                && let Some(p) = self.bg_player_mut_bg(actor_low)
            {
                p.battleground
                    .set_entry_point(map_id, pos.x, pos.y, pos.z, pos.o);
            }
        }
        let leader_name = self
            .player_name_race_of(actor_low)
            .map(|(n, _)| n)
            .unwrap_or_default();
        // Group size (queueable) for the premade check.
        let group_size = match group_key {
            Some(key) => self
                .groups
                .get(&key)
                .map_or(1, |g| g.members_count() as u32 - excluded.len() as u32),
            None => 1,
        };
        let is_premade = group_size >= self.config.battleground.premade_min_group_size;
        let ginfo_id = self.bg_queue_add_group(
            actor_low,
            group_key,
            bg_type_id,
            bracket_id,
            is_premade,
            desired_client_instance,
            &excluded,
            &leader_name,
        );
        // Per-player queue packets (AddPlayerToBgQueue:
        // status send happens for members; excluded get
        // the "not queued" system text).
        let members: Vec<u32> = match group_key {
            Some(key) => self
                .groups
                .get(&key)
                .map(|g| g.members().iter().map(|m| m.guid_low).collect())
                .unwrap_or_default(),
            None => vec![actor_low],
        };
        let now = self.server_ms();
        let ginfo = self
            .bg_manager
            .queue_ro(queue_type_id)
            .infos
            .get(&ginfo_id)
            .cloned();
        for member in members {
            if excluded.contains(&member) {
                // fork excluded-members chat: leader name
                // + "is not queued..." family are direct
                // sWorld texts; representative packet only
                // in fork is the group-joined failure,
                // handled above. Excluded members receive
                // nothing here in the fork either (they
                // were reported through leader messages on
                // error paths).
                continue;
            }
            let slot = {
                let Some(p) = self.bg_player_mut_bg(member) else {
                    continue;
                };
                p.battleground.add_queue_id(queue_type_id)
            };
            let avg = ginfo
                .as_ref()
                .map_or(0, |g| self.bg_average_wait_time(queue_type_id, g));
            let waited = ginfo
                .as_ref()
                .map_or(0, |g| now.wrapping_sub(g.join_time_ms));
            let info = if desired_client_instance != 0 {
                self.bg_through_client(desired_client_instance, bg_type_id)
                    .map(|h| BgStatusInfo {
                        map_id: h.map_key.map_id,
                        bracket_id: h.bracket_id,
                        client_instance_id: h.client_instance_id,
                    })
                    .or(Some(Self::bg_template_status_info(bg_type_id)))
            } else {
                Some(Self::bg_template_status_info(bg_type_id))
            };
            let packet =
                build_battleground_status_packet(info, slot, bg_status::WAIT_QUEUE, avg, waited);
            self.bg_send(member, packet);
        }
        // `AddPlayerToBgQueue` final schedule.
        self.bg_schedule_queue_update(queue_type_id, bg_type_id, bracket_id);
        let _ = (team, excluded.drain(..).count());
    }

    /// `HandleBattlemasterJoinOpcode`
    /// (`CMSG_BATTLEMASTER_JOIN`: guid, mapId, instanceId,
    /// asGroup).
    pub(crate) fn on_bg_battlemaster_join(
        &mut self,
        _id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
    ) {
        let mut reader = packet.reader();
        let Ok(_npc_raw) = reader.read_u64_le() else {
            return;
        };
        let Ok(map_id) = reader.read_u32_le() else {
            return;
        };
        let Ok(instance_id) = reader.read_u32_le() else {
            return;
        };
        let as_group = reader.read_u8().unwrap_or(0) != 0;
        let bg_type_id = bg_type_by_map_id(map_id);
        if bg_type_id == 0 {
            return;
        }
        // UNMODELED: battlemaster proximity re-check
        // (`GetNPCIfCanInteractWith`) — hello already
        // gated list display.
        self.bg_join_queue(actor_low, bg_type_id, instance_id, as_group);
    }

    /// `HandleBattlefieldJoinOpcode` (`CMSG_BATTLEFIELD_JOIN`:
    /// mapId, bgTypeId, instanceId, joinAsGroup).
    pub(crate) fn on_bg_battlefield_join(
        &mut self,
        _id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
    ) {
        let mut reader = packet.reader();
        let Ok(map_id) = reader.read_u32_le() else {
            return;
        };
        let Ok(bg_type_raw) = reader.read_u32_le() else {
            return;
        };
        let Ok(instance_id) = reader.read_u32_le() else {
            return;
        };
        let as_group = reader.read_u8().unwrap_or(0) != 0;
        let bg_type_id = if bg_type_raw != 0 {
            bg_type_raw
        } else {
            bg_type_by_map_id(map_id)
        };
        if !(1..=3).contains(&bg_type_id) {
            return;
        }
        self.bg_join_queue(actor_low, bg_type_id, instance_id, as_group);
    }

    /// `HandleBattleFieldPortOpcode` (mapId u32, action u8).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn on_bg_port(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(map_id) = reader.read_u32_le() else {
            return;
        };
        let mut action = reader.read_u8().unwrap_or(0);
        let bg_type_id = bg_type_by_map_id(map_id);
        if bg_type_id == 0 {
            return;
        }
        let queue_type_id = queue_type_of(bg_type_id);
        let in_queue = self
            .bg_player(actor_low)
            .is_some_and(|p| p.battleground.in_battleground_queue());
        if !in_queue {
            return;
        }
        let Some(ginfo) = self.bg_group_info(queue_type_id, actor_low) else {
            return;
        };
        if ginfo.invited_to_instance == 0 && action == 1 {
            return;
        }
        let invited_instance = ginfo.invited_to_instance;
        let handle = if invited_instance != 0 {
            self.bg_live(invited_instance, bg_type_id).cloned()
        } else {
            None
        };
        if handle.is_none() && action == 1 {
            return;
        }
        if action == 1 {
            if !self.bg_can_join(actor_low) {
                let packet = build_group_joined_packet(-2);
                self.bg_send(actor_low, packet);
                action = 0;
            }
            let level = self.bg_player(actor_low).map_or(0, |p| p.unit.level());
            let max_level = handle.as_ref().map_or(u32::MAX, |h| {
                self.maps
                    .get(&h.map_key)
                    .and_then(|m| m.bg.as_ref())
                    .map_or(u32::MAX, |bg| bg.level_max)
            });
            if level > max_level {
                action = 0;
            }
            let status = handle.as_ref().and_then(|h| {
                self.maps
                    .get(&h.map_key)
                    .and_then(|m| m.bg.as_ref())
                    .map(|bg| bg.status)
            });
            if status == Some(bg_status::WAIT_LEAVE) {
                action = 0;
            }
        }
        let queue_slot = self
            .bg_player(actor_low)
            .map_or(MAX_BATTLEGROUND_QUEUES, |p| {
                p.battleground.queue_index(queue_type_id)
            });
        match action {
            1 => {
                let invited = self
                    .bg_player(actor_low)
                    .is_some_and(|p| p.battleground.is_invited_for_queue_type(queue_type_id));
                if !invited {
                    return;
                }
                let Some(handle) = handle else { return };
                // Resurrect the (dead) player
                // (SpawnCorpseBones rides the corpse
                // gap); taxi stop has no Rust system.
                if self
                    .bg_player(actor_low)
                    .is_some_and(|p| !p.unit.is_alive())
                    && let Some(&cur_key) = self.player_map.get(&actor_low)
                    && let Some(map) = self.maps.get_mut(&cur_key)
                {
                    map.bg_resurrect_player(player_guid(actor_low), 1.0);
                }
                let (start_time, dest) = {
                    let Some(map) = self.maps.get(&handle.map_key) else {
                        return;
                    };
                    let Some(bg) = map.bg.as_ref() else { return };
                    let loc = bg.team_start[usize::from(ginfo.team == Team::Horde)];
                    (
                        bg.start_time_ms,
                        Position {
                            x: loc.x,
                            y: loc.y,
                            z: loc.z,
                            o: loc.orientation,
                        },
                    )
                };
                let info = Some(BgStatusInfo {
                    map_id: handle.map_key.map_id,
                    bracket_id: handle.bracket_id,
                    client_instance_id: handle.client_instance_id,
                });
                let packet = build_battleground_status_packet(
                    info,
                    queue_slot,
                    bg_status::IN_PROGRESS,
                    0,
                    start_time,
                );
                self.bg_send(actor_low, packet);
                self.bg_queue_remove_player(queue_type_id, actor_low, false);
                // BG "jumping": close out the current BG
                // before the id flips.
                let current_instance = self
                    .bg_player(actor_low)
                    .map_or(0, |p| p.battleground.bg_instance_id);
                if current_instance != 0
                    && current_instance != invited_instance
                    && let Some(current) = self.bg_manager.live.get(&current_instance).cloned()
                    && let Some(map) = self.maps.get_mut(&current.map_key)
                {
                    map.bg_remove_player_at_leave(player_guid(actor_low), false, true);
                }
                {
                    let pos = self
                        .bg_player(actor_low)
                        .map(|p| (p.unit.world.map_id, p.unit.world.position()));
                    if let Some(p) = self.bg_player_mut_bg(actor_low) {
                        if let Some((mid, pos)) = pos {
                            p.battleground
                                .set_entry_point(mid, pos.x, pos.y, pos.z, pos.o);
                        }
                        p.battleground
                            .set_battleground_id(invited_instance, bg_type_id);
                        p.battleground.set_bg_team(ginfo.team);
                        // Fork clears the queue slot in
                        // `SetBattleGroundId`'s queue fixup
                        // (Player.cpp `SetBattleGroundId`
                        // tail via worldport ack path);
                        // the slot must die before AddPlayer
                        // so status lookups agree.
                        p.battleground.remove_queue_id(queue_type_id);
                    }
                }
                self.drain_map_events();
                self.teleport_player_to_key(id, handle.map_key, dest, false, false);
            }
            0 => {
                if let Some(p) = self.bg_player_mut_bg(actor_low) {
                    p.battleground.remove_queue_id(queue_type_id);
                }
                let info = handle.as_ref().map_or_else(
                    || Some(Self::bg_template_status_info(bg_type_id)),
                    |h| {
                        Some(BgStatusInfo {
                            map_id: h.map_key.map_id,
                            bracket_id: h.bracket_id,
                            client_instance_id: h.client_instance_id,
                        })
                    },
                );
                let packet =
                    build_battleground_status_packet(info, queue_slot, bg_status::NONE, 0, 0);
                self.bg_queue_remove_player(queue_type_id, actor_low, true);
                let bracket = self
                    .bg_bracket_of(actor_low, bg_type_id)
                    .unwrap_or(ginfo.bracket_id);
                self.bg_schedule_queue_update(queue_type_id, bg_type_id, bracket);
                self.bg_send(actor_low, packet);
            }
            _ => {}
        }
    }

    /// `HandleBattlefieldStatusOpcode`.
    pub(crate) fn on_bg_status(&mut self, _id: SessionId, actor_low: u32, _packet: &WorldPacket) {
        let slots: Vec<(u32, u32, u32)> = {
            let Some(p) = self.bg_player(actor_low) else {
                return;
            };
            p.battleground
                .queue_slots
                .iter()
                .enumerate()
                .map(|(i, s)| (i as u32, s.queue_type_id, s.invited_to_instance))
                .collect()
        };
        let now = self.server_ms();
        for (slot, queue_type_id, invited) in slots {
            if queue_type_id == 0 {
                let packet = build_battleground_status_packet(None, slot, bg_status::NONE, 0, 0);
                self.bg_send(actor_low, packet);
                continue;
            }
            let bg_type_id = bg_type_of_queue(queue_type_id);
            if invited != 0 {
                if let Some(handle) = self.bg_live(invited, bg_type_id).cloned() {
                    let remaining = self
                        .bg_group_info(queue_type_id, actor_low)
                        .map_or(0, |g| g.remove_invite_time_ms.saturating_sub(now));
                    let packet = build_battleground_status_packet(
                        Some(BgStatusInfo {
                            map_id: handle.map_key.map_id,
                            bracket_id: handle.bracket_id,
                            client_instance_id: handle.client_instance_id,
                        }),
                        slot,
                        bg_status::WAIT_JOIN,
                        remaining,
                        0,
                    );
                    self.bg_send(actor_low, packet);
                }
            } else if let Some(ginfo) = self.bg_group_info(queue_type_id, actor_low) {
                let avg = self.bg_average_wait_time(queue_type_id, &ginfo);
                let packet = build_battleground_status_packet(
                    Some(Self::bg_template_status_info(bg_type_id)),
                    slot,
                    bg_status::WAIT_QUEUE,
                    avg,
                    now.wrapping_sub(ginfo.join_time_ms),
                );
                self.bg_send(actor_low, packet);
            }
        }
    }

    /// `HandleBattlefieldListOpcode` (mapId).
    pub(crate) fn on_bg_list(&mut self, _id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(map_id) = packet.reader().read_u32_le() else {
            return;
        };
        let bg_type_id = bg_type_by_map_id(map_id);
        if bg_type_id == 0 {
            return;
        }
        // Fork sends with the player's own guid as the
        // battlemaster when none is near? It re-derives
        // from the last interacted guid
        // (`GetBattleGroundLastBattleMasterGuid` — not
        // modeled; use the player guid, matching the
        // fork fallback when no BM is set).
        self.bg_send_list(actor_low, player_guid(actor_low), bg_type_id);
    }

    /// `HandleLeaveBattlefieldOpcode` (mapId u32).
    pub(crate) fn on_bg_leave(&mut self, _id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(map_id) = packet.reader().read_u32_le() else {
            return;
        };
        let Some(player) = self.bg_player(actor_low) else {
            return;
        };
        if player.unit.world.map_id != map_id {
            return;
        }
        // Combat gate: `IsInCombat` rides the unit
        // combat flag.
        let in_combat = player.unit.is_in_combat();
        let bg_instance = player.battleground.bg_instance_id;
        if in_combat && bg_instance != 0 {
            let status = self.bg_manager.live.get(&bg_instance).and_then(|h| {
                self.maps
                    .get(&h.map_key)
                    .and_then(|m| m.bg.as_ref())
                    .map(|bg| bg.status)
            });
            if status != Some(bg_status::WAIT_LEAVE) {
                return;
            }
        }
        self.leave_battleground(actor_low, true);
    }

    /// `HandlePVPLogDataOpcode`.
    pub(crate) fn on_bg_pvp_log(&mut self, _id: SessionId, actor_low: u32, _packet: &WorldPacket) {
        let Some(&key) = self.player_map.get(&actor_low) else {
            return;
        };
        let packet = {
            let Some(map) = self.maps.get(&key) else {
                return;
            };
            if map.bg.is_none() {
                return;
            }
            map.bg_build_pvp_log_data()
        };
        self.bg_send(actor_low, packet);
    }

    /// `HandlePlayerPositionsOpcode`
    /// (`SMSG_BATTLEGROUND_PLAYER_POSITIONS`).
    pub(crate) fn on_bg_positions(
        &mut self,
        _id: SessionId,
        actor_low: u32,
        _packet: &WorldPacket,
    ) {
        let Some(&key) = self.player_map.get(&actor_low) else {
            return;
        };
        let Some(map) = self.maps.get(&key) else {
            return;
        };
        let Some(bg) = map.bg.as_ref() else {
            return;
        };
        let mut writer = PacketWriter::new();
        if bg.bg_type_id == 2 {
            {
                // WSG: flag carrier positions (kind step
                // fills the carriers; until then the fork
                // empty form writes 0/0).
                let (alliance, horde) = self.ws_flag_carriers(key);
                match alliance {
                    Some(guid) => {
                        writer.write_u32_le(1);
                        writer.write_u64_le(guid.raw());
                        let pos = map.object_position(guid).unwrap_or_default();
                        writer.write_f32_le(pos.x);
                        writer.write_f32_le(pos.y);
                    }
                    None => {
                        writer.write_u32_le(0);
                    }
                }
                match horde {
                    Some(guid) => {
                        writer.write_u32_le(1);
                        writer.write_u64_le(guid.raw());
                        let pos = map.object_position(guid).unwrap_or_default();
                        writer.write_f32_le(pos.x);
                        writer.write_f32_le(pos.y);
                    }
                    None => {
                        writer.write_u32_le(0);
                    }
                }
            }
        } else {
            {
                writer.write_u32_le(0);
                writer.write_u32_le(0);
            }
        }
        let mut packet = WorldPacket::new(opcode_id::MSG_BATTLEGROUND_PLAYER_POSITIONS);
        packet.take_writer(writer);
        self.bg_send(actor_low, packet);
    }

    fn ws_flag_carriers(&self, key: MapKey) -> (Option<ObjectGuid>, Option<ObjectGuid>) {
        let Some(map) = self.maps.get(&key) else {
            return (None, None);
        };
        map.ws_flag_carriers_pub()
    }

    /// `HandleAreaSpiritHealerQueryOpcode` /
    /// `HandleAreaSpiritHealerQueueOpcode` — the spirit
    /// guide script (`npc_spirit_guide`) owns the
    /// resurrection queue; until that script ports
    /// (step 7), replies mirror its no-queue behavior.
    ///
    /// UNMODELED: spirit-guide resurrect timers
    /// (script step 7).
    pub(crate) fn on_bg_spirit_healer_query(
        &mut self,
        _id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
    ) {
        let Ok(guid_raw) = packet.reader().read_u64_le() else {
            return;
        };
        let guid = ObjectGuid::from_wire_raw(guid_raw);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        // Time to next resurrection wave; 0 = unknown in
        // the script path, which makes the client fall
        // back to its default timer display.
        writer.write_u32_le(0);
        let mut packet = WorldPacket::new(opcode_id::SMSG_AREA_SPIRIT_HEALER_TIME);
        packet.take_writer(writer);
        self.bg_send(actor_low, packet);
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn on_bg_spirit_healer_queue(
        &mut self,
        _id: SessionId,
        _actor_low: u32,
        _packet: &WorldPacket,
    ) {
        // Spirit-guide script step (step 7).
    }
}

impl World {
    /// `GiveLevel` lower-bracket queue ejection.
    pub(crate) fn bg_on_player_leveled(&mut self, guid_low: u32, new_level: u32) {
        let slots: Vec<(u32, u32)> = {
            let Some(p) = self.bg_player(guid_low) else {
                return;
            };
            p.battleground
                .queue_slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.queue_type_id != 0)
                .map(|(i, s)| (i as u32, s.queue_type_id))
                .collect()
        };
        for (slot, queue_type_id) in slots {
            let bg_type_id = bg_type_of_queue(queue_type_id);
            let Some(template) = self
                .deps
                .world_stores
                .battlegrounds
                .get(bg_type_id)
                .cloned()
            else {
                continue;
            };
            let level_now = self
                .bg_player(guid_low)
                .map_or(new_level, |p| p.unit.level());
            if bracket_id_from_level(template.min_level, new_level)
                == bracket_id_from_level(template.min_level, level_now)
            {
                continue;
            }
            let Some(ginfo) = self.bg_group_info(queue_type_id, guid_low) else {
                continue;
            };
            let info = if ginfo.invited_to_instance != 0 {
                self.bg_status_info(ginfo.invited_to_instance)
            } else {
                Some(Self::bg_template_status_info(bg_type_id))
            };
            if let Some(p) = self.bg_player_mut_bg(guid_low) {
                p.battleground.remove_queue_id(queue_type_id);
            }
            let packet = build_battleground_status_packet(info, slot, bg_status::NONE, 0, 0);
            self.bg_queue_remove_player(queue_type_id, guid_low, true);
            let bracket_now =
                bracket_id_from_level(template.min_level, level_now).unwrap_or(ginfo.bracket_id);
            self.bg_schedule_queue_update(queue_type_id, bg_type_id, bracket_now);
            self.bg_send(guid_low, packet);
        }
    }
}
