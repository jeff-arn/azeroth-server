//! Live `character_social` write/read test. `#[ignore]`d +
//! gated on `TEST_MYSQL_URL_CHARACTERS` (bench `characters`
//! DB). Writes only between the two bot fixture characters
//! (guid 3 Zulrath, guid 4 Morka) and deletes its row again,
//! so the bench state is unchanged after the run.

use mangos_db::DbError;
use mangos_db::actor::DbActor;
use mangos_db::row::Fields;
use mangos_db::social::{
    add_social_flag, clear_social_flag, delete_social, insert_social, load_player_data_by_name,
};
use mysql::prelude::Queryable;

fn actor() -> DbActor {
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    DbActor::spawn("test-characters", &url).expect("spawn")
}

fn read_flags(conn: &mut mysql::Conn, guid: u32, friend: u32) -> Result<Option<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `flags` FROM `character_social` WHERE `guid` = ? AND `friend` = ?",
        (guid, friend),
    )?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| Fields::from_row(row).u32(0)))
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn social_write_sequence_round_trips() {
    let observed = actor()
        .call(|conn| {
            delete_social(conn, 3, 4)?;
            insert_social(conn, 3, 4, 0x01)?;
            let after_insert = read_flags(conn, 3, 4)?;
            add_social_flag(conn, 3, 4, 0x02)?;
            let after_add = read_flags(conn, 3, 4)?;
            clear_social_flag(conn, 3, 4, 0x01)?;
            let after_clear = read_flags(conn, 3, 4)?;
            delete_social(conn, 3, 4)?;
            let after_delete = read_flags(conn, 3, 4)?;
            Ok((after_insert, after_add, after_clear, after_delete))
        })
        .expect("social write sequence");
    assert_eq!(observed, (Some(0x01), Some(0x03), Some(0x02), None));
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn player_data_by_name_matches_bench() {
    let by_name = actor()
        .call(|conn| load_player_data_by_name(conn, "Ruikanis"))
        .expect("lookup");
    let data = by_name.expect("Ruikanis exists on the bench");
    assert_eq!(data.guid_low, 1);
    assert_eq!(data.race, 1); // human

    let missing = actor()
        .call(|conn| load_player_data_by_name(conn, "Nosuchchar"))
        .expect("lookup");
    assert_eq!(missing, None);
}
