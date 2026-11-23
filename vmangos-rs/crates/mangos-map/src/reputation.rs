//! Reputation on creature kill: `Player::RewardReputation`
//! (`Player.cpp:6454-6520`) with `CalculateReputationGain`
//! (`Player.cpp:6346-6451`, KILL arm only), the
//! `ReputationMgr::SetReputation` / `SetOneFactionReputation`
//! state machine (`ReputationMgr.cpp:228-300`), and the
//! solo/group recipient sets (`Player.cpp:19712-19737`,
//! `Group.cpp:2238-2310`).
//!
//! Fork deltas, all named: quest/spell sources have no
//! runtime (this is the kill-only function, no source
//! enum); AV PvP-kill 729/730 awards are not creature
//! kills and stay out; `SMSG_SET_FACTION_ATWAR` is never
//! emitted here (its only fork senders are
//! `Creature.cpp:3517` and `Player.cpp:22439`); the stored
//! inactive bit keeps the pre-existing `0x04` projection
//! (fork `0x20` mismatch is a recorded follow-up, not this
//! slice).

use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::reputations::{
    FACTION_DB_ATWAR, FACTION_DB_VISIBLE, FACTION_FLAG_AT_WAR, FACTION_FLAG_HIDDEN,
    FACTION_FLAG_INVISIBLE_FORCED, FACTION_FLAG_PEACE_FORCED, FACTION_FLAG_VISIBLE,
    MAX_REPUTATION_RANK, ReputationOnKillEntry,
};

use crate::map::{Map, MapEvent};

/// `Reputation_Cap` (`ReputationMgr.h:72`).
pub const REPUTATION_CAP: i32 = 42999;
/// `Reputation_Bottom` (`ReputationMgr.h:73`).
pub const REPUTATION_BOTTOM: i32 = -42000;
/// `PointsInRank` (`ReputationMgr.cpp:29`).
pub const POINTS_IN_RANK: [i32; MAX_REPUTATION_RANK as usize] =
    [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];

/// `WOW_PATCH_110` (`Progression.h.in:74`): pets stop
/// modifying reputation at this patch
/// (`RewardReputation`, `Player.cpp:6461`).
pub const WOW_PATCH_110: u32 = 8;

/// `REP_HOSTILE` (`SharedDefines.h:118`).
pub const REP_HOSTILE: u32 = 1;
/// `REP_HATED` (`SharedDefines.h:117`).
pub const REP_HATED: u32 = 0;

/// `ReputationToRank` (`ReputationMgr.cpp:31-41`).
#[must_use]
pub fn reputation_to_rank(standing: i32) -> u32 {
    let mut limit = REPUTATION_CAP + 1;
    for rank in (0..MAX_REPUTATION_RANK).rev() {
        limit -= POINTS_IN_RANK[rank as usize];
        if standing >= limit {
            return rank;
        }
    }
    REP_HATED
}

/// Faction ids fit `i32` misc values on the bench
/// (mirrors the fork's `int32` comparison).
#[allow(clippy::cast_possible_wrap)]
fn faction_as_misc(faction: u32) -> i32 {
    faction as i32
}

/// Fork `round_float` (`Util.h:129-141`) reduced to a
/// deterministic rule: exact `.5` ties round up here,
/// while the fork flips a coin on them.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn round_reputation_gain(value: f32) -> i32 {
    (value + 0.5).floor() as i32
}

/// The KILL arm of `CalculateReputationGain`
/// (`Player.cpp:6346-6355`, `:6377-6382`, `:6420-6451`):
/// the quest-only `diffLvl` ladder never applies to
/// kills, so `victim_level` only feeds the gray arm.
/// `aura_bonus` is the signed `156` sum plus the exact
/// `190` faction match; `creature_rate` is `None` when
/// the faction has no `reputation_reward_rate` row (no
/// multiplier), and a present rate `<= 0.0` disables the
/// gain entirely.
#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
#[must_use]
pub fn calculate_kill_reputation_gain(
    repvalue: i32,
    victim_level: u32,
    player_level: u32,
    aura_bonus: i32,
    lowlevel_rate: f32,
    gain_rate: f32,
    creature_rate: Option<f32>,
) -> i32 {
    let aura = aura_bonus as f32;
    let mut percent = 100.0;
    percent += if repvalue > 0 { aura } else { -aura };
    // The fork compares the configured rate exactly
    // (`rate != 1.0f`, `Player.cpp:6420`).
    #[allow(clippy::float_cmp)]
    if lowlevel_rate != 1.0 && victim_level <= crate::xp::gray_level(player_level) {
        percent *= lowlevel_rate;
    }
    if percent <= 0.0 {
        return 0;
    }
    if let Some(creature_rate) = creature_rate {
        if creature_rate <= 0.0 {
            return 0;
        }
        percent *= creature_rate;
    }
    let scaled = gain_rate * repvalue as f32 * percent / 100.0;
    round_reputation_gain(scaled)
}

/// `SMSG_SET_FACTION_STANDING` (`SendState`,
/// `ReputationMgr.cpp:129-157`): count followed by one
/// `(repListID, standing)` pair per changed faction, the
/// changed (main) faction first.
#[must_use]
pub fn build_set_faction_standing(changes: &[(u32, i32)]) -> WorldPacket {
    let mut writer = PacketWriter::new();
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(changes.len() as u32);
    for &(rep_list_id, standing) in changes {
        writer.write_u32_le(rep_list_id);
        writer.write_u32_le(standing as u32);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_SET_FACTION_STANDING);
    packet.take_writer(writer);
    packet
}

/// `SMSG_SET_FACTION_VISIBLE` (`SendVisible`,
/// `ReputationMgr.cpp:194-203`): the newly visible
/// `repListID` only.
#[must_use]
pub fn build_set_faction_visible(rep_list_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(rep_list_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_SET_FACTION_VISIBLE);
    packet.take_writer(writer);
    packet
}

/// One faction's applied standing change, in fork
/// `SendState` order (main faction first, spillover
/// factions after in `repListID` order).
pub struct ReputationChange {
    /// Faction db id (deferred-row key).
    pub faction_id: u32,
    /// Client list id (`>= 0` always here).
    pub rep_list_id: u32,
    /// New stored standing (delta, as saved).
    pub standing: i32,
    /// Newly visible this kill (`SendVisible` fired).
    pub newly_visible: bool,
}

impl Map {
    /// `GetTotalAuraModifier(SPELL_AURA_MOD_REPUTATION_GAIN)`
    /// plus the kill-only
    /// `GetTotalAuraModifierByMiscValue(SPELL_AURA_MOD_FACTION_REPUTATION_GAIN,
    /// faction)` (`Player.cpp:6348-6354`): the 156 sum
    /// covers every aura of that type, while 190 matches
    /// by exact `misc_value` equality
    /// (`Unit.cpp:3292-3302`), not the mask reader.
    #[must_use]
    pub(crate) fn reputation_aura_bonus(&self, guid: ObjectGuid, faction: u32) -> i32 {
        let faction_misc = faction_as_misc(faction);
        let mut bonus = 0;
        for holder in self.aura_holders(guid) {
            for effect in holder.effects.iter().flatten() {
                if faction_bonus_matches(effect.aura_type, effect.misc_value, faction_misc) {
                    bonus += effect.amount;
                }
            }
        }
        bonus
    }

    /// The reputation arm of `Unit::Kill` for creature
    /// victims: `RewardGroupAtKill` /
    /// `RewardSinglePlayerAtKill` call `RewardReputation`
    /// only when loot is enabled (`Unit.cpp:996-1068`,
    /// `allowLoot`), so disabled maps return before any
    /// recipient work. XP and quest credit keep their
    /// pre-existing (ungated) behavior.
    pub(crate) fn reward_kill_reputation(
        &mut self,
        victim: ObjectGuid,
        killer: Option<ObjectGuid>,
    ) {
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            return;
        }
        // `RewardReputation` pet gate (`Player.cpp:6461`).
        if self.is_pet(victim) && self.config.wow_patch >= WOW_PATCH_110 {
            return;
        }
        let Some(tap) = self.reputation_tap_player(killer) else {
            return;
        };
        // PvP victims (player-owned creatures) grant no
        // reputation, mirroring the `reward_kill_xp` gate.
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
        let Some((victim_entry, victim_level, victim_info)) = self
            .creatures
            .get(&victim)
            .map(|c| (c.info.entry, c.unit.level(), c.info.clone()))
        else {
            return;
        };
        let Some(on_kill) = self.stores.reputation_kills.get_on_kill(victim_entry) else {
            return;
        };
        let recipients = self.kill_reputation_recipients(victim, tap, &victim_info);
        if recipients.is_empty() {
            return;
        }
        let rates = self.config.reputation_rates;
        for recipient in recipients {
            self.reward_reputation_to(recipient, victim_level, &on_kill, rates);
        }
    }

    /// `GetCharmerOrOwnerPlayerOrPlayerItself` over the
    /// modeled owner chain (direct summoner), shared with
    /// `reward_kill_xp` and `quest_kill_credit`.
    fn reputation_tap_player(&self, killer: Option<ObjectGuid>) -> Option<ObjectGuid> {
        let killer = killer?;
        if killer.is_player() && self.players.contains_key(&killer) {
            return Some(killer);
        }
        if let Some(summoner) = self
            .summons
            .get(&killer)
            .map(|st| st.summoner)
            .filter(|g| g.is_player() && self.players.contains_key(g))
        {
            return Some(summoner);
        }
        self.pet_owner(killer)
            .filter(|g| g.is_player() && self.players.contains_key(g))
    }

    /// Fork `RewardGroupAtKill` recipient set
    /// (`Group.cpp:2295-2310`): every same-group
    /// same-map member in reward distance, alive or dead,
    /// at full rate, plus the tap player last when in
    /// distance. Nothing runs without at least one alive
    /// in-distance member or an in-distance tap
    /// (`member_with_max_level`, `GetDataForXPAtKill`
    /// counts alive members plus the tap).
    fn kill_reputation_recipients(
        &self,
        victim: ObjectGuid,
        tap: ObjectGuid,
        victim_info: &mangos_world_data::creature_template::CreatureInfo,
    ) -> Vec<ObjectGuid> {
        let group_id = self
            .players
            .get(&tap)
            .and_then(|entry| entry.player.group_id);
        let Some(group_id) = group_id else {
            return vec![tap];
        };
        let victim_pos = self.unit_position(victim).unwrap_or_default();
        let mut recipients: Vec<ObjectGuid> = Vec::new();
        let mut alive_in_distance = false;
        for (guid, entry) in &self.players {
            if entry.player.group_id != Some(group_id) || *guid == tap {
                continue;
            }
            let Some(pos) = self.unit_position(*guid) else {
                continue;
            };
            if !self.at_group_reward_distance(pos, victim_pos, victim_info) {
                continue;
            }
            if entry.player.unit.is_alive() {
                alive_in_distance = true;
            }
            recipients.push(*guid);
        }
        // The tap player, last (`Group.cpp:2347-2352`).
        let tap_in_distance = self
            .unit_position(tap)
            .is_some_and(|pos| self.at_group_reward_distance(pos, victim_pos, victim_info));
        if tap_in_distance {
            if self
                .players
                .get(&tap)
                .is_some_and(|entry| entry.player.unit.is_alive())
            {
                alive_in_distance = true;
            }
            recipients.push(tap);
        }
        if !alive_in_distance && !tap_in_distance {
            return Vec::new();
        }
        recipients
    }

    /// One recipient's `RewardReputation` body
    /// (`Player.cpp:6454-6520`): both faction arms at the
    /// group rate `1.0f`, then each arm's team-award half.
    /// The team-award arm is a separate `if`, so it fires
    /// even when the main arm is cap-blocked.
    fn reward_reputation_to(
        &mut self,
        player: ObjectGuid,
        victim_level: u32,
        on_kill: &ReputationOnKillEntry,
        rates: crate::map::ReputationRates,
    ) {
        let Some((player_level, race_mask, class_mask, team)) =
            self.players.get(&player).map(|entry| {
                use mangos_world_data::reputations::{class_mask, race_mask};
                let race = entry.player.unit.race();
                let class = entry.player.unit.class();
                (
                    entry.player.unit.level(),
                    race_mask(race),
                    class_mask(class),
                    entry.player.team,
                )
            })
        else {
            return;
        };
        let arms = [
            (
                on_kill.repfaction1,
                on_kill.repvalue1,
                on_kill.reputation_max_cap1,
                on_kill.is_teamaward1,
                Team::Alliance,
            ),
            (
                on_kill.repfaction2,
                on_kill.repvalue2,
                on_kill.reputation_max_cap2,
                on_kill.is_teamaward2,
                Team::Horde,
            ),
        ];
        for (faction, repvalue, max_cap, is_teamaward, arm_team) in arms {
            if faction == 0 || (on_kill.team_dependent && team != arm_team) {
                continue;
            }
            let aura = self.reputation_aura_bonus(player, faction);
            let creature_rate = self
                .stores
                .reputation_kills
                .get_reward_rate(faction)
                .map(|rate| rate.creature_rate);
            let done = calculate_kill_reputation_gain(
                repvalue,
                victim_level,
                player_level,
                aura,
                rates.lowlevel_kill,
                rates.gain,
                creature_rate,
            );
            // `doneRep = int32(doneRep * rate)` with the
            // kill/group rate `1.0f`.
            let faction_entry = self.stores.factions.get(faction);
            if let Some(entry) = faction_entry {
                if cap_passes(
                    self.reputation_total(player, &entry, race_mask, class_mask),
                    max_cap,
                ) {
                    self.modify_reputation(player, &entry, done, false);
                }
            }
            if faction_entry.is_some() && is_teamaward {
                let team_id = faction_entry.map_or(0, |entry| entry.team);
                if let Some(team_entry) = self.stores.factions.get(team_id) {
                    self.modify_reputation(player, &team_entry, done / 2, true);
                }
            }
        }
    }

    /// Total reputation (`GetReputation`: base plus the
    /// stored delta) for the cap-gate rank check.
    fn reputation_total(
        &self,
        player: ObjectGuid,
        entry: &mangos_world_data::reputations::FactionEntry,
        race_mask: u32,
        class_mask: u32,
    ) -> i32 {
        let base = entry
            .index_fit_to(race_mask, class_mask)
            .map_or(0, |index| entry.base_values[index]);
        let stored = self
            .players
            .get(&player)
            .and_then(|record| {
                record
                    .player
                    .deferred
                    .reputation
                    .iter()
                    .find(|row| row.faction == entry.id)
            })
            .map_or(0, |row| row.standing);
        base.saturating_add(stored)
    }

    /// `ModifyReputation` = `SetReputation` with
    /// `incremental` (`ReputationMgr.h:104`): spillover
    /// first unless suppressed, then the faction itself,
    /// then one batched `SendState` plus inline
    /// `SendVisible` packets (`ReputationMgr.cpp:228-260`).
    fn modify_reputation(
        &mut self,
        player: ObjectGuid,
        entry: &mangos_world_data::reputations::FactionEntry,
        amount: i32,
        no_spillover: bool,
    ) {
        if entry.reputation_index < 0 {
            return;
        }
        let (race_mask, class_mask) = self.players.get(&player).map_or((0, 0), |record| {
            use mangos_world_data::reputations::{class_mask, race_mask};
            (
                race_mask(record.player.unit.race()),
                class_mask(record.player.unit.class()),
            )
        });
        // Spillover arms (`ReputationMgr.cpp:230-247`),
        // applied straight through `SetOneFactionReputation`
        // (no recursion), each newly-visible spillover
        // emitting its `SendVisible` inline.
        let mut spill_changes: Vec<(u32, i32)> = Vec::new();
        if !no_spillover
            && let Some(template) = self.stores.reputation_kills.get_spillover(entry.id)
        {
            for slot in 0..mangos_world_data::reputations::MAX_SPILLOVER_FACTIONS {
                let spill_id = template.faction[slot];
                if spill_id == 0 {
                    continue;
                }
                let Some(spill_entry) = self.stores.factions.get(spill_id) else {
                    continue;
                };
                let total = self.reputation_total(player, &spill_entry, race_mask, class_mask);
                if reputation_to_rank(total) > template.faction_rank[slot] {
                    continue;
                }
                let spill_amount = spillover_amount(amount, template.faction_rate[slot]);
                if let Some(change) =
                    self.set_one_faction_reputation(player, &spill_entry, spill_amount)
                {
                    if change.newly_visible {
                        self.events.push(MapEvent::Packet {
                            to: player,
                            packet: build_set_faction_visible(change.rep_list_id),
                        });
                    }
                    spill_changes.push((change.rep_list_id, change.standing));
                }
            }
        }
        let Some(change) = self.set_one_faction_reputation(player, entry, amount) else {
            return;
        };
        if change.newly_visible {
            self.events.push(MapEvent::Packet {
                to: player,
                packet: build_set_faction_visible(change.rep_list_id),
            });
        }
        // One `SendState`: the changed faction first,
        // then pending spillover factions in `repListID`
        // order (the fork's `m_factions` map order).
        spill_changes.sort_unstable();
        spill_changes.retain(|(rep_list_id, _)| *rep_list_id != change.rep_list_id);
        let mut pairs = vec![(change.rep_list_id, change.standing)];
        pairs.extend(spill_changes);
        self.events.push(MapEvent::Packet {
            to: player,
            packet: build_set_faction_standing(&pairs),
        });
    }

    /// `SetOneFactionReputation` (`ReputationMgr.cpp:262-291`)
    /// over the deferred rows: clamp, visible-once, and
    /// the hostile at-war transition. Returns `None` for
    /// factions with no client list id (the fork finds no
    /// `FactionState` and reports nothing).
    fn set_one_faction_reputation(
        &mut self,
        player: ObjectGuid,
        entry: &mangos_world_data::reputations::FactionEntry,
        amount: i32,
    ) -> Option<ReputationChange> {
        if entry.reputation_index < 0 {
            return None;
        }
        let (race_mask, class_mask) = self.players.get(&player).map_or((0, 0), |record| {
            use mangos_world_data::reputations::{class_mask, race_mask};
            (
                race_mask(record.player.unit.race()),
                class_mask(record.player.unit.class()),
            )
        });
        let base = entry
            .index_fit_to(race_mask, class_mask)
            .map_or(0, |index| entry.base_values[index]);
        let old_row = self
            .players
            .get(&player)
            .and_then(|record| {
                record
                    .player
                    .deferred
                    .reputation
                    .iter()
                    .find(|row| row.faction == entry.id)
            })
            .cloned();
        let current = mangos_world_data::reputations::runtime_state(
            entry,
            race_mask,
            class_mask,
            old_row.as_ref(),
        );
        let (new_stored, new_flags, newly_visible) = apply_standing_change(current, base, amount);
        let old_db = old_row.map_or(0, |row| row.flags);
        let new_db = (old_db & !(FACTION_DB_VISIBLE | FACTION_DB_ATWAR))
            | (i32::from(new_flags & FACTION_FLAG_VISIBLE != 0) * FACTION_DB_VISIBLE)
            | (i32::from(new_flags & FACTION_FLAG_AT_WAR != 0) * FACTION_DB_ATWAR);
        if let Some(record) = self.players.get_mut(&player) {
            match record
                .player
                .deferred
                .reputation
                .iter_mut()
                .find(|row| row.faction == entry.id)
            {
                Some(row) => {
                    row.standing = new_stored;
                    row.flags = new_db;
                }
                None => {
                    record.player.deferred.reputation.push(
                        mangos_db::character_load::ReputationRow {
                            faction: entry.id,
                            standing: new_stored,
                            flags: new_db,
                        },
                    );
                }
            }
        }
        Some(ReputationChange {
            faction_id: entry.id,
            #[allow(clippy::cast_sign_loss)]
            rep_list_id: entry.reputation_index as u32,
            standing: new_stored,
            newly_visible,
        })
    }
}

/// One aura effect's contribution to the kill percent
/// (`Player.cpp:6348-6354`): every 156 aura applies,
/// while 190 applies only on exact `misc_value` equality
/// with the rewarded faction (`Unit.cpp:3292-3302`),
/// unlike the mask reader.
#[must_use]
pub fn faction_bonus_matches(aura_type: u32, misc_value: i32, faction_misc: i32) -> bool {
    use mangos_spells::defines::aura::{MOD_FACTION_REPUTATION_GAIN, MOD_REPUTATION_GAIN};
    aura_type == MOD_REPUTATION_GAIN
        || (aura_type == MOD_FACTION_REPUTATION_GAIN && misc_value == faction_misc)
}

/// The 1.9+ `reputation_max_cap` arm
/// (`Player.cpp:6483`): the kill grants only while the
/// current rank is at or below the cap.
#[must_use]
pub fn cap_passes(current_total: i32, max_cap: u32) -> bool {
    reputation_to_rank(current_total) <= max_cap
}

/// `RepSpilloverTemplate` application
/// (`ReputationMgr.cpp:242`): `standing * rate`,
/// truncated toward zero like the fork's float-to-int
/// conversion.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn spillover_amount(standing_gain: i32, rate: f32) -> i32 {
    #[allow(clippy::cast_precision_loss)]
    let scaled = standing_gain as f32 * rate;
    scaled as i32
}

/// Pure `SetOneFactionReputation` transition
/// (`ReputationMgr.cpp:262-291`): incremental total
/// clamped to `[Bottom, Cap]`, stored back minus base,
/// visible-once unless forced-invisible/hidden, and the
/// at-war transition on hostile totals (with the fork's
/// `PEACE_FORCED` refusal keyed on the stored delta).
#[must_use]
pub fn apply_standing_change(current: (u32, i32), base: i32, amount: i32) -> (i32, u32, bool) {
    let (mut flags, stored) = current;
    let total = stored
        .saturating_add(base)
        .saturating_add(amount)
        .clamp(REPUTATION_BOTTOM, REPUTATION_CAP);
    let new_stored = total - base;
    let mut newly_visible = false;
    // `SetVisible` (`ReputationMgr.cpp:315-327`).
    if flags & (FACTION_FLAG_INVISIBLE_FORCED | FACTION_FLAG_HIDDEN) == 0
        && flags & FACTION_FLAG_VISIBLE == 0
    {
        flags |= FACTION_FLAG_VISIBLE;
        newly_visible = true;
    }
    // `SetAtWar(&faction, true)` on hostile totals
    // (`ReputationMgr.cpp:284-285`, `:345-359`).
    if reputation_to_rank(total) <= REP_HOSTILE
        && flags & FACTION_FLAG_AT_WAR == 0
        && (flags & FACTION_FLAG_PEACE_FORCED == 0 || reputation_to_rank(new_stored) == REP_HATED)
    {
        flags |= FACTION_FLAG_AT_WAR;
    }
    (new_stored, flags, newly_visible)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn rank_boundaries_match_points_in_rank() {
        // `PointsInRank` limits, from the bottom.
        assert_eq!(reputation_to_rank(REPUTATION_BOTTOM), REP_HATED);
        assert_eq!(reputation_to_rank(-41_999), REP_HATED);
        assert_eq!(reputation_to_rank(-6001), REP_HATED);
        assert_eq!(reputation_to_rank(-6000), 1);
        assert_eq!(reputation_to_rank(-3001), 1);
        assert_eq!(reputation_to_rank(-3000), 2);
        assert_eq!(reputation_to_rank(-1), 2);
        assert_eq!(reputation_to_rank(0), 3);
        assert_eq!(reputation_to_rank(2999), 3);
        assert_eq!(reputation_to_rank(3000), 4);
        assert_eq!(reputation_to_rank(8999), 4);
        assert_eq!(reputation_to_rank(9000), 5);
        assert_eq!(reputation_to_rank(20_999), 5);
        assert_eq!(reputation_to_rank(21_000), 6);
        assert_eq!(reputation_to_rank(41_999), 6);
        assert_eq!(reputation_to_rank(42_000), 7);
        assert_eq!(reputation_to_rank(REPUTATION_CAP), 7);
    }

    #[test]
    fn kill_gain_defaults_to_full_rate() {
        // Aura-free kill at default rates: the raw value.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, 0, 1.0, 1.0, None),
            25
        );
        // A present-but-zero creature rate disables the gain.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, 0, 1.0, 1.0, Some(0.0)),
            0
        );
        // A present rate multiplies.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, 0, 1.0, 1.0, Some(2.0)),
            50
        );
    }

    #[test]
    fn kill_gain_ignores_quest_level_ladder() {
        // A kill far below the player keeps 100%: the
        // `diffLvl` ladder (`Player.cpp:6397-6418`) is
        // quest-only and the kill arm leaves it at 0.
        assert_eq!(
            calculate_kill_reputation_gain(25, 6, 10, 0, 1.0, 1.0, None),
            25
        );
        // Gray victims only scale when the low-level
        // kill rate differs from 1.0.
        assert_eq!(
            calculate_kill_reputation_gain(25, 1, 10, 0, 0.2, 1.0, None),
            5
        );
        assert_eq!(
            calculate_kill_reputation_gain(25, 1, 10, 0, 1.0, 1.0, None),
            25
        );
    }

    #[test]
    fn kill_gain_applies_aura_percent_with_sign_flip() {
        // +10% on a positive reward: 25 * 1.1 = 27.5 -> 28.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, 10, 1.0, 1.0, None),
            28
        );
        // The same aura subtracts on a negative reward.
        assert_eq!(
            calculate_kill_reputation_gain(-25, 10, 10, 10, 1.0, 1.0, None),
            -22
        );
        // A fully cancelled percent grants nothing.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, -100, 1.0, 1.0, None),
            0
        );
        // The global gain rate scales last.
        assert_eq!(
            calculate_kill_reputation_gain(25, 10, 10, 0, 1.0, 2.0, None),
            50
        );
    }

    #[test]
    fn aura_match_is_exact_for_190_and_open_for_156() {
        use mangos_spells::defines::aura::{MOD_FACTION_REPUTATION_GAIN, MOD_REPUTATION_GAIN};
        // 156 sums unfiltered by misc value.
        assert!(faction_bonus_matches(MOD_REPUTATION_GAIN, 0, 72));
        assert!(faction_bonus_matches(MOD_REPUTATION_GAIN, 999, 72));
        // 190 matches only the exact rewarded faction.
        assert!(faction_bonus_matches(MOD_FACTION_REPUTATION_GAIN, 72, 72));
        assert!(!faction_bonus_matches(MOD_FACTION_REPUTATION_GAIN, 76, 72));
        // A bit-overlapping but unequal id is not equality.
        assert!(!faction_bonus_matches(MOD_FACTION_REPUTATION_GAIN, 74, 72));
        assert!(!faction_bonus_matches(13, 72, 72));
    }

    #[test]
    fn cap_gate_blocks_above_cap_rank() {
        // Exalted total passes a cap of Exalted, not Honored.
        assert!(cap_passes(42_999, 7));
        assert!(!cap_passes(42_999, 5));
        assert!(cap_passes(0, 3));
        assert!(!cap_passes(0, 2));
    }

    #[test]
    fn spillover_truncates_toward_zero() {
        assert_eq!(spillover_amount(25, 0.5), 12);
        assert_eq!(spillover_amount(25, 1.0), 25);
        assert_eq!(spillover_amount(-25, 0.5), -12);
    }

    #[test]
    fn standing_change_clamps_and_offsets_base() {
        // Top clamp: the total sticks at the cap.
        assert_eq!(
            apply_standing_change((FACTION_FLAG_VISIBLE, 42_000), 0, 5000),
            (REPUTATION_CAP, FACTION_FLAG_VISIBLE, false)
        );
        // Bottom clamp (a bottomed total is hated, so
        // the at-war flag rises with the clamp).
        assert_eq!(
            apply_standing_change((FACTION_FLAG_VISIBLE, -41_000), 0, -5000),
            (
                REPUTATION_BOTTOM,
                FACTION_FLAG_VISIBLE | FACTION_FLAG_AT_WAR,
                false
            )
        );
        // Base offsets the stored delta both ways.
        assert_eq!(
            apply_standing_change((0, 0), 3000, 100),
            (100, FACTION_FLAG_VISIBLE, true)
        );
    }

    #[test]
    fn standing_change_visible_once_and_hostile_war() {
        // First touch makes the faction visible.
        let (stored, flags, visible) = apply_standing_change((0, 0), 0, 10);
        assert_eq!((stored, visible), (10, true));
        assert_ne!(flags & FACTION_FLAG_VISIBLE, 0);
        // A second touch reports no new visibility.
        let (_, _, visible) = apply_standing_change((flags, stored), 0, 10);
        assert!(!visible);
        // Forced-invisible factions never become visible.
        let (_, flags, visible) = apply_standing_change((FACTION_FLAG_INVISIBLE_FORCED, 0), 0, 10);
        assert!(!visible);
        assert_eq!(flags & FACTION_FLAG_VISIBLE, 0);
        // Hidden defaults block visibility the same way.
        let (_, flags, visible) = apply_standing_change((FACTION_FLAG_HIDDEN, 0), 0, 10);
        assert!(!visible);
        assert_eq!(flags & FACTION_FLAG_VISIBLE, 0);
        // Hostile totals raise the at-war flag.
        let (_, flags, _) = apply_standing_change((FACTION_FLAG_VISIBLE, 0), 0, -6000);
        assert_ne!(flags & FACTION_FLAG_AT_WAR, 0);
    }

    #[test]
    fn standing_change_peace_forced_refuses_on_stored_delta() {
        // Peace-forced with a non-hated stored delta
        // refuses war even on a hostile total.
        let (_, flags, _) = apply_standing_change(
            (FACTION_FLAG_VISIBLE | FACTION_FLAG_PEACE_FORCED, 0),
            0,
            -6000,
        );
        assert_eq!(flags & FACTION_FLAG_AT_WAR, 0);
        // A hated stored delta lifts the refusal.
        let (_, flags, _) = apply_standing_change(
            (FACTION_FLAG_VISIBLE | FACTION_FLAG_PEACE_FORCED, -7000),
            0,
            500,
        );
        assert_ne!(flags & FACTION_FLAG_AT_WAR, 0);
    }

    #[test]
    fn standing_packet_bytes_match_send_state() {
        let packet = build_set_faction_standing(&[(6, 125), (8, -101)]);
        assert_eq!(packet.opcode(), opcode_id::SMSG_SET_FACTION_STANDING);
        let mut expected = 2u32.to_le_bytes().to_vec();
        expected.extend(6u32.to_le_bytes());
        expected.extend(125i32.to_le_bytes());
        expected.extend(8u32.to_le_bytes());
        expected.extend((-101i32).to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn visible_packet_bytes_match_send_visible() {
        let packet = build_set_faction_visible(6);
        assert_eq!(packet.opcode(), opcode_id::SMSG_SET_FACTION_VISIBLE);
        assert_eq!(packet.body(), 6u32.to_le_bytes());
    }
}
