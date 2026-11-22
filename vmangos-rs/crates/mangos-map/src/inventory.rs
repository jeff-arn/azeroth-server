//! Player inventory slot operations (Tier 4 step
//! 4b2): fork `ItemHandler.cpp` packet arms +
//! `Player` inventory methods over the row model
//! (`deferred.inventory` live rows; persistence via
//! the step-4a save diff). Client sync is
//! snapshot-diff: creates for new item GUIDs,
//! destroys for removed, values blocks for changed,
//! plus the player's inv-slot/visible recompute.
//!
//! Recorded unmodeled: `m_weaponChangeTimer` gate,
//! logout-in-progress equip gate, `IsBindedNotWith`
//! (rows are always the player's own), temporary
//! loot = `generated_loot` rows, auto-unequip
//! mail fallback, ammo-id refresh on stack change,
//! `SetAmmo`/sheath ride the derived-stat refresh.

use mangos_entities::item::{ItemProtoFacts, fill_item_values, item_guid, item_object};
use mangos_entities::player::InventoryRow;
use mangos_entities::player::Player;
use mangos_entities::update::{
    BuildEffects, ObjectView, Viewer, build_create_block, build_values_block, destroy_packet,
};
use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_data::UpdateData;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::item_prototype::{ItemPrototype, allowed_equip_slots};

use crate::map::{Map, MapEvent};

// Slot ranges (`Player.h`).
pub const EQUIPMENT_SLOT_END: u8 = 19;
pub const INVENTORY_SLOT_BAG_START: u8 = 19;
pub const INVENTORY_SLOT_BAG_END: u8 = 23;
pub const INVENTORY_SLOT_ITEM_START: u8 = 23;
pub const INVENTORY_SLOT_ITEM_END: u8 = 39;
pub const BANK_SLOT_ITEM_START: u8 = 39;
pub const BANK_SLOT_ITEM_END: u8 = 63;
pub const BANK_SLOT_BAG_START: u8 = 63;
pub const BANK_SLOT_BAG_END: u8 = 69;
pub const BUYBACK_SLOT_START: u8 = 69;
pub const BUYBACK_SLOT_END: u8 = 81;
pub const KEYRING_SLOT_START: u8 = 81;
pub const KEYRING_SLOT_END: u8 = 97;
pub const NULL_BAG: u8 = 0;
pub const NULL_SLOT: u8 = 0xff;
/// Packet bag byte for the main inventory.
pub const BAG_MAIN: u8 = 0xff;

pub const SLOT_MAIN_HAND: u8 = 15;
pub const SLOT_OFF_HAND: u8 = 16;
pub const SLOT_RANGED: u8 = 17;
pub const SLOT_TABARD: u8 = 18;

// `InventoryResult` (`ItemDefines.h`, 5875 values).
pub type InvResult = u8;
pub const ERR_OK: InvResult = 0;
pub const ERR_CANT_EQUIP_LEVEL_I: InvResult = 1;
pub const ERR_CANT_EQUIP_SKILL: InvResult = 2;
pub const ERR_DOESNT_GO_TO_SLOT: InvResult = 3;
pub const ERR_BAG_FULL: InvResult = 4;
pub const ERR_NONEMPTY_BAG_OVER_OTHER_BAG: InvResult = 5;
pub const ERR_CANT_TRADE_EQUIP_BAGS: InvResult = 6;
pub const ERR_ONLY_AMMO_CAN_GO_HERE: InvResult = 7;
pub const ERR_NO_PROFICIENCY: InvResult = 8;
pub const ERR_NO_EQUIPMENT_SLOT_AVAILABLE: InvResult = 9;
pub const ERR_CANT_EQUIP_EVER: InvResult = 10;
pub const ERR_TWOHANDED_EQUIPPED: InvResult = 13;
pub const ERR_CANT_DUAL_WIELD: InvResult = 14;
pub const ERR_ITEM_DOESNT_GO_INTO_BAG: InvResult = 15;
pub const ERR_ITEM_DOESNT_GO_INTO_BAG2: InvResult = 16;
pub const ERR_CANT_CARRY_MORE_OF_THIS: InvResult = 17;
pub const ERR_ITEM_CANT_BE_EQUIPPED: InvResult = 20;
pub const ERR_ITEMS_CANT_BE_SWAPPED: InvResult = 21;
pub const ERR_SLOT_IS_EMPTY: InvResult = 22;
pub const ERR_ITEM_NOT_FOUND: InvResult = 23;
pub const ERR_CANT_DROP_SOULBOUND: InvResult = 24;
pub const ERR_OUT_OF_RANGE: InvResult = 25;
pub const ERR_TOO_MANY_TO_SPLIT: InvResult = 26;
pub const ERR_COULDNT_SPLIT_ITEMS: InvResult = 27;
pub const ERR_NOT_A_BAG: InvResult = 30;
pub const ERR_TOO_FAR_AWAY_FROM_BANK: InvResult = 35;
pub const ERR_ITEM_LOCKED: InvResult = 36;
pub const ERR_YOU_ARE_STUNNED: InvResult = 37;
pub const ERR_YOU_ARE_DEAD: InvResult = 38;
pub const ERR_CANT_DO_RIGHT_NOW: InvResult = 39;
pub const ERR_INTERNAL_BAG_ERROR: InvResult = 40;
pub const ERR_ALREADY_LOOTED: InvResult = 49;
pub const ERR_INVENTORY_FULL: InvResult = 50;
pub const ERR_BANK_FULL: InvResult = 51;
pub const ERR_OBJECT_IS_BUSY: InvResult = 58;
pub const ERR_NOT_IN_COMBAT: InvResult = 60;
pub const ERR_NOT_WHILE_DISARMED: InvResult = 61;
pub const ERR_CANT_EQUIP_RANK: InvResult = 63;
pub const ERR_CANT_EQUIP_REPUTATION: InvResult = 64;
pub const ERR_TOO_MANY_SPECIAL_BAGS: InvResult = 65;

const ITEM_FLAG_INDESTRUCTIBLE: u32 = 0x0000_0020;
const ITEM_FLAG_UNIQUE_EQUIPPED: u32 = 0x0008_0000;
const ITEM_DYNFLAG_BINDED: u32 = 0x0000_0001;
const BAG_FAMILY_KEYS: i32 = 9;

/// Packet `(bag, slot)` position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvPos {
    pub bag: u8,
    pub slot: u8,
}

impl InvPos {
    #[must_use]
    pub fn new(bag: u8, slot: u8) -> Self {
        Self { bag, slot }
    }
    #[must_use]
    pub fn packed(self) -> u16 {
        (u16::from(self.bag) << 8) | u16::from(self.slot)
    }
    #[must_use]
    pub fn is_equipment(self) -> bool {
        self.bag == BAG_MAIN && self.slot < EQUIPMENT_SLOT_END
    }
    #[must_use]
    pub fn is_bag_slot(self) -> bool {
        self.bag == BAG_MAIN
            && ((INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&self.slot)
                || (BANK_SLOT_BAG_START..BANK_SLOT_BAG_END).contains(&self.slot))
    }
    #[must_use]
    pub fn is_inventory(self) -> bool {
        if self.bag == BAG_MAIN {
            (INVENTORY_SLOT_ITEM_START..INVENTORY_SLOT_ITEM_END).contains(&self.slot)
                || (KEYRING_SLOT_START..KEYRING_SLOT_END).contains(&self.slot)
        } else {
            (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&self.bag)
        }
    }
    #[must_use]
    pub fn is_bank(self) -> bool {
        if self.bag == BAG_MAIN {
            (BANK_SLOT_ITEM_START..BANK_SLOT_BAG_END).contains(&self.slot)
        } else {
            (BANK_SLOT_BAG_START..BANK_SLOT_BAG_END).contains(&self.bag)
        }
    }
}

/// Row index for a packet position.
pub(crate) fn row_index_at(rows: &[InventoryRow], pos: InvPos) -> Option<usize> {
    if pos.bag == BAG_MAIN {
        rows.iter().position(|r| r.bag == 0 && r.slot == pos.slot)
    } else {
        let bag_guid = rows
            .iter()
            .find(|r| r.bag == 0 && r.slot == pos.bag)
            .map(|r| r.item.item_guid)?;
        rows.iter()
            .position(|r| r.bag == bag_guid && r.slot == pos.slot)
    }
}

/// Bag row index + capacity for a bag position
/// (packet bag byte = the bag's own slot).
fn bag_capacity(rows: &[InventoryRow], bag_slot: u8, protos: &impl ProtoLookup) -> Option<u32> {
    let bag = rows.iter().find(|r| r.bag == 0 && r.slot == bag_slot)?;
    let proto = protos.get(bag.item.item_id)?;
    (proto.container_slots > 0).then_some(proto.container_slots)
}

/// Prototype lookup seam.
pub trait ProtoLookup {
    fn get(&self, item_id: u32) -> Option<&ItemPrototype>;
}

impl ProtoLookup for mangos_world_data::item_prototype::ItemPrototypes {
    fn get(&self, item_id: u32) -> Option<&ItemPrototype> {
        self.get(item_id)
    }
}

/// `Bag::IsEmpty`.
pub(crate) fn bag_is_empty(rows: &[InventoryRow], bag_item_guid: u32) -> bool {
    rows.iter().all(|r| r.bag != bag_item_guid)
}

/// `ItemCanGoIntoBag` (item proto, bag proto).
fn item_can_go_into_bag(item_proto: &ItemPrototype, bag_proto: &ItemPrototype) -> bool {
    match bag_proto.class {
        // ITEM_CLASS_CONTAINER
        1 => match bag_proto.subclass {
            0 => true,                       // normal bags
            1 => item_proto.bag_family == 3, // soul bag
            2 => item_proto.bag_family == 6, // herb bag
            3 => item_proto.bag_family == 7, // enchanting bag
            4 => item_proto.bag_family == 8, // engineering bag
            6 => item_proto.bag_family == 1, // quiver: arrows
            7 => item_proto.bag_family == 2, // ammo pouch: bullets
            _ => false,
        },
        // ITEM_CLASS_QUIVER
        11 => match bag_proto.subclass {
            2 => item_proto.bag_family == 1,
            3 => item_proto.bag_family == 2,
            _ => false,
        },
        _ => false,
    }
}

/// `Player::GetMaxKeyringSize`.
fn max_keyring_size(level: u32) -> u32 {
    if level > 60 {
        16
    } else if level >= 50 {
        12
    } else if level >= 40 {
        8
    } else {
        4
    }
}

/// `ItemPrototype::CanChangeEquipStateInCombat`.
fn can_change_equip_in_combat(proto: &ItemPrototype) -> bool {
    matches!(proto.inventory_type, 28 | 14 | 23) || proto.class == 2 || proto.class == 6
}

/// `Item::GetProficiencySkill`.
fn proficiency_skill(proto: &ItemPrototype) -> u32 {
    match proto.class {
        2 => match proto.subclass {
            0 => 44,      // 1H axes
            1 => 172,     // 2H axes
            2 | 14 => 45, // bows; misc (one-hand exotics class quirk)
            3 => 46,      // guns
            4 => 54,      // 1H maces
            5 => 160,     // 2H maces
            6 => 229,     // polearms
            7 => 43,      // 1H swords
            8 => 55,      // 2H swords
            10 => 136,    // staves
            13 => 473,    // fist weapons
            15 => 173,    // daggers
            16 => 176,    // thrown
            18 => 226,    // crossbows
            19 => 228,    // wands
            20 => 356,    // fishing pole
            _ => 0,
        },
        4 => match proto.subclass {
            1 => 415, // cloth
            2 => 414, // leather
            3 => 413, // mail
            4 => 293, // plate
            6 => 433, // shield
            _ => 0,
        },
        _ => 0,
    }
}

/// One store destination: position + count slice.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StoreDest {
    pub pos: InvPos,
    pub count: u32,
}

/// Landing plan for one side of a real swap.
enum DestPlan {
    Store(Vec<StoreDest>),
    Equip(u8),
}

/// Landing plan for the auto-equip-slot fallback chain.
enum Fallback {
    Store(Vec<StoreDest>),
    Equip(u8),
}

impl Map {
    pub(crate) fn inv_proto(&self, item_id: u32) -> Option<&ItemPrototype> {
        self.stores.item_prototypes.get(item_id)
    }

    /// `SendEquipError` packet event.
    pub(crate) fn send_inventory_error(
        &mut self,
        to: ObjectGuid,
        msg: InvResult,
        item1: ObjectGuid,
        item2: ObjectGuid,
        required_level: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u8(msg);
        if msg != ERR_OK {
            if msg == ERR_CANT_EQUIP_LEVEL_I {
                writer.write_u32_le(required_level);
            }
            writer.write_u64_le(item1.raw());
            writer.write_u64_le(item2.raw());
            writer.write_u8(0);
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_INVENTORY_CHANGE_FAILURE);
        packet.take_writer(writer);
        self.events.push(MapEvent::Packet { to, packet });
    }

    /// `GetSkillValue` over deferred skill rows
    /// (base value; bonus words ride the same rows).
    fn skill_value(player: &Player, skill: u32) -> u32 {
        player
            .deferred
            .skills
            .iter()
            .find(|s| s.skill == skill)
            .map_or(0, |s| s.value)
    }

    /// `ReputationMgr::GetRank` (same math as the
    /// actor-side mirror in `mangosd`).
    pub(crate) fn reputation_rank(&self, player: &Player, faction: u32) -> u32 {
        const POINTS_IN_RANK: [i32; 8] = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];
        let factions = &self.stores.factions;
        let Some(entry) = factions.get(faction) else {
            return 3; // unknown faction: Neutral
        };
        let race_mask = 1u32 << u32::from(player.unit.race().saturating_sub(1));
        let class_mask = 1u32 << u32::from(player.unit.class().saturating_sub(1));
        let base = entry
            .index_fit_to(race_mask, class_mask)
            .map_or(0, |idx| entry.base_values[idx]);
        let standing = player
            .deferred
            .reputation
            .iter()
            .find(|r| r.faction == faction)
            .map_or(0, |r| r.standing);
        let reputation = base + standing;
        let mut limit = 43_000i32; // Reputation_Cap + 1
        for rank in (0..8u32).rev() {
            limit -= POINTS_IN_RANK[rank as usize];
            if reputation >= limit {
                return rank;
            }
        }
        0
    }

    /// `Player::CanUseItem(Item)` + proto variant
    /// (`Player.cpp:9969,10001`), `not_loading` form.
    /// `IsBindedNotWith` is always false for own rows.
    pub(crate) fn can_use_item(&self, player: &Player, proto: &ItemPrototype) -> InvResult {
        if !player.unit.is_alive() {
            return ERR_YOU_ARE_DEAD;
        }
        let class_mask = 1u32 << (u32::from(player.unit.class().saturating_sub(1)).min(30));
        let race_mask = 1u32 << (u32::from(player.unit.race().saturating_sub(1)).min(30));
        if (proto.allowable_class & class_mask) == 0 || (proto.allowable_race & race_mask) == 0 {
            return ERR_CANT_EQUIP_EVER;
        }
        if proto.required_skill != 0 {
            let value = Self::skill_value(player, proto.required_skill);
            if value == 0 {
                return ERR_NO_PROFICIENCY;
            }
            if value < proto.required_skill_rank {
                return ERR_CANT_EQUIP_SKILL;
            }
        }
        if proto.required_spell != 0
            && !player
                .deferred
                .spells
                .iter()
                .any(|s| s.spell == proto.required_spell)
        {
            return ERR_NO_PROFICIENCY;
        }
        if i64::from(player.honor_highest_rank) < i64::from(proto.required_honor_rank) {
            return ERR_CANT_EQUIP_RANK;
        }
        if player.unit.level() < proto.required_level {
            return ERR_CANT_EQUIP_LEVEL_I;
        }
        let mut skill = proficiency_skill(proto);
        if proto.class == 2 && proto.subclass == 13 {
            skill = 473; // fist weapons presence check
        }
        if skill != 0 && Self::skill_value(player, skill) == 0 {
            return ERR_NO_PROFICIENCY;
        }
        if proto.required_reputation_faction != 0
            && self.reputation_rank(player, proto.required_reputation_faction)
                < proto.required_reputation_rank
        {
            return ERR_CANT_EQUIP_REPUTATION;
        }
        ERR_OK
    }

    /// `GetItemCount(entry, bank=true)` minus the
    /// source item.
    fn item_count_excluding(rows: &[InventoryRow], item_id: u32, except_guid: u32) -> u32 {
        rows.iter()
            .filter(|r| r.item.item_id == item_id && r.item.item_guid != except_guid)
            .map(|r| r.item.count)
            .sum()
    }

    /// `_CanTakeMoreSimilarItems`.
    fn can_take_more_similar(
        rows: &[InventoryRow],
        proto: &ItemPrototype,
        count: u32,
        src_guid: u32,
    ) -> InvResult {
        if proto.max_count > 0 {
            let cur = Self::item_count_excluding(rows, proto.item_id, src_guid);
            if cur + count > proto.max_count {
                return ERR_CANT_CARRY_MORE_OF_THIS;
            }
        }
        ERR_OK
    }

    /// `CanEquipUniqueItem` (flag + `UniqueEquipped`
    /// category arm; category limit table via
    /// item-prototype fields — 1.12 items use the
    /// flag arm).
    fn can_equip_unique(
        rows: &[InventoryRow],
        proto: &ItemPrototype,
        except_slot: u8,
    ) -> InvResult {
        if proto.flags & ITEM_FLAG_UNIQUE_EQUIPPED != 0 {
            for slot in 0..EQUIPMENT_SLOT_END {
                if slot == except_slot {
                    continue;
                }
                if rows
                    .iter()
                    .any(|r| r.bag == 0 && r.slot == slot && r.item.item_id == proto.item_id)
                {
                    return ERR_CANT_EQUIP_EVER;
                }
            }
        }
        ERR_OK
    }

    /// Bank bag slots bought (`PLAYER_BYTES_2` byte 2).
    fn bank_bag_slot_count(player: &Player) -> u8 {
        let word = player
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_BYTES_2);
        u8::try_from((word >> 16) & 0xff).unwrap_or(0)
    }

    /// `CanUseBank` over `m_currentBankerGuid`
    /// (fork
    /// `UNIT_NPC_FLAG_BANKER`; interact distance
    /// via the standard 11.1 yd combined bound
    /// used by `GetNPCIfCanInteractWith`).
    fn can_use_bank(&self, player: &Player) -> bool {
        // `CanUseBank(m_currentBankerGuid)`
        // (`Player.cpp:2450-2467`): the bank-command
        // self form, else the full NPC interaction
        // gate against the recorded banker (pure
        // check; planners do not run the preamble).
        let banker = player.current_banker_guid;
        if banker == player.unit.guid() {
            return true;
        }
        if banker == ObjectGuid::EMPTY {
            return false;
        }
        self.npc_can_interact(player.unit.guid(), banker, crate::vendor::NPC_FLAG_BANKER)
    }

    /// `IsValidPos(bag, slot, true)`.
    fn is_valid_pos(&self, player: &Player, pos: InvPos) -> bool {
        let rows = &player.deferred.inventory;
        if pos.bag == BAG_MAIN {
            if (BANK_SLOT_BAG_START..BANK_SLOT_BAG_END).contains(&pos.slot) {
                return pos.slot - BANK_SLOT_BAG_START < Self::bank_bag_slot_count(player);
            }
            return pos.slot < KEYRING_SLOT_END;
        }
        if ((INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&pos.bag)
            || (BANK_SLOT_BAG_START..BANK_SLOT_BAG_END).contains(&pos.bag))
            && let Some(cap) = bag_capacity(rows, pos.bag, &self.stores.item_prototypes)
        {
            return u32::from(pos.slot) < cap;
        }
        false
    }

    /// Bag proto + guid at a bag slot position.
    fn bag_at<'a>(
        &'a self,
        rows: &'a [InventoryRow],
        bag_slot: u8,
    ) -> Option<(&'a ItemPrototype, u32)> {
        let bag = rows.iter().find(|r| r.bag == 0 && r.slot == bag_slot)?;
        let proto = self.inv_proto(bag.item.item_id)?;
        (proto.container_slots > 0).then_some((proto, bag.item.item_guid))
    }

    /// `_CanStoreItem_InSpecificSlot` — returns the
    /// remaining count after this slot on success.
    #[allow(clippy::too_many_arguments)]
    fn can_store_specific(
        &self,
        player: &Player,
        dest: &mut Vec<StoreDest>,
        bag: u8,
        slot: u8,
        proto: &ItemPrototype,
        count: u32,
        swap: bool,
        src: Option<&InventoryRow>,
    ) -> Result<u32, InvResult> {
        let rows = &player.deferred.inventory;
        let pos = InvPos::new(bag, slot);
        let existing = row_index_at(rows, pos).map(|i| &rows[i]);
        // Source-item or matching merge candidate.
        if let Some(item) = existing
            && (src.is_some_and(|s| s.item.item_guid == item.item.item_guid)
                || (item.item.item_id == proto.item_id && proto.stackable > 1))
        {
            let need_space = proto.stackable.saturating_sub(item.item.count).min(count);
            dest.push(StoreDest {
                pos,
                count: item.item.count + need_space,
            });
            return Ok(count - need_space);
        }
        if existing.is_some() && !swap {
            return Err(ERR_BAG_FULL);
        }
        // Empty (or swap-occupied) slot fit checks.
        let mut err: Option<InvResult> = None;
        if bag != BAG_MAIN {
            let Some((bag_proto, _guid)) = self.bag_at(rows, bag) else {
                return Err(ERR_ITEM_NOT_FOUND);
            };
            let bag_proto = bag_proto.clone();
            if !item_can_go_into_bag(proto, &bag_proto) {
                err = Some(ERR_ITEM_DOESNT_GO_INTO_BAG);
            }
        } else if (KEYRING_SLOT_START..KEYRING_SLOT_END).contains(&slot) {
            if proto.bag_family != BAG_FAMILY_KEYS {
                err = Some(ERR_ITEM_DOESNT_GO_INTO_BAG);
            } else if u32::from(slot - KEYRING_SLOT_START) >= max_keyring_size(player.unit.level())
            {
                err = Some(ERR_BAG_FULL);
            }
        }
        if let Some(e) = err {
            return Err(e);
        }
        let need_space = proto.stackable.min(count);
        dest.push(StoreDest {
            pos,
            count: need_space,
        });
        Ok(count - need_space)
    }

    /// Auto-store planner (`CanStoreItem` inventory
    /// arm): specific slot if given, else merge
    /// phase then free phase in fork order. On
    /// error returns the code + leftover count.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)] // fork CanStoreItem signature
    pub(crate) fn can_store_auto(
        &self,
        player: &Player,
        bag: u8,
        slot: u8,
        proto: &ItemPrototype,
        mut count: u32,
        swap: bool,
        src: Option<&InventoryRow>,
    ) -> Result<Vec<StoreDest>, InvResult> {
        let mut dest: Vec<StoreDest> = Vec::new();
        let res = Self::can_take_more_similar(
            &player.deferred.inventory,
            proto,
            count,
            src.map_or(0, |s| s.item.item_guid),
        );
        if res != ERR_OK {
            return Err(res);
        }
        // Explicit position.
        if slot != NULL_SLOT {
            let bag = if bag == NULL_BAG { BAG_MAIN } else { bag };
            match self.can_store_specific(player, &mut dest, bag, slot, proto, count, swap, src) {
                Ok(0) => return Ok(dest),
                Ok(_) => return Err(ERR_BAG_FULL),
                Err(e) => return Err(e),
            }
        }
        // Specific bag only.
        if bag != NULL_BAG && bag != BAG_MAIN {
            let Some((bag_proto, _guid)) = self.bag_at(&player.deferred.inventory, bag) else {
                return Err(ERR_ITEM_NOT_FOUND);
            };
            let cap = bag_proto.container_slots;
            let mut remaining = count;
            for s in 0..cap {
                let slot8 = u8::try_from(s).unwrap_or(0xff);
                {
                    let left = self.can_store_specific(
                        player, &mut dest, bag, slot8, proto, remaining, swap, src,
                    )?;
                    remaining = left;
                    if remaining == 0 {
                        return Ok(dest);
                    }
                }
            }
            return Err(ERR_BAG_FULL);
        }
        // Merge phase: keyring, backpack, specialized
        // bags, general bags (fork loop order).
        let specials: Vec<u8> = (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END)
            .filter(|&b| {
                self.bag_at(&player.deferred.inventory, b)
                    .is_some_and(|(p, _)| !(p.class == 1 && p.subclass == 0))
            })
            .collect();
        let generals: Vec<u8> = (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END)
            .filter(|&b| !specials.contains(&b))
            .collect();
        let mut scan_slots: Vec<InvPos> = Vec::new();
        for s in KEYRING_SLOT_START..KEYRING_SLOT_END {
            scan_slots.push(InvPos::new(BAG_MAIN, s));
        }
        for s in INVENTORY_SLOT_ITEM_START..INVENTORY_SLOT_ITEM_END {
            scan_slots.push(InvPos::new(BAG_MAIN, s));
        }
        if proto.bag_family != 0 {
            for &b in &specials {
                if let Some((bp, _)) = self.bag_at(&player.deferred.inventory, b) {
                    for s in 0..bp.container_slots {
                        scan_slots.push(InvPos::new(b, u8::try_from(s).unwrap_or(0xff)));
                    }
                }
            }
        }
        for &b in &generals {
            if let Some((bp, _)) = self.bag_at(&player.deferred.inventory, b) {
                for s in 0..bp.container_slots {
                    scan_slots.push(InvPos::new(b, u8::try_from(s).unwrap_or(0xff)));
                }
            }
        }
        for pos in &scan_slots {
            if count == 0 {
                return Ok(dest);
            }
            // Merge phase skips empty slots (fork
            // `merge=true` arm).
            if row_index_at(&player.deferred.inventory, *pos).is_none() {
                continue;
            }
            if let Ok(left) = self.can_store_specific(
                player, &mut dest, pos.bag, pos.slot, proto, count, swap, src,
            ) {
                count = left;
            }
        }
        if count == 0 {
            return Ok(dest);
        }
        // Free phase: keyring (keys), specialized
        // bags (family), backpack, general bags.
        let mut free_scan: Vec<InvPos> = Vec::new();
        for s in KEYRING_SLOT_START..KEYRING_SLOT_END {
            free_scan.push(InvPos::new(BAG_MAIN, s));
        }
        if proto.bag_family != 0 {
            for &b in &specials {
                if let Some((bp, _)) = self.bag_at(&player.deferred.inventory, b) {
                    for s in 0..bp.container_slots {
                        free_scan.push(InvPos::new(b, u8::try_from(s).unwrap_or(0xff)));
                    }
                }
            }
        }
        for s in INVENTORY_SLOT_ITEM_START..INVENTORY_SLOT_ITEM_END {
            free_scan.push(InvPos::new(BAG_MAIN, s));
        }
        for &b in &generals {
            if let Some((bp, _)) = self.bag_at(&player.deferred.inventory, b) {
                for s in 0..bp.container_slots {
                    free_scan.push(InvPos::new(b, u8::try_from(s).unwrap_or(0xff)));
                }
            }
        }
        let mut err = ERR_BAG_FULL;
        for pos in &free_scan {
            if count == 0 {
                return Ok(dest);
            }
            if row_index_at(&player.deferred.inventory, *pos).is_some() {
                continue;
            }
            match self.can_store_specific(
                player, &mut dest, pos.bag, pos.slot, proto, count, swap, src,
            ) {
                Ok(left) => count = left,
                Err(e) => err = e,
            }
        }
        if count == 0 {
            Ok(dest)
        } else if proto.bag_family == BAG_FAMILY_KEYS {
            Err(ERR_BAG_FULL)
        } else {
            Err(err)
        }
    }

    /// Bank auto-store planner (`CanBankItem`
    /// inventory arm over bank slots).
    #[allow(clippy::too_many_arguments)] // fork CanBankItem signature
    fn can_bank_auto(
        &self,
        player: &Player,
        bag: u8,
        slot: u8,
        proto: &ItemPrototype,
        count: u32,
        swap: bool,
        src: Option<&InventoryRow>,
    ) -> Result<Vec<StoreDest>, InvResult> {
        let mut dest: Vec<StoreDest> = Vec::new();
        let res = Self::can_take_more_similar(
            &player.deferred.inventory,
            proto,
            count,
            src.map_or(0, |s| s.item.item_guid),
        );
        if res != ERR_OK {
            return Err(res);
        }
        if slot != NULL_SLOT {
            let bag = if bag == NULL_BAG { BAG_MAIN } else { bag };
            return match self
                .can_store_specific(player, &mut dest, bag, slot, proto, count, swap, src)
            {
                Ok(0) => Ok(dest),
                Ok(_) => Err(ERR_BANK_FULL),
                Err(e) => Err(e),
            };
        }
        let mut remaining = count;
        let mut slots: Vec<InvPos> = (BANK_SLOT_ITEM_START..BANK_SLOT_ITEM_END)
            .map(|s| InvPos::new(BAG_MAIN, s))
            .collect();
        for b in BANK_SLOT_BAG_START..BANK_SLOT_BAG_END {
            if let Some((bp, _)) = self.bag_at(&player.deferred.inventory, b) {
                for s in 0..bp.container_slots {
                    slots.push(InvPos::new(b, u8::try_from(s).unwrap_or(0xff)));
                }
            }
        }
        // Merge pass, then empties (specific-slot
        // helper already prefers matching stacks).
        for pos in &slots {
            if remaining == 0 {
                return Ok(dest);
            }
            if let Ok(left) = self.can_store_specific(
                player, &mut dest, pos.bag, pos.slot, proto, remaining, swap, src,
            ) {
                remaining = left;
            }
        }
        if remaining == 0 {
            Ok(dest)
        } else {
            Err(ERR_BANK_FULL)
        }
    }

    /// `IsTwoHandUsed` over rows.
    fn two_hand_used(&self, rows: &[InventoryRow]) -> bool {
        rows.iter()
            .find(|r| r.bag == 0 && r.slot == SLOT_MAIN_HAND)
            .and_then(|r| self.inv_proto(r.item.item_id))
            .is_some_and(|p| p.inventory_type == 17)
    }

    /// `CanDualWield`: dual-wield spell 674.
    fn can_dual_wield(player: &Player) -> bool {
        player.deferred.spells.iter().any(|s| s.spell == 674)
    }

    /// `FindEquipSlot`.
    fn find_equip_slot(player: &Player, proto: &ItemPrototype, slot: u8, swap: bool) -> Option<u8> {
        let rows = &player.deferred.inventory;
        let at = |s: u8| rows.iter().any(|r| r.bag == 0 && r.slot == s);
        let slots = allowed_equip_slots(
            proto,
            u32::from(player.unit.class()),
            Self::can_dual_wield(player),
        );
        let candidates: Vec<u8> = slots.into_iter().flatten().collect();
        if candidates.is_empty() {
            return None;
        }
        if slot != NULL_SLOT {
            return candidates.contains(&slot).then_some(slot);
        }
        if !swap {
            // Preferred empty slot (weapon specials
            // collapse to first-empty in the row
            // model: fork prefers mainhand for 1H,
            // empty hand, and trinket slot 2 falls
            // back to slot 1).
            for s in &candidates {
                if !at(*s) {
                    return Some(*s);
                }
            }
            return None;
        }
        // Swap: first slot (fork honors expected
        // unique-equip replacement and the
        // finger/trinket slot-2-last ordering via
        // the allowed-slot order).
        if proto.flags & ITEM_FLAG_UNIQUE_EQUIPPED != 0 {
            for s in &candidates {
                if rows
                    .iter()
                    .any(|r| r.bag == 0 && r.slot == *s && r.item.item_id == proto.item_id)
                {
                    return Some(*s);
                }
            }
        }
        candidates.first().copied()
    }

    /// `CanEquipItem(slot, dest, proto, src, swap,
    /// not_loading=true)`. Returns the equipment
    /// slot on success.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn can_equip_item(
        &self,
        player: &Player,
        slot: u8,
        proto: &ItemPrototype,
        src: Option<&InventoryRow>,
        swap: bool,
    ) -> Result<u8, InvResult> {
        let rows = &player.deferred.inventory;
        // Fork order (Player.cpp:9631-9678): stunned /
        // combat gates, FindEquipSlot, CanUseItem,
        // occupied-slot, then unique-equipped.
        // UNMODELED: CanTakeMoreSimilarItems precedes
        // these in the fork.
        if player.unit.is_in_combat() && proto.class == 2 && proto.subclass == 10 {
            // Staves cannot swap in combat (fork quirk).
            // (Weapon-class arm below covers the rest.)
        }
        if player.unit.is_in_combat() && !can_change_equip_in_combat(proto) {
            return Err(ERR_NOT_IN_COMBAT);
        }
        if player
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::UNIT_FIELD_FLAGS, 0x0004_0000)
        {
            // UNIT_FLAG_STUNNED
            return Err(ERR_YOU_ARE_STUNNED);
        }
        let Some(dest) = Self::find_equip_slot(player, proto, slot, swap) else {
            return Err(ERR_ITEM_CANT_BE_EQUIPPED);
        };
        let res = self.can_use_item(player, proto);
        if res != ERR_OK {
            return Err(res);
        }
        let res = Self::can_equip_unique(rows, proto, slot);
        if res != ERR_OK {
            return Err(res);
        }
        // 2H semantics.
        let is_2h = proto.inventory_type == 17;
        if dest == SLOT_OFF_HAND && self.two_hand_used(rows) {
            return Err(ERR_TWOHANDED_EQUIPPED);
        }
        if is_2h {
            let offhand = rows.iter().find(|r| r.bag == 0 && r.slot == SLOT_OFF_HAND);
            if let Some(off) = offhand {
                if !swap {
                    return Err(ERR_TWOHANDED_EQUIPPED);
                }
                let res = self.can_unequip_item(player, SLOT_OFF_HAND, false);
                if res != ERR_OK {
                    return Err(res);
                }
                let off_proto = self
                    .inv_proto(off.item.item_id)
                    .ok_or(ERR_INTERNAL_BAG_ERROR)?;
                if self
                    .can_store_auto(player, NULL_BAG, NULL_SLOT, off_proto, 1, false, None)
                    .is_err()
                {
                    return Err(ERR_INVENTORY_FULL);
                }
            }
        }
        if dest == SLOT_OFF_HAND && !Self::can_dual_wield(player) {
            return Err(ERR_CANT_DUAL_WIELD);
        }
        // Disarmed weapons block (dynflags unmodeled:
        // no disarm state in the row model).
        let _ = src;
        Ok(dest)
    }

    /// `CanUnequipItem(pos, swap)`.
    fn can_unequip_item(&self, player: &Player, slot: u8, swap: bool) -> InvResult {
        let rows = &player.deferred.inventory;
        let Some(item) = rows.iter().find(|r| r.bag == 0 && r.slot == slot) else {
            return ERR_OK;
        };
        let Some(proto) = self.inv_proto(item.item.item_id) else {
            return ERR_OK;
        };
        if !player.unit.is_alive() {
            return ERR_YOU_ARE_DEAD;
        }
        if player.unit.is_in_combat() && !can_change_equip_in_combat(proto) {
            return ERR_NOT_IN_COMBAT;
        }
        if !swap && proto.class == 1 && !bag_is_empty(rows, item.item.item_guid) {
            return ERR_NONEMPTY_BAG_OVER_OTHER_BAG;
        }
        ERR_OK
    }
}

impl Map {
    /// Owner viewer for item blocks.
    fn item_owner_viewer() -> Viewer {
        Viewer {
            is_self: false,
            is_game_master: false,
            has_debug_target_cheat: false,
            item_owner_is_target: true,
            owner_or_charmer_is_target: false,
            can_see_special_info: false,
            group_only_granted: false,
            can_see_health: false,
            show_health_values: false,
            strip_ffa_pvp: false,
        }
    }

    fn item_facts(&self, item_id: u32) -> ItemProtoFacts {
        self.inv_proto(item_id)
            .map_or_else(ItemProtoFacts::default, |proto| ItemProtoFacts {
                max_durability: proto.max_durability,
                container_slots: proto.container_slots,
            })
    }

    fn contents_of(rows: &[InventoryRow], bag_guid: u32) -> Vec<(u8, ObjectGuid)> {
        rows.iter()
            .filter(|r| r.bag == bag_guid)
            .map(|r| (r.slot, item_guid(r.item.item_guid)))
            .collect()
    }

    /// Snapshot-diff client sync after a row
    /// mutation: creates for new GUIDs, values
    /// blocks for changed rows (pre-base re-filled
    /// with post values), destroys for removed.
    /// Also recomputes visible equipment and, when
    /// an equipment slot changed hands, derived
    /// stats + weapon fields.
    pub(crate) fn sync_inventory(&mut self, guid: ObjectGuid, pre: &[InventoryRow]) {
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let post = &entry.player.deferred.inventory;
        let viewer = Self::item_owner_viewer();
        let view = ObjectView::default();
        let now_ms = self.now_ms;
        let mut data = UpdateData::new();
        let mut effects = BuildEffects::default();
        let mut destroys: Vec<WorldPacket> = Vec::new();
        let mut equip_touched = false;
        for row in post {
            let facts = self.item_facts(row.item.item_id);
            let contents = Self::contents_of(post, row.item.item_guid);
            match pre.iter().find(|r| r.item.item_guid == row.item.item_guid) {
                None => {
                    let base = item_object(row, guid, facts, &contents);
                    build_create_block(&base, &view, &viewer, &mut data, now_ms, &mut effects);
                    if row.bag == 0 && row.slot < EQUIPMENT_SLOT_END {
                        equip_touched = true;
                    }
                }
                Some(old) if old != row => {
                    let old_contents = Self::contents_of(pre, old.item.item_guid);
                    let mut base = item_object(old, guid, facts, &old_contents);
                    base.values_mut().clear_update_mask();
                    fill_item_values(&mut base, row, guid, facts, &contents);
                    build_values_block(&base, &view, &viewer, &mut data, &mut effects);
                    if (row.bag == 0 && row.slot < EQUIPMENT_SLOT_END)
                        || (old.bag == 0 && old.slot < EQUIPMENT_SLOT_END)
                    {
                        equip_touched = true;
                    }
                }
                Some(_) => {}
            }
        }
        for row in pre {
            if post.iter().all(|r| r.item.item_guid != row.item.item_guid) {
                let base = item_object(
                    row,
                    guid,
                    self.item_facts(row.item.item_id),
                    &Self::contents_of(pre, row.item.item_guid),
                );
                destroys.push(destroy_packet(&base));
                if row.bag == 0 && row.slot < EQUIPMENT_SLOT_END {
                    equip_touched = true;
                }
            }
        }
        self.send_update_data(guid, &mut data);
        for packet in destroys {
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.refresh_visible_equipment();
            if equip_touched {
                entry
                    .player
                    .refresh_equipped_bonuses(Some(&self.stores.item_prototypes));
            }
        }
        if equip_touched {
            self.refresh_weapon_damage_fields(guid);
        }
    }

    /// Move a row to a position (row identity =
    /// item guid).
    fn move_row(rows: &mut [InventoryRow], item_guid_low: u32, dst: InvPos, bag_guid_of: u32) {
        if let Some(row) = rows.iter_mut().find(|r| r.item.item_guid == item_guid_low) {
            if dst.bag == BAG_MAIN {
                row.bag = 0;
                row.slot = dst.slot;
            } else {
                row.bag = bag_guid_of;
                row.slot = dst.slot;
            }
        }
    }

    /// `StoreItem(dest, item, update)`: apply a
    /// store plan for `count` of `proto` sourced
    /// from `src` (an existing row being moved/
    /// merged) — new stacks clone the source row
    /// with a fresh guid (fork `CloneItem`).
    pub(crate) fn store_apply(&mut self, guid: ObjectGuid, dest: &[StoreDest], src: &InventoryRow) {
        let src_guid = src.item.item_guid;
        let mut first_free_used = false;
        for d in dest {
            let Some(entry) = self.players.get_mut(&guid) else {
                return;
            };
            let rows = &mut entry.player.deferred.inventory;
            let bag_guid = if d.pos.bag == BAG_MAIN {
                0
            } else {
                rows.iter()
                    .find(|r| r.bag == 0 && r.slot == d.pos.bag)
                    .map_or(0, |r| r.item.item_guid)
            };
            if let Some(idx) = row_index_at(rows, d.pos) {
                // Merge or same-slot count write.
                rows[idx].item.count = d.count;
            } else if !first_free_used {
                // The source row itself lands here
                // (still in the vec, or detached by
                // the caller beforehand).
                first_free_used = true;
                if rows.iter().any(|r| r.item.item_guid == src_guid) {
                    Self::move_row(rows, src_guid, d.pos, bag_guid);
                    if let Some(row) = rows.iter_mut().find(|r| r.item.item_guid == src_guid) {
                        row.item.count = d.count;
                    }
                } else {
                    let mut placed = src.clone();
                    placed.item.count = d.count;
                    placed.bag = bag_guid;
                    placed.slot = d.pos.slot;
                    rows.push(placed);
                }
            } else {
                let mut clone = src.clone();
                clone.item.item_guid = self.stores.generate_item_guid();
                clone.item.count = d.count;
                clone.bag = bag_guid;
                clone.slot = d.pos.slot;
                rows.push(clone);
            }
        }
    }

    /// Remove `count` from the row at `pos`
    /// (`DestroyItemCount` single-pos core); the
    /// row dies at 0. Returns false if missing.
    fn destroy_at(&mut self, guid: ObjectGuid, pos: InvPos, count: u32) -> bool {
        let Some(entry) = self.players.get_mut(&guid) else {
            return false;
        };
        let rows = &mut entry.player.deferred.inventory;
        let Some(idx) = row_index_at(rows, pos) else {
            return false;
        };
        if rows[idx].item.count <= count {
            rows.remove(idx);
        } else {
            rows[idx].item.count -= count;
        }
        true
    }

    /// `AutoUnequipOffhandIfNeed` (+ mail fallback
    /// unmodeled: full inventory leaves the offhand
    /// equipped and the caller errs).
    pub(crate) fn auto_unequip_offhand(&mut self, guid: ObjectGuid) -> bool {
        let Some(entry) = self.players.get(&guid) else {
            return false;
        };
        if !self.two_hand_used(&entry.player.deferred.inventory) {
            return true;
        }
        let Some(off) = entry
            .player
            .deferred
            .inventory
            .iter()
            .find(|r| r.bag == 0 && r.slot == SLOT_OFF_HAND)
            .cloned()
        else {
            return true;
        };
        let Some(proto) = self.inv_proto(off.item.item_id).cloned() else {
            return false;
        };
        let res = self.can_unequip_item(&entry.player, SLOT_OFF_HAND, false);
        if res != ERR_OK {
            return false;
        }
        let player = &self.players.get(&guid).expect("player").player;
        let Ok(dest) = self.can_store_auto(player, NULL_BAG, NULL_SLOT, &proto, 1, false, None)
        else {
            return false;
        };
        self.store_apply(guid, &dest, &off);
        true
    }
}

impl Map {
    /// `EquipItem` row effect: move + `BoP` binding.
    pub(crate) fn equip_apply(
        rows: &mut [InventoryRow],
        item_guid_low: u32,
        dest_slot: u8,
        proto: &ItemPrototype,
    ) {
        if let Some(row) = rows.iter_mut().find(|r| r.item.item_guid == item_guid_low) {
            row.bag = 0;
            row.slot = dest_slot;
            if proto.bonding == 2 {
                row.item.flags |= ITEM_DYNFLAG_BINDED;
            }
        }
    }

    pub(crate) fn snapshot(&self, guid: ObjectGuid) -> Vec<InventoryRow> {
        self.players
            .get(&guid)
            .map(|e| e.player.deferred.inventory.clone())
            .unwrap_or_default()
    }

    fn detach(&mut self, guid: ObjectGuid, pos: InvPos) -> Option<InventoryRow> {
        let entry = self.players.get_mut(&guid)?;
        let rows = &mut entry.player.deferred.inventory;
        let idx = row_index_at(rows, pos)?;
        Some(rows.remove(idx))
    }

    /// `HandleDestroyItemOpcode` + `DestroyItemCount`
    /// at one position.
    pub fn destroy_item(&mut self, guid: ObjectGuid, pos: InvPos, count: u8) {
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        if pos.is_equipment() || pos.is_bag_slot() {
            let msg = self.can_unequip_item(player, pos.slot, false);
            if msg != ERR_OK {
                let item = row_index_at(&player.deferred.inventory, pos)
                    .map_or(ObjectGuid::EMPTY, |i| {
                        item_guid(player.deferred.inventory[i].item.item_guid)
                    });
                self.send_inventory_error(guid, msg, item, ObjectGuid::EMPTY, 0);
                return;
            }
        }
        let Some(idx) = row_index_at(&player.deferred.inventory, pos) else {
            self.send_inventory_error(
                guid,
                ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let row = player.deferred.inventory[idx].clone();
        let Some(proto) = self.inv_proto(row.item.item_id).cloned() else {
            return;
        };
        if proto.flags & ITEM_FLAG_INDESTRUCTIBLE != 0 {
            self.send_inventory_error(
                guid,
                ERR_CANT_DROP_SOULBOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        let pre = self.snapshot(guid);
        let n = if count == 0 {
            row.item.count
        } else {
            u32::from(count)
        };
        self.destroy_at(guid, pos, n);
        self.sync_inventory(guid, &pre);
    }

    /// `HandleSplitItemOpcode` + `SplitItem`
    /// (trade-window arm unmodeled: no trade yet).
    /// # Panics
    ///
    /// Panics only if the acting player vanishes between
    /// the entry lookup and application (unreachable on
    /// the single map thread).
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::similar_names)] // fork src/dst dest locals
    pub fn split_item(&mut self, guid: ObjectGuid, src: InvPos, dst: InvPos, count: u8) {
        if src == dst || count == 0 {
            return;
        }
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        if !self.is_valid_pos(player, src) {
            self.send_inventory_error(
                guid,
                ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        // Autostore positions pass the non-explicit check.
        let dst_ok = (dst.slot == NULL_SLOT
            && (dst.bag == BAG_MAIN
                || (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&dst.bag)
                || (BANK_SLOT_BAG_START..BANK_SLOT_BAG_END).contains(&dst.bag)))
            || self.is_valid_pos(player, dst);
        if !dst_ok {
            self.send_inventory_error(
                guid,
                ERR_DOESNT_GO_TO_SLOT,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        let Some(src_row) = row_index_at(&player.deferred.inventory, src)
            .map(|i| player.deferred.inventory[i].clone())
        else {
            self.send_inventory_error(
                guid,
                ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let src_item = item_guid(src_row.item.item_guid);
        let count = u32::from(count);
        if src_row.item.generated_loot != 0 {
            self.send_inventory_error(
                guid,
                ERR_COULDNT_SPLIT_ITEMS,
                src_item,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if src_row.item.count == count {
            self.send_inventory_error(
                guid,
                ERR_COULDNT_SPLIT_ITEMS,
                src_item,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if src_row.item.count < count {
            self.send_inventory_error(guid, ERR_TOO_MANY_TO_SPLIT, src_item, ObjectGuid::EMPTY, 0);
            return;
        }
        let Some(proto) = self.inv_proto(src_row.item.item_id).cloned() else {
            self.send_inventory_error(guid, ERR_ITEM_NOT_FOUND, src_item, ObjectGuid::EMPTY, 0);
            return;
        };
        // Clone with fresh guid; src count pre-adjusted
        // so unique-count checks see the post state.
        let pre = self.snapshot(guid);
        let mut clone = src_row.clone();
        clone.item.item_guid = self.stores.generate_item_guid();
        let clone_guid = clone.item.item_guid;
        clone.item.count = count;
        {
            let entry = self.players.get_mut(&guid).expect("player");
            if let Some(row) = entry
                .player
                .deferred
                .inventory
                .iter_mut()
                .find(|r| r.item.item_guid == src_row.item.item_guid)
            {
                row.item.count -= count;
            }
        }
        let player = &self.players.get(&guid).expect("player").player;
        let placed = if dst.is_equipment() {
            match self.can_equip_item(player, dst.slot, &proto, Some(&clone), false) {
                Ok(e_dest) => {
                    clone.bag = 0;
                    clone.slot = e_dest;
                    if proto.bonding == 2 {
                        clone.item.flags |= ITEM_DYNFLAG_BINDED;
                    }
                    self.players
                        .get_mut(&guid)
                        .expect("player")
                        .player
                        .deferred
                        .inventory
                        .push(clone);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else if dst.is_bank() {
            match self.can_bank_auto(
                player,
                dst.bag,
                dst.slot,
                &proto,
                count,
                false,
                Some(&clone),
            ) {
                Ok(dest) => {
                    self.store_apply(guid, &dest, &clone);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else {
            match self.can_store_auto(
                player,
                dst.bag,
                dst.slot,
                &proto,
                count,
                false,
                Some(&clone),
            ) {
                Ok(dest) => {
                    self.store_apply(guid, &dest, &clone);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        match placed {
            Ok(()) => {
                if dst.is_equipment() {
                    self.auto_unequip_offhand(guid);
                }
                self.sync_inventory(guid, &pre);
            }
            Err(e) => {
                // Restore the source count; the clone
                // guid is burned (fork deletes it too).
                let entry = self.players.get_mut(&guid).expect("player");
                entry
                    .player
                    .deferred
                    .inventory
                    .retain(|r| r.item.item_guid != clone_guid);
                if let Some(row) = entry
                    .player
                    .deferred
                    .inventory
                    .iter_mut()
                    .find(|r| r.item.item_guid == src_row.item.item_guid)
                {
                    row.item.count += count;
                }
                self.send_inventory_error(
                    guid,
                    e,
                    src_item,
                    ObjectGuid::EMPTY,
                    proto.required_level,
                );
            }
        }
    }

    /// `HandleSwapInvItemOpcode` (bag-0 slots).
    pub fn swap_inv_item(&mut self, guid: ObjectGuid, src_slot: u8, dst_slot: u8) {
        if src_slot == dst_slot {
            return;
        }
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        let src = InvPos::new(BAG_MAIN, src_slot);
        let dst = InvPos::new(BAG_MAIN, dst_slot);
        if !self.is_valid_pos(player, src) {
            self.send_inventory_error(
                guid,
                ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if !self.is_valid_pos(player, dst) {
            self.send_inventory_error(
                guid,
                ERR_DOESNT_GO_TO_SLOT,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if (src.is_bank() || dst.is_bank()) && !self.can_use_bank(player) {
            self.send_inventory_error(
                guid,
                ERR_TOO_FAR_AWAY_FROM_BANK,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        self.swap_item(guid, src, dst);
    }

    /// `HandleSwapItem` (arbitrary positions).
    pub fn swap_item_packet(&mut self, guid: ObjectGuid, src: InvPos, dst: InvPos) {
        if src == dst {
            return;
        }
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        if !self.is_valid_pos(player, src) {
            self.send_inventory_error(
                guid,
                ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if !self.is_valid_pos(player, dst) {
            self.send_inventory_error(
                guid,
                ERR_DOESNT_GO_TO_SLOT,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        if (src.is_bank() || dst.is_bank()) && !self.can_use_bank(player) {
            self.send_inventory_error(
                guid,
                ERR_TOO_FAR_AWAY_FROM_BANK,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        }
        self.swap_item(guid, src, dst);
    }

    /// `Player::SwapItem`.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::similar_names)] // fork src/dst dest locals
    fn swap_item(&mut self, guid: ObjectGuid, src: InvPos, dst: InvPos) {
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        let rows = &player.deferred.inventory;
        let Some(src_row) = row_index_at(rows, src).map(|i| rows[i].clone()) else {
            return;
        };
        let dst_row = row_index_at(rows, dst).map(|i| rows[i].clone());
        let src_item = item_guid(src_row.item.item_guid);
        let dst_item = dst_row
            .as_ref()
            .map_or(ObjectGuid::EMPTY, |r| item_guid(r.item.item_guid));
        if !player.unit.is_alive() && (!src.is_inventory() || !dst.is_inventory()) {
            self.send_inventory_error(guid, ERR_YOU_ARE_DEAD, src_item, dst_item, 0);
            return;
        }
        let src_proto = self.inv_proto(src_row.item.item_id).cloned();
        let dst_proto = dst_row
            .as_ref()
            .and_then(|r| self.inv_proto(r.item.item_id).cloned());
        let src_is_bag = src_proto.as_ref().is_some_and(|p| p.container_slots > 0);
        let dst_is_bag = dst_proto.as_ref().is_some_and(|p| p.container_slots > 0);
        // SRC unequip check.
        if src.is_equipment() || src.is_bag_slot() {
            let swap_ok = !src.is_bag_slot()
                || dst.is_bag_slot()
                || dst_row
                    .as_ref()
                    .is_some_and(|r| dst_is_bag && bag_is_empty(rows, r.item.item_guid));
            let msg = self.can_unequip_item(player, src.slot, swap_ok);
            if msg != ERR_OK {
                self.send_inventory_error(guid, msg, src_item, dst_item, 0);
                return;
            }
        }
        // Bag into itself guards.
        if src.is_bag_slot() && src.slot == dst.bag {
            self.send_inventory_error(guid, ERR_NONEMPTY_BAG_OVER_OTHER_BAG, src_item, dst_item, 0);
            return;
        }
        if dst.is_bag_slot() && dst.slot == src.bag {
            self.send_inventory_error(guid, ERR_NONEMPTY_BAG_OVER_OTHER_BAG, dst_item, src_item, 0);
            return;
        }
        // DST unequip check.
        if dst_row.is_some() && (dst.is_equipment() || dst.is_bag_slot()) {
            let swap_ok = !dst.is_bag_slot()
                || src.is_bag_slot()
                || (src_is_bag && bag_is_empty(rows, src_row.item.item_guid));
            let msg = self.can_unequip_item(player, dst.slot, swap_ok);
            if msg != ERR_OK {
                self.send_inventory_error(guid, msg, src_item, dst_item, 0);
                return;
            }
        }
        let Some(src_proto) = src_proto else {
            return;
        };
        let pre = self.snapshot(guid);

        // Move case (empty destination).
        let Some(dst_row) = dst_row else {
            if dst.is_equipment() {
                match self.can_equip_item(player, dst.slot, &src_proto, Some(&src_row), false) {
                    Ok(e_dest) => {
                        let owned = self.detach(guid, src).expect("src");
                        let entry = self.players.get_mut(&guid).expect("player");
                        entry.player.deferred.inventory.push(owned);
                        Self::equip_apply(
                            &mut entry.player.deferred.inventory,
                            src_row.item.item_guid,
                            e_dest,
                            &src_proto,
                        );
                        self.auto_unequip_offhand(guid);
                        self.sync_inventory(guid, &pre);
                    }
                    Err(e) => self.send_inventory_error(
                        guid,
                        e,
                        src_item,
                        ObjectGuid::EMPTY,
                        src_proto.required_level,
                    ),
                }
            } else if dst.is_bank() {
                match self.can_bank_auto(
                    player,
                    dst.bag,
                    dst.slot,
                    &src_proto,
                    src_row.item.count,
                    false,
                    Some(&src_row),
                ) {
                    Ok(dest) => {
                        let owned = self.detach(guid, src).expect("src");
                        self.store_apply(guid, &dest, &owned);
                        self.sync_inventory(guid, &pre);
                    }
                    Err(e) => self.send_inventory_error(guid, e, src_item, ObjectGuid::EMPTY, 0),
                }
            } else {
                match self.can_store_auto(
                    player,
                    dst.bag,
                    dst.slot,
                    &src_proto,
                    src_row.item.count,
                    false,
                    Some(&src_row),
                ) {
                    Ok(dest) => {
                        let owned = self.detach(guid, src).expect("src");
                        self.store_apply(guid, &dest, &owned);
                        self.sync_inventory(guid, &pre);
                    }
                    Err(e) => self.send_inventory_error(guid, e, src_item, ObjectGuid::EMPTY, 0),
                }
            }
            return;
        };
        let Some(dst_proto) = dst_proto else {
            return;
        };

        // Merge/fill attempt (non-bags).
        if !src_is_bag && !dst_is_bag {
            let merge_check: Result<Vec<StoreDest>, InvResult> = if dst.is_bank() {
                self.can_bank_auto(
                    player,
                    dst.bag,
                    dst.slot,
                    &src_proto,
                    src_row.item.count,
                    false,
                    Some(&src_row),
                )
            } else if dst.is_equipment() {
                match self.can_equip_item(player, dst.slot, &src_proto, Some(&src_row), false) {
                    Ok(e_dest) => Ok(vec![StoreDest {
                        pos: InvPos::new(BAG_MAIN, e_dest),
                        count: src_row.item.count + dst_row.item.count,
                    }]),
                    Err(e) => Err(e),
                }
            } else {
                self.can_store_auto(
                    player,
                    dst.bag,
                    dst.slot,
                    &src_proto,
                    src_row.item.count,
                    false,
                    Some(&src_row),
                )
            };
            if let Ok(dest) = merge_check {
                if src_row.item.count + dst_row.item.count <= src_proto.stackable {
                    // Full merge: dst (or equip slot)
                    // keeps ITS row identity in fork
                    // StoreItem semantics — the merged
                    // count lands on the dest plan.
                    let owned = self.detach(guid, src).expect("src");
                    if dst.is_equipment() {
                        let e_dest = dest[0].pos.slot;
                        // Equipped stack absorbs the
                        // source clone (fork EquipItem
                        // merges into equipped item).
                        self.destroy_at(guid, InvPos::new(BAG_MAIN, e_dest), 0);
                        let entry = self.players.get_mut(&guid).expect("player");
                        let mut merged = owned;
                        merged.item.count = dest[0].count;
                        entry.player.deferred.inventory.push(merged);
                        Self::equip_apply(
                            &mut entry.player.deferred.inventory,
                            src_row.item.item_guid,
                            e_dest,
                            &src_proto,
                        );
                        self.auto_unequip_offhand(guid);
                    } else {
                        self.store_apply(guid, &dest, &owned);
                    }
                    self.sync_inventory(guid, &pre);
                } else {
                    // Partial fill: counts only.
                    let entry = self.players.get_mut(&guid).expect("player");
                    let rows = &mut entry.player.deferred.inventory;
                    if let Some(r) = rows
                        .iter_mut()
                        .find(|r| r.item.item_guid == src_row.item.item_guid)
                    {
                        r.item.count =
                            src_row.item.count + dst_row.item.count - src_proto.stackable;
                    }
                    if let Some(r) = rows
                        .iter_mut()
                        .find(|r| r.item.item_guid == dst_row.item.item_guid)
                    {
                        r.item.count = src_proto.stackable;
                    }
                    self.sync_inventory(guid, &pre);
                }
                return;
            }
        }

        // Real swap: check both directions.
        let plan = |this: &Self,
                    player: &Player,
                    to: InvPos,
                    proto: &ItemPrototype,
                    row: &InventoryRow|
         -> Result<DestPlan, InvResult> {
            if to.is_equipment() {
                let e = this.can_equip_item(player, to.slot, proto, Some(row), true)?;
                let res = this.can_unequip_item(player, e, true);
                if res != ERR_OK {
                    return Err(res);
                }
                Ok(DestPlan::Equip(e))
            } else if to.is_bank() {
                this.can_bank_auto(
                    player,
                    to.bag,
                    to.slot,
                    proto,
                    row.item.count,
                    true,
                    Some(row),
                )
                .map(DestPlan::Store)
            } else {
                this.can_store_auto(
                    player,
                    to.bag,
                    to.slot,
                    proto,
                    row.item.count,
                    true,
                    Some(row),
                )
                .map(DestPlan::Store)
            }
        };
        let player_ref = &self.players.get(&guid).expect("player").player;
        let s_dest = match plan(self, player_ref, dst, &src_proto, &src_row) {
            Ok(p) => p,
            Err(e) => {
                self.send_inventory_error(guid, e, src_item, dst_item, src_proto.required_level);
                return;
            }
        };
        let s_dest2 = match plan(self, player_ref, src, &dst_proto, &dst_row) {
            Ok(p) => p,
            Err(e) => {
                self.send_inventory_error(guid, e, dst_item, src_item, dst_proto.required_level);
                return;
            }
        };

        // Bag-contents exchange: an empty bag NOT in
        // a bag position receives the other bag's
        // contents before the shells swap.
        if src_is_bag && dst_is_bag {
            let (empty, full) = if bag_is_empty(&pre, src_row.item.item_guid) && !src.is_bag_slot()
            {
                (Some(&src_row), Some(&dst_row))
            } else if bag_is_empty(&pre, dst_row.item.item_guid) && !dst.is_bag_slot() {
                (Some(&dst_row), Some(&src_row))
            } else {
                (None, None)
            };
            if let (Some(empty), Some(full)) = (empty, full) {
                let empty_proto = self.inv_proto(empty.item.item_id).cloned().expect("proto");
                let contents: Vec<InventoryRow> = pre
                    .iter()
                    .filter(|r| r.bag == full.item.item_guid)
                    .cloned()
                    .collect();
                for c in &contents {
                    let fits = self
                        .inv_proto(c.item.item_id)
                        .is_some_and(|p| item_can_go_into_bag(p, &empty_proto));
                    if !fits {
                        self.send_inventory_error(
                            guid,
                            ERR_NONEMPTY_BAG_OVER_OTHER_BAG,
                            src_item,
                            dst_item,
                            0,
                        );
                        return;
                    }
                }
                if contents.len() as u32 > empty_proto.container_slots {
                    self.send_inventory_error(
                        guid,
                        ERR_ITEMS_CANT_BE_SWAPPED,
                        src_item,
                        dst_item,
                        0,
                    );
                    return;
                }
                let entry = self.players.get_mut(&guid).expect("player");
                let rows = &mut entry.player.deferred.inventory;
                let mut next_slot = 0u8;
                for c in &contents {
                    if let Some(r) = rows
                        .iter_mut()
                        .find(|r| r.item.item_guid == c.item.item_guid)
                    {
                        r.bag = empty.item.item_guid;
                        r.slot = next_slot;
                        next_slot += 1;
                    }
                }
            }
        }

        // Apply: detach both, place per plans.
        let src_owned = self.detach(guid, src).expect("src");
        let dst_owned = self.detach(guid, dst).expect("dst");
        let apply =
            |this: &mut Self, plan: &DestPlan, owned: InventoryRow, proto: &ItemPrototype| {
                match plan {
                    DestPlan::Store(dest) => this.store_apply(guid, dest, &owned),
                    DestPlan::Equip(e) => {
                        let entry = this.players.get_mut(&guid).expect("player");
                        let g = owned.item.item_guid;
                        entry.player.deferred.inventory.push(owned);
                        Self::equip_apply(&mut entry.player.deferred.inventory, g, *e, proto);
                    }
                }
            };
        apply(self, &s_dest, src_owned, &src_proto);
        apply(self, &s_dest2, dst_owned, &dst_proto);
        self.auto_unequip_offhand(guid);
        self.sync_inventory(guid, &pre);
    }

    /// `HandleAutoEquipItemOpcode`.
    /// # Panics
    ///
    /// Panics only if the acting player vanishes between
    /// the entry lookup and application (unreachable on
    /// the single map thread).
    #[allow(clippy::too_many_lines)]
    pub fn auto_equip_item(&mut self, guid: ObjectGuid, src: InvPos) {
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let player = &entry.player;
        let Some(src_row) = row_index_at(&player.deferred.inventory, src)
            .map(|i| player.deferred.inventory[i].clone())
        else {
            return;
        };
        let Some(src_proto) = self.inv_proto(src_row.item.item_id).cloned() else {
            return;
        };
        let src_is_bag = src_proto.container_slots > 0;
        let src_item = item_guid(src_row.item.item_guid);
        let dest =
            match self.can_equip_item(player, NULL_SLOT, &src_proto, Some(&src_row), !src_is_bag) {
                Ok(d) => d,
                Err(e) => {
                    self.send_inventory_error(
                        guid,
                        e,
                        src_item,
                        ObjectGuid::EMPTY,
                        src_proto.required_level,
                    );
                    return;
                }
            };
        if InvPos::new(BAG_MAIN, dest) == src {
            return;
        }
        let pre = self.snapshot(guid);
        let dst_pos = InvPos::new(BAG_MAIN, dest);
        let dst_row = row_index_at(&player.deferred.inventory, dst_pos)
            .map(|i| player.deferred.inventory[i].clone());
        let Some(dst_row) = dst_row else {
            let owned = self.detach(guid, src).expect("src");
            let entry = self.players.get_mut(&guid).expect("player");
            entry.player.deferred.inventory.push(owned);
            Self::equip_apply(
                &mut entry.player.deferred.inventory,
                src_row.item.item_guid,
                dest,
                &src_proto,
            );
            self.auto_unequip_offhand(guid);
            self.sync_inventory(guid, &pre);
            if dst_pos.is_bag_slot() {
                let mut writer = PacketWriter::new();
                writer.write_u64_le(src_item.raw());
                let mut packet = WorldPacket::new(opcode_id::SMSG_OPEN_CONTAINER);
                packet.take_writer(writer);
                self.events.push(MapEvent::Packet { to: guid, packet });
            }
            return;
        };
        let dst_item = item_guid(dst_row.item.item_guid);
        let res = self.can_unequip_item(player, dest, !src_is_bag);
        if res != ERR_OK {
            self.send_inventory_error(guid, res, dst_item, src_item, 0);
            return;
        }
        let Some(dst_proto) = self.inv_proto(dst_row.item.item_id).cloned() else {
            return;
        };
        // dest->src fallback chain.
        let player_ref = &self.players.get(&guid).expect("player").player;
        let fallback: Result<Fallback, InvResult> = if src.is_equipment() {
            match self.can_equip_item(player_ref, src.slot, &dst_proto, Some(&dst_row), true) {
                Ok(e) => {
                    let r = self.can_unequip_item(player_ref, e, true);
                    if r == ERR_OK {
                        Ok(Fallback::Equip(e))
                    } else {
                        Err(r)
                    }
                }
                Err(e) => Err(e),
            }
        } else if src.is_bank() {
            self.can_bank_auto(
                player_ref,
                src.bag,
                src.slot,
                &dst_proto,
                dst_row.item.count,
                true,
                Some(&dst_row),
            )
            .or_else(|_| {
                self.can_bank_auto(
                    player_ref,
                    src.bag,
                    NULL_SLOT,
                    &dst_proto,
                    dst_row.item.count,
                    true,
                    Some(&dst_row),
                )
            })
            .or_else(|_| {
                self.can_bank_auto(
                    player_ref,
                    NULL_BAG,
                    NULL_SLOT,
                    &dst_proto,
                    dst_row.item.count,
                    true,
                    Some(&dst_row),
                )
            })
            .map(Fallback::Store)
        } else {
            self.can_store_auto(
                player_ref,
                src.bag,
                src.slot,
                &dst_proto,
                dst_row.item.count,
                true,
                Some(&dst_row),
            )
            .or_else(|_| {
                self.can_store_auto(
                    player_ref,
                    src.bag,
                    NULL_SLOT,
                    &dst_proto,
                    dst_row.item.count,
                    true,
                    Some(&dst_row),
                )
            })
            .or_else(|_| {
                self.can_store_auto(
                    player_ref,
                    NULL_BAG,
                    NULL_SLOT,
                    &dst_proto,
                    dst_row.item.count,
                    true,
                    Some(&dst_row),
                )
            })
            .map(Fallback::Store)
        };
        let fallback = match fallback {
            Ok(f) => f,
            Err(e) => {
                self.send_inventory_error(guid, e, dst_item, src_item, dst_proto.required_level);
                return;
            }
        };
        let dst_owned = self.detach(guid, dst_pos).expect("dst");
        let src_owned = self.detach(guid, src).expect("src");
        {
            let entry = self.players.get_mut(&guid).expect("player");
            entry.player.deferred.inventory.push(src_owned);
            Self::equip_apply(
                &mut entry.player.deferred.inventory,
                src_row.item.item_guid,
                dest,
                &src_proto,
            );
        }
        match fallback {
            Fallback::Store(dest) => self.store_apply(guid, &dest, &dst_owned),
            Fallback::Equip(e) => {
                let entry = self.players.get_mut(&guid).expect("player");
                entry.player.deferred.inventory.push(dst_owned);
                Self::equip_apply(
                    &mut entry.player.deferred.inventory,
                    dst_row.item.item_guid,
                    e,
                    &dst_proto,
                );
            }
        }
        self.auto_unequip_offhand(guid);
        self.sync_inventory(guid, &pre);
        if dst_pos.is_bag_slot() {
            let mut writer = PacketWriter::new();
            writer.write_u64_le(src_item.raw());
            let mut packet = WorldPacket::new(opcode_id::SMSG_OPEN_CONTAINER);
            packet.take_writer(writer);
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
    }

    /// `HandleAutoEquipItemSlotOpcode`: equip the
    /// item identified by GUID into `dst_slot`.
    pub fn auto_equip_item_slot(&mut self, guid: ObjectGuid, item: ObjectGuid, dst_slot: u8) {
        if !(InvPos::new(BAG_MAIN, dst_slot)).is_equipment() {
            return;
        }
        let Some(entry) = self.players.get(&guid) else {
            return;
        };
        let rows = &entry.player.deferred.inventory;
        let Some(row) = rows.iter().find(|r| item_guid(r.item.item_guid) == item) else {
            return;
        };
        let src = if row.bag == 0 {
            InvPos::new(BAG_MAIN, row.slot)
        } else {
            // Contained: packet bag = containing
            // bag's own slot.
            let bag_slot = rows
                .iter()
                .find(|r| r.bag == 0 && r.item.item_guid == row.bag)
                .map_or(BAG_MAIN, |r| r.slot);
            InvPos::new(bag_slot, row.slot)
        };
        if src == InvPos::new(BAG_MAIN, dst_slot) {
            return;
        }
        self.swap_item(guid, src, InvPos::new(BAG_MAIN, dst_slot));
    }
}
