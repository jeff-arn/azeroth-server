//! Live gameobject-template load against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD`. Read-only.
//! Spot values from independent SQL (2026-10-19).

use mangos_db::DbActor;
use mangos_world_data::gameobject_template::{
    GameObjectValidation, TYPE_CHEST, TYPE_DOOR, TYPE_TRANSPORT, load_gameobject_template,
    load_gameobject_templates,
};

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_gameobject_templates_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let (templates, _report) = actor
        .call(|conn| load_gameobject_templates(conn, 10, &GameObjectValidation::default()))
        .expect("load templates");

    assert_eq!(templates.len(), 8_984);

    let chest = templates.get(32).expect("Sunken Chest");
    assert_eq!(chest.go_type, TYPE_CHEST);
    assert_eq!(chest.display_id, 10);
    assert_eq!(chest.name, "Sunken Chest");
    assert_eq!(chest.data[0], 43, "chest lockId");
    assert_eq!(chest.data[1], 1683, "chest lootId");

    let door = templates.get(13965).expect("Factory Door");
    assert_eq!(door.go_type, TYPE_DOOR);
    assert_eq!(door.data[1], 85, "door lockId");

    let elevator = templates.get(4170).expect("Mesa Elevator");
    assert_eq!(elevator.go_type, TYPE_TRANSPORT);
    let display_ids = templates.transport_display_ids();
    // SQL counts 15 distinct displays; C++ then erases 462 and 562
    // ("no model on which to path", ObjectMgr.cpp:8008-8009).
    assert_eq!(display_ids.len(), 13);
    assert!(!display_ids.contains(&462));
    assert!(!display_ids.contains(&562));
    assert!(display_ids.contains(&360));

    let (single, _) = actor
        .call(|conn| load_gameobject_template(conn, 32, 10, &GameObjectValidation::default()))
        .expect("single load");
    assert_eq!(single.expect("entry 32").data[1], 1683);
    assert!(templates.get(999_999).is_none());
}
