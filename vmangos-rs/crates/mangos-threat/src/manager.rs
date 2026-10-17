//! `ThreatManager` + `ThreatContainer` + the manager-side
//! behaviour of `HostileReference`
//! (`vmangos/src/game/Threat/ThreatManager.{h,cpp}`).

use std::cmp::Ordering;

use mangos_guid::ObjectGuid;

use crate::{AddDecision, HostileRef, SCHOOL_MASK_NONE, ThreatHost, ThreatSpell, calc_threat};

#[derive(Clone, Debug, Default)]
struct Container {
    entries: Vec<HostileRef>,
    dirty: bool,
}

impl Container {
    fn find(&self, target: ObjectGuid) -> Option<usize> {
        self.entries.iter().position(|e| e.target() == target)
    }

    fn get(&self, target: ObjectGuid) -> Option<&HostileRef> {
        self.entries.iter().find(|e| e.target() == target)
    }

    /// `ThreatContainer::update`: stable sort, descending threat
    /// (`std::list::sort` with `lhs > rhs`). The dirty flag is
    /// cleared even when no sort was needed.
    fn update(&mut self) {
        if self.dirty && self.entries.len() > 1 {
            self.entries.sort_by(|a, b| {
                b.threat()
                    .partial_cmp(&a.threat())
                    .unwrap_or(Ordering::Equal)
            });
        }
        self.dirty = false;
    }
}

/// One creature's threat table (`ThreatManager`).
#[derive(Clone, Debug)]
pub struct ThreatManager {
    owner: ObjectGuid,
    current_victim: Option<ObjectGuid>,
    online: Container,
    offline: Container,
}

impl ThreatManager {
    #[must_use]
    pub fn new(owner: ObjectGuid) -> Self {
        Self {
            owner,
            current_victim: None,
            online: Container::default(),
            offline: Container::default(),
        }
    }

    /// `getOwner()`.
    #[must_use]
    pub const fn owner(&self) -> ObjectGuid {
        self.owner
    }

    /// `getCurrentVictim()`.
    #[must_use]
    pub const fn current_victim(&self) -> Option<ObjectGuid> {
        self.current_victim
    }

    /// `setCurrentVictim(...)`.
    pub fn set_current_victim(&mut self, victim: Option<ObjectGuid>) {
        self.current_victim = victim;
    }

    /// `setCurrentVictimIfCan(pVictim)`: only if an online
    /// reference exists.
    pub fn set_current_victim_if_can(&mut self, victim: ObjectGuid) {
        if self.online.get(victim).is_some() {
            self.current_victim = Some(victim);
        }
    }

    /// Online threat list in current order (`getThreatList()`).
    #[must_use]
    pub fn threat_list(&self) -> &[HostileRef] {
        &self.online.entries
    }

    /// Offline (unattackable) threat list in current order.
    #[must_use]
    pub fn offline_list(&self) -> &[HostileRef] {
        &self.offline.entries
    }

    /// `isThreatListEmpty()` (online list only).
    #[must_use]
    pub fn is_threat_list_empty(&self) -> bool {
        self.online.entries.is_empty()
    }

    /// `ThreatContainer::isDirty` on the online container.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.online.dirty
    }

    /// `setDirty(...)` (online container).
    pub fn set_dirty(&mut self, dirty: bool) {
        self.online.dirty = dirty;
    }

    /// `ThreatContainer::getMostHated`: front of the online list
    /// **without** sorting first, as in C++.
    #[must_use]
    pub fn most_hated(&self) -> Option<&HostileRef> {
        self.online.entries.first()
    }

    /// `ThreatContainer::getReferenceByTarget` (online list).
    #[must_use]
    pub fn reference_by_target(&self, victim: ObjectGuid) -> Option<&HostileRef> {
        self.online.get(victim)
    }

    /// Reference lookup across both containers (the C++ reference
    /// object is container-agnostic); online list first.
    #[must_use]
    pub fn find_reference(&self, victim: ObjectGuid) -> Option<&HostileRef> {
        self.online.get(victim).or_else(|| self.offline.get(victim))
    }

    /// `clearReferences()`: both containers emptied, current
    /// victim cleared (the dirty flag is left untouched, as in
    /// C++).
    pub fn clear_references(&mut self) {
        self.online.entries.clear();
        self.offline.entries.clear();
        self.current_victim = None;
    }

    /// `getThreat(pVictim, pAlsoSearchOfflineList)`.
    #[must_use]
    pub fn get_threat(&self, victim: ObjectGuid, also_search_offline: bool) -> f32 {
        if let Some(r) = self.online.get(victim) {
            return r.threat();
        }
        if also_search_offline {
            if let Some(r) = self.offline.get(victim) {
                return r.threat();
            }
        }
        0.0
    }

    fn locate_mut(&mut self, target: ObjectGuid) -> Option<&mut HostileRef> {
        if let Some(i) = self.online.find(target) {
            return Some(&mut self.online.entries[i]);
        }
        if let Some(i) = self.offline.find(target) {
            return Some(&mut self.offline.entries[i]);
        }
        None
    }

    /// `HostileReference::addThreat(pMod)` with its event effects
    /// (`processThreatEvent` `THREAT_CHANGE` dirty rule, offline
    /// refresh, pet-owner registration).
    fn entry_add_threat(&mut self, host: &dyn ThreatHost, target: ObjectGuid, delta: f32) {
        let Some(entry) = self.locate_mut(target) else {
            return;
        };
        let t = entry.threat + delta;
        entry.threat = if t < 0.0 { 0.0 } else { t };
        let was_online = entry.online;

        // if the link was cut before, relink it again
        if !was_online {
            self.update_online_status_of(host, target);
        }

        if delta != 0.0 {
            // UEV_THREAT_REF_THREAT_CHANGE
            let is_current = self.current_victim == Some(target);
            if (is_current && delta < 0.0) || (!is_current && delta > 0.0) {
                self.online.dirty = true;
            }
        }

        // create threat to the owner of a pet, if the pet attacks
        if delta >= 0.0 {
            if let Some(pet_owner) = host.pet_owner_to_register(self.owner, target) {
                self.add_threat_simple(host, pet_owner, 0.0);
            }
        }
    }

    /// `HostileReference::setOnlineOfflineState` with its
    /// `ONLINE_STATUS` event effects (container move, current
    /// victim clearing, dirty rules).
    pub fn set_online_offline_state_of(&mut self, target: ObjectGuid, online: bool) {
        let Some(entry) = self.locate_mut(target) else {
            return;
        };
        if entry.online == online {
            return;
        }
        entry.online = online;
        if !online {
            // if not online then not accessible as well
            entry.accessible = false;
        }

        if online {
            let threat = self.offline.get(target).map_or(0.0, HostileRef::threat);
            if let Some(current) = self.current_victim {
                let current_threat = self.get_threat(current, true);
                if threat > 1.1 * current_threat {
                    self.online.dirty = true;
                }
            }
            if let Some(i) = self.offline.find(target) {
                let entry = self.offline.entries.remove(i);
                self.online.entries.push(entry);
            }
        } else {
            if self.current_victim == Some(target) {
                self.current_victim = None;
                self.online.dirty = true;
            }
            if let Some(i) = self.online.find(target) {
                let entry = self.online.entries.remove(i);
                self.offline.entries.push(entry);
            }
        }
    }

    /// `HostileReference::setAccessibleState` (its status event is
    /// a no-op for the manager in C++).
    pub fn set_accessible_state_of(&mut self, target: ObjectGuid, accessible: bool) {
        if let Some(entry) = self.locate_mut(target) {
            entry.accessible = accessible;
        }
    }

    /// `HostileReference::updateOnlineStatus`: accessibility is
    /// always recomputed to false in this build; online state
    /// comes from the host.
    pub fn update_online_status_of(&mut self, host: &dyn ThreatHost, target: ObjectGuid) {
        if self.find_reference(target).is_none() {
            return;
        }
        let online = host.refresh_online(self.owner, target);
        self.set_accessible_state_of(target, false);
        self.set_online_offline_state_of(target, online);
    }

    /// `HostileReference::sourceObjectDestroyLink` — the hated
    /// unit object was destroyed.
    pub fn on_target_destroyed(&mut self, target: ObjectGuid) {
        self.set_online_offline_state_of(target, false);
    }

    /// `HostileReference::removeReference` + deletion: the
    /// `REMOVE_FROM_LIST` event effects, then removal.
    pub fn remove_reference(&mut self, target: ObjectGuid) {
        let Some(was_online) = self.find_reference(target).map(HostileRef::online) else {
            return;
        };
        if self.current_victim == Some(target) {
            self.current_victim = None;
            self.online.dirty = true;
        }
        let container = if was_online {
            &mut self.online
        } else {
            &mut self.offline
        };
        if let Some(i) = container.find(target) {
            container.entries.remove(i);
        }
    }

    /// `ThreatContainer::addThreat` on the chosen container:
    /// returns whether a reference was found there.
    fn container_add_threat(
        &mut self,
        host: &dyn ThreatHost,
        online_container: bool,
        victim: ObjectGuid,
        threat: f32,
    ) -> bool {
        let found = if online_container {
            self.online.get(victim).is_some()
        } else {
            self.offline.get(victim).is_some()
        };
        if found {
            self.entry_add_threat(host, victim, threat);
        }
        found
    }

    /// `addThreatDirectly(pVictim, threat, noNew)`: raw value,
    /// no calc; creates the reference unless `no_new`.
    pub fn add_threat_directly(
        &mut self,
        host: &dyn ThreatHost,
        victim: ObjectGuid,
        threat: f32,
        no_new: bool,
    ) {
        if victim == self.owner || !host.direct_gate(self.owner, victim) {
            return;
        }

        if self.container_add_threat(host, true, victim, threat) {
            return;
        }
        // Ref is not in the online refs, search the offline refs next
        if self.container_add_threat(host, false, victim, threat) {
            return;
        }

        // there was no ref => create a new one
        if !no_new {
            // threat has to be 0 here
            self.online.entries.push(HostileRef::new(victim, 0.0));
            self.entry_add_threat(host, victim, threat); // now we add the real threat
            if host.is_gamemaster_player(victim) {
                // GM is always offline
                self.set_online_offline_state_of(victim, false);
            }
        }
    }

    /// `addThreat(pVictim, threat, crit, schoolMask, threatSpell, isAssistThreat)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_threat(
        &mut self,
        host: &dyn ThreatHost,
        victim: ObjectGuid,
        threat: f32,
        crit: bool,
        school_mask: u32,
        spell: Option<&ThreatSpell>,
        is_assist_threat: bool,
    ) {
        // not to self
        if victim == self.owner {
            return;
        }
        let threat = match host.add_gate(self.owner, victim, is_assist_threat) {
            AddDecision::Reject => return,
            // don't add assist threat to targets under hard CC
            AddDecision::ZeroedAssist => 0.0,
            AddDecision::Allow => threat,
        };
        let total_threat = calc_threat(host, victim, threat, crit, school_mask, spell);
        self.add_threat_directly(
            host,
            victim,
            total_threat,
            spell.is_some_and(|s| s.no_threat),
        );
    }

    /// The 2-argument `addThreat(pVictim, threat)` overload.
    pub fn add_threat_simple(&mut self, host: &dyn ThreatHost, victim: ObjectGuid, threat: f32) {
        self.add_threat(host, victim, threat, false, SCHOOL_MASK_NONE, None, false);
    }

    /// `HostileReference::addThreatPercent`: the delta is
    /// `-threat` for exactly −100 (avoids rounding), else
    /// `threat * percent / 100`.
    fn percent_delta(threat: f32, percent: i32) -> f32 {
        if percent == -100 {
            -threat
        } else {
            #[allow(clippy::cast_precision_loss)]
            {
                threat * percent as f32 / 100.0
            }
        }
    }

    /// `HostileReference::addThreatPercent` on a reference in
    /// either container (the `HostileRefManager::addThreatPercent`
    /// fan-out path).
    pub fn add_threat_percent_to(
        &mut self,
        host: &dyn ThreatHost,
        target: ObjectGuid,
        percent: i32,
    ) {
        let Some(threat) = self.find_reference(target).map(HostileRef::threat) else {
            return;
        };
        let delta = Self::percent_delta(threat, percent);
        self.entry_add_threat(host, target, delta);
    }

    /// `modifyThreatPercent` (online container only): below −100
    /// removes and deletes the reference.
    pub fn modify_threat_percent(
        &mut self,
        host: &dyn ThreatHost,
        victim: ObjectGuid,
        percent: i32,
    ) {
        if self.online.get(victim).is_none() {
            return;
        }
        if percent < -100 {
            self.remove_reference(victim);
        } else {
            self.add_threat_percent_to(host, victim, percent);
        }
    }

    /// `HostileReference::setThreat`.
    pub fn set_threat_of(&mut self, host: &dyn ThreatHost, target: ObjectGuid, threat: f32) {
        let Some(current) = self.find_reference(target).map(HostileRef::threat) else {
            return;
        };
        self.entry_add_threat(host, target, threat - current);
    }

    /// `HostileReference::setTempThreat`: move threat to the
    /// given level, remembering the modification.
    pub fn set_temp_threat_of(&mut self, host: &dyn ThreatHost, target: ObjectGuid, threat: f32) {
        let Some(entry) = self.locate_mut(target) else {
            return;
        };
        let modifier = threat - entry.threat;
        entry.temp_modifier = modifier;
        if modifier != 0.0 {
            self.entry_add_threat(host, target, modifier);
        }
    }

    /// `HostileReference::setTempThreatModifier`: apply a raw
    /// temporary modification.
    pub fn set_temp_modifier_of(
        &mut self,
        host: &dyn ThreatHost,
        target: ObjectGuid,
        modifier: f32,
    ) {
        let Some(entry) = self.locate_mut(target) else {
            return;
        };
        entry.temp_modifier = modifier;
        self.entry_add_threat(host, target, modifier);
    }

    /// `HostileReference::resetTempThreat`.
    pub fn reset_temp_threat_of(&mut self, host: &dyn ThreatHost, target: ObjectGuid) {
        let Some(modifier) = self.find_reference(target).map(HostileRef::temp_modifier) else {
            return;
        };
        if modifier != 0.0 {
            self.entry_add_threat(host, target, -modifier);
            if let Some(entry) = self.locate_mut(target) {
                entry.temp_modifier = 0.0;
            }
        }
    }

    /// `tauntApply`: raise the taunter's (online) threat to the
    /// current victim's level via temp threat, if unused.
    pub fn taunt_apply(&mut self, host: &dyn ThreatHost, taunter: ObjectGuid) {
        let Some(r) = self.online.get(taunter) else {
            return;
        };
        let (threat, temp) = (r.threat(), r.temp_modifier());
        let Some(current) = self.current_victim else {
            return;
        };
        let current_threat = self.get_threat(current, true);
        if threat < current_threat && temp == 0.0 {
            self.set_temp_threat_of(host, taunter, current_threat);
        }
    }

    /// `tauntFadeOut` (online list only, as in C++).
    pub fn taunt_fade_out(&mut self, host: &dyn ThreatHost, taunter: ObjectGuid) {
        if self.online.get(taunter).is_some() {
            self.reset_temp_threat_of(host, taunter);
        }
    }

    /// `ThreatContainer::update` on the online list.
    pub fn update(&mut self) {
        self.online.update();
    }

    /// `ThreatContainer::selectNextVictim`, including the
    /// two-attempt low-priority fallback and the 110%/130% rules.
    /// Does not sort (call [`ThreatManager::update`] first, as
    /// `getHostileTarget` does).
    #[must_use]
    pub fn select_next_victim(&self, host: &dyn ThreatHost) -> Option<ObjectGuid> {
        // The C++ current-victim pointer always denotes a live
        // online reference; resolve its threat once.
        let mut current: Option<(ObjectGuid, f32)> = self
            .current_victim
            .and_then(|g| self.online.get(g).map(|r| (g, r.threat())));
        let mut allow_low_priority = false;

        for attempt in 0..2 {
            if attempt > 0 {
                allow_low_priority = true;
            }
            for entry in &self.online.entries {
                let target = entry.target();

                if host.out_of_threat_area(self.owner, target) {
                    return None;
                }
                if !host.valid_attack_target(self.owner, target) {
                    if current.is_some_and(|(g, _)| g == target) {
                        current = None;
                    }
                    continue;
                }
                if !allow_low_priority && host.low_priority_target(self.owner, target) {
                    // second-choice target: don't compare threat with it
                    if current.is_some_and(|(g, _)| g == target) {
                        current = None;
                    }
                    continue;
                }

                if let Some((current_guid, current_threat)) = current {
                    // list sorted and we check current target, best case
                    if current_guid == target || entry.threat() <= 1.1 * current_threat {
                        return Some(current_guid);
                    }
                    // 110% rule in melee range, 130% at range
                    if entry.threat() > 1.3 * current_threat
                        || (entry.threat() > 1.1 * current_threat
                            && host.can_reach_with_melee(self.owner, target))
                    {
                        return Some(target);
                    }
                    // otherwise keep scanning
                } else {
                    return Some(target);
                }
            }
        }
        None
    }

    /// `getHostileTarget()`: sort if dirty, select, remember.
    pub fn get_hostile_target(&mut self, host: &dyn ThreatHost) -> Option<ObjectGuid> {
        self.online.update();
        let next = self.select_next_victim(host);
        self.current_victim = next;
        next
    }
}
