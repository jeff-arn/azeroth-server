//! Dynamic (non-DB) gameobject runtime (Tier 4
//! B0-5): `WorldObject::SummonGameObject`
//! (`Object.cpp:2423`), `Unit::AddGameObject` /
//! `RemoveGameObject` / `RemoveAllGameObjects`
//! (`Unit.cpp:4270-4356`), the dynamic subset of
//! `GameObject::Update` (`GameObject.cpp:298`:
//! `GO_NOT_READY` arming, `GO_READY` timer expiry,
//! charges exhaustion, `GO_JUST_DEACTIVATED` deletion)
//! and the trap trigger arm.
//!
//! Scope: the fork's full GO loot-state machine for
//! *static* DB objects remains approximated by the
//! existing use/despawn helpers (recorded at those
//! sites); this module ticks dynamic objects fully
//! and static traps for their trigger arm only.
//! UNMODELED: GOOBER just-deactivated unique-user
//! cast, pool/linked-trap respawn on static expiry,
//! `JustSummoned` GO-AI notification (creature-AI
//! arm wired, GO-AI arm has no ported consumer).

use mangos_entities::gameobject::{
    GO_ANIMPROGRESS_DEFAULT, GO_STATE_ACTIVE, GO_STATE_READY, GameObject,
};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::gameobject_template::GameObjectInfo;

use crate::map::Map;

/// `GAMEOBJECT_TYPE_TRAP` (`GameObjectDefines.h:33`).
pub const GO_TYPE_TRAP: u32 = 6;
/// `GAMEOBJECT_TYPE_FLAGDROP`.
const GO_TYPE_FLAGDROP: u32 = 26;
/// `GAMEOBJECT_TYPE_FISHINGNODE`.
const GO_TYPE_FISHINGNODE: u32 = 17;
/// `GAMEOBJECT_TYPE_CHEST` (`GameObjectDefines.h:29`).
const GO_TYPE_CHEST: u32 = 3;
/// `GAMEOBJECT_TYPE_SUMMONING_RITUAL`.
const GO_TYPE_SUMMONING_RITUAL: u32 = 18;
/// `SPELL_ATTR_COOLDOWN_ON_EVENT`
/// (`SpellDefines.h:802`).
const ATTR_COOLDOWN_ON_EVENT: u32 = 0x0200_0000;

/// `LootState` (`GameObjectDefines.h:109-115`),
/// server-side only (never a client field at
/// 1.12.1); only its wire effects (`GO_STATE`
/// anims, despawn/spawn anims, loot packets)
/// replicate.
pub const GO_LOOT_NOT_READY: u8 = 0;
pub const GO_LOOT_READY: u8 = 1;
pub const GO_LOOT_ACTIVATED: u8 = 2;
pub const GO_LOOT_JUST_DEACTIVATED: u8 = 3;

/// Per-GO runtime state mirroring the fork members
/// the entity aggregate does not carry: `m_spellId`,
/// `m_cooldownTime` (unix seconds), `m_useTimes`,
/// `m_lootState`, and the `GO_NOT_READY` ->
/// `GO_READY` arming edge.
#[derive(Clone, Copy, Debug, Default)]
pub struct GoRuntime {
    pub spell_id: u32,
    pub cooldown_until: u64,
    pub uses: u32,
    pub armed: bool,
    /// `GO_JUST_DEACTIVATED` pending (slotted
    /// replacement; deleted on the next tick).
    pub just_deactivated: bool,
    /// `m_lootState` (`GameObject.h:280`): chest
    /// loot windows fill only at `GO_LOOT_READY` on
    /// a spawned GO; releases move to `ACTIVATED`
    /// (partial) or `JUST_DEACTIVATED` (full).
    pub loot_state: u8,
}

/// `trap` union words (`GameObjectDefines.h:292`).
pub struct TrapData {
    pub level: u32,
    pub radius: u32,
    pub spell_id: u32,
    pub charges: u32,
    pub cooldown: u32,
    pub start_delay: u32,
}

#[must_use]
pub fn trap_data(info: &GameObjectInfo) -> TrapData {
    TrapData {
        level: info.data_u32(1),
        radius: info.data_u32(2),
        spell_id: info.data_u32(3),
        charges: info.data_u32(4),
        cooldown: info.data_u32(5),
        start_delay: info.data_u32(7),
    }
}

impl Map {
    /// `Map::GenerateLocalLowGuid(HIGHGUID_GAMEOBJECT)`
    /// (`Map.cpp:2968`).
    fn next_go_low(&mut self) -> u32 {
        loop {
            self.go_low_counter += 1;
            let cand = self.go_low_counter;
            if !self
                .gameobjects
                .keys()
                .any(|g| g.counter() == cand && g.is_game_object())
            {
                return cand;
            }
        }
    }

    fn unit_world_mask(&self, guid: ObjectGuid) -> Option<u32> {
        if let Some(e) = self.players.get(&guid) {
            return Some(e.player.unit.world.world_mask);
        }
        self.creatures.get(&guid).map(|c| c.unit.world.world_mask)
    }

    fn object_world_mask(&self, guid: ObjectGuid) -> Option<u32> {
        self.unit_world_mask(guid)
            .or_else(|| self.gameobjects.get(&guid).map(|g| g.world.world_mask))
    }

    /// `GetOwnerGuid` at 5875 reads
    /// `OBJECT_FIELD_CREATED_BY` (`GameObject.h:72`).
    #[must_use]
    pub(crate) fn go_owner_guid(&self, guid: ObjectGuid) -> ObjectGuid {
        self.gameobjects.get(&guid).map_or(ObjectGuid::EMPTY, |go| {
            go.world
                .object
                .values()
                .get_guid(offsets::OBJECT_FIELD_CREATED_BY)
        })
    }

    /// `SetOwnerGuid` at 5875 (`GameObject.h:72`):
    /// all objects with an owner despawn after delay.
    pub(crate) fn go_set_owner_guid(&mut self, guid: ObjectGuid, owner: ObjectGuid) {
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.spawned_by_default = false;
            go.world
                .object
                .values_mut()
                .set_guid(offsets::OBJECT_FIELD_CREATED_BY, owner);
        }
    }

    /// `SetSpellId` (`GameObject.h:93`): all summoned
    /// objects despawn after delay.
    pub(crate) fn go_set_spell_id(&mut self, guid: ObjectGuid, spell_id: u32) {
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.spawned_by_default = false;
        }
        self.go_runtime.entry(guid).or_default().spell_id = spell_id;
    }

    #[must_use]
    pub(crate) fn go_spell_id(&self, guid: ObjectGuid) -> u32 {
        self.go_runtime.get(&guid).map_or(0, |r| r.spell_id)
    }

    /// `CreateGameObject` + `Create` + `Map::Add`:
    /// insert a fresh dynamic GO. Field-setting order
    /// vs the fork is tick-internal (updates flush at
    /// tick end).
    pub(crate) fn create_dynamic_gameobject(
        &mut self,
        entry: u32,
        position: Position,
        rotation: [f32; 4],
    ) -> Option<ObjectGuid> {
        let guid_low = self.next_go_low();
        self.create_dynamic_gameobject_with_low(entry, position, rotation, guid_low)
    }

    /// As above with a caller-fixed low guid (BG
    /// blueprint re-materialization keeps the slot
    /// guid stable, fork `LoadFromDB` parity).
    pub(crate) fn create_dynamic_gameobject_with_low(
        &mut self,
        entry: u32,
        position: Position,
        rotation: [f32; 4],
        guid_low: u32,
    ) -> Option<ObjectGuid> {
        let info = self.stores.gameobject_templates.get(entry)?.clone();
        let force_active = self.config.force_active_objects;
        let mut go = GameObject::spawn_dynamic(
            guid_low,
            info,
            position,
            self.map_id,
            rotation,
            GO_ANIMPROGRESS_DEFAULT,
            GO_STATE_READY,
            force_active,
        )
        .ok()?;
        go.world.instance_id = self.instance_id;
        go.world.object.set_is_new_object(true);
        go.world.object.add_to_world();
        let guid = go.world.object.object_guid();
        let cell = Self::cell_of(go.world.position());
        self.cells.entry(cell).or_default().gameobjects.insert(guid);
        self.gameobjects.insert(guid, go);
        self.go_runtime.insert(guid, GoRuntime::default());
        // `Map::Add` grid notification: nearby players
        // re-scan so the create reaches stationary
        // viewers (dynobject precedent).
        for entry in self.players.values_mut() {
            entry.scan_due = true;
        }
        self.instance_on_object_create(guid);
        self.go_ai_create(guid);
        Some(guid)
    }

    /// `Unit::AddGameObject` (`Unit.cpp:4270`):
    /// ownership, world mask, and the
    /// cooldown-on-event hold.
    pub(crate) fn unit_add_gameobject(&mut self, owner: ObjectGuid, guid: ObjectGuid) {
        debug_assert!(self.go_owner_guid(guid).is_empty());
        self.go_set_owner_guid(guid, owner);
        if let Some(mask) = self.unit_world_mask(owner)
            && let Some(go) = self.gameobjects.get_mut(&guid)
        {
            go.world.world_mask = mask;
        }
        let spell_id = self.go_spell_id(guid);
        if spell_id != 0
            && let Some(spell) = self.stores.spells.get(spell_id).cloned()
            && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0
        {
            let recovery = spell.recovery_time;
            self.add_cooldowns(owner, &spell, &mut Vec::new(), Some(recovery));
        }
    }

    /// `Unit::RemoveGameObject(GameObject*, bool)`
    /// (`Unit.cpp:4289`) without the object deletion
    /// itself (callers delete when the fork's `del`
    /// or `JUST_DEACTIVATED` arm applies).
    pub(crate) fn unit_remove_gameobject_ref(&mut self, owner: ObjectGuid, guid: ObjectGuid) {
        self.go_set_owner_guid(guid, ObjectGuid::EMPTY);
        let spell_id = self.go_spell_id(guid);
        if spell_id != 0 {
            self.remove_auras_due_to_spell(owner, spell_id);
            let go_type = self.gameobjects.get(&guid).map_or(0, |g| g.info.go_type);
            if let Some(spell) = self.stores.spells.get(spell_id).cloned()
                && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0
                && go_type != GO_TYPE_SUMMONING_RITUAL
            {
                let recovery = spell.recovery_time;
                self.add_cooldowns(owner, &spell, &mut Vec::new(), Some(recovery));
            }
        }
        if let Some(slots) = self.unit_object_slots.get_mut(&owner) {
            for slot in slots.iter_mut() {
                if *slot == Some(guid) {
                    *slot = None;
                }
            }
        }
    }

    /// `Unit::RemoveAllGameObjects` (`Unit.cpp:4347`,
    /// from `Unit::RemoveFromWorld`): owned GOs are
    /// deleted with their owner reference.
    pub(crate) fn unit_remove_all_gameobjects(&mut self, owner: ObjectGuid) {
        let owned_gos: Vec<ObjectGuid> = self
            .gameobjects
            .keys()
            .copied()
            .filter(|&g| self.go_owner_guid(g) == owner)
            .collect();
        for guid in owned_gos {
            self.go_set_owner_guid(guid, ObjectGuid::EMPTY);
            if let Some(go) = self.gameobjects.get_mut(&guid) {
                let now = self.now_unix;
                go.set_respawn_time(0, now);
            }
            self.despawn_gameobject_now(guid);
        }
        self.unit_object_slots.remove(&owner);
    }

    /// `WorldObject::SummonGameObject`
    /// (`Object.cpp:2423`) for unit/GO summoners on
    /// this map. `respawn_secs` is the fork
    /// `respawnTime` (despawn delay for dynamic GOs).
    pub(crate) fn summon_gameobject(
        &mut self,
        summoner: ObjectGuid,
        entry: u32,
        position: Position,
        rotation: [f32; 4],
        respawn_secs: u32,
        attach: bool,
    ) -> Option<ObjectGuid> {
        let summoner_in_world = self.players.contains_key(&summoner)
            || self.creatures.contains_key(&summoner)
            || self.gameobjects.contains_key(&summoner);
        if !summoner_in_world {
            return None;
        }
        let guid = self.create_dynamic_gameobject(entry, position, rotation)?;
        let now = self.now_unix;
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.set_respawn_time(respawn_secs, now);
        }
        if attach && (summoner.is_player() || summoner.is_creature_or_pet()) {
            self.unit_add_gameobject(summoner, guid);
        } else if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.spawned_by_default = false;
        }
        if summoner.is_creature_or_pet() {
            self.creature_script_just_summoned(summoner, guid);
        }
        if let Some(mask) = self.object_world_mask(summoner)
            && let Some(go) = self.gameobjects.get_mut(&guid)
        {
            go.world.world_mask = mask;
        }
        Some(guid)
    }

    /// `GameObject::SummonLinkedTrapIfAny`
    /// (`GameObject.cpp:1213`).
    pub(crate) fn go_summon_linked_trap(&mut self, guid: ObjectGuid) {
        let Some((linked, pos, delay, spell_id, owner, level)) =
            self.gameobjects.get(&guid).map(|go| {
                (
                    mangos_entities::gameobject::linked_trap_entry(&go.info),
                    go.world.position(),
                    go.respawn_delay_time,
                    self.go_spell_id(guid),
                    self.go_owner_guid(guid),
                    go.world.object.values().get_u32(offsets::GAMEOBJECT_LEVEL),
                )
            })
        else {
            return;
        };
        if linked == 0 {
            return;
        }
        let Some(trap) = self.create_dynamic_gameobject(linked, pos, [0.0; 4]) else {
            return;
        };
        let now = self.now_unix;
        if let Some(go) = self.gameobjects.get_mut(&trap) {
            #[allow(clippy::cast_sign_loss)]
            go.set_respawn_time(delay.max(0) as u32, now);
        }
        self.go_set_spell_id(trap, spell_id);
        if !owner.is_empty() {
            self.go_set_owner_guid(trap, owner);
            if let Some(go) = self.gameobjects.get_mut(&trap) {
                go.world
                    .object
                    .values_mut()
                    .set_u32(offsets::GAMEOBJECT_LEVEL, level);
            }
        }
    }

    /// `SetLootState(GO_JUST_DEACTIVATED)` effects:
    /// dynamic objects delete on the next tick;
    /// static objects hide until their respawn delay
    /// (`GameObject.cpp:589-671`; despawn-anim and
    /// flag-reset visual arms unmodeled).
    pub(crate) fn go_set_just_deactivated(&mut self, guid: ObjectGuid) {
        let Some(go) = self.gameobjects.get(&guid) else {
            return;
        };
        if go.data.is_none() {
            self.go_runtime.entry(guid).or_default().just_deactivated = true;
            return;
        }
        let delay = go.respawn_delay_time;
        if delay > 0
            && let Some(go) = self.gameobjects.get_mut(&guid)
        {
            go.respawn_time = self.now_unix + u64::from(u32::from_ne_bytes(delay.to_ne_bytes()));
        }
    }

    /// The `GO_JUST_DEACTIVATED` deletion arm for
    /// dynamic objects (`GameObject.cpp:628`): owner
    /// cleanup then `Delete()`.
    fn go_delete_dynamic(&mut self, guid: ObjectGuid) {
        let owner = self.go_owner_guid(guid);
        if !owner.is_empty()
            && (self.players.contains_key(&owner) || self.creatures.contains_key(&owner))
        {
            self.unit_remove_gameobject_ref(owner, guid);
        }
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            let now = self.now_unix;
            go.set_respawn_time(0, now);
        }
        self.despawn_gameobject_now(guid);
    }

    /// Dynamic subset of `GameObject::Update` plus
    /// the trap trigger arm for all trap GOs. Runs
    /// once per map tick beside `update_respawns`.
    #[allow(clippy::too_many_lines)] // one fork Update pass incl. chest loot arms
    pub(crate) fn update_gameobjects(&mut self) {
        let now = self.now_unix;
        let guids: Vec<ObjectGuid> = self.gameobjects.keys().copied().collect();
        for guid in guids {
            let Some((dynamic, go_type, info)) = self
                .gameobjects
                .get(&guid)
                .map(|go| (go.data.is_none(), go.info.go_type, go.info.clone()))
            else {
                continue;
            };
            // GO_NOT_READY arming edge (dynamic GOs
            // and static traps on first sight).
            if dynamic || go_type == GO_TYPE_TRAP {
                let runtime = self.go_runtime.entry(guid).or_default();
                if !runtime.armed {
                    runtime.armed = true;
                    if go_type == GO_TYPE_TRAP {
                        let delay = trap_data(&info).start_delay;
                        if delay > 0 {
                            runtime.cooldown_until = now + u64::from(delay);
                        }
                    }
                }
            }
            if dynamic
                && self
                    .go_runtime
                    .get(&guid)
                    .is_some_and(|r| r.just_deactivated)
            {
                self.go_delete_dynamic(guid);
                continue;
            }
            // `FISHINGNODE` bobber-ready splash (fork
            // `GameObject::Update` `GO_NOT_READY` arm,
            // `GameObject.cpp:344-365`): once past
            // `respawn - FISHING_BOBBER_READY_TIME` the
            // owner sees `GO_STATE_ACTIVE` + splash
            // sound + custom anim, and the node turns
            // `GO_READY`.
            if dynamic
                && go_type == GO_TYPE_FISHINGNODE
                && self
                    .go_runtime
                    .get(&guid)
                    .is_none_or(|r| r.loot_state == GO_LOOT_NOT_READY)
                && self.gameobjects.get(&guid).is_some_and(|go| {
                    go.respawn_time > 0
                        && now > go.respawn_time.saturating_sub(FISHING_BOBBER_READY_TIME)
                })
            {
                let _ = self.object_set_field(
                    guid,
                    u32::from(offsets::GAMEOBJECT_STATE),
                    GO_STATE_ACTIVE,
                );
                if let Some(go) = self.gameobjects.get_mut(&guid) {
                    go.world
                        .object
                        .force_values_update_at(offsets::GAMEOBJECT_STATE);
                }
                self.send_bobber_splash(guid);
                if let Some(r) = self.go_runtime.get_mut(&guid) {
                    r.loot_state = GO_LOOT_READY;
                }
            }
            if dynamic {
                // GO_READY timer expiry.
                let (expired, spawned_default) =
                    self.gameobjects.get(&guid).map_or((false, true), |go| {
                        (
                            go.respawn_time > 0 && go.respawn_time <= now,
                            go.spawned_by_default,
                        )
                    });
                if expired {
                    if let Some(go) = self.gameobjects.get_mut(&guid) {
                        go.respawn_time = 0;
                    }
                    match go_type {
                        GO_TYPE_FISHINGNODE => {
                            // Fork `GameObject::Update`
                            // `GO_READY` expiry
                            // (`GameObject.cpp:399-411`):
                            // the owner hooks its
                            // channel and sees
                            // `SMSG_FISH_NOT_HOOKED`,
                            // then the node retires
                            // (`GO_JUST_DEACTIVATED`,
                            // deleted on the next tick).
                            let owner = self.go_owner_guid(guid);
                            if !owner.is_empty() && self.players.contains_key(&owner) {
                                self.finish_channeled_spell(owner);
                                let packet = WorldPacket::with_body(
                                    opcode_id::SMSG_FISH_NOT_HOOKED,
                                    Vec::new(),
                                );
                                self.send_to_player(owner, packet);
                            }
                            if let Some(r) = self.go_runtime.get_mut(&guid) {
                                r.loot_state = GO_LOOT_JUST_DEACTIVATED;
                            }
                            self.go_set_just_deactivated(guid);
                            continue;
                        }
                        _ if !spawned_default => {
                            self.go_delete_dynamic(guid);
                            continue;
                        }
                        _ => {}
                    }
                }
            }
            if !dynamic {
                // Static GO_READY timer expiry (fork
                // `Map::Add` show arm): clear the
                // timer so `is_spawned` flips back.
                // Door/button/spell-focus/goober
                // extra arms (reset/linked trap) ride
                // their existing Rust use paths; chests
                // clear through the loot-state arm below.
                let due = self.gameobjects.get(&guid).is_some_and(|g| {
                    g.spawned_by_default
                        && g.respawn_time > 0
                        && g.respawn_time <= now
                        // Chests and pools consume their
                        // respawn timers in their own
                        // loot-state arms below.
                        && !matches!(go_type, 0 | 1 | 3 | 8 | 10 | 25)
                });
                if due && let Some(g) = self.gameobjects.get_mut(&guid) {
                    g.respawn_time = 0;
                }
            }
            if !dynamic && go_type == GO_TYPE_CHEST {
                // Static chest loot-state pass (fork
                // `GameObject::Update` `GO_NOT_READY`
                // and `GO_JUST_DEACTIVATED` chest arms,
                // map-local: no persist write, no
                // despawn-anim/flag-reset packets).
                let restock = info.data_u32(2);
                let consumable = info.data_u32(3) != 0;
                let (loot_state, cooldown_until) = self
                    .go_runtime
                    .get(&guid)
                    .map_or((GO_LOOT_NOT_READY, 0), |r| (r.loot_state, r.cooldown_until));
                if loot_state == GO_LOOT_NOT_READY {
                    if restock != 0 && !consumable {
                        // Restock chest: `READY` when the
                        // restock cooldown lapses (fork
                        // also forces a dynflags update).
                        if cooldown_until <= now {
                            if let Some(r) = self.go_runtime.get_mut(&guid) {
                                r.cooldown_until = 0;
                                r.loot_state = GO_LOOT_READY;
                            }
                            if let Some(go) = self.gameobjects.get_mut(&guid) {
                                go.world.object.force_values_update_at(
                                    mangos_protocol::update_field_table::offsets::GAMEOBJECT_DYN_FLAGS,
                                );
                            }
                        }
                    } else if let Some(r) = self.go_runtime.get_mut(&guid) {
                        r.loot_state = GO_LOOT_READY;
                    } else {
                        self.go_runtime.insert(
                            guid,
                            GoRuntime {
                                loot_state: GO_LOOT_READY,
                                ..GoRuntime::default()
                            },
                        );
                    }
                }
                let loot_state = self
                    .go_runtime
                    .get(&guid)
                    .map_or(GO_LOOT_NOT_READY, |r| r.loot_state);
                if loot_state == GO_LOOT_JUST_DEACTIVATED {
                    // Fork clears the loot, re-arms
                    // `READY`, and (re)starts the
                    // respawn delay at deactivation; the
                    // delay was armed by the release arm,
                    // so expiry here just re-shows.
                    let due = self.gameobjects.get(&guid).is_some_and(|g| {
                        g.spawned_by_default && g.respawn_time > 0 && g.respawn_time <= now
                    });
                    if due {
                        self.loots.remove(&guid);
                        if let Some(r) = self.go_runtime.get_mut(&guid) {
                            r.loot_state = GO_LOOT_READY;
                        }
                        if let Some(g) = self.gameobjects.get_mut(&guid) {
                            g.respawn_time = 0;
                        }
                    }
                }
            }
            // Static + dynamic pool arming (fork
            // `GameObject::Update` `GO_NOT_READY` default
            // arm, `GameObject.cpp:382-383`, and the
            // `GO_JUST_DEACTIVATED` re-show,
            // `GameObject.cpp:655-671`): first sight
            // flips `READY`; depleted delay-lapped
            // statics clear and re-show (map-local, no
            // persist write).
            if go_type == mangos_world_data::gameobject_template::TYPE_FISHINGHOLE {
                let loot_state = self
                    .go_runtime
                    .get(&guid)
                    .map_or(GO_LOOT_NOT_READY, |r| r.loot_state);
                if loot_state == GO_LOOT_NOT_READY {
                    if let Some(r) = self.go_runtime.get_mut(&guid) {
                        r.loot_state = GO_LOOT_READY;
                    }
                } else if !dynamic && loot_state == GO_LOOT_JUST_DEACTIVATED {
                    let due = self.gameobjects.get(&guid).is_some_and(|g| {
                        g.spawned_by_default && g.respawn_time > 0 && g.respawn_time <= now
                    });
                    if due {
                        self.loots.remove(&guid);
                        if let Some(r) = self.go_runtime.get_mut(&guid) {
                            r.loot_state = GO_LOOT_READY;
                        }
                        if let Some(g) = self.gameobjects.get_mut(&guid) {
                            g.respawn_time = 0;
                        }
                    }
                }
            }
            if go_type == GO_TYPE_TRAP {
                self.go_trap_tick(guid);
            }
            // Charges exhaustion (`GetCharges`,
            // `GameObject.cpp:550`): JUST_DEACTIVATED.
            let max_charges = go_max_charges(&info);
            if max_charges > 0
                && self
                    .go_runtime
                    .get(&guid)
                    .is_some_and(|r| r.uses >= max_charges)
            {
                if let Some(r) = self.go_runtime.get_mut(&guid) {
                    r.uses = 0;
                }
                self.go_set_just_deactivated(guid);
            }
        }
    }
}

impl Map {
    /// Position of any world object kind (fork
    /// `WorldObject::GetPosition`).
    pub(crate) fn world_object_position(&self, guid: ObjectGuid) -> Option<Position> {
        self.unit_position(guid)
            .or_else(|| self.gameobjects.get(&guid).map(|g| g.world.position()))
    }

    /// `Spell::EffectSummonObjectWild`
    /// (`SpellEffects.cpp:3490`). UNMODELED:
    /// `focusObject` (spell-focus GO) map/orientation
    /// source and the GO-AI `JustSummoned` arm.
    pub(crate) fn summon_object_wild_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &mangos_spells::entry::SpellEntry,
        eff_index: usize,
        client_dest: Option<Position>,
    ) {
        let eff = &spell.effects[eff_index];
        #[allow(clippy::cast_sign_loss)]
        let entry = eff.misc_value.max(0) as u32;
        if entry == 0 {
            return;
        }
        let Some(caster_pos) = self.world_object_position(caster) else {
            return;
        };
        let pos = client_dest.map_or(caster_pos, |dest| {
            Position::new(dest.x, dest.y, dest.z, caster_pos.o)
        });
        let Some(guid) = self.create_dynamic_gameobject(entry, pos, [0.0; 4]) else {
            return;
        };
        let mut duration_ms = spell.calculate_duration_ms(&self.stores.spells.tables);
        if spell.id == 28535 {
            // Sapphiron ice blocks: just long enough
            // to survive until the ice bomb despawn.
            duration_ms = 30_000;
        }
        let now = self.now_unix;
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            #[allow(clippy::cast_sign_loss)]
            go.set_respawn_time(
                if duration_ms > 0 {
                    (duration_ms / 1000) as u32
                } else {
                    0
                },
                now,
            );
        }
        self.go_set_spell_id(guid, spell.id);
        // Wild objects have no owner; the WSG dropped
        // flag registers with the battleground.
        let go_type = self.gameobjects.get(&guid).map_or(0, |g| g.info.go_type);
        if go_type == GO_TYPE_FLAGDROP && caster.is_player() {
            self.ws_register_dropped_flag(caster, guid);
        }
        if let Some(mask) = self.object_world_mask(caster)
            && let Some(go) = self.gameobjects.get_mut(&guid)
        {
            go.world.world_mask = mask;
        }
        self.go_summon_linked_trap(guid);
        if caster.is_creature_or_pet() {
            self.creature_script_just_summoned(caster, guid);
        }
        self.spell_script_on_summon(caster, guid, spell);
    }

    /// `Spell::EffectSummonObject`
    /// (`SpellEffects.cpp:5045`, slots 1-4). The
    /// near-point collision selector is unmodeled:
    /// the front point at `bounding + DEFAULT_SIZE`
    /// rides, as in the pet materialize path.
    pub(crate) fn summon_object_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &mangos_spells::entry::SpellEntry,
        eff_index: usize,
        client_dest: Option<Position>,
    ) {
        use mangos_spells::defines::effect;
        // `m_casterUnit` gate: GO casters no-op.
        if !caster.is_player() && !caster.is_creature_or_pet() {
            return;
        }
        let eff = &spell.effects[eff_index];
        let slot = match eff.effect {
            effect::SUMMON_OBJECT_SLOT1 => 0usize,
            effect::SUMMON_OBJECT_SLOT2 => 1,
            effect::SUMMON_OBJECT_SLOT3 => 2,
            effect::SUMMON_OBJECT_SLOT4 => 3,
            _ => return,
        };
        #[allow(clippy::cast_sign_loss)]
        let entry = eff.misc_value.max(0) as u32;
        if entry == 0 {
            return;
        }
        if let Some(old) = self
            .unit_object_slots
            .get(&caster)
            .and_then(|slots| slots[slot])
        {
            if self.gameobjects.contains_key(&old)
                && let Some(r) = self.go_runtime.get_mut(&old)
            {
                r.just_deactivated = true;
            }
            if let Some(slots) = self.unit_object_slots.get_mut(&caster) {
                slots[slot] = None;
            }
        }
        let Some(caster_pos) = self.unit_position(caster) else {
            return;
        };
        let pos = if let Some(dest) = client_dest {
            Position::new(dest.x, dest.y, dest.z, caster_pos.o)
        } else {
            let radius =
                self.bounding_radius_of(caster) + mangos_entities::unit::DEFAULT_WORLD_OBJECT_SIZE;
            Position::new(
                caster_pos.x + radius * caster_pos.o.cos(),
                caster_pos.y + radius * caster_pos.o.sin(),
                caster_pos.z,
                caster_pos.o,
            )
        };
        let Some(guid) = self.create_dynamic_gameobject(entry, pos, [0.0; 4]) else {
            return;
        };
        let level = self.unit_level(caster);
        let duration_ms = spell.calculate_duration_ms(&self.stores.spells.tables);
        let now = self.now_unix;
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            go.world
                .object
                .values_mut()
                .set_u32(offsets::GAMEOBJECT_LEVEL, level);
            #[allow(clippy::cast_sign_loss)]
            go.set_respawn_time(
                if duration_ms > 0 {
                    (duration_ms / 1000) as u32
                } else {
                    0
                },
                now,
            );
        }
        self.go_set_spell_id(guid, spell.id);
        self.unit_add_gameobject(caster, guid);
        // `SMSG_GAMEOBJECT_SPAWN_ANIM` to the caster's
        // set (`SendMessageToSet(&data, true)`).
        {
            let mut payload = mangos_protocol::buffer::PacketWriter::new();
            payload.write_u64_le(guid.raw());
            let packet = mangos_protocol::world::WorldPacket::with_body(
                mangos_protocol::opcode_table::opcode_id::SMSG_GAMEOBJECT_SPAWN_ANIM,
                payload.into_vec(),
            );
            self.broadcast_movement(caster, &packet);
        }
        self.unit_object_slots.entry(caster).or_insert([None; 4])[slot] = Some(guid);
        self.go_summon_linked_trap(guid);
        if caster.is_creature_or_pet() {
            self.creature_script_just_summoned(caster, guid);
        }
        self.spell_script_on_summon(caster, guid, spell);
    }

    /// GO-originated cast (`SpellCaster::CastSpell`
    /// with a gameobject `this`): a triggered cast
    /// over the normal effect engine with the GO as
    /// caster; cast times tick slot-less like other
    /// triggered casts. Fork `prepare` gates
    /// (range/LOS) are unmodeled for GO casters, as
    /// are channeled and missile-delay GO casts
    /// (land instantly here or not at all); aura
    /// holders record the GO as caster, matching
    /// fork `m_originalCaster` for these calls.
    pub(crate) fn go_cast_spell(&mut self, go: ObjectGuid, spell_id: u32, victim: ObjectGuid) {
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        if spell.is_channeled() {
            return;
        }
        let Some(position) = self.world_object_position(go) else {
            return;
        };
        let cast_time = spell.cast_time_ms(&self.stores.spells.tables);
        let (proc_attacker_flags, proc_victim_flags, can_trigger_procs) =
            crate::procs::spell_proc_context(&spell, true, false);
        let cast = crate::spells::ActiveCast {
            spell_id,
            cast_counter: 0,
            target: crate::spells::CastTarget::Unit(victim),
            remaining_ms: i32::try_from(cast_time).unwrap_or(i32::MAX),
            total_ms: cast_time,
            triggered: true,
            triggered_by_aura: false,
            delay_count: 0,
            channeled: false,
            channeling: false,
            channel_cost_ms: 1000,
            applied_mods: Vec::new(),
            proc_attacker_flags,
            proc_victim_flags,
            can_trigger_procs,
            from_proc: false,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
            script_effect_pick: None,
            position,
            original_caster: None,
        };
        if cast_time == 0 {
            self.execute_cast(go, cast);
        } else {
            self.pending_triggered.push((go, cast));
        }
    }

    /// 3D distance from a unit to this GO minus both
    /// bounding radii (fork `GetDistance`).
    fn go_distance_to_unit(&self, go_pos: Position, go_radius: f32, unit: ObjectGuid) -> f32 {
        let Some(upos) = self.unit_position(unit) else {
            return f32::MAX;
        };
        let d = ((upos.x - go_pos.x).powi(2)
            + (upos.y - go_pos.y).powi(2)
            + (upos.z - go_pos.z).powi(2))
        .sqrt();
        (d - go_radius - self.bounding_radius_of(unit)).max(0.0)
    }

    /// Trap trigger arm of `GameObject::Update`
    /// `GO_READY` (`GameObject.cpp:462-547`).
    /// Deviations ride the engine-wide hostility
    /// approximation (`is_hostile_pair`): the fork's
    /// reaction matrix, PvP-flag and duel gates of
    /// `HunterTrapTargetSelectorCheck`, and
    /// `GameObjectAI::OnUse` interception are
    /// unmodeled; the owner cast records the owner
    /// (not the trap GO) as aura caster.
    #[allow(clippy::too_many_lines)] // one fork Update trap arm
    pub(crate) fn go_trap_tick(&mut self, guid: ObjectGuid) {
        let now = self.now_unix;
        let Some(go) = self.gameobjects.get(&guid) else {
            return;
        };
        if !go.is_spawned() {
            return;
        }
        if self
            .go_runtime
            .get(&guid)
            .is_some_and(|r| r.cooldown_until != 0 && r.cooldown_until >= now)
        {
            return;
        }
        let info = go.info.clone();
        let trap = trap_data(&info);
        let go_pos = go.world.position();
        let display_id = go
            .world
            .object
            .values()
            .get_u32(offsets::GAMEOBJECT_DISPLAYID);
        // `GameObject::GetObjectBoundingRadius` is
        // the constant default at 1.12.1
        // (`GameObject.cpp:2198`).
        let go_radius = mangos_entities::unit::DEFAULT_WORLD_OBJECT_SIZE;
        let world_mask = go.world.world_mask;
        let respawn_time = go.respawn_time;
        let owner = self.go_owner_guid(guid);
        let owner_live = (!owner.is_empty()
            && (self.players.contains_key(&owner) || self.creatures.contains_key(&owner)))
        .then_some(owner);

        #[allow(clippy::cast_precision_loss)]
        let mut radius = trap.radius as f32;
        let mut is_bg_trap = false;
        if radius == 0.0 {
            if trap.cooldown != 3 {
                return;
            }
            if respawn_time > 0 {
                return;
            }
            #[allow(clippy::cast_precision_loss)]
            {
                radius = trap.cooldown as f32;
            }
            is_bg_trap = true;
        }
        // Hunter-trap float radius table
        // (`GameObject.cpp:484`).
        if matches!(
            info.id,
            2561 | 164_638
                | 164_639
                | 164_839
                | 164_872
                | 164_873
                | 164_874
                | 164_875
                | 164_876
                | 164_877
                | 164_879
                | 164_880
        ) {
            radius = 2.5;
        }

        let ok = if let Some(owner) = owner_live.filter(|_| trap.charges > 0) {
            // Hunter trap: nearest valid attack
            // target of the owner (`UnitLastSearcher`
            // keeps the closest match as range shrinks).
            let mut best: Option<(ObjectGuid, f32)> = None;
            let mut candidates: Vec<ObjectGuid> = self.players.keys().copied().collect();
            candidates.extend(self.creatures.keys().copied());
            let owner_pet = self.pet_of(owner);
            for cand in candidates {
                // `IsValidAttackTarget`: never the
                // owner or its pet (party-friendly
                // faction screen rides the recorded
                // hostility approximation).
                if cand == owner || Some(cand) == owner_pet {
                    continue;
                }
                if !self.unit_is_alive(cand) {
                    continue;
                }
                // Fork masks are never 0 in-world
                // (`WORLD_DEFAULT_OBJECT` = 1); Rust
                // players may still carry the unset 0,
                // treated as the default world here.
                let cand_mask = self.unit_world_mask(cand).unwrap_or(0);
                if world_mask != 0
                    && cand_mask != 0
                    && !crate::visibility::can_see_in_world(world_mask, cand_mask)
                {
                    continue;
                }
                // `CREATURE_TYPE_TOTEM` = 11.
                let is_totem = self
                    .creatures
                    .get(&cand)
                    .is_some_and(|c| c.info.creature_type == 11);
                let range = if is_totem { radius / 3.0 } else { radius };
                let dist = self.go_distance_to_unit(go_pos, go_radius, cand);
                if dist > range {
                    continue;
                }
                if !Self::is_hostile_pair(owner, cand) {
                    continue;
                }
                if best.is_none_or(|(_, d)| dist < d) {
                    best = Some((cand, dist));
                }
            }
            best.map(|(g, _)| g)
        } else {
            // Environmental trap: any alive player in
            // range (`AnyPlayerInObjectRangeCheck`).
            let players: Vec<ObjectGuid> = self.players.keys().copied().collect();
            players.into_iter().find(|&cand| {
                self.unit_is_alive(cand)
                    && self.go_distance_to_unit(go_pos, go_radius, cand) <= radius
            })
        };

        let Some(victim) = ok else {
            return;
        };
        if trap.spell_id != 0 {
            if let Some(owner) = owner_live {
                // `originalCaster` = trap GO (aura
                // attribution nuance recorded above).
                let _ = self.cast_spell(
                    owner,
                    trap.spell_id,
                    crate::spells::CastTarget::Unit(victim),
                    crate::spells::CastOptions {
                        triggered: true,
                        skip_known: true,
                        ..Default::default()
                    },
                );
            } else {
                self.go_cast_spell(guid, trap.spell_id, victim);
            }
        }
        let cooldown = if trap.cooldown != 0 { trap.cooldown } else { 4 };
        if let Some(r) = self.go_runtime.get_mut(&guid) {
            r.cooldown_until = now + u64::from(cooldown);
            if trap.charges > 0 {
                r.uses += 1;
            }
        }
        if is_bg_trap && victim.is_player() {
            self.bg_handle_trigger_buff(guid);
        }
        if go_has_custom_anim(display_id) {
            let mut payload = mangos_protocol::buffer::PacketWriter::new();
            payload.write_u64_le(guid.raw());
            payload.write_u32_le(0);
            let packet = mangos_protocol::world::WorldPacket::with_body(
                mangos_protocol::opcode_table::opcode_id::SMSG_GAMEOBJECT_CUSTOM_ANIM,
                payload.into_vec(),
            );
            self.broadcast_movement(guid, &packet);
        }
    }
}

impl Map {
    /// `GameObject::TriggerLinkedGameObject`
    /// (`GameObject.cpp:1238`): use the nearest
    /// spawned linked trap on `target`.
    pub(crate) fn go_trigger_linked(&mut self, user: ObjectGuid, go_guid: ObjectGuid) {
        let Some((trap_entry, go_pos)) = self.gameobjects.get(&go_guid).map(|g| {
            (
                mangos_entities::gameobject::linked_trap_entry(&g.info),
                g.world.position(),
            )
        }) else {
            return;
        };
        if trap_entry == 0 {
            return;
        }
        let Some(trap_info) = self.stores.gameobject_templates.get(trap_entry) else {
            return;
        };
        if trap_info.go_type != GO_TYPE_TRAP {
            return;
        }
        let range = self
            .stores
            .spells
            .get(trap_data(trap_info).spell_id)
            .map_or(0.5, |s| s.range_max(&self.stores.spells.tables));
        let mut best: Option<(ObjectGuid, f32)> = None;
        for (guid, go) in &self.gameobjects {
            if go.info.id != trap_entry {
                continue;
            }
            let p = go.world.position();
            let d =
                ((p.x - go_pos.x).powi(2) + (p.y - go_pos.y).powi(2) + (p.z - go_pos.z).powi(2))
                    .sqrt();
            if d <= range && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((*guid, d));
            }
        }
        if let Some((trap_go, _)) = best
            && self
                .gameobjects
                .get(&trap_go)
                .is_some_and(mangos_entities::gameobject::GameObject::is_spawned)
        {
            self.go_use_direct(user, trap_go);
        }
    }

    /// Script-side `GameObject::Use(Unit*)` subset
    /// (`GameObject.cpp:1352`) for direct calls
    /// (summoned-object uses, linked traps): AI
    /// intercept, the `GetCooldown` gate, and the
    /// trap/generic/door/button/chest/spell-focus/
    /// spellcaster arms plus the spell tail (the GO
    /// is the default caster). The player opcode path
    /// keeps its own gates in `gameobject_use`;
    /// summoning-ritual and remaining type arms are
    /// unmodeled here.
    #[allow(clippy::too_many_lines)] // one fork Use() subset
    pub(crate) fn go_use_direct(&mut self, user: ObjectGuid, go_guid: ObjectGuid) {
        let Some(info) = self.gameobjects.get(&go_guid).map(|g| g.info.clone()) else {
            return;
        };
        if user.is_player() && self.go_ai_on_use(user, go_guid) {
            return;
        }
        let now = self.now_unix;
        // `GetCooldown` gate (trap data5 / goober
        // data6).
        let cooldown = match info.go_type {
            GO_TYPE_TRAP => info.data_u32(5),
            10 => info.data_u32(6),
            _ => 0,
        };
        if cooldown > 0 {
            let runtime = self.go_runtime.entry(go_guid).or_default();
            if runtime.cooldown_until > now {
                return;
            }
            runtime.cooldown_until = now + u64::from(cooldown);
        }
        match info.go_type {
            GO_TYPE_TRAP => {
                let trap = trap_data(&info);
                if trap.spell_id != 0 {
                    let owner = self.go_owner_guid(go_guid);
                    if !owner.is_empty()
                        && (self.players.contains_key(&owner)
                            || self.creatures.contains_key(&owner))
                    {
                        let _ = self.cast_spell(
                            owner,
                            trap.spell_id,
                            crate::spells::CastTarget::Unit(user),
                            crate::spells::CastOptions {
                                triggered: true,
                                skip_known: true,
                                ..Default::default()
                            },
                        );
                    } else {
                        self.go_cast_spell(go_guid, trap.spell_id, user);
                    }
                }
                let display_id = self.gameobjects.get(&go_guid).map_or(0, |g| {
                    g.world
                        .object
                        .values()
                        .get_u32(offsets::GAMEOBJECT_DISPLAYID)
                });
                if go_has_custom_anim(display_id) {
                    let mut payload = mangos_protocol::buffer::PacketWriter::new();
                    payload.write_u64_le(go_guid.raw());
                    payload.write_u32_le(0);
                    let packet = mangos_protocol::world::WorldPacket::with_body(
                        mangos_protocol::opcode_table::opcode_id::SMSG_GAMEOBJECT_CUSTOM_ANIM,
                        payload.into_vec(),
                    );
                    self.broadcast_movement(go_guid, &packet);
                }
                if trap.charges > 0
                    && let Some(r) = self.go_runtime.get_mut(&go_guid)
                {
                    r.uses += 1;
                }
            }
            0 | 1 => {
                // `UseDoorOrButton` toggle subset (as
                // the opcode path) + DB script start.
                let state = self.gameobjects.get(&go_guid).map_or(0, |g| {
                    g.world.object.values().get_u32(offsets::GAMEOBJECT_STATE)
                });
                let next = u32::from(state == 0);
                if let Some(g) = self.gameobjects.get_mut(&go_guid) {
                    g.world
                        .object
                        .values_mut()
                        .set_u32(offsets::GAMEOBJECT_STATE, next);
                }
                self.scripts_start_gameobject(go_guid.counter(), user, go_guid);
                if info.go_type == 1 {
                    self.go_trigger_linked(user, go_guid);
                }
            }
            3 | 8 => {
                if info.go_type == 3 {
                    self.scripts_start_gameobject(go_guid.counter(), user, go_guid);
                }
                self.go_trigger_linked(user, go_guid);
            }
            5 => {
                self.go_set_just_deactivated(go_guid);
            }
            22 => {
                // SPELLCASTER: lock flags, count the
                // use, tail-cast on the user.
                if let Some(g) = self.gameobjects.get_mut(&go_guid) {
                    g.world
                        .object
                        .values_mut()
                        .set_u32(offsets::GAMEOBJECT_FLAGS, GO_FLAG_LOCKED);
                }
                self.go_runtime.entry(go_guid).or_default().uses += 1;
                let spell_id = info.data_u32(0);
                if spell_id != 0 {
                    self.go_cast_spell(go_guid, spell_id, user);
                }
            }
            _ => {}
        }
    }
}

/// `GO_FLAG_LOCKED` (`GameObjectDefines.h:76`).
const GO_FLAG_LOCKED: u32 = 0x0000_0002;

/// `HasCustomAnim` display whitelist
/// (`GameObject.cpp:2347`).
fn go_has_custom_anim(display_id: u32) -> bool {
    matches!(
        display_id,
        2570 | 3071 | 3072 | 3073 | 3074 | 4392 | 4472 | 4491 | 6785 | 6747 | 6871
    )
}

/// `GameObjectInfo::GetCharges`
/// (`GameObjectDefines.h:665`): trap charges ride
/// data4; other charged types read their own words.
#[must_use]
pub fn go_max_charges(info: &GameObjectInfo) -> u32 {
    match info.go_type {
        GO_TYPE_TRAP => info.data_u32(4),
        21 | 22 => info.data_u32(1), // guardpost / spellcaster charges
        _ => 0,
    }
}

/// `FISHING_BOBBER_READY_TIME`
/// (`GameObjectDefines.h:199`): splash 5 s before
/// the bobber timer lapses.
const FISHING_BOBBER_READY_TIME: u64 = 5;
/// Splash sound (`GameObject.cpp:354`).
const FISHING_SPLASH_SOUND: u32 = 3355;
/// Hole search range (fork `GameObject.cpp:1629` +
/// `:1673`: `20.0f + CONTACT_DISTANCE`, with
/// `CONTACT_DISTANCE` = 0.5 from `ObjectDefines.h:23`).
pub(crate) const FISHING_HOLE_SEARCH_RANGE: f32 = 20.5;

impl Map {
    /// Fishable water at a bobber point (fork
    /// `EffectTransmitted` swimmable gate,
    /// `SpellEffects.cpp:5601-5612`, over
    /// `TerrainInfo::IsSwimmable` /
    /// `GridMap::getLiquidStatus`): the grid liquid
    /// surface, or `None` when the tile has no
    /// liquid, the surface sits below the ground, or
    /// the fork's `abs(depth_level) < 1` ground gate
    /// trips (`depth_level` is the ground level there,
    /// `GridMap.cpp:603-613`). No liquid-type
    /// discrimination: the fork accepts every
    /// `MAP_ALL_LIQUIDS` type, and so does this.
    /// LOS against the surface runs at the call site
    /// (it needs the caster height too).
    fn fishing_water_level(&mut self, x: f32, y: f32) -> Option<f32> {
        use mangos_assets::gridmap::INVALID_HEIGHT_VALUE;
        let level = self.heights.grid(x, y)?.liquid_level(x, y);
        if level == INVALID_HEIGHT_VALUE {
            return None;
        }
        let ground = self.ground_z(x, y, level);
        if level < ground || ground.abs() < 1.0 {
            return None;
        }
        Some(level)
    }

    /// `Spell::EffectTransmitted`
    /// (`SpellEffects.cpp:5538-5678`) fishing subset:
    /// the `FISHINGNODE` bobber summon. Returns false
    /// exactly when the fork calls `finish()` after
    /// `SendCastResult(NOT_FISHABLE)` (dry land or no
    /// surface LOS). Other GO types stay no-ops (the
    /// slice covers fishing only; ritual extras are
    /// unmodeled with the ritual use path).
    #[allow(clippy::too_many_lines)] // one fork EffectTransmitted port
    pub(crate) fn effect_transmitted_fishing(
        &mut self,
        caster: ObjectGuid,
        spell: &mangos_spells::entry::SpellEntry,
        eff_index: usize,
        client_dest: Option<Position>,
    ) -> bool {
        use mangos_spells::defines::cast_result;
        use mangos_world_data::gameobject_template::TYPE_FISHINGNODE;
        let Some(caster_pos) = self.unit_position(caster) else {
            return true;
        };
        let eff = spell.effects[eff_index];
        #[allow(clippy::cast_sign_loss)]
        let entry = eff.misc_value.max(0) as u32;
        if entry == 0 {
            return true;
        }
        let Some(info) = self.stores.gameobject_templates.get(entry).cloned() else {
            return true;
        };
        if info.go_type != TYPE_FISHINGNODE {
            return true;
        }
        // Placement (fork `:5555-5580`): client dest,
        // else the radius point, else the random
        // range/angle point; both computed points are
        // LOS-clamped (`GetLosHitPosition`).
        let (fx, fy, _fz_in) = if let Some(dest) = client_dest {
            (dest.x, dest.y, dest.z)
        } else if eff.radius_index != 0 && spell.speed == 0.0 {
            let dis = self.effect_radius(spell, eff_index);
            let (fx, fy) = (
                caster_pos.x + dis * caster_pos.o.cos(),
                caster_pos.y + dis * caster_pos.o.sin(),
            );
            let fz = caster_pos.z;
            if let Some(hit) = self.get_los_hit_position(
                caster_pos.x,
                caster_pos.y,
                caster_pos.z + 0.5,
                fx,
                fy,
                fz,
                -1.5,
            ) {
                hit
            } else {
                (fx, fy, fz)
            }
        } else {
            let (rmin, rmax) = self
                .stores
                .spells
                .tables
                .ranges
                .get(&spell.range_index)
                .map_or((0.0, 0.0), |r| (r.min, r.max));
            let dis = self.spell_rng().frand01() * (rmax - rmin) + rmin;
            let bounding = self.bounding_radius_of(caster);
            let max_angle = if rmax + bounding > 0.0 {
                (rmax - rmin) / (rmax + bounding)
            } else {
                0.0
            };
            let angle_offset = max_angle * (self.spell_rng().frand01() - 0.5);
            let (fx, fy) = (
                caster_pos.x + dis * (caster_pos.o + angle_offset).cos(),
                caster_pos.y + dis * (caster_pos.o + angle_offset).sin(),
            );
            let fz = caster_pos.z;
            if let Some(hit) = self.get_los_hit_position(
                caster_pos.x,
                caster_pos.y,
                caster_pos.z + 2.0,
                fx,
                fy,
                fz,
                -1.5,
            ) {
                hit
            } else {
                (fx, fy, fz)
            }
        };
        // Swimmable gate (fork `:5601-5612`): fail when
        // dry or when the caster has no surface LOS.
        // `fz` rides the water surface on success
        // (fork `:5611-5613`).
        let Some(level) = self.fishing_water_level(fx, fy) else {
            self.send_cast_result(caster, spell.id, cast_result::FAILED_NOT_FISHABLE);
            return false;
        };
        if !self.is_in_line_of_sight(
            caster_pos.x,
            caster_pos.y,
            caster_pos.z + 2.0,
            fx,
            fy,
            level,
            false,
        ) {
            self.send_cast_result(caster, spell.id, cast_result::FAILED_NOT_FISHABLE);
            return false;
        }
        // The pre-surface height only feeds the LOS
        // clamps above; the node rides the surface.
        let fz = level;
        // Summon (fork `:5615-5663`): dynamic GO at
        // `GO_STATE_READY`, channel object on the
        // caster, owner registration, `respawn =
        // duration - lastSec + 5s`, owner/level/spell
        // stamps, then map add. The owner-group stamp
        // has no Rust surface; the summon sound has no
        // fork packet here (the splash sound lands on
        // the ready tick).
        let pos = Position::new(fx, fy, fz, caster_pos.o);
        let Some(bobber) = self.create_dynamic_gameobject(entry, pos, [0.0; 4]) else {
            return true;
        };
        let duration_ms = spell.calculate_duration_ms(&self.stores.spells.tables);
        let last_sec = [3, 7, 13, 17][self.spell_rng().irand(0, 3) as usize];
        #[allow(clippy::cast_sign_loss)]
        let secs = if duration_ms - last_sec * 1000 + 5_000 > 0 {
            (duration_ms - last_sec * 1000 + 5_000) / 1000
        } else {
            0
        };
        let now = self.now_unix;
        if let Some(go) = self.gameobjects.get_mut(&bobber) {
            go.set_respawn_time(secs as u32, now);
        }
        if let Some(u) = self.unit_base_mut(caster) {
            u.values().set_u64(
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_CHANNEL_OBJECT,
                bobber.raw(),
            );
        }
        self.unit_add_gameobject(caster, bobber);
        self.go_set_spell_id(bobber, spell.id);
        let level = self.unit_level(caster);
        let _ = self.object_set_field(bobber, u32::from(offsets::GAMEOBJECT_LEVEL), level);
        self.go_summon_linked_trap(bobber);
        if caster.is_creature_or_pet() {
            self.creature_script_just_summoned(caster, bobber);
        }
        true
    }

    /// `GameObject::LookupFishingHoleAround` (fork
    /// `GameObject.cpp:1303-1312` over
    /// `NearestGameObjectFishingHoleCheck`,
    /// `GridNotifiers.h:702-727`): nearest spawned
    /// `FISHINGHOLE` within range of the bobber and
    /// within its own radius (3D, like
    /// `IsWithinDistInMap`).
    pub(crate) fn lookup_fishing_hole_around(
        &self,
        bobber: ObjectGuid,
        range: f32,
    ) -> Option<ObjectGuid> {
        use mangos_world_data::gameobject_template::TYPE_FISHINGHOLE;
        let bobber_pos = self.gameobjects.get(&bobber).map(|g| g.world.position())?;
        let mut best: Option<(ObjectGuid, f32)> = None;
        for (guid, go) in &self.gameobjects {
            if go.info.go_type != TYPE_FISHINGHOLE || !go.is_spawned() {
                continue;
            }
            let p = go.world.position();
            let dist = ((p.x - bobber_pos.x).powi(2)
                + (p.y - bobber_pos.y).powi(2)
                + (p.z - bobber_pos.z).powi(2))
            .sqrt();
            #[allow(clippy::cast_precision_loss)] // DB radius words are small
            let radius = go.info.data_u32(0) as f32;
            if dist <= range && dist <= radius && best.is_none_or(|(_, bd)| dist < bd) {
                best = Some((*guid, dist));
            }
        }
        best.map(|(guid, _)| guid)
    }

    /// The `FISHINGNODE` splash broadcast (fork
    /// `PlayDistanceSound` + `SendGameObjectCustomAnim`
    /// to the set): packed-guid packets, matching the
    /// `SMSG_PLAY_OBJECT_SOUND` / custom-anim wire
    /// shapes.
    fn send_bobber_splash(&mut self, bobber: ObjectGuid) {
        let mut sound = PacketWriter::new();
        sound.write_u32_le(FISHING_SPLASH_SOUND);
        sound.write_bytes(&Self::packed_guid_bytes(bobber));
        let packet = WorldPacket::with_body(opcode_id::SMSG_PLAY_OBJECT_SOUND, sound.into_vec());
        self.broadcast_movement(bobber, &packet);
        let mut anim = PacketWriter::new();
        anim.write_bytes(&Self::packed_guid_bytes(bobber));
        anim.write_u32_le(0);
        let packet =
            WorldPacket::with_body(opcode_id::SMSG_GAMEOBJECT_CUSTOM_ANIM, anim.into_vec());
        self.broadcast_movement(bobber, &packet);
    }
}
