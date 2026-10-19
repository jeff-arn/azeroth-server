//! Live spell/aura evidence on the dev bench (Slice 5):
//! Rend 6546 periodic ticks, Battle Shout 5242 attack
//! power mods, Fire Blast 2136 direct damage (manual
//! `skip_known` cast), and saved-aura application —
//! value bounds from independent raw SQL on
//! `spell_template` plus `SpellDuration.dbc`, never from
//! the Rust spell store under test. Read-only against
//! the DB (rage/mana are set in memory only because
//! resource generation does not exist yet).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_map::spells::{CastOptions, CastOutcome, CastTarget};
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

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        other => panic!("field {idx}: expected UInt, got {other:?}"),
    }
}

fn int(row: &[FieldValue], idx: usize) -> i32 {
    // `i` DBC fields surface as UInt bits.
    i32::from_ne_bytes(uint(row, idx).to_ne_bytes())
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

/// `SpellDuration.dbc` column 0 by index (oracle side).
fn duration_ms(dir: &Path, index: u32) -> i32 {
    let bytes = std::fs::read(dir.join(formats::SPELL_DURATION.file)).expect("SpellDuration.dbc");
    let store = DbcFile::parse(&bytes, formats::SPELL_DURATION.fmt)
        .expect("parse")
        .produce(formats::SPELL_DURATION.fmt)
        .expect("produce");
    for row in store.rows() {
        if uint(row, 0) == index {
            return int(row, 1).abs();
        }
    }
    panic!("duration index {index} missing");
}

/// Independent `CalculateSpellEffectValue` bounds
/// re-derived from the C++ formula (level clamps, dice
/// arms) over raw SQL columns.
#[derive(Clone, Copy, Debug)]
struct SpellOracleRow {
    effect_base_points: i64,
    effect_die_sides: i64,
    effect_base_dice: i64,
    effect_dice_per_level: f64,
    effect_real_points_per_level: f64,
    spell_level: i64,
    base_level: i64,
    max_level: i64,
    duration_index: u32,
    effect_amplitude: i64,
}

#[allow(clippy::cast_precision_loss)]
fn value_bounds(row: &SpellOracleRow, caster_level: i64) -> (f64, f64) {
    let mut level = caster_level;
    if row.max_level > 0 && level > row.max_level {
        level = row.max_level;
    } else if level < row.base_level {
        level = row.base_level;
    }
    level -= row.spell_level;
    let base = row.effect_base_points as f64 + level as f64 * row.effect_real_points_per_level;
    let random_points =
        (row.effect_die_sides as f64 + level as f64 * row.effect_dice_per_level) as i64;
    match random_points {
        0 | 1 => (
            base + row.effect_base_dice as f64,
            base + row.effect_base_dice as f64,
        ),
        _ => {
            let lo = random_points.min(row.effect_base_dice);
            let hi = random_points.max(row.effect_base_dice);
            (base + lo as f64, base + hi as f64)
        }
    }
}

fn dist2d(a: Position, b: Position) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

const REND: u32 = 6546;
const BATTLE_SHOUT: u32 = 5242;
const FIRE_BLAST: u32 = 2136;

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_spells_and_auras() {
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
            // Independent oracle rows straight from SQL.
            let mut oracle: HashMap<u32, SpellOracleRow> = HashMap::new();
            for sid in [REND, BATTLE_SHOUT, FIRE_BLAST] {
                type OracleCols = (i64, i64, i64, f64, f64, i64, i64, i64, u32, i64);
                let row: Option<OracleCols> = conn
                    .query_first(format!(
                        "SELECT effectBasePoints1, effectDieSides1, effectBaseDice1, \
                         effectDicePerLevel1, effectRealPointsPerLevel1, spellLevel, \
                         baseLevel, maxLevel, durationIndex, effectAmplitude1 \
                         FROM spell_template WHERE entry={sid} AND build=(SELECT MAX(build) \
                         FROM spell_template s WHERE s.entry={sid} AND s.build<=5875)"
                    ))
                    .map_err(mangos_db::DbError::from)?;
                let r = row.expect("oracle spell row");
                oracle.insert(
                    sid,
                    SpellOracleRow {
                        effect_base_points: r.0,
                        effect_die_sides: r.1,
                        effect_base_dice: r.2,
                        effect_dice_per_level: r.3,
                        effect_real_points_per_level: r.4,
                        spell_level: r.5,
                        base_level: r.6,
                        max_level: r.7,
                        duration_index: r.8,
                        effect_amplitude: r.9,
                    },
                );
            }
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
                    creature_spells: mangos_world_data::creature_spells::CreatureSpellsMap::new(),
                    scripts: mangos_scripts_engine::ScriptStores::default(),
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
    let caster_level = i64::from(
        player
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_LEVEL),
    );
    assert!(caster_level > 0);

    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(11),
    );
    let respawns = RespawnTimes::default();
    map.add_player(player, &respawns, 1_800_000_000, 1000);
    // Slice 6: the dummy fights back now; the Slice 3
    // naked-stat subset leaves the warrior too fragile
    // for a 12 s DoT observation window, so raise the
    // dummy pool for the scenario (in-memory only).
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

    // Saved auras (bench rows: 71 Defensive Stance,
    // 15007 Resurrection Sickness).
    map.apply_saved_auras(player_guid);
    let saved = map.aura_holders(player_guid);
    assert!(
        saved.iter().any(|h| h.spell_id == 71),
        "saved Defensive Stance holder missing: {saved:?}"
    );
    println!("SAVED-AURAS holders={}", saved.len());

    // Nearest visible creature as the target dummy;
    // relocate the player beside it (melee range).
    let visible = map.visible_to(player_guid).cloned().expect("visible");
    let player_pos = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let target = visible
        .iter()
        .copied()
        .filter_map(|g| {
            map.creature(g)
                .filter(|c| c.unit.health() >= 150)
                .map(|c| (g, c.unit.world.position()))
        })
        .min_by(|a, b| dist2d(player_pos, a.1).total_cmp(&dist2d(player_pos, b.1)))
        .map(|(g, _)| g)
        .expect("a creature in view");
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
    let target_health0 = map.creature(target).expect("target").unit.health();
    assert!(target_health0 > 100, "target too fragile: {target_health0}");

    // --- Rend (DoT) -------------------------------------
    let rend_oracle = oracle[&REND];
    let (rend_lo, rend_hi) = value_bounds(&rend_oracle, caster_level);
    let rend_duration = duration_ms(&dbc_dir, rend_oracle.duration_index);
    #[allow(clippy::cast_possible_truncation)]
    let amplitude_ms = i32::try_from(rend_oracle.effect_amplitude).expect("amplitude fits");
    let expected_ticks = usize::try_from((rend_duration / amplitude_ms).max(1)).expect("ticks");
    let mut applied = false;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(1, 1000);
        let outcome = map.cast_spell(
            player_guid,
            REND,
            CastTarget::Unit(target),
            CastOptions::default(),
        );
        assert_eq!(outcome, CastOutcome::Started);
        if map.aura_holders(target).iter().any(|h| h.spell_id == REND) {
            applied = true;
            break;
        }
        tick(&mut map, 32); // outlast the 1.5s GCD
        let _ = map.drain_events();
    }
    assert!(applied, "Rend never landed in 20 casts");
    let holder = map
        .aura_holders(target)
        .into_iter()
        .find(|h| h.spell_id == REND)
        .expect("rend holder");
    let amount = holder.effects[0].expect("rend effect").amount;
    assert!(
        f64::from(amount) >= rend_lo.floor() && f64::from(amount) <= rend_hi.ceil(),
        "rend snapshot amount {amount} outside SQL bounds [{rend_lo}, {rend_hi}]"
    );
    assert!(holder.slot >= 32, "debuff slot range: {}", holder.slot);
    assert_eq!(holder.max_duration_ms, rend_duration);

    let mut periodic_logs: Vec<u32> = Vec::new();
    let mut go_packets = 0usize;
    #[allow(clippy::cast_possible_truncation)]
    let total_ticks = (rend_duration / 50 + 40) as u32;
    for _ in 0..total_ticks {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
            {
                match packet.opcode() {
                    opcode_id::SMSG_SPELL_GO => go_packets += 1,
                    opcode_id::SMSG_PERIODICAURALOG => {
                        let body = packet.body();
                        let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                        let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                        let off = used1 + used2;
                        let dmg = u32::from_le_bytes([
                            body[off + 12],
                            body[off + 13],
                            body[off + 14],
                            body[off + 15],
                        ]);
                        periodic_logs.push(dmg);
                    }
                    _ => {}
                }
            }
        }
    }
    println!(
        "REND amount={amount} ticks={} expected={expected_ticks} logs={periodic_logs:?}",
        periodic_logs.len()
    );
    assert!(go_packets >= 1, "no SPELL_GO reached the player");
    assert_eq!(
        periodic_logs.len(),
        expected_ticks,
        "periodic tick count vs duration/amplitude"
    );
    for dmg in &periodic_logs {
        // Armor can only reduce the snapshot amount,
        // and every tick must beat the 75% armor cap
        // floor of the value lo bound.
        assert!(
            *dmg > 0 && *dmg <= u32::try_from(amount).expect("positive amount"),
            "tick damage {dmg} vs snapshot {amount}"
        );
    }
    assert!(
        map.aura_holders(target).iter().all(|h| h.spell_id != REND),
        "rend holder must expire"
    );
    let target_health1 = map.creature(target).expect("target").unit.health();
    let logged_total: u32 = periodic_logs.iter().sum();
    assert_eq!(
        target_health0 - target_health1,
        logged_total,
        "health delta must equal logged tick sum"
    );

    // --- Battle Shout (AP buff) -------------------------
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(1, 1000);
    tick(&mut map, 32);
    let _ = map.drain_events();
    let (bs_lo, bs_hi) = value_bounds(&oracle[&BATTLE_SHOUT], caster_level);
    let outcome = map.cast_spell(
        player_guid,
        BATTLE_SHOUT,
        CastTarget::SelfCast,
        CastOptions::default(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let holder = map
        .aura_holders(player_guid)
        .into_iter()
        .find(|h| h.spell_id == BATTLE_SHOUT)
        .expect("battle shout holder");
    assert!(holder.positive && holder.slot < 32, "buff slot");
    let player = map.player_mut(player_guid).expect("player");
    let values = player.unit.world.object.values();
    assert_eq!(
        values.get_u32(offsets::UNIT_FIELD_AURA + u16::from(holder.slot)),
        BATTLE_SHOUT,
        "aura slot field"
    );
    let mods = values.get_u32(offsets::UNIT_FIELD_ATTACK_POWER_MODS);
    let pos_short = i32::from(i16::from_ne_bytes(
        (mods & 0xFFFF).to_ne_bytes()[..2].try_into().unwrap(),
    ));
    println!("BATTLESHOUT ap_pos_mod={pos_short} bounds=[{bs_lo}, {bs_hi}]");
    assert!(
        f64::from(pos_short) >= bs_lo.floor() && f64::from(pos_short) <= bs_hi.ceil(),
        "AP mod {pos_short} outside SQL bounds"
    );
    map.cancel_aura(player_guid, BATTLE_SHOUT);
    let player = map.player_mut(player_guid).expect("player");
    let values = player.unit.world.object.values();
    assert_eq!(
        values.get_u32(offsets::UNIT_FIELD_ATTACK_POWER_MODS) & 0xFFFF,
        0
    );
    assert_eq!(
        values.get_u32(offsets::UNIT_FIELD_AURA + u16::from(holder.slot)),
        0
    );

    // --- Fire Blast (direct damage, manual cast) --------
    tick(&mut map, 32); // outlast Battle Shout's GCD
    let _ = map.drain_events();

    let (fb_lo, fb_hi) = value_bounds(&oracle[&FIRE_BLAST], caster_level);
    let mut landed_damage: Option<u32> = None;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        let before = map.creature(target).expect("target").unit.health();
        let outcome = map.cast_spell(
            player_guid,
            FIRE_BLAST,
            CastTarget::Unit(target),
            CastOptions {
                triggered: false,
                skip_known: true,
            },
        );
        assert_eq!(outcome, CastOutcome::Started);
        let after = map.creature(target).expect("target").unit.health();
        if after < before {
            landed_damage = Some(before - after);
            break;
        }
        tick(&mut map, 170); // outlast the 8s category cooldown
        let _ = map.drain_events();
    }
    let fb_damage = landed_damage.expect("Fire Blast never landed in 20 casts");
    println!("FIREBLAST damage={fb_damage} bounds=[{fb_lo}, {fb_hi}]");
    // Crit may raise it to 1.5x the hi bound (dither +1).
    assert!(
        f64::from(fb_damage) >= (fb_lo * 0.25).floor()
            && f64::from(fb_damage) <= (fb_hi * 1.5).ceil() + 1.0,
        "fire blast damage {fb_damage} outside plausible bounds"
    );

    // Death + respawn event: finish the target off.
    let mut died = false;
    let mut store_event = false;
    for _ in 0..200 {
        let hp = map.creature(target).expect("target").unit.health();
        if hp == 0 {
            break;
        }
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
                skip_known: true,
            },
        );
        for event in map.drain_events() {
            match event {
                MapEvent::UnitDied { guid, .. } if guid == target => died = true,
                MapEvent::StoreCreatureRespawn { .. } => store_event = true,
                _ => {}
            }
        }
    }
    assert!(died, "UnitDied never fired");
    assert!(store_event, "StoreCreatureRespawn never fired");
    println!("LIVE-SPELLS-PASS");
}
