//! Area templates and map linked zones
//! (`sAreaStore`, `TerrainManager::Get*ByAreaFlag`, and
//! `MapManager::GetLinkedZone`).

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// One `area_template` row (`AreaEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaEntry {
    pub id: u32,
    pub map_id: u32,
    pub zone_id: u32,
    pub explore_flag: u32,
    pub flags: u32,
    pub area_level: i32,
    pub team: u32,
    pub liquid_type: u32,
}

impl AreaEntry {
    #[must_use]
    pub fn is_zone(self) -> bool {
        self.zone_id == 0
    }
}

/// `area_template` by `(explore flag, map)` plus each map's
/// linked zone from `map_template`.
#[derive(Clone, Debug, Default)]
pub struct AreaStore {
    by_flag_and_map: HashMap<(u32, u32), AreaEntry>,
    linked_zone_by_map: HashMap<u32, u32>,
    name_by_id: HashMap<u32, String>,
}

impl AreaStore {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_flag_and_map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_flag_and_map.is_empty()
    }

    /// Area/zone display name by area id (empty when the
    /// id is unknown), for `HandleWhoOpcode` search strings.
    #[must_use]
    pub fn name(&self, area_id: u32) -> &str {
        self.name_by_id.get(&area_id).map_or("", String::as_str)
    }

    #[must_use]
    pub fn linked_zone(&self, map_id: u32) -> u32 {
        self.linked_zone_by_map.get(&map_id).copied().unwrap_or(0)
    }

    /// `TerrainManager::GetZoneAndAreaIdByAreaFlag`.
    #[must_use]
    pub fn zone_and_area_by_flag(&self, area_flag: u16, map_id: u32) -> (u32, u32) {
        let Some(entry) = self
            .by_flag_and_map
            .get(&(u32::from(area_flag), map_id))
            .copied()
        else {
            return (0, 0);
        };
        let zone_id = if entry.is_zone() {
            entry.id
        } else {
            entry.zone_id
        };
        (zone_id, entry.id)
    }

    fn from_rows(
        entries: impl IntoIterator<Item = AreaEntry>,
        linked_zones: impl IntoIterator<Item = (u32, u32)>,
        names: impl IntoIterator<Item = (u32, String)>,
    ) -> Self {
        Self {
            by_flag_and_map: entries
                .into_iter()
                .map(|entry| ((entry.explore_flag, entry.map_id), entry))
                .collect(),
            linked_zone_by_map: linked_zones.into_iter().collect(),
            name_by_id: names.into_iter().collect(),
        }
    }
}

/// Loads all `area_template` rows and the progression-selected
/// `map_template.linked_zone` values.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_areas(conn: &mut mysql::Conn, wow_patch: u32) -> Result<AreaStore, DbError> {
    let area_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `map_id`, `zone_id`, `explore_flag`, `flags`, `area_level`, \
         `team`, `liquid_type`, `name` FROM `area_template`",
        (),
    )?;
    let mut entries = Vec::with_capacity(area_rows.len());
    let mut names = Vec::with_capacity(area_rows.len());
    for row in area_rows {
        let f = mangos_db::row::Fields::from_row(row);
        let entry = AreaEntry {
            id: f.u32(0),
            map_id: f.u32(1),
            zone_id: f.u32(2),
            explore_flag: f.u32(3),
            flags: f.u32(4),
            area_level: f.i32(5),
            team: f.u32(6),
            liquid_type: f.u32(7),
        };
        names.push((entry.id, f.string(8)));
        entries.push(entry);
    }

    let map_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `linked_zone` FROM `map_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `map_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let linked_zones = map_rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        (f.u32(0), f.u32(1))
    });

    Ok(AreaStore::from_rows(entries, linked_zones, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(id: u32, zone_id: u32, flag: u32) -> AreaEntry {
        AreaEntry {
            id,
            map_id: 1,
            zone_id,
            explore_flag: flag,
            flags: 0,
            area_level: 0,
            team: 0,
            liquid_type: 0,
        }
    }

    #[test]
    fn zone_resolution_flattens_subareas_to_parent_zone() {
        let store = AreaStore::from_rows(
            [area(14, 0, 148), area(363, 14, 149)],
            [(1, 14)],
            [(14, "Durotar".to_string())],
        );
        assert_eq!(store.zone_and_area_by_flag(148, 1), (14, 14));
        assert_eq!(store.zone_and_area_by_flag(149, 1), (14, 363));
        assert_eq!(store.zone_and_area_by_flag(149, 0), (0, 0));
        assert_eq!(store.linked_zone(1), 14);
        assert_eq!(store.name(14), "Durotar");
        assert_eq!(store.name(999), "");
    }
}
