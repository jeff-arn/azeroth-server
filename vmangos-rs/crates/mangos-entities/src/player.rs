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
    InventoryRow, ItemLootRow, MailRow, MailedItemRow, QuestStatusRow, ReputationRow, SkillRow,
    SocialRow, SpellCooldownRow, SpellRow,
};
use mangos_db::characters::AccountDataEntry;
use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_movement::movement_info::{Position, is_valid_map_coord};
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{PLAYER_END, offsets};
use mangos_protocol::update_fields::TypeMask;
use mangos_world_data::item_prototype::ItemPrototypes;
use mangos_world_data::player_info::{PlayerInfos, RaceDbc};
use mangos_world_data::quest_template::{QuestTemplate, QuestTemplates};
use mangos_world_data::skill_info::{SkillDbc, SkillRange};
use mangos_world_data::spawns::WorldLocation;

use crate::object::ObjectBase;
use crate::unit::{
    DeathState, MAX_POWERS, MAX_SPELL_SCHOOL, MAX_STATS, ModelData, POWER_ENERGY, POWER_FOCUS,
    POWER_HAPPINESS, POWER_MANA, POWER_RAGE, STAT_AGILITY, STAT_INTELLECT, STAT_SPIRIT,
    STAT_STAMINA, STAT_STRENGTH, Team, UnitBase, WorldObjectBase, check_valid_scale,
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
const REST_STATE_RESTED: u8 = 0x01;

const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;
const PLAYER_FLAGS_RESTING: u32 = 0x0000_0020;
/// `PLAYER_FLAGS_PARTIAL_PLAY_TIME` (`Player.h:330`).
const PLAYER_FLAGS_PARTIAL_PLAY_TIME: u32 = 0x0000_1000;
/// `PLAYER_FLAGS_NO_PLAY_TIME` (`Player.h:331`).
const PLAYER_FLAGS_NO_PLAY_TIME: u32 = 0x0000_2000;
/// `SKILL_FLAG_ALWAYS_MAX_VALUE` (`DBCEnums.h:176`,
/// `SkillRaceClassInfo` flags).
const SKILL_FLAG_ALWAYS_MAX_VALUE: u32 = 0x10;
/// `GetConfigMaxSkillValue` tree default
/// (`MaxSkillValue` config, 300).
const MAX_SKILL_VALUE: u32 = 300;
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
    /// Full item prototypes for equipped-item stat
    /// application (`_ApplyItemMods` at load); `None` in
    /// prototype-free tests keeps the zero-item totals.
    pub item_prototypes: Option<&'a ItemPrototypes>,
    /// Quest template subset for the load-time timer
    /// conversion and `AdjustQuestReqItemCount` recount;
    /// `None` in template-free tests keeps quest rows
    /// verbatim (no timer conversion, no recount churn).
    pub quest_templates: Option<&'a QuestTemplates>,
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
    /// `character_pet` rows (Tier 1 Slice 2): the
    /// map runtime selects/mutates these as pets are
    /// summoned, loaded, and saved; DB writes leave
    /// through `MapEvent` save snapshots.
    pub pets: Vec<mangos_db::pet::PetRow>,
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

/// As-loaded copies of the save-diffed row sets
/// (Phase 6 Slice 1): fork tracks per-entry
/// `uState`; the Rust aggregate diffs live rows
/// against this baseline at save time and then
/// re-baselines, mirroring `uState = UNCHANGED`.
/// Inventory rows are also retained here: items are
/// DB-backed by the service layer today, so the
/// save-side item queue derives from this set until
/// runtime item state lands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedBaseline {
    pub spells: Vec<mangos_db::character_load::SpellRow>,
    pub skills: Vec<mangos_db::character_load::SkillRow>,
    pub quest_status: Vec<mangos_db::character_load::QuestStatusRow>,
    pub reputation: Vec<mangos_db::character_load::ReputationRow>,
    pub inventory: Vec<mangos_db::character_load::InventoryRow>,
    pub honor_cp: Vec<mangos_db::character_load::HonorCpRow>,
    pub actions: Vec<mangos_db::character_load::ActionRow>,
}

/// `RestType` (`Player.h:575-580`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestType {
    #[default]
    No = 0,
    InTavern = 1,
    InCity = 2,
}

/// Per-quest live timer state converted at load
/// (`_LoadQuestStatus`): `m_timer` remaining
/// milliseconds for timed quests, 0 for all others.
/// Save writes `now + remaining/1000`
/// (`_SaveQuestStatus`), so the baseline keeps the
/// stored absolute value while this carries the fork's
/// live remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestTimerState {
    pub remaining_ms_at_load: u32,
    /// In `m_timedquests` (timed template, not yet
    /// rewarded, status not NONE): the fork marks
    /// these `QUEST_CHANGED` on every tick, so every
    /// save rewrites them.
    pub timed_active: bool,
}

/// `ComputeRest` offline/online share (`Player.cpp:8393`):
/// seconds × next-level XP / 1152000, rates at the
/// fork defaults of 1.0 (the `Rate.Rest.*` config reads
/// are not wired yet, like the other `Rate.*` values).
#[must_use]
pub fn compute_rest(seconds: u64, next_level_xp: u32, offline: bool, in_rest_place: bool) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let mut bonus = seconds as f32 * (next_level_xp as f32 / 1_152_000.0);
    if offline && !in_rest_place {
        bonus /= 4.0;
    }
    bonus
}

/// `SetRestBonus` value rules (`Player.cpp:17940`):
/// zero at max level, floored at 0, capped at
/// next-level XP × 1.5 / 2.
#[must_use]
pub fn capped_rest_bonus(value: f32, next_level_xp: u32, level: u32, max_level: u32) -> f32 {
    if level >= max_level {
        return 0.0;
    }
    let value = if value < 0.0 { 0.0 } else { value };
    #[allow(clippy::cast_precision_loss)]
    let max = next_level_xp as f32 * 1.5 / 2.0;
    if value > max { max } else { value }
}

/// `CalculateTalentsPoints` (`Player.cpp:20549`) at
/// the `Rate.Talent` 1.0 default: one point per level
/// from 10.
#[must_use]
pub fn talent_points_for_level(level: u32) -> u32 {
    if level < 10 { 0 } else { level - 9 }
}

/// One `SMSG_LEVELUP_INFO` payload (`GiveLevel`,
/// `Player.cpp:3230`): deltas against the create-stat
/// baseline of the previous level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LevelUpInfo {
    pub level: u32,
    pub health_gain: u32,
    /// Zero unless the class power is mana (fork
    /// writes the mana delta only for `POWER_MANA`).
    pub mana_gain: u32,
    pub stat_gains: [i32; MAX_STATS],
}

/// What one [`Player::give_xp`] granted: the
/// `SendLogXPGain` amounts plus each level gained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XpGainOutcome {
    /// XP without the rested bonus.
    pub given_xp: u32,
    pub rest_xp: u32,
    pub level_ups: Vec<LevelUpInfo>,
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
    /// `HonorMgr` stored scalars (manager unmodeled:
    /// loaded and saved verbatim until honor updates
    /// land).
    pub honor_rank_points: f32,
    pub honor_highest_rank: i32,
    pub honor_standing: i32,
    pub honor_last_week_hk: i32,
    pub honor_last_week_cp: f32,
    pub honor_stored_hk: i32,
    pub honor_stored_dk: i32,
    pub logout_time: u64,
    pub drunk: u16,
    /// `m_restBonus`, with the offline accrual of
    /// `LoadFromDB` applied at materialize.
    pub rest_bonus: f32,
    /// `m_restType` (starts `REST_TYPE_NO`; the map's
    /// zone/tavern logic drives it from there).
    pub rest_type: RestType,
    /// `m_innTriggerId` while `InTavern`.
    pub inn_trigger_id: u32,
    /// Map-time milliseconds when rest (re)started
    /// (`m_timeInnEnter`, fork `time_t` seconds; the
    /// online grant consumes whole seconds exactly as
    /// the fork's second-resolution clock does).
    pub inn_enter_ms: Option<u64>,
    /// `time(nullptr)` at materialize: the epoch the
    /// quest timers' remainders are anchored to.
    pub load_now_unix: u64,
    /// Live quest timer remainders by quest id
    /// (see [`QuestTimerState`]).
    pub quest_timers: std::collections::HashMap<u32, QuestTimerState>,
    /// Quests `AdjustQuestReqItemCount` marked
    /// `QUEST_CHANGED` at load; cleared when a save
    /// re-baselines (fork `uState = QUEST_UNCHANGED`).
    pub quest_load_changed: std::collections::HashSet<u32>,
    pub stable_slots: u8,
    pub death_expire_time: u64,
    pub reset_talents_multiplier: u32,
    pub reset_talents_time: u64,
    /// `m_taxi` known-node mask words (`LoadTaxiMask` parse).
    pub taxi_mask: Vec<u32>,
    /// Loaded `PlayerTaxi` destinations. Taxi flight
    /// itself is unmodeled, but retaining the list keeps
    /// `current_taxi_path` persistence fork-shaped.
    pub taxi_destinations: Vec<u32>,
    pub homebind: Homebind,
    /// `SaveRecallPosition` snapshot after the final
    /// relocation of `LoadFromDB`.
    pub recall: WorldLocation,
    pub guild_id: u32,
    pub guild_rank: u8,
    pub group_id: Option<u32>,
    /// `m_regenTimer` (`Unit.h:354`, int32; only the
    /// player 2000 ms cadence is driven from it here —
    /// creature regen keeps its AI-side timer).
    pub regen_timer_ms: i32,
    /// `m_carryHealthRegen` (`Player.h:1456`).
    pub carry_health_regen: f32,
    /// Item-fed `m_auraBaseMod[SHIELD_BLOCK_VALUE][FLAT_MOD]`
    /// sum (equipped prototypes' `Block`); the aura-fed
    /// part joins the shield-block aura handlers.
    pub shield_block_flat: f32,
    /// Item-fed modifier aggregates retained for
    /// aura-time derived-stat refreshes.
    equipped: EquippedBonuses,
    /// `HaveOffhandWeapon` at materialize (gates the
    /// offhand damage-field writes exactly as in
    /// `UpdateAttackPowerAndDamage`).
    pub has_offhand_weapon: bool,
    pub deferred: DeferredLoad,
    /// Battleground queue/instance state (`BGData` +
    /// `m_bgBattleGroundQueueID`, Tier 3 Slice 3); seeded
    /// from `deferred.bg_data` at materialize.
    pub battleground: crate::battleground::BgPlayerState,
    /// Save-diff baseline (see [`SavedBaseline`]).
    pub saved: SavedBaseline,
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

/// Item-fed modifier-group `BASE_VALUE` sums plus
/// weapon state, accumulated exactly as
/// `_ApplyItemMods`/`_ApplyItemBonuses` do per equipped
/// item (`Player.cpp:6916-7100`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct EquippedBonuses {
    /// `UNIT_MOD_STAT_*` `BASE_VALUE` per stat index.
    stat_base: [f32; MAX_STATS],
    /// `PLAYER_FIELD_POSSTAT0` accumulations.
    pos_stat: [f32; MAX_STATS],
    /// `PLAYER_FIELD_NEGSTAT0` accumulations (negative).
    neg_stat: [f32; MAX_STATS],
    /// `UNIT_MOD_HEALTH` `BASE_VALUE`.
    health_base: f32,
    /// `UNIT_MOD_MANA` `BASE_VALUE`.
    mana_base: f32,
    /// `UNIT_MOD_ARMOR` `BASE_VALUE`.
    armor_base: f32,
    /// `UNIT_MOD_RESISTANCE_*` `BASE_VALUE` by school.
    resist_base: [f32; MAX_SPELL_SCHOOL],
    /// `SHIELD_BLOCK_VALUE` flat sum.
    block_flat: f32,
    /// Damage row 0 per attack type (fists default).
    weapon_damage: [(f32, f32); 3],
    /// Attack times per attack type (`BASE_ATTACK_TIME`
    /// 2000 until a weapon delay applies).
    attack_time_ms: [f32; 3],
    /// `HaveOffhandWeapon`.
    has_offhand_weapon: bool,
}

impl Default for EquippedBonuses {
    fn default() -> Self {
        Self {
            stat_base: [0.0; MAX_STATS],
            pos_stat: [0.0; MAX_STATS],
            neg_stat: [0.0; MAX_STATS],
            health_base: 0.0,
            mana_base: 0.0,
            armor_base: 0.0,
            resist_base: [0.0; MAX_SPELL_SCHOOL],
            block_flat: 0.0,
            weapon_damage: [(1.0, 2.0); 3],
            attack_time_ms: [2000.0; 3],
            has_offhand_weapon: false,
        }
    }
}

impl EquippedBonuses {
    /// `HandleStatModifier(UNIT_MOD_STAT_*, BASE_VALUE)`
    /// + `ApplyStatBuffMod` (field picked by sign).
    fn add_stat(&mut self, stat: usize, val: f32) {
        self.stat_base[stat] += val;
        if val > 0.0 {
            self.pos_stat[stat] += val;
        } else {
            self.neg_stat[stat] += val;
        }
    }
}

/// The load-time equipped-item accumulation
/// (`_LoadInventory` applying `_ApplyItemMods` per
/// equipped slot). Broken items (`IsBroken`: max
/// durability > 0 with durability 0) contribute
/// nothing, as in C++.
fn equipped_bonuses(load: &CharacterLoad, items: Option<&ItemPrototypes>) -> EquippedBonuses {
    equipped_bonuses_from_rows(&load.inventory, items)
}

/// The row-set form of [`equipped_bonuses`]: level-up
/// recomputes from the retained inventory rows (items
/// are DB-backed by the service layer, so the loaded
/// rows remain the equipped set for stat purposes,
/// exactly as at materialize).
fn equipped_bonuses_from_rows(
    inventory: &[InventoryRow],
    items: Option<&ItemPrototypes>,
) -> EquippedBonuses {
    let mut out = EquippedBonuses::default();
    let Some(items) = items else {
        return out;
    };
    for inv in inventory {
        if inv.bag != 0 || inv.slot >= EQUIPMENT_SLOT_END {
            continue;
        }
        let Some(proto) = items.get(inv.item.item_id) else {
            continue;
        };
        if proto.max_durability > 0 && inv.item.durability == 0 {
            continue;
        }
        for stat in &proto.stats {
            if stat.value == 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let val = stat.value as f32;
            match stat.stat_type {
                // ITEM_MOD_MANA / ITEM_MOD_HEALTH
                0 => out.mana_base += val,
                1 => out.health_base += val,
                // ITEM_MOD_AGILITY / STRENGTH / INTELLECT /
                // SPIRIT / STAMINA
                3 => out.add_stat(STAT_AGILITY, val),
                4 => out.add_stat(STAT_STRENGTH, val),
                5 => out.add_stat(STAT_INTELLECT, val),
                6 => out.add_stat(STAT_SPIRIT, val),
                7 => out.add_stat(STAT_STAMINA, val),
                _ => {}
            }
        }
        if proto.armor != 0 {
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let armor = proto.armor as f32;
            out.armor_base += armor;
        }
        if proto.block != 0 {
            #[allow(clippy::cast_precision_loss)] // C++ converts identically
            let block = proto.block as f32;
            out.block_flat += block;
        }
        // Schools: holy 1, fire 2, nature 3, frost 4,
        // shadow 5, arcane 6.
        for (school, res) in [
            (1, proto.holy_res),
            (2, proto.fire_res),
            (3, proto.nature_res),
            (4, proto.frost_res),
            (5, proto.shadow_res),
            (6, proto.arcane_res),
        ] {
            if res != 0 {
                #[allow(clippy::cast_precision_loss)] // C++ converts identically
                let val = res as f32;
                out.resist_base[school] += val;
            }
        }
        // `proto->IsWeapon()` (`ITEM_CLASS_WEAPON`).
        if proto.class == 2 {
            // Attack-type selection in `_ApplyItemBonuses`
            // (ranged slot only counts for ranged weapons;
            // `IsRangedWeapon` keys on inventory type
            // 15/25/26).
            let ranged_weapon = matches!(proto.inventory_type, 15 | 25 | 26);
            let att = match inv.slot {
                17 if ranged_weapon => 2,
                16 => 1,
                _ => 0,
            };
            if proto.damages[0].max != 0.0 {
                out.weapon_damage[att] = (proto.damages[0].min, proto.damages[0].max);
            }
            if inv.slot == 16 {
                out.has_offhand_weapon = true;
            }
            if proto.delay > 0 {
                #[allow(clippy::cast_precision_loss)] // C++ converts identically
                let delay = proto.delay as f32;
                match inv.slot {
                    15 => out.attack_time_ms[0] = delay,
                    16 => out.attack_time_ms[1] = delay,
                    17 => out.attack_time_ms[2] = delay,
                    _ => {}
                }
            }
        }
    }
    out
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
    #[allow(clippy::too_many_lines)]
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
            honor_rank_points: row.honor_rank_points,
            honor_highest_rank: row.honor_highest_rank,
            honor_standing: row.honor_standing,
            honor_last_week_hk: row.honor_last_week_hk,
            honor_last_week_cp: row.honor_last_week_cp,
            honor_stored_hk: row.honor_stored_hk,
            honor_stored_dk: row.honor_stored_dk,
            logout_time: row.logout_time,
            drunk: row.drunk,
            rest_bonus: row.rest_bonus,
            rest_type: RestType::No,
            inn_trigger_id: 0,
            inn_enter_ms: None,
            load_now_unix: inputs.now_unix,
            quest_timers: std::collections::HashMap::new(),
            quest_load_changed: std::collections::HashSet::new(),
            stable_slots: 0,
            death_expire_time: row.death_expire_time,
            reset_talents_multiplier: row.reset_talents_multiplier,
            reset_talents_time: row.reset_talents_time,
            taxi_mask: parse_u32_words(&row.known_taxi_mask),
            taxi_destinations: parse_u32_words(&row.current_taxi_path),
            homebind: Homebind::default(),
            recall: WorldLocation::default(),
            guild_id: 0,
            guild_rank: 0,
            group_id: load.group_id,
            regen_timer_ms: 0,
            carry_health_regen: 0.0,
            shield_block_flat: 0.0,
            equipped: EquippedBonuses::default(),
            has_offhand_weapon: false,
            saved: SavedBaseline {
                spells: load.spells.clone(),
                skills: load.skills.clone(),
                quest_status: load.quest_status.clone(),
                reputation: load.reputation.clone(),
                inventory: load.inventory.clone(),
                honor_cp: load.honor_cp.clone(),
                actions: load.actions.clone(),
            },
            battleground: crate::battleground::BgPlayerState::from_row(load.bg_data.as_ref()),
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
                pets: std::mem::take(&mut load.pets),
            },
        };
        player.init_identity_values(&load, inputs);
        player.load_skill_fields(inputs);
        player.init_position(&load, &create_info, inputs);
        player.init_stats_for_level(&load, inputs);
        player.init_combat_percentages();
        player.restore_health_powers(&load);
        player.apply_offline_rest(inputs);
        player.convert_quest_timers(inputs);
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
    /// `LearnSpell` bookkeeping subset: record the
    /// spell in the runtime spell set
    /// (`deferred.spells`, persisted by the save
    /// diff as a `character_spell` insert).
    /// Returns true when newly learned. Dependent
    /// spells (lower ranks, skill abilities) are
    /// not expanded here.
    pub fn learn_spell(&mut self, spell_id: u32) -> bool {
        if self.deferred.spells.iter().any(|r| r.spell == spell_id) {
            return false;
        }
        self.deferred.spells.push(SpellRow {
            spell: spell_id,
            active: 1,
            disabled: 0,
        });
        true
    }

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
        self.write_skill_fields(inputs.skills);
    }

    /// Project `deferred.skills` into the
    /// `PLAYER_SKILL_INFO_1_1` fields (the
    /// [`Player::load_skill_fields`] body, also re-run
    /// after a level-up skill-max update).
    fn write_skill_fields(&mut self, skills: Option<&SkillDbc>) {
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
            if let Some(dbc) = skills {
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
        // `GetStat(STAT_AGILITY)` (unsigned word cast to
        // float, `Unit.h:390`) — not a bit reinterpret.
        let agility = self.unit.stat(STAT_AGILITY);
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

    /// `InitStatsForLevel` + the load-time
    /// `_ApplyItemMods` accumulation (`_LoadInventory`
    /// end state at zero aura modifiers: pct groups 1,
    /// total groups 0). Enchantments, random-property
    /// stats, item sets, and equip spells join the
    /// item domain; aura-fed modifier groups join aura
    /// stat application. `UpdateSkillsForLevel` and
    /// `InitDataForForm` stay deferred (form data is
    /// aura-driven).
    fn init_stats_for_level(&mut self, load: &CharacterLoad, inputs: &MaterializeInputs<'_>) {
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
            // Weapon damage fields are written by the
            // equipped-weapon arm below
            // (`UpdateDamagePhysical`).
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
        // Create stats (SetCreateStat); the SetStat
        // totals land after the item accumulation.
        if let Some(info) = level_info {
            for i in 0..MAX_STATS {
                #[allow(clippy::cast_precision_loss)]
                {
                    self.unit.create_stats[i] = f32::from(info.stats[i]);
                }
            }
        }
        if let Some(info) = class_info {
            self.unit.create_health = u32::from(info.basehealth);
            self.unit.create_mana = u32::from(info.basemana);
        }
        let bonuses = equipped_bonuses(load, inputs.item_prototypes);
        self.equipped = bonuses;
        // Refill included; restore_health_powers then
        // overwrites with the saved values like LoadFromDB.
        self.recompute_derived_stats(&bonuses, true);
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

    /// `UpdateAllStats` + refill at the zero-aura
    /// modifier defaults: derived stats from the item
    /// bonuses, max health/powers, weapon state, then
    /// health/mana/energy to max, rage clamped, focus
    /// and happiness zeroed (the `InitStatsForLevel`
    /// and `GiveLevel` tails are the same writes; both
    /// call sites are alive-player contexts, matching
    /// `GiveLevel`'s `IsAlive()` health gate).
    fn recompute_derived_stats(&mut self, bonuses: &EquippedBonuses, seed_pos_neg: bool) {
        self.apply_stat_bonuses(bonuses, seed_pos_neg);
        self.update_max_health(bonuses.health_base);
        for power in 0..MAX_POWERS {
            let base = if power == POWER_MANA {
                bonuses.mana_base
            } else {
                0.0
            };
            self.update_max_power(power, base);
        }
        self.apply_weapon_state(bonuses);
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
    }

    /// The `HandleStatModifier` update dispatch for aura
    /// stat changes: stat totals, max health/powers, and
    /// weapon state from the retained item aggregates and
    /// the live aura registers. No refill; shrunken
    /// maxima clamp currents in the setters, as in C++.
    pub fn refresh_derived_stats(&mut self) {
        let bonuses = self.equipped;
        self.apply_stat_bonuses(&bonuses, false);
        self.update_max_health(bonuses.health_base);
        for power in 0..MAX_POWERS {
            let base = if power == POWER_MANA {
                bonuses.mana_base
            } else {
                0.0
            };
            self.update_max_power(power, base);
        }
        self.apply_weapon_state(&bonuses);
    }

    /// `UpdateMaxHealth` (`StatSystem.cpp:164`):
    /// base-plus-create times `BASE_PCT` (1, no aura
    /// source), plus `TOTAL_VALUE` and the stamina
    /// bonus, all times `TOTAL_PCT`; `health_base`
    /// is the item-fed `UNIT_MOD_HEALTH` base sum.
    fn update_max_health(&mut self, health_base: f32) {
        let (total_value, total_pct) = self.unit.health_modifier_groups();
        #[allow(clippy::cast_precision_loss)]
        let value = (health_base
            + self.unit.create_health as f32
            + total_value
            + health_bonus_from_stamina(self.unit.stat(STAT_STAMINA)))
            * total_pct;
        // `std::max(1, int(value))`: float->int truncates.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max = if value < 1.0 { 1 } else { value as u32 };
        self.unit.set_max_health(max);
    }

    /// `UpdateMaxPower` at zero-aura modifier defaults;
    /// `base_value` is the item-fed group `BASE_VALUE`
    /// (only mana has an item source in 1.12 stats).
    fn update_max_power(&mut self, power: usize, base_value: f32) {
        let create = self.unit.create_power(power);
        let bonus = if power == POWER_MANA && create > 0 {
            mana_bonus_from_intellect(self.unit.stat(STAT_INTELLECT))
        } else {
            0.0
        };
        let (total_value, total_pct) = self.unit.power_modifier_groups(power);
        #[allow(clippy::cast_precision_loss)]
        let value = (base_value + create as f32 + total_value + bonus) * total_pct;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max = value as u32;
        self.unit.set_max_power(power, max);
    }

    /// The derived-stat writes from the accumulated
    /// item bonuses composed with the aura stat
    /// registers (`UpdateStats` via `GetTotalStatValue`),
    /// `UpdateArmor`, and `UpdateResistances` (holy is
    /// written 0 in C++). The `ApplyStatBuffMod` display
    /// halves are seeded from the item split only at
    /// load (`seed_pos_neg`); aura handlers adjust them
    /// incrementally afterwards, as in C++.
    fn apply_stat_bonuses(&mut self, bonuses: &EquippedBonuses, seed_pos_neg: bool) {
        for i in 0..MAX_STATS {
            let total = self.unit.total_stat_value(i, bonuses.stat_base[i]);
            #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
            self.unit.set_stat(i, total as i32);
        }
        if seed_pos_neg {
            let v = self.unit.world.object.values_mut();
            for i in 0..MAX_STATS {
                #[allow(clippy::cast_possible_truncation)]
                let idx = i as u16;
                v.set_f32(offsets::PLAYER_FIELD_POSSTAT0 + idx, bonuses.pos_stat[i]);
                v.set_f32(offsets::PLAYER_FIELD_NEGSTAT0 + idx, bonuses.neg_stat[i]);
            }
        }
        #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
        let armor = (bonuses.armor_base + self.unit.stat(STAT_AGILITY) * 2.0) as i32;
        self.unit.set_armor(armor);
        for school in 1..MAX_SPELL_SCHOOL {
            #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
            let value = if school == 1 {
                0
            } else {
                bonuses.resist_base[school] as i32
            };
            self.unit.set_resistance(school, value);
        }
        self.shield_block_flat = bonuses.block_flat;
        self.has_offhand_weapon = bonuses.has_offhand_weapon;
    }

    /// Weapon base ranges + attack times
    /// (`SetBaseWeaponDamage`/`SetAttackTime`), then
    /// `UpdateAttackPowerAndDamage` for both arms with
    /// its `UpdateDamagePhysical` writes (offhand only
    /// with the dual-wield passive 674 and an offhand
    /// weapon).
    fn apply_weapon_state(&mut self, bonuses: &EquippedBonuses) {
        self.unit.base_weapon_damage = bonuses.weapon_damage;
        {
            // `SetAttackTime`: raw delay × attack-speed pct.
            let pct = self.unit.mod_attack_speed_pct;
            let v = self.unit.world.object.values_mut();
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME,
                bonuses.attack_time_ms[0] * pct[0],
            );
            v.set_f32(
                offsets::UNIT_FIELD_BASEATTACKTIME + 1,
                bonuses.attack_time_ms[1] * pct[1],
            );
            v.set_f32(
                offsets::UNIT_FIELD_RANGEDATTACKTIME,
                bonuses.attack_time_ms[2] * pct[2],
            );
        }
        let melee_ap = self.unit.attack_power_from_stats(false);
        let ranged_ap = self.unit.attack_power_from_stats(true);
        self.unit.base_attack_power = melee_ap;
        {
            let v = self.unit.world.object.values_mut();
            #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
            v.set_i32(offsets::UNIT_FIELD_ATTACK_POWER, melee_ap as i32);
            #[allow(clippy::cast_possible_truncation)] // C++ truncates identically
            v.set_i32(offsets::UNIT_FIELD_RANGED_ATTACK_POWER, ranged_ap as i32);
        }
        let (min_d, max_d) = self.unit.calculate_min_max_damage(0);
        let (rmin_d, rmax_d) = self.unit.calculate_min_max_damage(2);
        let dual_wield = self
            .deferred
            .spells
            .iter()
            .any(|s| s.spell == 674 && s.active != 0);
        let offhand = if dual_wield && bonuses.has_offhand_weapon {
            Some(self.unit.calculate_min_max_damage(1))
        } else {
            None
        };
        let v = self.unit.world.object.values_mut();
        v.set_f32(offsets::UNIT_FIELD_MINDAMAGE, min_d);
        v.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, max_d);
        v.set_f32(offsets::UNIT_FIELD_MINRANGEDDAMAGE, rmin_d);
        v.set_f32(offsets::UNIT_FIELD_MAXRANGEDDAMAGE, rmax_d);
        if let Some((omin_d, omax_d)) = offhand {
            v.set_f32(offsets::UNIT_FIELD_MINOFFHANDDAMAGE, omin_d);
            v.set_f32(offsets::UNIT_FIELD_MAXOFFHANDDAMAGE, omax_d);
        }
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

    /// `SetRestBonus` (`Player.cpp:17940`): value rules
    /// plus the rest-state byte side effect (>10 rested,
    /// <=1 normal, unchanged in between).
    pub fn apply_rest_bonus(&mut self, value: f32, max_level: u32) {
        let next_xp = self
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_NEXT_LEVEL_XP);
        self.rest_bonus = capped_rest_bonus(value, next_xp, self.unit.level(), max_level);
        let v = self.unit.world.object.values_mut();
        if self.rest_bonus > 10.0 {
            v.set_byte(offsets::PLAYER_BYTES_2, 3, REST_STATE_RESTED);
        } else if self.rest_bonus <= 1.0 {
            v.set_byte(offsets::PLAYER_BYTES_2, 3, REST_STATE_NORMAL);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        v.set_u32(
            offsets::PLAYER_REST_STATE_EXPERIENCE,
            self.rest_bonus as u32,
        );
    }

    /// `GetXPRestBonus` (`Player.cpp:8380`): consume up
    /// to `xp` rested points (the total gain caps at
    /// 200%).
    fn xp_rest_bonus(&mut self, xp: u32, max_level: u32) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rested = (self.rest_bonus as u32).min(xp);
        #[allow(clippy::cast_precision_loss)]
        self.apply_rest_bonus(self.rest_bonus - rested as f32, max_level);
        rested
    }

    /// `GiveXP` (`Player.cpp:3062`), returning what the
    /// session must be told (`SendLogXPGain` amounts,
    /// then one [`LevelUpInfo`] per level gained, in
    /// fork send order); `None` when the fork returns
    /// before `SendLogXPGain`. `is_kill` is the fork's
    /// `victim != nullptr` (rest bonus applies to kill
    /// XP only). Fork deltas: the personal XP rate is
    /// its constructor default −1 (no loader sets it in
    /// the Rust runtime either, so the multiply is the
    /// same no-op), and trial restrictions are
    /// session-side and unmodeled.
    /// The `PLAYER_EXPLORED_ZONES_1 + flag/32` bit test
    /// from `CheckAreaExploreAndOutdoor` (out-of-range
    /// flags read as explored so callers skip them like
    /// the fork's error return).
    #[must_use]
    pub fn has_explored_flag(&self, area_flag: u16) -> bool {
        let offset = u32::from(area_flag) / 32;
        if offset >= EXPLORED_ZONES_SIZE {
            return true;
        }
        let word = self
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_EXPLORED_ZONES_1 + offset as u16);
        word & (1u32 << (u32::from(area_flag) % 32)) != 0
    }

    /// Sets the explored bit for `area_flag`
    /// (`SetUInt32Value(PLAYER_EXPLORED_ZONES_1 +
    /// offset, curr | val)`).
    pub fn set_explored_flag(&mut self, area_flag: u16) {
        let offset = u32::from(area_flag) / 32;
        if offset >= EXPLORED_ZONES_SIZE {
            return;
        }
        let index = offsets::PLAYER_EXPLORED_ZONES_1 + offset as u16;
        let values = self.unit.world.object.values_mut();
        let word = values.get_u32(index);
        values.set_u32(index, word | (1u32 << (u32::from(area_flag) % 32)));
    }

    pub fn give_xp(
        &mut self,
        mut xp: u32,
        is_kill: bool,
        infos: &PlayerInfos,
        items: Option<&ItemPrototypes>,
        skills: Option<&SkillDbc>,
    ) -> Option<XpGainOutcome> {
        {
            let flags = self
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_FLAGS);
            // SUPPORTED_CLIENT_BUILD 5875 > 1.7.1.
            if flags & PLAYER_FLAGS_NO_PLAY_TIME != 0 {
                return None;
            }
            if flags & PLAYER_FLAGS_PARTIAL_PLAY_TIME != 0 {
                xp /= 2;
            }
        }
        if xp < 1 || !self.unit.is_alive() {
            return None;
        }
        let max_level = infos.max_player_level();
        let mut level = self.unit.level();
        if level >= max_level {
            return None;
        }
        let rest_xp = if is_kill {
            self.xp_rest_bonus(xp, max_level)
        } else {
            0
        };
        let (mut new_xp, mut next_lvl_xp) = {
            let v = self.unit.world.object.values();
            (
                v.get_u32(offsets::PLAYER_XP) + xp + rest_xp,
                v.get_u32(offsets::PLAYER_NEXT_LEVEL_XP),
            )
        };
        let mut level_ups = Vec::new();
        // A zero threshold means an empty xp table; the
        // fork would spin, real data never does this.
        while next_lvl_xp > 0 && new_xp >= next_lvl_xp && level < max_level {
            new_xp -= next_lvl_xp;
            level_ups.push(self.give_level(level + 1, infos, items, skills));
            level = self.unit.level();
            next_lvl_xp = self
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_NEXT_LEVEL_XP);
        }
        self.unit
            .world
            .object
            .values_mut()
            .set_u32(offsets::PLAYER_XP, new_xp);
        Some(XpGainOutcome {
            given_xp: xp,
            rest_xp,
            level_ups,
        })
    }

    /// `GiveLevel` (`Player.cpp:3115`) minus its
    /// logging/GM-alert and battleground-queue sides
    /// (BGs are out of this goal's scope) and the pet
    /// level sync (no pet runtime yet; Slice 3). The
    /// caller owns `PLAYER_XP`, as in the fork.
    /// Talent points move by the `CalculateTalentsPoints`
    /// delta: the used-point count is untracked until
    /// a talent DBC store and learn handler land, so
    /// the loaded free/used split is preserved rather
    /// than recomputed (`Rate.Talent` at its 1.0
    /// default, like the other unwired `Rate.*`).
    fn give_level(
        &mut self,
        level: u32,
        infos: &PlayerInfos,
        items: Option<&ItemPrototypes>,
        skills: Option<&SkillDbc>,
    ) -> LevelUpInfo {
        let old_level = self.unit.level();
        let race = u32::from(self.unit.race());
        let class = u32::from(self.unit.class());
        let level_info = infos.level_info(race, class, level);
        let class_info = infos.class_level_info(class, level);
        // Deltas against the create baseline, computed
        // before it is re-based (fork sends the packet
        // before applying the new level).
        let create_stamina = self.unit.create_stats[STAT_STAMINA];
        let create_intellect = self.unit.create_stats[STAT_INTELLECT];
        #[allow(clippy::cast_possible_wrap)]
        let health_gain = match (level_info, class_info) {
            (Some(info), Some(class)) => {
                (i32::from(class.basehealth) - self.unit.create_health as i32)
                    + (health_bonus_from_stamina(f32::from(info.stats[STAT_STAMINA]))
                        - health_bonus_from_stamina(create_stamina)) as i32
            }
            _ => 0,
        };
        #[allow(clippy::cast_possible_wrap)]
        let mana_gain = match (level_info, class_info) {
            (Some(info), Some(class)) => {
                (i32::from(class.basemana) - self.unit.create_mana as i32)
                    + (mana_bonus_from_intellect(f32::from(info.stats[STAT_INTELLECT]))
                        - mana_bonus_from_intellect(create_intellect)) as i32
            }
            _ => 0,
        };
        let mut stat_gains = [0i32; MAX_STATS];
        if let Some(info) = level_info {
            for (i, gain) in stat_gains.iter_mut().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                {
                    *gain = i32::from(info.stats[i]) - self.unit.create_stats[i] as i32;
                }
            }
        }
        {
            let v = self.unit.world.object.values_mut();
            v.set_u32(offsets::PLAYER_NEXT_LEVEL_XP, infos.xp_for_level(level));
            v.set_u32(offsets::UNIT_FIELD_LEVEL, level);
            let points = v.get_u32(offsets::PLAYER_CHARACTER_POINTS1)
                + talent_points_for_level(level)
                - talent_points_for_level(old_level);
            v.set_u32(offsets::PLAYER_CHARACTER_POINTS1, points);
        }
        self.played_time_level = 0;
        self.update_skills_for_level(skills);
        if let Some(info) = level_info {
            for i in 0..MAX_STATS {
                self.unit.create_stats[i] = f32::from(info.stats[i]);
            }
        }
        if let Some(class) = class_info {
            self.unit.create_health = u32::from(class.basehealth);
            self.unit.create_mana = u32::from(class.basemana);
        }
        let bonuses = equipped_bonuses_from_rows(&self.saved.inventory, items);
        self.equipped = bonuses;
        self.recompute_derived_stats(&bonuses, false);
        LevelUpInfo {
            level,
            #[allow(clippy::cast_sign_loss)]
            health_gain: health_gain as u32,
            mana_gain: if self.unit.power_type() == POWER_MANA as u8 {
                #[allow(clippy::cast_sign_loss)]
                {
                    mana_gain as u32
                }
            } else {
                0
            },
            stat_gains,
        }
    }

    /// `UpdateSkillsForLevel` (`Player.cpp:5613`) over
    /// the retained skill rows: level-range skills get
    /// the new `level * 5` max (always-max race/class
    /// skills also gain the value; `AlwaysMaxSkill`
    /// config is at its false default), then the
    /// fields are re-projected.
    fn update_skills_for_level(&mut self, skills: Option<&SkillDbc>) {
        if let Some(dbc) = skills {
            let race = u32::from(self.unit.race());
            let class = u32::from(self.unit.class());
            let max_skill = self.unit.level() * 5;
            for row in &mut self.deferred.skills {
                let Some(category) = dbc.category(row.skill) else {
                    continue;
                };
                let Some(rc) = dbc.race_class_info(row.skill, race, class) else {
                    continue;
                };
                if dbc.range_type(category, rc) != SkillRange::Level || row.max == 1 {
                    continue;
                }
                if rc.flags & SKILL_FLAG_ALWAYS_MAX_VALUE != 0 {
                    row.value = max_skill;
                    row.max = max_skill;
                } else if row.max != MAX_SKILL_VALUE {
                    row.max = max_skill;
                }
            }
        }
        self.write_skill_fields(skills);
    }

    /// Offline rest accrual (`LoadFromDB`,
    /// `Player.cpp:15081-15085`): the stored bonus plus
    /// `ComputeRest(logout elapsed, offline, was resting)`,
    /// applied only when time actually passed.
    fn apply_offline_rest(&mut self, inputs: &MaterializeInputs<'_>) {
        // Fork `time_t` difference; accrual only runs
        // when time actually passed.
        if inputs.now_unix <= self.logout_time {
            return;
        }
        let time_diff = inputs.now_unix - self.logout_time;
        let next_xp = self
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_NEXT_LEVEL_XP);
        let in_rest_place = self.character_flags & CHARACTER_FLAG_RESTING != 0;
        let gain = compute_rest(time_diff, next_xp, true, in_rest_place);
        let bonus = self.rest_bonus + gain;
        self.apply_rest_bonus(bonus, inputs.infos.max_player_level());
    }

    /// `SetRestType` (`Player.cpp:21181`): flag + inn
    /// bookkeeping. `now_ms` is the caller's map clock.
    pub fn set_rest_type(&mut self, rest_type: RestType, area_trigger_id: u32, now_ms: u64) {
        self.rest_type = rest_type;
        let v = self.unit.world.object.values_mut();
        if rest_type == RestType::No {
            v.remove_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_RESTING);
            self.inn_trigger_id = 0;
            self.inn_enter_ms = None;
        } else {
            v.set_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_RESTING);
            self.inn_trigger_id = area_trigger_id;
            self.inn_enter_ms = Some(now_ms);
        }
    }

    /// Load-time quest timer conversion
    /// (`_LoadQuestStatus`, `Player.cpp:15864-15876`)
    /// plus the post-inventory `AdjustQuestReqItemCount`
    /// pass (`Player.cpp:15125-15131` + `13948`): the
    /// fork recounts deliver-item objectives from live
    /// inventory and marks those quests changed
    /// unconditionally, so the next save rewrites them
    /// (timer included). Quests with no template are
    /// skipped exactly as the fork skips them.
    fn convert_quest_timers(&mut self, inputs: &MaterializeInputs<'_>) {
        let Some(templates) = inputs.quest_templates else {
            return;
        };
        let now = inputs.now_unix;
        // `GetItemCount(item, checkInBank=true)`: every
        // character_inventory row (equipped, bags, bank).
        let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for inv in &self.saved.inventory {
            *counts.entry(inv.item.item_id).or_default() += inv.item.count;
        }
        for row in &mut self.deferred.quest_status {
            let Some(tmpl) = templates.get(row.quest) else {
                continue;
            };
            let timed_active = tmpl.limit_time != 0 && row.rewarded == 0 && row.status != 0;
            let remaining_ms_at_load = if timed_active {
                if row.timer <= now {
                    1
                } else {
                    // Fork truncates through `uint32(diff) * IN_MILLISECONDS`.
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        ((row.timer - now) as u32).wrapping_mul(1000)
                    }
                }
            } else {
                0
            };
            self.quest_timers.insert(
                row.quest,
                QuestTimerState {
                    remaining_ms_at_load,
                    timed_active,
                },
            );
            if tmpl.is_deliver() {
                let mut marked = false;
                for idx in 0..4 {
                    let req_id = tmpl.req_item_ids[idx];
                    let mut req_count = tmpl.req_item_counts[idx];
                    // Quest load zeroes the count when the
                    // item prototype does not exist
                    // (`ObjectMgr.cpp:5610`).
                    if req_id != 0
                        && inputs
                            .item_prototypes
                            .is_some_and(|protos| protos.get(req_id).is_none())
                    {
                        req_count = 0;
                    }
                    if req_count != 0 {
                        let cur = counts.get(&req_id).copied().unwrap_or(0);
                        row.item_count[idx] = cur.min(req_count);
                        marked = true;
                    }
                }
                if marked {
                    self.quest_load_changed.insert(row.quest);
                }
            }
        }
        self.rebuild_quest_log(templates);
    }

    /// `GetQuestStatus` over the runtime rows
    /// (`QUEST_STATUS_NONE` when absent).
    #[must_use]
    pub fn quest_status(&self, quest_id: u32) -> i32 {
        self.deferred
            .quest_status
            .iter()
            .find(|r| r.quest == quest_id)
            .map_or(0, |r| r.status)
    }

    #[must_use]
    pub fn quest_row(&self, quest_id: u32) -> Option<&QuestStatusRow> {
        self.deferred
            .quest_status
            .iter()
            .find(|r| r.quest == quest_id)
    }

    /// Mutable row, inserting the fork default
    /// (status NONE, zeroed counters) when absent.
    ///
    /// # Panics
    ///
    /// Never in practice: the index comes from a
    /// `position` scan (or push) on this same vec.
    pub fn quest_row_or_insert(&mut self, quest_id: u32) -> &mut QuestStatusRow {
        let pos = if let Some(pos) = self
            .deferred
            .quest_status
            .iter()
            .position(|r| r.quest == quest_id)
        {
            pos
        } else {
            self.deferred.quest_status.push(QuestStatusRow {
                quest: quest_id,
                status: 0,
                rewarded: 0,
                explored: 0,
                timer: 0,
                mob_count: [0; 4],
                item_count: [0; 4],
                reward_choice: 0,
            });
            self.deferred.quest_status.len() - 1
        };
        &mut self.deferred.quest_status[pos]
    }

    /// `SetQuestStatus` (row created on first touch).
    pub fn set_quest_status(&mut self, quest_id: u32, status: i32) {
        self.quest_row_or_insert(quest_id).status = status;
    }

    /// `FindQuestSlot`: scan the log fields.
    #[must_use]
    pub fn find_quest_slot(&self, quest_id: u32) -> Option<u16> {
        let v = self.unit.world.object.values();
        (0..20u16).find(|&slot| {
            v.get_u32(offsets::PLAYER_QUEST_LOG_1_1 + slot * 3) == quest_id && quest_id != 0
        })
    }

    /// `SetQuestSlot` (id + zeroed count/state + timer).
    pub fn set_quest_slot(&mut self, slot: u16, quest_id: u32, timer: u32) {
        let base = offsets::PLAYER_QUEST_LOG_1_1 + slot * 3;
        let v = self.unit.world.object.values_mut();
        v.set_u32(base, quest_id);
        v.set_u32(base + 1, 0);
        v.set_u32(base + 2, timer);
    }

    /// `SetQuestSlotCounter`: 6-bit packed counter.
    pub fn set_quest_slot_counter(&mut self, slot: u16, counter: u8, count: u8) {
        let field = offsets::PLAYER_QUEST_LOG_1_1 + slot * 3 + 1;
        let v = self.unit.world.object.values_mut();
        let mut val = v.get_u32(field);
        val &= !(0x3Fu32 << (counter * 6));
        val |= u32::from(count) << (counter * 6);
        v.set_u32(field, val);
    }

    /// `SetQuestSlotState` / `RemoveQuestSlotState`
    /// (byte 3 of the count/state word).
    pub fn set_quest_slot_state(&mut self, slot: u16, state: u8, on: bool) {
        let field = offsets::PLAYER_QUEST_LOG_1_1 + slot * 3 + 1;
        let v = self.unit.world.object.values_mut();
        let mut bytes = v.get_u32(field).to_le_bytes();
        if on {
            bytes[3] |= state;
        } else {
            bytes[3] &= !state;
        }
        v.set_u32(field, u32::from_le_bytes(bytes));
    }

    /// `_LoadQuestStatus` log reconstruction: slots
    /// for INCOMPLETE/COMPLETE/FAILED rows that are
    /// unrewarded (or repeatable), counters and
    /// state flags from the rows, tail cleared.
    pub fn rebuild_quest_log(&mut self, templates: &QuestTemplates) {
        let rows: Vec<QuestStatusRow> = self.deferred.quest_status.clone();
        let mut slot = 0u16;
        for row in &rows {
            if slot >= 20 {
                break;
            }
            if !matches!(row.status, 1 | 3 | 5) {
                continue;
            }
            let repeatable = templates
                .get(row.quest)
                .is_some_and(QuestTemplate::is_repeatable);
            if row.rewarded != 0 && !repeatable {
                continue;
            }
            let timer_secs = self
                .quest_timers
                .get(&row.quest)
                .map_or(0, |t| t.remaining_ms_at_load / 1000);
            self.set_quest_slot(slot, row.quest, timer_secs);
            if row.explored != 0 || row.status == 1 {
                self.set_quest_slot_state(slot, 0x01, true);
            }
            if row.status == 5 {
                self.set_quest_slot_state(slot, 0x02, true);
            }
            for idx in 0..4u8 {
                let count = row.mob_count[idx as usize];
                if count != 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    self.set_quest_slot_counter(slot, idx, count as u8);
                }
            }
            slot += 1;
        }
        for tail in slot..20u16 {
            self.set_quest_slot(tail, 0, 0);
        }
    }

    /// Container guids of the bags in the four
    /// inventory bag slots (fork `INVENTORY_SLOT_BAG`
    /// 19..23). Row `bag` values are container item
    /// guids; 0 marks a direct character slot.
    fn inventory_bag_guids(&self) -> Vec<u32> {
        self.saved
            .inventory
            .iter()
            .filter(|inv| inv.bag == 0 && (19..23).contains(&inv.slot))
            .map(|inv| inv.item.item_guid)
            .collect()
    }

    /// Inventory count of an item
    /// (`GetItemCount(item, inBankAlso=false)` and
    /// `HasItemCount` share the slot coverage:
    /// equipment, backpack, and keyring direct rows
    /// plus inventory-bag contents; bank rows and
    /// bank-bag contents excluded. Session grants
    /// and destroys keep `saved.inventory` in
    /// lockstep with their DB-row writes. The fork's
    /// `IsInTrade` exclusion in `HasItemCount` has
    /// no row flag here.)
    #[must_use]
    pub fn quest_item_count(&self, item_id: u32) -> u32 {
        let bags = self.inventory_bag_guids();
        self.saved
            .inventory
            .iter()
            .filter(|inv| inv.item.item_id == item_id)
            .filter(|inv| {
                if inv.bag == 0 {
                    inv.slot < 39 || (81..97).contains(&inv.slot)
                } else {
                    bags.contains(&inv.bag)
                }
            })
            .map(|inv| inv.item.count)
            .sum()
    }

    /// Stacks `DestroyItemCount` walks for an item,
    /// in fork order (backpack direct rows, keyring
    /// direct rows, then each inventory bag's
    /// contents in slot order), as
    /// (item guid, stack count) pairs. Equipped and
    /// bank rows are skipped, as in the fork.
    #[must_use]
    pub fn quest_destroy_candidates(&self, item_id: u32) -> Vec<(u32, u32)> {
        let direct = |range: std::ops::Range<u8>| -> Vec<(u32, u32)> {
            let mut rows: Vec<(u8, u32, u32)> = self
                .saved
                .inventory
                .iter()
                .filter(|inv| {
                    inv.bag == 0 && inv.item.item_id == item_id && range.contains(&inv.slot)
                })
                .map(|inv| (inv.slot, inv.item.item_guid, inv.item.count))
                .collect();
            rows.sort_by_key(|row| row.0);
            rows.into_iter()
                .map(|(_, guid, count)| (guid, count))
                .collect()
        };
        let mut out = direct(23..39);
        out.extend(direct(81..97));
        let mut bag_slots: Vec<(u8, u32)> = self
            .saved
            .inventory
            .iter()
            .filter(|inv| inv.bag == 0 && (19..23).contains(&inv.slot))
            .map(|inv| (inv.slot, inv.item.item_guid))
            .collect();
        bag_slots.sort_by_key(|row| row.0);
        for (_, bag_guid) in bag_slots {
            let mut rows: Vec<(u8, u32, u32)> = self
                .saved
                .inventory
                .iter()
                .filter(|inv| inv.bag == bag_guid && inv.item.item_id == item_id)
                .map(|inv| (inv.slot, inv.item.item_guid, inv.item.count))
                .collect();
            rows.sort_by_key(|row| row.0);
            out.extend(rows.into_iter().map(|(_, guid, count)| (guid, count)));
        }
        out
    }

    /// `AdjustQuestReqItemCount`: recount the item
    /// objectives from inventory, capped at the
    /// required counts, for DELIVER quests.
    pub fn adjust_quest_item_counts(&mut self, q: &QuestTemplate) {
        if !q.is_deliver() {
            return;
        }
        let counts: [u32; 4] = core::array::from_fn(|i| {
            if q.req_item_ids[i] == 0 {
                0
            } else {
                self.quest_item_count(q.req_item_ids[i])
                    .min(q.req_item_counts[i])
            }
        });
        if let Some(row) = self
            .deferred
            .quest_status
            .iter_mut()
            .find(|r| r.quest == q.entry)
        {
            row.item_count = counts;
        }
    }

    /// `CanCompleteQuest` (auto-complete verdict
    /// included). Item objectives are recounted
    /// first. The fork's timed-quest half-elapsed
    /// branch and reputation-objective stacking are
    /// read directly; live timer ticking is
    /// unmodeled, so an armed timer never blocks.
    pub fn can_complete_quest(&mut self, q: &QuestTemplate) -> bool {
        let Some(mut row) = self.quest_row(q.entry).cloned() else {
            return false;
        };
        if row.status != 3 {
            return false;
        }
        if q.is_deliver() {
            self.adjust_quest_item_counts(q);
            if let Some(updated) = self.quest_row(q.entry) {
                row = updated.clone();
            }
        }
        if q.method == 0 {
            return true;
        }
        for i in 0..4 {
            if q.req_creature_or_go_ids[i] != 0 && row.mob_count[i] < q.req_creature_or_go_counts[i]
            {
                return false;
            }
            if q.req_item_ids[i] != 0 && row.item_count[i] < q.req_item_counts[i] {
                return false;
            }
            if q.req_source_ids[i] != 0
                && self.quest_item_count(q.req_source_ids[i]) < q.req_source_counts[i]
            {
                return false;
            }
        }
        if q.rep_objective_faction != 0 {
            let standing = self
                .deferred
                .reputation
                .iter()
                .find(|r| r.faction == q.rep_objective_faction)
                .map_or(0, |r| r.standing);
            if q.rep_objective_value > 0 && standing < q.rep_objective_value {
                return false;
            }
            if q.rep_objective_value < 0 && standing > q.rep_objective_value {
                return false;
            }
        }
        if q.special_flags & 0x002 != 0 && row.explored == 0 {
            return false;
        }
        true
    }

    /// `CompleteQuest`: status → COMPLETE + log-slot
    /// state flag. Returns true on transition (the
    /// caller sends `SMSG_QUESTUPDATE_COMPLETE`).
    pub fn complete_quest(&mut self, quest_id: u32) -> bool {
        let Some(row) = self.quest_row(quest_id) else {
            return false;
        };
        if row.status != 3 {
            return false;
        }
        self.set_quest_status(quest_id, 1);
        if let Some(slot) = self.find_quest_slot(quest_id) {
            self.set_quest_slot_state(slot, 0x01, true);
        }
        true
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
