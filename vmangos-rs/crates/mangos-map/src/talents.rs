//! Talent learn/reset (Tier 4 step 4g), transcribed
//! from the fork:
//! - `Player::LearnTalent` (`Player.cpp:20429`)
//! - `Player::ResetTalents` + cost curve
//!   (`Player.cpp:4076-4210`)
//! - `UpdateFreeTalentPoints` / `InitTalentForLevel`
//!   (`Player.cpp:3258-3290`)
//! - The talent/profession bookkeeping arms of
//!   `Player::AddSpell` / `RemoveSpell`
//!   (`Player.cpp:3570-3795`, `3880-3935`) in their
//!   reachable subset: talent rank replacement,
//!   used-point counting, primary-profession point
//!   adjust, passive-like cast on learn.
//!
//! Recorded remainders:
//! - UNMODELED: non-talent spellbook supersede on
//!   learn (rank chains learned via trainer keep
//!   every rank row; the fork deactivates replaced
//!   ranks and sends `SMSG_SUPERCEDED_SPELL`).
//! - UNMODELED: `spell_learn_spell` dependent-spell
//!   autolearn and `UpdateSpellTrainedSkills` on
//!   learn/remove (no dependent-spell store yet).
//! - UNMODELED: pet reagent return on the reset
//!   unsummon (as in `taxi.rs`).

use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::{aura as aura_type, effect};
use mangos_spells::entry::MAX_TALENT_RANK;

use crate::map::Map;
use crate::spells::{CastOptions, CastTarget};

/// `GOLD` (10000 copper).
const GOLD: u64 = 10_000;
/// `MONTH` in seconds (`Common.h:131`).
const MONTH_SECS: u64 = 30 * 24 * 60 * 60;
/// `SPELL_ATTR_PASSIVE`.
const ATTR_PASSIVE: u32 = 0x40;
/// `SPELL_ATTR_EX_CAST_WHEN_LEARNED` is part of the
/// passive-like check; stances/aura-state sub-gates
/// collapse to the passive bit in this model (the
/// reachable talent set at a trainer desk).
/// `BUY_ERR_NOT_ENOUGHT_MONEY`.
const BUY_ERR_NOT_ENOUGHT_MONEY: u8 = 1;
/// `CHARACTER_FLAG_RESET_TALENTS_ON_LOGIN`.
pub const CHARACTER_FLAG_RESET_TALENTS_ON_LOGIN: u32 = 0x0000_0100;

impl Map {
    fn free_talent_points(&self, guid: ObjectGuid) -> u32 {
        self.unit_base(guid).map_or(0, |u| {
            u.world
                .object
                .values()
                .get_u32(offsets::PLAYER_CHARACTER_POINTS1)
        })
    }

    fn set_free_talent_points(&mut self, guid: ObjectGuid, points: u32) {
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values()
                .set_u32(offsets::PLAYER_CHARACTER_POINTS1, points);
        }
    }

    /// `Player::UpdateFreeTalentPoints`.
    pub(crate) fn update_free_talent_points(&mut self, guid: ObjectGuid, reset_if_need: bool) {
        let level = self.unit_level(guid);
        let used = self
            .players
            .get(&guid)
            .map_or(0, |e| e.player.used_talent_count);
        if level < 10 {
            if used > 0 {
                if reset_if_need {
                    self.reset_talents(guid, true);
                }
                self.set_free_talent_points(guid, 0);
            }
            return;
        }
        let for_level = mangos_entities::player::talent_points_for_level(level);
        if used > for_level {
            // Fork resets only for non-admin sessions;
            // security is actor-side and every live
            // session here is a normal player session.
            if reset_if_need {
                self.reset_talents(guid, true);
            } else {
                self.set_free_talent_points(guid, 0);
            }
        } else {
            self.set_free_talent_points(guid, for_level - used);
        }
    }

    /// `Player::InitTalentForLevel` with the load-time
    /// used-point recount (`_LoadSpells` `AddSpell`
    /// counting).
    pub fn init_talent_for_level(&mut self, guid: ObjectGuid) {
        let used: u32 = self.players.get(&guid).map_or(0, |e| {
            e.player
                .deferred
                .spells
                .iter()
                .filter(|row| row.disabled == 0)
                .map(|row| self.stores.spells.talent_spell_cost(row.spell))
                .sum()
        });
        // Talent rank replacement at learn keeps only
        // the highest rank per talent, but a saved
        // book can hold several ranks (fork AddSpell
        // counts each known row the same way before
        // supersede deactivation; the recount matches
        // the fork's settled state because the fork
        // removes other ranks when one is added).
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.used_talent_count = used;
        }
        self.update_free_talent_points(guid, true);
    }

    /// Primary-profession free-point adjust shared by
    /// learn/remove (`AddSpell`/`RemoveSpell` arms).
    fn adjust_profession_points(&mut self, guid: ObjectGuid, spell_id: u32, learned: bool) {
        let is_first_rank = self
            .stores
            .spells
            .profession_skill_of_spell(spell_id)
            .is_some_and(|skill| self.stores.skills.category(skill) == Some(11))
            && self.stores.spells.spell_rank(spell_id) == 1;
        if !is_first_rank {
            return;
        }
        let current = self.unit_base(guid).map_or(0, |u| {
            u.world
                .object
                .values()
                .get_u32(offsets::PLAYER_CHARACTER_POINTS2)
        });
        let next = if learned {
            current.saturating_sub(1)
        } else {
            // Fork caps at MAX_PRIMARY_TRADE_SKILL (2).
            (current + 1).min(2)
        };
        if learned && current == 0 {
            return; // fork: `if (freeProfs)` guard
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values()
                .set_u32(offsets::PLAYER_CHARACTER_POINTS2, next);
        }
    }

    /// `IsNeedCastPassiveLikeSpellAtLearn` reachable
    /// subset: passive spells (the entry subset has
    /// no stance mask, so stance-gated form passives
    /// — handled by shapeshift boosts in the fork —
    /// are approximated as castable).
    fn need_cast_passive_at_learn(&self, spell_id: u32) -> bool {
        self.stores
            .spells
            .get(spell_id)
            .is_some_and(|spell| spell.attributes & ATTR_PASSIVE != 0)
    }

    /// The map-side `LearnSpell` (packet + `AddSpell`
    /// bookkeeping subset). Returns true when the
    /// spell was newly learned (fork `learning`).
    pub(crate) fn learn_spell_full(&mut self, guid: ObjectGuid, spell_id: u32) -> bool {
        if self.player_knows_spell(guid, spell_id) {
            return false;
        }
        // Talent: unlearn all other ranks first.
        if let Some(pos) = self.stores.spells.talent_spell_pos(spell_id)
            && let Some(talent) = self.stores.spells.talents.get(&pos.talent_id).copied()
        {
            for rank_spell in talent.rank_spells {
                if rank_spell != 0 && rank_spell != spell_id {
                    self.remove_spell_full(guid, rank_spell);
                }
            }
        }
        let fresh = self
            .players
            .get_mut(&guid)
            .is_some_and(|entry| entry.player.learn_spell(spell_id));
        if !fresh {
            return false;
        }
        let mut writer = PacketWriter::new();
        writer.write_u32_le(spell_id);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LEARNED_SPELL, writer.into_vec());
        self.send_to_player(guid, packet);

        if let Some(pos) = self.stores.spells.talent_spell_pos(spell_id) {
            let _ = pos;
            let cost = self.stores.spells.talent_spell_cost(spell_id);
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.player.used_talent_count += cost;
            }
            self.update_free_talent_points(guid, false);
        }
        self.adjust_profession_points(guid, spell_id, true);

        let has_learn_effect = self
            .stores
            .spells
            .get(spell_id)
            .is_some_and(|s| s.effects.iter().any(|e| e.effect == effect::LEARN_SPELL));
        let has_skill_step = self
            .stores
            .spells
            .get(spell_id)
            .is_some_and(|s| s.effects.iter().any(|e| e.effect == 44));
        if has_learn_effect || has_skill_step || self.need_cast_passive_at_learn(spell_id) {
            let _ = self.cast_spell(
                guid,
                spell_id,
                CastTarget::SelfCast,
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

    /// The map-side `RemoveSpell` bookkeeping subset.
    /// Returns true when a known spell was removed.
    pub(crate) fn remove_spell_full(&mut self, guid: ObjectGuid, spell_id: u32) -> bool {
        let removed = self.players.get_mut(&guid).is_some_and(|entry| {
            let before = entry.player.deferred.spells.len();
            entry
                .player
                .deferred
                .spells
                .retain(|row| row.spell != spell_id);
            entry.player.deferred.spells.len() != before
        });
        if !removed {
            return false;
        }
        self.remove_auras_due_to_spell(guid, spell_id);
        let talent_cost = self.stores.spells.talent_spell_cost(spell_id);
        if talent_cost > 0 {
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.player.used_talent_count =
                    entry.player.used_talent_count.saturating_sub(talent_cost);
            }
            self.update_free_talent_points(guid, false);
        }
        self.adjust_profession_points(guid, spell_id, false);
        let mut writer = PacketWriter::new();
        writer.write_u16_le(u16::try_from(spell_id).unwrap_or(0));
        let packet = WorldPacket::with_body(opcode_id::SMSG_REMOVED_SPELL, writer.into_vec());
        self.send_to_player(guid, packet);
        true
    }

    /// `Player::LearnTalent`.
    pub fn learn_talent(&mut self, guid: ObjectGuid, talent_id: u32, talent_rank: u32) {
        let cur_points = self.free_talent_points(guid);
        if cur_points == 0 || talent_rank as usize >= MAX_TALENT_RANK {
            return;
        }
        let Some(talent) = self.stores.spells.talents.get(&talent_id).copied() else {
            return;
        };
        let Some(&tab_mask) = self.stores.spells.talent_tabs.get(&talent.tab) else {
            return;
        };
        let class_mask = self
            .unit_base(guid)
            .map_or(0, |u| 1u32 << u32::from(u.class().saturating_sub(1)));
        if class_mask & tab_mask == 0 {
            return;
        }
        let mut cur_max_rank = 0u32;
        for (k, &rank_spell) in talent.rank_spells.iter().enumerate().rev() {
            if rank_spell != 0 && self.player_knows_spell(guid, rank_spell) {
                cur_max_rank = u32::try_from(k).unwrap_or(0) + 1;
                break;
            }
        }
        if cur_max_rank > talent_rank {
            return;
        }
        if cur_points < talent_rank + 1 - cur_max_rank {
            return;
        }
        if talent.depends_on > 0
            && let Some(dep) = self.stores.spells.talents.get(&talent.depends_on).copied()
        {
            let has_enough = dep
                .rank_spells
                .iter()
                .skip(talent.depends_on_rank as usize)
                .any(|&s| s != 0 && self.player_knows_spell(guid, s));
            if !has_enough {
                return;
            }
        }
        if talent.depends_on_spell != 0 && !self.player_knows_spell(guid, talent.depends_on_spell) {
            return;
        }
        if talent.row > 0 {
            let mut spent: u32 = 0;
            for other in self.stores.spells.talents.values() {
                if other.tab != talent.tab {
                    continue;
                }
                for (j, &rank_spell) in other.rank_spells.iter().enumerate() {
                    if rank_spell != 0 && self.player_knows_spell(guid, rank_spell) {
                        spent += u32::try_from(j).unwrap_or(0) + 1;
                    }
                }
            }
            if spent < talent.row * u32::try_from(MAX_TALENT_RANK).unwrap_or(5) {
                return;
            }
        }
        let spell_id = talent.rank_spells[talent_rank as usize];
        if spell_id == 0 || self.player_knows_spell(guid, spell_id) {
            return;
        }
        self.learn_spell_full(guid, spell_id);
    }

    /// `Player::UpdateResetTalentsMultiplier` +
    /// `GetResetTalentsCost` (gold -> copper).
    fn reset_talents_cost(&mut self, guid: ObjectGuid) -> u32 {
        // Decay runs at patch 1.11+ regardless of the
        // config switch (fork condition); bench build
        // is 1.12.1.
        let now = self.now_unix;
        if let Some(entry) = self.players.get_mut(&guid) {
            let months = now.saturating_sub(entry.player.reset_talents_time) / MONTH_SECS;
            if months > 0 {
                let min_multi = self.config.respec.min_multiplier;
                let clamp = entry.player.reset_talents_multiplier >= min_multi;
                let months32 = u32::try_from(months).unwrap_or(u32::MAX);
                if months32 > entry.player.reset_talents_multiplier {
                    entry.player.reset_talents_multiplier = 0;
                } else {
                    entry.player.reset_talents_multiplier -= months32;
                }
                if clamp && entry.player.reset_talents_multiplier < min_multi {
                    entry.player.reset_talents_multiplier = min_multi;
                }
            }
        }
        let multiplier = self
            .players
            .get(&guid)
            .map_or(0, |e| e.player.reset_talents_multiplier);
        let gold = if multiplier == 0 {
            u64::from(self.config.respec.base_cost_gold)
        } else if multiplier > self.config.respec.max_multiplier {
            u64::from(self.config.respec.multiplicative_cost_gold)
                * u64::from(self.config.respec.max_multiplier)
        } else {
            u64::from(multiplier) * u64::from(self.config.respec.multiplicative_cost_gold)
        };
        u32::try_from(gold * GOLD).unwrap_or(u32::MAX)
    }

    /// `Player::SendTalentWipeConfirm` (gossip
    /// `GOSSIP_OPTION_UNLEARNTALENTS` response; the
    /// default-gossip menu that reaches it is a
    /// pre-existing gap).
    pub fn send_talent_wipe_confirm(&mut self, guid: ObjectGuid, npc: ObjectGuid) {
        let cost = self.reset_talents_cost(guid);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc.raw());
        writer.write_u32_le(cost);
        let packet = WorldPacket::with_body(opcode_id::MSG_TALENT_WIPE_CONFIRM, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `HandleTalentWipeConfirmOpcode`.
    pub fn handle_talent_wipe_confirm(&mut self, guid: ObjectGuid, npc: ObjectGuid) {
        if !self.npc_if_can_interact_with(guid, npc, crate::trainer::NPC_FLAG_TRAINER) {
            return;
        }
        if self
            .unit_base(guid)
            .is_some_and(|u| u.has_unit_state(mangos_entities::unit::UNIT_STATE_FEIGN_DEATH))
        {
            self.remove_spells_causing_aura(guid, aura_type::FEIGN_DEATH);
        }
        if !self.reset_talents(guid, false) {
            let mut writer = PacketWriter::new();
            writer.write_u64_le(0);
            writer.write_u32_le(0);
            let packet =
                WorldPacket::with_body(opcode_id::MSG_TALENT_WIPE_CONFIRM, writer.into_vec());
            self.send_to_player(guid, packet);
            return;
        }
        // Spell 14867 "Untalent Visual Effect", cast
        // by the trainer on the player.
        let _ = self.cast_spell(
            npc,
            14867,
            CastTarget::Unit(guid),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    /// `Player::ResetTalents`.
    pub fn reset_talents(&mut self, guid: ObjectGuid, no_cost: bool) -> bool {
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.character_flags &= !CHARACTER_FLAG_RESET_TALENTS_ON_LOGIN;
        }
        let used = self
            .players
            .get(&guid)
            .map_or(0, |e| e.player.used_talent_count);
        if used == 0 {
            self.update_free_talent_points(guid, false);
            return false;
        }
        let cost = if no_cost {
            0
        } else {
            self.reset_talents_cost(guid)
        };
        if !no_cost && self.player_money(guid) < cost {
            self.send_buy_error(guid, BUY_ERR_NOT_ENOUGHT_MONEY, None, 0);
            return false;
        }
        let class_mask = self
            .unit_base(guid)
            .map_or(0, |u| 1u32 << u32::from(u.class().saturating_sub(1)));
        let talents: Vec<mangos_spells::entry::TalentEntry> =
            self.stores.spells.talents.values().copied().collect();
        for talent in talents {
            let Some(&tab_mask) = self.stores.spells.talent_tabs.get(&talent.tab) else {
                continue;
            };
            if class_mask & tab_mask == 0 {
                continue;
            }
            for rank_spell in talent.rank_spells {
                if rank_spell == 0 {
                    continue;
                }
                if let Some(entry) = self.stores.spells.get(rank_spell).cloned() {
                    for eff in &entry.effects {
                        if eff.trigger_spell != 0 {
                            self.remove_auras_due_to_spell(guid, eff.trigger_spell);
                        }
                    }
                }
                self.remove_spell_full(guid, rank_spell);
            }
        }
        self.update_free_talent_points(guid, false);
        if !no_cost {
            self.modify_money(guid, -i64::from(cost));
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.player.reset_talents_multiplier += 1;
                let max_multi = self.config.respec.max_multiplier;
                if entry.player.reset_talents_multiplier > max_multi {
                    entry.player.reset_talents_multiplier = max_multi;
                }
                entry.player.reset_talents_time = self.now_unix;
            }
        }
        self.unsummon_owner_pet(guid, 100);
        true
    }
}
