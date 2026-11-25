//! `playerbot` table access (`PlayerBotMgr::Load`
//! and the director's chance updates).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// One `playerbot` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerbotRow {
    pub char_guid: u32,
    pub chance: u32,
    pub ai: String,
}

/// `SELECT char_guid, chance, ai FROM playerbot`.
/// A missing table reads as empty (fork logs
/// "Table `playerbot` is empty." on no result).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_playerbots(conn: &mut mysql::Conn) -> Result<Vec<PlayerbotRow>, DbError> {
    let rows: Vec<mysql::Row> =
        conn.query("SELECT `char_guid`, `chance`, `ai` FROM `playerbot`")?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            PlayerbotRow {
                char_guid: f.u32(0),
                chance: f.u32(1),
                ai: f.string(2),
            }
        })
        .collect())
}

/// `UPDATE playerbot SET chance = ? WHERE char_guid = ?`
/// for each (guid, chance) pair.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_playerbot_chances(
    conn: &mut mysql::Conn,
    chances: &[(u32, u32)],
) -> Result<(), DbError> {
    for (guid, chance) in chances {
        conn.exec_drop(
            "UPDATE `playerbot` SET `chance` = ? WHERE `char_guid` = ?",
            (*chance, *guid),
        )?;
    }
    Ok(())
}
