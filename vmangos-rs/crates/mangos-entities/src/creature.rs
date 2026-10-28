//! The Creature aggregate and the Slice 3 spawn subset of
//! `Creature::LoadFromDB` → `Create` → `UpdateEntry` →
//! `LoadCreatureAddon` (`Creature.cpp:295-593,1225-1265,
//! 1658-1801,2723-2762`).
//!
//! Included: entry/display choice (addon override,
//! probability roll, equal-chance roll), random-gender
//! display swap, scale/native display, level roll,
//! `InitStatsForLevel` health/mana/damage/armor/stats from
//! CLS, resistances, attack times, faction/NPC flags,
//! sheath/stand/emote from the addon, mount display,
//! equipment-template roll into virtual item fields, walk
//! vs always-run, react state default, respawn bookkeeping
//! inputs, and the naked `UpdateAllStats` projections
//! (stats + resistances copied from create values).
//!
//! Deferred (recorded): spawn auras and `UpdateAllStats`
//! combat fields (attack power, damage words — Slice 5),
//! AI initialization (Slice 6), creature groups/linking and
//! game-event overrides, loot-wield generation, flying
//! ground-relocation for dead spawns (needs terrain height;
//! the map slice supplies it), and equipment stat effects.

use mangos_guid::{HighGuid, TypeId};
use mangos_movement::movement_info::{MovementFlags, Position};
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{UNIT_END, offsets};
use mangos_protocol::update_fields::TypeMask;
use mangos_world_data::creature_template::CreatureInfo;
use mangos_world_data::spawn_support::{
    ClassLevelStats, CreatureDataAddon, CreatureDisplayInfoAddon, EquipmentTemplates,
};
use mangos_world_data::spawns::{
    CreatureSpawn, SPAWN_FLAG_ACTIVE, SPAWN_FLAG_DEAD, SPAWN_FLAG_DISABLED,
};

use mangos_world_data::pet_data::PetLevelInfo;

use crate::object::ObjectBase;
use crate::unit::{
    DeathState, MAX_SPELL_SCHOOL, MAX_STATS, MOVE_RUN, MOVE_WALK, ModelData, POWER_ENERGY,
    POWER_MANA, POWER_RAGE, STAT_INTELLECT, STAT_STAMINA, UnitBase, WorldObjectBase,
    check_valid_scale,
};

/// `urand(min, max)` inclusive, as the C++ global. The map
/// layer owns the RNG; goldens plug a deterministic one.
pub trait SpawnRng {
    fn urand(&mut self, min: u32, max: u32) -> u32;
}

/// Per-rank rate config (`Rate.Creature.*`, corpse decays).
/// Indexed by `CreatureElite` rank; the C++ default arm uses
/// the elite rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CreatureRates {
    /// HP rates [normal, elite, rareelite, worldboss, rare].
    pub hp: [f32; 5],
    /// Damage rates, same order.
    pub damage: [f32; 5],
    /// Spell-damage rates (`Rate.Creature.*.SpellDamage`,
    /// `Creature::_GetSpellDamageMod`), same order.
    pub spell_damage: [f32; 5],
    /// Corpse decay seconds [normal, elite, rareelite,
    /// worldboss, rare].
    pub corpse_decay: [u32; 5],
}

impl Default for CreatureRates {
    /// `mangosd.conf.example` defaults: all rates 1.0;
    /// decays 60/300/300/3600/300.
    fn default() -> Self {
        Self {
            hp: [1.0; 5],
            damage: [1.0; 5],
            spell_damage: [1.0; 5],
            corpse_decay: [60, 300, 300, 3600, 300],
        }
    }
}

impl CreatureRates {
    /// `Creature::_GetSpellDamageMod(rank)`.
    #[must_use]
    pub fn spell_damage_by_rank(&self, rank: u32) -> f32 {
        Self::by_rank(self.spell_damage, rank)
    }

    fn by_rank(rates: [f32; 5], rank: u32) -> f32 {
        // CREATURE_ELITE_NORMAL 0, ELITE 1, RAREELITE 2,
        // WORLDBOSS 3, RARE 4; default arm = elite.
        rates[match rank {
            0 => 0,
            2 => 2,
            3 => 3,
            4 => 4,
            _ => 1,
        }]
    }

    fn decay_by_rank(&self, rank: u32) -> u32 {
        self.corpse_decay[match rank {
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            _ => 0,
        }]
    }
}

/// `UNIT_DISPLAY_ID_BOX` placeholder (`UnitDefines.h:862`).
pub const UNIT_DISPLAY_ID_BOX: u32 = 4;
const UNIT_BYTE2_FLAG_AURAS: u8 = 0x10;
const UNIT_BYTE2_FLAG_PVP: u8 = 0x01;
const UNIT_FLAG_PLUS_MOB: u32 = 0x0000_0040;
/// `UNIT_FLAG_PET_RENAME`.
pub const UNIT_FLAG_PET_RENAME: u32 = 0x0000_0010;
/// `UNIT_FLAG_PET_ABANDON`.
pub const UNIT_FLAG_PET_ABANDON: u32 = 0x0000_0020;
const UNIT_BYTE2_FLAG_UNK3: u8 = 0x08;
const UNIT_BYTE2_FLAG_UNK5: u8 = 0x20;

/// `enum PetType` subset carried on the aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PetKind {
    Summon,
    Hunter,
    Guardian,
}
const UNIT_DYNFLAG_DEAD: u32 = 0x0000_0020;
/// `CREATURE_STATIC_FLAG_PVP_ENABLING` (`static_flags1`).
const STATIC_FLAG_PVP_ENABLING: u32 = 0x0040_0000;
/// `CREATURE_STATIC_FLAG_FLOATING` (`static_flags1`).
const STATIC_FLAG_FLOATING: u32 = 0x2000_0000;
/// `CREATURE_FLAG_EXTRA_ALWAYS_RUN` (`flags_extra`).
const FLAG_EXTRA_ALWAYS_RUN: u32 = 0x0000_0040;
/// `CREATURE_FLAG_EXTRA_APPEAR_DEAD`.
const FLAG_EXTRA_APPEAR_DEAD: u32 = 0x0000_4000;
/// `REACT_DEFENSIVE` (`enum ReactStates`).
pub const REACT_DEFENSIVE: u8 = 1;

/// Why a spawn was refused (`LoadFromDB` false returns).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnError {
    /// `SPAWN_FLAG_DISABLED` and not forced.
    Disabled,
    /// Chosen entry missing from `creature_template`.
    UnknownEntry,
    /// No display could be chosen.
    NoDisplay,
    /// No `creature_display_info_addon` row for the display.
    NoDisplayAddon,
}

/// Resolves DBC-derived display data (keeps this crate
/// DBC-free, like the player inputs).
pub struct DisplayResolver<'a> {
    /// `GetScaleForDisplayId(display_id)`.
    pub scale_for_display: &'a dyn Fn(u32) -> f32,
    /// `UpdateModelData` inputs for a display id.
    pub model_for_display: &'a dyn Fn(u32) -> Option<ModelData>,
}

/// Item prototype fields `SetVirtualItem` copies
/// (`Creature.cpp:4007-4031`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualItemPrototype {
    pub display_id: u32,
    pub class: u8,
    pub subclass: u8,
    pub material: u8,
    pub inventory_type: u8,
    pub sheath: u8,
}

/// The Creature aggregate.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // mirrors the C++ member set
pub struct Creature {
    pub unit: UnitBase,
    /// `m_originalEntry`.
    pub original_entry: u32,
    /// Chosen template (owned clone of the store row).
    pub info: CreatureInfo,
    /// Spawn row, when DB-spawned (`m_creatureData`).
    pub data: Option<CreatureSpawn>,
    pub home_position: Position,
    pub wander_distance: f32,
    /// `m_respawnDelay` (seconds; rolled per spawn).
    pub respawn_delay: u32,
    /// `m_respawnTime` (unix seconds; 0 = ready).
    pub respawn_time: u64,
    /// Set when an expired persisted respawn row was
    /// consumed: the map layer must persist 0
    /// (`SaveCreatureRespawnTime(guid, 0)`).
    pub respawn_row_expired: bool,
    /// `m_corpseDelay`.
    pub corpse_delay: u32,
    /// `m_isActiveObject` (`SPAWN_FLAG_ACTIVE`).
    pub is_active_object: bool,
    /// `m_visibilityModifier`.
    pub visibility_modifier: f32,
    /// `SelectLevel` rolled maxima before aura stat
    /// deltas (the fork's `UNIT_MOD_HEALTH` /
    /// `UNIT_MOD_POWER_MANA` `BASE_VALUE` registers).
    pub spawn_max_health: u32,
    pub spawn_max_mana: u32,
    /// `m_defaultMovementType` (spawn row wins over
    /// template).
    pub default_movement_type: u32,
    /// `m_reactState` initial (defensive for non-pets).
    pub react_state: u8,
    /// `UNIT_STATE_RUNNING` default (`flags_extra` always-run).
    pub default_running: bool,
    /// `m_mountId`.
    pub mount_id: u32,
    /// `m_spells[CREATURE_MAX_SPELLS]` (template copy).
    pub spells: [u32; 4],
    /// `m_defaultGossipMenuId`.
    pub gossip_menu_id: u32,
    /// Base weapon damage ranges [(min,max) ×3: base, off,
    /// ranged] (`SetBaseWeaponDamage` results).
    pub base_weapon_damage: [(f32, f32); 3],
    /// Create resistances per school (armor from CLS,
    /// the rest from the template).
    pub create_resistances: [f32; MAX_SPELL_SCHOOL],
    /// Deferred spawn auras (template or addon list, addon
    /// non-empty wins in C++ truthiness terms; Slice 5
    /// applies them).
    pub pending_auras: Vec<u32>,
    /// `m_equipmentId`.
    pub equipment_id: u32,
    /// `m_isDeadByDefault` (spawned dead by data).
    pub is_dead_by_default: bool,
    /// `m_corpseDecayTimer` (ms remaining as a corpse).
    pub corpse_decay_timer: i64,
}

impl Creature {
    /// `LoadFromDB(guidlow, map, force)` + `Create` subset.
    /// `respawn_time` is the persisted value for this spawn
    /// (0 when none); `now_unix` is `time(nullptr)`.
    ///
    /// # Errors
    ///
    /// [`SpawnError`] for the C++ false returns.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_from_db(
        guid_low: u32,
        data: &CreatureSpawn,
        templates: &mangos_world_data::creature_template::CreatureTemplates,
        addon: Option<&CreatureDataAddon>,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        cls: &ClassLevelStats,
        display_addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
        display: &DisplayResolver<'_>,
        rates: &CreatureRates,
        respawn_time: u64,
        now_unix: u64,
        force: bool,
        rng: &mut dyn SpawnRng,
    ) -> Result<Creature, SpawnError> {
        if !force && data.spawn_flags & SPAWN_FLAG_DISABLED != 0 {
            return Err(SpawnError::Disabled);
        }
        let entry = data.choose_creature_id(|min, max| rng.urand(min, max));
        let info = templates
            .get(entry)
            .cloned()
            .ok_or(SpawnError::UnknownEntry)?;
        let mut creature = Creature::create(
            guid_low,
            data.creature_id[0],
            info,
            Some(data.clone()),
            addon,
            equipment,
            virtual_items,
            cls,
            display_addons,
            display,
            rates,
            rng,
        )?;
        creature.home_position = Position::new(
            data.position.x,
            data.position.y,
            data.position.z,
            data.position.o,
        );
        creature.unit.world.map_id = u32::from(data.position.map_id);
        creature.unit.world.instance_id = data.instanciated_continent_instance_id;
        creature.unit.world.relocate(creature.home_position);
        creature.wander_distance = data.wander_distance;
        creature.respawn_delay = data.random_respawn_time(|min, max| rng.urand(min, max));
        // Health/mana percentages from the spawn row were
        // applied in SelectLevel via UpdateEntry's
        // preserveHPAndPower=false at create (100%), then
        // LoadFromDB cannot re-scale; the C++ create path
        // passes 100. Death state from flags/percent:
        #[allow(clippy::float_cmp)] // C++ `!data->health_percent` is an exact zero test
        if data.spawn_flags & SPAWN_FLAG_DEAD != 0 || data.health_percent == 0.0 {
            creature.unit.death_state = DeathState::Dead;
            creature.is_dead_by_default = data.spawn_flags & SPAWN_FLAG_DEAD != 0;
        }
        if data.spawn_flags & SPAWN_FLAG_ACTIVE != 0 {
            creature.is_active_object = true;
        }
        if data.visibility_mod != 0.0 {
            creature.visibility_modifier = data.visibility_mod;
        }
        creature.respawn_time = respawn_time;
        if respawn_time > now_unix {
            creature.unit.death_state = DeathState::Dead;
            // Flying ground-relocation needs terrain height
            // (map slice); recorded deviation.
        } else if respawn_time != 0 {
            creature.respawn_time = 0;
            creature.respawn_row_expired = true;
        }
        Ok(creature)
    }

    /// `Pet::Create(guid, pos, cinfo, petNumber)`:
    /// creature create with the pet number in the GUID
    /// entry slot (this fork's `GetHighGuid` is always
    /// UNIT pre-3.x), followed by
    /// `Pet::InitStatsForLevel`. Position/map are the
    /// caller's (`CreatureCreatePos` final point).
    ///
    /// # Errors
    ///
    /// [`SpawnError`] for the C++ false returns.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_pet(
        guid_low: u32,
        pet_number: u32,
        info: CreatureInfo,
        kind: PetKind,
        level: u32,
        level_info: Option<&PetLevelInfo>,
        owner_is_player: bool,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        cls: &ClassLevelStats,
        display_addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
        display: &DisplayResolver<'_>,
        rates: &CreatureRates,
        rng: &mut dyn SpawnRng,
    ) -> Result<Creature, SpawnError> {
        let mut creature = Creature::create(
            guid_low,
            pet_number,
            info,
            None,
            None,
            equipment,
            virtual_items,
            cls,
            display_addons,
            display,
            rates,
            rng,
        )?;
        creature.is_active_object = true; // `m_isActiveObject` in Pet ctor path
        creature.init_pet_stats_for_level(level, kind, level_info, owner_is_player, cls, rates);
        Ok(creature)
    }

    /// `Pet::InitStatsForLevel(petlevel, owner)`
    /// (player-owner rate arms; creature-owner uses
    /// the per-rank config rates like the fork's
    /// `_GetHealthMod/_GetDamageMod`). Also the
    /// `GivePetLevel` re-init entry: current
    /// health/power percents survive via the max
    /// writes the fork's `UpdateAllStats` performs
    /// (values clamp to the new maxima).
    #[allow(clippy::cast_precision_loss)]
    pub fn init_pet_stats_for_level(
        &mut self,
        level: u32,
        kind: PetKind,
        level_info: Option<&PetLevelInfo>,
        owner_is_player: bool,
        cls: &ClassLevelStats,
        rates: &CreatureRates,
    ) {
        {
            let v = self.unit.world.object.values_mut();
            match kind {
                PetKind::Summon => {
                    v.set_u32(offsets::UNIT_FIELD_FLAGS, 0);
                }
                PetKind::Hunter => {
                    v.set_byte(offsets::UNIT_FIELD_BYTES_0, 1, 1); // CLASS_WARRIOR
                    v.set_byte(offsets::UNIT_FIELD_BYTES_0, 2, 0); // GENDER_NONE
                    v.set_byte(offsets::UNIT_FIELD_BYTES_2, 0, 1); // sheath melee
                    v.set_byte(
                        offsets::UNIT_FIELD_BYTES_2,
                        1,
                        UNIT_BYTE2_FLAG_UNK3 | UNIT_BYTE2_FLAG_AURAS | UNIT_BYTE2_FLAG_UNK5,
                    );
                    v.set_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PET_ABANDON);
                }
                PetKind::Guardian => {}
            }
            v.set_u32(offsets::UNIT_FIELD_LEVEL, level);
            // Attack times from the template (fork
            // re-asserts them here).
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME,
                self.info.base_attack_time as f32,
            );
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME + 1,
                self.info.base_attack_time as f32,
            );
            v.set_f32(
                offsets::UNIT_FIELD_RANGEDATTACKTIME,
                self.info.ranged_attack_time as f32,
            );
            // 1.12 client build: cast speed is the int 0.
            v.set_i32(offsets::UNIT_MOD_CAST_SPEED, 0);
        }
        let stats = cls.get(self.info.unit_class, level);
        let rank = self.info.rank;
        let health_mod = if owner_is_player {
            1.0
        } else {
            CreatureRates::by_rank(rates.hp, rank)
        };
        let damage_mod = if owner_is_player {
            1.0
        } else {
            CreatureRates::by_rank(rates.damage, rank)
        };
        // Power type via the creature arm (fork
        // `SetInitCreaturePowerType`); hunter pets are
        // forced to focus by the map layer after stats
        // (fork does it in CreateBaseAtCreature/load).
        if stats.mana > 0 {
            self.unit.world.object.values_mut().set_byte(
                offsets::UNIT_FIELD_BYTES_0,
                3,
                POWER_MANA as u8,
            );
        }
        let create =
            self.pet_create_values(kind, level, level_info, &stats, health_mod, damage_mod);
        self.set_pet_base_damage(create.0, create.1);
        self.create_resistances[0] = create.2;
        self.set_pet_create_health_mana(create.3, create.4);
        self.set_pet_create_stats(create.5);
        // Naked projections (stats/resistances/AP fields)
        // at the new create values, as `UpdateAllStats`
        // does after stat init.
        self.unit.base_attack_power = stats.attack_power as f32;
        {
            let v = self.unit.world.object.values_mut();
            v.set_i32(offsets::UNIT_FIELD_ATTACK_POWER, stats.attack_power);
            v.set_i32(
                offsets::UNIT_FIELD_RANGED_ATTACK_POWER,
                stats.ranged_attack_power,
            );
        }
        self.refresh_derived_stats();
        for school in 0..MAX_SPELL_SCHOOL {
            #[allow(clippy::cast_possible_truncation)]
            let value = self.create_resistances[school] as i32;
            if school == 0 {
                self.unit.set_armor(value);
            } else {
                self.unit.set_resistance(school, value);
            }
        }
    }

    /// The per-kind create-value computation of
    /// `Pet::InitStatsForLevel` (split out for the
    /// clippy line budget): (min dmg, max dmg, armor,
    /// health, mana, stats).
    #[allow(clippy::cast_precision_loss, clippy::too_many_arguments)]
    fn pet_create_values(
        &self,
        kind: PetKind,
        level: u32,
        level_info: Option<&PetLevelInfo>,
        stats: &mangos_world_data::spawn_support::CreatureClassLevelStats,
        health_mod: f32,
        damage_mod: f32,
    ) -> (f32, f32, f32, u32, Option<u32>, [f32; MAX_STATS]) {
        let cls_stats = [
            stats.strength as f32,
            stats.agility as f32,
            stats.stamina as f32,
            stats.intellect as f32,
            stats.spirit as f32,
        ];
        let variance_damage = || {
            let avg = stats.melee_damage * self.info.damage_multiplier * damage_mod;
            let var = avg * self.info.damage_variance;
            (avg - var, avg + var)
        };
        match kind {
            PetKind::Summon => {
                let (dmg_min, dmg_max) = if let Some(info) =
                    level_info.filter(|i| i.dmg_min != 0.0 && i.dmg_max != 0.0)
                {
                    (damage_mod * info.dmg_min, damage_mod * info.dmg_max)
                } else {
                    variance_damage()
                };
                let armor = if let Some(info) = level_info.filter(|i| i.armor != 0) {
                    f32::from(info.armor)
                } else {
                    stats.armor as f32 * self.info.armor_multiplier
                };
                let (health, mana, pet_stats) = if let Some(info) = level_info {
                    (
                        round_u32(f32::from(info.health) * health_mod),
                        Some(u32::from(info.mana)),
                        info.stats.map(f32::from),
                    )
                } else {
                    (
                        round_u32(stats.health as f32 * self.info.health_multiplier * health_mod),
                        Some(round_u32(stats.mana as f32 * self.info.mana_multiplier)),
                        cls_stats,
                    )
                };
                (dmg_min, dmg_max, armor, health, mana, pet_stats)
            }
            PetKind::Hunter => {
                let attack_time = self.info.base_attack_time as f32;
                let dmg_min = damage_mod * (level as f32 * 1.15 * 1.05) * attack_time / 2000.0;
                let dmg_max = damage_mod * (level as f32 * 1.45 * 1.05) * attack_time / 2000.0;
                let (armor, health, pet_stats) = if let Some(info) = level_info {
                    (
                        f32::from(info.armor),
                        round_u32(f32::from(info.health) * health_mod),
                        info.stats.map(f32::from),
                    )
                } else {
                    (
                        stats.armor as f32,
                        round_u32(stats.health as f32 * health_mod),
                        cls_stats,
                    )
                };
                (dmg_min, dmg_max, armor, health, None, pet_stats)
            }
            PetKind::Guardian => {
                let (dmg_min, dmg_max) = variance_damage();
                (
                    dmg_min,
                    dmg_max,
                    stats.armor as f32 * self.info.armor_multiplier,
                    round_u32(stats.health as f32 * self.info.health_multiplier * health_mod),
                    Some(round_u32(stats.mana as f32 * self.info.mana_multiplier)),
                    cls_stats,
                )
            }
        }
    }

    fn set_pet_base_damage(&mut self, min: f32, max: f32) {
        self.base_weapon_damage[0] = (min, max);
        self.base_weapon_damage[1] = (min / 2.0, f32::midpoint(min, max));
        let v = self.unit.world.object.values_mut();
        v.set_f32(offsets::UNIT_FIELD_MINDAMAGE, min);
        v.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, max);
        v.set_f32(offsets::UNIT_FIELD_MINOFFHANDDAMAGE, min / 2.0);
        v.set_f32(
            offsets::UNIT_FIELD_MAXOFFHANDDAMAGE,
            f32::midpoint(min, max),
        );
    }

    fn set_pet_create_health_mana(&mut self, health: u32, mana: Option<u32>) {
        let health = health.max(1);
        self.unit.create_health = health;
        self.spawn_max_health = health;
        self.unit.set_max_health(health);
        if self.unit.health() > health {
            self.unit.set_health(health);
        }
        if let Some(mana) = mana {
            self.unit.create_mana = mana;
            self.spawn_max_mana = mana;
            self.unit.set_max_power(POWER_MANA, mana);
            if self.unit.power(POWER_MANA) > mana {
                self.unit.set_power(POWER_MANA, mana);
            }
        }
    }

    fn set_pet_create_stats(&mut self, stats: [f32; MAX_STATS]) {
        self.unit.create_stats = stats;
    }

    /// `Create` + `CreateFromProto` + `UpdateEntry(entry,
    /// preserve=false)` + `LoadCreatureAddon` subset.
    #[allow(clippy::too_many_arguments)]
    fn create(
        guid_low: u32,
        first_creature_id: u32,
        info: CreatureInfo,
        data: Option<CreatureSpawn>,
        addon: Option<&CreatureDataAddon>,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        cls: &ClassLevelStats,
        display_addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
        display: &DisplayResolver<'_>,
        rates: &CreatureRates,
        rng: &mut dyn SpawnRng,
    ) -> Result<Creature, SpawnError> {
        let mut object = ObjectBase::new(
            TypeId::Unit,
            TypeMask::OBJECT.union(TypeMask::UNIT),
            ObjectUpdateFlags(
                ObjectUpdateFlags::ALL
                    | ObjectUpdateFlags::LIVING
                    | ObjectUpdateFlags::HAS_POSITION,
            ),
            UNIT_END,
        );
        // GetHighGuid is always HIGHGUID_UNIT pre-3.x.
        object.create(guid_low, first_creature_id, HighGuid::UNIT);
        let mut world = WorldObjectBase::new(object);
        world.name.clone_from(&info.name);
        let mut creature = Creature {
            unit: UnitBase::new(world),
            original_entry: info.entry,
            info,
            data,
            home_position: Position::default(),
            wander_distance: 0.0,
            respawn_delay: 0,
            respawn_time: 0,
            respawn_row_expired: false,
            corpse_delay: 0,
            is_active_object: false,
            visibility_modifier: 0.0,
            spawn_max_health: 0,
            spawn_max_mana: 0,
            default_movement_type: 0,
            react_state: REACT_DEFENSIVE,
            default_running: false,
            mount_id: 0,
            spells: [0; 4],
            gossip_menu_id: 0,
            base_weapon_damage: [(0.0, 0.0); 3],
            create_resistances: [0.0; MAX_SPELL_SCHOOL],
            pending_auras: Vec::new(),
            equipment_id: 0,
            is_dead_by_default: false,
            corpse_decay_timer: 0,
        };
        creature.init_entry(
            addon,
            equipment,
            virtual_items,
            display_addons,
            display,
            rng,
        )?;
        let level = if creature.info.level_min == creature.info.level_max {
            creature.info.level_min
        } else {
            rng.urand(creature.info.level_min, creature.info.level_max)
        };
        creature.select_level(level, 100.0, 100.0, cls, rates);
        creature.apply_entry_tail();
        creature.corpse_delay = rates.decay_by_rank(creature.info.rank);
        creature.load_addon(addon);
        creature.default_running = creature.info.flags_extra & FLAG_EXTRA_ALWAYS_RUN != 0;
        Ok(creature)
    }

    /// `InitEntry` subset (event data excluded).
    fn init_entry(
        &mut self,
        addon: Option<&CreatureDataAddon>,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        display_addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
        display: &DisplayResolver<'_>,
        rng: &mut dyn SpawnRng,
    ) -> Result<(), SpawnError> {
        self.unit.world.object.set_entry(self.info.entry);
        {
            let v = self.unit.world.object.values_mut();
            v.set_byte(offsets::UNIT_FIELD_BYTES_0, 0, 0);
            v.set_byte(
                offsets::UNIT_FIELD_BYTES_0,
                1,
                u8::try_from(self.info.unit_class).unwrap_or(0),
            );
        }
        let (mut display_id, scale) = choose_display(&self.info, addon, display, rng);
        if display_id == 0 {
            return Err(SpawnError::NoDisplay);
        }
        // GetCreatureDisplayInfoRandomGender: 50% swap when
        // an other-gender display exists.
        let minfo = random_gender_addon(display_addons, display_id, rng)
            .ok_or(SpawnError::NoDisplayAddon)?;
        display_id = minfo.display_id;
        self.unit
            .world
            .object
            .set_object_scale(check_valid_scale(scale));
        self.unit.set_native_display_id(display_id);
        self.unit
            .set_display_id(display_id, (display.model_for_display)(display_id));
        self.unit.display_speed_walk = minfo.speed_walk;
        self.unit.display_speed_run = minfo.speed_run;
        {
            let v = self.unit.world.object.values_mut();
            v.set_byte(offsets::UNIT_FIELD_BYTES_0, 2, minfo.gender);
            v.set_f32(offsets::UNIT_MOD_CAST_SPEED, 1.0);
        }
        self.load_default_equipment(addon, equipment, virtual_items, rng);
        self.update_speed_rates();
        self.default_movement_type = self
            .data
            .as_ref()
            .map_or(self.info.movement_type, |d| u32::from(d.movement_type));
        Ok(())
    }

    /// `UpdateSpeed(MOVE_WALK/RUN)` inputs:
    /// `GetBaseWalkSpeedRate`/`GetBaseRunSpeedRate` at
    /// native display, unmounted.
    fn update_speed_rates(&mut self) {
        self.unit.speed_rate[MOVE_WALK] = if self.info.speed_walk == 0.0 {
            self.unit.display_speed_walk
        } else {
            self.info.speed_walk
        };
        self.unit.speed_rate[MOVE_RUN] = if self.info.speed_run == 0.0 {
            self.unit.display_speed_run
        } else {
            self.info.speed_run
        };
        self.unit.base_speed_rate = self.unit.speed_rate;
    }

    /// `SelectLevel` (level already rolled by the caller,
    /// in C++ call order) + `InitStatsForLevel`.
    fn select_level(
        &mut self,
        level: u32,
        percent_health: f32,
        percent_mana: f32,
        cls: &ClassLevelStats,
        rates: &CreatureRates,
    ) {
        self.unit
            .world
            .object
            .values_mut()
            .set_u32(offsets::UNIT_FIELD_LEVEL, level);
        self.init_stats_for_level(percent_health, percent_mana, cls, rates);
    }

    /// `Creature::InitStatsForLevel` (non-pet path).
    #[allow(clippy::float_cmp)] // C++ compares percents exactly
    fn init_stats_for_level(
        &mut self,
        percent_health: f32,
        percent_mana: f32,
        cls: &ClassLevelStats,
        rates: &CreatureRates,
    ) {
        let stats = cls.get(self.info.unit_class, self.unit.level());
        // SetInitCreaturePowerType (Creature.cpp:1631-1656).
        if stats.mana > 0 {
            self.unit.world.object.values_mut().set_byte(
                offsets::UNIT_FIELD_BYTES_0,
                3,
                POWER_MANA as u8,
            );
        } else if self.unit.class() == 4 {
            // CLASS_ROGUE -> energy.
            self.unit.world.object.values_mut().set_byte(
                offsets::UNIT_FIELD_BYTES_0,
                3,
                POWER_ENERGY as u8,
            );
            let create = self.unit.create_power(POWER_ENERGY);
            self.unit.set_max_power(POWER_ENERGY, create);
            self.unit.set_power(POWER_ENERGY, 0);
        } else {
            self.unit.world.object.values_mut().set_byte(
                offsets::UNIT_FIELD_BYTES_0,
                3,
                POWER_RAGE as u8,
            );
            let create = self.unit.create_power(POWER_RAGE);
            self.unit.set_max_power(POWER_RAGE, create);
            self.unit.set_power(POWER_RAGE, 0);
        }
        let rank = self.info.rank;
        let health_mod = CreatureRates::by_rank(rates.hp, rank);
        #[allow(clippy::cast_precision_loss)]
        let health =
            round_u32(health_mod * stats.health as f32 * self.info.health_multiplier).max(1);
        #[allow(clippy::cast_precision_loss)]
        let base_health = round_u32(health_mod * stats.base_health as f32).max(1);
        self.unit.create_health = base_health;
        self.spawn_max_health = health;
        self.unit.set_max_health(health);
        if percent_health == 100.0 {
            self.unit.set_health(health);
        } else {
            self.unit.set_health(set_percent(health, percent_health));
        }
        #[allow(clippy::cast_precision_loss)]
        let mana = round_u32(stats.mana as f32 * self.info.mana_multiplier);
        self.unit.create_mana = stats.base_mana as u32;
        self.spawn_max_mana = mana;
        self.unit.set_max_power(POWER_MANA, mana);
        if percent_mana == 100.0 {
            self.unit.set_power(POWER_MANA, mana);
        } else {
            self.unit
                .set_power(POWER_MANA, set_percent(mana, percent_mana));
        }
        // Damage (base weapon ranges; field words are
        // UpdateAllStats combat math, Slice 5).
        let damage_mod = CreatureRates::by_rank(rates.damage, rank);
        let melee_avg = stats.melee_damage * self.info.damage_multiplier * damage_mod;
        let melee_var = melee_avg * self.info.damage_variance;
        let ranged_avg = stats.ranged_damage * self.info.damage_multiplier * damage_mod;
        let ranged_var = ranged_avg * self.info.damage_variance;
        self.base_weapon_damage = [
            (melee_avg - melee_var, melee_avg + melee_var),
            (
                (melee_avg - melee_var) / 2.0,
                f32::midpoint(melee_avg, melee_var),
            ),
            (ranged_avg - ranged_var, ranged_avg + ranged_var),
        ];
        // Spawn-time `UpdateAttackPowerAndDamage` +
        // `UpdateDamagePhysical` (Slice 6): modifiers are
        // all identity at spawn, so the AP factor is 1.0
        // and the field words equal the base ranges.
        #[allow(clippy::cast_precision_loss)]
        {
            self.unit.base_attack_power = stats.attack_power as f32;
            let v = self.unit.world.object.values_mut();
            v.set_i32(offsets::UNIT_FIELD_ATTACK_POWER, stats.attack_power);
            v.set_i32(
                offsets::UNIT_FIELD_RANGED_ATTACK_POWER,
                stats.ranged_attack_power,
            );
            let [(min, max), (omin, omax), (rmin, rmax)] = self.base_weapon_damage;
            v.set_f32(offsets::UNIT_FIELD_MINDAMAGE, min);
            v.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, max);
            v.set_f32(offsets::UNIT_FIELD_MINOFFHANDDAMAGE, omin);
            v.set_f32(offsets::UNIT_FIELD_MAXOFFHANDDAMAGE, omax);
            v.set_f32(offsets::UNIT_FIELD_MINRANGEDDAMAGE, rmin);
            v.set_f32(offsets::UNIT_FIELD_MAXRANGEDDAMAGE, rmax);
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.create_resistances[0] = stats.armor as f32 * self.info.armor_multiplier;
            let create = [
                stats.strength,
                stats.agility,
                stats.stamina,
                stats.intellect,
                stats.spirit,
            ];
            for (slot, value) in self.unit.create_stats.iter_mut().zip(create) {
                *slot = value as f32;
            }
        }
    }

    /// The `HandleStatModifier` update dispatch for aura
    /// stat changes: `Creature::UpdateStats` fields plus
    /// `UpdateMaxHealth` / `UpdateMaxPower(MANA)`
    /// (`StatSystem.cpp:766/781`; spawn maxima carry
    /// the spawn-time `BASE_VALUE`, aura TOTAL groups
    /// apply on top).
    pub fn refresh_derived_stats(&mut self) {
        for i in 0..MAX_STATS {
            #[allow(clippy::cast_possible_truncation)]
            let total = self.unit.total_stat_value(i, 0.0) as i32;
            self.unit.set_stat(i, total);
        }
        let (health_total, health_pct) = self.unit.health_modifier_groups();
        #[allow(clippy::cast_precision_loss)]
        let health = (self.spawn_max_health as f32
            + health_total
            + (self.unit.stat(STAT_STAMINA) - self.unit.create_stats[STAT_STAMINA]) * 10.0)
            * health_pct;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_health = if health < 1.0 { 1 } else { health as u32 };
        self.unit.set_max_health(max_health);
        let (mana_total, mana_pct) = self.unit.power_modifier_groups(POWER_MANA);
        #[allow(clippy::cast_precision_loss)]
        let mana = (self.spawn_max_mana as f32
            + mana_total
            + (self.unit.stat(STAT_INTELLECT) - self.unit.create_stats[STAT_INTELLECT]) * 15.0)
            * mana_pct;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_mana = if mana < 0.0 { 0 } else { mana as u32 };
        self.unit.set_max_power(POWER_MANA, max_mana);
    }

    /// The `UpdateEntry` tail after `SelectLevel` (resists,
    /// attack times, naked stat projection, faction, NPC
    /// flags, static/extra flag projections, `PvP` byte,
    /// spells).
    #[allow(clippy::cast_precision_loss)] // C++ converts res ints to float identically
    fn apply_entry_tail(&mut self) {
        // SetCreateResistance for magic schools (order in
        // C++: holy, fire, nature, frost, shadow, arcane =
        // schools 1..6).
        self.create_resistances[1] = self.info.holy_res as f32;
        self.create_resistances[2] = self.info.fire_res as f32;
        self.create_resistances[3] = self.info.nature_res as f32;
        self.create_resistances[4] = self.info.frost_res as f32;
        self.create_resistances[5] = self.info.shadow_res as f32;
        self.create_resistances[6] = self.info.arcane_res as f32;
        {
            let v = self.unit.world.object.values_mut();
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME,
                self.info.base_attack_time as f32,
            );
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME + 1,
                self.info.base_attack_time as f32,
            );
            v.set_f32(
                offsets::UNIT_FIELD_RANGEDATTACKTIME,
                self.info.ranged_attack_time as f32,
            );
        }
        // Naked UpdateAllStats: stats/resistances copy from
        // create values (float->int truncation like C++).
        for i in 0..MAX_STATS {
            let value = self.unit.create_stats[i] as i32;
            self.unit.set_stat(i, value);
        }
        for school in 0..MAX_SPELL_SCHOOL {
            let value = self.create_resistances[school] as i32;
            if school == 0 {
                self.unit.set_armor(value);
            } else {
                self.unit.set_resistance(school, value);
            }
        }
        self.unit.set_faction_template(self.info.faction);
        self.gossip_menu_id = self.info.gossip_menu_id;
        let npc_flags = self.info.npc_flags;
        let plus_mob = self.info.rank > 0;
        let appear_dead = self.info.flags_extra & FLAG_EXTRA_APPEAR_DEAD != 0;
        let pvp = self.info.static_flags1 & STATIC_FLAG_PVP_ENABLING != 0;
        let floating = self.info.static_flags1 & STATIC_FLAG_FLOATING != 0;
        {
            let v = self.unit.world.object.values_mut();
            v.set_u32(offsets::UNIT_NPC_FLAGS, npc_flags);
            // SetSheath(MELEE) + misc AURAS byte (UpdateEntry
            // before SelectLevel in C++; same result here
            // since nothing between touches bytes 2).
            v.set_byte(offsets::UNIT_FIELD_BYTES_2, 0, 1);
            let mut misc = UNIT_BYTE2_FLAG_AURAS;
            if pvp {
                misc |= UNIT_BYTE2_FLAG_PVP;
            }
            v.set_byte(offsets::UNIT_FIELD_BYTES_2, 1, misc);
            if plus_mob {
                v.set_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PLUS_MOB);
            }
            if appear_dead {
                v.set_flag(offsets::UNIT_DYNAMIC_FLAGS, UNIT_DYNFLAG_DEAD);
            }
        }
        if floating {
            self.unit.world.movement_info.flags =
                MovementFlags(self.unit.world.movement_info.flags.0 | MovementFlags::FIXED_Z);
        }
        self.spells = self.info.spells;
    }

    /// `LoadCreatureAddon(false)` value half (aura apply
    /// deferred: ids land in `pending_auras`; the addon list
    /// wins when present-and-nonempty, C++ pointer
    /// truthiness on the parsed array).
    fn load_addon(&mut self, addon: Option<&CreatureDataAddon>) {
        let (stand, sheath, emote) = if let Some(a) = addon {
            self.mount_id = if a.mount_display_id == u32::MAX {
                self.info.mount_display_id
            } else {
                a.mount_display_id
            };
            if a.auras.is_empty() {
                self.pending_auras.clone_from(&self.info.auras);
            } else {
                self.pending_auras.clone_from(&a.auras);
            }
            (a.stand_state, a.sheath_state, a.emote_state)
        } else {
            self.mount_id = self.info.mount_display_id;
            self.pending_auras.clone_from(&self.info.auras);
            (0, 1, 0)
        };
        let mount = self.mount_id;
        let v = self.unit.world.object.values_mut();
        if mount != 0 {
            v.set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, mount);
        }
        v.set_byte(offsets::UNIT_FIELD_BYTES_1, 0, stand);
        v.set_byte(offsets::UNIT_FIELD_BYTES_2, 0, sheath);
        v.set_u32(offsets::UNIT_NPC_EMOTESTATE, emote);
    }

    /// `LoadDefaultEquipment` + `LoadEquipment(id, true)`:
    /// addon equipment (>= 0) wins over the template's;
    /// addon -1 = template, -2 = none (our addon stores the
    /// words signed). Roll is `ChooseEquipmentEntry`:
    /// `urand(0, total-1)`, first cumulative sum above the
    /// roll. An equipment id of 0 clears the slots (force).
    fn load_default_equipment(
        &mut self,
        addon: Option<&CreatureDataAddon>,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        rng: &mut dyn SpawnRng,
    ) {
        let equip_id = match addon {
            Some(a) if a.equipment_id == -2 => 0,
            Some(a) if a.equipment_id >= 0 => a.equipment_id as u32,
            _ => self.info.equipment_id,
        };
        self.load_equipment(equip_id, equipment, virtual_items, rng);
    }

    /// `LoadEquipment(equip_id, force=true)`: apply one
    /// equipment template (0 clears the slots).
    pub fn load_equipment(
        &mut self,
        equip_id: u32,
        equipment: &EquipmentTemplates,
        virtual_items: &dyn Fn(u32) -> Option<VirtualItemPrototype>,
        rng: &mut dyn SpawnRng,
    ) {
        if equip_id == 0 {
            for slot in 0..3 {
                self.set_virtual_item(slot, None);
            }
            self.equipment_id = 0;
            return;
        }
        let Some(template) = equipment.get(equip_id) else {
            return;
        };
        self.equipment_id = equip_id;
        if template.total_probability == 0 {
            return;
        }
        let roll = rng.urand(0, template.total_probability - 1);
        let mut sum = 0u32;
        let mut chosen = None;
        for entry in &template.entries {
            if entry.probability == 0 {
                continue;
            }
            sum += entry.probability;
            if roll < sum {
                chosen = Some(entry.item);
                break;
            }
        }
        if let Some(items) = chosen {
            for (slot, item_id) in items.into_iter().enumerate() {
                let proto = if item_id == 0 {
                    None
                } else {
                    virtual_items(item_id)
                };
                // Unknown prototypes leave the slot untouched
                // in C++ (error log); fresh slots are 0 anyway.
                if item_id == 0 || proto.is_some() {
                    self.set_virtual_item(slot as u16, proto);
                }
            }
        }
    }

    /// `SetVirtualItem(slot, itemId)` for script
    /// equipment changes (Slice 6b).
    pub fn apply_virtual_item(&mut self, slot: u16, proto: Option<VirtualItemPrototype>) {
        self.set_virtual_item(slot, proto);
    }

    /// `SetVirtualItem(slot, proto)` value writes; `None`
    /// clears the slot.
    fn set_virtual_item(&mut self, slot: u16, proto: Option<VirtualItemPrototype>) {
        let v = self.unit.world.object.values_mut();
        let info = offsets::UNIT_VIRTUAL_ITEM_INFO + slot * 2;
        match proto {
            None => {
                v.set_u32(offsets::UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot, 0);
                v.set_u32(info, 0);
                v.set_u32(info + 1, 0);
            }
            Some(p) => {
                v.set_u32(offsets::UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot, p.display_id);
                v.set_byte(info, 0, p.class);
                v.set_byte(info, 1, p.subclass);
                v.set_byte(info, 2, p.material);
                v.set_byte(info, 3, p.inventory_type);
                v.set_byte(info + 1, 0, p.sheath);
            }
        }
    }
}

/// `roundf` to u32 with C++ cast semantics (negative wraps
/// in C++; inputs here are non-negative by construction).
fn round_u32(value: f32) -> u32 {
    value.round() as u32
}

/// `SetHealthPercent`/`SetPowerPercent` shape:
/// `uint32(max * percent / 100)` in float.
fn set_percent(max: u32, percent: f32) -> u32 {
    #[allow(clippy::cast_precision_loss)]
    let value = max as f32 * percent / 100.0;
    value as u32
}

/// `ChooseDisplayId` (no game-event data in Slice 3):
/// addon override, then probability roll, then equal
/// chance, then the box placeholder.
fn choose_display(
    info: &CreatureInfo,
    addon: Option<&CreatureDataAddon>,
    display: &DisplayResolver<'_>,
    rng: &mut dyn SpawnRng,
) -> (u32, f32) {
    if let Some(a) = addon {
        if a.display_id != 0 {
            return (a.display_id, (display.scale_for_display)(a.display_id));
        }
    }
    let mut index: Option<usize> = None;
    if info.display_total_probability != 0 {
        let roll = rng.urand(1, info.display_total_probability);
        let mut sum = 0u32;
        for (i, (&id, &chance)) in info
            .display_id
            .iter()
            .zip(info.display_probability.iter())
            .enumerate()
        {
            if id == 0 || chance == 0 {
                continue;
            }
            if roll > sum && roll <= sum + chance {
                index = Some(i);
                break;
            }
            sum += chance;
        }
    } else {
        let count = info.display_id.iter().take_while(|&&id| id != 0).count();
        if count > 0 {
            index = Some(rng.urand(0, count as u32 - 1) as usize);
        }
    }
    match index {
        Some(i) => {
            let display_id = info.display_id[i];
            let scale = if info.display_scale[i] == 0.0 {
                (display.scale_for_display)(display_id)
            } else {
                info.display_scale[i]
            };
            (display_id, scale)
        }
        None => (UNIT_DISPLAY_ID_BOX, 1.0),
    }
}

/// `Creature::ChooseDisplayId(template)` without a spawn
/// addon or game event (mount/transform aura handlers);
/// returns `(display_id, displayScale)`.
#[must_use]
pub fn choose_display_id_for_template(
    info: &CreatureInfo,
    display: &DisplayResolver<'_>,
    rng: &mut dyn SpawnRng,
) -> (u32, f32) {
    choose_display(info, None, display, rng)
}

/// `GetCreatureDisplayInfoRandomGender(display)->display_id`
/// fallback chain collapsed to the input display.
#[must_use]
pub fn random_gender_display(
    addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
    display_id: u32,
    rng: &mut dyn SpawnRng,
) -> u32 {
    random_gender_addon(addons, display_id, rng).map_or(display_id, |a| a.display_id)
}

/// `GetCreatureDisplayInfoRandomGender`: 50% swap to the
/// other-gender display when both addons exist.
fn random_gender_addon(
    addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
    display_id: u32,
    rng: &mut dyn SpawnRng,
) -> Option<CreatureDisplayInfoAddon> {
    let minfo = addons.creature(display_id).copied()?;
    if minfo.display_id_other_gender != 0 && rng.urand(0, 1) == 0 {
        if let Some(other) = addons.creature(minfo.display_id_other_gender) {
            return Some(*other);
        }
    }
    Some(minfo)
}
