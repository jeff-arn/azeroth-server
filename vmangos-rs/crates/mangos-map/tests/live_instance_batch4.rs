//! Tier 3 Slice 2 batch 4 (Scarlet Monastery,
//! Zul'Gurub, Uldaman, Sunken Temple) live evidence
//! on the dev bench. Expected values come from
//! independent SQL and the fork cpp.

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

fn visible_creatures(map: &Map, player: ObjectGuid, entry: u32) -> usize {
    let visible = map.visible_to(player).cloned().unwrap_or_default();
    visible
        .iter()
        .filter(|g| map.creature(**g).is_some_and(|c| c.info.entry == entry))
        .count()
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_scarlet_monastery_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    // TYPE_MOGRAINE_AND_WHITE_EVENT=1, TYPE_ASHBRINGER_EVENT=2.
    assert_eq!(map.instance_get_data(1), Some(0), "SM instance created");
    assert_eq!(map.instance_get_data(2), Some(0));
    // Fork Save() recomputes from the live encounter pair.
    assert_eq!(map.instance_save().as_deref(), Some("0 0"));
    assert!(map.instance_set_data(1, 4)); // STAGE_MOGRAINE_DONE
    assert_eq!(map.instance_save().as_deref(), Some("4 0"));
    assert!(map.instance_set_data(2, 3)); // Ashbringer DONE
    assert_eq!(map.instance_save().as_deref(), Some("4 3"));
    map.instance_load("1 1");
    assert_eq!(map.instance_get_data(1), Some(0), "IN_PROGRESS resets");
    assert_eq!(map.instance_get_data(2), Some(0));
    // Ashbringer proc (28441) on Mograine starts the apparition:
    // the Highlord (16062) is summoned.
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 1150.0, 1398.0, 34.0);
    tick(&mut map, &mut now, 120);
    let mograine = map
        .visible_to(player)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|g| map.creature(*g).is_some_and(|c| c.info.entry == 3976))
        .expect("Commander Mograine visible");
    // 28441 is a caster-centered AoE (implicit targets
    // 22/7); stand on Mograine so he is inside the radius.
    let mograine_pos = map
        .creature(mograine)
        .map(|c| c.unit.world.position())
        .expect("mograine pos");
    relocate(
        &mut map,
        player,
        &respawns,
        mograine_pos.x,
        mograine_pos.y,
        mograine_pos.z,
    );
    tick(&mut map, &mut now, 4);
    let before = map.creature_count();
    let outcome = map.cast_spell(
        player,
        28441,
        mangos_map::spells::CastTarget::Unit(mograine),
        mangos_map::spells::CastOptions {
            triggered: true,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    tick(&mut map, &mut now, 60);
    assert!(
        map.creature_count() > before,
        "Highlord Mograine apparition summoned (cast outcome {outcome:?})"
    );
    println!("LIVE-SCARLET-MONASTERY-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_zulgurub_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 309);
    let mut now = 1000u32;
    tick(&mut map, &mut now, 4);
    // Lazy Create(): game day for 1_800_000_000 is 20833,
    // weekmod 0 -> boss 15082.
    assert_eq!(map.instance_get_data(10), Some(15082), "random boss rolled");
    assert_eq!(
        map.instance_save().as_deref(),
        Some("0 0 0 0 0 0 0 0 0 15082 0 0")
    );
    // Hakkar power stacks track living high priests (5).
    relocate(&mut map, player, &respawns, -11790.0, -1650.0, 55.0);
    tick(&mut map, &mut now, 120);
    let hakkar = map.instance_get_data64(17).expect("Hakkar registered");
    assert!(map.instance_set_data(0, 0)); // TYPE_HAKKAR_POWER pulse
    let stack = map
        .aura_holders(hakkar)
        .iter()
        .find(|h| h.spell_id == 24692)
        .map_or(0, |h| h.stack);
    assert_eq!(stack, 5, "five priest stacks");
    assert!(map.instance_set_data(1, 3)); // ARLOKK DONE
    let stack = map
        .aura_holders(hakkar)
        .iter()
        .find(|h| h.spell_id == 24692)
        .map_or(0, |h| h.stack);
    assert_eq!(stack, 5, "stacks only move on the power pulse");
    assert!(map.instance_set_data(0, 0));
    let stack = map
        .aura_holders(hakkar)
        .iter()
        .find(|h| h.spell_id == 24692)
        .map_or(0, |h| h.stack);
    assert_eq!(stack, 4, "one priest down");
    // Hakkar's script Aggro set TYPE_HAKKAR IN_PROGRESS during
    // the temple wait (boss_hakkar.cpp:123, CreatureAI slice).
    assert_eq!(
        map.instance_save().as_deref(),
        Some("3 0 0 0 0 0 0 0 1 15082 0 0")
    );
    // Load: IN_PROGRESS resets; Create must not re-roll.
    map.instance_load("3 1 0 0 0 0 0 0 0 15083 0 0");
    assert_eq!(map.instance_get_data(2), Some(0));
    tick(&mut map, &mut now, 4);
    assert_eq!(
        map.instance_get_data(10),
        Some(15083),
        "no re-roll after Load"
    );
    assert!(!map.instance_is_encounter_in_progress());
    assert!(map.instance_set_data(11, 1)); // JINDO IN_PROGRESS
    assert!(map.instance_is_encounter_in_progress());
    println!("LIVE-ZULGURUB-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_uldaman_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 70);
    // Encounters: IRONAYA_DOOR=0, STONE_KEEPERS=1, ARCHAEDAS=2.
    assert_eq!(
        map.instance_get_data(0),
        Some(0),
        "Uldaman instance created"
    );
    assert_eq!(map.instance_get_data(2), Some(0));
    assert!(map.instance_set_data64(0, player));
    assert_eq!(map.instance_get_data64(0), Some(player));
    // Ironaya door DONE: 27s keystone timer unfreezes her
    // (faction 45 -> 415 via fork SetFactionTemplateId(415)).
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, -238.0, 310.0, -46.0);
    tick(&mut map, &mut now, 120);
    let ironaya = map
        .visible_to(player)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|g| map.creature(*g).is_some_and(|c| c.info.entry == 7228))
        .expect("Ironaya visible");
    let faction = |m: &Map| {
        m.creature(ironaya).map_or(0, |c| {
            c.unit
                .world
                .object
                .values()
                .get_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE)
        })
    };
    assert_ne!(faction(&map), 415, "frozen Ironaya keeps spawn faction");
    assert!(map.instance_set_data(0, 3)); // IRONAYA_DOOR DONE
    assert_eq!(map.instance_save().as_deref(), Some("3 0 0"));
    tick(&mut map, &mut now, 600); // 30s > 27s seal timer
    assert_eq!(faction(&map), 415, "Ironaya unfrozen with combat faction");
    // Load: only DONE persists.
    map.instance_load("3 1 3");
    assert_eq!(map.instance_get_data(0), Some(3));
    assert_eq!(map.instance_get_data(1), Some(0));
    assert_eq!(map.instance_get_data(2), Some(3));
    assert!(!map.instance_is_encounter_in_progress());
    assert!(map.instance_set_data(2, 1)); // ARCHAEDAS IN_PROGRESS
    assert!(map.instance_is_encounter_in_progress());
    println!("LIVE-ULDAMAN-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_sunken_temple_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 109);
    // TYPE_SECRET_CIRCLE=4, TYPE_PROTECTORS=5.
    assert_eq!(map.instance_get_data(4), Some(0), "ST instance created");
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, -516.0, 95.0, -148.0);
    tick(&mut map, &mut now, 120);
    let empty = ObjectGuid::EMPTY;
    // Wrong statue first (entry 148831 = statue 2): event starts,
    // counter does not advance (no DONE).
    assert!(map.instance_set_data64(148_831, empty));
    assert_eq!(map.instance_get_data(4), Some(1), "circle IN_PROGRESS");
    // Correct order 1..6 completes the circle.
    for entry in [148_830, 148_831, 148_832, 148_833, 148_834, 148_835] {
        assert!(map.instance_set_data64(entry, empty));
    }
    assert_eq!(map.instance_get_data(4), Some(3), "circle DONE");
    assert_eq!(map.instance_save().as_deref(), Some("3 0 0 0 0 0"));
    // Atal'arion revealed in the pit below.
    relocate(&mut map, player, &respawns, -467.0, 95.0, -189.0);
    tick(&mut map, &mut now, 10);
    let _ = &respawns;
    assert!(
        visible_creatures(&map, player, 8580) >= 1,
        "Atal'arion visible after circle completes"
    );
    map.instance_load("3 1 0 0 0 0");
    assert_eq!(map.instance_get_data(5), Some(0), "IN_PROGRESS resets");
    println!("LIVE-SUNKEN-TEMPLE-PASS");
}
