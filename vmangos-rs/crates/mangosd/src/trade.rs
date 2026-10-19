//! Player trade (`TradeHandler.cpp` / `TradeData`):
//! the per-player trade state, status/extended
//! packets, and the `CanBeTraded` row-level subset.
//! The trade spell (enchant/lockpick on the
//! non-traded slot) is not ported: `SetSpell` arrives
//! through the spell-cast path with an item target,
//! which the runtime item system owns in Phase 6, so
//! the spell field stays 0 and the non-traded item is
//! display-only.

use mangos_db::mail::StoredInventoryItem;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::item_prototype::ItemPrototype;

/// `TRADE_SLOT_COUNT`.
pub const TRADE_SLOT_COUNT: usize = 7;
/// `TRADE_SLOT_TRADED_COUNT`.
pub const TRADE_SLOT_TRADED_COUNT: usize = 6;
/// `TRADE_SLOT_NONTRADED`.
pub const TRADE_SLOT_NONTRADED: usize = 6;
/// `TRADE_DISTANCE` (`ObjectDefines.h`).
pub const TRADE_DISTANCE: f32 = 11.11;
/// The fork's anti-scam accept delay
/// (`SetScamPreventionDelay(200)`), milliseconds.
pub const SCAM_PREVENTION_DELAY_MS: i64 = 200;

/// `LANG_NOT_ENOUGH_GOLD` (`Language.h`).
pub const LANG_NOT_ENOUGH_GOLD: u32 = 801;
/// `LANG_NOT_FREE_TRADE_SLOTS`.
pub const LANG_NOT_FREE_TRADE_SLOTS: u32 = 802;
/// `LANG_NOT_PARTNER_FREE_TRADE_SLOTS`.
pub const LANG_NOT_PARTNER_FREE_TRADE_SLOTS: u32 = 803;

/// `enum TradeStatus` (`SharedDefines.h`).
pub mod status {
    pub const BUSY: u32 = 0;
    pub const BEGIN_TRADE: u32 = 1;
    pub const OPEN_WINDOW: u32 = 2;
    pub const TRADE_CANCELED: u32 = 3;
    pub const TRADE_ACCEPT: u32 = 4;
    pub const NO_TARGET: u32 = 6;
    pub const BACK_TO_TRADE: u32 = 7;
    pub const TRADE_COMPLETE: u32 = 8;
    pub const TARGET_TO_FAR: u32 = 10;
    pub const WRONG_FACTION: u32 = 11;
    pub const CLOSE_WINDOW: u32 = 12;
    pub const IGNORE_YOU: u32 = 14;
    pub const YOU_STUNNED: u32 = 15;
    pub const TARGET_STUNNED: u32 = 16;
    pub const YOU_DEAD: u32 = 17;
    pub const TARGET_DEAD: u32 = 18;
    pub const YOU_LOGOUT: u32 = 19;
    pub const TARGET_LOGOUT: u32 = 20;
    pub const TRIAL_ACCOUNT: u32 = 21;
    pub const ONLY_CONJURED: u32 = 22;
}

/// One offered item (identity + its inventory
/// position, re-read at accept like C++ re-resolves
/// the stored guid).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TradeItem {
    pub item_low: u32,
    pub bag: u32,
    pub slot: u32,
}

/// `TradeData` for one side of a trade.
#[derive(Clone, Debug)]
pub struct TradeData {
    /// Partner character guid (full wire guid).
    pub partner_guid: u64,
    pub accepted: bool,
    pub money: u32,
    pub items: [Option<TradeItem>; TRADE_SLOT_COUNT],
    /// `m_lastModificationTime` (unix seconds).
    pub last_modification: i64,
}

impl TradeData {
    /// A fresh trade side (`TradeData(player, trader)`).
    #[must_use]
    pub fn new(partner_guid: u64) -> Self {
        Self {
            partner_guid,
            accepted: false,
            money: 0,
            items: [None; TRADE_SLOT_COUNT],
            last_modification: 0,
        }
    }

    /// `HasItem`: the item is already in some slot.
    #[must_use]
    pub fn has_item(&self, item_low: u32) -> bool {
        self.items
            .iter()
            .any(|item| item.is_some_and(|entry| entry.item_low == item_low))
    }

    /// Offered traded-slot items (first six slots).
    pub fn traded_items(&self) -> impl Iterator<Item = TradeItem> + '_ {
        self.items[..TRADE_SLOT_TRADED_COUNT]
            .iter()
            .filter_map(|item| *item)
    }

    /// Count of offered traded-slot items.
    #[must_use]
    pub fn traded_count(&self) -> u32 {
        u32::try_from(self.traded_items().count()).unwrap_or(u32::MAX)
    }
}

/// One item block of `SMSG_TRADE_STATUS_EXTENDED`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeItemView {
    pub entry: u32,
    pub display_id: u32,
    pub count: u32,
    pub wrapped: bool,
    pub gift_creator_guid: u64,
    pub perm_enchant_id: u32,
    pub creator_guid: u64,
    pub charges: i32,
    pub suffix_factor: u32,
    pub random_property_id: i32,
    pub lock_id: u32,
    pub max_durability: u32,
    pub durability: u32,
}

impl TradeItemView {
    /// Build the view from a stored row + prototype
    /// (`SendUpdateTrade` field reads; suffix factor
    /// stays 0 until suffix data is loaded, as in the
    /// mail list).
    #[must_use]
    pub fn from_stored(item: &StoredInventoryItem, proto: &ItemPrototype) -> Self {
        let charges = item
            .charges
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<i32>().ok())
            .unwrap_or(0);
        let perm_enchant_id = item
            .enchantments
            .split_whitespace()
            .next()
            .and_then(|token| token.parse::<u32>().ok())
            .unwrap_or(0);
        Self {
            entry: item.item_id,
            display_id: proto.display_id,
            count: item.count,
            wrapped: item.flags & ITEM_DYNFLAG_WRAPPED != 0,
            gift_creator_guid: u64::from(item.gift_creator_guid),
            perm_enchant_id,
            creator_guid: u64::from(item.creator_guid),
            charges,
            suffix_factor: 0,
            random_property_id: i32::from(item.random_property_id),
            lock_id: proto.lock_id,
            max_durability: proto.max_durability,
            durability: u32::from(item.durability),
        }
    }
}

/// `ITEM_DYNFLAG_WRAPPED` in the stored flags word.
const ITEM_DYNFLAG_WRAPPED: u32 = 0x8;
/// Stored-flags soulbound bit (the mail slice's
/// `IsSoulBound` row rule).
const ITEM_FLAG_SOULBOUND: u32 = 0x1;

/// The row-level subset of `Item::CanBeTraded`:
/// soulbound, non-empty/equipped bags, and generated
/// loot are refused (`CanUnequipItem`, loot-guid, and
/// bound-by-enchant need the item system).
#[must_use]
pub fn can_be_traded(item: &StoredInventoryItem, proto: &ItemPrototype) -> bool {
    if item.flags & ITEM_FLAG_SOULBOUND != 0 {
        return false;
    }
    if proto.class == 1 && (is_bag_pos(item.bag, item.slot) || item.items_in_bag > 0) {
        return false;
    }
    item.generated_loot == 0
}

/// `IsBagPos` for a stored position (bag slots of the
/// backpack row set).
#[must_use]
pub fn is_bag_pos(bag: u32, slot: u32) -> bool {
    bag == 0 && (19..23).contains(&slot)
}

/// `Player::IsBankPos` over client positions
/// (`INVENTORY_SLOT_BAG_0` is 255 on the wire).
#[must_use]
pub fn is_bank_pos(bag: u32, slot: u32) -> bool {
    (bag == 255 && (39..69).contains(&slot)) || (63..69).contains(&bag)
}

/// `SendTradeStatus` for the generic statuses.
#[must_use]
pub fn status_packet(status: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(status);
    match status {
        status::BEGIN_TRADE => {
            writer.write_u64_le(0);
        }
        status::CLOSE_WINDOW => {
            writer.write_u32_le(0);
            writer.write_u8(0);
            writer.write_u32_le(0);
        }
        status::ONLY_CONJURED => {
            writer.write_u8(0);
        }
        _ => {}
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_TRADE_STATUS);
    packet.take_writer(writer);
    packet
}

/// The initiate packet: `TRADE_STATUS_BEGIN_TRADE`
/// with the initiator's guid (`HandleInitiateTrade
/// Opcode` crafts it directly, bypassing the
/// zero-guid generic form).
#[must_use]
pub fn begin_trade_packet(initiator_guid: u64) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(status::BEGIN_TRADE);
    writer.write_u64_le(initiator_guid);
    let mut packet = WorldPacket::new(opcode_id::SMSG_TRADE_STATUS);
    packet.take_writer(writer);
    packet
}

/// `SendUpdateTrade`: one side's window state. The
/// spell field is 0 (trade spells unported).
#[must_use]
pub fn extended_packet(
    trader_state: bool,
    money: u32,
    items: &[Option<TradeItemView>; TRADE_SLOT_COUNT],
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(u8::from(trader_state));
    writer.write_u32_le(7);
    writer.write_u32_le(7);
    writer.write_u32_le(money);
    writer.write_u32_le(0);
    for (index, item) in items.iter().enumerate() {
        writer.write_u8(u8::try_from(index).unwrap_or(u8::MAX));
        if let Some(view) = item {
            writer.write_u32_le(view.entry);
            writer.write_u32_le(view.display_id);
            writer.write_u32_le(view.count);
            writer.write_u32_le(u32::from(view.wrapped));
            writer.write_u64_le(view.gift_creator_guid);
            writer.write_u32_le(view.perm_enchant_id);
            writer.write_u64_le(view.creator_guid);
            #[allow(clippy::cast_sign_loss)] // wire bit pattern of the i32 charges
            writer.write_u32_le(view.charges as u32);
            writer.write_u32_le(view.suffix_factor);
            #[allow(clippy::cast_sign_loss)] // wire bit pattern of the i32 property id
            writer.write_u32_le(view.random_property_id as u32);
            writer.write_u32_le(view.lock_id);
            writer.write_u32_le(view.max_durability);
            writer.write_u32_le(view.durability);
        } else {
            for _ in 0..15 {
                writer.write_u32_le(0);
            }
        }
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_TRADE_STATUS_EXTENDED);
    packet.take_writer(writer);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(class_flags: u32, items_in_bag: u32) -> StoredInventoryItem {
        StoredInventoryItem {
            bag: 0,
            slot: 23,
            item_guid: 55,
            item_id: 2589,
            count: 3,
            duration: 0,
            charges: "7 0 0 0 0 ".to_owned(),
            flags: class_flags,
            enchantments: "13 0 0 ".to_owned(),
            random_property_id: -4,
            durability: 9,
            text: 0,
            creator_guid: 8,
            gift_creator_guid: 9,
            generated_loot: 0,
            items_in_bag,
        }
    }

    fn proto(class: u32) -> ItemPrototype {
        ItemPrototype {
            class,
            display_id: 100,
            lock_id: 4,
            max_durability: 20,
            ..Default::default()
        }
    }

    #[test]
    fn can_be_traded_rules() {
        assert!(can_be_traded(&stored(0, 0), &proto(7)));
        assert!(!can_be_traded(&stored(0x1, 0), &proto(7)));
        assert!(!can_be_traded(&stored(0, 2), &proto(1)));
        assert!(can_be_traded(&stored(0, 0), &proto(1)));
        let mut loot = stored(0, 0);
        loot.generated_loot = 1;
        assert!(!can_be_traded(&loot, &proto(7)));
    }

    #[test]
    fn extended_packet_layout() {
        let view = TradeItemView::from_stored(&stored(0x8, 0), &proto(7));
        let mut items: [Option<TradeItemView>; 7] = Default::default();
        items[0] = Some(view);
        let packet = extended_packet(true, 250, &items);
        let payload = packet.body();
        // header: u8 + 4*u32 = 17; each slot 1 + 60 = 61.
        assert_eq!(payload.len(), 17 + 7 * 61);
        assert_eq!(payload[0], 1);
        assert_eq!(u32::from_le_bytes(payload[9..13].try_into().unwrap()), 250);
        // slot 0: index byte, entry, display, count, wrapped.
        assert_eq!(payload[17], 0);
        assert_eq!(
            u32::from_le_bytes(payload[18..22].try_into().unwrap()),
            2589
        );
        assert_eq!(u32::from_le_bytes(payload[22..26].try_into().unwrap()), 100);
        assert_eq!(u32::from_le_bytes(payload[26..30].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(payload[30..34].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(payload[34..42].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(payload[42..46].try_into().unwrap()), 13);
    }

    #[test]
    fn status_packet_shapes() {
        assert_eq!(status_packet(status::BUSY).body().len(), 4);
        assert_eq!(status_packet(status::CLOSE_WINDOW).body().len(), 13);
        assert_eq!(status_packet(status::ONLY_CONJURED).body().len(), 5);
        let begin = begin_trade_packet(42);
        assert_eq!(begin.body().len(), 12);
        assert_eq!(
            u64::from_le_bytes(begin.body()[4..12].try_into().unwrap()),
            42
        );
    }
}
