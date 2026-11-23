//! `m_CastItem` threading parity slice (fork `Spell::m_CastItem`):
//! the script-effect lane carries the cast item to the arms
//! that need it, fork-exactly (`spell_shaman.cpp:20-41`,
//! `spell_item.cpp:360-397,510-519`).
//!
//! Fork pins (`vmangos/src`): `scripts/spells/spell_shaman.cpp:22-41`
//! (Flametongue weapon-speed-scaled 10444 proc, missing item
//! logs and skips), `scripts/spells/spell_item.cpp:362-376`
//! (Jumper Cables 67% backfire 8338 with the item),
//! `:383-397` (XL 50% backfire 23055 with the item),
//! `:510-519` (First Aid 11196 with the item).

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::{dmg_class, effect, power};
use mangos_spells::entry::SpellEffectData;
use mangos_spells::entry::SpellEntry;
use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
use mangos_world_data::item_prototype::ItemPrototype;
use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};

use crate::WorldRng;
use crate::map::{Map, MapConfig};
use crate::spells::{CastItemCtx, CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;

/// Flametongue-proc stand-in (fork 8026 family).
const PROC_SPELL: u32 = 9401;
/// Fork `10444` proc hit.
const PROC_HIT_SPELL: u32 = 10_444;
/// Weapon stand-in: 2000 ms delay.
const WEAPON_ENTRY: u32 = 9400;
/// Jumper Cables stand-in (fork 8342).
const JUMPER_SPELL: u32 = 8342;
/// Jumper backfire stand-in (fork 8338).
const BACKFIRE_SPELL: u32 = 8338;
/// Jumper XL stand-in (fork 22999).
const JUMPER_XL_SPELL: u32 = 22_999;
/// XL backfire stand-in (fork 23055).
const BACKFIRE_XL_SPELL: u32 = 23_055;
/// First Aid stand-in (fork 746).
const FIRST_AID_SPELL: u32 = 746;
/// Recently Bandaged stand-in (fork 11196).
const BANDAGE_SPELL: u32 = 11_196;
/// Creature template entry for victims.
const VICTIM_ENTRY: u32 = 9402;
/// `TARGET_UNIT` implicit target (see insignia fixture).
const TARGET_UNIT: u32 = 25;

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
            name: format!("CastItem{guid}"),
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
        max_money: MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
        rest_rates: RestRates::default(),
        skills: None,
        item_prototypes: None,
        quest_templates: None,
        taxi: None,
    };
    Player::materialize(test_load(guid), &inputs).expect("materialize")
}

fn unit_target_effect(effect_id: u32, base_points: i32) -> SpellEffectData {
    SpellEffectData {
        effect: effect_id,
        implicit_target_a: TARGET_UNIT,
        base_points,
        dmg_multiplier: 1.0,
        ..Default::default()
    }
}

#[allow(clippy::too_many_lines)] // one spell/item stand-in table per arm
fn cast_item_stores() -> WorldStores {
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
    stores.item_prototypes.insert_for_tests(ItemPrototype {
        item_id: WEAPON_ENTRY,
        delay: 2000,
        ..Default::default()
    });
    // Flametongue proc: DUMMY, fire school, 500 base damage.
    stores.spells.spells.insert(
        PROC_SPELL,
        SpellEntry {
            id: PROC_SPELL,
            school: 2,
            script_name: "spell_shaman_flametongue_proc_dummy".to_string(),
            effects: [
                unit_target_effect(effect::DUMMY, 500),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // 10444 hit: melee class so the lane never crits.
    stores.spells.spells.insert(
        PROC_HIT_SPELL,
        SpellEntry {
            id: PROC_HIT_SPELL,
            dmg_class: dmg_class::MELEE,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Jumper Cables pair: backfire costs 100 mana.
    stores.spells.spells.insert(
        JUMPER_SPELL,
        SpellEntry {
            id: JUMPER_SPELL,
            script_name: "spell_goblin_jumper_cables".to_string(),
            effects: [
                unit_target_effect(effect::DUMMY, 0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.spells.spells.insert(
        BACKFIRE_SPELL,
        SpellEntry {
            id: BACKFIRE_SPELL,
            power_type: power::MANA,
            mana_cost: 100,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.spells.spells.insert(
        JUMPER_XL_SPELL,
        SpellEntry {
            id: JUMPER_XL_SPELL,
            script_name: "spell_goblin_jumper_cables_xl".to_string(),
            effects: [
                unit_target_effect(effect::DUMMY, 0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.spells.spells.insert(
        BACKFIRE_XL_SPELL,
        SpellEntry {
            id: BACKFIRE_XL_SPELL,
            power_type: power::MANA,
            mana_cost: 100,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // First Aid pair: bandage costs 50 mana.
    stores.spells.spells.insert(
        FIRST_AID_SPELL,
        SpellEntry {
            id: FIRST_AID_SPELL,
            script_name: "spell_first_aid".to_string(),
            effects: [
                unit_target_effect(effect::DUMMY, 0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.spells.spells.insert(
        BANDAGE_SPELL,
        SpellEntry {
            id: BANDAGE_SPELL,
            power_type: power::MANA,
            mana_cost: 50,
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
    let stores = Arc::new(cast_item_stores());
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

fn cast_options_with_item() -> CastOptions {
    CastOptions {
        cast_item: Some(CastItemCtx {
            item_guid: 555,
            item_entry: WEAPON_ENTRY,
        }),
        ..cast_options()
    }
}

fn victim_health(map: &Map, victim: ObjectGuid) -> u32 {
    map.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH)
}

#[test]
fn flametongue_with_item_deals_speed_scaled_damage() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(11);
    let before = victim_health(&map, victim);
    let outcome = map.cast_spell(
        caster,
        PROC_SPELL,
        CastTarget::Unit(victim),
        cast_options_with_item(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    // (500 + 3.85 * 0) * 0.01 * 2.0 s = 10.0 exactly;
    // dither of an integer value is exact.
    assert_eq!(
        before - victim_health(&map, victim),
        10,
        "weapon-delay-scaled proc damage must land"
    );
}

#[test]
fn flametongue_without_item_skips_effect() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(12);
    let before = victim_health(&map, victim);
    let outcome = map.cast_spell(caster, PROC_SPELL, CastTarget::Unit(victim), cast_options());
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        victim_health(&map, victim),
        before,
        "missing cast item logs and skips the proc"
    );
}

#[test]
fn flametongue_unknown_item_skips_effect() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(13);
    let before = victim_health(&map, victim);
    let options = CastOptions {
        cast_item: Some(CastItemCtx {
            item_guid: 555,
            item_entry: 999_999,
        }),
        ..cast_options()
    };
    let outcome = map.cast_spell(caster, PROC_SPELL, CastTarget::Unit(victim), options);
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        victim_health(&map, victim),
        before,
        "item without a prototype cannot supply a delay"
    );
}

#[test]
fn jumper_backfire_forwards_cast_item_keeps_mana() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(18);
    map.unit_set_power(caster, power::MANA, 1000);
    let outcome = map.cast_spell(
        caster,
        JUMPER_SPELL,
        CastTarget::Unit(victim),
        cast_options_with_item(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    // Seed 18 backfires (67%, verified pre-fix by the 900-mana
    // failure): the sub-cast then either rides the item (no
    // power cost) or pays 100 mana.
    assert_eq!(
        map.unit_power(caster, power::MANA),
        1000,
        "item-carried backfire costs no mana"
    );
}

#[test]
fn jumper_xl_backfire_forwards_cast_item_keeps_mana() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(15);
    map.unit_set_power(caster, power::MANA, 1000);
    let outcome = map.cast_spell(
        caster,
        JUMPER_XL_SPELL,
        CastTarget::Unit(victim),
        cast_options_with_item(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    // Seed 15 backfires (50%, verified pre-fix by the 900-mana
    // failure): same item-carry expectation.
    assert_eq!(
        map.unit_power(caster, power::MANA),
        1000,
        "item-carried XL backfire costs no mana"
    );
}

#[test]
fn first_aid_forwards_cast_item_keeps_mana() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(16);
    map.unit_set_power(caster, power::MANA, 1000);
    let outcome = map.cast_spell(
        caster,
        FIRST_AID_SPELL,
        CastTarget::Unit(victim),
        cast_options_with_item(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        map.unit_power(caster, power::MANA),
        1000,
        "item-carried bandage costs no mana"
    );
}
