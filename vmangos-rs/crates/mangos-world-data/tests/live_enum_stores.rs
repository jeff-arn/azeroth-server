//! Live enum-store loads against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD`. Read-only.
//! Spot values were read from the bench by independent SQL.

use mangos_db::DbActor;
use mangos_world_data::enum_stores::{ItemDisplay, load_enum_stores};

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_enum_stores_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let stores = actor
        .call(|conn| load_enum_stores(conn, 10))
        .expect("load stores");

    assert_eq!(stores.item_count(), 17_707);
    assert_eq!(stores.creature_count(), 10_381);
    assert_eq!(stores.player_info_count(), 40);

    // Worn Shortsword (entry 25, single patch row).
    assert_eq!(
        stores.item_display(25),
        Some(ItemDisplay {
            display_id: 1542,
            inventory_type: 21
        })
    );
    // Entry 835 has patch rows 0/5/9, same display in all.
    assert_eq!(
        stores.item_display(835),
        Some(ItemDisplay {
            display_id: 1007,
            inventory_type: 0
        })
    );
    assert_eq!(stores.item_display(999_999), None);

    // Creature 416: pet_family 23.
    assert_eq!(stores.pet_family(416), Some(23));
    assert_eq!(stores.pet_family(999_999), None);

    assert!(stores.has_player_info(1, 1)); // human warrior
    assert!(!stores.has_player_info(1, 7)); // human shaman: invalid in vanilla
}
