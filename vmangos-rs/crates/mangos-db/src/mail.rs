//! Mail persistence (`MailDraft::SendMailTo`,
//! `MasterPlayer::SaveMails`, `ObjectMgr::
//! ReturnOrDeleteOldMails`): the `mail`, `mail_items`,
//! and `item_text` statements, the item-row moves
//! between `character_inventory` and mail, and the
//! boot sweep for expired mails. Item moves are
//! DB-backed like the petition charter until the
//! runtime Item system lands (Phase 6); the on-disk
//! result matches C++ because every mail action
//! saves inventory/gold and mails immediately.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::petition::{first_free_backpack_slot, next_item_guid};
use crate::row::Fields;

/// `MAIL_BODY_ITEM_TEMPLATE` (`Mail.h:46`): the plain
/// letter created by "make a permanent copy".
pub const MAIL_BODY_ITEM_ENTRY: u32 = 8383;

/// One mail to insert (`MailDraft::SendMailTo` fields).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewMail {
    pub message_type: u8,
    pub stationery: u8,
    pub mail_template_id: u16,
    pub sender_low: u32,
    pub receiver_low: u32,
    pub subject: String,
    pub item_text_id: u32,
    pub expire_time: i64,
    pub deliver_time: i64,
    pub money: u32,
    pub cod: u32,
    pub checked: u32,
    /// `(item_guid, item_id)` of the single attachment
    /// (`MAX_MAIL_ITEMS` is 1 at 5875).
    pub item: Option<(u32, u32)>,
}

/// The changed-mail field set `SaveMails` writes back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailUpdate {
    pub mail_id: u32,
    pub item_text_id: u32,
    pub has_items: bool,
    pub expire_time: i64,
    pub deliver_time: i64,
    pub money: u32,
    pub cod: u32,
    pub checked: u32,
}

/// A receiver resolved by name (`GetPlayerGuidByName` +
/// team/account reads).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailReceiverInfo {
    pub guid_low: u32,
    pub race: u8,
    pub account_id: u32,
}

/// One `character_inventory` + `item_instance` row for
/// the send-item checks and the mail list item block.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredInventoryItem {
    pub bag: u32,
    pub slot: u32,
    pub item_guid: u32,
    pub item_id: u32,
    pub count: u32,
    pub duration: i32,
    pub charges: String,
    pub flags: u32,
    pub enchantments: String,
    pub random_property_id: i16,
    pub durability: u16,
    pub text: u32,
    pub creator_guid: u32,
    pub gift_creator_guid: u32,
    pub generated_loot: u8,
    /// Inventory rows stored inside this item when it
    /// is a bag (the `CanBeTraded` non-empty check).
    pub items_in_bag: u32,
}

/// `MailDraft::SendMailTo` insert; allocates the mail
/// id (`ObjectMgr::GenerateMailID` is seeded past
/// `MAX(id)`; the world actor serializes sends) and
/// returns it.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_mail(conn: &mut mysql::Conn, mail: &NewMail) -> Result<u32, DbError> {
    let max_rows: Vec<mysql::Row> = conn.exec("SELECT IFNULL(MAX(`id`), 0) FROM `mail`", ())?;
    let mail_id = max_rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0))
        .saturating_add(1)
        .max(1);
    let has_items = u8::from(mail.item.is_some());
    conn.exec_drop(
        "INSERT INTO `mail` (`id`, `message_type`, `stationery`, `mail_template_id`, \
         `sender_guid`, `receiver_guid`, `subject`, `item_text_id`, `has_items`, \
         `expire_time`, `deliver_time`, `money`, `cod`, `checked`) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        mysql::Params::Positional(vec![
            mail_id.into(),
            mail.message_type.into(),
            mail.stationery.into(),
            mail.mail_template_id.into(),
            mail.sender_low.into(),
            mail.receiver_low.into(),
            mail.subject.as_str().into(),
            mail.item_text_id.into(),
            has_items.into(),
            mail.expire_time.into(),
            mail.deliver_time.into(),
            mail.money.into(),
            mail.cod.into(),
            mail.checked.into(),
        ]),
    )
    .map_err(DbError::Driver)?;
    if let Some((item_guid, item_id)) = mail.item {
        conn.exec_drop(
            "INSERT INTO `mail_items` (`mail_id`, `item_guid`, `item_id`, `receiver_guid`) \
             VALUES (?, ?, ?, ?)",
            (mail_id, item_guid, item_id, mail.receiver_low),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(mail_id)
}

/// The `SaveMails` changed-mail UPDATE.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_mail(conn: &mut mysql::Conn, update: &MailUpdate) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `mail` SET `item_text_id` = ?, `has_items` = ?, `expire_time` = ?, \
         `deliver_time` = ?, `money` = ?, `cod` = ?, `checked` = ? WHERE `id` = ?",
        (
            update.item_text_id,
            u8::from(update.has_items),
            update.expire_time,
            update.deliver_time,
            update.money,
            update.cod,
            update.checked,
            update.mail_id,
        ),
    )
    .map_err(DbError::Driver)
}

/// The `SaveMails` deleted-mail block: attached item
/// instances, the body text when stationery is not
/// the default, then the mail and item rows.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_mail(
    conn: &mut mysql::Conn,
    mail_id: u32,
    item_lows: &[u32],
    item_text_id: u32,
    stationery: u8,
) -> Result<(), DbError> {
    for item_low in item_lows {
        conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_low,))
            .map_err(DbError::Driver)?;
    }
    // MAIL_STATIONERY_DEFAULT = 41.
    if item_text_id != 0 && stationery != 41 {
        conn.exec_drop("DELETE FROM `item_text` WHERE `id` = ?", (item_text_id,))
            .map_err(DbError::Driver)?;
    }
    conn.exec_drop("DELETE FROM `mail` WHERE `id` = ?", (mail_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail_id,))
        .map_err(DbError::Driver)
}

/// Delete one `mail_items` row (`SaveMails` removed
/// items after take-item).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_mail_item_row(conn: &mut mysql::Conn, item_guid: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `mail_items` WHERE `item_guid` = ?",
        (item_guid,),
    )
    .map_err(DbError::Driver)
}

/// Delete the mail + `mail_items` rows only (the
/// return-to-sender pre-delete; items move to the
/// return mail instead of being destroyed).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_mail_rows(conn: &mut mysql::Conn, mail_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `mail` WHERE `id` = ?", (mail_id,))
        .map_err(DbError::Driver)?;
    conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail_id,))
        .map_err(DbError::Driver)
}

/// `ObjectMgr::CreateItemText`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_item_text(conn: &mut mysql::Conn, text: &str) -> Result<u32, DbError> {
    let max_rows: Vec<mysql::Row> =
        conn.exec("SELECT IFNULL(MAX(`id`), 0) FROM `item_text`", ())?;
    let id = max_rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0))
        .saturating_add(1)
        .max(1);
    conn.exec_drop(
        "INSERT INTO `item_text` (`id`, `text`) VALUES (?, ?)",
        (id, text),
    )
    .map_err(DbError::Driver)?;
    Ok(id)
}

/// `ObjectMgr::GetItemText` (empty when unknown).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn item_text(conn: &mut mysql::Conn, item_text_id: u32) -> Result<String, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `text` FROM `item_text` WHERE `id` = ?",
        (item_text_id,),
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map_or_else(String::new, |row| Fields::from_row(row).string(0)))
}

/// Receiver lookup by (normalized) character name.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn receiver_by_name(
    conn: &mut mysql::Conn,
    name: &str,
) -> Result<Option<MailReceiverInfo>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `race`, `account` FROM `characters` WHERE `name` = ? LIMIT 1",
        (name,),
    )?;
    Ok(rows.into_iter().next().map(|row| {
        let f = Fields::from_row(row);
        MailReceiverInfo {
            guid_low: f.u32(0),
            race: f.u8(1),
            account_id: f.u32(2),
        }
    }))
}

/// Mailbox size for the send cap (`SELECT COUNT(*)`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn mail_count(conn: &mut mysql::Conn, receiver_low: u32) -> Result<u32, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT COUNT(*) FROM `mail` WHERE `receiver_guid` = ?",
        (receiver_low,),
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0)))
}

/// `ObjectMgr::GetPlayerAccountIdByGUID` from the
/// characters row (0 in C++ when missing; `None` here).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn account_id_by_guid(conn: &mut mysql::Conn, guid_low: u32) -> Result<Option<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `account` FROM `characters` WHERE `guid` = ?",
        (guid_low,),
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| Fields::from_row(row).u32(0))
        .filter(|account| *account != 0))
}

/// One carried item by guid (`Player::GetItemByGuid`
/// over inventory rows).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn inventory_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
) -> Result<Option<StoredInventoryItem>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `ci`.`bag`, `ci`.`slot`, `ci`.`item_guid`, `ii`.`item_id`, `ii`.`count`, \
         `ii`.`duration`, `ii`.`charges`, `ii`.`flags`, `ii`.`enchantments`, \
         `ii`.`random_property_id`, `ii`.`durability`, `ii`.`text`, `ii`.`creator_guid`, \
         `ii`.`gift_creator_guid`, `ii`.`generated_loot`, \
         (SELECT COUNT(*) FROM `character_inventory` WHERE `bag` = `ci`.`item_guid`) \
         FROM `character_inventory` AS `ci` \
         JOIN `item_instance` AS `ii` ON `ii`.`guid` = `ci`.`item_guid` \
         WHERE `ci`.`guid` = ? AND `ci`.`item_guid` = ? LIMIT 1",
        (owner_low, item_low),
    )?;
    Ok(rows.into_iter().next().map(|row| {
        let f = Fields::from_row(row);
        StoredInventoryItem {
            bag: f.u32(0),
            slot: f.u32(1),
            item_guid: f.u32(2),
            item_id: f.u32(3),
            count: f.u32(4),
            duration: f.i32(5),
            charges: f.string(6),
            flags: f.u32(7),
            enchantments: f.string(8),
            random_property_id: f.i16(9),
            durability: f.u16(10),
            text: f.u32(11),
            creator_guid: f.u32(12),
            gift_creator_guid: f.u32(13),
            generated_loot: f.u8(14),
            items_in_bag: f.u32(15),
        }
    }))
}

/// Move a carried item into the mail system
/// (`MoveItemFromInventory` + owner rewrite): drop
/// the inventory row and re-own the instance.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn move_item_to_mail(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
    new_owner_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_inventory` WHERE `guid` = ? AND `item_guid` = ?",
        (owner_low, item_low),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "UPDATE `item_instance` SET `owner_guid` = ? WHERE `guid` = ?",
        (new_owner_low, item_low),
    )
    .map_err(DbError::Driver)
}

/// Re-own a mailed item without touching inventory
/// (return-to-sender item owner rewrite).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_item_owner(
    conn: &mut mysql::Conn,
    item_low: u32,
    new_owner_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `item_instance` SET `owner_guid` = ? WHERE `guid` = ?",
        (new_owner_low, item_low),
    )
    .map_err(DbError::Driver)
}

/// Destroy one item instance (dropped return items).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_item_instance(conn: &mut mysql::Conn, item_low: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_low,))
        .map_err(DbError::Driver)
}

/// Store an existing item instance into the first
/// free backpack slot (take-item). `false` when the
/// backpack is full.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn store_mailed_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
    item_id: u32,
) -> Result<bool, DbError> {
    let Some(slot) = first_free_backpack_slot(conn, owner_low)? else {
        return Ok(false);
    };
    conn.exec_drop(
        "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) \
         VALUES (?, 0, ?, ?, ?)",
        (owner_low, slot, item_low, item_id),
    )
    .map_err(DbError::Driver)?;
    Ok(true)
}

/// Create the permanent letter item (8383) carrying a
/// mail body. `false` when the backpack is full.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_text_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    creator_low: u32,
    item_text_id: u32,
) -> Result<bool, DbError> {
    let Some(slot) = first_free_backpack_slot(conn, owner_low)? else {
        return Ok(false);
    };
    let item_low = next_item_guid(conn)?;
    conn.exec_drop(
        "INSERT INTO `item_instance` (`guid`, `item_id`, `owner_guid`, `creator_guid`, \
         `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, \
         `random_property_id`, `durability`, `text`, `generated_loot`) \
         VALUES (?, ?, ?, ?, 0, 1, 0, '0 0 0 0 0 ', 0, \
         '0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0', 0, 0, ?, 0)",
        (
            item_low,
            MAIL_BODY_ITEM_ENTRY,
            owner_low,
            creator_low,
            item_text_id,
        ),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) \
         VALUES (?, 0, ?, ?, ?)",
        (owner_low, slot, item_low, MAIL_BODY_ITEM_ENTRY),
    )
    .map_err(DbError::Driver)?;
    Ok(true)
}

/// `ObjectMgr::ReturnOrDeleteOldMails(false)` at
/// boot: expired mails are deleted, or flipped back
/// to their sender when they carry a returnable item
/// from a player. The fork's sweep never deletes
/// `item_text` rows (its `itemTextId` read is
/// skipped), and neither does this port. Returns the
/// number of mails processed.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn return_or_delete_old_mails(conn: &mut mysql::Conn, now: i64) -> Result<u32, DbError> {
    conn.exec_drop(
        "DELETE FROM `mail` WHERE `expire_time` < ? AND `has_items` = 0 AND `item_text_id` = 0",
        (now,),
    )
    .map_err(DbError::Driver)?;
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `message_type`, `sender_guid`, `receiver_guid`, `has_items`, `checked` \
         FROM `mail` WHERE `expire_time` < ? ORDER BY `expire_time`",
        (now,),
    )?;
    // MAIL_NORMAL = 0; MAIL_CHECK_MASK_RETURNED = 0x02,
    // MAIL_CHECK_MASK_COD_PAYMENT = 0x08.
    let mut processed = 0u32;
    for row in rows {
        let f = Fields::from_row(row);
        let mail_id = f.u32(0);
        let message_type = f.u8(1);
        let sender_low = f.u32(2);
        let receiver_low = f.u32(3);
        let has_items = f.u8(4) != 0;
        let checked = f.u32(5);
        processed += 1;
        if !has_items {
            conn.exec_drop("DELETE FROM `mail` WHERE `id` = ?", (mail_id,))
                .map_err(DbError::Driver)?;
            continue;
        }
        let item_rows: Vec<mysql::Row> = conn.exec(
            "SELECT `item_guid` FROM `mail_items` WHERE `mail_id` = ?",
            (mail_id,),
        )?;
        let item_guid = item_rows
            .into_iter()
            .next()
            .map_or(0, |row| Fields::from_row(row).u32(0));
        let returnable = message_type == 0 && checked & (0x08 | 0x02) == 0;
        if returnable {
            conn.exec_drop(
                "UPDATE `mail` SET `sender_guid` = ?, `receiver_guid` = ?, \
                 `expire_time` = ?, `deliver_time` = ?, `cod` = 0, `checked` = 2 \
                 WHERE `id` = ?",
                (receiver_low, sender_low, now + 30 * 86_400, now, mail_id),
            )
            .map_err(DbError::Driver)?;
            if item_guid != 0 {
                conn.exec_drop(
                    "UPDATE `mail_items` SET `receiver_guid` = ? WHERE `item_guid` = ?",
                    (sender_low, item_guid),
                )
                .map_err(DbError::Driver)?;
                conn.exec_drop(
                    "UPDATE `item_instance` SET `owner_guid` = ? WHERE `guid` = ?",
                    (sender_low, item_guid),
                )
                .map_err(DbError::Driver)?;
            }
        } else {
            if item_guid != 0 {
                conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_guid,))
                    .map_err(DbError::Driver)?;
                conn.exec_drop("DELETE FROM `mail_items` WHERE `mail_id` = ?", (mail_id,))
                    .map_err(DbError::Driver)?;
            }
            conn.exec_drop("DELETE FROM `mail` WHERE `id` = ?", (mail_id,))
                .map_err(DbError::Driver)?;
        }
    }
    Ok(processed)
}
