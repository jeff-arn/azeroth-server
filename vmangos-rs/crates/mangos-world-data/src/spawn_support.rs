//! Spawn-support stores: creature addons, equipment templates,
//! class/level stats (CLS), and display-info addons, ported from
//! `ObjectMgr.cpp:1625-2273,7827` neighbours and the
//! `SQLStorage` progressive loaders (`SQLStorageImpl.h`:
//! `SELECT *` with `patch = max(patch <= WowPatch)` on column 1,
//! or `build <= SUPPORTED_CLIENT_BUILD` for the display addon;
//! supported build here is 5875).
//!
//! Notes:
//!
//! - There is no `creature_template_addon` table/storage at this
//!   build (the inventory's mention was inaccurate); addons are
//!   per-GUID only.
//! - Aura lists parse at load like the templates do; spell
//!   existence filtering needs the caller's spell id set.
//! - The equipment probability-0 flush quirk is replicated: a
//!   run of probability-0 rows after a nonzero row is silently
//!   dropped (`ObjectMgr.cpp:1735-1745`).

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::load_report::LoadReport;

/// `MAX_UNIT_STAND_STATE` (`UnitDefines.h:114`).
const MAX_UNIT_STAND_STATE: u8 = 10;
/// `SHEATH_STATE_RANGED` (`UnitDefines.h:172`).
const SHEATH_STATE_RANGED: u8 = 2;
/// `UNIT_STAND_STATE_STAND`.
const UNIT_STAND_STATE_STAND: u8 = 0;
/// `SHEATH_STATE_MELEE`.
const SHEATH_STATE_MELEE: u8 = 1;
/// `MAX_LEVEL` (`DBCEnums.h:34`).
const MAX_LEVEL: u32 = 100;
/// `CREATURE_MAX_LEVEL` (`DBCEnums.h:44`).
const CREATURE_MAX_LEVEL: u32 = 63;

// ---------------------------------------------------------------------------
// Creature addons

/// `struct CreatureDataAddon` (`CreatureDefines.h:361-368`).
#[derive(Clone, Debug, Default)]
pub struct CreatureDataAddon {
    pub guid: u32,
    pub display_id: u32,
    pub mount_display_id: u32,
    /// Byte words: -1 = use template equip, -2 = don't equip.
    pub equipment_id: i32,
    pub stand_state: u8,
    pub sheath_state: u8,
    pub emote_state: u32,
    pub auras: Vec<u32>,
}

/// Parse an `auras` string (`ConvertCreatureAurasField` for
/// addons): duplicates are **not** dropped for addons? They are:
/// the same template fn runs for both. Kept in one place in
/// [`crate::creature_template`] for templates; addons share the
/// semantics through this local copy of the loop.
fn convert_addon_auras(
    raw: &str,
    table: &str,
    guid: u32,
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
                    "Creature (GUID: {guid}) has wrong spell {id} defined in `auras` field in `{table}`."
                ));
                continue;
            }
        }
        if out.contains(&id) {
            report.error(format!(
                "Creature (GUID: {guid}) has duplicate spell {id} defined in `auras` field in `{table}`."
            ));
            continue;
        }
        out.push(id);
    }
    out
}

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

/// Loaded creature addons by DB guid (`sCreatureDataAddonStorage`).
#[derive(Clone, Debug, Default)]
pub struct CreatureAddons {
    by_guid: HashMap<u32, CreatureDataAddon>,
}

impl CreatureAddons {
    /// `ObjectMgr::GetCreatureAddon(lowguid)`.
    #[must_use]
    pub fn get(&self, guid: u32) -> Option<&CreatureDataAddon> {
        self.by_guid.get(&guid)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }
}

/// `LoadCreatureAddons` (`ObjectMgr.cpp:1630-1695`, the
/// `creature_addon` instantiation). `creature_guids` (existing
/// spawn guids) and `emote_ids` gate the existence/emote checks;
/// `spell_ids` gates aura filtering.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)] // callers use default sets like the other loaders
pub fn load_creature_addons(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    creature_guids: Option<&HashSet<u32>>,
    emote_ids: Option<&HashSet<u32>>,
    spell_ids: Option<&HashSet<u32>>,
) -> Result<(CreatureAddons, LoadReport), DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `guid`, `display_id`, `mount_display_id`, `equipment_id`, `stand_state`, \
         `sheath_state`, `emote_state`, `auras` FROM `creature_addon` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `creature_addon` t2 \
                          WHERE t1.`guid` = t2.`guid` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let mut report = LoadReport::default();
    if creature_guids.is_none() {
        report.error(
            "creature_addon load: spawn-guid existence validation skipped (no guid set supplied)"
                .to_string(),
        );
    }
    if emote_ids.is_none() {
        report.error(
            "creature_addon load: emote validation skipped (no id set supplied)".to_string(),
        );
    }
    let mut by_guid = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let guid = f.u32(0);
        let mut addon = CreatureDataAddon {
            guid,
            display_id: f.u32(1),
            mount_display_id: f.u32(2),
            equipment_id: f.i32(3),
            stand_state: f.u8(4),
            sheath_state: f.u8(5),
            emote_state: f.u32(6),
            auras: Vec::new(),
        };
        if addon.stand_state >= MAX_UNIT_STAND_STATE {
            report.error(format!(
                "Creature (GUID {guid}) has unknown stand state ({}) defined in `creature_addon`.",
                addon.stand_state
            ));
            addon.stand_state = UNIT_STAND_STATE_STAND;
        }
        if addon.sheath_state > SHEATH_STATE_RANGED {
            report.error(format!(
                "Creature (GUID {guid}) has unknown sheath state ({}) defined in `creature_addon`.",
                addon.sheath_state
            ));
            addon.sheath_state = SHEATH_STATE_MELEE;
        }
        if let Some(set) = emote_ids {
            if !set.contains(&addon.emote_state) {
                report.error(format!(
                    "Creature (GUID {guid}) have invalid emote ({}) defined in `creature_addon`.",
                    addon.emote_state
                ));
                addon.emote_state = 0;
            }
        }
        addon.auras =
            convert_addon_auras(&f.string(7), "creature_addon", guid, spell_ids, &mut report);
        by_guid.insert(guid, addon);
    }
    // Trailing existence sweep (ObjectMgr.cpp:1688-1694).
    if let Some(guids) = creature_guids {
        let mut addon_guids: Vec<u32> = by_guid.keys().copied().collect();
        addon_guids.sort_unstable();
        for guid in addon_guids {
            if !guids.contains(&guid) {
                report.error(format!(
                    "Creature (GUID: {guid}) does not exist but has a record in `creature_addon`"
                ));
            }
        }
    }
    Ok((CreatureAddons { by_guid }, report))
}

// ---------------------------------------------------------------------------
// Equipment templates

/// `struct EquipmentEntry` (`CreatureDefines.h:334-338`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EquipmentEntry {
    pub probability: u32,
    pub item: [u32; 3],
}

/// `struct EquipmentTemplate` (`CreatureDefines.h:340-…`).
#[derive(Clone, Debug, Default)]
pub struct EquipmentTemplate {
    pub total_probability: u32,
    pub entries: Vec<EquipmentEntry>,
}

/// Item existence/equipability input: `item id -> inventory type`.
const EQUIPABLE_INVENTORY_TYPES: [u32; 10] = [13, 14, 15, 17, 21, 22, 23, 25, 26, 28];

/// Loaded equipment templates (`m_CreatureEquipmentMap`).
#[derive(Clone, Debug, Default)]
pub struct EquipmentTemplates {
    by_entry: HashMap<u32, EquipmentTemplate>,
}

impl EquipmentTemplates {
    /// `GetEquipmentTemplate(entry)`.
    #[must_use]
    pub fn get(&self, entry: u32) -> Option<&EquipmentTemplate> {
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

    /// Total equipment rows across all templates (after
    /// probability-0 flushes).
    #[must_use]
    pub fn total_entry_rows(&self) -> usize {
        self.by_entry.values().map(|t| t.entries.len()).sum()
    }
}

/// `LoadEquipmentTemplates` (`ObjectMgr.cpp:1697-1777`), rows in
/// `(entry, probability DESC)` order. Item validation zeroes
/// unknown/non-equipable slots when `item_inventory_types` is
/// supplied.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)] // callers use default sets like the other loaders
pub fn load_equipment_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    item_inventory_types: Option<&HashMap<u32, u32>>,
) -> Result<(EquipmentTemplates, LoadReport), DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `probability`, `item1`, `item2`, `item3` \
         FROM `creature_equip_template` WHERE ? BETWEEN `patch_min` AND `patch_max` \
         ORDER BY `entry`, `probability` DESC",
        (wow_patch,),
    )?;
    let mut report = LoadReport::default();
    if item_inventory_types.is_none() {
        report.error(
            "creature_equip_template load: item validation skipped (no inventory types supplied)"
                .to_string(),
        );
    }
    let mut by_entry: HashMap<u32, EquipmentTemplate> = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let (entry, probability) = (f.u32(0), f.u32(1));
        let item = [f.u32(2), f.u32(3), f.u32(4)];
        if item == [0, 0, 0] {
            report.error(format!(
                "`creature_equip_template` template equipment entry={entry} probability={probability} with 0 items equipped."
            ));
            continue;
        }
        if probability == 0 {
            // Flush-previous semantics: clear accumulated entries
            // for this template, then skip the row itself.
            if let Some(tpl) = by_entry.get_mut(&entry) {
                if !tpl.entries.is_empty() {
                    report.error(format!(
                        "`creature_equip_template` template equipment entry={entry} generated equipment set after entry with probability=0."
                    ));
                    tpl.entries.clear();
                    tpl.total_probability = 0;
                }
            }
            continue;
        }
        let tpl = by_entry.entry(entry).or_default();
        tpl.total_probability += probability;
        tpl.entries.push(EquipmentEntry { probability, item });
    }
    // Cross-check + item validation pass (ObjectMgr.cpp:1750-1770).
    let mut entries: Vec<u32> = by_entry.keys().copied().collect();
    entries.sort_unstable();
    for entry in entries {
        let Some(tpl) = by_entry.get_mut(&entry) else {
            continue;
        };
        if tpl.total_probability != 100 {
            report.error(format!(
                "`creature_equip_template` template equipment entry={entry} total probability is not 100% but {}%.",
                tpl.total_probability
            ));
        }
        if let Some(items) = item_inventory_types {
            for equip in &mut tpl.entries {
                for (j, slot) in equip.item.iter_mut().enumerate() {
                    if *slot == 0 {
                        continue;
                    }
                    match items.get(slot) {
                        None => {
                            report.error(format!(
                                "Unknown item (entry={}) in creature_equip_template.item{} for entry = {entry}, forced to 0.",
                                *slot,
                                j + 1
                            ));
                            *slot = 0;
                        }
                        Some(inv) if !EQUIPABLE_INVENTORY_TYPES.contains(inv) => {
                            report.error(format!(
                                "Item (entry={}) in creature_equip_template.item{} for entry = {entry} is not equipable in a hand, forced to 0.",
                                *slot,
                                j + 1
                            ));
                            *slot = 0;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok((EquipmentTemplates { by_entry }, report))
}

// ---------------------------------------------------------------------------
// Creature class/level stats (CLS)

/// `struct CreatureClassLevelStats` (`CreatureDefines.h:438-457`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CreatureClassLevelStats {
    pub melee_damage: f32,
    pub ranged_damage: f32,
    pub attack_power: i32,
    pub ranged_attack_power: i32,
    pub health: i32,
    pub base_health: i32,
    pub mana: i32,
    pub base_mana: i32,
    pub strength: i32,
    pub agility: i32,
    pub stamina: i32,
    pub intellect: i32,
    pub spirit: i32,
    pub armor: i32,
}

/// Placeholder returned for a missing class
/// (`ObjectMgr.cpp:2264`: `{1.5, 1.3, 20, 20, 40, 20, 40, 20,
/// 20, 20, 20, 20, 20, 20}`).
pub const CLS_PLACEHOLDER: CreatureClassLevelStats = CreatureClassLevelStats {
    melee_damage: 1.5,
    ranged_damage: 1.3,
    attack_power: 20,
    ranged_attack_power: 20,
    health: 40,
    base_health: 20,
    mana: 40,
    base_mana: 20,
    strength: 20,
    agility: 20,
    stamina: 20,
    intellect: 20,
    spirit: 20,
    armor: 20,
};

/// `InterpolateValueAtIndex` (`Util.h:412-413`), all-f32 math.
#[allow(clippy::cast_precision_loss)] // C++ promotes ints to float identically
fn interpolate(
    start_index: f32,
    start_value: f32,
    end_index: f32,
    end_value: f32,
    current: f32,
) -> f32 {
    start_value + (current - start_index) / (end_index - start_index) * (end_value - start_value)
}

/// Loaded CLS tables (`m_CreatureCLSMap`): class -> per-level
/// rows indexed by `level - 1`.
#[derive(Clone, Debug, Default)]
pub struct ClassLevelStats {
    by_class: HashMap<u32, Vec<CreatureClassLevelStats>>,
}

impl ClassLevelStats {
    /// `GetCreatureClassLevelStats(class, level)`: level clamps
    /// to the table size; a missing class returns
    /// [`CLS_PLACEHOLDER`]. C++ indexes `level - 1` directly
    /// (level 0 is UB there); level 0 clamps to 1 here.
    #[must_use]
    pub fn get(&self, unit_class: u32, level: u32) -> CreatureClassLevelStats {
        match self.by_class.get(&unit_class) {
            Some(rows) if !rows.is_empty() => {
                let level = level.clamp(1, rows.len() as u32);
                rows[(level - 1) as usize]
            }
            _ => CLS_PLACEHOLDER,
        }
    }

    /// Number of class tables loaded.
    #[must_use]
    pub fn class_count(&self) -> usize {
        self.by_class.len()
    }

    /// Table length for a class (max level covered).
    #[must_use]
    pub fn class_table_len(&self, unit_class: u32) -> usize {
        self.by_class.get(&unit_class).map_or(0, Vec::len)
    }
}

/// `LoadCreatureClassLevelStats` (`ObjectMgr.cpp:2043-2273`).
/// The C++ `exit(1)` on missing per-level health becomes report
/// errors here (a library must not exit); the boot layer
/// decides. Deviation recorded in `PHASE4-PROGRESS.md`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_class_level_stats(
    conn: &mut mysql::Conn,
) -> Result<(ClassLevelStats, LoadReport), DbError> {
    let mut report = LoadReport::default();
    let classes: Vec<mysql::Row> =
        conn.query("SELECT DISTINCT `unit_class` FROM `creature_template`")?;
    let mut unit_classes: Vec<u32> = Vec::new();
    for row in classes {
        let f = mangos_db::row::Fields::from_row(row);
        unit_classes.push(f.u32(0));
    }
    unit_classes.sort_unstable();
    let mut by_class: HashMap<u32, Vec<CreatureClassLevelStats>> = HashMap::new();
    for unit_class in unit_classes {
        let rows: Vec<mysql::Row> = conn.exec(
            "SELECT MAX(`level_max`) FROM `creature_template` WHERE `unit_class` = ?",
            (unit_class,),
        )?;
        let mut required_max = MAX_LEVEL;
        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            let max_level = f.u32(0);
            if max_level > required_max {
                required_max = max_level;
            }
        }
        let rows: Vec<mysql::Row> = conn.exec(
            "SELECT MAX(`level`) FROM `creature_classlevelstats` WHERE `class` = ?",
            (unit_class,),
        )?;
        let mut current_max = CREATURE_MAX_LEVEL;
        let mut table_len = 0usize;
        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            let max_level = f.u32(0);
            if max_level > current_max {
                current_max = max_level;
            } else if max_level == 0 {
                report.error(format!(
                    "Missing creature CLS data for `class` = {unit_class} used in creature_template!"
                ));
            }
            table_len = required_max.max(current_max) as usize;
        }
        let table = by_class.entry(unit_class).or_default();
        table.resize(table_len, CreatureClassLevelStats::default());
        let rows: Vec<mysql::Row> = conn.exec(
            "SELECT `level`, `melee_damage`, `ranged_damage`, `attack_power`, \
             `ranged_attack_power`, `health`, `base_health`, `mana`, `base_mana`, \
             `strength`, `agility`, `stamina`, `intellect`, `spirit`, `armor` \
             FROM `creature_classlevelstats` WHERE `class` = ? ORDER BY `level`",
            (unit_class,),
        )?;
        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            let level = f.u32(0);
            if level == 0 {
                report.error(
                    "Table `creature_classlevelstats` contains data for invalid `level` = 0!"
                        .to_string(),
                );
                continue;
            }
            // C++ writes `table[level - 1]` unchecked; a row
            // beyond MAX(level) cannot exist by construction.
            let idx = (level - 1) as usize;
            if idx >= table.len() {
                continue;
            }
            let cls = &mut table[idx];
            cls.melee_damage = f.f32(1);
            cls.ranged_damage = f.f32(2);
            cls.attack_power = f.i32(3);
            cls.ranged_attack_power = f.i32(4);
            cls.health = f.i32(5);
            cls.base_health = f.i32(6);
            cls.mana = f.i32(7);
            cls.base_mana = f.i32(8);
            cls.strength = f.i32(9);
            cls.agility = f.i32(10);
            cls.stamina = f.i32(11);
            cls.intellect = f.i32(12);
            cls.spirit = f.i32(13);
            cls.armor = f.i32(14);
            fix_cls(cls, unit_class, level, &mut report);
        }
        for i in 0..current_max as usize {
            if i < table.len() && table[i].health == 0 {
                report.error(format!(
                    "Missing creature CLS data for `class` = {unit_class} and `level` = {}!",
                    i + 1
                ));
            }
        }
        if current_max < required_max {
            fill_cls_tail(table, current_max, required_max);
        }
    }
    Ok((ClassLevelStats { by_class }, report))
}

/// Per-field clamps (`ObjectMgr.cpp:2131-2225`). The C++ log
/// lines print `melee_damage` for every field (a copy-paste
/// bug); messages here name the actual field.
#[allow(clippy::float_cmp)] // C++ compares exactly against 0.0f
fn fix_cls(
    cls: &mut CreatureClassLevelStats,
    unit_class: u32,
    level: u32,
    report: &mut LoadReport,
) {
    fn err<V: std::fmt::Display>(
        report: &mut LoadReport,
        name: &str,
        value: V,
        unit_class: u32,
        level: u32,
    ) {
        report.error(format!(
            "Invalid `{name}` = {value} in `creature_classlevelstats` for `class`={unit_class} and `level`={level}!"
        ));
    }
    if cls.melee_damage <= 0.0 {
        err(report, "melee_damage", cls.melee_damage, unit_class, level);
        cls.melee_damage = 1.0;
    }
    if cls.ranged_damage <= 0.0 {
        err(
            report,
            "ranged_damage",
            cls.ranged_damage,
            unit_class,
            level,
        );
        cls.ranged_damage = 1.0;
    }
    for (slot, name) in [
        (&mut cls.attack_power, "attack_power"),
        (&mut cls.ranged_attack_power, "ranged_attack_power"),
        (&mut cls.health, "health"),
        (&mut cls.base_health, "base_health"),
    ] {
        if *slot <= 0 {
            err(report, name, *slot, unit_class, level);
            *slot = 1;
        }
    }
    for (slot, name) in [(&mut cls.mana, "mana"), (&mut cls.base_mana, "base_mana")] {
        if *slot < 0 {
            err(report, name, *slot, unit_class, level);
            // C++ `abs(INT_MIN)` is UB; wrapping_abs matches in practice.
            *slot = slot.wrapping_abs();
        }
    }
    for (slot, name) in [
        (&mut cls.strength, "strength"),
        (&mut cls.agility, "agility"),
        (&mut cls.stamina, "stamina"),
        (&mut cls.intellect, "intellect"),
        (&mut cls.spirit, "spirit"),
    ] {
        if *slot <= 0 {
            err(report, name, *slot, unit_class, level);
            *slot = 1;
        }
    }
    if cls.armor < 0 {
        err(report, "armor", cls.armor, unit_class, level);
        cls.armor = cls.armor.wrapping_abs();
    }
}

/// Extrapolates levels `(current_max, required_max]` linearly
/// between level 1 and level `current_max`
/// (`ObjectMgr.cpp:2237-2267`); rows that already have health
/// are left alone.
fn fill_cls_tail(table: &mut [CreatureClassLevelStats], current_max: u32, required_max: u32) {
    #[allow(clippy::cast_precision_loss)]
    let (end_index, min, max) = (
        current_max as f32,
        table[0],
        table[(current_max - 1) as usize],
    );
    for i in current_max..required_max {
        let idx = i as usize;
        if idx >= table.len() || table[idx].health != 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let at = (i + 1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let lerp_i = |a: i32, b: i32| interpolate(1.0, a as f32, end_index, b as f32, at) as i32;
        let cls = &mut table[idx];
        cls.melee_damage = interpolate(1.0, min.melee_damage, end_index, max.melee_damage, at);
        cls.ranged_damage = interpolate(1.0, min.ranged_damage, end_index, max.ranged_damage, at);
        cls.attack_power = lerp_i(min.attack_power, max.attack_power);
        cls.ranged_attack_power = lerp_i(min.ranged_attack_power, max.ranged_attack_power);
        cls.health = lerp_i(min.health, max.health);
        cls.base_health = lerp_i(min.base_health, max.base_health);
        cls.mana = lerp_i(min.mana, max.mana);
        cls.base_mana = lerp_i(min.base_mana, max.base_mana);
        cls.strength = lerp_i(min.strength, max.strength);
        cls.agility = lerp_i(min.agility, max.agility);
        cls.stamina = lerp_i(min.stamina, max.stamina);
        cls.intellect = lerp_i(min.intellect, max.intellect);
        cls.spirit = lerp_i(min.spirit, max.spirit);
        cls.armor = lerp_i(min.armor.max(1), max.armor.max(1));
    }
}

// ---------------------------------------------------------------------------
// Display-info addons

/// `GENDER_NONE`.
const GENDER_NONE: u8 = 2;
/// `GENDER_MALE`.
const GENDER_MALE: u8 = 0;
/// `DEFAULT_NPC_WALK_SPEED_RATE` (`CreatureDefines.h:230`).
const DEFAULT_NPC_WALK_SPEED_RATE: f32 = 1.0;
/// `DEFAULT_NPC_RUN_SPEED_RATE` (`CreatureDefines.h:231`).
const DEFAULT_NPC_RUN_SPEED_RATE: f32 = 1.14286;

/// `struct CreatureDisplayInfoAddon` (`CreatureDefines.h:427-436`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CreatureDisplayInfoAddon {
    pub display_id: u32,
    pub bounding_radius: f32,
    pub combat_reach: f32,
    pub speed_walk: f32,
    pub speed_run: f32,
    pub gender: u8,
    pub display_id_other_gender: u32,
}

/// `struct GameObjectDisplayInfoAddon` (`GameObjectDefines.h:930-939`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameObjectDisplayInfoAddon {
    pub display_id: u32,
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub max_z: f32,
}

/// Display-info addons for both object kinds.
#[derive(Clone, Debug, Default)]
pub struct DisplayInfoAddons {
    creature: HashMap<u32, CreatureDisplayInfoAddon>,
    gameobject: HashMap<u32, GameObjectDisplayInfoAddon>,
}

impl DisplayInfoAddons {
    #[must_use]
    pub fn creature(&self, display_id: u32) -> Option<&CreatureDisplayInfoAddon> {
        self.creature.get(&display_id)
    }

    #[must_use]
    pub fn gameobject(&self, display_id: u32) -> Option<&GameObjectDisplayInfoAddon> {
        self.gameobject.get(&display_id)
    }

    #[must_use]
    pub fn creature_len(&self) -> usize {
        self.creature.len()
    }

    #[must_use]
    pub fn gameobject_len(&self) -> usize {
        self.gameobject.len()
    }
}

/// One `creature_display_info_addon` row with its fix-ups
/// (`ObjectMgr.cpp:1801-1846`).
#[allow(clippy::float_cmp)] // C++ compares exactly against 0.0f
fn parse_creature_display_addon(
    row: mysql::Row,
    display_ids: Option<&HashSet<u32>>,
    report: &mut LoadReport,
) -> CreatureDisplayInfoAddon {
    let f = mangos_db::row::Fields::from_row(row);
    let mut addon = CreatureDisplayInfoAddon {
        display_id: f.u32(0),
        bounding_radius: f.f32(1),
        combat_reach: f.f32(2),
        speed_walk: f.f32(3),
        speed_run: f.f32(4),
        gender: f.u8(5),
        display_id_other_gender: f.u32(6),
    };
    if let Some(set) = display_ids {
        if !set.contains(&addon.display_id) {
            report.error(format!(
                "Table `creature_display_info_addon` has data for nonexistent display id ({}).",
                addon.display_id
            ));
        }
    }
    if addon.gender > GENDER_NONE {
        report.error(format!(
            "Table `creature_display_info_addon` has invalid gender ({}) for display id ({}).",
            addon.gender, addon.display_id
        ));
        addon.gender = GENDER_MALE;
    }
    if addon.display_id_other_gender != 0 {
        if addon.display_id_other_gender == addon.display_id {
            report.error(format!(
                    "Table `creature_display_info_addon` has redundant display_id_other_gender ({}) defined for display id {}.",
                    addon.display_id_other_gender, addon.display_id
                ));
            addon.display_id_other_gender = 0;
        } else if let Some(set) = display_ids {
            if !set.contains(&addon.display_id_other_gender) {
                report.error(format!(
                        "Table `creature_display_info_addon` has nonexistent display_id_other_gender ({}) defined for display id {}.",
                        addon.display_id_other_gender, addon.display_id
                    ));
                addon.display_id_other_gender = 0;
            }
        }
    }
    if addon.speed_walk <= 0.0 {
        report.error(format!(
                "Table `creature_display_info_addon` has invalid walk speed ({}) defined for display id {}.",
                addon.speed_walk, addon.display_id
            ));
        addon.speed_walk = DEFAULT_NPC_WALK_SPEED_RATE;
    }
    if addon.speed_run <= 0.0 {
        report.error(format!(
                "Table `creature_display_info_addon` has invalid run speed ({}) defined for display id {}.",
                addon.speed_run, addon.display_id
            ));
        addon.speed_run = DEFAULT_NPC_RUN_SPEED_RATE;
    }
    addon
}

/// `LoadCreatureDisplayInfoAddon` (`ObjectMgr.cpp:1797-1903`,
/// progressive on `build`) plus `LoadGameObjectDisplayInfoAddon`
/// (`ObjectMgr.cpp:8090-8096`, plain load, no fix-ups).
/// `display_ids` (CreatureDisplayInfo.dbc ids) and
/// `playable_race_models` (`(race, model_f, model_m)` from
/// ChrRaces.dbc) gate the DBC cross-checks; skipped checks are
/// reported. The race-model check only logs in C++ except for
/// the two radius/reach fix-ups, which apply here.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::float_cmp, clippy::implicit_hasher)] // exact 0.0f compares; default sets
pub fn load_display_info_addons(
    conn: &mut mysql::Conn,
    supported_build: u32,
    display_ids: Option<&HashSet<u32>>,
    playable_race_models: Option<&[(u32, u32, u32)]>,
) -> Result<(DisplayInfoAddons, LoadReport), DbError> {
    let mut report = LoadReport::default();
    if display_ids.is_none() {
        report.error(
            "creature_display_info_addon load: DBC display-id validation skipped (no id set supplied)"
                .to_string(),
        );
    }
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `display_id`, `bounding_radius`, `combat_reach`, `speed_walk`, `speed_run`, \
         `gender`, `display_id_other_gender` FROM `creature_display_info_addon` t1 \
         WHERE `build` = (SELECT max(`build`) FROM `creature_display_info_addon` t2 \
                          WHERE t1.`display_id` = t2.`display_id` AND `build` <= ?)",
        (supported_build,),
    )?;
    let mut creature: HashMap<u32, CreatureDisplayInfoAddon> = HashMap::with_capacity(rows.len());
    for row in rows {
        let addon = parse_creature_display_addon(row, display_ids, &mut report);
        creature.insert(addon.display_id, addon);
    }
    // Playable-race model fix-ups (radius/reach only; the other
    // race checks are log-only in C++ and need ChrRaces data).
    if let Some(races) = playable_race_models {
        for &(_race, model_f, model_m) in races {
            for model in [model_f, model_m] {
                if let Some(addon) = creature.get_mut(&model) {
                    if addon.bounding_radius <= 0.0 {
                        // DEFAULT_WORLD_OBJECT_SIZE (ObjectDefines.h:42).
                        // (Same f32 bits as the C++ literal.)
                        addon.bounding_radius = 0.389;
                    }
                    if addon.combat_reach <= 0.0 {
                        addon.combat_reach = 1.5;
                    }
                }
            }
        }
    } else {
        report.error(
            "creature_display_info_addon load: playable-race model checks skipped (no ChrRaces models supplied)"
                .to_string(),
        );
    }
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `display_id`, `min_x`, `min_y`, `min_z`, `max_x`, `max_y`, `max_z` \
         FROM `gameobject_display_info_addon`",
    )?;
    let mut gameobject: HashMap<u32, GameObjectDisplayInfoAddon> =
        HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let addon = GameObjectDisplayInfoAddon {
            display_id: f.u32(0),
            min_x: f.f32(1),
            min_y: f.f32(2),
            min_z: f.f32(3),
            max_x: f.f32(4),
            max_y: f.f32(5),
            max_z: f.f32(6),
        };
        gameobject.insert(addon.display_id, addon);
    }
    Ok((
        DisplayInfoAddons {
            creature,
            gameobject,
        },
        report,
    ))
}

// ---------------------------------------------------------------------------
// Respawn times (read-only half)

/// One persisted respawn row after validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RespawnEntry {
    pub guid: u32,
    /// Unix time the spawn comes back.
    pub respawn_time: u64,
    pub map_id: u32,
    pub instance_id: u32,
    /// Instance reset time from the LEFT JOIN (0 when none).
    pub reset_time: u64,
}

/// Persisted respawn times for creatures and gameobjects.
#[derive(Clone, Debug, Default)]
pub struct RespawnTimes {
    /// `(instance_id, guid)` -> entry. The C++ continent
    /// fan-out to instance ranges happens when entries are
    /// applied to persistent states (map slice), not here.
    creatures: HashMap<(u32, u32), RespawnEntry>,
    gameobjects: HashMap<(u32, u32), RespawnEntry>,
}

impl RespawnTimes {
    #[must_use]
    pub fn creature(&self, instance_id: u32, guid: u32) -> Option<&RespawnEntry> {
        self.creatures.get(&(instance_id, guid))
    }

    #[must_use]
    pub fn gameobject(&self, instance_id: u32, guid: u32) -> Option<&RespawnEntry> {
        self.gameobjects.get(&(instance_id, guid))
    }

    #[must_use]
    pub fn creature_len(&self) -> usize {
        self.creatures.len()
    }

    #[must_use]
    pub fn gameobject_len(&self) -> usize {
        self.gameobjects.len()
    }
}

/// Read-only half of `LoadCreatureRespawnTimes` /
/// `LoadGameobjectRespawnTimes`
/// (`MapPersistentStateMgr.cpp:986-1120`). C++ first **deletes**
/// outdated rows from the characters DB; that write stays out
/// of Phase 4 (write register), so outdated rows are filtered
/// against `now_unix` instead — the resulting store is the
/// same. Rows whose guid has no spawn data, whose stored map
/// differs from the spawn's map, or whose map id is unknown
/// are dropped silently exactly as in C++.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)] // callers use default sets like the other loaders
pub fn load_respawn_times(
    conn: &mut mysql::Conn,
    now_unix: u64,
    creature_spawn_maps: &HashMap<u32, u32>,
    gameobject_spawn_maps: &HashMap<u32, u32>,
    map_ids: &HashSet<u32>,
) -> Result<RespawnTimes, DbError> {
    let creatures = load_respawn_table(
        conn,
        now_unix,
        "creature_respawn",
        creature_spawn_maps,
        map_ids,
    )?;
    let gameobjects = load_respawn_table(
        conn,
        now_unix,
        "gameobject_respawn",
        gameobject_spawn_maps,
        map_ids,
    )?;
    Ok(RespawnTimes {
        creatures,
        gameobjects,
    })
}

fn load_respawn_table(
    conn: &mut mysql::Conn,
    now_unix: u64,
    table: &str,
    spawn_maps: &HashMap<u32, u32>,
    map_ids: &HashSet<u32>,
) -> Result<HashMap<(u32, u32), RespawnEntry>, DbError> {
    // Fork loader cleanup: outdated rows are deleted
    // before the load (`LoadCreatureRespawnTimes`).
    conn.exec_drop(
        format!("DELETE FROM `{table}` WHERE `respawn_time` <= ?"),
        (now_unix,),
    )?;
    // Table names are fixed internal constants, never user input.
    let sql = format!(
        "SELECT `guid`, `respawn_time`, `{table}`.`map`, `instance`, `reset_time` \
         FROM `{table}` LEFT JOIN `instance` ON `instance` = `id` \
         WHERE `respawn_time` > ?"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, (now_unix,))?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let entry = RespawnEntry {
            guid: f.u32(0),
            respawn_time: f.u64(1),
            map_id: f.u32(2),
            instance_id: f.u32(3),
            reset_time: f.u64(4),
        };
        match spawn_maps.get(&entry.guid) {
            Some(&spawn_map) if spawn_map == entry.map_id && map_ids.contains(&entry.map_id) => {
                out.insert((entry.instance_id, entry.guid), entry);
            }
            _ => {}
        }
    }
    Ok(out)
}
