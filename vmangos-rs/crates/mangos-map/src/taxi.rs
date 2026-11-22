//! Taxi (flight master) services (Tier 4 step 4f),
//! transcribed from the fork:
//! - `TaxiHandler.cpp` (status query, available
//!   nodes / learn node, menu, activate, express)
//! - `Player::ActivateTaxiPathTo` /
//!   `ContinueTaxiFlight` / `TaxiStepFinished`
//!   (`Player.cpp:17709-18025`, `21668-21745`)
//! - `FlightPathMovementGenerator` landing logic
//!   (`WaypointMovementGenerator.cpp:347-422`; the
//!   spline driving itself lives in `motion.rs`)
//!
//! Recorded remainders:
//! - UNMODELED: `TradeCancel` at flight start
//!   (trade state is actor-side; the client cancels
//!   trade when the taxi map opens).
//! - UNMODELED: anticheat report on unknown-node
//!   activation attempts (fork logs/reports only).
//! - UNMODELED: pet reagent return on the
//!   flight-start unsummon (`PET_SAVE_REAGENTS`
//!   collapses to `PET_SAVE_NOT_IN_SLOT` here).
//! - UNMODELED: landing honorless-target cast in
//!   PvP-enforced areas and the hostile-ref
//!   online/offline toggle during flight.

use mangos_entities::creature::{
    DisplayResolver, choose_display_id_for_template, random_gender_display,
};
use mangos_entities::unit::{Team, UNIT_STATE_FEIGN_DEATH, UNIT_STATE_TAXI_FLIGHT};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::{MovementFlags, Position};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::aura as aura_type;
use mangos_world_data::taxi::{TAXI_MASK_SIZE, TaxiPathNode};

use crate::map::{Map, MapEvent};
use crate::motion::{UNIT_FLAG_REMOVE_CLIENT_CONTROL, UNIT_FLAG_TAXI_FLIGHT};

/// `UNIT_NPC_FLAG_FLIGHTMASTER` (`UnitDefines.h:656`).
pub(crate) const NPC_FLAG_FLIGHTMASTER: u32 = 0x0000_0008;
/// `UNIT_FLAG_REMOVE_CLIENT_CONTROL` gate bit read
/// by `ActivateTaxiPathTo`.
const PLAYER_EXTRA_TAXICHEAT: u32 = 0x0008;
/// `PET_SAVE_NOT_IN_SLOT`.
const PET_SAVE_NOT_IN_SLOT: u32 = 100;
/// `INTERACTION_DISTANCE`.
const INTERACTION_DISTANCE: f32 = 5.0;
// `ActivateTaxiReplies` (`Player.h:402-416`).
const ERR_TAXIOK: u32 = 0;
const ERR_TAXIUNSPECIFIEDSERVERERROR: u32 = 1;
const ERR_TAXINOSUCHPATH: u32 = 2;
const ERR_TAXINOTENOUGHMONEY: u32 = 3;
const ERR_TAXITOOFARAWAY: u32 = 4;
const ERR_TAXIPLAYERBUSY: u32 = 7;
const ERR_TAXIPLAYERALREADYMOUNTED: u32 = 8;
const ERR_TAXIPLAYERSHAPESHIFTED: u32 = 9;

/// Mask field/bit for a 1-based taxi node id.
fn node_mask(node: u32) -> (usize, u32) {
    let idx = node.saturating_sub(1);
    ((idx / 32) as usize, 1u32 << (idx % 32))
}

impl Map {
    fn taxi_is_cheater(&self, guid: ObjectGuid) -> bool {
        self.players
            .get(&guid)
            .is_some_and(|e| e.player.extra_flags & PLAYER_EXTRA_TAXICHEAT != 0)
    }

    fn taxi_node_known(&self, guid: ObjectGuid, node: u32) -> bool {
        let (field, submask) = node_mask(node);
        self.players.get(&guid).is_some_and(|e| {
            e.player.taxi_mask.get(field).copied().unwrap_or(0) & submask == submask
        })
    }

    /// `SetTaximaskNode`: true when newly learned.
    fn taxi_set_node_known(&mut self, guid: ObjectGuid, node: u32) -> bool {
        let (field, submask) = node_mask(node);
        let Some(entry) = self.players.get_mut(&guid) else {
            return false;
        };
        if field >= entry.player.taxi_mask.len() {
            entry.player.taxi_mask.resize(TAXI_MASK_SIZE, 0);
        }
        if entry.player.taxi_mask[field] & submask == submask {
            return false;
        }
        entry.player.taxi_mask[field] |= submask;
        true
    }

    fn player_alliance(&self, guid: ObjectGuid) -> bool {
        self.players
            .get(&guid)
            .is_some_and(|e| e.player.team == Team::Alliance)
    }

    /// Nearest taxi node to a creature (fork handlers
    /// always resolve through the flight master's
    /// position).
    fn nearest_taxi_node_for(&self, guid: ObjectGuid, npc: ObjectGuid) -> u32 {
        let Some(centry) = self.creatures.get(&npc) else {
            return 0;
        };
        let pos = centry.unit.world.position();
        self.stores.taxi.nearest_taxi_node(
            pos.x,
            pos.y,
            pos.z,
            self.map_id,
            self.player_alliance(guid),
        )
    }

    /// `WorldSession::SendTaxiStatus`.
    pub fn send_taxi_status(&mut self, guid: ObjectGuid, npc: ObjectGuid) {
        if !self.creatures.contains_key(&npc) {
            return;
        }
        let curloc = self.nearest_taxi_node_for(guid, npc);
        if curloc == 0 {
            return;
        }
        let mut writer = PacketWriter::new();
        writer.write_u64_le(npc.raw());
        writer.write_u8(u8::from(self.taxi_node_known(guid, curloc)));
        let packet = WorldPacket::with_body(opcode_id::SMSG_TAXINODE_STATUS, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `WorldSession::SendLearnNewTaxiNode`: true
    /// when the node was learned (or none exists),
    /// suppressing the menu.
    pub(crate) fn send_learn_new_taxi_node(&mut self, guid: ObjectGuid, npc: ObjectGuid) -> bool {
        let curloc = self.nearest_taxi_node_for(guid, npc);
        if curloc == 0 {
            return true;
        }
        if self.taxi_set_node_known(guid, curloc) {
            let packet = WorldPacket::with_body(opcode_id::SMSG_NEW_TAXI_PATH, Vec::new());
            self.send_to_player(guid, packet);
            let mut writer = PacketWriter::new();
            writer.write_u64_le(npc.raw());
            writer.write_u8(1);
            let packet = WorldPacket::with_body(opcode_id::SMSG_TAXINODE_STATUS, writer.into_vec());
            self.send_to_player(guid, packet);
            return true;
        }
        false
    }

    /// `WorldSession::SendTaxiMenu`.
    pub(crate) fn send_taxi_menu(&mut self, guid: ObjectGuid, npc: ObjectGuid) {
        let curloc = self.nearest_taxi_node_for(guid, npc);
        if curloc == 0 {
            return;
        }
        let mut writer = PacketWriter::new();
        writer.write_u32_le(1);
        writer.write_u64_le(npc.raw());
        writer.write_u32_le(curloc);
        if self.taxi_is_cheater(guid) {
            for word in self.stores.taxi.nodes_mask {
                writer.write_u32_le(word);
            }
        } else {
            let mask: [u32; TAXI_MASK_SIZE] =
                self.players.get(&guid).map_or([0; TAXI_MASK_SIZE], |e| {
                    let mut out = [0u32; TAXI_MASK_SIZE];
                    for (slot, word) in out.iter_mut().zip(e.player.taxi_mask.iter()) {
                        *slot = *word;
                    }
                    out
                });
            for word in mask {
                writer.write_u32_le(word);
            }
        }
        let packet = WorldPacket::with_body(opcode_id::SMSG_SHOWTAXINODES, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `WorldSession::HandleTaxiQueryAvailableNodes`.
    pub fn taxi_query_available_nodes(&mut self, guid: ObjectGuid, npc: ObjectGuid) {
        if !self.npc_if_can_interact_with(guid, npc, NPC_FLAG_FLIGHTMASTER) {
            return;
        }
        if self
            .unit_base(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_FEIGN_DEATH))
        {
            self.remove_spells_causing_aura(guid, aura_type::FEIGN_DEATH);
        }
        if self.send_learn_new_taxi_node(guid, npc) {
            return;
        }
        self.send_taxi_menu(guid, npc);
    }

    /// `GetTaxiMountDisplayId` display resolution.
    fn taxi_mount_display_id(&mut self, node_id: u32, guid: ObjectGuid, alt_team: bool) -> u32 {
        let creature_id =
            self.stores
                .taxi
                .mount_creature_id(node_id, self.player_alliance(guid), alt_team);
        if creature_id == 0 {
            return 0;
        }
        let Some(info) = self.stores.creature_templates.get(creature_id).cloned() else {
            return 0;
        };
        let stores = &self.stores;
        let resolver = DisplayResolver {
            scale_for_display: &|id| stores.scale_for_display(id),
            model_for_display: &|id| stores.model_for_display(id),
        };
        let (display, _) = choose_display_id_for_template(&info, &resolver, &mut self.rng);
        if display == 0 {
            return 0;
        }
        random_gender_display(&self.stores.display_addons, display, &mut self.rng)
    }

    /// `Player::Mount(display)` taxi arm: mount
    /// display + mount-cancel aura strip + temporary
    /// pet disable (the codebase's mount pet model).
    fn taxi_mount(&mut self, guid: ObjectGuid, display: u32) {
        self.interrupt_spells_with_channel_flags(
            guid,
            mangos_spells::defines::aura_interrupt_flag::MOUNT_CANCELS,
        );
        self.remove_auras_with_interrupt_flags(
            guid,
            mangos_spells::defines::aura_interrupt_flag::MOUNT_CANCELS,
            0,
        );
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values()
                .set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, display);
        }
        self.pet_mount_transition(guid, true);
    }

    /// `WorldSession::SendDoFlight`.
    pub(crate) fn send_do_flight(
        &mut self,
        guid: ObjectGuid,
        mount_display_id: u32,
        path: u32,
        path_node: u32,
    ) {
        if self
            .unit_base(guid)
            .is_some_and(|u| u.has_unit_state(UNIT_STATE_FEIGN_DEATH))
        {
            self.remove_spells_causing_aura(guid, aura_type::FEIGN_DEATH);
        }
        self.motion_expire_flight(guid);
        if mount_display_id != 0 {
            self.taxi_mount(guid, mount_display_id);
        }
        let stitched = self
            .players
            .get(&guid)
            .is_some_and(|e| !e.player.taxi_path_nodes.is_empty());
        if stitched {
            let nodes = self
                .players
                .get(&guid)
                .map_or_else(Vec::new, |e| e.player.taxi_path_nodes.clone());
            self.player_move_taxi_flight(guid, nodes, 0);
        } else {
            let nodes = self.stores.taxi.path_nodes(path).to_vec();
            self.player_move_taxi_flight(guid, nodes, path_node);
        }
    }

    fn send_taxi_reply(&mut self, guid: ObjectGuid, code: u32) {
        let mut writer = PacketWriter::new();
        writer.write_u32_le(code);
        let packet = WorldPacket::with_body(opcode_id::SMSG_ACTIVATETAXIREPLY, writer.into_vec());
        self.send_to_player(guid, packet);
    }

    /// `Unit::IsInDisallowedMountForm` form arm (the
    /// display/model arm is unmodeled: native display
    /// is the only reachable transformed state at
    /// taxi activation here).
    fn in_disallowed_mount_form(&self, guid: ObjectGuid) -> bool {
        let form = self.unit_base(guid).map_or(0, |u| {
            u.world
                .object
                .values()
                .get_byte(offsets::UNIT_FIELD_BYTES_1, 2)
        });
        // FORM_NONE, battle/defensive/berserker
        // stance, shadow, stealth are allowed.
        !matches!(form, 0 | 0x11 | 0x12 | 0x13 | 0x1C | 0x1E)
    }

    fn clear_taxi_destinations(&mut self, guid: ObjectGuid) {
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.taxi_destinations.clear();
            entry.player.taxi_path_nodes.clear();
            entry.player.taxi_discount = 1.0;
        }
    }

    /// `PlayerTaxi::GetCurrentTaxiCost` with the
    /// stored destinations/discount.
    fn current_taxi_cost(&self, guid: ObjectGuid) -> u32 {
        let Some(entry) = self.players.get(&guid) else {
            return 0;
        };
        let dests = &entry.player.taxi_destinations;
        if dests.len() < 2 {
            return 0;
        }
        let (_, cost) = self.stores.taxi.taxi_path(dests[0], dests[1]);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let charged = (cost as f32 * entry.player.taxi_discount + 0.5) as u32;
        charged
    }

    /// Flight-generator boundary charge
    /// (`NextTaxiDestination` + `ModifyMoney`).
    pub(crate) fn taxi_next_leg_charge(&mut self, guid: ObjectGuid) {
        if let Some(entry) = self.players.get_mut(&guid)
            && !entry.player.taxi_destinations.is_empty()
        {
            entry.player.taxi_destinations.remove(0);
        }
        let cost = self.current_taxi_cost(guid);
        self.modify_money(guid, -i64::from(cost));
    }

    /// Packet-handler arm (`HandleActivateTaxiOpcode`
    /// / express): interact gate, then activation.
    pub fn activate_taxi_from_npc(&mut self, guid: ObjectGuid, nodes: &[u32], npc: ObjectGuid) {
        if !self.npc_if_can_interact_with(guid, npc, NPC_FLAG_FLIGHTMASTER) {
            return;
        }
        self.activate_taxi_path_to(guid, nodes, Some(npc), 0, false);
    }

    /// `Player::ActivateTaxiPathTo(nodes, npc, spellid, nocheck)`.
    /// `npc` is `None` for the spell/script arm.
    #[allow(clippy::too_many_lines)] // one ActivateTaxiPathTo port
    pub fn activate_taxi_path_to(
        &mut self,
        guid: ObjectGuid,
        nodes: &[u32],
        npc: Option<ObjectGuid>,
        spell_id: u32,
        nocheck: bool,
    ) {
        if nodes.len() < 2 {
            return;
        }
        let in_combat = self
            .unit_base(guid)
            .is_some_and(mangos_entities::unit::UnitBase::is_in_combat);
        if in_combat {
            self.send_taxi_reply(guid, ERR_TAXIPLAYERBUSY);
            return;
        }
        if self.unit_base(guid).is_some_and(|u| {
            u.world
                .object
                .values()
                .has_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_REMOVE_CLIENT_CONTROL)
        }) {
            return;
        }
        if !self.taxi_is_cheater(guid) && !nocheck {
            for &node in nodes {
                if !self.taxi_node_known(guid, node) {
                    return;
                }
            }
        }
        if npc.is_some() {
            let mounted = self.unit_base(guid).is_some_and(|u| {
                u.world
                    .object
                    .values()
                    .get_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID)
                    != 0
            });
            if mounted {
                self.send_taxi_reply(guid, ERR_TAXIPLAYERALREADYMOUNTED);
                return;
            }
            if self.in_disallowed_mount_form(guid) {
                self.send_taxi_reply(guid, ERR_TAXIPLAYERSHAPESHIFTED);
                return;
            }
            if self.casts.contains_key(&guid) {
                self.send_taxi_reply(guid, ERR_TAXIPLAYERBUSY);
                return;
            }
        } else {
            self.remove_spells_causing_aura(guid, aura_type::MOUNTED);
            if self.in_disallowed_mount_form(guid) {
                self.remove_spells_causing_aura(guid, aura_type::MOD_SHAPESHIFT);
                self.remove_spells_causing_aura(guid, aura_type::TRANSFORM);
            }
            // Fork interrupts generic/channeled casts
            // other than the sending spell; scripted
            // SEND_TAXI casts run this from inside the
            // spell's own effect, so only channeled
            // casts are interrupted here (a generic
            // cast in progress is the sender itself).
            let channeling = self.casts.get(&guid).is_some_and(|c| c.channeling);
            if channeling {
                self.cancel_cast(guid, true);
            }
        }

        let source_node = nodes[0];
        let Some(node) = self.stores.taxi.node(source_node).copied() else {
            self.send_taxi_reply(guid, ERR_TAXINOSUCHPATH);
            return;
        };
        if node.x != 0.0 || node.y != 0.0 || node.z != 0.0 {
            let pos = self
                .unit_base(guid)
                .map_or(Position::default(), |u| u.world.position());
            let dist2 =
                (node.x - pos.x).powi(2) + (node.y - pos.y).powi(2) + (node.z - pos.z).powi(2);
            // Fork compares against `(2*D)^3` (sic).
            let limit = (2.0 * INTERACTION_DISTANCE).powi(3);
            if node.map_id != self.map_id || dist2 > limit {
                self.send_taxi_reply(guid, ERR_TAXITOOFARAWAY);
                return;
            }
        } else if npc.is_some() {
            self.send_taxi_reply(guid, ERR_TAXIUNSPECIFIEDSERVERERROR);
            return;
        }

        self.combat_stop(guid, false);
        self.clear_taxi_destinations(guid);
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.taxi_destinations.push(source_node);
        }
        let discount = npc.map_or(1.0, |npc| self.reputation_price_discount(guid, npc));
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.taxi_discount = discount;
        }

        let (source_path, source_cost) = self.stores.taxi.taxi_path(source_node, nodes[1]);
        if source_path == 0 {
            self.clear_taxi_destinations(guid);
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let source_cost = (source_cost as f32 * discount + 0.5) as u32;
        let mut total_cost = source_cost;
        let mut last_path = source_path;
        let mut last_node = nodes[1];

        if nodes.len() > 2 {
            let mut last_out_node: u32 = 0;
            for &next_node in &nodes[2..] {
                let (next_path, next_cost) = self.stores.taxi.taxi_path(last_node, next_node);
                if next_path == 0 {
                    self.clear_taxi_destinations(guid);
                    return;
                }
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                {
                    total_cost += (next_cost as f32 * discount + 0.5) as u32;
                }
                let transition = self.stores.taxi.transition(last_path, next_path).copied();
                let last_vertices = self.stores.taxi.path_nodes(last_path).to_vec();
                let in_node = transition.map_or_else(
                    || u32::try_from(last_vertices.len().saturating_sub(2)).unwrap_or(0),
                    |t| t.in_node,
                );
                let out_node = transition.map_or(1, |t| t.out_node);
                if let Some(entry) = self.players.get_mut(&guid) {
                    for vertex in last_vertices
                        .iter()
                        .skip(last_out_node as usize)
                        .take((in_node as usize + 1).saturating_sub(last_out_node as usize))
                    {
                        entry.player.taxi_path_nodes.push(*vertex);
                    }
                    entry.player.taxi_destinations.push(last_node);
                }
                last_node = next_node;
                last_path = next_path;
                last_out_node = out_node;
            }
            let last_vertices = self.stores.taxi.path_nodes(last_path).to_vec();
            if let Some(entry) = self.players.get_mut(&guid) {
                for vertex in last_vertices.iter().skip(last_out_node as usize) {
                    entry.player.taxi_path_nodes.push(*vertex);
                }
                entry.player.taxi_destinations.push(last_node);
            }
        } else if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.taxi_destinations.push(last_node);
        }

        let mount_display_id = self.taxi_mount_display_id(source_node, guid, npc.is_none());
        if mount_display_id == 0 && spell_id == 0 {
            self.send_taxi_reply(guid, ERR_TAXIUNSPECIFIEDSERVERERROR);
            self.clear_taxi_destinations(guid);
            return;
        }
        if self.player_money(guid) < total_cost {
            self.send_taxi_reply(guid, ERR_TAXINOTENOUGHMONEY);
            self.clear_taxi_destinations(guid);
            return;
        }
        self.modify_money(guid, -i64::from(source_cost));
        self.remove_spells_causing_aura(guid, aura_type::MOD_STEALTH);
        self.unsummon_owner_pet(guid, PET_SAVE_NOT_IN_SLOT);
        self.send_taxi_reply(guid, ERR_TAXIOK);
        self.send_do_flight(guid, mount_display_id, source_path, 0);
    }

    /// `Player::ContinueTaxiFlight` (login resume).
    pub fn continue_taxi_flight(&mut self, guid: ObjectGuid) {
        let (source, dest) = {
            let Some(entry) = self.players.get(&guid) else {
                return;
            };
            let dests = &entry.player.taxi_destinations;
            if dests.len() < 2 {
                return;
            }
            (dests[0], dests[1])
        };
        if source == 0 {
            return;
        }
        let mount_display_id = self.taxi_mount_display_id(source, guid, true);
        let (path, _) = self.stores.taxi.taxi_path(source, dest);
        if path == 0 {
            return;
        }
        let node_list = self.stores.taxi.path_nodes(path).to_vec();
        if node_list.is_empty() {
            return;
        }
        let pos = self
            .unit_base(guid)
            .map_or(Position::default(), |u| u.world.position());
        let dist2 = |n: &TaxiPathNode| {
            (n.x - pos.x).powi(2) + (n.y - pos.y).powi(2) + (n.z - pos.z).powi(2)
        };
        let mut start_node: u32 = 0;
        let mut dist_prev = f32::MAX;
        let mut dist_next = dist2(&node_list[0]);
        for i in 1..node_list.len() {
            let node = &node_list[i];
            if node.map_id != self.map_id {
                continue;
            }
            let prev = &node_list[i - 1];
            dist_prev = dist_next;
            dist_next = dist2(node);
            let dist_nodes =
                (node.x - prev.x).powi(2) + (node.y - prev.y).powi(2) + (node.z - prev.z).powi(2);
            if dist_next + dist_prev < dist_nodes {
                start_node = u32::try_from(i).unwrap_or(0);
                break;
            }
        }
        let _ = dist_prev;
        self.send_do_flight(guid, mount_display_id, path, start_node);
    }

    /// `FlightPathMovementGenerator::Finalize` +
    /// `Player::TaxiStepFinished`.
    pub(crate) fn taxi_flight_finalize(
        &mut self,
        guid: ObjectGuid,
        nodes: &[TaxiPathNode],
        current: u32,
        _in_progress: bool,
    ) {
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.clear_unit_state(UNIT_STATE_TAXI_FLIGHT);
            unit.world.movement_info.flags.remove(MovementFlags::FLYING);
            unit.values().set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, 0);
            unit.values().remove_flag(
                offsets::UNIT_FIELD_FLAGS,
                UNIT_FLAG_REMOVE_CLIENT_CONTROL | UNIT_FLAG_TAXI_FLIGHT,
            );
        }
        self.taxi_step_finished(guid, nodes, current);
        let empty = self
            .players
            .get(&guid)
            .is_some_and(|e| e.player.taxi_destinations.is_empty());
        if empty {
            self.stop_moving(guid);
            self.clear_taxi_destinations(guid);
        }
    }

    /// `Player::TaxiStepFinished`.
    fn taxi_step_finished(&mut self, guid: ObjectGuid, nodes: &[TaxiPathNode], current: u32) {
        let cur_dest = self.players.get(&guid).map_or(0, |e| {
            e.player.taxi_destinations.get(1).copied().unwrap_or(0)
        });
        if cur_dest == 0 {
            return;
        }
        let cur_dest_node = self.stores.taxi.node(cur_dest).copied();
        // Far-teleport case: the destination node is
        // on another map — hop to the first vertex
        // there and resume after the transfer.
        if let Some(dest_node) = cur_dest_node
            && dest_node.map_id != self.map_id
        {
            // `SetCurrentNodeAfterTeleport`.
            let first_map = nodes.first().map_or(self.map_id, |n| n.map_id);
            let mut resume = current as usize;
            for (i, node) in nodes.iter().enumerate().skip(1) {
                if node.map_id != first_map {
                    resume = i;
                    break;
                }
            }
            let Some(target) = nodes.get(resume).copied() else {
                return;
            };
            // Stash the flight state on the player for
            // the destination map (fork keeps the same
            // generator across the worldport).
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.player.taxi_path_nodes = nodes.to_vec();
            }
            let orientation = self.unit_base(guid).map_or(0.0, |u| u.world.position().o);
            self.events.push(MapEvent::TeleportPlayer {
                player: guid,
                map_id: dest_node.map_id,
                position: Position {
                    x: target.x,
                    y: target.y,
                    z: target.z,
                    o: orientation,
                },
            });
            // Resume index rides the destinations-adjacent
            // stash: encode via a dedicated event below.
            self.events.push(MapEvent::TaxiResume {
                player: guid,
                resume_node: u32::try_from(resume + 1).unwrap_or(0),
            });
            return;
        }

        // `NextTaxiDestination`: pop the finished
        // source; the return is the new `GetTaxiDestination`
        // (index 1 of the remaining list, 0 when fewer
        // than two remain — the flight is over and the
        // single remaining entry stays, as in the fork).
        if let Some(entry) = self.players.get_mut(&guid)
            && !entry.player.taxi_destinations.is_empty()
        {
            entry.player.taxi_destinations.remove(0);
        }
        let next_dest = self.players.get(&guid).map_or(0, |e| {
            e.player.taxi_destinations.get(1).copied().unwrap_or(0)
        });
        if next_dest == 0 {
            return;
        }
        let source = self
            .players
            .get(&guid)
            .and_then(|e| e.player.taxi_destinations.first().copied())
            .unwrap_or(0);
        if self.taxi_is_cheater(guid) && self.taxi_set_node_known(guid, source) {
            let packet = WorldPacket::with_body(opcode_id::SMSG_NEW_TAXI_PATH, Vec::new());
            self.send_to_player(guid, packet);
        }
        let mount_display_id = self.taxi_mount_display_id(source, guid, false);
        let (path, _) = self.stores.taxi.taxi_path(source, next_dest);
        if path != 0 && mount_display_id != 0 {
            self.send_do_flight(guid, mount_display_id, path, 1);
        } else {
            self.clear_taxi_destinations(guid);
        }
    }

    /// Worldport landing half of a cross-map flight
    /// (fork `flight->Reset` in
    /// `HandleMoveWorldportAckOpcode`): relaunch the
    /// generator from the stashed vertex list.
    pub fn taxi_resume_after_teleport(&mut self, guid: ObjectGuid, resume_node: u32) {
        let nodes = self
            .players
            .get(&guid)
            .map_or_else(Vec::new, |e| e.player.taxi_path_nodes.clone());
        if nodes.is_empty() {
            return;
        }
        // The old map's finalize unmounted; the fork
        // generator (and its mount) survives the
        // worldport, so re-apply the mount display.
        let source = self
            .players
            .get(&guid)
            .and_then(|e| e.player.taxi_destinations.first().copied())
            .unwrap_or(0);
        let mount_display_id = self.taxi_mount_display_id(source, guid, true);
        if mount_display_id != 0 {
            self.taxi_mount(guid, mount_display_id);
        }
        self.player_move_taxi_flight(guid, nodes, resume_node);
    }
}
