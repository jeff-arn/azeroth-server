//! Battleground insignia (`SPELL_EFFECT_SKIN_PLAYER_CORPSE`,
//! effect 116) parity slice: `RemovedInsignia`, corpse
//! conversion, the corpse loot window with BG reference
//! loot, and release semantics.
//!
//! Fork pins (`vmangos/src`):
//! `SpellEffects.cpp:5735-5760` (`EffectSkinPlayerCorpse`),
//! `Spell.cpp:3305` (unit-target capture) + `:4870`
//! (no range/LOS gate), `Player.cpp:7655-7677`
//! (`RemovedInsignia`), `ObjectAccessor.cpp:247-280` +
//! `Map.cpp:3487-3586` (convert + bones + auto-loot),
//! `Player.cpp:7837-7871` (corpse loot arm,
//! `LOOT_INSIGNIA` = 22, wire `LOOT_CORPSE`),
//! `LootHandler.cpp:524-540` (corpse release).

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::corpse::{
    CORPSE_FLAG_BONES, CORPSE_FLAG_UNK2, Corpse, CorpseAppearance, CorpseType,
};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
use mangos_entities::unit::DeathState;
use mangos_guid::ObjectGuid;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::effect;
use mangos_spells::entry::{SpellEffectData, SpellEntry};
use mangos_world_data::loot::{LootStore, LootStoreItem};

use crate::WorldRng;
use crate::battlegrounds::BgInit;
use crate::map::{Map, MapConfig, MapEvent};
use crate::spells::{CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;

/// Remove-Insignia stand-in: effect 116 on the unit
/// target, always-hit like the skinning fixture.
const INSIGNIA_SPELL: u32 = 8877;
/// BG reference-loot id carried by the test template.
const REF_LOOT_ID: u32 = 77;
/// Reference-loot item (linen 2589, 100%).
const REF_ITEM: u32 = 2589;

fn insignia_stores() -> WorldStores {
    let mut stores = WorldStores::empty();
    // Linen 2589 stacks to 20 (the default zero would
    // plan zero-space destinations and fail the take).
    stores
        .item_prototypes
        .insert_for_tests(mangos_world_data::item_prototype::ItemPrototype {
            item_id: REF_ITEM,
            stackable: 20,
            ..Default::default()
        });
    stores.loot.reference = LootStore::from_fixtures_for_tests(
        vec![(
            REF_LOOT_ID,
            LootStoreItem {
                itemid: REF_ITEM,
                chance: 100.0,
                mincount_or_ref: 1,
                group: 0,
                needs_quest: false,
                maxcount: 1,
                condition_id: 0,
            },
        )],
        true,
    );
    stores.spells.spells.insert(
        INSIGNIA_SPELL,
        SpellEntry {
            id: INSIGNIA_SPELL,
            range_index: 9001,
            attributes_ex3: 0x0004_0000, // SPELL_ATTR_EX3_ALWAYS_HIT
            dmg_class: 1,                // MAGIC
            effects: [
                SpellEffectData {
                    effect: effect::SKIN_PLAYER_CORPSE,
                    implicit_target_a: 25, // TARGET_UNIT
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
            name: format!("Insignia{guid}"),
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

fn ws_init(ref_loot_id: u32) -> BgInit {
    let start = mangos_world_data::battlegrounds::WorldSafeLoc {
        id: 1,
        map_id: 30,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        orientation: 0.0,
    };
    BgInit {
        bg_type_id: 2,
        bracket_id: 0,
        client_instance_id: 1,
        name: "Warsong Gulch".to_string(),
        min_players_per_team: 10,
        max_players_per_team: 10,
        alliance_win_spell: 0,
        alliance_lose_spell: 0,
        horde_win_spell: 0,
        horde_lose_spell: 0,
        alliance_start: start,
        horde_start: start,
        player_skin_ref_loot_id: ref_loot_id,
        level_min: 10,
        level_max: 60,
    }
}

fn insignia_options() -> CastOptions {
    CastOptions {
        triggered: true,
        skip_known: true,
        ..Default::default()
    }
}

struct Fixture {
    map: Map,
    caster: ObjectGuid,
    victim: ObjectGuid,
    corpse: ObjectGuid,
}

fn setup(bg_ref_loot: Option<u32>, seed: u64) -> Fixture {
    let infos = test_infos();
    let stores = Arc::new(insignia_stores());
    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(seed),
    );
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    let caster = materialize(7, &infos).unit.guid();
    let victim = materialize(8, &infos).unit.guid();
    assert!(caster.is_player() && victim.is_player());
    map.add_player(materialize(7, &infos), &respawns, 1_800_000_000, 0);
    map.add_player(materialize(8, &infos), &respawns, 1_800_000_000, 0);
    if let Some(ref_loot_id) = bg_ref_loot {
        map.init_battleground(ws_init(ref_loot_id));
    }
    map.player_mut(victim).expect("victim").unit.death_state = DeathState::Corpse;
    let pos = map.player(victim).expect("victim").unit.world.position();
    map.seed_corpse_counter(1000);
    let corpse = Corpse::create(
        1001,
        0,
        0,
        victim,
        CorpseType::ResurrectablePvP,
        pos,
        &CorpseAppearance::default(),
        1_800_000_000,
    );
    let corpse_guid = corpse.guid();
    map.add_corpse(corpse);
    Fixture {
        map,
        caster,
        victim,
        corpse: corpse_guid,
    }
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

/// Full BG round trip (fork `EffectSkinPlayerCorpse`
/// dead-player arm): the caster skins a released corpse
/// on a BG map; the victim is repopped, skinned
/// (`SMSG_PLAYER_SKINNED`), converted to lootable bones
/// with a victim-level gold + reference-loot insignia
/// window; the window re-opens, and release after a full
/// take clears the flag without refilling.
fn assert_no_looting_flag(map: &mut Map, guid: ObjectGuid) {
    let flags = map
        .unit_base_mut(guid)
        .expect("unit")
        .values()
        .get_u32(offsets::UNIT_FIELD_FLAGS);
    assert_eq!(flags & 0x0000_0400, 0);
}

#[test]
fn bg_insignia_round_trip() {
    let mut fx = setup(Some(REF_LOOT_ID), 7);
    let outcome = fx.map.cast_spell(
        fx.caster,
        INSIGNIA_SPELL,
        CastTarget::Unit(fx.victim),
        insignia_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let events = fx.map.drain_events();

    // `RemovedInsignia`: victim repopped to ghost ...
    assert_eq!(
        fx.map.player(fx.victim).expect("victim").unit.death_state,
        DeathState::Dead,
        "repop ghost form"
    );
    // ... and told it was skinned (fork `Player.cpp:7672`).
    let skinned = packets_to(&events, fx.victim, opcode_id::SMSG_PLAYER_SKINNED);
    assert_eq!(skinned.len(), 1, "exactly one skinned notify");
    assert_eq!(skinned[0], vec![0u8], "skinned byte 0");
    // Corpse gone, bones in its place ...
    assert!(
        fx.map.corpse_for_player(fx.victim).is_none(),
        "no resurrectable corpse"
    );
    let bones = fx
        .map
        .corpses
        .get(&fx.corpse)
        .expect("bones share the corpse guid");
    assert_eq!(bones.corpse_type, CorpseType::Bones);
    let flags = bones
        .world
        .object
        .values()
        .get_u32(offsets::CORPSE_FIELD_FLAGS);
    assert_eq!(flags, CORPSE_FLAG_UNK2 | CORPSE_FLAG_BONES, "bones flags");
    // ... announced to the owner (fork `Map.cpp:3557-3562`).
    let query = packets_to(&events, fx.victim, opcode_id::MSG_CORPSE_QUERY);
    assert_eq!(query.len(), 1, "corpse query to owner");
    assert_eq!(query[0], vec![0u8]);

    // Auto-loot window on the caster (wire
    // `LOOT_PICKPOCKETING`, the `LOOT_INSIGNIA` remap
    // at fork `:8051-8068`).
    let responses = packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE);
    assert_eq!(responses.len(), 1, "one loot response");
    let view = &responses[0];
    assert_eq!(&view[0..8], &fx.corpse.raw().to_le_bytes(), "bones guid");
    assert_eq!(view[8], 2, "insignia wire byte remaps to pickpocketing");
    assert_no_looting_flag(&mut fx.map, fx.caster);
    let gold = u32::from_le_bytes(view[9..13].try_into().expect("gold"));
    // Fork `Player.cpp:7857`: 50..=150 x 0.016 x
    // (60/5.76)^2.5 at rate 1.0 -> 280..=840.
    assert!((280..=840).contains(&gold), "victim-level gold, got {gold}");
    // Reference fill from the BG template row.
    let state = fx.map.loots.get(&fx.corpse).expect("bones loot state");
    assert!(state.personal, "fork m_personal");
    assert_eq!(state.items.len(), 1, "reference fill");
    assert_eq!(state.items[0].itemid, REF_ITEM);
    assert_eq!(fx.map.player_loot_guid.get(&fx.caster), Some(&fx.corpse));

    // Re-open serves the same window (fork `lootForBody`).
    fx.map.handle_loot(fx.caster, fx.corpse);
    let reopened = packets_to(
        &fx.map.drain_events(),
        fx.caster,
        opcode_id::SMSG_LOOT_RESPONSE,
    );
    assert_eq!(reopened.len(), 1, "reopen response");
    assert_eq!(reopened[0][8], 1, "wire byte stable");

    // Full take: item + money, then release clears the
    // flag without refilling (fork `LootHandler:524-540`).
    assert!(!fx.map.loots.get(&fx.corpse).expect("state").is_looted());
    fx.map.handle_autostore_loot_item(fx.caster, 0);
    assert!(fx.map.loots.get(&fx.corpse).expect("state").items[0].is_looted);
    fx.map.handle_loot_money(fx.caster);
    assert_eq!(fx.map.loots.get(&fx.corpse).expect("state").gold, 0);
    assert!(fx.map.loots.get(&fx.corpse).expect("state").is_looted());
    fx.map.handle_loot_release(fx.caster);
    let released = fx.map.drain_events();
    assert_eq!(
        packets_to(&released, fx.caster, opcode_id::SMSG_LOOT_RELEASE_RESPONSE).len(),
        1,
        "release packet"
    );
    let dynflags = fx
        .map
        .corpses
        .get(&fx.corpse)
        .expect("bones")
        .world
        .object
        .values()
        .get_u32(offsets::CORPSE_FIELD_DYNAMIC_FLAGS);
    assert_eq!(dynflags & 1, 0, "LOOTABLE cleared");
    // Entry survives the clear (fork `loot->clear()`,
    // `lootForBody` stays set): reopen is empty, not
    // refilled.
    fx.map.handle_loot(fx.caster, fx.corpse);
    let empty = packets_to(
        &fx.map.drain_events(),
        fx.caster,
        opcode_id::SMSG_LOOT_RESPONSE,
    );
    assert_eq!(empty.len(), 1, "post-release reopen");
    assert_eq!(
        u32::from_le_bytes(empty[0][9..13].try_into().expect("gold")),
        0
    );
    assert_eq!(empty[0][13], 0, "no items refilled");
}

/// Corpse-guid arm without a BG: the Remove-Insignia
/// item targets dead players, so the dead-player arm is
/// a BG-gated no-op here (fork `GetBattleGroundId`
/// gate); the resurrectable corpse is untouched.
#[test]
fn non_bg_dead_player_cast_is_noop() {
    let mut fx = setup(None, 9);
    let outcome = fx.map.cast_spell(
        fx.caster,
        INSIGNIA_SPELL,
        CastTarget::Unit(fx.victim),
        insignia_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let events = fx.map.drain_events();
    assert!(packets_to(&events, fx.victim, opcode_id::SMSG_PLAYER_SKINNED).is_empty());
    assert!(packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE).is_empty());
    assert!(!fx.map.loots.contains_key(&fx.corpse), "no window opened");
    assert_eq!(
        fx.map.corpse_for_player(fx.victim),
        Some(fx.corpse),
        "corpse untouched"
    );
    assert_eq!(
        fx.map.player(fx.victim).expect("victim").unit.death_state,
        DeathState::Corpse,
        "no repop outside BG"
    );
}

/// Corpse-guid arm on a live BG map (fork arm (a):
/// owner offline): convert + insignia window without any
/// `RemovedInsignia` repop or skinned notify.
#[test]
fn bg_corpse_guid_cast_converts_without_repop() {
    let mut fx = setup(Some(REF_LOOT_ID), 11);
    let outcome = fx.map.cast_spell(
        fx.caster,
        INSIGNIA_SPELL,
        CastTarget::Unit(fx.corpse),
        insignia_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let events = fx.map.drain_events();
    assert!(packets_to(&events, fx.victim, opcode_id::SMSG_PLAYER_SKINNED).is_empty());
    assert_eq!(
        fx.map.player(fx.victim).expect("victim").unit.death_state,
        DeathState::Corpse,
        "corpse arm never repops"
    );
    assert!(fx.map.corpse_for_player(fx.victim).is_none());
    let responses = packets_to(&events, fx.caster, opcode_id::SMSG_LOOT_RESPONSE);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0][8], 2);
    let state = fx.map.loots.get(&fx.corpse).expect("bones loot state");
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].itemid, REF_ITEM);
}
