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

/// Everything `Player::SaveNewPlayer` (`Player.cpp:16051`)
/// writes for a fresh character, precomputed by the world
/// actor from the create-info stores (position, base
/// health/power, taxi mask, equipment cache, starter
/// actions). Starting *items* are not rows yet, exactly
/// like the fork: they materialize at first login
/// (`AddStartingItems`, `Player.cpp:14883`).
#[derive(Clone, Debug, PartialEq)]
pub struct NewCharacter {
    pub account_id: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub level: u32,
    pub money: u32,
    pub map_id: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation: f32,
    /// `known_taxi_mask` text: 8 words, word 0 the race
    /// `startingTaxiMask` (`PlayerTaxi::InitTaxiNodes`).
    pub taxi_mask: String,
    /// `PLAYER_EXTRA_*` bits (`SaveNewPlayer`: whispers for
    /// players, GM tickets above `SEC_PLAYER`).
    pub extra_flags: u32,
    pub health: u32,
    /// `power1..=power5` (`MAX_POWERS`).
    pub powers: [u32; 5],
    /// `equipment_cache` text: 20 `(item, enchant)` pairs
    /// (`INVENTORY_SLOT_BAG_START + 1` slots).
    pub equipment_cache: String,
    pub ammo_id: u32,
    /// `playercreateinfo_action` rows, written to
    /// `character_action`.
    pub actions: Vec<(u8, u32, u8)>,
    /// `create_time`/`logout_time` stamp.
    pub now: u64,
}

/// The delete-time identity read (`PlayerCacheData`
/// fields the delete handler consults: account gate,
/// `CharDelete.MinLevel` resolution, log line).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterOwner {
    pub account_id: u32,
    pub name: String,
    pub level: u32,
}

/// A resolved `Player::DeleteFromDB` plan
/// (`Player.cpp:4361`): the actor already folded
/// `CharDelete.Method`/`MinLevel` into `method`
/// (0 = full cascade, 1 = soft unlink).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeletePlan {
    pub method: u8,
    /// The deleted character's account (COD-return
    /// delivery-delay comparison, `SendReturnToSender`).
    pub account_id: u32,
    /// `MailDeliveryDelay` seconds for returned COD mail
    /// crossing accounts.
    pub mail_delivery_delay_secs: u32,
    pub now: u64,
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

    /// `ObjectMgr::GetPlayerGuidByName` existence half
    /// (name-in-use gate at create; soft-deleted rows
    /// carry `name=''` so they never match).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn name_in_use(&self, name: &str) -> Result<bool, DbError> {
        let name = name.to_owned();
        self.actor.call(move |conn| {
            let row: Option<mysql::Row> = conn.exec_first(
                "SELECT `guid` FROM `characters` WHERE `name` = ?",
                (name.as_str(),),
            )?;
            Ok(row.is_some())
        })
    }

    /// `SELECT COUNT(guid) ... WHERE account = ?`
    /// (`World::_UpdateRealmCharCount` recount and the
    /// per-account create gate input).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn character_count(&self, account_id: u32) -> Result<u32, DbError> {
        self.actor.call(move |conn| {
            let row: Option<mysql::Row> = conn.exec_first(
                "SELECT COUNT(`guid`) FROM `characters` WHERE `account` = ?",
                (account_id,),
            )?;
            Ok(row.map_or(0, |row| crate::row::Fields::from_row(row).u32(0)))
        })
    }

    /// The `PlayerCacheData` identity subset for delete;
    /// `Ok(None)` = no such character.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn character_owner(&self, guid_low: u32) -> Result<Option<CharacterOwner>, DbError> {
        self.actor.call(move |conn| {
            let row: Option<mysql::Row> = conn.exec_first(
                "SELECT `account`, `name`, `level` FROM `characters` WHERE `guid` = ?",
                (guid_low,),
            )?;
            Ok(row.map(|row| {
                let fields = crate::row::Fields::from_row(row);
                CharacterOwner {
                    account_id: fields.u32(0),
                    name: fields.string(1),
                    level: fields.u32(2),
                }
            }))
        })
    }

    /// `HandleCharRenameOpcode` gate + callback
    /// (`CharacterHandler.cpp:839,863`): the character
    /// must belong to the account, carry
    /// `CHARACTER_FLAG_RENAME` (0x4000), and no
    /// character may hold the new name; the update
    /// renames, clears the rename flag, and sets
    /// `CHARACTER_FLAG_RENAME_NEEDS_GM_REVIEW`
    /// (0x8000). `Ok(None)` = gate failed (fork replies
    /// `CHAR_CREATE_ERROR` for every gate miss).
    /// Returns the old name on success.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn rename_character(
        &self,
        guid_low: u32,
        account_id: u32,
        new_name: &str,
    ) -> Result<Option<String>, DbError> {
        const RENAME: u32 = 0x0000_4000;
        const NEEDS_GM_REVIEW: u32 = 0x0000_8000;
        let new_name = new_name.to_owned();
        self.actor.call(move |conn| {
            let row: Option<mysql::Row> = conn.exec_first(
                "SELECT `name` FROM `characters` WHERE `guid` = ? AND `account` = ? \
                 AND (`character_flags` & ?) = ? AND NOT EXISTS \
                 (SELECT NULL FROM `characters` WHERE `name` = ?)",
                (guid_low, account_id, RENAME, RENAME, new_name.as_str()),
            )?;
            let Some(row) = row else {
                return Ok(None);
            };
            let old_name = crate::row::Fields::from_row(row).string(0);
            conn.exec_drop(
                "UPDATE `characters` SET `name` = ?, \
                 `character_flags` = ((`character_flags` & ~?) | ?) WHERE `guid` = ?",
                (new_name.as_str(), RENAME, NEEDS_GM_REVIEW, guid_low),
            )?;
            Ok(Some(old_name))
        })
    }

    /// `Player::SaveNewPlayer` + `MasterPlayer::Create`/
    /// `SaveActions` (`Player.cpp:16051`,
    /// `MasterPlayer.cpp`): one transaction with the
    /// `characters` REPLACE and the starter
    /// `character_action` inserts. Returns the allocated
    /// low guid (`GeneratePlayerLowGuid`; the fork seeds
    /// its generator past `MAX(guid)` at boot and this
    /// actor is the only writer, so `MAX+1` inside the
    /// transaction is the same allocation).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn create_character(&self, new: &NewCharacter) -> Result<u32, DbError> {
        let new = new.clone();
        self.actor.call(move |conn| {
            conn.query_drop("START TRANSACTION")
                .map_err(DbError::Driver)?;
            let result = create_character_txn(conn, &new);
            match result {
                Ok(guid) => {
                    conn.query_drop("COMMIT").map_err(DbError::Driver)?;
                    Ok(guid)
                }
                Err(e) => {
                    let _ = conn.query_drop("ROLLBACK");
                    Err(e)
                }
            }
        })
    }

    /// `Player::DeleteFromDB` DB half (`Player.cpp:4361`)
    /// for an already-resolved `method`: method 1 is the
    /// soft unlink UPDATE; method 0 returns COD mail to
    /// senders, deletes pets (`Pet::DeleteFromDB`), then
    /// runs the fork's cascade in one transaction. Live
    /// side effects (guild/group/petition/corpse/cache)
    /// are the world actor's, before this call.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn delete_character(&self, guid_low: u32, plan: &DeletePlan) -> Result<(), DbError> {
        let plan = *plan;
        self.actor.call(move |conn| {
            if plan.method == 1 {
                conn.exec_drop(
                    "UPDATE `characters` SET `deleted_name` = `name`, \
                     `deleted_account` = `account`, `deleted_time` = ?, \
                     `name` = '', `account` = 0 WHERE `guid` = ?",
                    (plan.now, guid_low),
                )
                .map_err(DbError::Driver)?;
                return Ok(());
            }
            return_cod_mails(conn, guid_low, &plan)?;
            let pet_ids: Vec<u32> = conn.exec_map(
                "SELECT `id` FROM `character_pet` WHERE `owner_guid` = ?",
                (guid_low,),
                |row| crate::row::Fields::from_row(row).u32(0),
            )?;
            conn.query_drop("START TRANSACTION")
                .map_err(DbError::Driver)?;
            let result = delete_cascade_txn(conn, guid_low, &pet_ids);
            match result {
                Ok(()) => conn.query_drop("COMMIT").map_err(DbError::Driver),
                Err(e) => {
                    let _ = conn.query_drop("ROLLBACK");
                    Err(e)
                }
            }
        })
    }

    /// `Player::DeleteOldCharacters` sweep candidates
    /// (`Player.cpp:4563`): soft-deleted rows past
    /// `CharDelete.KeepDays`, at most two per call like
    /// the fork. The caller runs the full method-0
    /// delete (with live side effects) per row.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    pub fn old_deleted_characters(
        &self,
        keep_days: u32,
        now: u64,
    ) -> Result<Vec<(u32, u32)>, DbError> {
        self.actor.call(move |conn| {
            if keep_days == 0 {
                return Ok(Vec::new());
            }
            let cutoff = now.saturating_sub(u64::from(keep_days) * 86_400);
            let rows: Vec<(u32, u32)> = conn.exec_map(
                "SELECT `guid`, `deleted_account` FROM `characters` \
                 WHERE `deleted_account` IS NOT NULL AND `deleted_time` IS NOT NULL \
                 AND `deleted_time` < ? LIMIT 0, 2",
                (cutoff,),
                |row| {
                    let fields = crate::row::Fields::from_row(row);
                    (fields.u32(0), fields.u32(1))
                },
            )?;
            Ok(rows)
        })
    }
}

/// The `SaveNewPlayer` transaction body; see
/// [`CharacterDb::create_character`].
fn create_character_txn(conn: &mut mysql::Conn, new: &NewCharacter) -> Result<u32, DbError> {
    let max_rows: Vec<mysql::Row> =
        conn.exec("SELECT IFNULL(MAX(`guid`), 0) FROM `characters`", ())?;
    let guid = max_rows
        .into_iter()
        .next()
        .map_or(0, |row| crate::row::Fields::from_row(row).u32(0))
        .saturating_add(1)
        .max(1);
    let explored = "0 ".repeat(64);
    conn.exec_drop(
        "REPLACE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, \
         `level`, `money`, `skin`, `face`, `hair_style`, `hair_color`, `facial_hair`, \
         `map`, `position_x`, `position_y`, `position_z`, `orientation`, \
         `known_taxi_mask`, `current_taxi_path`, `online`, `extra_flags`, \
         `health`, `power1`, `power2`, `power3`, `power4`, `power5`, \
         `explored_zones`, `equipment_cache`, `ammo_id`, `world_phase_mask`, \
         `create_time`, `logout_time`) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '', 0, ?, \
         ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
        mysql::Params::Positional(vec![
            guid.into(),
            new.account_id.into(),
            new.name.as_str().into(),
            new.race.into(),
            new.class.into(),
            new.gender.into(),
            new.level.into(),
            new.money.into(),
            new.skin.into(),
            new.face.into(),
            new.hair_style.into(),
            new.hair_color.into(),
            new.facial_hair.into(),
            new.map_id.into(),
            new.position_x.into(),
            new.position_y.into(),
            new.position_z.into(),
            new.orientation.into(),
            new.taxi_mask.as_str().into(),
            new.extra_flags.into(),
            new.health.into(),
            new.powers[0].into(),
            new.powers[1].into(),
            new.powers[2].into(),
            new.powers[3].into(),
            new.powers[4].into(),
            explored.as_str().into(),
            new.equipment_cache.as_str().into(),
            new.ammo_id.into(),
            new.now.into(),
            new.now.into(),
        ]),
    )
    .map_err(DbError::Driver)?;
    for &(button, action, action_type) in &new.actions {
        conn.exec_drop(
            "INSERT INTO `character_action` (`guid`, `button`, `action`, `type`) \
             VALUES (?, ?, ?, ?)",
            (guid, button, action, action_type),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(guid)
}

/// One COD mail row pending return at delete time.
struct CodMail {
    id: u32,
    message_type: u16,
    template_id: u16,
    sender: u32,
    subject: String,
    item_text_id: u32,
    money: u32,
}

/// The method-0 COD return pass (`Player.cpp:4409-4485`,
/// `MailDraft::SendReturnToSender`): each COD mail with
/// items is re-mailed to its sender with items re-owned,
/// `MAIL_CHECK_MASK_RETURNED`, a 30-day expiry, and the
/// cross-account delivery delay. Non-player mail and
/// mail from a missing sender lose their items, as in
/// the fork. Item-prototype/`LoadFromDB` validity drops
/// are not reproduced (this layer has no item store;
/// mailed items were validated at send).
fn return_cod_mails(
    conn: &mut mysql::Conn,
    guid_low: u32,
    plan: &DeletePlan,
) -> Result<(), DbError> {
    let mails: Vec<CodMail> = conn.exec_map(
        "SELECT `id`, `message_type`, `mail_template_id`, `sender_guid`, `subject`, \
         `item_text_id`, `money` FROM `mail` \
         WHERE `receiver_guid` = ? AND `has_items` <> 0 AND `cod` <> 0",
        (guid_low,),
        |row| {
            let f = crate::row::Fields::from_row(row);
            CodMail {
                id: f.u32(0),
                message_type: f.u16(1),
                template_id: f.u16(2),
                sender: f.u32(3),
                subject: f.string(4),
                item_text_id: f.u32(5),
                money: f.u32(6),
            }
        },
    )?;
    for mail in mails {
        conn.exec_drop("DELETE FROM `mail` WHERE `id` = ?", (mail.id,))
            .map_err(DbError::Driver)?;
        // MAIL_NORMAL = 0; anything else cannot be returned.
        if mail.message_type != 0 {
            conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail.id,))
                .map_err(DbError::Driver)?;
            continue;
        }
        let sender_account: Option<mysql::Row> = conn.exec_first(
            "SELECT `account` FROM `characters` WHERE `guid` = ?",
            (mail.sender,),
        )?;
        let Some(sender_account) = sender_account else {
            // Sender no longer exists: items are deleted
            // (`deleteIncludedItems(true)`).
            conn.exec_drop(
                "DELETE `item_instance` FROM `item_instance` \
                 JOIN `mail_items` ON `item_guid` = `guid` WHERE `mail_id` = ?",
                (mail.id,),
            )
            .map_err(DbError::Driver)?;
            conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail.id,))
                .map_err(DbError::Driver)?;
            continue;
        };
        let sender_account = crate::row::Fields::from_row(sender_account).u32(0);
        let items: Vec<(u32, u32)> = conn.exec_map(
            "SELECT `item_guid`, `item_id` FROM `mail_items` WHERE `mail_id` = ?",
            (mail.id,),
            |row| {
                let f = crate::row::Fields::from_row(row);
                (f.u32(0), f.u32(1))
            },
        )?;
        for &(item_guid, _) in &items {
            conn.exec_drop(
                "UPDATE `item_instance` SET `owner_guid` = ? WHERE `guid` = ?",
                (mail.sender, item_guid),
            )
            .map_err(DbError::Driver)?;
        }
        conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail.id,))
            .map_err(DbError::Driver)?;
        let delay = if !items.is_empty() && sender_account != plan.account_id {
            u64::from(plan.mail_delivery_delay_secs)
        } else {
            0
        };
        let deliver_time = plan.now + delay;
        let expire_time = deliver_time + 30 * 86_400;
        let new_id = crate::mail::insert_mail(
            conn,
            &crate::mail::NewMail {
                message_type: 0,
                stationery: 41,
                mail_template_id: mail.template_id,
                sender_low: guid_low,
                receiver_low: mail.sender,
                subject: mail.subject.clone(),
                item_text_id: mail.item_text_id,
                expire_time: i64::try_from(expire_time).unwrap_or(i64::MAX),
                deliver_time: i64::try_from(deliver_time).unwrap_or(i64::MAX),
                money: mail.money,
                cod: 0,
                checked: 0x02, // MAIL_CHECK_MASK_RETURNED
                item: items.first().copied(),
            },
        )?;
        // `insert_mail` links one item; link any rest the
        // same way (`SendMailTo` item loop).
        for &(item_guid, item_id) in items.iter().skip(1) {
            conn.exec_drop(
                "INSERT INTO `mail_items` (`mail_id`, `item_guid`, `item_id`, `receiver_guid`) \
                 VALUES (?, ?, ?, ?)",
                (new_id, item_guid, item_id, mail.sender),
            )
            .map_err(DbError::Driver)?;
        }
    }
    Ok(())
}

/// The method-0 cascade (`Player.cpp:4505-4527`, pets
/// via `Pet::DeleteFromDB`), statements in fork order.
fn delete_cascade_txn(
    conn: &mut mysql::Conn,
    guid_low: u32,
    pet_ids: &[u32],
) -> Result<(), DbError> {
    for &pet_id in pet_ids {
        for sql in [
            "DELETE FROM `character_pet` WHERE `id` = ?",
            "DELETE FROM `pet_aura` WHERE `guid` = ?",
            "DELETE FROM `pet_spell` WHERE `guid` = ?",
            "DELETE FROM `pet_spell_cooldown` WHERE `guid` = ?",
        ] {
            conn.exec_drop(sql, (pet_id,)).map_err(DbError::Driver)?;
        }
    }
    for sql in [
        "DELETE FROM `characters` WHERE `guid` = ?",
        "DELETE FROM `character_action` WHERE `guid` = ?",
        "DELETE FROM `character_aura` WHERE `guid` = ?",
        "DELETE FROM `character_battleground_data` WHERE `guid` = ?",
        "DELETE FROM `character_deleted_items` WHERE `player_guid` = ?",
        "DELETE FROM `character_gifts` WHERE `guid` = ?",
        "DELETE FROM `character_homebind` WHERE `guid` = ?",
        "DELETE FROM `character_instance` WHERE `guid` = ?",
        "DELETE FROM `group_instance` WHERE `leader_guid` = ?",
        "DELETE FROM `character_inventory` WHERE `guid` = ?",
        "DELETE FROM `character_queststatus` WHERE `guid` = ?",
        "DELETE FROM `character_reputation` WHERE `guid` = ?",
        "DELETE FROM `character_skills` WHERE `guid` = ?",
        "DELETE FROM `character_forgotten_skills` WHERE `guid` = ?",
        "DELETE FROM `character_spell` WHERE `guid` = ?",
        "DELETE FROM `character_spell_cooldown` WHERE `guid` = ?",
        "DELETE FROM `item_instance` WHERE `owner_guid` = ?",
        "DELETE FROM `mail` WHERE `receiver_guid` = ?",
        "DELETE FROM `mail_items` WHERE `receiver_guid` = ?",
        "DELETE FROM `character_pet` WHERE `owner_guid` = ?",
    ] {
        conn.exec_drop(sql, (guid_low,)).map_err(DbError::Driver)?;
    }
    conn.exec_drop(
        "DELETE FROM `character_social` WHERE `guid` = ? OR `friend` = ?",
        (guid_low, guid_low),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `guild_eventlog` WHERE `player_guid1` = ? OR `player_guid2` = ?",
        (guid_low, guid_low),
    )
    .map_err(DbError::Driver)
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
