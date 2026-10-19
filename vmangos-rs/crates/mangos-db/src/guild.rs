//! Guild persistence (`Guild` / `GuildMgr::LoadGuilds`,
//! `Guild.cpp`, `GuildMgr.cpp:102-160`): the `guild`,
//! `guild_rank`, `guild_member`, and `guild_eventlog`
//! statements the C++ guild service issues, plus the
//! boot load. Boot-repair decisions that need world
//! data (default rank names from `mangos_string`,
//! structure repair) live in the consumer; this module
//! applies only the row-level cleanups `LoadGuilds`
//! performs unconditionally: rank/member rows for
//! missing guilds are deleted, members with a broken
//! character row (level/class/name) are deleted, and
//! event-log rows above the configured count are
//! pruned. Errors are returned for the caller to log —
//! C++ queues the writes with `PExecute` and cannot
//! observe failure.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// One `guild` row (`LoadGuildFromDB` field order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredGuild {
    pub guild_id: u32,
    pub name: String,
    pub leader_low: u32,
    pub emblem_style: i32,
    pub emblem_color: i32,
    pub border_style: i32,
    pub border_color: i32,
    pub background_color: i32,
    pub info: String,
    pub motd: String,
    pub create_date: u64,
}

/// One `guild_rank` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredGuildRank {
    pub rank_id: u32,
    pub name: String,
    pub rights: u32,
}

/// One boot-loaded `guild_member` row joined with its
/// `characters` fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedGuildMember {
    pub guid_low: u32,
    pub rank_id: u32,
    pub pnote: String,
    pub offnote: String,
    pub name: String,
    pub level: u8,
    pub class: u8,
    pub zone_id: u32,
    pub logout_time: u64,
    pub account_id: u32,
}

/// One `guild_eventlog` row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredGuildEvent {
    pub log_guid: u32,
    pub event_type: u8,
    pub player_guid1: u32,
    pub player_guid2: u32,
    pub new_rank: u8,
    pub timestamp: u64,
}

/// One boot-loaded guild with its rows. `events` are
/// newest-first as queried (`LoadGuildEventLogFromDB`
/// reverses them into oldest-first order in memory).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedGuild {
    pub guild: StoredGuild,
    pub ranks: Vec<StoredGuildRank>,
    pub members: Vec<LoadedGuildMember>,
    pub events: Vec<StoredGuildEvent>,
}

/// Boot-load output plus the next guild id
/// (`GenerateGuildId` seeded from `MAX(guild_id)`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedGuilds {
    pub guilds: Vec<LoadedGuild>,
    pub next_guild_id: u32,
}

/// `PLAYER_STRONG_MAX_LEVEL` cap used by the member
/// validation, and `CLASSMASK_ALL_PLAYABLE` shape check.
const STRONG_MAX_LEVEL: u8 = 60;

fn class_playable(class: u8) -> bool {
    // (1 << (class - 1)) & CLASSMASK_ALL_PLAYABLE, with
    // the mask covering the nine 1.12 classes (1-9,11).
    matches!(class, 1..=9 | 11)
}

/// `GuildMgr::LoadGuilds` row loading with its
/// unconditional cleanups (see module docs). Guilds
/// with no surviving members are returned with an
/// empty member list; the consumer disbands them, as
/// `LoadMembersFromDB` returning false does in C++.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_guilds(conn: &mut mysql::Conn, event_log_count: u32) -> Result<LoadedGuilds, DbError> {
    let guild_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guild_id`, `name`, `leader_guid`, `emblem_style`, `emblem_color`, \
         `border_style`, `border_color`, `background_color`, `info`, `motd`, \
         `create_date` FROM `guild` ORDER BY `guild_id` ASC",
        (),
    )?;
    let mut max_id = 0u32;
    let mut guilds: Vec<LoadedGuild> = Vec::new();
    for row in guild_rows {
        let f = Fields::from_row(row);
        let guild = StoredGuild {
            guild_id: f.u32(0),
            name: f.string(1),
            leader_low: f.u32(2),
            emblem_style: f.i32(3),
            emblem_color: f.i32(4),
            border_style: f.i32(5),
            border_color: f.i32(6),
            background_color: f.i32(7),
            info: f.string(8),
            motd: f.string(9),
            create_date: f.u64(10),
        };
        max_id = max_id.max(guild.guild_id);
        guilds.push(LoadedGuild {
            guild,
            ranks: Vec::new(),
            members: Vec::new(),
            events: Vec::new(),
        });
    }

    load_ranks_into(conn, &mut guilds)?;
    load_members_into(conn, &mut guilds)?;
    for loaded in &mut guilds {
        load_events_into(conn, loaded)?;
    }
    conn.exec_drop(
        "DELETE FROM `guild_eventlog` WHERE `log_guid` > ?",
        (event_log_count,),
    )
    .map_err(DbError::Driver)?;

    Ok(LoadedGuilds {
        guilds,
        next_guild_id: max_id.saturating_add(1).max(1),
    })
}

/// `guild_rank` rows into their guilds; rows for missing
/// guilds delete that id's ranks (C++ logs and deletes).
fn load_ranks_into(conn: &mut mysql::Conn, guilds: &mut [LoadedGuild]) -> Result<(), DbError> {
    let rank_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guild_id`, `id`, `name`, `rights` FROM `guild_rank` \
         ORDER BY `guild_id` ASC, `id` ASC",
        (),
    )?;
    for row in rank_rows {
        let f = Fields::from_row(row);
        let guild_id = f.u32(0);
        let Some(loaded) = guilds.iter_mut().find(|g| g.guild.guild_id == guild_id) else {
            delete_all_ranks(conn, guild_id)?;
            continue;
        };
        loaded.ranks.push(StoredGuildRank {
            rank_id: f.u32(1),
            name: f.string(2),
            rights: f.u32(3),
        });
    }
    Ok(())
}

/// `guild_member` rows joined to `characters`, with the
/// broken-row cleanups.
fn load_members_into(conn: &mut mysql::Conn, guilds: &mut [LoadedGuild]) -> Result<(), DbError> {
    let member_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guild_id`, `guild_member`.`guid`, `rank`, `player_note`, \
         `officer_note`, `characters`.`name`, `characters`.`level`, \
         `characters`.`class`, `characters`.`zone`, `characters`.`logout_time`, \
         `characters`.`account` FROM `guild_member` \
         LEFT JOIN `characters` ON `characters`.`guid` = `guild_member`.`guid` \
         ORDER BY `guild_id` ASC",
        (),
    )?;
    for row in member_rows {
        let f = Fields::from_row(row);
        let guild_id = f.u32(0);
        let guid_low = f.u32(1);
        let Some(loaded) = guilds.iter_mut().find(|g| g.guild.guild_id == guild_id) else {
            conn.exec_drop(
                "DELETE FROM `guild_member` WHERE `guild_id` = ?",
                (guild_id,),
            )
            .map_err(DbError::Driver)?;
            continue;
        };
        let member = LoadedGuildMember {
            guid_low,
            rank_id: f.u32(2),
            pnote: f.string(3),
            offnote: f.string(4),
            name: f.string(5),
            level: f.u8(6),
            class: f.u8(7),
            zone_id: f.u32(8),
            logout_time: f.u64(9),
            account_id: f.u32(10),
        };
        if member.level < 1 || member.level > STRONG_MAX_LEVEL {
            delete_guild_member(conn, guid_low)?;
            continue;
        }
        if !class_playable(member.class) {
            delete_guild_member(conn, guid_low)?;
            continue;
        }
        if member.name.is_empty() {
            delete_guild_member(conn, guid_low)?;
            continue;
        }
        loaded.members.push(member);
    }
    Ok(())
}

/// One guild's newest 100 event-log rows.
fn load_events_into(conn: &mut mysql::Conn, loaded: &mut LoadedGuild) -> Result<(), DbError> {
    let event_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `log_guid`, `event_type`, `player_guid1`, `player_guid2`, \
         `new_rank`, `timestamp` FROM `guild_eventlog` WHERE `guild_id` = ? \
         ORDER BY `timestamp` DESC, `log_guid` DESC LIMIT 100",
        (loaded.guild.guild_id,),
    )?;
    for row in event_rows {
        let f = Fields::from_row(row);
        loaded.events.push(StoredGuildEvent {
            log_guid: f.u32(0),
            event_type: f.u8(1),
            player_guid1: f.u32(2),
            player_guid2: f.u32(3),
            new_rank: f.u8(4),
            timestamp: f.u64(5),
        });
    }
    Ok(())
}

/// The `characters` fields `Guild::AddMember` reads from
/// the player cache for an offline target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuildCandidate {
    pub name: String,
    pub level: u8,
    pub class: u8,
    pub zone_id: u32,
    pub account_id: u32,
}

/// `ObjectMgr::GetPlayerDataByGUID` subset for guild
/// joins.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_guild_candidate(
    conn: &mut mysql::Conn,
    guid_low: u32,
) -> Result<Option<GuildCandidate>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `name`, `level`, `class`, `zone`, `account` FROM `characters` \
         WHERE `guid` = ?",
        (guid_low,),
    )?;
    Ok(rows.into_iter().next().map(|row| {
        let f = Fields::from_row(row);
        GuildCandidate {
            name: f.string(0),
            level: f.u8(1),
            class: f.u8(2),
            zone_id: f.u32(3),
            account_id: f.u32(4),
        }
    }))
}

/// `Guild::Create` store: clear member rows for the id,
/// then insert the `guild` row.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_guild(conn: &mut mysql::Conn, guild: &StoredGuild) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `guild_member` WHERE `guild_id` = ?",
        (guild.guild_id,),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "INSERT INTO `guild` (`guild_id`, `name`, `leader_guid`, `info`, `motd`, \
         `create_date`, `emblem_style`, `emblem_color`, `border_style`, \
         `border_color`, `background_color`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            guild.guild_id,
            guild.name.as_str(),
            guild.leader_low,
            guild.info.as_str(),
            guild.motd.as_str(),
            guild.create_date,
            guild.emblem_style,
            guild.emblem_color,
            guild.border_style,
            guild.border_color,
            guild.background_color,
        ),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::Disband` table deletes (member rows are
/// removed per member by `DelMember` first).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_guild(conn: &mut mysql::Conn, guild_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `guild` WHERE `guild_id` = ?", (guild_id,))
        .map_err(DbError::Driver)?;
    delete_all_ranks(conn, guild_id)?;
    conn.exec_drop(
        "DELETE FROM `guild_eventlog` WHERE `guild_id` = ?",
        (guild_id,),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::Rename` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn rename_guild(conn: &mut mysql::Conn, guild_id: u32, name: &str) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild` SET `name` = ? WHERE `guild_id` = ?",
        (name, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetLeader` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_guild_leader(
    conn: &mut mysql::Conn,
    guild_id: u32,
    leader_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild` SET `leader_guid` = ? WHERE `guild_id` = ?",
        (leader_low, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetMOTD` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_guild_motd(conn: &mut mysql::Conn, guild_id: u32, motd: &str) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild` SET `motd` = ? WHERE `guild_id` = ?",
        (motd, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetGINFO` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_guild_info(conn: &mut mysql::Conn, guild_id: u32, info: &str) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild` SET `info` = ? WHERE `guild_id` = ?",
        (info, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetEmblem` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_guild_emblem(
    conn: &mut mysql::Conn,
    guild_id: u32,
    emblem: [i32; 5],
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild` SET `emblem_style` = ?, `emblem_color` = ?, \
         `border_style` = ?, `border_color` = ?, `background_color` = ? \
         WHERE `guild_id` = ?",
        (
            emblem[0], emblem[1], emblem[2], emblem[3], emblem[4], guild_id,
        ),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::AddMember` insert.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_guild_member(
    conn: &mut mysql::Conn,
    guild_id: u32,
    member_low: u32,
    rank_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `guild_member` (`guild_id`, `guid`, `rank`, `player_note`, \
         `officer_note`) VALUES (?, ?, ?, '', '')",
        (guild_id, member_low, rank_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::DelMember` delete (by member).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_guild_member(conn: &mut mysql::Conn, member_low: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `guild_member` WHERE `guid` = ?", (member_low,))
        .map_err(DbError::Driver)?;
    Ok(())
}

/// `MemberSlot::ChangeRank` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_member_rank(
    conn: &mut mysql::Conn,
    member_low: u32,
    rank_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild_member` SET `rank` = ? WHERE `guid` = ?",
        (rank_id, member_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `MemberSlot::SetPNOTE` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_member_pnote(
    conn: &mut mysql::Conn,
    member_low: u32,
    note: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild_member` SET `player_note` = ? WHERE `guid` = ?",
        (note, member_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `MemberSlot::SetOFFNOTE` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_member_offnote(
    conn: &mut mysql::Conn,
    member_low: u32,
    note: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild_member` SET `officer_note` = ? WHERE `guid` = ?",
        (note, member_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::CreateRank` insert.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_rank(
    conn: &mut mysql::Conn,
    guild_id: u32,
    rank_id: u32,
    name: &str,
    rights: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `guild_rank` (`guild_id`, `id`, `name`, `rights`) VALUES (?, ?, ?, ?)",
        (guild_id, rank_id, name, rights),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// The rank-table clear inside `CreateDefaultGuildRanks`
/// and the broken-ranks repair.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_all_ranks(conn: &mut mysql::Conn, guild_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `guild_rank` WHERE `guild_id` = ?", (guild_id,))
        .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::DelRank` delete (the lowest rank and beyond).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_ranks_from(
    conn: &mut mysql::Conn,
    guild_id: u32,
    rank_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `guild_rank` WHERE `id` >= ? AND `guild_id` = ?",
        (rank_id, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetRankName` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_rank_name(
    conn: &mut mysql::Conn,
    guild_id: u32,
    rank_id: u32,
    name: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild_rank` SET `name` = ? WHERE `id` = ? AND `guild_id` = ?",
        (name, rank_id, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::SetRankRights` update.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_rank_rights(
    conn: &mut mysql::Conn,
    guild_id: u32,
    rank_id: u32,
    rights: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `guild_rank` SET `rights` = ? WHERE `id` = ? AND `guild_id` = ?",
        (rights, rank_id, guild_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `Guild::LogGuildEvent` replace.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn replace_event_log(
    conn: &mut mysql::Conn,
    guild_id: u32,
    event: StoredGuildEvent,
) -> Result<(), DbError> {
    conn.exec_drop(
        "REPLACE INTO `guild_eventlog` (`guild_id`, `log_guid`, `event_type`, \
         `player_guid1`, `player_guid2`, `new_rank`, `timestamp`) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        (
            guild_id,
            event.log_guid,
            u32::from(event.event_type),
            event.player_guid1,
            event.player_guid2,
            u32::from(event.new_rank),
            event.timestamp,
        ),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}
