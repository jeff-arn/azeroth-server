//! Spell data and deterministic spell math for the
//! Rust `mangosd` (Phase 4 Slice 5): the `SpellEntry`
//! subset loaded from `spell_template` (world DB, as in
//! this fork — not `Spell.dbc`), the DBC aux tables
//! (cast times/durations/ranges/radii), and the pure
//! calculation functions the cast engine in
//! `mangos-map` consumes.

pub mod dbc_tables;
pub mod defines;
pub mod entry;

pub use dbc_tables::load_spell_dbc_tables;

pub use entry::{
    CastTimeEntry, DurationEntry, RangeEntry, SpellDbcTables, SpellEffectData, SpellEntry,
    SpellRng, SpellStore, armor_reduced_damage, ditheru,
};
