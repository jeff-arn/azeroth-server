//! Live kill-XP evidence on the dev bench (depth-gap
//! Slice 1): a Fire Blast kill grants the fork
//! `XP::Gain` amount, consumes rested bonus, emits
//! `SMSG_LOG_XPGAIN`, and — with `PLAYER_XP` primed
//! one point below the threshold in memory — dings
//! with `SMSG_LEVELUP_INFO` stat deltas checked
//! against raw `player_levelstats` SQL, never the
//! Rust stores under test. Read-only against the DB.

use std::collections::HashMap;
use std::path::PathBuf;
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

const FIRE_BLAST: u32 = 2136;

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        other => panic!("field {idx}: expected UInt, got {other:?}"),
    }
}

fn race_class_dbc(dir: &std::path::Path) -> RaceClassDbc {
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
    dbc
}

/// Independent `XP::Gain` re-derivation from the fork
/// formulas over raw SQL template columns.
#[allow(clippy::cast_precision_loss)]
fn expected_gain(pl_level: u32, mob_level: u32, t: &TemplateOracle) -> u32 {
    if t.static_flags1 & 0x2 != 0 {
        return 0;
    }
    let gray = if pl_level <= 5 {
        0
    } else if pl_level <= 39 {
        pl_level - 5 - pl_level / 10
    } else {
        pl_level - 1 - pl_level / 5
    };
    let factor = if mob_level >= pl_level {
        1.0f32 + 0.05 * ((mob_level - pl_level).min(4) as f32)
    } else if mob_level > gray {
        let zd = match pl_level {
            0..8 => 5,
            8..10 => 6,
            10..12 => 7,
            12..16 => 8,
            16..20 => 9,
            20..30 => 11,
            30..40 => 12,
            40..45 => 13,
            45..50 => 14,
            50..55 => 15,
            55..60 => 16,
            _ => 17,
        };
        (zd + mob_level - pl_level) as f32 / zd as f32
    } else {
        0.0
    };
    let mut gain = ((pl_level * 5) as f32 + 45.0) * factor;
    if gain == 0.0 {
        return 0;
    }
    // Map 0 is not a dungeon; elite doubles outdoors.
    if t.rank != 0 && t.rank != 4 {
        gain *= 2.0;
    }
    gain *= t.xp_multiplier;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        gain.round_ties_even() as u32
    }
}

#[derive(Clone, Copy)]
struct TemplateOracle {
    rank: u32,
    xp_multiplier: f32,
    static_flags1: u32,
}

fn dist2d(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_kill_xp_and_level_up() {
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
    let (stores, templates, next_level_xp, hunt_pos) = world
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
            // Independent oracle columns for every
            // creature template (candidates picked by
            // entry below).
            let rows: Vec<(u32, u32, f32, u32)> = conn
                .query(
                    "SELECT entry, `rank`, xp_multiplier, static_flags1 \
                     FROM creature_template",
                )
                .map_err(mangos_db::DbError::from)?;
            let templates: HashMap<u32, TemplateOracle> = rows
                .into_iter()
                .map(|(entry, rank, xp_multiplier, static_flags1)| {
                    (
                        entry,
                        TemplateOracle {
                            rank,
                            xp_multiplier,
                            static_flags1,
                        },
                    )
                })
                .collect();
            let next_level_xp: Vec<(u32, u32)> = conn
                .query("SELECT lvl, xp_for_next_level FROM player_xp_for_level")
                .map_err(mangos_db::DbError::from)?;
            // A hunting ground: nearest map-0 spawn that
            // is green-or-better for a level-19 fixture
            // (template min level above the gray line,
            // XP multiplier and NO_XP flag sane). The
            // fixture's saved spot only has grays/NPCs.
            let hunt_pos: Option<(f32, f32, f32)> = conn
                .query_first(
                    "SELECT c.position_x, c.position_y, c.position_z \
                     FROM creature c JOIN creature_template t ON t.entry = c.id \
                     WHERE c.map = 0 AND t.level_min >= 14 AND t.level_max <= 30 \
                       AND t.xp_multiplier > 0 AND (t.static_flags1 & 2) = 0 \
                     ORDER BY (c.position_x + 10641) * (c.position_x + 10641) \
                            + (c.position_y - 1181) * (c.position_y - 1181) \
                     LIMIT 1",
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
                    creature_spells: mangos_world_data::creature_spells::CreatureSpellsMap::new(),
                    scripts: mangos_scripts_engine::ScriptStores::default(),
                    display_scales: HashMap::new(),
                    model_data: HashMap::new(),
                    shapeshift_forms: HashMap::new(),
                    skills: mangos_world_data::skill_info::SkillDbc::default(),
                    chat_channels: mangos_world_data::chat_channels::ChatChannelDbc::default(),
                    auction_houses: mangos_world_data::auction_house::AuctionHouseDbc::default(),
                    maps: mangos_world_data::map_template::MapStore::default(),
                    area_triggers: mangos_world_data::area_trigger::AreaTriggerStore::default(),
                    quests: mangos_world_data::quest_template::QuestTemplatesLite::default(),
                    quest_templates: mangos_world_data::quest_template::QuestTemplates::default(),
                    quest_relations: mangos_world_data::quest_template::QuestRelations::default(),
                    tavern_triggers: std::collections::HashSet::new(),
                },
                templates,
                next_level_xp.into_iter().collect::<HashMap<u32, u32>>(),
                hunt_pos,
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
        skills: None,
        item_prototypes: None,
        quest_templates: None,
    };
    let player = Player::materialize(load, &inputs).expect("player");
    let player_guid: ObjectGuid = player.unit.guid();
    let pl_level = player.unit.level();
    assert!(pl_level > 0 && pl_level < 60, "fixture level {pl_level}");

    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(11),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    {
        let player = map.player_mut(player_guid).expect("player");
        player
            .unit
            .world
            .object
            .values_mut()
            .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 100_000);
        player.unit.set_health(100_000);
    }
    let _ = map.drain_events();
    let mut now_ms = 1000u32;
    let mut tick = |map: &mut Map, n: u32| {
        for _ in 0..n {
            now_ms += 50;
            map.tick(50, now_ms, 1_800_000_000);
        }
    };

    // Move to the hunting ground and let the cells
    // around it load.
    let (hx, hy, hz) = hunt_pos.expect("a green spawn on map 0");
    {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(hx + 2.0, hy, hz, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    }
    tick(&mut map, 10);
    let _ = map.drain_events();

    // Nearest visible creature that grants XP.
    let visible = map.visible_to(player_guid).cloned().expect("visible");
    let player_pos = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let (target, expected_xp) = visible
        .iter()
        .copied()
        .filter_map(|g| {
            let c = map.creature(g)?;
            let t = templates.get(&c.original_entry)?;
            let xp = expected_gain(pl_level, c.unit.level(), t);
            (xp > 0).then_some((g, xp, c.unit.world.position()))
        })
        .min_by(|a, b| dist2d(player_pos, a.2).total_cmp(&dist2d(player_pos, b.2)))
        .map(|(g, xp, _)| (g, xp))
        .expect("an XP-granting creature in view");
    let target_pos = map.creature(target).expect("target").unit.world.position();
    {
        let mut info = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .movement_info;
        info.pos = Position::new(target_pos.x + 2.0, target_pos.y, target_pos.z, 0.0);
        map.relocate_from_client(player_guid, info, &respawns);
    }
    let _ = map.drain_events();

    // Prime the ding in memory: PLAYER_XP one point
    // below the SQL next-level threshold.
    let threshold = next_level_xp[&pl_level];
    let rest_before = map.player(player_guid).expect("player").rest_bonus;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let expected_rest = (rest_before as u32).min(expected_xp);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::PLAYER_XP, threshold - 1);

    // One-shot the target: 1 HP, then Fire Blast
    // until one lands (triggered casts skip the GCD
    // and category cooldown, as in live_spells).
    map.creature_mut(target).expect("target").unit.set_health(1);
    let mut died = false;
    let mut log_packet: Option<Vec<u8>> = None;
    let mut levelup_packet: Option<Vec<u8>> = None;
    let mut saw_values_update = false;
    for _ in 0..60 {
        if map.creature(target).expect("target").unit.health() > 0 {
            map.player_mut(player_guid)
                .expect("player")
                .unit
                .set_power(0, 1000);
            map.cast_spell(
                player_guid,
                FIRE_BLAST,
                CastTarget::Unit(target),
                CastOptions {
                    triggered: true,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
        }
        tick(&mut map, 2);
        for event in map.drain_events() {
            match event {
                MapEvent::UnitDied { guid, .. } if guid == target => died = true,
                MapEvent::Packet { to, packet } if to == player_guid => match packet.opcode() {
                    opcode_id::SMSG_LOG_XPGAIN => log_packet = Some(packet.body().to_vec()),
                    opcode_id::SMSG_LEVELUP_INFO => {
                        levelup_packet = Some(packet.body().to_vec());
                    }
                    opcode_id::SMSG_UPDATE_OBJECT | opcode_id::SMSG_COMPRESSED_UPDATE_OBJECT => {
                        saw_values_update = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if died && log_packet.is_some() && levelup_packet.is_some() {
            break;
        }
    }
    assert!(died, "target never died");
    let log = log_packet.expect("SMSG_LOG_XPGAIN never emitted");
    let victim_raw = u64::from_le_bytes(log[0..8].try_into().expect("guid"));
    assert_eq!(victim_raw, target.raw());
    let total = u32::from_le_bytes(log[8..12].try_into().expect("total"));
    assert_eq!(log[12], 0, "kill XP type byte");
    let given = u32::from_le_bytes(log[13..17].try_into().expect("given"));
    assert_eq!(given, expected_xp, "given XP vs SQL-derived Gain");
    assert_eq!(total, expected_xp + expected_rest, "total incl. rested");

    let up = levelup_packet.expect("SMSG_LEVELUP_INFO never emitted");
    let new_level = u32::from_le_bytes(up[0..4].try_into().expect("level"));
    assert_eq!(new_level, pl_level + 1);

    let player = map.player(player_guid).expect("player");
    let v = player.unit.world.object.values();
    assert_eq!(v.get_u32(offsets::UNIT_FIELD_LEVEL), pl_level + 1);
    assert_eq!(
        v.get_u32(offsets::PLAYER_XP),
        threshold - 1 + expected_xp + expected_rest - threshold
    );
    assert_eq!(
        v.get_u32(offsets::PLAYER_NEXT_LEVEL_XP),
        next_level_xp[&(pl_level + 1)]
    );
    assert_eq!(player.unit.health(), player.unit.max_health());
    // The ding's field writes must actually ship: one more
    // tick of `SendObjectUpdates` leaves the mirror synced
    // (only the values flush syncs it for in-world objects)
    // and an update packet addressed to the gainer was seen.
    // Regression: the flush once keyed only on
    // `is_object_updated()`, which no writer ever set, so a
    // stock client's character sheet never left level 19.
    tick(&mut map, 1);
    for event in map.drain_events() {
        if let MapEvent::Packet { to, packet } = event
            && to == player_guid
            && matches!(
                packet.opcode(),
                opcode_id::SMSG_UPDATE_OBJECT | opcode_id::SMSG_COMPRESSED_UPDATE_OBJECT
            )
        {
            saw_values_update = true;
        }
    }
    assert!(saw_values_update, "no values update packet for the gainer");
    assert!(
        !map.player(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .has_changes(),
        "values mirror never synced (flush did not ship the ding)"
    );
    println!(
        "LIVE-XP-PASS given={expected_xp} rest={expected_rest} level {} -> {}",
        pl_level,
        pl_level + 1
    );
}
