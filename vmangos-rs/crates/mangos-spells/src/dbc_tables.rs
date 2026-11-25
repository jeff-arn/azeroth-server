//! DBC aux table loading (`SpellCastTimes`,
//! `SpellDuration`, `SpellRange`, `SpellRadius`) from a
//! `dbc/` directory, mirroring `DBCStores.cpp` loads.

use std::path::Path;

use mangos_dbc::{DbcFile, FieldValue, formats};

use crate::entry::{CastTimeEntry, DurationEntry, RangeEntry, SpellDbcTables};

fn int(row: &[FieldValue], idx: usize) -> i32 {
    match row.get(idx) {
        // DBC `i` fields surface as UInt bit patterns.
        Some(FieldValue::UInt(v)) => i32::from_ne_bytes(v.to_ne_bytes()),
        _ => 0,
    }
}

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        _ => 0,
    }
}

fn float(row: &[FieldValue], idx: usize) -> f32 {
    match row.get(idx) {
        Some(FieldValue::Float(v)) => *v,
        _ => 0.0,
    }
}

fn read_rows(dir: &Path, def: formats::StoreDef) -> std::io::Result<Vec<Vec<FieldValue>>> {
    let bytes = std::fs::read(dir.join(def.file))?;
    let store = DbcFile::parse(&bytes, def.fmt)
        .and_then(|f| f.produce(def.fmt))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(store.rows().to_vec())
}

/// Loads the four aux tables. Missing/invalid files are
/// hard errors (boot must not run with empty spell
/// tables, matching the C++ DBC load failure).
///
/// # Errors
///
/// Returns the io/parse error for the first bad file.
pub fn load_spell_dbc_tables(dir: &Path) -> std::io::Result<SpellDbcTables> {
    let mut tables = SpellDbcTables::default();
    for row in read_rows(dir, formats::SPELL_CAST_TIMES)? {
        tables.cast_times.insert(
            uint(&row, 0),
            CastTimeEntry {
                cast_time: int(&row, 1),
                cast_time_per_level: int(&row, 2),
                min_cast_time: int(&row, 3),
            },
        );
    }
    for row in read_rows(dir, formats::SPELL_DURATION)? {
        tables.durations.insert(
            uint(&row, 0),
            DurationEntry {
                duration: int(&row, 1),
                duration_per_level: int(&row, 2),
                max_duration: int(&row, 3),
            },
        );
    }
    for row in read_rows(dir, formats::SPELL_RANGE)? {
        tables.ranges.insert(
            uint(&row, 0),
            RangeEntry {
                min: float(&row, 1),
                max: float(&row, 2),
            },
        );
    }
    for row in read_rows(dir, formats::SPELL_RADIUS)? {
        tables.radii.insert(uint(&row, 0), float(&row, 1));
    }
    Ok(tables)
}
