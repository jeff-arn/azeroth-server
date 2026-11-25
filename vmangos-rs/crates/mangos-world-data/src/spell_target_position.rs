//! `spell_target_position` loader
//! (`SpellMgr::LoadSpellTargetPositions`): database
//! teleport destinations for spells with
//! `TARGET_LOCATION_DATABASE` (17) in any effect
//! target slot.

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_db::row::Fields;
use mysql::prelude::Queryable;

use crate::spawns::WorldLocation;

/// Destinations by spell id (`SpellTargetPositionMap`).
#[derive(Debug, Clone, Default)]
pub struct SpellTargetPositions {
    positions: HashMap<u32, WorldLocation>,
}

impl SpellTargetPositions {
    /// `SpellMgr::GetSpellTargetPosition`.
    #[must_use]
    pub fn get(&self, spell_id: u32) -> Option<WorldLocation> {
        self.positions.get(&spell_id).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// Loads build-visible `spell_target_position` rows,
/// dropping rows the fork rejects: unknown map,
/// all-zero coordinates, unknown spell, and spells
/// without target 17 (`spell_has_db_target` covers
/// the last two).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_spell_target_positions(
    conn: &mut mysql::Conn,
    supported_build: u32,
    known_maps: &dyn Fn(u32) -> bool,
    spell_has_db_target: &dyn Fn(u32) -> bool,
) -> Result<SpellTargetPositions, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `target_map`, `target_position_x`, `target_position_y`, \
         `target_position_z`, `target_orientation` FROM `spell_target_position` \
         WHERE ? BETWEEN `build_min` AND `build_max`",
        (supported_build,),
    )?;
    let mut positions: HashMap<u32, WorldLocation> = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = Fields::from_row(row);
        let (x, y, z) = (f.f32(2), f.f32(3), f.f32(4));
        if x == 0.0 && y == 0.0 && z == 0.0 {
            continue;
        }
        let map_id = f.u32(1);
        if !known_maps(map_id) {
            continue;
        }
        let spell_id = f.u32(0);
        if !spell_has_db_target(spell_id) {
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        positions.insert(
            spell_id,
            WorldLocation {
                map_id: map_id as u16,
                x,
                y,
                z,
                o: f.f32(5),
            },
        );
    }
    Ok(SpellTargetPositions { positions })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_misses() {
        let store = SpellTargetPositions::default();
        assert!(store.is_empty());
        assert_eq!(store.get(3561), None);
    }
}
