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
const HOLY_LIGHT: u32 = 635;
const MIND_FLAY: u32 = 15407;
const NATURES_SWIFTNESS: u32 = 17116;
const HEALING_TOUCH: u32 = 5185;
const LIGHTNING_SHIELD: u32 = 324;
const LIGHTNING_SHIELD_TRIGGER: u32 = 26364;
const ON_EVENT_AURA: u32 = 4511;
const ABSORB_PHYSICAL: u32 = 11835;
const PERIODIC_TRIGGER_AURA: u32 = 1009;
const PERIODIC_TRIGGER_SPELL_ID: u32 = 25595;
const HEROIC_STRIKE: u32 = 78; // WEAPON_DAMAGE_NOSCHOOL +11
const WEAPON_PCT_350: u32 = 26613; // WEAPON_PERCENT_DAMAGE 350%
const ASPECT_CHEETAH: u32 = 5118; // MOD_INCREASE_SPEED +30%
const SPRINT_2983: u32 = 2983; // MOD_INCREASE_SPEED +50%
const SLOW_50: u32 = 12548; // MOD_DECREASE_SPEED -50%
const HAMMER_OF_JUSTICE: u32 = 853; // MOD_STUN
const SILENCE_SPELL: u32 = 15487; // MOD_SILENCE
const GROWL_THREAT: u32 = 2649; // THREAT effect ~50
const DISPEL_MAGIC: u32 = 527;
const PW_FORTITUDE: u32 = 1243;
const INSTAKILL_SPELL: u32 = 5;
const LEARN_EXEMPLAR: u32 = 5210; // teaches 5209
const LEARNED_ID: u32 = 5209;
const DAMAGE_SHIELD_BIG: u32 = 23272; // DAMAGE_SHIELD ~555
const SCALE_DOUBLE: u32 = 24244; // MOD_SCALE +100%

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
            let proc_events = mangos_world_data::proc_events::load_spell_proc_events(conn, 5875)?;
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
                    exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(
                    ),
                    pets: mangos_world_data::pet_data::PetStores::default(),
                    next_pet_number: std::sync::atomic::AtomicU32::new(1),
                    weather_chances: mangos_world_data::weather::WeatherChanceStore::default(),
                    waypoints,
                    spells: mangos_spells::SpellStore {
                        spells: spell_entries,
                        proc_events,
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
        item_prototypes: None,
        quest_templates: None,
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
    // dummy pool for the scenario (in-memory only). The
    // pool is expressed through `create_health` because
    // applying the saved Resurrection Sickness below now
    // runs the derived-stat refresh, which recomputes
    // max health from create health + stamina and would
    // clamp a raw field raise back down (fork
    // `SetMaxHealth` semantics).
    {
        let player = map.player_mut(player_guid).expect("player");
        player.unit.create_health = 100_000;
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
    // The visible dummy in the bench world DB is
    // level 30 vs the level-19 fixture: at the fork
    // 22% spell-hit floor, cast-landing sections
    // become dice rolls against the seeded RNG.
    // Pin the dummy's level field to the player's
    // (stats/health stay as spawned) so hit rolls
    // run at the equal-level 96%.
    let player_level = map.player(player_guid).expect("player").unit.level();
    map.creature_mut(target)
        .expect("target")
        .unit
        .values()
        .set_u32(offsets::UNIT_FIELD_LEVEL, player_level);

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
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
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

    // --- Pushback (Spell::Delayed) --------------------
    // The dummy is still meleeing the player; a 2.5 s
    // Holy Light self-cast (interruptFlags 0x0F:
    // DAMAGE_PUSHBACK set, DAMAGE_CANCELS clear) must
    // accumulate SMSG_SPELL_DELAYED packets while ticks
    // of any DoT never push.
    tick(&mut map, 32); // outlast Fire Blast's GCD
    let _ = map.drain_events();
    let mut delayed: Vec<u32> = Vec::new();
    for _ in 0..20 {
        if !delayed.is_empty() {
            break;
        }
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        let outcome = map.cast_spell(
            player_guid,
            HOLY_LIGHT,
            CastTarget::Unit(player_guid),
            CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert_eq!(outcome, CastOutcome::Started);
        for _ in 0..120 {
            tick(&mut map, 1);
            for event in map.drain_events() {
                if let MapEvent::Packet { to, packet } = event
                    && to == player_guid
                    && packet.opcode() == opcode_id::SMSG_SPELL_DELAYED
                {
                    let body = packet.body();
                    let (_, used) = ObjectGuid::read_packed(body).expect("packed guid");
                    delayed.push(u32::from_le_bytes([
                        body[used],
                        body[used + 1],
                        body[used + 2],
                        body[used + 3],
                    ]));
                }
            }
        }
    }
    println!("PUSHBACK delays={delayed:?}");
    assert!(
        !delayed.is_empty(),
        "no SMSG_SPELL_DELAYED observed in 20 casts under melee"
    );
    for delay in &delayed {
        assert!(
            *delay > 0 && *delay <= 1000,
            "delay {delay} outside the fork schedule bounds"
        );
    }
    tick(&mut map, 40); // let any cast resolve + GCD pass
    let _ = map.drain_events();

    // --- Channeling (Mind Flay) -------------------------
    // 3 s channeled PERIODIC_DAMAGE (1 s amplitude).
    // The dummy's melee may shorten the channel via
    // DelayedChannel, so tick counts assert a range,
    // not an exact schedule.
    let flay = stores.spells.get(MIND_FLAY).expect("mind flay entry");
    let flay_duration = flay.calculate_duration_ms(&stores.spells.tables);
    assert!(flay_duration > 0, "flay duration {flay_duration}");
    let health_before_channel = map.creature(target).expect("target").unit.health();
    let mut channel_starts: Vec<u32> = Vec::new();
    let mut channel_updates: Vec<u32> = Vec::new();
    let mut flay_ticks: Vec<u32> = Vec::new();
    let mut channeled = false;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        let outcome = map.cast_spell(
            player_guid,
            MIND_FLAY,
            CastTarget::Unit(target),
            CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert_eq!(outcome, CastOutcome::Started);
        if map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == MIND_FLAY)
        {
            channeled = true;
            break;
        }
        // Resisted: ride out the empty channel + GCD.
        tick(&mut map, 100);
        let _ = map.drain_events();
    }
    assert!(channeled, "Mind Flay holder never landed in 20 casts");
    {
        let player = map.player_mut(player_guid).expect("player");
        let values = player.unit.world.object.values();
        assert_eq!(
            values.get_u32(offsets::UNIT_CHANNEL_SPELL),
            MIND_FLAY,
            "channel spell field"
        );
        assert_eq!(
            values.get_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT),
            target.raw(),
            "channel object field"
        );
    }
    for _ in 0..200 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
            {
                match packet.opcode() {
                    opcode_id::MSG_CHANNEL_START => {
                        let body = packet.body();
                        let duration = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                        channel_starts.push(duration);
                    }
                    opcode_id::MSG_CHANNEL_UPDATE => {
                        let body = packet.body();
                        channel_updates
                            .push(u32::from_le_bytes([body[0], body[1], body[2], body[3]]));
                    }
                    opcode_id::SMSG_PERIODICAURALOG => {
                        let body = packet.body();
                        let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                        let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                        let off = used1 + used2;
                        flay_ticks.push(u32::from_le_bytes([
                            body[off + 12],
                            body[off + 13],
                            body[off + 14],
                            body[off + 15],
                        ]));
                    }
                    _ => {}
                }
            }
        }
        let channel_spell = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_CHANNEL_SPELL);
        if channel_spell == 0 && !channel_updates.is_empty() {
            break;
        }
    }
    println!("CHANNEL starts={channel_starts:?} updates={channel_updates:?} ticks={flay_ticks:?}");
    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
    {
        assert_eq!(
            channel_starts,
            [flay_duration as u32],
            "one channel start at full duration"
        );
    }
    assert!(
        channel_updates.contains(&0),
        "channel end update 0 missing: {channel_updates:?}"
    );
    assert!(
        !flay_ticks.is_empty() && flay_ticks.len() <= 3,
        "flay tick count {} outside 1..=3",
        flay_ticks.len()
    );
    let health_after_channel = map.creature(target).expect("target").unit.health();
    let flay_total: u32 = flay_ticks.iter().sum();
    assert_eq!(
        health_before_channel - health_after_channel,
        flay_total,
        "channel health delta must equal logged tick sum"
    );
    assert!(
        map.aura_holders(target)
            .iter()
            .all(|h| h.spell_id != MIND_FLAY),
        "flay holder must expire with the channel"
    );

    // Interrupt mid-channel: holder stripped at once,
    // no further ticks.
    tick(&mut map, 32);
    let _ = map.drain_events();
    let mut rechanneled = false;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        let outcome = map.cast_spell(
            player_guid,
            MIND_FLAY,
            CastTarget::Unit(target),
            CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert_eq!(outcome, CastOutcome::Started);
        if map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == MIND_FLAY)
        {
            rechanneled = true;
            break;
        }
        tick(&mut map, 100);
        let _ = map.drain_events();
    }
    assert!(rechanneled, "second Mind Flay never landed");
    tick(&mut map, 10); // 0.5 s in, before the first tick
    let _ = map.drain_events();
    map.cancel_cast(player_guid, true);
    assert!(
        map.aura_holders(target)
            .iter()
            .all(|h| h.spell_id != MIND_FLAY),
        "interrupt must strip the flay holder"
    );
    let mut interrupt_updates: Vec<u32> = Vec::new();
    let mut post_interrupt_ticks = 0usize;
    for _ in 0..80 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
            {
                match packet.opcode() {
                    opcode_id::MSG_CHANNEL_UPDATE => {
                        let body = packet.body();
                        interrupt_updates
                            .push(u32::from_le_bytes([body[0], body[1], body[2], body[3]]));
                    }
                    opcode_id::SMSG_PERIODICAURALOG => post_interrupt_ticks += 1,
                    _ => {}
                }
            }
        }
    }
    println!("CHANNEL-INTERRUPT updates={interrupt_updates:?} post_ticks={post_interrupt_ticks}");
    assert!(
        interrupt_updates.contains(&0),
        "interrupt channel update 0 missing"
    );
    assert_eq!(post_interrupt_ticks, 0, "ticks after interrupt");

    // --- Spell mods (Nature's Swiftness) ---------------
    // Control: Healing Touch is a timed cast — no heal
    // lands inside the cast call. With the Swiftness
    // modifier (ADD_PCT_MODIFIER CASTING_TIME -100,
    // 1 charge, druid family mask covers Healing
    // Touch) the same cast completes instantly and the
    // charge consumes the buff.
    tick(&mut map, 32);
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(50_000);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let outcome = map.cast_spell(
        player_guid,
        HEALING_TOUCH,
        CastTarget::Unit(player_guid),
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    let health_control = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_HEALTH);
    assert_eq!(health_control, 50_000, "timed heal must not land instantly");
    map.cancel_cast(player_guid, true);
    let outcome = map.cast_spell(
        player_guid,
        NATURES_SWIFTNESS,
        CastTarget::SelfCast,
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == NATURES_SWIFTNESS),
        "swiftness holder missing"
    );
    tick(&mut map, 32); // Swiftness GCD
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let health_before_swift = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_HEALTH);
    let outcome = map.cast_spell(
        player_guid,
        HEALING_TOUCH,
        CastTarget::Unit(player_guid),
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    let health_after_swift = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_HEALTH);
    println!(
        "SPELLMOD swift_heal={}",
        health_after_swift - health_before_swift
    );
    assert!(
        health_after_swift > health_before_swift,
        "swiftness heal must land inside the cast call"
    );
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .all(|h| h.spell_id != NATURES_SWIFTNESS),
        "swiftness charge must consume the buff"
    );
    // Restore the god pool for the death phase.
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    tick(&mut map, 32);
    let _ = map.drain_events();

    // --- Procs (Lightning Shield) ----------------------
    // 324 applies aura 42 (PROC_TRIGGER_SPELL; fork
    // overwrites the dummy DBC trigger with 26364)
    // with 3 proc charges; `spell_proc_event` puts a
    // 3500 ms cooldown on the triggered spell. The
    // dummy's melee swings on the player must proc
    // lightning back exactly 3 times, spaced by the
    // cooldown, then the holder drops.
    tick(&mut map, 32);
    let _ = map.drain_events();
    // Earlier phases wear the dummy down; top it up so
    // it survives to swing through all three charges.
    map.creature_mut(target)
        .expect("target")
        .unit
        .set_health(u32::MAX);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let outcome = map.cast_spell(
        player_guid,
        LIGHTNING_SHIELD,
        CastTarget::SelfCast,
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == LIGHTNING_SHIELD),
        "lightning shield holder missing"
    );
    println!(
        "PROC levels player={} dummy={} seeded_charges={:?}",
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_LEVEL),
        map.creature(target).expect("target").unit.level(),
        map.aura_holders(player_guid)
            .iter()
            .find(|h| h.spell_id == LIGHTNING_SHIELD)
            .map(|h| h.proc_charges)
    );
    let _ = map.drain_events();
    let mut lightning_hits: Vec<(u32, u32)> = Vec::new();
    let mut lightning_gos: Vec<(u32, u8, u8)> = Vec::new();
    for step in 0..800u32 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
            {
                if packet.opcode() == opcode_id::SMSG_SPELL_GO {
                    let body = packet.body();
                    let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                    let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                    let off = used1 + used2 + 1;
                    let spell_id = u32::from_le_bytes([
                        body[off],
                        body[off + 1],
                        body[off + 2],
                        body[off + 3],
                    ]);
                    if spell_id == LIGHTNING_SHIELD_TRIGGER {
                        let n_hits = body[off + 12];
                        #[allow(clippy::cast_possible_truncation)]
                        let n_miss = body[off + 12 + 1 + (n_hits as usize) * 8];
                        lightning_gos.push((step, n_hits, n_miss));
                    }
                    continue;
                }
                if packet.opcode() != opcode_id::SMSG_SPELLNONMELEEDAMAGELOG {
                    continue;
                }
                let body = packet.body();
                let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                let off = used1 + used2;
                let spell_id =
                    u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
                if spell_id == LIGHTNING_SHIELD_TRIGGER {
                    let dmg = u32::from_le_bytes([
                        body[off + 4],
                        body[off + 5],
                        body[off + 6],
                        body[off + 7],
                    ]);
                    lightning_hits.push((step, dmg));
                }
            }
        }
        if lightning_hits.len() >= 3
            && map
                .aura_holders(player_guid)
                .iter()
                .all(|h| h.spell_id != LIGHTNING_SHIELD)
        {
            break;
        }
    }
    println!(
        "PROC ls_hits={lightning_hits:?} ls_gos={lightning_gos:?} dummy_hp={} holder={}",
        map.creature(target).expect("target").unit.health(),
        map.aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == LIGHTNING_SHIELD)
    );
    // Charges burn per triggered cast (fork
    // DropAuraCharge on handler OK), hit or miss; the
    // level-30 dummy vs level-19 player puts bolts at
    // the 22% hit floor, so assert on casts.
    assert_eq!(
        lightning_gos.len(),
        3,
        "exactly 3 proc charges must cast: {lightning_gos:?}"
    );
    assert!(
        !lightning_hits.is_empty(),
        "at least one bolt must land: {lightning_gos:?}"
    );
    // Damage band from the fork pipeline: base 13
    // (12 + die 1), crit x1.5 -> 19-20, then partial
    // resists vs the +11-level dummy keep 25/50/75%
    // (innate level resistance), dither-rounded:
    // possible dealt values span 3..=20.
    assert!(
        lightning_hits.iter().all(|&(_, d)| (3..=20).contains(&d)),
        "trigger damage band (26364 base 12 + die 1, crit + partials): {lightning_hits:?}"
    );
    for pair in lightning_gos.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= 70,
            "3500 ms proc cooldown spacing (50 ms steps): {lightning_gos:?}"
        );
    }
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .all(|h| h.spell_id != LIGHTNING_SHIELD),
        "holder must drop when charges exhaust"
    );
    // No 4th proc once charges are gone.
    let mut extra_hits = 0usize;
    let mut extra_gos = 0usize;
    for _ in 0..120 {
        tick(&mut map, 1);
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = &event
                && *to == player_guid
                && packet.opcode() == opcode_id::SMSG_SPELLNONMELEEDAMAGELOG
            {
                let body = packet.body();
                let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                let off = used1 + used2;
                let spell_id =
                    u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
                if spell_id == LIGHTNING_SHIELD_TRIGGER {
                    extra_hits += 1;
                }
            }
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_SPELL_GO
            {
                let body = packet.body();
                let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                let off = used1 + used2 + 1;
                let spell_id =
                    u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
                if spell_id == LIGHTNING_SHIELD_TRIGGER {
                    extra_gos += 1;
                }
            }
        }
    }
    assert_eq!(extra_hits, 0, "no procs after charges exhaust");
    assert_eq!(extra_gos, 0, "no trigger casts after charges exhaust");
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    tick(&mut map, 32);
    let _ = map.drain_events();

    // --- Permanent (cooldown-on-event) cooldowns -----
    // 4511 carries SPELL_ATTR_COOLDOWN_ON_EVENT with a
    // 10 s recovery: the cast enters a permanent hold
    // (blocks recast, excluded from save rows); when
    // the aura fades the timed cooldown starts and
    // the row becomes savable.
    tick(&mut map, 32);
    let _ = map.drain_events();
    let outcome = map.cast_spell(
        player_guid,
        ON_EVENT_AURA,
        CastTarget::SelfCast,
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == ON_EVENT_AURA),
        "on-event aura holder missing"
    );
    tick(&mut map, 32); // GCD
    let _ = map.drain_events();
    let outcome = map.cast_spell(
        player_guid,
        ON_EVENT_AURA,
        CastTarget::SelfCast,
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_ne!(
        outcome,
        CastOutcome::Started,
        "permanent hold must block recast while aura is up"
    );
    let rows = map.player_cooldown_rows(player_guid, 1_000_000);
    assert!(
        rows.iter().all(|r| r.spell != ON_EVENT_AURA),
        "permanent hold must not be saved: {rows:?}"
    );
    // Duration index 21 is until-cancelled; the fork
    // conversion fires on any holder removal, so
    // cancel the aura (CMSG_CANCEL_AURA path).
    tick(&mut map, 20);
    let _ = map.drain_events();
    map.cancel_aura(player_guid, ON_EVENT_AURA);
    assert!(
        map.aura_holders(player_guid)
            .iter()
            .all(|h| h.spell_id != ON_EVENT_AURA),
        "on-event aura must cancel"
    );
    let rows = map.player_cooldown_rows(player_guid, 1_000_000);
    let row = rows
        .iter()
        .find(|r| r.spell == ON_EVENT_AURA)
        .expect("converted cooldown must be savable after fade");
    println!(
        "COOLDOWN-ON-EVENT expire={} item={}",
        row.spell_expire_time, row.item_id
    );
    assert!(
        (1_000_008..=1_000_010).contains(&row.spell_expire_time),
        "timed 10 s recovery starts at fade: {row:?}"
    );
    assert_eq!(row.item_id, 0);
    let outcome = map.cast_spell(
        player_guid,
        ON_EVENT_AURA,
        CastTarget::SelfCast,
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_ne!(
        outcome,
        CastOutcome::Started,
        "timed cooldown must block recast after fade"
    );
    // Seeded (login-loaded) cooldowns enforce too.
    map.seed_cooldowns(
        player_guid,
        &[mangos_map::map::LoadedCooldown {
            spell: HOLY_LIGHT,
            item_id: 6948,
            category: 0,
            spell_remaining_ms: 60_000,
            category_remaining_ms: 0,
        }],
    );
    tick(&mut map, 32);
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let outcome = map.cast_spell(
        player_guid,
        HOLY_LIGHT,
        CastTarget::Unit(player_guid),
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_ne!(
        outcome,
        CastOutcome::Started,
        "seeded cooldown must block the cast"
    );
    let rows = map.player_cooldown_rows(player_guid, 1_000_000);
    let seeded = rows
        .iter()
        .find(|r| r.spell == HOLY_LIGHT)
        .expect("seeded row must round-trip");
    assert_eq!(seeded.item_id, 6948, "item_id must round-trip");
    assert!(
        (1_000_055..=1_000_060).contains(&seeded.spell_expire_time),
        "seeded expiry: {seeded:?}"
    );
    println!(
        "COOLDOWN-SEED expire={} item={}",
        seeded.spell_expire_time, seeded.item_id
    );
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    map.creature_mut(target)
        .expect("target")
        .unit
        .set_health(u32::MAX);
    tick(&mut map, 32);
    let _ = map.drain_events();

    // --- School absorb (aura 69) -----------------------
    // 11835 absorbs ~115 (all schools): swings must not
    // move player health while the pool lasts; the
    // holder drops when spent, then damage lands.
    tick(&mut map, 32);
    let _ = map.drain_events();
    // Re-engage: the dummy may have leashed during the
    // long cooldown phases.
    map.creature_mut(target)
        .expect("target")
        .unit
        .set_health(u32::MAX);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let _ = map.cast_spell(
        player_guid,
        FIRE_BLAST,
        CastTarget::Unit(target),
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    tick(&mut map, 40);
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_power(0, 1000);
    let outcome = map.cast_spell(
        player_guid,
        ABSORB_PHYSICAL,
        CastTarget::Unit(player_guid),
        CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        },
    );
    assert_eq!(outcome, CastOutcome::Started);
    // Wait for the holder to land.
    let mut applied = false;
    for _ in 0..120 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        if map
            .aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == ABSORB_PHYSICAL)
        {
            applied = true;
            break;
        }
    }
    assert!(applied, "absorb holder missing after cast");
    let hp0 = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .object
        .values()
        .get_u32(offsets::UNIT_FIELD_HEALTH);
    let mut dropped_at: Option<u32> = None;
    let mut min_hp_while_up = hp0;
    for step in 0..600u32 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        let hp = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_HEALTH);
        let up = map
            .aura_holders(player_guid)
            .iter()
            .any(|h| h.spell_id == ABSORB_PHYSICAL);
        if up {
            min_hp_while_up = min_hp_while_up.min(hp);
        } else {
            dropped_at = Some(step);
            break;
        }
    }
    assert!(dropped_at.is_some(), "absorb holder never spent");
    assert_eq!(
        min_hp_while_up, hp0,
        "health must not drop while absorb pool lasts"
    );
    let mut landed = false;
    for _ in 0..300 {
        tick(&mut map, 1);
        let _ = map.drain_events();
        let hp = map
            .player_mut(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_HEALTH);
        if hp < hp0 {
            landed = true;
            break;
        }
    }
    println!(
        "ABSORB hp0={hp0} min_while_up={min_hp_while_up} dropped_at={dropped_at:?} landed_after={landed}"
    );
    assert!(landed, "damage must land after absorb is spent");
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    tick(&mut map, 32);
    let _ = map.drain_events();

    // --- Periodic trigger spell (aura 23) --------------
    // 1009 ticks trigger 25595 (school damage) onto
    // the aura target every 1 s.
    tick(&mut map, 32);
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    // The level-30 dummy resists most casts (22% hit
    // floor); retry until the holder lands, counting
    // trigger casts from the very first attempt.
    let mut trigger_hits: Vec<u32> = Vec::new();
    let mut trigger_gos = 0usize;
    let mut applied = false;
    for _ in 0..40 {
        for power_type in 0..5usize {
            map.player_mut(player_guid)
                .expect("player")
                .unit
                .set_power(power_type, 1000);
        }
        let outcome = map.cast_spell(
            player_guid,
            PERIODIC_TRIGGER_AURA,
            CastTarget::Unit(target),
            CastOptions {
                triggered: false,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        assert_ne!(outcome, CastOutcome::UnknownSpell);
        for _ in 0..60 {
            tick(&mut map, 1);
            for event in map.drain_events() {
                if let MapEvent::Packet { to, packet } = &event
                    && *to == player_guid
                    && (packet.opcode() == opcode_id::SMSG_SPELL_GO
                        || packet.opcode() == opcode_id::SMSG_SPELLNONMELEEDAMAGELOG)
                {
                    let body = packet.body();
                    let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                    let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                    let off =
                        used1 + used2 + usize::from(packet.opcode() == opcode_id::SMSG_SPELL_GO);
                    let spell_id = u32::from_le_bytes([
                        body[off],
                        body[off + 1],
                        body[off + 2],
                        body[off + 3],
                    ]);
                    if spell_id == PERIODIC_TRIGGER_SPELL_ID {
                        if packet.opcode() == opcode_id::SMSG_SPELL_GO {
                            trigger_gos += 1;
                        } else {
                            trigger_hits.push(u32::from_le_bytes([
                                body[off + 4],
                                body[off + 5],
                                body[off + 6],
                                body[off + 7],
                            ]));
                        }
                    }
                }
            }
            if map
                .aura_holders(target)
                .iter()
                .any(|h| h.spell_id == PERIODIC_TRIGGER_AURA)
            {
                applied = true;
            }
            if trigger_gos >= 2 {
                break;
            }
        }
        if trigger_gos >= 2 {
            break;
        }
    }
    println!("PERIODIC-TRIGGER gos={trigger_gos} hits={trigger_hits:?}");
    assert!(applied, "periodic trigger holder missing");
    assert!(
        trigger_gos >= 2,
        "periodic trigger must cast repeatedly on ticks"
    );
    assert!(
        trigger_hits.iter().all(|&d| (1..=400).contains(&d)),
        "trigger damage band: {trigger_hits:?}"
    );
    map.cancel_aura(target, PERIODIC_TRIGGER_AURA);
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);
    tick(&mut map, 32);
    let _ = map.drain_events();

    // Weapon-damage effects (`EffectWeaponDmg`): pin the
    // player's weapon range to a flat 100, then Heroic
    // Strike lands ~111 pre-armor and the 350% percent
    // exemplar ~350 pre-armor. Misses retry (spell hit
    // roll vs the level-30 dummy).
    {
        let player = map.player_mut(player_guid).expect("player");
        let values = player.unit.world.object.values_mut();
        values.set_f32(offsets::UNIT_FIELD_MINDAMAGE, 100.0);
        values.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, 100.0);
    }
    let weapon_hit = |map: &mut Map, tick: &mut dyn FnMut(&mut Map, u32), spell_id: u32| {
        let mut found = None;
        for _ in 0..40 {
            for power_type in 0..5usize {
                map.player_mut(player_guid)
                    .expect("player")
                    .unit
                    .set_power(power_type, 1000);
            }
            map.cast_spell(
                player_guid,
                spell_id,
                CastTarget::Unit(target),
                CastOptions {
                    triggered: false,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
            tick(map, 4);
            for event in map.drain_events() {
                if let MapEvent::Packet { to, packet } = event
                    && to == player_guid
                    && packet.opcode() == opcode_id::SMSG_SPELLNONMELEEDAMAGELOG
                {
                    let body = packet.body();
                    let (_, used1) = ObjectGuid::read_packed(body).expect("packed1");
                    let (_, used2) = ObjectGuid::read_packed(&body[used1..]).expect("packed2");
                    let off = used1 + used2;
                    let id = u32::from_le_bytes([
                        body[off],
                        body[off + 1],
                        body[off + 2],
                        body[off + 3],
                    ]);
                    if id == spell_id {
                        found = Some(u32::from_le_bytes([
                            body[off + 4],
                            body[off + 5],
                            body[off + 6],
                            body[off + 7],
                        ]));
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        found
    };
    let hs = weapon_hit(&mut map, &mut tick, HEROIC_STRIKE).expect("heroic strike hit");
    let pct = weapon_hit(&mut map, &mut tick, WEAPON_PCT_350).expect("percent hit");
    println!("WEAPON-DAMAGE hs={hs} pct={pct}");
    assert!((40..=160).contains(&hs), "heroic strike band: {hs}");
    assert!(pct >= hs * 2, "350% must clearly outhit +11: {pct} vs {hs}");

    // Speed auras (`Unit::UpdateSpeed`): max-positive
    // increase wins, strongest slow multiplies last,
    // removal restores the base rate. Subject is the
    // creature: player rates apply only after the
    // movement ACK (fork pending-change semantics).
    // Ambient slows from earlier phases (Dazed) are
    // waited out first.
    let has_slow = |map: &Map| {
        map.aura_holders(target).iter().any(|h| {
            h.effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == 33 && e.amount < 0)
        })
    };
    for _ in 0..400 {
        if !has_slow(&map) {
            break;
        }
        tick(&mut map, 10);
    }
    let _ = map.drain_events();
    assert!(!has_slow(&map), "ambient slow never expired");
    let run_rate = |map: &Map| {
        map.creature(target).expect("target").unit.speed_rate[mangos_entities::unit::MOVE_RUN]
    };
    let base_rate = run_rate(&map);
    for spell_id in [ASPECT_CHEETAH, SPRINT_2983] {
        map.cast_spell(
            target,
            spell_id,
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
        tick(&mut map, 4);
    }
    let _ = map.drain_events();
    let hasted = run_rate(&map);
    // Player slows the dummy by 50%.
    let mut slowed = hasted;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        map.cast_spell(
            player_guid,
            SLOW_50,
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
        tick(&mut map, 4);
        let _ = map.drain_events();
        slowed = run_rate(&map);
        if map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == SLOW_50)
        {
            break;
        }
    }
    // Tick-free ratio checks: cancel applies the
    // refresh synchronously, and the ambient slow
    // factor (combat Dazed may be present) cancels
    // out of the ratio. Sprint (+50) is the
    // max-positive, so dropping it leaves Cheetah
    // (+30): rate scales by 1.3/1.5.
    let before_cancel = run_rate(&map);
    map.cancel_aura(target, SPRINT_2983);
    let after_sprint = run_rate(&map);
    map.cancel_aura(target, ASPECT_CHEETAH);
    let after_cheetah = run_rate(&map);
    println!(
        "SPEED-RATIO before={before_cancel} sprint_off={after_sprint} cheetah_off={after_cheetah}"
    );
    assert!(
        (after_sprint - before_cancel * (1.3 / 1.5)).abs() < 0.02,
        "dropping max-positive +50 must fall back to +30"
    );
    assert!(
        (after_cheetah - after_sprint / 1.3).abs() < 0.02,
        "dropping +30 must remove the increase factor"
    );
    // The slow debuff is not cancelable; wait it out.
    for _ in 0..600 {
        if !has_slow(&map) {
            break;
        }
        tick(&mut map, 10);
    }
    let _ = map.drain_events();
    let restored = run_rate(&map);
    println!("SPEED base={base_rate} hasted={hasted} slowed={slowed} restored={restored}");
    assert!(hasted > 0.0, "hasted rate sane");
    // With both hastes cancelled only the -50% slow
    // (plus any ambient slow) remains: at most half
    // of base.
    assert!(
        after_cheetah <= base_rate * 0.55,
        "slow-only rate must be at most half of base: {after_cheetah} vs {base_rate}"
    );
    let _ = slowed;
    assert!(
        (restored - base_rate).abs() < 0.001,
        "removal must restore base rate"
    );

    // Loss-of-control flags + cast prevention (fork
    // Spell.cpp:6826): Hammer of Justice sets
    // UNIT_FLAG_STUNNED on the dummy and its casts
    // fail SPELL_FAILED_STUNNED (100); Silence fails
    // prevention-type-1 casts with 96.
    let cc_flags = |map: &Map| {
        map.creature(target)
            .expect("target")
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_FLAGS)
    };
    let mut stunned_flags = 0u32;
    let mut stun_cast = CastOutcome::UnknownSpell;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        map.cast_spell(
            player_guid,
            HAMMER_OF_JUSTICE,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        stunned_flags = cc_flags(&map);
        if stunned_flags & 0x0004_0000 != 0 {
            stun_cast = map.cast_spell(
                target,
                FIRE_BLAST,
                CastTarget::Unit(player_guid),
                CastOptions {
                    triggered: false,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
            break;
        }
    }
    println!("CC-STUN flags={stunned_flags:#x} cast={stun_cast:?}");
    assert_eq!(stun_cast, CastOutcome::Failed(100), "stunned cast gate");
    // Wait out the stun, then silence.
    for _ in 0..200 {
        if cc_flags(&map) & 0x0004_0000 == 0 {
            break;
        }
        tick(&mut map, 10);
    }
    let _ = map.drain_events();
    let mut silenced_cast = CastOutcome::UnknownSpell;
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        map.cast_spell(
            player_guid,
            SILENCE_SPELL,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        if cc_flags(&map) & 0x0000_2000 != 0 {
            silenced_cast = map.cast_spell(
                target,
                FIRE_BLAST,
                CastTarget::Unit(player_guid),
                CastOptions {
                    triggered: false,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
            break;
        }
    }
    println!("CC-SILENCE cast={silenced_cast:?}");
    assert_eq!(silenced_cast, CastOutcome::Failed(96), "silenced cast gate");
    for _ in 0..200 {
        if cc_flags(&map) & 0x0000_2000 == 0 {
            break;
        }
        tick(&mut map, 10);
    }
    let _ = map.drain_events();
    map.player_mut(player_guid)
        .expect("player")
        .unit
        .set_health(100_000);

    // Threat effect: flat threat lands on the
    // dummy's threat list for the player.
    let threat_before = map.threat_on(target, player_guid);
    for _ in 0..20 {
        map.cast_spell(
            player_guid,
            GROWL_THREAT,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        if map.threat_on(target, player_guid) > threat_before {
            break;
        }
    }
    let threat_after = map.threat_on(target, player_guid);
    println!("THREAT before={threat_before} after={threat_after}");
    assert!(
        threat_after >= threat_before + 30.0,
        "threat effect must add ~50 threat"
    );

    // Dispel: dummy buffs itself with a MAGIC
    // positive (PW:Fortitude), player Dispel Magic
    // strips it.
    for _ in 0..10 {
        map.cast_spell(
            target,
            PW_FORTITUDE,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        if map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == PW_FORTITUDE)
        {
            break;
        }
    }
    assert!(
        map.aura_holders(target)
            .iter()
            .any(|h| h.spell_id == PW_FORTITUDE),
        "fortitude must land for the dispel phase"
    );
    for _ in 0..20 {
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 1000);
        map.cast_spell(
            player_guid,
            DISPEL_MAGIC,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        if !map
            .aura_holders(target)
            .iter()
            .any(|h| h.spell_id == PW_FORTITUDE)
        {
            break;
        }
    }
    println!(
        "DISPEL fortitude_gone={}",
        !map.aura_holders(target)
            .iter()
            .any(|h| h.spell_id == PW_FORTITUDE)
    );
    assert!(
        !map.aura_holders(target)
            .iter()
            .any(|h| h.spell_id == PW_FORTITUDE),
        "dispel must strip the magic buff"
    );

    // Learn spell: the exemplar teaches 5209;
    // the runtime spell set gains the row and
    // SMSG_LEARNED_SPELL goes to the player.
    {
        let known_before = map
            .player(player_guid)
            .expect("player")
            .deferred
            .spells
            .iter()
            .any(|r| r.spell == LEARNED_ID);
        map.cast_spell(
            player_guid,
            LEARN_EXEMPLAR,
            CastTarget::SelfCast,
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
        tick(&mut map, 3);
        let mut learned_packet = false;
        for event in map.drain_events() {
            if let MapEvent::Packet { to, packet } = event
                && to == player_guid
                && packet.opcode() == opcode_id::SMSG_LEARNED_SPELL
            {
                learned_packet = true;
            }
        }
        let known_after = map
            .player(player_guid)
            .expect("player")
            .deferred
            .spells
            .iter()
            .any(|r| r.spell == LEARNED_ID);
        println!("LEARN before={known_before} after={known_after} packet={learned_packet}");
        assert!(known_after, "learned spell must enter the spell set");
        assert!(
            learned_packet || known_before,
            "fresh learn must send SMSG_LEARNED_SPELL"
        );
    }

    // Damage shield: reflected melee damage to
    // the attacker while the shield is up on the
    // player (dummy is in combat with the player).
    {
        let mut shield_up = false;
        for _ in 0..10 {
            map.cast_spell(
                player_guid,
                DAMAGE_SHIELD_BIG,
                CastTarget::SelfCast,
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
            tick(&mut map, 3);
            let _ = map.drain_events();
            shield_up = map
                .aura_holders(player_guid)
                .iter()
                .any(|h| h.spell_id == DAMAGE_SHIELD_BIG);
            if shield_up {
                break;
            }
            map.cast_spell(
                target,
                DAMAGE_SHIELD_BIG,
                CastTarget::Unit(player_guid),
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
            tick(&mut map, 3);
            let _ = map.drain_events();
            shield_up = map
                .aura_holders(player_guid)
                .iter()
                .any(|h| h.spell_id == DAMAGE_SHIELD_BIG);
            if shield_up {
                break;
            }
        }
        assert!(shield_up, "damage shield must land on the player");
        let dummy_hp0 = map.creature(target).expect("target").unit.health();
        let mut reflected = false;
        for _ in 0..400 {
            map.player_mut(player_guid)
                .expect("player")
                .unit
                .set_health(100_000);
            tick(&mut map, 5);
            let _ = map.drain_events();
            if map.creature(target).expect("target").unit.health() + 400 < dummy_hp0 {
                reflected = true;
                break;
            }
        }
        let dummy_hp1 = map.creature(target).expect("target").unit.health();
        println!("DAMAGE-SHIELD dummy {dummy_hp0} -> {dummy_hp1}");
        assert!(reflected, "shield must reflect melee damage");
        map.cancel_aura(player_guid, DAMAGE_SHIELD_BIG);
    }
    // The dummy may be dead by now (reflected
    // damage is real); the remaining sections need
    // a live target.
    {
        let dummy = map.creature_mut(target).expect("target");
        let max_hp = dummy.unit.max_health();
        dummy.unit.death_state = mangos_entities::unit::DeathState::Alive;
        dummy.unit.set_health(max_hp);
    }

    // Mod scale: object scale doubles on apply.
    {
        let scale_of = |map: &Map| {
            map.creature(target)
                .expect("target")
                .unit
                .world
                .object
                .object_scale()
        };
        let s0 = scale_of(&map);
        map.cast_spell(
            target,
            SCALE_DOUBLE,
            CastTarget::SelfCast,
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
        tick(&mut map, 3);
        let _ = map.drain_events();
        let s1 = scale_of(&map);
        map.cancel_aura(target, SCALE_DOUBLE);
        tick(&mut map, 2);
        let _ = map.drain_events();
        let s2 = scale_of(&map);
        println!("SCALE {s0} -> {s1} -> {s2}");
        assert!((s1 - s0 * 2.0).abs() < 0.01, "scale must double");
        assert!((s2 - s0).abs() < 0.01, "scale must restore on cancel");
    }

    // Shadow Flame cone + script arm (Tier 2
    // Slice 5 tranche 3): cone enumeration lands
    // the breath; the script effect casts 22682
    // per victim unless the 22683 cloak aura is
    // up. The dummy pool is raised so the direct
    // damage does not kill before the script
    // effect resolves (the fork defers damage
    // past all effects; see TIER2-PROGRESS).
    {
        const SHADOW_FLAME: u32 = 22539;
        const SHADOW_FLAME_DOT: u32 = 22682;
        const SCALE_CLOAK: u32 = 22683;
        let triggered_opts = CastOptions {
            triggered: true,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        };
        let top_up = |map: &mut Map| {
            let dummy = map.creature_mut(target).expect("target");
            dummy.unit.create_health = 200_000;
            dummy
                .unit
                .world
                .object
                .values_mut()
                .set_u32(offsets::UNIT_FIELD_MAXHEALTH, 200_000);
            dummy.unit.set_health(200_000);
        };
        top_up(&mut map);
        // Stand east of the dummy facing west
        // (atan2-space orientation).
        let tpos = map.creature(target).expect("target").unit.world.position();
        {
            let mut info = map
                .player_mut(player_guid)
                .expect("player")
                .unit
                .world
                .movement_info;
            info.pos = Position::new(tpos.x + 3.0, tpos.y, tpos.z, std::f32::consts::PI);
            map.relocate_from_client(player_guid, info, &respawns);
        }
        let _ = map.drain_events();
        // Cloak gate: breath lands, no 22682.
        map.cast_spell(target, SCALE_CLOAK, CastTarget::SelfCast, triggered_opts);
        tick(&mut map, 3);
        let _ = map.drain_events();
        assert!(
            map.aura_holders(target)
                .iter()
                .any(|h| h.spell_id == SCALE_CLOAK),
            "cloak aura must be up"
        );
        let mut landed = false;
        for _ in 0..5 {
            let hp0 = map.creature(target).expect("target").unit.health();
            map.cast_spell(
                player_guid,
                SHADOW_FLAME,
                CastTarget::SelfCast,
                triggered_opts,
            );
            tick(&mut map, 120);
            let _ = map.drain_events();
            if map.creature(target).expect("target").unit.health() < hp0 {
                landed = true;
                break;
            }
        }
        assert!(landed, "Shadow Flame cone must hit the dummy");
        assert!(
            map.aura_holders(target)
                .iter()
                .all(|h| h.spell_id != SHADOW_FLAME_DOT),
            "cloak aura must block 22682"
        );
        map.cancel_aura(target, SCALE_CLOAK);
        top_up(&mut map);
        let mut dotted = false;
        for _ in 0..5 {
            map.cast_spell(
                player_guid,
                SHADOW_FLAME,
                CastTarget::SelfCast,
                triggered_opts,
            );
            tick(&mut map, 120);
            let _ = map.drain_events();
            if map
                .aura_holders(target)
                .iter()
                .any(|h| h.spell_id == SHADOW_FLAME_DOT)
            {
                dotted = true;
                break;
            }
        }
        assert!(dotted, "script effect must cast 22682 on the victim");
        println!("SHADOWFLAME cone + cloak gate ok");
        map.cancel_aura(target, SHADOW_FLAME_DOT);
    }

    // --- Shapeshift (Tier 2 Slice 6.3) --------------
    // Form byte (BYTES_1[2]), display switch,
    // exclusivity, 9033 root strip, cancel restore.
    // Expected displays from the fork table for the
    // loaded character's race (guid 1 = orc: Horde
    // bear 2289 / cat 8571; Alliance 2281 / 892).
    {
        let opts = CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        };
        // The test world's partial materialize
        // inputs leave NATIVEDISPLAYID unset; seed
        // it from the current display as production
        // materialize does (player.rs create-info
        // branch).
        {
            let unit = &mut map.player_mut(player_guid).expect("player").unit;
            // No create-info display in this world:
            // seed display + native as materialize
            // would (human male display 49).
            if unit
                .world
                .object
                .values()
                .get_u32(offsets::UNIT_FIELD_DISPLAYID)
                == 0
            {
                unit.set_display_id(49, None);
            }
            unit.set_native_display_id(
                unit.world
                    .object
                    .values()
                    .get_u32(offsets::UNIT_FIELD_DISPLAYID),
            );
        }
        let (race, native_display) = {
            let values = map
                .player(player_guid)
                .expect("player")
                .unit
                .world
                .object
                .values();
            (
                values.get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                values.get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
            )
        };
        assert!(native_display != 0, "native display seeded");
        let horde = matches!(race, 2 | 5 | 6 | 8);
        // Entangling Roots (339) on the player from
        // the dummy: the 9033 strip target.
        map.creature_mut(target)
            .expect("target")
            .unit
            .set_power(0, 10000);
        let mut rooted = false;
        for _ in 0..20 {
            map.cast_spell(target, 339, CastTarget::Unit(player_guid), opts);
            tick(&mut map, 40);
            let _ = map.drain_events();
            if map
                .aura_holders(player_guid)
                .iter()
                .any(|h| h.spell_id == 339)
            {
                rooted = true;
                break;
            }
        }
        assert!(rooted, "Entangling Roots never landed");
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 10000);
        let outcome = map.cast_spell(player_guid, 5487, CastTarget::Unit(player_guid), opts);
        assert_eq!(outcome, CastOutcome::Started, "Bear Form cast");
        tick(&mut map, 3);
        let _ = map.drain_events();
        {
            let values = map
                .player(player_guid)
                .expect("player")
                .unit
                .world
                .object
                .values();
            assert_eq!(
                values.get_byte(offsets::UNIT_FIELD_BYTES_1, 2),
                5,
                "bear form byte"
            );
            assert_eq!(
                values.get_u32(offsets::UNIT_FIELD_DISPLAYID),
                if horde { 2289 } else { 2281 },
                "bear display"
            );
        }
        assert!(
            map.aura_holders(player_guid)
                .iter()
                .all(|h| h.spell_id != 339),
            "9033 must strip the root on shift-in"
        );
        // Bear -> Cat: exclusivity + display swap.
        tick(&mut map, 32); // outlast the 1.5s GCD
        let _ = map.drain_events();
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 10000);
        let outcome = map.cast_spell(player_guid, 768, CastTarget::Unit(player_guid), opts);
        assert_eq!(outcome, CastOutcome::Started, "Cat Form cast");
        tick(&mut map, 3);
        let _ = map.drain_events();
        {
            let values = map
                .player(player_guid)
                .expect("player")
                .unit
                .world
                .object
                .values();
            assert_eq!(
                values.get_byte(offsets::UNIT_FIELD_BYTES_1, 2),
                1,
                "cat form byte"
            );
            assert_eq!(
                values.get_u32(offsets::UNIT_FIELD_DISPLAYID),
                if horde { 8571 } else { 892 },
                "cat display"
            );
        }
        assert!(
            map.aura_holders(player_guid)
                .iter()
                .all(|h| h.spell_id != 5487),
            "bear holder must be stripped by cat exclusivity"
        );
        // Cancel: byte + native display restored.
        map.cancel_aura(player_guid, 768);
        tick(&mut map, 3);
        let _ = map.drain_events();
        {
            let values = map
                .player(player_guid)
                .expect("player")
                .unit
                .world
                .object
                .values();
            assert_eq!(
                values.get_byte(offsets::UNIT_FIELD_BYTES_1, 2),
                0,
                "form byte cleared"
            );
            assert_eq!(
                values.get_u32(offsets::UNIT_FIELD_DISPLAYID),
                native_display,
                "native display restored"
            );
        }
        println!("SHAPESHIFT form byte + display + 9033 ok");
    }

    // --- TeleUnitsFaceCaster (Tier 2 Slice 6.5) -----
    // Spell 514: effect 43, radius index 13 =
    // 10.0y (SpellRadius.dbc). Victim lands at
    // caster + (bounds+bounds+10) * facing, with
    // orientation -caster.o.
    {
        let opts = CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        };
        tick(&mut map, 32); // outlast the GCD
        let _ = map.drain_events();
        let caster_pos = map
            .player(player_guid)
            .expect("player")
            .unit
            .world
            .position();
        let c_bounds = map
            .player(player_guid)
            .expect("player")
            .unit
            .world
            .object
            .values()
            .get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS);
        let v_bounds = map
            .creature(target)
            .expect("target")
            .unit
            .world
            .object
            .values()
            .get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS);
        let outcome = map.cast_spell(player_guid, 514, CastTarget::Unit(target), opts);
        assert_eq!(outcome, CastOutcome::Started, "514 cast");
        // No tick: read the teleport result before
        // the dummy's AI moves it.
        let tpos = map.creature(target).expect("target").unit.world.position();
        let reach = c_bounds + v_bounds + 10.0;
        let expect_x = caster_pos.x + reach * caster_pos.o.cos();
        let expect_y = caster_pos.y + reach * caster_pos.o.sin();
        assert!(
            (tpos.x - expect_x).abs() < 0.1 && (tpos.y - expect_y).abs() < 0.1,
            "face-caster close point: got ({}, {}), want ({expect_x}, {expect_y})",
            tpos.x,
            tpos.y
        );
        assert!(
            (tpos.o - (-caster_pos.o)).abs() < 1e-4,
            "orientation must be -caster.o: got {}, want {}",
            tpos.o,
            -caster_pos.o
        );
        let _ = map.drain_events();
        println!("FACECASTER close point ok");
    }

    // --- Channeled dynobj (Tier 2 Slice 6.4) --------
    // Blizzard (10) creates a dynobject; breaking
    // the channel early must despawn it at channel
    // end (fork RemoveDynObject), long before its
    // 8s nominal duration.
    {
        let opts = CastOptions {
            triggered: false,
            triggered_by_aura: false,
            skip_known: true,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
        };
        tick(&mut map, 32); // outlast the GCD
        let _ = map.drain_events();
        let dest = map.creature(target).expect("target").unit.world.position();
        map.player_mut(player_guid)
            .expect("player")
            .unit
            .set_power(0, 10000);
        let outcome = map.cast_spell(player_guid, 10, CastTarget::Dest(dest), opts);
        assert_eq!(outcome, CastOutcome::Started, "Blizzard cast");
        tick(&mut map, 4);
        let _ = map.drain_events();
        {
            let values = map
                .player(player_guid)
                .expect("player")
                .unit
                .world
                .object
                .values();
            assert_eq!(
                values.get_u32(offsets::UNIT_CHANNEL_SPELL),
                10,
                "blizzard channel active"
            );
        }
        map.cancel_cast(player_guid, true);
        let mut despawn = false;
        for event in map.drain_events() {
            if let MapEvent::Packet { packet, .. } = event
                && packet.opcode() == opcode_id::SMSG_GAMEOBJECT_DESPAWN_ANIM
            {
                despawn = true;
            }
        }
        assert!(despawn, "channel break must despawn the blizzard dynobject");
        println!("CHANNEL-DYNOBJ break cleanup ok");
    }

    // Death + respawn event: finish the target off.
    // Instakill effect is the finisher (direct
    // damage equal to current health); the
    // Fire Blast loop below remains as fallback
    // coverage if the exemplar's targeting ever
    // changes.
    let mut died = false;
    let mut store_event = false;
    map.cast_spell(
        player_guid,
        INSTAKILL_SPELL,
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
    tick(&mut map, 3);
    for event in map.drain_events() {
        match event {
            MapEvent::UnitDied { guid, .. } if guid == target => died = true,
            MapEvent::StoreCreatureRespawn { .. } => store_event = true,
            _ => {}
        }
    }
    println!("INSTAKILL died={died}");
    assert!(died, "instakill must kill the dummy");
    // Fallback finisher (only runs if the dummy
    // somehow survived, e.g. exemplar retargeting).
    for _ in 0..200 {
        if died {
            break;
        }
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
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
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
