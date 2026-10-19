//! Area triggers (`ObjectMgr::LoadAreaTriggers` +
//! `LoadAreaTriggerTeleports`, `IsPointInAreaTriggerZone`):
//! trigger volumes from `areatrigger_template` and
//! teleport destinations from `areatrigger_teleport`.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// Client build the trigger template load is capped at
/// (`SUPPORTED_CLIENT_BUILD`).
pub const SUPPORTED_CLIENT_BUILD: u32 = 5875;

/// One `areatrigger_template` row (`AreaTriggerEntry`).
#[derive(Clone, Debug, PartialEq)]
pub struct AreaTriggerEntry {
    pub id: u32,
    pub name: String,
    pub map_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub box_x: f32,
    pub box_y: f32,
    pub box_z: f32,
    pub box_orientation: f32,
    pub cooldown: u32,
    pub condition_id: u32,
    pub script_id: u32,
    /// DB script name (`script_name` column; the fork
    /// resolves it to a core script id at load).
    pub script_name: String,
}

impl AreaTriggerEntry {
    /// Whether any script hook is attached (fork checks
    /// `script_id || script_name`).
    #[must_use]
    pub fn has_script(&self) -> bool {
        self.script_id != 0 || !self.script_name.is_empty()
    }

    /// `IsPointInAreaTriggerZone` with fork-identical
    /// radius/box math (`delta` widens every edge).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn contains(&self, map_id: u32, x: f32, y: f32, z: f32, delta: f32) -> bool {
        if map_id != self.map_id {
            return false;
        }
        if self.radius > 0.0 {
            let dist2 = (x - self.x) * (x - self.x)
                + (y - self.y) * (y - self.y)
                + (z - self.z) * (z - self.z);
            return dist2 <= (self.radius + delta) * (self.radius + delta);
        }
        let rotation = 2.0 * std::f64::consts::PI - f64::from(self.box_orientation);
        let (sin_val, cos_val) = rotation.sin_cos();
        let player_box_dist_x = f64::from(x - self.x);
        let player_box_dist_y = f64::from(y - self.y);
        let dx = (player_box_dist_x * cos_val - player_box_dist_y * sin_val) as f32;
        let dy = (player_box_dist_y * cos_val + player_box_dist_x * sin_val) as f32;
        let dz = z - self.z;
        dx.abs() <= self.box_x / 2.0 + delta
            && dy.abs() <= self.box_y / 2.0 + delta
            && dz.abs() <= self.box_z / 2.0 + delta
    }
}

/// One `areatrigger_teleport` row (`AreaTriggerTeleport`).
#[derive(Clone, Debug, PartialEq)]
pub struct AreaTriggerTeleport {
    pub message: String,
    pub required_level: u8,
    pub required_condition: u32,
    pub dest_map_id: u32,
    pub dest_x: f32,
    pub dest_y: f32,
    pub dest_z: f32,
    pub dest_o: f32,
}

/// Trigger volumes + teleport definitions by trigger id.
#[derive(Clone, Debug, Default)]
pub struct AreaTriggerStore {
    triggers: HashMap<u32, AreaTriggerEntry>,
    teleports: HashMap<u32, AreaTriggerTeleport>,
    taverns: std::collections::HashSet<u32>,
    /// `areatrigger_involvedrelation`: trigger -> quest.
    quest_relations: HashMap<u32, u32>,
}

impl AreaTriggerStore {
    #[must_use]
    pub fn trigger_count(&self) -> usize {
        self.triggers.len()
    }

    #[must_use]
    pub fn teleport_count(&self) -> usize {
        self.teleports.len()
    }

    #[must_use]
    pub fn tavern_count(&self) -> usize {
        self.taverns.len()
    }

    /// `ObjectMgr::GetQuestForAreaTrigger`.
    #[must_use]
    pub fn quest_for_trigger(&self, trigger_id: u32) -> u32 {
        self.quest_relations.get(&trigger_id).copied().unwrap_or(0)
    }

    /// `ObjectMgr::IsTavernAreaTrigger`.
    #[must_use]
    pub fn is_tavern(&self, trigger_id: u32) -> bool {
        self.taverns.contains(&trigger_id)
    }

    /// `ObjectMgr::GetAreaTrigger`.
    #[must_use]
    pub fn get(&self, trigger_id: u32) -> Option<&AreaTriggerEntry> {
        self.triggers.get(&trigger_id)
    }

    /// `ObjectMgr::GetAreaTriggerTeleport`.
    #[must_use]
    pub fn teleport(&self, trigger_id: u32) -> Option<&AreaTriggerTeleport> {
        self.teleports.get(&trigger_id)
    }

    /// `ObjectMgr::GetMapEntranceTrigger`: first teleport
    /// whose destination is `map_id` (fork iterates the
    /// unordered map; single-entrance maps are the norm).
    #[must_use]
    pub fn entrance_for_map(&self, map_id: u32) -> Option<(u32, &AreaTriggerTeleport)> {
        self.teleports
            .iter()
            .filter(|(id, tele)| tele.dest_map_id == map_id && self.triggers.contains_key(id))
            .min_by_key(|(id, _)| **id)
            .map(|(id, tele)| (*id, tele))
    }

    /// `ObjectMgr::GetGoBackTrigger`: teleport out of a
    /// dungeon to its ghost-entrance continent map.
    #[must_use]
    pub fn go_back_trigger(
        &self,
        map_id: u32,
        ghost_entrance_map: i32,
    ) -> Option<(u32, &AreaTriggerTeleport)> {
        if ghost_entrance_map < 0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        let target = ghost_entrance_map as u32;
        self.teleports
            .iter()
            .filter(|(id, tele)| {
                tele.dest_map_id == target
                    && self.triggers.get(id).is_some_and(|t| t.map_id == map_id)
            })
            .min_by_key(|(id, _)| **id)
            .map(|(id, tele)| (*id, tele))
    }

    fn from_rows(
        triggers: impl IntoIterator<Item = AreaTriggerEntry>,
        teleports: impl IntoIterator<Item = (u32, AreaTriggerTeleport)>,
        known_maps: &dyn Fn(u32) -> bool,
    ) -> Self {
        let mut trigger_map: HashMap<u32, AreaTriggerEntry> =
            triggers.into_iter().map(|t| (t.id, t)).collect();
        // Fork fixups: condition/cooldown without a
        // script are zeroed.
        for trigger in trigger_map.values_mut() {
            if !trigger.has_script() {
                trigger.condition_id = 0;
                trigger.cooldown = 0;
            }
        }
        // Fork drops teleports with unknown trigger,
        // unknown target map, or zero destination.
        let teleport_map: HashMap<u32, AreaTriggerTeleport> = teleports
            .into_iter()
            .filter(|(id, tele)| {
                trigger_map.contains_key(id)
                    && known_maps(tele.dest_map_id)
                    && !(tele.dest_x == 0.0 && tele.dest_y == 0.0 && tele.dest_z == 0.0)
            })
            .collect();
        Self {
            triggers: trigger_map,
            teleports: teleport_map,
            taverns: std::collections::HashSet::new(),
            quest_relations: HashMap::new(),
        }
    }
}

/// Loads `areatrigger_template` (max build ≤ 5875) and
/// `areatrigger_teleport` (max patch ≤ `wow_patch`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_area_triggers(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    known_maps: &dyn Fn(u32) -> bool,
) -> Result<AreaTriggerStore, DbError> {
    let trigger_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `name`, `map_id`, `x`, `y`, `z`, `radius`, `box_x`, \
         `box_y`, `box_z`, `box_orientation`, `cooldown`, `condition_id`, \
         `script_id`, `script_name` FROM `areatrigger_template` t1 \
         WHERE `build` = (SELECT max(`build`) FROM `areatrigger_template` t2 \
                          WHERE t1.`id` = t2.`id` AND `build` <= ?)",
        (SUPPORTED_CLIENT_BUILD,),
    )?;
    let triggers = trigger_rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        AreaTriggerEntry {
            id: f.u32(0),
            name: f.string(1),
            map_id: f.u32(2),
            x: f.f32(3),
            y: f.f32(4),
            z: f.f32(5),
            radius: f.f32(6),
            box_x: f.f32(7),
            box_y: f.f32(8),
            box_z: f.f32(9),
            box_orientation: f.f32(10),
            cooldown: f.u32(11),
            condition_id: f.u32(12),
            script_id: f.u32(13),
            script_name: f.string(14),
        }
    });
    let teleport_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `required_level`, `required_condition`, `message`, \
         `target_map`, `target_position_x`, `target_position_y`, \
         `target_position_z`, `target_orientation` \
         FROM `areatrigger_teleport` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `areatrigger_teleport` t2 \
                          WHERE t1.`id` = t2.`id` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let teleports = teleport_rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        (
            f.u32(0),
            AreaTriggerTeleport {
                required_level: f.u8(1),
                required_condition: f.u32(2),
                message: f.string(3),
                dest_map_id: f.u32(4),
                dest_x: f.f32(5),
                dest_y: f.f32(6),
                dest_z: f.f32(7),
                dest_o: f.f32(8),
            },
        )
    });
    let mut store = AreaTriggerStore::from_rows(triggers, teleports, known_maps);
    let tavern_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id` FROM `areatrigger_tavern` WHERE `patch_min` <= ?",
        (wow_patch,),
    )?;
    store.taverns = tavern_rows
        .into_iter()
        .map(|row| mangos_db::row::Fields::from_row(row).u32(0))
        .filter(|id| store.triggers.contains_key(id))
        .collect();
    let quest_rows: Vec<mysql::Row> =
        conn.query("SELECT `id`, `quest` FROM `areatrigger_involvedrelation`")?;
    store.quest_relations = quest_rows
        .into_iter()
        .map(|row| {
            let f = mangos_db::row::Fields::from_row(row);
            (f.u32(0), f.u32(1))
        })
        .collect();
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(id: u32) -> AreaTriggerEntry {
        AreaTriggerEntry {
            id,
            name: String::new(),
            map_id: 0,
            x: 100.0,
            y: 200.0,
            z: 10.0,
            radius: 5.0,
            box_x: 0.0,
            box_y: 0.0,
            box_z: 0.0,
            box_orientation: 0.0,
            cooldown: 0,
            condition_id: 0,
            script_id: 0,
            script_name: String::new(),
        }
    }

    #[test]
    fn radius_zone_with_delta() {
        let t = sphere(78);
        assert!(t.contains(0, 104.0, 200.0, 10.0, 0.0));
        assert!(!t.contains(0, 106.0, 200.0, 10.0, 0.0));
        // Fork handler passes delta 5.0.
        assert!(t.contains(0, 109.0, 200.0, 10.0, 5.0));
        assert!(!t.contains(1, 100.0, 200.0, 10.0, 5.0));
    }

    #[test]
    fn box_zone_rotates_point() {
        let mut t = sphere(1);
        t.radius = 0.0;
        t.box_x = 10.0;
        t.box_y = 4.0;
        t.box_z = 6.0;
        t.box_orientation = 0.0;
        assert!(t.contains(0, 104.9, 201.9, 12.9, 0.0));
        assert!(!t.contains(0, 105.1, 200.0, 10.0, 0.0));
        assert!(!t.contains(0, 100.0, 200.0, 13.1, 0.0));
    }

    #[test]
    fn store_fixups_and_entrance_lookup() {
        let mut scriptless = sphere(78);
        scriptless.condition_id = 7;
        scriptless.cooldown = 30;
        let store = AreaTriggerStore::from_rows(
            [scriptless],
            [(
                78,
                AreaTriggerTeleport {
                    message: String::new(),
                    required_level: 10,
                    required_condition: 0,
                    dest_map_id: 36,
                    dest_x: -15.0,
                    dest_y: -385.0,
                    dest_z: 62.0,
                    dest_o: 0.0,
                },
            )],
            &|m| m == 36,
        );
        assert_eq!(store.get(78).map(|t| t.condition_id), Some(0));
        assert_eq!(store.get(78).map(|t| t.cooldown), Some(0));
        assert_eq!(store.entrance_for_map(36).map(|(id, _)| id), Some(78));
        assert_eq!(store.teleport_count(), 1);
    }
}
