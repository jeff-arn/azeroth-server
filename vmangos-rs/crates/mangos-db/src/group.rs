//! Group persistence (`Group` / `ObjectMgr::LoadGroups`,
//! `Group.cpp`, `ObjectMgr.cpp:5131-5240`): the `groups`
//! and `group_member` statements the C++ group service
//! issues, plus the boot load with its cleanup rules
//! (member rows for missing groups or nonexistent
//! characters are deleted; groups ending up with fewer
//! than two or more than forty members are disbanded).
//! `group_instance` binds are not ported (instance saves
//! land with character save, Phase 6); C++ writes them
//! from `_setLeader`/`Create` around the statements kept
//! here. Errors are returned for the caller to log — C++
//! queues the writes with `PExecute` and cannot observe
//! failure, so callers must not gate behavior on them.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// One `groups` row (`LoadGroupFromDB` field order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredGroup {
    pub group_id: u32,
    pub leader_low: u32,
    pub main_tank_low: u32,
    pub main_assistant_low: u32,
    pub loot_method: u8,
    pub looter_low: u32,
    pub loot_threshold: u16,
    pub icons: [u64; 8],
    pub is_raid: bool,
}

/// One boot-loaded member with its `PlayerCacheData`
/// fields resolved from `characters`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedMember {
    pub member_low: u32,
    pub name: String,
    pub race: u8,
    pub assistant: bool,
    pub subgroup: u8,
}

/// Boot-load output: surviving groups with members, the
/// surviving leader names, and the next group id
/// (`m_GroupIds` seeded from `MAX(group_id)`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedGroups {
    pub groups: Vec<(StoredGroup, String, Vec<LoadedMember>)>,
    pub next_group_id: u32,
}

/// `ObjectMgr::GetPlayerNameByGUID` + race over the
/// characters table (the C++ player cache).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_player_name_race(
    conn: &mut mysql::Conn,
    guid_low: u32,
) -> Result<Option<(String, u8)>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `name`, `race` FROM `characters` WHERE `guid` = ?",
        (guid_low,),
    )?;
    Ok(rows.into_iter().next().map(|row| {
        let f = Fields::from_row(row);
        (f.string(0), f.u8(1))
    }))
}

/// `ObjectMgr::LoadGroups` (without the instance binds):
/// loads `groups`, resolves leader names (missing leader
/// disbands the group), loads `group_member` ordered by
/// group (rows for missing groups or characters are
/// deleted), then disbands groups with `< 2` or `> 40`
/// members. Disband here is just the two table deletes —
/// no player is online at boot.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_groups(conn: &mut mysql::Conn) -> Result<LoadedGroups, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `main_tank_guid`, `main_assistant_guid`, `loot_method`, \
         `looter_guid`, `loot_threshold`, `icon1`, `icon2`, `icon3`, `icon4`, \
         `icon5`, `icon6`, `icon7`, `icon8`, `is_raid`, `leader_guid`, \
         `group_id` FROM `groups`",
        (),
    )?;
    let mut max_id = 0u32;
    let mut groups: Vec<(StoredGroup, String, Vec<LoadedMember>)> = Vec::new();
    for row in rows {
        let f = Fields::from_row(row);
        let stored = StoredGroup {
            main_tank_low: f.u32(0),
            main_assistant_low: f.u32(1),
            loot_method: f.u8(2),
            looter_low: f.u32(3),
            loot_threshold: f.u16(4),
            icons: [
                f.u64(5),
                f.u64(6),
                f.u64(7),
                f.u64(8),
                f.u64(9),
                f.u64(10),
                f.u64(11),
                f.u64(12),
            ],
            is_raid: f.u8(13) != 0,
            leader_low: f.u32(14),
            group_id: f.u32(15),
        };
        max_id = max_id.max(stored.group_id);
        match load_player_name_race(conn, stored.leader_low)? {
            Some((leader_name, _)) => groups.push((stored, leader_name, Vec::new())),
            None => delete_group(conn, stored.group_id)?,
        }
    }

    let member_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `member_guid`, `assistant`, `subgroup`, `group_id` \
         FROM `group_member` ORDER BY `group_id`",
        (),
    )?;
    for row in member_rows {
        let f = Fields::from_row(row);
        let (member_low, assistant, subgroup, group_id) =
            (f.u32(0), f.u8(1) != 0, f.u8(2), f.u32(3));
        let Some((_, _, members)) = groups.iter_mut().find(|(g, _, _)| g.group_id == group_id)
        else {
            delete_group_member(conn, member_low)?;
            continue;
        };
        if members
            .iter()
            .any(|m: &LoadedMember| m.member_low == member_low)
        {
            delete_group_member(conn, member_low)?;
            continue;
        }
        match load_player_name_race(conn, member_low)? {
            Some((name, race)) => members.push(LoadedMember {
                member_low,
                name,
                race,
                assistant,
                subgroup,
            }),
            None => delete_group_member(conn, member_low)?,
        }
    }

    let mut surviving = Vec::new();
    for entry in groups {
        if entry.2.len() < 2 || entry.2.len() > 40 {
            delete_group(conn, entry.0.group_id)?;
        } else {
            surviving.push(entry);
        }
    }
    Ok(LoadedGroups {
        groups: surviving,
        next_group_id: max_id.saturating_add(1).max(1),
    })
}

/// `Group::Create` store: clear any stale rows for the id,
/// then the `REPLACE INTO groups` with all columns.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_group(conn: &mut mysql::Conn, group: &StoredGroup) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `groups` WHERE `group_id` = ?",
        (group.group_id,),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `group_member` WHERE `group_id` = ?",
        (group.group_id,),
    )
    .map_err(DbError::Driver)?;
    // Sixteen columns exceed the driver's tuple-Params
    // arity; positional values keep one statement.
    let mut values: Vec<mysql::Value> = vec![
        mysql::Value::UInt(u64::from(group.group_id)),
        mysql::Value::UInt(u64::from(group.leader_low)),
        mysql::Value::UInt(u64::from(group.main_tank_low)),
        mysql::Value::UInt(u64::from(group.main_assistant_low)),
        mysql::Value::UInt(u64::from(group.loot_method)),
        mysql::Value::UInt(u64::from(group.looter_low)),
        mysql::Value::UInt(u64::from(group.loot_threshold)),
    ];
    values.extend(group.icons.iter().map(|icon| mysql::Value::UInt(*icon)));
    values.push(mysql::Value::UInt(u64::from(group.is_raid)));
    conn.exec_drop(
        "REPLACE INTO `groups` (`group_id`, `leader_guid`, `main_tank_guid`, \
         `main_assistant_guid`, `loot_method`, `looter_guid`, `loot_threshold`, \
         `icon1`, `icon2`, `icon3`, `icon4`, `icon5`, `icon6`, `icon7`, `icon8`, \
         `is_raid`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        mysql::Params::Positional(values),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::Disband` deletes.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_group(conn: &mut mysql::Conn, group_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `groups` WHERE `group_id` = ?", (group_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `group_member` WHERE `group_id` = ?",
        (group_id,),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_addMember` insert.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_group_member(
    conn: &mut mysql::Conn,
    group_id: u32,
    member_low: u32,
    assistant: bool,
    subgroup: u8,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `group_member` (`group_id`, `member_guid`, `assistant`, \
         `subgroup`) VALUES (?, ?, ?, ?)",
        (
            group_id,
            member_low,
            u32::from(assistant),
            u32::from(subgroup),
        ),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_removeMember` delete (by member, not group).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_group_member(conn: &mut mysql::Conn, member_low: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `group_member` WHERE `member_guid` = ?",
        (member_low,),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_setLeader` leader update (the `group_instance`
/// shuffling around it is Phase 6).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_group_leader(
    conn: &mut mysql::Conn,
    group_id: u32,
    leader_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `groups` SET `leader_guid` = ? WHERE `group_id` = ?",
        (leader_low, group_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::ConvertToRaid` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_group_raid(conn: &mut mysql::Conn, group_id: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `groups` SET `is_raid` = 1 WHERE `group_id` = ?",
        (group_id,),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_setMembersGroup` / `_swapMembersGroup` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_member_subgroup(
    conn: &mut mysql::Conn,
    member_low: u32,
    subgroup: u8,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `group_member` SET `subgroup` = ? WHERE `member_guid` = ?",
        (u32::from(subgroup), member_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_setAssistantFlag` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_member_assistant(
    conn: &mut mysql::Conn,
    member_low: u32,
    assistant: bool,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `group_member` SET `assistant` = ? WHERE `member_guid` = ?",
        (u32::from(assistant), member_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_setMainTank` update (main tank is only written
/// here; the client never sends it in 1.12 — kept for the
/// stored-row round trip).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_main_tank(
    conn: &mut mysql::Conn,
    group_id: u32,
    main_tank_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `groups` SET `main_tank_guid` = ? WHERE `group_id` = ?",
        (main_tank_low, group_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Group::_setMainAssistant` update (same note as
/// [`update_main_tank`]).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_main_assistant(
    conn: &mut mysql::Conn,
    group_id: u32,
    main_assistant_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `groups` SET `main_assistant_guid` = ? WHERE `group_id` = ?",
        (main_assistant_low, group_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}
