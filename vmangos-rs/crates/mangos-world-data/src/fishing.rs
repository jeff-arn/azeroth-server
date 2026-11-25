//! Fishing base skill levels (`ObjectMgr::LoadFishingBaseSkillLevel`,
//! `ObjectMgr.cpp:10024-10063`): `skill_fishing_base_level`
//! maps an area id to the minimum fishing skill for its
//! water. The fishing-node Use arm reads the subzone row
//! first, then the zone row (fork `GameObject.cpp:1607`);
//! missing rows read as 0 there. DBC-only area existence
//! is validated here (`AreaEntry::GetById` gate, fork
//! `:10044-10049`); no DB/save shape change.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

use crate::areas::AreaStore;
use crate::load_report::LoadReport;

/// `ObjectMgr::m_FishingBaseSkillMap` (`ObjectMgr.h:937`).
#[derive(Clone, Debug, Default)]
pub struct FishingBaseSkills {
    by_area: HashMap<u32, i32>,
}

impl FishingBaseSkills {
    /// Test-fixture constructor (`(area, skill)` rows).
    #[must_use]
    pub fn from_entries_for_tests(entries: Vec<(u32, i32)>) -> Self {
        Self {
            by_area: entries.into_iter().collect(),
        }
    }

    /// `ObjectMgr::GetFishingBaseSkillLevel`
    /// (`ObjectMgr.h:939`): 0 when the area has no row.
    #[must_use]
    pub fn get(&self, area_id: u32) -> i32 {
        self.by_area.get(&area_id).copied().unwrap_or(0)
    }

    /// Row count, for the binary boot log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_area.len()
    }

    /// Whether any area row loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_area.is_empty()
    }
}

/// `ObjectMgr::LoadFishingBaseSkillLevel` (fork
/// `ObjectMgr.cpp:10024-10063` verbatim query): empty
/// tables log and return empty (fork keeps the map, only
/// warns); unknown area ids are reported and skipped.
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn load_fishing_base_skills(
    conn: &mut mysql::Conn,
    areas: &AreaStore,
    report: &mut LoadReport,
) -> Result<FishingBaseSkills, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `skill` FROM `skill_fishing_base_level`",
        (),
    )?;
    if rows.is_empty() {
        report.error(">> Loaded `skill_fishing_base_level`, table is empty!".to_string());
        return Ok(FishingBaseSkills::default());
    }
    let mut by_area = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let entry = f.u32(0);
        let skill = f.i32(1);
        // Fork `:10044-10049`: unknown areas are a data
        // error, skipped.
        if areas.entry(entry).is_none() {
            report.error(format!(
                "AreaId {entry} defined in `skill_fishing_base_level` does not exist"
            ));
            continue;
        }
        by_area.insert(entry, skill);
    }
    Ok(FishingBaseSkills { by_area })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn areas() -> AreaStore {
        AreaStore::from_entries_for_tests(vec![
            crate::areas::AreaEntry {
                id: 12,
                map_id: 0,
                zone_id: 0,
                explore_flag: 7,
                flags: 0,
                area_level: 10,
                team: 7,
                liquid_type: 0,
            },
            crate::areas::AreaEntry {
                id: 34,
                map_id: 0,
                zone_id: 12,
                explore_flag: 9,
                flags: 0,
                area_level: 10,
                team: 7,
                liquid_type: 0,
            },
        ])
    }

    #[test]
    fn missing_area_reads_zero() {
        let skills = FishingBaseSkills::from_entries_for_tests(vec![(12, 200)]);
        assert_eq!(skills.get(12), 200);
        assert_eq!(skills.get(34), 0);
    }

    #[test]
    fn unknown_area_rows_are_skipped() {
        // Simulates the loader filter without a DB: only
        // rows the area store knows survive.
        let areas = areas();
        let rows = [(12, 200), (999, 300), (34, 100)];
        let mut kept = Vec::new();
        for (entry, skill) in rows {
            if areas.entry(entry).is_some() {
                kept.push((entry, skill));
            }
        }
        let skills = FishingBaseSkills::from_entries_for_tests(kept);
        assert_eq!(skills.get(12), 200);
        assert_eq!(skills.get(34), 100);
        assert_eq!(skills.get(999), 0);
    }
}
