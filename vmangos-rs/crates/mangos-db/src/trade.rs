//! Trade persistence (`TradeHandler.cpp` item moves):
//! position-keyed inventory reads, backpack capacity,
//! and the cross-player item-row move the accept path
//! performs. Like mail, item exchanges are DB-backed
//! until the runtime Item system lands (Phase 6); the
//! on-disk result matches C++ because a completed
//! trade saves both inventories immediately
//! (`SaveInventoryAndGoldToDB`).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::mail::StoredInventoryItem;
use crate::row::Fields;

/// Backpack slot range (`INVENTORY_SLOT_ITEM_START`
/// 23, 16 slots).
const BACKPACK_SLOT_START: u32 = 23;
const BACKPACK_SLOT_END: u32 = 39;

/// The item at one inventory position (`GetItemByPos`
/// over the stored rows), with the bag-emptiness count
/// `CanBeTraded` needs. `bag`/`slot` are client
/// positions: wire bag 255 is the backpack/equipment
/// row set (stored bag 0); any other wire bag is the
/// equipment-array slot of the containing bag item
/// (stored bag is that item's guid).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn item_at_pos(
    conn: &mut mysql::Conn,
    owner_low: u32,
    bag: u32,
    slot: u32,
) -> Result<Option<StoredInventoryItem>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `ci`.`bag`, `ci`.`slot`, `ci`.`item_guid`, `ii`.`item_id`, `ii`.`count`, \
         `ii`.`duration`, `ii`.`charges`, `ii`.`flags`, `ii`.`enchantments`, \
         `ii`.`random_property_id`, `ii`.`durability`, `ii`.`text`, `ii`.`creator_guid`, \
         `ii`.`gift_creator_guid`, `ii`.`generated_loot`, \
         (SELECT COUNT(*) FROM `character_inventory` WHERE `bag` = `ci`.`item_guid`) \
         FROM `character_inventory` AS `ci` \
         JOIN `item_instance` AS `ii` ON `ii`.`guid` = `ci`.`item_guid` \
         WHERE `ci`.`guid` = ? AND `ci`.`slot` = ? AND `ci`.`bag` = \
         CASE WHEN ? = 255 THEN 0 ELSE \
         (SELECT `item_guid` FROM `character_inventory` \
         WHERE `guid` = ? AND `bag` = 0 AND `slot` = ?) END LIMIT 1",
        (owner_low, slot, bag, owner_low, bag),
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

/// Free backpack slots (the `CanStoreItems` capacity
/// approximation the DB-backed trade uses; the full
/// bag/stack placement lands with the item system).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn free_backpack_slots(conn: &mut mysql::Conn, owner_low: u32) -> Result<u32, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT COUNT(*) FROM `character_inventory` WHERE `guid` = ? AND `bag` = 0 \
         AND `slot` >= ? AND `slot` < ?",
        (owner_low, BACKPACK_SLOT_START, BACKPACK_SLOT_END),
    )?;
    let used = rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0));
    Ok((BACKPACK_SLOT_END - BACKPACK_SLOT_START).saturating_sub(used))
}

/// Move one item between two players' inventories
/// (`MoveItemFromInventory` + `MoveItemToInventory`):
/// drop the giver's inventory row, rewrite the owner
/// (and the gift creator for wrapped items), insert
/// the receiver's row at a concrete backpack slot.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn move_item(
    conn: &mut mysql::Conn,
    from_low: u32,
    to_low: u32,
    item_low: u32,
    item_id: u32,
    to_slot: u32,
    gift_creator_low: Option<u32>,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_inventory` WHERE `guid` = ? AND `item_guid` = ?",
        (from_low, item_low),
    )
    .map_err(DbError::Driver)?;
    if let Some(gift_creator_low) = gift_creator_low {
        conn.exec_drop(
            "UPDATE `item_instance` SET `owner_guid` = ?, `gift_creator_guid` = ? \
             WHERE `guid` = ?",
            (to_low, gift_creator_low, item_low),
        )
        .map_err(DbError::Driver)?;
    } else {
        conn.exec_drop(
            "UPDATE `item_instance` SET `owner_guid` = ? WHERE `guid` = ?",
            (to_low, item_low),
        )
        .map_err(DbError::Driver)?;
    }
    conn.exec_drop(
        "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) \
         VALUES (?, 0, ?, ?, ?)",
        (to_low, to_slot, item_low, item_id),
    )
    .map_err(DbError::Driver)?;
    Ok(())
}
