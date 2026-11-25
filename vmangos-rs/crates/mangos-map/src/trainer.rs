//! Trainer NPC services (Tier 4 step 4e): list and
//! buy, transcribed from the fork:
//! - `HandleTrainerListOpcode` / `SendTrainerList` /
//!   `SendTrainerSpellHelper` (`NPCHandler.cpp:97-245`)
//! - `HandleTrainerBuySpellOpcode`
//!   (`NPCHandler.cpp:260-356`)
//! - `Creature::IsTrainerOf` (`Creature.cpp:1270-1407`)
//! - `Player::GetTrainerSpellState`
//!   (`Player.cpp:4281-4352`) and
//!   `Player::IsSpellFitByClassAndRace`
//!   (`Player.cpp:19308-19350`, train/check form)
//!
//! Learning runs through the normal cast path: the
//! wrapper spell's `SPELL_EFFECT_LEARN_SPELL` effect
//! teaches the trigger spell (`spells.rs`), and money
//! is charged only when the cast starts, mirroring
//! the fork's charge-on-`SPELL_CAST_OK` rule.
//!
//! Recorded remainders:
//! - UNMODELED: the buy path's `IsWithinLOSInMap`
//!   gate (no LOS helper on Map yet; the same
//!   `NEEDS-MAP-HELPER` recorded across content).
//! - UNMODELED: greeting locales beyond the default
//!   content (session locale is always default).

use mangos_entities::player::Player;
use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::{aura as aura_type, aura_interrupt_flag, effect};
use mangos_world_data::trainer_spells::{TrainerSpell, TrainerSpellData};

use crate::map::Map;
use crate::spells::{CastOptions, CastOutcome, CastTarget};

/// `UNIT_NPC_FLAG_TRAINER` (`UnitDefines.h`).
pub(crate) const NPC_FLAG_TRAINER: u32 = 0x0000_0010;
/// `SKILL_CATEGORY_PROFESSION` (`SharedDefines.h:1123`).
const SKILL_CATEGORY_PROFESSION: i32 = 11;
/// `SKILL_FLAG_NOT_TRAINABLE` (`DBCEnums.h:179`).
const SKILL_FLAG_NOT_TRAINABLE: u32 = 0x100;
// `TrainerSpellState` (`Player.h`).
const TRAINER_SPELL_GREEN: u8 = 0;
const TRAINER_SPELL_RED: u8 = 1;
const TRAINER_SPELL_GRAY: u8 = 2;
const TRAINER_SPELL_GREEN_DISABLED: u8 = 10;
// `TrainerFailReason` (`SharedDefines.h:1133-1135`).
const TRAIN_FAIL_UNAVAILABLE: u32 = 0;
const TRAIN_FAIL_NOT_ENOUGH_MONEY: u32 = 1;
const TRAIN_FAIL_NOT_ENOUGH_SKILL: u32 = 2;
// `TrainerType` (`CreatureDefines.h:32-35`).
const TRAINER_TYPE_CLASS: u32 = 0;
const TRAINER_TYPE_MOUNTS: u32 = 1;
const TRAINER_TYPE_TRADESKILLS: u32 = 2;
const TRAINER_TYPE_PETS: u32 = 3;
/// `REP_EXALTED` rank value in the local rank scale.
const REP_EXALTED: u32 = 7;
/// `CLASS_HUNTER`.
const CLASS_HUNTER: u8 = 3;

/// `GetSkillValueBase`: the saved skill value without
/// bonuses (the row model stores base values).
fn skill_value_base(player: &Player, skill: u32) -> u32 {
    player
        .deferred
        .skills
        .iter()
        .find(|s| s.skill == skill)
        .map_or(0, |s| s.value)
}

impl Map {
    /// `IsSpellFitByClassAndRace(spellId)` in its
    /// check-availability form (`pReqlevel == nullptr`).
    fn is_spell_fit_by_class_and_race(&self, player: &Player, spell_id: u32) -> bool {
        let race_mask = 1u32 << u32::from(player.unit.race().saturating_sub(1));
        let class_mask = 1u32 << u32::from(player.unit.class().saturating_sub(1));
        let mut any_ability = false;
        for ability in self.stores.spells.abilities_for_spell(spell_id) {
            any_ability = true;
            if ability.race_mask != 0 && ability.race_mask & race_mask == 0 {
                continue;
            }
            if ability.class_mask != 0 && ability.class_mask & class_mask == 0 {
                continue;
            }
            for rc in self
                .stores
                .skills
                .race_class
                .iter()
                .filter(|rc| rc.skill_id == ability.skill_id)
            {
                if rc.race_mask & race_mask != 0 && rc.class_mask & class_mask != 0 {
                    if rc.flags & SKILL_FLAG_NOT_TRAINABLE != 0 {
                        return false;
                    }
                    if rc.req_level != 0 && player.unit.level() < rc.req_level {
                        return false;
                    }
                }
            }
            return true;
        }
        !any_ability
    }

    /// `SpellMgr::IsPrimaryProfessionFirstRankSpell`.
    fn is_primary_profession_first_rank_spell(&self, spell_id: u32) -> bool {
        self.stores
            .spells
            .profession_skill_of_spell(spell_id)
            .is_some_and(|skill| {
                self.stores.skills.category(skill) == Some(SKILL_CATEGORY_PROFESSION)
            })
            && self.stores.spells.spell_rank(spell_id) == 1
    }

    /// `Player::GetTrainerSpellState`.
    fn trainer_spell_state(&self, guid: ObjectGuid, row: &TrainerSpell) -> u8 {
        let Some(pentry) = self.players.get(&guid) else {
            return TRAINER_SPELL_RED;
        };
        let player = &pentry.player;
        if row.spell == 0 {
            return TRAINER_SPELL_RED;
        }
        let Some(wrapper) = self.stores.spells.get(row.spell) else {
            return TRAINER_SPELL_RED;
        };
        let trigger_id = wrapper.effects.first().map_or(0, |e| e.trigger_spell);
        let Some(trigger) = self.stores.spells.get(trigger_id) else {
            return TRAINER_SPELL_RED;
        };
        if self.player_knows_spell(guid, trigger_id) {
            return TRAINER_SPELL_GRAY;
        }
        if !self.is_spell_fit_by_class_and_race(player, trigger_id) {
            return TRAINER_SPELL_RED;
        }
        let spell_level = if row.req_level != 0 {
            row.req_level
        } else {
            trigger.spell_level
        };
        if player.unit.level() < spell_level {
            return TRAINER_SPELL_RED;
        }
        if let Some(node) = self.stores.spells.chain_node(trigger_id) {
            if node.prev != 0 && !self.player_knows_spell(guid, node.prev) {
                return TRAINER_SPELL_RED;
            }
            if node.req != 0 && !self.player_knows_spell(guid, node.req) {
                return TRAINER_SPELL_RED;
            }
        }
        if row.req_skill != 0 && skill_value_base(player, row.req_skill) < row.req_skill_value {
            return TRAINER_SPELL_RED;
        }
        // Fork quirk: `skill` reads the *wrapper's*
        // effect-1 misc value, not the taught spell's.
        let wrapper_skill = wrapper
            .effects
            .get(1)
            .map_or(0, |e| u32::try_from(e.misc_value).unwrap_or(0));
        let mut is_profession_learn_spell =
            wrapper.effects.get(1).map_or(0, |e| e.effect) == effect::SKILL;
        let mut is_profession_first_rank = self.is_primary_profession_first_rank_spell(row.spell);
        if !is_profession_learn_spell {
            for eff in &wrapper.effects {
                if eff.effect != effect::LEARN_SPELL {
                    continue;
                }
                let Some(taught) = self.stores.spells.get(eff.trigger_spell) else {
                    continue;
                };
                if taught.effects.get(1).map_or(0, |e| e.effect) == effect::SKILL {
                    is_profession_learn_spell = true;
                }
                if self.is_primary_profession_first_rank_spell(eff.trigger_spell) {
                    is_profession_first_rank = true;
                }
            }
        }
        let primary_skill =
            self.stores.skills.category(wrapper_skill) == Some(SKILL_CATEGORY_PROFESSION);
        if !is_profession_learn_spell || !primary_skill {
            return TRAINER_SPELL_GREEN;
        }
        let free_points = player
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_CHARACTER_POINTS2);
        if is_profession_first_rank && free_points == 0 {
            return TRAINER_SPELL_GREEN_DISABLED;
        }
        TRAINER_SPELL_GREEN
    }

    /// `PlayerMenu::SendGossipMenu(textId, guid)` with
    /// an empty menu (the trainer rejection shape).
    fn send_trainer_gossip(&mut self, guid: ObjectGuid, npc_guid: ObjectGuid, text_id: u32) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc_guid.raw());
        writer.write_u32_le(text_id);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let packet = WorldPacket::with_body(opcode_id::SMSG_GOSSIP_MESSAGE, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `Creature::IsTrainerOf(player, msg)`.
    pub(crate) fn is_trainer_of(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        msg: bool,
    ) -> bool {
        let Some(centry) = self.creatures.get(&npc_guid) else {
            return false;
        };
        if !centry
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::UNIT_NPC_FLAGS, NPC_FLAG_TRAINER)
        {
            return false;
        }
        let info = centry.info.clone();
        let c_spells = self.stores.trainer_spells.creature_spells(info.entry);
        let t_spells = if info.trainer_id != 0 {
            self.stores.trainer_spells.template_spells(info.trainer_id)
        } else {
            None
        };
        let empty = |data: Option<&TrainerSpellData>| data.is_none_or(|d| d.spells.is_empty());
        if empty(c_spells) && empty(t_spells) {
            return false;
        }
        let Some(pentry) = self.players.get(&guid) else {
            return false;
        };
        let player = &pentry.player;
        match info.trainer_type {
            TRAINER_TYPE_CLASS => {
                if u32::from(player.unit.class()) != info.trainer_class {
                    if msg {
                        // Wrong-class rejection menus
                        // (`Creature.cpp:1296-1324`).
                        let text_id = match info.trainer_class {
                            11 => 4913, // druid
                            3 => 10090, // hunter
                            8 => 328,   // mage
                            2 => 1635,  // paladin
                            5 => 4436,  // priest
                            4 => 4797,  // rogue
                            7 => 5003,  // shaman
                            9 => 5836,  // warlock
                            1 => 4985,  // warrior
                            _ => 0,
                        };
                        if text_id != 0 {
                            self.send_trainer_gossip(guid, npc_guid, text_id);
                        }
                    }
                    return false;
                }
            }
            TRAINER_TYPE_PETS => {
                if player.unit.class() != CLASS_HUNTER {
                    if msg {
                        self.send_trainer_gossip(guid, npc_guid, 3620);
                    }
                    return false;
                }
            }
            TRAINER_TYPE_MOUNTS => {
                if info.trainer_race != 0 && u32::from(player.unit.race()) != info.trainer_race {
                    let exalted = self
                        .stores
                        .factions
                        .template(centry.unit.faction_template())
                        .is_some_and(|tpl| {
                            self.reputation_rank(player, tpl.faction) == REP_EXALTED
                        });
                    if exalted {
                        return true;
                    }
                    if msg {
                        // Fork quirk: the rejection
                        // switch reads `trainer_class`
                        // (holding race ids here).
                        let text_id = match info.trainer_class {
                            3 => 5865, // dwarf
                            7 => 4881, // gnome
                            1 => 5861, // human
                            4 => 5862, // night elf
                            2 => 5863, // orc
                            6 => 5864, // tauren
                            8 => 5816, // troll
                            5 => 624,  // undead
                            _ => 0,
                        };
                        if text_id != 0 {
                            self.send_trainer_gossip(guid, npc_guid, text_id);
                        }
                    }
                    return false;
                }
            }
            TRAINER_TYPE_TRADESKILLS => {
                if info.trainer_spell != 0 && !self.player_knows_spell(guid, info.trainer_spell) {
                    if msg {
                        self.send_trainer_gossip(guid, npc_guid, 11031);
                    }
                    return false;
                }
            }
            _ => return false,
        }
        true
    }

    /// The creature's own + template trainer lists.
    fn trainer_lists(
        &self,
        npc_guid: ObjectGuid,
    ) -> (Option<TrainerSpellData>, Option<TrainerSpellData>, u32) {
        let Some(centry) = self.creatures.get(&npc_guid) else {
            return (None, None, 0);
        };
        let entry = centry.info.entry;
        let trainer_id = centry.info.trainer_id;
        let own = self.stores.trainer_spells.creature_spells(entry).cloned();
        let template = if trainer_id != 0 {
            self.stores
                .trainer_spells
                .template_spells(trainer_id)
                .cloned()
        } else {
            None
        };
        (own, template, entry)
    }

    /// `WorldSession::SendTrainerList` (the default
    /// title is `LANG_NPC_TAINER_HELLO`, resolved by
    /// the actor which owns the string table).
    pub fn send_trainer_list(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        default_title: &str,
    ) {
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_TRAINER) {
            return;
        }
        if !self.is_trainer_of(guid, npc_guid, true) {
            return;
        }
        let (c_spells, t_spells, entry) = self.trainer_lists(npc_guid);
        if c_spells.is_none() && t_spells.is_none() {
            return;
        }
        self.interrupt_spells_with_channel_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS);
        self.remove_auras_with_interrupt_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS, 0);
        let trainer_type = match (&c_spells, &t_spells) {
            (Some(c), _) if c.trainer_type != 0 => c.trainer_type,
            (_, Some(t)) => t.trainer_type,
            _ => 0,
        };
        let title = self
            .stores
            .trainer_spells
            .greeting(entry)
            .unwrap_or(default_title)
            .to_owned();
        let discount = self.reputation_price_discount(guid, npc_guid);
        let can_learn_primary_prof = self.players.get(&guid).is_some_and(|e| {
            e.player
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_CHARACTER_POINTS2)
                > 0
        });

        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc_guid.raw());
        writer.write_u32_le(trainer_type);
        let count_at = writer.len();
        writer.write_u32_le(0);
        let mut count: u32 = 0;
        for list in [&c_spells, &t_spells] {
            let Some(data) = list else { continue };
            for row in &data.spells {
                let Some(wrapper) = self.stores.spells.get(row.spell) else {
                    continue;
                };
                let trigger_id = wrapper.effects.first().map_or(0, |e| e.trigger_spell);
                let fits = self
                    .players
                    .get(&guid)
                    .is_some_and(|e| self.is_spell_fit_by_class_and_race(&e.player, trigger_id));
                if !fits {
                    continue;
                }
                let state = self.trainer_spell_state(guid, row);
                self.write_trainer_spell(
                    &mut writer,
                    row,
                    trigger_id,
                    state,
                    discount,
                    can_learn_primary_prof,
                );
                count += 1;
            }
        }
        writer.write_cstring(&title);
        let mut bytes = writer.into_vec();
        bytes[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
        let packet = WorldPacket::with_body(opcode_id::SMSG_TRAINER_LIST, bytes);
        self.send_to_player(guid, packet);
    }

    /// `SendTrainerSpellHelper` row encoding.
    fn write_trainer_spell(
        &self,
        writer: &mut PacketWriter,
        row: &TrainerSpell,
        trigger_id: u32,
        state: u8,
        discount: f32,
        can_learn_primary_prof: bool,
    ) {
        let trigger = self.stores.spells.get(trigger_id);
        let spell_level = if row.req_level != 0 {
            row.req_level
        } else {
            trigger.map_or(0, |t| t.spell_level)
        };
        let primary_first_rank = self.is_primary_profession_first_rank_spell(trigger_id);
        writer.write_u32_le(row.spell);
        writer.write_u8(if state == TRAINER_SPELL_GREEN_DISABLED {
            TRAINER_SPELL_GREEN
        } else {
            state
        });
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let cost = (row.spell_cost as f32 * discount + 0.5) as u32;
        writer.write_u32_le(cost);
        writer.write_u32_le(u32::from(primary_first_rank && can_learn_primary_prof));
        writer.write_u32_le(u32::from(primary_first_rank));
        writer.write_u8(u8::try_from(spell_level).unwrap_or(u8::MAX));
        writer.write_u32_le(row.req_skill);
        writer.write_u32_le(row.req_skill_value);
        match self.stores.spells.chain_node(trigger_id) {
            Some(node) if node.req != 0 => {
                writer.write_u32_le(node.req);
                writer.write_u32_le(node.prev);
            }
            Some(node) => {
                writer.write_u32_le(node.prev);
                writer.write_u32_le(0);
            }
            None => {
                writer.write_u32_le(0);
                writer.write_u32_le(0);
            }
        }
        writer.write_u32_le(0);
    }

    /// `SendTrainingFailure`.
    fn send_training_failure(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        spell_id: u32,
        error: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc_guid.raw());
        writer.write_u32_le(spell_id);
        writer.write_u32_le(error);
        let packet = WorldPacket::with_body(opcode_id::SMSG_TRAINER_BUY_FAILED, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `WorldSession::HandleTrainerBuySpellOpcode`.
    pub fn trainer_buy_spell(&mut self, guid: ObjectGuid, npc_guid: ObjectGuid, spell_id: u32) {
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_TRAINER)
            || !self.is_trainer_of(guid, npc_guid, true)
        {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_UNAVAILABLE);
            return;
        }
        let (c_spells, t_spells, _) = self.trainer_lists(npc_guid);
        if c_spells.is_none() && t_spells.is_none() {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_UNAVAILABLE);
            return;
        }
        let found = c_spells
            .as_ref()
            .and_then(|data| data.find(spell_id).copied())
            .or_else(|| {
                t_spells
                    .as_ref()
                    .and_then(|data| data.find(spell_id).copied())
            });
        let Some(row) = found else {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_UNAVAILABLE);
            return;
        };
        if self.trainer_spell_state(guid, &row) != TRAINER_SPELL_GREEN {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_NOT_ENOUGH_SKILL);
            return;
        }
        let Some(proto) = self.stores.spells.get(row.spell).cloned() else {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_UNAVAILABLE);
            return;
        };
        let discount = self.reputation_price_discount(guid, npc_guid);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let cost = (row.spell_cost as f32 * discount + 0.5) as u32;
        if self.player_money(guid) < cost {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_NOT_ENOUGH_MONEY);
            return;
        }
        self.interrupt_spells_with_channel_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS);
        self.remove_auras_with_interrupt_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS, 0);
        self.remove_spells_causing_aura(guid, aura_type::MOUNTED);
        // Visual 222 wrappers are self-cast by the
        // player; all others are cast by the trainer.
        let caster = if proto.spell_visual == 222 {
            guid
        } else {
            npc_guid
        };
        let outcome = self.cast_spell(
            caster,
            row.spell,
            CastTarget::Unit(guid),
            CastOptions {
                skip_known: true,
                ..CastOptions::default()
            },
        );
        if matches!(outcome, CastOutcome::Started) {
            self.modify_money(guid, -i64::from(cost));
            let mut writer = PacketWriter::new();
            writer.write_u64_le(npc_guid.raw());
            writer.write_u32_le(spell_id);
            let packet =
                WorldPacket::with_body(opcode_id::SMSG_TRAINER_BUY_SUCCEEDED, writer.into_vec());
            self.send_to_player(guid, packet);
        } else {
            self.send_training_failure(guid, npc_guid, spell_id, TRAIN_FAIL_UNAVAILABLE);
        }
    }
}
