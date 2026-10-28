//! `SpellEntry` subset + the deterministic spell math
//! (`CalculateSpellEffectValue`, `GetCastTime`,
//! `GetDuration`/`CalculateDuration`, `CalculatePowerCost`,
//! `CalcArmorReducedDamage`, `ditheru`) ported from
//! `SpellCaster.cpp` / `SpellEntry.cpp` / `Spell.cpp`.
//! Spell mods, scripts, bonus coefficients and creature
//! level scaling are out of the Slice 5 subset (the
//! no-aura/no-spellpower reduction of
//! `SpellDamageBonusDone/Taken` is the identity, which
//! the map layer relies on and records).

use std::collections::HashMap;

/// Dice/rounding source (C++ `irand`/`frand` semantics;
/// values compatible, streams not bit-identical).
pub trait SpellRng {
    /// Inclusive `irand(min, max)`.
    fn irand(&mut self, min: i32, max: i32) -> i32;
    /// `frand(0, 1)` for `ditheru`.
    fn frand01(&mut self) -> f32;
}

/// One effect slot of a spell (indices 0..3 in C++).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpellEffectData {
    /// `Effect[]` (fields[62]).
    pub effect: u32,
    /// `EffectDieSides[]` (fields[65]).
    pub die_sides: i32,
    /// `EffectBaseDice[]` (fields[68]).
    pub base_dice: u32,
    /// `EffectDicePerLevel[]` (fields[71]).
    pub dice_per_level: f32,
    /// `EffectRealPointsPerLevel[]` (fields[74]).
    pub real_points_per_level: f32,
    /// `EffectBasePoints[]` (fields[77]).
    pub base_points: i32,
    /// `EffectBonusCoefficient[]` (fields[80]).
    pub bonus_coefficient: f32,
    /// `EffectMechanic[]` (fields[83]).
    pub mechanic: u32,
    /// `EffectImplicitTargetA[]` (fields[86]).
    pub implicit_target_a: u32,
    /// `EffectImplicitTargetB[]` (fields[89]).
    pub implicit_target_b: u32,
    /// `EffectRadiusIndex[]` (fields[92]).
    pub radius_index: u32,
    /// `EffectApplyAuraName[]` (fields[95]).
    pub apply_aura_name: u32,
    /// `EffectAmplitude[]` (fields[98]), with the
    /// load-time 5000 default already applied.
    pub amplitude: u32,
    /// `EffectMultipleValue[]` (fields[101]).
    pub multiple_value: f32,
    /// `EffectChainTarget[]` (fields[104]).
    pub chain_target: u32,
    /// `EffectItemType[]` (fields[107]); for modifier
    /// auras this is the spell-family affect mask
    /// (`SpellMgr::GetSpellAffectMask`).
    pub item_type: u64,
    /// `EffectMiscValue[]` (fields[110]).
    pub misc_value: i32,
    /// `EffectTriggerSpell[]` (fields[113]).
    pub trigger_spell: u32,
    /// `EffectPointsPerComboPoint[]` (fields[116]).
    pub points_per_combo_point: f32,
    /// `DmgMultiplier[]` (fields[143]).
    pub dmg_multiplier: f32,
}

/// `class SpellEntry` subset (`SpellMgr::LoadSpell`
/// field indices in doc comments).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellEntry {
    pub id: u32,
    /// fields[2] (`SpellSchools`).
    pub school: u32,
    /// fields[3].
    pub category: u32,
    /// fields[5].
    pub dispel: u32,
    /// fields[6].
    pub mechanic: u32,
    /// fields[7..12].
    pub attributes: u32,
    pub attributes_ex: u32,
    pub attributes_ex2: u32,
    pub attributes_ex3: u32,
    pub attributes_ex4: u32,
    /// fields[14] (target mask bits).
    pub targets: u32,
    /// fields[19].
    pub casting_time_index: u32,
    /// fields[20].
    pub recovery_time: u32,
    /// fields[21].
    pub category_recovery_time: u32,
    /// fields[22..25] (`SpellMgr.cpp:3801-3803`).
    pub interrupt_flags: u32,
    pub aura_interrupt_flags: u32,
    pub channel_interrupt_flags: u32,
    /// fields[25..28].
    pub proc_flags: u32,
    pub proc_chance: u32,
    pub proc_charges: u32,
    /// fields[28..31].
    pub max_level: u32,
    pub base_level: u32,
    pub spell_level: u32,
    /// fields[31].
    pub duration_index: u32,
    /// fields[32].
    pub power_type: u32,
    /// fields[33..37].
    pub mana_cost: u32,
    pub mana_cost_per_level: u32,
    pub mana_per_second: u32,
    pub mana_per_second_per_level: u32,
    /// fields[37].
    pub range_index: u32,
    /// fields[38] (projectile speed; > 0 makes the spell
    /// delayed — unsupported in the slice).
    pub speed: f32,
    /// fields[40].
    pub stack_amount: u32,
    /// Effect slots 0..3.
    pub effects: [SpellEffectData; 3],
    /// fields[119].
    pub spell_visual: u32,
    /// fields[121] (`SpellIconID`; dummy-proc and
    /// proc-event icon gates key on it).
    pub spell_icon_id: u32,
    /// fields[132].
    pub mana_cost_percentage: u32,
    /// fields[133..135].
    pub start_recovery_category: u32,
    pub start_recovery_time: u32,
    /// fields[137..139].
    pub spell_family_name: u32,
    pub spell_family_flags: u64,
    /// fields[139].
    pub max_affected_targets: u32,
    /// fields[140].
    pub dmg_class: u32,
    /// fields[141].
    pub prevention_type: u32,
    /// fields[149].
    pub custom: u32,
    /// fields[124] (diagnostics only).
    pub name: String,
}

/// `SpellCastTimes.dbc` row (`niii`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CastTimeEntry {
    pub cast_time: i32,
    pub cast_time_per_level: i32,
    pub min_cast_time: i32,
}

/// `SpellDuration.dbc` row (`niii`):
/// (`Duration`, `DurationPerLevel`, `MaxDuration`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DurationEntry {
    pub duration: i32,
    pub duration_per_level: i32,
    pub max_duration: i32,
}

/// `SpellRange.dbc` row (`nff...`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeEntry {
    pub min: f32,
    pub max: f32,
}

/// The DBC aux tables the slice reads.
#[derive(Clone, Debug, Default)]
pub struct SpellDbcTables {
    pub cast_times: HashMap<u32, CastTimeEntry>,
    pub durations: HashMap<u32, DurationEntry>,
    pub ranges: HashMap<u32, RangeEntry>,
    /// `SpellRadius.dbc` first radius column.
    pub radii: HashMap<u32, f32>,
}

impl SpellEntry {
    /// `SpellEntry::GetDuration` (abs of column 0, -1
    /// stays -1).
    #[must_use]
    pub fn duration_ms(&self, tables: &SpellDbcTables) -> i32 {
        match tables.durations.get(&self.duration_index) {
            None => 0,
            Some(d) if d.duration == -1 => -1,
            Some(d) => d.duration.abs(),
        }
    }

    /// `SpellEntry::GetMaxDuration`.
    #[must_use]
    pub fn max_duration_ms(&self, tables: &SpellDbcTables) -> i32 {
        match tables.durations.get(&self.duration_index) {
            None => 0,
            Some(d) if d.max_duration == -1 => -1,
            Some(d) => d.max_duration.abs(),
        }
    }

    /// `CalculateDuration` subset (no combo points, no
    /// spell mods, no aura script).
    #[must_use]
    pub fn calculate_duration_ms(&self, tables: &SpellDbcTables) -> i32 {
        self.duration_ms(tables)
    }

    /// `SpellEntry::GetCastTime` subset: DBC row +
    /// per-level with `spellRank = 0` (no chain data in
    /// the slice — instant rows dominate the candidates),
    /// `MinCastTime` clamp, no cast-speed modifiers.
    #[must_use]
    pub fn cast_time_ms(&self, tables: &SpellDbcTables) -> u32 {
        let Some(row) = tables.cast_times.get(&self.casting_time_index) else {
            return 0;
        };
        // spellRank/5 == 0 in the subset.
        #[allow(clippy::cast_possible_wrap)]
        let mut cast_time = row.cast_time + row.cast_time_per_level * (-(self.base_level as i32));
        cast_time = cast_time.max(row.min_cast_time);
        if cast_time > 0 { cast_time as u32 } else { 0 }
    }

    /// `IsAreaEffectTarget` (`SpellEntry.h:374`) target set.
    #[must_use]
    pub fn is_area_effect_target(target: u32) -> bool {
        matches!(
            target,
            7 | 8 | 15 | 16 | 20 | 24 | 28 | 30 | 31 | 33 | 34 | 37 | 52 | 56 | 61
        )
    }

    /// `GetAuraMaxTicks` (`SpellEntry.cpp:752`).
    #[must_use]
    pub fn aura_max_ticks(&self, tables: &SpellDbcTables) -> u16 {
        use crate::defines::{aura as aura_type, effect};
        let mut dot_duration = self.duration_ms(tables);
        if dot_duration == 0 {
            return 1;
        }
        if dot_duration > 30000 {
            dot_duration = 30000;
        }
        for eff in &self.effects {
            if eff.effect == effect::APPLY_AURA
                && matches!(
                    eff.apply_aura_name,
                    aura_type::PERIODIC_DAMAGE
                        | aura_type::PERIODIC_HEAL
                        | aura_type::PERIODIC_LEECH
                )
            {
                if eff.amplitude != 0 {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        clippy::cast_sign_loss
                    )]
                    return (dot_duration / eff.amplitude as i32) as u16;
                }
                break;
            }
        }
        6
    }

    /// `GetCastTimeForBonus` (`SpellEntry.cpp:514`); `dot`
    /// is the fork `DamageEffectType == DOT` test.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn cast_time_for_bonus(&self, tables: &SpellDbcTables, dot: bool) -> u32 {
        use crate::defines::{aura as aura_type, effect};
        let channeled = self.is_channeled();
        let mut casting_time: u32 = if channeled {
            self.duration_ms(tables).max(0) as u32
        } else {
            self.cast_time_ms(tables)
        };
        casting_time = casting_time.clamp(1500, 7000);
        if dot && !channeled {
            casting_time = 3500;
        }
        let mut over_time = 0i32;
        let mut effects = 0u32;
        let mut direct_damage = false;
        let area_effect = self.effects.iter().any(|e| {
            Self::is_area_effect_target(e.implicit_target_a)
                || Self::is_area_effect_target(e.implicit_target_b)
        });
        for eff in &self.effects {
            match eff.effect {
                effect::SCHOOL_DAMAGE
                | effect::POWER_DRAIN
                | effect::HEALTH_LEECH
                | effect::ENVIRONMENTAL_DAMAGE
                | effect::POWER_BURN
                | effect::HEAL => direct_damage = true,
                effect::APPLY_AURA => match eff.apply_aura_name {
                    aura_type::PERIODIC_DAMAGE
                    | aura_type::PERIODIC_HEAL
                    | aura_type::PERIODIC_LEECH => {
                        let d = self.duration_ms(tables);
                        if d != 0 {
                            over_time = d;
                        }
                    }
                    aura_type::DUMMY | aura_type::MOD_DECREASE_SPEED => effects += 1,
                    aura_type::MOD_CONFUSE | aura_type::MOD_STUN | aura_type::MOD_ROOT => {
                        effects += 2;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if over_time > 0 && casting_time > 0 && direct_damage {
            let original = self.cast_time_ms(tables).clamp(1500, 7000);
            let pt_ot = (over_time as f32 / 15000.0)
                / ((over_time as f32 / 15000.0) + (original as f32 / 3500.0));
            if dot {
                casting_time = (casting_time as f32 * pt_ot) as u32;
            } else if pt_ot < 1.0 {
                casting_time = (casting_time as f32 * (1.0 - pt_ot)) as u32;
            } else {
                casting_time = 0;
            }
        }
        if area_effect {
            casting_time /= 2;
        }
        if self.effects.iter().any(|e| {
            e.effect == effect::HEALTH_LEECH
                || (e.effect == effect::APPLY_AURA
                    && e.apply_aura_name == aura_type::PERIODIC_LEECH)
        }) {
            casting_time /= 2;
        }
        for _ in 0..effects {
            casting_time = (casting_time as f32 * 0.95) as u32;
        }
        casting_time
    }

    /// `CalculateDefaultCoefficient` (`SpellEntry.cpp:619`).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn default_coefficient(&self, tables: &SpellDbcTables, dot: bool) -> f32 {
        let mut dot_factor = 1.0f32;
        if dot {
            if !self.is_channeled() {
                dot_factor = self.duration_ms(tables) as f32 / 15000.0;
            }
            let ticks = self.aura_max_ticks(tables);
            if ticks != 0 {
                dot_factor /= f32::from(ticks);
            }
        }
        (self.cast_time_for_bonus(tables, dot) as f32 / 3500.0) * dot_factor
    }

    /// `CalculateCustomCoefficient` (`SpellEntry.cpp:638`)
    /// subset: Seal of Command by id; shaman chain scaling
    /// needs the cast's target count (`chain_targets` <= 1
    /// leaves coeff unchanged; the fork also applies
    /// `SPELLMOD_EFFECT_PAST_FIRST` to the multiplier past
    /// the first target — caller passes the modded
    /// multiplier via `chain_multiplier` when known).
    /// Seal of Righteousness needs `SpellIconID`, which
    /// this entry subset does not carry — recorded.
    #[must_use]
    pub fn custom_coefficient(
        &self,
        coeff: f32,
        done_part: bool,
        chain_targets: u32,
        chain_multiplier: Option<f32>,
    ) -> f32 {
        const FAMILY_PALADIN: u32 = 10;
        const FAMILY_SHAMAN: u32 = 11;
        match self.spell_family_name {
            FAMILY_PALADIN if self.id == 20424 => {
                if done_part {
                    0.20
                } else {
                    0.29
                }
            }
            FAMILY_SHAMAN if self.spell_family_flags & 0x142 != 0 && chain_targets > 1 => {
                let multiplier = chain_multiplier.unwrap_or(self.effects[0].dmg_multiplier);
                let mut out = coeff;
                for _ in 1..chain_targets {
                    out *= multiplier;
                }
                out
            }
            _ => coeff,
        }
    }

    /// `SpellCaster::CalculateLevelPenalty`
    /// (`SpellCaster.cpp:1528`, Nostalrius formula).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn level_penalty(&self) -> f32 {
        let spell_level = self.spell_level;
        if spell_level == 0 || spell_level > 20 {
            return 1.0;
        }
        1.0 - ((20.0 - spell_level as f32) * 0.0375)
    }

    /// Max range from `SpellRange.dbc` (0 when the row is
    /// missing).
    #[must_use]
    pub fn range_max(&self, tables: &SpellDbcTables) -> f32 {
        tables.ranges.get(&self.range_index).map_or(0.0, |r| r.max)
    }

    /// `Spell::CalculatePowerCost` subset: flat +
    /// per-level (rank 0) + percentage of max power
    /// (health for `POWER_HEALTH`). No spell mods or
    /// cost auras. `SPELL_ATTR_EX_USE_ALL_MANA` handled
    /// by draining the full pool.
    #[must_use]
    pub fn power_cost(&self, caster_power: u32, caster_max_power: u32, caster_health: u32) -> u32 {
        const ATTR_EX_USE_ALL_MANA: u32 = 0x0000_0002;
        if self.attributes_ex & ATTR_EX_USE_ALL_MANA != 0 {
            return if self.power_type == crate::defines::power::HEALTH {
                caster_health
            } else {
                caster_power
            };
        }
        #[allow(clippy::cast_possible_wrap)]
        let mut cost: i32 =
            self.mana_cost as i32 + self.mana_cost_per_level as i32 * -(self.base_level as i32);
        if self.mana_cost_percentage != 0 {
            let base = if self.power_type == crate::defines::power::HEALTH {
                caster_health
            } else {
                caster_max_power
            };
            #[allow(clippy::cast_possible_wrap)]
            {
                cost += (base * self.mana_cost_percentage / 100) as i32;
            }
        }
        if cost < 0 { 0 } else { cost as u32 }
    }

    /// `SpellCaster::CalculateSpellEffectValue`
    /// (`SpellCaster.cpp:876`) subset: no spell mods, no
    /// combo target check beyond the points themselves;
    /// creature-level scaling lives in
    /// `creature_level_scaled` (needs the CLS store).
    /// Returns the float value; C++ callers truncate
    /// for aura amounts.
    #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
    pub fn calculate_effect_value(
        &self,
        effect_index: usize,
        caster_level: u32,
        combo_points: u8,
        rng: &mut dyn SpellRng,
    ) -> f32 {
        self.calculate_effect_value_bp(effect_index, caster_level, combo_points, rng, None)
    }

    /// As [`Self::calculate_effect_value`], with a
    /// custom base-points override (`effBasePoints`
    /// in `CalculateSpellEffectValue`: the value
    /// starts at `bp - baseDice`, then the usual
    /// level/dice/combo math runs).
    #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
    pub fn calculate_effect_value_bp(
        &self,
        effect_index: usize,
        caster_level: u32,
        combo_points: u8,
        rng: &mut dyn SpellRng,
        bp: Option<i32>,
    ) -> f32 {
        let eff = &self.effects[effect_index];
        #[allow(clippy::cast_possible_wrap)]
        let mut level = caster_level as i32;
        if level > self.max_level as i32 && self.max_level > 0 {
            level = self.max_level as i32;
        } else if level < self.base_level as i32 {
            level = self.base_level as i32;
        }
        level -= self.spell_level as i32;

        #[allow(clippy::cast_possible_wrap)]
        let base_dice = eff.base_dice as i32;
        #[allow(clippy::cast_precision_loss)]
        let mut value = bp.map_or(eff.base_points as f32, |bp| (bp - base_dice) as f32);
        value += level as f32 * eff.real_points_per_level;
        #[allow(clippy::cast_possible_truncation)]
        let random_points = (eff.die_sides as f32 + level as f32 * eff.dice_per_level) as i32;
        match random_points {
            0 | 1 => value += base_dice as f32,
            _ => {
                let rand = if base_dice >= random_points {
                    rng.irand(random_points, base_dice)
                } else {
                    rng.irand(base_dice, random_points)
                };
                value += rand as f32;
            }
        }
        if eff.points_per_combo_point != 0.0 {
            value += eff.points_per_combo_point * f32::from(combo_points);
        }
        value
    }

    /// `SPELL_ATTR_SCALES_WITH_CREATURE_LEVEL` tail of
    /// `CalculateSpellEffectValue`: rescale by the
    /// class-1 `CreatureClassLevelStats.melee_damage`
    /// ratio between caster level and spell level.
    /// `cls_melee_damage(level)` supplies the store.
    #[must_use]
    pub fn creature_level_scaled(
        &self,
        effect_index: usize,
        value: f32,
        caster_level: u32,
        cls_melee_damage: &dyn Fn(u32) -> f32,
    ) -> f32 {
        use crate::defines::{aura as aura_type, effect};
        const ATTR_SCALES_WITH_CREATURE_LEVEL: u32 = 0x0008_0000;
        let eff = &self.effects[effect_index];
        if self.attributes & ATTR_SCALES_WITH_CREATURE_LEVEL == 0
            || self.spell_level == 0
            || eff.effect == effect::WEAPON_PERCENT_DAMAGE
            || eff.effect == effect::KNOCK_BACK
            || (eff.effect == effect::APPLY_AURA
                && eff.apply_aura_name == aura_type::MOD_DECREASE_SPEED)
        {
            return value;
        }
        let power_spell = cls_melee_damage(self.spell_level);
        if power_spell == 0.0 {
            return value;
        }
        value * (cls_melee_damage(caster_level) / power_spell)
    }

    /// `CalculateSimpleValue` (`SpellEntry.h:1074`):
    /// base points + base dice (sign/power comparisons).
    #[must_use]
    pub fn simple_value(&self, effect_index: usize) -> i32 {
        let eff = &self.effects[effect_index];
        #[allow(clippy::cast_possible_wrap)]
        {
            eff.base_points + eff.base_dice as i32
        }
    }
}

/// `ditheru` (`Util.cpp:582`): stochastic rounding for
/// non-negative damage (`copysign(floor(abs(v)+frand), v)`).
pub fn ditheru(v: f32, rng: &mut dyn SpellRng) -> u32 {
    let magnitude = (v.abs() + rng.frand01()).floor();
    if magnitude <= 0.0 {
        0
    } else {
        #[allow(clippy::cast_possible_truncation)]
        {
            magnitude as u32
        }
    }
}

/// `dither` (`Util.cpp`): signed stochastic rounding
/// (`copysign(floor(abs(v)+frand), v)` truncated).
pub fn dither(v: f32, rng: &mut dyn SpellRng) -> i32 {
    let magnitude = (v.abs() + rng.frand01()).floor();
    #[allow(clippy::cast_possible_truncation)]
    let m = magnitude as i32;
    if v < 0.0 { -m } else { m }
}

/// `CalcArmorReducedDamage` (`SpellCaster.cpp:850`)
/// without target-resistance auras. Note the C++
/// signature takes `uint32 damage` (already truncated)
/// and returns float; minimum 1.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn armor_reduced_damage(damage: f32, victim_armor: i32, attacker_level: u32) -> f32 {
    let armor = (victim_armor as f32).max(0.0);
    let mut tmp = 0.1 * armor / (8.5 * attacker_level as f32 + 40.0);
    tmp /= 1.0 + tmp;
    tmp = tmp.clamp(0.0, 0.75);
    let new_damage = damage - damage * tmp;
    if new_damage > 1.0 { new_damage } else { 1.0 }
}

/// The `Player::ApplySpellMod` composition
/// (`Player.cpp:22510`): flat sums add to the base,
/// then the percent sum scales the sum —
/// `base + ((base + flat) * pct / 100 + flat)`.
#[must_use]
pub fn spell_mod_apply(base: f32, flat: f32, pct: f32) -> f32 {
    base + ((base + flat) * pct / 100.0 + flat)
}

/// `Spells::IsPositiveTarget` (`SpellEntry.h:210`).
#[must_use]
pub fn is_positive_target(target_a: u32, target_b: u32) -> bool {
    use crate::defines::target as t;
    match target_a {
        // UNIT_ENEMY, ENUM enemy AoE (src/dest/cone/
        // dynobj), LOCATION_CASTER_TARGET_POSITION.
        t::UNIT_ENEMY | 15 | 16 | 24 | 28 | 53 => return false,
        // LOCATION_CASTER_SRC: positive only for party/
        // friend/script AoE in B.
        22 => return matches!(target_b, 33 | 30 | 7),
        _ => {}
    }
    if target_b != 0 {
        return is_positive_target(target_b, 0);
    }
    true
}

impl SpellEntry {
    /// `IsPositiveEffect` subset (`SpellEntry.cpp:795`):
    /// custom bits, the aura-type arms the slice can
    /// produce, then the target-mode fallback and the
    /// debuff attribute. Family/dummy/script arms are
    /// out of scope (recorded).
    #[must_use]
    pub fn is_positive_effect(&self, effect_index: usize) -> bool {
        use crate::defines::{aura, effect};
        const CUSTOM_NEGATIVE: u32 = 0x002;
        const CUSTOM_POSITIVE: u32 = 0x004;
        const ATTR_AURA_IS_DEBUFF: u32 = 0x0400_0000;
        if self.custom & CUSTOM_POSITIVE != 0 {
            return true;
        }
        if self.custom & CUSTOM_NEGATIVE != 0 {
            return false;
        }
        let eff = &self.effects[effect_index];
        match eff.effect {
            effect::HEAL => return true,
            effect::APPLY_AURA => match eff.apply_aura_name {
                aura::MOD_STAT => {
                    if self.simple_value(effect_index) < 0 {
                        return false;
                    }
                }
                aura::MOD_INCREASE_HEALTH => return true,
                aura::MOD_STUN => {
                    if effect_index == 0
                        && self.effects[1].effect == 0
                        && self.effects[2].effect == 0
                    {
                        return false;
                    }
                }
                aura::MOD_ROOT | aura::PERIODIC_DAMAGE_PERCENT => return false,
                aura::PERIODIC_DAMAGE
                    if eff.implicit_target_a == crate::defines::target::UNIT_CASTER =>
                {
                    return false;
                }
                _ => {}
            },
            _ => {}
        }
        if !is_positive_target(eff.implicit_target_a, eff.implicit_target_b) {
            return false;
        }
        if self.attributes & ATTR_AURA_IS_DEBUFF != 0 {
            return false;
        }
        true
    }

    /// Holder positivity: positive iff every applied
    /// effect is positive (`SpellAuraHolder::IsPositive`
    /// over `Aura::IsPositive`).
    #[must_use]
    pub fn is_positive_spell(&self) -> bool {
        (0..3).all(|i| self.effects[i].effect == 0 || self.is_positive_effect(i))
    }

    /// `IsChanneledSpell` (`SpellEntry.h:1071`):
    /// `SPELL_ATTR_EX_IS_CHANNELED` (0x04) or
    /// `SPELL_ATTR_EX_IS_SELF_CHANNELED` (0x40).
    #[must_use]
    pub fn is_channeled(&self) -> bool {
        self.attributes_ex & (0x0000_0004 | 0x0000_0040) != 0
    }
}

/// All loaded spells by id.
#[derive(Clone, Debug, Default)]
pub struct SpellStore {
    pub spells: HashMap<u32, SpellEntry>,
    pub tables: SpellDbcTables,
    /// `spell_proc_event` rows by aura spell id
    /// (rank-filled at load).
    pub proc_events: HashMap<u32, ProcEventEntry>,
    /// `spell_cone` arcs in radians
    /// (`mSpellCones`; negative = back cone).
    pub cones: HashMap<u32, f32>,
}

/// One `spell_proc_event` row (`SpellProcEventEntry`,
/// `SpellMgr.h:43`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcEventEntry {
    pub school_mask: u32,
    pub spell_family_name: u32,
    pub spell_family_mask: [u64; 3],
    pub proc_flags: u32,
    pub proc_ex: u32,
    pub ppm_rate: f32,
    pub custom_chance: f32,
    /// Hidden cooldown applied to the *triggered*
    /// spell (ms).
    pub cooldown: u32,
}

impl SpellStore {
    /// `GetSpellCone`: table arc or the 60-degree
    /// default, in radians.
    #[must_use]
    pub fn cone_angle(&self, id: u32) -> f32 {
        self.cones
            .get(&id)
            .copied()
            .unwrap_or(60.0 * std::f32::consts::PI / 180.0)
    }

    #[must_use]
    pub fn get(&self, id: u32) -> Option<&SpellEntry> {
        self.spells.get(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.spells.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{SpellDbcTables, SpellEffectData, SpellEntry, spell_mod_apply};
    use crate::defines::{aura as aura_type, effect};

    fn tables_with(cast_ms: i32, duration_ms: i32) -> SpellDbcTables {
        let mut tables = SpellDbcTables::default();
        tables.cast_times.insert(
            1,
            crate::entry::CastTimeEntry {
                cast_time: cast_ms,
                cast_time_per_level: 0,
                min_cast_time: 0,
            },
        );
        tables.durations.insert(
            1,
            crate::entry::DurationEntry {
                duration: duration_ms,
                duration_per_level: 0,
                max_duration: duration_ms,
            },
        );
        tables
    }

    #[allow(clippy::large_types_passed_by_value)]
    fn entry_with(effects: [SpellEffectData; 3]) -> SpellEntry {
        SpellEntry {
            casting_time_index: 1,
            duration_index: 1,
            spell_level: 60,
            effects,
            ..SpellEntry::default()
        }
    }

    #[test]
    fn coefficient_math_matches_fork_formulas() {
        // Direct 3.5s cast: coeff = 3500/3500 = 1.0.
        let e = entry_with(
            [SpellEffectData {
                effect: effect::SCHOOL_DAMAGE,
                ..SpellEffectData::default()
            }; 3],
        );
        let t = tables_with(3500, 0);
        assert!((e.default_coefficient(&t, false) - 1.0).abs() < 1e-6);
        // Instant direct clamps to 1500/3500.
        let t0 = tables_with(0, 0);
        assert!((e.default_coefficient(&t0, false) - 1500.0 / 3500.0).abs() < 1e-6);
        // Area target halves.
        let mut area = e.clone();
        area.effects[0].implicit_target_a = 15;
        assert!((area.default_coefficient(&t, false) - 0.5).abs() < 1e-6);

        // DoT 15s, 3s ticks (5 ticks), instant cast:
        // cast-for-bonus 3500, DotFactor 15000/15000/5=0.2.
        let dot = entry_with(
            [SpellEffectData {
                effect: effect::APPLY_AURA,
                apply_aura_name: aura_type::PERIODIC_DAMAGE,
                amplitude: 3000,
                ..SpellEffectData::default()
            }; 3],
        );
        let td = tables_with(0, 15000);
        assert_eq!(dot.aura_max_ticks(&td), 5);
        assert!((dot.default_coefficient(&td, true) - 0.2).abs() < 1e-6);

        // Level penalty: spellLevel 12 -> 1 - 8*0.0375 = 0.7;
        // 0 and >20 are penalty-free.
        let mut low = e.clone();
        low.spell_level = 12;
        assert!((low.level_penalty() - 0.7).abs() < 1e-6);
        low.spell_level = 0;
        assert!((low.level_penalty() - 1.0).abs() < 1e-6);

        // Seal of Command custom coefficient.
        let mut soc = e;
        soc.id = 20424;
        soc.spell_family_name = 10;
        assert!((soc.custom_coefficient(0.99, true, 1, None) - 0.20).abs() < 1e-6);
        assert!((soc.custom_coefficient(0.99, false, 1, None) - 0.29).abs() < 1e-6);
    }

    #[test]
    fn effect_value_scaling_matches_fork() {
        // Spell 6118 shape (audit exemplar): baseLevel
        // 22, spellLevel 99, maxLevel 27, base 59,
        // +1/level — fork yields a negative amount at
        // caster 19 (level clamps to 22, 22-99 = -77);
        // downstream fork guards clamp it. The Rust
        // formula must reproduce that exactly.
        struct ZeroRng;
        impl crate::entry::SpellRng for ZeroRng {
            fn irand(&mut self, min: i32, _max: i32) -> i32 {
                min
            }
            fn frand01(&mut self) -> f32 {
                0.0
            }
        }
        let mut e = entry_with(
            [SpellEffectData {
                effect: effect::APPLY_AURA,
                base_points: 59,
                base_dice: 1,
                die_sides: 1,
                real_points_per_level: 1.0,
                ..SpellEffectData::default()
            }; 3],
        );
        e.base_level = 22;
        e.spell_level = 99;
        e.max_level = 27;
        let v = e.calculate_effect_value(0, 19, 0, &mut ZeroRng);
        assert!((v - (59.0 - 77.0 + 1.0)).abs() < 1e-6, "value {v}");

        // SCALES_WITH_CREATURE_LEVEL ratio (class-1 CLS
        // melee damage 20 at spell level, 30 at caster).
        e.attributes = 0x0008_0000;
        e.spell_level = 20;
        let scaled =
            e.creature_level_scaled(0, 100.0, 30, &|lvl| if lvl == 20 { 20.0 } else { 30.0 });
        assert!((scaled - 150.0).abs() < 1e-6, "scaled {scaled}");
        // No attribute -> identity.
        e.attributes = 0;
        let same = e.creature_level_scaled(0, 100.0, 30, &|_| 99.0);
        assert!((same - 100.0).abs() < 1e-6);
    }

    #[test]
    fn custom_base_points_replace_base() {
        // `CalculateSpellEffectValue` with
        // effBasePoints: value starts at bp - baseDice,
        // then level/dice math (here dieSides 1 adds
        // baseDice back, so bp + level term).
        struct ZeroRng;
        impl crate::entry::SpellRng for ZeroRng {
            fn irand(&mut self, min: i32, _max: i32) -> i32 {
                min
            }
            fn frand01(&mut self) -> f32 {
                0.0
            }
        }
        let mut e = entry_with(
            [SpellEffectData {
                effect: effect::SCHOOL_DAMAGE,
                base_points: 10,
                base_dice: 1,
                die_sides: 1,
                real_points_per_level: 2.0,
                ..SpellEffectData::default()
            }; 3],
        );
        e.spell_level = 10;
        // No override: 10 + (20-10)*2 + 1 = 31.
        let plain = e.calculate_effect_value_bp(0, 20, 0, &mut ZeroRng, None);
        assert!((plain - 31.0).abs() < 1e-6, "plain {plain}");
        // bp 100: (100-1) + 20 + 1 = 120.
        let custom = e.calculate_effect_value_bp(0, 20, 0, &mut ZeroRng, Some(100));
        assert!((custom - 120.0).abs() < 1e-6, "custom {custom}");
        // bp 0 engaged (TriggerProccedSpell quirk):
        // (0-1) + 20 + 1 = 20.
        let zero = e.calculate_effect_value_bp(0, 20, 0, &mut ZeroRng, Some(0));
        assert!((zero - 20.0).abs() < 1e-6, "zero {zero}");
        // Signed dither keeps the sign.
        assert_eq!(super::dither(-2.5, &mut ZeroRng), -2);
        assert_eq!(super::dither(2.5, &mut ZeroRng), 2);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact binary fractions under test
    fn spell_mod_composition_matches_fork() {
        // Flat adds before percent scales the sum.
        assert_eq!(spell_mod_apply(100.0, 10.0, 20.0), 132.0);
        // Nature's Swiftness shape: -100% cast time.
        assert_eq!(spell_mod_apply(1500.0, 0.0, -100.0), 0.0);
        // Identity at no mods.
        assert_eq!(spell_mod_apply(42.0, 0.0, 0.0), 42.0);
    }
}
