//! Player regeneration: `Player::RegenerateAll` +
//! `Player::Regenerate` + `Player::RegenerateHealth`
//! (`Player.cpp:2310-2448`) with `UpdateManaRegen`
//! (`StatSystem.cpp:633-653`), driven from the players
//! gate in `Map::tick` at the fork's
//! `MapUpdate.UpdatePlayersDiff` cadence
//! (`Map::UpdatePlayers` -> `Player::Update`,
//! `Map.cpp:888-919`, `Player.cpp:1236-1245`).
//!
//! Rates are the fork config defaults 1.0
//! (`Rate.Health`, `Rate.Mana`, `Rate.Rage.Loss`,
//! `Rate.Energy`; `World.cpp:471-478`): the world
//! config stage does not parse `Rate.*` yet.
//!
//! Recorded deviations: `m_modManaRegen` /
//! `m_modManaRegenInterrupt` are computed live at the
//! tick instead of cached by stat changes (identical
//! values — inputs are the current spirit and auras);
//! `IsPolymorphed` reads the active TRANSFORM holder
//! instead of `m_transform` state, which the aura
//! engine does not track (equal unless two transforms
//! overlap; the fork's last-applied-wins bookkeeping
//! joins TRANSFORM aura handling); `MOD_REGEN` food
//! auras with a zero periodic time are skipped where
//! the fork divides by zero (no such spell data).

use mangos_entities::aura::AuraHolder;
use mangos_entities::player::Player;
use mangos_entities::unit::{POWER_ENERGY, POWER_MANA, POWER_RAGE};
use mangos_guid::ObjectGuid;
use mangos_spells::defines::aura as aura_type;

use crate::map::Map;

/// `REGEN_TIME_PLAYER_FULL` (`UnitDefines.h:65`).
const REGEN_TIME_PLAYER_FULL_MS: i32 = 2000;
/// `Rate.Health` default (`World.cpp:471`).
const RATE_HEALTH: f32 = 1.0;
/// `Rate.Mana` default (`World.cpp:472`).
const RATE_POWER_MANA: f32 = 1.0;
/// `Rate.Rage.Loss` default (`World.cpp:474`).
const RATE_POWER_RAGE_LOSS: f32 = 1.0;
/// `Rate.Energy` default (`World.cpp:478`).
const RATE_POWER_ENERGY: f32 = 1.0;
/// `SPELLFAMILY_MAGE` (`DBCEnums.h:197`).
const SPELLFAMILY_MAGE: u32 = 3;
/// `SPELL_PREVENTION_TYPE_SILENCE`.
const PREVENTION_TYPE_SILENCE: u32 = 1;

/// `HasAuraType` over applied holders.
fn has_aura_type(auras: &[AuraHolder], wanted: u32) -> bool {
    auras
        .iter()
        .any(|h| h.effects.iter().flatten().any(|e| e.aura_type == wanted))
}

/// `GetTotalAuraModifier` (`Unit.cpp:3264-3273`): raw
/// `m_amount` sum (no stack multiplier, as in C++).
fn total_aura_modifier(auras: &[AuraHolder], wanted: u32) -> i32 {
    let mut modifier = 0i32;
    for effect in auras.iter().flat_map(|h| h.effects.iter().flatten()) {
        if effect.aura_type == wanted {
            modifier += effect.amount;
        }
    }
    modifier
}

/// `GetTotalAuraModifierByMiscValue`
/// (`Unit.cpp:3292-3304`).
fn total_aura_modifier_by_misc(auras: &[AuraHolder], wanted: u32, misc: i32) -> i32 {
    let mut modifier = 0i32;
    for effect in auras.iter().flat_map(|h| h.effects.iter().flatten()) {
        if effect.aura_type == wanted && effect.misc_value == misc {
            modifier += effect.amount;
        }
    }
    modifier
}

/// `GetTotalAuraMultiplierByMiscValue`
/// (`Unit.cpp:3306-3319`).
fn total_aura_multiplier_by_misc(auras: &[AuraHolder], wanted: u32, misc: i32) -> f32 {
    let mut multiplier = 1.0f32;
    for effect in auras.iter().flat_map(|h| h.effects.iter().flatten()) {
        if effect.aura_type == wanted && effect.misc_value == misc {
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let factor = (100 + effect.amount) as f32;
            multiplier *= factor / 100.0;
        }
    }
    multiplier
}

impl Map {
    /// `UpdatePlayers` subset: one `Player::Update`
    /// regen pass per player with the accumulated
    /// players-gate diff.
    pub(crate) fn update_players(&mut self, diff_ms: u32) {
        let guids: Vec<ObjectGuid> = self.players.keys().copied().collect();
        for guid in guids {
            self.update_player_regen(guid, diff_ms);
            self.update_player_rest(guid);
        }
    }

    /// `Spells::GetSpellSpecific(GetTransForm()) ==
    /// SPELL_MAGE_POLYMORPH` reduced to the active
    /// TRANSFORM holder whose spell classifies as mage
    /// polymorph (`SpellEntry.cpp:68-74`: family mage,
    /// effect 0 aura `MOD_CONFUSE`, prevention silence).
    fn player_is_polymorphed(&self, guid: ObjectGuid) -> bool {
        let Some(entry) = self.players.get(&guid) else {
            return false;
        };
        entry.player.unit.auras.iter().any(|holder| {
            let transforms = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::TRANSFORM);
            if !transforms {
                return false;
            }
            self.stores
                .spells
                .get(holder.spell_id)
                .is_some_and(|spell| {
                    spell.spell_family_name == SPELLFAMILY_MAGE
                        && spell.effects[0].apply_aura_name == aura_type::MOD_CONFUSE
                        && spell.prevention_type == PREVENTION_TYPE_SILENCE
                })
        })
    }

    /// The regen arm of `Player::Update`
    /// (`Player.cpp:1236-1245`) plus the
    /// `Unit::Update` five-second-rule countdown that
    /// precedes it.
    /// Online rest grant (`Player::Update`,
    /// `Player.cpp:1195-1204`): while the resting flag is
    /// set and an inn-enter time exists, every full
    /// 10-second window since rest (re)started grants
    /// `ComputeRest` for the whole elapsed seconds and
    /// re-anchors the enter time to now.
    fn update_player_rest(&mut self, guid: ObjectGuid) {
        let now_ms = u64::from(self.now_ms);
        let max_level = self.stores.player_infos.max_player_level();
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let player = &mut entry.player;
        let Some(entered_ms) = player.inn_enter_ms else {
            return;
        };
        let elapsed_s = now_ms.saturating_sub(entered_ms) / 1000;
        if elapsed_s < 10 {
            return;
        }
        let next_xp = player
            .unit
            .world
            .object
            .values()
            .get_u32(mangos_protocol::update_field_table::offsets::PLAYER_NEXT_LEVEL_XP);
        let gain = mangos_entities::player::compute_rest(elapsed_s, next_xp, false, true);
        let bonus = player.rest_bonus + gain;
        player.apply_rest_bonus(bonus, max_level);
        player.inn_enter_ms = Some(now_ms);
    }

    fn update_player_regen(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let polymorphed = self.player_is_polymorphed(guid);
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let player = &mut entry.player;
        player.unit.update_last_mana_use(diff_ms);
        if !player.unit.is_alive() {
            player.regen_timer_ms = 0;
            return;
        }
        let diff = i32::try_from(diff_ms).unwrap_or(i32::MAX);
        player.regen_timer_ms = player.regen_timer_ms.saturating_sub(diff);
        if player.regen_timer_ms > 0 {
            return;
        }
        regenerate_all(player, polymorphed);
        player.regen_timer_ms += REGEN_TIME_PLAYER_FULL_MS;
    }
}

/// `Player::RegenerateAll` (`Player.cpp:2310-2327`);
/// the timer gate lives in the caller as in C++.
fn regenerate_all(player: &mut Player, polymorphed: bool) {
    let in_combat = player.unit.is_in_combat();
    let auras = &player.unit.auras;
    // Not in combat or they have regeneration
    if !in_combat
        || has_aura_type(auras, aura_type::MOD_REGEN_DURING_COMBAT)
        || has_aura_type(auras, aura_type::MOD_HEALTH_REGEN_IN_COMBAT)
        || polymorphed
    {
        regenerate_health(player, polymorphed);
        if !player.unit.is_in_combat()
            && !has_aura_type(&player.unit.auras, aura_type::INTERRUPT_REGEN)
        {
            regenerate_power(player, POWER_RAGE);
        }
    }

    regenerate_power(player, POWER_ENERGY);
    regenerate_power(player, POWER_MANA);
}

/// `Player::Regenerate` (`Player.cpp:2329-2395`).
fn regenerate_power(player: &mut Player, power: usize) {
    let unit = &mut player.unit;
    let mut cur_value = unit.power(power);
    let max_value = unit.max_power(power);
    let mut add_value = 0.0f32;

    match power {
        POWER_MANA => {
            // `Player::UpdateManaRegen` live
            // (`StatSystem.cpp:633-653`).
            let mut power_regen = unit.regen_mp_per_spirit();
            power_regen *=
                total_aura_multiplier_by_misc(&unit.auras, aura_type::MOD_POWER_REGEN_PERCENT, 0);
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let power_regen_mp5 =
                total_aura_modifier_by_misc(&unit.auras, aura_type::MOD_POWER_REGEN, 0) as f32
                    / 5.0;
            let mut mod_mana_regen_interrupt_pct =
                total_aura_modifier(&unit.auras, aura_type::MOD_MANA_REGEN_INTERRUPT);
            if mod_mana_regen_interrupt_pct > 100 {
                mod_mana_regen_interrupt_pct = 100;
            }
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let pct = mod_mana_regen_interrupt_pct as f32;
            let mod_mana_regen_interrupt = power_regen_mp5 + power_regen * pct / 100.0;
            let mod_mana_regen = power_regen_mp5 + power_regen;

            if unit.is_under_last_mana_use_effect() {
                add_value = mod_mana_regen_interrupt * RATE_POWER_MANA * 2.0;
            } else {
                add_value = mod_mana_regen * RATE_POWER_MANA * 2.0;
            }
        }
        // 20 = 2 rage by tick (units are tenths).
        POWER_RAGE => add_value = 20.0 * RATE_POWER_RAGE_LOSS,
        POWER_ENERGY => add_value = 20.0 * RATE_POWER_ENERGY,
        _ => {}
    }

    // `SPELL_AURA_MOD_POWER_REGEN_PERCENT` per-aura
    // multipliers (mana folds its own in above).
    if power != POWER_MANA {
        let misc = i32::try_from(power).unwrap_or(i32::MAX);
        for effect in unit.auras.iter().flat_map(|h| h.effects.iter().flatten()) {
            if effect.aura_type == aura_type::MOD_POWER_REGEN_PERCENT && effect.misc_value == misc {
                #[allow(clippy::cast_precision_loss)] // C++ converts identically
                let factor = (effect.amount + 100) as f32;
                add_value *= factor / 100.0;
            }
        }
    }

    // C++ `uint32(addValue)` truncates toward zero;
    // the Rust cast saturates where C++ is undefined
    // for negatives (unreachable for well-formed data:
    // regen amounts are non-negative).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let add_u32 = add_value as u32;
    if power != POWER_RAGE {
        cur_value = cur_value.saturating_add(add_u32);
        if cur_value > max_value {
            cur_value = max_value;
        }
    } else if cur_value <= add_u32 {
        cur_value = 0;
    } else {
        cur_value -= add_u32;
    }
    unit.set_power(power, cur_value);
}

/// `Player::RegenerateHealth` (`Player.cpp:2397-2448`).
fn regenerate_health(player: &mut Player, polymorphed: bool) {
    let unit = &mut player.unit;
    let cur_value = unit.health();
    let max_value = unit.max_health();
    if cur_value >= max_value {
        return;
    }

    let mut add_value = 0.0f32;
    let in_combat = unit.is_in_combat();
    let auras = &unit.auras;

    if polymorphed {
        #[allow(clippy::cast_precision_loss)] // C++ converts identically
        let max = max_value as f32;
        add_value = max / 10.0;
    } else if !in_combat || has_aura_type(auras, aura_type::MOD_REGEN_DURING_COMBAT) {
        add_value = unit.regen_hp_per_spirit() * RATE_HEALTH;
        if !in_combat {
            for effect in auras.iter().flat_map(|h| h.effects.iter().flatten()) {
                if effect.aura_type == aura_type::MOD_HEALTH_REGEN_PERCENT {
                    #[allow(clippy::cast_precision_loss)] // C++ converts identically
                    let amount = effect.amount as f32;
                    add_value *= (100.0 + amount) / 100.0;
                }
            }
        } else if has_aura_type(auras, aura_type::MOD_REGEN_DURING_COMBAT) {
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let total = total_aura_modifier(auras, aura_type::MOD_REGEN_DURING_COMBAT) as f32;
            add_value *= total / 100.0;
        }

        if !unit.is_standing_up() {
            add_value *= 1.5;
        }

        // Food.
        if !in_combat {
            for effect in auras.iter().flat_map(|h| h.effects.iter().flatten()) {
                if effect.aura_type == aura_type::MOD_REGEN && effect.period_ms > 0 {
                    #[allow(clippy::cast_precision_loss)] // C++ converts identically
                    let amount = effect.amount as f32;
                    #[allow(clippy::cast_precision_loss)] // C++ converts identically
                    let period = effect.period_ms as f32;
                    add_value += amount * (f32::from(2000u16) / period);
                }
            }
        }
    }

    // Always-regeneration bonus (including combat).
    #[allow(clippy::cast_precision_loss)] // C++ converts identically
    let in_combat_bonus = total_aura_modifier(auras, aura_type::MOD_HEALTH_REGEN_IN_COMBAT) as f32;
    add_value += RATE_HEALTH * 2.0 * (in_combat_bonus / 5.0);

    // Health fractions carry to the next tick.
    add_value += player.carry_health_regen;
    #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
    let whole = add_value as i32;
    #[allow(clippy::cast_precision_loss)] // C++ converts identically
    let whole_f = whole as f32;
    player.carry_health_regen = add_value - whole_f;

    if add_value < 0.0 {
        add_value = 0.0;
    }

    // `ModifyHealth(int32(addValue))`, clamped at max
    // (regen never reduces after the zero clamp).
    #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
    let add = add_value as i32;
    if add > 0 {
        #[allow(clippy::cast_sign_loss)] // guarded positive above
        let add_u32 = add as u32;
        unit.set_health(cur_value.saturating_add(add_u32).min(max_value));
    }
}

#[cfg(test)]
mod tests {
    use super::{total_aura_modifier, total_aura_multiplier_by_misc};
    use mangos_entities::aura::{AuraEffectState, AuraHolder};
    use mangos_guid::ObjectGuid;
    use mangos_spells::defines::aura as aura_type;

    fn holder(effect: AuraEffectState) -> AuraHolder {
        let mut holder = AuraHolder::new(1, ObjectGuid::default(), 1, 1000, true, true);
        holder.effects[0] = Some(effect);
        holder
    }

    fn effect(aura_type: u32, amount: i32, misc_value: i32) -> AuraEffectState {
        AuraEffectState {
            aura_type,
            amount,
            base_points: amount,
            period_ms: 0,
            periodic_timer: 0,
            periodic_ticks: 0,
            misc_value,
        }
    }

    #[test]
    fn totals_sum_raw_amounts_ignoring_stack() {
        let mut stacked = holder(effect(aura_type::MOD_POWER_REGEN, 10, 0));
        stacked.stack = 3;
        let auras = vec![
            stacked,
            holder(effect(aura_type::MOD_POWER_REGEN, -4, 0)),
            holder(effect(aura_type::MOD_STAT, 99, 0)),
        ];
        assert_eq!(total_aura_modifier(&auras, aura_type::MOD_POWER_REGEN), 6);
    }

    #[test]
    fn multiplier_folds_per_aura_percent() {
        let auras = vec![
            holder(effect(aura_type::MOD_POWER_REGEN_PERCENT, 50, 0)),
            holder(effect(aura_type::MOD_POWER_REGEN_PERCENT, 100, 0)),
            holder(effect(aura_type::MOD_POWER_REGEN_PERCENT, 100, 1)),
        ];
        let got = total_aura_multiplier_by_misc(&auras, aura_type::MOD_POWER_REGEN_PERCENT, 0);
        assert!((got - 3.0).abs() < 1e-6, "{got}");
    }
}
