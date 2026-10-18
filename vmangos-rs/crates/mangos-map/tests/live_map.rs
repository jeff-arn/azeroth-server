//! Live map behavior on the dev bench: spawn activation,
//! visibility, init-self, and two-player movement relay.
//! `#[ignore]`d; gated on `TEST_MYSQL_URL_WORLD`,
//! `TEST_MYSQL_URL_CHARACTERS`, `CHARACTER_LOAD_GUID`,
//! `MANGOS_DBC_DIR`. Read-only (respawn clears surface as
//! events only).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::{Map, MapConfig, MapEvent, WorldRng, WorldStores};
use mangos_movement::movement_info::{MovementInfo, Position};
use mangos_protocol::opcode_table::opcode_id;
use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
use mangos_world_data::item_prototype::load_item_prototypes_lite;
use mangos_world_data::player_info::{RaceClassDbc, RaceDbc, load_player_infos};
use mangos_world_data::spawn_support::{
    RespawnTimes, load_class_level_stats, load_creature_addons, load_display_info_addons,
    load_equipment_templates,
};
use mangos_world_data::spawns::{SpawnData, load_map_ids};

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        other => panic!("field {idx}: expected UInt, got {other:?}"),
    }
}

fn race_class_dbc() -> RaceClassDbc {
    let dir: PathBuf = std::env::var_os("MANGOS_DBC_DIR")
        .expect("MANGOS_DBC_DIR")
        .into();
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

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_map_visibility_and_relay() {
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD");
    let chars_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID")
        .parse()
        .expect("guid");
    let dbc = race_class_dbc();

    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let (stores, dbc) = world
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
            let (player_infos, _) = load_player_infos(conn, 60, Some(&dbc), Some(&map_ids), None)?;
            let items = load_item_prototypes_lite(conn, 10)?;
            let mut wp_report = mangos_world_data::load_report::LoadReport::default();
            let waypoints = mangos_world_data::waypoints::load_waypoints(conn, &mut wp_report)?;
            Ok((
                WorldStores {
                    creature_templates,
                    gameobject_templates,
                    spawns,
                    creature_addons,
                    equipment,
                    cls,
                    display_addons,
                    player_infos,
                    items,
                    waypoints,
                    spells: mangos_spells::SpellStore::default(),
                    creature_spells: mangos_world_data::creature_spells::CreatureSpellsMap::new(),
                    scripts: mangos_scripts_engine::ScriptStores::default(),
                    display_scales: HashMap::new(),
                    model_data: HashMap::new(),
                },
                dbc,
            ))
        })
        .expect("stores");
    let stores = Arc::new(stores);

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
    };
    let player1 = Player::materialize(load.clone(), &inputs).expect("player1");
    let guid1 = player1.unit.guid();
    let pos1 = player1.unit.world.position();

    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(42),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player1, &respawns, 1_800_000_000, 1000);
    assert!(map.creature_count() > 0, "cells around Westfall must spawn");
    let visible = map.visible_to(guid1).cloned().expect("visible set");
    assert!(!visible.is_empty(), "bench spot has visible objects");
    // Every visible creature is within visibility + grey.
    for guid in &visible {
        if let Some(c) = map.creature(*guid) {
            let p = c.unit.world.position();
            let d = ((p.x - pos1.x).powi(2) + (p.y - pos1.y).powi(2)).sqrt();
            assert!(d <= 101.0, "creature at {d} beyond grey edge");
        }
    }
    // Creates were emitted as SMSG_UPDATE_OBJECT packets.
    map.build_init_self(guid1);
    let events = map.drain_events();
    let update_packets = events
        .iter()
        .filter(|e| {
            matches!(e, MapEvent::Packet { to, packet } if *to == guid1
            && (packet.opcode() == opcode_id::SMSG_UPDATE_OBJECT
                || packet.opcode() == opcode_id::SMSG_COMPRESSED_UPDATE_OBJECT))
        })
        .count();
    assert!(update_packets >= 2, "visibility creates + init-self");

    // Second player 10 yd east: mutual visibility + relay.
    let mut load2 = load.clone();
    load2.row.guid += 100_000;
    load2.row.position_x += 10.0;
    let player2 = Player::materialize(load2, &inputs).expect("player2");
    let guid2 = player2.unit.guid();
    map.add_player(player2, &respawns, 1_800_000_000, 1050);
    assert!(map.visible_to(guid1).expect("v1").contains(&guid2));
    assert!(map.visible_to(guid2).expect("v2").contains(&guid1));
    let _ = map.drain_events();

    let info = MovementInfo {
        pos: Position::new(pos1.x + 5.0, pos1.y, pos1.z, pos1.o),
        server_time: 1100,
        ..MovementInfo::default()
    };
    assert!(map.apply_movement(guid1, opcode_id::MSG_MOVE_HEARTBEAT, info, &respawns));
    let events = map.drain_events();
    let relayed: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, MapEvent::Packet { to, packet } if *to == guid2 && packet.opcode() == opcode_id::MSG_MOVE_HEARTBEAT))
        .collect();
    assert_eq!(relayed.len(), 1, "exactly one relay to the watcher");
    let MapEvent::Packet { packet, .. } = relayed[0] else {
        unreachable!()
    };
    // Payload starts with the mover's packed GUID then the
    // MovementInfo (28-byte base when flagless).
    let mut packed = Vec::new();
    guid1.write_packed(&mut packed);
    assert!(packet.body().starts_with(&packed));
    assert_eq!(packet.body().len(), packed.len() + 28);
    // The mover does not receive its own relay.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, MapEvent::Packet { to, .. } if *to == guid1))
    );

    // Move far away: relocation scan drops mutual visibility
    // and emits out-of-range updates.
    let far = MovementInfo {
        pos: Position::new(pos1.x + 500.0, pos1.y, pos1.z, pos1.o),
        server_time: 1200,
        ..MovementInfo::default()
    };
    assert!(map.apply_movement(guid1, opcode_id::MSG_MOVE_HEARTBEAT, far, &respawns));
    map.tick(150, 1350, 1_800_000_000);
    assert!(!map.visible_to(guid2).expect("v2").contains(&guid1));
    assert!(!map.visible_to(guid1).expect("v1").contains(&guid2));
    let events = map.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MapEvent::Packet { to, packet }
            if *to == guid2 && packet.opcode() == opcode_id::SMSG_UPDATE_OBJECT)),
        "out-of-range destroy reaches the watcher"
    );

    // Removal returns the aggregate.
    let removed = map.remove_player(guid2).expect("remove");
    assert_eq!(removed.unit.guid(), guid2);
    let _: ObjectGuid = guid2;
}
