//! Item prototype fields needed before the full
//! `LoadItemPrototypes` port: the virtual-item projection
//! (`Creature::SetVirtualItem`) and inventory-type checks.
//! The full prototype (stats, damage, spells) lands with
//! the inventory domain; this store grows then.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

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

/// `MAX_ITEM_PROTO_STATS`.
pub const MAX_ITEM_PROTO_STATS: usize = 10;
/// `MAX_ITEM_PROTO_DAMAGES`.
pub const MAX_ITEM_PROTO_DAMAGES: usize = 5;
/// `MAX_ITEM_PROTO_SPELLS`.
pub const MAX_ITEM_PROTO_SPELLS: usize = 5;
/// `ITEM_EXTRA_MAIL_STATIONERY`.
pub const ITEM_EXTRA_MAIL_STATIONERY: u32 = 0x01;

/// `_ItemStat`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ItemStat {
    pub stat_type: u32,
    pub value: i32,
}

/// `_ItemDamage`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemDamage {
    pub min: f32,
    pub max: f32,
    pub damage_type: u32,
}

/// `_ItemSpell`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemSpell {
    pub spell_id: u32,
    pub trigger: u32,
    pub charges: i32,
    pub ppm_rate: f32,
    pub cooldown: i32,
    pub category: u32,
    pub category_cooldown: i32,
}

/// Full `ItemPrototype` fields loaded by
/// `ObjectMgr::LoadItemPrototypes`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemPrototype {
    pub item_id: u32,
    pub class: u32,
    pub subclass: u32,
    pub name: String,
    pub description: String,
    pub display_id: u32,
    pub quality: u32,
    pub flags: u32,
    pub buy_count: u32,
    pub buy_price: u32,
    pub sell_price: u32,
    pub inventory_type: u32,
    pub allowable_class: u32,
    pub allowable_race: u32,
    pub item_level: u32,
    pub required_level: u32,
    pub required_skill: u32,
    pub required_skill_rank: u32,
    pub required_spell: u32,
    pub required_honor_rank: u32,
    pub required_city_rank: u32,
    pub required_reputation_faction: u32,
    pub required_reputation_rank: u32,
    pub max_count: u32,
    pub stackable: u32,
    pub container_slots: u32,
    pub stats: [ItemStat; MAX_ITEM_PROTO_STATS],
    pub delay: u32,
    pub ranged_mod_range: f32,
    pub ammo_type: u32,
    pub damages: [ItemDamage; MAX_ITEM_PROTO_DAMAGES],
    pub block: u32,
    pub armor: i32,
    pub holy_res: i32,
    pub fire_res: i32,
    pub nature_res: i32,
    pub frost_res: i32,
    pub shadow_res: i32,
    pub arcane_res: i32,
    pub spells: [ItemSpell; MAX_ITEM_PROTO_SPELLS],
    pub bonding: u32,
    pub page_text: u32,
    pub language_id: u32,
    pub page_material: u32,
    pub start_quest: u32,
    pub lock_id: u32,
    pub material: i32,
    pub sheath: u32,
    pub random_property: u32,
    pub item_set: u32,
    pub max_durability: u32,
    pub area: u32,
    pub map: i32,
    pub duration: u32,
    pub bag_family: i32,
    pub disenchant_id: u32,
    pub food_type: u32,
    pub min_money_loot: u32,
    pub max_money_loot: u32,
    pub wrapped_gift: u32,
    pub extra_flags: u32,
    pub other_team_entry: u32,
    pub discovered: bool,
}

/// Loaded full prototypes by entry.
#[derive(Clone, Debug, Default)]
pub struct ItemPrototypes {
    by_entry: HashMap<u32, ItemPrototype>,
}

impl ItemPrototypes {
    #[must_use]
    pub fn get(&self, entry: u32) -> Option<&ItemPrototype> {
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
}

/// Distinct item templates currently instantiated for
/// characters (`FillObtainedItemsList`, characters half).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_obtained_item_ids(conn: &mut mysql::Conn) -> Result<HashSet<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.query("SELECT DISTINCT `item_id` FROM `item_instance`")?;
    Ok(rows
        .into_iter()
        .map(|row| mangos_db::row::Fields::from_row(row).u32(0))
        .collect())
}

/// Loads the full item prototypes at the progressive
/// max patch, mirroring `ObjectMgr::LoadItemPrototypes`.
/// `obtained_items` is the C++ `FillObtainedItemsList`
/// set; discovered state follows the C++ rule exactly.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::too_many_lines)]
pub fn load_item_prototypes<S: BuildHasher>(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    prevent_datamining: bool,
    obtained_items: &HashSet<u32, S>,
) -> Result<ItemPrototypes, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `class`, `subclass`, `name`, `description`, `display_id`, `quality`, \
         `flags`, `buy_count`, `buy_price`, `sell_price`, `inventory_type`, `allowable_class`, \
         `allowable_race`, `item_level`, `required_level`, `required_skill`, \
         `required_skill_rank`, `required_spell`, `required_honor_rank`, `required_city_rank`, \
         `required_reputation_faction`, `required_reputation_rank`, `max_count`, `stackable`, \
         `container_slots`, `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`, \
         `stat_type3`, `stat_value3`, `stat_type4`, `stat_value4`, `stat_type5`, `stat_value5`, \
         `stat_type6`, `stat_value6`, `stat_type7`, `stat_value7`, `stat_type8`, `stat_value8`, \
         `stat_type9`, `stat_value9`, `stat_type10`, `stat_value10`, `delay`, `range_mod`, \
         `ammo_type`, `dmg_min1`, `dmg_max1`, `dmg_type1`, `dmg_min2`, `dmg_max2`, `dmg_type2`, \
         `dmg_min3`, `dmg_max3`, `dmg_type3`, `dmg_min4`, `dmg_max4`, `dmg_type4`, `dmg_min5`, \
         `dmg_max5`, `dmg_type5`, `block`, `armor`, `holy_res`, `fire_res`, `nature_res`, \
         `frost_res`, `shadow_res`, `arcane_res`, `spellid_1`, `spelltrigger_1`, \
         `spellcharges_1`, `spellppmrate_1`, `spellcooldown_1`, `spellcategory_1`, \
         `spellcategorycooldown_1`, `spellid_2`, `spelltrigger_2`, `spellcharges_2`, \
         `spellppmrate_2`, `spellcooldown_2`, `spellcategory_2`, `spellcategorycooldown_2`, \
         `spellid_3`, `spelltrigger_3`, `spellcharges_3`, `spellppmrate_3`, `spellcooldown_3`, \
         `spellcategory_3`, `spellcategorycooldown_3`, `spellid_4`, `spelltrigger_4`, \
         `spellcharges_4`, `spellppmrate_4`, `spellcooldown_4`, `spellcategory_4`, \
         `spellcategorycooldown_4`, `spellid_5`, `spelltrigger_5`, `spellcharges_5`, \
         `spellppmrate_5`, `spellcooldown_5`, `spellcategory_5`, `spellcategorycooldown_5`, \
         `bonding`, `page_text`, `page_language`, `page_material`, `start_quest`, `lock_id`, \
         `material`, `sheath`, `random_property`, `set_id`, `max_durability`, `area_bound`, \
         `map_bound`, `duration`, `bag_family`, `disenchant_id`, `food_type`, `min_money_loot`, \
         `max_money_loot`, `wrapped_gift`, `extra_flags`, `other_team_entry` \
         FROM `item_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `item_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)",
        (wow_patch,),
    )?;

    let mut by_entry = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let entry = f.u32(0);
        let mut stats = [ItemStat::default(); MAX_ITEM_PROTO_STATS];
        for (i, stat) in stats.iter_mut().enumerate() {
            *stat = ItemStat {
                stat_type: u32::from(f.u8(26 + i * 2)),
                value: i32::from(f.i16(27 + i * 2)),
            };
        }
        let mut damages = [ItemDamage::default(); MAX_ITEM_PROTO_DAMAGES];
        for (i, damage) in damages.iter_mut().enumerate() {
            *damage = ItemDamage {
                min: f.f32(49 + i * 3),
                max: f.f32(50 + i * 3),
                damage_type: u32::from(f.u8(51 + i * 3)),
            };
        }
        let mut spells = [ItemSpell::default(); MAX_ITEM_PROTO_SPELLS];
        for (i, spell) in spells.iter_mut().enumerate() {
            *spell = ItemSpell {
                spell_id: f.u32(72 + i * 7),
                trigger: u32::from(f.u8(73 + i * 7)),
                charges: i32::from(f.i16(74 + i * 7)),
                ppm_rate: f.f32(75 + i * 7),
                cooldown: f.i32(76 + i * 7),
                category: u32::from(f.u16(77 + i * 7)),
                category_cooldown: f.i32(78 + i * 7),
            };
        }

        let extra_flags = u32::from(f.u8(127));
        let mut proto = ItemPrototype {
            item_id: entry,
            class: u32::from(f.u8(1)),
            subclass: u32::from(f.u8(2)),
            name: f.string(3),
            description: f.string(4),
            display_id: f.u32(5),
            quality: u32::from(f.u8(6)),
            flags: f.u32(7),
            buy_count: u32::from(f.u8(8)),
            buy_price: f.u32(9),
            sell_price: f.u32(10),
            inventory_type: u32::from(f.u8(11)),
            allowable_class: f.i32(12) as u32,
            allowable_race: f.i32(13) as u32,
            item_level: u32::from(f.u8(14)),
            required_level: u32::from(f.u8(15)),
            required_skill: u32::from(f.u16(16)),
            required_skill_rank: u32::from(f.u16(17)),
            required_spell: f.u32(18),
            required_honor_rank: f.u32(19),
            required_city_rank: f.u32(20),
            required_reputation_faction: u32::from(f.u16(21)),
            required_reputation_rank: u32::from(f.u16(22)),
            max_count: u32::from(f.u16(23)),
            stackable: u32::from(f.u16(24)),
            container_slots: u32::from(f.u8(25)),
            stats,
            delay: u32::from(f.u16(46)),
            ranged_mod_range: f.f32(47),
            ammo_type: u32::from(f.u8(48)),
            damages,
            block: f.u32(64),
            armor: i32::from(f.i16(65)),
            holy_res: i32::from(f.i16(66)),
            fire_res: i32::from(f.i16(67)),
            nature_res: i32::from(f.i16(68)),
            frost_res: i32::from(f.i16(69)),
            shadow_res: i32::from(f.i16(70)),
            arcane_res: i32::from(f.i16(71)),
            spells,
            bonding: u32::from(f.u8(107)),
            page_text: f.u32(108),
            language_id: u32::from(f.u8(109)),
            page_material: u32::from(f.u8(110)),
            start_quest: f.u32(111),
            lock_id: f.u32(112),
            material: i32::from(f.i16(113)),
            sheath: u32::from(f.u8(114)),
            random_property: f.u32(115),
            item_set: f.u32(116),
            max_durability: f.u32(117),
            area: f.u32(118),
            map: i32::from(f.i16(119)),
            duration: f.u32(120),
            bag_family: f.i32(121),
            disenchant_id: f.u32(122),
            food_type: u32::from(f.u8(123)),
            min_money_loot: f.u32(124),
            max_money_loot: f.u32(125),
            wrapped_gift: f.u32(126),
            extra_flags,
            other_team_entry: f.u32(128),
            discovered: false,
        };
        proto.discovered = obtained_items.contains(&entry)
            || proto.extra_flags & ITEM_EXTRA_MAIL_STATIONERY != 0
            || !prevent_datamining;
        if proto.buy_count == 0 {
            proto.buy_count = 1;
        }
        by_entry.insert(entry, proto);
    }
    Ok(ItemPrototypes { by_entry })
}
