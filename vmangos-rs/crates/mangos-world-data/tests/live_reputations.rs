//! Live `faction` load against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD` (URL must
//! point at the bench `mangos` database). Read-only.

use mangos_db::DbActor;
use mangos_world_data::reputations::load_factions;

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_factions_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let factions = actor.call(load_factions).expect("load factions");
    assert!(
        !factions.is_empty(),
        "bench faction table must not be empty"
    );
    assert!(factions.get(76).is_some(), "Orgrimmar faction present");
    assert_eq!(factions.initial_slots(2, 1, &[]).len(), 64);
}
