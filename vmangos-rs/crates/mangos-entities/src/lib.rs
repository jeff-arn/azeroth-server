//! Entity core for the Phase 4 simulation cluster: the
//! word-array values store every `Object` carries, and (as slices
//! land) the object-update block builders and entity aggregates
//! from `vmangos/src/game/Objects/`. Player is decomposed into
//! domain modules under one aggregate (crate map §12), never
//! independent owners — the C++ domains cross-read each other.

pub mod aura;
pub mod battleground;
pub mod creature;
pub mod dynobject;
pub mod gameobject;
pub mod object;
pub mod player;
pub mod unit;
pub mod update;
pub mod values;

pub use mangos_db::pet::{
    PET_SAVE_AS_CURRENT, PET_SAVE_LAST_STABLE_SLOT, PET_SAVE_NOT_IN_SLOT, PET_TYPE_HUNTER,
    PET_TYPE_SUMMON, PetRow,
};
pub use object::{DelayedEffects, MarkEffect, ObjectBase};
pub use values::Values;
