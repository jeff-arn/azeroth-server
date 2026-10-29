//! Tier 3 Slice 3 step 3 (Warsong Gulch) live
//! evidence on the dev bench: match setup, fixed
//! graveyards, score counters, flag-taken timer path
//! and win honor through the production `bg_ws.rs`.
//! Expected values from fork `BattleGroundWS.cpp`
//! constants (STEP 3 marker `LIVE-WSG-PASS`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::{Map, MapConfig, WorldRng, WorldStores};
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
                safe_locs: load_safe_locs(),
                battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates::default(),
                battlemasters: mangos_world_data::battlegrounds::BattleMasters::default(),
                battle_event_indexes: mangos_world_data::battlegrounds::BattleEventIndexes::default(
                ),
                graveyard_links: mangos_world_data::battlegrounds::GraveyardLinks::default(),
                bg_entrances: mangos_world_data::battlegrounds::BattlegroundEntrances::default(),
                area_triggers: mangos_world_data::area_trigger::AreaTriggerStore::default(),
                quests: mangos_world_data::quest_template::QuestTemplatesLite::default(),
                quest_templates: mangos_world_data::quest_template::load_quest_templates(conn, 10)?,
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

fn heal(map: &mut Map, player: ObjectGuid) {
    let p = map.player_mut(player).expect("player");
    p.unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
    p.unit.set_health(100_000);
}

fn load_safe_locs() -> mangos_world_data::battlegrounds::SafeLocStore {
    let dbc_dir = std::env::var("MANGOS_DBC_DIR").expect("MANGOS_DBC_DIR");
    let bytes = std::fs::read(std::path::Path::new(&dbc_dir).join(formats::WORLD_SAFE_LOCS.file))
        .expect("WorldSafeLocs.dbc");
    let store = DbcFile::parse(&bytes, formats::WORLD_SAFE_LOCS.fmt)
        .expect("parse")
        .produce(formats::WORLD_SAFE_LOCS.fmt)
        .expect("produce");
    let rows: Vec<(u32, u32, f32, f32, f32)> = store
        .rows()
        .iter()
        .map(|row| {
            let float = |i: usize| match row.get(i) {
                Some(FieldValue::Float(v)) => *v,
                other => panic!("field {i}: expected Float, got {other:?}"),
            };
            (uint(row, 0), uint(row, 1), float(2), float(3), float(4))
        })
        .collect();
    mangos_world_data::battlegrounds::SafeLocStore::from_rows(&rows, &HashMap::new())
}

use mangos_entities::unit::Team;
use mangos_map::battlegrounds::{BgInit, bg_status};
use mangos_map::map::MapEvent;
use mangos_protocol::world::WorldPacket;

fn expect_worldstate(packets: &[WorldPacket], field: u32, value: u32, what: &str) {
    let want: Vec<u8> = field
        .to_le_bytes()
        .into_iter()
        .chain(value.to_le_bytes())
        .collect();
    assert!(
        packets
            .iter()
            .any(|p| p.body().windows(8).any(|w| w == want.as_slice())),
        "{what}: no worldstate {field}={value} in {} packets",
        packets.len()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
#[ignore = "live bench DB"]
fn live_wsg_match() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 489);

    let mut init = BgInit {
        bg_type_id: 2,
        bracket_id: 5,
        client_instance_id: 1,
        name: "Warsong Gulch".to_string(),
        min_players_per_team: 8,
        max_players_per_team: 10,
        alliance_win_spell: 0,
        alliance_lose_spell: 0,
        horde_win_spell: 0,
        horde_lose_spell: 0,
        alliance_start: stores.safe_locs.get(769).expect("769"),
        horde_start: stores.safe_locs.get(770).expect("770"),
        player_skin_ref_loot_id: 0,
        level_min: 51,
        level_max: 60,
    };
    let _ = &mut init;
    map.init_battleground(init);
    map.bg_add_player(player);
    let _ = map.drain_events();

    let team_of = map.player(player).map_or(Team::None, |p| p.team);
    eprintln!("wsg probe: fixture team = {team_of:?}");
    let flagroom = if team_of == Team::Alliance { 769 } else { 770 };
    let main_gy = if team_of == Team::Alliance { 771 } else { 772 };

    // pre-start graveyard = flagroom
    let gy = map.ws_closest_graveyard(player).expect("graveyard");
    assert_eq!(gy.id, flagroom, "pre-start graveyard");

    let gy_events: Vec<MapEvent> = map.drain_events();
    let _gy_packets: Vec<WorldPacket> = gy_events
        .iter()
        .filter_map(|e| match e {
            MapEvent::Packet { packet, .. } => Some(packet.clone()),
            _ => None,
        })
        .collect();

    let mut now_ms = 1000u32;
    // Starting system runs the full 2-minute start
    // delay before IN_PROGRESS.
    for _ in 0..2600 {
        tick(&mut map, &mut now_ms, 1);
        if map.bg.as_ref().map(|b| b.status) == Some(bg_status::IN_PROGRESS) {
            break;
        }
    }

    let bg = map.bg.as_ref().expect("bg");
    let _ = &respawns;
    assert_eq!(bg.status, bg_status::IN_PROGRESS, "match started");
    assert_eq!(bg.active_events.get(&0), Some(&0), "flag A active");
    let gy = map.ws_closest_graveyard(player).expect("graveyard");
    assert_eq!(gy.id, main_gy, "in-progress graveyard");

    let enemy = if team_of == Team::Alliance {
        Team::Horde
    } else {
        Team::Alliance
    };
    let enemy_idx = mangos_map::battlegrounds::team_index(enemy);

    // score counters through production dispatch
    map.bg_update_player_score(player, 7, 2);
    map.bg_update_player_score(player, 8, 1);
    map.bg_update_player_score(player, 3, 1);
    let score = *map
        .bg
        .as_ref()
        .expect("bg")
        .scores
        .get(&player)
        .expect("score");
    assert_eq!(score.flag_captures, 2);
    assert_eq!(score.flag_returns, 1);
    assert_eq!(score.honorable_kills, 1);

    // flag-taken drop timer: simulate pickup + drop
    if let Some(mangos_map::battlegrounds::BgKind::Ws(ws)) = map.bg.as_mut().map(|b| &mut b.kind) {
        ws.flag_keepers[enemy_idx] = player;
        ws.flag_state[enemy_idx] = 2;
    }
    map.bg_event_dropped_flag(player);
    let events = map.drain_events();
    let packets: Vec<WorldPacket> = events
        .iter()
        .filter_map(|e| match e {
            MapEvent::Packet { packet, .. } => Some(packet.clone()),
            _ => None,
        })
        .collect();
    let want_taken = if team_of == Team::Alliance {
        1546
    } else {
        1545
    };
    expect_worldstate(&packets, want_taken, u32::MAX, "flag dropped worldstate");
    let state =
        if let Some(mangos_map::battlegrounds::BgKind::Ws(ws)) = map.bg.as_ref().map(|b| &b.kind) {
            ws.flag_state[enemy_idx]
        } else {
            99
        };
    assert_eq!(state, 3, "flag on ground");
    tick(&mut map, &mut now_ms, 220);
    let state =
        if let Some(mangos_map::battlegrounds::BgKind::Ws(ws)) = map.bg.as_ref().map(|b| &b.kind) {
            ws.flag_state[enemy_idx]
        } else {
            99
        };
    assert_eq!(state, 0, "flag respawned at base after drop time");

    // win honor at end (bracket 5: WinMatchHonor 198)
    let before = map
        .bg
        .as_ref()
        .and_then(|b| b.scores.get(&player))
        .map_or(0, |s| s.bonus_honor);
    map.bg_end_battleground(team_of);
    let after = map
        .bg
        .as_ref()
        .and_then(|b| b.scores.get(&player))
        .map_or(0, |s| s.bonus_honor);
    assert_eq!(after - before, 198, "win honor bracket 5");
    assert_eq!(
        map.bg.as_ref().expect("bg").status,
        bg_status::WAIT_LEAVE,
        "match ended"
    );
    println!("LIVE-WSG-PASS");
}
