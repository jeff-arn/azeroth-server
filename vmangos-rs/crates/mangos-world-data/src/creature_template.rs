//! `creature_template` → [`CreatureInfo`], ported from
//! `ObjectMgr::LoadCreatureTemplates` / `LoadCreatureInfo` /
//! `CheckCreatureTemplate` (`ObjectMgr.cpp:1173-1620`) and
//! `CreatureDefines.h:234-332`, with the C++ max-patch selection
//! (`patch = max(patch <= WowPatch)` per entry).
//!
//! Deviations, all recorded rather than hidden:
//!
//! - `auras` is parsed into a spell-id list at load (the C++
//!   stores a duplicated C string cast to `uint32 const*` and
//!   converts it inside `CheckCreatureTemplate`). Spell
//!   existence filtering needs the spell store; when no spell id
//!   set is supplied the filter is skipped and noted in the
//!   report — dedupe still applies.
//! - `script_id` comes from the C++ script registry
//!   (`sScriptMgr.GetScriptId`), which does not exist in Rust
//!   yet: the raw `script_name` is kept and `script_id` stays 0
//!   until the scripts slice (Phase 4 Slice 6/7) provides the
//!   registry.
//! - DBC-backed fix-ups (faction/display/type/family validity)
//!   run only when the caller supplies the id sets in
//!   [`TemplateValidation`]; skipped checks are reported once.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::load_report::LoadReport;

/// `MAX_DISPLAY_IDS_PER_CREATURE`.
pub const MAX_DISPLAY_IDS_PER_CREATURE: usize = 4;
/// `CREATURE_MAX_SPELLS`.
pub const CREATURE_MAX_SPELLS: usize = 4;
/// `CREATURE_TYPE_BEAST` (`CreatureDefines.h:43`).
pub const CREATURE_TYPE_BEAST: u32 = 1;
/// `CREATURE_TYPE_HUMANOID` (`CreatureDefines.h:49`).
pub const CREATURE_TYPE_HUMANOID: u32 = 7;
/// `CREATURE_FAMILY_HORSE_CUSTOM` (`CreatureDefines.h:71`).
pub const CREATURE_FAMILY_HORSE_CUSTOM: u32 = 10;
/// `INHABIT_ANYWHERE` (`CreatureDefines.h:488`).
pub const INHABIT_ANYWHERE: u32 = 7;
/// `BASE_ATTACK_TIME` (`UnitDefines.h:73`).
pub const BASE_ATTACK_TIME: u32 = 2000;
/// `MAX_SPELL_SCHOOL` (`SpellDefines.h:724`).
pub const MAX_SPELL_SCHOOL: u32 = 7;
/// `MAX_TRAINER_TYPE` (`CreatureDefines.h:38`).
pub const MAX_TRAINER_TYPE: u32 = 4;
/// `MAX_DB_MOTION_TYPE` (`MotionMaster.h:42`).
pub const MAX_DB_MOTION_TYPE: u32 = 4;
/// `UNIT_NPC_FLAG_TRAINER` (`UnitDefines.h:657`).
pub const UNIT_NPC_FLAG_TRAINER: u32 = 0x10;
/// `UNIT_NPC_FLAG_VENDOR` (`UnitDefines.h:655`).
pub const UNIT_NPC_FLAG_VENDOR: u32 = 0x04;
/// `CLASSMASK_ALL_CREATURES` (`SharedDefines.h:100`):
/// warrior|paladin|rogue|mage class bits `(1 << (class - 1))`.
pub const CLASSMASK_ALL_CREATURES: u32 = (1 << 0) | (1 << 1) | (1 << 3) | (1 << 7);

const STATIC_FLAG_TAMEABLE: u32 = 0x0000_0010;
const STATIC_FLAG_DESPAWN_INSTANTLY: u32 = 0x0000_0800;
const STATIC_FLAG_RAID_BOSS_MOB: u32 = 0x0001_0000;
const STATIC_FLAG_VISIBLE_TO_GHOSTS: u32 = 0x0020_0000;
const STATIC_FLAG_DO_NOT_PLAY_WOUND_ANIM: u32 = 0x0080_0000;
const STATIC_FLAG_NO_FACTION_TOOLTIP: u32 = 0x0100_0000;
const STATIC_FLAG_MORE_AUDIBLE: u32 = 0x4000_0000;
const STATIC_FLAG2_NO_HARMFUL_VERTEX_COLORING: u32 = 0x08;

/// Client `CreatureTypeFlags` bits derived by
/// [`CreatureInfo::type_flags`] (`CreatureDefines.h:147-153`).
pub mod type_flags {
    pub const TAMEABLE: u32 = 0x01;
    pub const VISIBLE_TO_GHOSTS: u32 = 0x02;
    pub const RAID_BOSS_MOB: u32 = 0x04;
    pub const DO_NOT_PLAY_WOUND_ANIM: u32 = 0x08;
    pub const NO_FACTION_TOOLTIP: u32 = 0x10;
    pub const MORE_AUDIBLE: u32 = 0x20;
    pub const NO_HARMFUL_VERTEX_COLORING: u32 = 0x40;
}

/// `struct CreatureInfo` (`CreatureDefines.h:234-301`). Field
/// defaults preserve the C++ member initializers (they matter
/// because `LoadCreatureInfo` overwrites a live struct on reload
/// without resetting it first — same merge here).
#[derive(Clone, Debug)]
pub struct CreatureInfo {
    pub entry: u32,
    pub name: String,
    pub subname: String,
    pub level_min: u32,
    pub level_max: u32,
    pub faction: u32,
    pub npc_flags: u32,
    pub gossip_menu_id: u32,
    pub display_id: [u32; MAX_DISPLAY_IDS_PER_CREATURE],
    pub display_scale: [f32; MAX_DISPLAY_IDS_PER_CREATURE],
    pub display_probability: [u32; MAX_DISPLAY_IDS_PER_CREATURE],
    pub display_total_probability: u32,
    pub mount_display_id: u32,
    pub speed_walk: f32,
    pub speed_run: f32,
    pub detection_range: f32,
    pub call_for_help_range: f32,
    pub leash_range: f32,
    /// `type` column (enum `CreatureType`).
    pub creature_type: u32,
    pub pet_family: u32,
    pub rank: u32,
    pub unit_class: u32,
    pub xp_multiplier: f32,
    pub health_multiplier: f32,
    pub mana_multiplier: f32,
    pub armor_multiplier: f32,
    pub damage_multiplier: f32,
    pub damage_variance: f32,
    pub damage_school: u32,
    pub base_attack_time: u32,
    pub ranged_attack_time: u32,
    pub holy_res: i32,
    pub fire_res: i32,
    pub nature_res: i32,
    pub frost_res: i32,
    pub shadow_res: i32,
    pub arcane_res: i32,
    pub trainer_type: u32,
    pub trainer_spell: u32,
    pub trainer_class: u32,
    pub trainer_race: u32,
    pub loot_id: u32,
    pub pickpocket_loot_id: u32,
    pub skinning_loot_id: u32,
    pub gold_min: u32,
    pub gold_max: u32,
    pub spells: [u32; CREATURE_MAX_SPELLS],
    pub spell_list_id: u32,
    pub pet_spell_list_id: u32,
    pub spawn_spell_id: u32,
    /// Parsed `auras` list (converted at load; see module docs).
    pub auras: Vec<u32>,
    pub ai_name: String,
    pub movement_type: u32,
    pub inhabit_type: u32,
    pub civilian: bool,
    pub racial_leader: bool,
    pub equipment_id: u32,
    pub trainer_id: u32,
    pub vendor_id: u32,
    pub mechanic_immune_mask: u32,
    pub school_immune_mask: u32,
    pub immunity_flags: u32,
    pub static_flags1: u32,
    pub static_flags2: u32,
    pub flags_extra: u32,
    /// Raw `script_name`; `script_id` resolves when the script
    /// registry lands (module docs).
    pub script_name: String,
    pub script_id: u32,
}

impl Default for CreatureInfo {
    fn default() -> Self {
        Self {
            entry: 0,
            name: String::new(),
            subname: String::new(),
            level_min: 1,
            level_max: 1,
            faction: 0,
            npc_flags: 0,
            gossip_menu_id: 0,
            display_id: [0; MAX_DISPLAY_IDS_PER_CREATURE],
            display_scale: [0.0; MAX_DISPLAY_IDS_PER_CREATURE],
            display_probability: [0; MAX_DISPLAY_IDS_PER_CREATURE],
            display_total_probability: 0,
            mount_display_id: 0,
            speed_walk: 1.0,
            speed_run: 1.14286,
            detection_range: 18.0,
            call_for_help_range: 5.0,
            leash_range: 0.0,
            creature_type: 0,
            pet_family: 0,
            rank: 0,
            unit_class: 0,
            xp_multiplier: 1.0,
            health_multiplier: 1.0,
            mana_multiplier: 1.0,
            armor_multiplier: 1.0,
            damage_multiplier: 1.0,
            damage_variance: 0.14,
            damage_school: 0,
            base_attack_time: 2000,
            ranged_attack_time: 2000,
            holy_res: 0,
            fire_res: 0,
            nature_res: 0,
            frost_res: 0,
            shadow_res: 0,
            arcane_res: 0,
            trainer_type: 0,
            trainer_spell: 0,
            trainer_class: 0,
            trainer_race: 0,
            loot_id: 0,
            pickpocket_loot_id: 0,
            skinning_loot_id: 0,
            gold_min: 0,
            gold_max: 0,
            spells: [0; CREATURE_MAX_SPELLS],
            spell_list_id: 0,
            pet_spell_list_id: 0,
            spawn_spell_id: 0,
            auras: Vec::new(),
            ai_name: String::new(),
            movement_type: 0,
            inhabit_type: 3,
            civilian: false,
            racial_leader: false,
            equipment_id: 0,
            trainer_id: 0,
            vendor_id: 0,
            mechanic_immune_mask: 0,
            school_immune_mask: 0,
            immunity_flags: 0,
            static_flags1: 0,
            static_flags2: 0,
            flags_extra: 0,
            script_name: String::new(),
            script_id: 0,
        }
    }
}

impl CreatureInfo {
    /// `IsTameable` (`CreatureDefines.h:311`).
    #[must_use]
    pub fn is_tameable(&self) -> bool {
        self.creature_type == CREATURE_TYPE_BEAST
            && self.pet_family != 0
            && self.static_flags1 & STATIC_FLAG_TAMEABLE != 0
    }

    /// `GetTypeFlags` (`CreatureDefines.h:313-331`): the client
    /// `UNIT_FIELD_BYTES_2` type-flags word (TOP byte offsets 0–6
    /// per the header comments are pre-shifted here as bit values).
    #[must_use]
    pub fn type_flags(&self) -> u32 {
        let mut flags = 0;
        if self.static_flags1 & STATIC_FLAG_TAMEABLE != 0 {
            flags |= type_flags::TAMEABLE;
        }
        if self.static_flags1 & STATIC_FLAG_VISIBLE_TO_GHOSTS != 0 {
            flags |= type_flags::VISIBLE_TO_GHOSTS;
        }
        if self.static_flags1 & STATIC_FLAG_RAID_BOSS_MOB != 0 {
            flags |= type_flags::RAID_BOSS_MOB;
        }
        if self.static_flags1 & STATIC_FLAG_DO_NOT_PLAY_WOUND_ANIM != 0 {
            flags |= type_flags::DO_NOT_PLAY_WOUND_ANIM;
        }
        if self.static_flags1 & STATIC_FLAG_NO_FACTION_TOOLTIP != 0 {
            flags |= type_flags::NO_FACTION_TOOLTIP;
        }
        if self.static_flags1 & STATIC_FLAG_MORE_AUDIBLE != 0 {
            flags |= type_flags::MORE_AUDIBLE;
        }
        if self.static_flags2 & STATIC_FLAG2_NO_HARMFUL_VERTEX_COLORING != 0 {
            flags |= type_flags::NO_HARMFUL_VERTEX_COLORING;
        }
        flags
    }

    /// Despawn-instantly static flag (loot warnings in
    /// `CheckCreatureTemplate` key off it).
    #[must_use]
    pub fn despawn_instantly(&self) -> bool {
        self.static_flags1 & STATIC_FLAG_DESPAWN_INSTANTLY != 0
    }
}

/// DBC/other-store id sets for the `CheckCreatureTemplate`
/// fix-ups that consult them. `None` = check skipped (and
/// reported once by the loader).
#[derive(Clone, Copy, Debug, Default)]
pub struct TemplateValidation<'a> {
    pub faction_template_ids: Option<&'a HashSet<u32>>,
    pub creature_display_ids: Option<&'a HashSet<u32>>,
    pub display_addon_ids: Option<&'a HashSet<u32>>,
    pub creature_type_ids: Option<&'a HashSet<u32>>,
    pub creature_family_ids: Option<&'a HashSet<u32>>,
    pub spell_ids: Option<&'a HashSet<u32>>,
    /// Spells that both exist and have `SPELL_EFFECT_SPAWN`.
    pub spawn_spell_ids: Option<&'a HashSet<u32>>,
    pub pet_spell_list_ids: Option<&'a HashSet<u32>>,
    pub creature_spell_list_ids: Option<&'a HashSet<u32>>,
    pub equipment_ids: Option<&'a HashSet<u32>>,
}

/// Loaded creature templates by entry (`m_creatureInfoMap`).
#[derive(Clone, Debug, Default)]
pub struct CreatureTemplates {
    by_entry: HashMap<u32, CreatureInfo>,
}

impl CreatureTemplates {
    /// `sObjectMgr.GetCreatureTemplate(entry)`.
    #[must_use]
    pub fn get(&self, entry: u32) -> Option<&CreatureInfo> {
        self.by_entry.get(&entry)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &CreatureInfo)> {
        self.by_entry.iter()
    }
}

/// Parse the space-separated `auras` field
/// (`ConvertCreatureAurasField<CreatureInfo>`, ObjectMgr.cpp:1350-1408):
/// `atoi` tokens, unknown spells dropped when `spell_ids` is
/// known, duplicates dropped, order preserved.
fn convert_auras(
    raw: &str,
    entry: u32,
    spell_ids: Option<&HashSet<u32>>,
    report: &mut LoadReport,
) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for tok in raw.split(' ') {
        if tok.is_empty() {
            continue;
        }
        let id = c_atoi(tok);
        if let Some(spells) = spell_ids {
            if !spells.contains(&id) {
                report.error(format!(
                    "Creature (Entry: {entry}) has wrong spell {id} defined in `auras` field in `creature_template`."
                ));
                continue;
            }
        }
        if out.contains(&id) {
            report.error(format!(
                "Creature (Entry: {entry}) has duplicate spell {id} defined in `auras` field in `creature_template`."
            ));
            continue;
        }
        out.push(id);
    }
    out
}

/// C `atoi` into `uint32` (sign + digit prefix, wrap).
fn c_atoi(tok: &str) -> u32 {
    let bytes = tok.as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let mut mag: u32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        mag = mag
            .wrapping_mul(10)
            .wrapping_add(u32::from(bytes[i] - b'0'));
        i += 1;
    }
    if neg { mag.wrapping_neg() } else { mag }
}

/// `CorrectCreatureDisplayIds` (`ObjectMgr.cpp:1306-1347`), the
/// three patch-gated remaps compiled in at build 5875.
fn correct_display_id(entry: u32, display_id: &mut u32, wow_patch: u32) {
    // enum WowPatch (Progression.h.in): 102=0, 106=4, 107=5.
    if wow_patch == 0 {
        match (entry, *display_id) {
            (644 | 1180, 1124) => *display_id = 14403,
            (5974, 11541) => *display_id = 14402,
            (5977, 11540) => *display_id = 14401,
            _ => {}
        }
    }
    if wow_patch == 4 {
        match (entry, *display_id) {
            (347, 15092) => *display_id = 15099,
            (14981, 15093) => *display_id = 15098,
            (14982, 15094) => *display_id = 15100,
            _ => {}
        }
    }
    if wow_patch == 5 && (entry, *display_id) == (15103, 15261) {
        *display_id = 15388;
    }
}

/// Display-id arm of `CheckCreatureTemplate` (returns the probability sum).
fn check_display_ids(
    info: &mut CreatureInfo,
    wow_patch: u32,
    v: &TemplateValidation<'_>,
    report: &mut LoadReport,
) -> u32 {
    let e = info.entry;
    let mut display_sum = 0u32;
    for i in 0..MAX_DISPLAY_IDS_PER_CREATURE {
        display_sum += info.display_probability[i];
        correct_display_id(e, &mut info.display_id[i], wow_patch);
        if info.display_id[i] != 0 {
            if info.display_probability[i] == 0 && info.display_total_probability != 0 {
                info.display_id[i] = 0;
                continue;
            }
            if let Some(set) = v.creature_display_ids {
                if !set.contains(&info.display_id[i]) {
                    report.error(format!(
                        "Creature (Entry: {e}) has nonexistent display_id{} ({}), can crash client",
                        i + 1,
                        info.display_id[i]
                    ));
                    info.display_id[i] = 0;
                    continue;
                }
            }
            if let Some(set) = v.display_addon_ids {
                if !set.contains(&info.display_id[i]) {
                    report.error(format!(
                        "Creature (Entry: {e}) is using display_id{} ({}), but creature_display_info_addon data is missing for this id.",
                        i + 1,
                        info.display_id[i]
                    ));
                }
            }
        }
    }
    display_sum
}

/// `CheckCreatureTemplate` (`ObjectMgr.cpp:1410-1620`): applies
/// the fix-ups in C++ order. Log-only checks are reported;
/// mutating fixes apply exactly as in C++ (the suggested
/// `UPDATE` SQL in C++ logs is not reproduced).
fn check_creature_template(
    info: &mut CreatureInfo,
    raw_auras: &str,
    wow_patch: u32,
    v: &TemplateValidation<'_>,
    report: &mut LoadReport,
) {
    let e = info.entry;
    if let Some(set) = v.faction_template_ids {
        if !set.contains(&info.faction) {
            report.error(format!(
                "Creature (Entry: {e}) has nonexistent faction template ({})",
                info.faction
            ));
        }
    }

    let display_sum = check_display_ids(info, wow_patch, v, report);
    if info.mount_display_id != 0 {
        if let Some(set) = v.creature_display_ids {
            if !set.contains(&info.mount_display_id) {
                report.error(format!(
                    "Creature (Entry: {e}) has nonexistent mount_display_id ({}), can crash client",
                    info.mount_display_id
                ));
                info.mount_display_id = 0;
            }
        }
    }
    if display_sum != info.display_total_probability {
        report.error(format!(
            "Creature (Entry: {e}) has a total display probability ({}) that does not match sum of individual probabilities ({display_sum}).",
            info.display_total_probability
        ));
    }

    if info.unit_class == 0 {
        report.error(format!(
            "Creature (Entry: {e}) does not have proper unit_class({}) for creature_template",
            info.unit_class
        ));
    } else if (1u32 << (info.unit_class - 1)) & CLASSMASK_ALL_CREATURES == 0 {
        report.error(format!(
            "Creature (Entry: {e}) has invalid unit_class({}) for creature_template",
            info.unit_class
        ));
    }

    if info.damage_school >= MAX_SPELL_SCHOOL {
        report.error(format!(
            "Creature (Entry: {e}) has invalid spell school value ({}) in `damage_school`",
            info.damage_school
        ));
        info.damage_school = 0; // SPELL_SCHOOL_NORMAL
    }
    if info.base_attack_time == 0 {
        info.base_attack_time = BASE_ATTACK_TIME;
    }
    if info.ranged_attack_time == 0 {
        info.ranged_attack_time = BASE_ATTACK_TIME;
    }
    if info.npc_flags & UNIT_NPC_FLAG_TRAINER != 0 && info.trainer_type >= MAX_TRAINER_TYPE {
        report.error(format!(
            "Creature (Entry: {e}) has wrong trainer type {}",
            info.trainer_type
        ));
    }
    if info.creature_type != 0 {
        if let Some(set) = v.creature_type_ids {
            if !set.contains(&info.creature_type) {
                report.error(format!(
                    "Creature (Entry: {e}) has invalid creature type ({}) in `type`",
                    info.creature_type
                ));
                info.creature_type = CREATURE_TYPE_HUMANOID;
            }
        }
    }
    if info.pet_family != 0 && info.pet_family != CREATURE_FAMILY_HORSE_CUSTOM {
        if let Some(set) = v.creature_family_ids {
            if !set.contains(&info.pet_family) {
                report.error(format!(
                    "Creature (Entry: {e}) has invalid creature family ({}) in `pet_family`",
                    info.pet_family
                ));
                info.pet_family = 0;
            }
        }
    }
    if info.inhabit_type == 0 || info.inhabit_type > INHABIT_ANYWHERE {
        report.error(format!(
            "Creature (Entry: {e}) has wrong value ({}) in `inhabit_type`, creature will not correctly walk/swim",
            info.inhabit_type
        ));
        info.inhabit_type = INHABIT_ANYWHERE;
    }
    check_creature_template_refs(info, raw_auras, v, report);
}

fn check_creature_template_ranges(
    info: &mut CreatureInfo,
    raw_auras: &str,
    v: &TemplateValidation<'_>,
    report: &mut LoadReport,
) {
    let e = info.entry;
    if info.xp_multiplier < 0.0 {
        report.error(format!(
            "Creature (Entry: {e}) has negative experience multiplier ({})",
            info.xp_multiplier
        ));
        info.xp_multiplier = 1.0;
    }
    if info.detection_range < 0.0 {
        report.error(format!(
            "Creature (Entry: {e}) has negative detection distance ({})",
            info.detection_range
        ));
        info.detection_range = 20.0;
    }
    if info.call_for_help_range < 0.0 {
        report.error(format!(
            "Creature (Entry: {e}) has negative call for help radius ({})",
            info.call_for_help_range
        ));
        info.call_for_help_range = 5.0;
    }
    if info.leash_range != 0.0 {
        if info.leash_range < 0.0 {
            report.error(format!(
                "Creature (Entry: {e}) has negative leash distance ({})",
                info.leash_range
            ));
            info.leash_range = 0.0;
        } else if info.leash_range < info.detection_range {
            report.error(format!(
                "Creature (Entry: {e}) has leash distance below detection distance ({})",
                info.leash_range
            ));
            info.leash_range = 0.0;
        }
    }
    if info.despawn_instantly() {
        if info.gold_min != 0 || info.gold_max != 0 {
            report.error(format!(
                "Creature (Entry: {e}) with despawn instantly flag has gold loot assigned. It will never be lootable."
            ));
        }
        if info.loot_id != 0 {
            report.error(format!(
                "Creature (Entry: {e}) with despawn instantly flag has corpse loot assigned. It will never be lootable."
            ));
        }
        if info.skinning_loot_id != 0 {
            report.error(format!(
                "Creature (Entry: {e}) with despawn instantly flag has skinning loot assigned. It will never be lootable."
            ));
        }
    }
    if info.level_min > info.level_max {
        std::mem::swap(&mut info.level_min, &mut info.level_max);
        report.error(format!(
            "Creature (Entry: {e}) has level_min ({}) greater than level_max ({}), values have been swapped.",
            info.level_max, info.level_min
        ));
    }

    info.auras = convert_auras(raw_auras, e, v.spell_ids, report);
}

fn check_creature_template_refs(
    info: &mut CreatureInfo,
    raw_auras: &str,
    v: &TemplateValidation<'_>,
    report: &mut LoadReport,
) {
    let e = info.entry;
    if info.pet_spell_list_id != 0 {
        if let Some(set) = v.pet_spell_list_ids {
            if !set.contains(&info.pet_spell_list_id) {
                report.error(format!(
                    "Creature (Entry: {e}) has nonexistent pet_spell_list_id ({})",
                    info.pet_spell_list_id
                ));
            }
        }
    }
    if info.spell_list_id != 0 {
        if let Some(set) = v.creature_spell_list_ids {
            if !set.contains(&info.spell_list_id) {
                report.error(format!(
                    "Creature (Entry: {e}) has nonexistent spell list id ({})",
                    info.spell_list_id
                ));
                info.spell_list_id = 0;
            }
        }
    }
    if info.spawn_spell_id != 0 {
        if let Some(set) = v.spawn_spell_ids {
            if !set.contains(&info.spawn_spell_id) {
                report.error(format!(
                    "Creature (Entry: {e}) has invalid spawn_spell_id ({}), set to 0",
                    info.spawn_spell_id
                ));
                info.spawn_spell_id = 0;
            }
        }
    }
    for (j, spell) in info.spells.iter_mut().enumerate() {
        if *spell != 0 {
            if let Some(set) = v.spell_ids {
                if !set.contains(spell) {
                    report.error(format!(
                        "Creature (Entry: {e}) has nonexistent spell_id{} ({spell}), set to 0",
                        j + 1
                    ));
                    *spell = 0;
                }
            }
        }
    }
    if info.movement_type >= MAX_DB_MOTION_TYPE {
        report.error(format!(
            "Creature (Entry: {e}) has wrong movement generator type ({}), ignored and set to IDLE.",
            info.movement_type
        ));
        info.movement_type = 0; // IDLE_MOTION_TYPE
    }
    if info.equipment_id > 0 {
        if let Some(set) = v.equipment_ids {
            if !set.contains(&info.equipment_id) {
                report.error(format!(
                    "Table `creature_template` have creature (Entry: {e}) with equipment_id {} not found in table `creature_equip_template`, set to no equipment.",
                    info.equipment_id
                ));
                info.equipment_id = 0;
            }
        }
    }
    if info.vendor_id > 0 && info.npc_flags & UNIT_NPC_FLAG_VENDOR == 0 {
        report.error(format!(
            "Table `creature_template` have creature (Entry: {e}) with vendor_id {} but not have flag UNIT_NPC_FLAG_VENDOR ({UNIT_NPC_FLAG_VENDOR}), vendor items will ignored.",
            info.vendor_id
        ));
    }
    check_creature_template_ranges(info, raw_auras, v, report);
}

const SELECT_COLUMNS: &str = "`entry`, `name`, `subname`, `level_min`, `level_max`, `faction`, \
    `npc_flags`, `gossip_menu_id`, `display_id1`, `display_id2`, `display_id3`, `display_id4`, \
    `display_scale1`, `display_scale2`, `display_scale3`, `display_scale4`, \
    `display_probability1`, `display_probability2`, `display_probability3`, \
    `display_probability4`, `display_total_probability`, `mount_display_id`, `speed_walk`, \
    `speed_run`, `detection_range`, `call_for_help_range`, `leash_range`, `type`, `pet_family`, \
    `rank`, `unit_class`, `xp_multiplier`, `health_multiplier`, `mana_multiplier`, \
    `armor_multiplier`, `damage_multiplier`, `damage_variance`, `damage_school`, \
    `base_attack_time`, `ranged_attack_time`, `holy_res`, `fire_res`, `nature_res`, \
    `frost_res`, `shadow_res`, `arcane_res`, `trainer_type`, `trainer_spell`, `trainer_class`, \
    `trainer_race`, `loot_id`, `pickpocket_loot_id`, `skinning_loot_id`, `gold_min`, \
    `gold_max`, `spell_id1`, `spell_id2`, `spell_id3`, `spell_id4`, `spell_list_id`, \
    `pet_spell_list_id`, `spawn_spell_id`, `auras`, `ai_name`, `movement_type`, \
    `inhabit_type`, `civilian`, `racial_leader`, `equipment_id`, `trainer_id`, `vendor_id`, \
    `mechanic_immune_mask`, `school_immune_mask`, `immunity_flags`, `static_flags1`, \
    `static_flags2`, `flags_extra`, `script_name`";

/// `LoadCreatureInfo` (`ObjectMgr.cpp:1217-1304`) for one row:
/// fills `info` positionally (merged over existing content, as
/// C++ reload does), then runs the template checks.
fn load_creature_info(
    f: &mangos_db::row::Fields,
    wow_patch: u32,
    v: &TemplateValidation<'_>,
    report: &mut LoadReport,
) -> CreatureInfo {
    let mut display_id = [0; MAX_DISPLAY_IDS_PER_CREATURE];
    let mut display_scale = [0.0; MAX_DISPLAY_IDS_PER_CREATURE];
    let mut display_probability = [0; MAX_DISPLAY_IDS_PER_CREATURE];
    for i in 0..MAX_DISPLAY_IDS_PER_CREATURE {
        display_id[i] = f.u32(8 + i);
        display_scale[i] = f.f32(12 + i);
        display_probability[i] = f.u32(16 + i);
    }
    let mut spells = [0; CREATURE_MAX_SPELLS];
    for (i, slot) in spells.iter_mut().enumerate() {
        *slot = f.u32(55 + i);
    }
    let raw_auras = f.string(62);
    let mut info = CreatureInfo {
        entry: f.u32(0),
        name: f.string(1),
        subname: f.string(2),
        level_min: f.u32(3),
        level_max: f.u32(4),
        faction: f.u32(5),
        npc_flags: f.u32(6),
        gossip_menu_id: f.u32(7),
        display_id,
        display_scale,
        display_probability,
        display_total_probability: f.u32(20),
        mount_display_id: f.u32(21),
        speed_walk: f.f32(22),
        speed_run: f.f32(23),
        detection_range: f.f32(24),
        call_for_help_range: f.f32(25),
        leash_range: f.f32(26),
        creature_type: f.u32(27),
        pet_family: f.u32(28),
        rank: f.u32(29),
        unit_class: f.u32(30),
        xp_multiplier: f.f32(31),
        health_multiplier: f.f32(32),
        mana_multiplier: f.f32(33),
        armor_multiplier: f.f32(34),
        damage_multiplier: f.f32(35),
        damage_variance: f.f32(36),
        damage_school: f.u32(37),
        base_attack_time: f.u32(38),
        ranged_attack_time: f.u32(39),
        holy_res: f.i32(40),
        fire_res: f.i32(41),
        nature_res: f.i32(42),
        frost_res: f.i32(43),
        shadow_res: f.i32(44),
        arcane_res: f.i32(45),
        trainer_type: f.u32(46),
        trainer_spell: f.u32(47),
        trainer_class: f.u32(48),
        trainer_race: f.u32(49),
        loot_id: f.u32(50),
        pickpocket_loot_id: f.u32(51),
        skinning_loot_id: f.u32(52),
        gold_min: f.u32(53),
        gold_max: f.u32(54),
        spells,
        spell_list_id: f.u32(59),
        pet_spell_list_id: f.u32(60),
        spawn_spell_id: f.u32(61),
        ai_name: f.string(63),
        movement_type: f.u32(64),
        inhabit_type: f.u32(65),
        civilian: f.u32(66) != 0,
        racial_leader: f.u32(67) != 0,
        equipment_id: f.u32(68),
        trainer_id: f.u32(69),
        vendor_id: f.u32(70),
        mechanic_immune_mask: f.u32(71),
        school_immune_mask: f.u32(72),
        immunity_flags: f.u32(73),
        static_flags1: f.u32(74),
        static_flags2: f.u32(75),
        flags_extra: f.u32(76),
        script_name: f.string(77),
        ..CreatureInfo::default()
    };
    // script_id stays 0 until the script registry lands.
    check_creature_template(&mut info, &raw_auras, wow_patch, v, report);
    info
}

/// Report each skipped DBC-dependent check once per load.
fn note_skipped_checks(v: &TemplateValidation<'_>, report: &mut LoadReport) {
    let skipped: &[(&str, bool)] = &[
        ("faction template", v.faction_template_ids.is_none()),
        ("creature display (DBC)", v.creature_display_ids.is_none()),
        ("display addon", v.display_addon_ids.is_none()),
        ("creature type (DBC)", v.creature_type_ids.is_none()),
        ("creature family (DBC)", v.creature_family_ids.is_none()),
        ("spell existence", v.spell_ids.is_none()),
        ("spawn-spell effect", v.spawn_spell_ids.is_none()),
        ("pet spell list", v.pet_spell_list_ids.is_none()),
        ("creature spell list", v.creature_spell_list_ids.is_none()),
        ("equipment template", v.equipment_ids.is_none()),
    ];
    for (name, is_skipped) in skipped {
        if *is_skipped {
            report.error(format!(
                "creature_template load: {name} validation skipped (no id set supplied)"
            ));
        }
    }
}

/// `LoadCreatureTemplates` (`ObjectMgr.cpp:1173-1193`) at the
/// given content patch.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_creature_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    validation: &TemplateValidation<'_>,
) -> Result<(CreatureTemplates, LoadReport), DbError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM `creature_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `creature_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, (wow_patch,))?;
    let mut report = LoadReport::default();
    note_skipped_checks(validation, &mut report);
    let mut by_entry = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let info = load_creature_info(&f, wow_patch, validation, &mut report);
        // C++ reuses the map slot and overwrites every field: last row wins.
        by_entry.insert(info.entry, info);
    }
    Ok((CreatureTemplates { by_entry }, report))
}

/// `LoadCreatureTemplate(entry)` (`ObjectMgr.cpp:1195-1215`)
/// single-entry form.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_creature_template(
    conn: &mut mysql::Conn,
    entry: u32,
    wow_patch: u32,
    validation: &TemplateValidation<'_>,
) -> Result<(Option<CreatureInfo>, LoadReport), DbError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM `creature_template` t1 \
         WHERE `entry` = ? AND `patch` = (SELECT max(`patch`) FROM `creature_template` t2 \
                                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, (entry, wow_patch))?;
    let mut report = LoadReport::default();
    note_skipped_checks(validation, &mut report);
    let info = rows.into_iter().next().map(|row| {
        load_creature_info(
            &mangos_db::row::Fields::from_row(row),
            wow_patch,
            validation,
            &mut report,
        )
    });
    Ok((info, report))
}
