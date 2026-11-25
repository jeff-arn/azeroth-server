//! Mixed unit-target + area-effect targeting (fork `Spell.cpp`
//! `SetTargetMap` + `PushTarget` vs `FillAreaTargets`):
//! the explicit unit target bypasses the fill-time
//! `SpellScript::OnCheckTarget` gate, so a unit-targeted effect
//! still lands on it, while area effects keep excluding it.
//!
//! Fork pins (`vmangos/src`): `scripts/spells/spell_warrior.cpp:20-29`
//! (Intimidating Shout 5246: the `AoE` fear does not apply to the
//! spell main target, stunned by the 20511 trigger instead).
//! Regression: the merged victim list dropped the main target in
//! `collect_targets`, so 5246 landed `SPELL_GO` with no observable
//! outcome on the dummy (the trigger never fired).
//!
//! Stand-ins: `MAIN_SPELL` (5246 shape: trigger on the hostile unit
//! target plus an areaselect fear) and `TRIGGER_SPELL` (20511 shape:
//! direct damage on the unit target).

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MaterializeInputs, Player, RestRates};
use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::{dmg_class, effect, target};
use mangos_spells::entry::SpellEffectData;
use mangos_spells::entry::SpellEntry;
use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};

use crate::WorldRng;
use crate::map::{Map, MapConfig};
use crate::spells::{CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;

/// 5246 stand-in: trigger + areaselect fear, gated by the
/// `spell_warrior_intimidating_shout` exemption script.
const MAIN_SPELL: u32 = 9452;
/// 20511 stand-in: direct damage on the explicit unit target.
const TRIGGER_SPELL: u32 = 9453;
/// Creature template entry for the dummy.
const VICTIM_ENTRY: u32 = 9454;
/// Fear aura stand-in (`7` is the real fear aura id).
const FEAR_AURA: u32 = 7;
/// Fear areaselect mode (`22` at-caster, the 5246 shape).
const AREASELECT_AT_CASTER: u32 = 22;

fn test_infos() -> mangos_world_data::player_info::PlayerInfos {
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };
    PlayerInfos::from_maps_for_tests(
        60,
        HashMap::from([(
            (1, 1),
            PlayerCreateInfo {
                map_id: 0,
                area_id: 14,
                position_x: -8948.0,
                position_y: -130.0,
                position_z: 83.5,
                orientation: 0.0,
                display_id_m: 51,
                display_id_f: 52,
            },
        )]),
        HashMap::from([(
            1,
            vec![
                PlayerClassLevelInfo {
                    basehealth: 100,
                    basemana: 100
                };
                60
            ],
        )]),
        HashMap::from([((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
        vec![400; 61],
    )
}

fn test_load(guid: u32) -> CharacterLoad {
    CharacterLoad {
        row: CharacterRow {
            guid,
            account: 42,
            name: format!("Shout{guid}"),
            race: 1,
            class: 1,
            gender: 0,
            level: 60,
            xp: 0,
            money: 0,
            skin: 1,
            face: 2,
            hair_style: 3,
            hair_color: 4,
            facial_hair: 5,
            bank_bag_slots: 0,
            character_flags: 0,
            position_x: -8948.0,
            position_y: -130.0,
            position_z: 83.5,
            map: 0,
            orientation: 0.0,
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
            power1: 100,
            power2: 0,
            power3: 0,
            power4: 0,
            power5: 0,
            explored_zones: String::new(),
            ammo_id: 0,
            action_bars: 0,
            world_phase_mask: 0xFFFF_FFFF,
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
        skills: vec![],
        mails: vec![],
        mailed_items: vec![],
        forgotten_skills: vec![],
        pets: Vec::new(),
    }
}

fn materialize(guid: u32, infos: &mangos_world_data::player_info::PlayerInfos) -> Player {
    let inputs = MaterializeInputs {
        infos,
        race: None,
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: mangos_entities::player::MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
        rest_rates: RestRates::default(),
        skills: None,
        item_prototypes: None,
        quest_templates: None,
        taxi: None,
    };
    Player::materialize(test_load(guid), &inputs).expect("materialize")
}

fn shout_stores() -> WorldStores {
    let mut stores = WorldStores::empty();
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![CreatureInfo {
        entry: VICTIM_ENTRY,
        display_id: [100, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    }]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 100,
            ..Default::default()
        }]);
    // Trigger: direct damage on the explicit unit target.
    stores.spells.spells.insert(
        TRIGGER_SPELL,
        SpellEntry {
            id: TRIGGER_SPELL,
            dmg_class: dmg_class::MAGIC,
            effects: [
                SpellEffectData {
                    effect: effect::SCHOOL_DAMAGE,
                    implicit_target_a: target::UNIT,
                    base_points: 9,
                    dmg_multiplier: 1.0,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Main: trigger on the hostile unit target plus an areaselect
    // fear, under the main-target exemption script.
    stores.spells.spells.insert(
        MAIN_SPELL,
        SpellEntry {
            id: MAIN_SPELL,
            script_name: "spell_warrior_intimidating_shout".to_string(),
            effects: [
                SpellEffectData {
                    effect: effect::TRIGGER_SPELL,
                    implicit_target_a: target::UNIT_ENEMY,
                    trigger_spell: TRIGGER_SPELL,
                    ..Default::default()
                },
                SpellEffectData {
                    effect: effect::APPLY_AURA,
                    implicit_target_a: AREASELECT_AT_CASTER,
                    apply_aura_name: FEAR_AURA,
                    ..Default::default()
                },
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores
}

struct Fixture {
    map: Map,
    caster: ObjectGuid,
    victim: ObjectGuid,
}

fn setup(seed: u64) -> Fixture {
    let infos = test_infos();
    let stores = Arc::new(shout_stores());
    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(seed),
    );
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    let caster = materialize(7, &infos).unit.guid();
    assert!(caster.is_player());
    map.add_player(materialize(7, &infos), &respawns, 1_800_000_000, 0);
    let pos = map.unit_position(caster).expect("caster positioned");
    let victim = map
        .summon_creature(VICTIM_ENTRY, pos, caster, 0, 0)
        .expect("summon victim");
    let _ = map.drain_events();
    Fixture {
        map,
        caster,
        victim,
    }
}

fn cast_options() -> CastOptions {
    CastOptions {
        triggered: false,
        triggered_by_aura: false,
        skip_known: true,
        cast_item: None,
        custom_base_points: [None; 3],
        triggered_by_aura_spell: 0,
        triggered_by_spell: 0,
    }
}

fn victim_health(map: &Map, victim: ObjectGuid) -> u32 {
    map.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH)
}

#[test]
fn mixed_unit_trigger_lands_on_script_exempt_main_target() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(11);
    let before = victim_health(&map, victim);
    let outcome = map.cast_spell(caster, MAIN_SPELL, CastTarget::Unit(victim), cast_options());
    assert_eq!(outcome, CastOutcome::Started);
    // The unit-targeted trigger bypasses the fill-time exemption
    // script (fork `PushTarget`): its damage must land even though
    // the script rejects the main target for the area fill.
    assert!(
        victim_health(&map, victim) < before,
        "unit trigger must damage the exempt main target"
    );
}

#[test]
fn mixed_area_fear_skips_script_exempt_main_target() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(11);
    let outcome = map.cast_spell(caster, MAIN_SPELL, CastTarget::Unit(victim), cast_options());
    assert_eq!(outcome, CastOutcome::Started);
    // The area fear honors the exemption per effect (fork
    // per-effect `CheckTarget`): the stunned main target is never
    // feared.
    assert!(
        !map.unit_has_aura(victim, MAIN_SPELL),
        "area fear must not stick to the exempt main target"
    );
}
