//! `game_event_status` persistence (`GameEventMgr::
//! Initialize`/`ApplyNewEvent`/`UnApplyEvent`
//! character-DB half, Tier 4 B0-8).
//!
//! Fork statements, verbatim:
//! - boot: `SELECT event FROM game_event_status`,
//!   then `TRUNCATE game_event_status` — the loaded
//!   set is "active at shutdown" (its only fork use
//!   is the `resume` flag suppressing event mails;
//!   mail sending itself is unmodeled, see
//!   TIER4-PROGRESS B0-8).
//! - start: `INSERT IGNORE INTO game_event_status
//!   (event) VALUES (?)`.
//! - stop: `DELETE FROM game_event_status WHERE
//!   event = ?`.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// Boot read + truncate (`GameEventMgr::Initialize`
/// character-DB half). Returns the events that were
/// active at the previous shutdown.
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn load_and_clear_status(conn: &mut mysql::Conn) -> Result<Vec<u16>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec("SELECT event FROM game_event_status", ())?;
    let events: Vec<u16> = rows
        .into_iter()
        .map(|row| Fields::from_row(row).u32(0))
        .filter_map(|v| u16::try_from(v).ok())
        .collect();
    conn.exec_drop("TRUNCATE game_event_status", ())?;
    Ok(events)
}

/// `ApplyNewEvent` status write.
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn insert_status(conn: &mut mysql::Conn, event: u16) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT IGNORE INTO game_event_status (event) VALUES (?)",
        (u32::from(event),),
    )?;
    Ok(())
}

/// `UnApplyEvent` status write.
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn delete_status(conn: &mut mysql::Conn, event: u16) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM game_event_status WHERE event = ?",
        (u32::from(event),),
    )?;
    Ok(())
}
