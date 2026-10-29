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
    baron_pos: (f32, f32, f32),
    fenrus_pos: (f32, f32, f32),
    vincent_pos: (f32, f32, f32),
    guard_pos: (f32, f32, f32),
    door_states: HashMap<u32, u32>,
    go_pos: HashMap<u32, (f32, f32, f32)>,
    haunt_period_ms: u32,
    haunt_summon_entry: u32,
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)]
fn bench_shadowfang_slice() {
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
                     WHERE map=33 AND id={entry} ORDER BY guid LIMIT 1"
                ))
                .map_err(mangos_db::DbError::from)
                .expect("query")
                .expect("spawn row")
            };
            let go_rows: Vec<(u32, f32, f32, f32, u32)> = conn
                .query(
                    "SELECT id, position_x, position_y, position_z, state FROM gameobject \
                     WHERE map=33 AND id IN (18895,18971,18972)",
                )
                .map_err(mangos_db::DbError::from)?;
            let mut door_states = HashMap::new();
            let mut go_pos = HashMap::new();
            for (id, x, y, z, state) in go_rows {
                door_states.insert(id, state);
                go_pos.insert(id, (x, y, z));
            }
            let guard_pos: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE map=33 AND id=3854 AND spawntimesecsmin=7201 ORDER BY guid LIMIT 1",
                )
                .map_err(mangos_db::DbError::from)?;
            let haunt = spell_entries.get(&7057).expect("7057 loaded");
            let haunt_period_ms = haunt.effects[0].amplitude;
            let haunt_summon_entry =
                spell_entries.get(&7067).expect("7067 loaded").effects[0].misc_value;
            let oracle = Oracle {
                baron_pos: creature_pos(conn, 3887),
                fenrus_pos: creature_pos(conn, 4274),
                vincent_pos: creature_pos(conn, 4444),
                guard_pos: guard_pos.expect("7201 wolf guard spawn"),
                door_states,
                go_pos,
                haunt_period_ms,
                haunt_summon_entry: u32::try_from(haunt_summon_entry).expect("summon entry"),
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
                    exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(
                    ),
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
                    battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates::default(
                    ),
                    battlemasters: mangos_world_data::battlegrounds::BattleMasters::default(),
                    battle_event_indexes:
                        mangos_world_data::battlegrounds::BattleEventIndexes::default(),
                    graveyard_links: mangos_world_data::battlegrounds::GraveyardLinks::default(),
                    bg_entrances: mangos_world_data::battlegrounds::BattlegroundEntrances::default(
                    ),
                    area_triggers: mangos_world_data::area_trigger::AreaTriggerStore::default(),
                    quests: mangos_world_data::quest_template::QuestTemplatesLite::default(),
                    quest_templates: mangos_world_data::quest_template::QuestTemplates::default(),
                    quest_relations: mangos_world_data::quest_template::QuestRelations::default(),
                    tavern_triggers: std::collections::HashSet::new(),
                },
                oracle,
            ))
        })
        .expect("stores");
    for (entry, state) in &oracle.door_states {
        assert_eq!(*state, 1, "oracle: GO {entry} must start closed (state 1)");
    }
    println!(
        "ORACLE haunt_period={} haunt_summon={}",
        oracle.haunt_period_ms, oracle.haunt_summon_entry
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
        item_prototypes: None,
        quest_templates: None,
    };
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();

    let mut map = Map::new(
        33,
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

    // ---- instance host: SFK variant from script_name --
    assert_eq!(
        map.instance_save(),
        Some(String::new()),
        "SFK instance created from map_template.script_name"
    );
    assert_eq!(map.instance_get_data(1), Some(0));

    // ---- TYPE_INTRO DONE before Vincent spawns: dead --
    assert!(map.instance_set_data(5, 3));
    assert_eq!(map.instance_save().as_deref(), Some("0 0 0 0 3 0"));
    relocate(
        &mut map,
        oracle.vincent_pos.0 + 2.0,
        oracle.vincent_pos.1,
        oracle.vincent_pos.2,
    );
    tick!(6);
    let vincent = find_creature(&map, 4444).expect("Vincent in view");
    let bytes1 = map
        .creature(vincent)
        .expect("vincent")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_BYTES_1);
    println!("VINCENT bytes1={bytes1:#x}");
    assert_eq!(
        bytes1 & 0xFF,
        7,
        "Vincent UNIT_STAND_STATE_DEAD after intro"
    );

    // ---- wolf guard hidden until Baron death ---------
    let guards_visible = |map: &Map| -> Vec<ObjectGuid> {
        let visible = map.visible_to(player_guid).cloned().unwrap_or_default();
        visible
            .iter()
            .copied()
            .filter(|g| {
                map.creature(*g).is_some_and(|c| {
                    c.info.entry == 3854
                        && c.data.as_ref().map_or(0, |d| d.spawntimesecsmin) == 7201
                })
            })
            .collect()
    };
    relocate(
        &mut map,
        oracle.guard_pos.0 + 2.0,
        oracle.guard_pos.1,
        oracle.guard_pos.2,
    );
    tick!(6);
    assert_eq!(guards_visible(&map).len(), 0, "wolf guard starts hidden");
    relocate(
        &mut map,
        oracle.baron_pos.0 + 3.0,
        oracle.baron_pos.1,
        oracle.baron_pos.2,
    );
    tick!(6);
    let baron = find_creature(&map, 3887).expect("Baron Silverlaine in view");
    kill(&mut map, baron, &mut now_ms, &mut chat_log);
    tick!(140); // 7 s > 6 s patrol timer
    relocate(
        &mut map,
        oracle.guard_pos.0 + 2.0,
        oracle.guard_pos.1,
        oracle.guard_pos.2,
    );
    tick!(6);
    let guards = guards_visible(&map);
    println!("GUARDS visible_after={}", guards.len());
    assert_eq!(guards.len(), 1, "7201 wolf guard revealed after Baron");
    let faction = map
        .creature(guards[0])
        .expect("guard")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE);
    assert_eq!(faction, 17, "revealed guard faction 17");
    heal(&mut map);

    // ---- Fenrus howl on enter combat -----------------
    relocate(
        &mut map,
        oracle.fenrus_pos.0 + 3.0,
        oracle.fenrus_pos.1,
        oracle.fenrus_pos.2,
    );
    let mut heard = false;
    for _ in 0..60 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_PLAY_SOUND
            {
                let body = packet.body();
                if body.len() >= 4
                    && u32::from_le_bytes([body[0], body[1], body[2], body[3]]) == 6017
                {
                    heard = true;
                }
            }
        }
        if heard {
            break;
        }
    }
    println!("FENRUS howl_heard={heard}");
    assert!(heard, "Fenrus howl (6017) on enter combat");
    let fenrus = find_creature(&map, 4274).expect("Fenrus in view");
    kill(&mut map, fenrus, &mut now_ms, &mut chat_log);
    heal(&mut map);

    // ---- doors via SetData -----------------------------
    let arugal_door_pos = oracle.go_pos[&18971];
    relocate(
        &mut map,
        arugal_door_pos.0 + 2.0,
        arugal_door_pos.1,
        arugal_door_pos.2,
    );
    tick!(6);
    let arugal_door = find_go(&map, 18971).expect("Arugal door in view");
    assert_eq!(
        go_state(&map, arugal_door),
        oracle.door_states[&18971],
        "Arugal door starts at DB state"
    );
    assert!(map.instance_set_data(4, 3)); // TYPE_NANDOS DONE
    assert_eq!(go_state(&map, arugal_door), 0, "Arugal door opens");
    let sorcerer_door_pos = oracle.go_pos[&18972];
    relocate(
        &mut map,
        sorcerer_door_pos.0 + 2.0,
        sorcerer_door_pos.1,
        sorcerer_door_pos.2,
    );
    tick!(6);
    let sorcerer_door = find_go(&map, 18972).expect("Sorcerer door in view");
    for _ in 0..3 {
        assert!(map.instance_set_data(6, 3)); // TYPE_VOIDWALKER DONE
    }
    assert_eq!(
        go_state(&map, sorcerer_door),
        oracle.door_states[&18972],
        "Sorcerer door still closed at 3 voidwalkers"
    );
    assert!(map.instance_set_data(6, 3));
    assert_eq!(go_state(&map, sorcerer_door), 0, "Sorcerer door opens at 4");
    // encounter[2] (TYPE_FENRUS) was set by the DB
    // death script when Fenrus died above.
    assert_eq!(map.instance_save().as_deref(), Some("0 0 3 3 3 4"));

    // ---- Load roundtrip --------------------------------
    map.instance_load("3 1 0 0 3 7");
    assert_eq!(map.instance_get_data(1), Some(3));
    assert_eq!(
        map.instance_get_data(2),
        Some(0),
        "IN_PROGRESS resets on Load"
    );
    assert_eq!(map.instance_get_data(5), Some(3));

    // ---- Haunting Spirits AuraScript (7057) ------------
    let baseline: std::collections::HashSet<ObjectGuid> = map
        .visible_to(player_guid)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    // 7057 is a hostile debuff: a creature casts
    // it on the player (fork SFK ghosts). Stand
    // next to the nearest live creature first.
    let player_pos = map
        .player(player_guid)
        .map(|p| p.unit.world.position())
        .unwrap_or_default();
    let haunt_caster = map
        .visible_to(player_guid)
        .cloned()
        .unwrap_or_default()
        .iter()
        .copied()
        .filter_map(|g| {
            map.creature(g)
                .filter(|c| c.unit.is_alive())
                .map(|c| (g, c.unit.world.position()))
        })
        .min_by(|a, b| dist2d(player_pos, a.1).total_cmp(&dist2d(player_pos, b.1)))
        .map(|(g, _)| g)
        .expect("a live creature to cast Haunting Spirits");
    let caster_pos = map
        .creature(haunt_caster)
        .expect("caster")
        .unit
        .world
        .position();
    relocate(&mut map, caster_pos.x + 2.0, caster_pos.y, caster_pos.z);
    tick!(2);
    heal(&mut map);
    let outcome = map.cast_spell(
        haunt_caster,
        7057,
        mangos_map::spells::CastTarget::Unit(player_guid),
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
    assert_eq!(
        outcome,
        mangos_map::spells::CastOutcome::Started,
        "Haunting Spirits cast starts"
    );
    let mut holder = None;
    for _ in 0..100 {
        tick!(1);
        heal(&mut map);
        if let Some(h) = map
            .aura_holders(player_guid)
            .into_iter()
            .find(|h| h.spell_id == 7057)
        {
            holder = Some(h);
            break;
        }
    }
    let holder = holder.expect("7057 holder applied");
    let timer0 = holder.effects[0].expect("7057 effect 0").periodic_timer;
    println!(
        "HAUNT period={} first_timer={timer0}",
        oracle.haunt_period_ms
    );
    assert!(
        timer0 > i32::try_from(oracle.haunt_period_ms).unwrap_or(0) && timer0 <= 5000,
        "SetPeriodicTimer(5s) override on apply"
    );
    let mut summoned = false;
    for _ in 0..3000 {
        tick!(1);
        let visible = map.visible_to(player_guid).cloned().unwrap_or_default();
        if visible.iter().any(|g| {
            !baseline.contains(g)
                && map
                    .creature(*g)
                    .is_some_and(|c| c.info.entry == oracle.haunt_summon_entry)
        }) {
            summoned = true;
            break;
        }
        if !map
            .aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == 7057)
        {
            break;
        }
    }
    println!(
        "HAUNT summoned={summoned} entry={}",
        oracle.haunt_summon_entry
    );
    assert!(summoned, "Haunting Spirit summoned by periodic dummy tick");

    println!("LIVE-SHADOWFANG-PASS");
}
