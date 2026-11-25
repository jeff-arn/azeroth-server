//! Pet stable master (Tier 4 step 4h, server
//! handlers): `NPCHandler.cpp` stable arms —
//! `SendStablePet` list encoding, `HandleStablePet`,
//! `HandleUnstablePet`, `HandleBuyStableSlot`,
//! `HandleStableRevivePet` (fork no-op),
//! `HandleStableSwapPet`, over the player's loaded
//! `character_pet` rows (fork `CharacterDatabaseCache`)
//! plus the live pet.
//!
//! Remainder (recorded, not modeled): the
//! `CheckStableMaster` GM/self case (`stable` chat
//! command lookup — self-guid requests fail here);
//! `SendStablePet` emits the live pet or the slot-0 row
//! then stabled rows in loaded-row order (the fork
//! iterates its cache map).

use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;

use crate::map::Map;
use mangos_entities::{PET_SAVE_LAST_STABLE_SLOT, PET_TYPE_HUNTER};

/// `UNIT_NPC_FLAG_STABLEMASTER`
/// (`UnitDefines.h:666`).
pub const NPC_FLAG_STABLEMASTER: u32 = 0x0000_2000;
/// `MAX_PET_STABLES` (`Pet.h:37`).
pub const MAX_PET_STABLES: u32 = 2;
/// `PET_SAVE_FIRST_STABLE_SLOT` (`Pet.h:44`).
pub const PET_SAVE_FIRST_STABLE_SLOT: u32 = 1;

/// `STABLE_ERR_MONEY` (`NPCHandler.cpp:42`).
const STABLE_ERR_MONEY: u8 = 0x01;
/// `STABLE_ERR_STABLE`.
const STABLE_ERR_STABLE: u8 = 0x06;
/// `STABLE_SUCCESS_STABLE`.
const STABLE_SUCCESS_STABLE: u8 = 0x08;
/// `STABLE_SUCCESS_UNSTABLE`.
const STABLE_SUCCESS_UNSTABLE: u8 = 0x09;
/// `STABLE_SUCCESS_BUY_SLOT`.
const STABLE_SUCCESS_BUY_SLOT: u8 = 0x0A;

/// `CLASS_HUNTER` (`SharedDefines.h`).
const CLASS_HUNTER: u8 = 3;

impl Map {
    /// `WorldSession::SendStableResult`.
    fn send_stable_result(&mut self, player: ObjectGuid, result: u8) {
        let mut writer = PacketWriter::new();
        writer.write_u8(result);
        let packet = WorldPacket::with_body(opcode_id::SMSG_STABLE_RESULT, writer.into_vec());
        self.send_to_player(player, packet);
    }

    /// `WorldSession::CheckStableMaster`
    /// (`NPCHandler.cpp:634`). The fork's hunter-class
    /// arm lives inside `GetNPCIfCanInteractWith`
    /// (`Player.cpp:2526`); modeled here.
    fn check_stable_master(&mut self, player: ObjectGuid, npc: ObjectGuid) -> bool {
        if npc == player {
            // Spell/GM case: fork requires the `stable`
            // chat command; not modeled (see header).
            return false;
        }
        let is_hunter = self
            .players
            .get(&player)
            .is_some_and(|e| e.player.unit.class() == CLASS_HUNTER);
        is_hunter && self.npc_if_can_interact_with(player, npc, NPC_FLAG_STABLEMASTER)
    }

    /// `WorldSession::SendStablePet`
    /// (`NPCHandler.cpp:550`): the `MSG_LIST_STABLED_PETS`
    /// reply.
    pub fn send_stable_pet(&mut self, player: ObjectGuid, npc: ObjectGuid) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc.raw());
        // Placeholder for the shown-pet count, then
        // the purchased stable slots (fork order).
        writer.write_u8(0);
        let (stable_slots, rows) = match self.players.get(&player) {
            Some(entry) => (
                entry.player.stable_slots,
                entry.player.deferred.pets.clone(),
            ),
            None => return,
        };
        writer.write_u8(stable_slots);
        let mut num: u8 = 0;
        let mut wrote_current = false;
        // Not let move dead pet in slot: the live pet
        // first when alive and a hunter pet.
        if let Some(pet) = self.pet_of(player) {
            let live = self.pets.get(&pet).and_then(|state| {
                let creature = self.creatures.get(&pet)?;
                if state.pet_type != PET_TYPE_HUNTER || !creature.unit.is_alive() {
                    return None;
                }
                let v = creature.unit.world.object.values();
                Some((
                    state.pet_number,
                    creature.original_entry,
                    creature.unit.level(),
                    creature.unit.world.name.clone(),
                    u32::from(v.get_byte(offsets::UNIT_FIELD_BYTES_1, 2)),
                ))
            });
            if let Some((id, entry, level, name, loyalty)) = live {
                writer.write_u32_le(id);
                writer.write_u32_le(entry);
                writer.write_u32_le(level);
                writer.write_cstring(&name);
                writer.write_u32_le(loyalty);
                writer.write_u8(0x01);
                num += 1;
                wrote_current = true;
            }
        }
        if !wrote_current && let Some(row) = rows.iter().find(|r| r.slot == 0) {
            writer.write_u32_le(row.id);
            writer.write_u32_le(row.entry);
            writer.write_u32_le(row.level);
            writer.write_cstring(&row.name);
            writer.write_u32_le(row.loyalty);
            writer.write_u8(0x01);
            num += 1;
        }
        for row in &rows {
            if (PET_SAVE_FIRST_STABLE_SLOT..=PET_SAVE_LAST_STABLE_SLOT).contains(&row.slot) {
                writer.write_u32_le(row.id);
                writer.write_u32_le(row.entry);
                writer.write_u32_le(row.level);
                writer.write_cstring(&row.name);
                writer.write_u32_le(row.loyalty);
                writer.write_u8(u8::try_from(row.slot + 1).unwrap_or(0));
                num += 1;
            }
        }
        let mut bytes = writer.into_vec();
        // Fork `data.put<uint8>(wpos, num)`: body
        // offset 8 (after the guid).
        bytes[8] = num;
        let packet = WorldPacket::with_body(opcode_id::MSG_LIST_STABLED_PETS, bytes);
        self.send_to_player(player, packet);
    }

    /// `WorldSession::HandleListStabledPetsOpcode`
    /// (`NPCHandler.cpp:530`).
    pub fn list_stabled_pets(&mut self, player: ObjectGuid, npc: ObjectGuid) {
        // The fork's `GetNPCIfCanInteractWith` rejects
        // non-hunters for the stablemaster mask
        // (`Player.cpp:2526`); failure is silent here.
        let is_hunter = self
            .players
            .get(&player)
            .is_some_and(|e| e.player.unit.class() == CLASS_HUNTER);
        if !is_hunter || !self.npc_if_can_interact_with(player, npc, NPC_FLAG_STABLEMASTER) {
            return;
        }
        self.send_stable_pet(player, npc);
    }

    /// `WorldSession::HandleStablePet`
    /// (`NPCHandler.cpp:658`).
    pub fn stable_pet(&mut self, player: ObjectGuid, npc: ObjectGuid) {
        if !self.unit_is_alive(player) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        if !self.check_stable_master(player, npc) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let Some(pet) = self.pet_of(player) else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        };
        // Can't place in stable dead pet.
        let valid = self.pets.get(&pet).is_some_and(|state| {
            state.pet_type == PET_TYPE_HUNTER
                && self.creatures.get(&pet).is_some_and(|c| c.unit.is_alive())
        });
        if !valid {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let (stable_slots, used): (u32, [bool; 2]) = match self.players.get(&player) {
            Some(entry) => {
                let mut used = [false; 2];
                for row in &entry.player.deferred.pets {
                    if (PET_SAVE_FIRST_STABLE_SLOT..=PET_SAVE_LAST_STABLE_SLOT).contains(&row.slot)
                    {
                        used[(row.slot - PET_SAVE_FIRST_STABLE_SLOT) as usize] = true;
                    }
                }
                (u32::from(entry.player.stable_slots), used)
            }
            None => return,
        };
        let mut free_slot = PET_SAVE_FIRST_STABLE_SLOT;
        while free_slot <= PET_SAVE_LAST_STABLE_SLOT
            && used[(free_slot - PET_SAVE_FIRST_STABLE_SLOT) as usize]
        {
            free_slot += 1;
        }
        if free_slot <= stable_slots {
            self.unsummon_pet(pet, free_slot);
            self.send_stable_result(player, STABLE_SUCCESS_STABLE);
        } else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
        }
    }

    /// `WorldSession::HandleUnstablePet`
    /// (`NPCHandler.cpp:704`).
    pub fn unstable_pet(&mut self, player: ObjectGuid, npc: ObjectGuid, pet_number: u32) {
        if !self.check_stable_master(player, npc) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let row = self.players.get(&player).and_then(|entry| {
            entry
                .player
                .deferred
                .pets
                .iter()
                .find(|r| r.id == pet_number)
                .cloned()
        });
        let Some(row) = row else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        };
        if !(PET_SAVE_FIRST_STABLE_SLOT..=PET_SAVE_LAST_STABLE_SLOT).contains(&row.slot) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let tameable = self
            .stores
            .creature_templates
            .get(row.entry)
            .is_some_and(mangos_world_data::creature_template::CreatureInfo::is_tameable);
        if !tameable {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        // Player may have a pet, but unsummoned
        // currently. Do not erase this pet!
        let has_current = self.pet_of(player).is_some()
            || self
                .players
                .get(&player)
                .is_some_and(|entry| entry.player.deferred.pets.iter().any(|r| r.slot == 0));
        if has_current {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        if self.load_pet_by_number(player, pet_number) {
            self.send_stable_result(player, STABLE_SUCCESS_UNSTABLE);
        } else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
        }
    }

    /// `WorldSession::HandleBuyStableSlot`
    /// (`NPCHandler.cpp:748`).
    pub fn buy_stable_slot(&mut self, player: ObjectGuid, npc: ObjectGuid) {
        if !self.check_stable_master(player, npc) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let slots = self
            .players
            .get(&player)
            .map_or(0, |e| u32::from(e.player.stable_slots));
        if slots >= MAX_PET_STABLES {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let price = self
            .stores
            .stable_slot_prices
            .get((slots + 1) as usize)
            .copied()
            .unwrap_or(0);
        if self.player_money(player) < price {
            self.send_stable_result(player, STABLE_ERR_MONEY);
            return;
        }
        if let Some(entry) = self.players.get_mut(&player) {
            entry.player.stable_slots = u8::try_from(slots + 1).unwrap_or(2);
        }
        self.modify_money(player, -i64::from(price));
        self.send_stable_result(player, STABLE_SUCCESS_BUY_SLOT);
    }

    /// `WorldSession::HandleStableRevivePet`
    /// (`NPCHandler.cpp:768`): the fork handler is
    /// empty; `CMSG_STABLE_REVIVE_PET` is a no-op.
    pub fn stable_revive_pet(&mut self, _player: ObjectGuid) {}

    /// `WorldSession::HandleStableSwapPet`
    /// (`NPCHandler.cpp:770`).
    pub fn stable_swap_pet(&mut self, player: ObjectGuid, npc: ObjectGuid, pet_number: u32) {
        if !self.check_stable_master(player, npc) {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let Some(pet) = self.pet_of(player) else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        };
        let valid = self.pets.get(&pet).is_some_and(|state| {
            state.pet_type == PET_TYPE_HUNTER
                && self.creatures.get(&pet).is_some_and(|c| c.unit.is_alive())
        });
        if !valid {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let row = self.players.get(&player).and_then(|entry| {
            entry
                .player
                .deferred
                .pets
                .iter()
                .find(|r| r.id == pet_number)
                .cloned()
        });
        let Some(row) = row else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        };
        if row.entry == 0 {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        let tameable = self
            .stores
            .creature_templates
            .get(row.entry)
            .is_some_and(mangos_world_data::creature_template::CreatureInfo::is_tameable);
        if !tameable {
            self.send_stable_result(player, STABLE_ERR_STABLE);
            return;
        }
        self.unsummon_pet(pet, row.slot);
        if self.load_pet_by_number(player, pet_number) {
            self.send_stable_result(player, STABLE_SUCCESS_UNSTABLE);
        } else {
            self.send_stable_result(player, STABLE_ERR_STABLE);
        }
    }
}
