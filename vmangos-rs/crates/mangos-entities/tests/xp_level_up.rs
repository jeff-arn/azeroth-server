//! XP gain and level-up runtime (depth-gap Slice 1):
//! `GiveXP` gates and rest-bonus consumption,
//! `GiveLevel` stat growth against the create
//! baseline, talent-point delta, and the multi-level
//! loop, over synthetic `PlayerInfos` where stat
//! growth is +1/level and base health/mana grow by
//! 10/5 per level, so every delta is exact.
#![allow(clippy::float_cmp)]

use std::collections::HashMap;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::player_info::{
    PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
};

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
            (1..=60)
                .map(|l| PlayerClassLevelInfo {
                    basehealth: 100 + l * 10,
                    basemana: 50 + l * 5,
                })
                .collect::<Vec<_>>(),
        )]),
        HashMap::from([(
            (2, 1),
            (1..=60)
                .map(|l| PlayerLevelInfo { stats: [10 + l; 5] })
                .collect::<Vec<_>>(),
        )]),
        // 400 XP per level at every level.
        vec![400; 61],
    )
}

fn row(level: u8) -> CharacterRow {
    CharacterRow {
        guid: 2,
        account: 42,
        name: "Xptest".into(),
        race: 2,
        class: 1,
        gender: 0,
        level,
        xp: 350,
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
        rest_bonus: 100.0,
        // No offline accrual in these tests.
        logout_time: NOW,
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
        health: 10,
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

fn materialize(level: u8) -> Player {
    let infos = infos();
    let load = CharacterLoad {
        row: row(level),
        group_id: None,
        bound_instances: vec![],
        auras: vec![],
        spells: vec![],
        quest_status: vec![],
        honor_cp: vec![],
        reputation: vec![],
        inventory: vec![],
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
    };
    let inputs = MaterializeInputs {
        infos: &infos,
        race: None,
        // Rage class: levelup mana delta must read 0.
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: NOW,
        skills: None,
        item_prototypes: None,
        quest_templates: None,
    };
    Player::materialize(load, &inputs).expect("materialize")
}

#[test]
fn kill_xp_consumes_rest_and_dings() {
    let infos = infos();
    let mut player = materialize(9);
    // 100 kill XP + 100 rested (stored 100, cap = xp)
    // = 350 + 200 = 550, ding at 400.
    let outcome = player
        .give_xp(100, true, &infos, None, None)
        .expect("xp granted");
    assert_eq!(outcome.given_xp, 100);
    assert_eq!(outcome.rest_xp, 100);
    assert_eq!(outcome.level_ups.len(), 1);
    let up = outcome.level_ups[0];
    assert_eq!(up.level, 10);
    // Base health +10, stamina bonus 19 -> 20 = +1.
    assert_eq!(up.health_gain, 11);
    // Rage class: packet mana delta is 0.
    assert_eq!(up.mana_gain, 0);
    assert_eq!(up.stat_gains, [1; 5]);

    let v = player.unit.world.object.values();
    assert_eq!(v.get_u32(offsets::PLAYER_XP), 150);
    assert_eq!(v.get_u32(offsets::PLAYER_NEXT_LEVEL_XP), 400);
    // First talent point lands at level 10.
    assert_eq!(v.get_u32(offsets::PLAYER_CHARACTER_POINTS1), 1);
    assert_eq!(player.unit.level(), 10);
    assert_eq!(player.rest_bonus, 0.0);
    assert_eq!(player.played_time_level, 0);
    // Create baseline re-based to the new level.
    assert_eq!(player.unit.create_stats[0], 20.0);
    assert_eq!(player.unit.create_health, 200);
    // Refill to the new max (create 200 + stamina
    // bonus 20, no items).
    assert_eq!(player.unit.max_health(), 220);
    assert_eq!(player.unit.health(), 220);
}

#[test]
fn non_kill_xp_keeps_rest_and_no_ding() {
    let infos = infos();
    let mut player = materialize(9);
    let outcome = player
        .give_xp(40, false, &infos, None, None)
        .expect("xp granted");
    assert_eq!(outcome.given_xp, 40);
    assert_eq!(outcome.rest_xp, 0);
    assert!(outcome.level_ups.is_empty());
    assert_eq!(player.rest_bonus, 100.0);
    let v = player.unit.world.object.values();
    assert_eq!(v.get_u32(offsets::PLAYER_XP), 390);
    assert_eq!(player.unit.level(), 9);
}

#[test]
fn multi_level_gain_loops() {
    let infos = infos();
    let mut player = materialize(9);
    // 350 + 1000 = 1350: dings at 400/800/1200.
    let outcome = player
        .give_xp(1000, false, &infos, None, None)
        .expect("xp granted");
    assert_eq!(outcome.level_ups.len(), 3);
    assert_eq!(outcome.level_ups[2].level, 12);
    let v = player.unit.world.object.values();
    assert_eq!(v.get_u32(offsets::PLAYER_XP), 150);
    assert_eq!(v.get_u32(offsets::PLAYER_CHARACTER_POINTS1), 3);
    assert_eq!(player.unit.level(), 12);
}

#[test]
fn max_level_grants_nothing() {
    let infos = infos();
    let mut player = materialize(60);
    assert!(player.give_xp(100, true, &infos, None, None).is_none());
    // Stored rest bonus is inert at max level: the
    // fork only zeroes it when SetRestBonus runs
    // (offline accrual), and GiveXP returns earlier.
    assert_eq!(player.rest_bonus, 100.0);
}
