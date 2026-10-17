//! The three static lookups `Player::BuildEnumData` needs
//! (`Player.cpp:1627-1727`): valid (race, class) pairs
//! (`ObjectMgr::LoadPlayerInfo` over `playercreateinfo`),
//! creature pet families (`creature_template.pet_family`), and
//! item display/inventory types (`item_template`).
//!
//! Patch selection follows the C++ loaders: per entry, the row
//! with the greatest `patch <= WowPatch`. Only the columns the
//! enum builder reads are loaded — a deliberate Phase 3
//! narrowing of the full template loads, recorded in
//! PHASE3-PROGRESS.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ItemDisplay {
    pub display_id: u32,
    pub inventory_type: u8,
}

#[derive(Clone, Debug, Default)]
pub struct EnumStores {
    items: HashMap<u32, ItemDisplay>,
    pet_families: HashMap<u32, u32>,
    player_info_pairs: HashSet<(u8, u8)>,
}

impl EnumStores {
    /// Assemble from parts (fixtures/tests, or a future
    /// flattened cache). The DB loader below is the norm.
    #[must_use]
    pub fn from_parts(
        items: HashMap<u32, ItemDisplay>,
        pet_families: HashMap<u32, u32>,
        player_info_pairs: HashSet<(u8, u8)>,
    ) -> EnumStores {
        EnumStores {
            items,
            pet_families,
            player_info_pairs,
        }
    }

    /// `sObjectMgr.GetPlayerInfo(race, class)` existence.
    #[must_use]
    pub fn has_player_info(&self, race: u8, class: u8) -> bool {
        self.player_info_pairs.contains(&(race, class))
    }

    /// `sObjectMgr.GetItemPrototype(id)` display fields.
    #[must_use]
    pub fn item_display(&self, entry: u32) -> Option<ItemDisplay> {
        self.items.get(&entry).copied()
    }

    /// `sObjectMgr.GetCreatureTemplate(entry)->pet_family`;
    /// `None` when the template does not exist (enum then shows
    /// no pet), `Some(0)` for a family-less template.
    #[must_use]
    pub fn pet_family(&self, entry: u32) -> Option<u32> {
        self.pet_families.get(&entry).copied()
    }

    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn creature_count(&self) -> usize {
        self.pet_families.len()
    }

    #[must_use]
    pub fn player_info_count(&self) -> usize {
        self.player_info_pairs.len()
    }
}

/// Load all three stores from the world DB at the given
/// content patch.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_enum_stores(conn: &mut mysql::Conn, wow_patch: u32) -> Result<EnumStores, DbError> {
    let item_rows: Vec<(u32, u32, u8)> = conn.exec(
        "SELECT `entry`, `display_id`, `inventory_type` FROM `item_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `item_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let items = item_rows
        .into_iter()
        .map(|(entry, display_id, inventory_type)| {
            (
                entry,
                ItemDisplay {
                    display_id,
                    inventory_type,
                },
            )
        })
        .collect();

    let family_rows: Vec<(u32, u32)> = conn.exec(
        "SELECT `entry`, `pet_family` FROM `creature_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `creature_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let pet_families = family_rows.into_iter().collect();

    let pair_rows: Vec<(u8, u8)> = conn.query("SELECT `race`, `class` FROM `playercreateinfo`")?;
    let player_info_pairs = pair_rows.into_iter().collect();

    Ok(EnumStores {
        items,
        pet_families,
        player_info_pairs,
    })
}
