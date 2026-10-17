//! Formation state and slot math (`PlayerBots/BotFormation.{h,cpp}`,
//! minus the `Group`/`Player`/`Map` reach-ins, which are the
//! [`FormationHost`] trait plus plain inputs).

use std::collections::HashMap;

use mangos_guid::ObjectGuid;
use mangos_movement::Vec3;

use crate::Role;
use crate::breadcrumb::BreadcrumbTrail;

/// `BotFormationDefaults` constants (yards).
pub mod defaults {
    /// Tank trail distance behind leader.
    pub const TANK_TRAIL_DIST: f32 = 2.0;
    /// Melee DPS trail distance.
    pub const MELEE_TRAIL_DIST: f32 = 4.0;
    /// Ranged DPS trail distance.
    pub const RANGED_TRAIL_DIST: f32 = 8.0;
    /// Healer trail distance.
    pub const HEALER_TRAIL_DIST: f32 = 10.0;
    /// Melee lateral offset from trail center.
    pub const MELEE_LATERAL: f32 = 1.0;
    /// Ranged lateral offset.
    pub const RANGED_LATERAL: f32 = 1.0;
    /// Flee search starts this far back.
    pub const FLEE_MIN_DIST: f32 = 15.0;
    /// Flee search stops here.
    pub const FLEE_MAX_DIST: f32 = 30.0;
    /// Hostile scan radius around flee candidates.
    pub const FLEE_HOSTILE_SCAN: f32 = 12.0;
    /// Position dead-zone to prevent jitter (behaviour layer).
    pub const MOVE_DEADZONE: f32 = 2.0;
    /// Aggro safety scan radius for lateral slot validation.
    pub const SLOT_HOSTILE_SCAN: f32 = 12.0;
}

/// One bot's assigned position in the column (`FormationSlot`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormationSlot {
    pub slot_index: u8,
    pub role: Role,
    /// Trail arc-distance behind the leader.
    pub trail_dist_behind: f32,
    /// +right / −left relative to trail direction.
    pub lateral_offset: f32,
    pub assigned_bot: ObjectGuid,
}

/// The two world queries formation math needs.
pub trait FormationHost {
    /// `Map::GetWalkHitPosition(nullptr, from, to...)`: the
    /// navmesh-adjusted `to`, or `None` when unwalkable.
    fn walk_hit_position(&self, from: Vec3, to: Vec3) -> Option<Vec3>;
    /// `HasHostilesNearPoint(bot, map, at, range)`.
    fn hostiles_near(&self, bot: ObjectGuid, at: Vec3, range: f32) -> bool;
}

/// A group member for slot assignment: `bot_role: None` marks a
/// human (or non-party-bot) member, who gets no slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberInfo {
    pub guid: ObjectGuid,
    pub bot_role: Option<Role>,
}

/// Per-group trail + slot assignments (`FormationState`).
#[derive(Clone, Debug, Default)]
pub struct FormationState {
    pub trail: BreadcrumbTrail,
    pub slots: Vec<FormationSlot>,
    pub leader_guid: ObjectGuid,
    pub last_group_size: u8,
}

impl FormationState {
    /// `AssignSlots`: rebuild slot assignments from group
    /// composition (iteration order preserved; humans skipped).
    /// The C++ also stamps each bot AI's slot index — look it up
    /// by GUID in [`FormationState::slots`].
    #[allow(clippy::cast_possible_truncation)]
    pub fn assign_slots(&mut self, members: &[MemberInfo]) {
        self.slots.clear();
        self.last_group_size = members.len() as u8;

        let mut melee_count = 0u8;
        let mut ranged_count = 0u8;
        for member in members {
            let Some(role) = member.bot_role else {
                continue;
            };
            let (trail_dist_behind, lateral_offset) = match role {
                Role::Tank => (defaults::TANK_TRAIL_DIST, 0.0),
                Role::MeleeDps | Role::Invalid => {
                    // Invalid is treated as melee DPS, as in C++
                    let lat = if melee_count % 2 == 0 {
                        defaults::MELEE_LATERAL
                    } else {
                        -defaults::MELEE_LATERAL
                    };
                    melee_count += 1;
                    (defaults::MELEE_TRAIL_DIST, lat)
                }
                Role::RangeDps => {
                    let lat = if ranged_count % 2 == 0 {
                        defaults::RANGED_LATERAL
                    } else {
                        -defaults::RANGED_LATERAL
                    };
                    ranged_count += 1;
                    (defaults::RANGED_TRAIL_DIST, lat)
                }
                Role::Healer => (defaults::HEALER_TRAIL_DIST, 0.0),
            };
            self.slots.push(FormationSlot {
                slot_index: self.slots.len() as u8,
                role,
                trail_dist_behind,
                lateral_offset,
                assigned_bot: member.guid,
            });
        }
    }

    /// `CalculateSlotPosition`. `bot: None` skips the hostile
    /// scan, as the C++ null check does.
    #[must_use]
    pub fn calculate_slot_position(
        &self,
        host: &dyn FormationHost,
        slot_index: u32,
        bot: Option<ObjectGuid>,
    ) -> Option<Vec3> {
        let slot = self.slots.get(slot_index as usize)?;
        if self.trail.is_empty() {
            return None;
        }

        // 1. Base trail point at this slot's distance behind.
        let base = self.trail.get_point_at_distance(slot.trail_dist_behind)?;
        let center = Vec3::new(base.x, base.y, base.z);

        // 2. No lateral offset: done (tank/healer centers).
        if slot.lateral_offset.abs() < 0.01 {
            return Some(center);
        }

        // 3. Trail direction for the perpendicular vector.
        let (dir_x, dir_y) = match self
            .trail
            .get_point_at_distance(slot.trail_dist_behind + 1.0)
        {
            Some(prev) => (base.x - prev.x, base.y - prev.y),
            None => (base.orientation.cos(), base.orientation.sin()),
        };

        let len = (dir_x * dir_x + dir_y * dir_y).sqrt();
        if len < 0.001 {
            return Some(center); // degenerate — stay at center
        }
        let (dir_x, dir_y) = (dir_x / len, dir_y / len);

        // 4. Perpendicular (rotate 90° clockwise in XY).
        let (perp_x, perp_y) = (dir_y, -dir_x);

        // 5. Candidate lateral position.
        let candidate = Vec3::new(
            base.x + perp_x * slot.lateral_offset,
            base.y + perp_y * slot.lateral_offset,
            base.z,
        );

        // 6. Navmesh validate from trail center to candidate.
        let Some(test) = host.walk_hit_position(center, candidate) else {
            return Some(center);
        };
        if (test - candidate).length() > 3.0 {
            return Some(center); // corridor too narrow
        }

        // 7. Aggro safety: hostiles near the candidate collapse
        // to trail center to avoid pulls.
        if let Some(bot) = bot {
            if host.hostiles_near(bot, test, defaults::SLOT_HOSTILE_SCAN) {
                return Some(center);
            }
        }
        Some(test)
    }

    /// `CalculateFleePoint`: walk backward from the bot's nearest
    /// trail point in 5 yd steps (15–30 yd) toward cleared area.
    #[must_use]
    pub fn calculate_flee_point(
        &self,
        host: &dyn FormationHost,
        bot: ObjectGuid,
        bot_pos: Vec3,
    ) -> Option<Vec3> {
        if self.trail.size() < 2 {
            return None;
        }
        let nearest = self
            .trail
            .find_nearest_index(bot_pos.x, bot_pos.y, bot_pos.z);

        let mut search_dist = defaults::FLEE_MIN_DIST;
        while search_dist <= defaults::FLEE_MAX_DIST {
            let candidate = self.trail.get_point_further_back(nearest, search_dist)?;
            let at = Vec3::new(candidate.x, candidate.y, candidate.z);
            if !host.hostiles_near(bot, at, defaults::FLEE_HOSTILE_SCAN) {
                if let Some(test) = host.walk_hit_position(bot_pos, at) {
                    return Some(test);
                }
            }
            search_dist += 5.0;
        }
        None // caller uses the run-away fallback
    }
}

/// Formations keyed by leader GUID (`BotFormationMgr`, minus the
/// singleton: owned by the future `BotSystem`).
#[derive(Clone, Debug, Default)]
pub struct FormationManager {
    formations: HashMap<u64, FormationState>,
}

impl FormationManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `GetFormation`: get or create.
    pub fn formation(&mut self, leader: ObjectGuid) -> &mut FormationState {
        let state = self.formations.entry(leader.raw()).or_default();
        state.leader_guid = leader;
        state
    }

    /// Read-only lookup (no creation).
    #[must_use]
    pub fn get(&self, leader: ObjectGuid) -> Option<&FormationState> {
        self.formations.get(&leader.raw())
    }

    /// `RemoveFormation`.
    pub fn remove_formation(&mut self, leader: ObjectGuid) {
        self.formations.remove(&leader.raw());
    }

    /// `UpdateTrail(leaderGuid, pLeader, now)` with the leader's
    /// position passed as data.
    pub fn update_trail(&mut self, leader: ObjectGuid, pos: Vec3, orientation: f32, now: u32) {
        self.formation(leader)
            .trail
            .record_point(pos.x, pos.y, pos.z, orientation, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: u64) -> ObjectGuid {
        ObjectGuid::from_raw(n)
    }

    #[derive(Default)]
    struct FakeHost {
        walkable: bool,
        drift: f32,
        hostiles: bool,
    }

    impl FormationHost for FakeHost {
        fn walk_hit_position(&self, _from: Vec3, to: Vec3) -> Option<Vec3> {
            self.walkable
                .then_some(Vec3::new(to.x + self.drift, to.y, to.z))
        }

        fn hostiles_near(&self, _bot: ObjectGuid, _at: Vec3, _range: f32) -> bool {
            self.hostiles
        }
    }

    /// Trail straight +x, points every 3 yd, newest at 3*(n-1).
    #[allow(clippy::cast_precision_loss)]
    fn state_with_trail(n: u32) -> FormationState {
        let mut fs = FormationState::default();
        for i in 0..n {
            fs.trail
                .record_point(3.0 * i as f32, 0.0, 0.0, 0.0, 1000 * (i + 1));
        }
        fs
    }

    #[test]
    fn assign_slots_roles_laterals_and_humans() {
        let mut fs = FormationState::default();
        let members = [
            MemberInfo {
                guid: g(1),
                bot_role: None,
            }, // human leader: skipped
            MemberInfo {
                guid: g(2),
                bot_role: Some(Role::Tank),
            },
            MemberInfo {
                guid: g(3),
                bot_role: Some(Role::MeleeDps),
            },
            MemberInfo {
                guid: g(4),
                bot_role: Some(Role::MeleeDps),
            },
            MemberInfo {
                guid: g(5),
                bot_role: Some(Role::RangeDps),
            },
            MemberInfo {
                guid: g(6),
                bot_role: Some(Role::Invalid),
            }, // counts as melee
            MemberInfo {
                guid: g(7),
                bot_role: Some(Role::Healer),
            },
        ];
        fs.assign_slots(&members);
        assert_eq!(fs.last_group_size, 7);
        assert_eq!(fs.slots.len(), 6);
        let got: Vec<(u64, f32, f32)> = fs
            .slots
            .iter()
            .map(|s| (s.assigned_bot.raw(), s.trail_dist_behind, s.lateral_offset))
            .collect();
        assert_eq!(
            got,
            vec![
                (2, 2.0, 0.0),
                (3, 4.0, 1.0),
                (4, 4.0, -1.0),
                (5, 8.0, 1.0),
                (6, 4.0, 1.0), // third "melee" (Invalid) alternates back to +1
                (7, 10.0, 0.0),
            ]
        );
        assert_eq!(fs.slots[5].slot_index, 5);
    }

    #[test]
    fn slot_position_center_and_lateral() {
        let mut fs = state_with_trail(8); // newest x=21, 21 yd of trail
        fs.assign_slots(&[
            MemberInfo {
                guid: g(2),
                bot_role: Some(Role::Tank),
            },
            MemberInfo {
                guid: g(3),
                bot_role: Some(Role::MeleeDps),
            },
        ]);
        let host = FakeHost {
            walkable: true,
            drift: 0.0,
            hostiles: false,
        };

        // Tank: 2 yd behind newest (x=19), center line.
        let tank = fs.calculate_slot_position(&host, 0, Some(g(2))).unwrap();
        assert_eq!(
            (tank.x.to_bits(), tank.y.to_bits()),
            (19.0f32.to_bits(), 0.0f32.to_bits())
        );

        // Melee: 4 yd behind -> base x=17; trail dir is +x,
        // perp = (0,-1); lateral +1 -> y=-1.
        let melee = fs.calculate_slot_position(&host, 1, Some(g(3))).unwrap();
        assert_eq!(melee.x.to_bits(), 17.0f32.to_bits());
        assert_eq!(melee.y.to_bits(), (-1.0f32).to_bits());

        // Unwalkable candidate collapses to center.
        let blocked = FakeHost {
            walkable: false,
            ..FakeHost::default()
        };
        let p = fs.calculate_slot_position(&blocked, 1, Some(g(3))).unwrap();
        assert_eq!(
            (p.x.to_bits(), p.y.to_bits()),
            (17.0f32.to_bits(), 0.0f32.to_bits())
        );

        // Excess drift collapses to center.
        let drifting = FakeHost {
            walkable: true,
            drift: 3.5,
            hostiles: false,
        };
        let p = fs
            .calculate_slot_position(&drifting, 1, Some(g(3)))
            .unwrap();
        assert_eq!(
            (p.x.to_bits(), p.y.to_bits()),
            (17.0f32.to_bits(), 0.0f32.to_bits())
        );

        // Hostiles near candidate collapse to center (but skipped for None bot).
        let danger = FakeHost {
            walkable: true,
            drift: 0.0,
            hostiles: true,
        };
        let p = fs.calculate_slot_position(&danger, 1, Some(g(3))).unwrap();
        assert_eq!(
            (p.x.to_bits(), p.y.to_bits()),
            (17.0f32.to_bits(), 0.0f32.to_bits())
        );
        let p = fs.calculate_slot_position(&danger, 1, None).unwrap();
        assert_eq!(p.y.to_bits(), (-1.0f32).to_bits());

        assert!(fs.calculate_slot_position(&host, 9, None).is_none());
    }

    /// Unsafe above x=10, everything walkable.
    struct FleeHost;
    impl FormationHost for FleeHost {
        fn walk_hit_position(&self, _from: Vec3, to: Vec3) -> Option<Vec3> {
            Some(to)
        }
        fn hostiles_near(&self, _bot: ObjectGuid, at: Vec3, _range: f32) -> bool {
            at.x > 10.0
        }
    }

    /// Every point hostile.
    struct AllHostile;
    impl FormationHost for AllHostile {
        fn walk_hit_position(&self, _from: Vec3, to: Vec3) -> Option<Vec3> {
            Some(to)
        }
        fn hostiles_near(&self, _bot: ObjectGuid, _at: Vec3, _range: f32) -> bool {
            true
        }
    }

    #[test]
    fn flee_point_steps_back_to_safety() {
        let fs = state_with_trail(12); // newest x=33
        // Bot at newest (x=33): 15 yd back -> x=18 (unsafe), 20 -> 13
        // (unsafe), 25 -> 8 (safe).
        let p = fs
            .calculate_flee_point(&FleeHost, g(2), Vec3::new(33.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(p.x.to_bits(), 8.0f32.to_bits());

        // Short trail (< 15 yd): GetPointFurtherBack clamps to the
        // oldest point, which is then the (safe) candidate.
        let fs2 = state_with_trail(4);
        let p = fs2
            .calculate_flee_point(&FleeHost, g(2), Vec3::new(9.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(
            p.x.to_bits(),
            0.0f32.to_bits(),
            "clamped to oldest trail point"
        );

        // Every candidate hostile -> None (run-away fallback).
        assert!(
            fs.calculate_flee_point(&AllHostile, g(2), Vec3::new(33.0, 0.0, 0.0))
                .is_none()
        );
    }

    #[test]
    fn manager_keyed_create_remove() {
        let mut mgr = FormationManager::new();
        mgr.update_trail(g(1), Vec3::new(0.0, 0.0, 0.0), 0.0, 1000);
        mgr.update_trail(g(1), Vec3::new(3.0, 0.0, 0.0), 0.0, 2000);
        assert_eq!(mgr.get(g(1)).unwrap().trail.size(), 2);
        assert!(mgr.get(g(2)).is_none());
        mgr.remove_formation(g(1));
        assert!(mgr.get(g(1)).is_none());
    }
}
