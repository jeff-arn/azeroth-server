//! Tier 3 Slice 3 step 5 (Alterac Valley) live
//! evidence on the dev bench: match setup, node
//! timers, resource ticks, node graveyards, score
//! counters and win flow through the production
//! `bg_av.rs`. Expected values from fork
//! `BattleGroundAV.cpp` constants (marker
//! `LIVE-AV-PASS`).

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
                quests: mangos_world_data::quest_template::load_quest_templates_lite(conn)?,
                quest_templates: mangos_world_data::quest_template::load_quest_templates(conn, 10)?,
                quest_relations: mangos_world_data::quest_template::load_quest_relations(conn)?,
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
use mangos_movement::movement_info::Position;

#[test]
#[allow(clippy::too_many_lines)]
#[ignore = "live bench DB"]
fn live_av_match() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 30);
    let team_of = map.player(player).map_or(Team::None, |p| p.team);
    eprintln!("av probe: fixture team = {team_of:?}");
    let ally = team_of == Team::Alliance;
    let t = mangos_map::battlegrounds::team_index(team_of);
    let main_gy = if ally { 611 } else { 610 };

    let init = BgInit {
        bg_type_id: 1,
        bracket_id: 0,
        client_instance_id: 1,
        name: "Alterac Valley".to_string(),
        min_players_per_team: 32,
        max_players_per_team: 40,
        alliance_win_spell: 0,
        alliance_lose_spell: 0,
        horde_win_spell: 0,
        horde_lose_spell: 0,
        alliance_start: stores.safe_locs.get(611).expect("611"),
        horde_start: stores.safe_locs.get(610).expect("610"),
        player_skin_ref_loot_id: 0,
        level_min: 51,
        level_max: 60,
    };
    map.init_battleground(init);
    assert_eq!(
        map.av_closest_graveyard(player).map(|g| g.id),
        Some(main_gy),
        "pre-start main graveyard"
    );
    map.bg_add_player(player);
    let _ = map.drain_events();

    let mut now_ms = 1000u32;
    for _ in 0..2600 {
        tick(&mut map, &mut now_ms, 1);
        if map.bg.as_ref().map(|b| b.status) == Some(bg_status::IN_PROGRESS) {
            break;
        }
    }
    let bg = map.bg.as_ref().expect("bg");
    assert_eq!(bg.status, bg_status::IN_PROGRESS, "match started");
    assert_eq!(bg.team_scores, [600, 600], "initial reinforcements");

    let kind = if let mangos_map::battlegrounds::BgKind::Av(av) = &map.bg.as_ref().expect("bg").kind
    {
        av.clone()
    } else {
        panic!("not AV");
    };
    assert_eq!(kind.nodes[0].owner, 0, "firstaid alliance");
    assert_eq!(kind.nodes[3].owner, 2, "snowfall neutral");
    assert_eq!(kind.nodes[7].owner, 0, "dunbaldar south alliance");
    assert_eq!(kind.nodes[11].owner, 1, "iceblood tower horde");
    assert!(kind.nodes[7].tower, "tower flag");

    // mine tick: +1 reinforcement per 45 s while owned
    if let mangos_map::battlegrounds::BgKind::Av(av) = &mut map.bg.as_mut().expect("bg").kind {
        av.mine_owner[0] = t as u8;
        av.mine_timer[0] = 40;
    }
    tick(&mut map, &mut now_ms, 2);
    assert_eq!(
        map.bg.as_ref().expect("bg").team_scores[t],
        601,
        "mine reinforcement tick"
    );

    // player death: -1 reinforcement
    map.av_on_kill_player(player, None);
    assert_eq!(
        map.bg.as_ref().expect("bg").team_scores[t],
        600,
        "death reinforcement tick"
    );

    // assaulted -> destroyed point (snowfall)
    if let mangos_map::battlegrounds::BgKind::Av(av) = &mut map.bg.as_mut().expect("bg").kind {
        av.nodes[3].owner = t as u8;
        av.nodes[3].state = 0; // POINT_ASSAULTED
        av.nodes[3].timer = 100;
    }
    tick(&mut map, &mut now_ms, 3);
    let kind = if let mangos_map::battlegrounds::BgKind::Av(av) = &map.bg.as_ref().expect("bg").kind
    {
        av.clone()
    } else {
        panic!("not AV");
    };
    assert_eq!(kind.nodes[3].state, 1, "snowfall controlled");
    assert_eq!(kind.nodes[3].total_owner, t as u8, "snowfall total owner");

    // closest owned graveyard at snowfall position
    let pos = stores.safe_locs.get(169).expect("169");
    relocate(&mut map, player, &respawns, pos.x, pos.y, pos.z);
    assert_eq!(
        map.av_closest_graveyard(player).map(|g| g.id),
        Some(169),
        "snowfall graveyard"
    );

    // battleground_alterac blood collector (step 5b): the
    // quest turn-in feeds the world-boss challenge counter and
    // the smith gossip reports supply progress.
    let smith_entry = if ally { 13176 } else { 13257 };
    let spawn_pos = stores
        .spawns
        .creatures()
        .filter(|(_, sp)| sp.position.map_id == 30 && sp.creature_id.contains(&smith_entry))
        .map(|(_, sp)| (sp.position.x, sp.position.y, sp.position.z))
        .next()
        .expect("smith spawn");
    relocate(
        &mut map,
        player,
        &respawns,
        spawn_pos.0,
        spawn_pos.1,
        spawn_pos.2,
    );
    tick(&mut map, &mut now_ms, 4);
    let smith = map
        .visible_to(player)
        .expect("visible")
        .iter()
        .copied()
        .find(|g| {
            map.creature(*g)
                .is_some_and(|c| c.info.entry == smith_entry)
        })
        .expect("smith materialized");
    let blood_counter = |map: &Map| -> u32 {
        if let mangos_map::battlegrounds::BgKind::Av(av) = &map.bg.as_ref().expect("bg").kind {
            av.challenge_status[t][7] // BLOOD_WORLDBOSS_ASSAULT
        } else {
            panic!("not AV")
        }
    };
    let before_counter = blood_counter(&map);
    map.quest_script_on_rewarded(player, smith, if ally { 7385 } else { 7386 });
    assert_eq!(
        blood_counter(&map),
        before_counter + 5,
        "blood counter += ReqItemCount1 (5)"
    );
    let menu = map
        .gossip_script_on_hello(player, smith)
        .expect("smith menu");
    assert_eq!(menu.text_id, 6073, "basic reinforcement menu");
    assert_eq!(menu.items.len(), 1, "next-upgrade item only");
    let outcome = map
        .gossip_script_on_select(player, smith, 1, menu.items[0].action)
        .expect("handled select");
    match outcome {
        mangos_map::gossip::GossipSelectOutcome::ShowMenu(m) => {
            assert_eq!(m.text_id, 6784, "barely any supplies menu");
        }
        mangos_map::gossip::GossipSelectOutcome::Close => panic!("expected supplies menu"),
    }

    // step 5b content materialization: alliance landmine
    // GOs (179325, `go_av_landmine`) and Balinda (11949,
    // `npc_Balinda`) spawn on this map. (Combat pulls need
    // an enemy-team fixture — recorded remainder.)
    let mine_pos = stores
        .spawns
        .gameobjects()
        .filter(|(_, sp)| sp.position.map_id == 30 && sp.id == 179_325)
        .map(|(_, sp)| (sp.position.x, sp.position.y, sp.position.z))
        .next()
        .expect("landmine spawn");
    relocate(
        &mut map, player, &respawns, mine_pos.0, mine_pos.1, mine_pos.2,
    );
    tick(&mut map, &mut now_ms, 4);
    let visible = map.visible_to(player).expect("visible");
    assert!(
        visible
            .iter()
            .copied()
            .any(|g| map.gameobject(g).is_some_and(|go| go.info.id == 179_325)),
        "landmine materialized"
    );
    let bal_pos = stores
        .spawns
        .creatures()
        .filter(|(_, sp)| sp.position.map_id == 30 && sp.creature_id.contains(&11949))
        .map(|(_, sp)| (sp.position.x, sp.position.y, sp.position.z))
        .next()
        .expect("balinda spawn");
    relocate(&mut map, player, &respawns, bal_pos.0, bal_pos.1, bal_pos.2);
    tick(&mut map, &mut now_ms, 4);
    let visible = map.visible_to(player).expect("visible");
    assert!(
        visible
            .iter()
            .copied()
            .any(|g| map.creature(g).is_some_and(|c| c.info.entry == 11949)),
        "balinda materialized"
    );

    // score counters through production dispatch
    map.bg_update_player_score(player, 11, 1);
    map.bg_update_player_score(player, 13, 1);
    map.bg_update_player_score(player, 15, 2);
    let score = *map
        .bg
        .as_ref()
        .expect("bg")
        .scores
        .get(&player)
        .expect("score");
    assert_eq!(score.graveyards_assaulted, 1);
    assert_eq!(score.towers_assaulted, 1);
    assert_eq!(score.secondary_objectives, 2);

    // end: winner rewards include surviving towers /
    // owned graves / surviving captain honor
    let before = map
        .bg
        .as_ref()
        .and_then(|b| b.scores.get(&player))
        .map_or(0, |s| s.bonus_honor);
    map.bg_end_battleground(team_of);
    assert_eq!(
        map.bg.as_ref().expect("bg").status,
        bg_status::WAIT_LEAVE,
        "match ended"
    );
    let after = map
        .bg
        .as_ref()
        .and_then(|b| b.scores.get(&player))
        .map_or(0, |s| s.bonus_honor);
    assert!(after > before, "end rewards honor ({before} -> {after})");
    println!("LIVE-AV-PASS");
}
