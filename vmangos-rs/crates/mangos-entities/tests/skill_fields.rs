//! `_LoadSkills` field population (Phase 5 B.2a):
//! `PLAYER_SKILL_INFO_1_1` triplets from saved skill
//! rows with C++ validation and range fixups.

use std::collections::{HashMap, HashSet};

use mangos_db::character_load::{CharacterLoad, CharacterRow, SkillRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::player_info::{
    PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
};
use mangos_world_data::skill_info::{RaceClassSkill, SKILL_CATEGORY_LANGUAGES, SkillDbc};

fn orc_warrior_load(skills: Vec<SkillRow>) -> CharacterLoad {
    CharacterLoad {
        row: CharacterRow {
            guid: 2,
            account: 42,
            name: "Skilltest".into(),
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
            health: 100,
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
        },
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
        skills,
        mails: vec![],
        mailed_items: vec![],
        forgotten_skills: vec![],
    }
}

fn orc_infos() -> PlayerInfos {
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
                    basemana: 0,
                };
                60
            ],
        )]),
        HashMap::from([((2, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
        vec![400; 61],
    )
}

fn skill_dbc() -> SkillDbc {
    SkillDbc {
        // 109 Orcish (languages), 43 swords (weapon),
        // 95 defense (level range), 98 Common (languages).
        categories: HashMap::from([
            (109, SKILL_CATEGORY_LANGUAGES),
            (98, SKILL_CATEGORY_LANGUAGES),
            (43, 6),
            (95, 11),
        ]),
        race_class: vec![
            RaceClassSkill {
                skill_id: 109,
                // Horde races 2,5,6,8 -> bits 1,4,5,7.
                race_mask: 0b1011_0010,
                class_mask: 0,
                ..Default::default()
            },
            RaceClassSkill {
                skill_id: 98,
                // Alliance races 1,3,4,7 -> bits 0,2,3,6.
                race_mask: 0b0100_1101,
                class_mask: 0,
                ..Default::default()
            },
            RaceClassSkill {
                skill_id: 43,
                race_mask: 0,
                class_mask: 0,
                ..Default::default()
            },
            RaceClassSkill {
                skill_id: 95,
                race_mask: 0,
                class_mask: 0,
                ..Default::default()
            },
        ],
        tiers: HashSet::new(),
    }
}

fn skill_field(player: &Player, slot: u16) -> (u32, u32) {
    let v = player.unit.world.object.values();
    let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
    (v.get_u32(base), v.get_u32(base + 1))
}

#[test]
fn skill_fields_load_with_cpp_fixups() {
    let infos = orc_infos();
    let dbc = skill_dbc();
    let load = orc_warrior_load(vec![
        // Language: forced 300/300 regardless of saved.
        SkillRow {
            skill: 109,
            value: 1,
            max: 1,
        },
        // Level range: max becomes level*5 = 50.
        SkillRow {
            skill: 95,
            value: 37,
            max: 300,
        },
        // Forbidden for orc (alliance language): skipped.
        SkillRow {
            skill: 98,
            value: 300,
            max: 300,
        },
        // Unknown skill: skipped.
        SkillRow {
            skill: 9999,
            value: 10,
            max: 10,
        },
        // Zero value: skipped.
        SkillRow {
            skill: 43,
            value: 0,
            max: 50,
        },
    ]);
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
        skills: Some(&dbc),
        item_prototypes: None,
        quest_templates: None,
    };
    let player = Player::materialize(load, &inputs).expect("materialize");

    assert_eq!(skill_field(&player, 0), (109, 0x012C | (0x012C << 16)));
    assert_eq!(skill_field(&player, 1), (95, 0x25 | (0x32 << 16)));
    assert_eq!(skill_field(&player, 2), (0, 0));
    // Tail slots are explicitly zeroed to PLAYER_MAX_SKILLS.
    assert_eq!(skill_field(&player, 126), (0, 0));
}
