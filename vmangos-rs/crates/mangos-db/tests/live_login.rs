//! Live login-DB tests for the Phase 1 actor: account creation with
//! Rust-computed SRP v/s (the rows `dashboard/tests/test_auth.py`
//! semantics must verify), the proof-success session stamp, and the
//! wrong-password autoban writes. `#[ignore]`d + `TEST_MYSQL_URL`-gated
//! like `live_mysql.rs`; fixture setup via `scripts/setup-dev-mysql.sh`.

use mangos_db::LoginDb;
use mangos_protocol::srp::{
    bn_from_hex, bn_to_hex, calculate_verifier, generate_salt, sha_pass_hash,
};
use mysql::prelude::Queryable;

const USERNAME: &str = "RUSTLIVETEST";
const PASSWORD: &str = "hunter2case";

fn url() -> String {
    std::env::var("TEST_MYSQL_URL").expect("TEST_MYSQL_URL must be set for live DB tests")
}

/// Raw connection for raw asserts (columns the actor API does not read).
fn raw_conn() -> mysql::Conn {
    mysql::Conn::new(mysql::Opts::from_url(&url()).expect("url")).expect("raw connect")
}

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn live_account_create_verify_and_session_stamp() {
    let db = LoginDb::spawn(&url()).expect("spawn login db");
    let _ = db.delete_account_row(USERNAME.to_string());

    // Create via the same math C++ AccountMgr + dashboard share.
    let salt = generate_salt();
    let verifier = calculate_verifier(&sha_pass_hash(USERNAME, PASSWORD), &salt);
    let (v_hex, s_hex) = (bn_to_hex(&verifier), bn_to_hex(&salt));
    db.create_account(USERNAME.to_string(), v_hex.clone(), s_hex.clone())
        .expect("create account");

    // Read back: identical uppercase hex, parseable, non-zero.
    let row = db
        .account_by_username(USERNAME.to_string())
        .expect("lookup")
        .expect("account exists");
    assert_eq!(row.v, v_hex, "v stored uppercase hex verbatim");
    assert_eq!(row.s, s_hex, "s stored uppercase hex verbatim");
    assert_eq!(bn_from_hex(&row.v), Some(verifier.clone()));
    assert_eq!(bn_from_hex(&row.s), Some(salt.clone()));
    assert!(s_hex.len() == 64, "salt always 32 bytes (BN_rand top bit)");

    // Re-derive the verifier from the stored salt: the exact check
    // dashboard/auth.py _verify_srp6_password performs.
    let rederived = calculate_verifier(&sha_pass_hash(USERNAME, PASSWORD), &salt);
    assert_eq!(rederived, verifier, "password verifies against stored row");
    let wrong = calculate_verifier(&sha_pass_hash(USERNAME, "wrongpass"), &salt);
    assert_ne!(wrong, verifier, "wrong password must not verify");

    // Proof-success stamp lands all seven columns.
    let key_hex = "AB".repeat(40);
    db.update_session_on_proof(
        USERNAME.to_string(),
        key_hex.clone(),
        "203.0.113.9".to_string(),
        0,
        "Win".to_string(),
        "x86".to_string(),
    )
    .expect("session stamp");
    let (stored_key, account_id) = db
        .sessionkey_by_username(USERNAME.to_string())
        .expect("sessionkey read")
        .expect("row");
    assert_eq!(stored_key, key_hex);
    assert_eq!(account_id, row.id);
    let after = db
        .account_by_username(USERNAME.to_string())
        .expect("lookup")
        .expect("row");
    assert_eq!(after.last_ip, "203.0.113.9");

    // Wrong-password bookkeeping + autoban rows.
    let (id1, failed1) = db
        .record_failed_login(USERNAME.to_string())
        .expect("record failed")
        .expect("row");
    assert_eq!((id1, failed1), (row.id, 1));
    db.insert_account_autoban(row.id, 600)
        .expect("account autoban");
    let ban = db.active_account_ban(row.id).expect("ban read");
    assert!(ban.is_some(), "autoban row active");
    let (bandate, unbandate) = ban.expect("ban");
    assert_eq!(unbandate - bandate, 600);

    db.insert_ip_autoban("203.0.113.9".to_string(), 600)
        .expect("ip autoban");
    assert!(
        db.ip_ban_unbandate("203.0.113.9".to_string())
            .expect("ip ban read")
            .is_some(),
        "ip ban active"
    );

    // Cleanup (test-only deletes; never a runtime path).
    let mut raw = raw_conn();
    raw.exec_drop("DELETE FROM `account_banned` WHERE `id` = ?", (row.id,))
        .expect("cleanup account_banned");
    raw.exec_drop("DELETE FROM `ip_banned` WHERE `ip` = ?", ("203.0.113.9",))
        .expect("cleanup ip_banned");
    db.delete_account_row(USERNAME.to_string())
        .expect("cleanup account");
}

#[test]
#[ignore = "requires TEST_MYSQL_URL pointing at the dev fixture DB"]
fn live_migration_gate_and_realm_reads() {
    let db = LoginDb::spawn(&url()).expect("spawn login db");
    let (applied, db_name) = db.applied_migrations().expect("migrations");
    assert!(applied.len() >= 6, "fixture applies all *_logon migrations");
    assert!(!db_name.is_empty());

    let clients = db.allowed_clients().expect("allowed clients");
    assert!(
        clients.iter().any(|c| c.build == 5875 && c.os == "Win"),
        "1.12.1 build present in fixture allowed_clients"
    );
}
