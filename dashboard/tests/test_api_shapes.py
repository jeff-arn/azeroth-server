"""Flask test client + API shape regression tests for the dashboard.

Production phase rule: the dashboard JSON API is shape-stable. Adding
fields is fine; renaming or removing them is not. These tests lock the
current response shape for each endpoint by asserting both:

1. The set of top-level keys is exactly what we expect (no removals
   without a versioned route).
2. Field types match the documented contract.

DB access is fully mocked — these tests run without a live MySQL
connection. Query correctness is out of scope here; that's phase 5.
"""

from unittest.mock import MagicMock, patch

import pytest


@pytest.fixture
def app(monkeypatch):
    """Import dashboard_app fresh and return its Flask `app` object."""
    import sys

    sys.modules.pop("dashboard_app", None)
    import importlib.util
    from pathlib import Path

    src = Path(__file__).resolve().parent.parent / "dashboard.py"
    spec = importlib.util.spec_from_file_location("dashboard_app", src)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["dashboard_app"] = mod
    spec.loader.exec_module(mod)
    return mod.app


@pytest.fixture
def client(app):
    app.config["TESTING"] = True
    return app.test_client()


@pytest.fixture
def mock_db():
    """Patch mysql.connector.connect to return a configurable mock cursor.

    Tests set `mock_db.fetchall_returns` / `mock_db.fetchone_returns` to
    queue up cursor results. The cursor is a MagicMock that returns
    them in FIFO order across .fetchall() / .fetchone() calls.
    """
    fetchall_queue = []
    fetchone_queue = []

    def make_cursor(*args, **kwargs):
        cursor = MagicMock()
        cursor.fetchall.side_effect = lambda: fetchall_queue.pop(0) if fetchall_queue else []
        cursor.fetchone.side_effect = lambda: fetchone_queue.pop(0) if fetchone_queue else None
        cursor.rowcount = 1
        return cursor

    conn = MagicMock()
    conn.cursor.side_effect = make_cursor

    with patch("mysql.connector.connect", return_value=conn):
        holder = MagicMock()
        holder.fetchall_queue = fetchall_queue
        holder.fetchone_queue = fetchone_queue
        yield holder


# ---- helpers --------------------------------------------------------------


def assert_keys(obj, expected_keys):
    """Lock the top-level keys of a dict against a fixed set."""
    assert isinstance(obj, dict), f"Expected dict, got {type(obj).__name__}"
    actual = set(obj.keys())
    expected = set(expected_keys)
    missing = expected - actual
    assert not missing, f"Missing keys: {missing} (got {actual})"


def assert_types(obj, type_spec):
    """Assert {key: type} or {key: tuple_of_types} for each field."""
    for key, expected_type in type_spec.items():
        assert key in obj, f"Missing field {key}"
        assert isinstance(obj[key], expected_type), (
            f"Field {key}: expected {expected_type}, got {type(obj[key]).__name__}"
        )


# ---- GET /api/status ------------------------------------------------------


def test_status_returns_documented_shape(client, mock_db):
    # Counts: 5 humans online, 12 bots online
    mock_db.fetchone_queue.extend([(5,), (12,)])
    resp = client.get("/api/status")
    assert resp.status_code == 200
    body = resp.get_json()
    assert_keys(body, {"online", "maintenance", "uptime_display", "players_online", "bots_online"})
    assert_types(
        body,
        {
            "online": bool,
            "maintenance": bool,
            "uptime_display": str,
            "players_online": int,
            "bots_online": int,
        },
    )


# ---- GET /api/players -----------------------------------------------------


def test_players_returns_list(client, mock_db):
    mock_db.fetchall_queue.append(
        [
            {
                "guid": 1,
                "name": "Thrall",
                "level": 60,
                "race": 2,  # Orc
                "cls": 7,  # Shaman
                "zone": 1637,  # Orgrimmar
                "money": 12345,
                "played_time_total": 90061,
                "username": "thrall_player",
            }
        ]
    )
    resp = client.get("/api/players")
    assert resp.status_code == 200
    body = resp.get_json()
    assert isinstance(body, list)
    assert len(body) == 1
    player = body[0]
    assert_keys(
        player,
        {"name", "level", "race", "class", "class_color", "zone", "faction", "money", "played_time", "account"},
    )
    assert_types(
        player,
        {
            "name": str,
            "level": int,
            "race": str,
            "class": str,
            "class_color": str,
            "zone": str,
            "faction": str,
            "money": str,
            "played_time": str,
            "account": str,
        },
    )
    # Orc → Horde, Shaman → blue class color
    assert player["faction"] == "Horde"
    assert player["class"] == "Shaman"


def test_players_returns_empty_list_on_db_error(client):
    # No mock_db — mysql.connector.connect will raise; the handler should
    # catch and return []
    with patch("mysql.connector.connect", side_effect=Exception("db down")):
        resp = client.get("/api/players")
        assert resp.status_code == 200
        assert resp.get_json() == []


# ---- GET /api/raids -------------------------------------------------------


def test_raids_returns_phase_ordered_list(client, mock_db):
    mock_db.fetchall_queue.append([])  # no disabled maps
    resp = client.get("/api/raids")
    assert resp.status_code == 200
    body = resp.get_json()
    assert isinstance(body, list)
    assert len(body) >= 1
    for raid in body:
        assert_keys(raid, {"name", "phase", "unlocked"})
        assert_types(raid, {"name": str, "phase": int, "unlocked": bool})
    # Phase-ordered: phase values are non-decreasing across the list
    phases = [r["phase"] for r in body]
    assert phases == sorted(phases)


# ---- GET /api/auctions ----------------------------------------------------


def test_auctions_shape(client, mock_db):
    mock_db.fetchone_queue.append((42,))  # total auctions
    mock_db.fetchall_queue.append([(2, 10), (3, 5)])  # quality breakdown
    resp = client.get("/api/auctions")
    assert resp.status_code == 200
    body = resp.get_json()
    assert_keys(body, {"total", "by_quality"})
    assert_types(body, {"total": int, "by_quality": list})
    for q in body["by_quality"]:
        assert_keys(q, {"name", "color", "count"})
        assert_types(q, {"name": str, "color": str, "count": int})


# ---- GET /api/bots --------------------------------------------------------


def test_bots_shape(client, mock_db):
    mock_db.fetchone_queue.append((25,))  # total bots online
    mock_db.fetchall_queue.append([(1637, 10), (12, 5)])
    resp = client.get("/api/bots")
    assert resp.status_code == 200
    body = resp.get_json()
    assert_keys(body, {"total_online", "by_zone"})
    assert_types(body, {"total_online": int, "by_zone": list})
    for entry in body["by_zone"]:
        assert_keys(entry, {"zone", "count"})


# ---- GET /api/activity ----------------------------------------------------


def test_activity_empty_log_returns_empty_list(client, tmp_path, monkeypatch):
    # Point LEVELUP_LOG to a non-existent file; handler should return []
    import dashboard_app

    monkeypatch.setattr(dashboard_app, "LEVELUP_LOG", str(tmp_path / "missing.log"))
    resp = client.get("/api/activity")
    assert resp.status_code == 200
    assert resp.get_json() == []


def test_activity_parses_levelup_lines(client, tmp_path, monkeypatch):
    import dashboard_app

    log = tmp_path / "levelup.log"
    log.write_text(
        "2025-06-01 12:34:56 INFO: Character Thrall:42 [c7 r2] reaches level 30\n"
        "2025-06-01 13:45:00 INFO: Character Botto:99 <BOT> [c1 r1] reaches level 10\n"
        "2025-06-01 14:00:00 INFO: Character Jaina:7 [c8 r1] reaches level 60\n"
    )
    monkeypatch.setattr(dashboard_app, "LEVELUP_LOG", str(log))
    resp = client.get("/api/activity")
    assert resp.status_code == 200
    events = resp.get_json()
    # Two real events; the <BOT> line is filtered out
    assert len(events) == 2
    for e in events:
        assert_keys(e, {"timestamp", "name", "class", "class_color", "level"})
        assert_types(e, {"timestamp": str, "name": str, "class": str, "class_color": str, "level": int})


# ---- POST /api/guild/bot/<guid>/spec — input validation -------------------


def _logged_in_officer(client):
    """Inject a guild officer session (rank 1) so guild_write_required passes."""
    with client.session_transaction() as sess:
        sess["account_id"] = 1
        sess["guild_rank"] = 1


def test_spec_post_requires_auth(client):
    resp = client.post("/api/guild/bot/1/spec", json={"role": "tank"})
    assert resp.status_code == 401
    assert "error" in resp.get_json()


def test_spec_post_rejects_non_officer(client):
    with client.session_transaction() as sess:
        sess["account_id"] = 1
        sess["guild_rank"] = 5  # not officer
    resp = client.post("/api/guild/bot/1/spec", json={"role": "tank"})
    assert resp.status_code == 403


def test_spec_post_rejects_invalid_role(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/bot/1/spec", json={"role": "necromancer"})
    assert resp.status_code == 400
    assert "Invalid role" in resp.get_json()["error"]


def test_spec_post_rejects_invalid_gear_tier(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/bot/1/spec", json={"gear_tier": "super_geared"})
    assert resp.status_code == 400
    assert "Invalid gear_tier" in resp.get_json()["error"]


def test_spec_post_rejects_empty_update(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/bot/1/spec", json={})
    assert resp.status_code == 400


def test_spec_post_accepts_valid_role(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/bot/1/spec", json={"role": "tank"})
    assert resp.status_code == 200
    assert resp.get_json() == {"ok": True}


@pytest.mark.parametrize("tier", ["fresh_60", "dungeon_geared", "pre_raid_bis"])
def test_spec_post_accepts_each_valid_gear_tier(client, mock_db, tier):
    _logged_in_officer(client)
    resp = client.post("/api/guild/bot/1/spec", json={"gear_tier": tier})
    assert resp.status_code == 200


# ---- POST /api/guild/gear-tier — input validation -------------------------


def test_gear_tier_post_rejects_invalid_tier(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/gear-tier", json={"gear_tier": "ultra_bis"})
    assert resp.status_code == 400


def test_gear_tier_post_requires_tier_field(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/gear-tier", json={})
    assert resp.status_code == 400


# ---- POST /api/guild/raid-plan --------------------------------------------


def test_raid_plan_post_requires_auth(client):
    resp = client.post("/api/guild/raid-plan", json={"assignments": []})
    assert resp.status_code == 401


def test_raid_plan_post_rejects_missing_assignments(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/raid-plan", json={})
    assert resp.status_code == 400


def test_raid_plan_post_accepts_empty_assignments(client, mock_db):
    _logged_in_officer(client)
    resp = client.post("/api/guild/raid-plan", json={"assignments": []})
    assert resp.status_code == 200


# ---- GET /api/addons ------------------------------------------------------


def test_addons_returns_summary_list(client, tmp_path, monkeypatch):
    import json

    import dashboard_app

    addons_file = tmp_path / "addons.json"
    addons_file.write_text(
        json.dumps(
            [
                {
                    "id": "abc12345",
                    "name": "Threat-O-Meter",
                    "description": "Threat tracking",
                    "author": "Jeff",
                    "releases": [
                        {"version": "1.0", "filename": "abc12345_v1.0.zip", "date": "2025-01-01", "changelog": "init"},
                    ],
                },
            ]
        )
    )
    monkeypatch.setattr(dashboard_app, "ADDONS_JSON", str(addons_file))
    resp = client.get("/api/addons")
    assert resp.status_code == 200
    body = resp.get_json()
    assert isinstance(body, list)
    assert len(body) == 1
    addon = body[0]
    assert_keys(
        addon,
        {"id", "name", "description", "author", "latest_version", "latest_date", "release_count"},
    )


def test_addons_post_requires_admin(client):
    resp = client.post("/api/addons", json={"name": "Test"})
    assert resp.status_code == 401


def test_addons_post_rejects_non_admin(client):
    with client.session_transaction() as sess:
        sess["account_id"] = 1
        sess["gmlevel"] = 0
    resp = client.post("/api/addons", json={"name": "Test"})
    assert resp.status_code == 403


def test_addons_post_rejects_missing_name(client):
    with client.session_transaction() as sess:
        sess["account_id"] = 1
        sess["gmlevel"] = 2
    resp = client.post("/api/addons", json={})
    assert resp.status_code == 400


# ---- /login ---------------------------------------------------------------


def test_login_get_renders_form(client):
    resp = client.get("/login")
    assert resp.status_code == 200
    assert b"<form" in resp.data.lower()


def test_login_post_bad_credentials_re_renders_form(client):
    with patch("mysql.connector.connect") as conn:
        conn.return_value.cursor.return_value.fetchone.return_value = None
        resp = client.post("/login", data={"username": "nobody", "password": "wrong"})
    assert resp.status_code == 200
    assert b"Invalid" in resp.data
