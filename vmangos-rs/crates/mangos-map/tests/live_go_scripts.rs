//! Tier 3 Slice 2 step 2 (GO hello/AI framework)
//! live evidence on the dev bench: Fire of
//! Aku'mai `GOGetAI` `OnUse`, MC rune `pGOHello`.
//! Expected values from independent SQL + fork
//! cpp. (The coffer/firecracker despawn tick has
//! no live test: every binding spawn is
//! game-event-gated and spell GO-summon is
//! unmodeled — see TIER3-PROGRESS.)

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

fn find_creature(map: &Map, player: ObjectGuid, entry: u32) -> Option<ObjectGuid> {
    map.visible_to(player)?
        .iter()
        .copied()
        .find(|g| map.creature(*g).is_some_and(|c| c.info.entry == entry))
}

fn find_go(map: &Map, player: ObjectGuid, entry: u32) -> Option<ObjectGuid> {
    map.visible_to(player)?
        .iter()
        .copied()
        .find(|g| map.gameobject(*g).is_some_and(|go| go.info.id == entry))
}

fn go_field(map: &Map, go: ObjectGuid, offset: u16) -> u32 {
    map.gameobject(go)
        .map_or(u32::MAX, |g| g.world.object.values().get_u32(offset))
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_go_fire_of_akumai_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 48);
    // Fire spawn (SQL: map 48, -813.47/-158.535/-24.5271).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, -813.47, -158.535, -24.5271);
    tick(&mut map, &mut now, 120);
    let fire = find_go(&map, player, 21_118).expect("fire of Aku'mai in view");
    // Kelris alive: OnUse refuses, shrine stays idle and the
    // fire stays interactable.
    map.gameobject_use(player, fire);
    assert_eq!(map.instance_get_data(11), Some(0), "shrine not started");
    assert_eq!(
        go_field(&map, fire, offsets::GAMEOBJECT_FLAGS) & 0x10,
        0,
        "fire still interactable"
    );
    // Kelris dead: the fire lights (ACTIVE + NO_INTERACT) and the
    // shrine event starts.
    assert!(map.instance_set_data(10, 3)); // TYPE_KELRIS DONE
    map.gameobject_use(player, fire);
    assert_eq!(
        go_field(&map, fire, offsets::GAMEOBJECT_STATE),
        0,
        "GO_STATE_ACTIVE"
    );
    assert_ne!(
        go_field(&map, fire, offsets::GAMEOBJECT_FLAGS) & 0x10,
        0,
        "GO_FLAG_NO_INTERACT set"
    );
    assert_eq!(map.instance_get_data(11), Some(1), "shrine IN_PROGRESS");
    println!("LIVE-GO-FIRE-OF-AKUMAI-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_go_rune_mc_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 409);
    // Sulfuron rune spawn (SQL: map 409, 601.672/-1174.61/-196.072).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 601.672, -1174.61, -196.072);
    tick(&mut map, &mut now, 120);
    let rune = find_go(&map, player, 176_951).expect("Sulfuron rune in view");
    // Boss alive: the douse is refused (hello handled, no change).
    map.gameobject_use(player, rune);
    assert_eq!(map.instance_get_data(16), Some(0), "rune still active");
    // Boss dead: dousing marks the rune done.
    assert!(map.instance_set_data(0, 3)); // TYPE_SULFURON DONE
    map.gameobject_use(player, rune);
    assert_eq!(map.instance_get_data(16), Some(3), "rune doused");
    assert_eq!(map.instance_get_data(23), Some(0), "Majordomo not yet");
    // Complete the remaining bosses + runes, then use the rune
    // again: Majordomo is summoned at the Ragnaros lair.
    for field in 1..=6 {
        assert!(map.instance_set_data(field, 3));
    }
    for field in 17..=22 {
        assert!(map.instance_set_data(field, 3));
    }
    map.gameobject_use(player, rune);
    assert_eq!(map.instance_get_data(23), Some(3), "DATA_DOMO_SPAWNED");
    relocate(&mut map, player, &respawns, 758.089, -1176.71, -118.64);
    // Majordomo's script summons his 8 adds on spawn and the
    // rune-area scripts can kill the fixture player during the
    // waits; revive (batch-6 fixture pattern) and stay topped.
    for _ in 0..120 {
        if let Some(pl) = map.player_mut(player) {
            pl.unit.death_state = mangos_entities::unit::DeathState::Alive;
            pl.unit.set_health(100_000);
        }
        tick(&mut map, &mut now, 1);
    }
    assert!(
        find_creature(&map, player, 12_018).is_some(),
        "Majordomo summoned"
    );
    println!("LIVE-GO-RUNE-MC-PASS");
}

// GO lane ports (step 2 lanes): Herod lever door toggle
// (`boss_herod.cpp`), Stratholme postbox chain
// (`go_open_stratholme_postbox`, via the still-uncalled
// `go_open` dispatcher), Scholomance viewing-room door
// persistence flag.

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_go_herod_lever_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    // Lever spawn (SQL: map 189, 1939.48/-427.998/17.0815);
    // door 101854 at 1933.69/-431.654/18.671 (within 40).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 1939.48, -427.998, 17.0815);
    tick(&mut map, &mut now, 120);
    let lever = find_go(&map, player, 101_855).expect("Herod lever in view");
    let door = find_go(&map, player, 101_854).expect("Herod door in view");
    assert_eq!(
        go_field(&map, door, offsets::GAMEOBJECT_STATE),
        1,
        "door starts READY (closed)"
    );
    map.gameobject_use(player, lever);
    assert_eq!(
        go_field(&map, door, offsets::GAMEOBJECT_STATE),
        0,
        "lever opens the door (ACTIVE)"
    );
    map.gameobject_use(player, lever);
    assert_eq!(
        go_field(&map, door, offsets::GAMEOBJECT_STATE),
        1,
        "lever closes the door again (READY)"
    );
    println!("LIVE-GO-HEROD-LEVER-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_go_stratholme_postbox_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 329);
    let mut now = 1000u32;
    // Postbox 176346 (SQL: 3690.03/-3405.43/132.838).
    relocate(&mut map, player, &respawns, 3690.03, -3405.43, 132.838);
    tick(&mut map, &mut now, 120);
    let box1 = find_go(&map, player, 176_346).expect("postbox 1 in view");
    assert!(!map.go_open(player, box1), "fork returns false");
    assert_eq!(map.instance_get_data(9), Some(1), "POSTMASTER IN_PROGRESS");
    relocate(&mut map, player, &respawns, 3690.03, -3405.43, 132.838);
    tick(&mut map, &mut now, 120);
    assert!(
        find_creature(&map, player, 11_142).is_some(),
        "Undead Postman summoned"
    );
    // Postbox 176349 (SQL: 3651.28/-3165.66/128.177): second
    // use arms the Postmaster (SPECIAL=4).
    relocate(&mut map, player, &respawns, 3651.28, -3165.66, 128.177);
    tick(&mut map, &mut now, 120);
    let box2 = find_go(&map, player, 176_349).expect("postbox 2 in view");
    assert!(!map.go_open(player, box2));
    assert_eq!(map.instance_get_data(9), Some(4), "POSTMASTER SPECIAL");
    // Postbox 176350 (SQL: 3659.79/-3476.73/138.381): third
    // use fires the summon spell arm and marks DONE.
    relocate(&mut map, player, &respawns, 3659.79, -3476.73, 138.381);
    tick(&mut map, &mut now, 120);
    let box3 = find_go(&map, player, 176_350).expect("postbox 3 in view");
    assert!(!map.go_open(player, box3));
    assert_eq!(map.instance_get_data(9), Some(3), "POSTMASTER DONE");
    println!("LIVE-GO-STRATHOLME-POSTBOX-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_go_viewing_room_door_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 289);
    // Viewing room door (SQL: map 289, 174.378/77.9398/104.802).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 174.378, 77.9398, 104.802);
    tick(&mut map, &mut now, 120);
    let door = find_go(&map, player, 175_167).expect("viewing room door in view");
    assert_eq!(map.instance_get_data(14), Some(0), "door flag idle");
    map.gameobject_use(player, door);
    assert_eq!(map.instance_get_data(14), Some(3), "door flag DONE");
    println!("LIVE-GO-VIEWING-ROOM-DOOR-PASS");
}
