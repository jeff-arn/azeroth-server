//! Combo-points parity slice (`Player::AddComboPoints` /
//! `ClearComboPoints`, `Spell::EffectAddComboPoints`,
//! rogue Eviscerate + druid Ferocious Bite finishers,
//! `Spell::finish` combo tail): failing-first maintained
//! tests (no DB).
//!
//! Fork pins (`vmangos/src`): `Objects/Player.h:1503-1507`,
//! `Objects/Player.cpp:18829-18891` (points lifecycle +
//! `SetComboPoints`), `Spells/SpellEffects.cpp:4513-4526`
//! (`EffectAddComboPoints`), `Spells/Spell.cpp:4462-4482`
//! (finish tail), `scripts/spells/spell_rogue.cpp:20-44`,
//! `scripts/spells/spell_druid.cpp:20-71`.

use std::collections::HashMap;
use std::sync::Arc;

use mangos_db::character_load::{CharacterLoad, CharacterRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::{dmg_class, effect, power};
use mangos_spells::entry::SpellEffectData;
use mangos_spells::entry::SpellEntry;
use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};

use crate::WorldRng;
use crate::map::{Map, MapConfig};
use crate::spells::{CastOptions, CastOutcome, CastTarget};
use crate::stores::WorldStores;

/// Combo-builder stand-in: effect 80, count 1, unit target.
const BUILDER_SPELL: u32 = 9201;
/// Eviscerate stand-in (rogue family, CM0 0x20000).
const EVISCERATE_SPELL: u32 = 9202;
/// Ferocious Bite stand-in (energy x2.0).
const BITE_SPELL: u32 = 9203;
/// Finishing-move stand-in (`ATTR_EX_FINISHING_MOVE_DAMAGE`).
const FINISHER_SPELL: u32 = 9204;
/// Mixed-target stand-in: damage on the unit target plus a
/// `TARGET_UNIT_CASTER` instakill that must land on the caster
/// (Sacrifice shape).
const MIXED_SELF_KILL_SPELL: u32 = 9205;
/// Execute stand-in: `TargetAuraState` `HEALTHLESS_20` gate.
const EXECUTE_SPELL: u32 = 9206;
/// Backstab stand-in: from-behind-only gate
/// (`AttributesEx2 == 0x100000` + `AttributesEx 0x200`).
const BACKSTAB_SPELL: u32 = 9207;
/// Stealth stand-in: cooldown-on-event shapeshift whose
/// only recovery is the category recovery (recovery 0,
/// category 38 at 10 s, like the real Stealth rows).
const STEALTH_SPELL: u32 = 9208;
/// Resurrect stand-in: `RESURRECT_NEW` with flat
/// health/mana like Rebirth (base health, misc mana).
const RESURRECT_SPELL: u32 = 9209;
/// Creature template entry for victims.
const VICTIM_ENTRY: u32 = 9301;
/// `TARGET_UNIT` implicit target (see insignia fixture).
const TARGET_UNIT: u32 = 25;

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
            name: format!("Combo{guid}"),
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

fn unit_target_effect(effect_id: u32, base_points: i32, multiplier: f32) -> SpellEffectData {
    SpellEffectData {
        effect: effect_id,
        implicit_target_a: TARGET_UNIT,
        base_points,
        dmg_multiplier: multiplier,
        ..Default::default()
    }
}

/// Cast-gate stand-ins (Execute health gate, Backstab
/// behind gate, Stealth cooldown-on-event conversion).
fn insert_gate_spells(stores: &mut WorldStores) {
    // Execute stand-in: unit damage gated on the target
    // being under 20% health (fork TargetAuraState).
    stores.spells.spells.insert(
        EXECUTE_SPELL,
        SpellEntry {
            id: EXECUTE_SPELL,
            target_aura_state: mangos_spells::defines::aura_state::HEALTHLESS_20_PERCENT,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10, 1.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Backstab stand-in: unit damage usable only from
    // behind the target (fork IsFromBehindOnlySpell
    // template predicate).
    stores.spells.spells.insert(
        BACKSTAB_SPELL,
        SpellEntry {
            id: BACKSTAB_SPELL,
            attributes_ex: 0x0000_0200,
            attributes_ex2: 0x0010_0000,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10, 1.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Stealth stand-in: shapeshift aura on the caster,
    // cooldown-on-event with category-only recovery.
    stores.spells.spells.insert(
        STEALTH_SPELL,
        SpellEntry {
            id: STEALTH_SPELL,
            attributes: 0x0200_0000, // SPELL_ATTR_COOLDOWN_ON_EVENT
            category: 38,
            category_recovery_time: 10_000,
            effects: [
                SpellEffectData {
                    effect: effect::APPLY_AURA,
                    apply_aura_name: 36, // MOD_SHAPESHIFT
                    misc_value: 30,      // FORM_STEALTH
                    implicit_target_a: 1,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Resurrect stand-in: RESURRECT_NEW, corpse-targeted
    // like the real rows (implicit target NONE, spell
    // Targets carries the corpse flag fork-side).
    stores.spells.spells.insert(
        RESURRECT_SPELL,
        SpellEntry {
            id: RESURRECT_SPELL,
            effects: [
                SpellEffectData {
                    effect: effect::RESURRECT_NEW,
                    base_points: 500,
                    misc_value: 300,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
}

fn combo_stores() -> WorldStores {
    let mut stores = WorldStores::empty();
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![CreatureInfo {
        entry: VICTIM_ENTRY,
        display_id: [100, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    }]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 100,
            ..Default::default()
        }]);
    // Builder: effect 80 count 1.
    stores.spells.spells.insert(
        BUILDER_SPELL,
        SpellEntry {
            id: BUILDER_SPELL,
            effects: [
                unit_target_effect(effect::ADD_COMBO_POINTS, 1, 1.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Eviscerate: rogue family CM0 0x20000, no base damage
    // (the AP x combo x 0.03 bonus is the whole hit).
    stores.spells.spells.insert(
        EVISCERATE_SPELL,
        SpellEntry {
            id: EVISCERATE_SPELL,
            dmg_class: dmg_class::MELEE,
            spell_family_name: 8, // SPELLFAMILY_ROGUE
            spell_family_flags: 0x0002_0000,
            script_name: "spell_rogue_eviscerate".to_string(),
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 0, 1.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Ferocious Bite: energy x2.0.
    stores.spells.spells.insert(
        BITE_SPELL,
        SpellEntry {
            id: BITE_SPELL,
            dmg_class: dmg_class::MELEE,
            script_name: "spell_druid_ferocious_bite".to_string(),
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 0, 2.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Generic finisher (finishing-move damage bit; debuff
    // bit so the spell reads non-positive like a real
    // finishing move).
    stores.spells.spells.insert(
        FINISHER_SPELL,
        SpellEntry {
            id: FINISHER_SPELL,
            dmg_class: dmg_class::MELEE,
            attributes: 0x0400_0000,    // ATTR_AURA_IS_DEBUFF
            attributes_ex: 0x0010_0000, // FINISHING_MOVE_DAMAGE
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10, 1.0),
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    insert_gate_spells(&mut stores);
    // Mixed-target spell: unit damage + caster-targeted
    // instakill (Sacrifice's shield-master/pet-suicide shape,
    // minus the aura).
    stores.spells.spells.insert(
        MIXED_SELF_KILL_SPELL,
        SpellEntry {
            id: MIXED_SELF_KILL_SPELL,
            effects: [
                unit_target_effect(effect::SCHOOL_DAMAGE, 10, 1.0),
                SpellEffectData {
                    effect: effect::INSTAKILL,
                    implicit_target_a: 1, // TARGET_UNIT_CASTER
                    ..Default::default()
                },
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
    victim: ObjectGuid,
}

fn setup(seed: u64) -> Fixture {
    let infos = test_infos();
    let stores = Arc::new(combo_stores());
    let mut map = Map::new(
        0,
        0,
        MapConfig::default(),
        Arc::clone(&stores),
        WorldRng::from_seed(seed),
    );
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    let caster = materialize(7, &infos).unit.guid();
    assert!(caster.is_player());
    map.add_player(materialize(7, &infos), &respawns, 1_800_000_000, 0);
    let pos = map.unit_position(caster).expect("caster positioned");
    let victim = map
        .summon_creature(VICTIM_ENTRY, pos, caster, 0, 0)
        .expect("summon victim");
    let _ = map.drain_events();
    Fixture {
        map,
        caster,
        victim,
    }
}

fn cast_options() -> CastOptions {
    CastOptions {
        triggered: false,
        triggered_by_aura: false,
        skip_known: true,
        cast_item: None,
        custom_base_points: [None; 3],
        triggered_by_aura_spell: 0,
        triggered_by_spell: 0,
    }
}

fn victim_health(map: &Map, victim: ObjectGuid) -> u32 {
    map.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH)
}

#[test]
fn combo_add_clear_clamp_and_retarget() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(1);
    assert_eq!(map.combo_points(caster), 0);
    // No target: clear is a no-op.
    map.clear_combo_points(caster);
    assert_eq!(map.combo_points(caster), 0);
    map.add_combo_points(caster, victim, 3);
    assert_eq!(map.combo_points(caster), 3);
    // Same target accumulates, clamped to 5.
    map.add_combo_points(caster, victim, 10);
    assert_eq!(map.combo_points(caster), 5);
    // Zero count is a no-op.
    map.add_combo_points(caster, victim, 0);
    assert_eq!(map.combo_points(caster), 5);
    // Retarget resets the count.
    let other = ObjectGuid::new(mangos_guid::HighGuid::UNIT, 0x00FF_FFFF);
    map.add_combo_points(caster, other, 2);
    assert_eq!(map.combo_points(caster), 2);
    map.clear_combo_points(caster);
    assert_eq!(map.combo_points(caster), 0);
    map.clear_combo_points(caster);
    assert_eq!(map.combo_points(caster), 0);
}

#[test]
fn combo_mirrors_update_fields() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(2);
    map.add_combo_points(caster, victim, 4);
    let player = map.player(caster).expect("caster");
    assert_eq!(player.combo_points, 4);
    assert_eq!(player.combo_target, victim);
    let values = &player.unit.world.object.values();
    assert_eq!(
        values.get_byte(offsets::PLAYER_FIELD_BYTES, 1),
        4,
        "combo byte mirrors the count"
    );
    assert_eq!(
        values.get_guid(offsets::PLAYER__FIELD_COMBO_TARGET),
        victim,
        "combo target mirrors the victim"
    );
}

#[test]
fn combo_holders_sweep_clears_every_holder() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(3);
    map.add_combo_points(caster, victim, 5);
    assert_eq!(map.combo_points(caster), 5);
    map.clear_combo_point_holders(victim);
    assert_eq!(map.combo_points(caster), 0);
    // Unrelated targets are untouched.
    map.add_combo_points(caster, victim, 2);
    let other = ObjectGuid::new(mangos_guid::HighGuid::UNIT, 0x00EE_EEEE);
    map.clear_combo_point_holders(other);
    assert_eq!(map.combo_points(caster), 2);
}

#[test]
fn effect_add_combo_points_builds_on_hit() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(4);
    let outcome = map.cast_spell(
        caster,
        BUILDER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(map.combo_points(caster), 1);
    assert_eq!(map.player(caster).expect("caster").combo_target, victim);
    // Non-player casters build nothing (fork `ToPlayer` gate).
    let outcome = map.cast_spell(
        victim,
        BUILDER_SPELL,
        CastTarget::Unit(caster),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
}

#[test]
fn eviscerate_scales_with_combo_and_attack_power() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(5);
    // AP 200, no combo: script adds nothing, base is 0.
    if let Some(unit) = map.unit_base_mut(caster) {
        unit.base_attack_power = 200.0;
    }
    let before = victim_health(&map, victim);
    let outcome = map.cast_spell(
        caster,
        EVISCERATE_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        victim_health(&map, victim),
        before,
        "no combo means no bonus damage"
    );
    // 5 combo x 200 AP x 0.03 = 30.0 exactly; dither of an
    // integer value is exact, armor is 0, melee class never
    // crits — the drop is exactly 30.
    map.add_combo_points(caster, victim, 5);
    let outcome = map.cast_spell(
        caster,
        EVISCERATE_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        before - victim_health(&map, victim),
        30,
        "AP x combo x 0.03 bonus"
    );
}

#[test]
fn ferocious_bite_drains_energy() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(6);
    map.unit_set_power(caster, power::ENERGY, 100);
    map.add_combo_points(caster, victim, 2);
    let before = victim_health(&map, victim);
    let outcome = map.cast_spell(caster, BITE_SPELL, CastTarget::Unit(victim), cast_options());
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        map.unit_power(caster, power::ENERGY),
        0,
        "energy drains to 0 after the bite"
    );
    assert!(
        victim_health(&map, victim) < before,
        "combo AP bonus + energy x multiplier must land"
    );
}

#[test]
fn finisher_cast_clears_combo_on_hit() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(7);
    map.add_combo_points(caster, victim, 3);
    let outcome = map.cast_spell(
        caster,
        FINISHER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        map.combo_points(caster),
        0,
        "successful finish drops the combo"
    );
}

#[test]
fn finisher_miss_keeps_combo() {
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(8);
    map.add_combo_points(caster, victim, 3);
    let spell = map
        .stores
        .spells
        .get(FINISHER_SPELL)
        .expect("finisher entry")
        .clone();
    // A non-caster miss on a non-positive finisher keeps
    // the points (`needDrop == false`).
    assert!(!spell.is_positive_spell());
    map.clear_combo_on_spell_finish(caster, &spell, true);
    assert_eq!(map.combo_points(caster), 3);
    map.clear_combo_on_spell_finish(caster, &spell, false);
    assert_eq!(map.combo_points(caster), 0);
}

#[test]
fn finisher_without_combo_target_rejected() {
    // Fork `Spell::CheckCast` (`Spell.cpp:7108`): a player
    // finisher (`NeedsComboPoints`) with an explicitly
    // selected unit target that is not the combo target
    // fails `SPELL_FAILED_NO_COMBO_POINTS` (70) before any
    // effect runs. Found live by the Druid spell campaign:
    // Rip/Ferocious Bite landed at 0 combo points.
    use mangos_spells::defines::cast_result;
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(9);
    // The shared fixture is a Warrior (class 1): the fork
    // reads BAD_TARGETS there (Overpower marking), and
    // NO_COMBO_POINTS for every other class. Assert both
    // by flipping the class byte to Druid (11) after the
    // warrior-shaped rejection.
    let outcome = map.cast_spell(
        caster,
        FINISHER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS),
        "warrior finisher at 0 combo points reads BAD_TARGETS"
    );
    map.player_mut(caster)
        .expect("player")
        .unit
        .world
        .object
        .values_mut()
        .set_byte(offsets::UNIT_FIELD_BYTES_0, 1, 11);
    let outcome = map.cast_spell(
        caster,
        FINISHER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_NO_COMBO_POINTS),
        "finisher at 0 combo points must be rejected at cast time"
    );
    // Combo on a *different* unit does not satisfy the gate.
    let pos = map.unit_position(caster).expect("caster positioned");
    let other = map
        .summon_creature(VICTIM_ENTRY, pos, caster, 0, 0)
        .expect("summon other");
    map.add_combo_points(caster, other, 2);
    let outcome = map.cast_spell(
        caster,
        FINISHER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_NO_COMBO_POINTS),
        "finisher on a non-combo target must be rejected"
    );
    // Combo built on the victim: the finisher lands.
    map.add_combo_points(caster, victim, 2);
    let outcome = map.cast_spell(
        caster,
        FINISHER_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
}

#[test]
fn unit_caster_effect_lands_on_caster_not_victim() {
    // Per-effect target map (fork `SetTargetMap` per
    // effect): inside a mixed spell, the
    // `TARGET_UNIT_CASTER` instakill strikes the caster
    // while the unit-target damage strikes the victim.
    // Found live by the Warlock campaign (Sacrifice
    // would otherwise shield/kill the wrong unit).
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(10);
    let victim_before = victim_health(&map, victim);
    let outcome = map.cast_spell(
        caster,
        MIXED_SELF_KILL_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    assert_eq!(
        map.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
        0,
        "caster-targeted instakill must land on the caster"
    );
    let victim_after = victim_health(&map, victim);
    assert!(
        victim_after < victim_before && victim_after > 0,
        "victim takes only the unit-target damage ({victim_before} -> {victim_after})"
    );
}

#[test]
fn execute_gate_rejects_above_20_percent_health() {
    // Fork `CheckCast` TargetAuraState gate
    // (`Spell.cpp:5868`): Execute on a target at or above
    // 20% health fails `SPELL_FAILED_BAD_TARGETS`; found
    // live by the Warrior campaign, whose Execute rows
    // passed at full dummy health.
    use mangos_spells::defines::cast_result;
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(11);
    let outcome = map.cast_spell(
        caster,
        EXECUTE_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS),
        "Execute above 20% target health must be rejected"
    );
    // Wound the victim below 20% directly (builder bites
    // overshoot a 100 hp fixture and kill it outright).
    let max_hp = map.unit_value_u32(victim, offsets::UNIT_FIELD_MAXHEALTH);
    map.creatures
        .get_mut(&victim)
        .expect("victim creature")
        .unit
        .world
        .object
        .values_mut()
        .set_u32(offsets::UNIT_FIELD_HEALTH, max_hp / 10);
    assert!(
        victim_health(&map, victim) * 5 < max_hp,
        "victim must be below 20% for the open-gate cast (hp {} max {max_hp})",
        victim_health(&map, victim)
    );
    let outcome = map.cast_spell(
        caster,
        EXECUTE_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
}

#[test]
fn backstab_gate_rejects_from_the_front() {
    // Fork `CheckCast` behind-target gate
    // (`Spell.cpp:5772-5777`, `IsFromBehindOnlySpell`
    // at `SpellEntry.h:867`): a from-behind-only spell
    // cast while the target faces the caster fails
    // `SPELL_FAILED_NOT_BEHIND` (51); found live by
    // the Rogue campaign, whose Backstab rows passed
    // from the front.
    use mangos_movement::movement_info::Position;
    use mangos_spells::defines::cast_result;
    let Fixture {
        mut map,
        caster,
        victim,
    } = setup(12);
    let cpos = map.unit_position(caster).expect("caster positioned");
    // Victim two yards east, facing west (toward the
    // caster): the caster is in its front arc.
    map.relocate_unit(
        victim,
        Position::new(cpos.x + 2.0, cpos.y, cpos.z, std::f32::consts::PI),
    );
    let outcome = map.cast_spell(
        caster,
        BACKSTAB_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_NOT_BEHIND),
        "Backstab from the front must be rejected"
    );
    // Same geometry with the victim facing east (away):
    // the caster is behind it and the cast opens.
    map.relocate_unit(victim, Position::new(cpos.x + 2.0, cpos.y, cpos.z, 0.0));
    let outcome = map.cast_spell(
        caster,
        BACKSTAB_SPELL,
        CastTarget::Unit(victim),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
}

#[test]
fn stealth_category_only_cooldown_on_event_converts_on_fade() {
    // Fork `Player::AddCooldown` permanent hold: a
    // cooldown-on-event spell whose only recovery is
    // the category recovery (Stealth: recovery 0,
    // category 38 at 10 s) still records a spell-key
    // permanent entry, so the fade conversion can
    // start the timed category cooldown. Without the
    // spell-key hold the permanent category entry
    // blocks recasts forever (live Rogue probe:
    // NOT_READY at +121 s).
    use mangos_spells::defines::cast_result;
    let Fixture {
        mut map, caster, ..
    } = setup(13);
    let outcome = map.cast_spell(caster, STEALTH_SPELL, CastTarget::SelfCast, cast_options());
    assert_eq!(outcome, CastOutcome::Started);
    assert!(
        map.aura_holders(caster)
            .iter()
            .any(|h| h.spell_id == STEALTH_SPELL),
        "stealth holder missing"
    );
    map.cancel_aura(caster, STEALTH_SPELL);
    assert!(
        map.aura_holders(caster)
            .iter()
            .all(|h| h.spell_id != STEALTH_SPELL),
        "stealth holder must cancel"
    );
    let outcome = map.cast_spell(caster, STEALTH_SPELL, CastTarget::SelfCast, cast_options());
    assert_eq!(
        outcome,
        CastOutcome::Failed(cast_result::FAILED_NOT_READY),
        "converted 10 s category cooldown must block an immediate recast"
    );
    let mut now_ms = map.now_ms;
    for _ in 0..220 {
        now_ms += 50;
        map.tick(50, now_ms, 1_800_000_000);
        let _ = map.drain_events();
    }
    let outcome = map.cast_spell(caster, STEALTH_SPELL, CastTarget::SelfCast, cast_options());
    assert_eq!(
        outcome,
        CastOutcome::Started,
        "stealth must recast after the 10 s category cooldown"
    );
}

#[test]
fn resurrect_new_requests_and_accept_resurrects_at_caster() {
    // Fork resurrect flow (`EffectResurrectNew`,
    // `SpellEffects.cpp:202`; `SendResurrectRequest`,
    // `Spell.cpp:5147`; `HandleResurrectResponseOpcode`,
    // `MiscHandler.cpp:647`): a corpse-targeted cast
    // sends SMSG_RESURRECT_REQUEST to the dead owner;
    // accepting teleports the ghost to the caster and
    // resurrects with the effect's flat health/mana,
    // leaving bones. Found by the spell campaign:
    // corpse targets were dropped by the cast parser
    // and the effect had no body.
    use mangos_entities::corpse::{Corpse, CorpseAppearance, CorpseType};
    use mangos_entities::unit::DeathState;
    use mangos_protocol::opcode_table::opcode_id as op;
    let Fixture {
        mut map, caster, ..
    } = setup(14);
    let infos = test_infos();
    let dead = materialize(8, &infos).unit.guid();
    let respawns = mangos_world_data::spawn_support::RespawnTimes::default();
    map.add_player(materialize(8, &infos), &respawns, 1_800_000_000, 0);
    {
        let player = map.player_mut(dead).expect("dead player");
        player.unit.death_state = DeathState::Dead;
        player.unit.set_health(0);
    }
    let pos = map.unit_position(dead).expect("dead positioned");
    map.seed_corpse_counter(1000);
    let corpse = Corpse::create(
        1001,
        0,
        0,
        dead,
        CorpseType::ResurrectablePvE,
        pos,
        &CorpseAppearance::default(),
        1_800_000_000,
    );
    let corpse_guid = corpse.guid();
    map.add_corpse(corpse);
    let _ = map.drain_events();
    let outcome = map.cast_spell(
        caster,
        RESURRECT_SPELL,
        CastTarget::Corpse(corpse_guid),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let events = map.drain_events();
    let requests: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            crate::map::MapEvent::Packet { to, packet }
                if *to == dead && packet.opcode() == op::SMSG_RESURRECT_REQUEST =>
            {
                Some(packet.body().to_vec())
            }
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "one resurrect request to the dead owner");
    assert_eq!(
        u64::from_le_bytes(requests[0][0..8].try_into().unwrap()),
        caster.raw(),
        "request carries the resurrector guid"
    );
    // Decline clears the request without resurrecting.
    map.resurrect_response(dead, caster, false);
    assert_eq!(
        map.player(dead).expect("dead player").unit.death_state,
        DeathState::Dead
    );
    // Re-cast and accept: alive at the caster with the
    // flat health/mana, corpse converted to bones.
    let outcome = map.cast_spell(
        caster,
        RESURRECT_SPELL,
        CastTarget::Corpse(corpse_guid),
        cast_options(),
    );
    assert_eq!(outcome, CastOutcome::Started);
    let _ = map.drain_events();
    map.resurrect_response(dead, caster, true);
    let player = map.player(dead).expect("resurrected player");
    assert_eq!(player.unit.death_state, DeathState::Alive);
    let hp = map.unit_value_u32(dead, offsets::UNIT_FIELD_HEALTH);
    let max_hp = map.unit_value_u32(dead, offsets::UNIT_FIELD_MAXHEALTH);
    assert_eq!(hp, max_hp.min(500), "resurrect health capped at max");
    let max_mana = map.unit_value_u32(dead, offsets::UNIT_FIELD_MAXPOWER1);
    assert_eq!(
        map.unit_value_u32(dead, offsets::UNIT_FIELD_POWER1),
        max_mana.min(300)
    );
    assert_eq!(
        map.corpses.get(&corpse_guid).map(|c| c.corpse_type),
        Some(CorpseType::Bones),
        "corpse converts to bones on resurrect"
    );
}
