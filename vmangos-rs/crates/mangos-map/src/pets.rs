//! Pet runtime (Tier 1 Slice 2): `Unit::EffectSummonPet`
//! / `Pet::LoadPetFromDB` materialization, charm state
//! (action bar, react/command), follow + defensive
//! combat through the Chase generator, hunter pet XP,
//! and `character_pet` save snapshots emitted as
//! [`MapEvent`]s at the fork's trigger points.
//!
//! Deviations recorded in
//! `docs/rust-migration-evidence/TIER1-PROGRESS.md`
//! (Slice 2 inventory): no taming, stables, mini
//! pets, happiness/loyalty ticking, pet aura/cooldown
//! persistence, or guardian typing of temp summons.

use std::f32::consts::FRAC_PI_2;

use mangos_entities::creature::{
    Creature, DisplayResolver, PetKind, SpawnRng as _, UNIT_FLAG_PET_RENAME, VirtualItemPrototype,
};
use mangos_entities::unit::POWER_MANA;
use mangos_entities::{
    PET_SAVE_AS_CURRENT, PET_SAVE_LAST_STABLE_SLOT, PET_SAVE_NOT_IN_SLOT, PET_TYPE_HUNTER,
    PET_TYPE_SUMMON, PetRow,
};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::defines::effect;

use crate::map::{Map, MapEvent};
use crate::spells::{CastOptions, CastOutcome, CastTarget};

pub(crate) mod op {
    pub use mangos_protocol::opcode_table::opcode_id::*;
}

/// `PET_FOLLOW_DIST`.
pub const PET_FOLLOW_DIST: f32 = 2.0;
/// `PET_FOLLOW_ANGLE`.
pub const PET_FOLLOW_ANGLE: f32 = FRAC_PI_2;

/// `COMMAND_STAY`.
pub const COMMAND_STAY: u8 = 0;
/// `COMMAND_FOLLOW`.
pub const COMMAND_FOLLOW: u8 = 1;
/// `COMMAND_ATTACK`.
pub const COMMAND_ATTACK: u8 = 2;
/// `COMMAND_DISMISS`.
pub const COMMAND_DISMISS: u8 = 3;

/// `REACT_PASSIVE`.
pub const REACT_PASSIVE: u8 = 0;
/// `REACT_DEFENSIVE`.
pub const REACT_DEFENSIVE: u8 = 1;
/// `REACT_AGGRESSIVE`.
pub const REACT_AGGRESSIVE: u8 = 2;

/// `ACT_PASSIVE`.
pub const ACT_PASSIVE: u32 = 0x01;
/// `ACT_DISABLED`.
pub const ACT_DISABLED: u32 = 0x81;
/// `ACT_ENABLED` (autocast on).
pub const ACT_ENABLED: u32 = 0xC1;
/// `ACT_COMMAND`.
pub const ACT_COMMAND: u32 = 0x07;
/// `ACT_REACTION`.
pub const ACT_REACTION: u32 = 0x06;

/// `PET_SAVE_AS_DELETED` (no row written).
pub const PET_SAVE_AS_DELETED: i32 = -1;

/// `MAKE_UNIT_ACTION_BUTTON(A, T)`.
#[must_use]
pub fn make_action_button(action: u32, kind: u32) -> u32 {
    (action & 0x00FF_FFFF) | (kind << 24)
}

/// `CharmInfo::InitPetActionBar`: attack/follow/stay,
/// 4 disabled spell slots, aggressive/defensive/
/// passive. The fork writes `COMMAND_ATTACK - i` for
/// both command and reaction triples.
#[must_use]
pub fn default_action_bar() -> [(u32, u32); 10] {
    let mut bar = [(ACT_DISABLED, 0u32); 10];
    for i in 0..3u32 {
        bar[i as usize] = (ACT_COMMAND, 2 - i);
    }
    for i in 0..3u32 {
        bar[7 + i as usize] = (ACT_REACTION, 2 - i);
    }
    bar
}

/// One known pet spell (`PetSpell` state subset).
#[derive(Clone, Copy, Debug)]
pub struct PetSpell {
    pub spell_id: u32,
    /// `ActiveStates` wire value (autocast when
    /// `ACT_ENABLED`).
    pub active: u32,
}

/// Live charm state for one controlled pet
/// (`CharmInfo` + `Pet` members that persist).
#[derive(Clone, Debug)]
pub struct PetState {
    pub owner: ObjectGuid,
    pub pet_number: u32,
    /// `PET_TYPE_SUMMON` / `PET_TYPE_HUNTER`.
    pub pet_type: u32,
    pub created_by_spell: u32,
    pub react_state: u8,
    pub command_state: u8,
    /// Explicit `COMMAND_ATTACK` victim (fork keeps it
    /// as the pet AI's attack target).
    pub commanded_victim: Option<ObjectGuid>,
    pub action_bar: [(u32, u32); 10],
    pub spells: Vec<PetSpell>,
    pub teach_spells: Vec<(u32, u32)>,
    pub loyalty_points: i32,
    pub training_points: i32,
    pub reset_talents_cost: u32,
    pub reset_talents_time: u64,
    /// `Pet::m_enabled` (action bar usability).
    pub enabled: bool,
}

impl PetState {
    fn is_hunter(&self) -> bool {
        self.pet_type == PET_TYPE_HUNTER
    }
}

/// Where a materialized pet came from, for the
/// health/mana/death rules of `LoadPetFromDB`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PetLoadKind {
    /// Fresh `EffectSummonPet` creation.
    New,
    /// DB row loaded as the login pet (`current`).
    Current,
    /// DB row loaded by a summon spell.
    Called,
}

impl Map {
    /// The controlled pet of `owner`, if any
    /// (`Unit::GetPet` by `UNIT_FIELD_SUMMON` guid).
    #[must_use]
    pub fn pet_of(&self, owner: ObjectGuid) -> Option<ObjectGuid> {
        self.pets
            .iter()
            .find(|(_, st)| st.owner == owner)
            .map(|(guid, _)| *guid)
    }

    /// `IsPet` for guids this runtime manages.
    #[must_use]
    pub fn is_pet(&self, guid: ObjectGuid) -> bool {
        self.pets.contains_key(&guid)
    }

    /// Pet owner for threat/XP attribution
    /// (`GetCharmerOrOwnerPlayerOrPlayerItself` pet arm).
    #[must_use]
    pub fn pet_owner(&self, guid: ObjectGuid) -> Option<ObjectGuid> {
        self.pets.get(&guid).map(|st| st.owner)
    }

    /// `Spell::EffectSummonPet` + `Unit::EffectSummonPet`
    /// for the controlled-pet subset. Returns the pet
    /// guid on success.
    pub fn summon_pet_effect(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        pet_entry: u32,
        pet_level: u32,
    ) -> Option<ObjectGuid> {
        // UnsummonOldPetBeforeNewSummon(entry, true).
        if let Some(old) = self.pet_of(caster) {
            let old_entry = self.creatures.get(&old).map_or(0, |c| c.original_entry);
            let old_dead = !self.unit_is_alive(old);
            let old_hunter = self.pets.get(&old).is_some_and(PetState::is_hunter);
            if old_dead && (pet_entry == 0 || old_entry == pet_entry) {
                if pet_entry != 0 {
                    self.unsummon_pet(old, PET_SAVE_NOT_IN_SLOT);
                } else {
                    return None; // corpse-state pet can't be unsummoned
                }
            } else if caster.is_player() {
                if pet_entry != 0 {
                    let mode = if old_hunter {
                        PET_SAVE_AS_DELETED_MODE
                    } else {
                        PET_SAVE_NOT_IN_SLOT
                    };
                    self.unsummon_pet(old, mode);
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }

        // Player casters load rows first (incl. Call Pet).
        if caster.is_player()
            && let Some(guid) = self.load_pet_from_rows(caster, pet_entry, PetLoadKind::Called)
        {
            return Some(guid);
        }
        if pet_entry == 0 {
            return None; // hunter call-pet with no pet: silent
        }
        let template = self.stores.creature_templates.get(pet_entry)?.clone();
        let pet_number = self.stores.generate_pet_number();
        let level = pet_level.max(1);
        let guid = self.materialize_pet(
            caster,
            pet_number,
            template,
            PetKind::Summon,
            level,
            None,
            PetLoadKind::New,
        )?;
        // New summon pet row state (fork field writes).
        let name = self.generate_pet_name(pet_entry);
        let mut spells = Vec::new();
        let mut teach = Vec::new();
        let mut used_tp = 0i32;
        self.init_pet_create_spells(caster, pet_entry, &mut spells, &mut teach, &mut used_tp);
        if let Some(c) = self.creatures.get_mut(&guid) {
            c.unit.world.name = name;
            let now = self.now_unix;
            #[allow(clippy::cast_possible_truncation)]
            {
                let v = c.unit.world.object.values_mut();
                v.set_u32(offsets::UNIT_FIELD_PET_NAME_TIMESTAMP, now as u32);
                v.set_u32(offsets::UNIT_FIELD_PETEXPERIENCE, 0);
                v.set_u32(offsets::UNIT_FIELD_PETNEXTLEVELEXP, 1000);
                v.set_u32(offsets::UNIT_CREATED_BY_SPELL, spell_id);
            }
            let max_health = c.unit.max_health();
            c.unit.set_health(max_health);
            let max_mana = c.unit.max_power(POWER_MANA);
            c.unit.set_power(POWER_MANA, max_mana);
        }
        let state = PetState {
            owner: caster,
            pet_number,
            pet_type: PET_TYPE_SUMMON,
            created_by_spell: spell_id,
            react_state: if caster.is_player() {
                REACT_DEFENSIVE
            } else {
                REACT_AGGRESSIVE
            },
            command_state: COMMAND_FOLLOW,
            commanded_victim: None,
            action_bar: default_action_bar(),
            spells,
            teach_spells: teach,
            loyalty_points: 0,
            training_points: -used_tp,
            reset_talents_cost: 0,
            reset_talents_time: 0,
            enabled: true,
        };
        self.register_pet(guid, state);
        if caster.is_player() {
            // Fork saves immediately + pushes the row
            // into its cache; mirror into deferred rows
            // so a same-session re-summon loads it.
            let row = self.pet_snapshot(guid, PET_SAVE_AS_CURRENT);
            if let Some(row) = row {
                self.upsert_pet_row(caster, row.clone());
                self.events.push(MapEvent::SavePet {
                    row: Box::new(row),
                    mode: PET_SAVE_AS_CURRENT,
                });
            }
            self.send_pet_spells(caster, guid);
        }
        Some(guid)
    }

    /// `Player::LoadPet` at world entry: the slot-0
    /// row (temporary summons excluded by the fork).
    pub fn load_current_pet(&mut self, owner: ObjectGuid) {
        if self.pet_of(owner).is_some() {
            return;
        }
        let has_current = self.players.get(&owner).is_some_and(|e| {
            e.player
                .deferred
                .pets
                .iter()
                .any(|r| r.slot == 0 && r.entry != 0)
        });
        if has_current {
            self.load_pet_from_rows(owner, 0, PetLoadKind::Current);
        }
    }

    /// `LoadPetFromDB` selection over the player's
    /// loaded rows. `pet_entry == 0` loads any
    /// current/non-stabled row (Call Pet / login).
    fn load_pet_from_rows(
        &mut self,
        owner: ObjectGuid,
        pet_entry: u32,
        kind: PetLoadKind,
    ) -> Option<ObjectGuid> {
        let row = {
            let entry = self.players.get(&owner)?;
            entry
                .player
                .deferred
                .pets
                .iter()
                .find(|r| {
                    r.entry != 0 && {
                        if kind == PetLoadKind::Current {
                            r.slot == PET_SAVE_AS_CURRENT
                        } else if pet_entry != 0 {
                            r.entry == pet_entry
                                && (r.slot == PET_SAVE_AS_CURRENT
                                    || r.slot > PET_SAVE_LAST_STABLE_SLOT)
                        } else {
                            r.slot == PET_SAVE_AS_CURRENT || r.slot > PET_SAVE_LAST_STABLE_SLOT
                        }
                    }
                })
                // Fork takes the first matching row.
                .cloned()?
        };
        self.stores.observe_pet_number(row.id);
        let template = self.stores.creature_templates.get(row.entry)?.clone();
        // Temporary summoned pets never load as current.
        if kind == PetLoadKind::Current
            && let Some(spell) = self.stores.spells.get(row.created_by_spell)
            && spell.calculate_duration_ms(&self.stores.spells.tables) > 0
        {
            return None;
        }
        let pet_kind = if row.pet_type == PET_TYPE_HUNTER {
            if !template.is_tameable() {
                return None;
            }
            PetKind::Hunter
        } else {
            PetKind::Summon
        };
        let level = if row.pet_type == PET_TYPE_SUMMON {
            self.unit_level(owner).max(1)
        } else {
            row.level.max(1)
        };
        let guid =
            self.materialize_pet(owner, row.id, template, pet_kind, level, Some(&row), kind)?;
        // Slot promotion (fork UPDATE pair + cache).
        if row.slot != PET_SAVE_AS_CURRENT {
            let owner_low = owner.counter();
            self.events.push(MapEvent::PromotePet {
                owner_low,
                pet_id: row.id,
            });
            if let Some(entry) = self.players.get_mut(&owner) {
                for r in &mut entry.player.deferred.pets {
                    if r.slot == PET_SAVE_AS_CURRENT && r.id != row.id {
                        r.slot = PET_SAVE_NOT_IN_SLOT;
                    }
                    if r.id == row.id {
                        r.slot = PET_SAVE_AS_CURRENT;
                    }
                }
            }
        }
        let state = PetState {
            owner,
            pet_number: row.id,
            pet_type: row.pet_type,
            created_by_spell: row.created_by_spell,
            #[allow(clippy::cast_possible_truncation)]
            react_state: row.react_state as u8,
            command_state: COMMAND_FOLLOW,
            commanded_victim: None,
            action_bar: cleanup_action_bar(row.action_bar, &row.spells),
            spells: row
                .spells
                .iter()
                .map(|(spell_id, active)| PetSpell {
                    spell_id: *spell_id,
                    active: *active,
                })
                .collect(),
            teach_spells: row.teach_spells.clone(),
            loyalty_points: row.loyalty_points,
            training_points: row.training_points,
            reset_talents_cost: row.reset_talents_cost,
            reset_talents_time: row.reset_talents_time,
            enabled: true,
        };
        self.register_pet(guid, state);
        self.send_pet_spells(owner, guid);
        Some(guid)
    }

    /// Spawn the creature aggregate and insert it
    /// into the map (fork `Create` + `Map::Add`).
    #[allow(clippy::too_many_arguments)]
    fn materialize_pet(
        &mut self,
        owner: ObjectGuid,
        pet_number: u32,
        template: mangos_world_data::creature_template::CreatureInfo,
        kind: PetKind,
        level: u32,
        row: Option<&PetRow>,
        load_kind: PetLoadKind,
    ) -> Option<ObjectGuid> {
        let owner_pos = self.unit_position(owner)?;
        let entry = template.entry;
        let level_info_entry = if kind == PetKind::Hunter { 1 } else { entry };
        let level_info = self
            .stores
            .pets
            .level_info(level_info_entry, level)
            .copied();
        self.summon_counter += 1;
        let guid_low = self.summon_counter;
        let stores = &self.stores;
        let resolver = DisplayResolver {
            scale_for_display: &|id| stores.scale_for_display(id),
            model_for_display: &|id| stores.model_for_display(id),
        };
        let item_store = &stores.items;
        let items = |id: u32| {
            item_store.get(id).map(|p| VirtualItemPrototype {
                display_id: p.display_id,
                class: p.class,
                subclass: p.subclass,
                material: p.material,
                inventory_type: p.inventory_type,
                sheath: p.sheath,
            })
        };
        let rates = self.config.creature_rates;
        let mut creature = Creature::spawn_pet(
            guid_low,
            pet_number,
            template,
            kind,
            level,
            level_info.as_ref(),
            owner.is_player(),
            &stores.equipment,
            &items,
            &stores.cls,
            &stores.display_addons,
            &resolver,
            &rates,
            &mut self.rng,
        )
        .ok()?;
        // CreatureCreatePos(owner, owner_o, FOLLOW_DIST,
        // FOLLOW_ANGLE): near point at (o + angle).
        let radius = creature
            .unit
            .world
            .object
            .values()
            .get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS)
            + PET_FOLLOW_DIST;
        let abs_angle = owner_pos.o + PET_FOLLOW_ANGLE;
        let pos = Position::new(
            owner_pos.x + radius * abs_angle.cos(),
            owner_pos.y + radius * abs_angle.sin(),
            owner_pos.z,
            owner_pos.o,
        );
        creature.unit.world.map_id = self.map_id;
        creature.unit.world.instance_id = self.instance_id;
        creature.unit.world.relocate(pos);
        creature.home_position = pos;
        creature.react_state = match row {
            Some(r) => {
                #[allow(clippy::cast_possible_truncation)]
                {
                    r.react_state as u8
                }
            }
            None => REACT_DEFENSIVE,
        };
        // Owner linkage fields (fork SetOwnerGuid /
        // SetCreatorGuid write SUMMONEDBY/CREATEDBY).
        let owner_faction = self
            .unit_base(owner)
            .map_or(0, mangos_entities::unit::UnitBase::faction_template);
        {
            let v = creature.unit.world.object.values_mut();
            v.set_guid(offsets::UNIT_FIELD_SUMMONEDBY, owner);
            v.set_guid(offsets::UNIT_FIELD_CREATEDBY, owner);
            v.set_u32(offsets::UNIT_FIELD_PETNUMBER, pet_number);
            v.set_u32(offsets::UNIT_NPC_FLAGS, 0);
            #[allow(clippy::cast_possible_truncation)]
            v.set_u32(offsets::UNIT_FIELD_PET_NAME_TIMESTAMP, self.now_unix as u32);
        }
        creature.unit.set_faction_template(owner_faction);
        if let Some(r) = row {
            self.apply_pet_row(&mut creature, r, kind, load_kind, level);
        }
        creature.unit.world.object.set_is_new_object(true);
        creature.unit.world.object.add_to_world();
        let guid = creature.unit.guid();
        let cell = Self::cell_of(creature.unit.world.position());
        self.cells.entry(cell).or_default().creatures.insert(guid);
        self.creatures.insert(guid, creature);
        self.motion_initialize(guid);
        Some(guid)
    }

    /// Row application tail of `LoadPetFromDB`
    /// (display, name, xp fields, health/mana,
    /// hunter loyalty/power, death-on-load).
    fn apply_pet_row(
        &self,
        creature: &mut Creature,
        row: &PetRow,
        kind: PetKind,
        load_kind: PetLoadKind,
        level: u32,
    ) {
        if row.display_id != 0 {
            creature.unit.set_native_display_id(row.display_id);
            creature.unit.set_display_id(
                row.display_id,
                self.stores.model_for_display(row.display_id),
            );
        }
        if !row.name.is_empty() {
            creature.unit.world.name.clone_from(&row.name);
        }
        let saved_health = row.current_health;
        let saved_mana = row.current_mana;
        {
            let v = creature.unit.world.object.values_mut();
            v.set_u32(offsets::UNIT_CREATED_BY_SPELL, row.created_by_spell);
            v.set_u32(offsets::UNIT_FIELD_PETEXPERIENCE, row.xp);
            if kind == PetKind::Hunter {
                v.set_u32(
                    offsets::UNIT_FIELD_PETNEXTLEVELEXP,
                    self.xp_for_pet_level(level),
                );
                // Loyalty level byte + rename flag.
                #[allow(clippy::cast_possible_truncation)]
                v.set_byte(offsets::UNIT_FIELD_BYTES_1, 2, row.loyalty as u8);
                if row.renamed != 0 {
                    v.remove_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PET_RENAME);
                } else {
                    v.set_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PET_RENAME);
                }
            } else {
                v.set_u32(offsets::UNIT_FIELD_PETNEXTLEVELEXP, 1000);
            }
        }
        if kind == PetKind::Hunter {
            // Happiness max 1050000-ish create value is
            // GetCreatePowers(HAPPINESS); unit register
            // default applies. Power type focus.
            let max_happiness = creature.unit.create_power(crate::pets::POWER_HAPPINESS_IDX);
            creature
                .unit
                .set_max_power(crate::pets::POWER_HAPPINESS_IDX, max_happiness);
            creature
                .unit
                .set_power(crate::pets::POWER_HAPPINESS_IDX, row.current_happiness);
            creature
                .unit
                .world
                .object
                .values_mut()
                .set_byte(offsets::UNIT_FIELD_BYTES_0, 3, 2); // POWER_FOCUS
        }
        let full = kind == PetKind::Summon && load_kind == PetLoadKind::Called;
        let max_health = creature.unit.max_health();
        let max_mana = creature.unit.max_power(POWER_MANA);
        if full {
            creature.unit.set_health(max_health);
            creature.unit.set_power(POWER_MANA, max_mana);
        } else {
            creature.unit.set_health(saved_health.min(max_health));
            creature
                .unit
                .set_power(POWER_MANA, saved_mana.min(max_mana));
        }
        if load_kind == PetLoadKind::Current && saved_health == 0 {
            creature.unit.death_state = mangos_entities::unit::DeathState::JustDied;
            creature.unit.set_health(0);
        }
    }

    /// Register live state + owner fields (`SetPet`).
    fn register_pet(&mut self, guid: ObjectGuid, state: PetState) {
        let owner = state.owner;
        let react = state.react_state;
        if let Some(c) = self.creatures.get_mut(&guid) {
            c.react_state = react;
        }
        self.pets.insert(guid, state);
        if let Some(u) = self.unit_base_mut(owner) {
            let v = u.world.object.values_mut();
            v.set_guid(offsets::UNIT_FIELD_SUMMON, guid);
        }
        if owner.is_player()
            && let Some(entry) = self.players.get_mut(&owner)
        {
            let v = entry.player.unit.world.object.values_mut();
            let cur = v.get_byte(offsets::PLAYER_FIELD_BYTES, 0);
            v.set_byte(offsets::PLAYER_FIELD_BYTES, 0, cur | 0x01);
        }
    }

    /// Clear owner linkage (`SetPet(nullptr)`).
    fn unregister_pet(&mut self, guid: ObjectGuid) {
        let Some(state) = self.pets.remove(&guid) else {
            return;
        };
        if let Some(u) = self.unit_base_mut(state.owner) {
            let v = u.world.object.values_mut();
            v.set_guid(offsets::UNIT_FIELD_SUMMON, ObjectGuid::EMPTY);
        }
        if state.owner.is_player()
            && let Some(entry) = self.players.get_mut(&state.owner)
        {
            let v = entry.player.unit.world.object.values_mut();
            let cur = v.get_byte(offsets::PLAYER_FIELD_BYTES, 0);
            v.set_byte(offsets::PLAYER_FIELD_BYTES, 0, cur & !0x01);
        }
    }
}

/// `PET_SAVE_AS_DELETED` as an unsigned mode marker
/// for `unsummon_pet` (delete, no row write).
pub const PET_SAVE_AS_DELETED_MODE: u32 = u32::MAX;

/// `POWER_HAPPINESS` index (unit crate constant
/// re-exported for the row application).
pub const POWER_HAPPINESS_IDX: usize = mangos_entities::unit::POWER_HAPPINESS;

/// `CleanupActionBar`: drop bar spell entries the
/// pet doesn't know (type becomes `ACT_DISABLED`/0 in
/// the fork for unknown spells — it zeroes the
/// action; spell slots only).
fn cleanup_action_bar(mut bar: [(u32, u32); 10], spells: &[(u32, u32)]) -> [(u32, u32); 10] {
    for slot in &mut bar {
        if matches!(slot.0, ACT_DISABLED | ACT_ENABLED | ACT_PASSIVE)
            && slot.1 != 0
            && !spells.iter().any(|(s, _)| *s == slot.1)
        {
            *slot = (ACT_DISABLED, 0);
        }
    }
    bar
}

impl Map {
    /// `Pet::Unsummon(mode, owner)` runtime half:
    /// combat stop, action-bar removal, save/delete
    /// snapshot, despawn. `mode` is a `PET_SAVE` slot or
    /// [`PET_SAVE_AS_DELETED_MODE`].
    pub fn unsummon_pet(&mut self, guid: ObjectGuid, mode: u32) {
        let Some(state) = self.pets.get(&guid).cloned() else {
            return;
        };
        self.attack_stop(guid);
        self.cancel_cast(guid, false);
        if state.owner.is_player() {
            // RemovePetActionBar: SMSG_PET_SPELLS with
            // an empty guid.
            let mut payload = PacketWriter::new();
            payload.write_u64_le(0);
            let packet = WorldPacket::with_body(op::SMSG_PET_SPELLS, payload.into_vec());
            self.send_to_player(state.owner, packet);
        }
        if mode == PET_SAVE_AS_DELETED_MODE {
            self.events.push(MapEvent::DeletePet {
                pet_id: state.pet_number,
            });
            if let Some(entry) = self.players.get_mut(&state.owner) {
                entry
                    .player
                    .deferred
                    .pets
                    .retain(|r| r.id != state.pet_number);
            }
        } else if let Some(row) = self.pet_snapshot(guid, mode) {
            // Dead pets never save AS_CURRENT (fork).
            let mode = if mode == PET_SAVE_AS_CURRENT && !self.unit_is_alive(guid) {
                PET_SAVE_NOT_IN_SLOT
            } else {
                mode
            };
            let mut row = row;
            row.slot = mode;
            if state.owner.is_player() {
                self.upsert_pet_row(state.owner, row.clone());
                self.events.push(MapEvent::SavePet {
                    row: Box::new(row),
                    mode,
                });
            }
        }
        self.unregister_pet(guid);
        self.despawn_creature_now(guid, u32::MAX);
    }

    /// Unsummon the owner's pet if present (logout
    /// `RemovePet(PET_SAVE_AS_CURRENT)`).
    pub fn unsummon_owner_pet(&mut self, owner: ObjectGuid, mode: u32) {
        if let Some(pet) = self.pet_of(owner) {
            self.unsummon_pet(pet, mode);
        }
    }

    /// Live pet -> `character_pet` row (the field
    /// reads of `SavePetToDB`).
    #[must_use]
    pub fn pet_snapshot(&self, guid: ObjectGuid, mode: u32) -> Option<PetRow> {
        let state = self.pets.get(&guid)?;
        let creature = self.creatures.get(&guid)?;
        let v = creature.unit.world.object.values();
        let flags = v.get_u32(offsets::UNIT_FIELD_FLAGS);
        Some(PetRow {
            id: state.pet_number,
            entry: creature.original_entry,
            owner_guid: state.owner.counter(),
            display_id: creature.unit.native_display_id(),
            level: creature.unit.level(),
            xp: v.get_u32(offsets::UNIT_FIELD_PETEXPERIENCE),
            react_state: u32::from(state.react_state),
            loyalty_points: state.loyalty_points,
            loyalty: u32::from(v.get_byte(offsets::UNIT_FIELD_BYTES_1, 2)),
            training_points: state.training_points,
            slot: mode,
            name: creature.unit.world.name.clone(),
            renamed: u32::from(flags & UNIT_FLAG_PET_RENAME == 0),
            current_health: creature.unit.health(),
            current_mana: creature.unit.power(POWER_MANA),
            current_happiness: creature.unit.power(POWER_HAPPINESS_IDX),
            action_bar: state.action_bar,
            teach_spells: state.teach_spells.clone(),
            save_time: self.now_unix,
            reset_talents_cost: state.reset_talents_cost,
            reset_talents_time: state.reset_talents_time,
            created_by_spell: v.get_u32(offsets::UNIT_CREATED_BY_SPELL),
            pet_type: state.pet_type,
            spells: state
                .spells
                .iter()
                .map(|s| (s.spell_id, s.active))
                .collect(),
        })
    }

    fn upsert_pet_row(&mut self, owner: ObjectGuid, row: PetRow) {
        if let Some(entry) = self.players.get_mut(&owner) {
            if let Some(existing) = entry
                .player
                .deferred
                .pets
                .iter_mut()
                .find(|r| r.id == row.id)
            {
                *existing = row;
            } else {
                entry.player.deferred.pets.push(row);
            }
        }
    }

    /// `ObjectMgr::GeneratePetName` with the family /
    /// template-name fallback.
    fn generate_pet_name(&mut self, entry: u32) -> String {
        if let Some(name) = self
            .stores
            .pets
            .generated_name(entry, &mut |lo, hi| self.rng.urand(lo, hi))
        {
            return name;
        }
        self.stores
            .creature_templates
            .get(entry)
            .map_or_else(|| "Pet".to_string(), |t| t.name.clone())
    }

    /// `Pet::InitPetCreateSpells` (create-spell list,
    /// teach-spell registration for hunter owners,
    /// training-point debit). Family passives
    /// (`LearnPetPassives`) and owner pet auras
    /// (`CastPetAuras`) are unmodeled — no
    /// `sPetFamilySpellsStore` equivalent is loaded
    /// yet; recorded in the Slice 2 inventory.
    fn init_pet_create_spells(
        &mut self,
        owner: ObjectGuid,
        entry: u32,
        spells: &mut Vec<PetSpell>,
        teach: &mut Vec<(u32, u32)>,
        used_tp: &mut i32,
    ) {
        let Some(create_ids) = self.stores.pets.create_spells.get(&entry).copied() else {
            return;
        };
        for id in create_ids {
            if id == 0 {
                break;
            }
            let Some(learn_proto) = self.stores.spells.get(id).cloned() else {
                continue;
            };
            let first = learn_proto.effects[0];
            let pet_spell_id =
                if first.effect == effect::LEARN_SPELL || first.effect == effect::LEARN_PET_SPELL {
                    let taught = first.trigger_spell;
                    if owner.is_player() && !self.player_knows_spell(owner, id) {
                        // `Spells::IsPassiveSpell`:
                        // SPELL_ATTR_PASSIVE.
                        let passive = self
                            .stores
                            .spells
                            .get(taught)
                            .is_some_and(|s| s.attributes & 0x0000_0040 != 0);
                        if passive {
                            // Owner learns the passive
                            // directly (fork LearnSpell).
                            if let Some(entry) = self.players.get_mut(&owner) {
                                entry.player.learn_spell(id);
                            }
                        } else {
                            teach.push((taught, id));
                        }
                    }
                    taught
                } else {
                    id
                };
            if pet_spell_id != 0 && !spells.iter().any(|s| s.spell_id == pet_spell_id) {
                spells.push(PetSpell {
                    spell_id: pet_spell_id,
                    active: ACT_DISABLED,
                });
            }
            *used_tp += self.training_points_for_spell(first.trigger_spell);
        }
    }

    /// `GetTPForSpell` core: `SkillLineAbility`
    /// `reqtrainpoints` of the first matching row.
    /// No `SkillLineAbility` store is loaded yet, so
    /// training costs read as 0 (recorded deviation;
    /// hunter-pet TP economy is balance-only here).
    #[allow(clippy::unused_self)] // grows with the SkillLineAbility store
    fn training_points_for_spell(&self, _spell_id: u32) -> i32 {
        0
    }

    /// `ObjectMgr::GetXPForPetLevel`.
    #[must_use]
    pub fn xp_for_pet_level(&self, level: u32) -> u32 {
        self.stores.player_infos.xp_for_level(level) / 4
    }

    /// `Pet::GivePetXP` (hunter pets only; personal
    /// XP-rate customization is a fork feature the
    /// Rust player does not carry yet).
    pub fn give_pet_xp(&mut self, pet: ObjectGuid, xp: u32) {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if !state.is_hunter() || xp < 1 || !self.unit_is_alive(pet) {
            return;
        }
        let owner_level = self.unit_level(state.owner);
        let max_level = owner_level.min(60);
        let mut level = self.unit_level(pet);
        if level >= max_level {
            return;
        }
        let (cur_xp, mut next_xp) = {
            let Some(c) = self.creatures.get(&pet) else {
                return;
            };
            let v = c.unit.world.object.values();
            (
                v.get_u32(offsets::UNIT_FIELD_PETEXPERIENCE),
                v.get_u32(offsets::UNIT_FIELD_PETNEXTLEVELEXP),
            )
        };
        let mut new_xp = cur_xp + xp;
        while new_xp >= next_xp && level < max_level {
            new_xp -= next_xp;
            level += 1;
            self.give_pet_level(pet, level);
            next_xp = self.xp_for_pet_level(level);
        }
        if let Some(c) = self.creatures.get_mut(&pet) {
            c.unit.world.object.values_mut().set_u32(
                offsets::UNIT_FIELD_PETEXPERIENCE,
                if level < max_level { new_xp } else { 0 },
            );
        }
    }

    /// `Pet::GivePetLevel`.
    fn give_pet_level(&mut self, pet: ObjectGuid, level: u32) {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if level == 0 || level == self.unit_level(pet) {
            return;
        }
        let loyalty = self.creatures.get(&pet).map_or(1, |c| {
            c.unit
                .world
                .object
                .values()
                .get_byte(offsets::UNIT_FIELD_BYTES_1, 2)
        });
        let kind = if state.is_hunter() {
            PetKind::Hunter
        } else {
            PetKind::Summon
        };
        let entry = self.creatures.get(&pet).map_or(0, |c| c.original_entry);
        let info_entry = if state.is_hunter() { 1 } else { entry };
        let level_info = self.stores.pets.level_info(info_entry, level).copied();
        let cls = self.stores.cls.clone();
        let rates = self.config.creature_rates;
        let next_level_xp = self.xp_for_pet_level(level);
        if let Some(c) = self.creatures.get_mut(&pet) {
            if state.is_hunter() {
                let v = c.unit.world.object.values_mut();
                v.set_u32(offsets::UNIT_FIELD_PETEXPERIENCE, 0);
                v.set_u32(offsets::UNIT_FIELD_PETNEXTLEVELEXP, next_level_xp);
            }
            c.init_pet_stats_for_level(
                level,
                kind,
                level_info.as_ref(),
                state.owner.is_player(),
                &cls,
                &rates,
            );
        }
        if let Some(st) = self.pets.get_mut(&pet) {
            st.training_points += i32::from(loyalty) - 1;
        }
    }

    /// `Spell::EffectLearnPetSpell`.
    pub fn learn_pet_spell_effect(&mut self, owner: ObjectGuid, learn_spell_id: u32) {
        let Some(pet) = self.pet_of(owner) else {
            return;
        };
        if !self.unit_is_alive(pet) {
            return;
        }
        if !self.can_learn_pet_spell(pet, learn_spell_id) {
            return;
        }
        let cost = self.training_points_for_spell(learn_spell_id);
        if let Some(st) = self.pets.get_mut(&pet) {
            st.training_points -= cost;
            if !st.spells.iter().any(|s| s.spell_id == learn_spell_id) {
                st.spells.push(PetSpell {
                    spell_id: learn_spell_id,
                    active: ACT_DISABLED,
                });
            }
            // Fork LearnSpell also fills the first
            // empty bar spell slot.
            if !st.action_bar.iter().any(|(_, a)| *a == learn_spell_id)
                && let Some(slot) = st
                    .action_bar
                    .iter_mut()
                    .find(|(t, a)| *t == ACT_DISABLED && *a == 0)
            {
                *slot = (ACT_DISABLED, learn_spell_id);
            }
        }
        if let Some(row) = self.pet_snapshot(pet, PET_SAVE_AS_CURRENT) {
            self.upsert_pet_row(owner, row.clone());
            self.events.push(MapEvent::SavePet {
                row: Box::new(row),
                mode: PET_SAVE_AS_CURRENT,
            });
        }
        self.send_pet_spells(owner, pet);
    }

    /// `Pet::CanLearnPetSpell` subset: not already
    /// known at an equal/higher rank, enough training
    /// points, pet level meets the spell's level
    /// requirement. (Family/skill-line gating beyond
    /// TP cost rides the spell's own data; the fork's
    /// `GetPetFamily`-skill checks are recorded as
    /// unmodeled remainder.)
    fn can_learn_pet_spell(&self, pet: ObjectGuid, spell_id: u32) -> bool {
        let Some(state) = self.pets.get(&pet) else {
            return false;
        };
        if state.spells.iter().any(|s| s.spell_id == spell_id) {
            return false;
        }
        let Some(spell) = self.stores.spells.get(spell_id) else {
            return false;
        };
        if spell.spell_level > self.unit_level(pet) {
            return false;
        }
        state.training_points >= self.training_points_for_spell(spell_id)
    }

    /// `Player::PetSpellInitialize` packet for the
    /// owner's session.
    pub fn send_pet_spells(&mut self, owner: ObjectGuid, pet: ObjectGuid) {
        let Some(packet) = self.build_pet_spells(pet) else {
            return;
        };
        self.send_to_player(owner, packet);
    }

    fn build_pet_spells(&self, pet: ObjectGuid) -> Option<WorldPacket> {
        let state = self.pets.get(&pet)?;
        let mut payload = PacketWriter::new();
        payload.write_u64_le(pet.raw());
        payload.write_u32_le(0);
        payload.write_u8(state.react_state);
        payload.write_u8(state.command_state);
        payload.write_u8(0);
        payload.write_u8(if state.enabled { 0 } else { 0x8 });
        for (kind, action) in state.action_bar {
            payload.write_u32_le(make_action_button(action, kind));
        }
        // Permanent pets of players list known spells.
        let list_spells = state.owner.is_player();
        if list_spells {
            #[allow(clippy::cast_possible_truncation)]
            payload.write_u8(state.spells.len() as u8);
            for s in &state.spells {
                payload.write_u32_le(make_action_button(s.spell_id, s.active));
            }
        } else {
            payload.write_u8(0);
        }
        // WritePetSpellsCooldown.
        let mut cooldowns: Vec<(u32, u16, u32, u32)> = Vec::new();
        for ((caster, key), entry) in &self.cooldowns {
            if *caster != pet || *key & 0x8000_0000 != 0 {
                continue;
            }
            let spell_cd = entry.expiry_ms.saturating_sub(self.now_ms);
            let category = self.stores.spells.get(*key).map_or(0, |s| s.category);
            let mut cat_cd = if category != 0 {
                self.cooldowns
                    .get(&(pet, category | 0x8000_0000))
                    .map_or(0, |e| e.expiry_ms.saturating_sub(self.now_ms))
            } else {
                0
            };
            if spell_cd == 0 && cat_cd == 0 && !entry.permanent {
                continue;
            }
            if entry.permanent {
                cat_cd |= 0x0800_0000;
            }
            #[allow(clippy::cast_possible_truncation)]
            cooldowns.push((*key, category as u16, spell_cd, cat_cd));
        }
        #[allow(clippy::cast_possible_truncation)]
        payload.write_u16_le(cooldowns.len() as u16);
        for (spell, category, spell_cd, cat_cd) in cooldowns {
            payload.write_u32_le(spell);
            payload.write_u16_le(category);
            payload.write_u32_le(spell_cd);
            payload.write_u32_le(cat_cd);
        }
        Some(WorldPacket::with_body(
            op::SMSG_PET_SPELLS,
            payload.into_vec(),
        ))
    }
}

impl Map {
    /// `PetAI::UpdateAI` subset: commanded/defensive
    /// victim selection, chase + melee through the
    /// shared combat paths, autocast, follow/stay.
    /// Owner-attacker discovery uses melee attackers
    /// (`bot_attackers_of`); the fork's attacker set
    /// also includes pure spell attackers (recorded).
    pub(crate) fn pet_ai_update(&mut self, guid: ObjectGuid) {
        let Some(state) = self.pets.get(&guid).cloned() else {
            return;
        };
        let owner = state.owner;
        if self.unit_position(owner).is_none() {
            return;
        }
        if self.casts.contains_key(&guid) {
            return;
        }
        let current_victim = self
            .unit_base(guid)
            .and_then(|u| u.melee_victim)
            .filter(|v| self.unit_is_alive(*v));
        let mut victim = state
            .commanded_victim
            .filter(|v| self.unit_is_alive(*v))
            .or(current_victim);
        if state.react_state == REACT_PASSIVE && state.commanded_victim.is_none() {
            victim = None;
        }
        if victim.is_none() && state.react_state != REACT_PASSIVE {
            let mut candidates = self.bot_attackers_of(owner);
            candidates.extend(self.bot_attackers_of(guid));
            victim = candidates
                .into_iter()
                .find(|a| *a != guid && *a != owner && self.unit_is_alive(*a));
        }
        if victim.is_none() && state.react_state == REACT_AGGRESSIVE {
            victim = self
                .unit_base(owner)
                .and_then(|u| u.melee_victim)
                .filter(|v| *v != guid && self.unit_is_alive(*v));
        }
        if let Some(victim) = victim {
            if current_victim != Some(victim) {
                self.attack_start(guid, victim);
                // The victim fights back (`SetInCombatWith`
                // in `Unit::Attack`): seed its threat so
                // creature AI can select the pet.
                if self.creatures.contains_key(&victim) {
                    self.add_threat(victim, guid, 0.0);
                    self.set_in_combat_flag(victim, true);
                    if let Some(ai) = self.ai.get_mut(&victim) {
                        ai.in_combat = true;
                    }
                }
            }
            if self.motion_type_of(guid) != crate::motion::CHASE_MOTION_TYPE
                || self.chase_target_of(guid) != Some(victim)
            {
                self.creature_move_chase(guid, victim, 0.0, 0.0);
            }
            self.pet_autocast(guid, victim);
            return;
        }
        // No victim: stop attacking, then follow/stay.
        if current_victim.is_some() {
            self.attack_stop(guid);
        }
        if let Some(st) = self.pets.get_mut(&guid)
            && st.commanded_victim.is_some()
        {
            st.commanded_victim = None;
        }
        match state.command_state {
            COMMAND_STAY => {
                if self.motion_type_of(guid) != crate::motion::IDLE_MOTION_TYPE {
                    self.creature_move_idle(guid);
                }
            }
            _ => {
                if self.motion_type_of(guid) != crate::motion::CHASE_MOTION_TYPE
                    || self.chase_target_of(guid) != Some(owner)
                {
                    self.creature_move_chase(guid, owner, PET_FOLLOW_DIST, PET_FOLLOW_ANGLE);
                }
            }
        }
    }

    /// Autocast pass (`PetAI` spell loop subset): the
    /// first enabled autocast spell that starts.
    fn pet_autocast(&mut self, guid: ObjectGuid, victim: ObjectGuid) {
        let Some(state) = self.pets.get(&guid).cloned() else {
            return;
        };
        for pet_spell in &state.spells {
            if pet_spell.active != ACT_ENABLED {
                continue;
            }
            let Some(spell) = self.stores.spells.get(pet_spell.spell_id).cloned() else {
                continue;
            };
            if spell.attributes & 0x0000_0040 != 0 {
                continue; // passive
            }
            let target_guid = if spell.is_positive_effect(0) {
                guid
            } else {
                victim
            };
            let outcome = self.cast_spell(
                guid,
                pet_spell.spell_id,
                CastTarget::Unit(target_guid),
                CastOptions {
                    triggered: false,
                    skip_known: true,
                    cast_item: None,
                },
            );
            if matches!(outcome, CastOutcome::Started) {
                return;
            }
        }
    }

    /// `HandlePetAbandon` pet arm.
    pub fn pet_abandon(&mut self, owner: ObjectGuid, pet: ObjectGuid) {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if state.owner != owner {
            return;
        }
        if state.is_hunter() {
            self.unsummon_pet(pet, PET_SAVE_AS_DELETED_MODE);
        } else {
            self.unsummon_pet(pet, PET_SAVE_NOT_IN_SLOT);
        }
    }

    /// `Unit::HandlePetCommand` for a controlled pet.
    pub fn pet_command(
        &mut self,
        owner: ObjectGuid,
        pet: ObjectGuid,
        command: u8,
        target: Option<ObjectGuid>,
    ) {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if state.owner != owner || !state.enabled {
            return;
        }
        match command {
            COMMAND_STAY => {
                self.creature_move_idle(pet);
                if let Some(st) = self.pets.get_mut(&pet) {
                    st.command_state = COMMAND_STAY;
                    st.commanded_victim = None;
                }
            }
            COMMAND_FOLLOW => {
                self.attack_stop(pet);
                self.cancel_cast(pet, false);
                if let Some(st) = self.pets.get_mut(&pet) {
                    st.command_state = COMMAND_FOLLOW;
                    st.commanded_victim = None;
                }
                self.creature_move_chase(pet, owner, PET_FOLLOW_DIST, PET_FOLLOW_ANGLE);
            }
            COMMAND_ATTACK => {
                let Some(target) = target.filter(|t| self.unit_is_alive(*t)) else {
                    self.send_pet_action_feedback(owner, 2); // NOTHING_TO_ATT
                    return;
                };
                if target == owner || target == pet {
                    self.send_pet_action_feedback(owner, 3); // CANT_ATT_TARGET
                    return;
                }
                if let Some(st) = self.pets.get_mut(&pet) {
                    st.commanded_victim = Some(target);
                }
                self.send_pet_ai_reaction(owner, pet);
            }
            COMMAND_DISMISS if !state.is_hunter() => {
                self.unsummon_pet(pet, PET_SAVE_NOT_IN_SLOT);
            }
            _ => {}
        }
    }

    /// `SMSG_PET_ACTION_FEEDBACK`.
    fn send_pet_action_feedback(&mut self, owner: ObjectGuid, feedback: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u8(feedback);
        let packet = WorldPacket::with_body(op::SMSG_PET_ACTION_FEEDBACK, payload.into_vec());
        self.send_to_player(owner, packet);
    }

    /// `SMSG_PET_ACTION_SOUND` growl (`SendPetAIReaction`:
    /// guid + `PET_ACTION_SOUND` 0). The fork writes
    /// the pet guid and action 0; pet talk
    /// (10% `PET_TALK_ATTACK`) needs the talk table,
    /// unmodeled.
    fn send_pet_ai_reaction(&mut self, owner: ObjectGuid, pet: ObjectGuid) {
        let mut payload = PacketWriter::new();
        payload.write_u64_le(pet.raw());
        payload.write_u32_le(0);
        let packet = WorldPacket::with_body(op::SMSG_PET_ACTION_SOUND, payload.into_vec());
        self.send_to_player(owner, packet);
    }

    /// React-state arm of `CMSG_PET_ACTION`.
    pub fn pet_set_react(&mut self, owner: ObjectGuid, pet: ObjectGuid, react: u8) {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if state.owner != owner
            || !matches!(react, REACT_PASSIVE | REACT_DEFENSIVE | REACT_AGGRESSIVE)
        {
            return;
        }
        if react == REACT_PASSIVE {
            self.cancel_cast(pet, false);
            self.attack_stop(pet);
        }
        if let Some(st) = self.pets.get_mut(&pet) {
            st.react_state = react;
            if react == REACT_PASSIVE {
                st.commanded_victim = None;
            }
        }
        if let Some(c) = self.creatures.get_mut(&pet) {
            c.react_state = react;
        }
    }

    /// `CMSG_PET_STOP_ATTACK`.
    pub fn pet_stop_attack(&mut self, owner: ObjectGuid, pet: ObjectGuid) {
        if self.pets.get(&pet).is_some_and(|st| st.owner == owner) && self.unit_is_alive(pet) {
            if let Some(st) = self.pets.get_mut(&pet) {
                st.commanded_victim = None;
            }
            self.attack_stop(pet);
        }
    }

    /// Spell arms of `CMSG_PET_ACTION` /
    /// `CMSG_PET_CAST_SPELL` (explicit target already
    /// resolved by the caller to a unit when the fork
    /// requires one).
    pub fn pet_cast_spell(
        &mut self,
        owner: ObjectGuid,
        pet: ObjectGuid,
        spell_id: u32,
        target: Option<ObjectGuid>,
    ) {
        use mangos_spells::defines::cast_result;
        let Some(state) = self.pets.get(&pet).cloned() else {
            return;
        };
        if state.owner != owner || !state.enabled || !self.unit_is_alive(pet) {
            return;
        }
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        if self.has_cooldown(pet, &spell, None) {
            self.send_pet_cast_fail(owner, spell_id, cast_result::FAILED_NOT_READY);
            return;
        }
        let known = state.spells.iter().any(|s| s.spell_id == spell_id);
        if !known || spell.attributes & 0x0000_0040 != 0 {
            self.send_pet_cast_fail(owner, spell_id, cast_result::FAILED_NOT_KNOWN);
            return;
        }
        let target_guid = target
            .filter(|t| self.unit_position(*t).is_some())
            .or_else(|| self.unit_base(pet).and_then(|u| u.melee_victim))
            .unwrap_or(pet);
        let outcome = self.cast_spell(
            pet,
            spell_id,
            CastTarget::Unit(target_guid),
            CastOptions {
                triggered: false,
                skip_known: true,
                cast_item: None,
            },
        );
        match outcome {
            CastOutcome::Started => {
                self.pet_check_learning(owner, pet, spell_id);
                self.send_pet_ai_reaction(owner, pet);
            }
            CastOutcome::Failed(code) => self.send_pet_cast_fail(owner, spell_id, code),
            CastOutcome::UnknownSpell => {}
        }
    }

    /// `Unit::SendPetCastFail`.
    fn send_pet_cast_fail(&mut self, owner: ObjectGuid, spell_id: u32, result: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(spell_id);
        payload.write_u8(result);
        payload.write_u8(0); // no failure-reason extras in this subset
        let packet = WorldPacket::with_body(op::SMSG_PET_CAST_FAILED, payload.into_vec());
        self.send_to_player(owner, packet);
    }

    /// `Pet::CheckLearning`: 10% teach-roll for
    /// hunter pets.
    fn pet_check_learning(&mut self, owner: ObjectGuid, pet: ObjectGuid, spell_id: u32) {
        let teacher = {
            let Some(state) = self.pets.get(&pet) else {
                return;
            };
            if !state.is_hunter() {
                return;
            }
            state
                .teach_spells
                .iter()
                .find(|(ability, _)| *ability == spell_id)
                .map(|(_, t)| *t)
        };
        let Some(teacher) = teacher else {
            return;
        };
        if self.rng.urand(0, 100) < 10 {
            if let Some(entry) = self.players.get_mut(&owner) {
                entry.player.learn_spell(teacher);
            }
            if let Some(st) = self.pets.get_mut(&pet) {
                st.teach_spells.retain(|(a, _)| *a != spell_id);
            }
        }
    }

    /// `HandlePetSetAction` application (parse and
    /// swap validation in the world actor handler
    /// shape; the fork does both in the handler —
    /// this keeps the state writes map-side).
    pub fn pet_set_action(&mut self, owner: ObjectGuid, pet: ObjectGuid, updates: &[(u32, u32)]) {
        let known: Vec<u32> = match self.pets.get(&pet) {
            Some(st) if st.owner == owner && st.enabled => {
                st.spells.iter().map(|s| s.spell_id).collect()
            }
            _ => return,
        };
        // Command/reaction buttons can't be removed
        // singly; swaps arrive as two updates.
        let move_command = updates.iter().any(|(_, data)| {
            let kind = data >> 24;
            kind == ACT_COMMAND || kind == ACT_REACTION
        });
        if move_command && updates.len() == 1 {
            return;
        }
        if updates.iter().any(|(pos, _)| *pos >= 10) {
            return;
        }
        if move_command && updates.len() == 2 {
            // Swap must be consistent with current bar.
            let bar = self.pets[&pet].action_bar;
            for (i, (pos, data)) in updates.iter().enumerate() {
                let kind = data >> 24;
                if kind == ACT_COMMAND || kind == ACT_REACTION {
                    let other = updates[1 - i];
                    let _ = pos;
                    let at_other = bar[other.0 as usize];
                    if at_other != (kind, data & 0x00FF_FFFF) {
                        return;
                    }
                }
            }
        }
        for (pos, data) in updates {
            let kind = data >> 24;
            let action = data & 0x00FF_FFFF;
            if matches!(kind, ACT_ENABLED | ACT_DISABLED | ACT_PASSIVE)
                && action != 0
                && !known.contains(&action)
            {
                continue;
            }
            if let Some(st) = self.pets.get_mut(&pet) {
                if action != 0 && kind == ACT_ENABLED {
                    Self::toggle_autocast(st, action, true);
                } else if action != 0 && kind == ACT_DISABLED {
                    Self::toggle_autocast(st, action, false);
                }
                st.action_bar[*pos as usize] = (kind, action);
            }
        }
    }

    /// `Pet::ToggleAutocast`.
    fn toggle_autocast(state: &mut PetState, spell_id: u32, on: bool) {
        for s in &mut state.spells {
            if s.spell_id == spell_id {
                s.active = if on { ACT_ENABLED } else { ACT_DISABLED };
            }
        }
        for slot in &mut state.action_bar {
            if slot.1 == spell_id && matches!(slot.0, ACT_ENABLED | ACT_DISABLED) {
                slot.0 = if on { ACT_ENABLED } else { ACT_DISABLED };
            }
        }
    }

    /// `CMSG_PET_SPELL_AUTOCAST`.
    pub fn pet_set_autocast(
        &mut self,
        owner: ObjectGuid,
        pet: ObjectGuid,
        spell_id: u32,
        on: bool,
    ) {
        if let Some(st) = self.pets.get_mut(&pet)
            && st.owner == owner
        {
            Self::toggle_autocast(st, spell_id, on);
        }
    }

    /// `SendPetNameQuery` data: (name, name
    /// timestamp, pet number) for a live pet.
    #[must_use]
    pub fn pet_name_query(&self, guid: ObjectGuid) -> Option<(String, u32, u32)> {
        let state = self.pets.get(&guid)?;
        let creature = self.creatures.get(&guid)?;
        let timestamp = creature
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_PET_NAME_TIMESTAMP);
        Some((
            creature.unit.world.name.clone(),
            timestamp,
            state.pet_number,
        ))
    }

    /// Rename application half of `HandlePetRename`
    /// (name validation is the caller's; the DB
    /// UPDATE rides a `MapEvent` like the fork's
    /// immediate statement).
    pub fn pet_rename(&mut self, owner: ObjectGuid, pet: ObjectGuid, name: String) -> bool {
        let Some(state) = self.pets.get(&pet).cloned() else {
            return false;
        };
        if state.owner != owner || !state.is_hunter() {
            return false;
        }
        let can_rename = self.creatures.get(&pet).is_some_and(|c| {
            c.unit
                .world
                .object
                .values()
                .get_u32(offsets::UNIT_FIELD_FLAGS)
                & UNIT_FLAG_PET_RENAME
                != 0
        });
        if !can_rename {
            return false;
        }
        if let Some(c) = self.creatures.get_mut(&pet) {
            c.unit.world.name.clone_from(&name);
            let v = c.unit.world.object.values_mut();
            v.remove_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PET_RENAME);
            #[allow(clippy::cast_possible_truncation)]
            v.set_u32(offsets::UNIT_FIELD_PET_NAME_TIMESTAMP, self.now_unix as u32);
        }
        if let Some(entry) = self.players.get_mut(&owner)
            && let Some(row) = entry
                .player
                .deferred
                .pets
                .iter_mut()
                .find(|r| r.id == state.pet_number)
        {
            row.name.clone_from(&name);
            row.renamed = 1;
        }
        self.events.push(MapEvent::RenamePet {
            owner_low: owner.counter(),
            pet_id: state.pet_number,
            name,
        });
        true
    }
}
