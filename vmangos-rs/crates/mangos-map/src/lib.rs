//! Map simulation for the Rust `mangosd`: grid activation
//! from the DB spawn stores, player visibility, object
//! update replication, and movement relay, mirroring
//! `vmangos/src/game/Maps/` at the Slice 3 scope recorded
//! in `docs/rust-migration-evidence/PHASE4-PROGRESS.md`.
//!
//! One [`Map`] is one (map id, instance id). The binary's
//! world actor owns the maps and drives them at 50 ms with
//! real elapsed diffs; everything here is synchronous and
//! session-free — outbound packets are tagged with the
//! receiving player's GUID and drained by the caller.

pub mod ai;
pub mod content;
pub mod map;
pub mod motion;
pub mod scripts;
pub mod spells;
pub mod stores;
pub mod visibility;

pub use map::{Map, MapConfig, MapEvent, PlayerSlotId};
pub use stores::WorldStores;

use mangos_entities::creature::SpawnRng;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// `urand` over a seeded standard RNG (the C++ global uses
/// `rand()`; only the inclusive-range contract matters).
#[derive(Debug)]
pub struct WorldRng(StdRng);

impl WorldRng {
    #[must_use]
    pub fn from_entropy() -> Self {
        Self(StdRng::from_os_rng())
    }

    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self(StdRng::seed_from_u64(seed))
    }

    /// `rand_norm_f()`: uniform in [0, 1).
    pub fn rand_norm(&mut self) -> f32 {
        self.0.random::<f32>()
    }

    /// `frand(min, max)`.
    pub fn frand(&mut self, min: f32, max: f32) -> f32 {
        if max <= min {
            return min;
        }
        min + self.rand_norm() * (max - min)
    }
}

impl mangos_spells::SpellRng for WorldRng {
    fn irand(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        // Span in i64 to cover the full i32 range.
        let span = (i64::from(max) - i64::from(min)) as u64;
        let roll = self.0.random::<u64>() % (span + 1);
        #[allow(clippy::cast_possible_truncation)]
        {
            min + roll as i32
        }
    }

    fn frand01(&mut self) -> f32 {
        self.rand_norm()
    }
}

impl SpawnRng for WorldRng {
    fn urand(&mut self, min: u32, max: u32) -> u32 {
        if max <= min {
            return min;
        }
        self.0.random_range(min..=max)
    }
}
