//! Entity core for the Phase 4 simulation cluster: the
//! word-array values store every `Object` carries, and (as slices
//! land) the object-update block builders and entity aggregates
//! from `vmangos/src/game/Objects/`. Player is decomposed into
//! domain modules under one aggregate (crate map §12), never
//! independent owners — the C++ domains cross-read each other.

pub mod aura;
pub mod creature;
pub mod gameobject;
pub mod object;
pub mod player;
pub mod unit;
pub mod update;
pub mod values;

pub use object::{DelayedEffects, MarkEffect, ObjectBase};
pub use values::Values;
