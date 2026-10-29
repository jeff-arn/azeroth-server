//! Tier 3 Slice 2 step 3 (`CreatureAI` script
//! framework) live evidence on the dev bench:
//! `boss_herod` + `mob_scarlet_trainee` through
//! the generalized dispatchers. Expected values
//! from independent SQL + fork `boss_herod.cpp`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::spells::{CastOptions, CastOutcome, CastTarget};
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

fn find_go(map: &Map, player: ObjectGuid, entry: u32) -> Option<ObjectGuid> {
    map.visible_to(player)?
        .iter()
        .copied()
        .find(|g| map.gameobject(*g).is_some_and(|go| go.info.id == entry))
}

fn go_field(map: &Map, go: ObjectGuid, offset: u16) -> u32 {
    map.gameobject(go)
        .map_or(u32::MAX, |g| g.world.object.values().get_u32(offset))
}

fn aura_present(map: &Map, guid: ObjectGuid, spell: u32) -> bool {
    let Some(c) = map.creature(guid) else {
        return false;
    };
    let values = c.unit.world.object.values();
    (0..56u16).any(|slot| values.get_u32(offsets::UNIT_FIELD_AURA + slot) == spell)
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_boss_herod_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    let mut now = 1000u32;
    // Herod spawn (SQL: map 189, 1965.09/-431.607/6.26076).
    relocate(&mut map, player, &respawns, 1957.0, -431.6, 6.3);
    tick(&mut map, &mut now, 120);
    let herod = find_creature(&map, player, 3975).expect("Herod in view");
    assert!(map.ai_in_combat(herod), "Herod engaged by proximity");
    // Aggro summoned 4 myrmidons at 1926.03/-370.61/18.
    relocate(&mut map, player, &respawns, 1926.0, -375.0, 18.0);
    tick(&mut map, &mut now, 120);
    assert!(
        find_creature(&map, player, 4295).is_some(),
        "Scarlet Myrmidon summoned on aggro"
    );
    // Back to the boss; drop to 40% -> Frenzy (8269) enrage.
    relocate(&mut map, player, &respawns, 1957.0, -431.6, 6.3);
    tick(&mut map, &mut now, 40);
    let max = map
        .creature(herod)
        .expect("herod")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_MAXHEALTH);
    if let Some(c) = map.creature_mut(herod) {
        c.unit.set_health(max * 40 / 100);
    }
    let mut enraged = false;
    for _ in 0..200 {
        tick(&mut map, &mut now, 1);
        if aura_present(&map, herod, 8269) {
            enraged = true;
            break;
        }
    }
    assert!(enraged, "Frenzy aura after 50% threshold");
    // Kill from 1 HP by player swings; JustDied summons the
    // trainee rush and opens the door.
    // Kill from 1 HP with a direct-damage spell (batch-test
    // pattern; melee swings whiff against +levels). Track the
    // boss so range/reach never gates the cast.
    for _ in 0..400 {
        if let Some(pos) = map.creature(herod).map(|c| c.unit.world.movement_info.pos) {
            relocate(&mut map, player, &respawns, pos.x + 1.0, pos.y, pos.z);
        }
        if let Some(c) = map.creature_mut(herod) {
            c.unit.set_health(1);
        }
        map.cast_spell(
            player,
            8_921, // Moonfire: instant direct damage
            CastTarget::Unit(herod),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..Default::default()
            },
        );
        tick(&mut map, &mut now, 1);
        if !map.creature(herod).is_some_and(|c| c.unit.is_alive()) {
            break;
        }
    }
    assert!(
        !map.creature(herod).is_some_and(|c| c.unit.is_alive()),
        "Herod dead"
    );
    relocate(&mut map, player, &respawns, 1939.0, -431.6, 17.1);
    tick(&mut map, &mut now, 120);
    assert!(
        find_creature(&map, player, 6575).is_some(),
        "Scarlet Trainee rush summoned"
    );
    let door = find_go(&map, player, 101_854).expect("Herod door in view");
    assert_eq!(
        go_field(&map, door, offsets::GAMEOBJECT_STATE),
        0,
        "JustDied opened the door (ACTIVE)"
    );
    println!("LIVE-BOSS-HEROD-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_anubrekhan_los_pull() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 533);
    let mut now = 1000u32;
    heal(&mut map, player);
    // 40yd west of the spawn (SQL 3308.59/-3476.29/287.161):
    // beyond aggro range, inside the script's 55yd
    // `MoveInLineOfSight` pull.
    relocate(&mut map, player, &respawns, 3268.59, -3476.29, 287.161);
    tick(&mut map, &mut now, 120);
    let anub = find_creature(&map, player, 15_956).expect("Anub'Rekhan in view");
    assert!(
        map.ai_in_combat(anub),
        "MoveInLineOfSight pull engaged the boss"
    );
    assert_eq!(
        map.instance_get_data(0),
        Some(1),
        "Aggro set TYPE_ANUB_REKHAN IN_PROGRESS"
    );
    println!("LIVE-ANUBREKHAN-LOS-PULL-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_core_rager_damage_taken() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 409);
    let mut now = 1000u32;
    heal(&mut map, player);
    // Golemagg pack (SQL: rager 785.579/-974.27/-207.111).
    relocate(&mut map, player, &respawns, 787.0, -976.0, -207.0);
    tick(&mut map, &mut now, 120);
    let rager = find_creature(&map, player, 11_672).expect("Core Rager in view");
    let max = map
        .creature(rager)
        .expect("rager")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_MAXHEALTH);
    // Just above 50%: any hit over ~100 would cross the fork
    // threshold, so `DamageTaken` zeroes it (and heals).
    let set_hp = max / 2 + 100;
    if let Some(c) = map.creature_mut(rager) {
        c.unit.set_health(set_hp);
    }
    map.cast_spell(
        player,
        8_921, // Moonfire: instant direct damage
        CastTarget::Unit(rager),
        CastOptions {
            triggered: true,
            skip_known: true,
            ..Default::default()
        },
    );
    tick(&mut map, &mut now, 10);
    let hp = map
        .creature(rager)
        .expect("rager")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_HEALTH);
    assert!(
        hp >= set_hp,
        "DamageTaken zeroed the crossing hit (hp {hp}, set {set_hp})"
    );
    println!("LIVE-CORE-RAGER-DAMAGE-TAKEN-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_ossirian_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 509);
    let mut now = 1000u32;
    heal(&mut map, player);
    // Ossirian spawn (SQL: -9502.8/2042.65/105.31).
    relocate(&mut map, player, &respawns, -9495.0, 2042.0, 105.0);
    tick(&mut map, &mut now, 120);
    let boss = find_creature(&map, player, 15_339).expect("Ossirian in view");
    assert!(map.ai_in_combat(boss), "Ossirian engaged");
    // Aggro casts Strength of Ossirian (25176) and summons the
    // two tornados at the fork spawn points.
    let mut aura_ok = false;
    for _ in 0..200 {
        tick(&mut map, &mut now, 1);
        if aura_present(&map, boss, 25_176) {
            aura_ok = true;
            break;
        }
    }
    assert!(aura_ok, "Strength of Ossirian aura after aggro");
    relocate(&mut map, player, &respawns, -9400.0, 1950.0, 85.5);
    tick(&mut map, &mut now, 60);
    assert!(
        find_creature(&map, player, 15_428).is_some(),
        "tornado summoned on aggro"
    );
    println!("LIVE-OSSIRIAN-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_onyxia_slice() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 249);
    let mut now = 1000u32;
    heal(&mut map, player);
    // Onyxia spawn (SQL: -4.8689/-217.171/-86.7104).
    relocate(&mut map, player, &respawns, -12.0, -217.0, -86.0);
    tick(&mut map, &mut now, 120);
    let boss = find_creature(&map, player, 10_184).expect("Onyxia in view");
    assert!(map.ai_in_combat(boss), "Onyxia engaged");
    // Drop under 65% -> phase 2 transition -> Hover (17131).
    let max = map
        .creature(boss)
        .expect("boss")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_MAXHEALTH);
    if let Some(c) = map.creature_mut(boss) {
        c.unit.set_health(max * 60 / 100);
    }
    let mut hover = false;
    for _ in 0..1200 {
        if let Some(pos) = map.creature(boss).map(|c| c.unit.world.movement_info.pos) {
            relocate(&mut map, player, &respawns, pos.x + 5.0, pos.y, pos.z);
        }
        tick(&mut map, &mut now, 1);
        if aura_present(&map, boss, 17_131) {
            hover = true;
            break;
        }
    }
    assert!(hover, "Hover aura after phase 2 transition");
    println!("LIVE-ONYXIA-PASS");
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_holy_shock_spell_script() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    let mut now = 1000u32;
    heal(&mut map, player);
    if let Some(p) = map.player_mut(player) {
        p.unit
            .world
            .object
            .values_mut()
            .set_u32(offsets::UNIT_FIELD_LEVEL, 60);
    }
    // Herod spawn point (SQL-verified in the herod slice); he
    // walks to the room center out of combat.
    relocate(&mut map, player, &respawns, 1937.95, -396.44, 23.0);
    tick(&mut map, &mut now, 120);
    let herod = find_creature(&map, player, 3975).expect("Herod in view");
    // Stand 8yd east of Herod's current position, facing east
    // (o=0, away from him).
    let hp = map
        .creature(herod)
        .expect("herod")
        .unit
        .world
        .movement_info
        .pos;
    relocate(&mut map, player, &respawns, hp.x + 8.0, hp.y, hp.z);
    tick(&mut map, &mut now, 5);
    if let Some(p) = map.player_mut(player) {
        p.unit.set_power(0, 100_000);
    }
    // Facing away: OnCheckCast must fail 124.
    let outcome = map.cast_spell(
        player,
        20_473, // Holy Shock rank 1
        CastTarget::Unit(herod),
        CastOptions {
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Failed(124), "not-in-front gate");
    // Move onto Herod and face west: hurt sub-spell 25912 lands.
    let hp = map
        .creature(herod)
        .expect("herod")
        .unit
        .world
        .movement_info
        .pos;
    relocate(&mut map, player, &respawns, hp.x, hp.y, hp.z);
    {
        let mut info = map.player(player).expect("player").unit.world.movement_info;
        info.pos.o = std::f32::consts::PI;
        map.relocate_from_client(player, info, &respawns);
    }
    tick(&mut map, &mut now, 2);
    if let Some(p) = map.player_mut(player) {
        p.unit.set_power(0, 100_000);
    }
    let hp_before = map.creature(herod).expect("herod").unit.health();
    let outcome = map.cast_spell(
        player,
        20_473,
        CastTarget::Unit(herod),
        CastOptions {
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    tick(&mut map, &mut now, 20);
    let hp_after = map.creature(herod).expect("herod").unit.health();
    assert!(hp_after < hp_before, "hurt sub-spell damaged the hostile");
    // Friendly (self) cast: heal sub-spell 25914. Holy Shock
    // has a 30s cooldown; wait it out at full health first.
    heal(&mut map, player);
    tick(&mut map, &mut now, 700);
    if let Some(p) = map.player_mut(player) {
        p.unit.set_health(1000);
        p.unit.set_power(0, 100_000);
    }
    let outcome = map.cast_spell(
        player,
        20_473,
        CastTarget::Unit(player),
        CastOptions {
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    tick(&mut map, &mut now, 20);
    let self_hp = map.player(player).expect("player").unit.health();
    assert!(self_hp > 1000, "heal sub-spell healed the friendly");
    println!("LIVE-HOLY-SHOCK-SPELL-SCRIPT-PASS");
}

/// Tier 3 Slice 2 step 4 (aura-script host): Curse of
/// the Bleakheart (6946, `spell_curse_of_bleakheart`)
/// `OnBeforeApply` sets the first periodic tick to 5s;
/// `OnPeriodicDummy` gives 5% per tick for the target
/// to cast 6945 on itself. Applied to Herod via a
/// triggered cast.
#[test]
#[ignore = "live bench: needs .env.test MySQL + data dir"]
fn bench_bleakheart_aura_script() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 1937.95, -396.44, 23.0);
    tick(&mut map, &mut now, 120);
    let herod = find_creature(&map, player, 3975).expect("Herod in view");
    let hp = map
        .creature(herod)
        .expect("herod")
        .unit
        .world
        .movement_info
        .pos;
    relocate(&mut map, player, &respawns, hp.x, hp.y, hp.z);
    let outcome = map.cast_spell(
        player,
        6946,
        CastTarget::Unit(herod),
        CastOptions {
            triggered: true,
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    let holder = map
        .creature(herod)
        .expect("herod")
        .unit
        .auras
        .iter()
        .find(|h| h.spell_id == 6946)
        .expect("6946 holder applied")
        .clone();
    let eff0 = holder.effects[0].expect("effect 0 aura");
    println!(
        "6946 holder: duration {}ms period {}ms timer {}ms aura_type {}",
        holder.duration_ms, eff0.period_ms, eff0.periodic_timer, eff0.aura_type
    );
    assert!(
        (4900..=5000).contains(&eff0.periodic_timer),
        "OnBeforeApply set the periodic timer to 5s"
    );
    // Tick until the 5% periodic dummy fires 6945 (or
    // the curse expires): track via holder scan.
    let mut fired = false;
    for _ in 0..8000 {
        tick(&mut map, &mut now, 1);
        let c = map.creature(herod).expect("herod");
        if c.unit.auras.iter().any(|h| h.spell_id == 6945) {
            fired = true;
            break;
        }
        if !c.unit.auras.iter().any(|h| h.spell_id == 6946) {
            break;
        }
    }
    assert!(fired, "OnPeriodicDummy fired the 6945 self-cast");
    println!("LIVE-BLEAKHEART-AURA-SCRIPT-PASS");
}

/// Tier 3 Slice 2 step 4 (spell lanes): Conflagrate
/// (17962, `spell_warlock_conflagrate`) `OnCheckCast`
/// requires the caster's own Immolate on the target;
/// without it the cast fails `TARGET_AURASTATE` (103).
#[test]
#[ignore = "live bench: needs .env.test MySQL + data dir"]
fn bench_conflagrate_check_cast_gate() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 189);
    let mut now = 1000u32;
    heal(&mut map, player);
    relocate(&mut map, player, &respawns, 1937.95, -396.44, 23.0);
    tick(&mut map, &mut now, 120);
    let herod = find_creature(&map, player, 3975).expect("Herod in view");
    let hp = map
        .creature(herod)
        .expect("herod")
        .unit
        .world
        .movement_info
        .pos;
    relocate(&mut map, player, &respawns, hp.x, hp.y, hp.z);
    if let Some(p) = map.player_mut(player) {
        p.unit.set_power(0, 100_000);
    }
    let outcome = map.cast_spell(
        player,
        17_962,
        CastTarget::Unit(herod),
        CastOptions {
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Failed(103));
    println!("LIVE-CONFLAGRATE-CHECKCAST-PASS");
}

/// Tier 3 Slice 2 step 5 (gossip host): Horde Res
/// Fixer (13857, `npc_res_fixes`) — hello builds the
/// 6440 menu with one item (sender 1, action 1001);
/// select removes resurrection sickness (15007)
/// and closes. Needs the bench spawn row for
/// 13857 at (1660,-4370) on map 1 (see evidence).
#[test]
#[ignore = "live bench: needs .env.test MySQL + data dir + res fixer spawn"]
fn bench_res_fixer_gossip() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 1);
    let mut now = 1000u32;
    relocate(&mut map, player, &respawns, 1660.0, -4370.0, 31.25);
    tick(&mut map, &mut now, 120);
    let npc = find_creature(&map, player, 13857).expect("res fixer in view");
    let menu = map
        .gossip_script_on_hello(player, npc)
        .expect("script handles hello");
    assert_eq!(menu.text_id, 6440);
    assert_eq!(menu.items.len(), 1);
    assert_eq!((menu.items[0].sender, menu.items[0].action), (1, 1001));
    assert!(
        !menu.items[0].text.is_empty(),
        "broadcast text 8995 resolved"
    );
    // Resurrection sickness on the player (15007
    // is passive; seed the holder directly).
    if let Some(p) = map.player_mut(player) {
        p.unit.auras.push(mangos_entities::aura::AuraHolder::new(
            15_007, player, 60, 600_000, false, false,
        ));
    }
    tick(&mut map, &mut now, 5);
    assert!(
        map.player(player)
            .expect("player")
            .unit
            .auras
            .iter()
            .any(|h| h.spell_id == 15_007),
        "res sickness applied"
    );
    let outcome = map.gossip_script_on_select(player, npc, 1, 1001);
    assert!(matches!(
        outcome,
        Some(mangos_map::gossip::GossipSelectOutcome::Close)
    ));
    tick(&mut map, &mut now, 5);
    assert!(
        !map.player(player)
            .expect("player")
            .unit
            .auras
            .iter()
            .any(|h| h.spell_id == 15_007),
        "res sickness removed by select"
    );
    println!("LIVE-RES-FIXER-GOSSIP-PASS");
}

/// Tier 3 step-6 exemplar: `at_murloc_camp`
/// (`darkshore.cpp:1134`) — quest 4740 incomplete +
/// trigger 1966 summons Murkdeep (10323); a second
/// trigger while he lives is a fork no-op.
#[test]
#[ignore = "requires vmangos-rs/.env.test + local dev bench DB"]
fn bench_at_murloc_camp() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 1);
    let mut now = 1000u32;
    // South of the Murkdeep summon point (4984.772,
    // 596.975) in Darkshore.
    relocate(&mut map, player, &respawns, 4984.0, 590.0, 0.0);
    tick(&mut map, &mut now, 120);
    assert!(
        find_creature(&map, player, 10323).is_none(),
        "murkdeep absent before trigger"
    );
    map.player_mut(player)
        .expect("player")
        .set_quest_status(4740, 3);
    let handled = map.area_trigger_script(player, 1966, "at_murloc_camp");
    assert!(!handled, "fork script acts but returns false");
    tick(&mut map, &mut now, 120);
    let murkdeep = find_creature(&map, player, 10323).expect("murkdeep summoned");
    let count = |map: &Map| {
        map.visible_to(player).map_or(0, |v| {
            v.iter()
                .filter(|g| map.creature(**g).is_some_and(|c| c.info.entry == 10323))
                .count()
        })
    };
    assert_eq!(count(&map), 1);
    // Second trigger while Murkdeep lives: no second summon.
    let handled = map.area_trigger_script(player, 1966, "at_murloc_camp");
    assert!(!handled);
    tick(&mut map, &mut now, 120);
    assert_eq!(count(&map), 1, "no duplicate murkdeep");
    let _ = murkdeep;
    println!("LIVE-MURKDEEP-AREATRIGGER-PASS");
}

/// Tier 3 step-6 dummy host exemplar: `peon_wake_up`
/// (`durotar.cpp`) — Awaken Peon (19938) on a sleeping
/// Lazy Peon (10556, sleep buff 17743) grants quest
/// 5441 monster credit via `pEffectDummyCreature`.
#[test]
#[ignore = "requires vmangos-rs/.env.test + local dev bench DB"]
fn bench_peon_wake_up_dummy() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 1);
    let mut now = 1000u32;
    // Lazy Peon spawn 7375 (-263.542, -4143.64, 56.5239).
    relocate(&mut map, player, &respawns, -263.542, -4143.64, 56.5239);
    tick(&mut map, &mut now, 120);
    // Nearest peon (several spawns share the camp;
    // visible-set order is not stable).
    let player_pos = map
        .player(player)
        .expect("player")
        .unit
        .world
        .movement_info
        .pos;
    let peon = map
        .visible_to(player)
        .expect("visibility")
        .iter()
        .copied()
        .filter(|g| map.creature(*g).is_some_and(|c| c.info.entry == 10556))
        .min_by_key(|g| {
            let cp = map.creature(*g).expect("peon").unit.world.movement_info.pos;
            let dx = cp.x - player_pos.x;
            let dy = cp.y - player_pos.y;
            (dx * dx + dy * dy).to_bits()
        })
        .expect("lazy peon in view");
    // The AI re-casts the sleep buff in STATE_SLEEPING;
    // seed the holder when that arm has not landed yet.
    let sleeping = map
        .creature(peon)
        .expect("peon")
        .unit
        .auras
        .iter()
        .any(|h| h.spell_id == 17743);
    if !sleeping {
        if let Some(c) = map.creature_mut(peon) {
            c.unit.auras.push(mangos_entities::aura::AuraHolder::new(
                17743, peon, 1, 600_000, false, false,
            ));
        }
    }
    map.player_mut(player)
        .expect("player")
        .set_quest_status(5441, 3);
    let credit_before: u32 = map
        .player(player)
        .expect("player")
        .quest_row(5441)
        .map_or(0, |r| r.mob_count.iter().sum());
    assert_eq!(credit_before, 0);
    let outcome = map.cast_spell(
        player,
        19938,
        CastTarget::Unit(peon),
        CastOptions {
            skip_known: true,
            ..CastOptions::default()
        },
    );
    assert_eq!(outcome, CastOutcome::Started, "awaken cast starts");
    tick(&mut map, &mut now, 20);
    let credit_after: u32 = map
        .player(player)
        .expect("player")
        .quest_row(5441)
        .map_or(0, |r| r.mob_count.iter().sum());
    assert_eq!(credit_after, 1, "dummy granted monster credit");
    println!("LIVE-PEON-WAKEUP-DUMMY-PASS");
}

/// Tier 3 step-6 process-event exemplar:
/// `event_test_of_endurance` (747) via the Harpy
/// Foodstuffs goober (20447, quest gate 1150) summons
/// Grenka (4490); a second use while she lives is a
/// fork no-op.
#[test]
#[ignore = "requires vmangos-rs/.env.test + local dev bench DB"]
fn bench_test_of_endurance_process_event() {
    let stores = load_stores();
    let (mut map, player, respawns) = make_map(&stores, 1);
    let mut now = 1000u32;
    // Harpy Foodstuffs spawn 17208 (-5570.18, -1571.24, 9.65319).
    relocate(&mut map, player, &respawns, -5570.18, -1571.24, 9.65319);
    tick(&mut map, &mut now, 120);
    let go = find_go(&map, player, 20447).expect("foodstuffs in view");
    assert!(
        find_creature(&map, player, 4490).is_none(),
        "grenka absent before use"
    );
    // Goober quest gate: quest 1150 must be INCOMPLETE.
    map.player_mut(player)
        .expect("player")
        .set_quest_status(1150, 3);
    map.gameobject_use(player, go);
    tick(&mut map, &mut now, 120);
    assert!(
        find_creature(&map, player, 4490).is_some(),
        "grenka summoned by process event"
    );
    map.gameobject_use(player, go);
    tick(&mut map, &mut now, 10);
    let count = map.visible_to(player).map_or(0, |v| {
        v.iter()
            .filter(|g| map.creature(**g).is_some_and(|c| c.info.entry == 4490))
            .count()
    });
    assert_eq!(count, 1, "no duplicate grenka");
    println!("LIVE-TEST-OF-ENDURANCE-EVENT-PASS");
}
