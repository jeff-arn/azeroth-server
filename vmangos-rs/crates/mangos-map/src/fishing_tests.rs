//! Fishing slice (`SPELL_EFFECT_TRANS_DOOR` 50
//! `FISHINGNODE` arm, bobber channel lifecycle,
//! fishing-node/hole use, fish loot, packets):
//! failing-first parity tests.
//!
//! Fork pins (`vmangos/src`): `SpellEffects.cpp:5538-5678`
//! (`EffectTransmitted`), `GameObject.cpp:344-365`
//! (bobber-ready splash), `:399-411` (expiry),
//! `:1589-1686` (`FISHINGNODE` use), `:1802-1811`
//! (`FISHINGHOLE` use), `:848-863` (`getFishLoot`),
//! `:1303-1312` (hole search), `ObjectMgr.cpp:10024-10063`
//! (base-skill loader), `Player.cpp:5394-5407`
//! (`UpdateFishingSkill`), `Player.cpp:7696-7793` +
//! `:8051-8068` (`SendLoot` GO arm + `FISHING` remap),
//! `LootHandler.cpp:498-507` (hole release),
//! `Spell.cpp:2945-2948` (fishing-spot target),
//! `GridNotifiers.h:702-727` (hole checker).

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
use mangos_guid::ObjectGuid;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::{cast_result, effect, target};
use mangos_spells::entry::{DurationEntry, RangeEntry, SpellEffectData, SpellEntry};
use mangos_world_data::gameobject_template::{GameObjectInfo, GameObjectTemplates};
use mangos_world_data::loot::{LootStore, LootStoreItem};

use crate::WorldRng;
use crate::gameobjects::{GO_LOOT_JUST_DEACTIVATED, GO_LOOT_READY};
use crate::map::{Map, MapConfig, MapEvent};
use crate::spells::{CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;

/// Fishing cast stand-in (effect 50, misc = bobber
/// entry, fishing-spot target, channeled like the
/// fork spell).
const FISHING_SPELL: u32 = 8080;
/// Bobber template entry.
const BOBBER_ENTRY: u32 = 8081;
/// Fishing-hole template entry.
const HOLE_ENTRY: u32 = 8082;
/// Fish item (linen-like stackable).
const FISH_ITEM: u32 = 8083;
/// Hole loot entry.
const HOLE_LOOT: u32 = 900;
/// Subzone fished in the subzone-first fixtures.
const SUBZONE: u32 = 34;
/// Zone of the fixtures.
const ZONE: u32 = 12;

fn test_infos() -> mangos_world_data::player_info::PlayerInfos {
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };
    PlayerInfos::from_maps_for_tests(
        60,
        HashMap::from([(
            (1, 1),
            PlayerCreateInfo {
                map_id: 0,
                area_id: 14,
                position_x: -8948.0,
                position_y: -130.0,
                position_z: 83.5,
                orientation: 0.0,
                display_id_m: 51,
                display_id_f: 52,
            },
        )]),
        HashMap::from([(
            1,
            vec![
                PlayerClassLevelInfo {
                    basehealth: 100,
                    basemana: 100
                };
                60
            ],
        )]),
        HashMap::from([((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
        vec![400; 61],
    )
}

fn test_load(guid: u32) -> CharacterLoad {
    CharacterLoad {
        row: CharacterRow {
            guid,
            account: 42,
            name: format!("Fisher{guid}"),
            race: 1,
            class: 1,
            gender: 0,
            level: 60,
            xp: 0,
            money: 0,
            skin: 1,
            face: 2,
            hair_style: 3,
            hair_color: 4,
            facial_hair: 5,
            bank_bag_slots: 0,
            character_flags: 0,
            position_x: -8948.0,
            position_y: -130.0,
            position_z: 83.5,
            map: 0,
            orientation: 0.0,
            known_taxi_mask: String::new(),
            played_time_total: 60,
            played_time_level: 60,
            rest_bonus: 0.0,
            logout_time: 0,
            reset_talents_multiplier: 0,
            reset_talents_time: 0,
            transport_guid: 0,
            transport_x: 0.0,
            transport_y: 0.0,
            transport_z: 0.0,
            transport_o: 0.0,
            extra_flags: 0,
            stable_slots: 0,
            death_expire_time: 0,
            current_taxi_path: String::new(),
            honor_rank_points: 0.0,
            honor_highest_rank: 0,
            honor_standing: 0,
            honor_last_week_hk: 0,
            honor_last_week_cp: 0.0,
            honor_stored_hk: 0,
            honor_stored_dk: 0,
            watched_faction: -1,
            drunk: 0,
            health: 100,
            power1: 100,
            power2: 0,
            power3: 0,
            power4: 0,
            power5: 0,
            explored_zones: String::new(),
            ammo_id: 0,
            action_bars: 0,
            world_phase_mask: 0,
            create_time: 0,
            instance: 0,
            zone: 0,
        },
        group_id: None,
        bound_instances: vec![],
        auras: vec![],
        spells: vec![],
        quest_status: vec![],
        honor_cp: vec![],
        reputation: vec![],
        inventory: vec![],
        item_loot: vec![],
        actions: vec![],
        social: vec![],
        homebind: None,
        spell_cooldowns: vec![],
        guild: None,
        bg_data: None,
        account_data: vec![],
        skills: vec![],
        mails: vec![],
        mailed_items: vec![],
        forgotten_skills: vec![],
        pets: Vec::new(),
    }
}

fn materialize(guid: u32, infos: &mangos_world_data::player_info::PlayerInfos) -> Player {
    let inputs = MaterializeInputs {
        infos,
        race: None,
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
        rest_rates: RestRates::default(),
        skills: None,
        item_prototypes: None,
        quest_templates: None,
        taxi: None,
    };
    Player::materialize(test_load(guid), &inputs).expect("materialize")
}

fn bobber_info() -> GameObjectInfo {
    GameObjectInfo {
        id: BOBBER_ENTRY,
        go_type: mangos_world_data::gameobject_template::TYPE_FISHINGNODE,
        display_id: 49,
        name: "Bobbertest".into(),
        faction: 35,
        size: 1.0,
        ..Default::default()
    }
}

fn hole_info(min: i32, max: i32) -> GameObjectInfo {
    let mut data = [0i32; 24];
    data[0] = 20; // radius
    data[1] = i32::try_from(HOLE_LOOT).expect("fixture id fits"); // lootId
    data[2] = min; // minSuccessOpens
    data[3] = max; // maxSuccessOpens
    GameObjectInfo {
        id: HOLE_ENTRY,
        go_type: mangos_world_data::gameobject_template::TYPE_FISHINGHOLE,
        display_id: 49,
        name: "Holetest".into(),
        faction: 35,
        size: 1.0,
        data,
        ..Default::default()
    }
}

fn fish_item() -> mangos_world_data::item_prototype::ItemPrototype {
    mangos_world_data::item_prototype::ItemPrototype {
        item_id: FISH_ITEM,
        display_id: 100,
        stackable: 20,
        buy_count: 1,
        ..Default::default()
    }
}

fn fish_row(item: u32) -> LootStoreItem {
    LootStoreItem {
        itemid: item,
        chance: 100.0,
        mincount_or_ref: 1,
        group: 0,
        needs_quest: false,
        maxcount: 1,
        condition_id: 0,
    }
}

/// Area rows resolving `(zone, subzone)` without grid
/// tiles (`area_flag_at` falls back to the linked-area
/// flag 0, so the single row carries both).
fn area_rows() -> Vec<mangos_world_data::areas::AreaEntry> {
    vec![mangos_world_data::areas::AreaEntry {
        id: SUBZONE,
        map_id: 0,
        zone_id: ZONE,
        explore_flag: 0,
        flags: 0,
        area_level: 10,
        team: 7,
        liquid_type: 0,
    }]
}

fn fishing_stores(
    fishing_rows: Vec<(u32, LootStoreItem)>,
    gameobject_rows: Vec<(u32, LootStoreItem)>,
) -> WorldStores {
    let mut stores = WorldStores::empty();
    stores.player_infos = test_infos();
    stores.item_prototypes.insert_for_tests(fish_item());
    stores.gameobject_templates =
        GameObjectTemplates::from_entries_for_tests(vec![bobber_info(), hole_info(2, 5)]);
    stores.loot.fishing = LootStore::from_fixtures_for_tests(fishing_rows, true);
    // Pool loot lives in `gameobject_loot_template`
    // (fork `Player.cpp:7764`); bobber fish loot in
    // `fishing_loot_template`.
    stores.loot.gameobject = LootStore::from_fixtures_for_tests(gameobject_rows, true);
    stores.areas = mangos_world_data::areas::AreaStore::from_entries_for_tests(area_rows());
    stores.fishing_base_skills =
        mangos_world_data::fishing::FishingBaseSkills::from_entries_for_tests(vec![
            (SUBZONE, 100),
            (ZONE, 200),
        ]);
    stores.spells.tables.ranges.insert(
        9001,
        RangeEntry {
            min: 0.0,
            max: 100.0,
        },
    );
    stores.spells.tables.durations.insert(
        7,
        DurationEntry {
            duration: 60_000,
            duration_per_level: 0,
            max_duration: 60_000,
        },
    );
    stores.spells.spells.insert(
        FISHING_SPELL,
        SpellEntry {
            id: FISHING_SPELL,
            range_index: 9001,
            duration_index: 7,
            attributes_ex: 0x0000_0004,  // channeled
            attributes_ex3: 0x0004_0000, // SPELL_ATTR_EX3_ALWAYS_HIT
            effects: [
                SpellEffectData {
                    effect: effect::TRANS_DOOR,
                    misc_value: i32::try_from(BOBBER_ENTRY).expect("fixture id fits"),
                    implicit_target_a: target::LOCATION_CASTER_FISHING_SPOT,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores
}

struct Fixture {
    map: Map,
    caster: ObjectGuid,
}

fn setup(skill: u32, stores: WorldStores, seed: u64) -> Fixture {
    let infos = test_infos();
    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::new(stores),
        WorldRng::from_seed(seed),
    );
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    let caster = materialize(7, &infos).unit.guid();
    map.add_player(materialize(7, &infos), &respawns, 1_800_000_000, 0);
    map.player_mut(caster)
        .expect("caster")
        .deferred
        .skills
        .push(mangos_db::character_load::SkillRow {
            skill: 356, // SKILL_FISHING
            value: skill,
            max: 300,
        });
    map.player_mut(caster)
        .expect("caster")
        .learn_spell(FISHING_SPELL);
    Fixture { map, caster }
}

fn packets_to(events: &[MapEvent], to: ObjectGuid, op: u16) -> Vec<Vec<u8>> {
    events
        .iter()
        .filter_map(|e| match e {
            MapEvent::Packet { to: t, packet } if *t == to && packet.opcode() == op => {
                Some(packet.body().to_vec())
            }
            _ => None,
        })
        .collect()
}

fn caster_pos(fx: &Fixture) -> mangos_movement::movement_info::Position {
    fx.map
        .player(fx.caster)
        .expect("caster")
        .unit
        .world
        .position()
}

/// A channel the tests pretend a real cast opened
/// (no grid water exists in unit tests, so the cast
/// site can only fail; the bobber is placed directly
/// and the channel faked to prove the Use/expiry
/// `FinishSpell` link ends it).
fn start_fake_channel(fx: &mut Fixture) {
    use crate::spells::ActiveCast;
    fx.map.casts.insert(
        fx.caster,
        ActiveCast {
            spell_id: FISHING_SPELL,
            cast_counter: 0,
            target: CastTarget::SelfCast,
            remaining_ms: 60_000,
            total_ms: 60_000,
            triggered: false,
            triggered_by_aura: false,
            delay_count: 0,
            channeled: true,
            channeling: true,
            channel_cost_ms: 1000,
            applied_mods: Vec::new(),
            proc_attacker_flags: 0,
            proc_victim_flags: 0,
            can_trigger_procs: false,
            from_proc: false,
            cast_item: None,
            custom_base_points: [None; 3],
            triggered_by_aura_spell: 0,
            triggered_by_spell: 0,
            script_effect_pick: None,
            position: caster_pos(fx),
            original_caster: None,
        },
    );
}

/// Direct bobber placement (what the effect arm does
/// after the water gate): dynamic GO + owner + spell
/// + level + armed despawn timer, `NOT_READY`.
fn place_bobber(fx: &mut Fixture, secs: u32, now: u64) -> ObjectGuid {
    let pos = caster_pos(fx);
    let guid = fx
        .map
        .create_dynamic_gameobject(BOBBER_ENTRY, pos, [0.0; 4])
        .expect("bobber");
    fx.map.unit_add_gameobject(fx.caster, guid);
    fx.map.go_set_spell_id(guid, FISHING_SPELL);
    let level = fx.map.player(fx.caster).expect("caster").unit.level();
    let _ = fx
        .map
        .object_set_field(guid, u32::from(offsets::GAMEOBJECT_LEVEL), level);
    if let Some(go) = fx.map.gameobjects.get_mut(&guid) {
        go.set_respawn_time(secs, now);
    }
    start_fake_channel(fx);
    guid
}

/// Dry-land cast: no grid liquid exists, so the effect
/// 50 water gate must fail the cast fork-exactly.
#[test]
fn fishing_cast_without_water_fails_not_fishable() {
    let stores = fishing_stores(vec![(SUBZONE, fish_row(FISH_ITEM))], vec![]);
    let mut fx = setup(300, stores, 3);
    let outcome = fx.map.cast_spell(
        fx.caster,
        FISHING_SPELL,
        CastTarget::Dest(caster_pos(&fx)),
        CastOptions::default(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let events = fx.map.drain_events();
    let results = packets_to(&events, fx.caster, opcode_id::SMSG_CAST_RESULT);
    assert_eq!(results.len(), 1, "one cast result");
    let mut expected = FISHING_SPELL.to_le_bytes().to_vec();
    expected.push(cast_result::FAILED_NOT_FISHABLE);
    assert_eq!(results[0], expected, "NOT_FISHABLE for dry land");
    assert!(
        fx.map.gameobjects.is_empty(),
        "no bobber on failed water gate"
    );
    assert!(!fx.map.casts.contains_key(&fx.caster), "no channel");
}

/// Bobber tick arms (fork `GameObject.cpp:344-365` +
/// `:399-411`): splash readies the node (ACTIVE +
/// sound + custom anim + forced update), expiry hooks
/// the channel, notifies, and retires it.
#[test]
fn bobber_splash_readies_and_expiry_hooks_channel() {
    let stores = fishing_stores(vec![(SUBZONE, fish_row(FISH_ITEM))], vec![]);
    let mut fx = setup(300, stores, 5);
    let now = 1_800_000_000u64;
    let bobber = place_bobber(&mut fx, 30, now);
    // Pre-splash tick: still NOT_READY, no packets.
    fx.map.tick(1000, 1_000, now + 10);
    assert_eq!(
        fx.map.go_runtime.get(&bobber).map_or(255, |r| r.loot_state),
        0,
        "NOT_READY before splash"
    );
    // Splash tick (past respawn - 5).
    fx.map.tick(1000, 2_000, now + 26);
    assert_eq!(
        fx.map.go_runtime.get(&bobber).map_or(255, |r| r.loot_state),
        GO_LOOT_READY,
        "READY after splash"
    );
    assert_eq!(
        fx.map.object_get_field(bobber, offsets::GAMEOBJECT_STATE),
        0,
        "GO_STATE_ACTIVE splash anim"
    );
    let events = fx.map.drain_events();
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_GAMEOBJECT_CUSTOM_ANIM).len(),
        1,
        "splash custom anim"
    );
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_PLAY_OBJECT_SOUND).len(),
        1,
        "splash sound 3355"
    );
    // Expiry tick: NOT_HOOKED + channel end + retire
    // (the delete lands on the following tick, fork
    // `GO_JUST_DEACTIVATED` `Update` arm).
    fx.map.tick(1000, 3_000, now + 31);
    let events = fx.map.drain_events();
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_FISH_NOT_HOOKED).len(),
        1,
        "expiry not-hooked"
    );
    assert!(
        !fx.map.casts.contains_key(&fx.caster),
        "expiry ends the channel"
    );
    assert_eq!(
        fx.map.go_runtime.get(&bobber).map(|r| r.loot_state),
        Some(GO_LOOT_JUST_DEACTIVATED),
        "retired on expiry"
    );
    fx.map.tick(1000, 4_000, now + 32);
    assert!(
        !fx.map.gameobjects.contains_key(&bobber),
        "retired bobber deleted next tick"
    );
}

/// Node use, success path (fork `:1600-1686`): skill
/// beats the subzone base, the fish window opens on
/// wire byte 3 and the channel ends; a full take +
/// release retires the bobber. The skill is maxed so
/// the gain arm is a capped no-op here (gains ride
/// the dedicated test below).
#[test]
fn bobber_use_success_opens_fish_window() {
    let stores = fishing_stores(vec![(SUBZONE, fish_row(FISH_ITEM))], vec![]);
    let mut fx = setup(300, stores, 7);
    let now = 1_800_000_000u64;
    let bobber = place_bobber(&mut fx, 30, now);
    fx.map.tick(1000, 1_000, now + 26);
    assert_eq!(
        fx.map.go_runtime.get(&bobber).map(|r| r.loot_state),
        Some(GO_LOOT_READY)
    );
    fx.map.gameobject_use(fx.caster, bobber);
    let events = fx.map.drain_events();
    let responses = packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE);
    assert_eq!(responses.len(), 1, "one fish window");
    let view = &responses[0];
    assert_eq!(&view[0..8], &bobber.raw().to_le_bytes(), "bobber guid");
    assert_eq!(view[8], 3, "LOOT_FISHING wire byte");
    assert_eq!(
        u32::from_le_bytes(view[9..13].try_into().expect("gold")),
        0,
        "no money on fish loot"
    );
    assert_eq!(view[13], 1, "one fish item");
    assert!(
        !fx.map.casts.contains_key(&fx.caster),
        "catch ends the channel"
    );
    assert_eq!(
        fx.map.player_loot_guid.get(&fx.caster),
        Some(&bobber),
        "window registered"
    );
    let skill = fx
        .map
        .player(fx.caster)
        .expect("caster")
        .deferred
        .skills
        .iter()
        .find(|s| s.skill == 356)
        .map_or(0, |s| s.value);
    assert_eq!(skill, 300, "maxed skill stays capped");
    // Full take + release retires the bobber.
    fx.map.handle_autostore_loot_item(fx.caster, 0);
    fx.map.handle_loot_release(fx.caster);
    let events = fx.map.drain_events();
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RELEASE_RESPONSE).len(),
        1,
        "release ack"
    );
    fx.map.tick(1000, 2_000, now + 27);
    assert!(
        !fx.map.gameobjects.contains_key(&bobber),
        "fully-looted bobber deleted"
    );
}

/// Node use, fail path (fork `:1680-1687` with the
/// `SkillFail` defaults: loot/gain false, pool-rescue
/// true): the fish escapes even next to a hole, no
/// window opens, no skill moves, the bobber retires.
#[test]
fn bobber_use_fail_escapes_without_hole() {
    let stores = fishing_stores(vec![(SUBZONE, fish_row(FISH_ITEM))], vec![]);
    // Skill 1 vs subzone base 100: success impossible
    // (`skill >= zone` fails).
    let mut fx = setup(1, stores, 11);
    let now = 1_800_000_000u64;
    let bobber = place_bobber(&mut fx, 30, now);
    // A pool in range must NOT rescue the roll under
    // the `SkillFail.Possible.FishingPool` default
    // (fork `:1638-1645` skips the lookup on failure).
    let pos = caster_pos(&fx);
    let hole = fx
        .map
        .create_dynamic_gameobject(HOLE_ENTRY, pos, [0.0; 4])
        .expect("hole");
    fx.map.tick(1000, 1_000, now + 26);
    fx.map.gameobject_use(fx.caster, bobber);
    let events = fx.map.drain_events();
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_FISH_ESCAPED).len(),
        1,
        "escape on failed roll"
    );
    assert!(
        packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE).is_empty(),
        "no junk window (SkillFail.Loot.Fishing default false)"
    );
    assert!(
        !fx.map.casts.contains_key(&fx.caster),
        "escape ends the channel"
    );
    let skill = fx
        .map
        .player(fx.caster)
        .expect("caster")
        .deferred
        .skills
        .iter()
        .find(|s| s.skill == 356)
        .map_or(0, |s| s.value);
    assert_eq!(skill, 1, "no gain (SkillFail.Gain.Fishing default false)");
    fx.map.tick(1000, 2_000, now + 27);
    assert!(
        !fx.map.gameobjects.contains_key(&bobber),
        "escaped bobber deleted"
    );
    assert!(
        fx.map.gameobjects.contains_key(&hole),
        "unfished pool untouched"
    );
}

/// Success with a hole in range (fork `:1672-1678`):
/// the catch redirects to the pool window instead of
/// the bobber window, and the bobber retires.
#[test]
fn bobber_use_success_redirects_to_hole() {
    let stores = fishing_stores(vec![], vec![(HOLE_LOOT, fish_row(FISH_ITEM))]);
    // Skill 300 vs subzone base 100: success certain
    // (chance 205 beats any `irand(1, 100)`).
    let mut fx = setup(300, stores, 13);
    let now = 1_800_000_000u64;
    let bobber = place_bobber(&mut fx, 30, now);
    let pos = caster_pos(&fx);
    let hole = fx
        .map
        .create_dynamic_gameobject(HOLE_ENTRY, pos, [0.0; 4])
        .expect("hole");
    fx.map.tick(1000, 1_000, now + 26);
    fx.map.gameobject_use(fx.caster, bobber);
    let events = fx.map.drain_events();
    assert!(
        packets_to(&events, fx.caster, opcode_id::SMSG_FISH_ESCAPED).is_empty(),
        "success never escapes"
    );
    let responses = packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE);
    assert_eq!(responses.len(), 1, "one hole window");
    assert_eq!(&responses[0][0..8], &hole.raw().to_le_bytes(), "hole guid");
    assert_eq!(responses[0][8], 3, "LOOT_FISHING wire byte");
    assert_eq!(responses[0][13], 1, "hole loot item");
    assert_eq!(
        fx.map.player_loot_guid.get(&fx.caster),
        Some(&hole),
        "window registered on the hole"
    );
    fx.map.tick(1000, 2_000, now + 27);
    assert!(
        !fx.map.gameobjects.contains_key(&bobber),
        "redirected bobber deleted"
    );
    assert!(
        fx.map.gameobjects.contains_key(&hole),
        "hole survives (uses 0 < min 2)"
    );
}

/// `UpdateFishingSkill` (fork `Player.cpp:5394-5407`):
/// skill below 75 always gains one point (chance
/// 100), capped at max.
#[test]
fn fishing_skill_gain_below_75_always() {
    let stores = fishing_stores(vec![(SUBZONE, fish_row(FISH_ITEM))], vec![]);
    let mut fx = setup(60, stores, 29);
    fx.map.update_fishing_skill(fx.caster);
    let skill = fx
        .map
        .player(fx.caster)
        .expect("caster")
        .deferred
        .skills
        .iter()
        .find(|s| s.skill == 356)
        .map_or(0, |s| s.value);
    assert_eq!(skill, 61, "skill 60 always gains (chance 100)");
}

/// Hole release uses roll (fork `LootHandler.cpp:498-507`
/// and the `GO_JUST_DEACTIVATED` re-show): a
/// delay-lapped static pool depletes and re-arms after
/// its respawn delay; a below-min release re-arms at
/// once.
#[test]
fn hole_release_uses_roll() {
    // Depleting static-like hole: min = max = 1,
    // respawn delay 60 (simulating a DB static row).
    let mut stores = fishing_stores(vec![], vec![(HOLE_LOOT, fish_row(FISH_ITEM))]);
    stores.gameobject_templates =
        GameObjectTemplates::from_entries_for_tests(vec![bobber_info(), hole_info(1, 1)]);
    let mut fx = setup(300, stores, 17);
    let now = 1_800_000_000u64;
    let pos = caster_pos(&fx);
    let hole = fx
        .map
        .create_dynamic_gameobject(HOLE_ENTRY, pos, [0.0; 4])
        .expect("hole");
    if let Some(go) = fx.map.gameobjects.get_mut(&hole) {
        // Static stand-in: a spawn row + default flag
        // like `LoadFromDB`, so the release arms a
        // map-local respawn instead of deleting.
        go.data = Some(mangos_world_data::spawns::GameObjectSpawn::default());
        go.spawned_by_default = true;
        go.respawn_delay_time = 60;
    }
    // First sight flips the pool `READY` (fork
    // `GameObject.cpp:382-383`).
    fx.map.tick(1000, 1_000, now + 1);
    assert_eq!(
        fx.map.go_runtime.get(&hole).map(|r| r.loot_state),
        Some(GO_LOOT_READY),
        "pool readies on first tick"
    );
    fx.map.gameobject_use(fx.caster, hole);
    let events = fx.map.drain_events();
    assert_eq!(
        packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE).len(),
        1,
        "hole window"
    );
    fx.map.handle_autostore_loot_item(fx.caster, 0);
    fx.map.handle_loot_release(fx.caster);
    assert_eq!(
        fx.map.go_runtime.get(&hole).map(|r| r.loot_state),
        Some(GO_LOOT_JUST_DEACTIVATED),
        "uses 1 >= urand(1, 1) depletes"
    );
    // Delay lapses: the pool clears and re-shows.
    fx.map.tick(1000, 2_000, now + 61);
    assert_eq!(
        fx.map.go_runtime.get(&hole).map(|r| r.loot_state),
        Some(GO_LOOT_READY),
        "delay-lapped pool re-arms"
    );
    fx.map.gameobject_use(fx.caster, hole);
    assert_eq!(
        packets_to(
            &fx.map.drain_events(),
            fx.caster,
            opcode_id::SMSG_LOOT_RESPONSE
        )
        .len(),
        1,
        "re-armed pool re-opens"
    );

    // Mining-style hole: first full release re-arms
    // (uses 1 < min 2), so it re-opens.
    let stores = fishing_stores(vec![], vec![(HOLE_LOOT, fish_row(FISH_ITEM))]);
    let mut fx = setup(300, stores, 19);
    let pos = caster_pos(&fx);
    let hole = fx
        .map
        .create_dynamic_gameobject(HOLE_ENTRY, pos, [0.0; 4])
        .expect("hole");
    fx.map.tick(1000, 1_000, 1_800_000_001);
    fx.map.gameobject_use(fx.caster, hole);
    assert_eq!(
        packets_to(
            &fx.map.drain_events(),
            fx.caster,
            opcode_id::SMSG_LOOT_RESPONSE
        )
        .len(),
        1,
        "hole window"
    );
    fx.map.handle_autostore_loot_item(fx.caster, 0);
    fx.map.handle_loot_release(fx.caster);
    fx.map.gameobject_use(fx.caster, hole);
    assert_eq!(
        packets_to(
            &fx.map.drain_events(),
            fx.caster,
            opcode_id::SMSG_LOOT_RESPONSE
        )
        .len(),
        1,
        "re-armed hole re-opens after first full release"
    );
}

/// `getFishLoot` zone fallback (fork `:848-863`): no
/// subzone template falls back to the zone template.
#[test]
fn fish_loot_falls_back_to_zone_template() {
    // Store carries only the zone (12) template; the
    // bobber sits in subzone 34.
    let stores = fishing_stores(vec![(ZONE, fish_row(FISH_ITEM))], vec![]);
    let mut fx = setup(300, stores, 23);
    let now = 1_800_000_000u64;
    let bobber = place_bobber(&mut fx, 30, now);
    fx.map.tick(1000, 1_000, now + 26);
    fx.map.gameobject_use(fx.caster, bobber);
    let responses = packets_to(
        &fx.map.drain_events(),
        fx.caster,
        opcode_id::SMSG_LOOT_RESPONSE,
    );
    assert_eq!(responses.len(), 1, "zone-fallback window");
    assert_eq!(responses[0][8], 3, "LOOT_FISHING wire byte");
    assert_eq!(responses[0][13], 1, "zone template item");
}
