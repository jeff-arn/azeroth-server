//! `spell_template` loader: `SpellMgr::LoadSpells` +
//! `LoadSpell` (`SpellMgr.cpp:3530/3625`) subset. Reads
//! `SELECT *` with the build gate and maps columns by
//! index exactly like the C++ (which relies on schema
//! order), filling the Slice 5 `SpellEntry` subset.
//! Includes the load-time fixups that affect the subset
//! (periodic amplitude 0 -> 5000). Spell chains, proc
//! events, groups, threats and the other `Load*` stores
//! land with their consumers.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_db::row::Fields;
use mangos_spells::defines::aura;
use mangos_spells::entry::{SpellEffectData, SpellEntry};

use crate::load_report::LoadReport;

/// Loads all spells visible at `supported_build`
/// (max build <= supported per entry).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_spell_templates(
    conn: &mut mysql::Conn,
    supported_build: u32,
    report: &mut LoadReport,
) -> Result<HashMap<u32, SpellEntry>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT * FROM `spell_template` t1 WHERE `build`=(SELECT max(`build`) \
         FROM `spell_template` t2 WHERE t1.`entry`=t2.`entry` && `build` <= ?)",
        (supported_build,),
    )?;
    let mut spells = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = Fields::from_row(row);
        let entry = load_spell(&f, report);
        spells.insert(entry.id, entry);
    }
    Ok(spells)
}

/// `SPELL_EFFECT_CREATE_ITEM` (`SpellDefines.h:235`).
const SPELL_EFFECT_CREATE_ITEM: u32 = 24;

/// Item templates referenced by spells as totems,
/// reagents, or created items — the spell half of
/// `ObjectMgr::FillObtainedItemsList`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_spell_referenced_item_ids(
    conn: &mut mysql::Conn,
    supported_build: u32,
) -> Result<HashSet<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `totem1`, `totem2`, `reagent1`, `reagent2`, `reagent3`, `reagent4`, \
         `reagent5`, `reagent6`, `reagent7`, `reagent8`, `effect1`, `effect2`, `effect3`, \
         `effectItemType1`, `effectItemType2`, `effectItemType3` \
         FROM `spell_template` t1 WHERE `build`=(SELECT max(`build`) \
         FROM `spell_template` t2 WHERE t1.`entry`=t2.`entry` && `build` <= ?)",
        (supported_build,),
    )?;
    let mut items = HashSet::new();
    for row in rows {
        let f = Fields::from_row(row);
        for index in 0..2 {
            let item = f.u32(index);
            if item != 0 {
                items.insert(item);
            }
        }
        for index in 2..10 {
            let item = f.i32(index);
            if item != 0 {
                items.insert(item as u32);
            }
        }
        for i in 0..3 {
            if f.u32(10 + i) == SPELL_EFFECT_CREATE_ITEM {
                items.insert(f.u64(13 + i) as u32);
            }
        }
    }
    Ok(items)
}

fn effect_data(f: &Fields, i: usize) -> SpellEffectData {
    // Column groups start at 62 and stride by group, not
    // by effect (all Effect[0..3], then all DieSides...).
    SpellEffectData {
        effect: f.u32(62 + i),
        die_sides: f.i32(65 + i),
        base_dice: f.u32(68 + i),
        dice_per_level: f.f32(71 + i),
        real_points_per_level: f.f32(74 + i),
        base_points: f.i32(77 + i),
        bonus_coefficient: f.f32(80 + i),
        mechanic: f.u32(83 + i),
        implicit_target_a: f.u32(86 + i),
        implicit_target_b: f.u32(89 + i),
        radius_index: f.u32(92 + i),
        apply_aura_name: f.u32(95 + i),
        amplitude: f.u32(98 + i),
        multiple_value: f.f32(101 + i),
        chain_target: f.u32(104 + i),
        item_type: f.u64(107 + i),
        misc_value: f.i32(110 + i),
        trigger_spell: f.u32(113 + i),
        points_per_combo_point: f.f32(116 + i),
        dmg_multiplier: f.f32(143 + i),
    }
}

fn load_spell(f: &Fields, report: &mut LoadReport) -> SpellEntry {
    let mut effects = [effect_data(f, 0), effect_data(f, 1), effect_data(f, 2)];
    // Load-time amplitude default for periodic auras.
    for eff in &mut effects {
        if eff.amplitude == 0 && aura::amplitude_defaults_5000(eff.apply_aura_name) {
            eff.amplitude = 5000;
        }
        if eff.effect >= mangos_spells::defines::effect::TOTAL {
            report.error(format!(
                "spell_template entry {}: effect {} out of range",
                f.u32(0),
                eff.effect
            ));
        }
    }
    SpellEntry {
        id: f.u32(0),
        school: f.u32(2),
        category: f.u32(3),
        dispel: f.u32(5),
        mechanic: f.u32(6),
        attributes: f.u32(7),
        attributes_ex: f.u32(8),
        attributes_ex2: f.u32(9),
        attributes_ex3: f.u32(10),
        attributes_ex4: f.u32(11),
        targets: f.u32(14),
        casting_time_index: f.u32(19),
        recovery_time: f.u32(20),
        category_recovery_time: f.u32(21),
        interrupt_flags: f.u32(22),
        aura_interrupt_flags: f.u32(23),
        channel_interrupt_flags: f.u32(24),
        proc_flags: f.u32(25),
        proc_chance: f.u32(26),
        proc_charges: f.u32(27),
        max_level: f.u32(28),
        base_level: f.u32(29),
        spell_level: f.u32(30),
        duration_index: f.u32(31),
        power_type: f.u32(32),
        mana_cost: f.u32(33),
        mana_cost_per_level: f.u32(34),
        mana_per_second: f.u32(35),
        mana_per_second_per_level: f.u32(36),
        range_index: f.u32(37),
        speed: f.f32(38),
        stack_amount: f.u32(40),
        effects,
        spell_visual: f.u32(119),
        spell_icon_id: f.u32(121),
        mana_cost_percentage: f.u32(132),
        start_recovery_category: f.u32(133),
        start_recovery_time: f.u32(134),
        spell_family_name: f.u32(137),
        spell_family_flags: f.u64(138),
        max_affected_targets: f.u32(139),
        dmg_class: f.u32(140),
        prevention_type: f.u32(141),
        custom: f.u32(149),
        name: f.string(124),
        script_name: f.string(150),
    }
}
