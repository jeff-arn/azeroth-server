//! Instance binding persistence (`instance`,
//! `character_instance`, `group_instance`): the
//! entry-critical subset of `MapPersistentStateMgr` /
//! `Player::BindToInstance` / `Group::BindToInstance`
//! with fork-exact statements. Reset scheduling,
//! per-instance respawn persistence, and instance
//! data blobs land with the actor wiring.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// `RESERVED_INSTANCES_LAST` (`Map.h:181`): instance
/// ids at or below this are reserved; allocation
/// starts above the persisted maximum.
pub const RESERVED_INSTANCES_LAST: u32 = 100;

/// One `instance` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRow {
    pub id: u32,
    pub map_id: u32,
    pub reset_time: i64,
}

/// One `character_instance` / `group_instance` bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceBind {
    pub guid_low: u32,
    pub instance_id: u32,
    pub permanent: bool,
}

/// Boot snapshot of the binding tables.
#[derive(Clone, Debug, Default)]
pub struct LoadedInstances {
    pub instances: Vec<InstanceRow>,
    pub character_binds: Vec<InstanceBind>,
    pub group_binds: Vec<InstanceBind>,
    /// `MapManager::InitMaxInstanceId` result:
    /// `max(MAX(instance.id), RESERVED_INSTANCES_LAST)`.
    pub max_instance_id: u32,
}

/// Loads instances + binds and the allocation floor.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_instances(conn: &mut mysql::Conn) -> Result<LoadedInstances, DbError> {
    // Fork boot cleanup for respawn rows of instances
    // that no longer exist (`CleanupInstances` respawn
    // arm); without it a reused instance id would
    // inherit a dead instance's respawn times. The
    // bind/instance cleanup arms and `PackInstances`
    // renumbering remain unported instance-lifecycle
    // debt (see PHASE6-PROGRESS).
    conn.exec_drop(
        "DELETE FROM `creature_respawn` WHERE `instance` >= ? \
         AND `instance` NOT IN (SELECT `id` FROM `instance`)",
        (RESERVED_INSTANCES_LAST,),
    )?;
    conn.exec_drop(
        "DELETE FROM `gameobject_respawn` WHERE `instance` >= ? \
         AND `instance` NOT IN (SELECT `id` FROM `instance`)",
        (RESERVED_INSTANCES_LAST,),
    )?;
    let max_rows: Vec<mysql::Row> = conn.query("SELECT MAX(`id`) FROM `instance`")?;
    let max_id = max_rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0));
    let instance_rows: Vec<mysql::Row> =
        conn.query("SELECT `id`, `map`, `reset_time` FROM `instance`")?;
    let instances = instance_rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            InstanceRow {
                id: f.u32(0),
                map_id: f.u32(1),
                reset_time: f.i64(2),
            }
        })
        .collect();
    let character_binds = load_binds(conn, "character_instance", "guid")?;
    let group_binds = load_binds(conn, "group_instance", "leader_guid")?;
    Ok(LoadedInstances {
        instances,
        character_binds,
        group_binds,
        max_instance_id: max_id.max(RESERVED_INSTANCES_LAST),
    })
}

fn load_binds(
    conn: &mut mysql::Conn,
    table: &str,
    guid_col: &str,
) -> Result<Vec<InstanceBind>, DbError> {
    let sql = format!("SELECT `{guid_col}`, `instance`, `permanent` FROM `{table}`");
    let rows: Vec<mysql::Row> = conn.query(sql)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            InstanceBind {
                guid_low: f.u32(0),
                instance_id: f.u32(1),
                permanent: f.u8(2) != 0,
            }
        })
        .collect())
}

/// `DungeonPersistentState::SaveToDB` (data blob is
/// written by the map layer once instance data exists;
/// entry slice writes the empty string like a state
/// whose map has no save data).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_instance(
    conn: &mut mysql::Conn,
    instance_id: u32,
    map_id: u32,
    reset_time: i64,
    data: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `instance` (`id`, `map`, `reset_time`, `data`) VALUES (?, ?, ?, ?)",
        (instance_id, map_id, reset_time, data),
    )?;
    Ok(())
}

/// `Player::BindToInstance` insert arm.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_character_bind(
    conn: &mut mysql::Conn,
    guid_low: u32,
    instance_id: u32,
    permanent: bool,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `character_instance` (`guid`, `instance`, `permanent`) VALUES (?, ?, ?)",
        (guid_low, instance_id, u8::from(permanent)),
    )?;
    Ok(())
}

/// `Player::BindToInstance` update arm (rebind to a
/// new instance / permanence change).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_character_bind(
    conn: &mut mysql::Conn,
    guid_low: u32,
    old_instance_id: u32,
    new_instance_id: u32,
    permanent: bool,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_instance` SET `instance` = ?, `permanent` = ? \
         WHERE `guid` = ? AND `instance` = ?",
        (
            new_instance_id,
            u8::from(permanent),
            guid_low,
            old_instance_id,
        ),
    )?;
    Ok(())
}

/// `Group::BindToInstance` insert arm.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_group_bind(
    conn: &mut mysql::Conn,
    leader_low: u32,
    instance_id: u32,
    permanent: bool,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `group_instance` (`leader_guid`, `instance`, `permanent`) VALUES (?, ?, ?)",
        (leader_low, instance_id, u8::from(permanent)),
    )?;
    Ok(())
}

/// Deletes one character bind (`Player::UnbindInstance`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_character_bind(
    conn: &mut mysql::Conn,
    guid_low: u32,
    instance_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_instance` WHERE `guid` = ? AND `instance` = ?",
        (guid_low, instance_id),
    )?;
    Ok(())
}

/// Deletes every character bind to an instance
/// (`MapPersistentStateManager::DeleteInstanceFromDB`
/// bind cleanup).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_character_binds_for_instance(
    conn: &mut mysql::Conn,
    instance_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_instance` WHERE `instance` = ?",
        (instance_id,),
    )?;
    Ok(())
}

/// Deletes one character's binds (character deletion
/// path, `Player.cpp:4512`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_character_binds(conn: &mut mysql::Conn, guid_low: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_instance` WHERE `guid` = ?",
        (guid_low,),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_round_trip_shape() {
        let bind = InstanceBind {
            guid_low: 5,
            instance_id: 101,
            permanent: true,
        };
        assert_eq!(bind.instance_id, RESERVED_INSTANCES_LAST + 1);
    }
}
