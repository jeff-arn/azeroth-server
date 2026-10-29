//! Tier 3 Slice 2 batch 5 (Ruins of Ahn'Qiraj,
//! Molten Core, Blackrock Spire, Temple of
//! Ahn'Qiraj) live evidence on the dev bench.
//! Expected values from independent SQL + fork cpp.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::{Map, MapConfig, WorldRng, WorldStores};
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::creature_spells::load_creature_spells;
use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
use mangos_world_data::item_prototype::load_item_prototypes_lite;
use mangos_world_data::load_report::LoadReport;
use mangos_world_data::player_info::{RaceClassDbc, RaceDbc, load_player_infos};
use mangos_world_data::script_engine::load_script_stores;
use mangos_world_data::spawn_support::{
    RespawnTimes, load_class_level_stats, load_creature_addons, load_display_info_addons,
    load_equipment_templates,
};
use mangos_world_data::spawns::{SpawnData, load_map_ids};
use mangos_world_data::spell_template::load_spell_templates;
use mangos_world_data::waypoints::load_waypoints;

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        other => panic!("field {idx}: expected UInt, got {other:?}"),
    }
}

fn race_class_dbc(dir: &Path) -> RaceClassDbc {
    let mut dbc = RaceClassDbc::default();
    let bytes = std::fs::read(dir.join(formats::CHR_RACES.file)).expect("ChrRaces.dbc");
    let store = DbcFile::parse(&bytes, formats::CHR_RACES.fmt)
        .expect("parse")
        .produce(formats::CHR_RACES.fmt)
        .expect("produce");
    for row in store.rows() {
        dbc.races.insert(
            uint(row, 0),
            RaceDbc {
                faction_id: uint(row, 2),
                model_m: uint(row, 3),
                model_f: uint(row, 4),
                res_sickness_spell_id: uint(row, 9),
                team_id: uint(row, 5),
            },
        );
    }
    let bytes = std::fs::read(dir.join(formats::CHR_CLASSES.file)).expect("ChrClasses.dbc");
    let store = DbcFile::parse(&bytes, formats::CHR_CLASSES.fmt)
        .expect("parse")
        .produce(formats::CHR_CLASSES.fmt)
        .expect("produce");
    for row in store.rows() {
        dbc.classes.insert(uint(row, 0));
        dbc.class_power.insert(uint(row, 0), uint(row, 1));
    }
    dbc
}

fn load_stores() -> Arc<WorldStores> {
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD");
    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let stores = world
        .call(move |conn| {
            let (creature_templates, _) =
                load_creature_templates(conn, 10, &TemplateValidation::default())?;
            let (gameobject_templates, _) =
                load_gameobject_templates(conn, 10, &GameObjectValidation::default())?;
            let map_ids = load_map_ids(conn)?;
            let mut spawns = SpawnData::new();
            spawns.load_creatures(conn, 10, &creature_templates, Some(&map_ids), false)?;
            spawns.load_gameobjects(
                conn,
                10,
                &gameobject_templates,
                Some(&map_ids),
                None,
                false,
            )?;
            let (creature_addons, _) = load_creature_addons(conn, 10, None, None, None)?;
            let (equipment, _) = load_equipment_templates(conn, 10, None)?;
            let (cls, _) = load_class_level_stats(conn)?;
            let (display_addons, _) = load_display_info_addons(conn, 5875, None, None)?;
            let (player_infos, _) = load_player_infos(conn, 60, None, Some(&map_ids), None)?;
            let items = load_item_prototypes_lite(conn, 10)?;
            let mut report = LoadReport::default();
            let waypoints = load_waypoints(conn, &mut report)?;
            let mut spell_report = LoadReport::default();
            let spell_entries = load_spell_templates(conn, 5875, &mut spell_report)?;
            let spell_ids: std::collections::HashSet<u32> = spell_entries.keys().copied().collect();
            let mut cs_report = LoadReport::default();
            let creature_spells = load_creature_spells(conn, &spell_ids, &mut cs_report)?;
            let template_ids: std::collections::HashSet<u32> =
                creature_templates.iter().map(|(id, _)| *id).collect();
            let mut script_report = LoadReport::default();
            let scripts = load_script_stores(conn, &template_ids, &mut script_report)?;

            Ok(WorldStores {
                creature_templates,
                gameobject_templates,
                spawns,
                creature_addons,
                equipment,
                cls,
                display_addons,
                player_infos,
                items,
                item_prototypes: mangos_world_data::item_prototype::ItemPrototypes::default(),
                factions: mangos_world_data::reputations::FactionStore::default(),
                areas: mangos_world_data::areas::AreaStore::default(),
                exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(),
                pets: mangos_world_data::pet_data::PetStores::default(),
                next_pet_number: std::sync::atomic::AtomicU32::new(1),
                weather_chances: mangos_world_data::weather::WeatherChanceStore::default(),
                waypoints,
                spells: mangos_spells::SpellStore {
                    spells: spell_entries,
                    proc_events: std::collections::HashMap::new(),
                    cones: std::collections::HashMap::new(),
                    tables: mangos_spells::load_spell_dbc_tables(std::path::Path::new(
                        &std::env::var("MANGOS_DBC_DIR").expect("MANGOS_DBC_DIR"),
                    ))
                    .expect("spell dbc tables"),
                },
                spell_target_positions:
                    mangos_world_data::spell_target_position::SpellTargetPositions::default(),
                creature_spells,
                scripts,
                display_scales: HashMap::new(),
                model_data: HashMap::new(),
                shapeshift_forms: HashMap::new(),
                skills: mangos_world_data::skill_info::SkillDbc::default(),
                chat_channels: mangos_world_data::chat_channels::ChatChannelDbc::default(),
                auction_houses: mangos_world_data::auction_house::AuctionHouseDbc::default(),
                maps: mangos_world_data::map_template::load_map_templates(conn, 10, 1.0)?,
                safe_locs: mangos_world_data::battlegrounds::SafeLocStore::default(),
                battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates::default(),
                battlemasters: mangos_world_data::battlegrounds::BattleMasters::default(),
                battle_event_indexes: mangos_world_data::battlegrounds::BattleEventIndexes::default(
                ),
                graveyard_links: mangos_world_data::battlegrounds::GraveyardLinks::default(),
                bg_entrances: mangos_world_data::battlegrounds::BattlegroundEntrances::default(),
                area_triggers: mangos_world_data::area_trigger::AreaTriggerStore::default(),
                quests: mangos_world_data::quest_template::QuestTemplatesLite::default(),
                quest_templates: mangos_world_data::quest_template::QuestTemplates::default(),
                quest_relations: mangos_world_data::quest_template::QuestRelations::default(),
                tavern_triggers: std::collections::HashSet::new(),
            })
        })
        .expect("stores");
    Arc::new(stores)
}

fn make_map(stores: &Arc<WorldStores>, map_id: u32) -> (Map, ObjectGuid, RespawnTimes) {
    let chars_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID")
        .parse()
        .expect("guid");
    let dbc_dir: PathBuf = std::env::var_os("MANGOS_DBC_DIR")
        .expect("MANGOS_DBC_DIR")
        .into();
    let dbc = race_class_dbc(&dbc_dir);
    let chars = DbActor::spawn("test-chars", &chars_url).expect("chars");
    let load = chars
        .call(move |conn| load_character(conn, guid))
        .expect("load")
        .expect("fixture exists");
    let inputs = MaterializeInputs {
        infos: &stores.player_infos,
        race: dbc.races.get(&1),
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
        skills: None,
        item_prototypes: None,
        quest_templates: None,
    };
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();
    let _ = &dbc;
    let mut map = Map::new(
        map_id,
        1,
        MapConfig::default(),
        Arc::clone(stores),
        WorldRng::from_seed(7),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    let _ = map.drain_events();
    heal(&mut map, player_guid);
    (map, player_guid, respawns)
}

fn tick(map: &mut Map, now_ms: &mut u32, n: u32) {
    for _ in 0..n {
        *now_ms += 50;
        map.tick(50, *now_ms, 1_800_000_000);
        let _ = map.drain_events();
    }
}

fn relocate(map: &mut Map, player: ObjectGuid, respawns: &RespawnTimes, x: f32, y: f32, z: f32) {
    let mut info = map
        .player_mut(player)
        .expect("player")
        .unit
        .world
        .movement_info;
    info.pos = Position::new(x, y, z, 0.0);
    map.relocate_from_client(player, info, respawns);
}

fn heal(map: &mut Map, player: ObjectGuid) {
    let p = map.player_mut(player).expect("player");
    p.unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
    p.unit.set_health(100_000);
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_molten_core_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 409);
    assert_eq!(map.instance_get_data(4), Some(0), "MC instance created");
    // Live-recompute Save(): 10 encounters + 7 runes.
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0")
    );
    assert!(map.instance_set_data(4, 3)); // GARR DONE
    assert!(map.instance_set_data(16, 3)); // RUNE_ACTIVE_0 DONE
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 0 0 0 3 0 0 0 0 0 3 0 0 0 0 0 0")
    );
    // Garr registers for GetData64(DATA_GARR=13).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 691.0, -498.0, -213.0);
    tick(&mut map, &mut now, 120);
    assert!(map.instance_get_data64(13).is_some(), "Garr guid stored");
    // Load: encounters IN_PROGRESS reset, runes preserved.
    map.instance_load("1 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0");
    assert_eq!(map.instance_get_data(0), Some(0));
    assert_eq!(map.instance_get_data(16), Some(1), "rune state not reset");
    assert!(!map.instance_is_encounter_in_progress());
    assert!(map.instance_set_data(0, 4)); // SPECIAL counts as in progress
    assert!(map.instance_is_encounter_in_progress());
    println!("LIVE-MOLTEN-CORE-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_ruins_of_ahnqiraj_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 509);
    assert_eq!(map.instance_get_data(0), Some(0), "AQ20 instance created");
    // Gladiator death counter (field 8 outside MAX_ENCOUNTER).
    assert!(map.instance_set_data(8, 2));
    assert_eq!(map.instance_get_data(8), Some(2));
    assert!(map.instance_set_data(8, 0));
    assert_eq!(map.instance_get_data(8), Some(0));
    // Cached save: only a DONE write refreshes.
    assert!(map.instance_set_data(0, 1)); // KURINNAXX IN_PROGRESS
    assert_eq!(map.instance_save().as_deref(), Some(""), "no refresh yet");
    assert!(map.instance_set_data(0, 3));
    assert_eq!(map.instance_save().as_deref(), Some("3 0 0 0 0 0 0"));
    // Kurinnaxx registers for GetData64(DATA_KURINNAXX=0). (Andorov's
    // spawn row is not-spawned-by-default: the fork loads him via
    // LoadCreatureSpawnWithGroup, unimplemented — see evidence.)
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, -8843.0, 1624.0, 21.0);
    tick(&mut map, &mut now, 120);
    assert!(
        map.instance_get_data64(0).is_some(),
        "Kurinnaxx guid stored"
    );
    // Load: Andorov DONE has its own extra reset.
    map.instance_load("3 3 1 0 0 0 0");
    assert_eq!(map.instance_get_data(1), Some(0), "Andorov DONE resets");
    assert_eq!(map.instance_get_data(2), Some(0), "IN_PROGRESS resets");
    assert_eq!(map.instance_get_data(0), Some(3));
    println!("LIVE-RUINS-OF-AHNQIRAJ-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_blackrock_spire_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 229);
    assert_eq!(map.instance_get_data(0), Some(0), "BRS instance created");
    // Room-event SetData64 refuses outside IN_PROGRESS.
    assert!(!map.instance_set_data64(0, player));
    assert!(map.instance_set_data(3, 3)); // STADIUM DONE
    assert_eq!(map.instance_save().as_deref(), Some("0 0 0 3 0 0 0 0"));
    map.instance_load("1 0 0 3 0 0 0 0");
    assert_eq!(map.instance_get_data(0), Some(0));
    assert_eq!(map.instance_get_data(3), Some(3));
    // UBRS door event: braziers then door open over ~11s.
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 127.0, -319.0, 72.0);
    tick(&mut map, &mut now, 120);
    let go_state = |m: &Map, entry: u32| {
        m.visible_to(player)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .find_map(|g| {
                m.gameobject(g).and_then(|go| {
                    (go.info.id == entry)
                        .then(|| go.world.object.values().get_u32(offsets::GAMEOBJECT_STATE))
                })
            })
    };
    assert!(map.instance_set_data(5, 3)); // EVENT_DOOR_UBRS DONE
    tick(&mut map, &mut now, 320); // 16s > 2s + 3x3s sequence
    assert_eq!(
        go_state(&map, 164_725),
        Some(0),
        "UBRS door opened (GO_STATE_ACTIVE)"
    );
    println!("LIVE-BLACKROCK-SPIRE-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_temple_of_ahnqiraj_slice() {
    let stores = load_stores();
    let (mut map, _player, _respawns) = make_map(&stores, 531);
    assert_eq!(map.instance_get_data(0), Some(0), "AQ40 instance created");
    assert_eq!(map.instance_get_data(99), None, "bounds-guarded GetData");
    // Bug trio: SPECIAL counts deaths without storing; 3rd -> DONE.
    assert!(map.instance_set_data(6, 4));
    assert!(map.instance_set_data(6, 4));
    assert_eq!(map.instance_get_data(6), Some(0), "SPECIAL not stored");
    assert!(map.instance_set_data(6, 4));
    assert_eq!(map.instance_get_data(6), Some(3), "third death completes");
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 0 0 0 0 0 3 0 0 0 "),
        "fork trailing-space save shape"
    );
    assert!(map.instance_check_condition(531, 6));
    assert!(!map.instance_check_condition(531, 99));
    // IN_PROGRESS resets the death counter.
    assert!(map.instance_set_data(6, 1));
    assert!(map.instance_set_data(6, 4));
    assert_eq!(map.instance_get_data(6), Some(1));
    // Twins repeat write is a no-op; Load resets IN_PROGRESS.
    map.instance_load("0 0 0 0 1 0 3 0 0 0");
    assert_eq!(map.instance_get_data(4), Some(0));
    assert_eq!(map.instance_get_data(6), Some(3), "DONE persists");
    println!("LIVE-TEMPLE-OF-AHNQIRAJ-PASS");
}
