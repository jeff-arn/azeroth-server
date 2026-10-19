//! Live Deadmines vertical-slice evidence (Slice 7
//! hand-port) on the dev bench: a real instance map
//! (36, instance 1) walked through the ported chains —
//! hidden patrols, Rhahk'Zor → door 1 + patrol
//! activation, gunpowder temp-summon walk, Shredder →
//! Eject Sneed (spell summon) → door 2, Gilnid →
//! door 3, cannon → iron door + Smite alarms, and the
//! Mr. Smite 66%/33% equip phases. All expected values
//! come from independent SQL in this test.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::{Map, MapConfig, MapEvent, WorldRng, WorldStores};
use mangos_movement::movement_info::Position;
use mangos_protocol::opcode_table::opcode_id;
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

fn parse_monster_chat(body: &[u8]) -> Option<(u8, u64, String)> {
    let msg_type = *body.first()?;
    let mut off = 5usize;
    let read_cstring = |body: &[u8], off: usize| -> Option<(String, usize)> {
        let end = body[off..].iter().position(|b| *b == 0)? + off;
        Some((
            String::from_utf8_lossy(&body[off..end]).into_owned(),
            end + 1,
        ))
    };
    let sender_raw = match msg_type {
        0x0D | 0x1A => {
            let name_len = u32::from_le_bytes(body.get(off..off + 4)?.try_into().ok()?) as usize;
            off += 4 + name_len + 8;
            0
        }
        0x0B | 0x0C => {
            let raw = u64::from_le_bytes(body.get(off..off + 8)?.try_into().ok()?);
            off += 8;
            let name_len = u32::from_le_bytes(body.get(off..off + 4)?.try_into().ok()?) as usize;
            off += 4 + name_len + 8;
            raw
        }
        _ => return None,
    };
    off += 4;
    let (text, _) = read_cstring(body, off)?;
    Some((msg_type, sender_raw, text))
}

struct Oracle {
    rhahk_pos: (f32, f32, f32),
    shredder_pos: (f32, f32, f32),
    gilnid_pos: (f32, f32, f32),
    smite_pos: (f32, f32, f32),
    door_states: HashMap<u32, u32>,
    go_pos: HashMap<u32, (f32, f32, f32)>,
    patrol_pos: (f32, f32, f32),
    axe_display: u32,
    hammer_display: u32,
    alarm1: String,
    alarm2: String,
    say_phase2: String,
    say_phase3: String,
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)]
fn bench_deadmines_slice() {
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD");
    let chars_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID")
        .parse()
        .expect("guid");
    let dbc_dir: PathBuf = std::env::var_os("MANGOS_DBC_DIR")
        .expect("MANGOS_DBC_DIR")
        .into();
    let dbc = race_class_dbc(&dbc_dir);

    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let (stores, oracle) = world
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

            let creature_pos = |conn: &mut mysql::Conn, entry: u32| -> (f32, f32, f32) {
                conn.query_first(format!(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE map=36 AND id={entry} ORDER BY guid LIMIT 1"
                ))
                .map_err(mangos_db::DbError::from)
                .expect("query")
                .expect("spawn row")
            };
            let go_rows: Vec<(u32, f32, f32, f32, u32)> = conn
                .query(
                    "SELECT id, position_x, position_y, position_z, state FROM gameobject \
                     WHERE map=36 AND id IN (13965,16400,16399,16397,16398,17155,144111)",
                )
                .map_err(mangos_db::DbError::from)?;
            let mut door_states = HashMap::new();
            let mut go_pos = HashMap::new();
            for (id, x, y, z, state) in go_rows {
                door_states.insert(id, state);
                go_pos.insert(id, (x, y, z));
            }
            let patrol_pos: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE map=36 AND spawntimesecsmin=43199 ORDER BY guid LIMIT 1",
                )
                .map_err(mangos_db::DbError::from)?;
            let item_display = |conn: &mut mysql::Conn, entry: u32| -> u32 {
                conn.query_first(format!(
                    "SELECT display_id FROM item_template WHERE entry={entry}"
                ))
                .map_err(mangos_db::DbError::from)
                .expect("query")
                .expect("item row")
            };
            let text_of = |conn: &mut mysql::Conn, id: u32| -> String {
                conn.query_first(format!(
                    "SELECT male_text FROM broadcast_text WHERE entry={id}"
                ))
                .map_err(mangos_db::DbError::from)
                .expect("query")
                .expect("text row")
            };
            let oracle = Oracle {
                rhahk_pos: creature_pos(conn, 644),
                shredder_pos: creature_pos(conn, 642),
                gilnid_pos: creature_pos(conn, 1763),
                smite_pos: creature_pos(conn, 646),
                door_states,
                go_pos,
                patrol_pos: patrol_pos.expect("patrol spawn"),
                axe_display: item_display(conn, 2183),
                hammer_display: item_display(conn, 10756),
                alarm1: text_of(conn, 1148),
                alarm2: text_of(conn, 1149),
                say_phase2: text_of(conn, 1344),
                say_phase3: text_of(conn, 1345),
            };
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
                    spells: mangos_spells::SpellStore {
                        spells: spell_entries,
                        tables: mangos_spells::load_spell_dbc_tables(std::path::Path::new(
                            &std::env::var("MANGOS_DBC_DIR").expect("MANGOS_DBC_DIR"),
                        ))
                        .expect("spell dbc tables"),
                    },
                    creature_spells,
                    scripts,
                    display_scales: HashMap::new(),
                    model_data: HashMap::new(),
                    skills: mangos_world_data::skill_info::SkillDbc::default(),
                    chat_channels: mangos_world_data::chat_channels::ChatChannelDbc::default(),
                    auction_houses: mangos_world_data::auction_house::AuctionHouseDbc::default(),
                },
                oracle,
            ))
        })
        .expect("stores");
    for (entry, state) in &oracle.door_states {
        assert_eq!(*state, 1, "oracle: GO {entry} must start closed (state 1)");
    }
    println!(
        "ORACLE axe={} hammer={} alarm1={:?} say2={:?}",
        oracle.axe_display, oracle.hammer_display, oracle.alarm1, oracle.say_phase2
    );

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
        skills: None,
    };
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();

    let mut map = Map::new(
        36,
        1,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(7),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    let _ = map.drain_events();
    let mut now_ms = 1000u32;
    let mut chat_log: Vec<(u8, u64, String)> = Vec::new();
    let drain = |map: &mut Map, chats: &mut Vec<(u8, u64, String)>| {
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_MESSAGECHAT
                && let Some(parsed) = parse_monster_chat(packet.body())
            {
                chats.push(parsed);
            }
        }
    };
    macro_rules! tick {
        ($n:expr) => {
            for _ in 0..$n {
                now_ms += 50;
                map.tick(50, now_ms, 1_800_000_000);
                drain(&mut map, &mut chat_log);
            }
        };
    }
    let relocate = |map: &mut Map, x: f32, y: f32, z: f32| {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(x, y, z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    };
    let heal = |map: &mut Map| {
        let p = map.player_mut(player_guid).expect("player");
        p.unit
            .world
            .object
            .values_mut()
            .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
        p.unit.set_health(100_000);
    };
    heal(&mut map);
    let find_creature = |map: &Map, entry: u32| -> Option<ObjectGuid> {
        let visible = map.visible_to(player_guid)?;
        visible
            .iter()
            .copied()
            .find(|g| map.creature(*g).is_some_and(|c| c.info.entry == entry))
    };
    let find_go = |map: &Map, entry: u32| -> Option<ObjectGuid> {
        let visible = map.visible_to(player_guid)?;
        visible.iter().copied().find(|g| {
            map.gameobject(*g).is_some_and(|go| {
                go.world
                    .object
                    .values()
                    .get_u32(offsets::OBJECT_FIELD_ENTRY)
                    == entry
            })
        })
    };
    let go_state = |map: &Map, guid: ObjectGuid| -> u32 {
        map.gameobject(guid).map_or(u32::MAX, |go| {
            go.world.object.values().get_u32(offsets::GAMEOBJECT_STATE)
        })
    };
    // Kill by player swings from 1 HP.
    let kill =
        |map: &mut Map, target: ObjectGuid, now_ms: &mut u32, sink: &mut Vec<(u8, u64, String)>| {
            if let Some(c) = map.creature_mut(target) {
                c.unit.set_health(1);
            }
            map.attack_start(player_guid, target);
            for _ in 0..400 {
                *now_ms += 50;
                map.tick(50, *now_ms, 1_800_000_000);
                drain(map, sink);
                if !map.creature(target).is_some_and(|c| c.unit.is_alive()) {
                    break;
                }
            }
            map.attack_stop(player_guid);
            assert!(
                !map.creature(target).is_some_and(|c| c.unit.is_alive()),
                "target did not die"
            );
        };

    // ---- Rhahk'Zor: door 1 + hidden patrol -----------
    relocate(
        &mut map,
        oracle.rhahk_pos.0 + 3.0,
        oracle.rhahk_pos.1,
        oracle.rhahk_pos.2,
    );
    tick!(6);
    let rhahk = find_creature(&map, 644).expect("Rhahk'Zor in view");
    let door1 = find_go(&map, 13965).expect("door 1 in view");
    assert_eq!(go_state(&map, door1), 1, "door 1 starts closed");
    // The 43199 patrol must still be hidden: standing on
    // a patrol spawn shows nothing of its entry there.
    relocate(
        &mut map,
        oracle.patrol_pos.0 + 2.0,
        oracle.patrol_pos.1,
        oracle.patrol_pos.2,
    );
    tick!(6);
    let patrol_visible_before = {
        let visible = map.visible_to(player_guid).cloned().unwrap_or_default();
        visible
            .iter()
            .filter(|g| {
                map.creature(**g)
                    .is_some_and(|c| c.data.as_ref().map_or(0, |d| d.spawntimesecsmin) == 43199)
            })
            .count()
    };
    println!("PATROL hidden_before_visible={patrol_visible_before}");
    assert_eq!(patrol_visible_before, 0, "patrol must start hidden");
    relocate(
        &mut map,
        oracle.rhahk_pos.0 + 3.0,
        oracle.rhahk_pos.1,
        oracle.rhahk_pos.2,
    );
    tick!(4);
    kill(&mut map, rhahk, &mut now_ms, &mut chat_log);
    tick!(4);
    assert_eq!(go_state(&map, door1), 0, "door 1 opens on Rhahk'Zor death");
    // 60 s later the patrol activates (visible at its
    // spawn point).
    tick!(1240);
    relocate(
        &mut map,
        oracle.patrol_pos.0 + 2.0,
        oracle.patrol_pos.1,
        oracle.patrol_pos.2,
    );
    tick!(6);
    let patrol_visible_after = {
        let visible = map.visible_to(player_guid).cloned().unwrap_or_default();
        visible
            .iter()
            .filter(|g| {
                map.creature(**g)
                    .is_some_and(|c| c.data.as_ref().map_or(0, |d| d.spawntimesecsmin) == 43199)
            })
            .count()
    };
    println!("PATROL visible_after={patrol_visible_after}");
    assert!(
        patrol_visible_after >= 1,
        "patrol activates 60 s after Rhahk'Zor"
    );
    heal(&mut map);

    // ---- Gunpowder summon walk ------------------------
    let gunpowder_pos = oracle.go_pos[&17155];
    relocate(
        &mut map,
        gunpowder_pos.0 + 2.0,
        gunpowder_pos.1,
        gunpowder_pos.2,
    );
    tick!(6);
    let gunpowder = find_go(&map, 17155).expect("gunpowder in view");
    map.gameobject_use(player_guid, gunpowder);
    tick!(4);
    let overseer = {
        let visible = map.visible_to(player_guid).cloned().unwrap_or_default();
        let spawn = Position::new(-131.290_83, -591.2431, 18.077_19, 0.0);
        visible
            .iter()
            .copied()
            .filter_map(|g| {
                map.creature(g)
                    .and_then(|c| (c.info.entry == 634).then_some((g, c.unit.world.position())))
            })
            .filter(|(_, pos)| dist2d(spawn, *pos) < 8.0)
            .min_by(|a, b| dist2d(spawn, a.1).total_cmp(&dist2d(spawn, b.1)))
            .map(|(g, _)| g)
            .expect("summoned overseer near the C++ summon point")
    };
    let walk_dest = Position::new(-115.263_67, -617.396_1, 13.579_387, 0.0);
    let start_dist = dist2d(
        walk_dest,
        map.creature(overseer)
            .expect("overseer")
            .unit
            .world
            .position(),
    );
    // Keep the player near the walk so cells stay active.
    relocate(
        &mut map,
        gunpowder_pos.0,
        gunpowder_pos.1 - 6.0,
        gunpowder_pos.2,
    );
    tick!(300);
    let end_dist = dist2d(
        walk_dest,
        map.creature(overseer)
            .expect("overseer")
            .unit
            .world
            .position(),
    );
    println!("GUNPOWDER overseer walk {start_dist:.1} -> {end_dist:.1}");
    assert!(
        end_dist < start_dist - 5.0,
        "overseer walks the scripted route"
    );
    heal(&mut map);

    // ---- Shredder -> Sneed -> door 2, Gilnid -> door 3
    let door2_pos = oracle.go_pos[&16400];
    relocate(&mut map, door2_pos.0 + 2.0, door2_pos.1, door2_pos.2);
    tick!(6);
    let door2 = find_go(&map, 16400).expect("door 2 in view");
    assert_eq!(go_state(&map, door2), 1, "door 2 starts closed");
    relocate(
        &mut map,
        oracle.shredder_pos.0 + 3.0,
        oracle.shredder_pos.1,
        oracle.shredder_pos.2,
    );
    tick!(6);
    let shredder = find_creature(&map, 642).expect("Shredder in view");
    kill(&mut map, shredder, &mut now_ms, &mut chat_log);
    let mut sneed = None;
    for _ in 0..200 {
        tick!(1);
        if let Some(g) = find_creature(&map, 643) {
            sneed = Some(g);
            break;
        }
    }
    let sneed = sneed.expect("Sneed ejected by spell 5141 on Shredder death");
    println!("SNEED ejected=true");
    heal(&mut map);
    kill(&mut map, sneed, &mut now_ms, &mut chat_log);
    tick!(4);
    assert_eq!(go_state(&map, door2), 0, "door 2 opens on Sneed death");

    let door3_pos = oracle.go_pos[&16399];
    relocate(&mut map, door3_pos.0 + 2.0, door3_pos.1, door3_pos.2);
    tick!(6);
    let door3 = find_go(&map, 16399).expect("door 3 in view");
    assert_eq!(go_state(&map, door3), 1, "door 3 starts closed");
    relocate(
        &mut map,
        oracle.gilnid_pos.0 + 3.0,
        oracle.gilnid_pos.1,
        oracle.gilnid_pos.2,
    );
    tick!(6);
    heal(&mut map);
    let gilnid = find_creature(&map, 1763).expect("Gilnid in view");
    kill(&mut map, gilnid, &mut now_ms, &mut chat_log);
    tick!(4);
    assert_eq!(go_state(&map, door3), 0, "door 3 opens on Gilnid death");
    println!("DOORS 1/2/3 open on boss deaths");

    // ---- Cannon -> iron door + alarms -----------------
    let cannon_pos = oracle.go_pos[&16398];
    let iron_pos = oracle.go_pos[&16397];
    relocate(&mut map, iron_pos.0 + 2.0, iron_pos.1, iron_pos.2);
    tick!(6);
    let iron = find_go(&map, 16397).expect("iron door in view");
    assert_eq!(go_state(&map, iron), 1, "iron door starts closed");
    relocate(&mut map, cannon_pos.0 + 2.0, cannon_pos.1, cannon_pos.2);
    tick!(6);
    heal(&mut map);
    let cannon = find_go(&map, 16398).expect("cannon in view");
    chat_log.clear();
    map.gameobject_use(player_guid, cannon);
    tick!(4);
    assert_eq!(go_state(&map, iron), 0, "cannon blast opens the iron door");
    // Smite activates on cell load near the ship before
    // the alarms can name him.
    relocate(
        &mut map,
        oracle.smite_pos.0 + 30.0,
        oracle.smite_pos.1,
        oracle.smite_pos.2,
    );
    tick!(500);
    let alarm1_seen = chat_log.iter().any(|(_, _, t)| *t == oracle.alarm1);
    let alarm2_seen = chat_log.iter().any(|(_, _, t)| *t == oracle.alarm2);
    println!(
        "CANNON alarm1={alarm1_seen} alarm2={alarm2_seen} chats={}",
        chat_log.len()
    );
    assert!(alarm1_seen, "INST_SAY_ALARM1 from Mr. Smite");
    assert!(alarm2_seen, "INST_SAY_ALARM2 from Mr. Smite");

    // ---- Mr. Smite equip phases -----------------------
    heal(&mut map);
    let smite = find_creature(&map, 646).expect("Mr. Smite in view");
    let chest_pos = {
        let (x, y, z) = oracle.go_pos[&144_111];
        Position::new(x, y, z, 0.0)
    };
    relocate(
        &mut map,
        oracle.smite_pos.0 + 8.0,
        oracle.smite_pos.1,
        oracle.smite_pos.2,
    );
    for _ in 0..200 {
        tick!(1);
        if map.ai_in_combat(smite) {
            break;
        }
    }
    assert!(map.ai_in_combat(smite), "Smite engaged");
    let smite_max = map
        .creature(smite)
        .expect("smite")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_MAXHEALTH);
    let slot_display = |map: &Map, slot: u16| -> u32 {
        map.creature(smite).map_or(0, |c| {
            c.unit
                .world
                .object
                .values()
                .get_u32(offsets::UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot)
        })
    };
    // Phase 2 at <66%.
    chat_log.clear();
    if let Some(c) = map.creature_mut(smite) {
        c.unit.set_health(smite_max * 65 / 100);
    }
    let mut axe_seen = false;
    let mut kneel_seen = false;
    let mut near_chest = false;
    for _ in 0..600 {
        tick!(1);
        let c = map.creature(smite).expect("smite");
        let bytes1 = c
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_BYTES_1);
        if bytes1 & 0xFF == 8 {
            kneel_seen = true;
            if dist2d(chest_pos, c.unit.world.position()) < 6.0 {
                near_chest = true;
            }
        }
        if slot_display(&map, 0) == oracle.axe_display {
            axe_seen = true;
            break;
        }
    }
    let say2_seen = chat_log.iter().any(|(_, _, t)| *t == oracle.say_phase2);
    println!("SMITE p2 say={say2_seen} kneel={kneel_seen} near_chest={near_chest} axe={axe_seen}");
    assert!(say2_seen, "SAY_PHASE_2 broadcast text");
    assert!(kneel_seen, "Smite kneels at the chest");
    assert!(near_chest, "Smite kneels next to the chest");
    assert!(axe_seen, "Smite equips the dual axes (SQL display id)");
    // Let him re-engage, then phase 3 at <33%.
    tick!(60);
    heal(&mut map);
    chat_log.clear();
    if let Some(c) = map.creature_mut(smite) {
        c.unit.set_health(smite_max * 30 / 100);
    }
    let mut hammer_seen = false;
    for _ in 0..600 {
        tick!(1);
        if slot_display(&map, 0) == oracle.hammer_display {
            hammer_seen = true;
            break;
        }
    }
    let say3_seen = chat_log.iter().any(|(_, _, t)| *t == oracle.say_phase3);
    println!("SMITE p3 say={say3_seen} hammer={hammer_seen}");
    assert!(say3_seen, "SAY_PHASE_3 broadcast text");
    assert!(hammer_seen, "Smite equips the hammer (SQL display id)");

    println!("LIVE-DEADMINES-PASS");
}
