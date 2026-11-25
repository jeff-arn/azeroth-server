//! Vendor NPC services (Tier 4 step 4c): list, buy,
//! buy-in-slot, sell, buyback, and repair, transcribed
//! from the fork:
//! - `HandleListInventoryOpcode` / `SendListInventory`
//!   (`ItemHandler.cpp:758-875`)
//! - `HandleBuyItemInSlotOpcode` / `HandleBuyItemOpcode`
//!   / `Player::BuyItemFromVendor`
//!   (`ItemHandler.cpp:656-761`, `Player.cpp:18169-18335`)
//! - `HandleSellItemOpcode` (`ItemHandler.cpp:479-650`)
//! - `HandleBuybackItem` (`ItemHandler.cpp:726-761`)
//! - `HandleRepairItemOpcode` (`NPCHandler.cpp:830-866`) plus
//!   `Player::DurabilityRepair(All)` (`Player.cpp:4935-5035`)
//! - `Player::GetNPCIfCanInteractWith`
//!   (`Player.cpp:2497-2583`) as the shared interaction
//!   gate for this and the later NPC-service families.
//!
//! Inventory placement reuses the 4b substrate
//! ([`crate::inventory`]): rows are the item model,
//! buys create detached rows (save-diff persists them),
//! and client sync is the snapshot diff.
//!
//! Recorded remainders (not silently dropped):
//! - UNMODELED: random-property generation for newly
//!   bought items (`GenerateItemRandomPropertyId`);
//!   the same gap exists on every Rust grant path.
//! - UNMODELED: `ItemAddedQuestCheck` /
//!   `ItemRemovedQuestCheck` quest-update packets on
//!   buy/sell (objective counts derive from live rows).
//! - UNMODELED: forced reputation rank overrides and
//!   the charmed-NPC active-player branch of
//!   `GetNPCIfCanInteractWith` (no charmer-pet vendors).
//! - UNMODELED: money-trade logging
//!   (`LogModifyMoney` audit lines).
//! - UNMODELED: selling an item that is currently
//!   looted (`GetLootGuid`): looted items are not
//!   inventory rows until taken, so the arm is
//!   unreachable in the row model.

use mangos_entities::creature::VendorItemCount;
use mangos_entities::item::item_guid;
use mangos_entities::player::{InventoryRow, Player};
use mangos_entities::unit::{
    UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL, UNIT_STATE_FEIGN_DEATH, UNIT_STATE_TAXI_FLIGHT,
};
use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::{aura as aura_type, aura_interrupt_flag};
use mangos_world_data::item_prototype::ItemPrototype;
use mangos_world_data::vendor_items::VendorItem;

use crate::inventory::{
    BAG_MAIN, BUYBACK_SLOT_END as INV_BUYBACK_END, BUYBACK_SLOT_START as INV_BUYBACK_START,
    ERR_DOESNT_GO_TO_SLOT, INVENTORY_SLOT_BAG_END, INVENTORY_SLOT_BAG_START, InvPos, NULL_SLOT,
    bag_is_empty, row_index_at,
};
use crate::map::Map;

/// `UNIT_NPC_FLAG_VENDOR` (`UnitDefines.h:654`).
pub(crate) const NPC_FLAG_VENDOR: u32 = 0x0000_0004;
/// `UNIT_NPC_FLAG_REPAIR` (`UnitDefines.h:661`).
pub(crate) const NPC_FLAG_REPAIR: u32 = 0x0000_1000;
/// `UNIT_NPC_FLAG_BANKER` (`UnitDefines.h:659`).
pub(crate) const NPC_FLAG_BANKER: u32 = 0x0000_0100;
// `BuyBankSlotResult` (`Player.h:89-95`).
const ERR_BANKSLOT_FAILED_TOO_MANY: u32 = 0;
const ERR_BANKSLOT_INSUFFICIENT_FUNDS: u32 = 1;
const ERR_BANKSLOT_NOTBANKER: u32 = 2;
/// `UNIT_NPC_FLAG_SPIRITHEALER | SPIRITGUIDE`.
const NPC_FLAG_SPIRIT_SERVICE: u32 = 0x0000_0020 | 0x0000_0040;
/// `UNIT_FLAG_NOT_SELECTABLE` (`UnitDefines.h:566`).
const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
/// `PLAYER_FLAGS_CONTESTED_PVP` (`Player.h:323`).
const PLAYER_FLAGS_CONTESTED_PVP: u32 = 0x0000_0100;
/// `PLAYER_EXTRA_GM_ON`.
const PLAYER_EXTRA_GM_ON: u32 = 0x0001;
/// `CREATURE_FLAG_EXTRA_NO_MOVEMENT_PAUSE`.
const CREATURE_FLAG_EXTRA_NO_MOVEMENT_PAUSE: u32 = 0x0000_0020;
/// `INTERACTION_DISTANCE`.
const INTERACTION_DISTANCE: f32 = 5.0;
/// `MAX_VENDOR_ITEMS` (`CreatureDefines.h:641`).
const MAX_VENDOR_ITEMS: usize = 128;
/// `MAX_MONEY_AMOUNT` (`Player.h`, 0x7FFFFFFF-1
/// headroom in the fork constant).
const MAX_MONEY_AMOUNT: u32 = 0x7FFF_FFFE;
// `BuyResult` (`ItemDefines.h:119-130`).
const BUY_ERR_CANT_FIND_ITEM: u8 = 0;
const BUY_ERR_ITEM_ALREADY_SOLD: u8 = 1;
const BUY_ERR_NOT_ENOUGH_MONEY: u8 = 2;
const BUY_ERR_DISTANCE_TOO_FAR: u8 = 5;
const BUY_ERR_RANK_REQUIRE: u8 = 11;
const BUY_ERR_REPUTATION_REQUIRE: u8 = 12;
// `SellResult` (`ItemDefines.h:132-140`).
const SELL_ERR_CANT_FIND_ITEM: u8 = 1;
const SELL_ERR_CANT_SELL_ITEM: u8 = 2;
const SELL_ERR_CANT_FIND_VENDOR: u8 = 3;
/// `SELL_ITEM_SUCCESS`.
const SELL_ITEM_SUCCESS: u8 = 0;
// Re-exported buyback bounds (single definition in
// `inventory`).
pub(crate) const BUYBACK_SLOT_START: u8 = INV_BUYBACK_START;
pub(crate) const BUYBACK_SLOT_END: u8 = INV_BUYBACK_END;

/// Whether a row is an equippable bag (fork
/// `Item::IsBag` for the sell path): container class
/// with slots.
fn row_is_bag(proto: &ItemPrototype) -> bool {
    proto.class == 1 && proto.container_slots > 0
}

/// Reads `PLAYER_FIELD_COINAGE`.
fn get_money(player: &Player) -> u32 {
    player
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::PLAYER_FIELD_COINAGE)
}

impl Map {
    /// `Player::ModifyMoney` (`Player.h:1036-1043`),
    /// without the money-trade audit logging. Trial
    /// restrictions do not exist on this server, so
    /// `GetMaxMoney` is the plain cap.
    pub(crate) fn modify_money(&mut self, guid: ObjectGuid, delta: i64) {
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let current = get_money(&entry.player);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs().min(u64::from(u32::MAX)) as u32)
        } else {
            u32::try_from(
                (u64::from(current) + delta.unsigned_abs()).min(u64::from(MAX_MONEY_AMOUNT)),
            )
            .unwrap_or(MAX_MONEY_AMOUNT)
        };
        entry
            .player
            .unit
            .values()
            .set_u32(offsets::PLAYER_FIELD_COINAGE, next);
    }

    pub(crate) fn player_money(&self, guid: ObjectGuid) -> u32 {
        self.players.get(&guid).map_or(0, |e| get_money(&e.player))
    }

    /// `SpellCaster::InterruptSpellsWithChannelFlags`
    /// (`SpellCaster.cpp:1777-1785`) for the player's
    /// current channeled cast.
    pub(crate) fn interrupt_spells_with_channel_flags(&mut self, guid: ObjectGuid, flags: u32) {
        let hit = self.casts.get(&guid).is_some_and(|cast| {
            cast.channeling
                && self
                    .stores
                    .spells
                    .get(cast.spell_id)
                    .is_some_and(|s| s.channel_interrupt_flags & flags != 0)
        });
        if hit {
            self.cancel_cast(guid, true);
        }
    }

    /// The interaction preamble shared by every NPC
    /// service handler in the fork: fake-death removal
    /// + interacting-interrupt cleanup.
    fn npc_interact_preamble(&mut self, guid: ObjectGuid) {
        // `GetNPCIfCanInteractWith` tail
        // (`Player.cpp:2575-2582`).
        let feigning = self
            .unit_base(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_FEIGN_DEATH));
        if feigning {
            let spells: Vec<u32> = self
                .aura_holders(guid)
                .iter()
                .filter(|h| {
                    h.effects
                        .iter()
                        .flatten()
                        .any(|e| e.aura_type == aura_type::FEIGN_DEATH)
                })
                .map(|h| h.spell_id)
                .collect();
            for spell_id in spells {
                self.cancel_aura(guid, spell_id);
            }
        }
        self.interrupt_spells_with_channel_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS);
        self.remove_auras_with_interrupt_flags(guid, aura_interrupt_flag::INTERACTING_CANCELS, 0);
    }

    /// `Unit::IsInvisibleForAlive` (`Unit.cpp:7803-7811`)
    /// for a creature in this map.
    pub(crate) fn creature_invisible_for_alive(&self, creature: ObjectGuid) -> bool {
        if self.creatures.get(&creature).is_some_and(|c| {
            c.unit
                .world
                .object
                .values()
                .has_flag(offsets::UNIT_NPC_FLAGS, NPC_FLAG_SPIRIT_SERVICE)
        }) {
            return true;
        }
        self.aura_holders(creature)
            .iter()
            .any(|h| h.spell_id == 9036)
    }

    /// `Player::GetReputationRank` at-war cap for the
    /// reaction gate: returns true when the player's
    /// state flags for `faction` carry `AtWar`.
    fn reputation_at_war(&self, player: &Player, faction: u32) -> bool {
        let Some(entry) = self.stores.factions.get(faction) else {
            return false;
        };
        if entry.reputation_index < 0 {
            return false;
        }
        let slots = self.stores.factions.initial_slots(
            player.unit.race(),
            player.unit.class(),
            &player.deferred.reputation,
        );
        slots
            .get(entry.reputation_index as usize)
            .is_some_and(|s| s.0 & 0x02 != 0)
    }

    /// `WorldObject::GetReactionTo` reduced to the
    /// hostile bit for creature → player
    /// (`Object.cpp:3436-3565`): contested guards,
    /// reputation rank when the creature faction can
    /// have reputation, else template friendship. The
    /// player-controlled self branch is skipped (its
    /// only vendor-reachable case is a pet merchant,
    /// which cannot carry vendor flags in practice).
    pub(crate) fn creature_hostile_to_player(
        &self,
        creature: ObjectGuid,
        guid: ObjectGuid,
    ) -> bool {
        let Some(centry) = self.creatures.get(&creature) else {
            return false;
        };
        let Some(pentry) = self.players.get(&guid) else {
            return false;
        };
        let player = &pentry.player;
        let self_tpl = self
            .stores
            .factions
            .template(centry.unit.faction_template());
        let target_tpl = self
            .stores
            .factions
            .template(player.unit.faction_template());
        let contested_pvp = player
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP);
        // `GetFactionReactionTo` contested-guard arm.
        if let Some(tpl) = self_tpl
            && tpl.is_contested_guard_faction()
            && contested_pvp
        {
            return true;
        }
        if let Some(tpl) = self_tpl
            && let Some(faction_entry) = self.stores.factions.get(tpl.faction)
            && faction_entry.reputation_index >= 0
        {
            let rank = self.reputation_rank(player, tpl.faction);
            let rank = if self.reputation_at_war(player, tpl.faction) {
                rank.min(3) // capped at REP_NEUTRAL
            } else {
                rank
            };
            return rank <= 1; // REP_HOSTILE
        }
        match (self_tpl, target_tpl) {
            (Some(a), Some(b)) => a.is_hostile_to(&b),
            _ => false,
        }
    }

    /// `Player::GetReputationPriceDiscount`
    /// (`Player.cpp:19264-19288`): 0.90 honored, 0.85
    /// revered, 0.80 exalted, else 1.0. Accumulative
    /// discounts are a later-patch fork option and do
    /// not apply to the 5875 branch.
    pub(crate) fn reputation_price_discount(&self, guid: ObjectGuid, creature: ObjectGuid) -> f32 {
        let Some(pentry) = self.players.get(&guid) else {
            return 1.0;
        };
        let Some(centry) = self.creatures.get(&creature) else {
            return 1.0;
        };
        let faction_id = self
            .stores
            .factions
            .faction_id_for_template(centry.unit.faction_template());
        if faction_id == 0 {
            return 1.0;
        }
        match self.reputation_rank(&pentry.player, faction_id) {
            7 => 0.80, // REP_EXALTED
            6 => 0.85, // REP_REVERED
            5 => 0.90, // REP_HONORED
            _ => 1.0,
        }
    }

    /// `Player::GetNPCIfCanInteractWith`
    /// (`Player.cpp:2497-2583`). On success this also
    /// runs the fork tail (feign-death + interacting
    /// interrupt cleanup).
    pub(crate) fn npc_can_interact(
        &self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        npc_flag_mask: u32,
    ) -> bool {
        if !npc_guid.is_creature_or_pet() {
            return false;
        }
        let Some(pentry) = self.players.get(&guid) else {
            return false;
        };
        let player = &pentry.player;
        let Some(centry) = self.creatures.get(&npc_guid) else {
            return false;
        };
        let creature = centry;
        let info = creature.info.clone();
        if !creature.unit.is_alive() {
            return false;
        }
        if creature
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE)
        {
            return false;
        }
        if npc_flag_mask != 0
            && !creature
                .unit
                .world
                .object
                .values()
                .has_flag(offsets::UNIT_NPC_FLAGS, npc_flag_mask)
        {
            return false;
        }
        // Charmed by someone else.
        let charmer = creature
            .unit
            .world
            .object
            .values()
            .get_guid(offsets::UNIT_FIELD_CHARMEDBY);
        if !charmer.is_empty() && charmer != guid {
            return false;
        }
        if creature.unit.is_in_combat() && !info.civilian {
            return false;
        }
        if creature
            .unit
            .has_unit_state(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL)
        {
            return false;
        }
        if player.unit.has_unit_state(UNIT_STATE_TAXI_FLIGHT) {
            return false;
        }
        if player.unit.is_alive() && self.creature_invisible_for_alive(npc_guid) {
            return false;
        }
        // Corpse/ghost visibility parity with the
        // fork's first branch.
        if !player.unit.is_alive() && info.static_flags1 & 0x0020_0000 == 0 {
            return false;
        }
        if self.creature_hostile_to_player(npc_guid, guid) {
            return false;
        }
        // Reputation unfriendly-or-worse block.
        let faction_id = self
            .stores
            .factions
            .faction_id_for_template(creature.unit.faction_template());
        if faction_id != 0
            && self
                .stores
                .factions
                .get(faction_id)
                .is_some_and(|f| f.reputation_index >= 0)
            && self.reputation_rank(player, faction_id) <= 2
        {
            return false;
        }
        if !self.within_dist(guid, npc_guid, INTERACTION_DISTANCE) {
            return false;
        }
        true
    }

    /// Gate + fork tail (feign-death removal and
    /// interacting-interrupt cleanup).
    pub(crate) fn npc_if_can_interact_with(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        npc_flag_mask: u32,
    ) -> bool {
        if !self.npc_can_interact(guid, npc_guid, npc_flag_mask) {
            return false;
        }
        self.npc_interact_preamble(guid);
        true
    }

    /// Merged vendor rows in wire/buy order: the
    /// creature's own list first, template rows after
    /// (`SendListInventory` two-loop order; buy lookup
    /// resolves the same way).
    pub(crate) fn vendor_rows(&self, creature: ObjectGuid) -> Vec<VendorItem> {
        let Some(centry) = self.creatures.get(&creature) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if let Some(list) = self.stores.vendor_items.creature_items(centry.info.entry) {
            rows.extend_from_slice(list);
        }
        if centry.info.vendor_id != 0
            && let Some(list) = self
                .stores
                .vendor_items
                .template_items(centry.info.vendor_id)
        {
            rows.extend_from_slice(list);
        }
        rows
    }

    /// `Creature::GetVendorItemCurrentCount`
    /// (`Creature.cpp:3269-3304`), mutating the
    /// restock state exactly as the fork does on read.
    fn vendor_item_current_count(&mut self, creature: ObjectGuid, vitem: &VendorItem) -> u32 {
        if vitem.max_count == 0 {
            return 0;
        }
        let now = self.now_unix;
        let buy_count = self
            .inv_proto(vitem.item_id)
            .map_or(1, |p| p.buy_count.max(1));
        let Some(centry) = self.creatures.get_mut(&creature) else {
            return vitem.max_count;
        };
        let Some(idx) = centry
            .vendor_item_counts
            .iter()
            .position(|c| c.item_id == vitem.item_id)
        else {
            return vitem.max_count;
        };
        let state = centry.vendor_item_counts[idx];
        if state.restock_delay > 0
            && state.last_increment_time + u64::from(state.restock_delay) <= now
        {
            let diff = ((now - state.last_increment_time) / u64::from(state.restock_delay))
                .min(u64::from(u32::MAX)) as u32;
            let grown = state.count.saturating_add(diff.saturating_mul(buy_count));
            if grown >= vitem.max_count {
                centry.vendor_item_counts.remove(idx);
                return vitem.max_count;
            }
            centry.vendor_item_counts[idx].count = grown;
            centry.vendor_item_counts[idx].last_increment_time = now;
            return grown;
        }
        state.count
    }

    /// `Creature::UpdateVendorItemCurrentCount`
    /// (`Creature.cpp:3306-3349`). Returns the new
    /// current count.
    fn vendor_item_consume(
        &mut self,
        creature: ObjectGuid,
        vitem: &VendorItem,
        used_count: u32,
    ) -> u32 {
        if vitem.max_count == 0 {
            return 0;
        }
        // Restock-first read (fork folds this into the
        // update; the observable result is identical).
        let current = self.vendor_item_current_count(creature, vitem);
        let new_count = current.saturating_sub(used_count);
        let now = self.now_unix;
        let mut restock_delay = f64::from(vitem.incr_time);
        if vitem.item_flags & 0x01 != 0 {
            // VENDOR_ITEM_FLAG_RANDOM_RESTOCK
            restock_delay *=
                f64::from(mangos_spells::SpellRng::irand(&mut self.rng, 80, 120) as u32) / 100.0;
        }
        let sessions = self.players.len() as u32;
        if vitem.item_flags & 0x02 != 0 && sessions > 2500 {
            // VENDOR_ITEM_FLAG_DYNAMIC_RESTOCK. The
            // fork divisor is the global session
            // count; the map only knows its own
            // players (threshold unreachable on one
            // map in practice).
            restock_delay *= 2500.0 / f64::from(sessions);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let restock_delay = restock_delay as u32;
        if let Some(centry) = self.creatures.get_mut(&creature) {
            if let Some(state) = centry
                .vendor_item_counts
                .iter_mut()
                .find(|c| c.item_id == vitem.item_id)
            {
                state.count = new_count;
                state.last_increment_time = now;
                state.restock_delay = restock_delay;
            } else {
                centry.vendor_item_counts.push(VendorItemCount {
                    item_id: vitem.item_id,
                    count: new_count,
                    last_increment_time: now,
                    restock_delay,
                });
            }
        }
        new_count
    }

    /// `WorldSession::SendListInventory`
    /// (`ItemHandler.cpp:763-875`), `menu_type` ALL
    /// (the only caller shape in the packet handler;
    /// gossip uses the same default).
    pub fn send_list_inventory(&mut self, guid: ObjectGuid, npc_guid: ObjectGuid) {
        // `HandleListInventoryOpcode` alive pre-check.
        if !self
            .players
            .get(&guid)
            .is_some_and(|e| e.player.unit.is_alive())
        {
            return;
        }
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_VENDOR) {
            self.send_sell_error(guid, SELL_ERR_CANT_FIND_VENDOR, None, ObjectGuid::EMPTY);
            return;
        }
        // Stop the NPC if moving (fork order: gate,
        // fake-death/interact cleanup already ran in
        // the gate, then pause, then build).
        let no_pause = self
            .creatures
            .get(&npc_guid)
            .is_some_and(|c| c.info.flags_extra & CREATURE_FLAG_EXTRA_NO_MOVEMENT_PAUSE != 0);
        if !no_pause {
            self.pause_out_of_combat_movement(npc_guid);
        }
        let discount = self.reputation_price_discount(guid, npc_guid);
        let (is_gm, class_mask, race_mask) = self.players.get(&guid).map_or((false, 0, 0), |e| {
            let p = &e.player;
            (
                p.extra_flags & PLAYER_EXTRA_GM_ON != 0,
                1u32 << (u32::from(p.unit.class().saturating_sub(1)).min(30)),
                1u32 << (u32::from(p.unit.race().saturating_sub(1)).min(30)),
            )
        });
        let vendor_faction_id = self.creatures.get(&npc_guid).map_or(0, |c| {
            self.stores
                .factions
                .faction_id_for_template(c.unit.faction_template())
        });
        let vendor_faction_rank = self
            .players
            .get(&guid)
            .map_or(3, |e| self.reputation_rank(&e.player, vendor_faction_id));
        let rows = self.vendor_rows(npc_guid);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc_guid.raw());
        // Count is patched after the loops, as in C++.
        let count_at = writer.len();
        writer.write_u8(0);
        let mut count = 0usize;
        for vitem in &rows {
            let Some(proto) = self.inv_proto(vitem.item_id).cloned() else {
                continue;
            };
            if !is_gm {
                // Class-wrong bind-on-pickup and
                // race-wrong items are hidden.
                if proto.allowable_class & class_mask == 0 && proto.bonding == 1 {
                    continue;
                }
                if proto.allowable_race & race_mask == 0 {
                    continue;
                }
                if proto.required_reputation_faction == 0
                    && proto.required_reputation_rank > vendor_faction_rank
                {
                    continue;
                }
                if vitem.condition_id != 0
                    && !self.condition_satisfied(vitem.condition_id, Some(guid), Some(npc_guid))
                {
                    continue;
                }
            }
            let current_count = self.vendor_item_current_count(npc_guid, vitem);
            count += 1;
            writer.write_u32_le(count as u32);
            writer.write_u32_le(vitem.item_id);
            writer.write_u32_le(proto.display_id);
            writer.write_u32_le(if vitem.max_count != 0 {
                current_count
            } else {
                0xFFFF_FFFF
            });
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let price = (f64::from(proto.buy_price) * f64::from(discount) + 0.5) as u32;
            writer.write_u32_le(price);
            writer.write_u32_le(proto.max_durability);
            writer.write_u32_le(proto.buy_count);
            if count >= MAX_VENDOR_ITEMS {
                break;
            }
        }
        let mut bytes = writer.into_vec();
        bytes[count_at] = count as u8;
        if count == 0 {
            // "Vendor has no inventory": count 0 plus
            // the trailing error byte.
            bytes.truncate(count_at + 1);
            bytes.push(0);
        }
        let packet = WorldPacket::with_body(opcode_id::SMSG_LIST_INVENTORY, bytes);
        self.send_to_player(guid, packet);
    }

    /// `Player::SendBuyError` (`Player.cpp:11491-11500`).
    pub(crate) fn send_buy_error(
        &mut self,
        guid: ObjectGuid,
        msg: u8,
        creature: Option<ObjectGuid>,
        item_id: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.map_or(0, ObjectGuid::raw));
        writer.write_u32_le(item_id);
        writer.write_u8(msg);
        let packet = WorldPacket::with_body(opcode_id::SMSG_BUY_FAILED, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `Player::SendSellError` (`Player.cpp:11502-11511`).
    fn send_sell_error(
        &mut self,
        guid: ObjectGuid,
        msg: u8,
        creature: Option<ObjectGuid>,
        item: ObjectGuid,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.map_or(0, ObjectGuid::raw));
        writer.write_u64_le(item.raw());
        writer.write_u8(msg);
        let packet = WorldPacket::with_body(opcode_id::SMSG_SELL_ITEM, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// Builds one freshly created item row (`Item::Create`
    /// defaults: full durability, proto spell charges,
    /// proto duration; random property UNMODELED).
    pub(crate) fn new_item_row(&self, proto: &ItemPrototype, count: u32) -> InventoryRow {
        let charges = proto
            .spells
            .iter()
            .map(|s| s.charges.to_string())
            .collect::<Vec<_>>()
            .join(" ")
            + " ";
        InventoryRow {
            item: mangos_entities::player::ItemInstanceRow {
                creator_guid: 0,
                gift_creator_guid: 0,
                count,
                duration: i32::try_from(proto.duration).unwrap_or(i32::MAX),
                charges,
                flags: 0,
                enchantments: "0 0 0 ".repeat(7),
                random_property_id: 0,
                durability: proto.max_durability.min(u32::from(u16::MAX)) as u16,
                text: 0,
                item_guid: self.stores.generate_item_guid(),
                item_id: proto.item_id,
                generated_loot: 0,
            },
            bag: 0,
            slot: NULL_SLOT,
        }
    }

    /// `Player::SendNewItem` (`Player.cpp:11854-11880`)
    /// for a received (not created) item.
    pub(crate) fn send_new_item(&mut self, guid: ObjectGuid, row: &InventoryRow, count: u32) {
        let bag_slot = if row.bag == 0 {
            BAG_MAIN
        } else {
            self.players
                .get(&guid)
                .and_then(|e| {
                    e.player
                        .deferred
                        .inventory
                        .iter()
                        .find(|r| r.bag == 0 && r.item.item_guid == row.bag)
                        .map(|r| r.slot)
                })
                .unwrap_or(BAG_MAIN)
        };
        let slot_field = if row.item.count == count {
            u32::from(row.slot)
        } else {
            0xFFFF_FFFF
        };
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u32_le(1); // received
        writer.write_u32_le(0); // created
        writer.write_u32_le(1); // showInChat
        writer.write_u8(bag_slot);
        writer.write_u32_le(slot_field);
        writer.write_u32_le(row.item.item_id);
        writer.write_u32_le(0); // suffix factor (no random suffix in 1.12 grants)
        writer.write_u32_le(i32::from(row.item.random_property_id) as u32);
        writer.write_u32_le(count);
        let packet = WorldPacket::with_body(opcode_id::SMSG_ITEM_PUSH_RESULT, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// Stores a detached new row through `dests` and
    /// returns the representative last-destination row
    /// for `SendNewItem` (`StoreItem` returns the last
    /// touched item).
    pub(crate) fn store_new_item(
        &mut self,
        guid: ObjectGuid,
        template: &InventoryRow,
        dests: &[crate::inventory::StoreDest],
    ) -> Option<InventoryRow> {
        let pre = self.snapshot(guid);
        self.store_apply(guid, dests, template);
        self.sync_inventory(guid, &pre);
        // `StoreNewItem` -> `ItemAddedQuestCheck`:
        // deliver-quest counters advance live once
        // the gained item is in inventory.
        let gained: u32 = dests.iter().map(|d| d.count).sum();
        if gained > 0 {
            self.item_added_quest_check(guid, template.item.item_id, gained);
        }
        let last = dests.last()?;
        let entry = self.players.get(&guid)?;
        let idx = row_index_at(&entry.player.deferred.inventory, last.pos)?;
        Some(entry.player.deferred.inventory[idx].clone())
    }

    /// `Player::BuyItemFromVendor`
    /// (`Player.cpp:18169-18335`). `bag`/`slot` are the
    /// fork packet pair already translated by the
    /// opcode handlers: `(BAG_MAIN, NULL_SLOT)` = auto,
    /// `(BAG_MAIN, equip-or-inventory slot)`, or
    /// `(bag slot 19..23, slot-in-bag)`.
    #[allow(clippy::too_many_lines)]
    pub fn buy_item_from_vendor(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        item_id: u32,
        count: u8,
        bag: u8,
        slot: u8,
    ) {
        let count = count.max(1);
        let alive = self
            .players
            .get(&guid)
            .is_some_and(|e| e.player.unit.is_alive());
        if !alive {
            return;
        }
        let Some(proto) = self.inv_proto(item_id).cloned() else {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, None, item_id);
            return;
        };
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_VENDOR) {
            self.send_buy_error(guid, BUY_ERR_DISTANCE_TOO_FAR, None, item_id);
            return;
        }
        // Own/template split is needed for the client
        // vendor slot numbering.
        let (own_len, rows) = {
            let entry = self.creatures.get(&npc_guid);
            let (entry_id, vendor_id) = entry.map_or((0, 0), |c| (c.info.entry, c.info.vendor_id));
            let own: Vec<VendorItem> = self
                .stores
                .vendor_items
                .creature_items(entry_id)
                .map(<[VendorItem]>::to_vec)
                .unwrap_or_default();
            let mut all = own.clone();
            if vendor_id != 0
                && let Some(t) = self.stores.vendor_items.template_items(vendor_id)
            {
                all.extend_from_slice(t);
            }
            (own.len(), all)
        };
        if rows.is_empty() {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, Some(npc_guid), item_id);
            return;
        }
        let Some(vendor_slot) = rows.iter().position(|v| v.item_id == item_id) else {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, Some(npc_guid), item_id);
            return;
        };
        let _ = own_len;
        let cr_item = rows[vendor_slot];
        let buy_count = proto.buy_count.max(1);
        let total_count = buy_count * u32::from(count);
        if cr_item.max_count != 0
            && self.vendor_item_current_count(npc_guid, &cr_item) < total_count
        {
            self.send_buy_error(guid, BUY_ERR_ITEM_ALREADY_SOLD, Some(npc_guid), item_id);
            return;
        }
        let Some(player_ref) = self.players.get(&guid).map(|e| &e.player) else {
            return;
        };
        let req_faction =
            if proto.required_reputation_faction == 0 && proto.required_reputation_rank > 0 {
                self.creatures.get(&npc_guid).map_or(0, |c| {
                    self.stores
                        .factions
                        .faction_id_for_template(c.unit.faction_template())
                })
            } else {
                proto.required_reputation_faction
            };
        if self.reputation_rank(player_ref, req_faction) < proto.required_reputation_rank {
            self.send_buy_error(guid, BUY_ERR_REPUTATION_REQUIRE, Some(npc_guid), item_id);
            return;
        }
        let player_rank = u32::from(crate::battlegrounds::calculate_rank(
            player_ref.honor_rank_points,
        ));
        if proto.required_honor_rank != 0
            && (player_rank < proto.required_honor_rank
                || player_ref.unit.level() < proto.required_level)
        {
            self.send_buy_error(guid, BUY_ERR_RANK_REQUIRE, Some(npc_guid), item_id);
            return;
        }
        let is_gm = player_ref.extra_flags & PLAYER_EXTRA_GM_ON != 0;
        if cr_item.condition_id != 0
            && !is_gm
            && !self.condition_satisfied(cr_item.condition_id, Some(guid), Some(npc_guid))
        {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, Some(npc_guid), item_id);
            return;
        }
        let discount = self.reputation_price_discount(guid, npc_guid);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let price =
            (f64::from(proto.buy_price * u32::from(count)) * f64::from(discount) + 0.5) as u32;
        if self.player_money(guid) < price {
            self.send_buy_error(guid, BUY_ERR_NOT_ENOUGH_MONEY, Some(npc_guid), item_id);
            return;
        }

        // Placement classification
        // (`IsInventoryPos` / `IsEquipmentPos`).
        let auto = bag == BAG_MAIN && slot == NULL_SLOT;
        let inventory_pos = auto
            || (bag == BAG_MAIN && ((23..39).contains(&slot) || (81..97).contains(&slot)))
            || (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&bag);
        let equipment_pos = bag == BAG_MAIN && slot < INVENTORY_SLOT_BAG_END;

        let placed: Option<InventoryRow> = if inventory_pos {
            let Some(dests) = self.players.get(&guid).map(|e| {
                self.can_store_auto(&e.player, bag, slot, &proto, total_count, false, None)
            }) else {
                return;
            };
            match dests {
                Err(msg) => {
                    self.send_inventory_error(
                        guid,
                        msg,
                        ObjectGuid::EMPTY,
                        ObjectGuid::EMPTY,
                        item_id,
                    );
                    return;
                }
                Ok(dests) => {
                    self.modify_money(guid, -i64::from(price));
                    let template = self.new_item_row(&proto, total_count);
                    self.store_new_item(guid, &template, &dests)
                }
            }
        } else if equipment_pos {
            if total_count != 1 {
                self.send_inventory_error(
                    guid,
                    crate::inventory::ERR_ITEM_CANT_BE_EQUIPPED,
                    ObjectGuid::EMPTY,
                    ObjectGuid::EMPTY,
                    0,
                );
                return;
            }
            let Some(dest) = self
                .players
                .get(&guid)
                .map(|e| self.can_equip_item(&e.player, slot, &proto, None, false))
            else {
                return;
            };
            match dest {
                Err(msg) => {
                    self.send_inventory_error(
                        guid,
                        msg,
                        ObjectGuid::EMPTY,
                        ObjectGuid::EMPTY,
                        item_id,
                    );
                    return;
                }
                Ok(dest_slot) => {
                    self.modify_money(guid, -i64::from(price));
                    let pre = self.snapshot(guid);
                    let mut row = self.new_item_row(&proto, 1);
                    row.slot = dest_slot;
                    if proto.bonding == 2 {
                        row.item.flags |= 0x0000_0001; // ITEM_DYNFLAG_BINDED
                    }
                    let placed = row.clone();
                    if let Some(entry) = self.players.get_mut(&guid) {
                        entry.player.deferred.inventory.push(row);
                    }
                    self.auto_unequip_offhand(guid);
                    self.sync_inventory(guid, &pre);
                    Some(placed)
                }
            }
        } else {
            self.send_inventory_error(
                guid,
                ERR_DOESNT_GO_TO_SLOT,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let Some(placed) = placed else {
            return;
        };
        let new_count = self.vendor_item_consume(npc_guid, &cr_item, total_count);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc_guid.raw());
        writer.write_u32_le(vendor_slot as u32 + 1);
        writer.write_u32_le(if cr_item.max_count > 0 {
            new_count
        } else {
            0xFFFF_FFFF
        });
        writer.write_u32_le(u32::from(count));
        let packet = WorldPacket::with_body(opcode_id::SMSG_BUY_ITEM, writer.into_vec());
        self.send_to_player(guid, packet);
        self.send_new_item(guid, &placed, total_count);
    }

    /// `WorldSession::HandleBuyItemInSlotOpcode`
    /// (`ItemHandler.cpp:699-724`): resolve the target
    /// bag by guid (player guid = backpack), then the
    /// shared buy flow. An unknown bag guid is a
    /// silent drop, as in the fork.
    pub fn buy_item_in_slot(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        item_id: u32,
        bag_guid: ObjectGuid,
        bag_slot: u8,
        count: u8,
    ) {
        let bag = if bag_guid == guid {
            BAG_MAIN
        } else {
            let found = self.players.get(&guid).and_then(|e| {
                e.player
                    .deferred
                    .inventory
                    .iter()
                    .find(|r| {
                        r.bag == 0
                            && (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&r.slot)
                            && item_guid(r.item.item_guid) == bag_guid
                    })
                    .map(|r| r.slot)
            });
            let Some(bag) = found else {
                return;
            };
            bag
        };
        self.buy_item_from_vendor(guid, npc_guid, item_id, count, bag, bag_slot);
    }

    /// `Player::RemoveItemFromBuyBackSlot`
    /// (`Player.cpp:11437-11471`): clears the row and
    /// the three buyback fields, and repoints the ring
    /// cursor at the freed slot when the cursor slot
    /// is occupied.
    fn remove_item_from_buyback_slot(&mut self, guid: ObjectGuid, slot: u8) {
        if !(BUYBACK_SLOT_START..BUYBACK_SLOT_END).contains(&slot) {
            return;
        }
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let player = &mut entry.player;
        player
            .deferred
            .inventory
            .retain(|r| !(r.bag == 0 && r.slot == slot));
        let eslot = u16::from(slot - BUYBACK_SLOT_START);
        let values = player.unit.values();
        values.set_guid(
            offsets::PLAYER_FIELD_VENDORBUYBACK_SLOT_1 + eslot * 2,
            ObjectGuid::EMPTY,
        );
        values.set_u32(offsets::PLAYER_FIELD_BUYBACK_PRICE_1 + eslot, 0);
        values.set_u32(offsets::PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + eslot, 0);
        let cursor = player.current_buyback_slot;
        if player
            .deferred
            .inventory
            .iter()
            .any(|r| r.bag == 0 && r.slot == cursor)
        {
            player.current_buyback_slot = slot;
        }
    }

    /// `Player::AddItemToBuyBackSlot`
    /// (`Player.cpp:11358-11435`), including the fork
    /// quirk where the empty-slot `break` result is
    /// overwritten by the oldest-slot selection.
    #[allow(unused_assignments)] // fork empty-slot break overwrite quirk
    fn add_item_to_buyback_slot(&mut self, guid: ObjectGuid, mut row: InventoryRow, money: u32) {
        // Pick the slot.
        let slot = {
            let Some(entry) = self.players.get(&guid) else {
                return;
            };
            let player = &entry.player;
            let rows = &player.deferred.inventory;
            let occupied = |s: u8| rows.iter().any(|r| r.bag == 0 && r.slot == s);
            let timestamp = |s: u8| {
                player.unit.world.object.values().get_u32(
                    offsets::PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + u16::from(s - BUYBACK_SLOT_START),
                )
            };
            let mut slot = player.current_buyback_slot;
            if occupied(slot) {
                let mut oldest_time = timestamp(BUYBACK_SLOT_START);
                let mut oldest_slot = BUYBACK_SLOT_START;
                for i in (BUYBACK_SLOT_START + 1)..BUYBACK_SLOT_END {
                    if !occupied(i) {
                        slot = i;
                        break;
                    }
                    let i_time = timestamp(i);
                    if oldest_time > i_time {
                        oldest_time = i_time;
                        oldest_slot = i;
                    }
                }
                // Fork quirk: unconditional, discards
                // the empty-slot break above.
                slot = oldest_slot;
            }
            slot
        };
        self.remove_item_from_buyback_slot(guid, slot);
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let player = &mut entry.player;
        row.bag = 0;
        row.slot = slot;
        let item_object_guid = item_guid(row.item.item_guid);
        player.deferred.inventory.push(row);
        // `etime = now - m_loginTime + 30h`;
        // `load_now_unix` is this session's login
        // timestamp.
        let etime = (self.now_unix.saturating_sub(player.load_now_unix) + 30 * 3600)
            .min(u64::from(u32::MAX)) as u32;
        let eslot = u16::from(slot - BUYBACK_SLOT_START);
        let values = player.unit.values();
        values.set_guid(
            offsets::PLAYER_FIELD_VENDORBUYBACK_SLOT_1 + eslot * 2,
            item_object_guid,
        );
        values.set_u32(offsets::PLAYER_FIELD_BUYBACK_PRICE_1 + eslot, money);
        values.set_u32(offsets::PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + eslot, etime);
        if player.current_buyback_slot < BUYBACK_SLOT_END - 1 {
            player.current_buyback_slot += 1;
        }
    }

    /// `WorldSession::HandleSellItemOpcode`
    /// (`ItemHandler.cpp:479-655`).
    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )] // fork float charge scaling
    pub fn sell_item(
        &mut self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
        item: ObjectGuid,
        count: u8,
    ) {
        if item.is_empty() {
            return;
        }
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_VENDOR) {
            self.send_sell_error(guid, SELL_ERR_CANT_FIND_VENDOR, None, item);
            return;
        }
        // Locate the row by item guid (own rows only
        // exist in the row model, so the fork owner
        // check is structurally satisfied).
        let located = self.players.get(&guid).and_then(|e| {
            e.player
                .deferred
                .inventory
                .iter()
                .position(|r| item_guid(r.item.item_guid) == item)
                .map(|i| (i, e.player.deferred.inventory[i].clone()))
        });
        let Some((_, row)) = located else {
            return; // fork: no packet
        };
        // Bank positions cannot be sold.
        let is_bank_pos = (row.bag == 0 && (39..69).contains(&row.slot))
            || (63..69).contains(&{
                // content rows: bag field holds the bag
                // item guid; resolve its slot
                self.players
                    .get(&guid)
                    .and_then(|e| {
                        e.player
                            .deferred
                            .inventory
                            .iter()
                            .find(|r| r.bag == 0 && r.item.item_guid == row.bag)
                            .map(|r| r.slot)
                    })
                    .unwrap_or(0)
            });
        if is_bank_pos {
            self.send_sell_error(guid, SELL_ERR_CANT_SELL_ITEM, Some(npc_guid), item);
            return;
        }
        let Some(proto) = self.inv_proto(row.item.item_id).cloned() else {
            self.send_sell_error(guid, SELL_ERR_CANT_FIND_ITEM, Some(npc_guid), item);
            return;
        };
        if row_is_bag(&proto) && !bag_is_empty(&self.snapshot(guid), row.item.item_guid) {
            self.send_sell_error(guid, SELL_ERR_CANT_SELL_ITEM, Some(npc_guid), item);
            return;
        }
        let count = if count == 0 {
            row.item.count
        } else {
            u32::from(count)
        };
        if count > row.item.count {
            self.send_sell_error(guid, SELL_ERR_CANT_SELL_ITEM, Some(npc_guid), item);
            return;
        }
        if proto.sell_price == 0 {
            self.send_sell_error(guid, SELL_ERR_CANT_SELL_ITEM, Some(npc_guid), item);
            return;
        }
        let mut money = proto.sell_price * count;
        // Remaining negative spell charges scale value.
        let charges: Vec<i32> = row
            .item
            .charges
            .split_whitespace()
            .map(|w| w.parse::<i32>().unwrap_or(0))
            .collect();
        for (i, spell) in proto.spells.iter().enumerate() {
            if spell.spell_id != 0 && spell.charges < 0 {
                let remaining = charges.get(i).copied().unwrap_or(0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    money = (money as f32 * (remaining as f32 / spell.charges as f32)) as u32;
                }
                break;
            }
        }
        // Durability deduction.
        if proto.max_durability > 0 {
            let lost = proto
                .max_durability
                .saturating_sub(u32::from(row.item.durability));
            if lost > 0 {
                let Some(mut repaircost) = self.stores.durability.repair_cost(
                    proto.item_level,
                    proto.quality,
                    proto.class,
                    proto.subclass,
                    lost,
                ) else {
                    self.send_sell_error(guid, SELL_ERR_CANT_SELL_ITEM, Some(npc_guid), item);
                    return;
                };
                if repaircost == 0 {
                    repaircost = 1;
                }
                money = if repaircost > money {
                    1
                } else {
                    money - repaircost
                };
            }
        }

        let pre = self.snapshot(guid);
        if count < row.item.count {
            // Split: clone into buyback, shrink source.
            let mut clone = row.clone();
            clone.item.item_guid = self.stores.generate_item_guid();
            clone.item.count = count;
            if let Some(entry) = self.players.get_mut(&guid)
                && let Some(src) = entry
                    .player
                    .deferred
                    .inventory
                    .iter_mut()
                    .find(|r| r.item.item_guid == row.item.item_guid)
            {
                src.item.count -= count;
            }
            self.add_item_to_buyback_slot(guid, clone, money);
        } else {
            // Full stack: interrupt a cast using it,
            // detach, and move to buyback.
            let cast_hit = self.casts.get(&guid).is_some_and(|cast| {
                cast.cast_item
                    .is_some_and(|ci| ci.item_guid == row.item.item_guid)
            });
            if cast_hit {
                self.cancel_cast(guid, true);
            }
            let detached = self.players.get_mut(&guid).and_then(|entry| {
                let rows = &mut entry.player.deferred.inventory;
                let idx = rows
                    .iter()
                    .position(|r| r.item.item_guid == row.item.item_guid)?;
                Some(rows.remove(idx))
            });
            let Some(detached) = detached else {
                return;
            };
            self.add_item_to_buyback_slot(guid, detached, money);
        }
        self.sync_inventory(guid, &pre);
        self.modify_money(guid, i64::from(money));
        // The fork sends no explicit sell-success
        // packet: money + buyback/item updates are
        // the wire evidence (see handler tail).
        let _ = SELL_ITEM_SUCCESS;
    }

    /// `WorldSession::HandleBuybackItem`
    /// (`ItemHandler.cpp:656-697`).
    pub fn buyback_item(&mut self, guid: ObjectGuid, npc_guid: ObjectGuid, slot: u32) {
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_VENDOR) {
            self.send_sell_error(guid, SELL_ERR_CANT_FIND_VENDOR, None, ObjectGuid::EMPTY);
            return;
        }
        let slot_ok = u8::try_from(slot)
            .ok()
            .filter(|s| (BUYBACK_SLOT_START..BUYBACK_SLOT_END).contains(s));
        let row = slot_ok.and_then(|s| {
            self.players.get(&guid).and_then(|e| {
                e.player
                    .deferred
                    .inventory
                    .iter()
                    .find(|r| r.bag == 0 && r.slot == s)
                    .cloned()
            })
        });
        let (Some(slot), Some(row)) = (slot_ok, row) else {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, Some(npc_guid), 0);
            return;
        };
        let price = self.players.get(&guid).map_or(0, |e| {
            e.player.unit.world.object.values().get_u32(
                offsets::PLAYER_FIELD_BUYBACK_PRICE_1 + u16::from(slot - BUYBACK_SLOT_START),
            )
        });
        if self.player_money(guid) < price {
            self.send_buy_error(
                guid,
                BUY_ERR_NOT_ENOUGH_MONEY,
                Some(npc_guid),
                row.item.item_id,
            );
            return;
        }
        let Some(proto) = self.inv_proto(row.item.item_id).cloned() else {
            self.send_buy_error(guid, BUY_ERR_CANT_FIND_ITEM, Some(npc_guid), 0);
            return;
        };
        let Some(dests) = self.players.get(&guid).map(|e| {
            self.can_store_auto(
                &e.player,
                BAG_MAIN,
                NULL_SLOT,
                &proto,
                row.item.count,
                false,
                Some(&row),
            )
        }) else {
            return;
        };
        let dests = match dests {
            Ok(d) => d,
            Err(msg) => {
                self.send_inventory_error(
                    guid,
                    msg,
                    item_guid(row.item.item_guid),
                    ObjectGuid::EMPTY,
                    0,
                );
                return;
            }
        };
        self.modify_money(guid, -i64::from(price));
        let pre = self.snapshot(guid);
        self.store_apply(guid, &dests, &row);
        // If every destination merged, `store_apply`
        // left the source row in the buyback slot;
        // the fork's `StoreItem` consumed it.
        let still_there = self.players.get(&guid).is_some_and(|e| {
            e.player
                .deferred
                .inventory
                .iter()
                .any(|r| r.bag == 0 && r.slot == slot && r.item.item_guid == row.item.item_guid)
        });
        if still_there && let Some(entry) = self.players.get_mut(&guid) {
            entry
                .player
                .deferred
                .inventory
                .retain(|r| !(r.bag == 0 && r.slot == slot));
        }
        self.remove_item_from_buyback_slot(guid, slot);
        self.sync_inventory(guid, &pre);
    }

    /// `WorldSession::CheckBanker`
    /// (`ItemHandler.cpp:936-959`). The self-guid GM
    /// form checks `.bank` command access in the
    /// fork; the map has no session security, so
    /// GM-on is the proxy (UNMODELED: command-table
    /// security check).
    pub(crate) fn check_banker(&mut self, guid: ObjectGuid, banker: ObjectGuid) -> bool {
        if banker == guid {
            return self
                .players
                .get(&guid)
                .is_some_and(|e| e.player.extra_flags & PLAYER_EXTRA_GM_ON != 0);
        }
        self.npc_if_can_interact_with(guid, banker, NPC_FLAG_BANKER)
    }

    /// `HandleBankerActivateOpcode` + `SendShowBank`
    /// (`NPCHandler.cpp:74-96`): records the current
    /// banker (bank reachability for slot ops) and
    /// opens the bank window.
    pub fn banker_activate(&mut self, guid: ObjectGuid, banker: ObjectGuid) {
        if !self.check_banker(guid, banker) {
            return;
        }
        let feigning = self
            .unit_base(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_FEIGN_DEATH));
        if feigning {
            let spells: Vec<u32> = self
                .aura_holders(guid)
                .iter()
                .filter(|h| {
                    h.effects
                        .iter()
                        .flatten()
                        .any(|e| e.aura_type == aura_type::FEIGN_DEATH)
                })
                .map(|h| h.spell_id)
                .collect();
            for spell_id in spells {
                self.cancel_aura(guid, spell_id);
            }
        }
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.current_banker_guid = banker;
        }
        let mut writer = PacketWriter::new();
        writer.write_u64_le(banker.raw());
        let packet = WorldPacket::with_body(opcode_id::SMSG_SHOW_BANK, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `HandleBuyBankSlotOpcode`
    /// (`ItemHandler.cpp:961-998`): result packet is
    /// sent on failure only; success is the
    /// `PLAYER_BYTES_2` bank-slot byte + money loss.
    pub fn buy_bank_slot(&mut self, guid: ObjectGuid, banker: ObjectGuid) {
        let send_result = |map: &mut Map, code: u32| {
            let mut writer = PacketWriter::new();
            writer.write_u32_le(code);
            let packet =
                WorldPacket::with_body(opcode_id::SMSG_BUY_BANK_SLOT_RESULT, writer.into_vec());
            map.send_to_player(guid, packet);
        };
        if !self.check_banker(guid, banker) {
            send_result(self, ERR_BANKSLOT_NOTBANKER);
            return;
        }
        let current = self.players.get(&guid).map_or(0, |e| {
            let word = e
                .player
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_BYTES_2);
            ((word >> 16) & 0xff) as u8
        });
        let slot = current + 1;
        let Some(price) = self
            .stores
            .bank_bag_slot_prices
            .get(usize::from(slot))
            .copied()
            .filter(|_| slot > 0)
        else {
            send_result(self, ERR_BANKSLOT_FAILED_TOO_MANY);
            return;
        };
        if self.player_money(guid) < price {
            send_result(self, ERR_BANKSLOT_INSUFFICIENT_FUNDS);
            return;
        }
        if let Some(entry) = self.players.get_mut(&guid) {
            entry
                .player
                .unit
                .values()
                .set_byte(offsets::PLAYER_BYTES_2, 2, slot);
        }
        self.modify_money(guid, -i64::from(price));
    }

    /// One item of `Player::DurabilityRepair`
    /// (`Player.cpp:4955-5000`): pays and restores
    /// when affordable; returns true when durability
    /// changed.
    fn durability_repair_one(&mut self, guid: ObjectGuid, pos: InvPos, discount: f32) -> bool {
        let located = self.players.get(&guid).and_then(|e| {
            row_index_at(&e.player.deferred.inventory, pos)
                .map(|i| e.player.deferred.inventory[i].clone())
        });
        let Some(row) = located else {
            return false;
        };
        let Some(proto) = self.inv_proto(row.item.item_id).cloned() else {
            return false;
        };
        if proto.max_durability == 0 {
            return false;
        }
        let lost = proto
            .max_durability
            .saturating_sub(u32::from(row.item.durability));
        if lost == 0 {
            return false;
        }
        let Some(base) = self.stores.durability.repair_cost(
            proto.item_level,
            proto.quality,
            proto.class,
            proto.subclass,
            lost,
        ) else {
            return false;
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cost = (f64::from(base) * f64::from(discount)) as u32;
        if self.player_money(guid) < cost {
            return false;
        }
        self.modify_money(guid, -i64::from(cost));
        if let Some(entry) = self.players.get_mut(&guid)
            && let Some(target) = entry
                .player
                .deferred
                .inventory
                .iter_mut()
                .find(|r| r.item.item_guid == row.item.item_guid)
        {
            target.item.durability = proto.max_durability.min(u32::from(u16::MAX)) as u16;
            return true;
        }
        false
    }

    /// `WorldSession::HandleRepairItemOpcode`
    /// (`NPCHandler.cpp:830-866`) +
    /// `DurabilityRepairAll` slot order (equipment,
    /// backpack, bag contents; never bank/buyback/
    /// keyring).
    pub fn repair_item(&mut self, guid: ObjectGuid, npc_guid: ObjectGuid, item: ObjectGuid) {
        if !self.npc_if_can_interact_with(guid, npc_guid, NPC_FLAG_REPAIR) {
            return;
        }
        let discount = self.reputation_price_discount(guid, npc_guid);
        let pre = self.snapshot(guid);
        if item.is_empty() {
            // Equipment + backpack (bag 0, slots
            // 0..39), then bag contents in slot order.
            let mut positions: Vec<InvPos> = (0..39).map(|s| InvPos::new(BAG_MAIN, s)).collect();
            if let Some(entry) = self.players.get(&guid) {
                let rows = &entry.player.deferred.inventory;
                for bag_slot in INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END {
                    if let Some(bag_row) = rows.iter().find(|r| r.bag == 0 && r.slot == bag_slot)
                        && let Some(bag_proto) = self.inv_proto(bag_row.item.item_id)
                    {
                        for s in 0..bag_proto.container_slots.min(255) {
                            positions.push(InvPos::new(bag_slot, s as u8));
                        }
                    }
                }
            }
            for pos in positions {
                self.durability_repair_one(guid, pos, discount);
            }
        } else {
            let pos = self.players.get(&guid).and_then(|e| {
                e.player
                    .deferred
                    .inventory
                    .iter()
                    .find(|r| item_guid(r.item.item_guid) == item)
                    .map(|r| {
                        if r.bag == 0 {
                            InvPos::new(BAG_MAIN, r.slot)
                        } else {
                            let bag_slot = e
                                .player
                                .deferred
                                .inventory
                                .iter()
                                .find(|b| b.bag == 0 && b.item.item_guid == r.bag)
                                .map_or(BAG_MAIN, |b| b.slot);
                            InvPos::new(bag_slot, r.slot)
                        }
                    })
            });
            if let Some(pos) = pos {
                self.durability_repair_one(guid, pos, discount);
            }
        }
        self.sync_inventory(guid, &pre);
    }
}
