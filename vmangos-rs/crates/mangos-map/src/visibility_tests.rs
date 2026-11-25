//! `SetVisibility` / `InterruptNonMeleeSpells` parity slice:
//! failing-first maintained tests (no DB).
//!
//! Fork pins (`vmangos/src`): `Objects/UnitDefines.h:527-535`
//! (visibility values), `Objects/Unit.cpp:6495-6602`
//! (`IsVisibleForOrDetect`), `:6654-6660` (`SetVisibility`),
//! `Objects/SpellCaster.cpp:1783-1796`
//! (`InterruptNonMeleeSpells`), `Spells/Spell.cpp:3620-3697`
//! (`Spell::cancel`).

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
use mangos_entities::unit::UnitVisibility;
use mangos_guid::ObjectGuid;
use mangos_protocol::opcode_table::opcode_id;
use mangos_spells::defines::{cast_result, effect};
use mangos_spells::entry::{CastTimeEntry, DurationEntry, SpellEffectData, SpellEntry};
use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};

use crate::WorldRng;
use crate::map::{Map, MapConfig, MapEvent};
use crate::spells::{CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;
use crate::visibility::unit_visibility_allows;

/// Timed generic cast stand-in (3 s, zero cost,
/// self-target dummy).
const TIMED_SPELL: u32 = 9001;
/// Channeled cast stand-in (3 s channel, zero cost).
const CHANNEL_SPELL: u32 = 9002;
/// Instant cast stand-in (zero cost, self dummy).
const INSTANT_SPELL: u32 = 9003;
/// Creature template entry for the visibility target.
const TARGET_ENTRY: u32 = 9101;

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
            name: format!("Vis{guid}"),
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
            // Overlapping phase mask so the two fixture
            // players can see each other (`CanSeeInWorld`
            // is a mask AND).
            world_phase_mask: 0xFFFF_FFFF,
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

fn helper_stores() -> WorldStores {
    let mut stores = WorldStores::empty();
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![CreatureInfo {
        entry: TARGET_ENTRY,
        display_id: [100, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    }]);
    stores.display_addons =
        mangos_world_data::spawn_support::DisplayInfoAddons::from_creature_addons_for_tests(vec![
            mangos_world_data::spawn_support::CreatureDisplayInfoAddon {
                display_id: 100,
                ..Default::default()
            },
        ]);
    stores.spells.tables.cast_times.insert(
        1,
        CastTimeEntry {
            cast_time: 3000,
            cast_time_per_level: 0,
            min_cast_time: 0,
        },
    );
    let dummy = SpellEffectData {
        effect: effect::DUMMY,
        implicit_target_a: 1, // TARGET_UNIT_CASTER
        ..Default::default()
    };
    stores.spells.tables.durations.insert(
        1,
        DurationEntry {
            duration: 3000,
            duration_per_level: 0,
            max_duration: 3000,
        },
    );
    for (id, casting, channeled, duration) in [
        (TIMED_SPELL, 1, 0, 0),
        (CHANNEL_SPELL, 1, 0x44, 1),
        (INSTANT_SPELL, 0, 0, 0),
    ] {
        stores.spells.spells.insert(
            id,
            SpellEntry {
                id,
                casting_time_index: casting,
                attributes_ex: channeled,
                duration_index: duration,
                effects: [
                    dummy,
                    SpellEffectData::default(),
                    SpellEffectData::default(),
                ],
                ..Default::default()
            },
        );
    }
    stores
}

struct Fixture {
    map: Map,
    viewer: ObjectGuid,
    other: ObjectGuid,
}

fn setup(seed: u64) -> Fixture {
    let infos = test_infos();
    let stores = Arc::new(helper_stores());
    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(seed),
    );
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    let viewer = materialize(7, &infos).unit.guid();
    let other = materialize(8, &infos).unit.guid();
    map.add_player(materialize(7, &infos), &respawns, 1_800_000_000, 0);
    map.add_player(materialize(8, &infos), &respawns, 1_800_000_000, 0);
    let _ = map.drain_events();
    Fixture { map, viewer, other }
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

fn visible_to(map: &Map, viewer: ObjectGuid, target: ObjectGuid) -> bool {
    map.players
        .get(&viewer)
        .is_some_and(|e| e.visible.contains(&target))
}

fn summon_nearby(map: &mut Map, summoner: ObjectGuid) -> ObjectGuid {
    let pos = map.unit_position(summoner).expect("summoner positioned");
    map.summon_creature(TARGET_ENTRY, pos, summoner, 0, 0)
        .expect("summon target")
}

/// Fork `Unit.cpp:6495-6602` branch matrix: only `On`
/// passes the flag gate (GM bypass and stealth/invis
/// detect arms unmodeled — see `unit_visibility_allows`).
#[test]
fn visibility_flag_matrix() {
    assert!(unit_visibility_allows(UnitVisibility::On));
    assert!(!unit_visibility_allows(UnitVisibility::Off));
    assert!(!unit_visibility_allows(UnitVisibility::GroupStealth));
    assert!(!unit_visibility_allows(UnitVisibility::GroupInvisibility));
    assert!(!unit_visibility_allows(UnitVisibility::GroupNoDetect));
    assert!(!unit_visibility_allows(UnitVisibility::Respawn));
}

/// Fork default (`Unit.cpp:104`): units start `On`.
#[test]
fn visibility_default_on() {
    let fixture = setup(1);
    assert_eq!(
        fixture.map.get_visibility(fixture.viewer),
        UnitVisibility::On
    );
    // A pre-migration `hidden_objects` entry still reads
    // back as `Off` (legacy alias fold).
    let mut map = fixture.map;
    map.hidden_objects.insert(fixture.other);
    assert_eq!(map.get_visibility(fixture.other), UnitVisibility::Off);
}

#[test]
fn set_visibility_off_hides_and_on_restores() {
    let Fixture {
        mut map, viewer, ..
    } = setup(2);
    let target = summon_nearby(&mut map, viewer);
    let _ = map.drain_events();
    // Summoned at the viewer's position and explicitly
    // revealed (summons do not re-scan existing viewers).
    map.set_visibility(target, UnitVisibility::On);
    let _ = map.drain_events();
    assert!(visible_to(&map, viewer, target), "target starts visible");
    map.set_visibility(target, UnitVisibility::Off);
    assert_eq!(map.get_visibility(target), UnitVisibility::Off);
    assert!(
        !visible_to(&map, viewer, target),
        "OFF must drop the target from the viewer set"
    );
    let events = map.drain_events();
    assert!(
        !packets_to(&events, viewer, opcode_id::SMSG_UPDATE_OBJECT).is_empty(),
        "OFF must queue an out-of-range destroy (169) for the viewer"
    );
    map.set_visibility(target, UnitVisibility::On);
    assert_eq!(map.get_visibility(target), UnitVisibility::On);
    assert!(
        visible_to(&map, viewer, target),
        "ON must re-create the target for the viewer"
    );
    let events = map.drain_events();
    assert!(
        !packets_to(&events, viewer, opcode_id::SMSG_COMPRESSED_UPDATE_OBJECT).is_empty(),
        "ON must queue a create (502) for the viewer"
    );
}

#[test]
fn set_visibility_respawn_cycle() {
    let Fixture {
        mut map, viewer, ..
    } = setup(3);
    let target = summon_nearby(&mut map, viewer);
    map.set_visibility(target, UnitVisibility::On);
    let _ = map.drain_events();
    assert!(visible_to(&map, viewer, target));
    // `RESPAWN` is the force-delete visibility: destroy
    // now, restore on the later `On` (fork respawn dance).
    map.set_visibility(target, UnitVisibility::Respawn);
    assert!(!visible_to(&map, viewer, target));
    map.set_visibility(target, UnitVisibility::On);
    assert!(visible_to(&map, viewer, target));
}

#[test]
fn set_visibility_skips_self_viewer() {
    let Fixture {
        mut map,
        viewer,
        other,
    } = setup(4);
    assert!(visible_to(&map, other, viewer));
    // The fan-out skips the target's own viewer slot;
    // only the *other* viewer loses the target.
    map.set_visibility(viewer, UnitVisibility::Off);
    assert!(!visible_to(&map, other, viewer));
    assert_eq!(map.get_visibility(viewer), UnitVisibility::Off);
}

#[test]
fn set_visibility_unknown_guid_is_noop() {
    let Fixture {
        mut map, viewer, ..
    } = setup(5);
    let missing = ObjectGuid::new(mangos_guid::HighGuid::UNIT, 0x00FF_FFFF);
    assert!(!map.creatures.contains_key(&missing));
    map.set_visibility(missing, UnitVisibility::Off);
    map.set_visibility(missing, UnitVisibility::On);
    assert!(map.drain_events().is_empty(), "no-op must emit nothing");
    assert!(!visible_to(&map, viewer, viewer));
}

fn start_timed_cast(map: &mut Map, caster: ObjectGuid, spell_id: u32) {
    let outcome = map.cast_spell(
        caster,
        spell_id,
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
    assert!(map.casts.contains_key(&caster));
}

#[test]
fn interrupt_matching_id_fails_cast() {
    let Fixture {
        mut map, viewer, ..
    } = setup(6);
    start_timed_cast(&mut map, viewer, TIMED_SPELL);
    let _ = map.drain_events();
    map.interrupt_non_melee_spells(viewer, false, TIMED_SPELL);
    assert!(
        !map.casts.contains_key(&viewer),
        "interrupted cast leaves the slot"
    );
    let events = map.drain_events();
    let results = packets_to(&events, viewer, opcode_id::SMSG_CAST_RESULT);
    assert_eq!(results.len(), 1, "one CAST_RESULT to the caster");
    assert_eq!(results[0][4], cast_result::FAILED_INTERRUPTED);
    assert_eq!(
        packets_to(&events, viewer, opcode_id::SMSG_SPELL_FAILURE).len(),
        1,
        "SMSG_SPELL_FAILURE to the caster session"
    );
    assert!(
        !packets_to(&events, viewer, opcode_id::SMSG_SPELL_FAILED_OTHER).is_empty(),
        "SMSG_SPELL_FAILED_OTHER broadcast"
    );
}

#[test]
fn interrupt_wrong_id_leaves_cast() {
    let Fixture {
        mut map, viewer, ..
    } = setup(7);
    start_timed_cast(&mut map, viewer, TIMED_SPELL);
    let _ = map.drain_events();
    map.interrupt_non_melee_spells(viewer, false, TIMED_SPELL + 1);
    assert!(
        map.casts.contains_key(&viewer),
        "mismatched id must not touch the slot"
    );
    let events = map.drain_events();
    assert!(
        packets_to(&events, viewer, opcode_id::SMSG_CAST_RESULT).is_empty()
            && packets_to(&events, viewer, opcode_id::SMSG_SPELL_FAILURE).is_empty(),
        "mismatched id is silent"
    );
}

#[test]
fn interrupt_zero_id_hits_any_cast() {
    let Fixture {
        mut map, viewer, ..
    } = setup(8);
    start_timed_cast(&mut map, viewer, TIMED_SPELL);
    let _ = map.drain_events();
    // `with_delayed` is a no-op in this subset either way.
    map.interrupt_non_melee_spells(viewer, true, 0);
    assert!(!map.casts.contains_key(&viewer));
    let events = map.drain_events();
    assert_eq!(
        packets_to(&events, viewer, opcode_id::SMSG_CAST_RESULT).len(),
        1
    );
}

#[test]
fn interrupt_channeled_closes_without_cast_result() {
    let Fixture {
        mut map, viewer, ..
    } = setup(9);
    start_timed_cast(&mut map, viewer, CHANNEL_SPELL);
    let _ = map.drain_events();
    // Run out the 3 s cast so the channel starts.
    for _ in 0..70 {
        map.tick(50, map.now_ms + 50, 1_800_000_000);
    }
    let _ = map.drain_events();
    assert!(
        map.casts.get(&viewer).is_some_and(|c| c.channeling),
        "channel must be running before the interrupt"
    );
    map.interrupt_non_melee_spells(viewer, false, CHANNEL_SPELL);
    assert!(!map.casts.contains_key(&viewer));
    let events = map.drain_events();
    let updates = packets_to(&events, viewer, opcode_id::MSG_CHANNEL_UPDATE);
    assert_eq!(updates.len(), 1, "one channel close");
    assert_eq!(&updates[0][..], &[0, 0, 0, 0]);
    assert!(
        packets_to(&events, viewer, opcode_id::SMSG_CAST_RESULT).is_empty(),
        "channeled interrupt sends no CAST_RESULT (existing end_channel convention)"
    );
}

#[test]
fn interrupt_spells_casted_on_me_hits_only_targeters() {
    let Fixture {
        mut map,
        viewer,
        other,
    } = setup(11);
    let hostile = summon_nearby(&mut map, viewer);
    let _ = map.drain_events();
    // Viewer casts at the hostile creature; the other
    // player casts at themselves (friendly, and aimed
    // elsewhere — both arms must survive).
    for (caster, target) in [(viewer, hostile), (other, other)] {
        let outcome = map.cast_spell(
            caster,
            TIMED_SPELL,
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
    }
    let _ = map.drain_events();
    map.interrupt_spells_casted_on_me(hostile, true);
    assert!(
        !map.casts.contains_key(&viewer),
        "the targeter's cast is interrupted"
    );
    assert!(
        map.casts.contains_key(&other),
        "casts aimed elsewhere survive"
    );
    let events = map.drain_events();
    assert_eq!(
        packets_to(&events, viewer, opcode_id::SMSG_CAST_RESULT).len(),
        1
    );
}

#[test]
fn interrupt_empty_slot_and_instant_are_silent() {
    let Fixture {
        mut map, viewer, ..
    } = setup(10);
    map.interrupt_non_melee_spells(viewer, false, 0);
    map.interrupt_non_melee_spells(viewer, false, TIMED_SPELL);
    // Instant casts never occupy the slot.
    let outcome = map.cast_spell(
        viewer,
        INSTANT_SPELL,
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
    assert!(!map.casts.contains_key(&viewer));
    let _ = map.drain_events();
    map.interrupt_non_melee_spells(viewer, false, INSTANT_SPELL);
    assert!(map.drain_events().is_empty(), "no cast means no packets");
}
