//! Live `EventAI` evidence on the dev bench (Slice 6b).
//! Defias Pillager (entry 589, `ai_name = EventAI`):
//! a `TIMER_OOC` self-cast (Frost Armor at 1 s) and an
//! HP event at <=15% that runs FLEE. Gibblewilt (entry
//! 8503): a 100%-chance AGGRO TALK. All expected values
//! come from independent SQL in this test, never from
//! the Rust stores. Read-only against the DB.

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

fn heal_player_full(map: &mut Map, player_guid: ObjectGuid) {
    let player = map.player_mut(player_guid).expect("player");
    player
        .unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
    player.unit.set_health(100_000);
}

/// Extract (msg type, sender guid raw, sender name,
/// text) from a `SMSG_MESSAGECHAT` body using the C++
/// monster layouts. Emote/whisper layouts carry no
/// sender guid (0 is returned).
fn parse_monster_chat(body: &[u8]) -> Option<(u8, u64, String, String)> {
    let msg_type = *body.first()?;
    let mut off = 5usize; // type + language
    let read_cstring = |body: &[u8], off: usize| -> Option<(String, usize)> {
        let end = body[off..].iter().position(|b| *b == 0)? + off;
        Some((
            String::from_utf8_lossy(&body[off..end]).into_owned(),
            end + 1,
        ))
    };
    let (sender_raw, sender_name);
    match msg_type {
        0x0D | 0x1A => {
            sender_raw = 0;
            let name_len = u32::from_le_bytes(body.get(off..off + 4)?.try_into().ok()?) as usize;
            off += 4;
            let (name, next) = read_cstring(body, off)?;
            debug_assert_eq!(next - off, name_len);
            sender_name = name;
            off = next + 8;
        }
        0x0B | 0x0C => {
            sender_raw = u64::from_le_bytes(body.get(off..off + 8)?.try_into().ok()?);
            off += 8;
            let _name_len = u32::from_le_bytes(body.get(off..off + 4)?.try_into().ok()?);
            off += 4;
            let (name, next) = read_cstring(body, off)?;
            sender_name = name;
            off = next + 8;
        }
        _ => return None,
    }
    let _len = u32::from_le_bytes(body.get(off..off + 4)?.try_into().ok()?);
    off += 4;
    let (text, _) = read_cstring(body, off)?;
    Some((msg_type, sender_raw, sender_name, text))
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)]
fn bench_eventai_pillager() {
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
    let (stores, pillager_spawn, oracle) = world
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

            // --- Independent SQL oracle ----------------
            let pillager_spawn: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE id=589 AND map=0 ORDER BY guid LIMIT 1",
                )
                .map_err(mangos_db::DbError::from)?;
            let events: Vec<(u32, u32, u32, u32, u32)> = conn
                .query(
                    "SELECT event_type, event_chance, action1_script, event_param1, \
                     event_param2 FROM creature_ai_events WHERE creature_id=589",
                )
                .map_err(mangos_db::DbError::from)?;
            let ooc_timer = events
                .iter()
                .find(|e| e.0 == 1)
                .map(|e| (e.3, e.2))
                .expect("TIMER_OOC event");
            let ooc_spell: Option<u32> = conn
                .query_first(format!(
                    "SELECT datalong FROM creature_ai_scripts \
                     WHERE id={} AND command=15",
                    ooc_timer.1
                ))
                .map_err(mangos_db::DbError::from)?;
            // Deterministic talk subject: entry 8503
            // (single spawn, aggressive, 100% AGGRO talk,
            // level 11 so the bench warrior can hit it).
            let talk_spawn: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE id=8503 AND map=0 ORDER BY guid LIMIT 1",
                )
                .map_err(mangos_db::DbError::from)?;
            let talk_text: Option<String> = conn
                .query_first(
                    "SELECT b.male_text FROM creature_ai_events e \
                     JOIN creature_ai_scripts s ON s.id=e.action1_script AND s.command=0 \
                     JOIN broadcast_text b ON b.entry=s.dataint \
                     WHERE e.creature_id=8503 AND e.event_type=4 AND e.event_chance=100",
                )
                .map_err(mangos_db::DbError::from)?;
            let hp_event = events.iter().find(|e| e.0 == 2).expect("HP event");
            let hp_max_pct = hp_event.3;
            let hp_cmd: Option<u32> = conn
                .query_first(format!(
                    "SELECT command FROM creature_ai_scripts WHERE id={}",
                    hp_event.2
                ))
                .map_err(mangos_db::DbError::from)?;
            let flee_text: Option<String> = conn
                .query_first("SELECT male_text FROM broadcast_text WHERE entry=1150")
                .map_err(mangos_db::DbError::from)?;
            let oracle = (
                ooc_timer.0,
                ooc_spell.expect("ooc cast spell"),
                talk_spawn.expect("talk spawn"),
                talk_text.expect("talk text"),
                hp_max_pct,
                hp_cmd.expect("hp script command"),
                flee_text.expect("flee text"),
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
                    maps: mangos_world_data::map_template::MapStore::default(),
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
                pillager_spawn.expect("pillager spawn"),
                oracle,
            ))
        })
        .expect("stores");
    let (ooc_initial_ms, ooc_spell, talk_spawn, talk_text, hp_max_pct, hp_cmd, flee_text) = oracle;
    assert_eq!(hp_cmd, 47, "oracle: HP event must run FLEE (47)");
    println!(
        "ORACLE ooc_initial={ooc_initial_ms} ooc_spell={ooc_spell} \
         talk_text={talk_text:?} hp_max_pct={hp_max_pct}"
    );

    let stores = Arc::new(stores);
    // Store-side sanity: the pillager really loaded
    // EventAI events (count only; behavior below is the
    // actual evidence).
    let event_count = stores.scripts.eventai_events.get(&589).map_or(0, Vec::len);
    assert_eq!(event_count, 3, "pillager events loaded");

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
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(41),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    let _ = map.drain_events();
    heal_player_full(&mut map, player_guid);
    let mut now_ms = 1000u32;
    let mut tick = |map: &mut Map, n: u32| {
        for _ in 0..n {
            now_ms += 50;
            map.tick(50, now_ms, 1_800_000_000);
        }
    };
    let spawn_pos = Position::new(pillager_spawn.0, pillager_spawn.1, pillager_spawn.2, 0.0);
    let relocate_to = |map: &mut Map, anchor: Position, offset: f32| {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(anchor.x + offset, anchor.y, anchor.z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    };

    // Activate the area from outside detection range
    // (18 yd + combat reach), then pick any pillager
    // that stayed out of combat at a safe distance
    // (several spawns share Moonbrook).
    relocate_to(&mut map, spawn_pos, 30.0);
    tick(&mut map, 4);
    let _ = map.drain_events();
    let player_pos = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let target = {
        let visible = map.visible_to(player_guid).cloned().expect("visible");
        visible
            .iter()
            .copied()
            .filter_map(|g| {
                map.creature(g).and_then(|c| {
                    (c.info.entry == 589 && !map.ai_in_combat(g))
                        .then_some((g, c.unit.world.position()))
                })
            })
            .filter(|(_, pos)| dist2d(player_pos, *pos) > 23.0)
            .min_by(|a, b| dist2d(player_pos, a.1).total_cmp(&dist2d(player_pos, b.1)))
            .map(|(g, _)| g)
            .expect("a calm Defias Pillager in view")
    };
    let target_home = map.creature(target).expect("target").unit.world.position();
    let target_max_health = map
        .creature(target)
        .expect("target")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_MAXHEALTH);

    // --- TIMER_OOC: Frost Armor self-cast --------------
    let mut frost = false;
    for _ in 0..160 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        if map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == ooc_spell)
        {
            frost = true;
            break;
        }
    }
    println!(
        "OOC-TIMER frost_armor({ooc_spell})={frost} in_combat={}",
        map.ai_in_combat(target)
    );
    assert!(frost, "TIMER_OOC never produced the self-cast aura");

    // --- Aggro the pillager (flee leg needs combat) --
    relocate_to(&mut map, target_home, 12.0);
    heal_player_full(&mut map, player_guid);
    for _ in 0..120 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        if map.ai_in_combat(target) {
            break;
        }
    }
    assert!(map.ai_in_combat(target), "not in combat for flee phase");
    heal_player_full(&mut map, player_guid);
    if let Some(c) = map.creature_mut(target) {
        c.unit.set_health(target_max_health * hp_max_pct / 100);
    }
    let flee_from = map.creature(target).expect("target").unit.world.position();
    let mut fled_dist = 0.0f32;
    let mut flee_emote = false;
    for _ in 0..120 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_MESSAGECHAT
                && let Some((0x0D, _, name, text)) = parse_monster_chat(packet.body())
                && name == "Defias Pillager"
                && text == flee_text
            {
                flee_emote = true;
            }
        }
        let pos = map.creature(target).expect("target").unit.world.position();
        fled_dist = fled_dist.max(dist2d(flee_from, pos));
        if fled_dist > 12.0 {
            break;
        }
    }
    println!(
        "FLEE fled_dist={fled_dist:.1} emote={flee_emote} home_dist={:.1}",
        dist2d(
            target_home,
            map.creature(target).expect("target").unit.world.position()
        )
    );
    assert!(fled_dist > 12.0, "creature did not flee (dist {fled_dist})");
    assert!(flee_emote, "flee text emote (broadcast 1150) not observed");

    // --- AGGRO TALK (deterministic): entry 8503 -------
    // Break pillager combat first, then walk into the
    // subject's detection range: proximity aggro fires
    // its 100% AGGRO event.
    relocate_to(&mut map, target_home, 160.0);
    for _ in 0..240 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        if !map.ai_in_combat(target) {
            break;
        }
    }
    heal_player_full(&mut map, player_guid);
    let talk_pos = Position::new(talk_spawn.0, talk_spawn.1, talk_spawn.2, 0.0);
    relocate_to(&mut map, talk_pos, 12.0);
    let mut talk_seen: Option<String> = None;
    let mut talker = ObjectGuid::EMPTY;
    for _ in 0..400 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_MESSAGECHAT
                && let Some((0x0B, sender, _, text)) = parse_monster_chat(packet.body())
                && map
                    .creature(ObjectGuid::from_raw(sender))
                    .is_some_and(|c| c.info.entry == 8503)
            {
                talker = ObjectGuid::from_raw(sender);
                talk_seen = Some(text);
            }
        }
        if talk_seen.is_some() {
            break;
        }
    }
    println!(
        "AGGRO-TALK subject_combat={} text={talk_seen:?}",
        map.ai_in_combat(talker)
    );
    assert_eq!(
        talk_seen.as_deref(),
        Some(talk_text.as_str()),
        "100% AGGRO talk did not arrive with the SQL broadcast text"
    );

    println!("LIVE-EVENTAI-PASS");
}
