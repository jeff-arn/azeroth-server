//! Tier 4 Slice 2 step 2 (player death loop) live
//! evidence on the dev bench: `KillPlayer` corpse
//! state, release-spirit repop (ghost form, corpse
//! creation + persistence event, graveyard
//! teleport), corpse reclaim resurrection, and
//! spirit-healer resurrection with sickness
//! through the production `death.rs`. Expected
//! values from fork `Player.cpp`/`Corpse.cpp`
//! (marker `LIVE-DEATH-PASS`). Read-only against
//! the characters DB: events are asserted, not
//! drained to persistence.

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

#[allow(clippy::too_many_lines)]
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

            let areas = mangos_world_data::areas::load_areas(conn, 10)?;
            let safe_locs = load_safe_locs();
            let mut gy_report = LoadReport::default();
            let graveyard_links = mangos_world_data::battlegrounds::load_graveyard_links(
                conn,
                10,
                &safe_locs,
                &|z| areas.entry(z).is_some(),
                &mut gy_report,
            )?;

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
                factions: mangos_world_data::reputations::load_factions(conn)?,
                reputation_kills: mangos_world_data::reputations::ReputationKillStores::default(),
                gossip: mangos_world_data::gossip_menus::GossipStores::default(),
                areas,
                exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(),
                pets: mangos_world_data::pet_data::PetStores::default(),
                next_pet_number: std::sync::atomic::AtomicU32::new(1),
                next_item_guid: std::sync::atomic::AtomicU32::new(1),
                vendor_items: mangos_world_data::vendor_items::VendorItemStores::default(),
                durability: mangos_world_data::durability::DurabilityTables::default(),
                bank_bag_slot_prices: Vec::new(),
                stable_slot_prices: Vec::new(),
                trainer_spells: mangos_world_data::trainer_spells::TrainerStores::default(),
                taxi: mangos_world_data::taxi::TaxiStore::default(),
                script_waypoints: mangos_world_data::script_waypoints::ScriptWaypointStore::default(
                ),
                game_events: mangos_world_data::game_events::GameEvents::default(),
                loot: mangos_world_data::loot::LootStores::default(),
                locks: mangos_world_data::locks::LockStore::default(),
                fishing_base_skills: mangos_world_data::fishing::FishingBaseSkills::default(),
                weather_chances: mangos_world_data::weather::WeatherChanceStore::default(),
                waypoints,
                spells: mangos_spells::SpellStore {
                    spells: spell_entries,
                    skill_line_abilities: Vec::new(),
                    spell_chains: std::collections::HashMap::new(),
                    talents: std::collections::HashMap::new(),
                    talent_tabs: std::collections::HashMap::new(),
                    talent_spell_pos: std::collections::HashMap::new(),
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
                safe_locs,
                battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates::default(),
                battlemasters: mangos_world_data::battlegrounds::BattleMasters::default(),
                battle_event_indexes: mangos_world_data::battlegrounds::BattleEventIndexes::default(
                ),
                graveyard_links,
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

fn make_map(stores: &Arc<WorldStores>) -> (Map, ObjectGuid, RespawnTimes) {
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
    let saved_map = load.row.map;
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
        rest_rates: mangos_entities::player::RestRates::default(),
        skills: None,
        item_prototypes: None,
        quest_templates: None,
        taxi: None,
    };
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();
    let _ = &dbc;
    let mut map = Map::new(
        saved_map,
        0,
        MapConfig::default(),
        Arc::clone(stores),
        WorldRng::from_seed(7),
    );
    // Terrain area flags drive the zone/area the
    // graveyard selector consumes.
    if let Ok(dir) = std::env::var("MANGOS_MAPS_DIR") {
        map.set_maps_dir(Some(PathBuf::from(dir)));
    }
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

/// One tick at an explicit unix time (reclaim-delay
/// gate consumes `ghost_time + delay <= now`).
fn tick_at(map: &mut Map, now_ms: &mut u32, now_unix: u64) {
    *now_ms += 50;
    map.tick(50, *now_ms, now_unix);
    let _ = map.drain_events();
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

use mangos_entities::unit::DeathState;
use mangos_map::map::MapEvent;

const SPELL_GHOST: u32 = 8326;
const SPELL_RESURRECTION_SICKNESS: u32 = 15007;
const PLAYER_FLAGS_GHOST: u32 = 0x0000_0010;

fn is_ghost(map: &Map, guid: ObjectGuid) -> bool {
    map.player(guid).is_some_and(|p| {
        let values = p.unit.world.object.values();
        values.has_flag(offsets::PLAYER_FLAGS, PLAYER_FLAGS_GHOST)
            // `HandleAuraGhost` also carries
            // `UNIT_VIS_FLAGS_GHOST` (0x01) in the
            // `UNIT_FIELD_BYTES_1` vis byte (offset 3).
            && values.get_byte(offsets::UNIT_FIELD_BYTES_1, 3) & 0x01 != 0
    })
}

fn has_aura(map: &Map, guid: ObjectGuid, spell_id: u32) -> bool {
    map.player(guid)
        .is_some_and(|p| p.unit.auras.iter().any(|h| h.spell_id == spell_id))
}

#[test]
#[allow(clippy::too_many_lines)]
#[ignore = "live bench DB"]
fn live_death_loop() {
    let stores = load_stores();
    let (mut map, player, _respawns) = make_map(&stores);
    let mut now_ms = 1000u32;
    tick(&mut map, &mut now_ms, 2);
    let level = map.player(player).expect("player").unit.level();
    eprintln!("death probe: fixture level {level}");
    let pos = map.player(player).expect("player").unit.world.position();
    eprintln!(
        "death probe: pos=({:.1},{:.1},{:.1}) graveyard={:?}",
        pos.x,
        pos.y,
        pos.z,
        map.player_graveyard(player).map(|gy| (gy.id, gy.map_id))
    );
    assert!(map.player(player).expect("player").unit.is_alive());

    // --- KillPlayer: CORPSE state, no ghost yet.
    map.kill_unit(player, None);
    let p = map.player(player).expect("player");
    assert_eq!(p.unit.death_state, DeathState::Corpse, "corpse state");
    assert!(!p.unit.is_alive(), "not alive after kill");
    assert_eq!(p.unit.health(), 0, "health zero at death");
    assert!(!is_ghost(&map, player), "no ghost flag before repop");
    assert!(
        map.corpse_for_player(player).is_none(),
        "corpse object only exists after repop"
    );
    let _ = map.drain_events();

    // --- Release spirit: ghost form, corpse +
    // persistence event, graveyard teleport event.
    map.repop_request(player);
    let events = map.drain_events();
    let p = map.player(player).expect("player");
    assert_eq!(p.unit.death_state, DeathState::Dead, "dead state as ghost");
    assert!(is_ghost(&map, player), "PLAYER_FLAGS_GHOST set");
    assert!(has_aura(&map, player, SPELL_GHOST), "ghost aura 8326");
    let corpse_guid = map.corpse_for_player(player).expect("resurrectable corpse");
    let save = events.iter().find_map(|e| match e {
        MapEvent::CorpseSave(row) => Some(row),
        _ => None,
    });
    let save = save.expect("CorpseSave persistence event");
    assert_eq!(save.player_guid, player.counter(), "corpse owner");
    assert_eq!(save.guid, corpse_guid.counter(), "corpse guid matches");
    let teleport = events
        .iter()
        .any(|e| matches!(e, MapEvent::TeleportPlayer { player: tp, .. } if *tp == player));
    assert!(teleport, "graveyard teleport event emitted");

    // --- Reclaim too early is gated (30s base
    // delay); after 31s the reclaim resurrects.
    map.reclaim_corpse(player, corpse_guid);
    assert!(
        !map.player(player).expect("player").unit.is_alive(),
        "reclaim blocked inside reclaim delay"
    );
    tick_at(&mut map, &mut now_ms, 1_800_000_032);
    map.reclaim_corpse(player, corpse_guid);
    let events = map.drain_events();
    let p = map.player(player).expect("player");
    assert!(p.unit.is_alive(), "reclaim resurrects");
    assert_eq!(p.unit.death_state, DeathState::Alive, "alive state");
    assert!(!is_ghost(&map, player), "ghost flag cleared");
    assert!(!has_aura(&map, player, SPELL_GHOST), "ghost aura removed");
    let health = p.unit.health();
    assert!(
        (1..=50_500).contains(&health),
        "50% resurrect health of 100k probe max, got {health}"
    );
    assert!(
        map.corpse_for_player(player).is_none(),
        "resurrectable corpse consumed (bones remain)"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            MapEvent::CorpseDelete { player_low } if *player_low == player.counter()
        )),
        "persisted corpse row deleted on resurrect"
    );

    // --- Second death: spirit-healer resurrection
    // (map half) + actor-half bones conversion.
    heal(&mut map, player);
    map.kill_unit(player, None);
    map.repop_request(player);
    let _ = map.drain_events();
    assert!(is_ghost(&map, player), "ghost again before spirit heal");
    map.spirit_healer_resurrect(player);
    map.spawn_corpse_bones_for(player);
    let events = map.drain_events();
    let p = map.player(player).expect("player");
    assert!(p.unit.is_alive(), "spirit resurrect revives");
    assert!(!is_ghost(&map, player), "ghost form removed");
    if level >= 11 {
        assert!(
            has_aura(&map, player, SPELL_RESURRECTION_SICKNESS),
            "resurrection sickness 15007 at level {level}"
        );
    }
    assert!(
        map.corpse_for_player(player).is_none(),
        "corpse converted to bones"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            MapEvent::CorpseDelete { player_low } if *player_low == player.counter()
        )),
        "persisted corpse row deleted on spirit heal"
    );

    eprintln!("LIVE-DEATH-PASS");
}
