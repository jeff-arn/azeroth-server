"""Tests for scripts/bot_bench_golden.py — golden bench character bootstrap.

Static tests cover fixture validation and SQL statement construction (the
safety-critical part: writes must never target the source schema). The
integration test exercises a full donor copy against synthetic MySQL
schemas and is skipped when no local MySQL is reachable.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest
from bot_bench_golden import (
    COPY_TABLES,
    Fixture,
    ZoneSpawn,
    build_copy_statements,
    build_morph_statements,
    load_fixtures,
    validate_fixture,
    verify_no_writes_to_source,
)

FIXTURES_PATH = Path(__file__).resolve().parents[1] / "bot_bench_fixtures.json"


def make_fixture(**overrides):
    base = {
        "fixture_id": "test_warrior",
        "name": "Testname",
        "class_id": 1,
        "race_preference": [5, 2],
        "level": 11,
        "money_copper": 5000,
        "spawn_offset_x": 0.0,
        "spawn_offset_y": 0.0,
        "ai": "PlayerBotAI",
        "chance": 100,
    }
    base.update(overrides)
    return Fixture(**base)


def make_spawn():
    return ZoneSpawn(
        zone_id=130,
        map_id=0,
        x=522.449,
        y=1626.15,
        z=104.997,
        orientation=0.0,
        homebind_x=510.916,
        homebind_y=1636.47,
        homebind_z=126.027,
    )


# ---- static: fixtures -----------------------------------------------------


def test_shipped_fixtures_load():
    spawn, fixtures = load_fixtures(FIXTURES_PATH)
    assert spawn.zone_id == 130  # Silverpine Forest
    assert {f.class_id for f in fixtures} == {1, 3, 5}  # warrior/hunter/priest


def test_undead_hunter_rejected():
    with pytest.raises(ValueError, match="cannot be Hunter"):
        validate_fixture(make_fixture(class_id=3, race_preference=[5]))


def test_bad_name_rejected():
    with pytest.raises(ValueError, match="letters"):
        validate_fixture(make_fixture(name="Bad Name!"))


def test_level_out_of_range_rejected():
    with pytest.raises(ValueError, match="1-60"):
        validate_fixture(make_fixture(level=61))


# ---- static: statement construction ----------------------------------------

FAKE_TABLES = {
    "characters": ["guid", "account", "name", "level", "xp", "money", "zone", "map"],
    "character_inventory": ["guid", "bag", "slot", "item_guid"],
    "item_instance": ["guid", "owner_guid", "item_id"],
    "character_homebind": ["guid", "map", "zone", "position_x", "position_y", "position_z"],
    "playerbot": ["char_guid", "chance", "comment", "ai"],
}


def test_copy_statements_only_write_target():
    statements = build_copy_statements("characters", "characters_bench", 43, FAKE_TABLES, FAKE_TABLES)
    verify_no_writes_to_source(statements, "characters")
    writes = [s for s in statements if s.split(" ", 1)[0] in ("DELETE", "INSERT")]
    assert writes and all(
        "`characters_bench`" in s.split(" FROM ")[0] or " INTO `characters_bench`" in s for s in writes
    )


def test_copy_statements_skip_missing_tables():
    partial = {"characters": ["guid", "name"]}
    statements = build_copy_statements("src", "dst_bench", 1, partial, partial)
    assert len(statements) == 2  # DELETE + INSERT for the one shared table


def test_morph_statements_guard_on_columns():
    statements = build_morph_statements("characters_bench", 43, make_fixture(), make_spawn(), 9001, FAKE_TABLES)
    update = statements[0]
    assert "`level` = 11" in update and "`name` = 'Testname'" in update
    assert "health" not in update  # column absent from the fake schema
    verify_no_writes_to_source(statements, "characters")


def test_quest_state_is_never_copied():
    assert "character_queststatus" not in COPY_TABLES
    assert "character_pet" not in COPY_TABLES  # pets re-acquired at runtime


# ---- integration: full copy against synthetic schemas ----------------------


def _mysql(args, sql):
    env = dict(os.environ)
    env["MYSQL_PWD"] = os.environ.get("TEST_DB_PASSWORD", "")
    return subprocess.run(
        ["mysql", "--batch", "-h", "127.0.0.1", "-u", os.environ.get("TEST_DB_USER", "root"), *args, "-e", sql],
        capture_output=True,
        text=True,
        env=env,
        timeout=60,
    )


@pytest.mark.integration
def test_full_donor_copy():
    if shutil.which("mysql") is None:
        pytest.skip("mysql client not installed")
    probe = _mysql(["-N"], "SELECT 1")
    if probe.returncode != 0:
        pytest.skip("no reachable local MySQL")
    # Minimal source schema with the columns the generator touches.
    schema = (
        "CREATE TABLE characters (guid INT PRIMARY KEY, account INT, name VARCHAR(12), race INT, class INT,"
        " level INT, xp INT DEFAULT 0, money INT DEFAULT 0, zone INT DEFAULT 0, map INT DEFAULT 0,"
        " position_x FLOAT DEFAULT 0, position_y FLOAT DEFAULT 0, position_z FLOAT DEFAULT 0,"
        " orientation FLOAT DEFAULT 0, online INT DEFAULT 0);"
        "CREATE TABLE playerbot (char_guid INT PRIMARY KEY, chance INT, comment VARCHAR(255), ai VARCHAR(50));"
        "CREATE TABLE character_inventory (guid INT, bag INT, slot INT, item_guid INT);"
        "CREATE TABLE item_instance (guid INT PRIMARY KEY, owner_guid INT, item_id INT);"
        "CREATE TABLE character_homebind (guid INT PRIMARY KEY, map INT, zone INT,"
        " position_x FLOAT, position_y FLOAT, position_z FLOAT);"
        "CREATE TABLE character_queststatus (guid INT, quest INT);"
    )
    script = Path(__file__).resolve().parents[1] / "bot_bench_golden.py"
    env = dict(
        os.environ, DB_USER=os.environ.get("TEST_DB_USER", "root"), DB_PASSWORD=os.environ.get("TEST_DB_PASSWORD", "")
    )
    try:
        setup = _mysql([], f"DROP DATABASE IF EXISTS golden_src; CREATE DATABASE golden_src; USE golden_src; {schema}")
        assert setup.returncode == 0, setup.stderr
        seed = _mysql(
            ["golden_src"],
            "INSERT INTO characters (guid, account, name, race, class, level) VALUES (43, 1043, 'Donor', 5, 1, 1);"
            "INSERT INTO playerbot VALUES (43, 30, NULL, 'DynamicBotAI');"
            "INSERT INTO character_inventory VALUES (43, 0, 1, 9001);"
            "INSERT INTO item_instance VALUES (9001, 43, 25);"
            "INSERT INTO character_queststatus VALUES (43, 1880);",
        )
        assert seed.returncode == 0, seed.stderr
        run = subprocess.run(
            ["python3", str(script), "--source-db", "golden_src", "--target-db", "golden_bench", "--init-schema"],
            capture_output=True,
            text=True,
            env=env,
            timeout=120,
        )
        assert run.returncode == 0, run.stderr
        out = _mysql(
            ["-N", "golden_bench"],
            "SELECT name, level, zone FROM characters WHERE name='Brannock';"
            "SELECT COUNT(*) FROM character_queststatus;"
            "SELECT ai FROM playerbot WHERE char_guid=43;",
        )
        lines = [ln for ln in out.stdout.splitlines() if ln]
        assert lines[0].split("\t") == ["Brannock", "11", "130"]
        assert lines[1] == "0"  # donor quest progress left behind
        assert lines[2] == "PlayerBotAI"
        # Source untouched.
        src = _mysql(["-N", "golden_src"], "SELECT name, level FROM characters WHERE guid=43")
        assert src.stdout.split() == ["Donor", "1"]
    finally:
        _mysql([], "DROP DATABASE IF EXISTS golden_src; DROP DATABASE IF EXISTS golden_bench")
