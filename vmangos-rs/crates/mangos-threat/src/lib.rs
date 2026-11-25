//! Threat tables, ported from `vmangos/src/game/Threat/`
//! (`ThreatManager.{h,cpp}`, `HostileRefManager.{h,cpp}`).
//!
//! Units are addressed by [`ObjectGuid`]; every place the C++
//! reaches into `Unit` (spell threat mods, attackability, chase
//! geometry, GM/taxi/validity) is a method on [`ThreatHost`],
//! implemented by the world layer. Everything else — the table
//! state machine, dirty-flag rules, online/offline container
//! moves, taunt temp-threat, the 110%/130% victim-selection rule,
//! threat-assist division — is pure and mirrors the C++ control
//! flow, including its quirks (documented at each site).
//!
//! The C++ event indirection (`ThreatRefStatusChangeEvent` →
//! `ThreatManager::processThreatEvent`) is folded into direct
//! state transitions with identical effects.

use mangos_guid::ObjectGuid;

pub mod hated_by;
pub mod manager;

pub use hated_by::{HatedByList, ThreatManagerResolver};
pub use manager::ThreatManager;

/// `SPELL_SCHOOL_MASK_NONE`.
pub const SCHOOL_MASK_NONE: u32 = 0;
/// `SPELL_SCHOOL_MASK_NORMAL` (physical).
pub const SCHOOL_MASK_NORMAL: u32 = 1;

/// The `SpellEntry` facts the threat code reads
/// (`AttributesEx` `NO_THREAT` bit 10, `AttributesEx4`
/// `NO_HELPFUL_THREAT` bit 3, school mask, id for spell mods).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ThreatSpell {
    pub id: u32,
    pub school_mask: u32,
    /// `SPELL_ATTR_EX_NO_THREAT`: may only add to existing refs.
    pub no_threat: bool,
    /// `SPELL_ATTR_EX4_NO_HELPFUL_THREAT`: `threatAssist` no-op.
    pub no_helpful_threat: bool,
}

/// One row of a threat table (`HostileReference` state; the
/// reference graph itself is replaced by GUID addressing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostileRef {
    target: ObjectGuid,
    threat: f32,
    temp_modifier: f32,
    online: bool,
    accessible: bool,
}

impl HostileRef {
    /// Fresh reference as the C++ constructor leaves it:
    /// online and accessible, zero temp modifier.
    #[must_use]
    pub const fn new(target: ObjectGuid, threat: f32) -> Self {
        Self {
            target,
            threat,
            temp_modifier: 0.0,
            online: true,
            accessible: true,
        }
    }

    /// `getUnitGuid()`.
    #[must_use]
    pub const fn target(&self) -> ObjectGuid {
        self.target
    }

    /// `getThreat()`.
    #[must_use]
    pub const fn threat(&self) -> f32 {
        self.threat
    }

    /// `getTempThreatModifyer()`.
    #[must_use]
    pub const fn temp_modifier(&self) -> f32 {
        self.temp_modifier
    }

    /// `isOnline()`.
    #[must_use]
    pub const fn online(&self) -> bool {
        self.online
    }

    /// `isAccessable()` [sic].
    #[must_use]
    pub const fn accessible(&self) -> bool {
        self.accessible
    }
}

/// Decision for the gated `ThreatManager::addThreat` entry points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddDecision {
    /// Proceed with the given threat.
    Allow,
    /// Drop entirely (null/GM/dead victim or dead owner).
    Reject,
    /// Assist threat against a hard-CC'd owner: proceed with 0.
    ZeroedAssist,
}

/// The world-query surface of the threat subsystem. Each method
/// corresponds to a specific `Unit` access in the C++ (named at
/// the method).
pub trait ThreatHost {
    /// `Player::ApplySpellMod(spellId, SPELLMOD_THREAT, threat)`
    /// via `Unit::GetSpellModOwner` (identity when no mod owner).
    fn spellmod_threat(&self, hated: ObjectGuid, spell_id: u32, threat: f32) -> f32;

    /// `Unit::GetTotalAuraMultiplierByMiscMask(SPELL_AURA_MOD_CRITICAL_THREAT, mask)`.
    fn crit_threat_multiplier(&self, hated: ObjectGuid, school_mask: u32) -> f32;

    /// `Unit::ApplyTotalThreatModifier(threat, mask)`.
    fn apply_total_threat_modifier(&self, hated: ObjectGuid, threat: f32, school_mask: u32) -> f32;

    /// `ThreatManager::addThreat` entry gates (victim not self is
    /// checked in-crate): null/GM/dead → [`AddDecision::Reject`];
    /// `is_assist` with owner confused/fleeing/isolated or
    /// breakably stunned → [`AddDecision::ZeroedAssist`].
    fn add_gate(&self, owner: ObjectGuid, victim: ObjectGuid, is_assist: bool) -> AddDecision;

    /// `ThreatManager::addThreatDirectly` entry gate (victim not
    /// self is checked in-crate): null/alive/in-same-map.
    fn direct_gate(&self, owner: ObjectGuid, victim: ObjectGuid) -> bool;

    /// `pVictim->GetTypeId() == TYPEID_PLAYER && IsGameMaster()`
    /// (new GM references are forced offline).
    fn is_gamemaster_player(&self, victim: ObjectGuid) -> bool;

    /// `HostileReference::updateOnlineStatus` result: the victim
    /// unit resolves again, is not a GM player, and is not
    /// taxi-flying.
    fn refresh_online(&self, owner: ObjectGuid, victim: ObjectGuid) -> bool;

    /// The pet-owner registration in `HostileReference::addThreat`:
    /// `Some(owner)` when the (valid) victim has an owner that is
    /// targetable by the threat-list owner.
    fn pet_owner_to_register(&self, owner: ObjectGuid, victim: ObjectGuid) -> Option<ObjectGuid>;

    /// `Creature::IsOutOfThreatArea(target)`.
    fn out_of_threat_area(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool;

    /// `Creature::IsValidAttackTarget(target)`.
    fn valid_attack_target(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool;

    /// The "second choice target" predicate in `selectNextVictim`:
    /// immune to the attacker's melee school, secondary threat
    /// target, or unreachable while the attacker is immobilized
    /// (caster chase distance / melee reach as applicable).
    fn low_priority_target(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool;

    /// `Creature::CanReachWithMeleeAutoAttack(target)` (the 110%
    /// melee-range rule).
    fn can_reach_with_melee(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool;

    /// `Unit::GetFactionId()` (for
    /// `HatedByList::delete_references_for_faction`).
    fn faction_id(&self, unit: ObjectGuid) -> u32;
}

/// `ThreatCalcHelper::CalcThreat` — control flow in-crate, the
/// three unit modifiers via [`ThreatHost`].
#[must_use]
pub fn calc_threat(
    host: &dyn ThreatHost,
    hated: ObjectGuid,
    threat: f32,
    crit: bool,
    school_mask: u32,
    spell: Option<&ThreatSpell>,
) -> f32 {
    // all flat mods applied early
    if threat == 0.0 {
        return 0.0;
    }
    let mut threat = threat;
    if let Some(spell) = spell {
        threat = host.spellmod_threat(hated, spell.id, threat);
        if crit {
            threat *= host.crit_threat_multiplier(hated, school_mask);
        }
    }
    host.apply_total_threat_modifier(hated, threat, school_mask)
}
