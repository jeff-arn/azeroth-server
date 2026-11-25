//! `PopulateSpellData` resolution: classifies a
//! bot's spellbook into the `m_spells` union
//! fields (by generated name-match chains), the
//! direct/periodic heal sets, taunt list, and
//! resurrection spell. The host supplies spell
//! metadata; this module is pure.

use std::collections::{HashMap, HashSet};

use crate::class_spells_table::chain_for;

/// Per-spell metadata the classifier needs (host
/// extracts from its spell store + spellbook).
#[derive(Clone, Debug)]
pub struct KnownSpellInfo {
    pub id: u32,
    pub name: String,
    /// `SpellEntry::GetRank` (0 = unranked).
    pub rank: u32,
    pub spell_level: u32,
    /// Sum of `SPELL_EFFECT_HEAL` base points, when
    /// the spell has such an effect.
    pub direct_heal_points: Option<i32>,
    /// Sum of periodic-heal aura base points, when
    /// present (set-order key, as in C++).
    pub periodic_heal_points: Option<i32>,
    /// Periodic-heal total over duration
    /// (`duration/amplitude * base`), the efficiency
    /// figure `SelectMostEfficientHealingSpell`
    /// compares.
    pub periodic_heal_total: Option<i32>,
    /// Taunt-effect occurrences (`ATTACK_ME` +
    /// `MOD_TAUNT` auras); C++ pushes once per
    /// matching effect.
    pub taunt_effects: u32,
    pub is_resurrect: bool,
    /// Skipped like C++ (`SPELL_ATTR_PASSIVE` /
    /// `DO_NOT_DISPLAY`).
    pub skip: bool,
}

/// One heal-set entry with its efficiency totals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealSpell {
    pub id: u32,
    /// Base-points sum (set order key).
    pub base_points: i32,
    /// Total healing figure (periodic spells:
    /// duration-scaled).
    pub total_points: i32,
}

/// Resolved class spellbook (`m_spells` union +
/// side lists).
#[derive(Clone, Debug, Default)]
pub struct ClassSpells {
    pub class_id: u8,
    fields: HashMap<&'static str, u32>,
    flags: HashSet<&'static str>,
    /// Descending by heal base points; equal-point
    /// spells collapse (the C++ `std::set` compare
    /// makes them equivalent keys — fork quirk,
    /// preserved).
    pub direct_heals: Vec<HealSpell>,
    pub periodic_heals: Vec<HealSpell>,
    /// Insertion order, duplicates kept (C++
    /// vector).
    pub taunts: Vec<u32>,
    pub resurrection: Option<u32>,
}

impl ClassSpells {
    /// Union field by C++ name (`pShieldSlam`).
    #[must_use]
    pub fn get(&self, field: &str) -> Option<u32> {
        self.fields.get(field).copied()
    }

    /// Poison-known style flags (`hasDeadlyPoison`).
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}

fn needle_matches(needle: &str, name: &str) -> bool {
    if let Some(exact) = needle.strip_prefix('=') {
        return name == exact;
    }
    if let Some((incl, excl)) = needle.split_once('!') {
        return name.contains(incl) && !name.contains(excl);
    }
    name.contains(needle)
}

/// Highest-rank rule (`IsHigherRankSpell`): ranked
/// spells compare rank, unranked compare id.
fn is_higher(new: &KnownSpellInfo, old: Option<(u32, u32)>) -> bool {
    let Some((old_id, old_rank)) = old else {
        return true;
    };
    if new.rank != 0 {
        new.rank > old_rank
    } else {
        new.id > old_id
    }
}

fn insert_heal(list: &mut Vec<HealSpell>, spell: HealSpell) {
    if list.iter().any(|h| h.base_points == spell.base_points) {
        return; // set-equivalence collapse
    }
    list.push(spell);
}

/// Runs the classifier. `pick` chooses an index
/// `< len` for the fork's random post-fixups
/// (`SelectRandomContainerElement`); `poison_lookup`
/// resolves the rogue poison temp-enchant by exact
/// name at or below `level` (global spell scan in
/// C++); `is_tank`/`is_melee_dps` mirror `m_role`
/// checks in the fixups.
pub fn resolve_class_spells(
    class_id: u8,
    level: u32,
    is_tank: bool,
    is_melee_dps: bool,
    known: &[KnownSpellInfo],
    poison_lookup: &dyn Fn(&str, u32) -> Option<u32>,
    pick: &mut dyn FnMut(usize) -> usize,
) -> ClassSpells {
    let mut out = ClassSpells {
        class_id,
        ..Default::default()
    };
    let mut stored: HashMap<&'static str, (u32, u32)> = HashMap::new();
    let mut direct: Vec<HealSpell> = Vec::new();
    let mut periodic: Vec<HealSpell> = Vec::new();
    let chain = chain_for(class_id);

    let mut ordered: Vec<&KnownSpellInfo> = known.iter().filter(|s| !s.skip).collect();
    ordered.sort_by_key(|s| s.id);
    for spell in ordered {
        for row in chain {
            if needle_matches(row.needle, &spell.name) {
                if let Some(flag) = row.field.strip_prefix("flag:") {
                    out.flags.insert(flag);
                } else if is_higher(spell, stored.get(row.field).copied()) {
                    stored.insert(row.field, (spell.id, spell.rank));
                    out.fields.insert(row.field, spell.id);
                }
                break;
            }
        }
        if let Some(points) = spell.direct_heal_points {
            insert_heal(
                &mut direct,
                HealSpell {
                    id: spell.id,
                    base_points: points,
                    total_points: points,
                },
            );
        }
        if let Some(points) = spell.periodic_heal_points {
            insert_heal(
                &mut periodic,
                HealSpell {
                    id: spell.id,
                    base_points: points,
                    total_points: spell.periodic_heal_total.unwrap_or(points),
                },
            );
        }
        for _ in 0..spell.taunt_effects {
            out.taunts.push(spell.id);
        }
        if spell.is_resurrect {
            out.resurrection = Some(spell.id);
        }
    }
    direct.sort_by_key(|h| std::cmp::Reverse(h.base_points));
    periodic.sort_by_key(|h| std::cmp::Reverse(h.base_points));
    out.direct_heals = direct;
    out.periodic_heals = periodic;

    apply_post_fixups(&mut out, level, is_tank, is_melee_dps, poison_lookup, pick);
    out
}

fn choose(
    out: &mut ClassSpells,
    field: &'static str,
    candidates: &[&'static str],
    pick: &mut dyn FnMut(usize) -> usize,
) {
    let present: Vec<u32> = candidates.iter().filter_map(|f| out.get(f)).collect();
    if !present.is_empty() {
        let idx = pick(present.len()).min(present.len() - 1);
        out.fields.insert(field, present[idx]);
    }
}

fn apply_post_fixups(
    out: &mut ClassSpells,
    level: u32,
    is_tank: bool,
    is_melee_dps: bool,
    poison_lookup: &dyn Fn(&str, u32) -> Option<u32>,
    pick: &mut dyn FnMut(usize) -> usize,
) {
    match out.class_id {
        2 => fixup_paladin(out, is_tank, pick),
        7 => fixup_shaman(out, is_melee_dps, pick),
        8 => fixup_mage(out, pick),
        4 => fixup_rogue(out, level, poison_lookup, pick),
        _ => {}
    }
}

fn copy_fields(out: &mut ClassSpells, pairs: &[(&'static str, &'static str)]) {
    for (dst, src) in pairs {
        if let Some(v) = out.get(src) {
            out.fields.insert(dst, v);
        }
    }
}

fn fixup_paladin(out: &mut ClassSpells, is_tank: bool, pick: &mut dyn FnMut(usize) -> usize) {
    let seal = if is_tank {
        out.get("local:pSealOfFury")
    } else {
        None
    }
    .or_else(|| out.get("local:pSealOfCommand"))
    .or_else(|| out.get("local:pSealOfRighteousness"));
    if let Some(seal) = seal {
        out.fields.insert("pSeal", seal);
    }
    if is_tank {
        if let Some(b) = out.get("local:pBlessingOfSanctuary") {
            out.fields.insert("pBlessingBuff", b);
        }
    }
    if out.get("pBlessingBuff").is_none() {
        choose(
            out,
            "pBlessingBuff",
            &[
                "local:pBlessingOfLight",
                "local:pBlessingOfMight",
                "local:pBlessingOfWisdom",
                "local:pBlessingOfKings",
                "local:pBlessingOfSanctuary",
            ],
            pick,
        );
    }
    copy_fields(
        out,
        &[
            ("pBlessingOfWisdom", "local:pBlessingOfWisdom"),
            ("pBlessingOfMight", "local:pBlessingOfMight"),
            ("pBlessingOfSalvation", "local:pBlessingOfSalvation"),
            ("pBlessingOfKings", "local:pBlessingOfKings"),
        ],
    );
    choose(
        out,
        "pAura",
        &[
            "local:pDevotionAura",
            "local:pConcentrationAura",
            "local:pRetributionAura",
            "local:pSanctityAura",
            "local:pShadowResistanceAura",
            "local:pFrostResistanceAura",
            "local:pFireResistanceAura",
        ],
        pick,
    );
}

fn fixup_shaman(out: &mut ClassSpells, is_melee_dps: bool, pick: &mut dyn FnMut(usize) -> usize) {
    choose(
        out,
        "pAirTotem",
        &[
            "local:pGraceOfAirTotem",
            "local:pNatureResistanceTotem",
            "local:pWindfuryTotem",
            "local:pWindwallTotem",
            "local:pTranquilAirTotem",
        ],
        pick,
    );
    choose(
        out,
        "pEarthTotem",
        &[
            "local:pEarthbindTotem",
            "local:pStoneclawtotem",
            "local:pStoneskinTotem",
            "local:pStrengthOfEarthTotem",
            "local:pTremorTotem",
        ],
        pick,
    );
    choose(
        out,
        "pFireTotem",
        &[
            "local:pFireNovaTotem",
            "local:pMagmaTotem",
            "local:pSearingTotem",
            "local:pFlametongueTotem",
            "local:pFrostResistanceTotem",
        ],
        pick,
    );
    choose(
        out,
        "pWaterTotem",
        &[
            "local:pFireResistanceTotem",
            "local:pDiseaseCleansingTotem",
            "local:pHealingStreamTotem",
            "local:pManaSpringTotem",
            "local:pPoisonCleansingTotem",
        ],
        pick,
    );
    copy_fields(
        out,
        &[
            ("pWindfuryTotem", "local:pWindfuryTotem"),
            ("pManaSpringTotem", "local:pManaSpringTotem"),
            ("pHealingStreamTotem", "local:pHealingStreamTotem"),
            ("pStrengthOfEarthTotem", "local:pStrengthOfEarthTotem"),
            ("pSearingTotem", "local:pSearingTotem"),
            ("pTremorTotem", "local:pTremorTotem"),
            ("pStoneclawTotem", "local:pStoneclawtotem"),
            ("pEarthbindTotem", "local:pEarthbindTotem"),
            ("pPoisonCleansingTotem", "local:pPoisonCleansingTotem"),
            ("pTranquilAirTotem", "local:pTranquilAirTotem"),
        ],
    );
    if is_melee_dps {
        if let Some(w) = out.get("local:pWindfuryWeapon") {
            out.fields.insert("pWeaponBuff", w);
        }
    }
    if out.get("pWeaponBuff").is_none() {
        choose(
            out,
            "pWeaponBuff",
            &[
                "local:pWindfuryWeapon",
                "local:pRockbiterWeapon",
                "local:pFrostbrandWeapon",
            ],
            pick,
        );
    }
}

fn fixup_mage(out: &mut ClassSpells, pick: &mut dyn FnMut(usize) -> usize) {
    if out.get("pIceArmor").is_none() {
        if let Some(frost) = out.get("local:pFrostArmor") {
            out.fields.insert("pIceArmor", frost);
        }
    }
    choose(
        out,
        "pPolymorph",
        &[
            "local:pPolymorphSheep",
            "local:pPolymorphCow",
            "local:pPolymorphPig",
            "local:pPolymorphTurtle",
        ],
        pick,
    );
}

fn fixup_rogue(
    out: &mut ClassSpells,
    level: u32,
    poison_lookup: &dyn Fn(&str, u32) -> Option<u32>,
    pick: &mut dyn FnMut(usize) -> usize,
) {
    let mut poisons: Vec<u32> = Vec::new();
    for (flag, name) in [
        ("hasDeadlyPoison", "Deadly Poison"),
        ("hasInstantPoison", "Instant Poison"),
        ("hasCripplingPoison", "Crippling Poison"),
        ("hasWoundPoison", "Wound Poison"),
        ("HasMindNumbingPoison", "Mind-numbing Poison"),
    ] {
        if out.flag(flag) {
            if let Some(id) = poison_lookup(name, level) {
                poisons.push(id);
            }
        }
    }
    if !poisons.is_empty() {
        let mh = poisons[pick(poisons.len()).min(poisons.len() - 1)];
        let oh = poisons[pick(poisons.len()).min(poisons.len() - 1)];
        out.fields.insert("pMainHandPoison", mh);
        out.fields.insert("pOffHandPoison", oh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(id: u32, name: &str, rank: u32) -> KnownSpellInfo {
        KnownSpellInfo {
            id,
            name: name.to_owned(),
            rank,
            spell_level: 0,
            direct_heal_points: None,
            periodic_heal_points: None,
            periodic_heal_total: None,
            taunt_effects: 0,
            is_resurrect: false,
            skip: false,
        }
    }

    #[test]
    fn warrior_chain_highest_rank_wins() {
        let known = vec![
            spell(6552, "Pummel", 0),
            spell(72, "Shield Bash", 1),
            spell(1671, "Shield Bash", 2),
            spell(78, "Heroic Strike", 1),
            spell(284, "Heroic Strike", 2),
        ];
        let out = resolve_class_spells(1, 60, false, true, &known, &|_, _| None, &mut |_| 0);
        assert_eq!(out.get("pPummel"), Some(6552));
        assert_eq!(out.get("pShieldBash"), Some(1671));
        assert_eq!(out.get("pHeroicStrike"), Some(284));
    }

    #[test]
    fn shaman_healing_wave_exclude() {
        let known = vec![
            spell(8004, "Lesser Healing Wave", 1),
            spell(331, "Healing Wave", 1),
        ];
        let out = resolve_class_spells(7, 60, false, false, &known, &|_, _| None, &mut |_| 0);
        assert_eq!(out.get("pHealingWave"), Some(331));
        assert_eq!(out.get("pLesserHealingWave"), Some(8004));
    }

    #[test]
    fn heal_set_collapses_equal_points() {
        let mut a = spell(100, "Heal A", 0);
        a.direct_heal_points = Some(500);
        let mut b = spell(200, "Heal B", 0);
        b.direct_heal_points = Some(500);
        let mut c = spell(300, "Heal C", 0);
        c.direct_heal_points = Some(900);
        let out = resolve_class_spells(5, 60, false, false, &[a, b, c], &|_, _| None, &mut |_| 0);
        let ids: Vec<u32> = out.direct_heals.iter().map(|h| h.id).collect();
        assert_eq!(ids, vec![300, 100]);
    }
}
