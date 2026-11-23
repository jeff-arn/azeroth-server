#!/usr/bin/env python3
"""Bootstrap golden bench characters for the bot leveling eval bench.

See docs/BOT_LEVELING_DESIGN.md ("Snapshot and reset machinery"). Golden
characters are NOT synthesized from scratch: fresh-character SQL would
have to replicate Player::SaveNewPlayer/Player::Create exactly and is a
fidelity trap. Instead each fixture copies an existing server-created bot
character (a "donor" from the `playerbot` table) out of a source
characters database -- strictly read-only -- into a bench characters
database, then morphs the copy: quest/social/group state is left behind,
level/money/position/homebind are set from the fixture, and a `playerbot`
row registers it. Gear, spells and stats for the target level are
completed at spawn by the bot runtime (GiveLevel + spell learning), the
same morphing existing bot AIs already perform.

Safety model:
- The target database name must end in `_bench` and differ from the
  source. All INSERT/UPDATE/DELETE statements are explicitly qualified
  with the target schema; the source schema appears only in SELECTs.
- Re-runnable: an existing fixture character in the target is deleted
  (target rows for its GUID only) and re-copied.
- Credentials come from the environment (DB_HOST/DB_PORT/DB_USER/
  DB_PASSWORD, matching scripts/backup-db.sh). The password reaches the
  mysql client via MYSQL_PWD, never via argv.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

LOG = logging.getLogger("bot_bench_golden")

# Vanilla 1.12 race ids (Horde subset) and class ids used by fixtures.
RACE_NAMES = {2: "Orc", 5: "Undead", 6: "Tauren", 8: "Troll"}
CLASS_NAMES = {1: "Warrior", 3: "Hunter", 5: "Priest"}
# Legal vanilla Horde race/class combinations for the fixture classes.
LEGAL_COMBOS = {
    1: {2, 5, 6, 8},  # Warrior: all Horde races
    3: {2, 6, 8},  # Hunter: no Undead hunters in 1.12
    5: {5, 8},  # Priest: Undead or Troll
}

# Tables copied from the donor, mapped to the column that owns the row.
# Everything character-owned that is NOT listed here is deliberately left
# behind for a clean slate: character_queststatus, character_aura,
# character_spell_cooldown, character_battleground_data,
# character_honor_cp, character_instance, character_social,
# character_gifts, character_pet + pet_* (pets are re-acquired at
# runtime), corpse, group_member, guild_member, mail, mail_items,
# auction, petition*, gm_*, bot_assignments, guild_bot_roster,
# character_stats (derived; the server rewrites it on save),
# character_account_data / character_tutorial (keyed by account, not
# character).
COPY_TABLES = {
    "characters": "guid",
    "character_action": "guid",
    "character_forgotten_skills": "guid",
    "character_homebind": "guid",
    "character_inventory": "guid",
    "character_reputation": "guid",
    "character_skills": "guid",
    "character_spell": "guid",
    "item_instance": "owner_guid",
}

TARGET_DB_RE = re.compile(r"^[A-Za-z0-9_]+_bench$")
NAME_RE = re.compile(r"^[A-Za-z]{2,12}$")


@dataclass
class Fixture:
    fixture_id: str
    name: str
    class_id: int
    race_preference: list[int]
    level: int
    money_copper: int
    spawn_offset_x: float
    spawn_offset_y: float
    ai: str
    chance: int


@dataclass
class ZoneSpawn:
    zone_id: int
    map_id: int
    x: float
    y: float
    z: float
    orientation: float
    homebind_x: float
    homebind_y: float
    homebind_z: float


def load_fixtures(path: Path) -> tuple[ZoneSpawn, list[Fixture]]:
    """Load and validate fixture definitions. Raises ValueError on bad input."""
    data = json.loads(path.read_text())
    zone = data["zone"]
    spawn = ZoneSpawn(
        zone_id=int(zone["zone_id"]),
        map_id=int(zone["map_id"]),
        x=float(zone["spawn"]["position_x"]),
        y=float(zone["spawn"]["position_y"]),
        z=float(zone["spawn"]["position_z"]),
        orientation=float(zone["spawn"]["orientation"]),
        homebind_x=float(zone["homebind"]["position_x"]),
        homebind_y=float(zone["homebind"]["position_y"]),
        homebind_z=float(zone["homebind"]["position_z"]),
    )
    fixtures: list[Fixture] = []
    seen_ids: set[str] = set()
    seen_names: set[str] = set()
    for raw in data["fixtures"]:
        fixture = Fixture(
            fixture_id=str(raw["fixture_id"]),
            name=str(raw["name"]),
            class_id=int(raw["class_id"]),
            race_preference=[int(r) for r in raw["race_preference"]],
            level=int(raw["level"]),
            money_copper=int(raw["money_copper"]),
            spawn_offset_x=float(raw.get("spawn_offset", {}).get("x", 0.0)),
            spawn_offset_y=float(raw.get("spawn_offset", {}).get("y", 0.0)),
            ai=str(raw.get("ai", "PlayerBotAI")),
            chance=int(raw.get("chance", 100)),
        )
        validate_fixture(fixture)
        if fixture.fixture_id in seen_ids:
            raise ValueError(f"duplicate fixture_id: {fixture.fixture_id}")
        if fixture.name.lower() in seen_names:
            raise ValueError(f"duplicate fixture name: {fixture.name}")
        seen_ids.add(fixture.fixture_id)
        seen_names.add(fixture.name.lower())
        fixtures.append(fixture)
    if not fixtures:
        raise ValueError("no fixtures defined")
    return spawn, fixtures


def validate_fixture(fixture: Fixture) -> None:
    if not NAME_RE.match(fixture.name):
        raise ValueError(f"{fixture.fixture_id}: name {fixture.name!r} must be 2-12 letters")
    if fixture.class_id not in CLASS_NAMES:
        raise ValueError(f"{fixture.fixture_id}: unsupported class_id {fixture.class_id}")
    if not fixture.race_preference:
        raise ValueError(f"{fixture.fixture_id}: race_preference is empty")
    illegal = [r for r in fixture.race_preference if r not in LEGAL_COMBOS[fixture.class_id]]
    if illegal:
        raise ValueError(
            f"{fixture.fixture_id}: races {illegal} cannot be {CLASS_NAMES[fixture.class_id]} in vanilla 1.12"
        )
    if not 1 <= fixture.level <= 60:
        raise ValueError(f"{fixture.fixture_id}: level {fixture.level} outside 1-60")
    if fixture.money_copper < 0:
        raise ValueError(f"{fixture.fixture_id}: money_copper must be >= 0")


@dataclass
class DbConfig:
    host: str
    port: int
    user: str
    password: str
    source_db: str
    target_db: str


def load_db_config(args: argparse.Namespace) -> DbConfig:
    password = os.environ.get("DB_PASSWORD")
    if password is None:
        raise SystemExit("DB_PASSWORD is not set (export it first; empty string is accepted for local dev)")
    config = DbConfig(
        host=os.environ.get("DB_HOST", "127.0.0.1"),
        port=int(os.environ.get("DB_PORT", "3306")),
        user=os.environ.get("DB_USER", "mangos"),
        password=password,
        source_db=args.source_db,
        target_db=args.target_db,
    )
    if config.source_db == config.target_db:
        raise SystemExit("target database must differ from source database")
    if not TARGET_DB_RE.match(config.target_db):
        raise SystemExit(f"target database {config.target_db!r} must end in '_bench' (safety guard)")
    return config


def run_mysql(config: DbConfig, sql: str) -> str:
    """Run SQL through the mysql client (batch mode), returning stdout.

    The password is passed via MYSQL_PWD so it never appears in argv.
    """
    env = dict(os.environ)
    env["MYSQL_PWD"] = config.password
    argv = [
        "mysql",
        "--batch",
        "--raw",
        "-h",
        config.host,
        "-P",
        str(config.port),
        "-u",
        config.user,
        "-e",
        sql,
    ]
    result = subprocess.run(argv, capture_output=True, text=True, env=env, timeout=300)
    if result.returncode != 0:
        raise RuntimeError(f"mysql failed: {result.stderr.strip()}\nSQL: {sql[:500]}")
    return result.stdout


def sql_rows(config: DbConfig, sql: str) -> list[list[str]]:
    """Run a SELECT via headerless batch output; returns rows of strings."""
    env = dict(os.environ)
    env["MYSQL_PWD"] = config.password
    argv = [
        "mysql",
        "--batch",
        "--raw",
        "--skip-column-names",
        "-h",
        config.host,
        "-P",
        str(config.port),
        "-u",
        config.user,
        "-e",
        sql,
    ]
    result = subprocess.run(argv, capture_output=True, text=True, env=env, timeout=300)
    if result.returncode != 0:
        raise RuntimeError(f"mysql failed: {result.stderr.strip()}\nSQL: {sql[:500]}")
    rows = []
    for line in result.stdout.splitlines():
        if line:
            rows.append(line.split("\t"))
    return rows


def schema_columns(config: DbConfig, schema: str) -> dict[str, list[str]]:
    """table -> column names for one schema, via information_schema."""
    rows = sql_rows(
        config,
        "SELECT table_name, column_name FROM information_schema.columns "
        f"WHERE table_schema = '{schema}' ORDER BY table_name, ordinal_position",
    )
    tables: dict[str, list[str]] = {}
    for table, column in rows:
        tables.setdefault(table, []).append(column)
    return tables


def init_schema(config: DbConfig) -> None:
    """Create the target DB (if needed) with the source's table shapes."""
    run_mysql(config, f"CREATE DATABASE IF NOT EXISTS `{config.target_db}`")
    source_tables = schema_columns(config, config.source_db)
    target_tables = schema_columns(config, config.target_db)
    create = [
        f"CREATE TABLE IF NOT EXISTS `{config.target_db}`.`{table}` LIKE `{config.source_db}`.`{table}`"
        for table in sorted(source_tables)
        if table not in target_tables
    ]
    if create:
        run_mysql(config, ";\n".join(create))
        LOG.info("initialized %d tables in %s", len(create), config.target_db)


@dataclass
class Donor:
    guid: int
    name: str
    race: int
    level: int


def select_donor_query(source_db: str, fixture: Fixture, exclude_guids: list[int], has_guild_roster: bool) -> str:
    """SELECT for the preferred donor bot for one fixture (source, read-only)."""
    race_order = ", ".join(str(r) for r in fixture.race_preference)
    exclude = ", ".join(str(g) for g in exclude_guids) or "0"
    guild_join = ""
    guild_filter = ""
    if has_guild_roster:
        guild_join = f" LEFT JOIN `{source_db}`.`guild_bot_roster` g ON g.guid = c.guid"
        guild_filter = " AND g.guid IS NULL"
    return (
        "SELECT c.guid, c.name, c.race, c.level"
        f" FROM `{source_db}`.`characters` c"
        f" JOIN `{source_db}`.`playerbot` p ON p.char_guid = c.guid"
        f"{guild_join}"
        f" WHERE c.class = {fixture.class_id}"
        f" AND c.race IN ({race_order})"
        f" AND c.guid NOT IN ({exclude})"
        f"{guild_filter}"
        f" ORDER BY FIELD(c.race, {race_order}), c.guid ASC"
        " LIMIT 1"
    )


def select_donor(config: DbConfig, fixture: Fixture, exclude_guids: list[int], has_guild_roster: bool) -> Donor | None:
    rows = sql_rows(config, select_donor_query(config.source_db, fixture, exclude_guids, has_guild_roster))
    if not rows:
        return None
    guid, name, race, level = rows[0]
    return Donor(guid=int(guid), name=name, race=int(race), level=int(level))


# Columns reset on the morphed `characters` row, applied only when the
# target schema actually has them (base schema + migrations drift).
MORPH_RESET_ZERO = (
    "xp",
    "online",
    "played_time_total",
    "played_time_level",
    "rest_bonus",
    "is_logout_resting",
    "death_expire_time",
    "transport_guid",
    "transport_x",
    "transport_y",
    "transport_z",
    "transport_o",
    "honor_rank_points",
    "honor_highest_rank",
    "honor_standing",
    "honor_last_week_hk",
    "honor_last_week_cp",
    "honor_stored_hk",
    "honor_stored_dk",
    "drunk",
    "player_flags",
    "at_login_flags",
)


def build_copy_statements(
    source_db: str,
    target_db: str,
    donor_guid: int,
    source_tables: dict[str, list[str]],
    target_tables: dict[str, list[str]],
) -> list[str]:
    """Target-qualified DELETE + INSERT..SELECT per copied table.

    Pure function over introspected schemas so tests can exercise it
    without a database. The DELETEs make re-runs idempotent; they only
    ever name the target schema and this donor's GUID.
    """
    statements: list[str] = []
    for table, owner_column in COPY_TABLES.items():
        if table not in source_tables or table not in target_tables:
            continue
        if owner_column not in source_tables[table]:
            continue
        shared = [c for c in source_tables[table] if c in target_tables[table]]
        if not shared:
            continue
        columns = ", ".join(f"`{c}`" for c in shared)
        statements.append(f"DELETE FROM `{target_db}`.`{table}` WHERE `{owner_column}` = {donor_guid}")
        statements.append(
            f"INSERT INTO `{target_db}`.`{table}` ({columns})"
            f" SELECT {columns} FROM `{source_db}`.`{table}`"
            f" WHERE `{owner_column}` = {donor_guid}"
        )
    return statements


def build_morph_statements(
    target_db: str,
    donor_guid: int,
    fixture: Fixture,
    spawn: ZoneSpawn,
    bench_account: int,
    target_tables: dict[str, list[str]],
) -> list[str]:
    """Target-qualified UPDATEs turning the copied donor into the fixture."""
    characters_columns = set(target_tables.get("characters", []))
    sets = {
        "account": str(bench_account),
        "name": f"'{fixture.name}'",
        "level": str(fixture.level),
        "money": str(fixture.money_copper),
        "zone": str(spawn.zone_id),
        "map": str(spawn.map_id),
        "position_x": repr(spawn.x + fixture.spawn_offset_x),
        "position_y": repr(spawn.y + fixture.spawn_offset_y),
        "position_z": repr(spawn.z),
        "orientation": repr(spawn.orientation),
        "current_taxi_path": "''",
        "watched_faction": "-1",
        "deleted_account": "NULL",
        "deleted_name": "NULL",
        "deleted_time": "NULL",
    }
    for column in MORPH_RESET_ZERO:
        sets.setdefault(column, "0")
    assignments = ", ".join(f"`{col}` = {val}" for col, val in sets.items() if col in characters_columns)
    statements = [f"UPDATE `{target_db}`.`characters` SET {assignments} WHERE `guid` = {donor_guid}"]
    if "character_homebind" in target_tables:
        statements.append(
            f"REPLACE INTO `{target_db}`.`character_homebind`"
            " (`guid`, `map`, `zone`, `position_x`, `position_y`, `position_z`)"
            f" VALUES ({donor_guid}, {spawn.map_id}, {spawn.zone_id},"
            f" {spawn.homebind_x!r}, {spawn.homebind_y!r}, {spawn.homebind_z!r})"
        )
    if "playerbot" in target_tables:
        statements.append(
            f"REPLACE INTO `{target_db}`.`playerbot` (`char_guid`, `chance`, `comment`, `ai`)"
            f" VALUES ({donor_guid}, {fixture.chance}, 'golden fixture {fixture.fixture_id}', '{fixture.ai}')"
        )
    return statements


def build_target_cleanup_statements(target_db: str, guid: int, target_tables: dict[str, list[str]]) -> list[str]:
    """Deletes for one GUID inside the target DB only (name-collision heal)."""
    statements = []
    for table, owner_column in COPY_TABLES.items():
        if table in target_tables and owner_column in target_tables[table]:
            statements.append(f"DELETE FROM `{target_db}`.`{table}` WHERE `{owner_column}` = {guid}")
    if "playerbot" in target_tables:
        statements.append(f"DELETE FROM `{target_db}`.`playerbot` WHERE `char_guid` = {guid}")
    return statements


def verify_no_writes_to_source(statements: list[str], source_db: str) -> None:
    """Fail if any write statement names the source schema as its target."""
    write_re = re.compile(r"^\s*(INSERT INTO|REPLACE INTO|UPDATE|DELETE FROM)\s+`([^`]+)`", re.IGNORECASE)
    for statement in statements:
        match = write_re.match(statement)
        if match and match.group(2) == source_db:
            raise AssertionError(f"refusing to write to source schema: {statement[:120]}")


def apply_fixtures(
    config: DbConfig, spawn: ZoneSpawn, fixtures: list[Fixture], bench_account: int, dry_run: bool
) -> int:
    source_tables = schema_columns(config, config.source_db)
    target_tables = schema_columns(config, config.target_db)
    if "characters" not in source_tables or "playerbot" not in source_tables:
        raise SystemExit(f"source DB {config.source_db} lacks characters/playerbot tables")
    if "characters" not in target_tables:
        raise SystemExit(f"target DB {config.target_db} has no schema; re-run with --init-schema")
    has_guild_roster = "guild_bot_roster" in source_tables

    used_guids: list[int] = []
    failures = 0
    for fixture in fixtures:
        donor = select_donor(config, fixture, used_guids, has_guild_roster)
        if donor is None:
            LOG.error(
                "%s: no donor bot found (class %s, races %s) in %s",
                fixture.fixture_id,
                CLASS_NAMES[fixture.class_id],
                fixture.race_preference,
                config.source_db,
            )
            failures += 1
            continue
        used_guids.append(donor.guid)
        LOG.info(
            "%s: donor %s (guid %d, %s, level %d) -> %s",
            fixture.fixture_id,
            donor.name,
            donor.guid,
            RACE_NAMES.get(donor.race, donor.race),
            donor.level,
            fixture.name,
        )
        statements: list[str] = []
        # Heal a name collision in the target (fixture renamed between runs).
        collisions = sql_rows(
            config,
            f"SELECT guid FROM `{config.target_db}`.`characters` WHERE name = '{fixture.name}' AND guid <> {donor.guid}",
        )
        for (other_guid,) in collisions:
            statements += build_target_cleanup_statements(config.target_db, int(other_guid), target_tables)
        statements += build_copy_statements(
            config.source_db, config.target_db, donor.guid, source_tables, target_tables
        )
        statements += build_morph_statements(config.target_db, donor.guid, fixture, spawn, bench_account, target_tables)
        verify_no_writes_to_source(statements, config.source_db)
        if dry_run:
            for statement in statements:
                print(f"{statement};")
            continue
        run_mysql(config, ";\n".join(statements))
        copied = sql_rows(
            config,
            f"SELECT COUNT(*) FROM `{config.target_db}`.`character_inventory` WHERE guid = {donor.guid}",
        )
        LOG.info("%s: applied (%s inventory rows copied)", fixture.fixture_id, copied[0][0] if copied else "?")
    return failures


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--source-db", default="characters", help="populated characters DB to copy donors FROM (read-only)"
    )
    parser.add_argument("--target-db", required=True, help="bench characters DB to write to (must end in _bench)")
    parser.add_argument(
        "--fixtures",
        type=Path,
        default=Path(__file__).with_name("bot_bench_fixtures.json"),
        help="fixture definitions JSON",
    )
    parser.add_argument("--bench-account", type=int, default=9001, help="account id stamped on golden characters")
    parser.add_argument("--init-schema", action="store_true", help="create target DB and clone table shapes first")
    parser.add_argument("--dry-run", action="store_true", help="print SQL instead of applying it")
    parser.add_argument("--quiet", action="store_true", help="warnings and errors only")
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=logging.WARNING if args.quiet else logging.INFO,
        format="%(levelname)s %(name)s: %(message)s",
    )
    config = load_db_config(args)
    try:
        spawn, fixtures = load_fixtures(args.fixtures)
    except (ValueError, KeyError, OSError) as exc:
        raise SystemExit(f"invalid fixtures: {exc}") from exc
    if args.init_schema and not args.dry_run:
        init_schema(config)
    failures = apply_fixtures(config, spawn, fixtures, args.bench_account, args.dry_run)
    if failures:
        LOG.error("%d fixture(s) failed", failures)
        return 1
    LOG.info("done: %d fixture(s) %s", len(fixtures), "planned (dry run)" if args.dry_run else "applied")
    return 0


if __name__ == "__main__":
    sys.exit(main())
