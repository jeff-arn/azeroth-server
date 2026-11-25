//! Persisted player corpses (`Corpse::SaveToDB`/
//! `DeleteFromDB`, `ObjectMgr::LoadCorpses`): the
//! `corpse` table rows plus the owner-appearance join
//! the boot load renders from.

use mysql::prelude::Queryable as _;

use crate::DbError;
use crate::row::Fields;

/// One `corpse` table row (`Corpse::SaveToDB` column
/// list; `corpse_type` 1 = `PvE`, 2 = `PvP`; bones are
/// never persisted).
#[derive(Clone, Debug, PartialEq)]
pub struct CorpseRow {
    pub guid: u32,
    pub player_guid: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation: f32,
    pub map: u32,
    /// Ghost (creation) time, unix seconds.
    pub time: u64,
    pub corpse_type: u32,
    pub instance: u32,
}

/// A boot-loaded corpse with the owner appearance the
/// `LoadCorpses` join supplies (`Corpse::LoadFromDB`
/// field order).
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedCorpse {
    pub row: CorpseRow,
    pub gender: u8,
    pub race: u8,
    pub class: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub equipment_cache: Option<String>,
    pub guild_id: u32,
    pub character_flags: u32,
}

/// `Corpse::SaveToDB` REPLACE.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_corpse(conn: &mut mysql::Conn, row: &CorpseRow) -> Result<(), DbError> {
    conn.exec_drop(
        "REPLACE INTO `corpse` (`guid`, `player_guid`, `position_x`, `position_y`, \
         `position_z`, `orientation`, `map`, `time`, `corpse_type`, `instance`) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            row.guid,
            row.player_guid,
            row.position_x,
            row.position_y,
            row.position_z,
            row.orientation,
            row.map,
            row.time,
            row.corpse_type,
            row.instance,
        ),
    )
    .map_err(DbError::Driver)
}

/// `Corpse::DeleteFromDB`: all resurrectable rows of a
/// player.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_corpse_for_player(conn: &mut mysql::Conn, player_low: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `corpse` WHERE `player_guid` = ? AND `corpse_type` <> 0",
        (player_low,),
    )
    .map_err(DbError::Driver)
}

/// The `LoadCorpses` boot read (appearance join
/// included), resurrectable rows only.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_corpses(conn: &mut mysql::Conn) -> Result<Vec<LoadedCorpse>, DbError> {
    let rows: Vec<LoadedCorpse> = conn.exec_map(
        "SELECT `corpse`.`guid`, `player_guid`, `corpse`.`position_x`, `corpse`.`position_y`, \
         `corpse`.`position_z`, `corpse`.`orientation`, `corpse`.`map`, \
         `time`, `corpse_type`, `corpse`.`instance`, `gender`, `race`, `class`, `skin`, \
         `face`, `hair_style`, `hair_color`, `facial_hair`, `equipment_cache`, `guild_id`, \
         `character_flags` FROM `corpse` \
         JOIN `characters` ON `player_guid` = `characters`.`guid` \
         LEFT JOIN `guild_member` ON `player_guid` = `guild_member`.`guid` \
         WHERE `corpse_type` <> 0",
        (),
        |row| {
            let f = Fields::from_row(row);
            LoadedCorpse {
                row: CorpseRow {
                    guid: f.u32(0),
                    player_guid: f.u32(1),
                    position_x: f.f32(2),
                    position_y: f.f32(3),
                    position_z: f.f32(4),
                    orientation: f.f32(5),
                    map: f.u32(6),
                    time: f.u64(7),
                    corpse_type: f.u32(8),
                    instance: f.u32(9),
                },
                gender: f.u8(10),
                race: f.u8(11),
                class: f.u8(12),
                skin: f.u8(13),
                face: f.u8(14),
                hair_style: f.u8(15),
                hair_color: f.u8(16),
                facial_hair: f.u8(17),
                equipment_cache: (!f.is_null(18)).then(|| f.string(18)),
                guild_id: f.u32(19),
                character_flags: f.u32(20),
            }
        },
    )?;
    Ok(rows)
}

/// `GenerateCorpseLowGuid` seed: highest persisted
/// corpse guid (0 when the table is empty).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn max_corpse_guid(conn: &mut mysql::Conn) -> Result<u32, DbError> {
    let row: Option<mysql::Row> =
        conn.exec_first("SELECT IFNULL(MAX(`guid`), 0) FROM `corpse`", ())?;
    Ok(row.map_or(0, |row| Fields::from_row(row).u32(0)))
}
