//! Vendor stock lists (`VendorItemData`, plus
//! `ObjectMgr::LoadVendors` / `LoadVendorTemplates` /
//! `IsVendorItemValid`).
//!
//! Loader order and gates are fork-exact: templates
//! load first; each row must resolve, for creature
//! lists, to a loaded creature template and, for all
//! lists, to a loaded item prototype; `maxcount` and
//! `incrtime` must be paired; a nonzero condition must
//! exist; duplicates in the same list are dropped; a
//! list may hold at most 254 rows before the next row
//! is rejected (`countItems >= UINT8_MAX`). Every row
//! whose prototype exists marks that item discovered,
//! even when a later gate rejects the row — the caller
//! applies the returned discovered ids to its mutable
//! prototype store because this loader only borrows it.
//!
//! Fork quirk preserved in the creature-list duplicate
//! gate: `IsVendorItemValid` looks up the template
//! list by the *creature entry* (not `vendor_id`) and
//! calls `VendorItemData::GetItem(item_id)`, which is
//! a slot lookup, not an entry lookup. The check below
//! reproduces that accidental slot test verbatim.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_db::row::Fields;

use crate::creature_template::CreatureTemplates;
use crate::item_prototype::ItemPrototypes;
use crate::load_report::LoadReport;

/// One `npc_vendor` / `npc_vendor_template` row
/// (`struct VendorItem`, `CreatureDefines.h:557-566`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VendorItem {
    pub item_id: u32,
    /// Limited stock cap; 0 = unlimited.
    pub max_count: u32,
    /// Restock interval in seconds.
    pub incr_time: u32,
    pub item_flags: u32,
    pub condition_id: u32,
}

/// Loaded vendor lists: per-creature-entry rows from
/// `npc_vendor` and per-template-id rows from
/// `npc_vendor_template` (`m_CacheVendorItemMap` /
/// `m_CacheVendorTemplateItemMap`).
#[derive(Clone, Debug, Default)]
pub struct VendorItemStores {
    by_creature: HashMap<u32, Vec<VendorItem>>,
    by_template: HashMap<u32, Vec<VendorItem>>,
}

impl VendorItemStores {
    /// Builds a store from in-memory lists (test
    /// fixtures; no fork validation is applied).
    #[must_use]
    pub fn from_maps_for_tests(
        by_creature: HashMap<u32, Vec<VendorItem>>,
        by_template: HashMap<u32, Vec<VendorItem>>,
    ) -> Self {
        Self {
            by_creature,
            by_template,
        }
    }

    /// `GetNpcVendorItemList(entry)`: `None` when the
    /// creature has no list at all (fork distinguishes
    /// missing from present-but-empty only transiently;
    /// a loaded list is never empty).
    #[must_use]
    pub fn creature_items(&self, entry: u32) -> Option<&[VendorItem]> {
        self.by_creature.get(&entry).map(Vec::as_slice)
    }

    /// `GetNpcVendorTemplateItemList(vendor_id)`.
    #[must_use]
    pub fn template_items(&self, vendor_id: u32) -> Option<&[VendorItem]> {
        self.by_template.get(&vendor_id).map(Vec::as_slice)
    }

    /// Loaded `npc_vendor` rows.
    #[must_use]
    pub fn creature_item_count(&self) -> usize {
        self.by_creature.values().map(Vec::len).sum()
    }

    /// Loaded `npc_vendor_template` rows.
    #[must_use]
    pub fn template_item_count(&self) -> usize {
        self.by_template.values().map(Vec::len).sum()
    }

    /// Total loaded rows across both maps.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.creature_item_count() + self.template_item_count()
    }
}

/// One raw DB row before validation.
#[derive(Clone, Copy)]
struct RawRow {
    entry: u32,
    item: VendorItem,
}

fn query_rows(conn: &mut mysql::Conn, table: &str, wow_patch: u32) -> Result<Vec<RawRow>, DbError> {
    // The table name is one of two internal literals,
    // matching the fork's formatted query.
    let query = format!(
        "SELECT `entry`, `item`, `maxcount`, `incrtime`, `itemflags`, `condition_id` \
         FROM `{table}` WHERE (`item` NOT IN (SELECT `entry` FROM `forbidden_items` \
         WHERE (`after_or_before` = 0 && `patch` <= ?) || \
         (`after_or_before` = 1 && `patch` >= ?))) ORDER BY `entry`, `slot`"
    );
    let rows: Vec<mysql::Row> = conn.exec(&query, (wow_patch, wow_patch))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            RawRow {
                entry: f.u32(0),
                item: VendorItem {
                    item_id: f.u32(1),
                    max_count: f.u32(2),
                    incr_time: f.u32(3),
                    item_flags: f.u32(4),
                    condition_id: f.u32(5),
                },
            }
        })
        .collect())
}

fn id_set(conn: &mut mysql::Conn, query: &str) -> Result<HashSet<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.query(query)?;
    Ok(rows
        .into_iter()
        .map(|row| Fields::from_row(row).u32(0))
        .collect())
}

/// Validates and inserts one table's rows. Returns the
/// number of accepted rows. `discovered` collects every
/// item id whose prototype exists (fork sets
/// `Discovered = true` before the remaining gates).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn load_table(
    rows: Vec<RawRow>,
    is_template: bool,
    table: &str,
    creature_templates: &CreatureTemplates,
    existing_creature_ids: &HashSet<u32>,
    item_prototypes: &ItemPrototypes,
    existing_item_ids: &HashSet<u32>,
    condition_ids: &HashSet<u32>,
    stores: &mut VendorItemStores,
    discovered: &mut Vec<u32>,
    report: &mut LoadReport,
) -> usize {
    let id_str = if is_template {
        "vendor template"
    } else {
        "vendor"
    };
    let mut accepted = 0usize;
    for row in rows {
        let entry = row.entry;
        let item = row.item;
        if !is_template && creature_templates.get(entry).is_none() {
            if !existing_creature_ids.contains(&entry) {
                report.error(format!(
                    "Table `{table}` has data for nonexistent creature (Entry: {entry}), ignoring"
                ));
            }
            continue;
        }
        if item_prototypes.get(item.item_id).is_some() {
            discovered.push(item.item_id);
        } else {
            if !existing_item_ids.contains(&item.item_id) {
                report.error(format!(
                    "Table `{table}` for {id_str} {entry} contain nonexistent item ({}), ignoring",
                    item.item_id
                ));
            }
            continue;
        }
        if item.max_count > 0 && item.incr_time == 0 {
            report.error(format!(
                "Table `{table}` has `maxcount` ({}) for item {} of {id_str} {entry} but `incrtime`=0, ignoring",
                item.max_count, item.item_id
            ));
            continue;
        }
        if item.max_count == 0 && item.incr_time > 0 {
            report.error(format!(
                "Table `{table}` has `maxcount`=0 for item {} of {id_str} {entry} but `incrtime`<>0, ignoring",
                item.item_id
            ));
            continue;
        }
        if item.condition_id != 0 && !condition_ids.contains(&item.condition_id) {
            report.error(format!(
                "Table `{table}` has `condition_id`={} for item {} of {id_str} {entry} but this condition is not valid, ignoring",
                item.condition_id, item.item_id
            ));
            continue;
        }

        let own_list = if is_template {
            stores.template_items(entry)
        } else {
            stores.creature_items(entry)
        };
        // Fork quirk: the creature-list template
        // operand is keyed by the creature entry.
        let template_list = if is_template {
            None
        } else {
            stores.template_items(entry)
        };
        if own_list.is_none() && template_list.is_none() {
            // First row for this entry: the fork returns
            // before the duplicate/size checks.
        } else {
            if let Some(list) = own_list
                && list.iter().any(|v| v.item_id == item.item_id)
            {
                report.error(format!(
                    "Table `{table}` has duplicate items {} for {id_str} {entry}, ignoring",
                    item.item_id
                ));
                report.error(format!(
                    "Table `npc_vendor` has duplicate items {} for vendor (Entry: {entry}), ignoring",
                    item.item_id
                ));
                continue;
            }
            if !is_template
                && let Some(list) = template_list
                && (item.item_id as usize) < list.len()
            {
                // `GetItem(item_id)` slot lookup (see
                // module docs): any in-range slot
                // counts as "already in list".
                let vendor_id = creature_templates
                    .get(entry)
                    .map_or(0, |info| info.vendor_id);
                if vendor_id == 0 {
                    report.error(format!(
                        "Table `{table}` has duplicate items {} for {id_str} {entry}, ignoring",
                        item.item_id
                    ));
                } else {
                    report.error(format!(
                        "Table `{table}` has duplicate items {} for {id_str} {entry} (or possible in vendor template {vendor_id}), ignoring",
                        item.item_id
                    ));
                }
                continue;
            }
            let count_items = own_list.map_or(0, <[VendorItem]>::len)
                + template_list.map_or(0, <[VendorItem]>::len);
            if count_items >= usize::from(u8::MAX) {
                report.error(format!(
                    "Table `{table}` has too many items ({count_items} >= {}) for {id_str} {entry}, ignoring",
                    u8::MAX
                ));
                continue;
            }
        }

        if is_template {
            stores.by_template.entry(entry).or_default().push(item);
        } else {
            stores.by_creature.entry(entry).or_default().push(item);
        }
        accepted += 1;
    }
    accepted
}

/// `LoadVendorTemplates` + `LoadVendors`
/// (`ObjectMgr.cpp:10397-10475`), including the
/// unused-template post-loading report.
///
/// Returns the stores, every item id the fork would
/// have marked discovered, and the load report.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)]
pub fn load_vendor_items(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    creature_templates: &CreatureTemplates,
    item_prototypes: &ItemPrototypes,
    condition_ids: &HashSet<u32>,
) -> Result<(VendorItemStores, Vec<u32>, LoadReport), DbError> {
    let mut report = LoadReport::default();
    let existing_creature_ids = id_set(conn, "SELECT `entry` FROM `creature_template`")?;
    let existing_item_ids = id_set(conn, "SELECT `entry` FROM `item_template`")?;
    let mut stores = VendorItemStores::default();
    let mut discovered = Vec::new();

    let template_rows = query_rows(conn, "npc_vendor_template", wow_patch)?;
    let template_count = load_table(
        template_rows,
        true,
        "npc_vendor_template",
        creature_templates,
        &existing_creature_ids,
        item_prototypes,
        &existing_item_ids,
        condition_ids,
        &mut stores,
        &mut discovered,
        &mut report,
    );

    // Post-loading check: template ids not used by any
    // creature template row (any patch), and vendor ids
    // with no loaded template list.
    let used_rows: Vec<mysql::Row> = conn.query(
        "SELECT `entry`, `patch`, `vendor_id` FROM `creature_template` WHERE `vendor_id` != 0",
    )?;
    let mut template_ids: HashSet<u32> = stores.by_template.keys().copied().collect();
    for row in used_rows {
        let f = Fields::from_row(row);
        let (creature_id, patch, vendor_id) = (f.u32(0), f.u32(1), f.u32(2));
        if template_ids.remove(&vendor_id) {
            continue;
        }
        if !stores.by_template.contains_key(&vendor_id) {
            report.error(format!(
                "Table `creature_template` has vendor_id {vendor_id} for creature (Entry: {creature_id}, Patch: {patch}) which does not exist in `npc_vendor_template`."
            ));
        }
    }
    for vendor_id in template_ids {
        report.error(format!(
            "Table `npc_vendor_template` has vendor template {vendor_id} not used by any vendors"
        ));
    }

    let creature_rows = query_rows(conn, "npc_vendor", wow_patch)?;
    let creature_count = load_table(
        creature_rows,
        false,
        "npc_vendor",
        creature_templates,
        &existing_creature_ids,
        item_prototypes,
        &existing_item_ids,
        condition_ids,
        &mut stores,
        &mut discovered,
        &mut report,
    );

    let _ = (template_count, creature_count);
    Ok((stores, discovered, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_round_trip() {
        let item = VendorItem {
            item_id: 25,
            max_count: 0,
            incr_time: 0,
            item_flags: 0,
            condition_id: 0,
        };
        let stores = VendorItemStores::from_maps_for_tests(
            HashMap::from([(66, vec![item])]),
            HashMap::from([(7, vec![item])]),
        );
        assert_eq!(stores.creature_items(66), Some([item].as_slice()));
        assert_eq!(stores.template_items(7), Some([item].as_slice()));
        assert_eq!(stores.creature_items(7), None);
        assert_eq!(stores.item_count(), 2);
    }
}
