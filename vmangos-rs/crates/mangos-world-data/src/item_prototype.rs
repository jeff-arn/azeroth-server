//! Item prototype fields needed before the full
//! `LoadItemPrototypes` port: the virtual-item projection
//! (`Creature::SetVirtualItem`) and inventory-type checks.
//! The full prototype (stats, damage, spells) lands with
//! the inventory domain; this store grows then.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// `ItemPrototype` fields for virtual items.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ItemPrototypeLite {
    pub display_id: u32,
    pub class: u8,
    pub subclass: u8,
    pub material: u8,
    pub inventory_type: u8,
    pub sheath: u8,
}

/// Loaded prototypes by entry.
#[derive(Clone, Debug, Default)]
pub struct ItemPrototypesLite {
    by_entry: HashMap<u32, ItemPrototypeLite>,
}

impl ItemPrototypesLite {
    #[must_use]
    pub fn get(&self, entry: u32) -> Option<ItemPrototypeLite> {
        self.by_entry.get(&entry).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    /// `item id -> inventory type` for the equipment
    /// template validator.
    #[must_use]
    pub fn inventory_types(&self) -> HashMap<u32, u32> {
        self.by_entry
            .iter()
            .map(|(&id, p)| (id, u32::from(p.inventory_type)))
            .collect()
    }
}

/// Loads the lite prototypes (progressive max-patch rows).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_item_prototypes_lite(
    conn: &mut mysql::Conn,
    wow_patch: u32,
) -> Result<ItemPrototypesLite, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `class`, `subclass`, `display_id`, `material`, `inventory_type`, \
         `sheath` FROM `item_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `item_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let mut by_entry = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        by_entry.insert(
            f.u32(0),
            ItemPrototypeLite {
                class: f.u8(1),
                subclass: f.u8(2),
                display_id: f.u32(3),
                material: f.u8(4),
                inventory_type: f.u8(5),
                sheath: f.u8(6),
            },
        );
    }
    Ok(ItemPrototypesLite { by_entry })
}
