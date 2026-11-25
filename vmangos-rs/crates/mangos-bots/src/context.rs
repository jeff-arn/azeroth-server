//! Tactical snapshot for one bot tick
//! (`BotSituationContext.h` `SituationContext`,
//! field-for-field; `Unit*` become GUIDs). The
//! host fills this once per tick
//! (`EvaluateSituation`) and behaviours score
//! against it — scoring sees only this snapshot,
//! exactly like the fork.

use mangos_guid::ObjectGuid;

use crate::patterns::LearnedPattern;
use crate::role::BotRole;

/// `SituationContext` (defaults mirror the C++
/// in-class initializers).
#[derive(Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "field-for-field mirror of the C++ SituationContext snapshot"
)]
pub struct SituationContext {
    // Rust-only identity (C++ reaches `me` via the AI).
    pub me: ObjectGuid,

    // === Self ===
    pub health_pct: f32,
    pub mana_pct: f32,
    pub rage_pct: f32,
    pub energy_pct: f32,
    /// `Powers` enum value.
    pub power_type: u8,
    pub is_being_meleed: bool,
    pub melee_attacker_count: u8,
    /// `ShapeshiftForm` byte.
    pub current_form: u8,
    pub combo_points: u8,
    pub player_class: u8,
    pub has_shield_equipped: bool,
    pub is_casting: bool,
    pub is_moving: bool,

    // === Threat ===
    pub threat_ratio: f32,
    pub should_throttle: bool,

    // === Target ===
    /// `me->GetVictim()` (melee victim, not the
    /// UI selection).
    pub current_target: Option<ObjectGuid>,
    pub target_health_pct: f32,
    pub distance_to_target: f32,
    pub target_is_casting: bool,
    pub target_cast_spell_id: u32,
    pub target_creature_entry: u32,
    pub target_is_player: bool,

    // === Party ===
    pub tank_health_pct: f32,
    pub tank_is_alive: bool,
    pub has_tank: bool,
    pub tank_unit: Option<ObjectGuid>,
    pub lowest_ally_health_pct: f32,
    pub most_injured_ally: Option<ObjectGuid>,
    pub alive_party_members: u8,
    pub dead_party_members: u8,
    /// Tank < 80% OR non-tank < 50%.
    pub any_ally_needs_heal: bool,
    /// Tank < 80%.
    pub tank_needs_heal: bool,
    /// Anyone (but self) < 30%.
    pub critical_heal_needed: bool,
    pub healer_available: bool,
    /// Un-CC'd mobs on the party that aren't the
    /// tank's target.
    pub has_loose_adds: bool,

    // === Encounter ===
    pub active_pattern: Option<LearnedPattern>,
    pub is_boss_encounter: bool,

    // === Role ===
    pub role: BotRole,

    // === Formation ===
    pub formation_target_x: f32,
    pub formation_target_y: f32,
    pub formation_target_z: f32,
    pub dist_to_formation_pos: f32,
    pub has_formation_target: bool,
    pub flee_target_x: f32,
    pub flee_target_y: f32,
    pub flee_target_z: f32,
    pub has_flee_target: bool,

    // Rust-only: in-combat for the debug line
    // (C++ reads `m_debugTarget->IsInCombat()`).
    pub in_combat: bool,
}

impl Default for SituationContext {
    fn default() -> Self {
        Self {
            me: ObjectGuid::EMPTY,
            health_pct: 100.0,
            mana_pct: 100.0,
            rage_pct: 0.0,
            energy_pct: 100.0,
            power_type: 0,
            is_being_meleed: false,
            melee_attacker_count: 0,
            current_form: 0,
            combo_points: 0,
            player_class: 0,
            has_shield_equipped: false,
            is_casting: false,
            is_moving: false,
            threat_ratio: 0.0,
            should_throttle: false,
            current_target: None,
            target_health_pct: 100.0,
            distance_to_target: 0.0,
            target_is_casting: false,
            target_cast_spell_id: 0,
            target_creature_entry: 0,
            target_is_player: false,
            tank_health_pct: 100.0,
            tank_is_alive: false,
            has_tank: false,
            tank_unit: None,
            lowest_ally_health_pct: 100.0,
            most_injured_ally: None,
            alive_party_members: 0,
            dead_party_members: 0,
            any_ally_needs_heal: false,
            tank_needs_heal: false,
            critical_heal_needed: false,
            healer_available: false,
            has_loose_adds: false,
            active_pattern: None,
            is_boss_encounter: false,
            role: BotRole::Invalid,
            formation_target_x: 0.0,
            formation_target_y: 0.0,
            formation_target_z: 0.0,
            dist_to_formation_pos: 0.0,
            has_formation_target: false,
            flee_target_x: 0.0,
            flee_target_y: 0.0,
            flee_target_z: 0.0,
            has_flee_target: false,
            in_combat: false,
        }
    }
}
