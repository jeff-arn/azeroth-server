//! Spell casting and auras: the `prepare -> cast -> go`
//! pipeline over [`SpellEntry`] data, applied in the
//! map tick, including target collection, GCD and
//! spell/category cooldowns, hit/resist/crit rolls,
//! aura slots/durations/periodic ticks, creature
//! death, and respawn scheduling.
//!
//! Recorded subset deviations: projectile delay is
//! implemented per target (dealer values computed at
//! landing rather than snapshotted at launch, and the
//! player spell-batching delay for instant spells is
//! not modeled), spell mods are implemented
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
//! no combos/area/container targets, and creature
//! corpses stay until respawn (no decay).

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
const UNIT_FLAG_POSSESSED: u32 = 0x0100_0000;
const UNIT_FLAG_CC_MASK: u32 = UNIT_FLAG_SILENCED
    | UNIT_FLAG_PACIFIED
    | UNIT_FLAG_STUNNED
    | UNIT_FLAG_CONFUSED
    | UNIT_FLAG_FLEEING;

use mangos_spells::defines::{
    aura as aura_type, aura_interrupt_flag, aura_state, cast_result, dmg_class, effect, power,
    proc_ex, proc_flag, school, spell_interrupt_flag, spellmod, target as tgt,
};

use crate::procs::{ProcArgs, ProcRaiseCtx, att_type as proc_att_type};
use mangos_spells::{SpellEntry, spell_mod_apply};

use mangos_world_data::spawn_support::RespawnTimes;

use crate::map::{CooldownEntry, LoadedCooldown, Map, MapEvent};

/// `UNIT_FLAG_IN_COMBAT`.
const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
/// Berserker Rage, whose first effect amplifies rage
/// gained from damage taken (`Player::RewardRage`).
const SPELL_BERSERKER_RAGE: u32 = 18_499;
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
/// `SPELL_ATTR_NOT_SHAPESHIFT` (Attributes bit 16).
const ATTR_NOT_SHAPESHIFT: u32 = 0x0001_0000;
/// `SPELL_ATTR_EX2_ALLOW_WHILE_NOT_SHAPESHIFTED` (bit 19).
const ATTR_EX2_ALLOW_WHILE_NOT_SHAPESHIFTED: u32 = 0x0008_0000;
/// `SHAPESHIFT_FLAG_STANCE` (`SpellShapeshiftForm` flags1 bit 0).
const SHAPESHIFT_FLAG_STANCE: u32 = 0x0000_0001;
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
    /// Corpse object target (`TARGET_FLAG_CORPSE_ALLY` /
    /// `CORPSE_ENEMY`): resurrect spells resolve it to
    /// the corpse's owner player at fill time.
    Corpse(ObjectGuid),
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
    /// Per-cast script state from
    /// `SpellScript::OnInit` (fork script-instance
    /// members): the pre-rolled effect index for
    /// `spell_gnomeregan_collecting_fallout`.
    pub script_effect_pick: Option<u32>,
    /// Original caster position (movement interrupts —
    /// handled by `apply_movement` cancellation, kept
    /// for diagnostics).
    pub position: Position,
    /// `m_originalCasterGUID` when a caller casts on
    /// another unit's behalf (Spirit Heal's gossip
    /// select); `None` means the caster itself.
    pub original_caster: Option<ObjectGuid>,
}

/// One projectile-delayed target hit (`Spell` in
/// `SPELL_STATE_DELAYED`, `TargetInfo::timeDelay`):
/// the launch already sent `SMSG_SPELL_GO` and
/// rolled hit/miss; effects land when the timer
/// elapses (`handle_delayed`). Damage/heal values
/// are computed at landing here; the fork snapshots
/// dealer values at launch
/// (`HandleDelayedSpellLaunch`) — recorded
/// deviation.
pub(crate) struct DelayedSpellHit {
    pub caster: ObjectGuid,
    pub spell_id: u32,
    pub caster_level: u32,
    pub victim: ObjectGuid,
    pub target: CastTarget,
    pub applied_mods: Vec<(u32, u8)>,
    pub ctx: ProcRaiseCtx,
    pub custom_base_points: [Option<i32>; 3],
    /// Item-use cast context (`m_CastItem`), carried to
    /// delayed hits so the script lane still sees it.
    pub cast_item: Option<CastItemCtx>,
    pub cast_counter: u8,
    pub triggered_by_aura_spell: u32,
    pub triggered_by_spell: u32,
    pub script_effect_pick: Option<u32>,
    pub original_caster: Option<ObjectGuid>,
    pub remaining_ms: i32,
    /// Last scheduled hit of its cast: runs the
    /// `finish()` tail after applying.
    pub finish: bool,
    /// A non-caster target missed at cast time, so a
    /// finishing-move spell keeps its combo points
    /// (`Spell.cpp:4467-4478` `needDrop`).
    pub missed_non_caster: bool,
}

/// Why a cast did not start (for the actor's logging;
/// the client packet is emitted by the map).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CastOutcome {
    Started,
    Failed(u8),
    UnknownSpell,
}

/// `AuraRemoveMode` subset the script seams read
/// (fork `Aura::GetRemoveMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuraRemoveMode {
    Default,
    Cancel,
    Dispel,
    Expire,
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

mod aura;
mod cast;
mod cooldown;
mod damage;
mod effects;
mod immunity;

/// `Player::RewardRage` amount, returned in the
/// stored power units (tenths of one rage point).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn rage_reward_amount(
    level: u32,
    damage: u32,
    attacker: bool,
    berserker_rage: bool,
    income_rate: f32,
) -> u32 {
    let level = level as f32;
    let conversion = 0.009_110_783 * level * level + 3.225_598 * level + 4.265_291;
    let mut added = damage as f32 / conversion * if attacker { 7.5 } else { 2.5 };
    if berserker_rage {
        added *= 1.3;
    }
    added *= income_rate;
    if added <= 0.0 {
        0
    } else {
        (added * 10.0) as u32
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
    use super::{next_delay_at_damage_ms, rage_reward_amount};

    #[test]
    fn rage_reward_formula_matches_fork() {
        assert_eq!(rage_reward_amount(19, 20, true, false, 1.0), 21);
        assert_eq!(rage_reward_amount(19, 23, false, false, 1.0), 8);
        assert_eq!(rage_reward_amount(19, 23, false, true, 1.0), 10);
        assert_eq!(rage_reward_amount(60, 1000, true, false, 1.0), 325);
        assert_eq!(rage_reward_amount(60, 1000, false, true, 1.0), 140);
        assert_eq!(rage_reward_amount(19, 20, true, false, 2.0), 43);
        assert_eq!(rage_reward_amount(19, 0, true, false, 1.0), 0);
    }

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
