//! Equipped-item stat application at materialize
//! (`_ApplyItemMods`/`_ApplyItemBonuses` end state):
//! stat totals, pos/neg display halves, armor and
//! resistances, max health/mana, attack power and
//! weapon damage fields, broken/bag exclusions. All
//! float assertions use exactly representable values,
//! so strict equality is the intended check.
#![allow(clippy::float_cmp)]

use std::collections::HashMap;

use mangos_db::character_load::{CharacterLoad, CharacterRow, InventoryRow, ItemInstanceRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::item_prototype::{
    ItemDamage, ItemPrototype, ItemPrototypes, ItemStat, MAX_ITEM_PROTO_DAMAGES,
    MAX_ITEM_PROTO_STATS,
};
use mangos_world_data::player_info::{
    PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
};

fn infos() -> PlayerInfos {
    PlayerInfos::from_maps_for_tests(
        60,
        HashMap::from([(
            (2, 1),
            PlayerCreateInfo {
                map_id: 1,
                area_id: 14,
                position_x: -618.52,
                position_y: -4251.67,
                position_z: 38.72,
                orientation: 1.5,
                display_id_m: 51,
                display_id_f: 52,
            },
        )]),
        HashMap::from([(
            1,
            vec![
                PlayerClassLevelInfo {
                    basehealth: 20,
                    basemana: 100,
                };
                60
            ],
        )]),
        HashMap::from([((2, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
        vec![400; 61],
    )
}

fn stats(pairs: &[(u32, i32)]) -> [ItemStat; MAX_ITEM_PROTO_STATS] {
    let mut out = [ItemStat::default(); MAX_ITEM_PROTO_STATS];
    for (i, &(stat_type, value)) in pairs.iter().enumerate() {
        out[i] = ItemStat { stat_type, value };
    }
    out
}

fn damages(rows: &[(f32, f32)]) -> [ItemDamage; MAX_ITEM_PROTO_DAMAGES] {
    let mut out = [ItemDamage::default(); MAX_ITEM_PROTO_DAMAGES];
    for (i, &(min, max)) in rows.iter().enumerate() {
        out[i] = ItemDamage {
            min,
            max,
            damage_type: 0,
        };
    }
    out
}

fn prototypes() -> ItemPrototypes {
    ItemPrototypes::from_entries_for_tests(HashMap::from([
        // Chest: armor, stamina/strength/health stats,
        // fire resistance.
        (
            1001,
            ItemPrototype {
                item_id: 1001,
                class: 4,
                armor: 200,
                fire_res: 7,
                stats: stats(&[(7, 10), (4, 6), (1, 50)]),
                ..ItemPrototype::default()
            },
        ),
        // One-hand sword: agility, 10-20 at 2.6s.
        (
            1002,
            ItemPrototype {
                item_id: 1002,
                class: 2,
                inventory_type: 13,
                delay: 2600,
                damages: damages(&[(10.0, 20.0)]),
                stats: stats(&[(3, 4)]),
                ..ItemPrototype::default()
            },
        ),
        // Shield: armor, block, stamina.
        (
            1003,
            ItemPrototype {
                item_id: 1003,
                class: 4,
                armor: 300,
                block: 25,
                stats: stats(&[(7, 5)]),
                ..ItemPrototype::default()
            },
        ),
        // Helm worn broken in the load: must not apply.
        (
            1004,
            ItemPrototype {
                item_id: 1004,
                class: 4,
                armor: 999,
                max_durability: 50,
                stats: stats(&[(4, 99)]),
                ..ItemPrototype::default()
            },
        ),
    ]))
}

fn item(item_id: u32, item_guid: u32, durability: u16) -> ItemInstanceRow {
    ItemInstanceRow {
        creator_guid: 0,
        gift_creator_guid: 0,
        count: 1,
        duration: 0,
        charges: "0 0 0 0 0 ".into(),
        flags: 0,
        enchantments: String::new(),
        random_property_id: 0,
        durability,
        text: 0,
        item_guid,
        item_id,
        generated_loot: 0,
    }
}

fn row() -> CharacterRow {
    CharacterRow {
        guid: 2,
        account: 42,
        name: "Geartest".into(),
        race: 2,
        class: 1,
        gender: 0,
        level: 10,
        xp: 0,
        money: 0,
        skin: 1,
        face: 2,
        hair_style: 3,
        hair_color: 4,
        facial_hair: 5,
        bank_bag_slots: 0,
        character_flags: 0,
        position_x: -618.52,
        position_y: -4251.67,
        position_z: 38.72,
        map: 1,
        orientation: 1.5,
        known_taxi_mask: String::new(),
        played_time_total: 60,
        played_time_level: 60,
        rest_bonus: 0.0,
        logout_time: 0,
        reset_talents_multiplier: 0,
        reset_talents_time: 0,
        transport_guid: 0,
        transport_x: 0.0,
        transport_y: 0.0,
        transport_z: 0.0,
        transport_o: 0.0,
        extra_flags: 0,
        stable_slots: 0,
        death_expire_time: 0,
        current_taxi_path: String::new(),
        honor_rank_points: 0.0,
        honor_highest_rank: 0,
        honor_standing: 0,
        honor_last_week_hk: 0,
        honor_last_week_cp: 0.0,
        honor_stored_hk: 0,
        honor_stored_dk: 0,
        watched_faction: -1,
        drunk: 0,
        health: 9999,
        power1: 0,
        power2: 0,
        power3: 0,
        power4: 0,
        power5: 0,
        explored_zones: String::new(),
        ammo_id: 0,
        action_bars: 0,
        world_phase_mask: 0,
        create_time: 0,
        instance: 0,
        zone: 0,
    }
}

fn load() -> CharacterLoad {
    CharacterLoad {
        row: row(),
        group_id: None,
        bound_instances: vec![],
        auras: vec![],
        spells: vec![],
        quest_status: vec![],
        honor_cp: vec![],
        reputation: vec![],
        inventory: vec![
            // Head, broken (durability 0 of 50).
            InventoryRow {
                item: item(1004, 9001, 0),
                bag: 0,
                slot: 0,
            },
            // Chest.
            InventoryRow {
                item: item(1001, 9002, 40),
                bag: 0,
                slot: 4,
            },
            // Main hand sword.
            InventoryRow {
                item: item(1002, 9003, 40),
                bag: 0,
                slot: 15,
            },
            // Offhand shield.
            InventoryRow {
                item: item(1003, 9004, 40),
                bag: 0,
                slot: 16,
            },
            // In a bag: never applies.
            InventoryRow {
                item: item(1004, 9005, 40),
                bag: 777,
                slot: 0,
            },
        ],
        item_loot: vec![],
        actions: vec![],
        social: vec![],
        homebind: None,
        spell_cooldowns: vec![],
        guild: None,
        bg_data: None,
        account_data: vec![],
        skills: vec![],
        mails: vec![],
        mailed_items: vec![],
        forgotten_skills: vec![],
        pets: Vec::new(),
    }
}

#[test]
fn equipped_items_feed_stat_pipeline() {
    let infos = infos();
    let protos = prototypes();
    let inputs = MaterializeInputs {
        infos: &infos,
        race: None,
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
        skills: None,
        item_prototypes: Some(&protos),
        quest_templates: None,
    };
    let player = Player::materialize(load(), &inputs).expect("materialize");
    let unit = &player.unit;

    // Create stats are 20 each; items add STR+6,
    // AGI+4, STA+15 (broken helm and bag copy add
    // nothing).
    assert_eq!(unit.stat(0), 26.0); // strength
    assert_eq!(unit.stat(1), 24.0); // agility
    assert_eq!(unit.stat(2), 35.0); // stamina
    assert_eq!(unit.stat(3), 20.0); // intellect
    assert_eq!(unit.stat(4), 20.0); // spirit

    let v = unit.world.object.values();
    assert_eq!(v.get_f32(offsets::PLAYER_FIELD_POSSTAT0), 6.0);
    assert_eq!(v.get_f32(offsets::PLAYER_FIELD_POSSTAT0 + 2), 15.0);
    assert_eq!(v.get_f32(offsets::PLAYER_FIELD_NEGSTAT0), 0.0);

    // Armor: 200 + 300 + agility 24 * 2.
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RESISTANCES), 548);
    // Fire resistance 7; holy is written 0 by the fork.
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RESISTANCES + 2), 7);
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RESISTANCES + 1), 0);

    // Max health: create 20 + item health 50 +
    // stamina bonus (20 + 15*10). Max mana: create
    // 100 + intellect bonus 20.
    assert_eq!(unit.max_health(), 240);
    assert_eq!(unit.max_power(0), 120);
    // Saved health clamps to the geared max.
    assert_eq!(unit.health(), 240);

    // Warrior L10 AP: melee 10*3 + 26*2 - 20 = 62;
    // ranged 10 + 24 - 10 = 24.
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_ATTACK_POWER), 62);
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RANGED_ATTACK_POWER), 24);

    // Weapon fields: AP/14 * 2.6 + (10, 20).
    let ap_term = 62.0f32 / 14.0 * 2.6;
    let min = v.get_f32(offsets::UNIT_FIELD_MINDAMAGE);
    let max = v.get_f32(offsets::UNIT_FIELD_MAXDAMAGE);
    assert!((min - (ap_term + 10.0)).abs() < 1e-4, "{min}");
    assert!((max - (ap_term + 20.0)).abs() < 1e-4, "{max}");
    assert_eq!(v.get_f32(offsets::UNIT_FIELD_BASEATTACKTIME), 2600.0);
    // Ranged: fists (1, 2) at 2.0s with ranged AP.
    let rterm = 24.0f32 / 14.0 * 2.0;
    let rmin = v.get_f32(offsets::UNIT_FIELD_MINRANGEDDAMAGE);
    assert!((rmin - (rterm + 1.0)).abs() < 1e-4, "{rmin}");
    // No dual wield: offhand fields stay zero.
    assert_eq!(v.get_f32(offsets::UNIT_FIELD_MINOFFHANDDAMAGE), 0.0);

    assert_eq!(player.shield_block_flat, 25.0);
    assert!(!player.has_offhand_weapon);
    assert_eq!(unit.base_weapon_damage[0], (10.0, 20.0));
}
