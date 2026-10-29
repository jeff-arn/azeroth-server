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
//! (speed > 0 spells fail), spell mods are implemented
//! for the wired ops (damage/dot, duration, range,
//! crit chance/bonus, casting time, cost, cooldown,
//! GCD, pushback resist, spell bonus damage; remaining
//! ops pending), spell bonus Done/Taken implemented
//! for magic-class damage/heals (melee-bonus audit,
//! totem delegation, wand equipped gates pending), no
//! dodge/parry/block on melee spells, no
//! LOS/facing checks (vmaps not wired), cast pushback
//! and single-target channeling (with
//! `DelayedChannel`) are implemented
//! (persistent-area/dynamic-object channels pending),
//! no combos/area/container targets, corpse stays
//! until respawn (no decay), player death is
//! state-only (no ghost flow).

use mangos_entities::aura::{
    AuraEffectState, AuraHolder, MAX_AURAS, MAX_POSITIVE_AURAS, NULL_AURA_SLOT,
};
use mangos_entities::aura::{CooldownSnapshot, SavableAura};
use mangos_entities::unit::{
    DeathState, MAX_STATS, StatModKind, UnitBase, apply_percent_mod_float_var,
};

/// `STAT_STAMINA` as an aura miscvalue.
const STAT_STAMINA_I32: i32 = 2;

/// `MAP_HALFSIZE - 0.5` clamp of `NormalizeMapCoord`
/// (`GridDefines.h:169`); fork `SIZE_OF_GRIDS` is
/// the f32 literal 533.33333.
#[allow(clippy::excessive_precision)]
const MAX_MAP_COORD: f32 = 32.0 * 533.333_33 - 0.5;

/// Druid forms whose apply/remove casts 9033
/// (`HandleAuraModShapeshift` form switch).
const DRUID_9033_FORMS: [u8; 7] = [1, 2, 3, 4, 5, 8, 31];
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
/// `UNIT_FLAG_*` loss-of-control bits in
/// `UNIT_FIELD_FLAGS` (fork UnitDefines.h:558-568),
/// holder-derived by `refresh_cc_flags`.
const UNIT_FLAG_SILENCED: u32 = 0x0000_2000;
const UNIT_FLAG_PACIFIED: u32 = 0x0002_0000;
const UNIT_FLAG_STUNNED: u32 = 0x0004_0000;
const UNIT_FLAG_CONFUSED: u32 = 0x0040_0000;
const UNIT_FLAG_FLEEING: u32 = 0x0080_0000;
const UNIT_FLAG_CC_MASK: u32 = UNIT_FLAG_SILENCED
    | UNIT_FLAG_PACIFIED
    | UNIT_FLAG_STUNNED
    | UNIT_FLAG_CONFUSED
    | UNIT_FLAG_FLEEING;

use mangos_spells::defines::{
    aura as aura_type, aura_interrupt_flag, cast_result, dmg_class, effect, power, proc_ex,
    proc_flag, school, spell_interrupt_flag, spellmod, target as tgt,
};

use crate::procs::{ProcArgs, ProcRaiseCtx, att_type as proc_att_type};
use mangos_spells::{SpellEntry, spell_mod_apply};

use mangos_world_data::spawn_support::RespawnTimes;

use crate::map::{CooldownEntry, LoadedCooldown, Map, MapEvent};

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
/// `SPELL_ATTR_COOLDOWN_ON_EVENT` (Attributes bit 25).
const ATTR_COOLDOWN_ON_EVENT: u32 = 0x0200_0000;
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CastTarget {
    SelfCast,
    Unit(ObjectGuid),
    /// Ground destination (`TARGET_FLAG_DEST_LOCATION`;
    /// mask 0 fills it from the caster in the fork).
    Dest(Position),
}

/// The cast item of an item-use cast (`m_CastItem`):
/// identity needed for the `SPELL_START`/`GO` first
/// GUID, item cooldown overrides, and
/// `TakeCastItem` charge consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CastItemCtx {
    /// `item_instance.guid` of the used stack.
    pub item_guid: u32,
    /// Item template entry.
    pub item_entry: u32,
}

/// Caller-side overrides (`CastSpell` overloads in C++).
#[derive(Clone, Copy, Debug, Default)]
pub struct CastOptions {
    /// Triggered cast: skips power/GCD/cooldown/known
    /// checks and the `SPELL_START` packet family changes.
    pub triggered: bool,
    /// The cast originates from an aura (periodic
    /// trigger, proc handler): `m_triggeredByAuraSpell`.
    /// Only these triggered casts skip `TakePower`.
    pub triggered_by_aura: bool,
    /// Skip the known-spell check (creature API casts,
    /// login auras).
    pub skip_known: bool,
    /// Item-use cast (`SetCastItem`): no power cost,
    /// known check skipped, item cooldown overrides.
    pub cast_item: Option<CastItemCtx>,
    /// Custom per-effect base points
    /// (`CastCustomSpell` bp0..2 →
    /// `m_currentBasePoints`).
    pub custom_base_points: [Option<i32>; 3],
    /// Aura spell that triggered this cast
    /// (`m_triggeredByAuraSpell->Id`; 0 = none).
    pub triggered_by_aura_spell: u32,
    /// Spell whose effect triggered this cast
    /// (`m_triggeredBySpellInfo->Id`; 0 = none).
    pub triggered_by_spell: u32,
}

/// One in-progress cast (`Spell` object subset).
#[allow(clippy::struct_excessive_bools)] // fork Spell flag set
pub struct ActiveCast {
    pub spell_id: u32,
    pub cast_counter: u8,
    pub target: CastTarget,
    pub remaining_ms: i32,
    pub total_ms: u32,
    pub triggered: bool,
    /// Aura-originated cast (`m_triggeredByAuraSpell`):
    /// skips `TakePower` where a plain triggered cast
    /// still pays.
    pub triggered_by_aura: bool,
    /// Pushback counter (`m_delayAtDamageCount`).
    pub delay_count: u8,
    /// The spell is channeled (`m_channeled`).
    pub channeled: bool,
    /// Channel has started (`SPELL_STATE_CASTING`);
    /// false during a cast-time preparing phase.
    pub channeling: bool,
    /// Per-second channel cost timer
    /// (holder `m_timeCla`, one per channel here).
    pub channel_cost_ms: i32,
    /// Charged spell modifiers already dropped for
    /// this cast (`m_appliedMods`): (source spell,
    /// effect index) keys.
    pub applied_mods: Vec<(u32, u8)>,
    /// Proc flags computed at prepare
    /// (`m_procAttacker`/`m_procVictim`).
    pub proc_attacker_flags: u32,
    pub proc_victim_flags: u32,
    /// This cast may raise proc events
    /// (`GetCaster()->CanTriggerProcs` subset).
    pub can_trigger_procs: bool,
    /// Cast by a proc handler
    /// (`m_triggeredByAuraSpell`).
    pub from_proc: bool,
    /// Item-use cast context (`m_CastItem`).
    pub cast_item: Option<CastItemCtx>,
    /// Custom per-effect base points
    /// (`m_currentBasePoints` overrides).
    pub custom_base_points: [Option<i32>; 3],
    /// `m_triggeredByAuraSpell->Id` (0 = none).
    pub triggered_by_aura_spell: u32,
    /// `m_triggeredBySpellInfo->Id` (0 = none).
    pub triggered_by_spell: u32,
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
pub(crate) struct PeriodicTick {
    pub(crate) target: ObjectGuid,
    pub(crate) caster: ObjectGuid,
    pub(crate) spell_id: u32,
    pub(crate) aura_type: u32,
    pub(crate) amount: i32,
    pub(crate) effect_index: u32,
    /// `EffectMiscValue` (power type for energize).
    pub(crate) misc_value: i32,
}

/// `GetNextDelayAtDamageMsTime` (`Spell.h:512`):
/// 1000/800/600/400/200 as the counter advances below
/// 5, then 200.
fn next_delay_at_damage_ms(count: &mut u8) -> i32 {
    if *count < 5 {
        let delay = 1000 - i32::from(*count) * 200;
        *count += 1;
        delay
    } else {
        200
    }
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

    pub(crate) fn unit_power(&self, guid: ObjectGuid, power_type: u32) -> u32 {
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

    pub(crate) fn unit_set_power(&mut self, guid: ObjectGuid, power_type: u32, value: u32) {
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

    pub(crate) fn is_hostile_pair(a: ObjectGuid, b: ObjectGuid) -> bool {
        // No faction/reaction system yet: player<->creature
        // is hostile, same-kind is not.
        a.is_player() != b.is_player()
    }

    pub(crate) fn send_to_player(&mut self, guid: ObjectGuid, packet: WorldPacket) {
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

    /// `_ItemSpell` row for one spell on an item
    /// prototype (`pickCooldowns` lookup).
    fn item_spell_row(
        &self,
        item_entry: u32,
        spell_id: u32,
    ) -> Option<mangos_world_data::item_prototype::ItemSpell> {
        self.stores
            .item_prototypes
            .get(item_entry)?
            .spells
            .iter()
            .find(|row| row.spell_id == spell_id)
            .copied()
    }

    /// `Player::AddCooldown` duration pick: an item
    /// spell row overrides the category (when
    /// nonzero), the spell cooldown (when >= 0),
    /// and the category cooldown (when >= 0).
    /// Returns (recovery, category,
    /// `category_recovery`, `item_id`).
    fn cooldown_durations(
        &self,
        spell: &SpellEntry,
        item_entry: Option<u32>,
    ) -> (u32, u32, u32, u32) {
        if let Some(entry) = item_entry
            && let Some(row) = self.item_spell_row(entry, spell.id)
        {
            #[allow(clippy::cast_sign_loss)]
            return (
                if row.cooldown >= 0 {
                    row.cooldown as u32
                } else {
                    spell.recovery_time
                },
                if row.category != 0 {
                    row.category
                } else {
                    spell.category
                },
                if row.category_cooldown >= 0 {
                    row.category_cooldown as u32
                } else {
                    spell.category_recovery_time
                },
                entry,
            );
        }
        (
            spell.recovery_time,
            spell.category,
            spell.category_recovery_time,
            0,
        )
    }

    /// `SPELLMOD_COOLDOWN` as a duration.
    fn modded_cooldown(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        raw: u32,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> u32 {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        {
            self.apply_spell_mod(caster, spell, spellmod::COOLDOWN, raw as f32, true, applied)
                .max(0.0) as u32
        }
    }

    pub(crate) fn has_cooldown(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        cast_item: Option<CastItemCtx>,
    ) -> bool {
        // `IsSpellCDExpired`/`IsCatCDExpired`: a
        // permanent (cooldown-on-event) hold never
        // expires until converted.
        let now = self.now_ms;
        let active = |e: &CooldownEntry| e.permanent || e.expiry_ms > now;
        let spell_cd = self.cooldowns.get(&(caster, spell.id)).is_some_and(active);
        let category = self
            .cooldown_durations(spell, cast_item.map(|item| item.item_entry))
            .1;
        let cat_cd = category != 0
            && self
                .cooldowns
                .get(&(caster, category | 0x8000_0000))
                .is_some_and(active);
        spell_cd || cat_cd
    }

    pub(crate) fn add_cooldowns(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied_mods: &mut Vec<(u32, u8)>,
        forced: Option<u32>,
    ) {
        // `SendSpellCooldown` never records passives
        // (the proc forced add calls `AddCooldown`
        // directly and stays ungated).
        const ATTR_PASSIVE: u32 = 0x40;
        if forced.is_none() && spell.attributes & ATTR_PASSIVE != 0 {
            return;
        }
        // Already-existing guard (`Player::AddCooldown`):
        // an unexpired non-permanent entry (spell or
        // its recorded category) is left untouched —
        // a triggered recast executes but does not
        // refresh its own running cooldown.
        let now = self.now_ms;
        if let Some(entry) = self.cooldowns.get(&(caster, spell.id)).copied()
            && !entry.permanent
        {
            let category_active = entry.category != 0
                && self
                    .cooldowns
                    .get(&(caster, entry.category | 0x8000_0000))
                    .is_some_and(|c| c.permanent || c.expiry_ms > now);
            if entry.expiry_ms > now || category_active {
                return;
            }
        }
        // `SPELL_ATTR_COOLDOWN_ON_EVENT` spells enter
        // a permanent hold at cast (`Spell.cpp:4173`
        // passes `permanent`); the timed cooldown only
        // starts when the aura fades (conversion in
        // `remove_aura_at`). Fork still records the
        // would-be expiry. Forced adds pass
        // `permanent=false` explicitly.
        let permanent = forced.is_none() && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0;
        let base_recovery = forced.unwrap_or(spell.recovery_time);
        if base_recovery > 0 {
            let recovery = self.modded_cooldown(caster, spell, base_recovery, Some(applied_mods));
            if recovery > 0 {
                self.cooldowns.insert(
                    (caster, spell.id),
                    CooldownEntry {
                        expiry_ms: self.now_ms + recovery,
                        item_id: 0,
                        permanent,
                        category: spell.category,
                        spell_id: spell.id,
                    },
                );
            }
        }
        if spell.category != 0 && spell.category_recovery_time > 0 {
            let recovery = self.modded_cooldown(
                caster,
                spell,
                spell.category_recovery_time,
                Some(applied_mods),
            );
            if recovery > 0 {
                self.cooldowns.insert(
                    (caster, spell.category | 0x8000_0000),
                    CooldownEntry {
                        expiry_ms: self.now_ms + recovery,
                        item_id: 0,
                        permanent,
                        category: 0,
                        spell_id: spell.id,
                    },
                );
            }
        }
    }

    /// `Player::AddCooldown` with a cast item
    /// (`Player.cpp:21871`): item duration overrides
    /// from `cooldown_durations`, `SPELLMOD_COOLDOWN`
    /// on the spell recovery when nonzero else the
    /// category recovery, and the spell-key entry
    /// records the item id + effective category.
    pub(crate) fn add_cooldowns_item(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        item: CastItemCtx,
        applied_mods: &mut Vec<(u32, u8)>,
    ) {
        // Same `SendSpellCooldown` passive gate and
        // already-existing guard as the plain path
        // (one fork `Player::AddCooldown`).
        const ATTR_PASSIVE: u32 = 0x40;
        if spell.attributes & ATTR_PASSIVE != 0 {
            return;
        }
        let now = self.now_ms;
        if let Some(entry) = self.cooldowns.get(&(caster, spell.id)).copied()
            && !entry.permanent
        {
            let category_active = entry.category != 0
                && self
                    .cooldowns
                    .get(&(caster, entry.category | 0x8000_0000))
                    .is_some_and(|c| c.permanent || c.expiry_ms > now);
            if entry.expiry_ms > now || category_active {
                return;
            }
        }
        let (rec, category, cat_rec, item_id) =
            self.cooldown_durations(spell, Some(item.item_entry));
        let (rec, cat_rec) = if rec > 0 {
            (
                self.modded_cooldown(caster, spell, rec, Some(applied_mods)),
                cat_rec,
            )
        } else if category != 0 && cat_rec > 0 {
            (
                rec,
                self.modded_cooldown(caster, spell, cat_rec, Some(applied_mods)),
            )
        } else {
            (rec, cat_rec)
        };
        let permanent = spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0;
        if rec > 0 {
            self.cooldowns.insert(
                (caster, spell.id),
                CooldownEntry {
                    expiry_ms: self.now_ms + rec,
                    item_id,
                    permanent,
                    category,
                    spell_id: spell.id,
                },
            );
        }
        if category != 0 && cat_rec > 0 {
            self.cooldowns.insert(
                (caster, category | 0x8000_0000),
                CooldownEntry {
                    expiry_ms: self.now_ms + cat_rec,
                    item_id: 0,
                    permanent,
                    category: 0,
                    spell_id: spell.id,
                },
            );
        }
    }

    /// Convert a permanent cooldown-on-event hold
    /// into the timed cooldown (`AddCooldown` at
    /// aura fade, `SpellAuras.cpp` holder cleanup):
    /// fresh recovery from now, spellmod applied
    /// with the fork null spell pointer (value
    /// applies, no charge drop). No-op without a
    /// permanent spell-key hold.
    fn start_event_cooldown(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        let Some(held) = self
            .cooldowns
            .get(&(caster, spell.id))
            .copied()
            .filter(|e| e.permanent)
        else {
            return;
        };
        self.cooldowns.remove(&(caster, spell.id));
        // The fork re-picks item cooldown durations
        // from the held entry's item when the hold
        // came from an item cast (`wasPermanent` arm
        // of `Player::AddCooldown`).
        let (rec, category, cat_rec, item_id) =
            self.cooldown_durations(spell, (held.item_id != 0).then_some(held.item_id));
        let held_category = if held.category != 0 {
            held.category
        } else {
            category
        };
        if held_category != 0 {
            self.cooldowns
                .remove(&(caster, held_category | 0x8000_0000));
        }
        let recovery = self.modded_cooldown(caster, spell, rec, None);
        if recovery > 0 {
            self.cooldowns.insert(
                (caster, spell.id),
                CooldownEntry {
                    expiry_ms: self.now_ms + recovery,
                    item_id,
                    permanent: false,
                    category,
                    spell_id: spell.id,
                },
            );
        }
        if category != 0 && cat_rec > 0 {
            let cat_recovery = self.modded_cooldown(caster, spell, cat_rec, None);
            if cat_recovery > 0 {
                self.cooldowns.insert(
                    (caster, category | 0x8000_0000),
                    CooldownEntry {
                        spell_id: spell.id,
                        ..CooldownEntry::timed(self.now_ms + cat_recovery)
                    },
                );
            }
        }
    }

    /// Seed loaded cooldown rows at login
    /// (`Player::_LoadSpellCooldowns` enforcement
    /// half: the client packet is built session-side;
    /// without this the server would not block).
    pub fn seed_cooldowns(&mut self, caster: ObjectGuid, rows: &[LoadedCooldown]) {
        // Login reload is authoritative: the fork's
        // cooldown map lives on the Player and dies
        // at logout, so any entries a previous
        // session left in this map are dropped
        // before seeding from the DB rows.
        self.cooldowns.retain(|(guid, _), _| *guid != caster);
        for row in rows {
            if row.spell_remaining_ms > 0 {
                self.cooldowns.insert(
                    (caster, row.spell),
                    CooldownEntry {
                        expiry_ms: self.now_ms + row.spell_remaining_ms,
                        item_id: row.item_id,
                        permanent: false,
                        category: row.category,
                        spell_id: row.spell,
                    },
                );
            }
            if row.category != 0 && row.category_remaining_ms > 0 {
                self.cooldowns.insert(
                    (caster, row.category | 0x8000_0000),
                    CooldownEntry {
                        spell_id: row.spell,
                        ..CooldownEntry::timed(self.now_ms + row.category_remaining_ms)
                    },
                );
            }
        }
    }

    /// `Player::ApplySpellMod` (`Player.cpp:22465`,
    /// players only as `HandleAddModifier` gates):
    /// collect the caster's
    /// `ADD_FLAT_MODIFIER`/`ADD_PCT_MODIFIER`
    /// effects for `op`, filter by family name +
    /// affect mask (`IsAffectedOnSpell`), compose in
    /// fork order, and drop charges once per cast
    /// (`applied` dedupe = fork `m_appliedMods`;
    /// `None` mirrors a null spell pointer: applies
    /// but never drops and ignores the exhausted
    /// gate). Modifier state lives on the holder
    /// effects, so removal is inherently reversible.
    #[allow(
        clippy::float_cmp, // fork compares basevalue to T(0) exactly
        clippy::cast_precision_loss
    )]
    /// `CalculateSpellEffectValue` + creature-level
    /// scaling tail (class-1 CLS melee-damage ratio).
    pub(crate) fn effect_value(
        &mut self,
        spell: &SpellEntry,
        eff_index: usize,
        caster_level: u32,
    ) -> f32 {
        self.effect_value_bp(spell, eff_index, caster_level, None)
    }

    /// [`Self::effect_value`] with a cast-custom
    /// base-points override (`m_currentBasePoints`).
    pub(crate) fn effect_value_bp(
        &mut self,
        spell: &SpellEntry,
        eff_index: usize,
        caster_level: u32,
        bp: Option<i32>,
    ) -> f32 {
        let value =
            spell.calculate_effect_value_bp(eff_index, caster_level, 0, self.spell_rng(), bp);
        let cls = &self.stores.cls;
        spell.creature_level_scaled(eff_index, value, caster_level, &|level| {
            cls.get(1, level).melee_damage
        })
    }

    pub(crate) fn apply_spell_mod(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        op: u32,
        base: f32,
        drop_charge: bool,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        // SPELL_ATTR_EX3_IGNORE_CASTER_MODIFIERS.
        if !self.players.contains_key(&caster) || spell.attributes_ex3 & 0x2000_0000 != 0 {
            return base;
        }
        // (holder index, source spell, effect index,
        // is_pct, value, charges)
        let mut hits: Vec<(usize, u32, u8, bool, i32, i32)> = Vec::new();
        for (holder_idx, holder) in self.aura_holders(caster).iter().enumerate() {
            let Some(source) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if source.spell_family_name != spell.spell_family_name {
                continue;
            }
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(state) = eff else { continue };
                let is_pct = match state.aura_type {
                    aura_type::ADD_FLAT_MODIFIER => false,
                    aura_type::ADD_PCT_MODIFIER => true,
                    _ => continue,
                };
                if u32::try_from(state.misc_value) != Ok(op) {
                    continue;
                }
                if spell.spell_family_flags & source.effects[eff_idx].item_type == 0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                hits.push((
                    holder_idx,
                    holder.spell_id,
                    eff_idx as u8,
                    is_pct,
                    state.amount,
                    state.mod_charges,
                ));
            }
        }
        if hits.is_empty() {
            return base;
        }
        let mut applied = applied;
        let mut total_flat = 0.0f32;
        let mut total_pct = 0.0f32;
        let mut drops: Vec<(usize, u8)> = Vec::new();
        for (holder_idx, source_id, eff_idx, is_pct, value, charges) in hits {
            let key = (source_id, eff_idx);
            // Exhausted mods only still affect the cast
            // that consumed them.
            if charges == -1
                && let Some(list) = &applied
                && !list.contains(&key)
            {
                continue;
            }
            if is_pct {
                if base == 0.0 {
                    continue;
                }
                // Instant-cast mods never apply to
                // 10 s+ casts.
                if op == spellmod::CASTING_TIME && base >= 10_000.0 && value <= -100 {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                {
                    total_pct += value as f32;
                }
            } else {
                #[allow(clippy::cast_precision_loss)]
                {
                    total_flat += value as f32;
                }
            }
            if drop_charge
                && charges > 0
                && let Some(list) = &mut applied
                && !list.contains(&key)
            {
                list.push(key);
                drops.push((holder_idx, eff_idx));
            }
            // A -100% casting-time mod zeroes the cast
            // outright (fork Barkskin/Swiftness fix).
            if op == spellmod::CASTING_TIME && is_pct && value == -100 {
                total_pct = -100.0;
                total_flat = 0.0;
                break;
            }
        }
        if !drops.is_empty()
            && let Some(unit) = self.unit_base_mut(caster)
        {
            for (holder_idx, eff_idx) in drops {
                if let Some(state) = unit
                    .auras
                    .get_mut(holder_idx)
                    .and_then(|h| h.effects[usize::from(eff_idx)].as_mut())
                    && state.mod_charges > 0
                {
                    state.mod_charges -= 1;
                    if state.mod_charges == 0 {
                        state.mod_charges = -1;
                    }
                }
            }
        }
        spell_mod_apply(base, total_flat, total_pct)
    }

    /// `Player::RemoveSpellMods` subset: holders whose
    /// modifier this cast exhausted are removed.
    pub(crate) fn remove_exhausted_spell_mods(
        &mut self,
        caster: ObjectGuid,
        applied: &[(u32, u8)],
    ) {
        let mut sources: Vec<u32> = Vec::new();
        for (source, _) in applied {
            if !sources.contains(source) {
                sources.push(*source);
            }
        }
        for source in sources {
            let exhausted = self
                .aura_holders(caster)
                .iter()
                .filter(|h| h.spell_id == source)
                .any(|h| h.effects.iter().flatten().any(|e| e.mod_charges == -1));
            if exhausted {
                self.cancel_aura(caster, source);
            }
        }
    }

    /// Spell duration with `SPELLMOD_DURATION`
    /// applied (positive durations only; a modded
    /// result <= 0 suppresses the aura where the fork
    /// would expire it on the first update).
    fn modded_duration_ms(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> i32 {
        let raw = spell.calculate_duration_ms(&self.stores.spells.tables);
        if raw <= 0 {
            return raw;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        {
            self.apply_spell_mod(caster, spell, spellmod::DURATION, raw as f32, true, applied)
                as i32
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

    pub(crate) fn player_knows_spell(&self, caster: ObjectGuid, spell_id: u32) -> bool {
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
    #[allow(clippy::too_many_lines)] // one prepare() port
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
        // Cast prevention by loss-of-control flags
        // (fork Spell.cpp:6826-6838; mechanic-immunity
        // and stun-interrupt-flag nuances unmodeled).
        {
            let flags = self.unit_value_u32(caster, offsets::UNIT_FIELD_FLAGS);
            if flags & UNIT_FLAG_STUNNED != 0 {
                return fail(self, cast_result::FAILED_STUNNED);
            }
            if flags & UNIT_FLAG_CONFUSED != 0 {
                return fail(self, cast_result::FAILED_CONFUSED);
            }
            if flags & UNIT_FLAG_FLEEING != 0 {
                return fail(self, cast_result::FAILED_FLEEING);
            }
            if spell.prevention_type == 1 && flags & UNIT_FLAG_SILENCED != 0 {
                return fail(self, cast_result::FAILED_SILENCED);
            }
            if spell.prevention_type == 2 && flags & UNIT_FLAG_PACIFIED != 0 {
                return fail(self, cast_result::FAILED_PACIFIED);
            }
        }
        if !options.skip_known
            && options.cast_item.is_none()
            && !self.player_knows_spell(caster, spell_id)
        {
            return fail(self, cast_result::FAILED_UNKNOWN);
        }
        // The current-cast slot gate applies only to
        // non-triggered casts (fork prepare:
        // `!m_IsTriggeredSpell && IsNonMeleeSpellCasted`).
        if !options.triggered && self.casts.contains_key(&caster) {
            return fail(self, cast_result::FAILED_SPELL_IN_PROGRESS);
        }
        // Projectile delay is not modeled in the slice.
        if spell.speed > 0.0 {
            return fail(self, cast_result::FAILED_UNKNOWN);
        }
        // Charged spell modifiers dropped for this
        // cast (`m_appliedMods`).
        let mut applied_mods: Vec<(u32, u8)> = Vec::new();
        if !options.triggered {
            let now = self.now_ms;
            let gcd = self
                .unit_base_mut(caster)
                .is_some_and(|u| u.gcd_until_ms > now);
            if gcd || self.has_cooldown(caster, &spell, options.cast_item) {
                return fail(self, cast_result::FAILED_NOT_READY);
            }
            // Item casts are free
            // (`CalculatePowerCost` returns 0 with a
            // cast item); GCD + cooldown gates above
            // still run for them.
            if options.cast_item.is_none() {
                let cost = spell.power_cost(
                    self.unit_power(caster, spell.power_type),
                    self.unit_max_power(caster, spell.power_type),
                    self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
                );
                // `SPELLMOD_COST` at the power check never
                // drops a charge (the take site does).
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let cost = self
                    .apply_spell_mod(
                        caster,
                        &spell,
                        spellmod::COST,
                        cost as f32,
                        false,
                        Some(&mut applied_mods),
                    )
                    .max(0.0) as u32;
                if self.unit_power(caster, spell.power_type) < cost {
                    return fail(self, cast_result::FAILED_NO_POWER);
                }
            }
        }
        // Target validation (subset of CheckCast target
        // rules driven by effect 0's implicit target).
        if let CastOutcome::Failed(reason) =
            self.check_target(caster, &spell, target, &mut applied_mods)
        {
            return fail(self, reason);
        }
        // `SpellScript::OnCheckCast` (end of `CheckCast`).
        if let Some(reason) = self.spell_script_on_check_cast(caster, &spell, target) {
            return fail(self, reason);
        }
        // `BattleGround::CheckSpellCast` override
        // (AV shredder charms 21544/21565;
        // `SPELL_FAILED_SPELL_UNAVAILABLE` = 0x2f).
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 1)
            && !self.av_check_spell_cast(caster, spell.id)
        {
            return fail(self, 0x2F);
        }

        // Triggered casts keep their DBC cast time
        // (fork `CalculateCastTime` has no triggered
        // zeroing; the spell simply shows no cast bar
        // and ticks outside the current-cast slot).
        let cast_time = {
            let raw = spell.cast_time_ms(&self.stores.spells.tables);
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            {
                self.apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::CASTING_TIME,
                    raw as f32,
                    true,
                    Some(&mut applied_mods),
                )
                .max(0.0) as u32
            }
        };
        let counter = self.unit_base_mut(caster).map_or(0, |u| {
            let c = u.cast_counter;
            u.cast_counter = u.cast_counter.wrapping_add(1);
            c
        });
        let position = self.unit_position(caster).unwrap_or_default();
        let from_proc = self.in_proc_cast;
        let (proc_attacker_flags, proc_victim_flags, can_trigger_procs) =
            crate::procs::spell_proc_context(&spell, options.triggered, from_proc);
        let cast = ActiveCast {
            spell_id,
            cast_counter: counter,
            target,
            remaining_ms: i32::try_from(cast_time).unwrap_or(i32::MAX),
            total_ms: cast_time,
            triggered: options.triggered,
            triggered_by_aura: options.triggered_by_aura,
            delay_count: 0,
            channeled: spell.is_channeled(),
            channeling: false,
            channel_cost_ms: 1000,
            applied_mods,
            proc_attacker_flags,
            proc_victim_flags,
            can_trigger_procs,
            from_proc,
            cast_item: options.cast_item,
            custom_base_points: options.custom_base_points,
            triggered_by_aura_spell: options.triggered_by_aura_spell,
            triggered_by_spell: options.triggered_by_spell,
            position,
        };
        if cast_time > 0 && !options.triggered {
            self.send_spell_start(caster, &cast, &spell);
        }
        // Slot placement (fork prepare): a triggered
        // non-channeled cast never occupies the
        // current-cast slot — timer 0 executes in
        // this call point, timer > 0 ticks on the
        // pending list. Channeled casts take the
        // slot (`SetCurrentCastedSpell`).
        if options.triggered && !cast.channeled {
            if cast_time == 0 {
                self.execute_cast(caster, cast);
            } else {
                self.pending_triggered.push((caster, cast));
            }
            return CastOutcome::Started;
        }
        self.casts.insert(caster, cast);
        if cast_time == 0 {
            self.update_cast(caster, 0);
        }
        CastOutcome::Started
    }

    fn check_target(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
        applied_mods: &mut Vec<(u32, u8)>,
    ) -> CastOutcome {
        let mode = spell
            .effects
            .iter()
            .find(|e| e.effect != effect::NONE)
            .map_or(tgt::UNIT_CASTER, |e| e.implicit_target_a);
        // Target mode 0 (NONE in DBC) behaves as
        // caster-targeted for check purposes.
        match mode {
            0 | tgt::UNIT_CASTER
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
            | tgt::LOCATION_UNIT_MINION_POSITION
            | tgt::LOCATION_CASTER_SRC
            | tgt::ENUM_UNITS_ENEMY_IN_CONE => CastOutcome::Started,
            // Ground enumerations: destination comes
            // from the client (or the caster for
            // scripted casts); range gates to the
            // destination like the fork's CheckRange.
            tgt::ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION => {
                if let CastTarget::Dest(pos) = target {
                    let range = self.apply_spell_mod(
                        caster,
                        spell,
                        spellmod::RANGE,
                        spell.range_max(&self.stores.spells.tables),
                        false,
                        Some(applied_mods),
                    ) + self.bounding_radius_of(caster);
                    let Some(cpos) = self.unit_position(caster) else {
                        return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                    };
                    let dist = ((cpos.x - pos.x).powi(2) + (cpos.y - pos.y).powi(2)).sqrt();
                    if dist > range {
                        return CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE);
                    }
                }
                CastOutcome::Started
            }
            tgt::UNIT_ENEMY | tgt::UNIT_FRIEND | tgt::UNIT => {
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
                // Positive-spell immunity gate
                // (`Spell::CheckTarget`).
                #[allow(clippy::cast_possible_wrap)]
                if victim != caster
                    && spell.is_positive_spell()
                    && (self.is_immune_to_spell(victim, spell, false)
                        || self.is_immune_to_damage(
                            victim,
                            1i32 << spell.school,
                            Some(spell),
                        ))
                {
                    return CastOutcome::Failed(cast_result::FAILED_TARGET_AURASTATE);
                }
                if mode == tgt::UNIT_ENEMY && !Self::is_hostile_pair(caster, victim) {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                if mode == tgt::UNIT_FRIEND && Self::is_hostile_pair(caster, victim) {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                let cpos = self.unit_position(caster).unwrap_or_default();
                // `SPELLMOD_RANGE` (`CheckRange`; range
                // checks never drop charges).
                let range = self.apply_spell_mod(
                    caster,
                    spell,
                    spellmod::RANGE,
                    spell.range_max(&self.stores.spells.tables),
                    false,
                    Some(applied_mods),
                ) + self.bounding_radius_of(caster)
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
            CastTarget::Dest(pos) => {
                payload.write_u16_le(0x0040); // TARGET_FLAG_DEST_LOCATION
                payload.write_f32_le(pos.x);
                payload.write_f32_le(pos.y);
                payload.write_f32_le(pos.z);
            }
        }
    }

    fn send_spell_start(&mut self, caster: ObjectGuid, cast: &ActiveCast, spell: &SpellEntry) {
        let target_guid = match cast.target {
            CastTarget::Unit(g) => g,
            CastTarget::SelfCast | CastTarget::Dest(_) => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(Self::cast_first_guid(
            caster, cast,
        )));
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
        // Channel interrupt closes the bar via
        // `MSG_CHANNEL_UPDATE(0)` in `end_channel`
        // (no SPELL_FAILURE family).
        if self.casts.get(&caster).is_some_and(|c| c.channeling) {
            self.end_channel(caster, true);
            return;
        }
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

    /// The shared pushback resist total
    /// (`GetTotalAuraModifier(SPELL_AURA_RESIST_PUSHBACK)`;
    /// the fork adds it to the spellmod-adjusted 100
    /// then subtracts 100, and spell mods are identity
    /// in this subset).
    fn pushback_resist_chance(&self, victim: ObjectGuid) -> i32 {
        self.aura_holders(victim)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| e.aura_type == aura_type::RESIST_PUSHBACK)
            .map(|e| e.amount)
            .sum()
    }

    /// The fork pushback resist roll shared by
    /// `Delayed`/`DelayedChannel`: 100 through
    /// `SPELLMOD_NOT_LOSE_CASTING_TIME`, plus the
    /// `RESIST_PUSHBACK` aura total, minus 100,
    /// vs `irand(0, 99)`.
    fn pushback_resisted(&mut self, victim: ObjectGuid, spell: &SpellEntry) -> bool {
        let mut applied = self
            .casts
            .get_mut(&victim)
            .map_or_else(Vec::new, |c| std::mem::take(&mut c.applied_mods));
        #[allow(clippy::cast_possible_truncation)]
        let modded = self.apply_spell_mod(
            victim,
            spell,
            spellmod::NOT_LOSE_CASTING_TIME,
            100.0,
            true,
            Some(&mut applied),
        ) as i32;
        if let Some(c) = self.casts.get_mut(&victim) {
            c.applied_mods = applied;
        }
        let resist_chance = modded + self.pushback_resist_chance(victim) - 100;
        resist_chance > self.spell_rng().irand(0, 99)
    }

    /// Damage-taken cast interference (the
    /// `DealDamage` current-spell loop, `Unit.cpp`
    /// ~896): channeled casts take the
    /// `DelayedChannel`/channel-cancel arm; preparing
    /// player casts take `Spell::Delayed`
    /// (`Spell.cpp:7686`) — `DAMAGE_CANCELS` interrupts,
    /// otherwise pushback gated by `DAMAGE_PUSHBACK`
    /// and the resist roll. The fork delayed-state arm
    /// is unreachable (no projectile delay), and
    /// creature casts are never pushed.
    fn delay_spell_at_damage(&mut self, attacker: ObjectGuid, victim: ObjectGuid) {
        let Some((spell_id, channeling)) =
            self.casts.get(&victim).map(|c| (c.spell_id, c.channeling))
        else {
            return;
        };
        if channeling {
            let Some(spell) = self.stores.spells.get(spell_id) else {
                return;
            };
            let cflags = spell.channel_interrupt_flags;
            if cflags & aura_interrupt_flag::DAMAGE_CHANNEL_DURATION != 0 {
                // Self-damage never shortens a channel,
                // and `DelayedChannel` is players-only.
                if attacker != victim && self.players.contains_key(&victim) {
                    self.delay_channel(victim);
                }
            } else if cflags & aura_interrupt_flag::DAMAGE_CANCELS != 0 {
                self.end_channel(victim, true);
            }
            return;
        }
        if !self.players.contains_key(&victim) {
            return;
        }
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        let flags = spell.interrupt_flags;
        if flags & spell_interrupt_flag::DAMAGE_CANCELS != 0 {
            self.cancel_cast(victim, true);
            return;
        }
        if flags & spell_interrupt_flag::DAMAGE_PUSHBACK == 0 {
            return;
        }
        if self.pushback_resisted(victim, &spell) {
            return;
        }
        let Some(cast) = self.casts.get_mut(&victim) else {
            return;
        };
        let mut delay_ms = next_delay_at_damage_ms(&mut cast.delay_count);
        #[allow(clippy::cast_possible_wrap)]
        let total_ms = cast.total_ms as i32;
        if cast.remaining_ms + delay_ms > total_ms {
            delay_ms = total_ms - cast.remaining_ms;
            cast.remaining_ms = total_ms;
        } else {
            cast.remaining_ms += delay_ms;
        }
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        #[allow(clippy::cast_sign_loss)]
        payload.write_u32_le(delay_ms as u32);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_DELAYED, payload.into_vec());
        self.send_to_player(victim, packet);
    }

    /// `Spell::DelayedChannel` (`Spell.cpp:7725`):
    /// shorten the channel and its holders by the
    /// scheduled delay, then refresh the client bar
    /// (or interrupt when nothing remains).
    fn delay_channel(&mut self, victim: ObjectGuid) {
        let Some(spell_id) = self.casts.get(&victim).map(|c| c.spell_id) else {
            return;
        };
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        if self.pushback_resisted(victim, &spell) {
            return;
        }
        let Some((spell_id, target, remaining_ms, delay_ms)) =
            self.casts.get_mut(&victim).map(|cast| {
                let mut delay_ms = next_delay_at_damage_ms(&mut cast.delay_count);
                if cast.remaining_ms < delay_ms {
                    delay_ms = cast.remaining_ms;
                    cast.remaining_ms = 0;
                } else {
                    cast.remaining_ms -= delay_ms;
                }
                (cast.spell_id, cast.target, cast.remaining_ms, delay_ms)
            })
        else {
            return;
        };
        if let CastTarget::Unit(unit) = target
            && unit != victim
        {
            self.delay_aura_holders(unit, spell_id, victim, delay_ms);
        }
        self.delay_aura_holders(victim, spell_id, victim, delay_ms);
        if remaining_ms <= 0 {
            self.end_channel(victim, true);
        } else {
            let mut payload = PacketWriter::new();
            #[allow(clippy::cast_sign_loss)]
            payload.write_u32_le(remaining_ms as u32);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_UPDATE, payload.into_vec());
            self.send_to_player(victim, packet);
        }
    }

    /// `Unit::DelaySpellAuraHolder` +
    /// `RefreshAuraPeriodicTimers`/`UpdatePeriodicTimer`:
    /// shorten matching holders and realign their
    /// periodic timers to the new duration so the
    /// remaining tick count still divides evenly.
    fn delay_aura_holders(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        caster: ObjectGuid,
        delay_ms: i32,
    ) {
        let mut duration_packets: Vec<(u8, i32)> = Vec::new();
        if let Some(unit) = self.unit_base_mut(victim) {
            for holder in &mut unit.auras {
                if holder.spell_id != spell_id || holder.caster != caster || holder.duration_ms <= 0
                {
                    continue;
                }
                holder.duration_ms = if holder.duration_ms < delay_ms {
                    0
                } else {
                    holder.duration_ms - delay_ms
                };
                for eff in holder.effects.iter_mut().flatten() {
                    if !eff.is_periodic() || eff.period_ms == 0 {
                        continue;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    let period = eff.period_ms as i32;
                    let mut timer = if holder.duration_ms > period {
                        holder.duration_ms % period
                    } else {
                        holder.duration_ms
                    };
                    if timer == 0 {
                        timer = period;
                    }
                    eff.periodic_timer = timer;
                }
                duration_packets.push((holder.slot, holder.duration_ms));
            }
        }
        for (slot, duration) in duration_packets {
            self.send_aura_duration(victim, slot, duration);
        }
    }

    /// `Spell::update` + `Spell::cast` subset.
    #[allow(clippy::too_many_lines)] // one cast() port
    pub(crate) fn update_cast(&mut self, caster: ObjectGuid, diff_ms: u32) {
        if self.casts.get(&caster).is_some_and(|c| c.channeling) {
            self.update_channel(caster, diff_ms);
            return;
        }
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
        self.execute_cast(caster, cast);
    }

    /// The `cast()` execution body for a cast whose
    /// timer has elapsed (also run inline for
    /// slot-less triggered casts, fork prepare:
    /// triggered timer 0 executes in the call point).
    #[allow(clippy::too_many_lines)] // one cast() port (split from update_cast)
    fn execute_cast(&mut self, caster: ObjectGuid, mut cast: ActiveCast) {
        let Some(spell) = self.stores.spells.get(cast.spell_id).cloned() else {
            return;
        };
        let alive = self.unit_base_mut(caster).is_some_and(|u| u.is_alive());
        // Triggered casts complete even from a dead
        // caster (death-event spells).
        if !alive && !cast.triggered {
            return;
        }

        // TakePower + GCD + cooldowns at cast(),
        // fork split: `TakePower` early-returns only
        // for cast-item and aura-triggered casts (a
        // plain triggered cast still pays, unchecked);
        // GCD is added only for non-triggered casts
        // (prepare); `SendSpellCooldown` adds the
        // spell/category cooldown for every cast.
        if cast.cast_item.is_none() && !cast.triggered_by_aura {
            let power_cost = spell.power_cost(
                self.unit_power(caster, spell.power_type),
                self.unit_max_power(caster, spell.power_type),
                self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
            );
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let power_cost = self
                .apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::COST,
                    power_cost as f32,
                    true,
                    Some(&mut cast.applied_mods),
                )
                .max(0.0) as u32;
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
        }
        if !cast.triggered && spell.start_recovery_time > 0 {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let gcd = self
                .apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::GLOBAL_COOLDOWN,
                    spell.start_recovery_time as f32,
                    true,
                    Some(&mut cast.applied_mods),
                )
                .max(0.0) as u32;
            let until = self.now_ms + gcd;
            if let Some(u) = self.unit_base_mut(caster) {
                u.gcd_until_ms = until;
            }
        }
        if let Some(item) = cast.cast_item {
            self.add_cooldowns_item(caster, &spell, item, &mut cast.applied_mods);
        } else {
            self.add_cooldowns(caster, &spell, &mut cast.applied_mods, None);
        }

        let victims = self.collect_targets(caster, &spell, cast.target);
        // `EffectPersistentAA` runs once per cast at
        // the destination (fork executes it from the
        // location target list, not per unit victim).
        if let Some(eff_index) = spell
            .effects
            .iter()
            .position(|e| e.effect == effect::PERSISTENT_AREA_AURA)
        {
            let dest = self.cast_dest(caster, &spell, cast.target);
            self.spawn_dynobject(
                caster,
                &spell,
                eff_index,
                dest,
                Some(&mut cast.applied_mods),
            );
        }
        let caster_level = self.unit_level(caster);
        let mut hits: Vec<ObjectGuid> = Vec::new();
        let mut misses: Vec<(ObjectGuid, u8)> = Vec::new();
        let mut hit_victims: Vec<ObjectGuid> = Vec::new();
        for victim in &victims {
            // Spell-level immunity (`Spell::TakeTarget`
            // missCondition): damage-immune (hostile
            // only) or spell-immune -> SPELL_MISS_IMMUNE.
            #[allow(clippy::cast_possible_wrap)]
            let school_mask = 1i32 << spell.school;
            if *victim != caster
                && ((!spell_is_positive_for_target(&spell)
                    && self.is_immune_to_damage(*victim, school_mask, Some(&spell)))
                    || self.is_immune_to_spell(*victim, &spell, false))
            {
                misses.push((*victim, 7)); // SPELL_MISS_IMMUNE
                continue;
            }
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
        // `TakeCastItem` runs in `cast()` right after
        // `SendSpellGo` (the item must outlive the GO
        // packet's cast-item GUID); triggered casts
        // skip it outside trade-item targets, which
        // are unmodeled.
        if let Some(item) = cast.cast_item
            && !cast.triggered
            && caster.is_player()
        {
            self.take_cast_item(caster, item);
        }
        // Cast-end proc raise (fork raises it in
        // `cast()` before effect application).
        if cast.can_trigger_procs {
            let main_target = victims.first().copied();
            let ex = if main_target.is_none_or(|t| hit_victims.contains(&t)) {
                proc_ex::CAST_END | proc_ex::NORMAL_HIT
            } else {
                proc_ex::CAST_END | proc_ex::MISS
            };
            let args = ProcArgs {
                target: main_target,
                attacker_flags: cast.proc_attacker_flags,
                victim_flags: proc_flag::NONE,
                proc_ex: ex,
                amount: 1,
                original_amount: 1,
                att_type: if spell.dmg_class == dmg_class::RANGED {
                    proc_att_type::RANGED
                } else {
                    proc_att_type::BASE
                },
                proc_spell_id: Some(spell.id),
                triggered_by_aura: cast.from_proc,
                proc_time_ms: self.now_ms,
                applied_mods: &cast.applied_mods,
            };
            self.proc_damage_and_spell(caster, args);
        }
        let ctx = ProcRaiseCtx {
            attacker_flags: cast.proc_attacker_flags,
            victim_flags: cast.proc_victim_flags,
            can_trigger: cast.can_trigger_procs,
            triggered_by_aura: cast.from_proc,
            att_type: if spell.dmg_class == dmg_class::RANGED {
                proc_att_type::RANGED
            } else {
                proc_att_type::BASE
            },
            applied_mods: cast.applied_mods.clone(),
        };
        for victim in hit_victims {
            self.apply_effects(
                caster,
                caster_level,
                &spell,
                victim,
                match cast.target {
                    CastTarget::Dest(pos) => Some(pos),
                    _ => None,
                },
                Some(&mut cast.applied_mods),
                Some(&ctx),
                cast.custom_base_points,
                cast.cast_item.is_some(),
                cast.cast_counter,
                cast.triggered_by_aura_spell,
                cast.triggered_by_spell,
            );
        }
        let _ = caster_level;
        // Start channeling (the `handle_immediate`
        // tail). Effects are already applied in this
        // subset, so `MSG_CHANNEL_START` follows their
        // packets where the fork sends it first.
        if cast.channeled && !cast.channeling {
            self.start_channel(caster, cast, &spell, &hits);
        } else {
            self.remove_exhausted_spell_mods(caster, &cast.applied_mods);
        }
    }

    /// `SendChannelStart` + channel state entry
    /// (`Spell.cpp:4002-4012, 5138`): fields, direct
    /// start packet to a player caster, cast retained
    /// with the channel duration as its timer.
    fn start_channel(
        &mut self,
        caster: ObjectGuid,
        mut cast: ActiveCast,
        spell: &SpellEntry,
        hits: &[ObjectGuid],
    ) {
        let duration = self.modded_duration_ms(caster, spell, Some(&mut cast.applied_mods));
        if duration <= 0 || !self.unit_is_alive(caster) {
            return;
        }
        let channel_target = hits.iter().copied().find(|g| *g != caster);
        cast.channeling = true;
        cast.remaining_ms = duration;
        #[allow(clippy::cast_sign_loss)]
        {
            cast.total_ms = duration as u32;
        }
        cast.channel_cost_ms = 1000;
        if let Some(u) = self.unit_base_mut(caster) {
            let values = u.values();
            values.set_u64(
                offsets::UNIT_FIELD_CHANNEL_OBJECT,
                channel_target.map_or(0, ObjectGuid::raw),
            );
            values.set_u32(offsets::UNIT_CHANNEL_SPELL, spell.id);
        }
        if self.players.contains_key(&caster) {
            let mut payload = PacketWriter::new();
            payload.write_u32_le(spell.id);
            #[allow(clippy::cast_sign_loss)]
            payload.write_u32_le(duration as u32);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_START, payload.into_vec());
            self.send_to_player(caster, packet);
        }
        let applied = cast.applied_mods.clone();
        self.casts.insert(caster, cast);
        // Fork sweeps exhausted mods right after
        // `SendChannelStart`.
        self.remove_exhausted_spell_mods(caster, &applied);
    }

    /// Channeled tick (`Spell::update` CASTING arm +
    /// the holder `UpdateHolder` cost/range arms,
    /// folded into the cast because holder ticking
    /// itself stays in `update_auras` here).
    fn update_channel(&mut self, caster: ObjectGuid, diff_ms: u32) {
        let Some((spell_id, target)) = self.casts.get(&caster).map(|c| (c.spell_id, c.target))
        else {
            return;
        };
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            self.end_channel(caster, true);
            return;
        };
        // Target validity + range (single-target
        // subset of `HasValidUnitPresentInTargetList`
        // and the holder range removal).
        if let CastTarget::Unit(victim) = target
            && victim != caster
        {
            if !self.unit_is_alive(victim) || self.unit_position(victim).is_none() {
                self.end_channel(caster, true);
                return;
            }
            let mut max_range = spell.range_max(&self.stores.spells.tables);
            if Self::is_hostile_pair(caster, victim) {
                max_range *= 1.33;
            } else {
                max_range += 1.25;
            }
            let cpos = self.unit_position(caster).unwrap_or_default();
            let vpos = self.unit_position(victim).unwrap_or_default();
            let dist = ((cpos.x - vpos.x).powi(2) + (cpos.y - vpos.y).powi(2)).sqrt();
            if dist > max_range {
                self.end_channel(caster, true);
                return;
            }
        }
        // Per-second drain (holder `m_timeCla`).
        let drain_due = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.channel_cost_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                if cast.channel_cost_ms <= 0 {
                    cast.channel_cost_ms = 1000;
                    true
                } else {
                    false
                }
            }
            None => return,
        };
        if drain_due {
            #[allow(clippy::cast_possible_wrap)]
            let amount = spell.mana_per_second as i32
                + spell.mana_per_second_per_level as i32 * self.unit_level(caster) as i32;
            if amount > 0 {
                #[allow(clippy::cast_sign_loss)]
                let amount = amount as u32;
                let current = self.unit_power(caster, spell.power_type);
                // Health-cost channels never pay the
                // last point (fork funnel guard).
                let can_pay = if spell.power_type == power::HEALTH {
                    current > amount
                } else {
                    current >= amount
                };
                if can_pay {
                    self.unit_set_power(caster, spell.power_type, current - amount);
                } else {
                    self.end_channel(caster, true);
                    if self.players.contains_key(&caster) {
                        self.send_cast_result(caster, spell_id, cast_result::FAILED_FIZZLE);
                    }
                    return;
                }
            }
        }
        let done = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                cast.remaining_ms <= 0
            }
            None => return,
        };
        if done {
            self.end_channel(caster, false);
        }
    }

    /// Remove one caster's holders of a spell from a
    /// unit (`RemoveAurasByCasterSpell`).
    pub(crate) fn remove_auras_by_caster_spell(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        caster: ObjectGuid,
    ) {
        loop {
            let index = self.unit_base_mut(victim).and_then(|u| {
                u.auras
                    .iter()
                    .rposition(|h| h.spell_id == spell_id && h.caster == caster)
            });
            let Some(index) = index else { return };
            self.remove_aura_at(victim, index);
        }
    }

    /// Channel end (`SendChannelUpdate(0)` + field
    /// reset). Interrupted channels strip their
    /// holders now; a natural finish leaves them for
    /// `update_auras` later this tick so the final
    /// periodic tick still lands (the fork updates
    /// holders inside the spell update first). The
    /// fork's 1 s delayed field reset on natural
    /// finish is an immediate clear here.
    fn end_channel(&mut self, caster: ObjectGuid, interrupted: bool) {
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        if interrupted {
            if let CastTarget::Unit(victim) = cast.target
                && victim != caster
            {
                self.remove_auras_by_caster_spell(victim, cast.spell_id, caster);
            }
            self.remove_auras_by_caster_spell(caster, cast.spell_id, caster);
        }
        if let Some(u) = self.unit_base_mut(caster) {
            let values = u.values();
            values.set_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT, 0);
            values.set_u32(offsets::UNIT_CHANNEL_SPELL, 0);
        }
        // RemoveDynObject(spellId) at channel end
        // (fork cancel + finish paths).
        self.remove_dynobjects_by_spell(caster, cast.spell_id);
        if self.players.contains_key(&caster) {
            let mut payload = PacketWriter::new();
            payload.write_u32_le(0);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_UPDATE, payload.into_vec());
            self.send_to_player(caster, packet);
        }
    }

    /// Resolved cast destination (`m_targets` dest):
    /// client destination, unit-target position, or
    /// the caster position (the fork's mask-0 fill).
    fn cast_dest(&self, caster: ObjectGuid, spell: &SpellEntry, target: CastTarget) -> Position {
        if let CastTarget::Dest(pos) = target {
            return pos;
        }
        // `TARGET_LOCATION_DATABASE` fill for
        // non-teleport effects (`Spell.cpp` target-map
        // build): the store row supplies the dest when
        // it is on this map.
        if spell.effects.iter().any(|e| {
            e.implicit_target_a == tgt::LOCATION_DATABASE && e.effect != effect::TELEPORT_UNITS
        }) && let Some(loc) = self.stores.spell_target_positions.get(spell.id)
            && u32::from(loc.map_id) == self.map_id
        {
            return loc.position();
        }
        match target {
            CastTarget::Dest(pos) => pos,
            CastTarget::Unit(g) => self
                .unit_position(g)
                .or_else(|| self.unit_position(caster))
                .unwrap_or_default(),
            CastTarget::SelfCast => self.unit_position(caster).unwrap_or_default(),
        }
    }

    /// `ComputeEffectRadius` data half: radius entry
    /// for one effect (spellmod applied by callers
    /// that need it).
    pub(crate) fn effect_radius(&self, spell: &SpellEntry, i: usize) -> f32 {
        self.stores
            .spells
            .tables
            .radii
            .get(&spell.effects[i].radius_index)
            .copied()
            .unwrap_or(0.0)
    }

    /// Units within `radius` (2D) of `center`
    /// hostile (`enemy`) or friendly to `caster`;
    /// the ground-enumeration victim search.
    pub(crate) fn units_around(
        &self,
        caster: ObjectGuid,
        center: Position,
        radius: f32,
        enemy: bool,
    ) -> Vec<ObjectGuid> {
        let mut out = Vec::new();
        let within = |pos: Position| {
            (pos.x - center.x).powi(2) + (pos.y - center.y).powi(2) <= radius * radius
        };
        for (guid, creature) in &self.creatures {
            if !creature.unit.is_alive() {
                continue;
            }
            if Self::is_hostile_pair(caster, *guid) == enemy
                && within(creature.unit.world.position())
            {
                out.push(*guid);
            }
        }
        for guid in self.players.keys() {
            if !self.unit_is_alive(*guid) {
                continue;
            }
            if Self::is_hostile_pair(caster, *guid) == enemy
                && self.unit_position(*guid).is_some_and(within)
            {
                out.push(*guid);
            }
        }
        out
    }

    #[allow(
        clippy::too_many_lines,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )] // one SetTargetMap port
    fn collect_targets(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Vec<ObjectGuid> {
        let eff_index = spell
            .effects
            .iter()
            .position(|e| e.effect != effect::NONE)
            .unwrap_or(0);
        let mut mode = spell.effects[eff_index].implicit_target_a;
        if spell.effects[eff_index].effect == effect::NONE {
            mode = tgt::UNIT_CASTER;
        }
        // `SpellScript::OnSetTargetMap` (fork
        // `Spell::SetTargetMap` head): script may
        // override mode/radius/max-targets/closest
        // before the target switch. Radius starts at
        // the effect radius the arms below used
        // before this seam (the fork's max-range
        // fallback for radius_index 0 is a
        // pre-existing subset gap). The cone arm
        // keeps its own radius computation.
        let mut radius = self.effect_radius(spell, 0);
        let mut max_targets = spell.max_affected_targets;
        let mut select_closest = false;
        self.spell_script_on_set_target_map(
            caster,
            spell,
            eff_index,
            &mut mode,
            &mut radius,
            &mut max_targets,
            &mut select_closest,
        );
        let mut targets = match mode {
            tgt::UNIT_ENEMY | tgt::UNIT_FRIEND | tgt::UNIT => match target {
                CastTarget::Unit(g) => vec![g],
                CastTarget::SelfCast | CastTarget::Dest(_) => vec![caster],
            },
            tgt::ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC | tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC => {
                let center = if mode == tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC {
                    self.unit_position(caster).unwrap_or_default()
                } else {
                    self.cast_dest(caster, spell, target)
                };
                self.units_around(caster, center, radius, true)
            }
            tgt::ENUM_UNITS_ENEMY_IN_CONE => {
                // `FillAreaTargets(PUSH_IN_CONE)`:
                // enemies within radius (effect
                // radius, else max range) inside the
                // spell's cone arc (negative arc =
                // back cone).
                let center = self.unit_position(caster).unwrap_or_default();
                let radius = if spell.effects[0].radius_index != 0 {
                    self.effect_radius(spell, 0)
                } else {
                    spell.range_max(&self.stores.spells.tables)
                };
                let angle = self.stores.spells.cone_angle(spell.id);
                self.units_around(caster, center, radius, true)
                    .into_iter()
                    .filter(|g| {
                        if angle > 0.0 {
                            self.has_in_arc(caster, *g, angle)
                        } else if angle < 0.0 {
                            !self.has_in_arc(caster, *g, std::f32::consts::TAU + angle)
                        } else {
                            false
                        }
                    })
                    .collect()
            }
            tgt::ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC | tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC => {
                let center = if mode == tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC {
                    self.unit_position(caster).unwrap_or_default()
                } else {
                    self.cast_dest(caster, spell, target)
                };
                self.units_around(caster, center, radius, false)
            }
            tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::UNIT_PARTY
            | tgt::UNIT_FRIEND_AND_PARTY => {
                // Party system not built: caster only
                // (Battle Shout self-buff evidence).
                vec![caster]
            }
            tgt::LOCATION_CASTER_SRC | tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT => {
                // A names the caster as the area source; B
                // selects the units there (fork `Spell.cpp`
                // target map). Script AoE (`7`) fills
                // `SPELL_TARGETS_ALL` around the source and
                // drops the caster; `spell_script_target`
                // bounds filtering is not modeled yet.
                let center = self.unit_position(caster).unwrap_or_default();
                let radius = radius;
                let target_b = spell
                    .effects
                    .iter()
                    .find(|e| e.effect != effect::NONE)
                    .map_or(0, |e| e.implicit_target_b);
                if mode == tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT
                    || target_b == tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT
                {
                    let mut all = self.units_around(caster, center, radius, true);
                    all.extend(self.units_around(caster, center, radius, false));
                    all.retain(|g| *g != caster);
                    all
                } else if target_b == tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC {
                    self.units_around(caster, center, radius, true)
                } else if target_b == tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC {
                    self.units_around(caster, center, radius, false)
                } else {
                    vec![caster]
                }
            }
            _ => vec![caster],
        };
        // `SpellScript::OnCheckTarget` per-candidate
        // filter (fork `Spell::CheckTarget` during
        // target-map fill; eff-index granularity is
        // the first-effect subset of this merged
        // victim list).
        let main_target = Self::script_unit_target(target);
        targets.retain(|v| {
            self.spell_script_on_check_target(caster, spell, main_target, *v, eff_index)
        });
        // Fork max-target trim (`Spell.cpp` tail of
        // SetTargetMap): closest-first when the
        // script asked, else random erasure; the
        // explicit unit target is kept (fork handles
        // it on the separate main-target path).
        if max_targets > 0 && targets.len() > max_targets as usize {
            if select_closest {
                let cpos = self.unit_position(caster).unwrap_or_default();
                targets.sort_by(|a, b| {
                    let da = self.unit_position(*a).map_or(f32::MAX, |p| {
                        (p.x - cpos.x).powi(2) + (p.y - cpos.y).powi(2)
                    });
                    let db = self.unit_position(*b).map_or(f32::MAX, |p| {
                        (p.x - cpos.x).powi(2) + (p.y - cpos.y).powi(2)
                    });
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });
                targets.truncate(max_targets as usize);
            } else {
                while targets.len() > max_targets as usize {
                    let candidates: Vec<usize> = targets
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| Some(**g) != main_target)
                        .map(|(i, _)| i)
                        .collect();
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let pick = if candidates.is_empty() {
                        self.spell_rng().irand(0, targets.len() as i32 - 1) as usize
                    } else {
                        candidates[self.spell_rng().irand(0, candidates.len() as i32 - 1) as usize]
                    };
                    targets.remove(pick);
                }
            }
        }
        targets
    }

    /// A bare `SendSpellGo` for effect customs that
    /// re-emit the parent cast's GO (Shrink Ray):
    /// caster as first guid (cast-item first-guid
    /// unmodeled here), one hit, unit target.
    pub(crate) fn send_spell_go_simple(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        target: ObjectGuid,
        cast_counter: u8,
    ) {
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(target));
        payload.write_u8(cast_counter);
        payload.write_u32_le(spell_id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(self.now_ms);
        payload.write_u8(1);
        payload.write_u64_le(target.raw());
        payload.write_u8(0);
        Self::targets_write(CastTarget::Unit(target), caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_GO, payload.into_vec());
        self.broadcast_movement(caster, &packet);
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
            CastTarget::SelfCast | CastTarget::Dest(_) => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(Self::cast_first_guid(
            caster, cast,
        )));
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
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one DoAllEffectOnTarget port
    fn apply_effects(
        &mut self,
        caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        victim: ObjectGuid,
        client_dest: Option<Position>,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        proc_ctx: Option<&ProcRaiseCtx>,
        bp: [Option<i32>; 3],
        has_cast_item: bool,
        cast_counter: u8,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
    ) {
        // EventAI SpellHit hooks (Slice 6b), once per
        // hit target as in `DoAllEffectOnTarget`.
        if victim.is_creature() && victim != caster {
            self.eventai_on_spell_hit(victim, caster, spell.id, spell.school);
        }
        if victim.is_creature() {
            self.creature_script_spell_hit(victim, caster, spell.id);
        }
        // `ZoneScript::OnCreatureSpellHit` (`Spell.cpp:1672`),
        // beside the EventAI hooks above (the fork fires
        // both at the end of `DoAllEffectOnTarget`).
        if victim.is_creature() {
            self.instance_on_creature_spell_hit(caster, victim, spell.id);
        }
        // `InstanceData::CustomSpellCasted`: the fork's
        // only call site is the spell 27517 dummy arm
        // (`SpellEffects.cpp:567`, BRD Theldren).
        if spell.id == 27_517 {
            self.instance_custom_spell_casted(spell.id, caster);
        }
        if caster.is_creature() && victim != caster {
            self.eventai_on_spell_hit_target(caster, victim, spell.id, spell.school);
        }
        if caster.is_creature() {
            self.creature_script_spell_hit_target(caster, victim, spell.id);
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
        let mut holder_applied = false;
        // `Spell::damage` script channel for this
        // victim's effect pass (`OnEffectExecute` writes).
        let mut script_damage = 0.0f32;
        for i in 0..3 {
            let eff = spell.effects[i];
            if eff.effect == effect::NONE {
                continue;
            }
            // Per-effect immunity (fork strips the
            // effect from the target's effectMask).
            if self.is_immune_to_spell_effect(victim, spell, i, victim == caster) {
                continue;
            }
            if !self.spell_script_on_effect_execute(
                caster,
                spell,
                victim,
                i,
                &mut script_damage,
                triggered_by_aura_spell,
                triggered_by_spell,
            ) {
                continue;
            }
            match eff.effect {
                // `EffectInstaKill` (fork
                // SpellEffects.cpp:265): direct damage
                // equal to current health, no armor.
                // `EffectDispel` (fork
                // SpellEffects.cpp:2402): candidates
                // are holders whose spell dispel type
                // is in the effect's mask; for MAGIC/
                // POISON, positive auras stay on
                // friendly targets and negative on
                // hostile. Removes up to the effect
                // value (fork picks randomly with a
                // level-based resist roll; stored
                // order + no resist here, recorded).
                effect::DISPEL => {
                    let dispel_mask: u32 = if eff.misc_value < 0 || eff.misc_value == 7 {
                        0x3FE
                    } else if eff.misc_value < 31 {
                        1u32 << eff.misc_value
                    } else {
                        0
                    };
                    let friendly = !Self::is_hostile_pair(caster, victim);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count = self.effect_value_bp(spell, i, caster_level, bp[i]) as usize;
                    let mut remove: Vec<usize> = Vec::new();
                    let holders = self.aura_holders(victim);
                    {
                        for (idx, holder) in holders.iter().enumerate() {
                            if remove.len() >= count {
                                break;
                            }
                            let Some(entry) = self.stores.spells.get(holder.spell_id) else {
                                continue;
                            };
                            if entry.dispel >= 32 || (1u32 << entry.dispel) & dispel_mask == 0 {
                                continue;
                            }
                            if matches!(entry.dispel, 1 | 4) && holder.positive == friendly {
                                continue;
                            }
                            remove.push(idx);
                        }
                    }
                    for idx in remove.into_iter().rev() {
                        self.remove_aura_at(victim, idx);
                    }
                }
                // `EffectLearnSpell` (fork
                // SpellEffects.cpp): teach the
                // trigger spell to a player victim.
                // Runtime set = `deferred.spells`,
                // which the save diff persists as
                // `character_spell` inserts; pet
                // branch unmodeled (no pet runtime).
                effect::LEARN_SPELL => {
                    if victim.is_player() && eff.trigger_spell != 0 {
                        let learned = eff.trigger_spell;
                        let fresh = self
                            .players
                            .get_mut(&victim)
                            .is_some_and(|entry| entry.player.learn_spell(learned));
                        if fresh {
                            let mut payload = PacketWriter::new();
                            payload.write_u32_le(learned);
                            let packet =
                                WorldPacket::with_body(op::SMSG_LEARNED_SPELL, payload.into_vec());
                            self.send_to_player(victim, packet);
                        }
                    }
                }
                effect::INSTAKILL => {
                    let hp = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
                    if hp > 0 {
                        self.deal_damage(caster, victim, hp, false);
                    }
                }
                // `EffectThreat` (fork
                // SpellEffects.cpp:3432): flat threat
                // from the effect value on the victim's
                // threat list.
                effect::THREAT => {
                    if victim.is_creature() {
                        let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
                        self.add_threat(victim, caster, value);
                    }
                }
                effect::SCHOOL_DAMAGE => {
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let base =
                        mangos_spells::ditheru(value + script_damage, self.spell_rng()) as i32;
                    self.school_damage(
                        caster,
                        spell,
                        victim,
                        base,
                        i as u32,
                        None,
                        applied_mods.as_deref_mut(),
                        proc_ctx,
                    );
                }
                // `EffectWeaponDmg` (fork SpellEffects.cpp:3321):
                // bonus = sum of WEAPON_DAMAGE / NOSCHOOL /
                // NORMALIZED effect values; WEAPON_PERCENT
                // multiplies the whole swing; only the first
                // weapon effect executes (one swing per cast).
                // Deviations: main-hand only; AP
                // normalization unmodeled (unit fields bake
                // AP/speed already); avoidance rides the
                // spell hit roll, not the melee table.
                effect::WEAPON_DAMAGE
                | effect::WEAPON_DAMAGE_NOSCHOOL
                | effect::WEAPON_PERCENT_DAMAGE
                | effect::NORMALIZED_WEAPON_DMG => {
                    let is_weapon = |e: u32| {
                        matches!(
                            e,
                            effect::WEAPON_DAMAGE
                                | effect::WEAPON_DAMAGE_NOSCHOOL
                                | effect::WEAPON_PERCENT_DAMAGE
                                | effect::NORMALIZED_WEAPON_DMG
                        )
                    };
                    if !spell.effects[..i].iter().any(|e| is_weapon(e.effect)) {
                        let mut bonus = script_damage;
                        let mut pct = 1.0f32;
                        for (j, e) in spell.effects.iter().enumerate() {
                            let v = self.effect_value_bp(spell, j, caster_level, bp[j]);
                            match e.effect {
                                effect::WEAPON_DAMAGE
                                | effect::WEAPON_DAMAGE_NOSCHOOL
                                | effect::NORMALIZED_WEAPON_DMG => bonus += v,
                                effect::WEAPON_PERCENT_DAMAGE => pct *= v / 100.0,
                                _ => {}
                            }
                        }
                        let min_dmg = f32::from_bits(
                            self.unit_value_u32(caster, offsets::UNIT_FIELD_MINDAMAGE),
                        );
                        let max_dmg = f32::from_bits(
                            self.unit_value_u32(caster, offsets::UNIT_FIELD_MAXDAMAGE),
                        );
                        let (lo, hi) = if min_dmg > max_dmg {
                            (max_dmg, min_dmg)
                        } else {
                            (min_dmg, max_dmg)
                        };
                        let swing = self
                            .rng
                            .frand(lo.max(0.0), if hi == 0.0 { 5.0 } else { hi });
                        #[allow(clippy::cast_possible_wrap)]
                        let damage = mangos_spells::ditheru(
                            ((swing + bonus) * pct).max(0.0),
                            self.spell_rng(),
                        ) as i32;
                        if damage > 0 {
                            self.school_damage(
                                caster,
                                spell,
                                victim,
                                damage,
                                i as u32,
                                None,
                                applied_mods.as_deref_mut(),
                                proc_ctx,
                            );
                        }
                    }
                }
                effect::HEAL => {
                    // Script `spell->damage` adjustment
                    // (e.g. Swiftmend) rides the same
                    // additive channel as SCHOOL_DAMAGE.
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]) + script_damage;
                    // `EffectHeal`: HealingBonusDone
                    // (coefficient on the advertised
                    // benefit, spellmod tail) then
                    // HealingBonusTaken.
                    let value = self.spell_healing_bonus_done(
                        caster,
                        spell,
                        i,
                        value,
                        false,
                        1,
                        applied_mods.as_deref_mut(),
                    );
                    let value = self.spell_healing_bonus_taken(
                        victim,
                        spell,
                        i,
                        value,
                        false,
                        1,
                        applied_mods.as_deref_mut(),
                    );
                    if value > 0.0 {
                        let amount = mangos_spells::ditheru(value, self.spell_rng());
                        self.heal_unit(victim, amount);
                        if let Some(ctx) = proc_ctx {
                            self.raise_damage_procs(
                                caster,
                                victim,
                                Some(spell.id),
                                ctx,
                                proc_ex::NORMAL_HIT,
                                amount,
                                amount,
                            );
                        }
                    }
                }
                // `Spell::EffectEnergize` +
                // `Unit::EnergizeBySpell`: direct power
                // gain (the SMSG_SPELL_ENERGIZE_LOG rides
                // the same deferral as heal's log here).
                effect::ENERGIZE => {
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
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
                                triggered_by_aura: false,
                                skip_known: true,
                                cast_item: None,
                                custom_base_points: [None; 3],
                                triggered_by_aura_spell: 0,
                                triggered_by_spell: spell.id,
                            },
                        );
                    }
                }
                effect::APPLY_AURA | effect::APPLY_AREA_AURA_PARTY => {
                    // Dynobj-location aura effects (28/29)
                    // are delivered by the dynamic object
                    // updater, never directly at cast time.
                    // All aura effects ride one holder
                    // (built once); later non-aura effects
                    // still execute (fork `HandleEffects`
                    // walks the full effect mask).
                    if !holder_applied
                        && eff.implicit_target_a != tgt::ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION
                        && eff.implicit_target_a != tgt::ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION
                    {
                        self.apply_spell_aura(
                            caster,
                            spell,
                            victim,
                            applied_mods.as_deref_mut(),
                            bp,
                        );
                        holder_applied = true;
                    }
                }
                // SPELL_EFFECT_SUMMON / SUMMON_WILD /
                // SUMMON_GUARDIAN (Slice 7).
                28 | 41 | 42 if eff.misc_value > 0 => {
                    self.summon_effect(caster, caster_level, spell, victim, i, bp[i]);
                }
                // SPELL_EFFECT_SUMMON_PET (Tier 1 Slice
                // 2): the fork ignores the effect victim
                // and summons for the caster; run once
                // on the caster's own hit.
                effect::SUMMON_PET if victim == caster => {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_sign_loss,
                        clippy::cast_possible_truncation
                    )]
                    let pet_level = if caster.is_player() {
                        caster_level
                    } else {
                        (caster_level as i32 + eff.multiple_value as i32).max(1) as u32
                    };
                    #[allow(clippy::cast_sign_loss)]
                    self.summon_pet_effect(
                        caster,
                        spell.id,
                        eff.misc_value.max(0) as u32,
                        pet_level,
                    );
                }
                // SPELL_EFFECT_LEARN_PET_SPELL.
                effect::LEARN_PET_SPELL
                    if victim == caster && caster.is_player() && eff.trigger_spell != 0 =>
                {
                    self.learn_pet_spell_effect(caster, eff.trigger_spell);
                }
                // SPELL_EFFECT_CREATE_ITEM (Tier 1
                // Slice 3a).
                effect::CREATE_ITEM if victim.is_player() => {
                    self.create_item_effect(victim, spell, i, caster_level, bp[i]);
                }
                // SPELL_EFFECT_TELEPORT_UNITS /
                // SPELL_EFFECT_LEAP (Tier 1 Slice 5).
                effect::TELEPORT_UNITS => {
                    self.teleport_units_effect(caster, spell, i, victim, client_dest);
                }
                effect::TELEPORT_UNITS_FACE_CASTER => {
                    self.tele_units_face_caster_effect(caster, spell, i, victim, client_dest);
                }
                effect::LEAP => {
                    self.leap_effect(caster, spell, i, victim, client_dest);
                }
                // SPELL_EFFECT_KNOCK_BACK /
                // SPELL_EFFECT_PLAYER_PULL (Tier 1
                // Slice 6). Values: horizontal =
                // misc/10, vertical = value/10;
                // PLAYER_PULL pulls toward the caster
                // with distance-clamped negative
                // horizontal speed and misc vertical.
                effect::KNOCK_BACK => {
                    #[allow(clippy::cast_precision_loss)]
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    #[allow(clippy::cast_precision_loss)]
                    let horizontal = eff.misc_value as f32 / 10.0;
                    self.remove_auras_due_to_spell(victim, 24778);
                    self.knock_back_from(victim, caster, horizontal, damage / 10.0);
                }
                effect::PLAYER_PULL => {
                    #[allow(clippy::cast_precision_loss)]
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    let dist = match (self.unit_position(victim), self.unit_position(caster)) {
                        (Some(v), Some(c)) => {
                            let d = ((v.x - c.x).powi(2) + (v.y - c.y).powi(2)).sqrt();
                            if damage != 0.0 && d > damage {
                                damage
                            } else {
                                d
                            }
                        }
                        _ => 0.0,
                    };
                    #[allow(clippy::cast_precision_loss)]
                    let vertical = eff.misc_value as f32 / 10.0;
                    self.knock_back_from(victim, caster, -dist, vertical);
                }
                // `EffectDummy` / `EffectScriptEffect`:
                // per-spell customs (customs.rs); the
                // value is the fork's per-effect
                // `CalculateDamage` result.
                effect::DUMMY => {
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    self.effect_dummy(
                        caster,
                        spell,
                        i,
                        victim,
                        damage,
                        has_cast_item,
                        cast_counter,
                        triggered_by_aura_spell,
                        triggered_by_spell,
                    );
                }
                effect::SCRIPT_EFFECT => {
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    self.effect_script_effect(caster, spell, victim, damage, has_cast_item);
                }
                // SPELL_EFFECT_PERSISTENT_AREA_AURA
                // spawns once per cast in
                // `finish_cast` (it is not a per-victim
                // effect; mixed spells like Flamestrike
                // enumerate direct-hit victims too).
                // Includes PERSISTENT_AREA_AURA (see
                // above): no per-victim work.
                _ => {}
            }
        }
        // `SpellScript::OnAfterHit` (fork `Spell.cpp`
        // DoAllEffectOnTarget unit case, after the
        // effect pass and procs for this target).
        self.spell_script_on_after_hit(caster, spell, victim);
    }

    /// `Unit::NearTeleportTo` dispatch: players take
    /// the controller ACK train, creatures the
    /// observer teleport packets.
    fn near_teleport_unit(&mut self, victim: ObjectGuid, position: Position) {
        if victim.is_player() {
            self.teleport_player_in_map(victim, position);
        } else {
            self.teleport_creature_in_map(victim, position);
        }
    }

    /// Same-map near teleport or cross-map player
    /// transfer (`EffectTeleportUnits` location arms;
    /// creatures are silently skipped cross-map,
    /// exactly as the fork).
    pub(crate) fn teleport_to_location(
        &mut self,
        victim: ObjectGuid,
        loc: mangos_world_data::spawns::WorldLocation,
    ) {
        if u32::from(loc.map_id) == self.map_id {
            self.near_teleport_unit(victim, loc.position());
        } else if victim.is_player() {
            self.events.push(MapEvent::TeleportPlayer {
                player: victim,
                map_id: u32::from(loc.map_id),
                position: loc.position(),
            });
        }
    }

    /// `GetObjectBoundingRadius` (the unit field).
    fn object_bounding_radius(&self, guid: ObjectGuid) -> f32 {
        self.unit_base(guid).map_or(0.389, |u| {
            u.world
                .object
                .values()
                .get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS)
        })
    }

    /// `Spell::EffectTeleUnitsFaceCaster`
    /// (`SpellEffects.cpp` effect 43): teleport the
    /// victim to the cast destination when the
    /// target map carries one, else to the caster's
    /// close point at the effect radius
    /// (`GetClosePoint` primary position; the
    /// `ObjectPosSelector` crowding walk and its
    /// vmap `IsWithinLOS` retries ride the recorded
    /// vmap-manager remainder), facing
    /// `-caster orientation` (fork quirk, not
    /// +π). The `TELE_TO_NOT_LEAVE_COMBAT |
    /// TELE_TO_NOT_UNSUMMON_PET | TELE_TO_SPELL`
    /// (self) flags match `near_teleport_unit`'s
    /// existing behavior. The taxi-flight gate has
    /// no system behind it yet (same as
    /// `EffectTeleportUnits`).
    fn tele_units_face_caster_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        let Some(caster_pos) = self.unit_position(caster) else {
            return;
        };
        if self.unit_position(victim).is_none() {
            return;
        }
        let mut dest = if let Some(dest) = client_dest {
            dest
        } else {
            let dis = self.effect_radius(spell, i);
            let reach =
                self.object_bounding_radius(caster) + self.object_bounding_radius(victim) + dis;
            // GetNearPoint2D + NormalizeMapCoord.
            let x =
                (caster_pos.x + reach * caster_pos.o.cos()).clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
            let y =
                (caster_pos.y + reach * caster_pos.o.sin()).clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
            // searcher == null: UpdateGroundPositionZ
            let z = self.ground_z(x, y, caster_pos.z);
            Position::new(x, y, z, caster_pos.o)
        };
        dest.o = -caster_pos.o;
        self.near_teleport_unit(victim, dest);
    }

    /// `Spell::EffectTeleportUnits`
    /// (`SpellEffects.cpp:1536`), switched on the
    /// effect's implicit target B. The taxi-flight
    /// gate has no system behind it yet.
    fn teleport_units_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        if self.unit_position(victim).is_none() {
            return;
        }
        match spell.effects[i].implicit_target_b {
            tgt::LOCATION_CASTER_HOME_BIND => {
                if !victim.is_player() {
                    return;
                }
                let Some(loc) = self
                    .players
                    .get(&victim)
                    .map(|entry| entry.player.homebind.location)
                else {
                    return;
                };
                self.teleport_to_location(victim, loc);
            }
            tgt::LOCATION_DATABASE | tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT => {
                let Some(loc) = self.stores.spell_target_positions.get(spell.id) else {
                    return;
                };
                self.teleport_to_location(victim, loc);
            }
            tgt::LOCATION_CASTER_DEST => {
                let (Some(mut dest), Some(caster_pos)) =
                    (self.unit_position(victim), self.unit_position(caster))
                else {
                    return;
                };
                dest.o = caster_pos.o;
                self.near_teleport_unit(caster, dest);
            }
            _ => {
                let Some(mut dest) = client_dest else {
                    return;
                };
                if let Some(victim_pos) = self.unit_position(victim) {
                    dest.o = victim_pos.o;
                }
                self.near_teleport_unit(victim, dest);
            }
        }
    }

    /// Directional dest-fill angle offset
    /// (`Spell.cpp` target-map build).
    fn directional_offset(mode: u32) -> Option<f32> {
        use std::f32::consts::PI;
        match mode {
            tgt::LOCATION_CASTER_FRONT_LEFT | tgt::LOCATION_UNIT_MINION_POSITION => Some(PI * 0.25),
            tgt::LOCATION_CASTER_BACK_LEFT => Some(PI * 0.75),
            tgt::LOCATION_CASTER_BACK => Some(PI),
            tgt::LOCATION_CASTER_BACK_RIGHT => Some(PI * 1.25),
            tgt::LOCATION_CASTER_FRONT_RIGHT => Some(PI * 1.75),
            tgt::LOCATION_CASTER_LEFT => Some(PI / 2.0),
            tgt::LOCATION_CASTER_RIGHT => Some(-PI / 2.0),
            tgt::LOCATION_CASTER_FRONT | tgt::LOCATION_CASTER_FRONT_LEAP => Some(0.0),
            _ => None,
        }
    }

    /// `WorldObject::MovePositionToFirstCollision`
    /// (`Object.cpp:1999`): navmesh path toward
    /// `from + dist·angle` (`PathFinder::calculate
    /// (src, dest, false, true)`, back point wins),
    /// `UpdateAllowedPositionZ`, the two vmap
    /// `GetLosHitPosition` trims at collision-height
    /// offsets (−1.0f horizontal, −0.5f vertical;
    /// static tree only — the dynamic gameobject
    /// half is not modeled), then the 10-step
    /// Z-safety walkback and `NormalizeMapCoord`.
    /// The water-surface arm of
    /// `UpdateAllowedPositionZ` rides the same
    /// `ground_z` approximation as the rest of the
    /// motion code.
    fn first_collision_dest(
        &mut self,
        from: Position,
        angle: f32,
        mut dist: f32,
        half_height: f32,
    ) -> Position {
        /// `ATTACK_DISTANCE` (`ObjectDefines.h:25`).
        const ATTACK_DISTANCE: f32 = 5.0;
        if dist <= 0.0 {
            return from;
        }
        let mut dest_x = from.x + dist * angle.cos();
        let mut dest_y = from.y + dist * angle.sin();
        let mut dest_z = from.z;
        let end = Position::new(dest_x, dest_y, dest_z + half_height, from.o);
        let path = self.find_path_points(from, end, false, true);
        if let Some(back) = path.points.last() {
            dest_x = back.x;
            dest_y = back.y;
            dest_z = back.z;
        }
        dest_z = self.ground_z(dest_x, dest_y, dest_z);
        // Horizontal trim at eye level, then the
        // vertical probe onto the destination.
        if let Some((hx, hy, hz)) = self.get_los_hit_position(
            from.x,
            from.y,
            from.z + half_height,
            dest_x,
            dest_y,
            dest_z + half_height,
            -1.0,
        ) {
            dest_x = hx;
            dest_y = hy;
            dest_z = hz - half_height;
            dist = ((from.x - dest_x).powi(2) + (from.y - dest_y).powi(2)).sqrt();
        }
        if let Some((_, _, hz)) = self.get_los_hit_position(
            dest_x,
            dest_y,
            dest_z + half_height,
            dest_x,
            dest_y,
            dest_z,
            -0.5,
        ) {
            dest_z = hz;
        }
        // Fork Z-safety probe: step back toward the
        // source in tenths until the terrain Z is
        // within ATTACK_DISTANCE of the source Z. A
        // step-to-step cliff (> ATTACK_DISTANCE/2)
        // marks the full destination unsafe; a fully
        // safe probe restores the full destination
        // (fork overwrites the accepted point with
        // `tempPos` too).
        let step = dist / 10.0;
        let full = (dest_x, dest_y, dest_z);
        let mut result = from;
        let mut z_safe = true;
        for _ in 0..10 {
            if (from.z - dest_z).abs() <= ATTACK_DISTANCE {
                result = Position::new(dest_x, dest_y, dest_z, from.o);
                break;
            }
            let previous_z = dest_z;
            dest_x -= step * angle.cos();
            dest_y -= step * angle.sin();
            dest_z = self.ground_z(dest_x, dest_y, dest_z);
            if (previous_z - dest_z).abs() > ATTACK_DISTANCE * 0.5 {
                z_safe = false;
            }
        }
        if z_safe {
            result = Position::new(full.0, full.1, full.2, from.o);
        }
        result.x = result.x.clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
        result.y = result.y.clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
        result.z = self.ground_z(result.x, result.y, result.z);
        result
    }

    /// `Spell::EffectLeapForward`
    /// (`SpellEffects.cpp:5187`) with the directional
    /// destination fill from target-map build
    /// (client dest wins when present, as in the
    /// fork).
    fn leap_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        let dest = if let Some(dest) = client_dest {
            dest
        } else {
            let Some(from) = self.unit_position(caster) else {
                return;
            };
            let eff = spell.effects[i];
            let offset = Self::directional_offset(eff.implicit_target_b)
                .or_else(|| Self::directional_offset(eff.implicit_target_a))
                .unwrap_or(0.0);
            let radius = if eff.radius_index == 0 {
                0.0
            } else {
                self.effect_radius(spell, i)
            };
            // GetCollisionHeight (model height
            // already scale-ratioed at display set).
            let half_height = self
                .unit_base(caster)
                .map_or(2.0, |u| u.model_collision_height);
            self.first_collision_dest(from, from.o + offset, radius, half_height)
        };
        let mut dest = dest;
        if let Some(victim_pos) = self.unit_position(victim) {
            dest.o = victim_pos.o;
        }
        self.near_teleport_unit(victim, dest);
    }

    /// `Unit::KnockBackFrom` + `Unit::KnockBack`
    /// (`Unit.cpp:9991`): stunned/rooted or
    /// mid-spline units ignore it; only
    /// player-moved units launch in this fork
    /// (creatures and pets are a deliberate no-op —
    /// see the Slice 6 inventory). The controller
    /// packet carries negated vertical speed, and
    /// the pending change stores the wire values.
    fn knock_back_from(
        &mut self,
        victim: ObjectGuid,
        source: ObjectGuid,
        horizontal_speed: f32,
        vertical_speed: f32,
    ) {
        use mangos_entities::unit::{
            KnockbackInfo, MOVEMENT_CHANGE_KNOCK_BACK, PendingMovementChange, UNIT_STATE_ROOT,
            UNIT_STATE_STUNNED,
        };
        let Some(victim_pos) = self.unit_position(victim) else {
            return;
        };
        let blocked = self.unit_base_mut(victim).is_some_and(|unit| {
            unit.has_unit_state(UNIT_STATE_STUNNED | UNIT_STATE_ROOT) || !unit.spline.finalized()
        });
        if blocked {
            return;
        }
        // `InterruptNonMeleeSpells(false)` runs for
        // every unit kind before the player branch.
        self.cancel_cast(victim, true);
        if !victim.is_player() {
            return;
        }
        let angle = if victim == source {
            victim_pos.o + std::f32::consts::PI
        } else {
            let Some(source_pos) = self.unit_position(source) else {
                return;
            };
            (victim_pos.y - source_pos.y).atan2(victim_pos.x - source_pos.x)
        };
        let info = KnockbackInfo {
            vcos: angle.cos(),
            vsin: angle.sin(),
            speed_xy: horizontal_speed,
            speed_z: -vertical_speed,
        };
        let Some(counter) = self.unit_base_mut(victim).map(|unit| {
            let counter = unit.movement_counter_and_inc();
            unit.pending_movement_changes
                .push_back(PendingMovementChange {
                    movement_counter: counter,
                    change_type: MOVEMENT_CHANGE_KNOCK_BACK,
                    new_value: 0.0,
                    apply: false,
                    knockback: Some(info),
                });
            counter
        }) else {
            return;
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(counter);
        payload.write_f32_le(info.vcos);
        payload.write_f32_le(info.vsin);
        payload.write_f32_le(info.speed_xy);
        payload.write_f32_le(info.speed_z);
        let packet = WorldPacket::with_body(op::SMSG_MOVE_KNOCK_BACK, payload.into_vec());
        self.events.push(MapEvent::Packet { to: victim, packet });
        self.events.push(MapEvent::KnockBack {
            player: victim,
            vertical_speed,
        });
    }

    /// `Player::SendEquipError` (`SMSG_INVENTORY_CHANGE_FAILURE`:
    /// result byte, two empty item GUIDs, zero byte).
    fn send_equip_error(&mut self, player: ObjectGuid, result: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u8(result);
        payload.write_u64_le(0);
        payload.write_u64_le(0);
        payload.write_u8(0);
        let packet = WorldPacket::with_body(op::SMSG_INVENTORY_CHANGE_FAILURE, payload.into_vec());
        self.send_to_player(player, packet);
    }

    /// `Spell::DoCreateItem` (`SpellEffects.cpp:1820`)
    /// map half: count from the effect value clamped
    /// to `[1, Stackable]`; unknown prototypes fail
    /// with `EQUIP_ERR_ITEM_NOT_FOUND` (23). The
    /// store itself crosses to the world actor as
    /// [`MapEvent::CreateItem`] (inventory baseline
    /// and DB writes live there); battleground-mark
    /// mail overflow and `UpdateCraftSkill` are
    /// recorded as unmodeled.
    fn create_item_effect(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        caster_level: u32,
        bp: Option<i32>,
    ) {
        let eff = spell.effects[i];
        #[allow(clippy::cast_possible_truncation)]
        let item_id = eff.item_type as u32;
        let Some(stackable) = self
            .stores
            .item_prototypes
            .get(item_id)
            .map(|proto| proto.stackable.max(1))
        else {
            self.send_equip_error(victim, 23);
            return;
        };
        let value = self.effect_value_bp(spell, i, caster_level, bp);
        let rolled = mangos_spells::ditheru(value, self.spell_rng());
        let count = rolled.clamp(1, stackable);
        self.events.push(MapEvent::CreateItem {
            player: victim,
            item_id,
            count,
        });
    }

    /// First packed GUID of `SMSG_SPELL_START`/`GO`:
    /// the cast item for item casts, else the caster
    /// (`Spell.cpp:4611/4645`).
    fn cast_first_guid(caster: ObjectGuid, cast: &ActiveCast) -> ObjectGuid {
        match cast.cast_item {
            Some(item) => ObjectGuid::with_entry(mangos_guid::HighGuid::ITEM, 0, item.item_guid),
            None => caster,
        }
    }

    /// `Spell::TakeCastItem` (`Spell.cpp:5166`):
    /// walk every prototype spell with charges —
    /// negative proto charges mark the item
    /// expendable, each use steps charges one
    /// toward 0 (written back only when
    /// `Stackable < 2`; `withoutCharges` is
    /// overwritten per charged spell, fork-faithful)
    /// — and an expendable item out of charges
    /// loses one count. Baseline edits land here;
    /// DB writes cross as events.
    fn take_cast_item(&mut self, caster: ObjectGuid, item: CastItemCtx) {
        let Some(proto) = self.stores.item_prototypes.get(item.item_entry).cloned() else {
            return;
        };
        let mut events = Vec::new();
        {
            let Some(entry) = self.players.get_mut(&caster) else {
                return;
            };
            let inv = &mut entry.player.saved.inventory;
            let Some(idx) = inv
                .iter()
                .position(|row| row.item.item_guid == item.item_guid)
            else {
                return;
            };
            let mut charges: Vec<i32> = inv[idx]
                .item
                .charges
                .split_whitespace()
                .map(|token| token.parse().unwrap_or(0))
                .collect();
            charges.resize(5, 0);
            let mut expendable = false;
            let mut without_charges = false;
            let mut changed = false;
            for (i, spell_row) in proto.spells.iter().enumerate() {
                if spell_row.spell_id == 0 || spell_row.charges == 0 {
                    continue;
                }
                if spell_row.charges < 0 {
                    expendable = true;
                }
                let mut charge = charges[i];
                if charge != 0 {
                    charge += if charge > 0 { -1 } else { 1 };
                    if proto.stackable < 2 {
                        charges[i] = charge;
                        changed = true;
                    }
                }
                without_charges = charge == 0;
            }
            if changed {
                let text = format!(
                    "{} {} {} {} {} ",
                    charges[0], charges[1], charges[2], charges[3], charges[4]
                );
                inv[idx].item.charges.clone_from(&text);
                events.push(MapEvent::SetItemCharges {
                    item_guid: item.item_guid,
                    charges: text,
                });
            }
            if expendable && without_charges {
                let remaining = inv[idx].item.count.saturating_sub(1);
                if remaining == 0 {
                    inv.remove(idx);
                } else {
                    inv[idx].item.count = remaining;
                }
                events.push(MapEvent::SetItemCount {
                    item_guid: item.item_guid,
                    count: remaining,
                });
            }
        }
        self.events.extend(events);
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
        bp: Option<i32>,
    ) {
        let eff = spell.effects[i];
        let value = self.effect_value_bp(spell, i, caster_level, bp);
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
            if let Some(summon) =
                self.summon_creature(eff.misc_value as u32, pos, caster, kind, duration_ms)
            {
                self.spell_script_on_summon(caster, summon, spell);
            }
        }
    }

    /// `Unit::GetCreatureTypeMask`: players read as
    /// humanoid (fork race-table default; shapeshift
    /// creature types unmodeled).
    pub(crate) fn creature_type_mask(&self, guid: ObjectGuid) -> i32 {
        let creature_type = if self.players.contains_key(&guid) {
            7
        } else {
            self.creatures
                .get(&guid)
                .map_or(0, |c| c.info.creature_type)
        };
        if creature_type == 0 {
            0
        } else {
            #[allow(clippy::cast_possible_wrap)]
            {
                1i32 << (creature_type as i32 - 1)
            }
        }
    }

    /// `Aura::IsAffectedOnSpell` non-spellmod arm.
    fn override_affected(&self, holder: &AuraHolder, eff_idx: usize, spell: &SpellEntry) -> bool {
        let Some(source) = self.stores.spells.get(holder.spell_id) else {
            return false;
        };
        source.spell_family_name == spell.spell_family_name
            && spell.spell_family_flags & source.effects[eff_idx].item_type != 0
    }

    /// `SpellCaster::SpellBonusWithCoeffs`
    /// (`SpellCaster.cpp:1465`): template coefficient
    /// wins over the default formula and skips the
    /// level penalty; custom coefficients next; the
    /// `SPELL_BONUS_DAMAGE` spellmod scales the
    /// coefficient through ×100. `mod_subject` is the
    /// fork `ToUnit()` (caster for Done, victim for
    /// Taken — fork quirk preserved).
    #[allow(clippy::too_many_arguments)]
    fn spell_bonus_with_coeffs(
        &mut self,
        mod_subject: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        total: f32,
        benefit: f32,
        dot: bool,
        done_part: bool,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        if benefit == 0.0 {
            return total;
        }
        let tables = &self.stores.spells.tables;
        let template_coeff = spell.effects[eff_index].bonus_coefficient;
        let (coeff, lvl_penalty) = if template_coeff >= 0.0 {
            (template_coeff, 1.0)
        } else {
            (
                spell.default_coefficient(tables, dot),
                spell.level_penalty(),
            )
        };
        let mut coeff = spell.custom_coefficient(coeff, done_part, 1, None);
        coeff *= 100.0;
        coeff = self.apply_spell_mod(
            mod_subject,
            spell,
            spellmod::SPELL_BONUS_DAMAGE,
            coeff,
            false,
            applied_mods,
        );
        coeff /= 100.0;
        total + benefit * coeff * lvl_penalty
    }

    /// `SpellBaseDamageBonusDone` +
    /// `MOD_FLAT_SPELL_DAMAGE_VERSUS` (wand equipped-item
    /// gates unmodeled — no equipped fields on the entry
    /// subset; pet `GetBonusDamage` unmodeled).
    fn spell_base_damage_bonus_done(
        &self,
        caster: ObjectGuid,
        school_mask: i32,
        creature_type_mask: i32,
    ) -> i32 {
        let mut benefit =
            self.total_aura_modifier(caster, aura_type::MOD_DAMAGE_DONE, Some(school_mask));
        if self.players.contains_key(&caster) {
            for e in self
                .aura_holders(caster)
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| {
                    e.aura_type == aura_type::MOD_SPELL_DAMAGE_OF_STAT_PERCENT
                        && e.misc_value & school_mask != 0
                })
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                {
                    benefit += (self.unit_stat(caster, 4) * e.amount as f32 / 100.0) as i32;
                }
            }
        }
        benefit
            + self.total_aura_modifier(
                caster,
                aura_type::MOD_FLAT_SPELL_DAMAGE_VERSUS,
                Some(creature_type_mask),
            )
    }

    /// `SpellDamageBonusDone` (`SpellCaster.cpp:1286`)
    /// magic arm. Totem owner delegation and the
    /// paladin-seal percent arm are recorded
    /// exclusions (no totem casters / equipped fields).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_damage_bonus_done(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        damage: f32,
        dot: bool,
        stack: u32,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const CUSTOM_FIXED_DAMAGE: u32 = 0x10;
        if spell.custom & CUSTOM_FIXED_DAMAGE != 0 || spell.attributes_ex3 & 0x2000_0000 != 0 {
            return damage;
        }
        // Ignite already includes modifiers (mage family
        // flag bit 27, CM0 0x08000000).
        if spell.spell_family_name == 3 && spell.spell_family_flags & 0x0800_0000 != 0 {
            return damage;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let creature_type_mask = self.creature_type_mask(victim);
        let mut done_total_mod = 1.0f32;
        let mut done_total = 0.0f32;
        let pet_owner = self.pet_owner(caster);
        if let Some(creature) = self.creatures.get(&caster)
            && !pet_owner.is_some_and(|o| self.players.contains_key(&o))
        {
            done_total_mod *= self
                .config
                .creature_rates
                .spell_damage_by_rank(creature.info.rank);
        }
        done_total_mod *= self.total_aura_multiplier(
            caster,
            aura_type::MOD_DAMAGE_PERCENT_DONE,
            Some(school_mask),
        );
        done_total_mod *= self.total_aura_multiplier(
            caster,
            aura_type::MOD_DAMAGE_DONE_VERSUS,
            Some(creature_type_mask),
        );
        done_total += self.total_aura_modifier(
            caster,
            aura_type::MOD_DAMAGE_DONE_CREATURE,
            Some(creature_type_mask),
        ) as f32;
        // Override class scripts (owner's auras).
        let script_owner = pet_owner.unwrap_or(caster);
        for holder in self.aura_holders(script_owner) {
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(e) = eff else { continue };
                if e.aura_type != aura_type::OVERRIDE_CLASS_SCRIPTS
                    || !self.override_affected(&holder, eff_idx, spell)
                {
                    continue;
                }
                match e.misc_value {
                    4418 | 4554 => done_total += e.amount as f32,
                    4555 => {
                        let divisor = if dot {
                            100.0 * f32::from(spell.aura_max_ticks(&self.stores.spells.tables))
                        } else {
                            800.0
                        };
                        if divisor != 0.0 {
                            done_total += e.amount as f32 * damage / divisor;
                        }
                    }
                    _ => {}
                }
            }
        }
        // Hunter pet happiness.
        if self
            .pets
            .get(&caster)
            .is_some_and(super::pets::PetState::is_hunter)
        {
            let happiness = self.unit_power(caster, 4);
            if happiness >= 666_000 {
                done_total_mod *= 1.25;
            } else if happiness < 333_000 {
                done_total_mod *= 0.75;
            }
        }
        let benefit = self.spell_base_damage_bonus_done(caster, school_mask, creature_type_mask);
        done_total = self.spell_bonus_with_coeffs(
            caster,
            spell,
            eff_index,
            done_total,
            benefit as f32,
            dot,
            true,
            applied_mods.as_deref_mut(),
        );
        #[allow(clippy::cast_possible_wrap)]
        let mut out = (damage + done_total * stack as i32 as f32) * done_total_mod;
        out = self.apply_spell_mod(
            caster,
            spell,
            if dot { spellmod::DOT } else { spellmod::DAMAGE },
            out,
            true,
            applied_mods,
        );
        if out > 0.0 { out } else { 0.0 }
    }

    /// `SpellDamageBonusTaken` (`Unit.cpp:5344`).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_damage_bonus_taken(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        damage: f32,
        dot: bool,
        stack: u32,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const EX4_IGNORE_TAKEN: u32 = 0x0000_0100;
        if spell.attributes_ex4 & EX4_IGNORE_TAKEN != 0 {
            return damage;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let taken_total_mod = self.total_aura_multiplier(
            victim,
            aura_type::MOD_DAMAGE_PERCENT_TAKEN,
            Some(school_mask),
        );
        let taken_flat =
            self.total_aura_modifier(victim, aura_type::MOD_DAMAGE_TAKEN, Some(school_mask));
        #[allow(clippy::cast_possible_wrap)]
        let mut taken_flat = self.spell_bonus_with_coeffs(
            victim,
            spell,
            eff_index,
            0.0,
            taken_flat as f32,
            dot,
            false,
            applied_mods,
        ) * stack as i32 as f32;
        if taken_flat < 0.0 && -taken_flat > damage / 2.0 {
            taken_flat = -(damage / 2.0);
        }
        let out = (damage + taken_flat) * taken_total_mod;
        if out > 0.0 { out } else { 0.0 }
    }

    /// `GetMaxPositive/NegativeAuraModifier` over holders.
    fn max_aura_modifier(&self, guid: ObjectGuid, kind: u32, positive: bool) -> i32 {
        let holders = self.aura_holders(guid);
        let amounts = holders
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| e.aura_type == kind && (e.amount > 0) == positive && e.amount != 0)
            .map(|e| e.amount);
        if positive {
            amounts.max().unwrap_or(0)
        } else {
            amounts.min().unwrap_or(0)
        }
    }

    /// `SpellBaseHealingBonusDone` (`SpellCaster.cpp:1255`).
    fn spell_base_healing_bonus_done(&self, caster: ObjectGuid, school_mask: i32) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let mut benefit =
            self.total_aura_modifier(caster, aura_type::MOD_HEALING_DONE, Some(school_mask)) as f32;
        if self.players.contains_key(&caster) {
            for e in self
                .aura_holders(caster)
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == aura_type::MOD_SPELL_HEALING_OF_STAT_PERCENT)
            {
                #[allow(clippy::cast_precision_loss)]
                {
                    benefit += self.unit_stat(caster, 4) * e.amount as f32 / 100.0;
                }
            }
        }
        benefit
    }

    /// `SpellHealingBonusDone` (`SpellCaster.cpp:1184`).
    /// Totem owner delegation recorded (no totem casters).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_healing_bonus_done(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        heal: f32,
        dot: bool,
        stack: u32,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const CUSTOM_FIXED_DAMAGE: u32 = 0x10;
        const ATTR_PASSIVE: u32 = 0x40;
        let gated = (spell.dmg_class == dmg_class::NONE && spell.attributes & ATTR_PASSIVE != 0)
            || spell.custom & CUSTOM_FIXED_DAMAGE != 0
            || spell.attributes_ex3 & 0x2000_0000 != 0;
        if gated {
            return if heal < 0.0 { 0.0 } else { heal };
        }
        let done_total_mod =
            self.total_aura_multiplier(caster, aura_type::MOD_HEALING_DONE_PERCENT, None);
        let mut done_total = 0.0f32;
        let script_owner = self.pet_owner(caster).unwrap_or(caster);
        for holder in self.aura_holders(script_owner) {
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(e) = eff else { continue };
                if e.aura_type != aura_type::OVERRIDE_CLASS_SCRIPTS
                    || !self.override_affected(&holder, eff_idx, spell)
                {
                    continue;
                }
                match e.misc_value {
                    4415 => done_total += e.amount as f32 / 4.0,
                    3736 => done_total += e.amount as f32,
                    _ => {}
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let benefit = self.spell_base_healing_bonus_done(caster, school_mask);
        done_total = self.spell_bonus_with_coeffs(
            caster,
            spell,
            eff_index,
            done_total,
            benefit,
            dot,
            true,
            applied_mods.as_deref_mut(),
        );
        #[allow(clippy::cast_possible_wrap)]
        let mut out = (heal + done_total * stack as i32 as f32) * done_total_mod;
        out = self.apply_spell_mod(
            caster,
            spell,
            if dot { spellmod::DOT } else { spellmod::DAMAGE },
            out,
            true,
            applied_mods,
        );
        if out < 0.0 { 0.0 } else { out }
    }

    /// Immunity entries `(spell_id, type)` for one
    /// fork `m_spellImmune[op]` list, derived from
    /// holders plus permanent creature entries
    /// (spell 0) — the `ApplySpellImmune` list as a
    /// recompute. Ops: 0 damage, 1 dispel, 2 effect,
    /// 3 mechanic, 4 school, 5 state.
    fn immune_entries(&self, victim: ObjectGuid, op: u32) -> Vec<(u32, i32)> {
        let aura_kind = match op {
            0 => aura_type::DAMAGE_IMMUNITY,
            1 => aura_type::DISPEL_IMMUNITY,
            2 => aura_type::EFFECT_IMMUNITY,
            3 => aura_type::MECHANIC_IMMUNITY,
            4 => aura_type::SCHOOL_IMMUNITY,
            _ => aura_type::STATE_IMMUNITY,
        };
        let mut out: Vec<(u32, i32)> = self
            .aura_holders(victim)
            .iter()
            .flat_map(|h| {
                h.effects
                    .iter()
                    .flatten()
                    .filter(move |e| e.aura_type == aura_kind)
                    .map(move |e| (h.spell_id, e.misc_value))
            })
            .collect();
        // Permanent creature entries (`Creature::UpdateEntry`
        // tail): type-based, immunity flags, sessile.
        if let Some(c) = self.creatures.get(&victim) {
            let info = &c.info;
            let mut push = |entry_op: u32, ty: i32| {
                if entry_op == op {
                    out.push((0, ty));
                }
            };
            if info.creature_type == 9 {
                push(2, 10);
                push(2, 67);
                push(5, 8);
                push(5, 53);
            }
            if matches!(info.creature_type, 9 | 4) {
                push(1, 3);
                push(1, 4);
            }
            if info.immunity_flags & 0x02 != 0 {
                push(2, 114);
                push(5, 11);
            }
            if info.immunity_flags & 0x04 != 0 {
                push(5, 29);
                push(5, 137);
            }
            if info.immunity_flags & 0x08 != 0 {
                push(5, 65);
            }
            if info.immunity_flags & 0x10 != 0 {
                push(1, 3);
            }
            if info.immunity_flags & 0x20 != 0 {
                push(1, 4);
            }
            if info.immunity_flags & 0x40 != 0 {
                push(1, 2);
            }
            if info.static_flags1 & 0x100 != 0 {
                push(2, 69);
                push(2, 70);
                push(2, 98);
                push(5, 5);
                push(5, 7);
                push(5, 26);
            }
        }
        out
    }

    /// The shared list-entry positivity rule: permanent
    /// (spell 0) or missing immunity spells always
    /// block; otherwise block on positivity mismatch
    /// or EX immunity-to-hostile-and-friendly.
    fn immune_entry_blocks(&self, immunity_spell_id: u32, incoming_positive: bool) -> bool {
        let Some(imm) = self.stores.spells.get(immunity_spell_id) else {
            return true;
        };
        if immunity_spell_id == 0 {
            return true;
        }
        imm.is_positive_spell() != incoming_positive || imm.attributes_ex & 0x0001_0000 != 0
    }

    /// `Unit::IsImmuneToDamage` + creature school-mask
    /// override (`Unit.cpp:5566`, `Creature.cpp:2323`).
    pub(crate) fn is_immune_to_damage(
        &self,
        victim: ObjectGuid,
        school_mask: i32,
        spell: Option<&SpellEntry>,
    ) -> bool {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        if let Some(spell) = spell
            && spell.attributes & NO_IMMUNITIES != 0
        {
            return false;
        }
        if let Some(c) = self.creatures.get(&victim)
            && c.info.school_immune_mask & school_mask as u32 != 0
        {
            return true;
        }
        for (_id, ty) in self.immune_entries(victim, 0) {
            if ty & school_mask != 0 {
                return true;
            }
        }
        let skip_school = spell.is_some_and(|s| s.attributes_ex2 & 0x0400_0000 != 0);
        if !skip_school {
            let incoming_positive = spell.is_some_and(mangos_spells::SpellEntry::is_positive_spell);
            for (id, ty) in self.immune_entries(victim, 4) {
                if ty & school_mask != 0 && self.immune_entry_blocks(id, incoming_positive) {
                    return true;
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSpell` + creature mask override
    /// (`Unit.cpp:5602`, `Creature.cpp:2306`). The WSG
    /// flag-carrier arm is Tier 3 (battlegrounds).
    pub(crate) fn is_immune_to_spell(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        cast_on_self: bool,
    ) -> bool {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        const PASSIVE: u32 = 0x40;
        if spell.attributes & NO_IMMUNITIES != 0 {
            return false;
        }
        if !cast_on_self && let Some(c) = self.creatures.get(&victim) {
            if spell.mechanic != 0
                && c.info.mechanic_immune_mask & (1u32 << (spell.mechanic - 1)) != 0
            {
                return true;
            }
            if c.info.school_immune_mask & (1u32 << spell.school) != 0 && !spell.is_positive_spell()
            {
                return true;
            }
        }
        let positive = spell.is_positive_spell();
        if spell.attributes & PASSIVE == 0 {
            for (id, ty) in self.immune_entries(victim, 1) {
                #[allow(clippy::cast_possible_wrap)]
                if ty == spell.dispel as i32 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        if spell.attributes_ex & 0x8000 == 0 && spell.attributes_ex2 & 0x0400_0000 == 0 {
            for (id, ty) in self.immune_entries(victim, 4) {
                if ty & school_mask != 0 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        if spell.mechanic != 0 {
            #[allow(clippy::cast_possible_wrap)]
            let mechanic = spell.mechanic as i32;
            for (id, ty) in self.immune_entries(victim, 3) {
                if ty == mechanic && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
            let mask = 1i32 << (mechanic - 1);
            for holder in self.aura_holders(victim) {
                for e in holder.effects.iter().flatten() {
                    if e.aura_type == aura_type::MECHANIC_IMMUNITY_MASK
                        && e.misc_value & mask != 0
                        && self.immune_entry_blocks(holder.spell_id, positive)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSpellEffect` + creature
    /// mechanic-mask override (`Unit.cpp:5706`,
    /// `Creature.cpp:2334`).
    pub(crate) fn is_immune_to_spell_effect(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        cast_on_self: bool,
    ) -> bool {
        let eff = &spell.effects[eff_index];
        if !cast_on_self
            && eff.mechanic != 0
            && let Some(c) = self.creatures.get(&victim)
            && c.info.mechanic_immune_mask & (1u32 << (eff.mechanic - 1)) != 0
        {
            return true;
        }
        let positive = spell.is_positive_effect(eff_index);
        #[allow(clippy::cast_possible_wrap)]
        for (id, ty) in self.immune_entries(victim, 2) {
            if ty == eff.effect as i32 && self.immune_entry_blocks(id, positive) {
                return true;
            }
        }
        if eff.mechanic != 0 {
            #[allow(clippy::cast_possible_wrap)]
            let mechanic = eff.mechanic as i32;
            for (id, ty) in self.immune_entries(victim, 3) {
                if ty == mechanic && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
            let mask = 1i32 << (mechanic - 1);
            for holder in self.aura_holders(victim) {
                for e in holder.effects.iter().flatten() {
                    if e.aura_type == aura_type::MECHANIC_IMMUNITY_MASK
                        && e.misc_value & mask != 0
                        && self.immune_entry_blocks(holder.spell_id, positive)
                    {
                        return true;
                    }
                }
            }
        }
        if eff.apply_aura_name != 0 {
            #[allow(clippy::cast_possible_wrap)]
            for (id, ty) in self.immune_entries(victim, 5) {
                if ty == eff.apply_aura_name as i32 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSchool` (`Unit.cpp:5791`),
    /// per-effect positivity, self-immunity skipped.
    pub(crate) fn is_immune_to_school(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) -> bool {
        if spell.attributes_ex & 0x8000 != 0 || spell.attributes_ex2 & 0x0400_0000 != 0 {
            return false;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let positive = spell.is_positive_effect(eff_index);
        for (id, ty) in self.immune_entries(victim, 4) {
            if id == spell.id {
                continue;
            }
            if ty & school_mask != 0 && self.immune_entry_blocks(id, positive) {
                return true;
            }
        }
        false
    }

    /// Mechanic mask of a spell (`HasMechanicMask`):
    /// spell mechanic bit + effect mechanic bits.
    pub(crate) fn mechanic_mask(spell: &SpellEntry) -> i32 {
        let mut mask = 0i32;
        if spell.mechanic != 0 {
            mask |= 1i32 << (spell.mechanic - 1);
        }
        for e in &spell.effects {
            if e.mechanic != 0 {
                mask |= 1i32 << (e.mechanic - 1);
            }
        }
        mask
    }

    /// Immunity-aura apply side effects (fork handlers
    /// at `SpellAuras.cpp:3933-4095`): purges gated by
    /// `EX_IMMUNITY_PURGES_EFFECT`, plus the school
    /// immunity flag-aura strip. WSG flag-carrier arm
    /// of effect immunity is Tier 3.
    pub(crate) fn apply_immunity_purges(&mut self, victim: ObjectGuid, spell: &SpellEntry) {
        const PURGES: u32 = 0x8000;
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        let purges = spell.attributes_ex & PURGES != 0;
        for e in &spell.effects {
            match e.apply_aura_name {
                aura_type::MECHANIC_IMMUNITY if purges && e.misc_value > 0 => {
                    self.remove_auras_at_mechanic_immunity(
                        victim,
                        1i32 << (e.misc_value - 1),
                        spell.id,
                        false,
                    );
                }
                aura_type::MECHANIC_IMMUNITY_MASK if purges => {
                    self.remove_auras_at_mechanic_immunity(victim, e.misc_value, spell.id, false);
                }
                aura_type::STATE_IMMUNITY if purges => {
                    let ids: Vec<u32> = self
                        .aura_holders(victim)
                        .iter()
                        .filter(|h| h.spell_id != spell.id)
                        .filter(|h| {
                            h.effects
                                .iter()
                                .flatten()
                                .any(|x| x.aura_type == e.misc_value as u32)
                        })
                        .map(|h| h.spell_id)
                        .collect();
                    for id in ids {
                        self.remove_auras_due_to_spell(victim, id);
                    }
                }
                aura_type::SCHOOL_IMMUNITY => {
                    if spell.is_positive_spell()
                        && self.players.contains_key(&victim)
                        && self.pet_owner(victim).is_none()
                    {
                        self.remove_auras_with_interrupt_flags(
                            victim,
                            aura_interrupt_flag::INVULNERABILITY_BUFF_CANCELS,
                            0,
                        );
                    }
                    if purges && spell.is_positive_spell() {
                        let ids: Vec<u32> = self
                            .aura_holders(victim)
                            .iter()
                            .filter(|h| !h.positive && h.spell_id != spell.id)
                            .filter(|h| {
                                self.stores.spells.get(h.spell_id).is_some_and(|s| {
                                    (1i32 << s.school) & e.misc_value != 0
                                        && s.attributes & NO_IMMUNITIES == 0
                                })
                            })
                            .map(|h| h.spell_id)
                            .collect();
                        for id in ids {
                            self.remove_auras_due_to_spell(victim, id);
                        }
                    }
                }
                aura_type::DISPEL_IMMUNITY if purges => {
                    let ids: Vec<u32> = self
                        .aura_holders(victim)
                        .iter()
                        .filter(|h| {
                            self.stores.spells.get(h.spell_id).is_some_and(|s| {
                                #[allow(clippy::cast_possible_wrap)]
                                {
                                    s.dispel as i32 == e.misc_value
                                }
                            })
                        })
                        .map(|h| h.spell_id)
                        .collect();
                    for id in ids {
                        self.remove_auras_due_to_spell(victim, id);
                    }
                }
                _ => {}
            }
        }
    }

    /// `Unit::RemoveAurasAtMechanicImmunity`.
    pub(crate) fn remove_auras_at_mechanic_immunity(
        &mut self,
        victim: ObjectGuid,
        mech_mask: i32,
        except_spell_id: u32,
        non_positive: bool,
    ) {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        let ids: Vec<u32> = self
            .aura_holders(victim)
            .iter()
            .filter(|h| h.spell_id != except_spell_id)
            .filter(|h| !(non_positive && h.positive))
            .filter(|h| {
                self.stores.spells.get(h.spell_id).is_some_and(|s| {
                    s.attributes & NO_IMMUNITIES == 0 && Self::mechanic_mask(s) & mech_mask != 0
                })
            })
            .map(|h| h.spell_id)
            .collect();
        for id in ids {
            self.remove_auras_due_to_spell(victim, id);
        }
    }

    /// Holder-derived `UNIT_FLAG_IMMUNE` +
    /// `UNIT_STATE_ISOLATED` (school-immunity handler
    /// flag arms as a recompute).
    pub(crate) fn refresh_immune_state(&mut self, guid: ObjectGuid) {
        const PURGES: u32 = 0x8000;
        const UNIT_FLAG_IMMUNE: u32 = 0x8000_0000;
        let mut immune_flag = false;
        let mut isolated = false;
        for holder in self.aura_holders(guid) {
            let Some(spell) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if spell.attributes_ex & PURGES != 0
                && spell.is_positive_spell()
                && holder
                    .effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == aura_type::SCHOOL_IMMUNITY)
            {
                immune_flag = true;
            }
            if spell.mechanic == 5 {
                isolated = true;
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let flags = unit.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            let new_flags = if immune_flag {
                flags | UNIT_FLAG_IMMUNE
            } else {
                flags & !UNIT_FLAG_IMMUNE
            };
            if new_flags != flags {
                unit.values().set_u32(offsets::UNIT_FIELD_FLAGS, new_flags);
            }
            if isolated {
                unit.add_unit_state(mangos_entities::unit::UNIT_STATE_ISOLATED);
            } else {
                unit.clear_unit_state(mangos_entities::unit::UNIT_STATE_ISOLATED);
            }
        }
    }

    /// Holder-derived attack-speed percent
    /// (`ApplyAttackTimePercentMod` as a recompute):
    /// rewrites the three attack-time fields by
    /// old→new pct ratio and stores the new pcts.
    /// Haste factor per amount: positive divides
    /// (100/(100+v)), negative multiplies
    /// ((100-v)/100) at build 5875. `SPELLMOD_HASTE`
    /// applies per holder amount (fork applies it
    /// once at aura apply).
    #[allow(clippy::float_cmp)] // exact pct change gate, as in the fork field rewrite
    pub(crate) fn refresh_attack_speed(&mut self, guid: ObjectGuid) {
        #[allow(clippy::cast_precision_loss)]
        let factor = |amount: i32| -> f32 {
            let v = amount as f32;
            if v > 0.0 {
                100.0 / (100.0 + v)
            } else if v < 0.0 {
                (100.0 - v) / 100.0
            } else {
                1.0
            }
        };
        let mut pct = [1.0f32; 3];
        for holder in self.aura_holders(guid) {
            let Some(spell) = self.stores.spells.get(holder.spell_id).cloned() else {
                continue;
            };
            for e in holder.effects.iter().flatten() {
                let slots: &[usize] = match e.aura_type {
                    aura_type::MOD_ATTACKSPEED => &[0, 1, 2],
                    aura_type::MOD_MELEE_HASTE => &[0, 1],
                    aura_type::MOD_RANGED_HASTE | aura_type::MOD_RANGED_AMMO_HASTE => &[2],
                    _ => continue,
                };
                #[allow(clippy::cast_precision_loss)]
                let amount = self.apply_spell_mod(
                    holder.caster,
                    &spell,
                    spellmod::HASTE,
                    e.amount as f32,
                    false,
                    None,
                );
                #[allow(clippy::cast_possible_truncation)]
                let f = factor(amount as i32);
                for &slot in slots {
                    pct[slot] *= f;
                }
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let old = unit.mod_attack_speed_pct;
            if old == pct {
                return;
            }
            for slot in 0..3 {
                if old[slot] == 0.0 || old[slot] == pct[slot] {
                    continue;
                }
                let field = match slot {
                    0 => offsets::UNIT_FIELD_BASEATTACKTIME,
                    1 => offsets::UNIT_FIELD_BASEATTACKTIME + 1,
                    _ => offsets::UNIT_FIELD_RANGEDATTACKTIME,
                };
                let cur = unit.values().get_f32(field);
                if cur != 0.0 {
                    unit.values().set_f32(field, cur / old[slot] * pct[slot]);
                }
            }
            unit.mod_attack_speed_pct = pct;
        }
    }

    /// Holder-derived `PLAYER_SKILL_BONUS` packed
    /// fields (`ModifySkillBonus` as a recompute):
    /// temporary `MOD_SKILL` low half, permanent
    /// `MOD_SKILL_TALENT` high half, keyed by the
    /// skill id in each slot.
    pub(crate) fn refresh_skill_bonuses(&mut self, guid: ObjectGuid) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut temp: std::collections::HashMap<u32, i32> = std::collections::HashMap::new();
        let mut perm: std::collections::HashMap<u32, i32> = std::collections::HashMap::new();
        for holder in self.aura_holders(guid) {
            for e in holder.effects.iter().flatten() {
                let map = match e.aura_type {
                    aura_type::MOD_SKILL => &mut temp,
                    aura_type::MOD_SKILL_TALENT => &mut perm,
                    _ => continue,
                };
                #[allow(clippy::cast_sign_loss)]
                {
                    *map.entry(e.misc_value as u32).or_insert(0) += e.amount;
                }
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            for slot in 0..127u16 {
                let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
                let skill = unit.values().get_u32(base);
                if skill == 0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let packed = |v: i32| (v as i16) as u16 as u32;
                let bonus = packed(temp.get(&skill).copied().unwrap_or(0))
                    | (packed(perm.get(&skill).copied().unwrap_or(0)) << 16);
                if unit.values().get_u32(base + 2) != bonus {
                    unit.values().set_u32(base + 2, bonus);
                }
            }
        }
    }

    /// `SendSpellOrDamageImmune` (`SpellCaster.cpp:673`).
    fn send_spell_or_damage_immune(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell_id: u32,
    ) {
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(spell_id);
        payload.write_u8(0);
        let packet = WorldPacket::with_body(op::SMSG_SPELLORDAMAGE_IMMUNE, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// `SpellHealingBonusTaken` (`Unit.cpp:5491`).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_healing_bonus_taken(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        heal: f32,
        dot: bool,
        stack: u32,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        let mut taken_total_mod = 1.0f32;
        let minval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, false);
        if minval != 0 {
            taken_total_mod *= (100.0 + minval as f32) / 100.0;
        }
        let maxval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, true);
        if maxval != 0 {
            taken_total_mod *= (100.0 + maxval as f32) / 100.0;
        }
        if spell.dmg_class == dmg_class::NONE {
            let out = heal * taken_total_mod;
            return if out < 0.0 { 0.0 } else { out };
        }
        // Healing Way (29203) dummy for Healing Wave
        // (shaman family 11, CM0 bit 6).
        if spell.spell_family_name == 11 && spell.spell_family_flags & 0x40 != 0 {
            for e in self
                .aura_holders(victim)
                .iter()
                .filter(|h| h.spell_id == 29203)
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == aura_type::DUMMY)
            {
                taken_total_mod *= (e.amount as f32 + 100.0) / 100.0;
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let mut taken_flat =
            self.total_aura_modifier(victim, aura_type::MOD_HEALING, Some(school_mask)) as f32;
        // Blessing of Light dummies (paladin family 10;
        // flash bit 13, holy light bit 31, blessings
        // bit 28, visual 300).
        if spell.spell_family_name == 10 && spell.spell_family_flags & (0x2000 | 0x8000_0000) != 0 {
            for holder in self.aura_holders(victim) {
                let Some(source) = self.stores.spells.get(holder.spell_id) else {
                    continue;
                };
                if source.spell_family_flags & 0x1000_0000 == 0 || source.spell_visual != 300 {
                    continue;
                }
                for (eff_idx, eff) in holder.effects.iter().enumerate() {
                    let Some(e) = eff else { continue };
                    if e.aura_type != aura_type::DUMMY {
                        continue;
                    }
                    let holy_light = spell.spell_family_flags & 0x8000_0000 != 0 && eff_idx == 0;
                    let flash = spell.spell_family_flags & 0x2000 != 0 && eff_idx == 1;
                    if holy_light || flash {
                        taken_flat += e.amount as f32;
                    }
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let mut taken_flat = self.spell_bonus_with_coeffs(
            victim,
            spell,
            eff_index,
            0.0,
            taken_flat,
            dot,
            false,
            applied_mods,
        ) * stack as i32 as f32;
        if taken_flat < 0.0 && -taken_flat > heal / 2.0 {
            taken_flat = -(heal / 2.0);
        }
        let out = (heal + taken_flat) * taken_total_mod;
        if out < 0.0 { 0.0 } else { out }
    }

    /// `SpellCaster::CalculateDamage` tail +
    /// `SpellNonMeleeDamageLog` + health application.
    /// `aura_holder` is Some for periodic ticks
    /// (PERIODICAURALOG instead of NONMELEE log).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::too_many_arguments,
        clippy::needless_option_as_deref // reborrows of the applied-mods list
    )]
    #[allow(clippy::too_many_lines)] // one CalculateDamage port
    pub(crate) fn school_damage(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        base_damage: i32,
        eff_index: u32,
        aura_holder: Option<u32>,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        proc_ctx: Option<&ProcRaiseCtx>,
    ) {
        if base_damage <= 0 {
            return;
        }
        // Fork `CalculateDamage` order: bonus Done →
        // Taken → crit → armor → ditheru. Periodic
        // amounts carry Done from the apply-time
        // snapshot, so ticks take Taken(DOT) only.
        // Melee/ranged-class spells keep the pre-slice
        // math (`MeleeDamageBonus` audit is separate).
        let dot = aura_holder.is_some();
        #[allow(clippy::cast_precision_loss)]
        let mut damage_f = base_damage as f32;
        if spell_meleedamage(spell.dmg_class) {
            damage_f = self.apply_spell_mod(
                caster,
                spell,
                if dot { spellmod::DOT } else { spellmod::DAMAGE },
                damage_f,
                true,
                applied_mods.as_deref_mut(),
            );
            if spell.attributes_ex3 & 0x2000_0000 == 0 {
                #[allow(clippy::cast_possible_wrap)]
                let school_mask = 1i32 << spell.school;
                let pct = self.total_aura_multiplier(
                    caster,
                    aura_type::MOD_DAMAGE_PERCENT_DONE,
                    Some(school_mask),
                ) * self.total_aura_multiplier(
                    victim,
                    aura_type::MOD_DAMAGE_PERCENT_TAKEN,
                    Some(school_mask),
                );
                damage_f *= pct;
            }
        } else if dot {
            let stack = aura_holder
                .and_then(|spell_id| {
                    self.aura_holders(victim)
                        .into_iter()
                        .find(|h| h.spell_id == spell_id && h.caster == caster)
                        .map(|h| h.stack)
                })
                .unwrap_or(1);
            damage_f = self.spell_damage_bonus_taken(
                victim,
                spell,
                eff_index as usize,
                damage_f,
                true,
                stack,
                applied_mods.as_deref_mut(),
            );
        } else {
            damage_f = self.spell_damage_bonus_done(
                caster,
                victim,
                spell,
                eff_index as usize,
                damage_f,
                false,
                1,
                applied_mods.as_deref_mut(),
            );
            damage_f = self.spell_damage_bonus_taken(
                victim,
                spell,
                eff_index as usize,
                damage_f,
                false,
                1,
                applied_mods.as_deref_mut(),
            );
        }

        // Crit: direct magic damage only (periodic ticks
        // never crit in 1.12.1); skipped entirely under
        // EX3 ignore-caster-modifiers (fork gate). Bonus
        // is +50% for magic (`SpellCriticalDamageBonus`
        // default arm, spellmod on the bonus).
        let mut crit = false;
        if !dot
            && spell.dmg_class == dmg_class::MAGIC
            && spell.attributes_ex3 & 0x2000_0000 == 0
            && damage_f > 0.0
        {
            let crit_chance = self.spell_crit_percent(caster, spell, applied_mods.as_deref_mut());
            let roll = self.spell_rng().frand01() * 100.0;
            if roll < crit_chance {
                crit = true;
                let bonus = self.apply_spell_mod(
                    caster,
                    spell,
                    spellmod::CRIT_DAMAGE_BONUS,
                    damage_f / 2.0,
                    true,
                    applied_mods.as_deref_mut(),
                );
                damage_f += bonus;
            }
        }

        // Armor after bonus+crit (fork mitigation
        // stage), NORMAL school only, then ditheru.
        if damage_f > 0.0 && spell.school == school::NORMAL {
            let victim_armor = self.unit_resistance(victim, school::NORMAL);
            damage_f = mangos_spells::armor_reduced_damage(
                damage_f,
                victim_armor,
                self.unit_level(caster),
            );
        }
        #[allow(clippy::cast_possible_truncation)]
        let mut damage = if damage_f > 0.0 {
            mangos_spells::ditheru(damage_f, self.spell_rng()) as i32
        } else {
            0
        };

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
            self.deal_damage_school(
                caster,
                victim,
                final_damage,
                aura_holder.is_some(),
                spell.school,
            );
        }
        // Proc raise for the landed hit (fork raises in
        // `DealDamage` with the spell's proc info).
        if let Some(ctx) = proc_ctx {
            let ex = if final_damage == 0 {
                proc_ex::MISS
            } else if crit {
                proc_ex::CRITICAL_HIT
            } else {
                proc_ex::NORMAL_HIT
            };
            self.raise_damage_procs(
                caster,
                victim,
                Some(spell.id),
                ctx,
                ex,
                final_damage,
                final_damage + final_resisted,
            );
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
    /// (`Unit::DealDamage` subset). `is_dot` is the
    /// fork `DamageEffectType::DOT` arm (periodic ticks
    /// never push back or cancel casts).
    pub(crate) fn deal_damage(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        damage: u32,
        is_dot: bool,
    ) {
        self.deal_damage_school(attacker, victim, damage, is_dot, 0);
    }

    /// `deal_damage` with the damage school (absorb
    /// mask matching); melee/script paths use NORMAL.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn deal_damage_school(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        mut damage: u32,
        is_dot: bool,
        school_id: u32,
    ) {
        if damage == 0 {
            return;
        }
        // Damage-immunity backstop (melee/script
        // funnels; spell paths resolve immunity before
        // their damage logs).
        #[allow(clippy::cast_possible_wrap)]
        if self.is_immune_to_damage(victim, 1i32 << school_id, None) {
            return;
        }
        if victim.is_creature() {
            if attacker != victim {
                self.creature_script_attacked_by(victim, attacker);
            }
            // `CreatureAI::DamageTaken` may rewrite the amount.
            let mut scripted = damage;
            self.creature_script_damage_taken(victim, attacker, &mut scripted);
            damage = scripted;
            if damage == 0 {
                return;
            }
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
        // `SPELL_AURA_SCHOOL_ABSORB` pools (fork
        // absorbs inside `DealDamage` before health;
        // a holder drops when its pool is spent).
        let absorbed_full;
        {
            #[allow(clippy::cast_possible_wrap)]
            let mask = 1i32 << school_id;
            let mut spent_holders: Vec<u32> = Vec::new();
            if let Some(u) = self.unit_base_mut(victim) {
                for holder in &mut u.auras {
                    if damage == 0 {
                        break;
                    }
                    for eff in holder.effects.iter_mut().flatten() {
                        if eff.aura_type == aura_type::SCHOOL_ABSORB
                            && eff.misc_value & mask != 0
                            && eff.amount > 0
                        {
                            #[allow(clippy::cast_possible_truncation)]
                            let take = damage.min(eff.amount as u32);
                            #[allow(clippy::cast_possible_wrap)]
                            {
                                eff.amount -= take as i32;
                            }
                            damage -= take;
                            if eff.amount == 0 {
                                spent_holders.push(holder.spell_id);
                            }
                            if damage == 0 {
                                break;
                            }
                        }
                    }
                }
            }
            absorbed_full = damage == 0;
            for spell_id in spent_holders {
                self.remove_auras_due_to_spell(victim, spell_id);
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
        if absorbed_full {
            // Fully absorbed: combat + pushback above
            // still apply in the fork; no kill/threat.
            if !is_dot {
                self.delay_spell_at_damage(attacker, victim);
            }
            return;
        }
        // Damage taken while casting (`DealDamage`
        // player arm): DAMAGE_CANCELS interrupts, else
        // `Spell::Delayed` pushback.
        if !is_dot {
            self.delay_spell_at_damage(attacker, victim);
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
    #[allow(clippy::too_many_lines)]
    pub(crate) fn kill_unit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let is_creature = victim.is_creature();
        if let Some(killer) = killer
            && self.unit_is_alive(victim)
            && killer != victim
        {
            self.raise_kill_proc(killer, victim);
        }
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
        // Kill XP (`Unit::Kill` rewards precede the
        // AI death hooks in the fork as well).
        if is_creature {
            self.reward_kill_xp(victim, killer);
            self.quest_kill_credit(victim, killer);
        }
        // EventAI death/kill hooks run while the AI
        // state still exists (`JustDied`/`KilledUnit`).
        self.eventai_on_death(victim, killer);
        if victim.is_player() {
            self.instance_on_player_death(victim);
        }
        // BG kill scoring (`BattleGround::HandleKillPlayer` /
        // `HandleKillUnit`, Tier 3 Slice 3; no-op off BG maps).
        if self.bg.is_some() {
            if victim.is_player() {
                self.bg_handle_kill_player(victim, killer);
            } else if victim.is_creature() {
                self.bg_handle_kill_unit(victim, killer);
            }
        }
        // `CreatureAI::KilledUnit` fires for any victim type
        // (fork `Unit::Kill`); bodies gate on player victims
        // themselves.
        if let Some(k) = killer
            && k != victim
            && k.is_creature()
        {
            self.creature_script_killed_unit(k, victim);
        }
        if is_creature {
            self.instance_on_creature_death(victim);
            self.go_ai_summoned_creature_just_died(victim);
            self.creature_script_just_died(victim, killer);
            self.creature_script_summoned_creature_just_died(victim);
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
        // Death ends a channel: strip the target-side
        // holders and clear the channel fields (no
        // channel packets to the dead; own holders go
        // with `remove_all_auras` below).
        if let Some(cast) = self.casts.remove(&victim)
            && cast.channeling
        {
            if let CastTarget::Unit(target) = cast.target
                && target != victim
            {
                self.remove_auras_by_caster_spell(target, cast.spell_id, victim);
            }
            if let Some(u) = self.unit_base_mut(victim) {
                let values = u.values();
                values.set_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT, 0);
                values.set_u32(offsets::UNIT_CHANNEL_SPELL, 0);
            }
        }
        self.attack_stop(victim);
        self.ai.remove(&victim);
        self.remove_all_auras(victim);
        self.stop_moving(victim);
        if let Some(mut master) = self.motion.remove(&victim) {
            master.clear(self, victim);
            self.motion.entry(victim).or_insert(master);
        }
        if is_creature {
            if self.is_pet(victim) {
                // Pet corpses persist for revive until
                // unsummoned (no DB respawn, no decay).
            } else if self.summons.contains_key(&victim) {
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
            self.creature_script_just_respawned(guid);
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
        // `SPELL_ATTR_PASSIVE`, channeled spells.
        const ATTR_PASSIVE: u32 = 0x0000_0040;
        let mut rows = Vec::new();
        for holder in holders {
            let Some(spell) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if spell.attributes & ATTR_PASSIVE != 0 || spell.is_channeled() {
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
    /// time to epoch seconds. Item-created rows keep
    /// their `item_id` and effective category.
    #[must_use]
    pub fn player_cooldown_rows(&self, guid: ObjectGuid, now_unix: i64) -> Vec<CooldownSnapshot> {
        let to_epoch = |expiry_ms: u32| -> i64 {
            now_unix + i64::from(expiry_ms.saturating_sub(self.now_ms)) / 1000
        };
        let mut rows = Vec::new();
        for ((caster, key), entry) in &self.cooldowns {
            // Permanent holds are never saved
            // (`_SaveSpellCooldowns` skips
            // `IsPermanent()`).
            if *caster != guid
                || entry.permanent
                || entry.expiry_ms <= self.now_ms
                || *key & 0x8000_0000 != 0
            {
                continue;
            }
            let category = if entry.category != 0 {
                entry.category
            } else {
                self.stores.spells.get(*key).map_or(0, |s| s.category)
            };
            let category_expire_time = if category != 0 {
                self.cooldowns
                    .get(&(guid, category | 0x8000_0000))
                    .filter(|e| !e.permanent && e.expiry_ms > self.now_ms)
                    .map_or(0, |e| to_epoch(e.expiry_ms))
            } else {
                0
            };
            rows.push(CooldownSnapshot {
                spell: *key,
                spell_expire_time: to_epoch(entry.expiry_ms),
                category,
                category_expire_time,
                item_id: entry.item_id,
            });
        }
        rows.sort_by_key(|r| r.spell);
        rows
    }

    pub(crate) fn send_aura_duration(&mut self, target: ObjectGuid, slot: u8, duration_ms: i32) {
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
            match eff.aura_type {
                aura_type::MOD_ATTACK_POWER => {
                    Self::apply_attack_power_mod(unit, holder.positive, eff.amount, apply);
                }
                aura_type::MOD_STAT
                | aura_type::MOD_PERCENT_STAT
                | aura_type::MOD_TOTAL_STAT_PERCENTAGE => {
                    Self::apply_stat_aura_mod(unit, eff, apply);
                }
                // `HandleModScale`: multiplicative
                // object-scale change, inverted on
                // removal (fork apply/unapply pair).
                aura_type::MOD_SCALE => {
                    #[allow(clippy::cast_precision_loss)]
                    let factor = (100.0 + eff.amount as f32) / 100.0;
                    if factor > 0.0 {
                        let cur = unit.world.object.object_scale();
                        let next = if apply { cur * factor } else { cur / factor };
                        unit.world.object.set_object_scale(next);
                    }
                }
                // Other aura types are state-only in the
                // slice (periodic handled by the tick loop;
                // speed/root/stun recorded as pending).
                _ => {}
            }
        }
    }

    /// `ApplyModSignedFloatValue` on one pos/neg display
    /// field.
    fn apply_stat_buff_mod(unit: &mut UnitBase, stat: usize, amount: f32, apply: bool) {
        #[allow(clippy::cast_possible_truncation)]
        let idx = stat as u16;
        let field = if amount > 0.0 {
            offsets::PLAYER_FIELD_POSSTAT0 + idx
        } else {
            offsets::PLAYER_FIELD_NEGSTAT0 + idx
        };
        let values = unit.values();
        let cur = values.get_f32(field);
        values.set_f32(field, if apply { cur + amount } else { cur - amount });
    }

    /// `ApplyStatPercentBuffMod`: percent-scale both
    /// display halves.
    fn apply_stat_percent_buff_mod(unit: &mut UnitBase, stat: usize, amount: f32, apply: bool) {
        #[allow(clippy::cast_possible_truncation)]
        let idx = stat as u16;
        let values = unit.values();
        for field in [
            offsets::PLAYER_FIELD_POSSTAT0 + idx,
            offsets::PLAYER_FIELD_NEGSTAT0 + idx,
        ] {
            let mut cur = values.get_f32(field);
            apply_percent_mod_float_var(&mut cur, amount, apply);
            values.set_f32(field, cur);
        }
    }

    /// Fork miscvalue stat selection (negative = all).
    fn stat_matches(misc_value: i32, stat: usize) -> bool {
        #[allow(clippy::cast_possible_wrap)]
        {
            misc_value < 0 || misc_value == stat as i32
        }
    }

    /// `HandleAuraModStat` / `HandleModPercentStat` /
    /// `HandleModTotalPercentStat` register writes (the
    /// `Update*` dispatch is `refresh_derived_stats` in
    /// the add/remove wrappers). The Scorpid Sting
    /// override-class-script branch of `HandleAuraModStat`
    /// is deferred with spell mods.
    fn apply_stat_aura_mod(
        unit: &mut UnitBase,
        eff: &mangos_entities::aura::AuraEffectState,
        apply: bool,
    ) {
        let is_player = unit.guid().is_player();
        #[allow(clippy::cast_precision_loss)]
        let amount = eff.amount as f32;
        match eff.aura_type {
            aura_type::MOD_STAT => {
                if !(-2..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::TotalValue, amount, apply);
                        if is_player {
                            Self::apply_stat_buff_mod(unit, stat, amount, apply);
                        }
                    }
                }
            }
            aura_type::MOD_PERCENT_STAT => {
                // Only players have base stats.
                if !is_player || !(-1..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::BasePct, amount, apply);
                    }
                }
            }
            aura_type::MOD_TOTAL_STAT_PERCENTAGE => {
                if !(-1..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::TotalPct, amount, apply);
                        if is_player {
                            Self::apply_stat_percent_buff_mod(unit, stat, amount, apply);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn holder_has_stat_mod(holder: &AuraHolder) -> bool {
        holder.effects.iter().flatten().any(|e| {
            matches!(
                e.aura_type,
                aura_type::MOD_STAT
                    | aura_type::MOD_PERCENT_STAT
                    | aura_type::MOD_TOTAL_STAT_PERCENTAGE
            )
        })
    }

    /// `UNIT_MOD_HEALTH` / `UNIT_MOD_POWER_*`
    /// TOTAL-group presence (`MOD_INCREASE_HEALTH`/
    /// `..._PERCENT`, `MOD_INCREASE_ENERGY`/
    /// `..._PERCENT`): drives the derived-stat
    /// refresh and the fork handlers' current-value
    /// adjustments.
    fn holder_has_health_energy_mod(holder: &AuraHolder) -> bool {
        holder.effects.iter().flatten().any(|e| {
            matches!(
                e.aura_type,
                aura_type::MOD_INCREASE_HEALTH
                    | aura_type::MOD_INCREASE_HEALTH_PERCENT
                    | aura_type::MOD_INCREASE_ENERGY
                    | aura_type::MOD_INCREASE_ENERGY_PERCENT
            )
        })
    }

    /// Post-refresh current health/power adjustments
    /// of `HandleAuraModIncreaseHealth[Percent]` and
    /// `HandleAuraModIncreaseEnergy[Percent]` for a
    /// just-applied holder (`SpellAuras.cpp:4723+`).
    /// `pre` is (health, max health, powers, max
    /// powers) captured before the holder landed.
    fn apply_health_energy_adjustments(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        pre: Option<(u32, u32, [u32; 5], [u32; 5])>,
    ) {
        let Some((old_hp, old_max, old_powers, old_max_powers)) = pre else {
            return;
        };
        let flat_health: i32 = holder
            .effects
            .iter()
            .flatten()
            .filter(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH)
            .map(|e| e.amount)
            .sum();
        let has_health_pct = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH_PERCENT);
        if let Some(unit) = self.unit_base_mut(victim).filter(|u| u.is_alive()) {
            if holder.spell_id == 12976 {
                // Last Stand: current health follows
                // the flat amount.
                let cur = unit.health();
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                let next = (i64::from(cur) + i64::from(flat_health)).clamp(0, i64::from(u32::MAX));
                unit.set_health(next as u32);
            } else if matches!(holder.spell_id, 1178 | 9635) {
                // Bear form: ceil percent preservation.
                if old_max > 0 {
                    #[allow(
                        clippy::cast_precision_loss,
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss
                    )]
                    let next =
                        (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)).ceil() as u32;
                    unit.set_health(next);
                }
            } else if has_health_pct && old_max > 0 {
                // SetHealthPercent (truncating).
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)) as u32;
                unit.set_health(next);
            }
            for e in holder.effects.iter().flatten() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let power = e.misc_value as usize;
                if power >= 5 {
                    continue;
                }
                match e.aura_type {
                    aura_type::MOD_INCREASE_ENERGY => {
                        let cur = i64::from(unit.power(power)) + i64::from(e.amount);
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let next = cur.clamp(0, i64::from(unit.max_power(power))) as u32;
                        unit.set_power(power, next);
                    }
                    aura_type::MOD_INCREASE_ENERGY_PERCENT => {
                        // SetPower by preserved percent.
                        let old_max = old_max_powers[power];
                        if old_max > 0 {
                            #[allow(
                                clippy::cast_precision_loss,
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss
                            )]
                            let next = (unit.max_power(power) as f32
                                * (old_powers[power] as f32 / old_max as f32))
                                as u32;
                            unit.set_power(power, next);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Post-refresh current health/power adjustments
    /// for a just-removed health/energy holder (the
    /// remove arms of the same fork handlers; the
    /// Last Stand pre-adjustment already ran in
    /// `remove_aura_at`).
    fn remove_health_energy_adjustments(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        pre: (u32, u32, [u32; 5], [u32; 5]),
    ) {
        let (old_hp, old_max, old_powers, old_max_powers) = pre;
        let Some(unit) = self.unit_base_mut(victim).filter(|u| u.is_alive()) else {
            return;
        };
        let has_health_pct = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH_PERCENT);
        if old_max > 0 && matches!(holder.spell_id, 1178 | 9635) {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)).ceil() as u32;
            unit.set_health(next);
        } else if old_max > 0 && has_health_pct {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)) as u32;
            unit.set_health(next);
        }
        for e in holder.effects.iter().flatten() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let power = e.misc_value as usize;
            if power >= 5 {
                continue;
            }
            match e.aura_type {
                aura_type::MOD_INCREASE_ENERGY => {
                    let cur = i64::from(unit.power(power)) - i64::from(e.amount);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let next = cur.clamp(0, i64::from(unit.max_power(power))) as u32;
                    unit.set_power(power, next);
                }
                aura_type::MOD_INCREASE_ENERGY_PERCENT => {
                    let old_max_power = old_max_powers[power];
                    if old_max_power > 0 {
                        #[allow(
                            clippy::cast_precision_loss,
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss
                        )]
                        let next = (unit.max_power(power) as f32
                            * (old_powers[power] as f32 / old_max_power as f32))
                            as u32;
                        unit.set_power(power, next);
                    }
                }
                _ => {}
            }
        }
    }

    /// The HP-ratio preservation of
    /// `HandleModTotalPercentStat`: stamina-misc percent
    /// mods on `Attributes & 0x10` spells.
    fn hp_ratio_preserve(&self, holder: &AuraHolder) -> bool {
        let has = holder.effects.iter().flatten().any(|e| {
            e.aura_type == aura_type::MOD_TOTAL_STAT_PERCENTAGE && e.misc_value == STAT_STAMINA_I32
        });
        has && self
            .stores
            .spells
            .get(holder.spell_id)
            .is_some_and(|s| s.attributes & 0x10 != 0)
    }

    /// `HandleStatModifier`'s derived-stat dispatch after
    /// a stat-aura register change.
    fn refresh_derived_stats(&mut self, guid: ObjectGuid) {
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.refresh_derived_stats();
            return;
        }
        if let Some(creature) = self.creatures.get_mut(&guid) {
            creature.refresh_derived_stats();
        }
        self.refresh_weapon_damage_fields(guid);
    }

    /// Add a holder: slot allocation (positive slots
    /// 0..32, negative 32..48 — `GetAuraSlot` ranges),
    /// field writes, modifier application.
    fn add_aura_holder(&mut self, victim: ObjectGuid, mut holder: AuraHolder) {
        holder.apply_time_ms = self.now_ms;
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
        let holder_spell_id = holder.spell_id;
        let holder_caster_guid = holder.caster;
        let duration_packet = (slot, holder.duration_ms);
        let has_ap = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
        let has_stat = Self::holder_has_stat_mod(&holder);
        let has_health_energy = Self::holder_has_health_energy_mod(&holder);
        let hp_before = if has_stat && self.hp_ratio_preserve(&holder) {
            self.unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| (u.health(), u.max_health()))
        } else {
            None
        };
        let vitals_before = if has_health_energy {
            self.unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| {
                    (
                        u.health(),
                        u.max_health(),
                        core::array::from_fn(|p| u.power(p)),
                        core::array::from_fn(|p| u.max_power(p)),
                    )
                })
        } else {
            None
        };
        // AuraScript::OnBeforeApply (holder level,
        // per present effect) runs before the apply
        // handlers, fork ApplyModifier order.
        self.aura_script_on_before_apply(victim, &mut holder, true);
        let landed = if let Some(unit) = self.unit_base_mut(victim) {
            Self::aura_effect_apply(unit, &holder, true);
            Self::write_aura_slot(unit, &holder, false);
            unit.auras.push(holder);
            true
        } else {
            false
        };
        if landed {
            self.aura_script_on_after_apply(victim, holder_spell_id, holder_caster_guid, true);
            self.aura_dummy_for_spell(victim, holder_spell_id, true);
        }
        if has_stat || has_health_energy {
            self.refresh_derived_stats(victim);
            if let Some((cur, max)) = hp_before
                && max > 0
                && let Some(unit) = self.unit_base_mut(victim)
            {
                #[allow(clippy::cast_possible_truncation)]
                let scaled =
                    (u64::from(unit.max_health()) * u64::from(cur) / u64::from(max)) as u32;
                unit.set_health(scaled);
            }
            if landed && has_health_energy {
                // Re-borrow the landed holder (last
                // pushed) for its effect amounts.
                let landed_holder = self.unit_base(victim).and_then(|u| u.auras.last().cloned());
                if let Some(h) = landed_holder {
                    self.apply_health_energy_adjustments(victim, &h, vitals_before);
                }
            }
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
        self.send_aura_duration(victim, duration_packet.0, duration_packet.1);
        self.refresh_display(victim);
        self.refresh_immune_state(victim);
        self.refresh_attack_speed(victim);
        self.refresh_skill_bonuses(victim);
        self.refresh_speed(victim);
        self.refresh_cc_flags(victim);
    }

    /// Holder-derived `UNIT_FIELD_FLAGS` CC bits
    /// (fork aura handlers `HandleModStun` etc. set
    /// these directly; recomputation is equivalent
    /// and removal-safe). Movement generators for
    /// fear/confuse are not modeled; the flags feed
    /// the cast-prevention gate and client display.
    pub(crate) fn refresh_cc_flags(&mut self, guid: ObjectGuid) {
        let holders = self.aura_holders(guid);
        let mut bits = 0u32;
        for e in holders.iter().flat_map(|h| h.effects.iter().flatten()) {
            bits |= match e.aura_type {
                aura_type::MOD_STUN => UNIT_FLAG_STUNNED,
                aura_type::MOD_CONFUSE => UNIT_FLAG_CONFUSED,
                aura_type::MOD_FEAR => UNIT_FLAG_FLEEING,
                aura_type::MOD_SILENCE => UNIT_FLAG_SILENCED,
                aura_type::MOD_PACIFY => UNIT_FLAG_PACIFIED,
                aura_type::MOD_PACIFY_SILENCE => UNIT_FLAG_PACIFIED | UNIT_FLAG_SILENCED,
                _ => 0,
            };
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let values = unit.world.object.values_mut();
            let cur = values.get_u32(offsets::UNIT_FIELD_FLAGS);
            let next = (cur & !UNIT_FLAG_CC_MASK) | bits;
            if next != cur {
                values.set_u32(offsets::UNIT_FIELD_FLAGS, next);
            }
        }
    }

    /// `RemoveAurasWithInterruptFlags`.
    /// `RemoveAurasWithInterruptFlags(flags,
    /// exceptSpellId)`.
    pub(crate) fn remove_auras_with_interrupt_flags(
        &mut self,
        victim: ObjectGuid,
        flags: u32,
        except_spell: u32,
    ) {
        let indices: Vec<usize> = self
            .aura_holders(victim)
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                h.spell_id != except_spell
                    && self
                        .stores
                        .spells
                        .get(h.spell_id)
                        .is_some_and(|s| s.aura_interrupt_flags & flags != 0)
            })
            .map(|(i, _)| i)
            .collect();
        for index in indices.into_iter().rev() {
            self.remove_aura_at(victim, index);
        }
    }

    /// `RemoveSpellsCausingAura`.
    pub(crate) fn remove_spells_causing_aura(&mut self, victim: ObjectGuid, kind: u32) {
        let indices: Vec<usize> = self
            .aura_holders(victim)
            .iter()
            .enumerate()
            .filter(|(_, h)| h.effects.iter().flatten().any(|e| e.aura_type == kind))
            .map(|(i, _)| i)
            .collect();
        for index in indices.into_iter().rev() {
            self.remove_aura_at(victim, index);
        }
    }

    fn send_mount_result(&mut self, guid: ObjectGuid, result: u32) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u32_le(result);
        self.send_to_player(
            guid,
            WorldPacket::with_body(op::SMSG_MOUNTRESULT, payload.into_vec()),
        );
    }

    fn send_dismount_result(&mut self, guid: ObjectGuid, result: u32) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u32_le(result);
        self.send_to_player(
            guid,
            WorldPacket::with_body(op::SMSG_DISMOUNTRESULT, payload.into_vec()),
        );
    }

    /// Holder-derived mount/transform display state
    /// (fork `HandleAuraMounted`/`HandleAuraTransform`
    /// Real paths, `SpellAuras.cpp:2151/2533`, as a
    /// recompute like `refresh_cc_flags`: display ids
    /// are chosen only on state transitions, matching
    /// the fork's apply-time rolls; the transform
    /// winner simulates the fork precedence over
    /// holders in apply order, which equals its
    /// removal-time last-negative-else-last re-apply).
    /// Pet disable/unsummon-at-mount, extra-attack
    /// reset, shapeshift-display fallback, and the
    /// channel mount-interrupt are unmodeled (no
    /// backing state).
    pub(crate) fn refresh_display(&mut self, guid: ObjectGuid) {
        use mangos_entities::creature::{
            DisplayResolver, choose_display_id_for_template, random_gender_display,
        };
        let holders = self.aura_holders(guid);
        let find_effect = |kind: u32| {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten().map(move |e| (h, e)))
                .find(|(_, e)| e.aura_type == kind)
        };
        // ---- MOUNTED (HandleAuraMounted) ----
        let mounted_now = self.unit_base_mut(guid).map_or(0, |u| {
            u.values().get_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID)
        });
        #[allow(clippy::cast_sign_loss)]
        let mount_entry = find_effect(aura_type::MOUNTED).map(|(_, e)| e.misc_value as u32);
        match (mounted_now != 0, mount_entry) {
            (false, Some(entry)) => {
                let stores = &self.stores;
                let resolver = DisplayResolver {
                    scale_for_display: &|id| stores.scale_for_display(id),
                    model_for_display: &|id| stores.model_for_display(id),
                };
                let display = stores.creature_templates.get(entry).map_or(0, |info| {
                    let (id, _) = choose_display_id_for_template(info, &resolver, &mut self.rng);
                    random_gender_display(&stores.display_addons, id, &mut self.rng)
                });
                if display == 0 {
                    // Player::Mount(0): strip + NOTMOUNTABLE.
                    self.remove_spells_causing_aura(guid, aura_type::MOUNTED);
                    self.send_mount_result(guid, 3);
                } else {
                    if let Some(unit) = self.unit_base_mut(guid) {
                        unit.values()
                            .set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, display);
                    }
                    self.remove_auras_with_interrupt_flags(
                        guid,
                        aura_interrupt_flag::MOUNT_CANCELS,
                        0,
                    );
                    self.send_mount_result(guid, 10);
                    self.pet_mount_transition(guid, true);
                }
            }
            (true, None) => {
                if let Some(unit) = self.unit_base_mut(guid) {
                    unit.values().set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, 0);
                }
                self.remove_auras_with_interrupt_flags(
                    guid,
                    aura_interrupt_flag::DISMOUNT_CANCELS,
                    0,
                );
                self.send_dismount_result(guid, 3);
                self.pet_mount_transition(guid, false);
            }
            _ => {}
        }
        // ---- TRANSFORM (HandleAuraTransform) ----
        let mut winner: Option<(u32, i32, bool)> = None;
        for h in &holders {
            for e in h.effects.iter().flatten() {
                if e.aura_type != aura_type::TRANSFORM {
                    continue;
                }
                let wins = match winner {
                    None => true,
                    Some((_, _, cur_positive)) => !h.positive || cur_positive,
                };
                if wins {
                    winner = Some((h.spell_id, e.misc_value, h.positive));
                }
            }
        }
        let current_spell = self.unit_base_mut(guid).map_or(0, |u| u.transform_spell);
        match winner {
            None if current_spell != 0 => {
                let native = self.unit_base_mut(guid).map_or(0, |u| {
                    u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID)
                });
                let model = self.stores.model_for_display(native);
                if let Some(unit) = self.unit_base_mut(guid) {
                    unit.transform_spell = 0;
                    // The fork restores native here even
                    // while shapeshifted; the form
                    // display is gone until re-shift.
                    unit.shapeshift_display_id = 0;
                    if native != 0 {
                        unit.set_display_id(native, model);
                    }
                    if let Some(base) = unit.transform_base_scale.take() {
                        unit.world.object.set_object_scale(base);
                    }
                }
                if let Some(equip) = self.creatures.get(&guid).map(|c| c.info.equipment_id) {
                    self.creature_load_equipment(guid, equip);
                }
            }
            Some((spell_id, misc, _)) if spell_id != current_spell => {
                self.apply_transform_display(guid, spell_id, misc);
            }
            _ => {}
        }
        // ---- SHAPESHIFT (HandleAuraModShapeshift,
        // display/form subset) ----
        self.refresh_shapeshift(guid, &holders);
    }

    /// `Player::Mount`/`Unmount` pet halves
    /// (`Player.cpp:18023`): aura mount disables
    /// the pet, unmount re-enables it (default
    /// `PetUnsummonAtMount=false` config). The
    /// config-true temporary-unsummon/resummon
    /// arms have no temporary-pet state yet —
    /// recorded in TIER2-PROGRESS Slice 6.2.
    fn pet_mount_transition(&mut self, guid: ObjectGuid, mounting: bool) {
        if guid.is_player() {
            if let Some(pet) = self.pet_of(guid) {
                self.set_pet_enabled(pet, !mounting);
            }
        }
    }

    /// `Aura::HandleAuraModShapeshift`
    /// (`SpellAuras.cpp:2321`) holder-derived subset:
    /// `SetShapeshiftForm` byte, form exclusivity,
    /// display/scale via `GetShapeshiftDisplayInfo`,
    /// the 9033 shift casts, the
    /// `SHAPESHIFTING_CANCELS` strip, and the 23397
    /// (Nefarian warrior call) → 2458 removal cast.
    /// Power-type switching, Furor, tactical-mastery
    /// rage caps, `HandleShapeshiftBoosts`, and
    /// `InitDataForForm` are recorded in
    /// TIER2-PROGRESS Slice 6.3 (form-driven stat
    /// recompute does not exist yet).
    fn refresh_shapeshift(
        &mut self,
        guid: ObjectGuid,
        holders: &[mangos_entities::aura::AuraHolder],
    ) {
        // Winner: last holder carrying MOD_SHAPESHIFT
        // (application order; the fork strips the
        // others inside the new form's apply).
        let mut winner: Option<(u32, u8)> = None;
        let mut other_forms: Vec<u32> = Vec::new();
        for h in holders {
            for e in h.effects.iter().flatten() {
                if e.aura_type == aura_type::MOD_SHAPESHIFT {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let form = e.misc_value as u8;
                    if let Some((prev, _)) = winner {
                        if prev != h.spell_id {
                            other_forms.push(prev);
                        }
                    }
                    winner = Some((h.spell_id, form));
                }
            }
        }
        let (old_form, old_spell, transformed) =
            self.unit_base_mut(guid).map_or((0, 0, false), |u| {
                (
                    u.values().get_byte(offsets::UNIT_FIELD_BYTES_1, 2),
                    u.shapeshift_spell,
                    u.transform_spell != 0,
                )
            });
        let (new_spell, new_form) = winner.unwrap_or((0, 0));
        // Fork: a form missing from
        // SpellShapeshiftForm.dbc makes the handler
        // log and return without touching the form
        // byte. An unloaded table (test worlds)
        // treats every form as known.
        if new_form != 0
            && !self.stores.shapeshift_forms.is_empty()
            && !self
                .stores
                .shapeshift_forms
                .contains_key(&u32::from(new_form))
        {
            return;
        }
        if new_form == old_form && new_spell == old_spell {
            return;
        }
        // State first: nested refreshes from the
        // removals/casts below must see the
        // transition as done.
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values()
                .set_byte(offsets::UNIT_FIELD_BYTES_1, 2, new_form);
            unit.shapeshift_spell = new_spell;
        }
        self.shapeshift_display_transition(guid, old_form, new_form, transformed);
        // Exclusivity (fork: inside apply).
        for spell_id in other_forms {
            self.remove_auras_due_to_spell(guid, spell_id);
        }
        // SHAPESHIFTING_CANCELS strip on apply for
        // non-stance forms.
        if new_form != 0 {
            let flags1 = self
                .stores
                .shapeshift_forms
                .get(&u32::from(new_form))
                .copied()
                .unwrap_or(0);
            if flags1 & 1 == 0 {
                self.remove_auras_with_interrupt_flags(
                    guid,
                    aura_interrupt_flag::SHAPESHIFTING_CANCELS,
                    new_spell,
                );
            }
        }
        // 9033 (Shapeshift Form Effect) fires on
        // both apply and remove of the druid forms
        // in the fork; one cast per transition is
        // observably identical (idempotent strip).
        if DRUID_9033_FORMS.contains(&old_form) || DRUID_9033_FORMS.contains(&new_form) {
            self.cast_spell(
                guid,
                9033,
                CastTarget::Unit(guid),
                CastOptions {
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
        // Removing the Nefarian warrior call (23397)
        // re-casts regular Berserker Stance.
        if old_spell == 23397 && new_spell != 23397 {
            self.cast_spell(
                guid,
                2458,
                CastTarget::Unit(guid),
                CastOptions {
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
    }

    /// Display half of a shapeshift transition
    /// (fork gates both sides on `!GetTransForm()`).
    fn shapeshift_display_transition(
        &mut self,
        guid: ObjectGuid,
        old_form: u8,
        new_form: u8,
        transformed: bool,
    ) {
        if transformed {
            // Form changed under a transform: fork
            // display handlers all skip; nothing of
            // the form is on screen.
            if let Some(unit) = self.unit_base_mut(guid) {
                unit.shapeshift_display_id = 0;
            }
            return;
        }
        let had_display = self
            .unit_base_mut(guid)
            .is_some_and(|u| u.shapeshift_display_id != 0);
        if old_form != 0 && had_display {
            let native = self.unit_base_mut(guid).map_or(0, |u| {
                u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID)
            });
            let model = self.stores.model_for_display(native);
            if let Some(unit) = self.unit_base_mut(guid) {
                unit.shapeshift_display_id = 0;
                if native != 0 {
                    unit.set_display_id(native, model);
                }
                if let Some(base) = unit.transform_base_scale.take() {
                    unit.world.object.set_object_scale(base);
                }
            }
        }
        if new_form != 0 {
            let (display_id, scale_mod) = self.shapeshift_display_info(guid, new_form);
            if display_id > 0 {
                let model = self.stores.model_for_display(display_id);
                let native_model_scale = self
                    .unit_base_mut(guid)
                    .map(|u| u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID))
                    .and_then(|n| self.stores.model_for_display(n))
                    .map_or(1.0, |m| m.native_scale);
                if let Some(unit) = self.unit_base_mut(guid) {
                    if unit.transform_base_scale.is_none() {
                        unit.transform_base_scale = Some(unit.world.object.object_scale());
                    }
                    unit.set_display_id(display_id, model);
                    unit.shapeshift_display_id = display_id;
                    if scale_mod != 0.0 && native_model_scale != 0.0 {
                        let base = unit.transform_base_scale.unwrap_or(1.0);
                        unit.world
                            .object
                            .set_object_scale(base * scale_mod / native_model_scale);
                    }
                }
            }
        }
    }

    /// `GetShapeshiftDisplayInfo`
    /// (`SpellAuras.cpp:2217`): (display id,
    /// transform-scale mod). Team via
    /// `Player::TeamForRace` on the bytes-0 race.
    fn shapeshift_display_info(&mut self, guid: ObjectGuid, form: u8) -> (u32, f32) {
        let (race, native) = self.unit_base_mut(guid).map_or((0, 0), |u| {
            (
                u.values().get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
            )
        });
        let horde = matches!(race, 2 | 5 | 6 | 8);
        let is_player = guid.is_player();
        match form {
            // FORM_CAT
            1 => (if is_player && horde { 8571 } else { 892 }, 0.80),
            // FORM_TRAVEL
            3 => (632, 0.80),
            // FORM_AQUA
            4 => (2428, 0.80),
            // FORM_BEAR / FORM_DIREBEAR
            5 | 8 => (if is_player && horde { 2289 } else { 2281 }, 1.0),
            // FORM_GHOUL (no IsPlayer check in fork)
            7 => (if horde { 0 } else { 10045 }, 1.0),
            // FORM_CREATUREBEAR: native model switch
            14 => {
                let model_id = self
                    .stores
                    .model_for_display(native)
                    .map_or(0, |m| m.model_id);
                let display = match model_id {
                    // MODEL_NELF_FEMALE/MALE
                    55 | 56 => 2281,
                    // MODEL_TAUREN_FEMALE/MALE
                    59 | 60 => 2289,
                    _ => 902,
                };
                (display, 1.0)
            }
            // FORM_GHOSTWOLF
            16 => (4613, 0.80),
            // FORM_MOONKIN
            31 => (if is_player && horde { 15375 } else { 15374 }, 1.0),
            // FORM_TREE
            2 => (864, 1.0),
            // FORM_SPIRITOFREDEMPTION
            32 => (16031, 1.0),
            _ => (0, 1.0),
        }
    }

    /// `HandleAuraTransform` apply half for a winning
    /// switch (display choice + `SetTransformScale` +
    /// creature equipment swap).
    fn apply_transform_display(&mut self, guid: ObjectGuid, spell_id: u32, misc: i32) {
        use mangos_entities::creature::{
            DisplayResolver, UNIT_DISPLAY_ID_BOX, choose_display_id_for_template,
        };
        let mut display_id = 0u32;
        let mut mod_x = 1.0f32;
        let mut equip: Option<u32> = None;
        if misc == 0 {
            // Orb of Deception (16739) race/gender table;
            // other misc-0 spells log-only in the fork.
            if spell_id == 16739 {
                let (race, cur_display, native) = self.unit_base_mut(guid).map_or((0, 0, 0), |u| {
                    (
                        u.values().get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                        u.values().get_u32(offsets::UNIT_FIELD_DISPLAYID),
                        u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
                    )
                });
                let male = self
                    .stores
                    .display_addons
                    .creature(cur_display)
                    .is_none_or(|a| a.gender == 0);
                let native_scale = self.stores.scale_for_display(native);
                // Fork race ids: human 1, orc 2, dwarf 3,
                // nightelf 4, undead 5, tauren 6, gnome 7,
                // troll 8.
                let pick = |m: u32, f: u32| if male { m } else { f };
                match race {
                    8 => display_id = pick(10135, 10134),
                    6 => {
                        display_id = pick(10136, 10147);
                        if native_scale != 0.0 {
                            mod_x = 1.15 / native_scale;
                        }
                    }
                    1 => display_id = pick(10137, 10138),
                    2 => display_id = pick(10139, 10140),
                    3 => display_id = pick(10141, 10142),
                    4 => display_id = pick(10143, 10144),
                    5 => display_id = pick(10146, 10145),
                    7 => {
                        display_id = pick(10148, 10149);
                        if native_scale != 0.0 {
                            mod_x = if male { 1.35 } else { 1.25 } / native_scale;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            #[allow(clippy::cast_sign_loss)]
            let entry = misc as u32;
            let stores = &self.stores;
            let resolver = DisplayResolver {
                scale_for_display: &|id| stores.scale_for_display(id),
                model_for_display: &|id| stores.model_for_display(id),
            };
            match stores.creature_templates.get(entry) {
                None => display_id = UNIT_DISPLAY_ID_BOX,
                Some(info) => {
                    let (id, scale) =
                        choose_display_id_for_template(info, &resolver, &mut self.rng);
                    display_id = id;
                    if self.creatures.contains_key(&guid) {
                        equip = Some(info.equipment_id);
                        mod_x = scale;
                    }
                }
            }
        }
        let model = self.stores.model_for_display(display_id);
        let native_model_scale = self
            .unit_base_mut(guid)
            .map(|u| u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID))
            .and_then(|n| self.stores.model_for_display(n))
            .map_or(1.0, |m| m.native_scale);
        if let Some(unit) = self.unit_base_mut(guid) {
            if display_id != 0 {
                if unit.transform_base_scale.is_none() {
                    unit.transform_base_scale = Some(unit.world.object.object_scale());
                }
                unit.set_display_id(display_id, model);
                // The transform owns the display now;
                // any shapeshift display is displaced.
                unit.shapeshift_display_id = 0;
                // SetTransformScale(mod_x): object scale
                // *= mod_x / native model scale.
                if mod_x != 0.0 && native_model_scale != 0.0 {
                    let base = unit.transform_base_scale.unwrap_or(1.0);
                    unit.world
                        .object
                        .set_object_scale(base * mod_x / native_model_scale);
                }
            }
            unit.transform_spell = spell_id;
        }
        if let Some(equip_id) = equip {
            self.creature_load_equipment(guid, equip_id);
        }
    }

    /// `Unit::UpdateSpeed` aura factor (fork
    /// `Unit.cpp:7195`), run + swim subset:
    /// main mod = max-positive increase; bonus =
    /// max(not-stack, always-multiplier); strongest
    /// slow multiplies last. Normalization/ghost
    /// arms unmodeled.
    pub(crate) fn refresh_speed(&mut self, guid: ObjectGuid) {
        let (base_run, base_swim) = {
            let Some(unit) = self.unit_base_mut(guid) else {
                return;
            };
            (
                unit.base_speed_rate[mangos_entities::unit::MOVE_RUN],
                unit.base_speed_rate[3],
            )
        };
        let holders = self.aura_holders(guid);
        let max_pos = |kind: u32| -> i32 {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == kind && e.amount > 0)
                .map(|e| e.amount)
                .max()
                .unwrap_or(0)
        };
        let max_neg = |kind: u32| -> i32 {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == kind && e.amount < 0)
                .map(|e| e.amount)
                .min()
                .unwrap_or(0)
        };
        // UpdateSpeed mounted arm: on-mount aura kinds.
        let mounted = self
            .unit_base_mut(guid)
            .is_some_and(|u| u.values().get_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID) != 0);
        #[allow(clippy::cast_precision_loss)]
        let factor = |increase_kind: u32, stack_kinds: Option<(u32, u32)>| -> f32 {
            let main_mod = max_pos(increase_kind);
            let (stack, non_stack) = match stack_kinds {
                Some((always, not_stack)) => (
                    self.total_aura_multiplier(guid, always, None),
                    (100.0 + max_pos(not_stack) as f32) / 100.0,
                ),
                None => (1.0, 1.0),
            };
            let bonus = stack.max(non_stack);
            let mut speed = if main_mod != 0 {
                bonus * (100.0 + main_mod as f32) / 100.0
            } else {
                bonus
            };
            let slow = max_neg(aura_type::MOD_DECREASE_SPEED);
            if slow != 0 {
                speed *= (100.0 + slow as f32) / 100.0;
            }
            speed
        };
        let (run_kind, run_stack) = if mounted {
            (
                aura_type::MOD_INCREASE_MOUNTED_SPEED,
                (
                    aura_type::MOD_MOUNTED_SPEED_ALWAYS,
                    aura_type::MOD_MOUNTED_SPEED_NOT_STACK,
                ),
            )
        } else {
            (
                aura_type::MOD_INCREASE_SPEED,
                (aura_type::MOD_SPEED_ALWAYS, aura_type::MOD_SPEED_NOT_STACK),
            )
        };
        let run = base_run * factor(run_kind, Some(run_stack));
        let swim = base_swim * factor(aura_type::MOD_INCREASE_SWIM_SPEED, None);
        self.set_speed_rate(guid, mangos_entities::unit::MOVE_RUN, run);
        self.set_speed_rate(guid, 3, swim);
    }

    #[allow(clippy::too_many_lines)] // one RemoveAura port
    pub(crate) fn remove_aura_at(&mut self, victim: ObjectGuid, index: usize) {
        let mut has_ap = false;
        let mut has_stat = false;
        let mut hp_before = None;
        let peek = self.unit_base_mut(victim).and_then(|u| {
            u.auras.get(index).map(|h| {
                (
                    h.spell_id,
                    h.effects.iter().flatten().any(|e| {
                        e.aura_type == aura_type::MOD_TOTAL_STAT_PERCENTAGE
                            && e.misc_value == STAT_STAMINA_I32
                    }),
                )
            })
        });
        let preserve = peek.is_some_and(|(spell_id, stamina)| {
            stamina
                && self
                    .stores
                    .spells
                    .get(spell_id)
                    .is_some_and(|s| s.attributes & 0x10 != 0)
        });
        if preserve {
            hp_before = self
                .unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| (u.health(), u.max_health()));
        }
        // Health/energy TOTAL-group removal state
        // (`HandleAuraModIncreaseHealth[Percent]` /
        // energy remove arms): Last Stand adjusts
        // current health *before* the stat removal;
        // percent forms preserve the ratio across it.
        let he_peek = self.unit_base(victim).and_then(|u| {
            u.auras.get(index).and_then(|h| {
                Self::holder_has_health_energy_mod(h).then(|| {
                    (
                        h.clone(),
                        u.is_alive(),
                        u.health(),
                        u.max_health(),
                        core::array::from_fn(|p| u.power(p)),
                        core::array::from_fn(|p| u.max_power(p)),
                    )
                })
            })
        });
        if let Some((h, alive, hp, _, _, _)) = &he_peek
            && h.spell_id == 12976
            && *alive
        {
            let flat: i32 = h
                .effects
                .iter()
                .flatten()
                .filter(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH)
                .map(|e| e.amount)
                .sum();
            if let Some(unit) = self.unit_base_mut(victim) {
                let next = if i64::from(*hp) > i64::from(flat) {
                    i64::from(*hp) - i64::from(flat)
                } else {
                    1
                };
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                unit.set_health(next.clamp(0, i64::from(u32::MAX)) as u32);
            }
        }
        let mut removed_holder: Option<(ObjectGuid, u32)> = None;
        let mut has_health_energy = false;
        let extracted = if let Some(unit) = self.unit_base_mut(victim) {
            if index >= unit.auras.len() {
                return;
            }
            Some(unit.auras.remove(index))
        } else {
            None
        };
        if let Some(mut holder) = extracted {
            removed_holder = Some((holder.caster, holder.spell_id));
            has_ap = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
            has_stat = Self::holder_has_stat_mod(&holder);
            has_health_energy = Self::holder_has_health_energy_mod(&holder);
            // AuraScript::OnBeforeApply(apply=false)
            // before the unapply handlers (fork
            // ApplyModifier order).
            self.aura_script_on_before_apply(victim, &mut holder, false);
            if let Some(unit) = self.unit_base_mut(victim) {
                Self::aura_effect_apply(unit, &holder, false);
                Self::write_aura_slot(unit, &holder, true);
            }
        }
        if let Some((removed_caster, removed_spell_id)) = removed_holder {
            self.aura_script_on_after_apply(victim, removed_spell_id, removed_caster, false);
            self.aura_dummy_for_spell(victim, removed_spell_id, false);
        }
        if has_stat || has_health_energy {
            self.refresh_derived_stats(victim);
            if let Some((cur, max)) = hp_before
                && max > 0
                && let Some(unit) = self.unit_base_mut(victim)
            {
                #[allow(clippy::cast_possible_truncation)]
                let scaled =
                    (u64::from(unit.max_health()) * u64::from(cur) / u64::from(max)) as u32;
                unit.set_health(scaled);
            }
            if let Some((h, alive, old_hp, old_max, old_powers, old_max_powers)) = he_peek
                && alive
            {
                self.remove_health_energy_adjustments(
                    victim,
                    &h,
                    (old_hp, old_max, old_powers, old_max_powers),
                );
            }
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
        // Cooldown-on-event conversion: the timed
        // cooldown starts when the aura fades
        // (`SpellAuras.cpp` holder cleanup).
        if let Some((caster, spell_id)) = removed_holder
            && let Some(spell) = self.stores.spells.get(spell_id).cloned()
            && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0
        {
            self.start_event_cooldown(caster, &spell);
        }
        self.refresh_display(victim);
        self.refresh_immune_state(victim);
        self.refresh_attack_speed(victim);
        self.refresh_skill_bonuses(victim);
        self.refresh_speed(victim);
        self.refresh_cc_flags(victim);
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

    /// `SpellAuraHolder::SetStackAmount` reduced: set the
    /// holder's stack count and rewrite its aura slot
    /// (stack byte). Modifier magnitude does not scale
    /// by stack in the Rust aura system yet (global
    /// stacking gap; the fork rescales amounts in the
    /// aura handlers) — recorded in TIER3-PROGRESS.
    pub(crate) fn set_aura_stack_amount(&mut self, victim: ObjectGuid, spell_id: u32, stack: u32) {
        let landed = self.unit_base_mut(victim).and_then(|unit| {
            let holder = unit.auras.iter_mut().find(|h| h.spell_id == spell_id)?;
            holder.stack = stack.max(1);
            let snapshot = holder.clone();
            Self::write_aura_slot(unit, &snapshot, false);
            Some(())
        });
        if landed.is_some() {
            self.refresh_derived_stats(victim);
        }
    }

    /// `RemoveAurasDueToSpell` (no cancelable gate;
    /// proc charge exhaustion path).
    pub fn remove_auras_due_to_spell(&mut self, victim: ObjectGuid, spell_id: u32) {
        loop {
            let found = self
                .unit_base_mut(victim)
                .and_then(|u| u.auras.iter().position(|h| h.spell_id == spell_id));
            match found {
                Some(i) => self.remove_aura_at(victim, i),
                None => return,
            }
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
            self.apply_spell_aura(caster, &spell, victim, None, [None; 3]);
        }
    }

    #[allow(clippy::too_many_lines, clippy::needless_range_loop)] // one EffectApplyAura port
    pub(crate) fn apply_spell_aura(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        bp: [Option<i32>; 3],
    ) {
        let raw_duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        // 0 and -1 are the permanent shapes; only
        // positive durations take the spellmod.
        let duration = match raw_duration.cmp(&0) {
            std::cmp::Ordering::Equal => -1,
            std::cmp::Ordering::Less => raw_duration,
            std::cmp::Ordering::Greater => {
                let modded = self.modded_duration_ms(caster, spell, applied_mods.as_deref_mut());
                if modded <= 0 {
                    return;
                }
                modded
            }
        };

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
        #[allow(clippy::cast_possible_wrap)]
        {
            holder.proc_charges = spell.proc_charges as i32;
        }
        for i in 0..3 {
            let eff = spell.effects[i];
            // `CreateAura` builds an aura for
            // APPLY_AURA-family and
            // PERSISTENT_AREA_AURA effects alike
            // (Flamestrike's periodic damage rides on
            // the persistent effect itself).
            if eff.effect != effect::APPLY_AURA
                && eff.effect != effect::APPLY_AREA_AURA_PARTY
                && eff.effect != effect::PERSISTENT_AREA_AURA
            {
                continue;
            }
            if eff.apply_aura_name == aura_type::NONE {
                continue;
            }
            let mut value = self.effect_value_bp(spell, i, caster_level, bp[i]);
            // Apply-time Done(DOT) snapshot
            // (`CalculateDotDamage` / periodic-heal
            // 1.11 snapshot rule); melee-class periodic
            // damage routes through MeleeDamageBonus
            // in the fork — recorded exclusion.
            if !spell_meleedamage(spell.dmg_class) {
                match eff.apply_aura_name {
                    aura_type::PERIODIC_DAMAGE | aura_type::PERIODIC_LEECH => {
                        value = self.spell_damage_bonus_done(
                            caster,
                            victim,
                            spell,
                            i,
                            value,
                            true,
                            1,
                            applied_mods.as_deref_mut(),
                        );
                    }
                    aura_type::PERIODIC_HEAL => {
                        value = self.spell_healing_bonus_done(
                            caster,
                            spell,
                            i,
                            value,
                            true,
                            1,
                            applied_mods.as_deref_mut(),
                        );
                    }
                    _ => {}
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            let amount = value as i32;
            // spell_haunting_spirits AuraScript
            // OnBeforeApply: SetPeriodicTimer(5s)
            // (sets isPeriodic + period + timer).
            let haunt = spell.id == 7057 && i == 0;
            let periodic = haunt
                || matches!(
                    eff.apply_aura_name,
                    aura_type::PERIODIC_DAMAGE
                        | aura_type::PERIODIC_HEAL
                        | aura_type::PERIODIC_ENERGIZE
                        | aura_type::PERIODIC_TRIGGER_SPELL
                        | aura_type::PERIODIC_DAMAGE_PERCENT
                        | aura_type::PERIODIC_LEECH
                );
            let period_ms = if haunt {
                5000
            } else if periodic {
                eff.amplitude
            } else {
                0
            };
            #[allow(clippy::cast_possible_wrap)]
            let timer = period_ms as i32;
            // `HandleAddModifier` charge seeding:
            // proc charges, infinite when the source
            // stacks, custom 1 for Shadow Trance /
            // Netherwind Focus.
            let mod_charges = if matches!(
                eff.apply_aura_name,
                aura_type::ADD_FLAT_MODIFIER | aura_type::ADD_PCT_MODIFIER
            ) {
                if matches!(spell.id, 17941 | 22008) {
                    1
                } else if spell.stack_amount > 1 {
                    0
                } else {
                    #[allow(clippy::cast_possible_wrap)]
                    {
                        spell.proc_charges as i32
                    }
                }
            } else {
                0
            };
            holder.effects[i] = Some(AuraEffectState {
                aura_type: eff.apply_aura_name,
                amount,
                base_points: amount,
                period_ms,
                periodic_timer: timer,
                periodic_ticks: 0,
                misc_value: eff.misc_value,
                mod_charges,
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
        self.apply_immunity_purges(victim, spell);
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
            // Saved charges are written 0 today; reseed
            // like a fresh application (see mod charges).
            #[allow(clippy::cast_possible_wrap)]
            {
                holder.proc_charges = spell.proc_charges as i32;
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
                // Saved rows carry no live mod charges
                // (save writes 0); a surviving modifier
                // aura therefore reseeds like a fresh
                // application.
                let mod_charges = if matches!(
                    eff.apply_aura_name,
                    aura_type::ADD_FLAT_MODIFIER | aura_type::ADD_PCT_MODIFIER
                ) {
                    if matches!(spell.id, 17941 | 22008) {
                        1
                    } else if spell.stack_amount > 1 {
                        0
                    } else {
                        #[allow(clippy::cast_possible_wrap)]
                        {
                            spell.proc_charges as i32
                        }
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
                    mod_charges,
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
        // Slot-less triggered casts tick on their
        // own timers (fork spell events); they are
        // not pushed back or interrupted by movement.
        if !self.pending_triggered.is_empty() {
            let pending = std::mem::take(&mut self.pending_triggered);
            let mut keep = Vec::new();
            let mut due = Vec::new();
            for (caster, mut cast) in pending {
                if self.unit_base(caster).is_none() {
                    continue;
                }
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                if cast.remaining_ms <= 0 {
                    due.push((caster, cast));
                } else {
                    keep.push((caster, cast));
                }
            }
            self.pending_triggered = keep;
            for (caster, cast) in due {
                self.execute_cast(caster, cast);
            }
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
        // `PersistentAreaAura::Update` pre-pass: a
        // holder sourced from a dynamic object is
        // removed when the object is gone or the
        // target left its radius; a persistent holder
        // without a dynobject (reloaded from DB)
        // removes itself immediately. Whole-holder
        // removal where the fork removes per-effect is
        // a recorded divergence.
        let mut orphaned: Vec<(ObjectGuid, usize)> = Vec::new();
        for target in &targets {
            let holders = self.aura_holders(*target);
            for (index, holder) in holders.iter().enumerate() {
                let is_persistent = holder.dynobj != ObjectGuid::EMPTY
                    || self.stores.spells.get(holder.spell_id).is_some_and(|s| {
                        s.effects
                            .iter()
                            .any(|e| e.effect == effect::PERSISTENT_AREA_AURA)
                    });
                if !is_persistent {
                    continue;
                }
                let remove = if holder.dynobj == ObjectGuid::EMPTY {
                    true
                } else if let Some(dynobj) = self.dynobjects.get(&holder.dynobj) {
                    self.unit_position(*target).is_none_or(|pos| {
                        crate::visibility::dist2d(pos, dynobj.world.position()) > dynobj.radius
                    })
                } else {
                    true
                };
                if remove {
                    orphaned.push((*target, index));
                }
            }
        }
        orphaned.sort_by_key(|&(_, index)| std::cmp::Reverse(index));
        for (target, index) in orphaned {
            self.remove_aura_at(target, index);
        }
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

    #[allow(clippy::too_many_lines)] // one periodic-tick drain port
    fn process_periodic_ticks(&mut self, ticks: Vec<PeriodicTick>) {
        for tick in ticks {
            match tick.aura_type {
                // `Aura::TriggerSpell` generic arm: cast
                // the effect's trigger spell at the aura
                // target (wearer casts with
                // `SPELL_ATTR_WEARER_CASTS_PROC_TRIGGER`).
                // The fork's per-spell trigger customs
                // are not ported.
                aura_type::PERIODIC_TRIGGER_SPELL => {
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    #[allow(clippy::cast_possible_truncation)]
                    let raw_trigger = spell.effects[tick.effect_index as usize].trigger_spell;
                    if !self.unit_is_alive(tick.target) {
                        continue;
                    }
                    let caster = if spell.attributes & 0x0000_0800 != 0 {
                        tick.target
                    } else {
                        tick.caster
                    };
                    if !self.unit_is_alive(caster) {
                        continue;
                    }
                    // AuraScript::OnPeriodicTrigger may
                    // cast a custom spell and suppress
                    // (None) or replace the default
                    // trigger cast (fork TriggerSpell).
                    let mut trigger = (raw_trigger != 0).then_some(raw_trigger);
                    self.aura_script_on_periodic_trigger(&tick, &spell, caster, &mut trigger);
                    let Some(trigger_id) = trigger else {
                        // `Aura::TriggerSpell` tail
                        // (`SpellAuras.cpp:1518-1524`): no
                        // triggered spell and no AuraScript
                        // (empty `script_name` proxy) ->
                        // creature dummy script on the
                        // wearer. The fork logs an error
                        // when unhandled; silent here.
                        if spell.script_name.is_empty() && tick.target.is_creature() {
                            self.effect_dummy_creature(
                                caster,
                                tick.spell_id,
                                tick.effect_index as usize,
                                tick.target,
                            );
                        }
                        continue;
                    };
                    self.cast_spell(
                        caster,
                        trigger_id,
                        CastTarget::Unit(tick.target),
                        CastOptions {
                            triggered: true,
                            triggered_by_aura: true,
                            skip_known: true,
                            cast_item: None,
                            custom_base_points: [None; 3],
                            triggered_by_aura_spell: tick.spell_id,
                            triggered_by_spell: 0,
                        },
                    );
                }
                aura_type::PERIODIC_DAMAGE | aura_type::PERIODIC_LEECH => {
                    let is_leech = tick.aura_type == aura_type::PERIODIC_LEECH;
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    // Caster must still exist on the map
                    // for the damage attribution.
                    if self.unit_position(tick.caster).is_none() {
                        continue;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    if self.is_immune_to_damage(tick.target, 1i32 << spell.school, Some(&spell)) {
                        self.send_spell_or_damage_immune(tick.caster, tick.target, tick.spell_id);
                        continue;
                    }
                    let health_before = if is_leech {
                        self.unit_value_u32(tick.target, offsets::UNIT_FIELD_HEALTH)
                    } else {
                        0
                    };
                    self.school_damage(
                        tick.caster,
                        &spell,
                        tick.target,
                        tick.amount,
                        tick.effect_index,
                        Some(tick.spell_id),
                        None,
                        Some(&ProcRaiseCtx {
                            attacker_flags: proc_flag::DEAL_HARMFUL_PERIODIC,
                            victim_flags: proc_flag::TAKE_HARMFUL_PERIODIC,
                            can_trigger: true,
                            triggered_by_aura: false,
                            att_type: 0,
                            applied_mods: Vec::new(),
                        }),
                    );
                    if is_leech {
                        // Heal the caster for damage
                        // actually dealt (health delta =
                        // fork DealDamage result in this
                        // sequential tick) × multiple value.
                        let dealt = health_before.saturating_sub(
                            self.unit_value_u32(tick.target, offsets::UNIT_FIELD_HEALTH),
                        );
                        if dealt > 0 {
                            let mut multiplier =
                                spell.effects[tick.effect_index as usize].multiple_value;
                            if multiplier <= 0.0 {
                                multiplier = 1.0;
                            }
                            multiplier = self.apply_spell_mod(
                                tick.caster,
                                &spell,
                                spellmod::MULTIPLE_VALUE,
                                multiplier,
                                false,
                                None,
                            );
                            #[allow(
                                clippy::cast_precision_loss,
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss
                            )]
                            let heal = (dealt as f32 * multiplier) as u32;
                            if heal > 0 {
                                self.heal_unit(tick.caster, heal);
                            }
                        }
                    }
                }
                aura_type::PERIODIC_HEAL => {
                    if tick.amount > 0 {
                        // Done snapshotted at apply;
                        // `SpellHealingBonusTaken`(DOT)
                        // per tick, then ditheru.
                        #[allow(clippy::cast_precision_loss)]
                        let amount = match self.stores.spells.get(tick.spell_id).cloned() {
                            Some(spell)
                                if self.is_immune_to_school(
                                    tick.target,
                                    &spell,
                                    tick.effect_index as usize,
                                ) =>
                            {
                                self.send_spell_or_damage_immune(
                                    tick.caster,
                                    tick.target,
                                    tick.spell_id,
                                );
                                0.0
                            }
                            Some(spell) => {
                                let stack = self
                                    .aura_holders(tick.target)
                                    .into_iter()
                                    .find(|h| {
                                        h.spell_id == tick.spell_id && h.caster == tick.caster
                                    })
                                    .map_or(1, |h| h.stack);
                                self.spell_healing_bonus_taken(
                                    tick.target,
                                    &spell,
                                    tick.effect_index as usize,
                                    tick.amount as f32,
                                    true,
                                    stack,
                                    None,
                                )
                            }
                            None => tick.amount as f32,
                        };
                        if amount > 0.0 {
                            let amount = mangos_spells::ditheru(amount, self.spell_rng());
                            self.heal_unit(tick.target, amount);
                        }
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
                        None,
                        Some(&ProcRaiseCtx {
                            attacker_flags: proc_flag::DEAL_HARMFUL_PERIODIC,
                            victim_flags: proc_flag::TAKE_HARMFUL_PERIODIC,
                            can_trigger: true,
                            triggered_by_aura: false,
                            att_type: 0,
                            applied_mods: Vec::new(),
                        }),
                    );
                }
                aura_type::DUMMY => {
                    self.periodic_dummy_tick(&tick);
                }
                _ => {}
            }
        }
    }

    /// `Aura::PeriodicDummyTick` (`SpellAuras.cpp:
    /// 6260`): `AuraScript` `OnPeriodicDummy`
    /// dispatch first, then the fork's hardcoded
    /// family switch. Only ported arms run;
    /// the rest of the switch is recorded in
    /// TIER3-PROGRESS Slice 2 step 4.
    fn periodic_dummy_tick(&mut self, tick: &PeriodicTick) {
        self.aura_script_on_periodic_dummy(tick);
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
        // `MOD_RESISTANCE` auras fold in at read (the
        // client resistance field is not refreshed
        // with them in this subset; recorded).
        i32::from_ne_bytes(self.unit_value_u32(guid, index).to_ne_bytes())
            + self.total_aura_modifier(guid, aura_type::MOD_RESISTANCE, Some(1 << school_id))
    }

    /// `GetTotalAuraModifier(ByMiscMask)`: sum of
    /// holder effect amounts for one aura type,
    /// optionally filtered by misc-value bitmask
    /// (school masks for the damage family).
    pub(crate) fn total_aura_modifier(
        &self,
        guid: ObjectGuid,
        aura_type_id: u32,
        misc_mask: Option<i32>,
    ) -> i32 {
        self.aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| {
                e.aura_type == aura_type_id && misc_mask.is_none_or(|mask| e.misc_value & mask != 0)
            })
            .map(|e| e.amount)
            .sum()
    }

    /// `GetTotalAuraMultiplier(ByMiscMask)`: product
    /// of `(100 + amount) / 100`.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn total_aura_multiplier(
        &self,
        guid: ObjectGuid,
        aura_type_id: u32,
        misc_mask: Option<i32>,
    ) -> f32 {
        self.aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| {
                e.aura_type == aura_type_id && misc_mask.is_none_or(|mask| e.misc_value & mask != 0)
            })
            .map(|e| (100.0 + e.amount as f32) / 100.0)
            .product()
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

    /// `GetSpellCritFromIntellect` (creatures flat
    /// 5%) with `SPELLMOD_CRITICAL_CHANCE` applied.
    #[allow(clippy::cast_precision_loss)]
    fn spell_crit_percent(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
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
        let base = if caster.is_player() {
            let class = self.unit_value_u32(caster, offsets::UNIT_FIELD_BYTES_0) >> 8 & 0xFF;
            let idx = (class as usize).min(11);
            let intellect = self.unit_stat(caster, 3);
            let level = self.unit_level(caster) as f32;
            let ratio = CRIT_DATA[idx][1] + CRIT_DATA[idx][2] * level;
            (CRIT_DATA[idx][0] + intellect / ratio).max(0.0)
        } else {
            5.0
        };
        let modded = self.apply_spell_mod(
            caster,
            spell,
            spellmod::CRITICAL_CHANCE,
            base,
            true,
            applied_mods,
        );
        // `MOD_CRIT_PERCENT` aura term.
        modded + self.total_aura_modifier(caster, aura_type::MOD_CRIT_PERCENT, None) as f32
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

#[cfg(test)]
mod tests {
    use super::next_delay_at_damage_ms;

    #[test]
    fn pushback_delay_schedule_matches_fork() {
        let mut count = 0u8;
        let schedule: Vec<i32> = (0..7)
            .map(|_| next_delay_at_damage_ms(&mut count))
            .collect();
        assert_eq!(schedule, [1000, 800, 600, 400, 200, 200, 200]);
        assert_eq!(count, 5, "counter stops advancing at 5");
    }
}
