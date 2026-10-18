//! Live server movement on the dev bench (Slice 4):
//! waypoint store counts vs an independent SQL oracle,
//! ground height at the bench character's saved position,
//! wandering/waypoint creatures actually moving under
//! ticks, and Point -> Home round trips.
//! `#[ignore]`d; gated on `TEST_MYSQL_URL_WORLD`,
//! `TEST_MYSQL_URL_CHARACTERS`, `CHARACTER_LOAD_GUID`,
//! `MANGOS_DBC_DIR`, `MANGOS_MAPS_DIR`. Read-only.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_map::motion::{POINT_MOTION_TYPE, RANDOM_MOTION_TYPE, WAYPOINT_MOTION_TYPE};
use mangos_map::{Map, MapConfig, MapEvent, WorldRng, WorldStores};
use mangos_movement::movement_info::Position;
use mangos_protocol::opcode_table::opcode_id;
use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
use mangos_world_data::item_prototype::load_item_prototypes_lite;
use mangos_world_data::load_report::LoadReport;
use mangos_world_data::player_info::{RaceClassDbc, RaceDbc, load_player_infos};
use mangos_world_data::spawn_support::{
    RespawnTimes, load_class_level_stats, load_creature_addons, load_display_info_addons,
    load_equipment_templates,
};
use mangos_world_data::spawns::{SpawnData, load_map_ids};
use mangos_world_data::waypoints::load_waypoints;

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

fn dist2d(a: Position, b: Position) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR, MANGOS_MAPS_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_creatures_move() {
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD");
    let chars_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID")
        .parse()
        .expect("guid");
    let maps_dir: PathBuf = std::env::var_os("MANGOS_MAPS_DIR")
        .expect("MANGOS_MAPS_DIR")
        .into();
    let dbc = race_class_dbc();

    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let (stores, dbc, oracle) = world
        .call(move |conn| {
            use mysql::prelude::Queryable;
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
            let mut report = LoadReport::default();
            let waypoints = load_waypoints(conn, &mut report)?;
            // Independent oracle: raw counts straight from
            // SQL (distinct path ids + total rows per
            // table).
            let count = |conn: &mut mysql::Conn, sql: &str| -> Result<usize, mangos_db::DbError> {
                let v: Option<i64> = conn.query_first(sql).map_err(mangos_db::DbError::from)?;
                Ok(usize::try_from(v.unwrap_or(0)).expect("non-negative count"))
            };
            let oracle = (
                count(conn, "SELECT COUNT(DISTINCT id) FROM creature_movement")?,
                count(conn, "SELECT COUNT(*) FROM creature_movement")?,
                count(
                    conn,
                    "SELECT COUNT(DISTINCT entry) FROM creature_movement_template",
                )?,
                count(conn, "SELECT COUNT(*) FROM creature_movement_template")?,
                count(conn, "SELECT COUNT(*) FROM creature_movement_special")?,
            );
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
                    item_prototypes: mangos_world_data::item_prototype::ItemPrototypes::default(),
                    factions: mangos_world_data::reputations::FactionStore::default(),
                    areas: mangos_world_data::areas::AreaStore::default(),
                    weather_chances: mangos_world_data::weather::WeatherChanceStore::default(),
                    waypoints,
                    spells: mangos_spells::SpellStore::default(),
                    creature_spells: mangos_world_data::creature_spells::CreatureSpellsMap::new(),
                    scripts: mangos_scripts_engine::ScriptStores::default(),
                    display_scales: HashMap::new(),
                    model_data: HashMap::new(),
                },
                dbc,
                oracle,
            ))
        })
        .expect("stores");

    // Waypoint store vs SQL oracle.
    let wp = &stores.waypoints;
    assert_eq!(wp.by_guid.len(), oracle.0, "guid path count");
    assert_eq!(wp.by_entry.len(), oracle.2, "entry path count");
    let nodes_in = |m: &HashMap<u32, mangos_world_data::waypoints::WaypointPath>| {
        m.values()
            .map(std::collections::BTreeMap::len)
            .sum::<usize>()
    };
    assert_eq!(nodes_in(&wp.by_guid), oracle.1, "guid nodes");
    assert_eq!(nodes_in(&wp.by_entry), oracle.3, "entry nodes");
    assert_eq!(nodes_in(&wp.special), oracle.4, "special nodes");

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
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid = player.unit.guid();
    let player_pos = player.unit.world.position();

    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(7),
    );
    map.set_maps_dir(Some(maps_dir));
    let navmesh_dir: PathBuf = std::env::var_os("MANGOS_MMAPS_DIR")
        .expect("MANGOS_MMAPS_DIR")
        .into();
    map.set_mmaps_dir(Some(navmesh_dir));
    assert!(
        map.navmesh_active_at(player_pos),
        "navmesh tile must cover the bench spot"
    );

    // Ground height vs the C++-saved character z (the
    // bench character stands on terrain; GridMap-only
    // height should land within a couple of yards).
    let ground = map.ground_z(player_pos.x, player_pos.y, player_pos.z);
    assert!(
        (ground - player_pos.z).abs() < 2.0,
        "ground {ground} vs saved z {}",
        player_pos.z
    );
    // The height must come from the tile, not the hint:
    // somewhere in the surrounding fields the terrain
    // height differs from the saved z.
    let mut saw_real_height = false;
    for d in [10.0f32, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0] {
        let g = map.ground_z(player_pos.x + d, player_pos.y, player_pos.z);
        if (g - player_pos.z).abs() > 0.05 {
            saw_real_height = true;
        }
    }
    assert!(saw_real_height, "heights are falling back to the hint");

    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    let _ = map.drain_events();

    // Find a moving-type creature in view.
    let visible = map.visible_to(player_guid).cloned().expect("visible");
    let mover = visible
        .iter()
        .copied()
        .filter(|g| map.creature(*g).is_some())
        .find(|g| {
            let t = map.motion_type_of(*g);
            t == RANDOM_MOTION_TYPE || t == WAYPOINT_MOTION_TYPE
        })
        .expect("a random/waypoint creature in view of the bench spot");
    let start = map.creature(mover).expect("mover").unit.world.position();

    // 20 simulated seconds: it must move and the player
    // must receive SMSG_MONSTER_MOVE for it.
    let mut monster_moves = 0usize;
    let mut now_ms = 1000u32;
    let tick_timer = std::time::Instant::now();
    for _ in 0..400 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_MONSTER_MOVE
            {
                monster_moves += 1;
            }
        }
    }
    // Perf probe (PHASE4-PROGRESS baseline): populated
    // Westfall map, 400 ticks of 50 ms.
    println!(
        "PERF ticks=400 creatures={} elapsed_ms={} us_per_tick={}",
        map.creature_count(),
        tick_timer.elapsed().as_millis(),
        tick_timer.elapsed().as_micros() / 400
    );
    let after = map.creature(mover).expect("mover").unit.world.position();
    assert!(
        dist2d(start, after) > 0.5 || monster_moves > 0,
        "creature never moved (start {start:?} after {after:?}, packets {monster_moves})"
    );
    assert!(monster_moves > 0, "no MONSTER_MOVE reached the player");

    // Point move 15 yd east, then Home: both generator
    // completions must surface, ending back at home.
    let creature = map.creature(mover).expect("mover");
    let home = creature.home_position;
    let here = creature.unit.world.position();
    let dest = Position::new(here.x + 15.0, here.y, here.z, here.o);
    map.creature_move_point(mover, 42, dest, 0, 0.0, -10.0);
    let mut point_informed = false;
    for _ in 0..600 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::MovementInform {
                guid,
                motion_type,
                point_id,
            } = event
                && guid == mover
                && motion_type == POINT_MOTION_TYPE
                && point_id == 42
            {
                point_informed = true;
            }
        }
        if point_informed {
            break;
        }
    }
    assert!(point_informed, "Point MovementInform never fired");
    let at_dest = map.creature(mover).expect("mover").unit.world.position();
    assert!(dist2d(at_dest, dest) < 1.0, "point arrival at {at_dest:?}");

    map.creature_move_home(mover);
    let mut reached_home = false;
    for _ in 0..1200 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::JustReachedHome { guid } = event
                && guid == mover
            {
                reached_home = true;
            }
        }
        if reached_home {
            break;
        }
    }
    assert!(reached_home, "JustReachedHome never fired");
    let back = map.creature(mover).expect("mover").unit.world.position();
    assert!(
        dist2d(back, home) < 1.0,
        "home arrival at {back:?} vs home {home:?}"
    );

    // Chase the (stationary) player: must close to melee
    // reach and fire ChaseReached; then the player steps
    // 25 yd east and the creature must follow and re-close.
    map.creature_move_chase(mover, player_guid, 0.0, 0.0);
    let mut reached = 0usize;
    for _ in 0..1200 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::ChaseReached { guid, target } = event
                && guid == mover
                && target == player_guid
            {
                reached += 1;
            }
        }
        if reached >= 1 {
            break;
        }
    }
    assert!(reached >= 1, "ChaseReached never fired on approach");
    let close = map.creature(mover).expect("mover").unit.world.position();
    assert!(
        dist2d(close, player_pos) < 8.0,
        "chaser stopped at {close:?}, player at {player_pos:?}"
    );

    let stepped = Position::new(
        player_pos.x + 25.0,
        player_pos.y,
        player_pos.z,
        player_pos.o,
    );
    let info = mangos_movement::movement_info::MovementInfo {
        pos: stepped,
        server_time: now_ms,
        ..mangos_movement::movement_info::MovementInfo::default()
    };
    assert!(map.apply_movement(player_guid, opcode_id::MSG_MOVE_HEARTBEAT, info, &respawns));
    let mut followed = false;
    for _ in 0..1200 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::ChaseReached { guid, target } = event
                && guid == mover
                && target == player_guid
            {
                followed = true;
            }
        }
        if followed {
            break;
        }
    }
    assert!(followed, "chaser did not re-close after the step");
    let again = map.creature(mover).expect("mover").unit.world.position();
    assert!(
        dist2d(again, stepped) < 8.0,
        "chaser at {again:?} after follow, player at {stepped:?}"
    );

    // Speed-change plumbing: a player speed change goes
    // through the pending/ACK loop (FORCE packet with
    // counter + flat speed to the controller only, real
    // rate unchanged until the ACK applies it); a
    // creature change applies immediately and broadcasts
    // the SPLINE_SET form.
    let _ = map.drain_events();
    map.set_speed_rate(player_guid, 1, 1.5); // MOVE_RUN
    let events = map.drain_events();
    let force: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            MapEvent::Packet { to, packet }
                if *to == player_guid
                    && packet.opcode() == opcode_id::SMSG_FORCE_RUN_SPEED_CHANGE =>
            {
                Some(packet)
            }
            _ => None,
        })
        .collect();
    assert_eq!(force.len(), 1, "one FORCE packet to the controller");
    let body = force[0].body();
    let mut packed = Vec::new();
    player_guid.write_packed(&mut packed);
    assert!(body.starts_with(&packed), "packed guid first");
    let rest = &body[packed.len()..];
    let counter = u32::from_le_bytes(rest[0..4].try_into().expect("counter"));
    let flat = f32::from_le_bytes(rest[4..8].try_into().expect("flat"));
    assert_eq!(counter, 0, "first movement counter is 0");
    assert!((flat - 10.5).abs() < 0.001, "flat = 1.5 * 7.0, got {flat}");
    let player = map.player(player_guid).expect("player");
    assert!(
        (player.unit.speed_rate[1] - 1.0).abs() == 0.0,
        "real rate must not change before the ACK"
    );
    assert_eq!(player.unit.pending_movement_changes.len(), 1);
    // Wrong counter must not consume the pending change;
    // the right one applies the real rate.
    let player = map.player_mut(player_guid).expect("player mut");
    assert!(!player.unit.find_pending_speed_change(flat, 99, 1));
    assert!(player.unit.find_pending_speed_change(flat, counter, 1));
    map.set_speed_rate_real(player_guid, 1, flat / 7.0);
    let player = map.player(player_guid).expect("player");
    assert!((player.unit.speed_rate[1] - 1.5).abs() < 0.001);
    assert!(player.unit.pending_movement_changes.is_empty());

    map.set_speed_rate(mover, 1, 2.0);
    let events = map.drain_events();
    let spline_set = events.iter().any(|e| {
        matches!(e, MapEvent::Packet { to, packet } if *to == player_guid
            && packet.opcode() == opcode_id::SMSG_SPLINE_SET_RUN_SPEED)
    });
    assert!(spline_set, "creature speed change broadcasts SPLINE_SET");
    let creature = map.creature(mover).expect("mover");
    assert!((creature.unit.speed_rate[1] - 2.0).abs() < 0.001);
}
