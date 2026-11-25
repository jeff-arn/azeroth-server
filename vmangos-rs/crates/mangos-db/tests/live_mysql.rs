//! Live DB tests: `#[ignore]`d and gated on `TEST_MYSQL_URL`, run via
//! `cargo test -- --ignored` (ci.sh does this automatically when the
//! variable is set). Build the fixture DBs first:
//! `vmangos-rs/scripts/setup-dev-mysql.sh`, then
//! `source vmangos-rs/.env.test`.

use mangos_db::connect;

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn live_connect_and_count_migrations() {
    let url =
        std::env::var("TEST_MYSQL_URL").expect("TEST_MYSQL_URL must be set for live DB tests");
    let mut conn = connect(&url).expect("connect to dev MySQL");
    // The fixture script applies every *_logon.sql migration (6 at the
    // time of writing); the gate table must be non-empty.
    let migrations = conn.count_migrations().expect("count migrations");
    assert!(migrations >= 6, "expected >= 6, got {migrations}");
}
