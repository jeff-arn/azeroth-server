//! Quest reward item rows (Tier 1 Slice 1b).
//!
//! The runtime has no live inventory-grant path yet
//! (Slice 3 builds it); quest rewards follow the
//! petition/mail precedent of writing `item_instance`
//! and `character_inventory` rows directly. The world
//! actor mirrors every write into the player's
//! in-memory inventory baseline so counts stay
//! consistent in-session, and the character save
//! treats those rows as Unchanged (no statements),
//! so the direct writes survive logout exactly as
//! petition charter items do.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::petition::next_item_guid;
use crate::row::Fields;

/// Fresh-item enchantments text: 7 slots x 3 zero
/// fields (`Item::SaveToDB` shape for a plain item).
fn plain_enchantments() -> String {
    let mut text = String::from("0");
    for _ in 0..20 {
        text.push_str(" 0");
    }
    text
}

/// `StoreNewItem` DB half: create one item stack at
/// a caller-chosen inventory position (`bag` is the
/// container item guid, 0 for the backpack). Slot
/// selection lives in the world actor over its
/// in-memory inventory baseline; the fork's
/// `CanRewardQuest` space check is not ported yet,
/// so when every bag is full the caller drops the
/// reward exactly as `RewardQuest` does when
/// `CanStoreNewItem` fails there.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_reward_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_id: u32,
    count: u32,
    bag: u32,
    slot: u32,
    creator_low: u32,
) -> Result<u32, DbError> {
    let item_low = next_item_guid(conn)?;
    conn.exec_drop(
        "INSERT INTO `item_instance` (`guid`, `item_id`, `owner_guid`, `creator_guid`, \
         `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, \
         `random_property_id`, `durability`, `text`, `generated_loot`) \
         VALUES (?, ?, ?, ?, 0, ?, 0, '0 0 0 0 0 ', 0, ?, 0, 0, 0, 0)",
        (
            item_low,
            item_id,
            owner_low,
            creator_low,
            count,
            plain_enchantments(),
        ),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) \
         VALUES (?, ?, ?, ?, ?)",
        (owner_low, bag, slot, item_low, item_id),
    )
    .map_err(DbError::Driver)?;
    Ok(item_low)
}

/// `DestroyItemCount` DB half for one stack: set the
/// remaining count, or delete both rows when it
/// reaches zero. The caller walks stacks.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_item_count(conn: &mut mysql::Conn, item_guid: u32, count: u32) -> Result<(), DbError> {
    if count == 0 {
        conn.exec_drop(
            "DELETE FROM `character_inventory` WHERE `item_guid` = ?",
            (item_guid,),
        )
        .map_err(DbError::Driver)?;
        conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_guid,))
            .map_err(DbError::Driver)?;
    } else {
        conn.exec_drop(
            "UPDATE `item_instance` SET `count` = ? WHERE `guid` = ?",
            (count, item_guid),
        )
        .map_err(DbError::Driver)?;
    }
    Ok(())
}

/// `TakeCastItem` charges write-back for one stack.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_item_charges(
    conn: &mut mysql::Conn,
    item_guid: u32,
    charges: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `item_instance` SET `charges` = ? WHERE `guid` = ?",
        (charges, item_guid),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// Item dynamic flags write (bind-on-use stamp).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_item_flags(conn: &mut mysql::Conn, item_guid: u32, flags: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `item_instance` SET `flags` = ? WHERE `guid` = ?",
        (flags, item_guid),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}

/// Total count of an item across inventory rows
/// (`GetItemCount(item, checkInBank=true)` DB half;
/// the in-memory baseline is authoritative in
/// session, this exists for completeness/tests).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn item_count(conn: &mut mysql::Conn, owner_low: u32, item_id: u32) -> Result<u32, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT IFNULL(SUM(`ii`.`count`), 0) FROM `character_inventory` `ci` \
         JOIN `item_instance` `ii` ON `ii`.`guid` = `ci`.`item_guid` \
         WHERE `ci`.`guid` = ? AND `ci`.`item_id` = ?",
        (owner_low, item_id),
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0)))
}
