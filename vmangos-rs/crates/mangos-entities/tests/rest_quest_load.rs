//! Load-time offline rest accrual and quest timer
//! conversion at materialize (Phase 6 Slice 1b):
//! `ComputeRest`/`SetRestBonus`, `_LoadQuestStatus`
//! timer semantics, and the `AdjustQuestReqItemCount`
//! recount that marks deliver quests changed.
#![allow(clippy::float_cmp)]

use std::collections::HashMap;

use mangos_db::character_load::{
    CharacterLoad, CharacterRow, InventoryRow, ItemInstanceRow, QuestStatusRow,
};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::item_prototype::{ItemPrototype, ItemPrototypes};
use mangos_world_data::player_info::{
    PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
};
use mangos_world_data::quest_template::{QuestTemplate, QuestTemplates};

const NOW: u64 = 1_700_000_000;

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
        // Next-level XP 400 at every level: rest math
        // 86400 * 400 / 1152000 = 30 exactly, cap 300.
        vec![400; 61],
    )
}

fn row() -> CharacterRow {
    CharacterRow {
        guid: 2,
        account: 42,
        name: "Resttest".into(),
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
        // CHARACTER_FLAG_RESTING (0x2): logged out resting.
        character_flags: 2,
        position_x: -618.52,
        position_y: -4251.67,
        position_z: 38.72,
        map: 1,
        orientation: 1.5,
        known_taxi_mask: String::new(),
        played_time_total: 60,
        played_time_level: 60,
        rest_bonus: 5.0,
        logout_time: NOW - 86_400,
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

fn quest_row(quest: u32, status: i32, rewarded: u8, timer: u64) -> QuestStatusRow {
    QuestStatusRow {
        quest,
        status,
        rewarded,
        explored: 0,
        timer,
        mob_count: [0; 4],
        item_count: [9, 0, 0, 0],
        reward_choice: 0,
    }
}

fn templates() -> QuestTemplates {
    let deliver = QuestTemplate {
        entry: 100,
        req_item_ids: [182, 0, 0, 0],
        req_item_counts: [4, 0, 0, 0],
        ..QuestTemplate::default()
    };
    let plain = QuestTemplate {
        entry: 101,
        ..QuestTemplate::default()
    };
    let timed = QuestTemplate {
        entry: 102,
        limit_time: 600,
        ..QuestTemplate::default()
    };
    let timed2 = QuestTemplate {
        entry: 103,
        limit_time: 600,
        ..QuestTemplate::default()
    };
    QuestTemplates::from_entries([deliver, plain, timed, timed2])
}

fn load() -> CharacterLoad {
    CharacterLoad {
        row: row(),
        group_id: None,
        bound_instances: vec![],
        auras: vec![],
        spells: vec![],
        quest_status: vec![
            quest_row(100, 3, 0, 111),
            quest_row(101, 3, 0, 222),
            quest_row(102, 3, 0, NOW + 500),
            quest_row(103, 3, 0, NOW - 10),
            quest_row(104, 1, 1, 333),
        ],
        honor_cp: vec![],
        reputation: vec![],
        inventory: vec![InventoryRow {
            item: ItemInstanceRow {
                creator_guid: 0,
                gift_creator_guid: 0,
                count: 3,
                duration: 0,
                charges: "0 0 0 0 0 ".into(),
                flags: 0,
                enchantments: String::new(),
                random_property_id: 0,
                durability: 10,
                text: 0,
                item_guid: 9001,
                item_id: 182,
                generated_loot: 0,
            },
            bag: 777,
            slot: 0,
        }],
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

fn materialize() -> Player {
    let infos = infos();
    let templates = templates();
    let protos = ItemPrototypes::from_entries_for_tests(HashMap::from([(
        182,
        ItemPrototype {
            item_id: 182,
            ..ItemPrototype::default()
        },
    )]));
    let inputs = MaterializeInputs {
        infos: &infos,
        race: None,
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: NOW,
        skills: None,
        item_prototypes: Some(&protos),
        quest_templates: Some(&templates),
    };
    Player::materialize(load(), &inputs).expect("materialize")
}

#[test]
fn offline_rest_accrues_at_load() {
    let player = materialize();
    // 5 stored + 86400s rested-place gain of 30.
    assert_eq!(player.rest_bonus, 35.0);
    let v = player.unit.world.object.values();
    // >10 flips the rest-state byte to RESTED (0x01).
    assert_eq!(v.get_byte(offsets::PLAYER_BYTES_2, 3), 0x01);
}

#[test]
fn deliver_recount_and_timer_conversion() {
    let player = materialize();
    let q = |id: u32| {
        player
            .deferred
            .quest_status
            .iter()
            .find(|row| row.quest == id)
            .expect("quest row")
    };
    // Deliver quest: stored count 9 recounted against
    // inventory (3 of item 182, min with required 4),
    // marked changed for the fork's unconditional rewrite.
    assert_eq!(q(100).item_count[0], 3);
    assert!(player.quest_load_changed.contains(&100));
    // Non-deliver quest untouched, not marked.
    assert_eq!(q(101).item_count[0], 9);
    assert!(!player.quest_load_changed.contains(&101));
    // Timed active quests: absolute deadline becomes a
    // remaining-ms live timer; expired becomes 1ms.
    let t102 = player.quest_timers[&102];
    assert!(t102.timed_active);
    assert_eq!(t102.remaining_ms_at_load, 500_000);
    let t103 = player.quest_timers[&103];
    assert!(t103.timed_active);
    assert_eq!(t103.remaining_ms_at_load, 1);
    // Non-timed / no-template quests carry a zero
    // remainder and are not timed-active.
    assert!(!player.quest_timers[&100].timed_active);
    assert_eq!(player.quest_timers[&100].remaining_ms_at_load, 0);
    assert!(!player.quest_timers.contains_key(&104));
    // Baseline keeps as-loaded rows: the recount only
    // mutated the live (deferred) side.
    assert_eq!(
        player
            .saved
            .quest_status
            .iter()
            .find(|row| row.quest == 100)
            .expect("baseline row")
            .item_count[0],
        9
    );
    assert_eq!(player.load_now_unix, NOW);
}
