//! Live kill-reputation evidence on the dev bench
//! (reputation-on-kill slice): a Fire Blast kill of a
//! `creature_onkill_reputation` creature grants the
//! fork `CalculateReputationGain` (KILL arm) amount into
//! the deferred reputation row, emits one batched
//! `SMSG_SET_FACTION_STANDING` plus `SMSG_SET_FACTION_VISIBLE`
//! when newly visible, and the save dirty set carries the
//! rewarded row. Read-only against the DB
//! (`diff_reputation` is asserted in memory, never saved).

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
                res_sickness_spell_id: uint(row, 9),
                team_id: uint(row, 5),
            },
        );
    }
    dbc
}

#[derive(Clone, Copy)]
struct RepOracle {
    faction: u32,
    repvalue: i32,
    max_cap: u32,
    creature_rate: Option<f32>,
}

fn gray_level(pl_level: u32) -> u32 {
    if pl_level <= 5 {
        0
    } else if pl_level <= 39 {
        pl_level - 5 - pl_level / 10
    } else {
        pl_level - 1 - pl_level / 5
    }
}

fn rank_of(standing: i32) -> u32 {
    const POINTS: [i32; 8] = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];
    let mut limit = 43000;
    for rank in (0..8u32).rev() {
        limit -= POINTS[rank as usize];
        if standing >= limit {
            return rank;
        }
    }
    0
}

/// Independent KILL-arm oracle over raw SQL columns
/// (never the Rust loaders/stores under test):
/// percent 100 (the fixture casts aura-free), gray arm
/// gated on a non-1.0 low-level rate, faction
/// creature-rate multiplier, deterministic round-up.
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
fn expected_gain(
    repvalue: i32,
    victim_level: u32,
    pl_level: u32,
    creature_rate: Option<f32>,
) -> i32 {
    let mut percent = 100.0f32;
    let lowlevel = 1.0f32;
    if lowlevel != 1.0 && victim_level <= gray_level(pl_level) {
        percent *= lowlevel;
    }
    if percent <= 0.0 {
        return 0;
    }
    if let Some(rate) = creature_rate {
        if rate <= 0.0 {
            return 0;
        }
        percent *= rate;
    }
    ((1.0 * repvalue as f32 * percent / 100.0) + 0.5).floor() as i32
}

fn dist2d(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
#[allow(clippy::too_many_lines)] // one end-to-end scenario
fn bench_kill_reputation() {
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
    let (stores, oracles, spawn_rows) = world
        .call(move |conn| {
            use mysql::prelude::Queryable;
            // Schemas are unverified statically: show them
            // first so a missing/renamed column fails here,
            // naming the exact table.
            for table in [
                "creature_onkill_reputation",
                "reputation_reward_rate",
                "reputation_spillover_template",
                "faction",
            ] {
                let ddl: Vec<(String, String)> = conn
                    .query(format!("SHOW CREATE TABLE `{table}`"))
                    .map_err(mangos_db::DbError::from)?;
                for (name, create) in &ddl {
                    println!("SCHEMA {name}: {create}");
                }
            }
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
            let factions = mangos_world_data::reputations::load_factions(conn)?;
            let mut rep_report = LoadReport::default();
            let reputation_kills = mangos_world_data::reputations::load_reputation_kill_stores(
                conn,
                10,
                &creature_templates,
                &factions,
                &mut rep_report,
            )?;
            assert!(
                reputation_kills.on_kill_len() > 0,
                "bench has no creature_onkill_reputation rows"
            );
            let mut report = LoadReport::default();
            let waypoints = load_waypoints(conn, &mut report)?;
            let mut spell_report = LoadReport::default();
            let spell_entries = load_spell_templates(conn, 5875, &mut spell_report)?;
            // Oracle columns per on-kill entry, single
            // non-team arm only (arm 1, no team awards, no
            // team dependence, no second faction).
            let rows: Vec<(u32, u32, i32, u32, Option<f32>)> = conn
                .query(
                    "SELECT k.creature_id, k.RewOnKillRepFaction1, k.RewOnKillRepValue1, \
                            k.MaxStanding1, r.creature_rate \
                     FROM creature_onkill_reputation k \
                     LEFT JOIN reputation_reward_rate r \
                       ON r.faction = k.RewOnKillRepFaction1 \
                     WHERE k.patch = (SELECT max(patch) FROM creature_onkill_reputation t2 \
                                      WHERE t2.creature_id = k.creature_id AND patch <= 10) \
                       AND k.RewOnKillRepFaction1 != 0 AND k.RewOnKillRepValue1 > 0 \
                       AND k.RewOnKillRepFaction2 = 0 \
                       AND k.TeamDependent = 0 \
                       AND k.IsTeamAward1 = 0 AND k.IsTeamAward2 = 0",
                )
                .map_err(mangos_db::DbError::from)?;
            let mut oracles: HashMap<u32, RepOracle> = HashMap::new();
            for (entry, faction, repvalue, max_cap, creature_rate) in rows {
                oracles.insert(
                    entry,
                    RepOracle {
                        faction,
                        repvalue,
                        max_cap,
                        creature_rate,
                    },
                );
            }
            // Map-0 spawn points for the candidate entries,
            // so the fixture can walk to one.
            let spawn_rows: Vec<(u32, f32, f32, f32, u32, u32)> = conn
                .query(
                    "SELECT k.creature_id, c.position_x, c.position_y, c.position_z, \
                            t.level_min, t.level_max \
                     FROM creature_onkill_reputation k \
                     JOIN creature_template t ON t.entry = k.creature_id \
                     JOIN creature c ON c.id = k.creature_id AND c.map = 0 \
                     WHERE k.patch = (SELECT max(patch) FROM creature_onkill_reputation t2 \
                                      WHERE t2.creature_id = k.creature_id AND patch <= 10) \
                       AND k.RewOnKillRepFaction1 != 0 AND k.RewOnKillRepValue1 > 0 \
                       AND k.RewOnKillRepFaction2 = 0 \
                       AND k.TeamDependent = 0 \
                       AND k.IsTeamAward1 = 0 AND k.IsTeamAward2 = 0",
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
                    factions,
                    reputation_kills,
                    gossip: mangos_world_data::gossip_menus::GossipStores::default(),
                    areas: mangos_world_data::areas::AreaStore::default(),
                    exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(
                    ),
                    pets: mangos_world_data::pet_data::PetStores::default(),
                    next_pet_number: std::sync::atomic::AtomicU32::new(1),
                    next_item_guid: std::sync::atomic::AtomicU32::new(1),
                    vendor_items: mangos_world_data::vendor_items::VendorItemStores::default(),
                    durability: mangos_world_data::durability::DurabilityTables::default(),
                    bank_bag_slot_prices: Vec::new(),
                    stable_slot_prices: Vec::new(),
                    trainer_spells: mangos_world_data::trainer_spells::TrainerStores::default(),
                    taxi: mangos_world_data::taxi::TaxiStore::default(),
                    script_waypoints:
                        mangos_world_data::script_waypoints::ScriptWaypointStore::default(),
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
                    creature_spells: mangos_world_data::creature_spells::CreatureSpellsMap::new(),
                    scripts: mangos_scripts_engine::ScriptStores::default(),
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
                oracles,
                spawn_rows,
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
        rest_rates: mangos_entities::player::RestRates::default(),
        skills: None,
        item_prototypes: None,
        quest_templates: None,
        taxi: None,
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
    // Aura-free fixture: the oracle's percent stays 100.
    assert!(
        map.player(player_guid)
            .expect("player")
            .unit
            .auras
            .is_empty(),
        "fixture has auras; oracle assumes none"
    );
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

    // Walk to a candidate spawn: the fixture's saved
    // spot has no reputation creatures, so pick the
    // nearest map-0 spawn of a non-gray candidate entry.
    let gray = gray_level(pl_level);
    let player_home = map
        .player(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let mut hunts: Vec<(f32, f32, f32)> = spawn_rows
        .into_iter()
        .filter(|(entry, _, _, _, level_min, _)| oracles.contains_key(entry) && *level_min > gray)
        .map(|(_, x, y, z, _, _)| (x, y, z))
        .collect();
    hunts.sort_by(|a, b| {
        let da = (a.0 - player_home.x).powi(2) + (a.1 - player_home.y).powi(2);
        let db = (b.0 - player_home.x).powi(2) + (b.1 - player_home.y).powi(2);
        da.total_cmp(&db)
    });
    let (hx, hy, hz) = hunts
        .into_iter()
        .next()
        .expect("a candidate spawn on map 0");
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

    // Nearest visible creature with a single-arm on-kill
    // entry whose oracle gain is positive and whose
    // current rank passes the max cap.
    let visible = map.visible_to(player_guid).cloned().expect("visible");
    let player_pos = map
        .player_mut(player_guid)
        .expect("player")
        .unit
        .world
        .position();
    let mut candidates: Vec<(ObjectGuid, RepOracle, i32, u32)> = visible
        .iter()
        .copied()
        .filter_map(|g| {
            let c = map.creature(g)?;
            if c.unit.level() <= gray {
                return None;
            }
            let oracle = oracles.get(&c.info.entry).copied()?;
            let gain = expected_gain(
                oracle.repvalue,
                c.unit.level(),
                pl_level,
                oracle.creature_rate,
            );
            if gain <= 0 {
                return None;
            }
            let faction = stores.factions.get(oracle.faction)?;
            if faction.reputation_index < 0 {
                return None;
            }
            let rep_list_id = u32::try_from(faction.reputation_index).ok()?;
            Some((g, oracle, gain, rep_list_id))
        })
        .collect();
    candidates.sort_by(|a, b| {
        let pa = map
            .creature(a.0)
            .map(|c| c.unit.world.position())
            .unwrap_or_default();
        let pb = map
            .creature(b.0)
            .map(|c| c.unit.world.position())
            .unwrap_or_default();
        dist2d(player_pos, pa).total_cmp(&dist2d(player_pos, pb))
    });
    let (target, oracle, gain, rep_list_id) = candidates
        .into_iter()
        .next()
        .expect("a reputation-granting creature in view");
    // The cap gate uses the live baseline row, exactly
    // like the runtime under test.
    let baseline: Vec<mangos_db::character_load::ReputationRow> = map
        .player(player_guid)
        .expect("player")
        .deferred
        .reputation
        .clone();
    let baseline_row = baseline.iter().find(|row| row.faction == oracle.faction);
    let baseline_standing = baseline_row.map_or(0, |row| row.standing);
    // Base reputation and default visibility come from
    // the same shared projection the runtime uses.
    let faction_entry = stores.factions.get(oracle.faction).expect("faction");
    let live_player = map.player(player_guid).expect("player");
    let race_mask = mangos_world_data::reputations::race_mask(live_player.unit.race());
    let class_mask = mangos_world_data::reputations::class_mask(live_player.unit.class());
    let base = faction_entry
        .index_fit_to(race_mask, class_mask)
        .map_or(0, |index| faction_entry.base_values[index]);
    let (pre_flags, _) = mangos_world_data::reputations::runtime_state(
        &faction_entry,
        race_mask,
        class_mask,
        baseline_row,
    );
    let was_visible = pre_flags & mangos_world_data::reputations::FACTION_FLAG_VISIBLE != 0;
    assert!(
        rank_of(base + baseline_standing) <= oracle.max_cap,
        "fixture already capped for faction {}",
        oracle.faction
    );
    assert!(gain > 0, "oracle gain must be positive");

    // Walk up and kill it with Fire Blast.
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
    map.creature_mut(target).expect("target").unit.set_health(1);
    let mut died = false;
    let mut standing_packet: Option<Vec<u8>> = None;
    let mut saw_visible = false;
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
                    triggered_by_spell: 0,
                    triggered_by_aura_spell: 0,
                },
            );
        }
        tick(&mut map, 2);
        for event in map.drain_events() {
            match event {
                MapEvent::UnitDied { guid, .. } if guid == target => died = true,
                MapEvent::Packet { to, packet } if to == player_guid => match packet.opcode() {
                    opcode_id::SMSG_SET_FACTION_STANDING => {
                        standing_packet = Some(packet.body().to_vec());
                    }
                    opcode_id::SMSG_SET_FACTION_VISIBLE => {
                        let id = u32::from_le_bytes(packet.body().try_into().expect("visible"));
                        if id == rep_list_id {
                            saw_visible = true;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if died && standing_packet.is_some() {
            break;
        }
    }
    assert!(died, "target never died");
    let expected_standing = baseline_standing + gain;
    assert!(
        (-42000..=42999).contains(&expected_standing),
        "oracle gain must not clamp"
    );

    // The deferred row carries the oracle delta.
    let after = map.player(player_guid).expect("player");
    let row = after
        .deferred
        .reputation
        .iter()
        .find(|row| row.faction == oracle.faction)
        .expect("rewarded faction row");
    assert_eq!(row.standing, expected_standing, "deferred delta vs oracle");
    // In-memory save assert only: the dirty set carries
    // the rewarded row (never written to the DB here).
    let dirty = mangos_db::character_save::diff_reputation(&baseline, &after.deferred.reputation);
    assert!(
        dirty
            .iter()
            .any(|row| row.faction == oracle.faction && row.standing == expected_standing),
        "save dirty set misses the rewarded faction"
    );

    // One batched STANDING packet addressed to the
    // killer carries the new standing; VISIBLE fires
    // only for newly-visible factions.
    let body = standing_packet.expect("SMSG_SET_FACTION_STANDING never emitted");
    let count = u32::from_le_bytes(body[0..4].try_into().expect("count"));
    assert!(count >= 1, "standing packet carries no factions");
    let mut found = false;
    for slot in 0..count {
        let base = 4 + (slot as usize) * 8;
        let id = u32::from_le_bytes(body[base..base + 4].try_into().expect("id"));
        let standing = i32::from_le_bytes(body[base + 4..base + 8].try_into().expect("standing"));
        if id == rep_list_id {
            assert_eq!(standing, expected_standing, "standing bytes vs oracle");
            found = true;
        }
    }
    assert!(found, "standing packet misses faction {rep_list_id}");
    assert_eq!(
        saw_visible, !was_visible,
        "VISIBLE iff the faction was not already visible"
    );
    println!(
        "LIVE-REPUTATION-KILL-PASS faction={} gain={gain} standing={expected_standing}",
        oracle.faction
    );
}
