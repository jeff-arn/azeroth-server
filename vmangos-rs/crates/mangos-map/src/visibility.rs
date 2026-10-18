//! Visibility predicates: the Slice 3 subset of
//! `IsWithinVisibilityDistanceOf` (`Object.cpp:2195-2215`),
//! `Unit::IsVisibleForOrDetect` (`Unit.cpp:6481-6640`), the
//! two `IsVisibleInGridForPlayer` bodies, and
//! `GameObject::IsVisibleForInState`
//! (`GameObject.cpp:1090-1125`).
//!
//! Included: alive/dead grid rules, ghost friendliness via
//! team, corpse-proximity sight, extra-flag invisibility,
//! visible-to-ghosts static flag, server-only gameobjects,
//! spawn state, and grey-distance thresholds. Excluded
//! until their systems exist (recorded): stealth and
//! invisibility masks, GM visibility, hunter's mark,
//! factions reactions for the only-visible-to-friendly
//! flag (no bench template sets it), transports, taxi
//! flight distances, phasing (`CanSeeInWorld` world-mask
//! test IS included — it is just a mask AND).

use mangos_entities::creature::Creature;
use mangos_entities::gameobject::{GameObject, is_server_only};
use mangos_entities::player::Player;
use mangos_entities::unit::{DeathState, Team};
use mangos_movement::movement_info::Position;

/// `CREATURE_FLAG_EXTRA_INVISIBLE`.
const FLAG_EXTRA_INVISIBLE: u32 = 0x0000_0080;
/// `CREATURE_STATIC_FLAG_VISIBLE_TO_GHOSTS`.
const STATIC_FLAG_VISIBLE_TO_GHOSTS: u32 = 0x0020_0000;
// Aggro-rate-scaled corpse sight uses `(20 + 25) * rate`
// in C++; the rate is a map input, see `aggro_rate`.

/// 2D distance (`IsWithinDistInMap(..., is3D=false)`
/// measures between positions, bounding radii excluded at
/// this call shape in `Object.cpp:2202`? — the C++ body
/// uses `GetDistance2d` on positions: plain Euclidean).
#[must_use]
pub fn dist2d(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Squared 3D distance for the relocation trigger
/// (`GetDistanceSq` in `Unit::OnRelocated`).
#[must_use]
pub fn dist_sq(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    dx * dx + dy * dy + dz * dz
}

/// `WorldObject::CanSeeInWorld`: phasing masks intersect.
#[must_use]
pub fn can_see_in_world(viewer_mask: u32, target_mask: u32) -> bool {
    viewer_mask & target_mask != 0
}

/// Unit distance gate (`IsWithinVisibilityDistanceOf` for
/// units): `dist <= max(visibility + (known ? grey : 0),
/// target_modifier)`.
#[must_use]
pub fn within_unit_visibility(
    dist: f32,
    visibility: f32,
    grey: f32,
    in_visible_list: bool,
    target_modifier: f32,
) -> bool {
    let limit = (visibility + if in_visible_list { grey } else { 0.0 }).max(target_modifier);
    dist <= limit
}

/// `Player::IsVisibleInGridForPlayer` subset (GM and raid
/// cases excluded; friendliness reduces to same team,
/// which is what `IsFriendlyTo` yields for players without
/// faction overrides).
#[must_use]
pub fn player_visible_in_grid(
    target: &Player,
    viewer: &Player,
    viewer_corpse: Option<Position>,
) -> bool {
    let viewer_dead = viewer.unit.death_state == DeathState::Dead;
    if !viewer_dead {
        return target.unit.death_state != DeathState::Dead;
    }
    if target.unit.death_state == DeathState::Dead && target.team == viewer.team {
        return true;
    }
    if target.unit.is_alive() {
        if let Some(corpse) = viewer_corpse {
            // Nostalrius fix in this tree: fixed 100.
            return dist2d(corpse, target.unit.world.position()) <= 100.0;
        }
    }
    false
}

/// `Creature::IsVisibleInGridForPlayer` subset (GM case
/// excluded; death timer is 0 in Slice 3).
#[must_use]
pub fn creature_visible_in_grid(
    target: &Creature,
    viewer: &Player,
    viewer_corpse: Option<Position>,
    aggro_rate: f32,
) -> bool {
    if target.info.flags_extra & FLAG_EXTRA_INVISIBLE != 0 {
        return false;
    }
    if viewer.unit.is_alive() {
        return target.unit.is_alive()
            || target.corpse_decay_timer > 0
            || (target.is_dead_by_default && target.unit.death_state == DeathState::Corpse);
    }
    if target.unit.is_alive() {
        if let Some(corpse) = viewer_corpse {
            return dist2d(corpse, target.unit.world.position()) <= 45.0 * aggro_rate;
        }
    }
    target.info.static_flags1 & STATIC_FLAG_VISIBLE_TO_GHOSTS != 0
}

/// Unit state gate for a player viewer: phasing + grid
/// rule. (Stealth/invisibility/GM arms land with auras.)
#[must_use]
pub fn unit_visible_for_player(
    viewer: &Player,
    viewer_corpse: Option<Position>,
    target_is_player: Option<&Player>,
    target_creature: Option<&Creature>,
    aggro_rate: f32,
) -> bool {
    if let Some(target) = target_is_player {
        if !can_see_in_world(viewer.unit.world.world_mask, target.unit.world.world_mask) {
            return false;
        }
        return player_visible_in_grid(target, viewer, viewer_corpse);
    }
    if let Some(target) = target_creature {
        if !can_see_in_world(viewer.unit.world.world_mask, target.unit.world.world_mask) {
            return false;
        }
        // Invisible-for-alive symmetry is aura-driven
        // (ghost aura 9036 etc.); no auras exist yet.
        return creature_visible_in_grid(target, viewer, viewer_corpse, aggro_rate);
    }
    false
}

/// `GameObject::IsVisibleForInState` subset: the GO body
/// includes its own distance test with the **object** grey
/// distance (callers pass the computed 2D distance).
#[must_use]
pub fn gameobject_visible(
    target: &GameObject,
    dist: f32,
    visibility: f32,
    grey_object: f32,
    in_visible_list: bool,
    now_unix: u64,
) -> bool {
    if !target.is_spawned(now_unix) {
        return false;
    }
    if is_server_only(&target.info) {
        return false;
    }
    // Stealthed-trap hostility needs faction reactions;
    // traps are inert in Slice 3 (recorded).
    let limit = (visibility + if in_visible_list { grey_object } else { 0.0 })
        .max(target.visibility_modifier);
    dist <= limit
}

/// Same-team test used by the ghost rules.
#[must_use]
pub fn same_team(a: Team, b: Team) -> bool {
    a == b && a != Team::None
}
