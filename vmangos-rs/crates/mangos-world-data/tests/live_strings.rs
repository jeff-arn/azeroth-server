//! Live `mangos_string` load against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD` (URL must
//! point at the bench `mangos` database). Read-only.

use mangos_db::DbActor;
use mangos_world_data::strings::load_mangos_strings;

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_mangos_strings_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let strings = actor.call(load_mangos_strings).expect("load strings");
    assert!(!strings.is_empty(), "bench mangos_string must not be empty");
    // Entry 1 exists in every vmangos world dump (first LANG_* id).
    assert!(strings.get(1).is_some(), "entry 1 present");
}
