//! Live AI/melee evidence on the dev bench (Slice 6a):
//! proximity aggro on a Defias Pillager, chase + white
//! swings with damage bounds from independent SQL
//! (`creature_classlevelstats` + template damage
//! columns), spell threat, leash evade + home regen,
//! and player auto-attacks. Read-only against the DB.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::spells::{CastOptions, CastTarget};
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

fn heal_player_full(map: &mut Map, player_guid: ObjectGuid) {
    // The Slice 3 naked-stat subset leaves the bench
    // warrior with very low max health, so the dummy
    // pool is raised for the scenario duration.
    let player = map.player_mut(player_guid).expect("player");
    player
        .unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
    player.unit.set_health(100_000);
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_creature_ai_and_melee() {
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

    // Oracle: pillager spawn position + cls damage row
    // + template damage columns, straight from SQL.
    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let (stores, pillager_spawn) = world
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
            let pillager_spawn: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT position_x, position_y, position_z FROM creature \
                     WHERE id=589 AND map=0 ORDER BY guid LIMIT 1",
                )
                .map_err(mangos_db::DbError::from)?;
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
                    spells: mangos_spells::SpellStore {
                        spells: spell_entries,
                        tables: mangos_spells::load_spell_dbc_tables(std::path::Path::new(
                            &std::env::var("MANGOS_DBC_DIR").expect("MANGOS_DBC_DIR"),
                        ))
                        .expect("spell dbc tables"),
                    },
                    creature_spells,
                    scripts: mangos_scripts_engine::ScriptStores::default(),
                    display_scales: HashMap::new(),
                    model_data: HashMap::new(),
                },
                pillager_spawn.expect("pillager spawn"),
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
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();
    let player_armor = player
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_RESISTANCES);
    let player_health0 = player.unit.health();

    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(23),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    let _ = map.drain_events();
    let mut now_ms = 1000u32;
    let mut tick = |map: &mut Map, n: u32| {
        for _ in 0..n {
            now_ms += 50;
            map.tick(50, now_ms, 1_800_000_000);
        }
    };

    // Relocate 12 yd from the pillager spawn (inside its
    // 18 yd detection range) and let cells activate.
    let spawn_pos = Position::new(pillager_spawn.0, pillager_spawn.1, pillager_spawn.2, 0.0);
    {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(spawn_pos.x + 12.0, spawn_pos.y, spawn_pos.z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    }
    tick(&mut map, 4);
    // Events stay queued: the aggro tick may already
    // have emitted ATTACKSTART; the loop below counts
    // from the same queue.

    // Find the pillager (nearest visible creature with a
    // spells list = template entry 589 family).
    let player_pos = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let visible = map.visible_to(player_guid).cloned().expect("visible");
    let target = visible
        .iter()
        .copied()
        .filter_map(|g| {
            map.creature(g).and_then(|c| {
                (c.info.spell_list_id == 5890).then_some((
                    g,
                    c.unit.world.position(),
                    c.unit.health(),
                ))
            })
        })
        .min_by(|a, b| dist2d(player_pos, a.1).total_cmp(&dist2d(player_pos, b.1)))
        .map(|(g, _, _)| g)
        .expect("a Defias Pillager in view");
    // The creature's position when found stands in for
    // its spawn home (wander may offset it a few yards).
    let target_home = map.creature(target).expect("target").unit.world.position();
    let target_max_health = {
        let c = map.creature(target).expect("target");
        c.unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_MAXHEALTH)
    };
    let (target_level, target_class, dmg_mult, dmg_var) = {
        let c = map.creature(target).expect("target");
        (
            c.unit.level(),
            c.info.unit_class,
            c.info.damage_multiplier,
            c.info.damage_variance,
        )
    };

    // --- Aggro + chase + melee -------------------------
    let mut attack_starts = 0usize;
    let mut state_updates: Vec<u32> = Vec::new();
    for _ in 0..400 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
            {
                match packet.opcode() {
                    opcode_id::SMSG_ATTACKSTART => attack_starts += 1,
                    opcode_id::SMSG_ATTACKERSTATEUPDATE => {
                        let body = packet.body();
                        let (_, u1) = ObjectGuid::read_packed(&body[4..]).expect("packed atk");
                        let (_, u2) = ObjectGuid::read_packed(&body[4 + u1..]).expect("packed vic");
                        let off = 4 + u1 + u2;
                        state_updates.push(u32::from_le_bytes([
                            body[off],
                            body[off + 1],
                            body[off + 2],
                            body[off + 3],
                        ]));
                    }
                    _ => {}
                }
            }
        }
        if state_updates.len() >= 4 {
            break;
        }
    }
    println!(
        "AGGRO in_combat={} attack_starts={} swings={state_updates:?}",
        map.ai_in_combat(target),
        attack_starts
    );
    assert!(map.ai_in_combat(target), "pillager never aggroed");
    assert!(attack_starts >= 1, "no SMSG_ATTACKSTART");
    assert!(
        map.creature(target).expect("target").unit.melee_victim == Some(player_guid),
        "pillager victim must be the player"
    );
    assert!(
        state_updates.len() >= 4,
        "too few swings: {state_updates:?}"
    );

    // Damage bounds from SQL cls (independent read via
    // a second connection to the world DB).
    let cls_row: (f32, i32) = {
        let world2 = DbActor::spawn("test-oracle", &world_url).expect("oracle");
        world2
            .call(move |conn| {
                use mysql::prelude::Queryable;
                let row: Option<(f32, i32)> = conn
                    .query_first(format!(
                        "SELECT melee_damage, attack_power FROM creature_classlevelstats \
                         WHERE level={target_level} AND class={target_class}"
                    ))
                    .map_err(mangos_db::DbError::from)?;
                Ok(row.expect("cls row"))
            })
            .expect("cls")
    };
    let avg = cls_row.0 * dmg_mult;
    let (raw_lo, raw_hi) = (avg - avg * dmg_var, avg + avg * dmg_var);
    // CalcArmorReducedDamage re-derivation.
    let armor_factor = {
        #[allow(clippy::cast_precision_loss)]
        let armor = player_armor as f32;
        #[allow(clippy::cast_precision_loss)]
        let reduced = armor / (armor + 400.0 + 85.0 * target_level as f32);
        1.0 - reduced.min(0.75)
    };
    let armored_lo = (raw_lo * armor_factor).floor() as i64 - 1;
    let armored_hi_crit = (raw_hi * armor_factor * 2.0).ceil() as i64 + 1;
    let damaging: Vec<u32> = state_updates.iter().copied().filter(|d| *d > 0).collect();
    println!(
        "MELEE damaging={damaging:?} armored_bounds=[{armored_lo}, {armored_hi_crit}] ap={}",
        cls_row.1
    );
    assert!(!damaging.is_empty(), "no damaging swings landed");
    for dmg in &damaging {
        assert!(
            i64::from(*dmg) >= armored_lo.max(0) && i64::from(*dmg) <= armored_hi_crit,
            "swing damage {dmg} outside SQL bounds"
        );
    }
    let player_health1 = map.player_mut(player_guid).expect("player").unit.health();
    assert!(player_health1 < player_health0, "player took no damage");

    // --- Spell threat (Rend from Slice 5) --------------
    // The pillager out-damages the level-19 bench warrior
    // over a long scenario, so health is topped up
    // between phases (in-memory only; healing systems
    // are not under test here).
    heal_player_full(&mut map, player_guid);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(1, 1000);
    let mut threat_seen = 0.0f32;
    for _ in 0..10 {
        map.cast_spell(
            player_guid,
            6546,
            CastTarget::Unit(target),
            CastOptions::default(),
        );
        tick(&mut map, 40);
        let _ = map.drain_events();
        threat_seen = map.threat_on(target, player_guid);
        if map.aura_holders(target).iter().any(|h| h.spell_id == 6546) {
            tick(&mut map, 70);
            let _ = map.drain_events();
            threat_seen = map.threat_on(target, player_guid);
            break;
        }
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(1, 1000);
    }
    println!("THREAT after rend ticks: {threat_seen}");
    assert!(threat_seen > 0.0, "no damage threat recorded");

    // --- Leash evade ------------------------------------
    heal_player_full(&mut map, player_guid);
    {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(spawn_pos.x + 160.0, spawn_pos.y, spawn_pos.z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    }
    let mut evaded = false;
    for _ in 0..1200 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        if !map.ai_in_combat(target) {
            evaded = true;
            break;
        }
    }
    assert!(evaded, "pillager never leash-evaded");
    assert!(
        map.threat_on(target, player_guid).abs() < f32::EPSILON,
        "threat not cleared"
    );
    // Home + regen to full.
    let mut home_full = false;
    for _ in 0..1600 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        let c = map.creature(target).expect("target");
        if dist2d(c.unit.world.position(), target_home) < 20.0
            && c.unit.health() == target_max_health
        {
            home_full = true;
            break;
        }
    }
    println!("EVADE home_full={home_full}");
    assert!(home_full, "pillager did not return home at full health");

    // --- Player auto-attack -----------------------------
    heal_player_full(&mut map, player_guid);
    {
        let cpos = map.creature(target).expect("target").unit.world.position();
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(cpos.x + 2.0, cpos.y, cpos.z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    }
    tick(&mut map, 2);
    let _ = map.drain_events();
    let before = map.creature(target).expect("target").unit.health();
    map.attack_start(player_guid, target);
    let mut player_swings = 0usize;
    for _ in 0..400 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_ATTACKERSTATEUPDATE
            {
                let body = packet.body();
                let (attacker_guid, _) = ObjectGuid::read_packed(&body[4..]).expect("packed atk");
                if attacker_guid == player_guid {
                    player_swings += 1;
                }
            }
        }
        if player_swings >= 3 {
            break;
        }
    }
    let after = map.creature(target).expect("target").unit.health();
    println!("PLAYER-MELEE swings={player_swings} health {before}->{after}");
    assert!(player_swings >= 3, "player swings not observed");
    assert!(after < before, "pillager took no player melee damage");

    println!("LIVE-AI-PASS");
}
