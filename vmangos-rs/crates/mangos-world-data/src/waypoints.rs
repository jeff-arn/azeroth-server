//! Waypoint paths: `creature_movement` (per spawn GUID) and
//! `creature_movement_template` (per entry), ported from
//! `WaypointManager::Load` (`WaypointManager.cpp:33-376`).
//! Paths are point-ordered maps like the C++
//! `WaypointPath = std::map<uint32, WaypointNode>`; the
//! `creature_movement_special` sub-path table is loaded too
//! (used via `path_id`).
//!
//! Deviations from the C++ loader, recorded: dangling-id
//! cross-checks against spawn/template stores are not
//! repeated here (they only log; lookups miss harmlessly),
//! and the in-DB `path_id` write-back fixups are not
//! performed (read-only port; C++ fixes bad rows in place).

use std::collections::{BTreeMap, HashMap};

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::load_report::LoadReport;

/// `struct WaypointNode` (`WaypointManager.h:38-52`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaypointNode {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 100.0 = no facing override (`StartMove` tests `!= 100`).
    pub orientation: f32,
    /// `waittime` (ms the creature waits at the node).
    pub delay: u32,
    pub wander_distance: f32,
    pub script_id: u32,
    /// Special sub-path id, 0 = none.
    pub path_id: u32,
}

/// Point id -> node, ascending (`WaypointPath`).
pub type WaypointPath = BTreeMap<u32, WaypointNode>;

/// All waypoint paths by origin.
#[derive(Clone, Debug, Default)]
pub struct WaypointStore {
    /// `creature_movement`, keyed by spawn GUID (`id`).
    pub by_guid: HashMap<u32, WaypointPath>,
    /// `creature_movement_template`, keyed by entry.
    pub by_entry: HashMap<u32, WaypointPath>,
    /// `creature_movement_special`, keyed by `id`.
    pub special: HashMap<u32, WaypointPath>,
}

impl WaypointStore {
    /// Path for a creature: GUID path wins, else template
    /// path (`WaypointManager::GetPath` origin order for
    /// `PATH_FROM_GUID`/`PATH_FROM_ENTRY`).
    #[must_use]
    pub fn path_for(&self, guid_low: u32, entry: u32) -> Option<&WaypointPath> {
        self.by_guid
            .get(&guid_low)
            .filter(|p| !p.is_empty())
            .or_else(|| self.by_entry.get(&entry).filter(|p| !p.is_empty()))
    }

    #[must_use]
    pub fn special_path(&self, id: u32) -> Option<&WaypointPath> {
        self.special.get(&id).filter(|p| !p.is_empty())
    }

    /// Total node count across all three tables (load log
    /// parity check).
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        let count = |m: &HashMap<u32, WaypointPath>| m.values().map(BTreeMap::len).sum::<usize>();
        count(&self.by_guid) + count(&self.by_entry) + count(&self.special)
    }
}

type Row = (u32, u32, f32, f32, f32, u32, f32, u32, f32, u32);

fn load_table(
    conn: &mut mysql::Conn,
    table: &str,
    id_column: &str,
    report: &mut LoadReport,
) -> Result<HashMap<u32, WaypointPath>, DbError> {
    let sql = format!(
        "SELECT `{id_column}`, `point`, `position_x`, `position_y`, `position_z`, \
         `waittime`, `wander_distance`, `script_id`, `orientation`, `path_id` \
         FROM `{table}`"
    );
    let rows: Vec<Row> = conn.query(sql).map_err(DbError::from)?;
    let mut out: HashMap<u32, WaypointPath> = HashMap::new();
    for (id, point, x, y, z, waittime, wander, script_id, orientation, path_id) in rows {
        let node = WaypointNode {
            x,
            y,
            z,
            orientation,
            delay: waittime,
            wander_distance: wander,
            script_id,
            path_id,
        };
        let path = out.entry(id).or_default();
        // C++ inserts into std::map without overwrite checks
        // (later duplicates silently replace via operator[]).
        if path.insert(point, node).is_some() {
            report.error(format!(
                "{table}: duplicate point {point} for id {id}; later row wins"
            ));
        }
    }
    Ok(out)
}

/// Loads all three waypoint tables from the world DB.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_waypoints(
    conn: &mut mysql::Conn,
    report: &mut LoadReport,
) -> Result<WaypointStore, DbError> {
    Ok(WaypointStore {
        by_guid: load_table(conn, "creature_movement", "id", report)?,
        by_entry: load_table(conn, "creature_movement_template", "entry", report)?,
        special: load_table(conn, "creature_movement_special", "id", report)?,
    })
}
