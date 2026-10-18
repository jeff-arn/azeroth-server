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
    /// combo target check beyond the points themselves,
    /// no creature-level scaling (`custom` scaling
    /// attribute spells are out of scope and will read
    /// low — recorded). Returns the float value; C++
    /// callers truncate for aura amounts.
    #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
    pub fn calculate_effect_value(
        &self,
        effect_index: usize,
        caster_level: u32,
        combo_points: u8,
        rng: &mut dyn SpellRng,
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
        let mut value = eff.base_points as f32;
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
}

/// All loaded spells by id.
#[derive(Clone, Debug, Default)]
pub struct SpellStore {
    pub spells: HashMap<u32, SpellEntry>,
    pub tables: SpellDbcTables,
}

impl SpellStore {
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
