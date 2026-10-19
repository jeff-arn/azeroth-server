//! Class-unique behaviours
//! (`BotBehaviors_ClassUnique.cpp`). Pet/weapon-buff
//! executes go through host verbs that light up
//! when those subsystems land host-side; scores
//! and structure are the fork's.

use crate::behavior::BotBehavior;
use crate::context::SituationContext;
use crate::healing;
use crate::host::{AURA_MOD_INVISIBILITY, AURA_MOD_STEALTH, BotHost, CastOutcome};
use crate::role::BotRole;
use crate::spell_kit::SpellKit;

/// Shapeshift forms (1.12 `ShapeshiftForm`).
const FORM_NONE: u8 = 0x00;
const FORM_CAT: u8 = 0x01;
const FORM_BEAR: u8 = 0x05;
const FORM_DIREBEAR: u8 = 0x08;
const FORM_DEFENSIVE_STANCE: u8 = 0x12;
const FORM_MOONKIN: u8 = 0x1F;

/// Warlock summon spell ids.
const SUMMON_IMP: u32 = 688;
const SUMMON_VOIDWALKER: u32 = 697;
const SUMMON_FELHUNTER: u32 = 691;
const SUMMON_SUCCUBUS: u32 = 712;

fn cast_self(host: &mut dyn BotHost, kit: &SpellKit, field: &str) -> bool {
    let me = host.me();
    cast_field(host, kit, field, Some(me))
}

fn cast_field(
    host: &mut dyn BotHost,
    kit: &SpellKit,
    field: &str,
    target: Option<mangos_guid::ObjectGuid>,
) -> bool {
    let Some(spell_id) = kit.spells.get(field) else {
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
// WARRIOR
// ============================================================
behavior!(WarriorStanceDance, "WarriorStanceDance");
impl BotBehavior for WarriorStanceDance {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 1 {
            return 0.0;
        }
        if ctx.role == BotRole::Tank {
            if ctx.current_form != FORM_DEFENSIVE_STANCE {
                return 88.0;
            }
        } else if ctx.current_form == FORM_DEFENSIVE_STANCE {
            return 88.0;
        }
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let me = Some(host.me());
        if ctx.role == BotRole::Tank {
            cast_field(host, kit, "pDefensiveStance", me)
        } else if kit.spells.get("pBerserkerStance").is_some() {
            cast_field(host, kit, "pBerserkerStance", me)
        } else {
            cast_field(host, kit, "pBattleStance", me)
        }
    }
}

behavior!(WarriorRageDump, "WarriorRageDump");
impl BotBehavior for WarriorRageDump {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 1 || ctx.rage_pct < 50.0 || ctx.current_target.is_none() {
            return 0.0;
        }
        45.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.melee_attacker_count > 1 && cast_field(host, kit, "pCleave", ctx.current_target) {
            return true;
        }
        cast_field(host, kit, "pHeroicStrike", ctx.current_target)
    }
}

// ============================================================
// ROGUE
// ============================================================
behavior!(RogueStealthOpener, "RogueStealthOpener");
impl RogueStealthOpener {
    fn resolve(ctx: &SituationContext, host: &dyn BotHost) -> Option<mangos_guid::ObjectGuid> {
        ctx.current_target
            .or_else(|| host.unit(host.me()).and_then(|v| v.target))
    }
    fn is_behind(host: &dyn BotHost, target: mangos_guid::ObjectGuid) -> bool {
        let (Some(me), Some(t)) = (host.unit(host.me()), host.unit(target)) else {
            return false;
        };
        // Behind = me is in the rear arc of target's
        // facing (C++ `IsBehindTarget` uses the
        // backstab arc).
        let angle_to_me = (me.position.1 - t.position.1).atan2(me.position.0 - t.position.0);
        let mut diff = (angle_to_me - t.orientation).abs() % std::f32::consts::TAU;
        if diff > std::f32::consts::PI {
            diff = std::f32::consts::TAU - diff;
        }
        diff > std::f32::consts::FRAC_PI_2
    }
}
impl BotBehavior for RogueStealthOpener {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 4 || !host.has_aura_type(host.me(), AURA_MOD_STEALTH) {
            return 0.0;
        }
        if Self::resolve(ctx, host).is_none() {
            return 0.0;
        }
        92.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(target) = Self::resolve(ctx, host) else {
            return false;
        };
        if !Self::is_behind(host, target) {
            let Some(t) = host.unit(target) else {
                return false;
            };
            let behind = t.orientation + std::f32::consts::PI;
            host.move_point(
                t.position.0 + 2.0 * behind.cos(),
                t.position.1 + 2.0 * behind.sin(),
                t.position.2,
            );
            return true;
        }
        host.face(target);
        if !host.can_reach_melee(target) {
            host.move_chase(target, 1.0, 3.0);
            return true;
        }
        // Premeditation (fire-and-forget like C++).
        if let Some(spell) = kit.spells.get("pPremeditation") {
            if host.can_try_cast(Some(target), spell) {
                host.cast_spell(Some(target), spell);
            }
        }
        cast_field(host, kit, "pCheapShot", Some(target))
            || cast_field(host, kit, "pGarrote", Some(target))
            || cast_field(host, kit, "pAmbush", Some(target))
    }
}

behavior!(RogueComboFinisher, "RogueComboFinisher");
impl BotBehavior for RogueComboFinisher {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 4 || ctx.combo_points < 5 {
            return 0.0;
        }
        70.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if let Some(snd) = kit.spells.get("pSliceAndDice") {
            if !host.has_aura(host.me(), snd) && cast_self(host, kit, "pSliceAndDice") {
                return true;
            }
        }
        if ctx.target_is_casting && cast_field(host, kit, "pKidneyShot", ctx.current_target) {
            return true;
        }
        cast_field(host, kit, "pEviscerate", ctx.current_target)
    }
}

/// `ShouldEnterStealth`.
#[must_use]
pub fn should_enter_stealth(ctx: &SituationContext, host: &dyn BotHost) -> bool {
    if ctx.current_target.is_some() {
        return true;
    }
    if ctx.health_pct < 10.0 {
        return true;
    }
    if let Some(leader) = host.party_leader() {
        if let Some(view) = host.unit(leader) {
            if view.in_combat || !view.is_alive {
                return true;
            }
        }
        if host.has_aura_type(leader, AURA_MOD_STEALTH)
            || host.has_aura_type(leader, AURA_MOD_INVISIBILITY)
        {
            return true;
        }
    }
    false
}

behavior!(RogueStealth, "RogueStealth");
impl BotBehavior for RogueStealth {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 4 || host.has_aura_type(host.me(), AURA_MOD_STEALTH) {
            return 0.0;
        }
        if !should_enter_stealth(ctx, host) {
            return 0.0;
        }
        60.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        // `EnterStealthIfNeeded` (also cancels when
        // stealth is no longer wanted).
        let Some(stealth) = kit.spells.get("pStealth") else {
            return false;
        };
        let want = should_enter_stealth(ctx, host);
        if host.has_aura(host.me(), stealth) {
            if !want {
                host.cancel_aura(stealth);
            }
            return false;
        }
        want && cast_self(host, kit, "pStealth")
    }
}

behavior!(RoguePoisonWeapons, "RoguePoisonWeapons");
impl BotBehavior for RoguePoisonWeapons {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 4 {
            return 0.0;
        }
        // Poison presence lives in the kit (fork
        // reads the union in Score); class gate
        // here, execute decides.
        50.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if let Some(mh) = kit.spells.get("pMainHandPoison") {
            if host.cast_weapon_buff(mh, false) {
                return true;
            }
        }
        if let Some(oh) = kit.spells.get("pOffHandPoison") {
            if host.cast_weapon_buff(oh, true) {
                return true;
            }
        }
        false
    }
}

// ============================================================
// HUNTER
// ============================================================
behavior!(HunterPetManage, "HunterPetManage");
impl BotBehavior for HunterPetManage {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 3 {
            return 0.0;
        }
        match host.pet() {
            None => 55.0,
            Some(pet) => {
                if ctx.current_target.is_some() && host.unit(pet).and_then(|v| v.victim).is_none() {
                    55.0
                } else {
                    0.0
                }
            }
        }
    }
    fn execute(&mut self, ctx: &SituationContext, _kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if host.pet().is_none() {
            host.summon_pet_if_needed();
            return true;
        }
        if let Some(target) = ctx.current_target {
            host.pet_attack(target);
            return true;
        }
        false
    }
}

behavior!(HunterAspectSwap, "HunterAspectSwap");
impl BotBehavior for HunterAspectSwap {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 3 {
            return 0.0;
        }
        // Fork gates each arm on CanTryToCast (which
        // fails when the aspect is already up);
        // execute re-checks via can_try_cast.
        45.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let me = Some(host.me());
        if ctx.distance_to_target < 8.0 {
            cast_field(host, kit, "pAspectOfTheMonkey", me)
        } else {
            cast_field(host, kit, "pAspectOfTheHawk", me)
        }
    }
}

behavior!(HunterDeadZone, "HunterDeadZone");
impl BotBehavior for HunterDeadZone {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 3 || ctx.current_target.is_none() {
            return 0.0;
        }
        if ctx.distance_to_target < 8.0 {
            return 88.0;
        }
        0.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        for field in ["pWingClip", "pMongooseBite", "pRaptorStrike"] {
            if let Some(spell) = kit.spells.get(field) {
                if host.can_try_cast(ctx.current_target, spell) {
                    host.cast_spell(ctx.current_target, spell);
                }
            }
        }
        // Run to tank/leader (`RunAwayFromTarget`).
        let safe = host.party_tank().or_else(|| host.party_leader());
        if let Some(safe) = safe {
            if safe != host.me() {
                if let (Some(me), Some(s)) = (host.unit(host.me()), host.unit(safe)) {
                    let d = ((me.position.0 - s.position.0).powi(2)
                        + (me.position.1 - s.position.1).powi(2)
                        + (me.position.2 - s.position.2).powi(2))
                    .sqrt();
                    if s.is_alive && d >= 5.0 {
                        host.move_point(s.position.0, s.position.1, s.position.2);
                    }
                }
            }
        }
        true
    }
}

// ============================================================
// PALADIN
// ============================================================
behavior!(PaladinBlessing, "PaladinBlessing");
impl BotBehavior for PaladinBlessing {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 2 {
            return 0.0;
        }
        50.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(buff) = kit.spells.get("pBlessingBuff") else {
            return false;
        };
        match healing::select_buff_target(host, buff) {
            Some(target) => cast_field(host, kit, "pBlessingBuff", Some(target)),
            None => false,
        }
    }
}

behavior!(PaladinSealJudge, "PaladinSealJudge");
impl BotBehavior for PaladinSealJudge {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 2 || ctx.current_target.is_none() {
            return 0.0;
        }
        55.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if cast_self(host, kit, "pSeal") {
            return true;
        }
        cast_field(host, kit, "pJudgement", ctx.current_target)
    }
}

// ============================================================
// SHAMAN
// ============================================================
behavior!(ShamanTotemManage, "ShamanTotemManage");
impl BotBehavior for ShamanTotemManage {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 7 {
            return 0.0;
        }
        60.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        // `SummonShamanTotems` (Air=3, Earth=1,
        // Fire=0, Water=2 slots).
        for (field, slot) in [
            ("pAirTotem", 3u32),
            ("pEarthTotem", 1),
            ("pFireTotem", 0),
            ("pWaterTotem", 2),
        ] {
            if kit.spells.get(field).is_some()
                && !host.totem_present(slot)
                && cast_self(host, kit, field)
            {
                host.note_totem_cast(slot);
                return true;
            }
        }
        false
    }
}

// ============================================================
// PRIEST
// ============================================================
behavior!(PriestShadowformToggle, "PriestShadowformToggle");
impl BotBehavior for PriestShadowformToggle {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 5 || ctx.role == BotRole::Healer {
            return 0.0;
        }
        65.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        cast_self(host, kit, "pShadowform")
    }
}

behavior!(PriestShieldLogic, "PriestShieldLogic");
impl PriestShieldLogic {
    fn select_target(host: &dyn BotHost) -> Option<mangos_guid::ObjectGuid> {
        let me = host.me();
        for member in host.party_members() {
            if member == me {
                continue;
            }
            let view = host.unit(member)?;
            if view.health_pct < 90.0 && !host.attackers_of(member).is_empty() {
                return Some(member);
            }
        }
        None
    }
}
impl BotBehavior for PriestShieldLogic {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 5 || ctx.role != BotRole::Healer {
            return 0.0;
        }
        if Self::select_target(host).is_some() {
            72.0
        } else {
            0.0
        }
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(target) = Self::select_target(host) else {
            return false;
        };
        // Warrior-tank rage exception.
        if let Some(view) = host.unit(target) {
            if view.class_id == 1 && host.party_tank() == Some(target) && view.health_pct > 15.0 {
                return false;
            }
        }
        cast_field(host, kit, "pPowerWordShield", Some(target))
    }
}

// ============================================================
// WARLOCK
// ============================================================
behavior!(WarlockPetSummon, "WarlockPetSummon");
impl BotBehavior for WarlockPetSummon {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 9 {
            return 0.0;
        }
        if host.pet().is_none() {
            return 55.0;
        }
        0.0 // grimoire-teach arm lands with pets
    }
    fn execute(
        &mut self,
        _ctx: &SituationContext,
        _kit: &SpellKit,
        host: &mut dyn BotHost,
    ) -> bool {
        if host.pet().is_some() {
            return false;
        }
        let has_tank = host.party_tank().is_some();
        let mut summons: Vec<u32> = Vec::new();
        if host.knows_spell(SUMMON_IMP) {
            summons.push(SUMMON_IMP);
        }
        if host.knows_spell(SUMMON_VOIDWALKER) && !has_tank {
            summons.push(SUMMON_VOIDWALKER);
        }
        if host.knows_spell(SUMMON_FELHUNTER) {
            summons.push(SUMMON_FELHUNTER);
        }
        if host.knows_spell(SUMMON_SUCCUBUS) {
            summons.push(SUMMON_SUCCUBUS);
        }
        if summons.is_empty() {
            return false;
        }
        let pick = summons[host.pick_index(summons.len()).min(summons.len() - 1)];
        host.cast_spell(Some(host.me()), pick);
        true
    }
}

behavior!(WarlockCurseManage, "WarlockCurseManage");
impl WarlockCurseManage {
    const CURSES: &'static [&'static str] = &[
        "pCurseofTongues",
        "pCurseoftheElements",
        "pCurseofShadow",
        "pCurseofDoom",
        "pCurseofRecklessness",
        "pCurseofAgony",
    ];
}
impl BotBehavior for WarlockCurseManage {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 9 || ctx.current_target.is_none() {
            return 0.0;
        }
        // Already-cursed check needs the kit's
        // spell ids (fork reads the union); execute
        // re-checks before casting.
        52.0
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        let Some(target) = ctx.current_target else {
            return false;
        };
        for field in Self::CURSES {
            if let Some(spell) = kit.spells.get(field) {
                if host.has_aura(target, spell) {
                    return false;
                }
            }
        }
        let target_is_caster = host.unit(target).is_some_and(|v| v.power_type == 0);
        if target_is_caster && cast_field(host, kit, "pCurseofTongues", Some(target)) {
            return true;
        }
        cast_field(host, kit, "pCurseoftheElements", Some(target))
            || cast_field(host, kit, "pCurseofShadow", Some(target))
            || cast_field(host, kit, "pCurseofAgony", Some(target))
    }
}

behavior!(WarlockLifeTap, "WarlockLifeTap");
impl BotBehavior for WarlockLifeTap {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 9
            || ctx.mana_pct > 30.0
            || ctx.health_pct < 60.0
            || ctx.melee_attacker_count > 0
        {
            return 0.0;
        }
        48.0
    }
    fn execute(&mut self, _ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        cast_self(host, kit, "pLifeTap")
    }
}

// ============================================================
// DRUID
// ============================================================
behavior!(DruidFormShift, "DruidFormShift");
impl BotBehavior for DruidFormShift {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.player_class != 11 {
            return 0.0;
        }
        match ctx.role {
            BotRole::Healer if ctx.current_form != FORM_NONE => 85.0,
            BotRole::Tank if !matches!(ctx.current_form, FORM_BEAR | FORM_DIREBEAR) => 80.0,
            BotRole::MeleeDps if ctx.current_form != FORM_CAT => 75.0,
            BotRole::RangeDps if ctx.current_form != FORM_MOONKIN => 70.0,
            _ => 0.0,
        }
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.role == BotRole::Healer && ctx.current_form != FORM_NONE {
            host.remove_shapeshift();
            return true;
        }
        let me = Some(host.me());
        match ctx.role {
            BotRole::Tank => cast_field(host, kit, "pBearForm", me),
            BotRole::MeleeDps => {
                cast_field(host, kit, "pCatForm", me) || cast_field(host, kit, "pBearForm", me)
            }
            BotRole::RangeDps => cast_field(host, kit, "pMoonkinForm", me),
            _ => false,
        }
    }
}

behavior!(DruidRebirth, "DruidRebirth");
impl DruidRebirth {
    fn select_target(host: &dyn BotHost) -> Option<mangos_guid::ObjectGuid> {
        let me = host.me();
        let me_view = host.unit(me)?;
        for member in host.party_members() {
            if member == me {
                continue;
            }
            let view = host.unit(member)?;
            if view.is_alive {
                continue;
            }
            let d = ((me_view.position.0 - view.position.0).powi(2)
                + (me_view.position.1 - view.position.1).powi(2)
                + (me_view.position.2 - view.position.2).powi(2))
            .sqrt();
            if d <= 30.0 {
                return Some(member);
            }
        }
        None
    }
}
impl BotBehavior for DruidRebirth {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 11 || ctx.dead_party_members == 0 {
            return 0.0;
        }
        if Self::select_target(host).is_some() {
            93.0
        } else {
            0.0
        }
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.current_form != FORM_NONE {
            host.remove_shapeshift();
            return true;
        }
        match Self::select_target(host) {
            Some(target) => cast_field(host, kit, "pRebirth", Some(target)),
            None => false,
        }
    }
}

behavior!(DruidInnervate, "DruidInnervate");
impl DruidInnervate {
    fn needy_healer(host: &dyn BotHost) -> Option<mangos_guid::ObjectGuid> {
        let me = host.me();
        for member in host.party_members() {
            if member == me {
                continue;
            }
            let view = host.unit(member)?;
            if view.is_alive && healing::is_healer_class(view.class_id) && view.power_pct < 30.0 {
                return Some(member);
            }
        }
        None
    }
}
impl BotBehavior for DruidInnervate {
    fn name(&self) -> &'static str {
        self.nm()
    }
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32 {
        if ctx.player_class != 11 {
            return 0.0;
        }
        if ctx.mana_pct < 20.0 || Self::needy_healer(host).is_some() {
            72.0
        } else {
            0.0
        }
    }
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.mana_pct < 20.0 && cast_self(host, kit, "pInnervate") {
            return true;
        }
        match Self::needy_healer(host) {
            Some(target) => cast_field(host, kit, "pInnervate", Some(target)),
            None => false,
        }
    }
}
