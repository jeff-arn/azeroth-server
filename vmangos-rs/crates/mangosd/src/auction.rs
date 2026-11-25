//! Auction house (`AuctionHouseHandler.cpp` /
//! `AuctionHouseMgr`): entries, house resolution,
//! deposit/cut math, list filtering, and packets.
//! List queries answer synchronously (the fork's
//! async task + one-flight flag exist to spread load
//! the actor does not have); the usable/recipe-known
//! filters and localized suffix names wait for the
//! Phase 6 item system, so `usable` is ignored and
//! name matching uses the base prototype name.

use std::collections::HashMap;

use mangos_db::auction::StoredAuction;
use mangos_db::character_load::ItemInstanceRow;
use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::auction_house::{AuctionHouseDbc, AuctionHouseEntry as HouseEntry};
use mangos_world_data::item_prototype::ItemPrototype;

/// `MIN_AUCTION_TIME` (2 hours).
pub const MIN_AUCTION_TIME: u32 = 7_200;
/// How long a fresh auction is IP-locked in lists
/// (the fork's anti-sniping 5 minutes).
pub const IP_LOCK_SECS: i64 = 5 * 60;
/// `EQUIP_ERR_ITEM_NOT_FOUND` at build 5875.
pub const EQUIP_ERR_ITEM_NOT_FOUND: u32 = 23;
/// `UNIT_NPC_FLAG_AUCTIONEER`.
pub const NPC_FLAG_AUCTIONEER: u32 = 0x1000;
/// Client price cap in the sell handler.
pub const MAX_AUCTION_PRICE: u32 = 2_000_000_000;

/// `enum AuctionAction`.
pub mod action {
    pub const STARTED: u32 = 0;
    pub const REMOVED: u32 = 1;
    pub const BID_PLACED: u32 = 2;
}

/// `enum AuctionError`.
pub mod error {
    pub const OK: u32 = 0;
    pub const INVENTORY: u32 = 1;
    pub const DATABASE: u32 = 2;
    pub const NOT_ENOUGH_MONEY: u32 = 3;
    pub const ITEM_NOT_FOUND: u32 = 4;
    pub const HIGHER_BID: u32 = 5;
    pub const BID_INCREMENT: u32 = 7;
    pub const BID_OWN: u32 = 10;
    pub const RESTRICTED_ACCOUNT: u32 = 13;
}

/// `enum MailAuctionAnswers` (subject suffix ids).
pub mod mail_answer {
    pub const OUTBIDDED: u32 = 0;
    pub const WON: u32 = 1;
    pub const SUCCESSFUL: u32 = 2;
    pub const EXPIRED: u32 = 3;
    pub const CANCELLED_TO_BIDDER: u32 = 4;
    pub const CANCELED: u32 = 5;
}

/// `GetAuctionHouseTeam` buckets for linked houses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HouseTeam {
    Alliance,
    Horde,
    Neutral,
}

/// `GetAuctionHouseTeam`: house ids 1-3 Alliance,
/// 4-6 Horde, everything else neutral.
#[must_use]
pub fn house_team(house_id: u32) -> HouseTeam {
    match house_id {
        1..=3 => HouseTeam::Alliance,
        4..=6 => HouseTeam::Horde,
        _ => HouseTeam::Neutral,
    }
}

/// `LoadAuctionHouses`: house id -> house object
/// index, plus the object count. Cross-faction
/// shares one object; unlinked (pre-1.9) gives every
/// DBC entry its own; otherwise one per team.
#[must_use]
pub fn build_house_map(
    dbc: &AuctionHouseDbc,
    two_side: bool,
    unlinked: bool,
    wow_patch: u32,
) -> (HashMap<u32, usize>, usize) {
    let mut map = HashMap::new();
    if two_side {
        for entry in &dbc.entries {
            map.insert(entry.house_id, 0);
        }
        return (map, 1);
    }
    if unlinked && wow_patch < 7 {
        for (index, entry) in dbc.entries.iter().enumerate() {
            map.insert(entry.house_id, index);
        }
        return (map, dbc.entries.len());
    }
    // 0 Alliance, 1 Horde, 2 Neutral.
    for entry in &dbc.entries {
        let index = match house_team(entry.house_id) {
            HouseTeam::Alliance => 0,
            HouseTeam::Horde => 1,
            HouseTeam::Neutral => 2,
        };
        map.insert(entry.house_id, index);
    }
    (map, 3)
}

/// `GetAuctionHouseId(factionTemplateId)` without the
/// FactionTemplate.dbc fallback masks (that store is
/// not loaded yet; unknown templates take the same
/// goblin default the no-entry arm uses).
#[must_use]
pub fn house_id_for_faction_template(faction_template_id: u32) -> u32 {
    match faction_template_id {
        11 | 12 => 1,
        29 | 85 => 6,
        55 | 57 | 534 => 2,
        68 | 71 => 4,
        79 | 80 => 3,
        104 | 105 => 5,
        _ => 7,
    }
}

/// One live auction (`AuctionEntry`).
#[derive(Clone, Debug)]
pub struct AuctionEntry {
    pub id: u32,
    pub item_guid_low: u32,
    pub item_template: u32,
    pub owner: u32,
    pub owner_account: u32,
    pub start_bid: u32,
    pub bid: u32,
    pub buyout: u32,
    pub locked_ip: String,
    pub deposit_time: i64,
    pub expire_time: i64,
    pub bidder: u32,
    pub deposit: u32,
    pub house: HouseEntry,
}

impl AuctionEntry {
    /// A boot-loaded entry (`depositTime` starts 0,
    /// so the IP lock is already stale).
    #[must_use]
    pub fn from_stored(stored: &StoredAuction, house: HouseEntry, owner_account: u32) -> Self {
        Self {
            id: stored.id,
            item_guid_low: stored.item_guid,
            item_template: stored.item_id,
            owner: stored.seller_guid,
            owner_account,
            start_bid: stored.start_bid,
            bid: stored.last_bid,
            buyout: stored.buyout_price,
            locked_ip: String::new(),
            deposit_time: 0,
            expire_time: stored.expire_time,
            bidder: stored.buyer_guid,
            deposit: stored.deposit,
            house,
        }
    }

    /// The persisted row (`SaveToDB` fields).
    #[must_use]
    pub fn to_stored(&self) -> StoredAuction {
        StoredAuction {
            id: self.id,
            house_id: self.house.house_id,
            item_guid: self.item_guid_low,
            item_id: self.item_template,
            seller_guid: self.owner,
            buyout_price: self.buyout,
            expire_time: self.expire_time,
            buyer_guid: self.bidder,
            last_bid: self.bid,
            start_bid: self.start_bid,
            deposit: self.deposit,
        }
    }

    /// `GetAuctionCut`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn auction_cut(&self, rate: f32) -> u32 {
        (self.house.cut_percent as f32 * self.bid as f32 * rate / 100.0) as u32
    }

    /// `GetAuctionOutBid`: 5% of the current bid,
    /// minimum 1.
    #[must_use]
    pub fn out_bid(&self) -> u32 {
        let outbid = (self.bid / 100) * 5;
        if outbid == 0 { 1 } else { outbid }
    }

    /// `IsAvailableFor` (list visibility while the
    /// IP lock is fresh).
    #[must_use]
    pub fn is_available_for(&self, remote_ip: &str) -> bool {
        self.locked_ip.is_empty() || self.locked_ip == remote_ip
    }
}

/// `GetAuctionDeposit`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn auction_deposit(
    house: &HouseEntry,
    etime_secs: u32,
    sell_price: u32,
    count: u32,
    min_deposit: u32,
    rate: f32,
) -> u32 {
    let mut deposit = (sell_price * count * (etime_secs / MIN_AUCTION_TIME)) as f32;
    deposit = deposit * house.deposit_percent as f32 / 100.0;
    if deposit < min_deposit as f32 {
        deposit = min_deposit as f32;
    }
    (deposit * rate) as u32
}

/// The list-search filters (`AuctionHouseClientQuery`).
#[derive(Clone, Debug, Default)]
pub struct ListQuery {
    pub list_from: u32,
    pub searched_name: String,
    pub level_min: u8,
    pub level_max: u8,
    pub slot_id: u32,
    pub main_category: u32,
    pub sub_category: u32,
    pub quality: u32,
    pub usable: u8,
}

impl ListQuery {
    /// The no-filter fast path condition.
    #[must_use]
    pub fn is_unfiltered(&self) -> bool {
        self.main_category == u32::MAX
            && self.sub_category == u32::MAX
            && self.slot_id == u32::MAX
            && self.quality == u32::MAX
            && self.level_min == 0
            && self.level_max == 0
            && self.usable == 0
            && self.searched_name.is_empty()
    }

    /// The per-prototype filter arms (usable/recipe
    /// arms unported; name is a case-insensitive
    /// substring test, `Utf8FitTo` reduced to the
    /// base name).
    #[must_use]
    pub fn matches(&self, proto: &ItemPrototype) -> bool {
        if self.main_category != u32::MAX && proto.class != self.main_category {
            return false;
        }
        if self.sub_category != u32::MAX && proto.subclass != self.sub_category {
            return false;
        }
        // INVTYPE_CHEST (5) also matches INVTYPE_ROBE (20).
        if self.slot_id != u32::MAX
            && proto.inventory_type != self.slot_id
            && !(self.slot_id == 5 && proto.inventory_type == 20)
        {
            return false;
        }
        if self.quality != u32::MAX && proto.quality < self.quality {
            return false;
        }
        if self.level_min != 0
            && (proto.required_level < u32::from(self.level_min)
                || (self.level_max != 0 && proto.required_level > u32::from(self.level_max)))
        {
            return false;
        }
        if !self.searched_name.is_empty()
            && !proto
                .name
                .to_lowercase()
                .contains(&self.searched_name.to_lowercase())
        {
            return false;
        }
        true
    }
}

/// First space-separated integer of a stored
/// charges/enchantments blob (spell charges slot 0 /
/// PERM enchantment id).
fn first_int(blob: &str) -> i32 {
    blob.split_whitespace()
        .next()
        .and_then(|token| token.parse::<i32>().ok())
        .unwrap_or(0)
}

/// The item's random property id for notifications.
#[must_use]
pub fn random_property_of(item: &ItemInstanceRow) -> i32 {
    i32::from(item.random_property_id)
}

/// Writes an i32's wire bit pattern (the C++ writes
/// `uint32(int32)` for charges/random property).
#[allow(clippy::cast_sign_loss)]
fn write_i32_bits(writer: &mut PacketWriter, value: i32) {
    writer.write_u32_le(value as u32);
}

/// `BuildAuctionInfo`: appends one listing. Suffix
/// factor stays 0 until suffix data is loaded.
pub fn write_auction_info(
    writer: &mut PacketWriter,
    entry: &AuctionEntry,
    item: &ItemInstanceRow,
    now: i64,
) {
    writer.write_u32_le(entry.id);
    writer.write_u32_le(item.item_id);
    write_i32_bits(writer, first_int(&item.enchantments));
    write_i32_bits(writer, i32::from(item.random_property_id));
    writer.write_u32_le(0);
    writer.write_u32_le(item.count);
    write_i32_bits(writer, first_int(&item.charges));
    writer.write_u64_le(u64::from(entry.owner));
    writer.write_u32_le(entry.start_bid);
    writer.write_u32_le(if entry.bid != 0 { entry.out_bid() } else { 0 });
    writer.write_u32_le(entry.buyout);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    writer.write_u32_le(((entry.expire_time - now) * 1000) as u32);
    writer.write_u64_le(u64::from(entry.bidder));
    writer.write_u32_le(entry.bid);
}

/// `SendAuctionHello`.
#[must_use]
pub fn hello_packet(auctioneer: ObjectGuid, house_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(auctioneer.raw());
    writer.write_u32_le(house_id);
    let mut packet = WorldPacket::new(opcode_id::MSG_AUCTION_HELLO);
    packet.take_writer(writer);
    packet
}

/// `SendAuctionCommandResult`.
#[must_use]
pub fn command_result_packet(
    auction: Option<&AuctionEntry>,
    action_id: u32,
    error_code: u32,
    inv_error: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(auction.map_or(0, |entry| entry.id));
    writer.write_u32_le(action_id);
    writer.write_u32_le(error_code);
    match error_code {
        error::OK => {
            if action_id == action::BID_PLACED
                && let Some(entry) = auction
            {
                writer.write_u32_le(entry.out_bid());
            }
        }
        error::INVENTORY => {
            writer.write_u32_le(inv_error);
        }
        error::HIGHER_BID => {
            if let Some(entry) = auction {
                writer.write_u64_le(u64::from(entry.bidder));
                writer.write_u32_le(entry.bid);
                writer.write_u32_le(entry.out_bid());
            }
        }
        _ => {}
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_AUCTION_COMMAND_RESULT);
    packet.take_writer(writer);
    packet
}

/// `SendAuctionBidderNotification`.
#[must_use]
pub fn bidder_notification_packet(entry: &AuctionEntry, won: bool, random_id: i32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry.house.house_id);
    writer.write_u32_le(entry.id);
    writer.write_u64_le(u64::from(entry.bidder));
    writer.write_u32_le(if won { 0 } else { entry.bid });
    writer.write_u32_le(entry.out_bid());
    writer.write_u32_le(entry.item_template);
    write_i32_bits(&mut writer, random_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_AUCTION_BIDDER_NOTIFICATION);
    packet.take_writer(writer);
    packet
}

/// `SendAuctionOwnerNotification`.
#[must_use]
pub fn owner_notification_packet(entry: &AuctionEntry, sold: bool, random_id: i32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry.id);
    writer.write_u32_le(entry.bid);
    writer.write_u32_le(entry.out_bid());
    writer.write_u64_le(if sold { 0 } else { u64::from(entry.bidder) });
    writer.write_u32_le(entry.item_template);
    write_i32_bits(&mut writer, random_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_AUCTION_OWNER_NOTIFICATION);
    packet.take_writer(writer);
    packet
}

/// `SendAuctionRemovedNotification`.
#[must_use]
pub fn removed_notification_packet(entry: &AuctionEntry, random_id: i32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry.id);
    writer.write_u32_le(entry.item_template);
    write_i32_bits(&mut writer, random_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_AUCTION_REMOVED_NOTIFICATION);
    packet.take_writer(writer);
    packet
}

/// An auction mail subject (`"{template}:0:{answer}"`).
#[must_use]
pub fn mail_subject(item_template: u32, answer: u32) -> String {
    format!("{item_template}:0:{answer}")
}

/// The won-mail body (`width(16)` right-aligned hex
/// owner, then decimal bid/buyout).
#[must_use]
pub fn won_body(owner: u32, bid: u32, buyout: u32) -> String {
    format!("{owner:>16x}:{bid}:{buyout}")
}

/// The successful-mail body (winner hex, then
/// bid/buyout/deposit/cut).
#[must_use]
pub fn successful_body(bidder: u32, bid: u32, buyout: u32, deposit: u32, cut: u32) -> String {
    format!("{bidder:>16x}:{bid}:{buyout}:{deposit}:{cut}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn house() -> HouseEntry {
        HouseEntry {
            house_id: 6,
            faction: 85,
            deposit_percent: 5,
            cut_percent: 5,
        }
    }

    fn entry(bid: u32) -> AuctionEntry {
        AuctionEntry {
            id: 9,
            item_guid_low: 55,
            item_template: 2589,
            owner: 5,
            owner_account: 103,
            start_bid: 100,
            bid,
            buyout: 900,
            locked_ip: String::new(),
            deposit_time: 0,
            expire_time: 2000,
            bidder: 6,
            deposit: 10,
            house: house(),
        }
    }

    #[test]
    fn out_bid_and_cut() {
        assert_eq!(entry(0).out_bid(), 1);
        assert_eq!(entry(199).out_bid(), 5);
        assert_eq!(entry(1000).out_bid(), 50);
        assert_eq!(entry(1000).auction_cut(1.0), 50);
    }

    #[test]
    fn deposit_formula() {
        // 100 copper sell price, 3 items, 2h: 300 * 5% = 15.
        assert_eq!(auction_deposit(&house(), 7200, 100, 3, 0, 1.0), 15);
        // 12h scales x6, min deposit floors.
        assert_eq!(auction_deposit(&house(), 43200, 100, 3, 0, 1.0), 90);
        assert_eq!(auction_deposit(&house(), 7200, 1, 1, 25, 1.0), 25);
    }

    #[test]
    fn house_map_modes() {
        let dbc = AuctionHouseDbc::from_entries(
            (1..=7)
                .map(|house_id| HouseEntry {
                    house_id,
                    faction: 0,
                    deposit_percent: 5,
                    cut_percent: 5,
                })
                .collect(),
        );
        let (map, count) = build_house_map(&dbc, true, true, 0);
        assert_eq!(count, 1);
        assert!(map.values().all(|&i| i == 0));
        let (map, count) = build_house_map(&dbc, false, true, 0);
        assert_eq!(count, 7);
        assert_eq!(map[&7], 6);
        let (map, count) = build_house_map(&dbc, false, true, 10);
        assert_eq!(count, 3);
        assert_eq!((map[&1], map[&6], map[&7]), (0, 1, 2));
    }

    #[test]
    fn query_filters() {
        let proto = ItemPrototype {
            class: 7,
            quality: 2,
            required_level: 10,
            name: "Linen Cloth".to_owned(),
            ..Default::default()
        };
        let mut query = ListQuery {
            main_category: u32::MAX,
            sub_category: u32::MAX,
            slot_id: u32::MAX,
            quality: u32::MAX,
            ..Default::default()
        };
        assert!(query.is_unfiltered());
        assert!(query.matches(&proto));
        query.searched_name = "linen".into();
        assert!(query.matches(&proto));
        query.searched_name = "wool".into();
        assert!(!query.matches(&proto));
        query.searched_name.clear();
        query.level_min = 11;
        assert!(!query.matches(&proto));
    }

    #[test]
    fn info_packet_layout() {
        let item = ItemInstanceRow {
            creator_guid: 0,
            gift_creator_guid: 0,
            count: 3,
            duration: 0,
            charges: "2 0 0 0 0 ".to_owned(),
            flags: 0,
            enchantments: "13 0 0 ".to_owned(),
            random_property_id: -4,
            durability: 0,
            text: 0,
            item_guid: 55,
            item_id: 2589,
            generated_loot: 0,
        };
        let mut writer = PacketWriter::new();
        write_auction_info(&mut writer, &entry(250), &item, 1000);
        let body = writer.into_vec();
        assert_eq!(body.len(), 64);
        let u32_at = |off: usize| u32::from_le_bytes(body[off..off + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 9);
        assert_eq!(u32_at(4), 2589);
        assert_eq!(u32_at(8), 13);
        assert_eq!(i32::from_le_bytes(body[12..16].try_into().unwrap()), -4);
        assert_eq!(u32_at(20), 3);
        assert_eq!(u32_at(24), 2);
        assert_eq!(u64::from_le_bytes(body[28..36].try_into().unwrap()), 5);
        assert_eq!(u32_at(40), 10); // outbid: (250/100)*5
        assert_eq!(u32_at(48), 1_000_000); // 1000s left in ms
    }
}
