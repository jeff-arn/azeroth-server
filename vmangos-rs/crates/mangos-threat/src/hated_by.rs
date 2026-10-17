//! `HostileRefManager` (`vmangos/src/game/Threat/HostileRefManager.{h,cpp}`):
//! the "hated by" back-reference list living on a victim unit.
//!
//! In C++ the list nodes are the same `HostileReference` objects
//! the attackers' threat tables own, maintained automatically by
//! reference linking. Here the list holds attacker GUIDs; the
//! world layer keeps it in sync when references are created and
//! removed (the Phase 3 glue point), and the fan-out algorithms
//! resolve each attacker's table through [`ThreatManagerResolver`].

use mangos_guid::ObjectGuid;

use crate::manager::ThreatManager;
use crate::{SCHOOL_MASK_NORMAL, ThreatHost, ThreatSpell};

/// Resolves an attacker's GUID to its threat table (the world
/// layer's creature registry in Phase 3).
pub trait ThreatManagerResolver {
    fn manager_mut(&mut self, owner: ObjectGuid) -> Option<&mut ThreatManager>;
}

/// A unit's hated-by list (`HostileRefManager`).
#[derive(Clone, Debug)]
pub struct HatedByList {
    owner: ObjectGuid,
    attackers: Vec<ObjectGuid>,
}

impl HatedByList {
    #[must_use]
    pub fn new(owner: ObjectGuid) -> Self {
        Self {
            owner,
            attackers: Vec::new(),
        }
    }

    /// `getOwner()`.
    #[must_use]
    pub const fn owner(&self) -> ObjectGuid {
        self.owner
    }

    /// Attackers currently linking to this unit, in link order.
    #[must_use]
    pub fn attackers(&self) -> &[ObjectGuid] {
        &self.attackers
    }

    /// Number of haters (`RefManager::getSize`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.attackers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attackers.is_empty()
    }

    /// Sync point replacing `HostileReference::targetObjectBuildLink`
    /// (`Unit::AddHatedBy`): called by the world layer when an
    /// attacker creates a reference to this unit.
    pub fn note_hater(&mut self, attacker: ObjectGuid) {
        if !self.attackers.contains(&attacker) {
            self.attackers.push(attacker);
        }
    }

    /// Sync point replacing `targetObjectDestroyLink`
    /// (`Unit::RemoveHatedBy`).
    pub fn forget_hater(&mut self, attacker: ObjectGuid) {
        self.attackers.retain(|a| *a != attacker);
    }

    /// `threatAssist`: send divided threat for `victim` to all my
    /// haters (buff/healing threat). Single-target spells do not
    /// divide.
    pub fn threat_assist(
        &self,
        host: &dyn ThreatHost,
        resolver: &mut dyn ThreatManagerResolver,
        victim: ObjectGuid,
        threat: f32,
        spell: Option<&ThreatSpell>,
        single_target: bool,
    ) {
        if spell.is_some_and(|s| s.no_helpful_threat) {
            return;
        }
        if self.attackers.is_empty() {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let size = if single_target {
            1.0
        } else {
            self.attackers.len() as f32
        };
        let each = threat / size;
        let school = spell.map_or(SCHOOL_MASK_NORMAL, |s| s.school_mask);
        for attacker in self.attackers.clone() {
            if let Some(manager) = resolver.manager_mut(attacker) {
                manager.add_threat(host, victim, each, false, school, spell, true);
            }
        }
    }

    /// `addThreatPercent` fan-out to every hater's reference for
    /// this unit.
    pub fn add_threat_percent_all(
        &self,
        host: &dyn ThreatHost,
        resolver: &mut dyn ThreatManagerResolver,
        value: i32,
    ) {
        for attacker in self.attackers.clone() {
            if let Some(manager) = resolver.manager_mut(attacker) {
                manager.add_threat_percent_to(host, self.owner, value);
            }
        }
    }

    /// `addTempThreat(threat, apply)` fan-out ("oubli"-style
    /// spells): apply only where no temp modifier is active.
    pub fn add_temp_threat_all(
        &self,
        host: &dyn ThreatHost,
        resolver: &mut dyn ThreatManagerResolver,
        threat: f32,
        apply: bool,
    ) {
        for attacker in self.attackers.clone() {
            if let Some(manager) = resolver.manager_mut(attacker) {
                if apply {
                    let temp = manager
                        .find_reference(self.owner)
                        .map_or(0.0, crate::HostileRef::temp_modifier);
                    if temp == 0.0 {
                        manager.set_temp_modifier_of(host, self.owner, threat);
                    }
                } else {
                    manager.reset_temp_threat_of(host, self.owner);
                }
            }
        }
    }

    /// `setOnlineOfflineState(bool)` fan-out (state precomputed).
    pub fn set_online_offline_state_all(
        &self,
        resolver: &mut dyn ThreatManagerResolver,
        online: bool,
    ) {
        for attacker in self.attackers.clone() {
            if let Some(manager) = resolver.manager_mut(attacker) {
                manager.set_online_offline_state_of(self.owner, online);
            }
        }
    }

    /// `setOnlineOfflineState(pCreature, bool)` for one attacker.
    pub fn set_online_offline_state_for(
        &self,
        resolver: &mut dyn ThreatManagerResolver,
        attacker: ObjectGuid,
        online: bool,
    ) {
        if let Some(manager) = resolver.manager_mut(attacker) {
            manager.set_online_offline_state_of(self.owner, online);
        }
    }

    /// `updateThreatTables`: recompute each reference's online
    /// status.
    pub fn update_threat_tables(
        &self,
        host: &dyn ThreatHost,
        resolver: &mut dyn ThreatManagerResolver,
    ) {
        for attacker in self.attackers.clone() {
            if let Some(manager) = resolver.manager_mut(attacker) {
                manager.update_online_status_of(host, self.owner);
            }
        }
    }

    /// `deleteReferences`: remove this unit from every hater's
    /// table and empty the list.
    pub fn delete_references(&mut self, resolver: &mut dyn ThreatManagerResolver) {
        for attacker in std::mem::take(&mut self.attackers) {
            if let Some(manager) = resolver.manager_mut(attacker) {
                manager.remove_reference(self.owner);
            }
        }
    }

    /// `deleteReference(pCreature)` for one attacker.
    pub fn delete_reference_for(
        &mut self,
        resolver: &mut dyn ThreatManagerResolver,
        attacker: ObjectGuid,
    ) {
        if let Some(manager) = resolver.manager_mut(attacker) {
            manager.remove_reference(self.owner);
        }
        self.forget_hater(attacker);
    }

    /// `deleteReferencesForFaction(faction)`.
    pub fn delete_references_for_faction(
        &mut self,
        host: &dyn ThreatHost,
        resolver: &mut dyn ThreatManagerResolver,
        faction: u32,
    ) {
        let mut kept = Vec::new();
        for attacker in std::mem::take(&mut self.attackers) {
            if host.faction_id(attacker) == faction {
                if let Some(manager) = resolver.manager_mut(attacker) {
                    manager.remove_reference(self.owner);
                }
            } else {
                kept.push(attacker);
            }
        }
        self.attackers = kept;
    }
}
