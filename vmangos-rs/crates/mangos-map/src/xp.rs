//! Kill XP: the `MaNGOS::XP` formulas (`Formulas.h`)
//! and the at-kill reward distribution
//! (`Player::RewardSinglePlayerAtKill` /
//! `Group::RewardGroupAtKill`, XP arms only — honor
//! scalars stay stored-verbatim and reputation-on-kill
//! has no runtime yet, matching the entities posture).
//!
//! Fork deltas, all in currently-unmodeled systems:
//! `GetXPModifierDueToDamageOrigin` is 1.0 (damage
//! origin untracked), `UNIT_STATE_NO_KILL_REWARD` is
//! never set, temp summons stand in for
//! `UNIT_CREATED_BY_SPELL`, pet XP waits for the pet
//! runtime, and the reward goes to the killing-blow
//! player (fork `Unit::Kill` likewise keys off the
//! killer; only loot uses the tap).

use mangos_entities::player::XpGainOutcome;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

use mangos_world_data::creature_template::CreatureInfo;

use crate::map::{Map, MapEvent};

/// `CREATURE_TYPE_CRITTER` (`CreatureDefines.h:50`).
const CREATURE_TYPE_CRITTER: u32 = 8;
/// `CREATURE_TYPE_NOT_SPECIFIED` (`CreatureDefines.h:52`).
const CREATURE_TYPE_NOT_SPECIFIED: u32 = 10;
/// `CREATURE_TYPE_TOTEM` (`CreatureDefines.h:53`).
const CREATURE_TYPE_TOTEM: u32 = 11;
/// `CREATURE_ELITE_NORMAL` / `CREATURE_ELITE_RARE`
/// (`CreatureDefines.h:90-94`; `IsElite` excludes both).
const RANK_NORMAL: u32 = 0;
const RANK_WORLDBOSS: u32 = 3;
const RANK_RARE: u32 = 4;
/// `CREATURE_STATIC_FLAG_NO_XP` (`CreatureDefines.h:100`).
const STATIC_FLAG_NO_XP: u32 = 0x0000_0002;
/// `CREATURE_STATIC_FLAG_CORPSE_RAID`
/// (`CreatureDefines.h:111`).
const STATIC_FLAG_CORPSE_RAID: u32 = 0x0000_1000;
/// `MaxGroupXPDistance` tree default (`World.cpp:561`).
const GROUP_XP_DISTANCE: f32 = 74.0;

/// `XP::GetGrayLevel`.
#[must_use]
pub fn gray_level(pl_level: u32) -> u32 {
    if pl_level <= 5 {
        0
    } else if pl_level <= 39 {
        pl_level - 5 - pl_level / 10
    } else {
        pl_level - 1 - pl_level / 5
    }
}

/// `XP::GetZeroDifference`.
#[must_use]
pub fn zero_difference(pl_level: u32) -> u32 {
    match pl_level {
        0..8 => 5,
        8..10 => 6,
        10..12 => 7,
        12..16 => 8,
        16..20 => 9,
        20..30 => 11,
        30..40 => 12,
        40..45 => 13,
        45..50 => 14,
        50..55 => 15,
        55..60 => 16,
        _ => 17,
    }
}

/// `XP::BaseGainLevelFactor`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn base_gain_level_factor(pl_level: u32, victim_level: u32) -> f32 {
    if victim_level >= pl_level {
        let diff = (victim_level - pl_level).min(4);
        1.0 + 0.05 * diff as f32
    } else if victim_level > gray_level(pl_level) {
        let zd = zero_difference(pl_level);
        (zd + victim_level - pl_level) as f32 / zd as f32
    } else {
        0.0
    }
}

/// `XP::BaseGain`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn base_gain(owner_level: u32, unit_level: u32, mob_level: u32) -> f32 {
    ((owner_level * 5) as f32 + 45.0) * base_gain_level_factor(unit_level, mob_level)
}

/// `XP::xp_in_group_rate` (the raid flag is unused in
/// the fork formula).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn xp_in_group_rate(count: u32) -> f32 {
    match count {
        0..=2 => 1.0,
        3 => 1.166,
        4 => 1.3,
        5 => 1.4,
        _ => (1.0 - count as f32 * 0.05).max(0.01),
    }
}

/// `Creature::IsElite` for a non-pet victim.
fn is_elite(info: &CreatureInfo) -> bool {
    info.rank != RANK_NORMAL && info.rank != RANK_RARE
}

/// `XP::Gain` for a player gainer (pet gainers join
/// the pet runtime): 0 for gray/no-XP victims.
#[must_use]
pub fn creature_xp_gain(
    gainer_level: u32,
    mob_level: u32,
    info: &CreatureInfo,
    created_by_spell: bool,
    non_raid_dungeon: bool,
    rates: crate::map::XpRates,
) -> u32 {
    if created_by_spell
        && (info.creature_type == CREATURE_TYPE_CRITTER
            || info.creature_type == CREATURE_TYPE_NOT_SPECIFIED
            || info.creature_type == CREATURE_TYPE_TOTEM
            || info.health_multiplier <= 0.1)
    {
        return 0;
    }
    if info.static_flags1 & STATIC_FLAG_NO_XP != 0 {
        return 0;
    }
    let mut xp_gain = base_gain(gainer_level, gainer_level, mob_level);
    if xp_gain == 0.0 {
        return 0;
    }
    if is_elite(info) {
        xp_gain *= if non_raid_dungeon { 2.5 } else { 2.0 };
        xp_gain *= rates.kill_elite;
    }
    xp_gain *= info.xp_multiplier;
    xp_gain *= rates.kill;
    // `std::nearbyint`: round half to even.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        xp_gain.round_ties_even() as u32
    }
}

/// The discovery XP of `CheckAreaExploreAndOutdoor`
/// (`Player.cpp:6336-6354`), with the two `GetBaseXP`
/// reads taken as arguments.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss
)]
pub fn exploration_xp(
    level: u32,
    area_level: i32,
    base_at_area: u32,
    base_at_plus5: u32,
    rate: f32,
) -> u32 {
    let diff = level as i32 - area_level;
    if diff < -5 {
        (base_at_plus5 as f32 * rate) as u32
    } else if diff > 5 {
        let percent = (100 - ((diff - 5) * 5)).clamp(0, 100);
        ((base_at_area * percent as u32 / 100) as f32 * rate) as u32
    } else {
        (base_at_area as f32 * rate) as u32
    }
}

fn build_exploration_experience(area: u32, xp: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(area);
    writer.write_u32_le(xp);
    let mut packet = WorldPacket::new(opcode_id::SMSG_EXPLORATION_EXPERIENCE);
    packet.take_writer(writer);
    packet
}

fn build_log_xpgain(victim: Option<ObjectGuid>, outcome: &XpGainOutcome) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(victim.map_or(0, ObjectGuid::raw));
    writer.write_u32_le(outcome.given_xp + outcome.rest_xp);
    writer.write_u8(u8::from(victim.is_none()));
    if victim.is_some() {
        writer.write_u32_le(outcome.given_xp);
        writer.write_f32_le(1.0);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_LOG_XPGAIN);
    packet.take_writer(writer);
    packet
}

fn build_levelup_info(info: &mangos_entities::player::LevelUpInfo) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(info.level);
    writer.write_u32_le(info.health_gain);
    writer.write_u32_le(info.mana_gain);
    for _ in 0..4 {
        writer.write_u32_le(0);
    }
    for gain in info.stat_gains {
        #[allow(clippy::cast_sign_loss)]
        writer.write_u32_le(gain as u32);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_LEVELUP_INFO);
    packet.take_writer(writer);
    packet
}

impl Map {
    /// Queue the `GiveXP` packets for one outcome
    /// (log first, then level-ups, as the fork sends).
    pub(crate) fn emit_xp_outcome(
        &mut self,
        to: ObjectGuid,
        victim: Option<ObjectGuid>,
        outcome: &XpGainOutcome,
    ) {
        self.events.push(MapEvent::Packet {
            to,
            packet: build_log_xpgain(victim, outcome),
        });
        for info in &outcome.level_ups {
            self.events.push(MapEvent::Packet {
                to,
                packet: build_levelup_info(info),
            });
        }
    }

    /// `IsAtGroupRewardDistance` via `IsWithinLootXPDist`
    /// (2D, strict; the fork's size-factor addition is
    /// unmodeled — object sizes are not tracked).
    fn at_group_reward_distance(
        &self,
        member_pos: Position,
        victim_pos: Position,
        victim: &CreatureInfo,
    ) -> bool {
        let map_entry = self.stores.maps.get(self.map_id);
        if let Some(entry) = map_entry
            && entry.instanceable()
            && (entry.is_raid() || victim.static_flags1 & STATIC_FLAG_CORPSE_RAID != 0)
        {
            return true;
        }
        let mut dist = GROUP_XP_DISTANCE;
        if victim.rank == RANK_WORLDBOSS {
            dist += 150.0;
        }
        let dx = member_pos.x - victim_pos.x;
        let dy = member_pos.y - victim_pos.y;
        dx * dx + dy * dy < dist * dist
    }

    /// The XP arm of `RewardSinglePlayerAtKill` /
    /// `RewardGroupAtKill`, from `kill_unit` for
    /// creature victims with a player(-owned) killer.
    pub(crate) fn reward_kill_xp(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let Some(killer) = killer else {
            return;
        };
        // PvP victims (player-owned creatures) grant no
        // XP in either fork path.
        if self
            .summons
            .get(&victim)
            .is_some_and(|st| st.summoner.is_player())
            || self
                .pet_owner(victim)
                .is_some_and(mangos_guid::ObjectGuid::is_player)
        {
            return;
        }
        // `GetCharmerOrOwnerPlayerOrPlayerItself` over
        // the modeled owner chain (direct summoner).
        let reward_player = if killer.is_player() && self.players.contains_key(&killer) {
            killer
        } else if let Some(summoner) = self
            .summons
            .get(&killer)
            .map(|st| st.summoner)
            .filter(|g| g.is_player() && self.players.contains_key(g))
        {
            summoner
        } else if let Some(owner) = self
            .pet_owner(killer)
            .filter(|g| g.is_player() && self.players.contains_key(g))
        {
            owner
        } else {
            return;
        };
        let Some((mob_level, info)) = self
            .creatures
            .get(&victim)
            .map(|c| (c.unit.level(), c.info.clone()))
        else {
            return;
        };
        let created_by_spell = self.summons.contains_key(&victim);
        let non_raid_dungeon = self
            .stores
            .maps
            .get(self.map_id)
            .is_some_and(mangos_world_data::map_template::MapEntry::is_non_raid_dungeon);
        let group_id = self
            .players
            .get(&reward_player)
            .and_then(|e| e.player.group_id);
        let Some(group_id) = group_id else {
            // Solo: `RewardSinglePlayerAtKill`.
            let level = self.players[&reward_player].player.unit.level();
            let xp = creature_xp_gain(
                level,
                mob_level,
                &info,
                created_by_spell,
                non_raid_dungeon,
                self.config.xp_rates,
            );
            self.grant_kill_xp(reward_player, victim, xp);
            // Pet XP only when the mob is not grey to
            // the owner (`RewardSinglePlayerAtKill`).
            if xp > 0
                && let Some(pet) = self.pet_of(reward_player)
            {
                let pet_xp = creature_xp_gain(
                    self.unit_level(pet),
                    mob_level,
                    &info,
                    created_by_spell,
                    non_raid_dungeon,
                    self.config.xp_rates,
                );
                self.give_pet_xp(pet, pet_xp);
            }
            return;
        };
        self.reward_group_kill_xp(
            victim,
            mob_level,
            &info,
            group_id,
            created_by_spell,
            non_raid_dungeon,
        );
    }

    /// `RewardGroupAtKill`'s XP arm with
    /// `GetDataForXPAtKill` over the same-map members
    /// (the fork iterates the whole group; cross-map
    /// members fail its distance test).
    fn reward_group_kill_xp(
        &mut self,
        victim: ObjectGuid,
        mob_level: u32,
        info: &CreatureInfo,
        group_id: u32,
        created_by_spell: bool,
        non_raid_dungeon: bool,
    ) {
        let victim_pos = self.unit_position(victim).unwrap_or_default();
        let mut count: u32 = 0;
        let mut sum_level: u32 = 0;
        let mut member_with_max: Option<(ObjectGuid, u32)> = None;
        let mut not_gray_with_max: Option<(ObjectGuid, u32)> = None;
        let mut members: Vec<ObjectGuid> = Vec::new();
        for (guid, entry) in &self.players {
            if entry.player.group_id != Some(group_id) || !entry.player.unit.is_alive() {
                continue;
            }
            let Some(pos) = self.unit_position(*guid) else {
                continue;
            };
            if !self.at_group_reward_distance(pos, victim_pos, info) {
                continue;
            }
            count += 1;
            members.push(*guid);
            let level = entry.player.unit.level();
            sum_level += level;
            if member_with_max.is_none_or(|(_, l)| l < level) {
                member_with_max = Some((*guid, level));
            }
            if mob_level > gray_level(level) && not_gray_with_max.is_none_or(|(_, l)| l < level) {
                not_gray_with_max = Some((*guid, level));
            }
        }
        let (Some((max_guid, _)), Some((not_gray_guid, not_gray_level))) =
            (member_with_max, not_gray_with_max)
        else {
            return;
        };
        let xp = creature_xp_gain(
            not_gray_level,
            mob_level,
            info,
            created_by_spell,
            non_raid_dungeon,
            self.config.xp_rates,
        );
        let group_rate = xp_in_group_rate(count);
        for guid in members {
            let level = self.players[&guid].player.unit.level();
            if level > not_gray_level {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let rate = group_rate * level as f32 / sum_level as f32;
            #[allow(clippy::cast_precision_loss)]
            let member_xp = if max_guid == not_gray_guid {
                xp as f32 * rate
            } else {
                xp as f32 * rate / 2.0 + 1.0
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let member_xp = member_xp as u32;
            self.grant_kill_xp(guid, victim, member_xp);
            // Group pet XP: same share as the owner
            // when the pet is not above the not-gray
            // max level (`RewardGroupAtKill_helper`).
            if let Some(pet) = self.pet_of(guid)
                && self.unit_level(pet) <= not_gray_level
            {
                self.give_pet_xp(pet, member_xp);
            }
        }
    }

    fn grant_kill_xp(&mut self, player: ObjectGuid, victim: ObjectGuid, xp: u32) {
        let infos = &self.stores.player_infos;
        let items = Some(&self.stores.item_prototypes);
        let skills = Some(&self.stores.skills);
        let outcome = {
            let Some(entry) = self.players.get_mut(&player) else {
                return;
            };
            entry.player.give_xp(xp, true, infos, items, skills)
        };
        if let Some(outcome) = outcome {
            self.emit_xp_outcome(player, Some(victim), &outcome);
        }
    }

    /// Quest kill credit (`KilledMonsterCredit`
    /// distribution mirroring the XP recipient set;
    /// raid-group exclusion unmodeled, groups here
    /// carry no raid flag).
    pub(crate) fn quest_kill_credit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let Some(killer) = killer else {
            return;
        };
        let reward_player = if killer.is_player() && self.players.contains_key(&killer) {
            killer
        } else if let Some(summoner) = self
            .summons
            .get(&killer)
            .map(|st| st.summoner)
            .filter(|g| g.is_player() && self.players.contains_key(g))
        {
            summoner
        } else if let Some(owner) = self
            .pet_owner(killer)
            .filter(|g| g.is_player() && self.players.contains_key(g))
        {
            owner
        } else {
            return;
        };
        let Some((entry, info)) = self
            .creatures
            .get(&victim)
            .map(|c| (c.original_entry, c.info.clone()))
        else {
            return;
        };
        if entry == 0 {
            return;
        }
        // Recipients: solo killer, or same-group
        // members at group reward distance.
        let group_id = self
            .players
            .get(&reward_player)
            .and_then(|e| e.player.group_id);
        let recipients: Vec<ObjectGuid> = match group_id {
            None => vec![reward_player],
            Some(group_id) => {
                let victim_pos = self.unit_position(victim).unwrap_or_default();
                self.players
                    .iter()
                    .filter(|(guid, e)| {
                        e.player.group_id == Some(group_id)
                            && e.player.unit.is_alive()
                            && self.unit_position(**guid).is_some_and(|pos| {
                                self.at_group_reward_distance(pos, victim_pos, &info)
                            })
                    })
                    .map(|(guid, _)| *guid)
                    .collect()
            }
        };
        for recipient in recipients {
            self.quest_kill_credit_one(recipient, victim, entry);
        }
    }

    /// One player's `KilledMonsterCredit` body.
    fn quest_kill_credit_one(&mut self, player_guid: ObjectGuid, victim: ObjectGuid, entry: u32) {
        use mangos_protocol::buffer::PacketWriter;
        // Snapshot candidate quests first (borrow
        // discipline against the stores).
        let candidates: Vec<u32> = {
            let Some(entry_rec) = self.players.get(&player_guid) else {
                return;
            };
            entry_rec
                .player
                .deferred
                .quest_status
                .iter()
                .filter(|r| r.status == 3)
                .map(|r| r.quest)
                .collect()
        };
        for quest_id in candidates {
            let Some(q) = self.stores.quest_templates.get(quest_id).cloned() else {
                continue;
            };
            for idx in 0..4 {
                if q.req_creature_or_go_ids[idx] <= 0 || q.req_spell_cast[idx] != 0 {
                    continue;
                }
                #[allow(clippy::cast_sign_loss)]
                if q.req_creature_or_go_ids[idx] as u32 != entry {
                    continue;
                }
                let req = q.req_creature_or_go_counts[idx];
                let new_count = {
                    let Some(entry_rec) = self.players.get_mut(&player_guid) else {
                        return;
                    };
                    let player = &mut entry_rec.player;
                    let Some(row) = player
                        .deferred
                        .quest_status
                        .iter_mut()
                        .find(|r| r.quest == quest_id)
                    else {
                        continue;
                    };
                    if row.mob_count[idx] >= req {
                        continue;
                    }
                    row.mob_count[idx] += 1;
                    let new_count = row.mob_count[idx];
                    if let Some(slot) = player.find_quest_slot(quest_id) {
                        #[allow(clippy::cast_possible_truncation)]
                        player.set_quest_slot_counter(slot, idx as u8, new_count as u8);
                    }
                    new_count
                };
                let mut writer = PacketWriter::new();
                writer.write_u32_le(quest_id);
                writer.write_u32_le(entry);
                writer.write_u32_le(new_count);
                writer.write_u32_le(req);
                writer.write_u64_le(victim.raw());
                let mut packet = WorldPacket::new(
                    mangos_protocol::opcode_table::opcode_id::SMSG_QUESTUPDATE_ADD_KILL,
                );
                packet.take_writer(writer);
                self.events.push(MapEvent::Packet {
                    to: player_guid,
                    packet,
                });
                // Completion transition.
                let completed = {
                    let Some(entry_rec) = self.players.get_mut(&player_guid) else {
                        return;
                    };
                    let player = &mut entry_rec.player;
                    player.can_complete_quest(&q) && player.complete_quest(quest_id)
                };
                if completed {
                    let mut writer = PacketWriter::new();
                    writer.write_u32_le(quest_id);
                    let mut packet = WorldPacket::new(
                        mangos_protocol::opcode_table::opcode_id::SMSG_QUESTUPDATE_COMPLETE,
                    );
                    packet.take_writer(writer);
                    self.events.push(MapEvent::Packet {
                        to: player_guid,
                        packet,
                    });
                }
            }
        }
    }

    /// Quest reward XP (`GiveXP(xp, nullptr)` from
    /// `RewardQuest`): no rested bonus (the fork only
    /// applies it with a victim), no victim in the log.
    pub fn reward_quest_xp(&mut self, player: ObjectGuid, xp: u32) {
        let infos = &self.stores.player_infos;
        let items = Some(&self.stores.item_prototypes);
        let skills = Some(&self.stores.skills);
        let outcome = {
            let Some(entry) = self.players.get_mut(&player) else {
                return;
            };
            entry.player.give_xp(xp, false, infos, items, skills)
        };
        if let Some(outcome) = outcome {
            self.emit_xp_outcome(player, None, &outcome);
        }
    }

    /// The discovery arm of `CheckAreaExploreAndOutdoor`
    /// (`Player.cpp:6250`), run on player position change
    /// (fork `Movement.RelocationVmapsCheckDelay` defaults
    /// to 0 = immediate). The taxi-flight and cinematic
    /// gates are unreachable (both unmodeled); the
    /// outdoor-spell and tavern-exit arms of the fork
    /// function are not ported here.
    pub(crate) fn check_area_explore(&mut self, player: ObjectGuid) {
        let Some((alive, position, level)) = self.players.get(&player).map(|entry| {
            (
                entry.player.unit.is_alive(),
                entry.player.unit.world.position(),
                entry.player.unit.level(),
            )
        }) else {
            return;
        };
        if !alive {
            return;
        }
        let area_flag = self.area_flag_at(position);
        if area_flag == 0xffff {
            return;
        }
        let Some(entry) = self.players.get_mut(&player) else {
            return;
        };
        if entry.player.has_explored_flag(area_flag) {
            return;
        }
        entry.player.set_explored_flag(area_flag);
        let Some(area) = self.stores.areas.by_flag_and_map(area_flag, self.map_id) else {
            return;
        };
        let mut xp = 0;
        if area.area_level > 0 && level < self.stores.player_infos.max_player_level() {
            xp = exploration_xp(
                level,
                area.area_level,
                self.stores
                    .exploration_base_xp
                    .base_xp(area.area_level as u32),
                self.stores.exploration_base_xp.base_xp(level + 5),
                self.config.xp_rates.explore,
            );
            let infos = &self.stores.player_infos;
            let items = Some(&self.stores.item_prototypes);
            let skills = Some(&self.stores.skills);
            let outcome = {
                let Some(entry) = self.players.get_mut(&player) else {
                    return;
                };
                entry.player.give_xp(xp, false, infos, items, skills)
            };
            if let Some(outcome) = outcome {
                self.emit_xp_outcome(player, None, &outcome);
            }
        }
        // Sent even when no XP is gained (`Player.cpp:6358`).
        self.events.push(MapEvent::Packet {
            to: player,
            packet: build_exploration_experience(area.id, xp),
        });
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn gray_and_zero_difference_tables() {
        assert_eq!(gray_level(5), 0);
        assert_eq!(gray_level(10), 4);
        assert_eq!(gray_level(39), 31);
        assert_eq!(gray_level(60), 47);
        assert_eq!(zero_difference(7), 5);
        assert_eq!(zero_difference(10), 7);
        assert_eq!(zero_difference(59), 16);
        assert_eq!(zero_difference(60), 17);
    }

    #[test]
    fn base_gain_factors() {
        // Same level: (10*5 + 45) * 1.0.
        assert_eq!(base_gain(10, 10, 10), 95.0);
        // +4 levels caps the bonus at 1.2x, +9 same
        // (f32: 95 * 1.2 = 114.00001, as in C++ float).
        assert!((base_gain(10, 10, 14) - 114.0).abs() < 1e-3);
        assert!((base_gain(10, 10, 19) - 114.0).abs() < 1e-3);
        // Green: ZD(10)=7, (7+6-10)/7 = 3/7 of base.
        assert!((base_gain(10, 10, 6) - 95.0 * 3.0 / 7.0).abs() < 1e-4);
        // Gray (level <= gray 4) grants nothing.
        assert_eq!(base_gain(10, 10, 4), 0.0);
    }

    #[test]
    fn group_rates() {
        assert_eq!(xp_in_group_rate(1), 1.0);
        assert_eq!(xp_in_group_rate(3), 1.166);
        assert_eq!(xp_in_group_rate(5), 1.4);
        assert_eq!(xp_in_group_rate(40), 0.01);
    }

    #[test]
    fn kill_rates_apply_in_fork_order() {
        let info = CreatureInfo {
            creature_type: 1,
            xp_multiplier: 1.0,
            health_multiplier: 1.0,
            ..CreatureInfo::default()
        };
        let prod = crate::map::XpRates {
            kill: 1.4,
            kill_elite: 1.0,
            quest: 1.4,
            explore: 1.4,
        };
        // Same level: base 95; production rate -> 133.
        assert_eq!(creature_xp_gain(10, 10, &info, false, false, prod), 133);
        assert_eq!(
            creature_xp_gain(10, 10, &info, false, false, crate::map::XpRates::default()),
            95
        );
        // Elite (rank 1) outdoors: 95 * 2 * elite_rate,
        // then xp_multiplier, then kill rate.
        let elite = CreatureInfo {
            rank: 1,
            ..info.clone()
        };
        let elite_rates = crate::map::XpRates {
            kill: 1.4,
            kill_elite: 2.0,
            quest: 1.0,
            explore: 1.0,
        };
        assert_eq!(
            creature_xp_gain(10, 10, &elite, false, false, elite_rates),
            532
        );
        // In a non-raid dungeon the elite factor is 2.5.
        assert_eq!(
            creature_xp_gain(10, 10, &elite, false, true, elite_rates),
            665
        );
    }

    #[test]
    fn exploration_xp_branches() {
        // In band (|diff| <= 5): BaseXP(area) * rate.
        assert_eq!(exploration_xp(20, 18, 100, 999, 1.4), 140);
        assert_eq!(exploration_xp(20, 25, 100, 999, 1.0), 100);
        // diff < -5: BaseXP(level + 5) * rate.
        assert_eq!(exploration_xp(20, 26, 100, 160, 1.4), 224);
        // diff > 5: (100 - (diff-5)*5)% of BaseXP(area),
        // integer math before the rate multiply.
        assert_eq!(exploration_xp(26, 20, 100, 999, 1.0), 95);
        assert_eq!(exploration_xp(35, 20, 110, 999, 1.4), 77);
        // diff >= 25 clamps to 0%.
        assert_eq!(exploration_xp(45, 20, 110, 999, 1.4), 0);
        assert_eq!(exploration_xp(60, 20, 110, 999, 1.4), 0);
    }
}
