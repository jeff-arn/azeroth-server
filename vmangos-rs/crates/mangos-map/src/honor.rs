//! Honor kill flows (Tier 4 B0-7): `Player::
//! RewardHonor` (creature arm: civilian DKs,
//! racial leaders), `Player::RewardHonorOnDeath`
//! (damage-history split), `HonorMgr::Add`
//! map half (PVP credit packet + field refresh),
//! and `MaNGOS::Honor::GetHonorGain` full form.
//! Rank math and CP rows live in
//! `mangos-entities::honor`; the weekly
//! maintenancer in `mangos-db::honor`.
//!
//! UNMODELED: honor `LOG_HONOR` file lines (fork
//! logs account/name/IP per Add; no honor log
//! sink exists), `IsHonorOrXPTarget`
//! `UNIT_STATE_NO_KILL_REWARD` arm (state untracked, as in
//! xp.rs), AV 1.10 kill reputation inside
//! `RewardGroupAtKill_helper` (BG slice recorded
//! it), quest-type honor from spell effects
//! (`SpellEffects.cpp:2914`, spell seam), GM
//! `.honor` commands.

use mangos_entities::honor::{
    HONOR_TYPE_DISHONORABLE, HONOR_TYPE_HONORABLE, HONOR_VICTIM_PLAYER, HONOR_VICTIM_UNIT,
    dishonorable_kill_points,
};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

use crate::map::{Map, MapEvent};
use crate::xp::{base_gain_level_factor, gray_level, xp_in_group_rate};

/// `RACIAL_LEADER_HONOR` at client build > 1.6.1
/// (`HonorMgr.h`).
const RACIAL_LEADER_HONOR: f32 = 488.0;
/// `CREATURE_TYPE_TOTEM`.
const CREATURE_TYPE_TOTEM: u32 = 11;
/// `SPELL_AURA_NO_PVP_CREDIT`
/// (`SpellAuraDefines.h:297`).
const AURA_NO_PVP_CREDIT: u32 = 159;

/// `MaNGOS::Honor::GetHonorGain` (`Formulas.h:
/// 198`) full form with patch gates. The
/// victim rank enters as the fork's sign-extended
/// `visualRank`-to-`uint32` (negative ranks make
/// the exponential overflow exactly as in C++).
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::too_many_arguments)]
pub fn honor_gain_full(
    wow_patch: u32,
    accurate_rewards: bool,
    killer_level: u32,
    victim_level: u32,
    victim_visual_rank: i8,
    total_kills: u32,
    group_size: u32,
) -> f32 {
    let diff_factor = f64::from(base_gain_level_factor(killer_level, victim_level));
    let penalty: f64 = if wow_patch >= 10 || !accurate_rewards {
        10.0
    } else {
        4.0
    };
    #[allow(clippy::cast_possible_truncation)]
    let same_victim_penalty = if total_kills >= penalty as u32 {
        0.0
    } else {
        1.0 - f64::from(total_kills) / penalty
    };
    let level_coeff: f64 = match killer_level {
        60.. => 1.0,
        50..=59 => 0.9545,
        40..=49 => 0.5707,
        30..=39 => 0.3434,
        20..=29 => 0.2070,
        _ => 0.1212,
    };
    let exp_factor: f64 = if wow_patch < 6 && accurate_rewards {
        157.4
    } else {
        188.3
    };
    let victim_rank = u32::from_ne_bytes(i32::from(victim_visual_rank).to_ne_bytes());
    #[allow(clippy::cast_possible_truncation)]
    let gain = (level_coeff
        * same_victim_penalty
        * (exp_factor * (0.05331 * f64::from(victim_rank)).exp())
        * diff_factor
        / f64::from(group_size.max(1))) as f32;
    gain
}

impl Map {
    /// `sWorld.GetGameDay()`:
    /// `(gameTime + timeZoneOffset) / DAY`.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub(crate) fn game_day(&self) -> u32 {
        ((self.now_unix as i64 + self.config.time_zone_offset_secs) / 86_400) as u32
    }

    /// Honor system timeline gate (`RewardHonor`
    /// head): pre-1.4 with accurate timeline off.
    fn honor_system_on(&self) -> bool {
        !(self.config.wow_patch < 2 && self.config.accurate_pvp_timeline)
    }

    fn unit_has_no_pvp_credit(&self, unit: ObjectGuid) -> bool {
        self.unit_base(unit).is_some_and(|u| {
            u.auras.iter().any(|h| {
                h.effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == AURA_NO_PVP_CREDIT)
            })
        })
    }

    /// `IsHonorOrXPTarget` (`Player.cpp:19699`)
    /// minus the untracked `NO_KILL_REWARD` arm.
    pub(crate) fn is_honor_or_xp_target(&self, player: ObjectGuid, victim: ObjectGuid) -> bool {
        let level = self.unit_level(player);
        if self.unit_level(victim) <= gray_level(level) {
            return false;
        }
        if victim.is_creature() {
            let Some(c) = self.creatures.get(&victim) else {
                return false;
            };
            if c.info.creature_type == CREATURE_TYPE_TOTEM
                || self.pet_owner(victim).is_some()
                || c.info.xp_multiplier == 0.0
            {
                return false;
            }
        }
        true
    }

    /// `SendPVPCredit` (`HonorMgr.cpp:990`).
    fn send_pvp_credit(&mut self, to: ObjectGuid, honor: f32, victim: Option<ObjectGuid>) {
        let mut writer = PacketWriter::new();
        #[allow(clippy::cast_possible_truncation)]
        writer.write_bytes(&(honor as i32).to_le_bytes());
        match victim {
            None => {
                writer.write_u64_le(0);
                writer.write_u32_le(0);
            }
            Some(victim) => {
                writer.write_u64_le(victim.raw());
                let rank: i32 = if victim.is_creature() {
                    if self
                        .creatures
                        .get(&victim)
                        .is_some_and(|c| c.info.racial_leader)
                    {
                        19
                    } else {
                        0
                    }
                } else if victim.is_player() {
                    let rank = self
                        .players
                        .get(&victim)
                        .map_or(0, |e| i32::from(e.player.honor_rank().rank));
                    if rank == 0 {
                        // First rank instead of bare
                        // "HK:" (fork comment).
                        5
                    } else {
                        rank
                    }
                } else {
                    0
                };
                writer.write_bytes(&rank.to_le_bytes());
            }
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_PVP_CREDIT);
        packet.take_writer(writer);
        self.events.push(MapEvent::Packet { to, packet });
    }

    /// `HonorMgr::Add` map half: record the row,
    /// send the credit packet, refresh fields.
    /// `source` None is the fork's null source
    /// (self row, empty credit victim).
    pub(crate) fn honor_add_player(
        &mut self,
        player: ObjectGuid,
        cp: f32,
        kind: u8,
        source: Option<ObjectGuid>,
    ) -> bool {
        if cp == 0.0 {
            return false;
        }
        let (victim_type, victim_id) = match source {
            Some(src) if src.is_player() => (HONOR_VICTIM_PLAYER, src.counter()),
            Some(src) if src.is_creature() => (
                HONOR_VICTIM_UNIT,
                self.creatures.get(&src).map_or(0, |c| c.original_entry),
            ),
            _ => (0, player.counter()),
        };
        let day = self.game_day();
        let week_begin = self.config.honor_week_begin_day;
        let added = {
            let Some(p) = self.player_mut(player) else {
                return false;
            };
            p.honor_add(cp, kind, victim_type, victim_id, day)
        };
        if !added {
            return false;
        }
        let honor = if kind == HONOR_TYPE_DISHONORABLE {
            -cp
        } else {
            cp
        };
        // Fork `realSource`: null stays null in the
        // credit packet (self rows included).
        self.send_pvp_credit(player, honor, source);
        if let Some(p) = self.player_mut(player) {
            p.honor_update(day, week_begin);
        }
        true
    }

    /// `HonorMgr::Update` at login/save times.
    pub fn honor_refresh_player(&mut self, player: ObjectGuid) {
        let day = self.game_day();
        let week_begin = self.config.honor_week_begin_day;
        if let Some(p) = self.player_mut(player) {
            p.honor_update(day, week_begin);
        }
    }

    /// `Player::RewardHonor` creature arm
    /// (`Player.cpp:21548`): civilian DK /
    /// racial leader honor for one rewarded
    /// player (solo or group member).
    pub(crate) fn reward_honor_creature(&mut self, player: ObjectGuid, victim: ObjectGuid) {
        if !self.honor_system_on() || self.unit_has_no_pvp_credit(victim) {
            return;
        }
        let (civilian, racial_leader) = match self.creatures.get(&victim) {
            Some(c) => (c.info.civilian, c.info.racial_leader),
            None => return,
        };
        if civilian && !self.is_honor_or_xp_target(player, victim) {
            if !self.config.enable_dk {
                return;
            }
            if self.config.wow_patch < 3 && self.config.accurate_pvp_timeline {
                return;
            }
            let level = self
                .players
                .get(&player)
                .map_or(0, |e| e.player.unit.level());
            #[allow(clippy::cast_possible_truncation)]
            let points = dishonorable_kill_points(level.min(255) as u8);
            self.honor_add_player(player, points, HONOR_TYPE_DISHONORABLE, Some(victim));
            return;
        }
        if racial_leader {
            self.honor_add_player(
                player,
                RACIAL_LEADER_HONOR,
                HONOR_TYPE_HONORABLE,
                Some(victim),
            );
        }
    }

    /// `IsAtGroupRewardDistance` for a player
    /// victim source: raid instances unbounded,
    /// else the 74yd loot/XP band (corpse arm
    /// unreachable — distribution gates alive).
    fn honor_at_reward_distance(&self, pos: Position, victim_pos: Position) -> bool {
        if self
            .stores
            .maps
            .get(self.map_id)
            .is_some_and(|m| m.instanceable() && m.is_raid())
        {
            return true;
        }
        let dx = pos.x - victim_pos.x;
        let dy = pos.y - victim_pos.y;
        dx * dx + dy * dy < self.config.max_group_xp_distance * self.config.max_group_xp_distance
    }

    /// `HonorableKillPoints` (`HonorMgr.cpp:977`).
    fn honorable_kill_points(&self, killer: ObjectGuid, victim: ObjectGuid) -> f32 {
        let day = self.game_day();
        let (total_kills, killer_level) = match self.players.get(&killer) {
            Some(e) => (
                e.player
                    .honor_total_kills_today(HONOR_VICTIM_PLAYER, victim.counter(), day),
                e.player.unit.level(),
            ),
            None => return 0.0,
        };
        let (victim_rank, victim_level) = match self.players.get(&victim) {
            Some(e) => (e.player.honor_rank().visual_rank, e.player.unit.level()),
            None => return 0.0,
        };
        honor_gain_full(
            self.config.wow_patch,
            self.config.accurate_pvp_rewards,
            killer_level,
            victim_level,
            victim_rank,
            total_kills,
            1,
        )
    }

    /// `Player::RewardHonorOnDeath`
    /// (`Player.cpp:21583`): split the victim's
    /// damage history over attacker groups and
    /// solo attackers.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn reward_honor_on_death(&mut self, victim: ObjectGuid) {
        if !self.honor_system_on() || self.unit_has_no_pvp_credit(victim) {
            return;
        }
        let history: Vec<(ObjectGuid, u32)> = match self.players.get_mut(&victim) {
            Some(entry) => {
                let h = entry
                    .player
                    .unit
                    .damage_taken_history
                    .iter()
                    .map(|(g, d)| (*g, *d))
                    .collect();
                entry.player.unit.damage_taken_history.clear();
                h
            }
            None => return,
        };
        let total_damage: u32 = history.iter().map(|(_, d)| *d).sum();
        if total_damage == 0 {
            return;
        }
        let (victim_team, victim_pos) = match self.players.get(&victim) {
            Some(e) => (e.player.team, e.player.unit.world.position()),
            None => return,
        };
        // Damage per group / per alone player.
        let mut per_group: Vec<(u32, u32)> = Vec::new();
        let mut per_alone: Vec<(ObjectGuid, u32)> = Vec::new();
        for (attacker, damage) in &history {
            let Some(entry) = self.players.get(attacker) else {
                continue;
            };
            if let Some(group_id) = entry.player.group_id {
                match per_group.iter_mut().find(|(g, _)| *g == group_id) {
                    Some((_, sum)) => *sum += *damage,
                    None => per_group.push((group_id, *damage)),
                }
            } else if entry.player.unit.is_alive()
                && entry.player.team != victim_team
                && self.honor_at_reward_distance(entry.player.unit.world.position(), victim_pos)
            {
                match per_alone.iter_mut().find(|(g, _)| g == attacker) {
                    Some((_, sum)) => *sum += *damage,
                    None => per_alone.push((*attacker, *damage)),
                }
            }
        }

        // Per-group distribution.
        for (group_id, group_damage) in per_group {
            let rewarded: Vec<ObjectGuid> = self
                .players
                .iter()
                .filter(|(_, e)| {
                    e.player.group_id == Some(group_id)
                        && e.player.unit.is_alive()
                        && e.player.team != victim_team
                        && self.honor_at_reward_distance(e.player.unit.world.position(), victim_pos)
                })
                .map(|(g, _)| *g)
                .collect();
            let total_rewarded = rewarded.len() as u32;
            if total_rewarded == 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let mut honor_rate = group_damage as f32;
            honor_rate *= xp_in_group_rate(total_rewarded);
            #[allow(clippy::cast_precision_loss)]
            {
                honor_rate /= total_damage as f32;
                honor_rate /= total_rewarded as f32;
            }
            for rew in rewarded {
                if !self.is_honor_or_xp_target(rew, victim) {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let points = (self.honorable_kill_points(rew, victim) * honor_rate) as u32;
                if points > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    self.honor_add_player(rew, points as f32, HONOR_TYPE_HONORABLE, Some(victim));
                }
            }
        }

        // Solo distribution.
        for (attacker, damage) in per_alone {
            if !self.is_honor_or_xp_target(attacker, victim) {
                continue;
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let points = (self.honorable_kill_points(attacker, victim) * damage as f32
                / total_damage as f32) as u32;
            if points > 0 {
                #[allow(clippy::cast_precision_loss)]
                self.honor_add_player(attacker, points as f32, HONOR_TYPE_HONORABLE, Some(victim));
            }
        }
    }
}
