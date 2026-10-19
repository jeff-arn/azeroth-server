//! Character social writes (`PlayerSocial::AddToSocialList` /
//! `RemoveFromSocialList`, `SocialMgr.cpp:50-107`) and the
//! player-cache name lookup the friend/ignore handlers use
//! (`ObjectMgr::GetPlayerDataByName`). The C++ statements are
//! the immediate `CharacterDatabase.PExecute` calls; the Rust
//! caller chooses insert-vs-update from its in-memory social
//! list exactly as `PlayerSocial` does (the table primary key
//! is `(guid, friend, flags)`, so a blind upsert would create
//! a second row instead of merging flags).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// The `PlayerCacheData` fields the friend/ignore handlers
/// consume: guid low and race (for the two-side check).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamedPlayerData {
    pub guid_low: u32,
    pub race: u8,
}

/// `ObjectMgr::GetPlayerDataByName` over the characters
/// table (the C++ cache is this table loaded at boot; the
/// name arriving here is already `normalizePlayerName`-shaped).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_player_data_by_name(
    conn: &mut mysql::Conn,
    name: &str,
) -> Result<Option<NamedPlayerData>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `race` FROM `characters` WHERE `name` = ?",
        (name,),
    )?;
    Ok(rows.into_iter().next().map(|row| {
        let f = Fields::from_row(row);
        NamedPlayerData {
            guid_low: f.u32(0),
            race: f.u8(1),
        }
    }))
}

/// `INSERT INTO character_social (guid, friend, flags)`.
/// Caller guarantees no row exists for the pair (C++
/// `AddToSocialList` else-arm).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_social(
    conn: &mut mysql::Conn,
    guid_low: u32,
    friend_low: u32,
    flag: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `character_social` (`guid`, `friend`, `flags`) VALUES (?, ?, ?)",
        (guid_low, friend_low, flag),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `UPDATE character_social SET flags = (flags | ?)` for an
/// existing pair (`AddToSocialList` if-arm).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn add_social_flag(
    conn: &mut mysql::Conn,
    guid_low: u32,
    friend_low: u32,
    flag: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_social` SET `flags` = (`flags` | ?) \
         WHERE `guid` = ? AND `friend` = ?",
        (flag, guid_low, friend_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `UPDATE character_social SET flags = (flags & ~?)` when
/// other flags remain (`RemoveFromSocialList` else-arm).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn clear_social_flag(
    conn: &mut mysql::Conn,
    guid_low: u32,
    friend_low: u32,
    flag: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `character_social` SET `flags` = (`flags` & ~?) \
         WHERE `guid` = ? AND `friend` = ?",
        (flag, guid_low, friend_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// `DELETE FROM character_social` when the last flag is
/// removed (`RemoveFromSocialList` if-arm).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_social(
    conn: &mut mysql::Conn,
    guid_low: u32,
    friend_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_social` WHERE `guid` = ? AND `friend` = ?",
        (guid_low, friend_low),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}
