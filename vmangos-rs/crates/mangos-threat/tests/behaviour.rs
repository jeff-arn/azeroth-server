//! Behaviour-parity tests for `mangos-threat`, derived from
//! `vmangos/src/game/Threat/{ThreatManager,HostileRefManager}.cpp`
//! control flow. The fake host supplies world facts with
//! configurable truth tables and records calc-hook calls.
//!
//! Floats are compared with `==` deliberately: every expectation
//! is an exactly representable value or f32 arithmetic spelled
//! out in the C++ operation order (bit-equality is the claim).
#![allow(clippy::float_cmp)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use mangos_guid::ObjectGuid;
use mangos_threat::hated_by::ThreatManagerResolver;
use mangos_threat::{
    AddDecision, HatedByList, ThreatHost, ThreatManager, ThreatSpell, calc_threat,
};

fn g(n: u64) -> ObjectGuid {
    ObjectGuid::from_raw(0xF130_0000_0000_0000 + n)
}

const MOB: u64 = 1;
const A: u64 = 10;
const B: u64 = 11;
const C: u64 = 12;
const D: u64 = 13;

#[derive(Default)]
struct FakeHost {
    gm: HashSet<u64>,
    rejected: HashSet<u64>,
    zeroed_assist: bool,
    direct_blocked: HashSet<u64>,
    refresh_offline: HashSet<u64>, // victims that stay offline
    pet_owners: HashMap<u64, u64>, // pet -> owner
    out_of_area: HashSet<u64>,
    invalid: HashSet<u64>,
    low_priority: HashSet<u64>,
    melee_reachable: HashSet<u64>,
    factions: HashMap<u64, u32>,
    log: RefCell<Vec<String>>,
}

impl FakeHost {
    fn raw(guid: ObjectGuid) -> u64 {
        guid.raw() - 0xF130_0000_0000_0000
    }
}

impl ThreatHost for FakeHost {
    fn spellmod_threat(&self, _hated: ObjectGuid, spell_id: u32, threat: f32) -> f32 {
        self.log
            .borrow_mut()
            .push(format!("spellmod:{spell_id}:{threat}"));
        threat + 1.0
    }

    fn crit_threat_multiplier(&self, _hated: ObjectGuid, _mask: u32) -> f32 {
        self.log.borrow_mut().push("critmult".into());
        2.0
    }

    fn apply_total_threat_modifier(&self, _hated: ObjectGuid, threat: f32, mask: u32) -> f32 {
        self.log.borrow_mut().push(format!("total:{threat}:{mask}"));
        threat
    }

    fn add_gate(&self, _owner: ObjectGuid, victim: ObjectGuid, is_assist: bool) -> AddDecision {
        if self.rejected.contains(&Self::raw(victim)) || self.gm.contains(&Self::raw(victim)) {
            AddDecision::Reject
        } else if is_assist && self.zeroed_assist {
            AddDecision::ZeroedAssist
        } else {
            AddDecision::Allow
        }
    }

    fn direct_gate(&self, _owner: ObjectGuid, victim: ObjectGuid) -> bool {
        !self.direct_blocked.contains(&Self::raw(victim))
    }

    fn is_gamemaster_player(&self, victim: ObjectGuid) -> bool {
        self.gm.contains(&Self::raw(victim))
    }

    fn refresh_online(&self, _owner: ObjectGuid, victim: ObjectGuid) -> bool {
        !self.refresh_offline.contains(&Self::raw(victim))
    }

    fn pet_owner_to_register(&self, _owner: ObjectGuid, victim: ObjectGuid) -> Option<ObjectGuid> {
        self.pet_owners.get(&Self::raw(victim)).map(|o| g(*o))
    }

    fn out_of_threat_area(&self, _attacker: ObjectGuid, target: ObjectGuid) -> bool {
        self.out_of_area.contains(&Self::raw(target))
    }

    fn valid_attack_target(&self, _attacker: ObjectGuid, target: ObjectGuid) -> bool {
        !self.invalid.contains(&Self::raw(target))
    }

    fn low_priority_target(&self, _attacker: ObjectGuid, target: ObjectGuid) -> bool {
        self.low_priority.contains(&Self::raw(target))
    }

    fn can_reach_with_melee(&self, _attacker: ObjectGuid, target: ObjectGuid) -> bool {
        self.melee_reachable.contains(&Self::raw(target))
    }

    fn faction_id(&self, unit: ObjectGuid) -> u32 {
        self.factions.get(&Self::raw(unit)).copied().unwrap_or(0)
    }
}

fn manager_with(host: &FakeHost, entries: &[(u64, f32)]) -> ThreatManager {
    let mut m = ThreatManager::new(g(MOB));
    for (victim, threat) in entries {
        m.add_threat_directly(host, g(*victim), *threat, false);
    }
    m
}

fn order(m: &ThreatManager) -> Vec<u64> {
    m.threat_list()
        .iter()
        .map(|r| FakeHost::raw(r.target()))
        .collect()
}

#[test]
fn add_clamps_at_zero_and_percent_rules() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0)]);

    m.add_threat_directly(&host, g(A), -150.0, false);
    assert_eq!(m.get_threat(g(A), false).to_bits(), 0.0f32.to_bits());

    m.add_threat_directly(&host, g(A), 100.0, false);
    m.modify_threat_percent(&host, g(A), 50);
    assert_eq!(m.get_threat(g(A), false), 150.0);
    m.modify_threat_percent(&host, g(A), -50);
    assert_eq!(m.get_threat(g(A), false), 75.0);

    // exactly -100: entry survives at 0 (no rounding path)
    m.modify_threat_percent(&host, g(A), -100);
    assert_eq!(m.get_threat(g(A), false).to_bits(), 0.0f32.to_bits());
    assert!(m.reference_by_target(g(A)).is_some());

    // below -100: reference removed and deleted
    m.add_threat_directly(&host, g(A), 10.0, false);
    m.modify_threat_percent(&host, g(A), -101);
    assert!(m.reference_by_target(g(A)).is_none());
    assert!(m.is_threat_list_empty());
}

#[test]
fn percent_uses_cpp_float_order() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 33.3)]);
    m.modify_threat_percent(&host, g(A), 33);
    let expected = 33.3f32 + (33.3f32 * 33.0 / 100.0);
    assert_eq!(m.get_threat(g(A), false).to_bits(), expected.to_bits());
}

#[test]
fn dirty_flag_quadrants_and_update_semantics() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0), (B, 90.0)]);
    m.set_current_victim_if_can(g(A));
    m.set_dirty(false);

    // raise current: no dirty
    m.add_threat_directly(&host, g(A), 10.0, false);
    assert!(!m.is_dirty());
    // lower non-current: no dirty
    m.add_threat_directly(&host, g(B), -10.0, false);
    assert!(!m.is_dirty());
    // raise non-current: dirty
    m.add_threat_directly(&host, g(B), 5.0, false);
    assert!(m.is_dirty());
    m.set_dirty(false);
    // lower current: dirty
    m.add_threat_directly(&host, g(A), -5.0, false);
    assert!(m.is_dirty());
    // zero delta never fires the event
    m.set_dirty(false);
    m.add_threat_directly(&host, g(B), 0.0, false);
    assert!(!m.is_dirty());

    // update sorts descending, stable on ties, clears dirty
    let mut m = manager_with(&host, &[(A, 50.0), (B, 50.0), (C, 80.0), (D, 50.0)]);
    m.set_dirty(true);
    m.update();
    assert_eq!(order(&m), vec![C, A, B, D], "desc, stable ties");
    assert!(!m.is_dirty());

    // dirty cleared even with <= 1 entries
    let mut m = manager_with(&host, &[(A, 1.0)]);
    m.set_dirty(true);
    m.update();
    assert!(!m.is_dirty());

    // most_hated does NOT sort (raw front, as in C++)
    let m = manager_with(&host, &[(A, 1.0), (B, 99.0)]);
    assert_eq!(m.most_hated().map(|r| FakeHost::raw(r.target())), Some(A));
}

#[test]
fn online_offline_moves_and_current_clearing() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0), (B, 90.0), (C, 80.0)]);
    m.set_current_victim_if_can(g(A));
    m.set_dirty(false);

    m.set_online_offline_state_of(g(A), false);
    assert_eq!(m.current_victim(), None, "current cleared");
    assert!(m.is_dirty());
    assert_eq!(order(&m), vec![B, C]);
    assert_eq!(m.offline_list().len(), 1);
    let off = m.offline_list()[0];
    assert!(
        !off.online() && !off.accessible(),
        "offline => inaccessible"
    );

    // threat lookup honours the offline flag
    assert_eq!(m.get_threat(g(A), false), 0.0);
    assert_eq!(m.get_threat(g(A), true), 100.0);

    // modify_threat_percent is online-only
    m.modify_threat_percent(&host, g(A), 100);
    assert_eq!(m.get_threat(g(A), true), 100.0);

    // coming back online APPENDS to the online list end
    m.set_current_victim_if_can(g(B)); // B at 90
    m.set_dirty(false);
    m.set_online_offline_state_of(g(A), true);
    assert_eq!(order(&m), vec![B, C, A], "push_back order");
    assert!(m.is_dirty(), "100 > 1.1 * 90 = 99");

    // no state change => no-op
    m.set_dirty(false);
    m.set_online_offline_state_of(g(A), true);
    assert!(!m.is_dirty());
}

#[test]
fn offline_add_refreshes_back_online() {
    let mut host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 50.0), (B, 40.0)]);
    m.set_online_offline_state_of(g(A), false);

    // refresh says offline stays: threat applies, stays offline
    host.refresh_offline.insert(A);
    m.add_threat_directly(&host, g(A), 10.0, false);
    assert_eq!(m.get_threat(g(A), true), 60.0);
    assert_eq!(m.offline_list().len(), 1);

    // refresh now online: moves during the add
    host.refresh_offline.remove(&A);
    m.add_threat_directly(&host, g(A), 10.0, false);
    assert_eq!(m.get_threat(g(A), false), 70.0);
    assert!(m.offline_list().is_empty());
    assert_eq!(order(&m), vec![B, A], "appended at end on move");
}

#[test]
fn direct_add_creation_rules() {
    let mut host = FakeHost::default();
    let mut m = ThreatManager::new(g(MOB));

    // no_new: nothing created
    m.add_threat_directly(&host, g(A), 10.0, true);
    assert!(m.is_threat_list_empty());

    // created online+accessible, real threat applied
    m.add_threat_directly(&host, g(A), 10.0, false);
    let r = m.reference_by_target(g(A)).unwrap();
    assert!(r.online() && r.accessible());
    assert_eq!(r.threat(), 10.0);

    // GM victim (direct path): created then forced offline
    host.gm.insert(B);
    m.add_threat_directly(&host, g(B), 10.0, false);
    assert!(m.reference_by_target(g(B)).is_none());
    assert_eq!(m.get_threat(g(B), true), 10.0);

    // self and direct-blocked rejected
    m.add_threat_directly(&host, g(MOB), 10.0, false);
    host.direct_blocked.insert(C);
    m.add_threat_directly(&host, g(C), 10.0, false);
    assert_eq!(m.threat_list().len(), 1);
}

#[test]
fn calc_threat_shape_and_gated_add() {
    let host = FakeHost::default();
    let spell = ThreatSpell {
        id: 42,
        school_mask: 0x20,
        no_threat: false,
        no_helpful_threat: false,
    };

    // zero threat: early-out, no host modifier calls
    assert_eq!(calc_threat(&host, g(A), 0.0, true, 0x20, Some(&spell)), 0.0);
    assert!(host.log.borrow().is_empty());

    // spellmod (+1) then crit (x2), then total (identity)
    let v = calc_threat(&host, g(A), 10.0, true, 0x20, Some(&spell));
    assert_eq!(v, 22.0);
    assert_eq!(
        &*host.log.borrow(),
        &["spellmod:42:10", "critmult", "total:22:32"]
    );

    // no spell: only total modifier
    host.log.borrow_mut().clear();
    let v = calc_threat(&host, g(A), 10.0, true, 0x20, None);
    assert_eq!(v, 10.0);
    assert_eq!(&*host.log.borrow(), &["total:10:32"]);

    // gated add: rejected victim
    let mut host = FakeHost::default();
    host.rejected.insert(A);
    let mut m = ThreatManager::new(g(MOB));
    m.add_threat_simple(&host, g(A), 100.0);
    assert!(m.is_threat_list_empty());

    // no_threat spell cannot create a reference but adds to existing
    let host = FakeHost::default();
    let no_threat_spell = ThreatSpell {
        no_threat: true,
        ..spell
    };
    m.add_threat(
        &host,
        g(A),
        10.0,
        false,
        0x20,
        Some(&no_threat_spell),
        false,
    );
    assert!(m.is_threat_list_empty(), "no_new applied");
    m.add_threat_directly(&host, g(A), 5.0, false);
    m.add_threat(
        &host,
        g(A),
        10.0,
        false,
        0x20,
        Some(&no_threat_spell),
        false,
    );
    assert_eq!(m.get_threat(g(A), false), 16.0, "5 + (10+1 spellmod)");

    // zeroed assist still creates the ref at 0 threat
    let host = FakeHost {
        zeroed_assist: true,
        ..FakeHost::default()
    };
    let mut m = ThreatManager::new(g(MOB));
    m.add_threat(&host, g(A), 100.0, false, 0, None, true);
    assert_eq!(m.get_threat(g(A), false), 0.0);
    assert!(m.reference_by_target(g(A)).is_some());
}

#[test]
fn pet_owner_registered_on_positive_threat() {
    let mut host = FakeHost::default();
    host.pet_owners.insert(A, B); // A is B's pet
    let mut m = ThreatManager::new(g(MOB));
    m.add_threat_directly(&host, g(A), 10.0, false);
    assert!(m.reference_by_target(g(B)).is_some(), "owner ref created");
    assert_eq!(m.get_threat(g(B), false), 0.0);

    // negative delta does not register (C: pet of D)
    host.pet_owners.insert(C, D);
    m.add_threat_directly(&host, g(C), 10.0, false); // creates D too (positive)
    m.remove_reference(g(D));
    m.add_threat_directly(&host, g(C), -5.0, false);
    assert!(m.reference_by_target(g(D)).is_none());
}

fn selected(host: &FakeHost, entries: &[(u64, f32)], current: Option<u64>) -> Option<u64> {
    let mut m = manager_with(host, entries);
    if let Some(c) = current {
        m.set_current_victim_if_can(g(c));
    }
    m.update();
    m.select_next_victim(host).map(FakeHost::raw)
}

#[test]
fn select_next_victim_rules() {
    let host = FakeHost::default();
    // no current: highest valid
    assert_eq!(selected(&host, &[(B, 90.0), (A, 100.0)], None), Some(A));
    // current on top: stays
    assert_eq!(selected(&host, &[(A, 100.0), (B, 90.0)], Some(A)), Some(A));
    // challenger exactly 110%: stays (<= 1.1 rule)
    assert_eq!(selected(&host, &[(B, 110.0), (A, 100.0)], Some(A)), Some(A));
    // challenger 120% without melee reach: scan reaches current -> stays
    assert_eq!(selected(&host, &[(B, 120.0), (A, 100.0)], Some(A)), Some(A));
    // challenger >130%: switches at range
    assert_eq!(selected(&host, &[(B, 131.0), (A, 100.0)], Some(A)), Some(B));

    // challenger 120% WITH melee reach: switches (110% melee rule)
    let mut host2 = FakeHost::default();
    host2.melee_reachable.insert(B);
    assert_eq!(
        selected(&host2, &[(B, 120.0), (A, 100.0)], Some(A)),
        Some(B)
    );

    // first entry out of threat area: immediate None
    let mut host3 = FakeHost::default();
    host3.out_of_area.insert(A);
    assert_eq!(selected(&host3, &[(A, 100.0), (B, 90.0)], Some(B)), None);

    // all invalid: None
    let mut host4 = FakeHost::default();
    host4.invalid.extend([A, B]);
    assert_eq!(selected(&host4, &[(A, 100.0), (B, 90.0)], Some(A)), None);

    // invalid current dropped mid-scan: next valid chosen freely
    let mut host5 = FakeHost::default();
    host5.invalid.insert(A);
    assert_eq!(selected(&host5, &[(A, 100.0), (B, 90.0)], Some(A)), Some(B));
}

#[test]
fn select_next_victim_low_priority_two_passes() {
    let mut host = FakeHost::default();
    host.low_priority.insert(A);
    // low-priority top skipped on pass 1
    assert_eq!(selected(&host, &[(A, 200.0), (B, 90.0)], None), Some(B));
    // ... but chosen on pass 2 when it is the only option
    assert_eq!(selected(&host, &[(A, 200.0)], None), Some(A));
    // low-priority CURRENT dropped from comparison in pass 1:
    // B at 150 would lose to A's 200 under the 110% rule if A
    // still counted; dropped, so B wins immediately.
    assert_eq!(selected(&host, &[(A, 200.0), (B, 150.0)], Some(A)), Some(B));
}

#[test]
fn get_hostile_target_updates_current() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0), (B, 90.0)]);
    assert_eq!(m.get_hostile_target(&host).map(FakeHost::raw), Some(A));
    m.add_threat_directly(&host, g(B), 50.0, false); // B = 140 > 130
    assert_eq!(m.get_hostile_target(&host).map(FakeHost::raw), Some(B));
    assert_eq!(m.current_victim().map(FakeHost::raw), Some(B));
}

#[test]
fn taunt_temp_threat_cycle() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0), (B, 40.0)]);
    m.set_current_victim_if_can(g(A));

    m.taunt_apply(&host, g(B));
    let r = m.reference_by_target(g(B)).unwrap();
    assert_eq!(r.threat(), 100.0, "taunter raised to current");
    assert_eq!(r.temp_modifier(), 60.0);

    // second taunt: temp already active => no-op even if current grew
    m.add_threat_directly(&host, g(A), 50.0, false);
    m.taunt_apply(&host, g(B));
    assert_eq!(m.reference_by_target(g(B)).unwrap().threat(), 100.0);

    m.taunt_fade_out(&host, g(B));
    let r = m.reference_by_target(g(B)).unwrap();
    assert_eq!(r.threat(), 40.0, "fade restores");
    assert_eq!(r.temp_modifier(), 0.0);

    // taunt from the top (no current above): no-op
    m.taunt_apply(&host, g(A));
    assert_eq!(m.reference_by_target(g(A)).unwrap().temp_modifier(), 0.0);
}

#[test]
fn temp_threat_can_lower_and_restore() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0)]);
    m.set_temp_threat_of(&host, g(A), 25.0);
    let r = m.reference_by_target(g(A)).unwrap();
    assert_eq!(r.threat(), 25.0);
    assert_eq!(r.temp_modifier(), -75.0);
    m.reset_temp_threat_of(&host, g(A));
    assert_eq!(m.reference_by_target(g(A)).unwrap().threat(), 100.0);
}

struct MapResolver {
    map: HashMap<u64, ThreatManager>,
}

impl ThreatManagerResolver for MapResolver {
    fn manager_mut(&mut self, owner: ObjectGuid) -> Option<&mut ThreatManager> {
        self.map.get_mut(&FakeHost::raw(owner))
    }
}

fn three_haters(host: &FakeHost) -> (MapResolver, HatedByList) {
    let mut map = HashMap::new();
    let mut list = HatedByList::new(g(A)); // A is the hated unit (e.g. healer)
    for mob in [20u64, 21, 22] {
        let mut m = ThreatManager::new(g(mob));
        m.add_threat_directly(host, g(A), 10.0, false);
        map.insert(mob, m);
        list.note_hater(g(mob));
    }
    (MapResolver { map }, list)
}

#[test]
fn threat_assist_divides_and_respects_spell_flag() {
    let host = FakeHost::default();
    let (mut res, list) = three_haters(&host);

    // 90 split across 3 haters: +30 each on victim B
    list.threat_assist(&host, &mut res, g(B), 90.0, None, false);
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(B), false), 30.0, "mob {mob}");
    }

    // single target: no division
    list.threat_assist(&host, &mut res, g(B), 90.0, None, true);
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(B), false), 120.0, "mob {mob}");
    }

    // NO_HELPFUL_THREAT: nothing happens
    let spell = ThreatSpell {
        id: 7,
        school_mask: 0x20,
        no_threat: false,
        no_helpful_threat: true,
    };
    list.threat_assist(&host, &mut res, g(B), 90.0, Some(&spell), false);
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(B), false), 120.0, "mob {mob}");
    }
}

#[test]
fn hated_by_fan_outs_and_deletes() {
    let mut host = FakeHost::default();
    host.factions.insert(21, 77);
    let (mut res, mut list) = three_haters(&host);

    // percent fan-out
    list.add_threat_percent_all(&host, &mut res, 100); // 10 -> 20 each
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(A), false), 20.0);
    }

    // temp-threat fan-out applies once, reset restores
    list.add_temp_threat_all(&host, &mut res, -5.0, true);
    list.add_temp_threat_all(&host, &mut res, -5.0, true);
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(A), false), 15.0);
    }
    list.add_temp_threat_all(&host, &mut res, 0.0, false);
    for mob in [20, 21, 22] {
        assert_eq!(res.map[&mob].get_threat(g(A), false), 20.0);
    }

    // faction delete: only mob 21 loses its reference
    list.delete_references_for_faction(&host, &mut res, 77);
    assert_eq!(list.attackers().len(), 2);
    assert!(res.map[&21].find_reference(g(A)).is_none());
    assert!(res.map[&20].find_reference(g(A)).is_some());

    // delete all
    list.delete_references(&mut res);
    assert!(list.is_empty());
    assert!(res.map[&20].find_reference(g(A)).is_none());
}

#[test]
fn remove_reference_clears_current_and_marks_dirty() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 100.0), (B, 50.0)]);
    m.set_current_victim_if_can(g(A));
    m.set_dirty(false);
    m.remove_reference(g(A));
    assert_eq!(m.current_victim(), None);
    assert!(m.is_dirty());
    assert_eq!(order(&m), vec![B]);

    // removing a non-current ref does not dirty
    let mut m = manager_with(&host, &[(A, 100.0), (B, 50.0)]);
    m.set_current_victim_if_can(g(A));
    m.set_dirty(false);
    m.remove_reference(g(B));
    assert_eq!(m.current_victim(), Some(g(A)));
    assert!(!m.is_dirty());
}

#[test]
fn on_target_destroyed_goes_offline() {
    let host = FakeHost::default();
    let mut m = manager_with(&host, &[(A, 10.0)]);
    m.on_target_destroyed(g(A));
    assert!(m.threat_list().is_empty());
    assert_eq!(m.get_threat(g(A), true), 10.0);
}
