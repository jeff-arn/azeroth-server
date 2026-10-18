//! Live spawn loads against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD`. Read-only.
//! Counts and spot values come from independent SQL (2026-10-19)
//! that encodes the C++ loader filters, not from the loaders.

use mangos_db::DbActor;
use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
use mangos_world_data::spawns::{SpawnData, cell_id, load_map_ids};

fn load_all(actor: &DbActor) -> SpawnData {
    actor
        .call(|conn| {
            let (creature_templates, _) =
                load_creature_templates(conn, 10, &TemplateValidation::default())?;
            let (go_templates, _) =
                load_gameobject_templates(conn, 10, &GameObjectValidation::default())?;
            let map_ids = load_map_ids(conn)?;
            assert_eq!(
                map_ids.len(),
                44,
                "distinct map entries (66 rows with patches)"
            );
            let mut spawns = SpawnData::new();
            spawns.load_creatures(conn, 10, &creature_templates, Some(&map_ids), false)?;
            spawns.load_gameobjects(conn, 10, &go_templates, Some(&map_ids), None, false)?;
            Ok(spawns)
        })
        .expect("spawn loads")
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_spawns_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let mut spawns = load_all(&actor);

    // Independent SQL counts (see file header).
    assert_eq!(spawns.creature_count(), 66_222);
    assert_eq!(spawns.gameobject_count(), 56_645);
    assert_eq!(spawns.grid().creature_count(), 62_760);
    assert_eq!(spawns.grid().gameobject_count(), 13_601);

    // Spot creature: guid 1, entry 2843, map 0 (SQL-fetched).
    let c = spawns.creature(1).expect("guid 1");
    assert_eq!(c.creature_id[0], 2843);
    assert_eq!(c.position.map_id, 0);
    assert_eq!(c.position.x.to_bits(), (-14467.8f32).to_bits());
    assert_eq!(c.position.y.to_bits(), 468.374f32.to_bits());
    assert_eq!((c.spawntimesecsmin, c.spawntimesecsmax), (300, 300));
    // Hand-computed home cell (f64 math over f32-promoted macros).
    assert_eq!(cell_id(c.position.x, c.position.y), 538_701);
    assert!(
        spawns
            .grid()
            .cell(0, 538_701)
            .expect("home cell")
            .creatures
            .contains(&1)
    );

    // Elevator transports register per map instead of grid-adding.
    assert!(spawns.elevator_transports(209).contains(&233));
    assert!(spawns.gameobject(233).is_some(), "transport spawn stored");

    // Reload into the same store must not double-add to the grid
    // (alreadyPresent). Templates/maps reload with it (C++ keeps
    // one ObjectMgr, so this mirrors the reload entry points).
    let templates = actor
        .call(|conn| load_creature_templates(conn, 10, &TemplateValidation::default()))
        .expect("templates")
        .0;
    let map_ids = actor.call(load_map_ids).expect("map ids");
    let taken = std::mem::take(&mut spawns);
    spawns = actor
        .call(move |conn| {
            let mut spawns = taken;
            spawns.load_creatures(conn, 10, &templates, Some(&map_ids), true)?;
            Ok(spawns)
        })
        .expect("reload");
    assert_eq!(spawns.grid().creature_count(), 62_760);
}
