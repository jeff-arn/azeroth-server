//! Script escort waypoints (`ScriptMgr::LoadScriptWaypoints`,
//! `ScriptMgr.cpp:2212`): `script_waypoint` rows grouped by
//! creature entry in `pointid` order. Rows for nonexistent
//! creature entries are dropped with a loader diagnostic.
//! `script_escort_data` has no fork consumer outside the
//! loader and is not loaded.

use std::collections::HashMap;

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::creature_template::CreatureTemplates;
use crate::load_report::LoadReport;

/// One `ScriptPointMove` (`ScriptMgr.h`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptWaypoint {
    pub point_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub wait_time_ms: u32,
}

/// `m_mPointMoveMap`: point lists by creature entry.
#[derive(Clone, Debug, Default)]
pub struct ScriptWaypointStore {
    by_entry: HashMap<u32, Vec<ScriptWaypoint>>,
}

impl ScriptWaypointStore {
    /// Test-fixture constructor.
    #[must_use]
    pub fn from_entries_for_tests(entries: Vec<(u32, Vec<ScriptWaypoint>)>) -> Self {
        Self {
            by_entry: entries.into_iter().collect(),
        }
    }

    /// `ScriptMgr::GetPointMoveList`.
    #[must_use]
    pub fn list(&self, entry: u32) -> Option<&[ScriptWaypoint]> {
        self.by_entry.get(&entry).map(Vec::as_slice)
    }

    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.by_entry.values().map(Vec::len).sum()
    }
}

/// `LoadScriptWaypoints` body.
///
/// # Errors
///
/// [`DbError`] on connection failures.
pub fn load_script_waypoints(
    conn: &mut mysql::Conn,
    templates: &CreatureTemplates,
    report: &mut LoadReport,
) -> Result<ScriptWaypointStore, DbError> {
    let rows: Vec<(u32, u32, f64, f64, f64, u32)> = conn.query_map(
        "SELECT `entry`, `pointid`, `location_x`, `location_y`, `location_z`, `waittime` FROM `script_waypoint` ORDER BY `pointid`",
        |(entry, pointid, x, y, z, waittime)| (entry, pointid, x, y, z, waittime),
    )?;
    let mut by_entry: HashMap<u32, Vec<ScriptWaypoint>> = HashMap::new();
    for (entry, point_id, x, y, z, wait_time_ms) in rows {
        if templates.get(entry).is_none() {
            report.error(format!(
                "script_waypoint entry {entry}: no such creature template"
            ));
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        by_entry.entry(entry).or_default().push(ScriptWaypoint {
            point_id,
            x: x as f32,
            y: y as f32,
            z: z as f32,
            wait_time_ms,
        });
    }
    Ok(ScriptWaypointStore { by_entry })
}
