//! Player death loop (`Player::KillPlayer`,
//! `BuildPlayerRepop`, `RepopAtGraveyard`,
//! `ResurrectPlayer`, corpse reclaim + spirit
//! resurrection): death timer, ghost form, corpse
//! objects, graveyard teleport, durability loss.
//!
//! Unmodeled (recorded in TIER4-PROGRESS): the
//! one-tick `JUST_DIED` intermediate before
//! `KillPlayer`, `HasPendingMovementChange` repop
//! deferral, spirit-of-redemption/soulstone
//! self-resurrect arms, pet-kill PvP-death flavor,
//! BG insignia looting of corpses, and the in-instance entrance-corpse
//! reclaim shortcut (ghost re-enters the instance
//! portal to resurrect — reclaim here needs the
//! corpse on the ghost's current map).

use mangos_entities::corpse::{
    CORPSE_FLAG_HIDE_CLOAK, CORPSE_FLAG_HIDE_HELM, CORPSE_FLAG_LOOTABLE, CORPSE_FLAG_UNK2, Corpse,
    CorpseAppearance, CorpseType,
};
use mangos_entities::unit::{DeathState, Team};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::battlegrounds::WorldSafeLoc;

use crate::map::{Map, MapEvent};

/// `CORPSE_REPOP_TIME` (`Player.h`): forced repop
/// after 6 minutes in `CORPSE` state.
const CORPSE_REPOP_TIME_MS: u32 = 360_000;
/// `CORPSE_RECLAIM_RADIUS` (`Player.h:141`).
const CORPSE_RECLAIM_RADIUS: f32 = 39.0;
/// `DEATH_EXPIRE_STEP` (5 minutes, seconds).
const DEATH_EXPIRE_STEP: u64 = 300;
/// `MAX_DEATH_COUNT`.
const MAX_DEATH_COUNT: u64 = 3;
/// `copseReclaimDelay` seconds per death count.
const CORPSE_RECLAIM_DELAY: [u32; 3] = [30, 60, 120];
/// Forced repop → bones expiry for resurrectable
/// corpses (`Corpse::IsExpired`: 3 days).
const CORPSE_EXPIRE_SECS: u64 = 3 * 86_400;
const SPELL_GHOST: u32 = 8326;
const SPELL_WISP_SPIRIT: u32 = 20585;
const SPELL_WISP_SPIRIT_GHOST: u32 = 20584;
const SPELL_RESURRECTION_SICKNESS: u32 = 15007;
/// `PLAYER_EXTRA_PVP_DEATH` (`AbstractPlayer.h:22`).
const PLAYER_EXTRA_PVP_DEATH: u32 = 0x0100;
/// `PLAYER_FLAGS_GHOST`.
const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;
/// `PLAYER_FLAGS_HIDE_HELM` / `_HIDE_CLOAK`.
const PLAYER_FLAGS_HIDE_HELM: u32 = 0x0000_0400;
const PLAYER_FLAGS_HIDE_CLOAK: u32 = 0x0000_0800;
/// `UNIT_NPC_FLAG_SPIRITHEALER` (`UnitDefines.h:658`;
/// templates carry 33 = gossip | spirit healer).
const NPC_FLAG_SPIRIT_HEALER: u32 = 0x0000_0020;

impl Map {
    /// Seeds `GenerateCorpseLowGuid` past the
    /// persisted table max (actor calls at map
    /// creation).
    pub fn seed_corpse_counter(&mut self, max_persisted: u32) {
        self.corpse_counter = self.corpse_counter.max(max_persisted);
    }

    fn next_corpse_low_guid(&mut self) -> u32 {
        self.corpse_counter = self.corpse_counter.saturating_add(1).max(1);
        self.corpse_counter
    }

    /// Inserts a corpse object into the map grids
    /// (`ObjectAccessor::AddCorpse` world half).
    pub fn add_corpse(&mut self, corpse: Corpse) {
        let guid = corpse.guid();
        let cell = Self::cell_of(corpse.world.position());
        self.cells.entry(cell).or_default().corpses.insert(guid);
        self.corpses.insert(guid, corpse);
        for entry in self.players.values_mut() {
            entry.scan_due = true;
        }
    }

    /// Removes a corpse/bones object and destroys it
    /// for current viewers.
    fn remove_corpse_object(&mut self, guid: ObjectGuid) {
        let Some(corpse) = self.corpses.remove(&guid) else {
            return;
        };
        let cell = Self::cell_of(corpse.world.position());
        if let Some(objects) = self.cells.get_mut(&cell) {
            objects.corpses.remove(&guid);
        }
        let viewers: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, e)| e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        for viewer in viewers {
            if let Some(entry) = self.players.get_mut(&viewer) {
                entry.visible.remove(&guid);
                entry.scan_due = true;
            }
        }
    }

    /// The resurrectable corpse owned by a player on
    /// this map (`Player::GetCorpse` map-local half).
    pub fn corpse_for_player(&self, player: ObjectGuid) -> Option<ObjectGuid> {
        self.corpses
            .values()
            .find(|c| c.owner == player && c.corpse_type != CorpseType::Bones)
            .map(Corpse::guid)
    }

    /// Corpse lookup for the actor's cross-map spirit
    /// healer flow: `(position, zone_id, area_id)` of
    /// the player's resurrectable corpse here.
    pub fn corpse_location_for(&mut self, player: ObjectGuid) -> Option<(Position, u32, u32)> {
        let corpse = self
            .corpses
            .values()
            .find(|c| c.owner == player && c.corpse_type != CorpseType::Bones)?;
        let pos = corpse.world.position();
        let (zone, area) = self.zone_area_at(pos);
        Some((pos, zone, area))
    }

    /// `ConvertCorpseForPlayer` on this map: bones
    /// replace the resurrectable corpse (world/BG
    /// config gates) and the persisted row dies.
    pub fn spawn_corpse_bones_for(&mut self, player: ObjectGuid) {
        let Some(guid) = self.corpse_for_player(player) else {
            return;
        };
        let spawns_bones = if self.bg.is_some() {
            self.config.death_bones_bg
        } else {
            self.config.death_bones_world
        };
        let bones = self.corpses.get(&guid).map(|corpse| {
            let mut bones = corpse.to_bones();
            bones.ghost_time = self.now_unix;
            bones
        });
        self.remove_corpse_object(guid);
        self.events.push(MapEvent::CorpseDelete {
            player_low: player.counter(),
        });
        if spawns_bones && let Some(bones) = bones {
            self.add_corpse(bones);
        }
    }

    /// `Player::KillPlayer` + the `Unit::Kill`
    /// durability arm, run from `kill_unit` for player
    /// victims: `PvP`-death flavor, 10% durability loss
    /// (creature kills only, never in battlegrounds),
    /// `CORPSE` state with the 6-minute forced-repop
    /// timer, reclaim-delay bookkeeping.
    pub(crate) fn kill_player(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let killer_is_player_side = killer.is_some_and(|k| {
            k.is_player() || self.pets.get(&k).is_some_and(|pet| pet.owner.is_player())
        });
        let in_bg = self.bg.is_some();
        if !in_bg
            && killer.is_some_and(|k| k != victim)
            && !killer_is_player_side
            && killer.is_some_and(ObjectGuid::is_creature)
        {
            self.durability_loss_all(victim, 0.10, false);
            let packet = WorldPacket::new(op::SMSG_DURABILITY_DAMAGE_DEATH);
            self.events.push(MapEvent::Packet { to: victim, packet });
        }
        let instanceable = self.stores.maps.is_instance_map(self.map_id);
        let Some(player) = self.player_mut(victim) else {
            return;
        };
        if killer_is_player_side {
            player.extra_flags |= PLAYER_EXTRA_PVP_DEATH;
        } else {
            player.extra_flags &= !PLAYER_EXTRA_PVP_DEATH;
        }
        player.unit.death_state = DeathState::Corpse;
        player.death_timer_ms = CORPSE_REPOP_TIME_MS;
        // Fork `KillPlayer` field writes: dynamic
        // flags cleared, and the release-timer byte
        // (`PLAYER_FIELD_BYTE_RELEASE_TIMER`, 0x08 at
        // `PLAYER_FIELD_BYTES` offset 0) shown on
        // non-instanceable maps.
        let values = player.unit.world.object.values_mut();
        values.set_u32(offsets::UNIT_DYNAMIC_FLAGS, 0);
        if instanceable {
            values.remove_byte_flag(offsets::PLAYER_FIELD_BYTES, 0, 0x08);
        } else {
            values.set_byte_flag(offsets::PLAYER_FIELD_BYTES, 0, 0x08);
        }
        self.update_corpse_reclaim_delay(victim);
    }

    /// The `CORPSE`-state arm of `Player::Update`:
    /// count the death timer down, then build the
    /// repop and run the graveyard teleport.
    pub(crate) fn update_death_timer(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let due = self.players.get(&guid).is_some_and(|entry| {
            entry.player.unit.death_state == DeathState::Corpse && self.bg.is_none()
        });
        if !due {
            return;
        }
        let expired = {
            let Some(player) = self.player_mut(guid) else {
                return;
            };
            if diff_ms >= player.death_timer_ms {
                true
            } else {
                player.death_timer_ms -= diff_ms;
                false
            }
        };
        if expired {
            self.build_player_repop(guid);
        }
    }

    /// `HandleRepopRequestOpcode` world arm (release
    /// spirit): only a `CORPSE`-state player outside
    /// battlegrounds repops here; BG release rides
    /// the battleground's own ghost wave.
    pub fn repop_request(&mut self, guid: ObjectGuid) {
        if self.bg.is_some() {
            return;
        }
        let is_corpse = self
            .players
            .get(&guid)
            .is_some_and(|e| e.player.unit.death_state == DeathState::Corpse);
        if is_corpse {
            self.build_player_repop(guid);
        }
    }

    /// `Player::BuildPlayerRepop` + `RepopAtGraveyard`
    /// world arm: ghost form, corpse creation,
    /// reclaim-delay notice, `DEAD` state, then the
    /// graveyard teleport (actor-completed via
    /// `MapEvent::TeleportPlayer`).
    fn build_player_repop(&mut self, guid: ObjectGuid) {
        self.apply_ghost_form(guid);
        self.create_corpse_for(guid);
        if let Some(player) = self.player_mut(guid) {
            player.unit.set_health(1);
            player.death_timer_ms = 0;
            player.unit.death_state = DeathState::Dead;
        }
        self.send_corpse_reclaim_delay(guid);
        self.repop_at_graveyard(guid);
    }

    /// `Player::ApplyGhostForm` (+ wisp spirit for
    /// night elves) and the water-walking flag.
    fn apply_ghost_form(&mut self, guid: ObjectGuid) {
        if self.player_knows_spell(guid, SPELL_WISP_SPIRIT) {
            self.apply_aura_by_id(guid, SPELL_WISP_SPIRIT_GHOST, guid);
        }
        self.apply_aura_by_id(guid, SPELL_GHOST, guid);
        self.set_movement_flag(
            guid,
            mangos_movement::movement_info::MovementFlags::WATER_WALKING,
            true,
        );
    }

    /// `Player::RemoveGhostForm`.
    fn remove_ghost_form(&mut self, guid: ObjectGuid) {
        if self.player_knows_spell(guid, SPELL_WISP_SPIRIT) {
            self.remove_auras_due_to_spell(guid, SPELL_WISP_SPIRIT_GHOST);
        }
        self.remove_auras_due_to_spell(guid, SPELL_GHOST);
        self.set_movement_flag(
            guid,
            mangos_movement::movement_info::MovementFlags::WATER_WALKING,
            false,
        );
    }

    /// `Player::CreateCorpse`: bones for any previous
    /// corpse, appearance/equipment field fill,
    /// persistence emit, map insertion.
    fn create_corpse_for(&mut self, guid: ObjectGuid) {
        self.spawn_corpse_bones_for(guid);
        let Some((appearance, corpse_type, position)) = self.players.get(&guid).map(|entry| {
            let player = &entry.player;
            let values = player.unit.world.object.values();
            let mut flags = CORPSE_FLAG_UNK2;
            if values.has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_HIDE_HELM) {
                flags |= CORPSE_FLAG_HIDE_HELM;
            }
            if values.has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_HIDE_CLOAK) {
                flags |= CORPSE_FLAG_HIDE_CLOAK;
            }
            if self.bg.is_some() {
                flags |= CORPSE_FLAG_LOOTABLE;
            }
            let mut items = [0u32; 19];
            for inv in &player.deferred.inventory {
                if inv.bag == 0 && usize::from(inv.slot) < items.len() {
                    if let Some(proto) = self.stores.items.get(inv.item.item_id) {
                        items[usize::from(inv.slot)] =
                            proto.display_id | (u32::from(proto.inventory_type) << 24);
                    }
                }
            }
            let appearance = CorpseAppearance {
                race: values.get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                gender: values.get_byte(offsets::UNIT_FIELD_BYTES_0, 2),
                skin: values.get_byte(offsets::PLAYER_BYTES, 0),
                face: values.get_byte(offsets::PLAYER_BYTES, 1),
                hair_style: values.get_byte(offsets::PLAYER_BYTES, 2),
                hair_color: values.get_byte(offsets::PLAYER_BYTES, 3),
                facial_hair: values.get_byte(offsets::PLAYER_BYTES_2, 0),
                display_id: values.get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
                guild_id: player.guild_id,
                flags,
                items,
            };
            let corpse_type = if player.extra_flags & PLAYER_EXTRA_PVP_DEATH != 0 {
                CorpseType::ResurrectablePvP
            } else {
                CorpseType::ResurrectablePvE
            };
            (appearance, corpse_type, player.unit.world.position())
        }) else {
            return;
        };
        if let Some(player) = self.player_mut(guid) {
            player.extra_flags &= !PLAYER_EXTRA_PVP_DEATH;
        }
        let guid_low = self.next_corpse_low_guid();
        let corpse = Corpse::create(
            guid_low,
            self.map_id,
            self.instance_id,
            guid,
            corpse_type,
            position,
            &appearance,
            self.now_unix,
        );
        if self.bg.is_none() {
            self.events.push(MapEvent::CorpseSave(Box::new(
                mangos_entities::corpse::CorpseSaveRow {
                    guid: guid_low,
                    player_guid: guid.counter(),
                    position_x: position.x,
                    position_y: position.y,
                    position_z: position.z,
                    orientation: position.o,
                    map: self.map_id,
                    time: self.now_unix,
                    corpse_type: corpse_type as u32,
                    instance: self.instance_id,
                },
            )));
        }
        self.add_corpse(corpse);
    }

    /// `ObjectMgr::GetClosestGraveYard` over the
    /// stored zone/area (area first, zone fallback).
    fn closest_graveyard(
        &mut self,
        pos: Position,
        team: Team,
        zone_id: u32,
        area_id: u32,
    ) -> Option<WorldSafeLoc> {
        if let Some(gy) = self.closest_graveyard_for_area(area_id, pos, team) {
            return Some(gy);
        }
        if area_id == zone_id {
            return None;
        }
        self.closest_graveyard_for_area(zone_id, pos, team)
    }

    /// `Player::RepopAtGraveyard` world arm: teleport
    /// the ghost to the closest graveyard (facing from
    /// the safe-loc store; no graveyard → stay). The
    /// actor performs the actual transfer.
    fn repop_at_graveyard(&mut self, guid: ObjectGuid) {
        let Some((pos, team, zone_id, area_id, orientation)) =
            self.players.get(&guid).map(|entry| {
                (
                    entry.player.unit.world.position(),
                    entry.player.team,
                    entry.zone_id,
                    entry.area_id,
                    entry.player.unit.world.position().o,
                )
            })
        else {
            return;
        };
        let Some(gy) = self.closest_graveyard(pos, team, zone_id, area_id) else {
            return;
        };
        let facing = if gy.orientation == 0.0 {
            orientation
        } else {
            gy.orientation
        };
        self.events.push(MapEvent::TeleportPlayer {
            player: guid,
            map_id: gy.map_id,
            position: Position::new(gy.x, gy.y, gy.z, facing),
        });
    }

    /// `Player::UpdateCorpseReclaimDelay` (death-count
    /// ladder over `m_deathExpireTime`).
    #[allow(clippy::similar_names)] // fork PvP/PvE config pair
    fn update_corpse_reclaim_delay(&mut self, guid: ObjectGuid) {
        let delay_pvp = self.config.corpse_reclaim_delay_pvp;
        let delay_pve = self.config.corpse_reclaim_delay_pve;
        let now = self.now_unix;
        let Some(player) = self.player_mut(guid) else {
            return;
        };
        let pvp = player.extra_flags & PLAYER_EXTRA_PVP_DEATH != 0;
        if (pvp && !delay_pvp) || (!pvp && !delay_pve) {
            return;
        }
        if now < player.death_expire_time {
            let count = (player.death_expire_time - now) / DEATH_EXPIRE_STEP + 1;
            player.death_expire_time = if count < MAX_DEATH_COUNT {
                now + (count + 1) * DEATH_EXPIRE_STEP
            } else {
                now + MAX_DEATH_COUNT * DEATH_EXPIRE_STEP
            };
        } else {
            player.death_expire_time = now + DEATH_EXPIRE_STEP;
        }
    }

    /// `Player::GetCorpseReclaimDelay`.
    fn corpse_reclaim_delay(&self, guid: ObjectGuid, pvp: bool) -> u32 {
        let enabled = if pvp {
            self.config.corpse_reclaim_delay_pvp
        } else {
            self.config.corpse_reclaim_delay_pve
        };
        if !enabled {
            return CORPSE_RECLAIM_DELAY[0];
        }
        let Some(entry) = self.players.get(&guid) else {
            return CORPSE_RECLAIM_DELAY[0];
        };
        let expire = entry.player.death_expire_time;
        let count = if self.now_unix < expire {
            (expire - self.now_unix) / DEATH_EXPIRE_STEP
        } else {
            0
        };
        let idx = usize::try_from(count).unwrap_or(0).min(2);
        CORPSE_RECLAIM_DELAY[idx]
    }

    /// `Player::SendCorpseReclaimDelay(false)`.
    fn send_corpse_reclaim_delay(&mut self, guid: ObjectGuid) {
        let Some(corpse_guid) = self.corpse_for_player(guid) else {
            return;
        };
        let pvp = self
            .corpses
            .get(&corpse_guid)
            .is_some_and(|c| c.corpse_type == CorpseType::ResurrectablePvP);
        let delay = self.corpse_reclaim_delay(guid, pvp);
        let mut payload = PacketWriter::new();
        payload.write_u32_le(delay * 1000);
        let packet = WorldPacket::with_body(op::SMSG_CORPSE_RECLAIM_DELAY, payload.into_vec());
        self.events.push(MapEvent::Packet { to: guid, packet });
    }

    /// `Player::ResurrectPlayer` world arm (the
    /// battleground spirit-guide wave keeps its own
    /// `bg_resurrect_player`): ghost form off, death
    /// state cleared, health/mana/energy at the
    /// restore percent, rage zeroed, optional
    /// resurrection sickness by level.
    #[allow(clippy::cast_precision_loss)]
    pub fn resurrect_player(
        &mut self,
        guid: ObjectGuid,
        restore_percent: f32,
        apply_sickness: bool,
    ) {
        self.remove_ghost_form(guid);
        let level = {
            let Some(player) = self.player_mut(guid) else {
                return;
            };
            player.unit.death_state = DeathState::Alive;
            player.death_timer_ms = 0;
            if restore_percent > 0.0 {
                let max_hp = player.unit.max_health();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                player
                    .unit
                    .set_health(((max_hp as f32 * restore_percent) as u32).max(1));
                let max_mana = player.unit.max_power(0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                player
                    .unit
                    .set_power(0, (max_mana as f32 * restore_percent) as u32);
                player.unit.set_power(1, 0);
                let max_energy = player.unit.max_power(3);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                player
                    .unit
                    .set_power(3, (max_energy as f32 * restore_percent) as u32);
            }
            #[allow(clippy::cast_possible_wrap)]
            {
                player.unit.level() as i32
            }
        };
        if apply_sickness && level >= self.config.death_sickness_level {
            self.apply_aura_by_id(guid, SPELL_RESURRECTION_SICKNESS, guid);
            // Fork duration override: (level-10) min,
            // capped at 10 minutes.
            let minutes = i64::from(level - 10).clamp(1, 10);
            #[allow(clippy::cast_possible_truncation)]
            let duration_ms = (minutes * 60_000) as i32;
            if let Some(unit) = self.unit_base_mut(guid) {
                for holder in &mut unit.auras {
                    if holder.spell_id == SPELL_RESURRECTION_SICKNESS {
                        holder.duration_ms = duration_ms;
                        holder.max_duration_ms = duration_ms;
                    }
                }
            }
        }
    }

    /// `Player::LoadCorpse` + the login `ApplyGhostForm`
    /// tail for a freshly loaded dead player. When a
    /// resurrectable corpse exists, refresh only its
    /// release-timer byte (based on the corpse's map)
    /// and the ghost aura if the saved load omitted it;
    /// otherwise resurrect at 50% rather than strand
    /// the player with no corpse to reclaim.
    pub fn finish_dead_login(&mut self, guid: ObjectGuid, corpse_map_id: Option<u32>) {
        let is_dead = self
            .players
            .get(&guid)
            .is_some_and(|entry| !entry.player.unit.is_alive());
        if !is_dead {
            return;
        }
        let Some(corpse_map_id) = corpse_map_id else {
            self.resurrect_player(guid, 0.5, false);
            if let Some(entry) = self.players.get_mut(&guid) {
                entry.scan_due = true;
            }
            return;
        };
        if !self
            .aura_holders(guid)
            .iter()
            .any(|h| h.spell_id == SPELL_GHOST)
        {
            self.apply_ghost_form(guid);
        }
        let instanceable = self.stores.maps.is_instance_map(corpse_map_id);
        if let Some(player) = self.player_mut(guid) {
            let values = player.unit.world.object.values_mut();
            if instanceable {
                values.remove_byte_flag(offsets::PLAYER_FIELD_BYTES, 0, 0x08);
            } else {
                values.set_byte_flag(offsets::PLAYER_FIELD_BYTES, 0, 0x08);
            }
        }
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.scan_due = true;
        }
    }

    /// `HandleReclaimCorpseOpcode`: dead player near
    /// their own corpse past the reclaim delay
    /// resurrects at 50% and leaves bones. The fork
    /// reads the packet guid and ignores it,
    /// resolving `Player::GetCorpse` (resurrectable
    /// only) instead — a bones guid must not reclaim.
    pub fn reclaim_corpse(&mut self, guid: ObjectGuid, _corpse_guid: ObjectGuid) {
        let is_ghost = self.players.get(&guid).is_some_and(|e| {
            e.player.unit.death_state == DeathState::Dead
                && e.player
                    .unit
                    .world
                    .object
                    .values()
                    .has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_GHOST)
        });
        if !is_ghost {
            return;
        }
        let Some(corpse_guid) = self.corpse_for_player(guid) else {
            return;
        };
        let Some((corpse_pos, ghost_time, corpse_type, player_pos)) =
            self.corpses.get(&corpse_guid).and_then(|corpse| {
                let player_pos = self.players.get(&guid)?.player.unit.world.position();
                Some((
                    corpse.world.position(),
                    corpse.ghost_time,
                    corpse.corpse_type,
                    player_pos,
                ))
            })
        else {
            return;
        };
        if crate::visibility::dist3d(player_pos, corpse_pos) > CORPSE_RECLAIM_RADIUS {
            return;
        }
        let delay = self.corpse_reclaim_delay(guid, corpse_type == CorpseType::ResurrectablePvP);
        if ghost_time + u64::from(delay) > self.now_unix {
            return;
        }
        self.resurrect_player(guid, 0.5, false);
        self.spawn_corpse_bones_for(guid);
    }

    /// Spirit-healer interaction gate
    /// (`HandleSpiritHealerActivateOpcode` head):
    /// creature exists on this map, carries the
    /// spirit-healer NPC flag, player within
    /// interact range. Returns the healer position.
    #[must_use]
    pub fn spirit_healer_position(
        &self,
        guid: ObjectGuid,
        npc_guid: ObjectGuid,
    ) -> Option<Position> {
        let creature = self.creatures.get(&npc_guid)?;
        let flags = creature
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_NPC_FLAGS);
        if flags & NPC_FLAG_SPIRIT_HEALER == 0 {
            return None;
        }
        let player_pos = self.players.get(&guid)?.player.unit.world.position();
        let npc_pos = creature.unit.world.position();
        // `IsValidInteractTarget` distance arm uses
        // the gossip interact distance (11.1 yd in the
        // fork's INTERACTION_DISTANCE + radii model;
        // the map's gossip path uses its own bound).
        (crate::visibility::dist3d(player_pos, npc_pos) <= 50.0).then_some(npc_pos)
    }

    /// `SendSpiritResurrect` player-local arm: 50%
    /// resurrect with sickness + 25% durability loss
    /// including inventory. Bones and the graveyard
    /// compare/teleport are the actor's (the corpse
    /// may live on another map).
    pub fn spirit_healer_resurrect(&mut self, guid: ObjectGuid) {
        let is_dead = self
            .players
            .get(&guid)
            .is_some_and(|e| e.player.unit.death_state == DeathState::Dead);
        if !is_dead {
            return;
        }
        self.resurrect_player(guid, 0.5, true);
        self.durability_loss_all(guid, 0.25, true);
        let packet = WorldPacket::new(op::SMSG_DURABILITY_DAMAGE_DEATH);
        self.events.push(MapEvent::Packet { to: guid, packet });
    }

    /// Team of a resident player (spirit-resurrect
    /// graveyard compare input).
    #[must_use]
    pub fn player_team_of(&self, guid: ObjectGuid) -> Option<Team> {
        self.players.get(&guid).map(|e| e.player.team)
    }

    /// Closest graveyard for an arbitrary position on
    /// this map (`GetClosestGraveYard` with caller-
    /// supplied zone/area of the position).
    pub fn graveyard_for(
        &mut self,
        pos: Position,
        team: Team,
        zone_id: u32,
        area_id: u32,
    ) -> Option<WorldSafeLoc> {
        self.closest_graveyard(pos, team, zone_id, area_id)
    }

    /// Closest graveyard to a resident player (the
    /// ghost grave of the spirit-resurrect compare).
    pub fn player_graveyard(&mut self, guid: ObjectGuid) -> Option<WorldSafeLoc> {
        let (pos, team, zone_id, area_id) = self.players.get(&guid).map(|entry| {
            (
                entry.player.unit.world.position(),
                entry.player.team,
                entry.zone_id,
                entry.area_id,
            )
        })?;
        self.closest_graveyard(pos, team, zone_id, area_id)
    }

    /// `Player::DurabilityLossAll` over the saved
    /// inventory baseline: worn slots always,
    /// backpack + bag contents when `inventory`.
    /// `DurabilityLoss` per item: loss =
    /// `max(1, max_durability * percent)` points.
    pub(crate) fn durability_loss_all(&mut self, guid: ObjectGuid, percent: f64, inventory: bool) {
        let Some(entry) = self.players.get_mut(&guid) else {
            return;
        };
        let prototypes = &self.stores.item_prototypes;
        for row in &mut entry.player.deferred.inventory {
            let worn = row.bag == 0 && row.slot < 19;
            let carried = inventory && (row.bag != 0 || row.slot >= 23);
            if !worn && !carried {
                continue;
            }
            let Some(proto) = prototypes.get(row.item.item_id) else {
                continue;
            };
            if proto.max_durability == 0 || row.item.durability == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let loss = ((f64::from(proto.max_durability) * percent) as u32).max(1);
            row.item.durability = row
                .item
                .durability
                .saturating_sub(u16::try_from(loss).unwrap_or(u16::MAX));
        }
    }

    /// `ObjectAccessor::RemoveOldCorpses` map-local
    /// arm: resurrectable corpses older than 3 days
    /// convert to bones; bones older than
    /// `Bones.ExpireMinutes` (60) are removed. Runs
    /// from the map update at a coarse cadence (the
    /// caller throttles).
    pub(crate) fn expire_corpses(&mut self) {
        let expired_reclaimable: Vec<ObjectGuid> = self
            .corpses
            .values()
            .filter(|c| {
                c.corpse_type != CorpseType::Bones
                    && c.ghost_time + CORPSE_EXPIRE_SECS < self.now_unix
            })
            .map(|c| c.owner)
            .collect();
        for owner in expired_reclaimable {
            self.spawn_corpse_bones_for(owner);
        }
        let expired_bones: Vec<ObjectGuid> = self
            .corpses
            .values()
            .filter(|c| c.corpse_type == CorpseType::Bones && c.ghost_time + 3600 < self.now_unix)
            .map(Corpse::guid)
            .collect();
        for guid in expired_bones {
            self.remove_corpse_object(guid);
        }
    }
}
