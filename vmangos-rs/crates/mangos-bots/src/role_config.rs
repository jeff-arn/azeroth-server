//! Behaviour-set assembly (`BotRoleConfig.cpp`
//! `BuildBehaviorSet`): shared behaviours with
//! per-role weights, combat vs out-of-combat
//! sets. Class-unique behaviours join in E.3
//! (marked below where the fork adds them).

use crate::behavior::BehaviorSet;
use crate::behaviors_class as class;
use crate::behaviors_shared as shared;
use crate::role::BotRole;

/// `GetWeight(role, tank, healer, melee, ranged)`.
fn weight(role: BotRole, tank: f32, healer: f32, melee: f32, ranged: f32) -> f32 {
    match role {
        BotRole::Tank => tank,
        BotRole::Healer => healer,
        BotRole::RangeDps => ranged,
        _ => melee, // MeleeDps + Invalid fallback
    }
}

/// Builds one set. `is_party` mirrors `partyAI`
/// non-null (party behaviours included).
#[must_use]
pub fn build_behavior_set(
    class_id: u8,
    role: BotRole,
    in_combat: bool,
    is_party: bool,
) -> BehaviorSet {
    let mut set = BehaviorSet::new();
    if in_combat {
        set.add(
            Box::new(shared::SelfPreserve),
            weight(role, 1.0, 1.2, 1.0, 1.0),
        );
        set.add(
            Box::new(shared::Interrupt),
            weight(role, 1.0, 0.8, 1.0, 1.0),
        );
        if is_party {
            set.add(Box::new(shared::Taunt), weight(role, 1.5, 0.0, 0.0, 0.0));
            set.add(
                Box::new(shared::CrowdControl),
                weight(role, 0.3, 0.5, 0.8, 1.0),
            );
            set.add(Box::new(shared::Kite), weight(role, 0.0, 1.0, 0.0, 1.2));
            set.add(Box::new(shared::Dispel), weight(role, 0.8, 1.2, 0.5, 0.5));
        }
        set.add(
            Box::new(shared::HealParty),
            weight(role, 0.3, 1.5, 0.0, 0.0),
        );
        set.add(
            Box::new(shared::EmergencyHeal),
            weight(role, 0.0, 0.0, 0.5, 0.5),
        );
        set.add(
            Box::new(shared::TankThreat),
            weight(role, 1.5, 0.0, 0.0, 0.0),
        );
        set.add(
            Box::new(shared::ThreatDump),
            weight(role, 0.0, 1.0, 1.2, 1.2),
        );
        set.add(
            Box::new(shared::ExecuteTarget),
            weight(role, 1.0, 0.0, 1.2, 1.0),
        );
        set.add(
            Box::new(shared::ApplyDots),
            weight(role, 0.3, 0.0, 0.8, 1.0),
        );
        set.add(
            Box::new(shared::AoeDamage),
            weight(role, 0.5, 0.0, 1.0, 1.0),
        );
        set.add(
            Box::new(shared::DirectDamage),
            weight(role, 0.5, 0.2, 1.0, 1.0),
        );
        set.add(Box::new(shared::SelfBuff), weight(role, 0.8, 0.8, 0.8, 0.8));
        set.add(Box::new(shared::Position), weight(role, 0.8, 0.8, 0.8, 1.0));
        if is_party {
            set.add(
                Box::new(shared::WandFiller),
                weight(role, 0.0, 1.0, 0.0, 0.0),
            );
        }
        add_class_combat(&mut set, class_id, role);
    } else {
        set.add(Box::new(shared::SelfBuff), 1.0);
        set.add(Box::new(shared::PartyBuff), 1.0);
        set.add(
            Box::new(shared::HealParty),
            weight(role, 0.3, 1.5, 0.0, 0.0),
        );
        set.add(Box::new(shared::Position), 0.5);
        if is_party {
            set.add(Box::new(shared::Dispel), 1.0);
        }
        add_class_ooc(&mut set, class_id, role);
    }
    set
}

/// `AddClassCombatBehaviors` (fork weights).
fn add_class_combat(set: &mut BehaviorSet, class_id: u8, _role: BotRole) {
    match class_id {
        1 => {
            set.add(Box::new(class::WarriorStanceDance), 1.1);
            set.add(Box::new(class::WarriorRageDump), 0.8);
        }
        4 => {
            set.add(Box::new(class::RogueStealthOpener), 1.2);
            set.add(Box::new(class::RogueComboFinisher), 1.0);
        }
        3 => {
            set.add(Box::new(class::HunterDeadZone), 1.2);
            set.add(Box::new(class::HunterPetManage), 1.0);
            set.add(Box::new(class::HunterAspectSwap), 0.5);
        }
        2 => {
            set.add(Box::new(class::PaladinSealJudge), 0.9);
        }
        7 => {
            set.add(Box::new(class::ShamanTotemManage), 1.1);
        }
        5 => {
            set.add(Box::new(class::PriestShadowformToggle), 0.9);
            set.add(Box::new(class::PriestShieldLogic), 1.0);
        }
        9 => {
            set.add(Box::new(class::WarlockPetSummon), 0.9);
            set.add(Box::new(class::WarlockCurseManage), 0.9);
            set.add(Box::new(class::WarlockLifeTap), 0.9);
        }
        11 => {
            set.add(Box::new(class::DruidFormShift), 1.1);
            set.add(Box::new(class::DruidRebirth), 1.2);
            set.add(Box::new(class::DruidInnervate), 0.9);
        }
        _ => {}
    }
}

/// `AddClassOOCBehaviors` (fork weights).
fn add_class_ooc(set: &mut BehaviorSet, class_id: u8, role: BotRole) {
    match class_id {
        2 => {
            set.add(Box::new(class::PaladinBlessing), 1.0);
        }
        4 => {
            set.add(Box::new(class::RoguePoisonWeapons), 1.0);
            set.add(Box::new(class::RogueStealth), 1.0);
        }
        3 => {
            set.add(Box::new(class::HunterPetManage), 1.0);
        }
        9 => {
            set.add(Box::new(class::WarlockPetSummon), 1.0);
        }
        11 if role == BotRole::Tank => {
            set.add(Box::new(class::DruidFormShift), 0.8);
        }
        _ => {}
    }
}
