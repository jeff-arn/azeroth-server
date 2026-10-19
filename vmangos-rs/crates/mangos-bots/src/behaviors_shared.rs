//! Shared behaviours (`BotBehaviors_Shared.cpp`,
//! ported behaviour-for-behaviour; scores read
//! only the snapshot, executes go through the
//! host). Class ids: warrior 1, paladin 2,
//! hunter 3, rogue 4, priest 5, shaman 7, mage 8,
//! warlock 9, druid 11.

use mangos_guid::ObjectGuid;

use crate::behavior::BotBehavior;
use crate::context::SituationContext;
use crate::healing;
use crate::host::{BotHost, CastOutcome, MotionKind};
use crate::patterns::PatternResponse;
use crate::role::BotRole;
use crate::spell_kit::SpellKit;

/// Formation move deadzone (`BotFormationDefaults::MOVE_DEADZONE`).
const MOVE_DEADZONE: f32 = 2.0;
/// `PB_SPELL_SHOOT_WAND_ID`.
const SHOOT_WAND: u32 = 5019;

/// `IsCasterRole`.
#[must_use]
pub fn is_caster_role(role: BotRole, class_id: u8, form: u8) -> bool {
    if role == BotRole::Healer || role == BotRole::RangeDps {
        return true;
    }
    // Druid in caster/moonkin form (not cat/bear).
    class_id == 11 && !matches!(form, 0x01 | 0x05 | 0x08)
}

fn cast_ok(host: &mut dyn BotHost, target: Option<ObjectGuid>, spell: Option<u32>) -> bool {
    let Some(spell_id) = spell else {
        return false;
    };
    if !host.can_try_cast(target, spell_id) {
        return false;
    }
    host.cast_spell(target, spell_id) == CastOutcome::Ok
}

macro_rules! behavior {
    ($ty:ident, $name:literal) => {
        pub struct $ty;
        impl $ty {
            fn nm(&self) -> &'static str {
                $name
            }
        }
    };
}

// ============================================================
// 1. SelfPreserve
// ============================================================
behavior!(SelfPreserve, "SelfPreserve");
impl BotBehavior for SelfPreserve {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.health_pct > 20.0 {
            return 0.0;
        }
        90.0 + (20.0 - ctx.health_pct) * 0.25
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let me = host.me();
        cast_ok(host, Some(me), kit.emergency_ability)
    }
}

// ============================================================
// 2. Interrupt
// ============================================================
behavior!(Interrupt, "Interrupt");
impl BotBehavior for Interrupt {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if !ctx.target_is_casting {
            return 0.0;
        }
        if let Some(pattern) = &ctx.active_pattern {
            if pattern.response_type == PatternResponse::Interrupt {
                return 90.0;
            }
        }
        85.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(target) = ctx.current_target else {
            return false;
        };
        cast_ok(host, Some(target), kit.interrupt) || cast_ok(host, Some(target), kit.interrupt_alt)
    }
}

// ============================================================
// 3. Taunt
// ============================================================
behavior!(Taunt, "Taunt");
impl Taunt {
    fn loose_attackers(host: &dyn BotHost) -> Vec<ObjectGuid> {
        let me = host.me();
        let mut out = Vec::new();
        for member in host.party_members() {
            if member == me {
                continue;
            }
            if !host.unit(member).is_some_and(|v| v.is_alive) {
                continue;
            }
            for attacker in host.attackers_of(member) {
                if host.unit(attacker).and_then(|v| v.victim) == Some(member) {
                    out.push(attacker);
                }
            }
        }
        out
    }
}
impl BotBehavior for Taunt {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.role != BotRole::Tank {
            return 0.0;
        }
        let me = host.me();
        for member in host.party_members() {
            if member == me || !host.unit(member).is_some_and(|v| v.is_alive) {
                continue;
            }
            if !host.attackers_of(member).is_empty() {
                return 82.0;
            }
        }
        0.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let loose = Self::loose_attackers(host);
        let me = host.me();
        if loose.len() >= 2 && cast_ok(host, Some(me), kit.aoe_taunt) {
            return true;
        }
        match loose.first() {
            Some(&first) => cast_ok(host, Some(first), kit.taunt),
            None => false,
        }
    }
}

// ============================================================
// 4. HealParty
// ============================================================
behavior!(HealParty, "HealParty");
impl BotBehavior for HealParty {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if !ctx.any_ally_needs_heal && ctx.health_pct > 80.0 {
            return 0.0;
        }
        if ctx.critical_heal_needed {
            return 85.0;
        }
        if ctx.any_ally_needs_heal {
            return 78.0;
        }
        if ctx.health_pct < 60.0 {
            return 76.0;
        }
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let spells = &kit.spells;
        let Some(me) = host.unit(host.me()) else {
            return false;
        };
        if ctx.mana_pct < 30.0 {
            if let Some(target) = healing::select_heal_target(host, &me, 30.0, 30.0) {
                return healing::heal_direct(host, spells, target);
            }
            return false;
        }
        if ctx.mana_pct < 50.0 {
            if let Some(tank) = ctx.tank_unit {
                if ctx.tank_health_pct < 70.0
                    && !host.has_periodic_heal_aura(tank)
                    && healing::heal_periodic(host, spells, tank)
                {
                    return true;
                }
            }
            if let Some(target) = healing::select_heal_target(host, &me, 50.0, 40.0) {
                if healing::heal_direct(host, spells, target) {
                    return true;
                }
            }
            if let Some(tank) = ctx.tank_unit {
                if ctx.tank_health_pct < 70.0 {
                    return healing::heal_direct(host, spells, tank);
                }
            }
            return false;
        }
        if let Some(tank) = ctx.tank_unit {
            if ctx.tank_health_pct < 80.0
                && !host.has_periodic_heal_aura(tank)
                && healing::heal_periodic(host, spells, tank)
            {
                return true;
            }
            if ctx.tank_health_pct < 60.0 && healing::heal_direct(host, spells, tank) {
                return true;
            }
        }
        if let Some(target) = healing::select_heal_target(host, &me, 50.0, 50.0) {
            if healing::heal_direct(host, spells, target) {
                return true;
            }
        }
        healing::find_and_pre_heal(host, spells)
    }
}

// ============================================================
// 5. EmergencyHeal
// ============================================================
behavior!(EmergencyHeal, "EmergencyHeal");
impl BotBehavior for EmergencyHeal {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if !ctx.critical_heal_needed
            || !healing::is_healer_class(ctx.player_class)
            || ctx.role == BotRole::Healer
            || ctx.healer_available
        {
            return 0.0;
        }
        80.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        healing::find_and_heal_injured_ally(host, &kit.spells, 60.0, 30.0)
    }
}

// ============================================================
// 6. TankThreat
// ============================================================
behavior!(TankThreat, "TankThreat");
impl BotBehavior for TankThreat {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.role != BotRole::Tank || ctx.current_target.is_none() {
            return 0.0;
        }
        75.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        match ctx.current_target {
            Some(target) => cast_ok(host, Some(target), kit.filler),
            None => false,
        }
    }
}

// ============================================================
// 7. ThreatDump
// ============================================================
behavior!(ThreatDump, "ThreatDump");
impl BotBehavior for ThreatDump {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.role == BotRole::Tank || ctx.threat_ratio < 0.80 {
            return 0.0;
        }
        if !matches!(ctx.player_class, 4 | 5 | 11 | 3) {
            return 0.0;
        }
        (70.0 + (ctx.threat_ratio - 0.80) * 40.0).min(78.0)
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let target = ctx.current_target.or_else(|| Some(host.me()));
        cast_ok(host, target, kit.threat_dump)
    }
}

// ============================================================
// 8. CrowdControl
// ============================================================
behavior!(CrowdControl, "CrowdControl");
impl CrowdControl {
    fn control_marked(kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(spell_id) = kit.crowd_control else {
            return false;
        };
        for icon in host.cc_marks() {
            let Some(target) = host.marked_target(icon) else {
                continue;
            };
            if host.is_ccd(target) {
                continue;
            }
            if cast_ok(host, Some(target), Some(spell_id)) {
                host.attack_stop();
                return true;
            }
        }
        false
    }
    fn select_auto_target(kit: &SpellKit, host: &dyn BotHost) -> Option<ObjectGuid> {
        let spell_id = kit.crowd_control?;
        let me_view = host.unit(host.me())?;
        let tank_target = host
            .party_tank()
            .and_then(|t| host.unit(t))
            .and_then(|v| v.victim);
        let leader_target = host
            .party_leader()
            .and_then(|t| host.unit(t))
            .and_then(|v| v.victim);
        for member in host.party_members() {
            if !host.unit(member).is_some_and(|v| v.is_alive) {
                continue;
            }
            for attacker in host.attackers_of(member) {
                if Some(attacker) == tank_target || Some(attacker) == leader_target {
                    continue;
                }
                if host.is_ccd(attacker) || host.is_marked(attacker) {
                    continue;
                }
                let Some(view) = host.unit(attacker) else {
                    continue;
                };
                let d = ((me_view.position.0 - view.position.0).powi(2)
                    + (me_view.position.1 - view.position.1).powi(2)
                    + (me_view.position.2 - view.position.2).powi(2))
                .sqrt();
                if d > 30.0 {
                    continue;
                }
                if !host.can_try_cast(Some(attacker), spell_id) {
                    continue;
                }
                return Some(attacker);
            }
        }
        None
    }
}
impl BotBehavior for CrowdControl {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if !matches!(ctx.player_class, 2 | 8 | 5 | 9 | 4 | 11) {
            return 0.0;
        }
        if !host.cc_marks().is_empty() {
            return 68.0;
        }
        if ctx.role == BotRole::Tank || ctx.role == BotRole::Healer {
            return 0.0;
        }
        if ctx.has_loose_adds { 66.0 } else { 0.0 }
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if Self::control_marked(kit, host) {
            return true;
        }
        if ctx.role != BotRole::Tank && ctx.role != BotRole::Healer && ctx.has_loose_adds {
            if let Some(target) = Self::select_auto_target(kit, host) {
                host.auto_mark_cc(target);
                return Self::control_marked(kit, host);
            }
        }
        false
    }
}

// ============================================================
// 9. Kite
// ============================================================
behavior!(Kite, "Kite");
impl BotBehavior for Kite {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if !ctx.is_being_meleed || !is_caster_role(ctx.role, ctx.player_class, ctx.current_form) {
            return 0.0;
        }
        if ctx.role == BotRole::Healer
            && ctx.tank_is_alive
            && ctx.tank_health_pct < 30.0
            && ctx.health_pct > 50.0
        {
            return 0.0;
        }
        30.0 + (100.0 - ctx.health_pct) * 0.57
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if let Some(target) = ctx.current_target {
            if cast_ok(host, Some(target), kit.root) {
                return true;
            }
        }
        if ctx.has_flee_target {
            host.move_point(ctx.flee_target_x, ctx.flee_target_y, ctx.flee_target_z);
            return true;
        }
        // `RunAwayFromTarget`: run to tank/leader.
        let safe = host.party_tank().or_else(|| host.party_leader());
        let Some(safe) = safe else {
            return false;
        };
        if safe == host.me() {
            return false;
        }
        let (Some(me), Some(safe_view)) = (host.unit(host.me()), host.unit(safe)) else {
            return false;
        };
        if !safe_view.is_alive {
            return false;
        }
        let d = ((me.position.0 - safe_view.position.0).powi(2)
            + (me.position.1 - safe_view.position.1).powi(2)
            + (me.position.2 - safe_view.position.2).powi(2))
        .sqrt();
        if d < 5.0 {
            return false;
        }
        host.move_point(
            safe_view.position.0,
            safe_view.position.1,
            safe_view.position.2,
        );
        true
    }
}

// ============================================================
// 10. Execute
// ============================================================
behavior!(ExecuteTarget, "Execute");
impl BotBehavior for ExecuteTarget {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.current_target.is_none() || ctx.target_health_pct > 20.0 || ctx.should_throttle {
            return 0.0;
        }
        if !matches!(ctx.player_class, 1 | 9 | 2) {
            return 0.0;
        }
        65.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        cast_ok(host, ctx.current_target, kit.execute_ability)
    }
}

// ============================================================
// 11. ApplyDots
// ============================================================
behavior!(ApplyDots, "ApplyDots");
impl BotBehavior for ApplyDots {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.current_target.is_none() || ctx.target_health_pct < 30.0 || ctx.should_throttle {
            return 0.0;
        }
        if !matches!(ctx.player_class, 9 | 5 | 11 | 3 | 7 | 4 | 1) {
            return 0.0;
        }
        55.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        for dot in &kit.dot_spells {
            if cast_ok(host, ctx.current_target, Some(*dot)) {
                return true;
            }
        }
        false
    }
}

// ============================================================
// 12. AoeDamage
// ============================================================
behavior!(AoeDamage, "AoeDamage");
impl BotBehavior for AoeDamage {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        let Some(target) = ctx.current_target else {
            return 0.0;
        };
        if ctx.should_throttle {
            return 0.0;
        }
        let around_me = u8::try_from(
            host.attackers_of(host.me())
                .iter()
                .filter(|a| {
                    host.unit(**a).is_some_and(|v| {
                        let me = host.unit(host.me());
                        me.is_some_and(|m| {
                            ((m.position.0 - v.position.0).powi(2)
                                + (m.position.1 - v.position.1).powi(2)
                                + (m.position.2 - v.position.2).powi(2))
                            .sqrt()
                                <= 10.0
                        })
                    })
                })
                .count(),
        )
        .unwrap_or(u8::MAX);
        let enemy_count = around_me.max(host.hostile_count_around(target, 10.0));
        if enemy_count < 3 {
            return 0.0;
        }
        55.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        cast_ok(host, ctx.current_target, kit.aoe_spell)
    }
}

// ============================================================
// 13. DirectDamage
// ============================================================
behavior!(DirectDamage, "DirectDamage");
impl BotBehavior for DirectDamage {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.current_target.is_none() || ctx.should_throttle {
            return 0.0;
        }
        45.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        cast_ok(host, ctx.current_target, kit.filler)
    }
}

// ============================================================
// 14. SelfBuff
// ============================================================
behavior!(SelfBuff, "SelfBuff");
impl BotBehavior for SelfBuff {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        // Fork reads the spell union + current aura
        // here (via `m_ai`); the union lives in the
        // kit, which score doesn't receive — gate
        // on class and let execute's aura check
        // decide (a wasted fallthrough when already
        // buffed, costing nothing but order).
        if matches!(ctx.player_class, 8 | 9 | 5 | 7 | 1) {
            40.0
        } else {
            0.0
        }
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(buff) = kit.self_buff else {
            return false;
        };
        if host.has_aura(host.me(), buff) {
            return false;
        }
        let me = host.me();
        cast_ok(host, Some(me), Some(buff))
    }
}

// ============================================================
// 15. Position
// ============================================================
behavior!(Position, "Position");
impl Position {
    fn resolve_target(ctx: &SituationContext, host: &dyn BotHost) -> Option<ObjectGuid> {
        if let Some(target) = ctx.current_target {
            return Some(target);
        }
        if ctx.role == BotRole::Healer {
            if let Some(tank) = host.party_tank() {
                return host.unit(tank).and_then(|v| v.victim);
            }
        }
        None
    }
}
impl BotBehavior for Position {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.current_target.is_none()
            && ctx.has_formation_target
            && ctx.dist_to_formation_pos > MOVE_DEADZONE
        {
            return 35.0;
        }
        let is_caster = is_caster_role(ctx.role, ctx.player_class, ctx.current_form);
        if is_caster {
            let Some(pos_target) = Self::resolve_target(ctx, host) else {
                return 0.0;
            };
            let (Some(me), Some(target)) = (host.unit(host.me()), host.unit(pos_target)) else {
                return 0.0;
            };
            let dist = ((me.position.0 - target.position.0).powi(2)
                + (me.position.1 - target.position.1).powi(2)
                + (me.position.2 - target.position.2).powi(2))
            .sqrt();
            if dist > 30.0
                && (ctx.role == BotRole::Healer || host.motion_kind() == MotionKind::Idle)
            {
                return 35.0;
            }
            if dist < 10.0 && !matches!(host.motion_kind(), MotionKind::Chase | MotionKind::Point) {
                return 35.0;
            }
            0.0
        } else {
            let Some(pos_target) = ctx.current_target else {
                return 0.0;
            };
            if !host.can_reach_melee(pos_target) && host.motion_kind() == MotionKind::Idle {
                35.0
            } else {
                0.0
            }
        }
    }
    fn execute(&mut self, ctx: &SituationContext, _kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.current_target.is_none()
            && ctx.has_formation_target
            && ctx.dist_to_formation_pos > MOVE_DEADZONE
        {
            host.move_point(
                ctx.formation_target_x,
                ctx.formation_target_y,
                ctx.formation_target_z,
            );
            return true;
        }
        let Some(target) = Self::resolve_target(ctx, host) else {
            return false;
        };
        if is_caster_role(ctx.role, ctx.player_class, ctx.current_form) {
            // Motion-state gates from the fork score,
            // re-checked here where the host is live.
            let dist = ctx.distance_to_target;
            if dist > 30.0 && ctx.role != BotRole::Healer && host.motion_kind() != MotionKind::Idle
            {
                return false;
            }
            if dist < 10.0 && matches!(host.motion_kind(), MotionKind::Chase | MotionKind::Point) {
                return false;
            }
            host.move_chase(target, 25.0, 0.0);
        } else {
            if host.can_reach_melee(target) || host.motion_kind() != MotionKind::Idle {
                return false;
            }
            host.move_chase(target, 0.0, 0.0);
        }
        true
    }
}

// ============================================================
// 16. Dispel (dispatcher pre-step; score is 0 upstream)
// ============================================================
behavior!(Dispel, "Dispel");
impl BotBehavior for Dispel {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, _ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        check_for_dispel_targets(ctx, kit, host)
    }
}

/// `PartyBotAI::CheckForDispelTargets`.
pub fn check_for_dispel_targets(
    ctx: &SituationContext,
    kit: &SpellKit,
    host: &mut dyn BotHost,
) -> bool {
    if ctx.current_form != 0 {
        return false;
    }
    let spells = &kit.spells;
    let candidates: &[&str] = match ctx.player_class {
        2 => &["pCleanse"],
        7 => &["pCureDisease", "pCurePoison"],
        8 => &["pRemoveLesserCurse"],
        5 => &["pDispelMagic", "pAbolishDisease"],
        11 => &["pAbolishPoison", "pCurePoison", "pRemoveCurse"],
        _ => &[],
    };
    for field in candidates {
        let Some(spell_id) = spells.get(field) else {
            continue;
        };
        if ctx.player_class == 11
            && *field == "pCurePoison"
            && spells.get("pAbolishPoison").is_some()
        {
            continue; // abolish preferred, as in C++
        }
        if let Some(friend) = host.dispel_target(spell_id) {
            if cast_ok(host, Some(friend), Some(spell_id)) {
                return true;
            }
        }
    }
    false
}

// ============================================================
// 17. PartyBuff
// ============================================================
behavior!(PartyBuff, "PartyBuff");
impl PartyBuff {
    fn buff_fields(class_id: u8) -> &'static [&'static str] {
        match class_id {
            11 => &["pGiftoftheWild", "pMarkoftheWild", "pThorns"],
            5 => &[
                "pPrayerofFortitude",
                "pPowerWordFortitude",
                "pPrayerofSpirit",
                "pDivineSpirit",
                "pPrayerofShadowProtection",
                "pShadowProtection",
            ],
            8 => &["pArcaneBrilliance", "pArcaneIntellect"],
            9 => &["pDetectInvisibility"],
            _ => &[],
        }
    }
    fn try_buff(host: &mut dyn BotHost, spell_id: Option<u32>, tank_first: bool) -> bool {
        let Some(spell_id) = spell_id else {
            return false;
        };
        if tank_first {
            if let Some(tank) = host.party_tank() {
                if cast_ok(host, Some(tank), Some(spell_id)) {
                    host.clear_target();
                    return true;
                }
            }
        }
        if let Some(target) = healing::select_buff_target(host, spell_id) {
            if cast_ok(host, Some(target), Some(spell_id)) {
                host.clear_target();
                return true;
            }
        }
        false
    }
}
impl BotBehavior for PartyBuff {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        // Fork probes SelectBuffTarget per buff
        // here; the spell ids live in the kit's
        // union, which score doesn't receive —
        // class gate + execute decides (same
        // fallthrough caveat as SelfBuff).
        if matches!(ctx.player_class, 11 | 5 | 8 | 9 | 2) {
            52.0
        } else {
            0.0
        }
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let spells = &kit.spells;
        if ctx.player_class == 2 {
            let me = host.me();
            return cast_ok(host, Some(me), spells.get("pAura"));
        }
        for field in Self::buff_fields(ctx.player_class) {
            let tank_first = *field == "pThorns";
            if Self::try_buff(host, spells.get(field), tank_first) {
                return true;
            }
        }
        false
    }
}

// ============================================================
// 18. WandFiller
// ============================================================
behavior!(WandFiller, "WandFiller");
impl BotBehavior for WandFiller {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.role != BotRole::Healer || !host.knows_spell(SHOOT_WAND) {
            return 0.0;
        }
        if ctx.is_casting || ctx.is_moving {
            return 0.0;
        }
        if ctx.tank_needs_heal || ctx.critical_heal_needed {
            return 0.0;
        }
        if ctx.current_target.is_some() {
            return 5.0;
        }
        if let Some(tank) = host.party_tank() {
            if host.unit(tank).and_then(|v| v.victim).is_some() {
                return 5.0;
            }
        }
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, _kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if !host.knows_spell(SHOOT_WAND) {
            return false;
        }
        let target = ctx.current_target.or_else(|| {
            host.party_tank()
                .and_then(|t| host.unit(t))
                .and_then(|v| v.victim)
        });
        let Some(target) = target else {
            return false;
        };
        host.cast_spell(Some(target), SHOOT_WAND);
        true
    }
}

// ============================================================
// 19. CasterAttackStop (dispatcher pre-step)
// ============================================================
behavior!(CasterAttackStop, "CasterAttackStop");
impl BotBehavior for CasterAttackStop {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, _ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, _kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if is_caster_role(ctx.role, ctx.player_class, ctx.current_form) {
            host.attack_stop();
        }
        false
    }
}
