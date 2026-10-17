//! Live tests for the world-auth login-DB surface and the
//! migration gate. `#[ignore]`d + `TEST_MYSQL_URL`-gated
//! (URL must point at the dev-bench `realmd` database).
//! Read-only: no test in this file writes.

use mangos_db::migrations::{MigrationDb, check_required_migrations};
use mangos_db::{WorldAuthDb, connect};

fn url() -> String {
    std::env::var("TEST_MYSQL_URL").expect("TEST_MYSQL_URL must be set for live DB tests")
}

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn logon_gate_passes_on_bench() {
    let db = WorldAuthDb::spawn(&url()).expect("spawn");
    let (db_name, outcome) = db
        .actor()
        .call(|conn| check_required_migrations(conn, MigrationDb::Logon))
        .expect("gate query");
    assert_eq!(db_name, "realmd");
    assert!(
        outcome.passes(),
        "bench realmd is missing migrations: {:?}",
        outcome.missing
    );
    assert!(
        outcome.extra.is_empty(),
        "unexpected extras: {:?}",
        outcome.extra
    );
}

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn world_auth_lookup_respects_last_login_window() {
    let db = WorldAuthDb::spawn(&url()).expect("spawn");
    // The bench snapshot's real accounts logged in long ago, so
    // DATEDIFF(NOW(), last_login) < 1 hides them from world auth
    // exactly as in C++ — a read-only proof against real data.
    let stale = db
        .lookup_world_account(1, "ADMIN".to_string())
        .expect("lookup");
    assert!(stale.is_none(), "stale-login account must not be found");
    let missing = db
        .lookup_world_account(1, "P3_NO_SUCH_ACCOUNT".to_string())
        .expect("lookup");
    assert!(missing.is_none());
    assert!(
        !db.is_ip_banned("203.0.113.1".to_string()).expect("ip ban"),
        "documentation-range IP must not be banned on the bench"
    );
}

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn gate_probe_via_plain_conn() {
    // The Phase 0 live probe still works alongside the gate.
    let mut conn = connect(&url()).expect("connect");
    let count = conn.count_migrations().expect("count");
    assert_eq!(count, 6, "bench realmd migrations census");
}
