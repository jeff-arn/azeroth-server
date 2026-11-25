//! The shared healing system (`CombatBotBaseAI`
//! heal helpers): target selection, most-efficient
//! spell selection over the resolved heal sets,
//! and buff upkeep helpers. Pure logic over the
//! host views; `LoS` is approximated by the 30y
//! distance gate (the host has no `LoS` query yet).

use mangos_guid::ObjectGuid;

use crate::class_spells::{ClassSpells, HealSpell};
use crate::host::{BotHost, CastOutcome, UnitView};

/// `IsHealerClass`.
#[must_use]
pub fn is_healer_class(class_id: u8) -> bool {
    matches!(class_id, 2 | 5 | 7 | 11)
}

/// `IsTankClass`.
#[must_use]
pub fn is_tank_class(class_id: u8) -> bool {
    matches!(class_id, 1 | 2 | 11)
}

fn dist(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// `IsValidHealTarget` (helpful + 30y; `LoS` folded
/// into the distance gate for now).
fn is_valid_heal_target(me: &UnitView, target: &UnitView, health_percent: f32) -> bool {
    target.health_pct < health_percent
        && target.is_alive
        && dist(me.position, target.position) <= 30.0
}

/// `AreOthersOnSameTarget(guid, false, true)` as
/// used by heal selection.
fn others_casting_on(host: &dyn BotHost, me: ObjectGuid, guid: ObjectGuid) -> bool {
    host.party_members().into_iter().any(|member| {
        member != me
            && member != guid
            && host
                .unit(member)
                .is_some_and(|view| view.target == Some(guid) && view.is_casting)
    })
}

/// `SelectHealTarget` (pet arm deferred with the
/// pet system).
#[must_use]
pub fn select_heal_target(
    host: &dyn BotHost,
    me: &UnitView,
    self_pct: f32,
    group_pct: f32,
) -> Option<ObjectGuid> {
    if me.health_pct < self_pct {
        return Some(me.guid);
    }
    let mut target: Option<ObjectGuid> = None;
    let mut health_percent = 100.0f32;
    for member_guid in host.party_members() {
        if member_guid == me.guid {
            continue;
        }
        let Some(member) = host.unit(member_guid) else {
            continue;
        };
        if target.is_some()
            && !is_tank_class(member.class_id)
            && others_casting_on(host, me.guid, member_guid)
        {
            continue;
        }
        if is_valid_heal_target(me, &member, group_pct) && health_percent > member.health_pct {
            health_percent = member.health_pct;
            target = Some(member_guid);
        }
    }
    if (health_percent - 100.0).abs() < f32::EPSILON {
        return None;
    }
    target
}

/// `SelectMostEfficientHealingSpell`: walk the
/// set (descending base points), keep the spell
/// whose total is closest to `missing_health`,
/// stop at the first spell that undershoots.
#[must_use]
pub fn select_most_efficient_heal(
    host: &dyn BotHost,
    target: ObjectGuid,
    missing_health: i32,
    spells: &[HealSpell],
) -> Option<u32> {
    let mut best: Option<u32> = None;
    let mut health_diff = i32::MAX;
    for spell in spells {
        if !host.can_try_cast(Some(target), spell.id) {
            continue;
        }
        let diff = spell.total_points - missing_health;
        if diff.abs() < health_diff {
            health_diff = diff;
            best = Some(spell.id);
        }
        if diff < 0 {
            break;
        }
    }
    best
}

fn missing_health(view: &UnitView) -> i32 {
    i32::try_from(view.max_health.saturating_sub(view.health)).unwrap_or(i32::MAX)
}

/// `HealInjuredTargetDirect`.
pub fn heal_direct(host: &mut dyn BotHost, spells: &ClassSpells, target: ObjectGuid) -> bool {
    let Some(view) = host.unit(target) else {
        return false;
    };
    let Some(spell_id) =
        select_most_efficient_heal(host, target, missing_health(&view), &spells.direct_heals)
    else {
        return false;
    };
    host.cast_spell(Some(target), spell_id) == CastOutcome::Ok
}

/// `HealInjuredTargetPeriodic`.
pub fn heal_periodic(host: &mut dyn BotHost, spells: &ClassSpells, target: ObjectGuid) -> bool {
    let Some(view) = host.unit(target) else {
        return false;
    };
    let Some(spell_id) =
        select_most_efficient_heal(host, target, missing_health(&view), &spells.periodic_heals)
    else {
        return false;
    };
    if !host.can_try_cast(Some(target), spell_id) {
        return false;
    }
    host.cast_spell(Some(target), spell_id) == CastOutcome::Ok
}

/// `HealInjuredTarget` (`HoT` when lightly hurt,
/// else direct).
pub fn heal_injured(host: &mut dyn BotHost, spells: &ClassSpells, target: ObjectGuid) -> bool {
    let Some(view) = host.unit(target) else {
        return false;
    };
    if view.health_pct >= 80.0
        && !host.has_periodic_heal_aura(target)
        && heal_periodic(host, spells, target)
    {
        return true;
    }
    heal_direct(host, spells, target)
}

/// `FindAndHealInjuredAlly`.
pub fn find_and_heal_injured_ally(
    host: &mut dyn BotHost,
    spells: &ClassSpells,
    self_pct: f32,
    group_pct: f32,
) -> bool {
    let Some(me) = host.unit(host.me()) else {
        return false;
    };
    let Some(target) = select_heal_target(host, &me, self_pct, group_pct) else {
        return false;
    };
    heal_injured(host, spells, target)
}

/// `FindAndPreHealTarget` (cast-time > 1000ms gate
/// approximated by can-try only; cast-time lookup
/// is host data the bots don't carry yet).
pub fn find_and_pre_heal(host: &mut dyn BotHost, spells: &ClassSpells) -> bool {
    let Some(me) = host.unit(host.me()) else {
        return false;
    };
    let mut target = me.guid;
    let mut max_incoming = host.incoming_damage(me.guid);
    for member_guid in host.party_members() {
        if member_guid == me.guid {
            continue;
        }
        let Some(member) = host.unit(member_guid) else {
            continue;
        };
        if target != me.guid
            && !is_tank_class(member.class_id)
            && others_casting_on(host, me.guid, member_guid)
        {
            continue;
        }
        let incoming = host.incoming_damage(member_guid);
        if incoming == 0 {
            continue;
        }
        if incoming > max_incoming && is_valid_heal_target(&me, &member, 100.0) {
            max_incoming = incoming;
            target = member_guid;
        }
    }
    if max_incoming == 0 {
        return false;
    }
    let Some(target_view) = host.unit(target) else {
        return false;
    };
    max_incoming += missing_health(&target_view);
    if max_incoming < i32::try_from(target_view.max_health / 2).unwrap_or(i32::MAX) {
        return false;
    }
    let Some(spell_id) =
        select_most_efficient_heal(host, target, max_incoming, &spells.direct_heals)
    else {
        // Fork quirk: returns `pTarget` (non-null =
        // true) when no spell qualifies.
        return true;
    };
    if !host.can_try_cast(Some(target), spell_id) {
        return true; // same fork quirk path
    }
    if host.cast_spell(Some(target), spell_id) == CastOutcome::Ok {
        return true;
    }
    true // fork returns pTarget here as well
}

/// `SelectBuffTarget`: first party member missing
/// the buff (rank/more-powerful equivalence
/// reduced to same-spell presence).
#[must_use]
pub fn select_buff_target(host: &dyn BotHost, spell_id: u32) -> Option<ObjectGuid> {
    let me = host.unit(host.me())?;
    for member_guid in host.party_members() {
        let Some(member) = host.unit(member_guid) else {
            continue;
        };
        if !member.is_alive || dist(me.position, member.position) > 30.0 {
            continue;
        }
        if !host.has_aura(member_guid, spell_id) {
            return Some(member_guid);
        }
    }
    None
}
