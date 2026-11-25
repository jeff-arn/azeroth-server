//! Default gossip menu data (`ObjectMgr::LoadGossipMenu`,
//! `LoadGossipMenuItems`, `LoadNpcGossips`): the DB rows
//! `Player::PrepareGossipMenu` / `GetGossipTextId` consult
//! for NPCs without a script `OnGossipHello` override.
//! Locales (`locales_gossip_menu_option`) are not loaded;
//! sessions run the default locale like the rest of the
//! runtime.

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// One `gossip_menu` row (`GossipMenus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GossipMenuText {
    pub text_id: u32,
    pub script_id: u32,
    pub condition_id: u32,
}

/// One `gossip_menu_option` row (`GossipMenuItems`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GossipMenuOption {
    pub id: u32,
    pub option_icon: u32,
    pub option_text: String,
    pub option_broadcast_text: u32,
    pub option_id: u32,
    pub npc_option_npcflag: u32,
    pub action_menu_id: i32,
    pub action_poi_id: u32,
    pub action_script_id: u32,
    pub box_coded: bool,
    pub box_money: u32,
    pub box_text: String,
    pub box_broadcast_text: u32,
    pub condition_id: u32,
}

/// Loaded gossip tables: menu texts and options keyed by
/// menu id (insertion order = `ORDER BY menu_id, id`, the
/// fork multimap iteration order). The `npc_gossip` half
/// of the fork load already lives in the script stores
/// (`ScriptStores::npc_gossips`).
#[derive(Clone, Debug, Default)]
pub struct GossipStores {
    menus: HashMap<u32, Vec<GossipMenuText>>,
    options: HashMap<u32, Vec<GossipMenuOption>>,
}

impl GossipStores {
    /// `GetGossipMenusMapBounds(menuId)`.
    #[must_use]
    pub fn menu_texts(&self, menu_id: u32) -> &[GossipMenuText] {
        self.menus.get(&menu_id).map_or(&[], Vec::as_slice)
    }

    /// `GetGossipMenuItemsMapBounds(menuId)`.
    #[must_use]
    pub fn menu_options(&self, menu_id: u32) -> &[GossipMenuOption] {
        self.options.get(&menu_id).map_or(&[], Vec::as_slice)
    }

    /// Builds a store from in-memory maps (test
    /// fixtures; no fork validation is applied).
    #[doc(hidden)]
    #[must_use]
    pub fn from_maps_for_tests(
        menus: HashMap<u32, Vec<GossipMenuText>>,
        options: HashMap<u32, Vec<GossipMenuOption>>,
    ) -> Self {
        Self { menus, options }
    }
}

/// Loads the three gossip tables.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_gossip_stores(conn: &mut mysql::Conn) -> Result<GossipStores, DbError> {
    let mut stores = GossipStores::default();
    let rows: Vec<mysql::Row> =
        conn.query("SELECT `entry`, `text_id`, `script_id`, `condition_id` FROM `gossip_menu`")?;
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        stores
            .menus
            .entry(f.u32(0))
            .or_default()
            .push(GossipMenuText {
                text_id: f.u32(1),
                script_id: f.u32(2),
                condition_id: f.u32(3),
            });
    }
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT `menu_id`, `id`, `option_icon`, `option_text`, `option_broadcast_text`, \
         `option_id`, `npc_option_npcflag`, `action_menu_id`, `action_poi_id`, \
         `action_script_id`, `box_coded`, `box_money`, `box_text`, `box_broadcast_text`, \
         `condition_id` FROM `gossip_menu_option` ORDER BY `menu_id`, `id`",
    )?;
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        stores
            .options
            .entry(f.u32(0))
            .or_default()
            .push(GossipMenuOption {
                id: f.u32(1),
                option_icon: f.u32(2),
                option_text: f.string(3),
                option_broadcast_text: f.u32(4),
                option_id: f.u32(5),
                npc_option_npcflag: f.u32(6),
                action_menu_id: f.i32(7),
                action_poi_id: f.u32(8),
                action_script_id: f.u32(9),
                box_coded: f.u8(10) != 0,
                box_money: f.u32(11),
                box_text: f.string(12),
                box_broadcast_text: f.u32(13),
                condition_id: f.u32(14),
            });
    }
    Ok(stores)
}
