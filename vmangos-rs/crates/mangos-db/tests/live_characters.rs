//! Live character-DB tests. `#[ignore]`d + gated on
//! `TEST_MYSQL_URL_CHARACTERS` (bench `characters` DB).
//! Read-only. Oracle values read from the bench snapshot by
//! independent SQL (account 5 = JEFF, one character: guid 1).

use mangos_db::CharacterDb;

fn db() -> CharacterDb {
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    CharacterDb::spawn(&url).expect("spawn")
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn char_enum_for_bench_account() {
    let rows = db().char_enum(5).expect("char enum");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.guid, 1);
    assert_eq!(row.name, "Ruikanis");
    assert_eq!(row.race, 1); // human
    assert_eq!(row.class, 1); // warrior
    assert_eq!(row.level, 19);
    assert!(row.equipment_cache.is_some());
    // Warrior: no current pet row expected in the snapshot.
    assert_eq!(row.pet_entry, None);
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn char_enum_empty_for_unknown_account() {
    let rows = db().char_enum(999_999).expect("char enum");
    assert!(rows.is_empty());
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn tutorials_and_account_data_read() {
    let db = db();
    // Shapes only: contents vary; calls must succeed read-only.
    let _tutorials = db.load_tutorials(5).expect("tutorials");
    let _global = db.load_global_account_data(5).expect("global data");
    let _per_char = db.load_character_account_data(1).expect("per-char data");
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn name_query_for_bench_character() {
    let db = db();
    let row = db.name_query(1).expect("name query").expect("Ruikanis row");
    assert_eq!(row.name, "Ruikanis");
    assert_eq!(row.race, 1);
    assert_eq!(row.gender, 0);
    assert_eq!(row.class, 1);
    assert!(
        db.name_query(999_999)
            .expect("missing name query")
            .is_none()
    );
}
