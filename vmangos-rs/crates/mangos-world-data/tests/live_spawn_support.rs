//! Live spawn-support loads against the dev-bench DBs.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD` /
//! `TEST_MYSQL_URL_CHARACTERS`. Read-only. Counts and spot
//! values come from independent SQL (2026-10-19) that encodes
//! the C++ loader algorithms, not from the loaders.

use std::collections::HashMap;

use mangos_db::DbActor;
use mangos_world_data::spawn_support::{
    CLS_PLACEHOLDER, load_class_level_stats, load_creature_addons, load_display_info_addons,
    load_equipment_templates, load_respawn_times,
};
use mangos_world_data::spawns::load_map_ids;

/// `WowPatch` 1.12.1 index used across the live suites.
const WOW_PATCH: u32 = 10;
/// `SUPPORTED_CLIENT_BUILD` for the 5875 client.
const BUILD: u32 = 5875;

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_addons_equip_cls_display_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");

    let (addons, equip, cls, display) = actor
        .call(|conn| {
            let (addons, addon_report) = load_creature_addons(conn, WOW_PATCH, None, None, None)?;
            let (equip, equip_report) = load_equipment_templates(conn, WOW_PATCH, None)?;
            let (cls, cls_report) = load_class_level_stats(conn)?;
            let (display, display_report) = load_display_info_addons(conn, BUILD, None, None)?;
            Ok((
                addons,
                (equip, equip_report.len()),
                (cls, cls_report.len()),
                (display, addon_report.len(), display_report.len()),
            ))
        })
        .expect("support loads");
    let (equip, equip_errors) = equip;
    let (cls, cls_errors) = cls;
    let (display, addon_errors, display_errors) = display;

    // Creature addons: progressive rows (max patch <= 10).
    assert_eq!(addons.len(), 3_404);
    // Only the two skipped-validation notices (no id sets given).
    assert_eq!(addon_errors, 2);
    let a5 = addons.get(5).expect("addon guid 5");
    assert_eq!(a5.display_id, 0);
    // mount_display_id -1 read as GetUInt32 wraps like C++.
    assert_eq!(a5.mount_display_id, u32::MAX);
    assert_eq!(a5.equipment_id, -1);
    assert_eq!((a5.stand_state, a5.sheath_state, a5.emote_state), (0, 2, 0));
    assert!(a5.auras.is_empty());
    // Aura parsing: guid 3277 has `auras = '12380'`.
    assert_eq!(addons.get(3277).expect("addon 3277").auras, vec![12380]);

    // Equipment: store holds entries with >= 1 usable row; a
    // probability-0 row after nonzero rows flushes the entry
    // (rows arrive probability-DESC, so it ends empty).
    assert_eq!(equip.len(), 5_124);
    assert_eq!(equip.total_entry_rows(), 5_117);
    let flushed = equip.get(646).expect("equip 646 present but flushed");
    assert!(flushed.entries.is_empty());
    assert_eq!(flushed.total_probability, 0);
    let e6 = equip.get(6).expect("equip 6");
    assert_eq!(e6.entries.len(), 1);
    assert_eq!(e6.entries[0].item, [5276, 0, 0]);
    assert_eq!(e6.entries[0].probability, 1);
    // Bench probabilities do not sum to 100 anywhere, so every
    // template reports the total-probability error (>= len).
    assert!(equip_errors >= 5_124, "equip errors {equip_errors}");

    // CLS: classes from creature_template are {1, 2, 8}; every
    // class tables 100 levels (max(MAX_LEVEL, template max)).
    assert_eq!(cls.class_count(), 3);
    assert_eq!(cls.class_table_len(1), 100);
    let l1 = cls.get(1, 1);
    assert_eq!(l1.melee_damage.to_bits(), 1.57945f32.to_bits());
    assert_eq!(l1.ranged_damage.to_bits(), 1.37945f32.to_bits());
    assert_eq!((l1.attack_power, l1.health, l1.base_health), (24, 42, 20));
    let l63 = cls.get(1, 63);
    assert_eq!((l63.health, l63.base_health, l63.armor), (3331, 1894, 3625));
    // Level 64 is extrapolated: 42 + (63/62)*(3331-42) truncates.
    assert_eq!(cls.get(1, 64).health, 3384);
    // Level clamps to the table size; unknown class -> placeholder.
    assert_eq!(cls.get(1, 10_000), cls.get(1, 100));
    assert_eq!(cls.get(99, 10), CLS_PLACEHOLDER);
    assert_eq!(cls_errors, 0, "bench CLS rows are all valid");

    // Display addons.
    assert_eq!(display.creature_len(), 10_534);
    assert_eq!(display.gameobject_len(), 1_618);
    let d4 = display.creature(4).expect("creature display 4");
    assert_eq!(
        (
            d4.bounding_radius.to_bits(),
            d4.combat_reach.to_bits(),
            d4.gender
        ),
        (2.0f32.to_bits(), 3.0f32.to_bits(), 2)
    );
    assert_eq!(d4.speed_run.to_bits(), 1.14286f32.to_bits());
    let g1 = display.gameobject(1).expect("go display 1");
    assert_eq!(g1.min_x.to_bits(), (-0.260_733f32).to_bits());
    assert_eq!(g1.max_z.to_bits(), 0.686_414f32.to_bits());
    // Only the two skipped-validation notices.
    assert_eq!(display_errors, 2);
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD and TEST_MYSQL_URL_CHARACTERS"]
fn bench_respawn_times_load_read_only() {
    let world = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let chars =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    let world_actor = DbActor::spawn("test-world", &world).expect("spawn world");
    // guid -> spawn map from raw SQL (the loader's input
    // contract), independent of the spawn loaders.
    let (creature_maps, go_maps, map_ids) = world_actor
        .call(|conn| {
            use mysql::prelude::Queryable;
            let map_ids = load_map_ids(conn)?;
            let creature: Vec<(u32, u32)> = conn.query("SELECT `guid`, `map` FROM `creature`")?;
            let go: Vec<(u32, u32)> = conn.query("SELECT `guid`, `map` FROM `gameobject`")?;
            Ok((
                creature.into_iter().collect::<HashMap<u32, u32>>(),
                go.into_iter().collect::<HashMap<u32, u32>>(),
                map_ids,
            ))
        })
        .expect("spawn maps");

    let chars_actor = DbActor::spawn("test-chars", &chars).expect("spawn chars");
    // The bench's own 25 expired rows were deleted by the
    // first Slice-3 server boot (approved write), so the
    // test seeds its rows in a rolled-back transaction:
    // one valid creature row (guid 2294 spawns on map 0),
    // one expired row (filtered by now), and one GO row
    // whose map mismatches its spawn (dropped silently).
    let times = chars_actor
        .call(move |conn| {
            use mysql::prelude::Queryable;
            conn.query_drop("START TRANSACTION")
                .map_err(mangos_db::DbError::Driver)?;
            let seed = |conn: &mut mysql::Conn| -> Result<(), mangos_db::DbError> {
                // Bench servers accumulate real respawn rows;
                // clear them only inside this rolled-back
                // transaction so the seeded rows are isolated.
                conn.exec_drop("DELETE FROM `creature_respawn`", ())
                    .map_err(mangos_db::DbError::Driver)?;
                conn.exec_drop("DELETE FROM `gameobject_respawn`", ())
                    .map_err(mangos_db::DbError::Driver)?;
                conn.exec_drop(
                    "INSERT INTO `creature_respawn` (`guid`, `respawn_time`, `instance`, `map`) \
                     VALUES (2294, 1792196641, 0, 0), (2299, 100, 0, 0)",
                    (),
                )
                .map_err(mangos_db::DbError::Driver)?;
                conn.exec_drop(
                    "INSERT INTO `gameobject_respawn` (`guid`, `respawn_time`, `instance`, `map`) \
                     VALUES (1, 1792196641, 0, 999)",
                    (),
                )
                .map_err(mangos_db::DbError::Driver)?;
                Ok(())
            };
            let result = seed(conn)
                .and_then(|()| load_respawn_times(conn, 1_000, &creature_maps, &go_maps, &map_ids));
            conn.query_drop("ROLLBACK")
                .map_err(mangos_db::DbError::Driver)?;
            result
        })
        .expect("respawn load");
    assert_eq!(times.creature_len(), 1);
    assert_eq!(times.gameobject_len(), 0);
    let spot = times.creature(0, 2294).expect("respawn 2294");
    assert_eq!(spot.respawn_time, 1_792_196_641);
    assert_eq!(spot.map_id, 0);
    assert_eq!(spot.reset_time, 0);
}
