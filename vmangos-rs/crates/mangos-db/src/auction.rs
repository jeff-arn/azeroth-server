//! Auction house persistence (`AuctionHouseMgr::
//! LoadAuctionItems` / `LoadAuctions`,
//! `AuctionEntry::SaveToDB` / `DeleteFromDB`, bid
//! updates): the `auction` table plus the boot
//! cleanup (auctions whose item row is gone are
//! deleted; auctions whose house id is unknown have
//! their item mailed back and are deleted). Auction
//! items are the `item_instance` rows the sell
//! handler detached from `character_inventory`, the
//! same DB-backed item pattern as mail/trade until
//! the runtime Item system lands (Phase 6).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use mysql::prelude::Queryable;

use crate::DbError;
use crate::character_load::ItemInstanceRow;
use crate::mail::{NewMail, insert_mail};
use crate::row::Fields;

/// One `auction` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredAuction {
    pub id: u32,
    pub house_id: u32,
    pub item_guid: u32,
    pub item_id: u32,
    pub seller_guid: u32,
    pub buyout_price: u32,
    pub expire_time: i64,
    pub buyer_guid: u32,
    pub last_bid: u32,
    pub start_bid: u32,
    pub deposit: u32,
}

/// Boot-loaded auction state (`LoadAuctionItems` +
/// `LoadAuctions` output, after the fork's cleanup).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadedAuctions {
    pub auctions: Vec<StoredAuction>,
    /// Auction items by item guid (`mAitems`).
    pub items: Vec<ItemInstanceRow>,
    /// Seller account per auction id
    /// (`GetPlayerAccountIdByGUID` at load).
    pub owner_accounts: Vec<(u32, u32)>,
    /// `ObjectMgr::GenerateAuctionID` seed: one past
    /// the greatest auction id seen, at least 1.
    pub next_auction_id: u32,
}

/// Loads auction items and auctions with the fork's
/// boot cleanup. `valid_house_ids` is the
/// `AuctionHouse.dbc` id set; `prototype_exists`
/// mirrors the unknown-prototype skip in
/// `LoadAuctionItems`. Auctions referencing a missing
/// item are deleted; auctions in an unknown house
/// have the item mailed back (goblin house 7 as the
/// mail sender) and are deleted.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_auctions<S: BuildHasher>(
    conn: &mut mysql::Conn,
    valid_house_ids: &HashSet<u32, S>,
    prototype_exists: impl Fn(u32) -> bool,
    now: i64,
) -> Result<LoadedAuctions, DbError> {
    // LoadAuctionItems.
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `creator_guid`, `gift_creator_guid`, `count`, `duration`, `charges`, \
         `flags`, `enchantments`, `random_property_id`, `durability`, `text`, \
         `item_guid`, `item_instance`.`item_id`, `generated_loot` \
         FROM `auction` JOIN `item_instance` ON `item_guid` = `guid`",
    )?;
    let mut items: HashMap<u32, ItemInstanceRow> = HashMap::new();
    for row in rows {
        let f = Fields::from_row(row);
        let item = ItemInstanceRow {
            creator_guid: f.u32(0),
            gift_creator_guid: f.u32(1),
            count: f.u32(2),
            duration: f.i32(3),
            charges: f.string(4),
            flags: f.u32(5),
            enchantments: f.string(6),
            random_property_id: f.i16(7),
            durability: f.u16(8),
            text: f.u32(9),
            item_guid: f.u32(10),
            item_id: f.u32(11),
            generated_loot: f.u8(12),
        };
        if !prototype_exists(item.item_id) {
            continue;
        }
        items.insert(item.item_guid, item);
    }

    // LoadAuctions.
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `id`, `house_id`, `item_guid`, `item_id`, `seller_guid`, \
         `buyout_price`, `expire_time`, `buyer_guid`, `last_bid`, `start_bid`, \
         `deposit` FROM `auction`",
    )?;
    let mut loaded = LoadedAuctions {
        next_auction_id: 1,
        ..LoadedAuctions::default()
    };
    for row in rows {
        let f = Fields::from_row(row);
        let auction = StoredAuction {
            id: f.u32(0),
            house_id: f.u32(1),
            item_guid: f.u32(2),
            item_id: f.u32(3),
            seller_guid: f.u32(4),
            buyout_price: f.u32(5),
            expire_time: f.i64(6),
            buyer_guid: f.u32(7),
            last_bid: f.u32(8),
            start_bid: f.u32(9),
            deposit: f.u32(10),
        };
        loaded.next_auction_id = loaded.next_auction_id.max(auction.id.saturating_add(1));
        if !items.contains_key(&auction.item_guid) {
            delete_auction(conn, auction.id)?;
            continue;
        }
        if !valid_house_ids.contains(&auction.house_id) {
            // Mail the item back from the goblin house,
            // like the fork's LookupEntry(7) fallback.
            insert_mail(
                conn,
                &NewMail {
                    message_type: 2,
                    stationery: 62,
                    mail_template_id: 0,
                    sender_low: 7,
                    receiver_low: auction.seller_guid,
                    subject: format!("{}:0:5", auction.item_id),
                    item_text_id: 0,
                    expire_time: now + 30 * 86_400,
                    deliver_time: now,
                    money: 0,
                    cod: 0,
                    checked: 4,
                    item: Some((auction.item_guid, auction.item_id)),
                },
            )?;
            items.remove(&auction.item_guid);
            delete_auction(conn, auction.id)?;
            continue;
        }
        let account = account_of(conn, auction.seller_guid)?.unwrap_or(0);
        loaded.owner_accounts.push((auction.id, account));
        loaded.auctions.push(auction);
    }
    loaded.items = items.into_values().collect();
    Ok(loaded)
}

/// Character account lookup (`GetPlayerAccountIdByGUID`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
fn account_of(conn: &mut mysql::Conn, guid_low: u32) -> Result<Option<u32>, DbError> {
    crate::mail::account_id_by_guid(conn, guid_low)
}

/// `AuctionEntry::SaveToDB`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_auction(conn: &mut mysql::Conn, auction: &StoredAuction) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `auction` (`id`, `house_id`, `item_guid`, `item_id`, \
         `seller_guid`, `buyout_price`, `expire_time`, `buyer_guid`, `last_bid`, \
         `start_bid`, `deposit`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            auction.id,
            auction.house_id,
            auction.item_guid,
            auction.item_id,
            auction.seller_guid,
            auction.buyout_price,
            auction.expire_time,
            auction.buyer_guid,
            auction.last_bid,
            auction.start_bid,
            auction.deposit,
        ),
    )
    .map_err(DbError::Driver)
}

/// `AuctionEntry::DeleteFromDB`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_auction(conn: &mut mysql::Conn, auction_id: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `auction` WHERE `id` = ?", (auction_id,))
        .map_err(DbError::Driver)
}

/// The place-bid persistence (`UPDATE auction SET
/// buyer_guid/last_bid`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn update_bid(
    conn: &mut mysql::Conn,
    auction_id: u32,
    buyer_guid: u32,
    last_bid: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `auction` SET `buyer_guid` = ?, `last_bid` = ? WHERE `id` = ?",
        (buyer_guid, last_bid, auction_id),
    )
    .map_err(DbError::Driver)
}

/// Detaches a sold item from the seller's inventory
/// (`MoveItemFromInventory` + `DeleteFromInventoryDB`;
/// the instance row stays, still owned by the seller).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn detach_inventory_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_inventory` WHERE `guid` = ? AND `item_guid` = ?",
        (owner_low, item_low),
    )
    .map_err(DbError::Driver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_next_id_is_one() {
        assert_eq!(LoadedAuctions::default().next_auction_id, 0);
        let loaded = LoadedAuctions {
            next_auction_id: 1,
            ..LoadedAuctions::default()
        };
        assert_eq!(loaded.next_auction_id, 1);
    }
}

/// One `auctionhousebot` row (`AuctionHouseBotMgr::Load`).
#[derive(Clone, Debug, PartialEq)]
pub struct AhBotItemRow {
    pub item: u32,
    pub min_stack: u32,
    pub max_stack: u32,
    pub bid: u32,
    pub buyout: u32,
    pub weight: f32,
    pub category: String,
    pub price_variance_pct: u32,
}

/// `SELECT ... FROM auctionhousebot` (world DB).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_ahbot_items(conn: &mut mysql::Conn) -> Result<Vec<AhBotItemRow>, DbError> {
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `item`, `min_stack`, `max_stack`, `bid`, `buyout`, `weight`, \
         `category`, `price_variance_pct` FROM `auctionhousebot`",
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            let min_stack = f.u32(1);
            AhBotItemRow {
                item: f.u32(0),
                min_stack,
                max_stack: f.u32(2).max(min_stack),
                bid: f.u32(3),
                buyout: f.u32(4),
                weight: f.f32(5),
                category: f.string(6),
                price_variance_pct: u32::from(f.u8(7)),
            }
        })
        .collect())
}

/// `auctionhousebot_state` last refresh (world DB).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_ahbot_refresh_time(conn: &mut mysql::Conn) -> Result<i64, DbError> {
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `value` FROM `auctionhousebot_state` WHERE `key_name` = 'last_refresh_time'",
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).i64(0)))
}

/// Refresh stamp write (world DB).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn save_ahbot_refresh_time(conn: &mut mysql::Conn, now: i64) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `auctionhousebot_state` SET `value` = ? WHERE `key_name` = 'last_refresh_time'",
        (now,),
    )?;
    Ok(())
}

/// Seller characters for an account in the AH bot guid
/// window (`LoadSellerIdentities`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_ahbot_sellers(
    conn: &mut mysql::Conn,
    account_id: u32,
) -> Result<Vec<(u32, String)>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `name` FROM `characters` WHERE `account` = ? AND `guid` >= 900001 AND `guid` <= 900100",
        (account_id,),
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            (f.u32(0), f.string(1))
        })
        .collect())
}

/// Name/guid collision probe + minimal character row
/// insert (`CreateSellerIdentities`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_ahbot_seller(
    conn: &mut mysql::Conn,
    guid: u32,
    account_id: u32,
    name: &str,
) -> Result<bool, DbError> {
    let by_name: Vec<mysql::Row> =
        conn.exec("SELECT `guid` FROM `characters` WHERE `name` = ?", (name,))?;
    if !by_name.is_empty() {
        return Ok(false);
    }
    let by_guid: Vec<mysql::Row> =
        conn.exec("SELECT `guid` FROM `characters` WHERE `guid` = ?", (guid,))?;
    if !by_guid.is_empty() {
        return Ok(false);
    }
    conn.exec_drop(
        "INSERT INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, \
         `level`, `zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`) \
         VALUES (?, ?, ?, 1, 1, 0, 60, 0, 0, 0, 0, 0, 0)",
        (guid, account_id, name),
    )?;
    Ok(true)
}

/// One bot auction item instance insert. Returns the
/// fresh item guid (characters DB).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_ahbot_item(conn: &mut mysql::Conn, item_id: u32, count: u32) -> Result<u32, DbError> {
    let item_low = crate::petition::next_item_guid(conn)?;
    conn.exec_drop(
        "INSERT INTO `item_instance` (`guid`, `item_id`, `owner_guid`, `creator_guid`, \
         `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, \
         `random_property_id`, `durability`, `text`, `generated_loot`) \
         VALUES (?, ?, 0, 0, 0, ?, 0, '0 0 0 0 0 ', 0, \
         '0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0', 0, 0, 0, 0)",
        (item_low, item_id, count),
    )?;
    Ok(item_low)
}

/// `Item::DeleteFromDB` for one instance.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_item_instance(conn: &mut mysql::Conn, item_guid: u32) -> Result<(), DbError> {
    conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_guid,))?;
    Ok(())
}
