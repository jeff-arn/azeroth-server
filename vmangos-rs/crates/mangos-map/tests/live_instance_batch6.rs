//! Tier 3 Slice 2 batch 6 (Stratholme, Blackrock
//! Depths, Dire Maul, Blackwing Lair, Naxxramas)
//! live evidence on the dev bench. Expected values
//! from independent SQL + fork cpp. Completes the
//! instance-script corpus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::{InventoryRow, ItemInstanceRow, load_character};
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::spells::{CastOptions, CastTarget};
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

fn count_creatures(map: &Map, player: ObjectGuid, entry: u32) -> usize {
    map.visible_to(player).map_or(0, |vis| {
        vis.iter()
            .filter(|g| map.creature(**g).is_some_and(|c| c.info.entry == entry))
            .count()
    })
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_stratholme_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 329);
    assert_eq!(
        map.instance_get_data(0),
        Some(0),
        "Stratholme instance created"
    );
    // Cached save: any DONE write restreams 11 values, trailing space.
    assert!(map.instance_set_data(1, 3)); // TYPE_BARONESS DONE
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 3 0 0 0 0 0 0 0 0 0 ")
    );
    // Baron Rivendare (SQL spawn 4035.83,-3336.31,115.144) registers
    // for GetData64(DATA_BARON=10).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 4035.83, -3336.31, 115.144);
    tick(&mut map, &mut now, 120);
    assert!(map.instance_get_data64(10).is_some(), "Baron guid stored");
    // Load: IN_PROGRESS resets, DONE persists.
    map.instance_load("0 3 1 0 0 0 0 0 0 0 0");
    assert_eq!(map.instance_get_data(1), Some(3));
    assert_eq!(map.instance_get_data(2), Some(0), "IN_PROGRESS resets");
    assert!(!map.instance_is_encounter_in_progress());
    assert!(map.instance_set_data(0, 1)); // BARON_RUN IN_PROGRESS
    assert!(map.instance_is_encounter_in_progress());
    println!("LIVE-STRATHOLME-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_blackrock_depths_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 230);
    assert_eq!(map.instance_get_data(0), Some(0), "BRD instance created");
    // Thunderbrew: three IN_PROGRESS writes flip the stored value to
    // DONE internally, but the cached save refreshes only when the
    // *passed* data is DONE (fork quirk, cpp 1014).
    for _ in 0..3 {
        assert!(map.instance_set_data(6, 1)); // TYPE_THUNDERBREW
    }
    assert_eq!(map.instance_get_data(6), Some(3));
    assert_eq!(map.instance_save().as_deref(), Some(""), "no refresh yet");
    assert!(map.instance_set_data(1, 3)); // TYPE_VAULT DONE
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 3 0 0 0 0 3 0 0 0 0 0 0 0 0 0 0 0 0 0"),
        "20 streamed values, no trailing space"
    );
    // Theldren invocation: spell 27517 hits go through the fork's
    // hardcoded CustomSpellCasted call site (SpellEffects.cpp:567).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 590.0, -155.0, -52.0);
    tick(&mut map, &mut now, 120);
    let outcome = map.cast_spell(
        player,
        27_517,
        CastTarget::SelfCast,
        CastOptions {
            triggered: true,
            skip_known: true,
            ..Default::default()
        },
    );
    println!("27517 cast outcome: {outcome:?}");
    tick(&mut map, &mut now, 40);
    assert_eq!(map.instance_get_data(30), Some(1), "DATA_THELDREN started");
    assert_eq!(
        map.instance_get_data64(42),
        Some(player),
        "challenger stored"
    );
    println!("LIVE-BLACKROCK-DEPTHS-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_dire_maul_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 429);
    assert_eq!(
        map.instance_get_data(1),
        Some(0),
        "Dire Maul instance created"
    );
    assert_eq!(map.instance_get_data(12), None, "bounds-guarded GetData");
    // Cho'Rush equipment: first read rolls urand(1,3) and caches.
    let equip = map.dm_chorush_equipment();
    assert!((1..=3).contains(&equip), "equipment roll in fork range");
    assert_eq!(map.dm_chorush_equipment(), equip, "roll cached");
    // Load streams 10 values into slots [1,2,3,4,6,7,8,9,10,11]:
    // the 5th token lands in slot 6, slot 5 is skipped.
    map.instance_load("0 0 0 0 3 0 0 0 0 0");
    assert_eq!(map.instance_get_data(6), Some(3));
    assert_eq!(map.instance_get_data(5), Some(0), "slot 5 not streamed");
    // OnPlayerLeave strips King of the Gordok (22799).
    let outcome = map.cast_spell(
        player,
        22_799,
        CastTarget::Unit(player),
        CastOptions {
            triggered: true,
            skip_known: true,
            ..Default::default()
        },
    );
    println!("22799 cast outcome: {outcome:?}");
    let mut now = 1000u32;
    let mut applied = false;
    for _ in 0..40 {
        tick(&mut map, &mut now, 10);
        if map
            .aura_holders(player)
            .iter()
            .any(|h| h.spell_id == 22_799)
        {
            applied = true;
            break;
        }
    }
    assert!(applied, "king aura applied");
    let mut player = map.remove_player(player).expect("player removed");
    assert!(
        map.aura_holders(player.unit.guid()).is_empty(),
        "aura stripped on leave"
    );
    // OnPlayerEnter destroys the inner door key while Moldar lives.
    player.saved.inventory.push(InventoryRow {
        item: ItemInstanceRow {
            creator_guid: 0,
            gift_creator_guid: 0,
            count: 1,
            duration: 0,
            charges: String::new(),
            flags: 0,
            enchantments: String::new(),
            random_property_id: 0,
            durability: 0,
            text: 0,
            item_guid: 999_001,
            item_id: 18_268,
            generated_loot: 0,
        },
        bag: 0,
        slot: 30,
    });
    let guid = player.unit.guid();
    assert_eq!(player.quest_item_count(18_268), 1);
    map.add_player(player, &respawns, 1_800_000_000, now);
    let _ = map.drain_events();
    assert_eq!(
        map.player(guid)
            .expect("player back")
            .quest_item_count(18_268),
        0,
        "key destroyed on enter"
    );
    println!("LIVE-DIRE-MAUL-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_blackwing_lair_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 469);
    assert_eq!(map.instance_get_data(0), Some(0), "BWL instance created");
    assert_eq!(map.instance_get_data(10), None, "bounds-guarded GetData");
    // Scepter-run status is excluded from IsEncounterInProgress.
    assert!(map.instance_set_data(0, 1)); // RAZORGORE IN_PROGRESS
    assert!(map.instance_is_encounter_in_progress());
    assert!(map.instance_set_data(0, 0));
    assert!(map.instance_set_data(9, 1)); // TYPE_SCEPTER_RUN IN_PROGRESS
    assert!(
        !map.instance_is_encounter_in_progress(),
        "scepter run excluded"
    );
    // No champion yet: the scepter conditions read false.
    assert!(!map.instance_check_condition(469, 1));
    assert!(map.instance_set_data(24, player.counter())); // DATA_SCEPTER_CHAMPION
    assert!(map.instance_set_data(9, 2)); // FAIL
    assert!(
        map.instance_check_condition(469, 1),
        "scepter FAIL condition"
    );
    // Razorgore (SQL spawn -7571.69,-1088.25,413.465) registers and
    // the first creature create performs the Initialize color roll.
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, -7571.69, -1088.25, 413.465);
    tick(&mut map, &mut now, 120);
    assert!(
        map.instance_get_data64(0).is_some(),
        "Razorgore guid stored"
    );
    assert!(map.instance_set_data(0, 3)); // RAZORGORE DONE
    let save = map.instance_save().expect("save");
    let toks: Vec<i64> = save
        .split_whitespace()
        .map(|t| t.parse().expect("numeric token"))
        .collect();
    assert_eq!(toks.len(), 14, "save shape: {save}");
    assert_eq!(toks[0], 3);
    assert!(
        (0..=19).contains(&toks[9]),
        "chrom breath rolled: {}",
        toks[9]
    );
    assert!(
        (0..=19).contains(&toks[10]),
        "nef color rolled: {}",
        toks[10]
    );
    assert_eq!(toks[11], 2, "scepter FAIL persisted");
    assert_eq!(
        toks[13],
        i64::from(player.counter()),
        "champion raw persisted"
    );
    println!("LIVE-BLACKWING-LAIR-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_naxxramas_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 533);
    assert_eq!(
        map.instance_get_data(0),
        Some(0),
        "Naxxramas instance created"
    );
    // Four Horsemen: SPECIAL writes count deaths; the 4th completes.
    for _ in 0..3 {
        assert!(map.instance_set_data(8, 4)); // SPECIAL
    }
    assert_eq!(map.instance_get_data(8), Some(4));
    assert!(map.instance_set_data(8, 4));
    assert_eq!(map.instance_get_data(8), Some(3), "fourth death completes");
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 0 0 0 0 0 0 0 3 0 0 0 0 0 0 "),
        "fork trailing-space save shape"
    );
    // Kel'Thuzad SPECIAL starts only with every player within 15yd
    // of the chamber center. The center is set by the unported
    // boss script, so it still reads the (0,0) default here.
    assert!(map.instance_set_data(14, 4)); // SPECIAL, player far away
    assert_eq!(map.instance_get_data(14), Some(0), "write dropped");
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 0.0, 0.0, 300.0);
    tick(&mut map, &mut now, 5);
    assert!(map.instance_set_data(14, 4));
    assert_eq!(map.instance_get_data(14), Some(1), "encounter begins");
    // Load: IN_PROGRESS resets; a loaded Thaddius SPECIAL becomes FAIL.
    map.instance_load("1 0 0 0 0 0 0 0 0 0 0 0 4 0 0");
    assert_eq!(map.instance_get_data(0), Some(0));
    assert_eq!(
        map.instance_get_data(12),
        Some(2),
        "Thaddius SPECIAL -> FAIL"
    );
    // OnPlayerDeath: five corpse scarabs while Anub'Rekhan (SQL
    // spawn 3308.59,-3476.29,287.161) is in progress.
    relocate(&mut map, player, &respawns, 3308.59, -3476.29, 287.161);
    tick(&mut map, &mut now, 120);
    let anub = find_creature(&map, player, 15_956).expect("Anub'Rekhan spawned");
    assert!(map.instance_set_data(0, 1)); // ANUB IN_PROGRESS
    map.player_mut(player).expect("player").unit.set_health(1);
    let outcome = map.cast_spell(
        anub,
        8_921, // Moonfire: instant direct damage
        CastTarget::Unit(player),
        CastOptions {
            triggered: true,
            skip_known: true,
            ..Default::default()
        },
    );
    println!("moonfire outcome: {outcome:?}");
    tick(&mut map, &mut now, 40);
    assert_eq!(
        map.player(player).expect("player").unit.health(),
        0,
        "player died"
    );
    // Revive the fixture so the visibility scan resumes and the
    // summons become observable (test-only manipulation).
    {
        let p = map.player_mut(player).expect("player");
        p.unit.death_state = mangos_entities::unit::DeathState::Alive;
        p.unit.set_health(100_000);
    }
    // Offset from the death position so the relocation passes
    // the scan-due distance gate and the visibility scan runs.
    relocate(&mut map, player, &respawns, 3298.59, -3476.29, 287.161);
    tick(&mut map, &mut now, 20);
    assert_eq!(count_creatures(&map, player, 16_698), 5, "corpse scarabs");
    println!("LIVE-NAXXRAMAS-PASS");
}
