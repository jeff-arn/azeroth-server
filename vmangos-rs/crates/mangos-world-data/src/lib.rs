//! Read-only static stores (architecture section 12:
//! `ObjectMgr` loaders, `SpellMgr`, `SQLStorage`).
//!
//! Phase 3 landed the boot gates an empty world passes
//! (`DataDir`, start-area asset checks, `mangos_string`, enum
//! stores). Phase 4 adds the simulation data loaders slice by
//! slice (creature templates first); loaders that do not exist
//! yet are still logged as deferred by the binary, never
//! silently skipped.

pub mod areas;
pub mod assets_check;
pub mod auction_house;
pub mod chat_channels;
pub mod creature_spells;
pub mod creature_template;
pub mod enum_stores;
pub mod gameobject_template;
pub mod item_prototype;
pub mod load_report;
pub mod player_info;
pub mod reputations;
pub mod script_engine;
pub mod skill_info;
pub mod spawn_support;
pub mod spawns;
pub mod spell_template;
pub mod strings;
pub mod waypoints;
pub mod weather;

pub use load_report::LoadReport;

use std::path::{Path, PathBuf};

use mangos_config::Config;

/// `sWorld.GetDataPath()`: the `DataDir` config value with the
/// C++ normalization (empty → `./`; `World.cpp:905-920`).
/// Subpaths join from here instead of the C++ string
/// concatenation, with identical results.
#[derive(Clone, Debug)]
pub struct DataDir(PathBuf);

impl DataDir {
    #[must_use]
    pub fn from_config(cfg: &Config) -> DataDir {
        let raw = cfg.string_default("DataDir", "./");
        DataDir::new(if raw.is_empty() {
            "./".to_string()
        } else {
            raw
        })
    }

    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> DataDir {
        DataDir(path.into())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `<DataDir>/maps`
    #[must_use]
    pub fn maps_dir(&self) -> PathBuf {
        self.0.join("maps")
    }

    /// `<DataDir>/vmaps`
    #[must_use]
    pub fn vmaps_dir(&self) -> PathBuf {
        self.0.join("vmaps")
    }

    /// `<DataDir>/mmaps`
    #[must_use]
    pub fn mmaps_dir(&self) -> PathBuf {
        self.0.join("mmaps")
    }

    /// `<DataDir>/5875` (the DBC path is suffixed with the
    /// supported client build, `World.cpp:1376-1382`).
    #[must_use]
    pub fn dbc_dir(&self) -> PathBuf {
        self.0.join("5875")
    }
}
