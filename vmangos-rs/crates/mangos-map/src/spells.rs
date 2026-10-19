//! Spell casting + auras (Slice 5): the `prepare ->
//! cast -> go` pipeline subset over [`SpellEntry`]
//! data, applied in the map tick. Supported: single
//! unit targets (enemy/self/party-subset),
//! `SPELL_EFFECT_SCHOOL_DAMAGE` and
//! `SPELL_EFFECT_APPLY_AURA` (periodic damage + attack
//! power mods), GCD + spell/category cooldowns,
//! hit/resist/crit rolls with the C++ formulas, aura
//! slots/durations/periodic ticks, creature death +
//! respawn scheduling.
//!
//! Recorded subset deviations: no projectile delay
//! (speed > 0 spells fail), no spell mods/talents, no
//! bonus coefficients (spell power system not built —
//! `SpellDamageBonusDone/Taken` at their no-aura
//! identity), no dodge/parry/block on melee spells, no
//! LOS/facing checks (vmaps not wired), no pushback,
//! no channeled spells, no combos/area/container
//! targets, corpse stays until respawn (no decay),
//! player death is state-only (no ghost flow).

use mangos_entities::aura::{
    AuraEffectState, AuraHolder, MAX_AURAS, MAX_POSITIVE_AURAS, NULL_AURA_SLOT,
};
use mangos_entities::aura::{CooldownSnapshot, SavableAura};
use mangos_entities::unit::{DeathState, UnitBase};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::SpellEntry;
use mangos_spells::defines::{
    aura as aura_type, cast_result, dmg_class, effect, power, school, target as tgt,
};

use mangos_world_data::spawn_support::RespawnTimes;

use crate::map::{Map, MapEvent};

/// `UNIT_FLAG_IN_COMBAT`.
const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
/// `CAST_FLAG_UNKNOWN1` (always set on non-triggered
/// visible casts... kept minimal: set when cast time > 0
/// like the ranged-weapon-independent clients expect).
const CAST_FLAG_NONE: u32 = 0x0;
/// `SPELL_HIT_TYPE_CRIT` for the damage log.
const SPELL_HIT_TYPE_CRIT: u16 = 0x02;
/// `SPELL_ATTR_EX3_ALWAYS_HIT`.
const ATTR_EX3_ALWAYS_HIT: u32 = 0x0004_0000;
/// `SPELL_ATTR_EX4_IGNORE_RESISTANCES`.
const ATTR_EX4_IGNORE_RESISTANCES: u32 = 0x0000_0001;
/// `SPELL_ATTR_NO_AURA_CANCEL` (Attributes bit 31).
const ATTR_NO_AURA_CANCEL: u32 = 0x8000_0000;
/// `SPELL_EFFECT_INTERRUPT_CAST` / `SPELL_EFFECT_KNOCK_BACK`
/// for the binary-spell rule.
const EFFECT_INTERRUPT_CAST: u32 = 68;
const EFFECT_KNOCK_BACK: u32 = 98;
/// Aura ids not yet in `defines::aura`, verified
/// against `SpellAuraDefines.h` (binary-spell rule).
const MOD_FEAR: u32 = 7;
const MOD_PACIFY: u32 = 25;
const MOD_SILENCE: u32 = 27;
const MOD_DISARM: u32 = 67;
const MOD_RESISTANCE: u32 = 22;
const MOD_DAMAGE_TAKEN: u32 = 14;

/// Target selector for a cast request (client-parsed
/// subset of `SpellCastTargets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastTarget {
    SelfCast,
    Unit(ObjectGuid),
}

/// Caller-side overrides (`CastSpell` overloads in C++).
#[derive(Clone, Copy, Debug, Default)]
pub struct CastOptions {
    /// Triggered cast: skips power/GCD/cooldown/known
    /// checks and the `SPELL_START` packet family changes.
    pub triggered: bool,
    /// Skip the known-spell check (creature API casts,
    /// login auras).
    pub skip_known: bool,
}

/// One in-progress cast (`Spell` object subset).
pub struct ActiveCast {
    pub spell_id: u32,
    pub cast_counter: u8,
    pub target: CastTarget,
    pub remaining_ms: i32,
    pub total_ms: u32,
    pub triggered: bool,
    /// Original caster position (movement interrupts —
    /// handled by `apply_movement` cancellation, kept
    /// for diagnostics).
    pub position: Position,
}

/// Why a cast did not start (for the actor's logging;
/// the client packet is emitted by the map).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CastOutcome {
    Started,
    Failed(u8),
    UnknownSpell,
}

/// One due periodic aura tick, collected under the
/// holder borrow and applied afterwards.
#[derive(Clone, Copy)]
struct PeriodicTick {
    target: ObjectGuid,
    caster: ObjectGuid,
    spell_id: u32,
    aura_type: u32,
    amount: i32,
    effect_index: u32,
    /// `EffectMiscValue` (power type for energize).
    misc_value: i32,
}

impl Map {
    pub(crate) fn spell_rng(&mut self) -> &mut dyn mangos_spells::SpellRng {
        &mut self.rng
    }

    pub(crate) fn unit_level(&self, guid: ObjectGuid) -> u32 {
        self.unit_value_u32(guid, offsets::UNIT_FIELD_LEVEL)
    }

    pub(crate) fn unit_value_u32(&self, guid: ObjectGuid, index: u16) -> u32 {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.world.object.values().get_u32(index);
        }
        self.creatures
            .get(&guid)
            .map_or(0, |c| c.unit.world.object.values().get_u32(index))
    }

    fn unit_power_index_of(power_type: u32) -> Option<u16> {
        Some(match power_type {
            power::MANA => offsets::UNIT_FIELD_POWER1,
            power::RAGE => offsets::UNIT_FIELD_POWER2,
            power::FOCUS => offsets::UNIT_FIELD_POWER3,
            power::ENERGY => offsets::UNIT_FIELD_POWER4,
            _ => return None,
        })
    }

    fn unit_max_power_index_of(power_type: u32) -> Option<u16> {
        Some(match power_type {
            power::MANA => offsets::UNIT_FIELD_MAXPOWER1,
            power::RAGE => offsets::UNIT_FIELD_MAXPOWER2,
            power::FOCUS => offsets::UNIT_FIELD_MAXPOWER3,
            power::ENERGY => offsets::UNIT_FIELD_MAXPOWER4,
            _ => return None,
        })
    }

    fn unit_power(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == power::HEALTH {
            return self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH);
        }
        Self::unit_power_index_of(power_type).map_or(0, |i| self.unit_value_u32(guid, i))
    }

    fn unit_max_power(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == power::HEALTH {
            return self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
        }
        Self::unit_max_power_index_of(power_type).map_or(0, |i| self.unit_value_u32(guid, i))
    }

    fn unit_set_power(&mut self, guid: ObjectGuid, power_type: u32, value: u32) {
        let index = if power_type == power::HEALTH {
            offsets::UNIT_FIELD_HEALTH
        } else if let Some(i) = Self::unit_power_index_of(power_type) {
            i
        } else {
            return;
        };
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values().set_u32(index, value);
        }
    }

    fn is_hostile_pair(a: ObjectGuid, b: ObjectGuid) -> bool {
        // No faction/reaction system yet: player<->creature
        // is hostile, same-kind is not.
        a.is_player() != b.is_player()
    }

    fn send_to_player(&mut self, guid: ObjectGuid, packet: WorldPacket) {
        if guid.is_player() {
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
    }

    /// `SendCastResult(Player*)` (failures only here).
    fn send_cast_result(&mut self, caster: ObjectGuid, spell_id: u32, result: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(spell_id);
        payload.write_u8(result);
        // No extra-reason reasons are produced by the
        // subset (equipment/reagent/etc.).
        let packet = WorldPacket::with_body(op::SMSG_CAST_RESULT, payload.into_vec());
        self.send_to_player(caster, packet);
    }

    fn has_cooldown(&self, caster: ObjectGuid, spell: &SpellEntry) -> bool {
        let now = self.now_ms;
        let spell_cd = self
            .cooldowns
            .get(&(caster, spell.id))
            .is_some_and(|&until| until > now);
        let cat_cd = spell.category != 0
            && self
                .cooldowns
                .get(&(caster, spell.category | 0x8000_0000))
                .is_some_and(|&until| until > now);
        spell_cd || cat_cd
    }

    fn add_cooldowns(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        if spell.recovery_time > 0 {
            self.cooldowns
                .insert((caster, spell.id), self.now_ms + spell.recovery_time);
        }
        if spell.category != 0 && spell.category_recovery_time > 0 {
            self.cooldowns.insert(
                (caster, spell.category | 0x8000_0000),
                self.now_ms + spell.category_recovery_time,
            );
        }
    }

    /// Known-spell check for players
    /// (`Player::HasActiveSpell` subset over the
    /// deferred `character_spell` rows; creatures always
    /// pass).
    /// Bot query: `HasAura`.
    #[must_use]
    pub fn bot_has_aura(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.aura_holders(guid)
            .iter()
            .any(|h| h.spell_id == spell_id)
    }

    /// Bot query: `HasAuraType(PERIODIC_HEAL)`.
    #[must_use]
    pub fn bot_has_periodic_heal(&self, guid: ObjectGuid) -> bool {
        self.aura_holders(guid).iter().any(|h| {
            h.effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_HEAL)
        })
    }

    /// Bot query: loss-of-control state
    /// (`UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL`
    /// approximated by holder aura types:
    /// `MOD_CONFUSE` 5, `MOD_FEAR` 7, `MOD_STUN`
    /// 12, `MOD_ROOT` 26).
    #[must_use]
    pub fn bot_is_ccd(&self, guid: ObjectGuid) -> bool {
        self.aura_holders(guid).iter().any(|h| {
            h.effects
                .iter()
                .flatten()
                .any(|e| matches!(e.aura_type, 5 | 7 | 12 | 26))
        })
    }

    /// Bot query: `HasAuraType`.
    #[must_use]
    pub fn bot_has_aura_type(&self, guid: ObjectGuid, aura_type: u32) -> bool {
        self.aura_holders(guid)
            .iter()
            .any(|h| h.effects.iter().flatten().any(|e| e.aura_type == aura_type))
    }

    /// Bot verb: `RemoveSpellsCausingAura(MOD_SHAPESHIFT)`.
    pub fn bot_remove_shapeshift(&mut self, guid: ObjectGuid) {
        let spells: Vec<u32> = self
            .aura_holders(guid)
            .iter()
            .filter(|h| h.effects.iter().flatten().any(|e| e.aura_type == 36))
            .map(|h| h.spell_id)
            .collect();
        for spell_id in spells {
            self.cancel_aura(guid, spell_id);
        }
    }

    /// Bot verb: `SetFacingToObject`.
    pub fn bot_face(&mut self, guid: ObjectGuid, target: ObjectGuid) {
        let (Some(from), Some(to)) = (self.unit_position(guid), self.unit_position(target)) else {
            return;
        };
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.world.position.o = (to.y - from.y).atan2(to.x - from.x);
        }
    }

    /// Bot verb: `AttackStop` (clears melee victim).
    pub fn bot_attack_stop(&mut self, guid: ObjectGuid) {
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.melee_victim = None;
        }
    }

    /// Bot query: melee reach (plain 5y gate; the
    /// fork subtracts combat reaches via
    /// `GetCombatDistance`).
    #[must_use]
    pub fn bot_can_reach_melee(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool {
        let (Some(a), Some(b)) = (self.unit_position(attacker), self.unit_position(target)) else {
            return false;
        };
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt() <= 5.0
    }

    /// In-progress cast spell id for a caster
    /// (`GetCurrentSpell` generic+channeled reduced
    /// to the single active-cast store), for the
    /// bot situation snapshot.
    #[must_use]
    pub fn bot_casting_spell(&self, caster: ObjectGuid) -> Option<u32> {
        self.casts.get(&caster).map(|cast| cast.spell_id)
    }

    /// Active spellbook ids for a player (bot
    /// spell classification input).
    #[must_use]
    pub fn bot_spell_ids(&self, guid: ObjectGuid) -> Vec<u32> {
        self.players.get(&guid).map_or_else(Vec::new, |entry| {
            entry
                .player
                .deferred
                .spells
                .iter()
                .filter(|s| s.active != 0)
                .map(|s| s.spell)
                .collect()
        })
    }

    /// Known-spell check for Host API consumers
    /// (`Player::HasSpell` parity helper).
    #[must_use]
    pub fn bot_knows_spell(&self, caster: ObjectGuid, spell_id: u32) -> bool {
        self.player_knows_spell(caster, spell_id)
    }

    fn player_knows_spell(&self, caster: ObjectGuid, spell_id: u32) -> bool {
        if !caster.is_player() {
            return true;
        }
        self.players.get(&caster).is_some_and(|entry| {
            entry
                .player
                .deferred
                .spells
                .iter()
                .any(|s| s.spell == spell_id && s.active != 0)
        })
    }

    /// `Unit::CastSpell` + `Spell::prepare` subset.
    /// Starts the cast (instant spells complete inside
    /// this call, like C++ update(0) chaining).
    pub fn cast_spell(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        target: CastTarget,
        options: CastOptions,
    ) -> CastOutcome {
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return CastOutcome::UnknownSpell;
        };
        let fail = |map: &mut Map, reason: u8| {
            if !options.triggered {
                map.send_cast_result(caster, spell_id, reason);
            }
            CastOutcome::Failed(reason)
        };

        let alive = self.unit_base_mut(caster).is_some_and(|u| u.is_alive());
        // Triggered casts skip the dead-caster gate
        // (EventAI DEATH casts like Eject Sneed).
        if !alive && !options.triggered {
            return fail(self, cast_result::FAILED_CASTER_DEAD);
        }
        if !options.skip_known && !self.player_knows_spell(caster, spell_id) {
            return fail(self, cast_result::FAILED_UNKNOWN);
        }
        if self.casts.contains_key(&caster) {
            return fail(self, cast_result::FAILED_SPELL_IN_PROGRESS);
        }
        // Projectile delay is not modeled in the slice.
        if spell.speed > 0.0 {
            return fail(self, cast_result::FAILED_UNKNOWN);
        }
        if !options.triggered {
            let now = self.now_ms;
            let gcd = self
                .unit_base_mut(caster)
                .is_some_and(|u| u.gcd_until_ms > now);
            if gcd || self.has_cooldown(caster, &spell) {
                return fail(self, cast_result::FAILED_NOT_READY);
            }
            let cost = spell.power_cost(
                self.unit_power(caster, spell.power_type),
                self.unit_max_power(caster, spell.power_type),
                self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
            );
            if self.unit_power(caster, spell.power_type) < cost {
                return fail(self, cast_result::FAILED_NO_POWER);
            }
        }
        // Target validation (subset of CheckCast target
        // rules driven by effect 0's implicit target).
        if let CastOutcome::Failed(reason) = self.check_target(caster, &spell, target) {
            return fail(self, reason);
        }

        let cast_time = if options.triggered {
            0
        } else {
            spell.cast_time_ms(&self.stores.spells.tables)
        };
        let counter = self.unit_base_mut(caster).map_or(0, |u| {
            let c = u.cast_counter;
            u.cast_counter = u.cast_counter.wrapping_add(1);
            c
        });
        let position = self.unit_position(caster).unwrap_or_default();
        let cast = ActiveCast {
            spell_id,
            cast_counter: counter,
            target,
            remaining_ms: i32::try_from(cast_time).unwrap_or(i32::MAX),
            total_ms: cast_time,
            triggered: options.triggered,
            position,
        };
        if cast_time > 0 && !options.triggered {
            self.send_spell_start(caster, &cast, &spell);
        }
        self.casts.insert(caster, cast);
        if cast_time == 0 {
            self.update_cast(caster, 0);
        }
        CastOutcome::Started
    }

    fn check_target(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> CastOutcome {
        let mode = spell
            .effects
            .iter()
            .find(|e| e.effect != effect::NONE)
            .map_or(tgt::UNIT_CASTER, |e| e.implicit_target_a);
        match mode {
            tgt::UNIT_CASTER
            | tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::UNIT_PARTY
            | tgt::UNIT_FRIEND_AND_PARTY
            // Destination-at-caster spells (summons):
            // the destination is the caster position in
            // the subset, so no unit target is required.
            // Caster-centered AoE enumerations likewise
            // start (victim expansion is a recorded
            // Slice 5 gap; effects land on the caster
            // target slot).
            | tgt::LOCATION_CASTER_DEST
            | tgt::LOCATION_CASTER_SRC => CastOutcome::Started,
            tgt::UNIT_ENEMY | tgt::UNIT_FRIEND => {
                let CastTarget::Unit(victim) = target else {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_IMPLICIT_TARGETS);
                };
                let Some(vpos) = self.unit_position(victim) else {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                };
                let alive = self
                    .creatures
                    .get(&victim)
                    .is_some_and(|c| c.unit.is_alive())
                    || self
                        .players
                        .get(&victim)
                        .is_some_and(|p| p.player.unit.is_alive());
                if !alive {
                    return CastOutcome::Failed(cast_result::FAILED_TARGETS_DEAD);
                }
                if mode == tgt::UNIT_ENEMY && !Self::is_hostile_pair(caster, victim) {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                if mode == tgt::UNIT_FRIEND && Self::is_hostile_pair(caster, victim) {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                let cpos = self.unit_position(caster).unwrap_or_default();
                let range = spell.range_max(&self.stores.spells.tables)
                    + self.bounding_radius_of(caster)
                    + self.bounding_radius_of(victim);
                let dist = ((cpos.x - vpos.x).powi(2) + (cpos.y - vpos.y).powi(2)).sqrt();
                if dist > range {
                    return CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE);
                }
                CastOutcome::Started
            }
            _ => CastOutcome::Failed(cast_result::FAILED_UNKNOWN),
        }
    }

    fn targets_write(target: CastTarget, caster: ObjectGuid, payload: &mut PacketWriter) {
        // `SpellCastTargets::write` subset: u16 mask +
        // packed unit guid.
        match target {
            CastTarget::Unit(guid) => {
                payload.write_u16_le(0x0002); // TARGET_FLAG_UNIT
                payload.write_bytes(&Self::packed_guid_bytes(guid));
            }
            CastTarget::SelfCast => {
                payload.write_u16_le(0x0002);
                payload.write_bytes(&Self::packed_guid_bytes(caster));
            }
        }
    }

    fn send_spell_start(&mut self, caster: ObjectGuid, cast: &ActiveCast, spell: &SpellEntry) {
        let target_guid = match cast.target {
            CastTarget::Unit(g) => g,
            CastTarget::SelfCast => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(target_guid));
        payload.write_u8(cast.cast_counter);
        payload.write_u32_le(spell.id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(cast.total_ms);
        Self::targets_write(cast.target, caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_START, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// `InterruptSpell` + cancel-cast: emits the failure
    /// packets for timed casts.
    pub fn cancel_cast(&mut self, caster: ObjectGuid, interrupted: bool) {
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        if cast.total_ms == 0 {
            return;
        }
        if interrupted {
            self.send_cast_result(caster, cast.spell_id, cast_result::FAILED_INTERRUPTED);
        }
        // SMSG_SPELL_FAILURE (caster session) +
        // SMSG_SPELL_FAILED_OTHER (set): raw GUID + id
        // (+ reason on FAILURE).
        let mut payload = PacketWriter::new();
        payload.write_u64_le(caster.raw());
        payload.write_u32_le(cast.spell_id);
        if interrupted {
            let mut with_reason = payload.clone();
            with_reason.write_u8(cast_result::FAILED_INTERRUPTED);
            let packet = WorldPacket::with_body(op::SMSG_SPELL_FAILURE, with_reason.into_vec());
            self.send_to_player(caster, packet);
        }
        let packet = WorldPacket::with_body(op::SMSG_SPELL_FAILED_OTHER, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// `Spell::update` + `Spell::cast` subset.
    pub(crate) fn update_cast(&mut self, caster: ObjectGuid, diff_ms: u32) {
        let done = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                cast.remaining_ms <= 0
            }
            None => return,
        };
        if !done {
            return;
        }
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        let Some(spell) = self.stores.spells.get(cast.spell_id).cloned() else {
            return;
        };
        let alive = self.unit_base_mut(caster).is_some_and(|u| u.is_alive());
        // Triggered casts complete even from a dead
        // caster (death-event spells).
        if !alive && !cast.triggered {
            return;
        }

        // TakePower + GCD + cooldowns at cast()
        // (StartRecoveryTime).
        if !cast.triggered {
            let power_cost = spell.power_cost(
                self.unit_power(caster, spell.power_type),
                self.unit_max_power(caster, spell.power_type),
                self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
            );
            let current = self.unit_power(caster, spell.power_type);
            self.unit_set_power(caster, spell.power_type, current.saturating_sub(power_cost));
            // `Spell::TakePower` five-second timer
            // (`Spell.cpp:5252-5255`): a positive mana
            // spend without
            // `SPELL_ATTR_EX2_DONT_BLOCK_MANA_REGEN`
            // (0x02000000) arms `SetLastManaUse`.
            if spell.power_type as usize == mangos_entities::unit::POWER_MANA
                && power_cost > 0
                && spell.attributes_ex2 & 0x0200_0000 == 0
                && let Some(u) = self.unit_base_mut(caster)
            {
                u.set_last_mana_use(spell.id);
            }
            if spell.start_recovery_time > 0 {
                let until = self.now_ms + spell.start_recovery_time;
                if let Some(u) = self.unit_base_mut(caster) {
                    u.gcd_until_ms = until;
                }
            }
            self.add_cooldowns(caster, &spell);
        }

        let victims = Self::collect_targets(caster, &spell, cast.target);
        let caster_level = self.unit_level(caster);
        let mut hits: Vec<ObjectGuid> = Vec::new();
        let mut misses: Vec<(ObjectGuid, u8)> = Vec::new();
        let mut hit_victims: Vec<ObjectGuid> = Vec::new();
        for victim in &victims {
            if spell_meleedamage(spell.dmg_class) {
                // DoMeleeAttackIfReady-style hit roll.
                let hit_chance = self.melee_spell_hit_chance(caster, *victim, &spell);
                let roll = self.spell_rng().frand01() * 100.0;
                if roll < hit_chance {
                    hits.push(*victim);
                    hit_victims.push(*victim);
                } else {
                    misses.push((*victim, 1)); // SPELL_MISS_MISS
                }
            } else if spell_is_positive_for_target(&spell) {
                hits.push(*victim);
                hit_victims.push(*victim);
            } else {
                let hit_chance = self.magic_hit_chance(caster, *victim, &spell);
                let roll = self.spell_rng().frand01() * 100.0;
                if roll < hit_chance {
                    hits.push(*victim);
                    hit_victims.push(*victim);
                } else {
                    misses.push((*victim, 2)); // SPELL_MISS_RESIST
                }
            }
        }

        self.send_spell_go(caster, &cast, &spell, &hits, &misses);
        for victim in hit_victims {
            self.apply_effects(caster, caster_level, &spell, victim);
        }
        let _ = caster_level;
    }

    fn collect_targets(
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Vec<ObjectGuid> {
        let mode = spell
            .effects
            .iter()
            .find(|e| e.effect != effect::NONE)
            .map_or(tgt::UNIT_CASTER, |e| e.implicit_target_a);
        match mode {
            tgt::UNIT_ENEMY | tgt::UNIT_FRIEND => match target {
                CastTarget::Unit(g) => vec![g],
                CastTarget::SelfCast => vec![caster],
            },
            tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::UNIT_PARTY
            | tgt::UNIT_FRIEND_AND_PARTY => {
                // Party system not built: caster only
                // (Battle Shout self-buff evidence).
                vec![caster]
            }
            _ => vec![caster],
        }
    }

    fn send_spell_go(
        &mut self,
        caster: ObjectGuid,
        cast: &ActiveCast,
        spell: &SpellEntry,
        hits: &[ObjectGuid],
        misses: &[(ObjectGuid, u8)],
    ) {
        let target_guid = match cast.target {
            CastTarget::Unit(g) => g,
            CastTarget::SelfCast => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(target_guid));
        payload.write_u8(cast.cast_counter);
        payload.write_u32_le(spell.id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(self.now_ms);
        payload.write_u8(hits.len() as u8);
        for h in hits {
            payload.write_u64_le(h.raw());
        }
        payload.write_u8(misses.len() as u8);
        for (m, cond) in misses {
            payload.write_u64_le(m.raw());
            payload.write_u8(*cond);
        }
        Self::targets_write(cast.target, caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_GO, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    #[allow(clippy::cast_possible_truncation)]
    fn apply_effects(
        &mut self,
        caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        victim: ObjectGuid,
    ) {
        // EventAI SpellHit hooks (Slice 6b), once per
        // hit target as in `DoAllEffectOnTarget`.
        if victim.is_creature() && victim != caster {
            self.eventai_on_spell_hit(victim, caster, spell.id, spell.school);
        }
        if caster.is_creature() && victim != caster {
            self.eventai_on_spell_hit_target(caster, victim, spell.id, spell.school);
        }
        if !self.unit_is_alive(victim) && victim.is_creature() {
            // The hit hook may have despawned the
            // target (scripts); C++ would still apply
            // to a live object only.
            if !self.creatures.contains_key(&victim) {
                return;
            }
        }
        // In-combat flags (DealDamage side effects land
        // with effects too via combat in ApplyAura).
        for i in 0..3 {
            let eff = spell.effects[i];
            if eff.effect == effect::NONE {
                continue;
            }
            match eff.effect {
                effect::SCHOOL_DAMAGE => {
                    let value = spell.calculate_effect_value(i, caster_level, 0, self.spell_rng());
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let base = mangos_spells::ditheru(value, self.spell_rng()) as i32;
                    self.school_damage(caster, spell, victim, base, i as u32, None);
                }
                effect::HEAL => {
                    let value = spell.calculate_effect_value(i, caster_level, 0, self.spell_rng());
                    if value > 0.0 {
                        let amount = mangos_spells::ditheru(value, self.spell_rng());
                        self.heal_unit(victim, amount);
                    }
                }
                // `Spell::EffectEnergize` +
                // `Unit::EnergizeBySpell`: direct power
                // gain (the SMSG_SPELL_ENERGIZE_LOG rides
                // the same deferral as heal's log here).
                effect::ENERGIZE => {
                    let value = spell.calculate_effect_value(i, caster_level, 0, self.spell_rng());
                    if value > 0.0
                        && let Ok(power_type) = u32::try_from(eff.misc_value)
                        && power_type < 5
                    {
                        let amount = mangos_spells::ditheru(value, self.spell_rng());
                        let max_power = self.unit_max_power(victim, power_type);
                        if max_power > 0 {
                            let current = self.unit_power(victim, power_type);
                            self.unit_set_power(
                                victim,
                                power_type,
                                (current + amount).min(max_power),
                            );
                        }
                    }
                }
                // `Spell::EffectTriggerSpell`: cast the
                // triggered spell at the effect's victim
                // through the triggered pipeline (no cost,
                // GCD, cooldown, or known checks; the
                // parent cast already left `casts`, so the
                // nested cast can start and complete here).
                effect::TRIGGER_SPELL => {
                    if eff.trigger_spell != 0 {
                        self.cast_spell(
                            caster,
                            eff.trigger_spell,
                            CastTarget::Unit(victim),
                            CastOptions {
                                triggered: true,
                                skip_known: true,
                            },
                        );
                    }
                }
                effect::APPLY_AURA | effect::APPLY_AREA_AURA_PARTY => {
                    self.apply_spell_aura(caster, spell, victim);
                    break; // all effects ride one holder
                }
                // SPELL_EFFECT_SUMMON / SUMMON_WILD /
                // SUMMON_GUARDIAN (Slice 7).
                28 | 41 | 42 if eff.misc_value > 0 => {
                    self.summon_effect(caster, caster_level, spell, victim, i);
                }
                _ => {}
            }
        }
    }

    /// `Spell::EffectSummonType` for `SUMMON` /
    /// `SUMMON_WILD` / `SUMMON_GUARDIAN`: temp-summon kind
    /// from effect + duration, scattered around the
    /// victim (caster fallback).
    fn summon_effect(
        &mut self,
        caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        victim: ObjectGuid,
        i: usize,
    ) {
        let eff = spell.effects[i];
        let value = spell.calculate_effect_value(i, caster_level, 0, self.spell_rng());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let amount = if value > 0.0 {
            (value as u32).clamp(1, 16)
        } else {
            1
        };
        let duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        #[allow(clippy::cast_sign_loss)]
        let duration_ms = if duration > 0 { duration as u32 } else { 0 };
        let kind = match eff.effect {
            41 => {
                if duration_ms == 0 {
                    7 // TEMPSUMMON_DEAD_DESPAWN
                } else {
                    11 // TEMPSUMMON_TIMED_DEATH_AND_DEAD_DESPAWN
                }
            }
            42 => {
                if duration_ms == 0 {
                    7
                } else {
                    1 // TEMPSUMMON_TIMED_OR_DEAD_DESPAWN
                }
            }
            _ => {
                if duration_ms == 0 {
                    7
                } else {
                    3 // TEMPSUMMON_TIMED_DESPAWN
                }
            }
        };
        let center = self
            .unit_position(victim)
            .or_else(|| self.unit_position(caster))
            .unwrap_or_default();
        for n in 0..amount {
            let pos = if n == 0 {
                center
            } else {
                self.random_point(center, 3.0).unwrap_or(center)
            };
            self.summon_creature(eff.misc_value as u32, pos, caster, kind, duration_ms);
        }
    }

    /// `SpellCaster::CalculateDamage` tail +
    /// `SpellNonMeleeDamageLog` + health application.
    /// `aura_holder` is Some for periodic ticks
    /// (PERIODICAURALOG instead of NONMELEE log).
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
    pub(crate) fn school_damage(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        base_damage: i32,
        _eff_index: u32,
        aura_holder: Option<u32>,
    ) {
        if base_damage <= 0 {
            return;
        }
        // Armor reduction applies to the NORMAL school
        // only (`CalcArmorReducedDamage` call site).
        let mut damage = if spell.school == school::NORMAL {
            let victim_armor = self.unit_resistance(victim, school::NORMAL);
            #[allow(clippy::cast_possible_truncation)]
            {
                mangos_spells::armor_reduced_damage(
                    base_damage as f32,
                    victim_armor,
                    self.unit_level(caster),
                ) as i32
            }
        } else {
            base_damage
        };

        // Crit: direct magic damage only (periodic ticks
        // never crit in 1.12.1). Bonus is +50% for magic
        // (`SpellCriticalDamageBonus` default arm).
        let mut crit = false;
        if aura_holder.is_none() && spell.dmg_class == dmg_class::MAGIC && damage > 0 {
            let crit_chance = self.spell_crit_percent(caster);
            let roll = self.spell_rng().frand01() * 100.0;
            if roll < crit_chance {
                crit = true;
                damage += damage / 2;
            }
        }

        // Partial resist (`CalculateDamageAbsorbAndResist`):
        // skipped for the NORMAL school, binary spells and
        // IGNORE_RESISTANCES; negative resistance chance
        // (vulnerability) still amplifies.
        let mut resisted: i32 = 0;
        if damage > 0 && spell.school != school::NORMAL {
            let can_resist =
                !spell_is_binary(spell) && spell.attributes_ex4 & ATTR_EX4_IGNORE_RESISTANCES == 0;
            let resist_chance = self.spell_resist_chance(caster, victim, spell.school, true);
            if can_resist || resist_chance < 0.0 {
                let multiplier = resist_multiplier_outcome(
                    resist_chance,
                    aura_holder.is_some(),
                    spell.id,
                    self.spell_rng(),
                );
                let raw = damage as f32 * multiplier;
                #[allow(clippy::cast_possible_truncation)]
                {
                    resisted = if raw >= 0.0 {
                        mangos_spells::ditheru(raw, self.spell_rng()) as i32
                    } else {
                        -(mangos_spells::ditheru(-raw, self.spell_rng()) as i32)
                    };
                }
                damage -= resisted;
            }
        }

        if damage <= 0 && resisted <= 0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation)]
        let final_damage = damage.max(0) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let final_resisted = resisted.max(0) as u32;
        self.send_damage_log(
            caster,
            victim,
            spell,
            final_damage,
            final_resisted,
            crit,
            aura_holder,
        );
        if final_damage > 0 {
            self.deal_damage(caster, victim, final_damage);
        }
    }

    /// Health increase clamped to max (heal log packet
    /// is a recorded slice deviation).
    pub(crate) fn heal_unit(&mut self, target: ObjectGuid, amount: u32) {
        if amount == 0 {
            return;
        }
        let current = self.unit_value_u32(target, offsets::UNIT_FIELD_HEALTH);
        let max = self.unit_value_u32(target, offsets::UNIT_FIELD_MAXHEALTH);
        if let Some(u) = self.unit_base_mut(target) {
            u.set_health((current + amount).min(max));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_damage_log(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
        damage: u32,
        resisted: u32,
        crit: bool,
        aura_holder: Option<u32>,
    ) {
        let mut payload = PacketWriter::new();
        if let Some(_holder) = aura_holder {
            // SMSG_PERIODICAURALOG:
            // packed(target) packed(caster) spellid
            // count=1 auraType amount school absorb resisted
            payload.write_bytes(&Self::packed_guid_bytes(victim));
            payload.write_bytes(&Self::packed_guid_bytes(caster));
            payload.write_u32_le(spell.id);
            payload.write_u32_le(1);
            payload.write_u32_le(aura_type::PERIODIC_DAMAGE);
            payload.write_u32_le(damage);
            payload.write_u32_le(school::mask(spell.school));
            payload.write_u32_le(0); // absorb
            payload.write_u32_le(resisted);
            let packet = WorldPacket::with_body(op::SMSG_PERIODICAURALOG, payload.into_vec());
            self.broadcast_movement(caster, &packet);
        } else {
            payload.write_bytes(&Self::packed_guid_bytes(victim));
            payload.write_bytes(&Self::packed_guid_bytes(caster));
            payload.write_u32_le(spell.id);
            payload.write_u32_le(damage);
            payload.write_u32_le(0); // overkill
            #[allow(clippy::cast_possible_truncation)]
            payload.write_u8(spell.school as u8);
            payload.write_u32_le(0); // absorb
            payload.write_u32_le(resisted);
            payload.write_u8(0); // physical log?
            payload.write_u32_le(0); // blocked
            payload.write_u16_le(if crit { SPELL_HIT_TYPE_CRIT } else { 0 });
            payload.write_u8(0); // debug output
            let packet =
                WorldPacket::with_body(op::SMSG_SPELLNONMELEEDAMAGELOG, payload.into_vec());
            self.broadcast_movement(caster, &packet);
        }
    }

    /// Health application + combat flag + death
    /// (`Unit::DealDamage` subset).
    pub(crate) fn deal_damage(&mut self, attacker: ObjectGuid, victim: ObjectGuid, damage: u32) {
        if damage == 0 {
            return;
        }
        // `SetInvincibilityHpThreshold` floor (Slice 6b).
        let mut damage = damage;
        if victim.is_creature()
            && let Some(threshold) = self
                .ai
                .get(&victim)
                .map(|ai| ai.invincibility_hp)
                .filter(|t| *t > 0)
        {
            let current = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
            if current <= threshold {
                return;
            }
            damage = damage.min(current - threshold);
            if damage == 0 {
                return;
            }
        }
        let now = self.now_ms;
        for guid in [attacker, victim] {
            if let Some(u) = self.unit_base_mut(guid) {
                let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
                u.values()
                    .set_u32(offsets::UNIT_FIELD_FLAGS, flags | UNIT_FLAG_IN_COMBAT);
                u.last_combat_ms = now;
            }
        }
        let current = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
        let new_health = current.saturating_sub(damage);
        if let Some(u) = self.unit_base_mut(victim) {
            u.set_health(new_health);
        }
        if new_health == 0 {
            self.kill_unit(victim, Some(attacker));
            return;
        }
        // Slice 6: `AttackedBy` — creature victims take
        // threat and engage (damage threat is 1:1 in the
        // no-modifier subset).
        if victim.is_creature() && attacker != victim && self.unit_is_alive(attacker) {
            let kind_basic = self
                .creatures
                .get(&victim)
                .is_some_and(Self::ai_kind_is_basic);
            if kind_basic {
                if !self.ai_in_combat(victim) {
                    self.enter_combat(victim, attacker);
                }
                #[allow(clippy::cast_precision_loss)]
                self.add_threat(victim, attacker, damage as f32);
            }
        }
    }

    /// Death: state, cleanup, respawn scheduling
    /// (`Kill`/`setDeathState(JUST_DIED)` subset).
    pub(crate) fn kill_unit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let is_creature = victim.is_creature();
        if let Some(u) = self.unit_base_mut(victim) {
            if !u.is_alive() {
                return;
            }
            u.death_state = if is_creature {
                DeathState::JustDied
            } else {
                DeathState::Dead
            };
            u.set_health(0);
        }
        // EventAI death/kill hooks run while the AI
        // state still exists (`JustDied`/`KilledUnit`).
        self.eventai_on_death(victim, killer);
        if is_creature {
            self.instance_on_creature_death(victim);
            if let Some(summoner) = self.summons.get(&victim).map(|st| st.summoner)
                && summoner.is_creature()
            {
                self.eventai_process_type(
                    summoner,
                    mangos_scripts_engine::event_type::SUMMONED_JUST_DIED,
                    Some(victim),
                );
            }
        }
        self.casts.remove(&victim);
        self.attack_stop(victim);
        self.ai.remove(&victim);
        self.remove_all_auras(victim);
        self.stop_moving(victim);
        if let Some(mut master) = self.motion.remove(&victim) {
            master.clear(self, victim);
            self.motion.entry(victim).or_insert(master);
        }
        if is_creature {
            if self.summons.contains_key(&victim) {
                // Temp summons never hit the DB respawn
                // path: the corpse is removed by the
                // summon lifecycle (Slice 7).
                let state = self.summons.remove(&victim);
                let corpse_delay = match state.as_ref().map(|st| st.kind) {
                    Some(5) => 0,
                    Some(_) => state.map_or(60_000, |st| u64::from(st.despawn_delay_ms).max(1)),
                    None => 60_000,
                };
                self.despawns_due
                    .insert(victim, (u64::from(self.now_ms) + corpse_delay, u32::MAX));
            } else {
                let delay = self.creatures.get(&victim).map_or(0, |c| c.respawn_delay);
                let when = self.now_unix + u64::from(delay);
                self.respawns_due.insert(victim, when);
                self.events.push(MapEvent::StoreCreatureRespawn {
                    guid_low: victim.counter(),
                    respawn_time: when,
                    map_id: self.map_id,
                    instance_id: self.creature_respawn_key(victim.counter()),
                });
            }
        }
        self.events.push(MapEvent::UnitDied {
            guid: victim,
            killer,
        });
    }

    /// Respawn due creatures (`Creature::Respawn`
    /// re-materialization subset).
    pub(crate) fn update_respawns(&mut self) {
        let now = self.now_unix;
        let due: Vec<ObjectGuid> = self
            .respawns_due
            .iter()
            .filter(|&(_, &when)| when <= now)
            .map(|(&g, _)| g)
            .collect();
        for guid in due {
            self.respawns_due.remove(&guid);
            // `spawn_creature` re-materializes from the DB
            // spawn row, replaces the corpse in the
            // creature map and re-initializes motion.
            self.spawn_creature(guid.counter(), &RespawnTimes::default());
            self.events.push(MapEvent::ClearCreatureRespawn {
                guid_low: guid.counter(),
                instance_id: self.creature_respawn_key(guid.counter()),
            });
        }
    }

    // ------------------------------------------------ auras

    /// Read-only holder snapshot (tests/inspection).
    #[must_use]
    pub fn aura_holders(&self, guid: ObjectGuid) -> Vec<AuraHolder> {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.auras.clone();
        }
        self.creatures
            .get(&guid)
            .map_or_else(Vec::new, |c| c.unit.auras.clone())
    }

    /// Savable aura rows for one player (`SaveAura`
    /// filter, `Player.cpp:16503`): passive and
    /// channeled holders and not-own single-target
    /// holders are skipped, as are the sight/charm
    /// aura types. Deviations: the
    /// leave/enter-world interrupt-flag gate needs
    /// `AuraInterruptFlags`, which the spell stores do
    /// not parse yet; cast-item guid and proc charges
    /// are unmodeled and written 0; area-aura holders
    /// are unmodeled (no `IsAreaAura`).
    #[must_use]
    pub fn player_savable_auras(&self, guid: ObjectGuid) -> Vec<SavableAura> {
        self.savable_auras_from(guid, self.aura_holders(guid))
    }

    /// Savable auras for a player currently outside a
    /// map (forced logout save during far teleport).
    #[must_use]
    pub fn savable_auras_for_player(
        &self,
        player: &mangos_entities::player::Player,
    ) -> Vec<SavableAura> {
        self.savable_auras_from(player.unit.guid(), player.unit.auras.clone())
    }

    fn savable_auras_from(&self, guid: ObjectGuid, holders: Vec<AuraHolder>) -> Vec<SavableAura> {
        // `SPELL_ATTR_PASSIVE`, `SPELL_ATTR_EX_CHANNELED_*`.
        const ATTR_PASSIVE: u32 = 0x0000_0040;
        const ATTR_EX_CHANNELED: u32 = 0x0000_0004 | 0x4000_0000;
        let mut rows = Vec::new();
        for holder in holders {
            let Some(spell) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if spell.attributes & ATTR_PASSIVE != 0 || spell.attributes_ex & ATTR_EX_CHANNELED != 0
            {
                continue;
            }
            let single_target = spell
                .effects
                .iter()
                .any(|e| e.effect != 0 && e.implicit_target_a == 1);
            if single_target && holder.caster != guid {
                continue;
            }
            let mut base_points = [0.0f32; 3];
            let mut periodic_time = [0i32; 3];
            let mut mask = 0u8;
            let mut skipped = false;
            for (i, eff) in spell.effects.iter().enumerate() {
                if matches!(eff.apply_aura_name, 1 | 2 | 6 | 76 | 177) {
                    skipped = true;
                    break;
                }
                if let Some(state) = &holder.effects[i] {
                    #[allow(clippy::cast_precision_loss)]
                    {
                        base_points[i] = state.amount as f32;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    {
                        periodic_time[i] = state.period_ms as i32;
                    }
                    mask |= 1u8 << i;
                }
            }
            if skipped || mask == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_wrap)]
            rows.push(SavableAura {
                caster_guid: holder.caster.raw(),
                item_guid: 0,
                spell: holder.spell_id,
                stacks: holder.stack as i32,
                charges: 0,
                base_points,
                periodic_time,
                max_duration: holder.max_duration_ms,
                duration: holder.duration_ms,
                effect_index_mask: mask,
            });
        }
        rows
    }

    /// Live cooldown rows for one player
    /// (`_SaveSpellCooldowns`): split spell/category
    /// keys merge into the fork's per-spell
    /// `CooldownData` shape; expiries convert from map
    /// time to epoch seconds. Item cooldowns are
    /// unmodeled (`item_id` 0).
    #[must_use]
    pub fn player_cooldown_rows(&self, guid: ObjectGuid, now_unix: i64) -> Vec<CooldownSnapshot> {
        let to_epoch = |expiry_ms: u32| -> i64 {
            now_unix + i64::from(expiry_ms.saturating_sub(self.now_ms)) / 1000
        };
        let mut rows = Vec::new();
        for ((caster, key), expiry) in &self.cooldowns {
            if *caster != guid || *expiry <= self.now_ms || *key & 0x8000_0000 != 0 {
                continue;
            }
            let category = self.stores.spells.get(*key).map_or(0, |s| s.category);
            let category_expire_time = if category != 0 {
                self.cooldowns
                    .get(&(guid, category | 0x8000_0000))
                    .filter(|e| **e > self.now_ms)
                    .map_or(0, |e| to_epoch(*e))
            } else {
                0
            };
            rows.push(CooldownSnapshot {
                spell: *key,
                spell_expire_time: to_epoch(*expiry),
                category,
                category_expire_time,
                item_id: 0,
            });
        }
        rows.sort_by_key(|r| r.spell);
        rows
    }

    fn send_aura_duration(&mut self, target: ObjectGuid, slot: u8, duration_ms: i32) {
        if slot >= MAX_AURAS || duration_ms < 0 {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u8(slot);
        #[allow(clippy::cast_possible_truncation)]
        payload.write_u32_le(duration_ms as u32);
        let packet = WorldPacket::with_body(op::SMSG_UPDATE_AURA_DURATION, payload.into_vec());
        self.send_to_player(target, packet);
    }

    /// Slot update-field writes (`SetAura`/`SetAuraFlag`/
    /// `SetAuraLevel`/`UpdateAuraApplication`).
    fn write_aura_slot(unit: &mut UnitBase, holder: &AuraHolder, remove: bool) {
        let slot = holder.slot;
        if slot == NULL_AURA_SLOT || slot >= MAX_AURAS {
            return;
        }
        let values = unit.values();
        values.set_u32(
            offsets::UNIT_FIELD_AURA + u16::from(slot),
            if remove { 0 } else { holder.spell_id },
        );
        let flags_index = offsets::UNIT_FIELD_AURAFLAGS + u16::from(slot >> 3);
        let shift = u32::from((slot & 7) << 2);
        let current = values.get_u32(flags_index);
        let cleared = current & !(0xFu32 << shift);
        let next = if remove {
            cleared
        } else {
            cleared | (holder.effect_flags() << shift)
        };
        values.set_u32(flags_index, next);
        #[allow(clippy::cast_possible_truncation)]
        let level_byte = holder.caster_level as u8;
        values.set_byte(
            offsets::UNIT_FIELD_AURALEVELS + u16::from(slot / 4),
            slot % 4,
            if remove { 0 } else { level_byte },
        );
        #[allow(clippy::cast_possible_truncation)]
        let stack_byte = (holder.stack.saturating_sub(1)).min(254) as u8;
        values.set_byte(
            offsets::UNIT_FIELD_AURAAPPLICATIONS + u16::from(slot / 4),
            slot % 4,
            if remove { 0 } else { stack_byte },
        );
    }

    /// `HandleAttackPowerModifier(MELEE, flat)` field
    /// write subset (no pct mods in the slice).
    #[allow(clippy::cast_precision_loss)]
    fn apply_attack_power_mod(unit: &mut UnitBase, positive: bool, amount: i32, apply: bool) {
        let bucket = usize::from(!positive);
        if apply {
            unit.attack_power_mods[bucket] += amount as f32;
        } else {
            unit.attack_power_mods[bucket] -= amount as f32;
        }
        for i in 0..2 {
            #[allow(clippy::cast_possible_truncation)]
            let value = unit.attack_power_mods[i] as i16;
            #[allow(clippy::cast_possible_truncation)]
            unit.values()
                .set_i16(offsets::UNIT_FIELD_ATTACK_POWER_MODS, i as u8, value);
        }
    }

    fn aura_effect_apply(unit: &mut UnitBase, holder: &AuraHolder, apply: bool) {
        for eff in holder.effects.iter().flatten() {
            if eff.aura_type == aura_type::MOD_ATTACK_POWER {
                Self::apply_attack_power_mod(unit, holder.positive, eff.amount, apply);
            }
            // Other aura types are state-only in the
            // slice (periodic handled by the tick loop;
            // stat/speed/root/stun recorded as pending).
        }
    }

    /// Add a holder: slot allocation (positive slots
    /// 0..32, negative 32..48 — `GetAuraSlot` ranges),
    /// field writes, modifier application.
    fn add_aura_holder(&mut self, victim: ObjectGuid, mut holder: AuraHolder) {
        let slot = {
            let range = if holder.positive {
                0..MAX_POSITIVE_AURAS
            } else {
                MAX_POSITIVE_AURAS..MAX_AURAS
            };
            let mut found = NULL_AURA_SLOT;
            if let Some(unit) = self.unit_base_mut(victim) {
                let values = unit.values();
                for s in range {
                    if values.get_u32(offsets::UNIT_FIELD_AURA + u16::from(s)) == 0 {
                        found = s;
                        break;
                    }
                }
            }
            found
        };
        holder.slot = slot;
        let duration_packet = (slot, holder.duration_ms);
        let has_ap = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
        if let Some(unit) = self.unit_base_mut(victim) {
            Self::aura_effect_apply(unit, &holder, true);
            Self::write_aura_slot(unit, &holder, false);
            unit.auras.push(holder);
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
        self.send_aura_duration(victim, duration_packet.0, duration_packet.1);
    }

    fn remove_aura_at(&mut self, victim: ObjectGuid, index: usize) {
        let mut has_ap = false;
        if let Some(unit) = self.unit_base_mut(victim) {
            if index >= unit.auras.len() {
                return;
            }
            let holder = unit.auras.remove(index);
            has_ap = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
            Self::aura_effect_apply(unit, &holder, false);
            Self::write_aura_slot(unit, &holder, true);
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
    }

    pub(crate) fn remove_all_auras(&mut self, victim: ObjectGuid) {
        loop {
            let len = self.unit_base_mut(victim).map_or(0, |u| u.auras.len());
            if len == 0 {
                return;
            }
            self.remove_aura_at(victim, len - 1);
        }
    }

    /// `CMSG_CANCEL_AURA` (`RemoveAurasDueToSpell` with
    /// the cancelable gate).
    pub fn cancel_aura(&mut self, victim: ObjectGuid, spell_id: u32) {
        loop {
            let found = self.unit_base_mut(victim).and_then(|u| {
                u.auras
                    .iter()
                    .position(|h| h.spell_id == spell_id && h.cancelable)
            });
            match found {
                Some(i) => self.remove_aura_at(victim, i),
                None => return,
            }
        }
    }

    /// `Spell::EffectApplyAura` subset: build the holder
    /// from every `APPLY_AURA` effect (amounts snapshot at
    /// apply, 1.12.1 rule) or refresh an existing
    /// same-caster application.
    /// Direct aura application by spell id (spawn auras,
    /// `AddAura` at materialization).
    pub(crate) fn apply_aura_by_id(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        victim: ObjectGuid,
    ) {
        if let Some(spell) = self.stores.spells.get(spell_id).cloned() {
            self.apply_spell_aura(caster, &spell, victim);
        }
    }

    fn apply_spell_aura(&mut self, caster: ObjectGuid, spell: &SpellEntry, victim: ObjectGuid) {
        let tables = &self.stores.spells.tables;
        let duration = spell.calculate_duration_ms(tables);
        let duration = if duration == 0 { -1 } else { duration };

        let refreshed = self.unit_base_mut(victim).and_then(|unit| {
            unit.auras
                .iter_mut()
                .find(|h| h.spell_id == spell.id && h.caster == caster)
                .map(|h| {
                    h.duration_ms = h.max_duration_ms;
                    (h.slot, h.duration_ms)
                })
        });
        if let Some((slot, dur)) = refreshed {
            self.send_aura_duration(victim, slot, dur);
            return;
        }

        let positive = spell.is_positive_spell();
        let cancelable = positive && spell.attributes & ATTR_NO_AURA_CANCEL == 0;
        let caster_level = self.unit_level(caster);
        let mut holder = AuraHolder::new(
            spell.id,
            caster,
            caster_level,
            duration,
            positive,
            cancelable,
        );
        for i in 0..3 {
            let eff = spell.effects[i];
            if eff.effect != effect::APPLY_AURA && eff.effect != effect::APPLY_AREA_AURA_PARTY {
                continue;
            }
            if eff.apply_aura_name == aura_type::NONE {
                continue;
            }
            let value = spell.calculate_effect_value(i, caster_level, 0, self.spell_rng());
            #[allow(clippy::cast_possible_truncation)]
            let amount = value as i32;
            let periodic = matches!(
                eff.apply_aura_name,
                aura_type::PERIODIC_DAMAGE
                    | aura_type::PERIODIC_HEAL
                    | aura_type::PERIODIC_ENERGIZE
                    | aura_type::PERIODIC_DAMAGE_PERCENT
            );
            let period_ms = if periodic { eff.amplitude } else { 0 };
            #[allow(clippy::cast_possible_wrap)]
            let timer = period_ms as i32;
            holder.effects[i] = Some(AuraEffectState {
                aura_type: eff.apply_aura_name,
                amount,
                base_points: amount,
                period_ms,
                periodic_timer: timer,
                periodic_ticks: 0,
                misc_value: eff.misc_value,
            });
        }
        if holder.effects.iter().all(Option::is_none) {
            return;
        }
        let applied_types: Vec<u32> = holder
            .effects
            .iter()
            .flatten()
            .map(|e| e.aura_type)
            .collect();
        self.add_aura_holder(victim, holder);
        if victim.is_creature() {
            for aura_type_id in applied_types {
                self.eventai_on_hit_by_aura(victim, aura_type_id, caster);
                if !self.creatures.contains_key(&victim) {
                    break;
                }
            }
        }
    }

    /// Login: apply the deferred `character_aura` rows
    /// (`Player::_LoadAuras` subset — amounts/durations
    /// from the saved row, effects restricted to the
    /// slice's aura types by construction of the tick
    /// loop).
    pub fn apply_saved_auras(&mut self, player_guid: ObjectGuid) {
        let rows = self
            .players
            .get(&player_guid)
            .map_or_else(Vec::new, |e| e.player.deferred.auras.clone());
        for row in rows {
            let Some(spell) = self.stores.spells.get(row.spell).cloned() else {
                continue;
            };
            let caster = ObjectGuid::from_raw(row.caster_guid);
            let positive = spell.is_positive_spell();
            let cancelable = positive && spell.attributes & ATTR_NO_AURA_CANCEL == 0;
            let caster_level = if caster == player_guid {
                self.unit_level(player_guid)
            } else {
                0
            };
            let mut holder = AuraHolder::new(
                row.spell,
                caster,
                caster_level,
                row.duration,
                positive,
                cancelable,
            );
            holder.max_duration_ms = row.max_duration;
            #[allow(clippy::cast_possible_truncation)]
            {
                holder.stack = row.stacks.max(1) as u32;
            }
            for i in 0..3 {
                if row.effect_index_mask & (1u8 << i) == 0 {
                    continue;
                }
                let eff = spell.effects[i];
                #[allow(clippy::cast_possible_truncation)]
                let amount = row.base_points[i] as i32;
                let period_ms = if row.periodic_time[i] > 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        row.periodic_time[i] as u32
                    }
                } else {
                    0
                };
                holder.effects[i] = Some(AuraEffectState {
                    aura_type: eff.apply_aura_name,
                    amount,
                    base_points: amount,
                    period_ms,
                    #[allow(clippy::cast_possible_wrap)]
                    periodic_timer: period_ms as i32,
                    periodic_ticks: 0,
                    misc_value: eff.misc_value,
                });
            }
            if holder.effects.iter().all(Option::is_none) {
                continue;
            }
            self.add_aura_holder(player_guid, holder);
        }
    }

    /// Advance all in-progress casts (`Spell::update`).
    pub(crate) fn update_casts(&mut self, diff_ms: u32) {
        let casters: Vec<ObjectGuid> = self.casts.keys().copied().collect();
        for caster in casters {
            self.update_cast(caster, diff_ms);
        }
    }

    /// Holder durations + periodic ticks
    /// (`SpellAuraHolder::Update` / `Aura::Update`).
    pub(crate) fn update_auras(&mut self, diff_ms: u32) {
        let targets: Vec<ObjectGuid> = self
            .players
            .keys()
            .chain(self.creatures.keys())
            .copied()
            .collect();
        let mut ticks: Vec<PeriodicTick> = Vec::new();
        let mut expired: Vec<(ObjectGuid, usize)> = Vec::new();
        for target in &targets {
            let Some(unit) = self.unit_base_mut(*target) else {
                continue;
            };
            if !unit.is_alive() {
                continue;
            }
            for (index, holder) in unit.auras.iter_mut().enumerate() {
                for (eff_index, eff) in holder.effects.iter_mut().enumerate() {
                    let Some(state) = eff else { continue };
                    if !state.is_periodic() {
                        continue;
                    }
                    state.periodic_timer -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                    if state.periodic_timer <= 0 {
                        #[allow(clippy::cast_possible_wrap)]
                        let period = state.period_ms as i32;
                        if state.periodic_timer < -period {
                            state.periodic_timer = 0;
                        } else {
                            state.periodic_timer += period;
                        }
                        state.periodic_ticks += 1;
                        #[allow(clippy::cast_possible_truncation)]
                        let effect_idx = eff_index as u32;
                        ticks.push(PeriodicTick {
                            target: *target,
                            caster: holder.caster,
                            spell_id: holder.spell_id,
                            aura_type: state.aura_type,
                            amount: state.amount,
                            effect_index: effect_idx,
                            misc_value: state.misc_value,
                        });
                    }
                }
                if holder.duration_ms > 0 {
                    holder.duration_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                    if holder.duration_ms <= 0 {
                        expired.push((*target, index));
                    }
                }
            }
        }
        self.process_periodic_ticks(ticks);
        // Remove expired holders back-to-front per unit.
        expired.sort_by_key(|&(_, index)| std::cmp::Reverse(index));
        for (target, index) in expired {
            self.remove_aura_at(target, index);
        }
    }

    fn process_periodic_ticks(&mut self, ticks: Vec<PeriodicTick>) {
        for tick in ticks {
            match tick.aura_type {
                aura_type::PERIODIC_DAMAGE => {
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    // Caster must still exist on the map
                    // for the damage attribution.
                    if self.unit_position(tick.caster).is_none() {
                        continue;
                    }
                    self.school_damage(
                        tick.caster,
                        &spell,
                        tick.target,
                        tick.amount,
                        tick.effect_index,
                        Some(tick.spell_id),
                    );
                }
                aura_type::PERIODIC_HEAL => {
                    if tick.amount > 0 {
                        #[allow(clippy::cast_possible_truncation)]
                        self.heal_unit(tick.target, tick.amount as u32);
                    }
                }
                // `Aura::PeriodicTick` PERIODIC_ENERGIZE
                // (`SpellAuras.cpp:6158`): gain capped at
                // max power. The periodic aura log packet
                // and 0.5x threat assist ride the same
                // deferral as periodic heal's here.
                aura_type::PERIODIC_ENERGIZE => {
                    if tick.amount <= 0 {
                        continue;
                    }
                    let Ok(power_type) = u32::try_from(tick.misc_value) else {
                        continue;
                    };
                    if power_type >= 5 {
                        continue;
                    }
                    let max_power = self.unit_max_power(tick.target, power_type);
                    if max_power == 0 {
                        continue;
                    }
                    let current = self.unit_power(tick.target, power_type);
                    #[allow(clippy::cast_sign_loss)]
                    let gain = (tick.amount as u32).min(max_power - current);
                    if gain > 0 {
                        self.unit_set_power(tick.target, power_type, current + gain);
                    }
                }
                aura_type::PERIODIC_DAMAGE_PERCENT => {
                    let max_health =
                        self.unit_value_u32(tick.target, offsets::UNIT_FIELD_MAXHEALTH);
                    #[allow(clippy::cast_possible_truncation)]
                    let amount =
                        (u64::from(max_health) * tick.amount.unsigned_abs() as u64 / 100) as i32;
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    if self.unit_position(tick.caster).is_none() {
                        continue;
                    }
                    self.school_damage(
                        tick.caster,
                        &spell,
                        tick.target,
                        amount,
                        tick.effect_index,
                        Some(tick.spell_id),
                    );
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------ hit/resist/crit math

    /// `SpellCaster::GetSpellResistChance` (fraction;
    /// no target-resistance penetration auras in the
    /// slice). Build 5875 takes the >1.8.4 clamp.
    #[allow(clippy::cast_precision_loss)]
    fn spell_resist_chance(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        school_id: u32,
        innate_resists: bool,
    ) -> f32 {
        let base_resistance = self.unit_resistance(victim, school_id) as f32;
        let mut chance = base_resistance;
        if chance < 0.0 && base_resistance >= 0.0 {
            chance = 0.0;
        }
        if chance < 0.0 {
            // Magic vulnerability: skill = max(level*5, 100).
            let skill = (self.unit_level(victim) * 5).max(100) as f32;
            chance /= skill;
            return chance.max(-0.75);
        }
        let caster_level = self.unit_level(caster).max(1);
        if innate_resists && !victim.is_player() {
            #[allow(clippy::cast_possible_wrap)]
            let level_diff = self.unit_level(victim) as i32 - self.unit_level(caster) as i32;
            #[allow(clippy::cast_possible_truncation)]
            {
                chance += ((8.0 * level_diff as f32) * caster_level as f32 / 63.0) as i32 as f32;
            }
        }
        chance *= 0.15 / caster_level as f32;
        chance.clamp(0.0, 0.75)
    }

    pub(crate) fn unit_resistance(&self, guid: ObjectGuid, school_id: u32) -> i32 {
        let index = offsets::UNIT_FIELD_RESISTANCES + school_id as u16;
        i32::from_ne_bytes(self.unit_value_u32(guid, index).to_ne_bytes())
    }

    /// `MagicSpellHitChance` returned as a percent
    /// (no spell mods / hit-chance auras in the slice).
    #[allow(clippy::cast_precision_loss)]
    fn magic_hit_chance(&self, caster: ObjectGuid, victim: ObjectGuid, spell: &SpellEntry) -> f32 {
        if spell.attributes_ex3 & ATTR_EX3_ALWAYS_HIT != 0 {
            return 100.0;
        }
        #[allow(clippy::cast_possible_wrap)]
        let level_dif = self.unit_level(victim) as i32 - self.unit_level(caster) as i32;
        let l_chance: i32 = if victim.is_player() { 7 } else { 11 };
        let mut hit = if level_dif < 3 {
            (96 - level_dif) as f32
        } else {
            (94 - (level_dif - 2) * l_chance) as f32
        };
        if hit < 22.0 {
            hit = 22.0;
        }
        if spell_is_binary(spell) {
            hit *= 1.0 - self.spell_resist_chance(caster, victim, spell.school, false);
        }
        // C++ returns int hundredths clamped [100, 9900].
        #[allow(clippy::cast_possible_truncation)]
        let hundredths = (hit * 100.0) as i32;
        hundredths.clamp(100, 9900) as f32 / 100.0
    }

    /// `MeleeSpellMissChance` complement (weapon/defense
    /// skills at their level maxima — no skill system in
    /// the slice; dodge/parry/block not rolled, recorded).
    #[allow(clippy::cast_precision_loss)]
    fn melee_spell_hit_chance(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
    ) -> f32 {
        if spell.attributes_ex3 & ATTR_EX3_ALWAYS_HIT != 0 {
            return 100.0;
        }
        #[allow(clippy::cast_possible_wrap)]
        let skill_diff =
            (self.unit_level(caster) * 5) as i32 - (self.unit_level(victim) * 5) as i32;
        let mut miss = if victim.is_player() {
            5.0 - skill_diff as f32 * 0.04
        } else if skill_diff < -10 {
            5.0 - skill_diff as f32 * 0.2
        } else {
            5.0 - skill_diff as f32 * 0.1
        };
        let victim_level = self.unit_level(victim);
        if !victim.is_player() && victim_level < 10 {
            miss *= victim_level as f32 / 10.0;
        }
        100.0 - miss.clamp(0.0, 60.0)
    }

    /// `GetSpellCritFromIntellect` (no aura mods;
    /// creatures flat 5%).
    #[allow(clippy::cast_precision_loss)]
    fn spell_crit_percent(&self, caster: ObjectGuid) -> f32 {
        // crit_data {base, rate0, rate1} per class
        // (`Unit.cpp:2833`).
        const CRIT_DATA: [[f32; 3]; 12] = [
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 10.0],
            [3.70, 14.77, 0.65],
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 10.0],
            [2.97, 10.03, 0.82],
            [0.0, 0.0, 10.0],
            [3.54, 11.51, 0.80],
            [3.70, 14.77, 0.65],
            [3.18, 11.30, 0.82],
            [0.0, 0.0, 10.0],
            [3.33, 12.41, 0.79],
        ];
        if caster.is_player() {
            let class = self.unit_value_u32(caster, offsets::UNIT_FIELD_BYTES_0) >> 8 & 0xFF;
            let idx = (class as usize).min(11);
            let intellect = self.unit_stat(caster, 3);
            let level = self.unit_level(caster) as f32;
            let ratio = CRIT_DATA[idx][1] + CRIT_DATA[idx][2] * level;
            (CRIT_DATA[idx][0] + intellect / ratio).max(0.0)
        } else {
            5.0
        }
    }

    fn unit_stat(&self, guid: ObjectGuid, stat: u16) -> f32 {
        let index = offsets::UNIT_FIELD_STAT0 + stat;
        f32::from_bits(self.unit_value_u32(guid, index))
    }
}

fn spell_meleedamage(damage_class: u32) -> bool {
    damage_class == dmg_class::MELEE || damage_class == dmg_class::RANGED
}

fn spell_is_positive_for_target(spell: &SpellEntry) -> bool {
    spell.is_positive_spell()
}

/// `SpellInternal::IsBinary` (`SpellMgr.cpp:3173`):
/// magic-class, non-physical spells carrying a
/// no-damage control aura (or interrupt/knockback),
/// plus the two hardcoded ids.
fn spell_is_binary(spell: &SpellEntry) -> bool {
    if spell.dmg_class != dmg_class::MAGIC || spell.school == school::NORMAL {
        return false;
    }
    for eff in &spell.effects {
        match eff.effect {
            EFFECT_INTERRUPT_CAST | EFFECT_KNOCK_BACK => return true,
            effect::APPLY_AURA => match eff.apply_aura_name {
                aura_type::MOD_DECREASE_SPEED
                | aura_type::MOD_STUN
                | aura_type::MOD_ROOT
                | MOD_FEAR
                | MOD_PACIFY
                | MOD_SILENCE
                | MOD_DISARM
                | MOD_RESISTANCE
                | MOD_DAMAGE_TAKEN => return true,
                _ => {}
            },
            _ => {}
        }
    }
    matches!(spell.id, 26143 | 26478)
}

/// `resistValues[]` verbatim (`Unit.cpp:1799`):
/// {resist100, resist75, resist50, resist25, resist0,
/// chanceResist}. Row for resistance 70 sums to 89 in
/// C++ — kept faithful.
const RESIST_VALUES: [[f32; 6]; 31] = [
    [0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
    [0.0, 0.0, 2.0, 6.0, 92.0, 3.0],
    [0.0, 1.0, 4.0, 12.0, 84.0, 5.0],
    [0.0, 1.0, 5.0, 18.0, 76.0, 8.0],
    [0.0, 1.0, 7.0, 23.0, 69.0, 10.0],
    [0.0, 2.0, 9.0, 28.0, 61.0, 13.0],
    [0.0, 2.0, 11.0, 33.0, 54.0, 15.0],
    [0.0, 2.0, 13.0, 37.0, 37.0, 18.0],
    [0.0, 3.0, 15.0, 41.0, 41.0, 20.0],
    [1.0, 3.0, 17.0, 46.0, 36.0, 23.0],
    [1.0, 4.0, 19.0, 47.0, 29.0, 25.0],
    [1.0, 5.0, 21.0, 48.0, 24.0, 28.0],
    [1.0, 6.0, 24.0, 49.0, 20.0, 30.0],
    [1.0, 8.0, 28.0, 47.0, 17.0, 33.0],
    [1.0, 9.0, 33.0, 43.0, 14.0, 35.0],
    [1.0, 11.0, 37.0, 39.0, 12.0, 38.0],
    [1.0, 13.0, 41.0, 35.0, 10.0, 40.0],
    [1.0, 16.0, 45.0, 30.0, 8.0, 43.0],
    [1.0, 18.0, 48.0, 26.0, 7.0, 45.0],
    [2.0, 20.0, 48.0, 24.0, 6.0, 48.0],
    [4.0, 23.0, 48.0, 21.0, 4.0, 50.0],
    [5.0, 25.0, 47.0, 19.0, 3.0, 53.0],
    [7.0, 28.0, 45.0, 17.0, 2.0, 55.0],
    [9.0, 31.0, 43.0, 16.0, 2.0, 58.0],
    [11.0, 34.0, 40.0, 14.0, 1.0, 60.0],
    [13.0, 37.0, 37.0, 12.0, 1.0, 62.0],
    [15.0, 41.0, 33.0, 10.0, 1.0, 65.0],
    [18.0, 44.0, 29.0, 8.0, 1.0, 68.0],
    [20.0, 48.0, 25.0, 7.0, 1.0, 70.0],
    [23.0, 51.0, 20.0, 5.0, 1.0, 73.0],
    [25.0, 55.0, 16.0, 3.0, 1.0, 75.0],
];

/// `RollMagicResistanceMultiplierOutcomeAgainst`
/// (`Unit.cpp:2536`): fraction in, resisted fraction
/// out (negative = vulnerability amplification). `DoT`s
/// use a tenth of the chance except the four named
/// spells; the 100% bucket folds into 75%.
#[allow(clippy::cast_precision_loss)]
fn resist_multiplier_outcome(
    resistance_chance: f32,
    is_dot: bool,
    spell_id: u32,
    rng: &mut dyn mangos_spells::SpellRng,
) -> f32 {
    let negative = resistance_chance < 0.0;
    let mut chance = resistance_chance.abs() * 100.0;
    if is_dot && !matches!(spell_id, 23461 | 24818 | 25812 | 28531) {
        chance *= 0.1;
    }
    let mut idx = 30usize;
    for (i, row) in RESIST_VALUES.iter().enumerate().skip(1) {
        if row[5] >= chance {
            idx = i;
            break;
        }
    }
    let prev = RESIST_VALUES[idx - 1];
    let next = RESIST_VALUES[idx];
    let coeff = (chance - prev[5]) / (next[5] - prev[5]);
    let interp = |j: usize| prev[j] + (next[j] - prev[j]) * coeff;
    let (r100, r75, r50, r25) = (interp(0), interp(1), interp(2), interp(3));
    let ran = rng.irand(0, 99) as f32;
    let resist_cnt = if ran < r100 + r75 {
        0.75
    } else if ran < r100 + r75 + r50 {
        0.5
    } else if ran < r100 + r75 + r50 + r25 {
        0.25
    } else {
        0.0
    };
    if negative { -resist_cnt } else { resist_cnt }
}
