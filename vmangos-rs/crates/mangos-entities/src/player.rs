//! The Player aggregate (crate map §12: domain state lives
//! inside one aggregate, never as independent owners) and
//! the Slice 3 materialization subset of
//! `Player::LoadFromDB` (`Player.cpp:14470-14980`) +
//! `InitStatsForLevel` (`Player.cpp:3292-3446`).
//!
//! Materialized here: identity/appearance bytes, level/xp,
//! explored zones, display ids + scale + model data, money,
//! character-flag projections, faction/team, position or
//! homebind fallback, drunk sobering, played/create times,
//! taxi mask, stable/death clamps, base stats and
//! naked-character max health/powers
//! (`UpdateMaxHealth`/`UpdateMaxPower` with zero aura
//! modifiers), health/power restore, guild fields, and the
//! visible-equipment projection of bag-0 equipment rows.
//!
//! Deferred, with the raw rows retained in
//! [`Player::deferred`] (recorded in `PHASE4-PROGRESS.md`):
//! auras (Slice 5), spells/talents (Slice 5), quests, real
//! Item objects and their stat bonuses (so max health/mana
//! are the naked values until Slice 5), reputation
//! internals, rest-bonus accrual, transport/taxi-flight and
//! dungeon/BG login relocation (continent logins only;
//! instance maps fall back to homebind in Slice 3).

use mangos_db::character_load::{
    ActionRow, AuraRow, BgDataRow, BoundInstanceRow, CharacterLoad, ForgottenSkillRow, HonorCpRow,
    ItemLootRow, MailRow, MailedItemRow, QuestStatusRow, ReputationRow, SkillRow, SocialRow,
    SpellCooldownRow, SpellRow,
};
use mangos_db::characters::AccountDataEntry;
use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_movement::movement_info::{Position, is_valid_map_coord};
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{PLAYER_END, offsets};
use mangos_protocol::update_fields::TypeMask;
use mangos_world_data::player_info::{PlayerInfos, RaceDbc};
use mangos_world_data::skill_info::{SkillDbc, SkillRange};
use mangos_world_data::spawns::WorldLocation;

use crate::object::ObjectBase;
use crate::unit::{
    DeathState, MAX_POWERS, MAX_SPELL_SCHOOL, MAX_STATS, ModelData, POWER_ENERGY, POWER_FOCUS,
    POWER_HAPPINESS, POWER_MANA, POWER_RAGE, STAT_AGILITY, STAT_INTELLECT, STAT_STAMINA, Team,
    UnitBase, WorldObjectBase, check_valid_scale,
};

/// `MAX_MONEY_AMOUNT` (`Player.h:656`).
pub const MAX_MONEY_AMOUNT: u32 = 0x7FFF_FFFF - 1;
/// `MAX_TRIAL_MONEY_AMOUNT` (`Player.h:657`).
pub const MAX_TRIAL_MONEY_AMOUNT: u32 = 100_000;
/// `MAX_PET_STABLES` (`Pet.h:37`).
const MAX_PET_STABLES: u32 = 2;
/// `MAX_DEATH_COUNT * DEATH_EXPIRE_STEP` (`Player.cpp:104-105`).
const DEATH_EXPIRE_CLAMP: u64 = 3 * 5 * 60;
/// `PLAYER_EXPLORED_ZONES_SIZE`.
const EXPLORED_ZONES_SIZE: u32 = 64;
/// `EQUIPMENT_SLOT_END` (`Player.h:503`).
const EQUIPMENT_SLOT_END: u8 = 19;
/// `MAX_VISIBLE_ITEM_OFFSET` at 5875 (`ItemDefines.h:157`).
const MAX_VISIBLE_ITEM_OFFSET: u16 = 12;
/// `MAX_INSPECTED_ENCHANTMENT_SLOT` (`ItemDefines.h:147`).
const MAX_INSPECTED_ENCHANTMENT_SLOT: usize = 2;
/// Enchantment string columns per slot (id, duration,
/// charges) × `MAX_ENCHANTMENT_SLOT` (7).
const ENCHANTMENT_WORDS_PER_SLOT: usize = 3;

const UNIT_BYTE2_FLAG_UNK3: u8 = 0x08;
const UNIT_BYTE2_FLAG_UNK5: u8 = 0x20;
const UNIT_BYTE2_FLAG_PVP: u8 = 0x01;
const UNIT_FLAG_PLAYER_CONTROLLED: u32 = 0x0000_0008;
const UNIT_FLAG_NOT_ATTACKABLE_1: u32 = 0x0000_0080;
const REST_STATE_NORMAL: u8 = 0x02;

const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;
const PLAYER_FLAGS_RESTING: u32 = 0x0000_0020;
const PLAYER_FLAGS_PVP_DESIRED: u32 = 0x0000_0200;
const PLAYER_FLAGS_HIDE_HELM: u32 = 0x0000_0400;
const PLAYER_FLAGS_HIDE_CLOAK: u32 = 0x0000_0800;

const CHARACTER_FLAG_RESTING: u32 = 0x0000_0002;
const CHARACTER_FLAG_BEASTMASTER: u32 = 0x0000_0020;
const CHARACTER_FLAG_PVP_ENABLED: u32 = 0x0000_0040;
const CHARACTER_FLAG_HIDE_HELM: u32 = 0x0000_0400;
const CHARACTER_FLAG_HIDE_CLOAK: u32 = 0x0000_0800;
const CHARACTER_FLAG_GHOST: u32 = 0x0000_2000;
const CHARACTER_FLAG_PVP_DESIRED: u32 = 0x0001_0000;

/// Inputs the binary resolves from DBC/config state before
/// materialization (this crate stays DBC-free).
#[derive(Clone, Copy, Debug)]
pub struct MaterializeInputs<'a> {
    pub infos: &'a PlayerInfos,
    /// The character's `ChrRaces` subset (team/faction).
    pub race: Option<&'a RaceDbc>,
    /// `ChrClassesEntry.powerType` for the character's class.
    pub power_type: u8,
    /// `GetScaleForDisplayId` result for the gendered display.
    pub display_scale: f32,
    /// `UpdateModelData` inputs for the gendered display.
    pub model: Option<ModelData>,
    /// Whether the saved map exists (`sMapStorage` lookup).
    pub saved_map_known: bool,
    /// Whether the saved map is a dungeon/BG (Slice 3 falls
    /// back to homebind instead of entrance relocation).
    pub saved_map_is_instance: bool,
    /// `GetMaxMoney()` for the session.
    pub max_money: u32,
    /// `time(nullptr)` at load.
    pub now_unix: u64,
    /// Skill DBC tables for `_LoadSkills` validation and
    /// range fixups; `None` in DBC-free tests writes the
    /// saved rows verbatim.
    pub skills: Option<&'a SkillDbc>,
}

/// Load rows retained verbatim for later slices.
#[derive(Clone, Debug, Default)]
pub struct DeferredLoad {
    pub auras: Vec<AuraRow>,
    pub spells: Vec<SpellRow>,
    pub spell_cooldowns: Vec<SpellCooldownRow>,
    pub skills: Vec<SkillRow>,
    pub forgotten_skills: Vec<ForgottenSkillRow>,
    pub quest_status: Vec<QuestStatusRow>,
    pub reputation: Vec<ReputationRow>,
    pub actions: Vec<ActionRow>,
    pub item_loot: Vec<ItemLootRow>,
    pub honor_cp: Vec<HonorCpRow>,
    pub mails: Vec<MailRow>,
    pub mailed_items: Vec<MailedItemRow>,
    pub bound_instances: Vec<BoundInstanceRow>,
    pub bg_data: Option<BgDataRow>,
    pub account_data: Vec<AccountDataEntry>,
    /// Live friend/ignore list (`PlayerSocial` rows): loaded
    /// here, mutated by the mangosd social handlers
    /// (Phase 5 Slice A), written back per change.
    pub social: Vec<SocialRow>,
}

/// Homebind location (`m_homebind*` + recall position seed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Homebind {
    pub location: WorldLocation,
    pub area_id: u32,
    /// True when synthesized from create info because no
    /// `character_homebind` row exists. C++ INSERTs the row
    /// in that case (`_LoadHomeBind`); that write is outside
    /// the Phase 4 register, so it is only flagged here.
    pub synthesized: bool,
}

/// The Player aggregate.
#[derive(Clone, Debug)]
pub struct Player {
    pub unit: UnitBase,
    pub account_id: u32,
    pub team: Team,
    pub character_flags: u32,
    pub extra_flags: u32,
    /// `m_playedTime[PLAYED_TIME_TOTAL]`.
    pub played_time_total: u32,
    /// `m_playedTime[PLAYED_TIME_LEVEL]`.
    pub played_time_level: u32,
    pub create_time: u64,
    pub logout_time: u64,
    pub drunk: u16,
    /// `m_restBonus` (saved value; offline accrual deferred).
    pub rest_bonus: f32,
    pub stable_slots: u8,
    pub death_expire_time: u64,
    pub reset_talents_multiplier: u32,
    pub reset_talents_time: u64,
    /// `m_taxi` known-node mask words (`LoadTaxiMask` parse).
    pub taxi_mask: Vec<u32>,
    pub homebind: Homebind,
    /// `SaveRecallPosition` snapshot after the final
    /// relocation of `LoadFromDB`.
    pub recall: WorldLocation,
    pub guild_id: u32,
    pub guild_rank: u8,
    pub group_id: Option<u32>,
    pub deferred: DeferredLoad,
}

/// Why materialization refused the character (C++ returns
/// `false` from `LoadFromDB`; the account/locked/name gates
/// run earlier in the session layer).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializeError {
    /// No `playercreateinfo` for the race/class pair
    /// (`_LoadHomeBind` nullptr).
    InvalidRaceClass,
}

/// `GetHealthBonusFromStamina` (`StatSystem.cpp:148-154`).
#[must_use]
pub fn health_bonus_from_stamina(stamina: f32) -> f32 {
    let base = if stamina < 20.0 { stamina } else { 20.0 };
    base + (stamina - base) * 10.0
}

/// `GetManaBonusFromIntellect` (`StatSystem.cpp:156-162`).
#[must_use]
pub fn mana_bonus_from_intellect(intellect: f32) -> f32 {
    let base = if intellect < 20.0 { intellect } else { 20.0 };
    base + (intellect - base) * 15.0
}

impl Player {
    /// Materializes a player from its read-only load. The
    /// account-match / transfer-lock / name gates belong to
    /// the session layer (they ran in Phase 3).
    ///
    /// # Errors
    ///
    /// [`MaterializeError::InvalidRaceClass`] when the pair
    /// has no create info.
    pub fn materialize(
        load: CharacterLoad,
        inputs: &MaterializeInputs<'_>,
    ) -> Result<Player, MaterializeError> {
        let mut load = load;
        let row = &load.row;
        let create_info = inputs
            .infos
            .create_info(u32::from(row.race), u32::from(row.class))
            .copied()
            .ok_or(MaterializeError::InvalidRaceClass)?;

        let mut object = ObjectBase::new(
            TypeId::Player,
            TypeMask::OBJECT
                .union(TypeMask::UNIT)
                .union(TypeMask::PLAYER),
            ObjectUpdateFlags(
                ObjectUpdateFlags::ALL
                    | ObjectUpdateFlags::LIVING
                    | ObjectUpdateFlags::HAS_POSITION,
            ),
            PLAYER_END,
        );
        object.create(row.guid, 0, HighGuid::PLAYER);
        let mut world = WorldObjectBase::new(object);
        world.name.clone_from(&row.name);
        world.world_mask = row.world_phase_mask;
        let unit = UnitBase::new(world);
        let mut player = Player {
            unit,
            account_id: row.account,
            team: Team::None,
            character_flags: row.character_flags,
            extra_flags: row.extra_flags,
            played_time_total: row.played_time_total,
            played_time_level: row.played_time_level,
            create_time: row.create_time,
            logout_time: row.logout_time,
            drunk: row.drunk,
            rest_bonus: row.rest_bonus,
            stable_slots: 0,
            death_expire_time: row.death_expire_time,
            reset_talents_multiplier: row.reset_talents_multiplier,
            reset_talents_time: row.reset_talents_time,
            taxi_mask: parse_u32_words(&row.known_taxi_mask),
            homebind: Homebind::default(),
            recall: WorldLocation::default(),
            guild_id: 0,
            guild_rank: 0,
            group_id: load.group_id,
            deferred: DeferredLoad {
                auras: std::mem::take(&mut load.auras),
                spells: std::mem::take(&mut load.spells),
                spell_cooldowns: std::mem::take(&mut load.spell_cooldowns),
                skills: std::mem::take(&mut load.skills),
                forgotten_skills: std::mem::take(&mut load.forgotten_skills),
                quest_status: std::mem::take(&mut load.quest_status),
                reputation: std::mem::take(&mut load.reputation),
                actions: std::mem::take(&mut load.actions),
                item_loot: std::mem::take(&mut load.item_loot),
                honor_cp: std::mem::take(&mut load.honor_cp),
                mails: std::mem::take(&mut load.mails),
                mailed_items: std::mem::take(&mut load.mailed_items),
                bound_instances: std::mem::take(&mut load.bound_instances),
                bg_data: load.bg_data.take(),
                account_data: std::mem::take(&mut load.account_data),
                social: std::mem::take(&mut load.social),
            },
        };
        player.init_identity_values(&load, inputs);
        player.load_skill_fields(inputs);
        player.init_position(&load, &create_info, inputs);
        player.init_stats_for_level(inputs);
        player.init_combat_percentages();
        player.restore_health_powers(&load);
        Ok(player)
    }

    /// The value writes before position handling
    /// (`Player.cpp:14533-14640` subset).
    fn init_identity_values(&mut self, load: &CharacterLoad, inputs: &MaterializeInputs<'_>) {
        let row = &load.row;
        let v = self.unit.world.object.values_mut();
        v.set_guid(
            offsets::OBJECT_FIELD_GUID,
            ObjectGuid::new(HighGuid::PLAYER, row.guid),
        );
        let gender = row.gender & 0x01;
        v.set_byte(offsets::UNIT_FIELD_BYTES_0, 0, row.race);
        v.set_byte(offsets::UNIT_FIELD_BYTES_0, 1, row.class);
        v.set_byte(offsets::UNIT_FIELD_BYTES_0, 2, gender);
        v.set_byte(offsets::UNIT_FIELD_BYTES_0, 3, inputs.power_type);
        v.set_byte(
            offsets::UNIT_FIELD_BYTES_2,
            1,
            UNIT_BYTE2_FLAG_UNK3 | UNIT_BYTE2_FLAG_UNK5,
        );
        v.set_u32(offsets::UNIT_FIELD_LEVEL, u32::from(row.level));
        v.set_u32(offsets::PLAYER_XP, row.xp);
        v.load_into_data_field(
            &row.explored_zones,
            u32::from(offsets::PLAYER_EXPLORED_ZONES_1),
            EXPLORED_ZONES_SIZE,
        );
        v.set_u32(
            offsets::PLAYER_FIELD_COINAGE,
            row.money.min(inputs.max_money),
        );
        v.set_byte(offsets::PLAYER_BYTES, 0, row.skin);
        v.set_byte(offsets::PLAYER_BYTES, 1, row.face);
        v.set_byte(offsets::PLAYER_BYTES, 2, row.hair_style);
        v.set_byte(offsets::PLAYER_BYTES, 3, row.hair_color);
        v.set_byte(offsets::PLAYER_BYTES_2, 0, row.facial_hair);
        v.set_byte(offsets::PLAYER_BYTES_2, 1, 0xEE);
        // SetBankBagSlotCount (PLAYER_BYTES_2 byte 2).
        v.set_byte(offsets::PLAYER_BYTES_2, 2, row.bank_bag_slots);
        v.set_byte(offsets::PLAYER_BYTES_2, 3, REST_STATE_NORMAL);
        v.set_u16(
            offsets::PLAYER_BYTES_3,
            0,
            (row.drunk & 0xFFFE) | u16::from(gender),
        );
        v.set_i32(
            offsets::PLAYER_FIELD_WATCHED_FACTION_INDEX,
            row.watched_faction,
        );
        v.set_u32(offsets::PLAYER_AMMO_ID, row.ammo_id);
        v.set_byte(offsets::PLAYER_FIELD_BYTES, 2, row.action_bars);
        if let Some(guild) = &load.guild {
            v.set_u32(offsets::PLAYER_GUILDID, guild.guild_id);
            v.set_u32(offsets::PLAYER_GUILDRANK, u32::from(guild.rank));
        }
        // Character-flag projections (Player.cpp:14572-14584).
        let flags = row.character_flags;
        if flags & CHARACTER_FLAG_RESTING != 0 {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_RESTING);
        }
        if flags & CHARACTER_FLAG_BEASTMASTER != 0 {
            v.set_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_ATTACKABLE_1);
        }
        if flags & CHARACTER_FLAG_PVP_ENABLED != 0 {
            // UpdatePvP(true) -> SetPvP: misc byte flag.
            let misc = v.get_byte(offsets::UNIT_FIELD_BYTES_2, 1);
            v.set_byte(offsets::UNIT_FIELD_BYTES_2, 1, misc | UNIT_BYTE2_FLAG_PVP);
        }
        if flags & CHARACTER_FLAG_PVP_DESIRED != 0 {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_PVP_DESIRED);
        }
        if flags & CHARACTER_FLAG_HIDE_HELM != 0 {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_HIDE_HELM);
        }
        if flags & CHARACTER_FLAG_HIDE_CLOAK != 0 {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_HIDE_CLOAK);
        }
        if flags & CHARACTER_FLAG_GHOST != 0 {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_GHOST);
        }
        self.guild_id = load.guild.as_ref().map_or(0, |g| g.guild_id);
        self.guild_rank = load.guild.as_ref().map_or(0, |g| g.rank);
        // InitPlayerDisplayIds + SetFactionForRace.
        self.init_display(load, inputs);
    }

    fn init_display(&mut self, load: &CharacterLoad, inputs: &MaterializeInputs<'_>) {
        let row = &load.row;
        let gender = row.gender & 0x01;
        if let Some(create) = inputs
            .infos
            .create_info(u32::from(row.race), u32::from(row.class))
        {
            let display = if gender == 1 {
                create.display_id_f
            } else {
                create.display_id_m
            };
            self.unit
                .world
                .object
                .set_object_scale(check_valid_scale(inputs.display_scale));
            self.unit.set_native_display_id(display);
            self.unit.set_display_id(display, inputs.model);
        }
        self.team = match inputs.race {
            None => Team::Alliance, // C++ logs + ALLIANCE
            Some(r) => match r.team_id {
                7 => Team::Alliance,
                1 => Team::Horde,
                _ => Team::None,
            },
        };
        let faction = inputs.race.map_or(0, |r| r.faction_id);
        self.unit.set_faction_template(faction);
    }
}

/// Space-separated decimal u32 words (`LoadTaxiMask` /
/// explored-zone storage shape), C `strtoul`-ish prefix
/// semantics via `atol` in the values store for zones; the
/// taxi mask retains plain parses.
fn parse_u32_words(data: &str) -> Vec<u32> {
    data.split_whitespace()
        .map(|tok| tok.parse::<u32>().unwrap_or(0))
        .collect()
}

/// `PLAYER_MAX_SKILLS` (`Player.h:69`).
const PLAYER_MAX_SKILLS: usize = 127;

impl Player {
    /// `_LoadSkills` (`Player.cpp:20285-20371`) field
    /// population: `PLAYER_SKILL_INFO_1_1` triplets
    /// (id, `MAKE_SKILL_VALUE(value, max)`, bonus 0) for
    /// the saved `character_skills` rows, with the C++
    /// existence/race-class validation and range fixups.
    /// Deviations: invalid/zero rows are skipped without
    /// the C++ corrective DB writes (character save is
    /// Phase 6), `UpdateSkillTrainedSpells` is not run
    /// (login spells come from `character_spell` plus
    /// creation defaults), and no skill-status save
    /// tracking is kept. Without skill DBC inputs the
    /// rows are written verbatim.
    fn load_skill_fields(&mut self, inputs: &MaterializeInputs<'_>) {
        let race = u32::from(self.unit.race());
        let class = u32::from(self.unit.class());
        // `GetSkillMaxForLevel` (`SpellCaster.h:320`).
        let max_for_level = u16::try_from(self.unit.level() * 5).unwrap_or(u16::MAX);
        let mut entries: Vec<(u32, u16, u16)> = Vec::new();
        for row in &self.deferred.skills {
            if entries.len() >= PLAYER_MAX_SKILLS {
                break;
            }
            // C++ reads `GetUInt16` (truncating).
            #[allow(clippy::cast_possible_truncation)]
            let (mut value, mut max) = (row.value as u16, row.max as u16);
            if let Some(dbc) = inputs.skills {
                let Some(category) = dbc.category(row.skill) else {
                    continue;
                };
                let Some(rc) = dbc.race_class_info(row.skill, race, class) else {
                    continue;
                };
                match dbc.range_type(category, rc) {
                    SkillRange::Language => {
                        value = 300;
                        max = 300;
                    }
                    SkillRange::Mono => {
                        value = 1;
                        max = 1;
                    }
                    SkillRange::Level => {
                        max = max_for_level;
                    }
                    SkillRange::Rank => {}
                }
            }
            if value == 0 {
                continue;
            }
            entries.push((row.skill, value, max));
        }
        let v = self.unit.world.object.values_mut();
        for slot in 0..PLAYER_MAX_SKILLS {
            let base = offsets::PLAYER_SKILL_INFO_1_1 + (slot as u16) * 3;
            let (skill, packed) = match entries.get(slot) {
                Some((skill, value, max)) => (*skill, u32::from(*value) | (u32::from(*max) << 16)),
                None => (0, 0),
            };
            v.set_u32(base, skill);
            v.set_u32(base + 1, packed);
            v.set_u32(base + 2, 0);
        }
    }

    /// Homebind + saved-position resolution
    /// (`Player.cpp:14615-14772` continent subset).
    fn init_position(
        &mut self,
        load: &CharacterLoad,
        create: &mangos_world_data::player_info::PlayerCreateInfo,
        inputs: &MaterializeInputs<'_>,
    ) {
        let row = &load.row;
        // _LoadHomeBind: saved row, else the create position
        // (C++ also INSERTs it; flagged via `synthesized`).
        self.homebind = match &load.homebind {
            Some(hb) => Homebind {
                location: WorldLocation {
                    map_id: hb.map as u16,
                    x: hb.position_x,
                    y: hb.position_y,
                    z: hb.position_z,
                    o: 0.0,
                },
                area_id: hb.zone,
                synthesized: false,
            },
            None => Homebind {
                location: WorldLocation {
                    map_id: create.map_id as u16,
                    x: create.position_x,
                    y: create.position_y,
                    z: create.position_z,
                    o: create.orientation,
                },
                area_id: create.area_id,
                synthesized: true,
            },
        };
        let saved = WorldLocation {
            map_id: row.map as u16,
            x: row.position_x,
            y: row.position_y,
            z: row.position_z,
            o: row.orientation,
        };
        let valid = is_valid_map_coord(saved.x, saved.y, saved.z, saved.o)
            && inputs.saved_map_known
            && !inputs.saved_map_is_instance
            && row.transport_guid == 0
            && row.current_taxi_path.trim().is_empty();
        let dest = if valid { saved } else { self.homebind.location };
        self.unit.world.map_id = u32::from(dest.map_id);
        self.unit
            .world
            .relocate(Position::new(dest.x, dest.y, dest.z, dest.o));
        self.recall = dest;

        // Sobering (Player.cpp:14777-14786) + misc clamps.
        #[allow(clippy::cast_precision_loss)] // C++ float(timeDiff) identically
        {
            let time_diff = inputs.now_unix.saturating_sub(row.logout_time);
            let sober = if time_diff > 15 * 60 {
                0.0f32
            } else {
                1.0 - time_diff as f32 / (15.0 * 60.0)
            };
            self.drunk = (sober * f32::from(row.drunk)) as u16;
            let gender = u16::from(row.gender & 0x01);
            self.unit.world.object.values_mut().set_u16(
                offsets::PLAYER_BYTES_3,
                0,
                (self.drunk & 0xFFFE) | gender,
            );
        }
        self.stable_slots =
            u8::try_from(u32::from(row.stable_slots).min(MAX_PET_STABLES)).unwrap_or(2);
        if self.death_expire_time > inputs.now_unix + DEATH_EXPIRE_CLAMP {
            self.death_expire_time = inputs.now_unix + DEATH_EXPIRE_CLAMP - 1;
        }
        // Ghost flag (post-aura in C++; aura load is Slice 5,
        // so the saved-flag projection decides here).
        if self
            .unit
            .world
            .object
            .values()
            .has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_GHOST)
        {
            self.unit.death_state = DeathState::Dead;
        }
    }

    /// `InitStatsForLevel(false)` + naked
    /// `UpdateDodgePercentage`/`UpdateParryPercentage`/
    /// `UpdateCritPercentage` at naked stats and maxed
    /// skills (Slice 6: the melee outcome table reads
    /// these fields like C++). Block stays 0 until
    /// shield detection lands with item stats.
    fn init_combat_percentages(&mut self) {
        // (dodge/crit class base additions, StatSystem)
        // and the per-class agility divisors
        // (valLevel1, valLevel60) shared by
        // `GetDodgeFromAgility`/`GetMeleeCritFromAgility`
        // except rogue/hunter crit divisors.
        fn class_base(class: u32) -> f32 {
            match class {
                11 => 0.9, // druid
                8 => 3.2,  // mage
                2 => 0.7,  // paladin
                5 => 3.0,  // priest
                7 => 1.7,  // shaman
                9 => 2.0,  // warlock
                _ => 0.0,
            }
        }
        fn dodge_rate(class: u32) -> (f32, f32) {
            match class {
                2 | 7 | 11 => (4.6, 20.0),
                8 => (12.9, 20.0),
                4 => (1.1, 14.5),
                3 => (1.8, 26.5),
                5 => (11.0, 20.0),
                9 => (8.4, 20.0),
                1 => (3.9, 20.0),
                _ => (0.0, 0.0),
            }
        }
        fn crit_rate(class: u32) -> (f32, f32) {
            match class {
                4 => (2.2, 29.0),
                3 => (3.5, 53.0),
                other => dodge_rate(other),
            }
        }
        let class = u32::from(self.unit.class());
        #[allow(clippy::cast_precision_loss)]
        let level = self.unit.level() as f32;
        let agility = f32::from_bits(
            self.unit
                .world
                .object
                .values()
                .get_u32(offsets::UNIT_FIELD_STAT0 + 1),
        );
        let interp = |(lvl1, lvl60): (f32, f32)| {
            if lvl1 == 0.0 && lvl60 == 0.0 {
                0.0
            } else {
                agility / (lvl1 * (60.0 - level) / 59.0 + lvl60 * (level - 1.0) / 59.0)
            }
        };
        let dodge = (class_base(class) + interp(dodge_rate(class))).max(0.0);
        let crit = (class_base(class) + interp(crit_rate(class))).max(0.0);
        // Parry passive spell 3127 gates PLAYER_PARRY.
        let parry = if self
            .deferred
            .spells
            .iter()
            .any(|s| s.spell == 3127 && s.active != 0)
        {
            5.0
        } else {
            0.0
        };
        let v = self.unit.world.object.values_mut();
        v.set_f32(offsets::PLAYER_DODGE_PERCENTAGE, dodge);
        v.set_f32(offsets::PLAYER_PARRY_PERCENTAGE, parry);
        v.set_f32(offsets::PLAYER_BLOCK_PERCENTAGE, 0.0);
        v.set_f32(offsets::PLAYER_CRIT_PERCENTAGE, crit);
        v.set_f32(offsets::PLAYER_RANGED_CRIT_PERCENTAGE, crit);
    }

    /// `UpdateMaxHealth`/`UpdateMaxPower` (zero aura
    /// modifiers). `UpdateSkillsForLevel` and
    /// `InitDataForForm` is deferred (form data is aura-driven;
    /// form data is aura-driven).
    fn init_stats_for_level(&mut self, inputs: &MaterializeInputs<'_>) {
        let race = u32::from(self.unit.race());
        let class = u32::from(self.unit.class());
        let level = self.unit.level();
        let class_info = inputs.infos.class_level_info(class, level);
        let level_info = inputs.infos.level_info(race, class, level);
        {
            let v = self.unit.world.object.values_mut();
            v.set_u32(
                offsets::PLAYER_NEXT_LEVEL_XP,
                inputs.infos.xp_for_level(level),
            );
            v.set_u32(offsets::UNIT_FIELD_AURASTATE, 0);
            // SUPPORTED_CLIENT_BUILD 5875 > 1.11.2: float 1.0.
            v.set_f32(offsets::UNIT_MOD_CAST_SPEED, 1.0);
            v.set_f32(offsets::UNIT_FIELD_BASEATTACKTIME, 2000.0);
            v.set_f32(offsets::UNIT_FIELD_BASEATTACKTIME + 1, 2000.0);
            v.set_f32(offsets::UNIT_FIELD_RANGEDATTACKTIME, 2000.0);
            // Unarmed base weapon damage (`BASE_MINDAMAGE`/
            // `BASE_MAXDAMAGE` through `UpdateDamagePhysical`
            // at identity modifiers; Slice 6 melee consumes
            // these fields). Item-stat projection replaces
            // them when inventory stats land.
            v.set_f32(offsets::UNIT_FIELD_MINDAMAGE, 1.0);
            v.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, 2.0);
            for i in 0..MAX_SPELL_SCHOOL as u16 {
                v.set_i32(offsets::PLAYER_FIELD_MOD_DAMAGE_DONE_NEG + i, 0);
                v.set_u32(offsets::PLAYER_FIELD_MOD_DAMAGE_DONE_POS + i, 0);
                v.set_f32(offsets::PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + i, 1.0);
                v.set_u32(offsets::UNIT_FIELD_POWER_COST_MODIFIER + i, 0);
                v.set_f32(offsets::UNIT_FIELD_POWER_COST_MULTIPLIER + i, 0.0);
            }
            v.set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, 0);
            v.set_flag(offsets::UNIT_FIELD_FLAGS, UNIT_FLAG_PLAYER_CONTROLLED);
            v.set_u32(offsets::PLAYER_FIELD_BYTES2, 0);
        }
        // Create stats + current stats (SetCreateStat/SetStat).
        if let Some(info) = level_info {
            for i in 0..MAX_STATS {
                #[allow(clippy::cast_precision_loss)]
                {
                    self.unit.create_stats[i] = f32::from(info.stats[i]);
                }
                self.unit.set_stat(i, i32::from(info.stats[i]));
            }
        }
        if let Some(info) = class_info {
            self.unit.create_health = u32::from(info.basehealth);
            self.unit.create_mana = u32::from(info.basemana);
        }
        // Armor = agility * 2, twice in C++ (idempotent).
        let armor = (self.unit.create_stats[STAT_AGILITY] * 2.0) as i32;
        self.unit.set_armor(armor);
        for school in 1..MAX_SPELL_SCHOOL {
            self.unit.set_resistance(school, 0);
        }
        // Max powers/health, then InitStatsForLevel's refill
        // (health/mana/energy to max, rage clamped, focus and
        // happiness zeroed) — restore_health_powers then
        // overwrites with the saved values like LoadFromDB.
        self.update_max_health_naked();
        for power in 0..MAX_POWERS {
            self.update_max_power_naked(power);
        }
        let max_health = self.unit.max_health();
        self.unit.set_health(max_health);
        let max_mana = self.unit.max_power(POWER_MANA);
        self.unit.set_power(POWER_MANA, max_mana);
        let max_energy = self.unit.max_power(POWER_ENERGY);
        self.unit.set_power(POWER_ENERGY, max_energy);
        let max_rage = self.unit.max_power(POWER_RAGE);
        if self.unit.power(POWER_RAGE) > max_rage {
            self.unit.set_power(POWER_RAGE, max_rage);
        }
        self.unit.set_power(POWER_FOCUS, 0);
        self.unit.set_power(POWER_HAPPINESS, 0);
        // Pet-loyalty byte for rage/mana classes.
        let power_type = self.unit.power_type();
        if power_type == POWER_RAGE as u8 || power_type == POWER_MANA as u8 {
            self.unit
                .world
                .object
                .values_mut()
                .set_byte(offsets::UNIT_FIELD_BYTES_1, 1, 0xEE);
        }
    }

    /// `UpdateMaxHealth` with the modifier groups at their
    /// zero-aura defaults (BASE 0, `BASE_PCT` 1, TOTAL 0,
    /// `TOTAL_PCT` 1).
    fn update_max_health_naked(&mut self) {
        #[allow(clippy::cast_precision_loss)]
        let value = self.unit.create_health as f32
            + health_bonus_from_stamina(self.unit.stat(STAT_STAMINA));
        // `std::max(1, int(value))`: float->int truncates.
        let max = if value < 1.0 { 1 } else { value as u32 };
        self.unit.set_max_health(max);
    }

    /// `UpdateMaxPower` at zero-aura modifier defaults.
    fn update_max_power_naked(&mut self, power: usize) {
        let create = self.unit.create_power(power);
        #[allow(clippy::cast_precision_loss)]
        let bonus = if power == POWER_MANA && create > 0 {
            mana_bonus_from_intellect(self.unit.stat(STAT_INTELLECT))
        } else {
            0.0
        };
        #[allow(clippy::cast_precision_loss)]
        let value = create as f32 + bonus;
        self.unit.set_max_power(power, value as u32);
    }

    /// Saved health/powers clamped to max
    /// (`Player.cpp:14965-14972`), plus the visible
    /// equipment projection.
    fn restore_health_powers(&mut self, load: &CharacterLoad) {
        let row = &load.row;
        let max_health = self.unit.max_health();
        self.unit.set_health(row.health.min(max_health));
        let saved = [row.power1, row.power2, row.power3, row.power4, row.power5];
        for (power, value) in saved.into_iter().enumerate() {
            let max = self.unit.max_power(power);
            self.unit.set_power(power, value.min(max));
        }
        self.project_visible_equipment(load);
    }

    /// `_LoadInventory` visible half for bag-0 equipment:
    /// inv-slot GUIDs + `SetVisibleItemSlot` words. Item
    /// objects and stat bonuses are deferred; suffix factor
    /// writes 0 (recomputed from loot tables in C++).
    fn project_visible_equipment(&mut self, load: &CharacterLoad) {
        for inv in &load.inventory {
            if inv.bag != 0 || inv.slot >= EQUIPMENT_SLOT_END {
                continue;
            }
            let slot = u16::from(inv.slot);
            let item = &inv.item;
            let v = self.unit.world.object.values_mut();
            v.set_guid(
                offsets::PLAYER_FIELD_INV_SLOT_HEAD + slot * 2,
                ObjectGuid::new(HighGuid::ITEM, item.item_guid),
            );
            let base = offsets::PLAYER_VISIBLE_ITEM_1_0 + slot * MAX_VISIBLE_ITEM_OFFSET;
            let creator = if item.creator_guid != 0 {
                ObjectGuid::new(HighGuid::PLAYER, item.creator_guid)
            } else {
                ObjectGuid::default()
            };
            v.set_guid(
                offsets::PLAYER_VISIBLE_ITEM_1_CREATOR + slot * MAX_VISIBLE_ITEM_OFFSET,
                creator,
            );
            v.set_u32(base, item.item_id);
            for (i, enchant) in parse_enchant_ids(&item.enchantments)
                .into_iter()
                .enumerate()
            {
                v.set_u32(base + 1 + i as u16, enchant);
            }
            v.set_i16(
                offsets::PLAYER_VISIBLE_ITEM_1_PROPERTIES + slot * MAX_VISIBLE_ITEM_OFFSET,
                0,
                item.random_property_id,
            );
            v.set_u32(
                offsets::PLAYER_VISIBLE_ITEM_1_PROPERTIES + 1 + slot * MAX_VISIBLE_ITEM_OFFSET,
                0,
            );
        }
    }
}

/// Enchantment ids from the `item_instance.enchantments`
/// text: 7 slots × (id, duration, charges); missing words
/// read 0 (`Item::LoadFromDB` tokenization).
fn parse_enchant_ids(data: &str) -> [u32; MAX_INSPECTED_ENCHANTMENT_SLOT] {
    let words: Vec<&str> = data.split_whitespace().collect();
    let mut out = [0u32; MAX_INSPECTED_ENCHANTMENT_SLOT];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = words
            .get(i * ENCHANTMENT_WORDS_PER_SLOT)
            .and_then(|w| w.parse::<u32>().ok())
            .unwrap_or(0);
    }
    out
}
