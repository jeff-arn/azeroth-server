//! Byte-exact `SMSG_CHAR_ENUM` check: Rust builder fed from the
//! live bench DBs vs the independent Python oracle
//! (`vmangos-rs/scripts/char_enum_oracle_gen.py`, from C++
//! source + raw SQL). `#[ignore]`d; needs
//! `TEST_MYSQL_URL_CHARACTERS`, `TEST_MYSQL_URL_WORLD`, and
//! `CHAR_ENUM_ORACLE` (path to the generated JSON).

use std::sync::Arc;

use mangos_db::{CharacterDb, DbActor};
use mangos_world_data::enum_stores::load_enum_stores;

use mangosd::char_enum::build_char_enum;

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

fn oracle_field(json: &str, key: &str) -> String {
    let pat = format!("\"{key}\": ");
    let start = json.find(&pat).expect("key") + pat.len();
    let tail = &json[start..];
    if let Some(rest) = tail.strip_prefix('"') {
        rest[..rest.find('"').expect("end quote")].to_string()
    } else {
        tail[..tail.find([',', '}']).expect("end")]
            .trim()
            .to_string()
    }
}

#[test]
#[ignore = "requires bench DB URLs and CHAR_ENUM_ORACLE"]
fn char_enum_matches_python_oracle() {
    let characters_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("characters url");
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("world url");
    let oracle_path = std::env::var("CHAR_ENUM_ORACLE").expect("oracle path");
    let json = std::fs::read_to_string(oracle_path).expect("read oracle");

    let account_id: u32 = oracle_field(&json, "account_id").parse().expect("account");
    let expected = hex_to_bytes(&oracle_field(&json, "body_hex"));

    let character_db = Arc::new(CharacterDb::spawn(&characters_url).expect("characters db"));
    let world_actor = DbActor::spawn("test-world", &world_url).expect("world db");
    let stores = world_actor
        .call(|conn| load_enum_stores(conn, 10))
        .expect("stores");
    let rows = character_db.char_enum(account_id).expect("enum rows");

    let built = build_char_enum(&rows, &stores);
    assert_eq!(built.packet.body(), expected.as_slice());
    assert_eq!(
        usize::from(built.count),
        oracle_field(&json, "count")
            .parse::<usize>()
            .expect("count")
    );
    assert_eq!(
        built.max_level,
        oracle_field(&json, "max_level")
            .parse::<u32>()
            .expect("max level")
    );
}
