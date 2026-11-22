//! Loot templates and generation (fork `LootMgr.h/.cpp`,
//! Tier 4 B0-10): the nine `*_loot_template` stores,
//! template/group processing with references, and
//! money generation. Player-dependent fill (quest /
//! FFA / conditional per-player lists, allowed
//! looters) lives in the map loot state; this layer
//! is pure template data + rolls.
//!
//! Random property ids are 0, matching every other
//! Rust grant path (`GenerateItemRandomPropertyId`
//! is a recorded global gap).
//!
//! `AllowedForTeam` condition evaluation needs the
//! map condition store; generation receives a
//! `team` (0 = crossfaction) plus a condition-team
//! probe, mirroring the loader existence probes.

use std::collections::HashMap;

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_db::row::Fields;

use crate::item_prototype::ItemPrototypes;
use crate::load_report::LoadReport;

/// `MAX_NR_LOOT_ITEMS`.
pub const MAX_NR_LOOT_ITEMS: usize = 16;
/// `MAX_NR_QUEST_ITEMS`.
pub const MAX_NR_QUEST_ITEMS: usize = 32;
/// `ITEM_FLAG_PARTY_LOOT` (free-for-all).
pub const ITEM_FLAG_PARTY_LOOT: u32 = 0x0000_0001;

/// One `*_loot_template` row (`LootStoreItem`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LootStoreItem {
    pub itemid: u32,
    /// Always positive drop chance (percent).
    pub chance: f32,
    /// >0: min count; <0: `-referenced template id`.
    pub mincount_or_ref: i32,
    pub group: u8,
    /// Negative `ChanceOrQuestChance` in DB.
    pub needs_quest: bool,
    pub maxcount: u8,
    pub condition_id: u16,
}

/// A rolled loot item (`LootItem` generation fields;
/// slot state lives in the map loot state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneratedLootItem {
    pub itemid: u32,
    pub count: u8,
    pub condition_id: u16,
    pub needs_quest: bool,
    pub freeforall: bool,
}

#[derive(Clone, Debug, Default)]
struct LootGroup {
    explicitly_chanced: Vec<LootStoreItem>,
    equal_chanced: Vec<LootStoreItem>,
    has_conditional_equal: bool,
}

impl LootGroup {
    fn add_entry(&mut self, item: LootStoreItem) {
        if item.chance == 0.0 {
            if item.condition_id != 0 {
                self.has_conditional_equal = true;
            }
            self.equal_chanced.push(item);
        } else {
            self.explicitly_chanced.push(item);
        }
    }

    fn has_quest_drop(&self) -> bool {
        self.explicitly_chanced.iter().any(|i| i.needs_quest)
            || self.equal_chanced.iter().any(|i| i.needs_quest)
    }
}

/// One loot id's template (`LootTemplate`).
#[derive(Clone, Debug, Default)]
pub struct LootTemplate {
    entries: Vec<LootStoreItem>,
    groups: Vec<LootGroup>,
}

impl LootTemplate {
    fn add_entry(&mut self, item: LootStoreItem) {
        if item.group > 0 && item.mincount_or_ref > 0 {
            let idx = (item.group - 1) as usize;
            if idx >= self.groups.len() {
                self.groups.resize_with(idx + 1, LootGroup::default);
            }
            self.groups[idx].add_entry(item);
        } else {
            self.entries.push(item);
        }
    }

    /// `HasQuestDrop` (group 0 = whole template).
    #[must_use]
    pub fn has_quest_drop(&self, refs: &LootStore, group_id: u8) -> bool {
        if group_id > 0 {
            return self
                .groups
                .get((group_id - 1) as usize)
                .is_some_and(LootGroup::has_quest_drop);
        }
        for itr in &self.entries {
            if itr.mincount_or_ref < 0 {
                if let Some(r) = refs.get(itr.mincount_or_ref.unsigned_abs()) {
                    if r.has_quest_drop(refs, itr.group) {
                        return true;
                    }
                }
            } else if itr.needs_quest {
                return true;
            }
        }
        self.groups.iter().any(LootGroup::has_quest_drop)
    }
}

/// One `*_loot_template` table (`LootStore`).
#[derive(Clone, Debug, Default)]
pub struct LootStore {
    templates: HashMap<u32, LootTemplate>,
    /// `IsRatesAllowed` (false for mail/reference).
    pub rates_allowed: bool,
}

impl LootStore {
    #[must_use]
    pub fn get(&self, loot_id: u32) -> Option<&LootTemplate> {
        self.templates.get(&loot_id)
    }

    #[must_use]
    pub fn have_loot_for(&self, loot_id: u32) -> bool {
        self.templates.contains_key(&loot_id)
    }

    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Build from fixture rows (tests).
    #[must_use]
    pub fn from_fixtures_for_tests(rows: Vec<(u32, LootStoreItem)>, rates_allowed: bool) -> Self {
        let mut store = LootStore {
            templates: HashMap::new(),
            rates_allowed,
        };
        for (entry, item) in rows {
            store.templates.entry(entry).or_default().add_entry(item);
        }
        store
    }
}

/// All nine stores.
#[derive(Clone, Debug, Default)]
pub struct LootStores {
    /// `map_loot_disabled` map ids (fork
    /// `ObjectMgr::LoadMapLootDisabled`): no loot
    /// generation and loot windows release at once.
    pub disabled_maps: std::collections::HashSet<u32>,
    pub creature: LootStore,
    pub disenchant: LootStore,
    pub fishing: LootStore,
    pub gameobject: LootStore,
    pub item: LootStore,
    pub mail: LootStore,
    pub pickpocketing: LootStore,
    pub reference: LootStore,
    pub skinning: LootStore,
}

/// `Rate.Drop.*` multipliers (fork `qualityToRate` +
/// referenced + money), all default 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LootRates {
    /// Index by item quality 0..=6 (Poor..Artifact).
    pub by_quality: [f32; 7],
    pub referenced: f32,
    pub money: f32,
}

impl Default for LootRates {
    fn default() -> Self {
        Self {
            by_quality: [1.0; 7],
            referenced: 1.0,
            money: 1.0,
        }
    }
}

/// Load one loot table with the fork query (patch
/// window + forbidden-items exclusion).
fn load_loot_table(
    conn: &mut mysql::Conn,
    table: &'static str,
    rates_allowed: bool,
    wow_patch: u32,
    report: &mut LoadReport,
) -> Result<LootStore, DbError> {
    let sql = format!(
        "SELECT entry, item, ChanceOrQuestChance, groupid, mincountOrRef, maxcount, \
         condition_id FROM {table} WHERE (({wow_patch} >= patch_min) && ({wow_patch} <= patch_max)) \
         && ((mincountOrRef < 0) || (item NOT IN (SELECT entry FROM forbidden_items \
         WHERE (after_or_before = 0 && patch <= {wow_patch}) || (after_or_before = 1 && patch >= {wow_patch}))))"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, ())?;
    let mut store = LootStore {
        templates: HashMap::new(),
        rates_allowed,
    };
    for row in rows {
        let f = Fields::from_row(row);
        let entry = f.u32(0);
        let chance_or_quest = f.f32(2);
        let maxcount_raw = f.u32(5);
        if maxcount_raw > u32::from(u8::MAX) {
            report.error(format!(
                "`{table}` entry {entry}: maxcount {maxcount_raw} > 255, skipped."
            ));
            continue;
        }
        let item = LootStoreItem {
            itemid: f.u32(1),
            chance: chance_or_quest.abs(),
            mincount_or_ref: f.i32(4),
            group: f.u32(3) as u8,
            needs_quest: chance_or_quest < 0.0,
            maxcount: maxcount_raw as u8,
            condition_id: f.u32(6) as u16,
        };
        if item.mincount_or_ref == 0 {
            report.error(format!(
                "`{table}` entry {entry} item {}: wrong mincountOrRef (0) - skipped",
                item.itemid
            ));
            continue;
        }
        store.templates.entry(entry).or_default().add_entry(item);
    }
    Ok(store)
}

/// Load all nine loot stores (`LoadLootTemplates_*`).
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn load_loot_stores(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    report: &mut LoadReport,
) -> Result<LootStores, DbError> {
    // Fork `LoadMapLootDisabled` query, verbatim.
    let disabled_rows: Vec<mysql::Row> =
        conn.exec("SELECT `map_id` FROM `map_loot_disabled`", ())?;
    let disabled_maps = disabled_rows
        .into_iter()
        .map(|row| Fields::from_row(row).u32(0))
        .collect();
    Ok(LootStores {
        disabled_maps,
        creature: load_loot_table(conn, "creature_loot_template", true, wow_patch, report)?,
        disenchant: load_loot_table(conn, "disenchant_loot_template", true, wow_patch, report)?,
        fishing: load_loot_table(conn, "fishing_loot_template", true, wow_patch, report)?,
        gameobject: load_loot_table(conn, "gameobject_loot_template", true, wow_patch, report)?,
        item: load_loot_table(conn, "item_loot_template", true, wow_patch, report)?,
        mail: load_loot_table(conn, "mail_loot_template", false, wow_patch, report)?,
        pickpocketing: load_loot_table(
            conn,
            "pickpocketing_loot_template",
            true,
            wow_patch,
            report,
        )?,
        reference: load_loot_table(conn, "reference_loot_template", false, wow_patch, report)?,
        skinning: load_loot_table(conn, "skinning_loot_template", true, wow_patch, report)?,
    })
}

/// Generation output (`Loot` items split).
#[derive(Clone, Debug, Default)]
pub struct GeneratedLoot {
    pub items: Vec<GeneratedLootItem>,
    pub quest_items: Vec<GeneratedLootItem>,
    pub has_ffa_quest_items: bool,
}

/// Loot roll RNG (`rand_chance_f` / `urand`).
pub trait LootRng {
    /// Uniform in [0, 100).
    fn chance100(&mut self) -> f32;
    /// Inclusive uniform in [min, max].
    fn urand(&mut self, min: u32, max: u32) -> u32;
}

/// Roll context: RNG + prototype/condition probes.
pub struct LootRollCtx<'a> {
    pub rng: &'a mut dyn LootRng,
    pub prototypes: &'a ItemPrototypes,
    pub rates: LootRates,
    /// Looting group team for `AllowedForTeam`:
    /// 0 crossfaction, 1 alliance, 2 horde.
    pub team: u32,
    /// `condition_id` -> condition team (same encoding,
    /// 0 = none/crossfaction); `None` = unknown
    /// condition (fork: not allowed).
    pub condition_team: &'a dyn Fn(u16) -> Option<u32>,
}

impl LootRollCtx<'_> {
    fn roll_chance(&mut self, chance: f32) -> bool {
        chance >= 100.0 || self.rng.chance100() < chance
    }

    fn allowed_for_team(&self, item: &LootStoreItem) -> bool {
        if item.condition_id == 0 {
            return true;
        }
        // Non-player-dependent condition bodies are
        // evaluated map-side at view time; generation
        // applies only the fork's team gate.
        match (self.condition_team)(item.condition_id) {
            None => false,
            Some(0) => true,
            Some(t) => self.team == 0 || self.team == t,
        }
    }

    fn item_roll(&mut self, item: &LootStoreItem, rate: bool) -> bool {
        if item.chance >= 100.0 {
            return true;
        }
        if item.mincount_or_ref < 0 {
            let mult = if rate { self.rates.referenced } else { 1.0 };
            return self.roll_chance(item.chance * mult);
        }
        let quality_modifier = if rate {
            self.prototypes.get(item.itemid).map_or(1.0, |p| {
                self.rates
                    .by_quality
                    .get(p.quality as usize)
                    .copied()
                    .unwrap_or(1.0)
            })
        } else {
            1.0
        };
        self.roll_chance(item.chance * quality_modifier)
    }

    fn add_item(&mut self, out: &mut GeneratedLoot, item: &LootStoreItem) {
        let proto = self.prototypes.get(item.itemid);
        let freeforall = proto.is_some_and(|p| p.flags & ITEM_FLAG_PARTY_LOOT != 0);
        if item.needs_quest && freeforall {
            out.has_ffa_quest_items = true;
        }
        let count = self
            .rng
            .urand(item.mincount_or_ref as u32, u32::from(item.maxcount.max(1)));
        let generated = GeneratedLootItem {
            itemid: item.itemid,
            count: count.min(u32::from(u8::MAX)) as u8,
            condition_id: item.condition_id,
            needs_quest: item.needs_quest,
            freeforall,
        };
        if item.needs_quest {
            if out.quest_items.len() < MAX_NR_QUEST_ITEMS {
                out.quest_items.push(generated);
            }
        } else if out.items.len() < MAX_NR_LOOT_ITEMS {
            out.items.push(generated);
        }
    }

    fn group_roll(&mut self, group: &LootGroup) -> Option<LootStoreItem> {
        if !group.explicitly_chanced.is_empty() {
            let mut roll = self.rng.chance100();
            for i in &group.explicitly_chanced {
                if !self.allowed_for_team(i) {
                    continue;
                }
                if i.chance >= 100.0 {
                    return Some(*i);
                }
                roll -= i.chance;
                if roll < 0.0 {
                    return Some(*i);
                }
            }
        }
        if !group.equal_chanced.is_empty() {
            if !group.has_conditional_equal || self.team == 0 {
                let idx = self.rng.urand(0, (group.equal_chanced.len() - 1) as u32) as usize;
                return Some(group.equal_chanced[idx]);
            }
            let ok: Vec<usize> = group
                .equal_chanced
                .iter()
                .enumerate()
                .filter(|(_, i)| self.allowed_for_team(i))
                .map(|(idx, _)| idx)
                .collect();
            if !ok.is_empty() {
                let pick = self.rng.urand(0, (ok.len() - 1) as u32) as usize;
                return Some(group.equal_chanced[ok[pick]]);
            }
        }
        None
    }

    fn process_template(
        &mut self,
        template: &LootTemplate,
        refs: &LootStore,
        rate: bool,
        group_id: u8,
        out: &mut GeneratedLoot,
    ) {
        if group_id > 0 {
            if let Some(group) = template.groups.get((group_id - 1) as usize) {
                if let Some(item) = self.group_roll(group) {
                    self.add_item(out, &item);
                }
            }
            return;
        }
        for itr in &template.entries {
            if !self.item_roll(itr, rate) {
                continue;
            }
            if itr.mincount_or_ref < 0 {
                let Some(referenced) = refs.get(itr.mincount_or_ref.unsigned_abs()) else {
                    continue;
                };
                for _ in 0..itr.maxcount {
                    self.process_template(referenced, refs, rate, itr.group, out);
                }
            } else {
                self.add_item(out, itr);
            }
        }
        for group in &template.groups {
            if let Some(item) = self.group_roll(group) {
                self.add_item(out, &item);
            }
        }
    }
}

/// `Loot::FillLoot` generation half. Returns false
/// when the store has no template for `loot_id`.
pub fn generate_loot(
    ctx: &mut LootRollCtx,
    stores: &LootStores,
    store: &LootStore,
    loot_id: u32,
) -> Option<GeneratedLoot> {
    let template = store.get(loot_id)?;
    let mut out = GeneratedLoot::default();
    ctx.process_template(
        template,
        &stores.reference,
        store.rates_allowed,
        0,
        &mut out,
    );
    Some(out)
}

/// `Loot::GenerateMoneyLoot`.
#[allow(clippy::cast_precision_loss)] // fork money math is float
pub fn generate_money_loot(ctx: &mut LootRollCtx, min_amount: u32, max_amount: u32) -> u32 {
    if max_amount == 0 {
        return 0;
    }
    let rate = ctx.rates.money;
    if max_amount <= min_amount {
        (max_amount as f32 * rate) as u32
    } else if (max_amount - min_amount) < 32700 {
        (ctx.rng.urand(min_amount, max_amount) as f32 * rate) as u32
    } else {
        ((ctx.rng.urand(min_amount >> 8, max_amount >> 8) as f32 * rate) as u32) << 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_prototype::ItemPrototype;

    struct TestRng;
    impl LootRng for TestRng {
        fn chance100(&mut self) -> f32 {
            0.0
        }
        fn urand(&mut self, min: u32, _max: u32) -> u32 {
            min
        }
    }

    fn store_item(itemid: u32, chance: f32, group: u8, minref: i32, maxcount: u8) -> LootStoreItem {
        LootStoreItem {
            itemid,
            chance: chance.abs(),
            mincount_or_ref: minref,
            group,
            needs_quest: chance < 0.0,
            maxcount,
            condition_id: 0,
        }
    }

    #[test]
    fn guaranteed_and_quest_split() {
        let protos = ItemPrototypes::default();
        let stores = LootStores {
            creature: LootStore::from_fixtures_for_tests(
                vec![
                    (10, store_item(100, 100.0, 0, 2, 2)),
                    (10, store_item(200, -100.0, 0, 1, 1)),
                    (10, store_item(300, 0.0, 1, 1, 1)),
                    (10, store_item(301, 0.0, 1, 1, 1)),
                ],
                true,
            ),
            ..LootStores::default()
        };
        let mut rng = TestRng;
        let mut c = LootRollCtx {
            rng: &mut rng,
            prototypes: &protos,
            rates: LootRates::default(),
            team: 0,
            condition_team: &|_| Some(0),
        };
        let out = generate_loot(&mut c, &stores, &stores.creature, 10).expect("template");
        assert_eq!(out.items.len(), 2); // 100 + one equal-chanced group pick
        assert_eq!(out.items[0].itemid, 100);
        assert_eq!(out.items[0].count, 2);
        assert_eq!(out.quest_items.len(), 1);
        assert_eq!(out.quest_items[0].itemid, 200);
    }

    #[test]
    fn reference_expansion() {
        let protos = ItemPrototypes::default();
        let stores = LootStores {
            creature: LootStore::from_fixtures_for_tests(
                vec![(10, store_item(0, 100.0, 0, -77, 2))],
                true,
            ),
            reference: LootStore::from_fixtures_for_tests(
                vec![(77, store_item(555, 100.0, 0, 1, 1))],
                false,
            ),
            ..LootStores::default()
        };
        let mut rng = TestRng;
        let mut c = LootRollCtx {
            rng: &mut rng,
            prototypes: &protos,
            rates: LootRates::default(),
            team: 0,
            condition_team: &|_| Some(0),
        };
        let out = generate_loot(&mut c, &stores, &stores.creature, 10).expect("template");
        assert_eq!(out.items.len(), 2);
        assert!(out.items.iter().all(|i| i.itemid == 555));
    }

    #[test]
    fn money_roll() {
        let protos = ItemPrototypes::default();
        let mut rng = TestRng;
        let mut c = LootRollCtx {
            rng: &mut rng,
            prototypes: &protos,
            rates: LootRates::default(),
            team: 0,
            condition_team: &|_| Some(0),
        };
        assert_eq!(generate_money_loot(&mut c, 0, 0), 0);
        assert_eq!(generate_money_loot(&mut c, 50, 50), 50);
        assert_eq!(generate_money_loot(&mut c, 10, 90), 10);
    }

    #[test]
    fn proto_shape_compiles() {
        let _ = ItemPrototype::default();
    }
}
