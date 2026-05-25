"""Integration tests for the dashboard against a real MySQL/MariaDB.

These hit a temporary schema with a tiny hand-rolled fixture and run
the actual SQL the dashboard issues. Catches column-name typos and
JOIN bugs that mocked-PyMySQL unit tests can't.

Gated behind @pytest.mark.integration so unit-only runs stay fast.
Skipped automatically when no DB is reachable.
"""

import importlib.util
import os
import sys
from pathlib import Path

import pytest


@pytest.fixture(scope="module")
def integration_db():
    """Set up `mangos_test_dash` + `realmd_test_dash` schemas with the
    minimum tables/rows the dashboard queries need."""
    mc = pytest.importorskip("mysql.connector")
    try:
        admin = mc.connect(
            host=os.environ.get("TEST_DB_HOST", "127.0.0.1"),
            user=os.environ.get("TEST_DB_USER", "root"),
            password=os.environ.get("TEST_DB_PASSWORD", ""),
            unix_socket=os.environ.get("TEST_DB_SOCKET", "/var/run/mysqld/mysqld.sock"),
            autocommit=True,
        )
    except mc.Error as exc:
        pytest.skip(f"No reachable MySQL/MariaDB ({exc})")

    cur = admin.cursor()
    for db in ("mangos_test_dash_characters", "mangos_test_dash_realmd"):
        cur.execute(f"DROP DATABASE IF EXISTS {db}")
        cur.execute(f"CREATE DATABASE {db} CHARACTER SET utf8mb4")

    cur.execute("USE mangos_test_dash_characters")
    cur.execute(
        "CREATE TABLE characters ("
        "guid INT UNSIGNED NOT NULL PRIMARY KEY,"
        "account INT UNSIGNED NOT NULL,"
        "name VARCHAR(64) NOT NULL,"
        "level TINYINT UNSIGNED NOT NULL,"
        "race TINYINT UNSIGNED NOT NULL,"
        "class TINYINT UNSIGNED NOT NULL,"
        "zone INT UNSIGNED NOT NULL,"
        "money INT UNSIGNED NOT NULL,"
        "played_time_total INT UNSIGNED NOT NULL,"
        "online TINYINT UNSIGNED NOT NULL DEFAULT 0"
        ")"
    )

    cur.execute("USE mangos_test_dash_realmd")
    cur.execute("CREATE TABLE account (id INT UNSIGNED NOT NULL PRIMARY KEY,username VARCHAR(32) NOT NULL)")

    # Seed: 1 human online (Alliance Mage), 1 human offline, 1 bot online
    cur.execute("INSERT INTO account VALUES (1, 'humanaccount'), (2, 'offlineplayer'), (101, 'botaccount')")
    cur.execute("USE mangos_test_dash_characters")
    cur.execute(
        "INSERT INTO characters VALUES "
        "(10, 1, 'Jaina', 60, 1, 8, 1519, 1234567, 90000, 1),"
        "(11, 2, 'OfflineDude', 30, 2, 1, 1637, 500, 50000, 0),"
        "(20, 101, 'BotPally', 50, 3, 2, 1537, 999, 100, 1)"
    )

    yield admin

    cur.execute("DROP DATABASE mangos_test_dash_characters")
    cur.execute("DROP DATABASE mangos_test_dash_realmd")
    cur.close()
    admin.close()


@pytest.fixture
def dashboard_with_test_db(integration_db, monkeypatch):
    """Import dashboard_app and rewire DB_CONFIG + get_db to the test DBs."""
    os.environ.setdefault("DB_PASSWORD", "test-only-dummy")
    sys.modules.pop("dashboard_app", None)
    src = Path(__file__).resolve().parent.parent / "dashboard.py"
    spec = importlib.util.spec_from_file_location("dashboard_app", src)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["dashboard_app"] = mod
    spec.loader.exec_module(mod)

    # Point both database lookups at the test schemas.
    test_config = {
        "host": os.environ.get("TEST_DB_HOST", "127.0.0.1"),
        "user": os.environ.get("TEST_DB_USER", "root"),
        "password": os.environ.get("TEST_DB_PASSWORD", ""),
        "port": int(os.environ.get("TEST_DB_PORT", "3306")),
        "unix_socket": os.environ.get("TEST_DB_SOCKET", "/var/run/mysqld/mysqld.sock"),
    }
    monkeypatch.setattr(mod, "DB_CONFIG", test_config)

    DB_NAME_MAP = {
        "characters": "mangos_test_dash_characters",
        "realmd": "mangos_test_dash_realmd",
    }
    original_get_db = mod.get_db

    def get_db_routed(database):
        return original_get_db(DB_NAME_MAP.get(database, database))

    monkeypatch.setattr(mod, "get_db", get_db_routed)

    # The JOIN in /api/players is "JOIN realmd.account a" — qualify with
    # the schema name. Patch the SQL to use the test schema's realmd.

    return mod


@pytest.mark.integration
def test_players_endpoint_returns_only_online_humans(dashboard_with_test_db):
    """/api/players should return Jaina (online human) but exclude
    OfflineDude (offline) and BotPally (account >= 100). The actual
    SQL runs against the test schema; this catches column-name typos
    and JOIN errors that the mocked unit test couldn't."""
    mod = dashboard_with_test_db
    # The query JOINs "realmd.account" — substitute that for our test schema.
    # We patch via a wrapper that rewrites SQL before execute().
    original_connect = mod.mysql.connector.connect

    class RewritingCursor:
        def __init__(self, real):
            self._real = real

        def execute(self, sql, params=None):
            rewritten = sql.replace("realmd.account", "mangos_test_dash_realmd.account")
            return self._real.execute(rewritten, params) if params is not None else self._real.execute(rewritten)

        def __getattr__(self, name):
            return getattr(self._real, name)

    class RewritingConn:
        def __init__(self, real):
            self._real = real

        def cursor(self, *args, **kwargs):
            return RewritingCursor(self._real.cursor(*args, **kwargs))

        def __getattr__(self, name):
            return getattr(self._real, name)

    def connect_rewriting(**kwargs):
        return RewritingConn(original_connect(**kwargs))

    import unittest.mock

    with unittest.mock.patch.object(mod.mysql.connector, "connect", side_effect=connect_rewriting):
        mod.app.config["TESTING"] = True
        client = mod.app.test_client()
        resp = client.get("/api/players")

    assert resp.status_code == 200
    players = resp.get_json()
    assert isinstance(players, list)
    names = [p["name"] for p in players]
    assert "Jaina" in names
    assert "OfflineDude" not in names  # offline
    assert "BotPally" not in names  # account >= 100
    assert len(players) == 1

    jaina = players[0]
    assert jaina["level"] == 60
    assert jaina["class"] == "Mage"
    assert jaina["race"] == "Human"
    assert jaina["faction"] == "Alliance"
    # money: 1234567 copper = 123g 45s 67c
    assert jaina["money"] == "123g 45s 67c"


@pytest.mark.integration
def test_status_endpoint_counts_players_and_bots_separately(dashboard_with_test_db):
    """The dashboard distinguishes humans (account<100) from bots
    (account>=100) via the same SQL — verify against real rows."""
    mod = dashboard_with_test_db
    mod.app.config["TESTING"] = True
    client = mod.app.test_client()
    resp = client.get("/api/status")
    assert resp.status_code == 200
    body = resp.get_json()
    # Jaina is online (human), BotPally is online (bot), OfflineDude offline.
    assert body["players_online"] == 1
    assert body["bots_online"] == 1
