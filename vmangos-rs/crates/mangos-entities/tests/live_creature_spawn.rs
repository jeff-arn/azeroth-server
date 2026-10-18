//! Live creature spawn for bench spawn guid 1 (entry 2843).
//! `#[ignore]`d; gated on `TEST_MYSQL_URL_WORLD`. Expected
//! values are independent SQL oracles (2026-10-19):
//! fixed level 43, CLS health 1716 × `health_multiplier` 1.2
//! → 2059, armor 2343, display 4481 (probability 1/1),
//! gender byte 1 from the display addon.

use mangos_db::DbActor;
use mangos_entities::creature::{Creature, CreatureRates, DisplayResolver, SpawnRng};
use mangos_entities::unit::{POWER_MANA, POWER_RAGE};
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
use mangos_world_data::spawn_support::{
    load_class_level_stats, load_creature_addons, load_display_info_addons,
    load_equipment_templates,
};
use mangos_world_data::spawns::{SpawnData, load_map_ids};

struct MinRng;

impl SpawnRng for MinRng {
    fn urand(&mut self, min: u32, _max: u32) -> u32 {
        min
    }
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_spawn_guid1_materializes() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let creature = actor
        .call(|conn| {
            let (templates, _) = load_creature_templates(conn, 10, &TemplateValidation::default())?;
            let (go_templates, _) =
                load_gameobject_templates(conn, 10, &GameObjectValidation::default())?;
            let map_ids = load_map_ids(conn)?;
            let mut spawns = SpawnData::new();
            spawns.load_creatures(conn, 10, &templates, Some(&map_ids), false)?;
            spawns.load_gameobjects(conn, 10, &go_templates, Some(&map_ids), None, false)?;
            let (addons, _) = load_creature_addons(conn, 10, None, None, None)?;
            let (equipment, _) = load_equipment_templates(conn, 10, None)?;
            let (cls, _) = load_class_level_stats(conn)?;
            let (display_addons, _) = load_display_info_addons(conn, 5875, None, None)?;
            let spawn = spawns.creature(1).expect("guid 1").clone();
            let resolver = DisplayResolver {
                scale_for_display: &|_| 1.0,
                model_for_display: &|_| None,
            };
            let mut rng = MinRng;
            let creature = Creature::spawn_from_db(
                1,
                &spawn,
                &templates,
                addons.get(1),
                &equipment,
                &|_| None,
                &cls,
                &display_addons,
                &resolver,
                &CreatureRates::default(),
                0,
                1_800_000_000,
                false,
                &mut rng,
            )
            .expect("spawn guid 1");
            Ok(creature)
        })
        .expect("creature spawn");

    let v = creature.unit.world.object.values();
    assert_eq!(creature.original_entry, 2843);
    assert_eq!(creature.unit.level(), 43);
    assert_eq!(creature.unit.class(), 1);
    assert_eq!(creature.unit.gender(), 1, "display addon gender");
    assert_eq!(creature.unit.display_id(), 4481);
    assert_eq!(creature.unit.native_display_id(), 4481);
    assert_eq!(creature.unit.faction_template(), 120);
    assert_eq!(v.get_u32(offsets::UNIT_NPC_FLAGS), 16_388);
    // Health: round(1716 * 1.2) = 2059; create = 761.
    assert_eq!(creature.unit.max_health(), 2059);
    assert_eq!(creature.unit.health(), 2059);
    assert_eq!(creature.unit.create_health, 761);
    // CLS mana 0 -> rage power type, max 1000, current 0.
    assert_eq!(creature.unit.power_type(), 1);
    assert_eq!(creature.unit.max_power(POWER_RAGE), 1000);
    assert_eq!(creature.unit.power(POWER_RAGE), 0);
    assert_eq!(creature.unit.max_power(POWER_MANA), 0);
    // Stats and armor from CLS 43.
    assert_eq!(
        [
            creature.unit.stat(0),
            creature.unit.stat(1),
            creature.unit.stat(2),
            creature.unit.stat(3),
            creature.unit.stat(4)
        ]
        .map(f32::to_bits),
        [91.0f32, 71.0, 174.0, 39.0, 58.0].map(f32::to_bits)
    );
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RESISTANCES), 2343);
    // Attack times + cast speed.
    assert_eq!(
        v.get_f32(offsets::UNIT_FIELD_BASEATTACKTIME).to_bits(),
        2000.0f32.to_bits()
    );
    assert_eq!(
        v.get_f32(offsets::UNIT_MOD_CAST_SPEED).to_bits(),
        1.0f32.to_bits()
    );
    // Melee damage range: avg 77.4149, variance 14%.
    let (min, max) = creature.base_weapon_damage[0];
    assert_eq!(min.to_bits(), (77.4149f32 - 77.4149 * 0.14).to_bits());
    assert_eq!(max.to_bits(), (77.4149f32 + 77.4149 * 0.14).to_bits());
    // Misc byte = AURAS only; sheath melee.
    assert_eq!(v.get_byte(offsets::UNIT_FIELD_BYTES_2, 1), 0x10);
    assert_eq!(v.get_byte(offsets::UNIT_FIELD_BYTES_2, 0), 1);
    // Position from the spawn row; respawn delay 300.
    let pos = creature.unit.world.position();
    assert_eq!(creature.unit.world.map_id, 0);
    assert_eq!(pos.x.to_bits(), (-14467.8f32).to_bits());
    assert_eq!(creature.respawn_delay, 300);
    assert_eq!(creature.corpse_delay, 60);
    // No model data supplied -> UpdateModelData fallback 1.5.
    assert_eq!(
        v.get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS).to_bits(),
        1.5f32.to_bits()
    );
    // Speed rates from the template (1.0 / 1.14286).
    assert_eq!(creature.unit.speed(0).to_bits(), 2.5f32.to_bits());
    assert_eq!(
        creature.unit.speed(1).to_bits(),
        (7.0f32 * 1.14286).to_bits()
    );
}
