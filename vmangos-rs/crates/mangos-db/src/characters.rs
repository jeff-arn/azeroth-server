//! Character-database access for the Phase 3 login path:
//! the char-enum query (`CharacterHandler.cpp:167-178`) and the
//! synchronous account-data/tutorial loads that run during
//! world auth (`WorldSession.cpp:872-908,972-995`). Phase 3 is
//! read-only here: `SetAccountData`/tutorial *saves* are write
//! paths and stay unported with character save (Phase 6).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::actor::DbActor;

/// One row of the 23-column char-enum SELECT, in column order.
/// Join-nullable fields (`guild_member`, `character_pet`) are
/// `Option`; C++ reads them through NULL-tolerant `Field`
/// accessors (NULL → 0 / "").
#[derive(Clone, Debug, PartialEq)]
pub struct CharEnumRow {
    pub guid: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub level: u8,
    pub zone: u32,
    pub map: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub guild_id: Option<u32>,
    pub character_flags: u32,
    pub played_time_total: u32,
    pub pet_entry: Option<u32>,
    pub pet_display_id: Option<u32>,
    pub pet_level: Option<u32>,
    pub equipment_cache: Option<String>,
}

/// Transcribed verbatim from `CharacterHandler.cpp:169-177`
/// (account becomes a bind; `PET_SAVE_AS_CURRENT` stays literal).
const CHAR_ENUM_SQL: &str = "SELECT `characters`.`guid`, `characters`.`name`, `characters`.`race`, \
    `characters`.`class`, `characters`.`gender`, `characters`.`skin`, `characters`.`face`, \
    `characters`.`hair_style`, `characters`.`hair_color`, `characters`.`facial_hair`, \
    `characters`.`level`, `characters`.`zone`, `characters`.`map`, `characters`.`position_x`, \
    `characters`.`position_y`, `characters`.`position_z`, `guild_member`.`guild_id`, \
    `characters`.`character_flags`, `characters`.`played_time_total`, `character_pet`.`entry`, \
    `character_pet`.`display_id`, `character_pet`.`level`, `characters`.`equipment_cache` \
    FROM `characters` \
    LEFT JOIN `character_pet` ON `characters`.`guid` = `character_pet`.`owner_guid` \
        AND `character_pet`.`slot` = '0' \
    LEFT JOIN `guild_member` ON `characters`.`guid` = `guild_member`.`guid` \
    WHERE `characters`.`account` = ? \
    ORDER BY `characters`.`create_time`, `characters`.`guid` LIMIT 0, 10";

fn map_char_enum_row(f: &crate::row::Fields) -> CharEnumRow {
    let opt_u32 = |i: usize| (!f.is_null(i)).then(|| f.u32(i));
    CharEnumRow {
        guid: f.u32(0),
        name: f.string(1),
        race: f.u8(2),
        class: f.u8(3),
        gender: f.u8(4),
        skin: f.u8(5),
        face: f.u8(6),
        hair_style: f.u8(7),
        hair_color: f.u8(8),
        facial_hair: f.u8(9),
        level: f.u8(10),
        zone: f.u32(11),
        map: f.u32(12),
        position_x: f.f32(13),
        position_y: f.f32(14),
        position_z: f.f32(15),
        guild_id: opt_u32(16),
        character_flags: f.u32(17),
        played_time_total: f.u32(18),
        pet_entry: opt_u32(19),
        pet_display_id: opt_u32(20),
        pet_level: opt_u32(21),
        equipment_cache: (!f.is_null(22)).then(|| f.string(22)),
    }
}

/// One `account_data` / `character_account_data` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDataEntry {
    pub data_type: u32,
    pub timestamp: u64,
    pub data: String,
}

/// The identity fields a `SMSG_NAME_QUERY_RESPONSE` serves
/// (`QueryHandler.cpp:57-79` over the `characters` row).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameQueryRow {
    pub name: String,
    pub race: u8,
    pub gender: u8,
    pub class: u8,
}

type TutorialRow = (u32, u32, u32, u32, u32, u32, u32, u32);

/// Handle to the character-DB actor.
pub struct CharacterDb {
    actor: DbActor,
}

impl CharacterDb {
    /// Connect and spawn (see [`DbActor::spawn`]).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on a bad URL or rejected connection.
    pub fn spawn(url: &str) -> Result<CharacterDb, DbError> {
        Ok(CharacterDb {
            actor: DbActor::spawn("character-db", url)?,
        })
    }

    #[must_use]
    pub fn actor(&self) -> &DbActor {
        &self.actor
    }

    /// The `CMSG_CHAR_ENUM` query (`PET_SAVE_AS_CURRENT = 0`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn char_enum(&self, account_id: u32) -> Result<Vec<CharEnumRow>, DbError> {
        self.actor.call(move |conn| {
            let rows: Vec<mysql::Row> = conn.exec(CHAR_ENUM_SQL, (account_id,))?;
            Ok(rows
                .into_iter()
                .map(|row| map_char_enum_row(&crate::row::Fields::from_row(row)))
                .collect())
        })
    }

    /// `SELECT type, time, data FROM account_data WHERE account = ?`
    /// (`WorldSession.cpp:874`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn load_global_account_data(
        &self,
        account_id: u32,
    ) -> Result<Vec<AccountDataEntry>, DbError> {
        self.actor.call(move |conn| {
            let rows: Vec<(u32, u64, Option<String>)> = conn.exec(
                "SELECT `type`, `time`, `data` FROM `account_data` WHERE `account` = ?",
                (account_id,),
            )?;
            Ok(map_account_data(rows))
        })
    }

    /// `SELECT type, time, data FROM character_account_data
    /// WHERE guid = ?` (`CharacterHandler.cpp:96`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn load_character_account_data(
        &self,
        guid_low: u32,
    ) -> Result<Vec<AccountDataEntry>, DbError> {
        self.actor.call(move |conn| {
            let rows: Vec<(u32, u64, Option<String>)> = conn.exec(
                "SELECT `type`, `time`, `data` FROM `character_account_data` WHERE `guid` = ?",
                (guid_low,),
            )?;
            Ok(map_account_data(rows))
        })
    }

    /// The full 21-query read-only character load
    /// (`LoginQueryHolder`, see [`crate::character_load`]).
    /// `Ok(None)` = no `characters` row.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn load_character(
        &self,
        guid_low: u32,
    ) -> Result<Option<crate::character_load::CharacterLoad>, DbError> {
        self.actor
            .call(move |conn| crate::character_load::load_character(conn, guid_low))
    }

    /// The `CMSG_NAME_QUERY` fallback identity lookup, serving
    /// the fields the C++ keeps in `PlayerCacheData`
    /// (`ObjectMgr.cpp:663-681`); `Ok(None)` = no such character.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn name_query(&self, guid_low: u32) -> Result<Option<NameQueryRow>, DbError> {
        self.actor.call(move |conn| {
            let row: Option<mysql::Row> = conn.exec_first(
                "SELECT `name`, `race`, `gender`, `class` FROM `characters` WHERE `guid` = ?",
                (guid_low,),
            )?;
            Ok(row.map(|row| {
                let fields = crate::row::Fields::from_row(row);
                NameQueryRow {
                    name: fields.string(0),
                    race: fields.u8(1),
                    gender: fields.u8(2),
                    class: fields.u8(3),
                }
            }))
        })
    }

    /// Tutorial flags; `None` when the account has no row
    /// (`TUTORIALDATA_NEW`, `WorldSession.cpp:972-995`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn load_tutorials(&self, account_id: u32) -> Result<Option<[u32; 8]>, DbError> {
        self.actor.call(move |conn| {
            let row: Option<TutorialRow> = conn.exec_first(
                "SELECT `tut0`, `tut1`, `tut2`, `tut3`, `tut4`, `tut5`, `tut6`, `tut7` \
                 FROM `character_tutorial` WHERE `account` = ?",
                (account_id,),
            )?;
            Ok(row.map(|r| [r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7]))
        })
    }
}

fn map_account_data(rows: Vec<(u32, u64, Option<String>)>) -> Vec<AccountDataEntry> {
    rows.into_iter()
        .map(|(data_type, timestamp, data)| AccountDataEntry {
            data_type,
            timestamp,
            data: data.unwrap_or_default(),
        })
        .collect()
}
