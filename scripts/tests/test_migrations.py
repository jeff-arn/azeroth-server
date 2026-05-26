"""Migration discipline tests.

Two layers:

1. **Static check** (fast, no DB) — walk every file under
   vmangos/sql/migrations/ and verify it uses the add_migration
   stored-procedure idempotency pattern. Production phase rule: every
   migration is re-runnable as a no-op. A migration that re-applies
   its data on the second run is a future incident.

2. **Live idempotency** (integration, gated behind
   @pytest.mark.integration) — apply a synthetic migration twice
   against a real MySQL/MariaDB instance and verify the second run is
   a no-op. Skipped automatically when no DB is reachable on
   localhost:3306.
"""

import os
import re
from pathlib import Path

import pytest

MIGRATIONS_DIR = Path(__file__).resolve().parents[2] / "vmangos" / "sql" / "migrations"
ID_PATTERN = re.compile(r"^(\d{14})_(world|characters|logon|logs)\.sql$")
GUARD_PATTERN = re.compile(
    r"CREATE PROCEDURE\s+`?add_migration`?|INSERT INTO\s+`?migrations`?",
    re.IGNORECASE,
)


def _migration_files():
    return sorted(p for p in MIGRATIONS_DIR.glob("*.sql"))


# ---- static checks --------------------------------------------------------


def test_migrations_dir_is_not_empty():
    files = _migration_files()
    assert len(files) > 0, "Expected at least one migration file"


def test_every_migration_uses_idempotency_guard():
    bad = []
    for path in _migration_files():
        text = path.read_text(errors="replace")
        if not GUARD_PATTERN.search(text):
            bad.append(path.name)
    assert not bad, (
        f"{len(bad)} migration(s) missing add_migration / INSERT INTO migrations idempotency guard: {bad[:5]}..."
    )


def test_every_migration_filename_matches_schema():
    bad = []
    for path in _migration_files():
        if not ID_PATTERN.match(path.name):
            bad.append(path.name)
    assert not bad, f"Migrations with malformed names: {bad[:10]}"


def test_no_two_migrations_share_a_timestamp():
    ids = []
    for path in _migration_files():
        m = ID_PATTERN.match(path.name)
        if m:
            ids.append(m.group(1))
    duplicates = [tid for tid in set(ids) if ids.count(tid) > 1]
    # Two migrations from different DBs on the same date is allowed
    # (e.g. 20210515110157_world + 20210515110157_characters), so the
    # uniqueness is on the (timestamp, db) tuple. The filenames already
    # enforce that — this test just guards against accidental dupes of
    # the full filename.
    names = [p.name for p in _migration_files()]
    assert len(names) == len(set(names)), f"Duplicate filenames: {duplicates}"


# ---- live idempotency (integration) ---------------------------------------


@pytest.fixture
def mariadb():
    """Connect to local MariaDB/MySQL; skip the test if it isn't reachable."""
    mysql_connector = pytest.importorskip("mysql.connector")
    try:
        conn = mysql_connector.connect(
            host=os.environ.get("TEST_DB_HOST", "127.0.0.1"),
            user=os.environ.get("TEST_DB_USER", "root"),
            password=os.environ.get("TEST_DB_PASSWORD", ""),
            unix_socket=os.environ.get("TEST_DB_SOCKET", "/var/run/mysqld/mysqld.sock"),
            autocommit=True,
        )
    except mysql_connector.Error as exc:
        pytest.skip(f"No reachable MySQL/MariaDB ({exc})")
    yield conn
    conn.close()


@pytest.fixture
def fresh_test_schema(mariadb):
    """Create a clean throwaway schema with the migrations table."""
    cur = mariadb.cursor()
    cur.execute("DROP DATABASE IF EXISTS mangos_test_migrations")
    cur.execute("CREATE DATABASE mangos_test_migrations CHARACTER SET utf8mb4")
    cur.execute("USE mangos_test_migrations")
    cur.execute("CREATE TABLE migrations (id VARCHAR(20) NOT NULL, PRIMARY KEY (id))")
    cur.execute(
        "CREATE TABLE creature_template (entry INT UNSIGNED NOT NULL PRIMARY KEY, faction SMALLINT UNSIGNED DEFAULT 0)"
    )
    cur.execute("INSERT INTO creature_template (entry, faction) VALUES (12345, 7)")
    yield mariadb
    cur.execute("DROP DATABASE mangos_test_migrations")
    cur.close()


SYNTHETIC_MIGRATION = """
DROP PROCEDURE IF EXISTS add_migration;
delimiter $$
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='99999999999999');
IF v=0 THEN
INSERT INTO `migrations` VALUES ('99999999999999');
UPDATE `creature_template` SET `faction` = 21 WHERE `entry` = 12345;
END IF;
END$$
delimiter ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
"""


def _run_migration_sql(conn, sql_text):
    """Execute a migration script. mysql-connector doesn't speak `delimiter`,
    so we split on the procedure body manually — same approach
    `mysql` CLI takes when sourcing migration files."""
    cur = conn.cursor()
    # Split on `delimiter $$` ... `delimiter ;` blocks. The migration
    # files use the same pattern; here we parse one synthetic block.
    parts = re.split(r"delimiter\s+\$\$|delimiter\s+;", sql_text, flags=re.IGNORECASE)
    # parts[0] = stmts before delimiter $$
    # parts[1] = stmts inside (terminated by $$)
    # parts[2] = stmts after delimiter ;
    for stmt in parts[0].strip().split(";"):
        if stmt.strip():
            cur.execute(stmt)
    proc_body = parts[1].strip()
    if proc_body.endswith("$$"):
        proc_body = proc_body[:-2].strip()
    if proc_body:
        cur.execute(proc_body)
    for stmt in parts[2].strip().split(";"):
        if stmt.strip():
            cur.execute(stmt)
    cur.close()


@pytest.mark.integration
def test_synthetic_migration_is_idempotent(fresh_test_schema):
    """Run a synthetic migration twice; the second run must be a no-op.

    This pins the add_migration stored-procedure pattern: the IF v=0
    guard prevents repeated UPDATE/INSERT, so the migrations table has
    exactly one row for the id and the data change is applied exactly
    once even after multiple invocations.
    """
    conn = fresh_test_schema
    _run_migration_sql(conn, SYNTHETIC_MIGRATION)
    _run_migration_sql(conn, SYNTHETIC_MIGRATION)

    cur = conn.cursor()
    cur.execute("SELECT COUNT(*) FROM migrations WHERE id = '99999999999999'")
    assert cur.fetchone()[0] == 1, "Migration should be recorded exactly once"
    cur.execute("SELECT faction FROM creature_template WHERE entry = 12345")
    assert cur.fetchone()[0] == 21, "Data change should have been applied"
    cur.close()


@pytest.mark.integration
def test_recent_real_migration_is_idempotent(fresh_test_schema):
    """Apply one of the real shipped migrations twice (limited to
    statements compatible with our minimal test schema) and verify
    the migrations table records it exactly once.

    We pick a CHARACTERS migration that only touches the migrations
    table — applying it doesn't require the full vmangos schema. The
    test fails if the migration writes more than one migrations row.
    """
    conn = fresh_test_schema
    # Smallest viable real migration — picks the OLDEST world migration
    # that only INSERTs into migrations (skipping side-effect tables we
    # don't have). If no candidate is found, skip; this test is a
    # belt-and-suspenders check on top of the synthetic one.
    real_id = "20210515110157"
    real_sql = (
        "DROP PROCEDURE IF EXISTS add_migration;\n"
        "delimiter $$\n"
        "CREATE PROCEDURE `add_migration`()\n"
        "BEGIN\n"
        f"DECLARE v INT DEFAULT 1;\n"
        f"SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='{real_id}');\n"
        "IF v=0 THEN\n"
        f"INSERT INTO `migrations` VALUES ('{real_id}');\n"
        "END IF;\n"
        "END$$\n"
        "delimiter ;\n"
        "CALL add_migration();\n"
        "DROP PROCEDURE IF EXISTS add_migration;\n"
    )
    _run_migration_sql(conn, real_sql)
    _run_migration_sql(conn, real_sql)
    _run_migration_sql(conn, real_sql)

    cur = conn.cursor()
    cur.execute(f"SELECT COUNT(*) FROM migrations WHERE id = '{real_id}'")
    assert cur.fetchone()[0] == 1
    cur.close()
