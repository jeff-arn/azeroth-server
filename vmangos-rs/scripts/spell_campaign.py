#!/usr/bin/env python3
"""Level-60 all-class spell campaign harness.

Implements `docs/SPELL_CAMPAIGN_ARCHITECTURE.md` (topology §1, fixtures
§2, dummies §3, scenarios §5, verdicts/artifacts §6, pilot gate §7,
parallel-safety §9). Reads cast rows from `spell_campaign_matrix.json`
(the `spell_matrix_gen.py` artifact); this file never queries
`spell_template` itself.

Subcommands (run from the repository root):

* `preflight` -- loopback guards, bench reachability, entry-327 probe,
  both probe self-tests. Exit 2 on any guard refusal.
* `fixtures` -- idempotent `ensure` of all nine §2 fixture
  accounts/characters (slot order; pilot legs only use their own
  three): account rows and the `CMSG_CHAR_CREATE`
  sender for missing characters only. Level/placement/spell seeding
  and drift checks run inside `pilot` before each leg. No `GM`
  opcodes exist server-side; all resets are logout + SQL + login.
* `pilot` -- the §7 gate: Warrior (5242/6546), Mage (168/133 + health),
  Druid (5187 heal on SQL-damaged ally, 5487/768 forms), one
  corpse-bucket classification (20484), one wrong-form Growl attempt
  (6795), one dummy-reset leg. Prints `SPELL-CAMPAIGN-PILOT-PASS`
  only when no unexpected FAIL/BLOCKED row remains.
* `run --spell <id> --char <name>` -- single-spell repro for the §8
  fix loop (same verdict/record path as the pilot).
* `run-class <Class> [--matrix <path>] [--only <id> ...]` --
  data-driven full-class run: every generated cast row for the class,
  serially, with per-bucket senders (self/hostile/ally/ground/item),
  the class gate, and `results-<Class>.jsonl` / `run-<Class>.log` /
  `manifest-<Class>.json` artifacts. Pilot classes read the pilot
  matrix; other classes read `spell_campaign_matrix-<Class>.json`
  unless `--matrix` overrides it. `--only` restricts to a smoke
  subset. Classify-only rows ride as BLOCKED without a cast.
* `--self-test` -- offline invariant checks (verdict enum, artifact
  writers, packet builders, raw-guid math). No server needed. Prints
  `SPELL-CAMPAIGN-SELFTEST-PASS`.

A spell passes only when the scripted client casts it and the intended
observable outcome is verified on the wire or in tracked state.
`SMSG_SPELL_GO` alone, `Started` alone, and "no error logged" are never
PASS evidence (known vacuity traps from the existing suites).

Bench/local/Colima only: every MySQL and world endpoint must resolve
to loopback or the harness refuses with exit 2. Never production.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import select
import socket
import struct
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import phase3_scripted_login as p3  # noqa: E402
import two_client_smoke as smoke  # noqa: E402
from scripted_driver import (  # noqa: E402
    SMSG_CAST_RESULT,
    SMSG_SPELL_GO,
    UNIT_FIELD_HEALTH,
    UNIT_FIELD_LEVEL,
    UNIT_FIELD_MAXHEALTH,
    ScriptedDriver,
)

UNIT_FIELD_POWER1 = 23
UNIT_FIELD_MAXPOWER1 = 30
#: Far-transfer train the cross-map leg verifies instead of an
#: object/damage outcome (`world_actor.teleport_player_to_key`:
#: TRANSFER_PENDING then NEW_WORLD; the scripted driver never ACKs
#: the worldport, so the session is always reseeded after).
SMSG_TRANSFER_PENDING = 63
SMSG_NEW_WORLD = 62
SPELL_FAILED_NO_POWER = 77
SPELL_FAILED_ONLY_SHAPESHIFT = 86
SPELL_FAILED_NOT_SHAPESHIFT = 61
SPELL_FAILED_OUT_OF_RANGE = 89
SPELL_FAILED_NOT_READY = 60
SPELL_FAILED_NOT_BEHIND = 51
SPELL_FAILED_BAD_TARGETS = 10
SPELL_FAILED_TARGETS_DEAD = 101
#: Death/resurrect wire shapes for the resurrect leg
#: (fork `Opcodes.cpp` 0x15B/0x15C, `ObjectDefines.h`).
CMSG_REPOP_REQUEST = 346
SMSG_RESURRECT_REQUEST = 347
CMSG_RESURRECT_RESPONSE = 348
MSG_MOVE_TELEPORT_ACK = 199
TYPEID_CORPSE = 7
TYPEID_GAMEOBJECT = 5
TYPEID_DYNAMICOBJECT = 6
TYPEID_UNIT = 3
#: `UNIT_CREATED_BY_SPELL` update-field index (1.12.1 table).
UNIT_CREATED_BY_SPELL = 146
CORPSE_FIELD_OWNER = 6
#: NO_POWER retry budget shared by every cast leg (self, unit,
#: hostile, ground, blessing, heal). A fixture's pool refills from
#: empty in ~100 s at bench regen, so rows behind expensive
#: shapeshift/channeled casts (Dire Bear, Hurricane) pace out
#: instead of failing. Rows that pass first try pay nothing, so
#: pilot behavior is unchanged.
POWER_RETRIES = 12

SCRIPT_DIR = pathlib.Path(__file__).parent
EVIDENCE_DIR = pathlib.Path("docs/rust-migration-evidence/spell-campaign")
MATRIX_PATH = SCRIPT_DIR / "spell_campaign_matrix.json"
LOCK_PATH = EVIDENCE_DIR / "RUNNING.lock"

# Pilot classes read the pilot matrix (byte-stable); every other
# class reads its per-class fan-out artifact, which full mode writes
# without ever touching MATRIX_PATH (mirrors the generator's
# class_matrix_path).
PILOT_MATRIX_CLASSES = ("Warrior", "Mage", "Druid")


def matrix_path_for(class_name: str) -> pathlib.Path:
    """Default matrix file backing one class: the pilot matrix for
    pilot classes, else the generator's per-class fan-out path."""
    if class_name in PILOT_MATRIX_CLASSES:
        return MATRIX_PATH
    try:
        import spell_matrix_gen as gen  # noqa: PLC0415

        return gen.class_matrix_path(class_name)
    except (ImportError, AttributeError):
        return SCRIPT_DIR / f"spell_campaign_matrix-{class_name}.json"


CMSG_CHAR_CREATE = 54
SMSG_CHAR_CREATE = 58
CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CHAR_CREATE_SUCCESS = 46
SMSG_SPELL_START = 305
SMSG_SPELLNONMELEEDAMAGELOG = 592
SMSG_PERIODICAURALOG = 590
SMSG_SPELLHEALLOG = 336
CMSG_SET_ACTIVE_MOVER = 618

AURA_FIELD_FIRST = 47
AURA_FIELD_LAST = 102

# (account, char name, race, class) per architecture §2 fixture table.
# Camppaladin (SPELLCAMP02, Human Paladin) is the union-seeded
# single-race fixture for the Paladin full-rank audit: per-race G1
# sets are both 160 (Human 1 and Dwarf 3 identical), so union ==
# intersection and one Human fixture legally covers the trainable set.
PILOT_FIXTURES = [
    ("SPELLCAMP01", "Campwarrior", 1, 1),  # Human Warrior
    ("SPELLCAMP07", "Campmage", 1, 8),  # Human Mage
    ("SPELLCAMP09", "Campdruid", 4, 11),  # NightElf Druid
    ("SPELLCAMP02", "Camppaladin", 1, 2),  # Human Paladin
]

# Full 9-class fixture table per architecture §2 (slot order). Only
# the Paladin slot is fanned out in this phase; the other slots are
# seeded identically (offline SQL + login) so later classes need no
# fixture-format change. The pilot legs keep using PILOT_FIXTURES
# rows verbatim, so `pilot` behavior is unchanged.
CLASS_FIXTURES = [
    ("SPELLCAMP01", "Campwarrior", 1, 1),  # Human Warrior
    ("SPELLCAMP02", "Camppaladin", 1, 2),  # Human Paladin
    ("SPELLCAMP03", "Camphunter", 3, 3),  # Dwarf Hunter
    ("SPELLCAMP04", "Camprogue", 1, 4),  # Human Rogue
    ("SPELLCAMP05", "Camppriest", 1, 5),  # Human Priest
    ("SPELLCAMP06", "Campshaman", 2, 7),  # Orc Shaman
    ("SPELLCAMP07", "Campmage", 1, 8),  # Human Mage
    ("SPELLCAMP08", "Campwarlock", 1, 9),  # Human Warlock
    ("SPELLCAMP09", "Campdruid", 4, 11),  # NightElf Druid
]
FIXTURE_BY_CLASS = {
    cls: {"account": account, "char": name, "race": race} for account, name, race, cls in CLASS_FIXTURES
}
# Class names keyed by the matrix "class" field.
CLASS_ACCOUNT = {
    "Warrior": "SPELLCAMP01",
    "Paladin": "SPELLCAMP02",
    "Hunter": "SPELLCAMP03",
    "Rogue": "SPELLCAMP04",
    "Priest": "SPELLCAMP05",
    "Shaman": "SPELLCAMP06",
    "Mage": "SPELLCAMP07",
    "Warlock": "SPELLCAMP08",
    "Druid": "SPELLCAMP09",
}
CLASS_CHAR = {
    "Warrior": "Campwarrior",
    "Paladin": "Camppaladin",
    "Hunter": "Camphunter",
    "Rogue": "Camprogue",
    "Priest": "Camppriest",
    "Shaman": "Campshaman",
    "Mage": "Campmage",
    "Warlock": "Campwarlock",
    "Druid": "Campdruid",
}

# Coldridge arena state (map 0, beside the proven 705/707 spawns).
CASTER_POS = (-6295.0, 315.0, 377.5)
# Heal-ally spot: ~15 yd from the nearest wolf counter so gray-mob
# aggro cannot maul the damaged ally mid-leg (z=380 falls to ground).
HEAL_CASTER_POS = (-6270.0, 292.0, 380.0)
ALLY_POS = (-6268.0, 290.0, 380.0)
# Every 705/707 spawn within 160 yd of the arena, nearest first
# (bench creature table): the original 10 counters starved once
# rank coverage tripled hostile rows per class, even with the
# 2-3 s dummy respawn (visibility + clean-reuse gates). Counter
# 1034 is curated out: in full runs its aura fields repeatedly
# never latch (Psychic Scream + Devouring Plague landed GOs with
# no observable aura) while damage rows pass and isolated probes
# pass — a wander/evade state the pool cannot rely on.
WOLF_COUNTERS = [
    1526,
    1525,
    344,
    345,
    1519,
    1527,
    1515,
    346,
    347,
    1032,
    1518,
    338,
    1036,
    1530,
    1031,
    1012,
    1516,
    1517,
    1541,
    1539,
    1014,
    1053,
    400,
]
TROGG_COUNTERS = [
    1039,
    1522,
    342,
    341,
    1521,
    1040,
    340,
    1540,
    1524,
    1037,
    1533,
    1535,
    1057,
    1513,
    335,
    1531,
    1532,
    1511,
    1534,
    1542,
    1038,
    1548,
    1543,
    1510,
]

#: Finisher dummies: Ice Claw Bear spawns (entry 1196, level 8)
#: east of the arena. A combo builder must land on the finisher's
#: own target first, and even rank-1 Claw kills a 100 hp wolf
#: outright; the bears survive builder + finisher. (guid counter,
#: spawn xyz from the bench `creature` table; the driver walks to
#: the fallback position until the spawn is visible.)
FINISHER_DUMMIES = (
    (3092, (-5833.81, 300.745, 420.966)),
    (2965, (-5722.07, 487.089, 394.889)),
)


def finisher_dummy(used: set[int]) -> tuple[int, tuple]:
    """First unused finisher bear as (raw guid, fallback xyz)."""
    for counter, xyz in FINISHER_DUMMIES:
        guid = raw_guid(1196, counter)
        if guid not in used:
            return guid, xyz
    counter, xyz = FINISHER_DUMMIES[0]
    return raw_guid(1196, counter), xyz


VERDICTS = {"PASS", "FAIL", "BLOCKED", "PASSIVE-VERIFIED", "PASSIVE-UNVERIFIED"}
BLOCKED_REASONS = {
    "scenario-missing-ally",
    "no-pet",
    "no-party",
    "wire-target-unsupported",
    "engine-totem",
    "fixture-gear",
    "fixture-reagent",
    "dummy-immune",
    "dummy-unavailable",
    "area-or-quest-gated",
    "effect-marker-only",
    "script-evidence-unknown",
    "target-not-dead",
    "scenario-target",
}


def raw_guid(entry: int, counter: int) -> int:
    """Raw creature GUID the bench world DB spawns use."""
    return (0xF130 << 48) | (entry << 24) | counter


def char_create_body(name: str, race: int, cls: int) -> bytes:
    """Wire shape `world_actor.rs::on_char_create` parses."""
    return name.encode() + b"\x00" + struct.pack("<9B", race, cls, 0, 0, 0, 0, 0, 0, 0)


def cast_self_body(spell_id: int) -> bytes:
    return struct.pack("<IH", spell_id, 0)


def cast_unit_body(spell_id: int, target_raw: int) -> bytes:
    return struct.pack("<IH", spell_id, 0x0002) + smoke.pack_guid(target_raw)


def cast_dest_body(spell_id: int, x: float, y: float, z: float) -> bytes:
    """New sender the architecture §5 requires (mask 0x0040 + xyz)."""
    return struct.pack("<IH", spell_id, 0x0040) + struct.pack("<fff", x, y, z)


def cast_corpse_body(spell_id: int, target_raw: int) -> bytes:
    """Fork-shaped corpse-target packet; the current parser
    skips-but-discards corpse GUIDs, so no SPELL_GO is expected
    (BLOCKED `wire-target-unsupported` until the parser returns them)."""
    return struct.pack("<IH", spell_id, 0x0200) + smoke.pack_guid(target_raw)


# --------------------------------------------------------------------------
# Verdicts and artifacts (§6)
# --------------------------------------------------------------------------


def make_verdict(
    row: dict,
    verdict: str,
    attempts: int = 1,
    reason: str | None = None,
    phase: str | None = None,
    evidence: str = "",
    pre: str = "",
    post: str = "",
    packets: list[int] | None = None,
    target_guid: int | None = None,
    expected_failure: bool = False,
) -> dict:
    assert verdict in VERDICTS, verdict
    if verdict == "BLOCKED":
        assert reason in BLOCKED_REASONS, reason
    if verdict == "FAIL":
        assert phase in {"prepare", "go", "flight", "apply", "persist"}, phase
    if expected_failure:
        assert verdict == "FAIL" and phase == "prepare", (verdict, phase)
    return {
        "class": row.get("class"),
        "spell_id": row.get("spell_id"),
        "name": row.get("name", ""),
        "bucket": row.get("bucket"),
        "rank": row.get("rank"),
        "rank_count": row.get("rank_count"),
        "is_last_rank": row.get("is_last_rank"),
        "attempts": attempts,
        "verdict": verdict,
        "reason": reason,
        "phase": phase,
        "expected_failure": expected_failure,
        "target_guid": target_guid,
        "evidence": evidence,
        "pre": pre,
        "post": post,
        "packets": sorted(set(packets or [])),
    }


def write_results(class_name: str, verdicts: list[dict]) -> pathlib.Path:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    path = EVIDENCE_DIR / f"results-{class_name}.jsonl"
    with open(path, "w") as fh:
        for v in verdicts:
            fh.write(json.dumps(v) + "\n")
    return path


def write_summary(all_verdicts: dict[str, list[dict]]) -> pathlib.Path:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    lines = ["# Spell campaign summary", ""]
    for class_name in sorted(all_verdicts):
        verdicts = all_verdicts[class_name]
        lines.append(f"## {class_name} ({len(verdicts)} rows)")
        counts: dict[str, int] = {}
        bucket_counts: dict[tuple[str, str], int] = {}
        for v in verdicts:
            counts[v["verdict"]] = counts.get(v["verdict"], 0) + 1
            key = (str(v.get("bucket")), v["verdict"])
            bucket_counts[key] = bucket_counts.get(key, 0) + 1
        lines.append("counts: " + ", ".join(f"{k}={counts.get(k, 0)}" for k in sorted(counts)))
        lines.append(
            "bucket counts: "
            + ", ".join(f"{bucket}/{verdict}={count}" for (bucket, verdict), count in sorted(bucket_counts.items()))
        )
        for v in verdicts:
            if v["verdict"] in ("FAIL", "BLOCKED"):
                lines.append(
                    f"- {v['spell_id']} {v.get('name') or ''} "
                    f"rank {v.get('rank')}/{v.get('rank_count')} "
                    f"{v['bucket']} target={v.get('target_guid')}: "
                    f"{v['verdict']} "
                    f"{v.get('reason') or ''} {v.get('phase') or ''} "
                    f"expected_failure={v.get('expected_failure')} "
                    f"| {v['evidence']}"
                )
        lines.append("")
    path = EVIDENCE_DIR / "summary.md"
    path.write_text("\n".join(lines))
    return path


def write_manifest(extra: dict) -> pathlib.Path:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    try:
        head = subprocess_head()
    except Exception:
        head = "unknown"
    manifest = {
        "git_head": head,
        "git_dirty": subprocess_dirty(),
        "binary_build": "unknown: bench mangosd was already running; this harness neither builds nor starts it",
        "world_host": p3.HOST,
        "world_port": p3.PORT,
        "db_host": os.environ.get("MYSQL_HOST", "127.0.0.1"),
        "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "ended": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        **extra,
    }
    path = EVIDENCE_DIR / "manifest.json"
    path.write_text(json.dumps(manifest, indent=1) + "\n")
    return path


def subprocess_head() -> str:
    import subprocess

    r = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
    return r.stdout.strip()


def subprocess_dirty() -> bool:
    import subprocess

    r = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True)
    return bool(r.stdout.strip())


# --------------------------------------------------------------------------
# Guards and preflight (§1, §7)
# --------------------------------------------------------------------------


def guard_loopback() -> None:
    host = p3.HOST
    assert host in ("127.0.0.1", "localhost"), f"refusing non-loopback world host {host!r}"
    resolved = socket.gethostbyname(host)
    assert resolved.startswith("127."), f"refusing {host=}->{resolved=}"
    db_host = os.environ.setdefault("MYSQL_HOST", "127.0.0.1")
    assert socket.gethostbyname(db_host).startswith("127."), f"refusing non-loopback MYSQL_HOST={db_host!r}"
    print(f"GUARD-OK world={host}:{p3.PORT} resolved={resolved} mysql_host={db_host}")


def preflight() -> int:
    import subprocess as _subprocess

    guard_loopback()
    for probe, marker in (
        ("scripted_driver.py", "SCRIPTED-DRIVER-SELFTEST-PASS"),
        ("two_client_smoke.py", "TWO-CLIENT-SELFTEST-PASS"),
    ):
        r = _subprocess.run([sys.executable, str(SCRIPT_DIR / probe), "--self-test"], capture_output=True, text=True)
        if r.returncode != 0 or marker not in (r.stdout + r.stderr):
            print(f"PREFLIGHT-FAIL {probe}:\n{r.stdout}\n{r.stderr}")
            return 2
        print(f"PREFLIGHT-OK {probe} {marker}")
    for probe, marker in (
        ("spell_matrix_gen.py", "SPELL-MATRIX-GEN-SELFTEST-PASS"),
        ("spell_campaign.py", "SPELL-CAMPAIGN-SELFTEST-PASS"),
    ):
        r = _subprocess.run([sys.executable, str(SCRIPT_DIR / probe), "--self-test"], capture_output=True, text=True)
        if r.returncode != 0 or marker not in (r.stdout + r.stderr):
            print(f"PREFLIGHT-FAIL {probe}:\n{r.stdout}\n{r.stderr}")
            return 2
        print(f"PREFLIGHT-OK {probe} {marker}")
    hostname = smoke.sql_scalar("realmd", "SELECT @@hostname")
    print(f"DB-OK @@hostname={hostname}")
    row = smoke.sql_scalar("mangos", "SELECT COUNT(*) FROM creature_template WHERE entry=327")
    spawns = smoke.sql_scalar("mangos", "SELECT COUNT(*) FROM creature WHERE id=327")
    print(f"PROBE-ENTRY-327 template={row} spawns={spawns} (informational; pilot runs on 705/707 regardless)")
    return 0


# --------------------------------------------------------------------------
# Fixtures (§2)
# --------------------------------------------------------------------------


def ensure_account(username: str) -> int:
    found = smoke.sql_scalar("realmd", f"SELECT id FROM account WHERE username='{username}'")
    if found:
        return int(found)
    smoke.sql_scalar("realmd", f"INSERT INTO account (username) VALUES ('{username}')")
    found = smoke.sql_scalar("realmd", f"SELECT id FROM account WHERE username='{username}'")
    print(f"ACCOUNT-CREATED {username} id={found}")
    return int(found)


def ensure_character(account: str, account_id: int, name: str, race: int, cls: int) -> int:
    found = smoke.sql_scalar("characters", f"SELECT guid FROM characters WHERE name='{name}'")
    if found:
        return int(found)
    p3.ACCOUNT = account
    p3.ACCOUNT_ID = account_id
    p3.setup_account()
    client = p3.Client()
    client.auth()
    # Refresh the session's character count first: the actor defaults
    # it to 10 (the C++ session default) and only CMSG_CHAR_ENUM
    # lowers it, otherwise creation always answers SERVER_LIMIT.
    client.send_packet(CMSG_CHAR_ENUM, b"")
    client.sock.settimeout(5)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        opcode, _ = client.read_packet()
        if opcode == SMSG_CHAR_ENUM:
            break
    client.send_packet(CMSG_CHAR_CREATE, char_create_body(name, race, cls))
    body, _ = smoke.read_until_opcode(client, SMSG_CHAR_CREATE, 10, f"create {name}")
    code = body[0] if body else None
    client.sock.close()
    if code != CHAR_CREATE_SUCCESS:
        raise AssertionError(f"CMSG_CHAR_CREATE {name} refused: code={code}")
    found = smoke.sql_scalar("characters", f"SELECT guid FROM characters WHERE name='{name}'")
    if not found:
        raise AssertionError(f"character {name} missing after code 46")
    print(f"CHAR-CREATED {name} guid={found} race={race} class={cls}")
    return int(found)


#: Hunter pet fixture (Call Pet 883 is family-0, outside the matrix,
#: so the harness seeds the saved pet itself). Timber Wolf 69 is
#: tameable (type 1 BEAST, pet_family 1, STATIC_FLAG_TAMEABLE) and
#: carries Growl; Pet Wolf 860 (type 7, family 0) is not. Rust
#: `load_pet_from_rows` (pets.rs) finds the first row with entry
#: != 0 and slot 0 (current); `load_pet_with_row` needs
#: pet_type 1 (HUNTER), a tameable template, and level from the row
#: for hunter pets, and ignores created_by_spell 0 for Called loads.
#: Slot/pet_type/level constants mirror mangos-db pet.rs.
HUNTER_PET_SETUP_SPELL = 883
HUNTER_PET_ENTRY = 69
HUNTER_PET_SPELL = 2649
HUNTER_PET_ROW_ID = 900001
HUNTER_PET_DISPLAY_ID = 604

#: Hunter ranged fixture (fork `CheckItems` honesty; the Rust engine
#: has no weapon/ammo gate, so the bench runs without it). Ashwood
#: Bow 5596 (class 2) in the ranged slot, Rough Arrows 2512 in the
#: backpack, and a Militia Shortsword 1161 in the mainhand for the
#: melee-weapon rows. Fixed item guids keep the fixture idempotent
#: and fixture-scoped (seed_character never touches other guids).
EQUIPMENT_SLOT_MAINHAND = 15
EQUIPMENT_SLOT_OFFHAND = 16
EQUIPMENT_SLOT_RANGED = 17
HUNTER_GEAR_ROWS = (
    # (item_guid, entry, bag, slot, count)
    (910001, 5596, 0, EQUIPMENT_SLOT_RANGED, 1),
    (910002, 2512, 0, 23, 200),
    (910003, 1161, 0, EQUIPMENT_SLOT_MAINHAND, 1),
)
#: Warrior fixture gear (fork honesty; the engine has no equipment
#: gate, so no audited Warrior row routes to the item leg's shield
#: gate — Shield Slam verifies on the dummy via the hostile leg).
#: Militia Shortsword 1161 in the mainhand and a Bark Shield 1166
#: in the offhand, so shield-gated rows cast shielded as the fork
#: requires. Fixed item guids keep the fixture idempotent and
#: fixture-scoped, like the Hunter gear above.
WARRIOR_GEAR_ROWS = (
    # (item_guid, entry, bag, slot, count)
    (920001, 1161, 0, EQUIPMENT_SLOT_MAINHAND, 1),
    (920002, 1166, 0, EQUIPMENT_SLOT_OFFHAND, 1),
)


def seed_hunter_pet(guid: int) -> None:
    """Insert the saved-pet rows Call Pet needs (opt-in Hunter
    fixture). Runs after seed_character's delete phase (which clears
    every pet row for this owner), so the insert is idempotent and
    cleanup stays fixture-scoped: the next seed deletes these rows
    again. Pet spells ride pet_spell (load-only companion data the
    Rust loader reads); no pet_aura rows (never read at load)."""
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_pet WHERE owner_guid={guid} AND id={HUNTER_PET_ROW_ID}",
    )
    smoke.sql_scalar(
        "characters",
        "INSERT INTO character_pet (id, entry, owner_guid, display_id, "
        "created_by_spell, pet_type, level, xp, react_state, "
        "loyalty_points, loyalty, training_points, slot, name, "
        "renamed, current_health, current_mana, current_happiness, "
        "save_time, reset_talents_cost, reset_talents_time, "
        "action_bar_data, teach_spell_data) VALUES "
        f"({HUNTER_PET_ROW_ID}, {HUNTER_PET_ENTRY}, {guid}, "
        f"{HUNTER_PET_DISPLAY_ID}, 0, 1, 60, 0, 1, 0, 1, 0, 0, "
        "'Pet', 0, 9999, 9999, 3, 0, 0, 0, "
        "'0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 ', '')",
    )
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM pet_spell WHERE guid={HUNTER_PET_ROW_ID}",
    )
    smoke.sql_scalar(
        "characters",
        f"INSERT INTO pet_spell (guid, spell, active) VALUES ({HUNTER_PET_ROW_ID}, {HUNTER_PET_SPELL}, 0)",
    )
    print(f"SEEDED hunter pet guid={guid} pet_id={HUNTER_PET_ROW_ID} entry={HUNTER_PET_ENTRY}")


def seed_hunter_gear(guid: int) -> None:
    """Equip the Hunter ranged fixture (idempotent, fixture-scoped).

    Deletes only this owner's slot rows plus our fixed item guids,
    then inserts the bow/arrows/sword. Starter-kit duplicates (Old
    Blunderbuss) land in bag slots when the equip slots are taken.
    """
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_inventory WHERE guid={guid} AND bag=0 AND slot IN ({EQUIPMENT_SLOT_MAINHAND}, "
        f"{EQUIPMENT_SLOT_RANGED}, 23)",
    )
    fixed = ",".join(str(item_guid) for item_guid, _entry, _bag, _slot, _count in HUNTER_GEAR_ROWS)
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM item_instance WHERE guid IN ({fixed}) AND owner_guid={guid}",
    )
    for item_guid, entry, bag, slot, count in HUNTER_GEAR_ROWS:
        smoke.sql_scalar(
            "characters",
            "INSERT INTO item_instance (guid, item_id, owner_guid, count, enchantments, durability) VALUES "
            f"({item_guid}, {entry}, {guid}, {count}, '', 100)",
        )
        smoke.sql_scalar(
            "characters",
            "INSERT INTO character_inventory (guid, bag, slot, item_guid, item_id) VALUES "
            f"({guid}, {bag}, {slot}, {item_guid}, {entry})",
        )
    print(f"SEEDED hunter gear guid={guid} rows={len(HUNTER_GEAR_ROWS)}")


def seed_warrior_gear(guid: int) -> None:
    """Equip the Warrior shield fixture (idempotent, fixture-scoped).

    Deletes only this owner's mainhand/offhand slot rows plus our
    fixed item guids, then inserts the sword/shield. Mirrors
    seed_hunter_gear; the engine never gates on equipment, but
    shield-gated rows (Shield Slam/Shield Bash) cast shielded as
    the fork requires.
    """
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_inventory WHERE guid={guid} AND bag=0 AND slot IN ({EQUIPMENT_SLOT_MAINHAND}, "
        f"{EQUIPMENT_SLOT_OFFHAND})",
    )
    fixed = ",".join(str(item_guid) for item_guid, _entry, _bag, _slot, _count in WARRIOR_GEAR_ROWS)
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM item_instance WHERE guid IN ({fixed}) AND owner_guid={guid}",
    )
    for item_guid, entry, bag, slot, count in WARRIOR_GEAR_ROWS:
        smoke.sql_scalar(
            "characters",
            "INSERT INTO item_instance (guid, item_id, owner_guid, count, enchantments, durability) VALUES "
            f"({item_guid}, {entry}, {guid}, {count}, '', 100)",
        )
        smoke.sql_scalar(
            "characters",
            "INSERT INTO character_inventory (guid, bag, slot, item_guid, item_id) VALUES "
            f"({guid}, {bag}, {slot}, {item_guid}, {entry})",
        )
    print(f"SEEDED warrior gear guid={guid} rows={len(WARRIOR_GEAR_ROWS)}")


#: Caster intellect cloth (equipment slots 0/2/4/6/9): the
#: bare level-60 fixture maxes at ~2808 mana, below Arcane
#: Brilliance's 3400 cost — a legal geared 60 casts it, so the
#: fixture wears fork-legal intellect cloth (+155 int total).
CASTER_GEAR_ROWS = (
    # (item_guid, entry, bag, slot, count)
    (940001, 16914, 0, 0, 1),  # Netherwind Crown +40
    (940002, 16917, 0, 2, 1),  # Netherwind Mantle +27
    (940003, 19844, 0, 4, 1),  # Zandalar Illusionist's Robe +24
    (940004, 20329, 0, 6, 1),  # 90 Epic Frost Leggings +37
    (940005, 20328, 0, 9, 1),  # 90 Epic Frost Gloves +27
)


def seed_caster_gear(guid: int) -> None:
    """Equip the caster intellect fixture (idempotent,
    fixture-scoped); mirrors seed_hunter_gear."""
    slots = ",".join(str(slot) for _g, _e, _b, slot, _c in CASTER_GEAR_ROWS)
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_inventory WHERE guid={guid} AND bag=0 AND slot IN ({slots})",
    )
    fixed = ",".join(str(item_guid) for item_guid, _e, _b, _s, _c in CASTER_GEAR_ROWS)
    # Fixture guids are campaign-global (see seed_reagents):
    # delete by guid alone, across characters.
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_inventory WHERE item_guid IN ({fixed})",
    )
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM item_instance WHERE guid IN ({fixed})",
    )
    for item_guid, entry, bag, slot, count in CASTER_GEAR_ROWS:
        # durability 100: a 0-durability row is a broken item
        # in the fork bonus rules and grants no stats.
        smoke.sql_scalar(
            "characters",
            "INSERT INTO item_instance (guid, item_id, owner_guid, count, enchantments, durability) VALUES "
            f"({item_guid}, {entry}, {guid}, {count}, '', 100)",
        )
        smoke.sql_scalar(
            "characters",
            "INSERT INTO character_inventory (guid, bag, slot, item_guid, item_id) VALUES "
            f"({guid}, {bag}, {slot}, {item_guid}, {entry})",
        )
    print(f"SEEDED caster gear guid={guid} rows={len(CASTER_GEAR_ROWS)}")


#: Fixed item-guid base for reagent fixture stacks (910xxx is
#: Hunter gear, 920xxx Warrior gear).
REAGENT_FIXTURE_GUID_BASE = 930001
REAGENT_FIXTURE_COUNT = 200


def seed_reagents(guid: int, rows: list[dict]) -> None:
    """Seed one generous stack of every distinct reagent the
    class matrix references into the backpack (bag 0, slots
    23+), idempotent and fixture-scoped like the gear seeds.

    The engine reagent gate (CheckItems/TakeReagents) makes
    these rows castable and consumable; the item leg then
    verdicts on the DB count decrement. Totem-item rows ride
    the same seed (presence is all the gate checks).
    """
    entries = sorted(
        {int(entry) for row in rows for entry in (row.get("reagents") or []) if int(entry or 0) > 0}
        | {int(t) for row in rows for t in (row.get("totem") or []) if int(t or 0) > 0}
    )
    if not entries:
        return
    fixed = ",".join(str(REAGENT_FIXTURE_GUID_BASE + i) for i in range(len(entries)))
    # Fixture guids are global to the campaign: delete by guid
    # alone, or a second class's seed collides on the PK with
    # the previous character's stacks.
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_inventory WHERE item_guid IN ({fixed})",
    )
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM item_instance WHERE guid IN ({fixed})",
    )
    for i, entry in enumerate(entries):
        item_guid = REAGENT_FIXTURE_GUID_BASE + i
        slot = 23 + i
        smoke.sql_scalar(
            "characters",
            f"DELETE FROM character_inventory WHERE guid={guid} AND bag=0 AND slot={slot}",
        )
        smoke.sql_scalar(
            "characters",
            "INSERT INTO item_instance (guid, item_id, owner_guid, count, enchantments) VALUES "
            f"({item_guid}, {entry}, {guid}, {REAGENT_FIXTURE_COUNT}, '')",
        )
        smoke.sql_scalar(
            "characters",
            "INSERT INTO character_inventory (guid, bag, slot, item_guid, item_id) VALUES "
            f"({guid}, 0, {slot}, {item_guid}, {entry})",
        )
    print(f"SEEDED reagents guid={guid} entries={entries}")


def seed_character(
    guid: int, spells: list[int], pos: tuple[float, float, float] = CASTER_POS, hunter_pet: bool = False
) -> None:
    x, y, z = pos
    smoke.ensure_offline(guid)
    smoke.sql_scalar(
        "characters",
        "UPDATE characters SET level=60, xp=0, health=999999, "
        "power1=999999,power2=999999,power3=999999,power4=999999,"
        "power5=999999, map=0, "
        f"position_x={x}, position_y={y}, position_z={z}, orientation=0, "
        "death_expire_time=0, "
        "character_flags=character_flags & ~8192, online=0 "
        f"WHERE guid={guid}",
    )
    smoke.sql_scalar("characters", f"DELETE FROM character_spell WHERE guid={guid}")
    for spell in spells:
        smoke.sql_scalar(
            "characters", f"INSERT INTO character_spell (guid,spell,active,disabled) VALUES ({guid},{spell},1,0)"
        )
    smoke.sql_scalar("characters", f"DELETE FROM character_spell_cooldown WHERE guid={guid}")
    smoke.sql_scalar("characters", f"DELETE FROM character_aura WHERE guid={guid}")
    smoke.sql_scalar("characters", f"DELETE FROM corpse WHERE player_guid={guid}")
    # A persisted pet from an earlier run re-summons at login and
    # attacks the caster's targets, contaminating every hostile
    # row's health evidence (Warlock first run). The fixture reset
    # owns the whole character: clear the pet and its satellite
    # rows; pet scenarios summon fresh in-session.
    for table in ("pet_aura", "pet_spell", "pet_spell_cooldown"):
        smoke.sql_scalar(
            "characters",
            f"DELETE FROM {table} WHERE guid IN (SELECT id FROM character_pet WHERE owner_guid={guid})",
        )
    smoke.sql_scalar("characters", f"DELETE FROM character_pet WHERE owner_guid={guid}")
    if hunter_pet:
        seed_hunter_pet(guid)
    print(f"SEEDED guid={guid} spells={len(spells)} pos=({x},{y},{z})")


SETUP_SPELLS: dict[str, tuple[int, ...]] = {
    # Stance entries + Bloodrage are scenario setup, not verdict
    # rows (Battle Stance seeds the stance-gated Rend pilot; 71 and
    # 2458 seed the Defensive/Berserker-gated fan-out rows;
    # Bloodrage 2687 tops rage up via the generic energize lane).
    # All are seeded and counted explicitly.
    "Warrior": (2457, 71, 2458, 2687),
    # Grimoire teach wrappers for Sacrifice ranks (7885 -> 7812,
    # 19447 -> 19443): a fresh voidwalker only knows Torment, so
    # the pet-cast leg teaches before ordering the cast, exactly
    # like using the grimoire item in game.
    "Warlock": (7885, 19447),
    # Call Pet is family-0 (outside the matrix, like Warrior
    # Battle Stance): the Hunter pet scenario casts it as setup,
    # and it must be seeded or the book lacks it.
    "Hunter": (HUNTER_PET_SETUP_SPELL,),
}

#: Druid shapeshift form spells (the fan-out cast set's `form`-bucket
#: rows: Cat, Travel, Aquatic, Dire Bear, Moonkin). The form leg
#: pre-clears any of these still active and post-cancels the entered
#: form (CMSG_CANCEL_AURA 310, the pilot Bear->Cat pattern) so later
#: caster-form rows gate cleanly.
DRUID_FORM_SPELLS = (768, 783, 1066, 5487, 9634, 24858)

#: Stances bit k means ShapeshiftForm k+1 (Cat 1, Travel 3, Aquatic
#: 4, Bear 5, DireBear 8, Moonkin 31): the form spell that enters
#: each, in preference order for stance-gated rows whose mask allows
#: more than one form (Bear|DireBear rows take Dire Bear).
FORM_SPELL_BY_STANCE_BIT = ((0, 768), (7, 9634), (4, 5487), (30, 24858), (2, 783), (3, 1066))

#: Warrior stance-entry spells (same Stances-bit machinery as Druid
#: forms, disjoint bits). Bit -> stance from the singleton-row
#: names: 16 Battle (Charge/Thunder Clap/Overpower/Mocking Blow),
#: 17 Defensive (Taunt/Disarm/Revenge/Shield Wall/Shield Block),
#: 18 Berserker (Whirlwind/Recklessness/Pummel/Intercept). The
#: audited cast set carries no other stance bits; dual-stance rows
#: (16+17 Shield Bash/Rend, 16+18 Hamstring/Execute) admit Battle
#: first, which satisfies either mask.
WARRIOR_STANCE_SPELLS = (2457, 71, 2458)
WARRIOR_STANCE_BY_BIT = ((16, 2457), (17, 71), (18, 2458))

#: Rogue Stealth entry spells (all ranks share FORM_STEALTH 30,
#: fork `SharedDefines.h` 0x1E; Stances bit 29 = form 30 by the
#: bit k -> form k+1 convention). Rank 1 (1784) is the setup
#: entry; the full rank set rides the form pre-clear so a
#: lingering higher-rank Stealth cannot leak into a gated row.
STEALTH_FORM_SPELLS = (1784, 1785, 1786, 1787)
STEALTH_BY_BIT = ((29, 1784),)

#: Every shapeshift/stance aura the form legs pre-clear.
ALL_FORM_SPELLS = DRUID_FORM_SPELLS + WARRIOR_STANCE_SPELLS + STEALTH_FORM_SPELLS


def required_form_spell(row: dict) -> int | None:
    """Form-entry spell for a stance-gated row, or None when the row
    is not form-enterable here (actual totem rows, no stance bits the
    harness can enter). Druid bits resolve first so Druid behavior
    is unchanged; Warrior stance and Rogue stealth bits resolve
    after. Pure: reads only the matrix row."""
    if any(row.get("totem") or ()):
        return None
    stances = int(row.get("stances") or 0)
    for bit, spell in FORM_SPELL_BY_STANCE_BIT:
        if stances & (1 << bit):
            return spell
    for bit, spell in WARRIOR_STANCE_BY_BIT:
        if stances & (1 << bit):
            return spell
    for bit, spell in STEALTH_BY_BIT:
        if stances & (1 << bit):
            return spell
    return None


def pilot_spells_for(class_name: str) -> list[int]:
    doc = json.loads(MATRIX_PATH.read_text())
    spells = {r["spell_id"] for r in doc["rows"] if r["class"] == class_name}
    spells.update(SETUP_SPELLS.get(class_name, ()))
    return sorted(spells)


def matrix_doc_for(class_name: str) -> dict:
    """Matrix doc backing one class: the per-class fan-out artifact
    when the generator wrote one, else the main matrix. Pilot classes
    always come from the main matrix, so pilot behavior is unchanged."""
    try:
        import spell_matrix_gen as gen  # noqa: PLC0415

        path = gen.class_matrix_path(class_name)
        if path.exists():
            return json.loads(path.read_text())
    except (OSError, ValueError):
        pass
    return json.loads(MATRIX_PATH.read_text())


def matrix_rows_for(class_name: str) -> list[dict]:
    return [r for r in matrix_doc_for(class_name).get("rows", []) if r["class"] == class_name]


def talent_known_spells(class_name: str) -> list[int]:
    """Selected-build known_spell_ids for one class (G2); [] when the
    class has no legal selected build."""
    try:
        doc = matrix_doc_for(class_name)
        full_known = doc.get("full_talent_known_spell_ids") or (doc.get("full_seed_spell_ids") or {})
        if isinstance(full_known, list):
            return sorted(full_known)
        selected = (doc.get("talent_builds") or {}).get("selected") or {}
        entry = selected.get(class_name) or {}
        return sorted(entry.get("known_spell_ids") or [])
    except (OSError, ValueError):
        return []


def seed_spells_for(class_name: str) -> list[int]:
    """Full seed list for one class (arch §2.4): the per-class
    artifact's recorded seed list when present (G1 union ranks plus
    the selected talent known spells), else the union of every cast
    row's ranks plus talent known plus setup. Pilot classes keep
    their exact legacy seed through pilot_spells_for; only fan-out
    classes use this."""
    doc = matrix_doc_for(class_name)
    seed_lists = doc.get("full_seed_spell_ids") or {}
    if isinstance(seed_lists, dict) and seed_lists.get(class_name):
        spells = set(seed_lists[class_name])
    else:
        spells = set()
        for r in doc.get("rows", []):
            if r["class"] != class_name:
                continue
            merged = (r.get("merged_chain") or {}).get("members")
            if merged:
                spells.update(merged)
            else:
                spells.update(r.get("all_rank_ids") or [r["spell_id"]])
    spells.update(talent_known_spells(class_name))
    spells.update(SETUP_SPELLS.get(class_name, ()))
    return sorted(spells)


def drift_gate(driver: ScriptedDriver, guid: int, expect_spells: int) -> None:
    level = driver.world.get(guid).values.get(UNIT_FIELD_LEVEL) if driver.world.get(guid) else None
    if level != 60:
        raise AssertionError(f"fixture drift: level={level} want 60")
    have = smoke.sql_scalar("characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid}")
    if int(have) != expect_spells:
        raise AssertionError(f"fixture drift: character_spell={have} want {expect_spells}")
    health = driver.self_health()
    max_health = driver.world.get(driver.own_guid).values.get(UNIT_FIELD_MAXHEALTH) if driver.own_guid else None
    if health != max_health or not health:
        raise AssertionError(f"fixture drift: health={health} max={max_health}")
    print(f"DRIFT-OK guid={guid} level=60 spells={have} health={health}")


def fixtures_ensure() -> int:
    """Idempotent ensure of all nine §2 fixture characters (slot order).

    Pilot legs only ever address their own three characters by name, so
    `pilot` behavior is unchanged; fan-out classes find their fixture
    (notably Camppaladin) already present."""
    guids: dict[str, int] = {}
    for account, name, race, cls in CLASS_FIXTURES:
        account_id = ensure_account(account)
        guid = ensure_character(account, account_id, name, race, cls)
        guids[name] = guid
    return 0


# --------------------------------------------------------------------------
# Cast helpers (§5)
# --------------------------------------------------------------------------


def aura_present(driver: ScriptedDriver, guid: int, spell_id: int) -> bool:
    obj = driver.world.get(guid)
    if obj is None:
        return False
    for field in range(AURA_FIELD_FIRST, AURA_FIELD_LAST + 1):
        if obj.values.get(field) == spell_id:
            return True
    return False


def aura_ids_on(driver: ScriptedDriver, guid: int) -> set[int]:
    """Spell ids currently in the unit's aura slots."""
    obj = driver.world.get(guid)
    if obj is None:
        return set()
    return {int(value) for field in range(AURA_FIELD_FIRST, AURA_FIELD_LAST + 1) if (value := obj.values.get(field))}


def heal_rise_ok(
    h0: float | None, h1: float | None, h2: float | None, t0: float, t1: float, t2: float, minimum_rise: float = 50.0
) -> tuple[float, bool]:
    """Regen-baselined heal rise (the heal_leg gate, pure part).

    h0/h1 bracket the pre-cast regen rate, h2 is post-cast; the rise
    is h2 minus the extrapolated baseline. A full ally yields a zero
    rise and is a scenario FAIL, never PASS. Returns (rise, ok).
    """
    if h0 is None or h1 is None or h2 is None or t1 <= t0:
        return (0.0, False)
    rate = (h1 - h0) / (t1 - t0)
    rise = h2 - (h1 + rate * (t2 - t1))
    return (rise, rise > minimum_rise)


def dummy_reusable(health: int | None, max_health: int | None, has_aura: bool) -> bool:
    """Clean-dummy reuse predicate (pure part of pick_dummy).

    Only an alive, full-health, aura-free dummy may be reused: DoTs,
    CC, and prior damage all leave either a health deficit or an
    aura holder, so both are required clean.
    """
    return health is not None and max_health is not None and 0 < health == max_health and not has_aura


def passive_learn_verdict(passive: bool, learned: bool, aura_ok: bool) -> str:
    """PASSIVE-VERIFIED rule: passive spells have no aura holder by
    design, so learned alone verifies; non-passive learned rows
    still need the aura observed."""
    if passive:
        return "PASSIVE-VERIFIED" if learned else "PASSIVE-UNVERIFIED"
    return "PASSIVE-VERIFIED" if (learned and aura_ok) else "PASSIVE-UNVERIFIED"


def parse_cast_result_body(body: bytes) -> tuple[int, int] | None:
    """Parse the Rust SMSG_CAST_RESULT body, accepting the fork's
    status-byte variant as well. Returns (spell_id, reason)."""
    if len(body) < 5:
        return None
    spell_id = struct.unpack_from("<I", body, 0)[0]
    if len(body) >= 6 and body[4] == 2:
        return spell_id, body[5]
    return spell_id, body[4]


def parse_spell_go_body(body: bytes) -> dict | None:
    """Parse the Rust `send_spell_go` body through its hit/miss lists."""
    try:
        caster_guid, offset = smoke.parse_packed_guid(body, 0)
        target_guid, offset = smoke.parse_packed_guid(body, offset)
        cast_counter = body[offset]
        offset += 1
        spell_id, _flags, _now = struct.unpack_from("<III", body, offset)
        offset += 12
        hit_count = body[offset]
        offset += 1
        hits = []
        for _ in range(hit_count):
            hits.append(struct.unpack_from("<Q", body, offset)[0])
            offset += 8
        miss_count = body[offset]
        offset += 1
        misses = []
        for _ in range(miss_count):
            guid = struct.unpack_from("<Q", body, offset)[0]
            offset += 8
            misses.append((guid, body[offset]))
            offset += 1
    except (IndexError, struct.error):
        return None
    return {
        "caster_guid": caster_guid,
        "target_guid": target_guid,
        "cast_counter": cast_counter,
        "spell_id": spell_id,
        "hits": hits,
        "misses": misses,
    }


def spell_go_matches(
    parsed: dict | None, spell_id: int, caster_guid: int | None, target_guid: int | None = None
) -> bool:
    if parsed is None or parsed["spell_id"] != spell_id:
        return False
    if caster_guid is not None and parsed["caster_guid"] != caster_guid:
        return False
    if target_guid is not None:
        return parsed["target_guid"] == target_guid or target_guid in parsed["hits"]
    return True


def parse_damage_log_body(body: bytes) -> dict | None:
    """Parse the shared SMSG_SPELLNONMELEEDAMAGELOG prefix."""
    try:
        target_guid, offset = smoke.parse_packed_guid(body, 0)
        attacker_guid, offset = smoke.parse_packed_guid(body, offset)
        spell_id, damage = struct.unpack_from("<II", body, offset)
    except (IndexError, struct.error):
        return None
    return {
        "target_guid": target_guid,
        "attacker_guid": attacker_guid,
        "spell_id": spell_id,
        "damage": damage,
    }


def parse_heal_log_body(body: bytes) -> dict | None:
    """Parse SMSG_SPELLHEALLOG (336): packed(victim)
    packed(caster) spell amount crit. The engine emits it from
    the direct-heal effect when the caster or victim is a player
    (fork `DealHeal` -> `SendHealSpellLog`); the heal leg binds
    it for heals health deltas cannot resolve."""
    try:
        target_guid, offset = smoke.parse_packed_guid(body, 0)
        caster_guid, offset = smoke.parse_packed_guid(body, offset)
        spell_id, amount = struct.unpack_from("<II", body, offset)
        crit = body[offset + 8]
    except (IndexError, struct.error):
        return None
    return {
        "target_guid": target_guid,
        "caster_guid": caster_guid,
        "spell_id": spell_id,
        "amount": amount,
        "crit": crit,
    }


def matching_heal_logs(
    events: list[tuple[int, bytes]], spell_id: int, target_guid: int, caster_guid: int | None = None
) -> list[dict]:
    """Attributed SMSG_SPELLHEALLOG amounts for one spell on one
    target. Regen never emits logs, so any attributed amount is a
    formula-derived heal, overheal included (the fork logs the
    applied amount even when the gain clamps at max)."""
    logs = []
    for opcode, body in events:
        if opcode != SMSG_SPELLHEALLOG:
            continue
        parsed = parse_heal_log_body(body)
        if parsed is None:
            continue
        if parsed["spell_id"] != spell_id:
            continue
        if parsed["target_guid"] != target_guid:
            continue
        if caster_guid is not None and parsed["caster_guid"] != caster_guid:
            continue
        logs.append(parsed)
    return logs


def event_mark(driver: ScriptedDriver) -> int:
    return len(driver.events)


def events_since(driver: ScriptedDriver, mark: int) -> list[tuple[int, bytes]]:
    return driver.events[mark:]


def parse_periodicaura_log_body(body: bytes) -> dict | None:
    """Parse SMSG_PERIODICAURALOG (590): packed(target)
    packed(caster) spell count auraType damage school absorb
    resisted. Ticks land here; direct hits use 592."""
    try:
        target_guid, offset = smoke.parse_packed_guid(body, 0)
        attacker_guid, offset = smoke.parse_packed_guid(body, offset)
        spell_id, _count, _aura, damage = struct.unpack_from("<IIII", body, offset)
    except (IndexError, struct.error):
        return None
    return {
        "target_guid": target_guid,
        "attacker_guid": attacker_guid,
        "spell_id": spell_id,
        "damage": damage,
    }


def matching_damage_logs(
    events: list[tuple[int, bytes]], spell_id: int, target_guid: int, attacker_guid: int | None = None
) -> list[dict]:
    logs = []
    for opcode, body in events:
        if opcode == SMSG_SPELLNONMELEEDAMAGELOG:
            parsed = parse_damage_log_body(body)
        elif opcode == SMSG_PERIODICAURALOG:
            parsed = parse_periodicaura_log_body(body)
        else:
            continue
        if parsed is None:
            continue
        if parsed["spell_id"] != spell_id:
            continue
        if parsed["target_guid"] != target_guid:
            continue
        if attacker_guid is not None and parsed["attacker_guid"] != attacker_guid:
            continue
        logs.append(parsed)
    return logs


def matching_periodic_ticks(
    events: list[tuple[int, bytes]], spell_id: int, target_guid: int, attacker_guid: int | None = None
) -> list[int]:
    """Attributed SMSG_PERIODICAURALOG (590) tick amounts for one
    spell on one target. Direct hits (592) are excluded: the tick
    band only constrains periodic amounts."""
    amounts = []
    for opcode, body in events:
        if opcode != SMSG_PERIODICAURALOG:
            continue
        parsed = parse_periodicaura_log_body(body)
        if parsed is None:
            continue
        if parsed["spell_id"] != spell_id:
            continue
        if parsed["target_guid"] != target_guid:
            continue
        if attacker_guid is not None and parsed["attacker_guid"] != attacker_guid:
            continue
        amounts.append(int(parsed["damage"]))
    return amounts


def tick_band_check(
    expected: dict, tick_amounts: list[int], target_alive: bool, pre_hp: int | None = None
) -> tuple[bool, str]:
    """Pure tick-band gate for periodic-damage rows (the hostile
    leg's 590 latch). Every observed tick must land in the oracle
    band, and at least 2 in-band ticks are required while the target
    lives (rank-1 casts that do not kill); a target that died before
    a tick could land leaves the band un-enforced rather than
    failing the row. Reports observed vs expected counts."""
    band = expected.get("tick_band") or [0, 0]
    low, high = float(band[0]), float(band[1])
    if low == 0 and high == 0:
        # Zero-damage periodic (Ignite cast directly: the
        # proc damage rides the triggering crit, so a bare
        # cast ticks for 0 and sends no periodic logs).
        # The aura presence itself is the gate.
        return True, "ticks un-enforced: zero-damage band (proc-sourced damage)"
    expected_count = int(expected.get("tick_count_expected") or 2)
    in_band = [amount for amount in tick_amounts if low <= amount <= high]
    out_of_band = len(tick_amounts) - len(in_band)
    if not target_alive and not tick_amounts:
        return True, (
            f"ticks 0/{expected_count} in oracle band [{low:g}-{high:g}]: "
            "target died before a tick could land; band un-enforced"
        )
    # Never demand more ticks than the aura produces (Curse of Doom
    # ticks once at 60 s): the 2-tick minimum only applies to
    # multi-tick auras on living targets. It also cannot apply when
    # two in-band ticks would kill the dummy outright (last-rank
    # Drain Soul: 91/tick vs a 100 hp wolf, with melee pushback
    # ending the channel): one in-band tick then carries the band
    # proof, and periodicity is proven by the chain's rank-1 row.
    required = min(2, expected_count) if target_alive else 1
    if pre_hp and low * 2 >= pre_hp:
        required = min(required, 1)
    ok = out_of_band == 0 and len(in_band) >= required
    return ok, (
        f"ticks {len(in_band)}/{expected_count} in oracle band [{low:g}-{high:g}] "
        f"(required>={required}, target_alive={target_alive}, out_of_band={out_of_band}, "
        f"amounts={sorted(tick_amounts)})"
    )


def send_and_wait(
    driver: ScriptedDriver,
    body: bytes,
    spell_id: int,
    timeout: float = 10.0,
    target_guid: int | None = None,
) -> tuple[bool, int | None, list[int]]:
    """One cast over the real wire; returns (go, result_reason, opcodes).

    Only a SPELL_GO/CAST_RESULT body naming this spell (and, for GO,
    this caster and target when known) can bind the result. Other
    casters' traffic is observed but cannot satisfy this cast.
    `go=False` with reason None is a timeout (no bound answer).
    """
    driver.client.send_packet(302, body)
    deadline = time.monotonic() + timeout
    ops: list[int] = []
    driver.client.sock.settimeout(5)
    while time.monotonic() < deadline:
        ready, _, _ = select.select([driver.client.sock], [], [], 0.25)
        if not ready:
            continue
        opcode, pkt = driver.client.read_packet()
        driver.observe(opcode, pkt)
        ops.append(opcode)
        if opcode == SMSG_SPELL_GO:
            parsed = parse_spell_go_body(pkt)
            if spell_go_matches(parsed, spell_id, driver.own_guid, target_guid):
                return True, None, ops
        if opcode == SMSG_CAST_RESULT:
            parsed = parse_cast_result_body(pkt)
            if parsed is not None and parsed[0] == spell_id:
                return False, parsed[1], ops
    return False, None, ops


def login_as(
    account: str,
    account_id: int,
    guid: int,
) -> tuple[object, ScriptedDriver]:
    """Login like `two_client_smoke.login_client`, but every packet of
    the entry burst is fed to the driver's `WorldState` first: the
    stock helper drains the burst into a throwaway list, which would
    leave self values/health/auras unobservable until later traffic.
    """
    p3.ACCOUNT = account
    p3.ACCOUNT_ID = account_id
    p3.setup_account()
    client = p3.Client()
    client.auth()
    client.send_packet(smoke.CMSG_CHAR_ENUM, b"")
    opcode, _ = client.read_packet()
    if opcode != smoke.SMSG_CHAR_ENUM:
        raise AssertionError(f"{account}: expected SMSG_CHAR_ENUM, got {opcode}")
    client.send_packet(smoke.CMSG_PLAYER_LOGIN, struct.pack("<Q", guid))
    driver = ScriptedDriver(client, (0.0, 0.0, 0.0, 0.0), own_guid=guid)
    verify: bytes | None = None
    seen_update = False
    client.sock.settimeout(5)
    deadline = time.monotonic() + 15
    last_packet_at = time.monotonic()
    while time.monotonic() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if not ready:
            if verify is not None and seen_update and time.monotonic() - last_packet_at >= 1.5:
                break
            continue
        op, body = client.read_packet()
        last_packet_at = time.monotonic()
        driver.observe(op, body)
        if op == smoke.SMSG_LOGIN_VERIFY_WORLD:
            verify = body
        if op in (smoke.SMSG_UPDATE_OBJECT, smoke.SMSG_COMPRESSED_UPDATE_OBJECT):
            seen_update = True
    if verify is None:
        raise AssertionError(f"{account}: no SMSG_LOGIN_VERIFY_WORLD")
    map_id, x, y, z, orientation = struct.unpack("<Iffff", verify)
    driver.position = (x, y, z, orientation)
    client.send_packet(smoke.CMSG_SET_ACTIVE_MOVER, struct.pack("<Q", guid))
    print(f"LOGIN-OK account={account} guid={guid} map={map_id} pos=({x:.2f},{y:.2f},{z:.2f})")
    return client, driver


def power1(driver: ScriptedDriver, guid: int) -> int | None:
    obj = driver.world.get(guid)
    return None if obj is None else obj.values.get(UNIT_FIELD_POWER1)


def face_guid(driver: ScriptedDriver, guid: int) -> None:
    pos = driver.world.position(guid)
    if pos is not None:
        driver.face(pos[0], pos[1])


def cast_unit_faced(
    driver: ScriptedDriver, spell_id: int, target: int, timeout: float = 10.0
) -> tuple[bool, int | None, list[int]]:
    """Face the target first (1.12.1 casts require facing); otherwise
    identical to `send_and_wait`."""
    face_guid(driver, target)
    return send_and_wait(driver, cast_unit_body(spell_id, target), spell_id, timeout, target_guid=target)


def cast_self_retry_power(
    driver: ScriptedDriver,
    spell_id: int,
    timeout: float = 10.0,
    retries: int = POWER_RETRIES,
) -> tuple[bool, int | None, list[int], int]:
    """Self cast with the `cast_until_dead` mana pacing treatment:
    NO_POWER(77) waits 9 s (five-second rule + regen tick) and retries.
    Returns (go, reason, ops, power_retries)."""
    waited = 0
    for _ in range(retries + 1):
        go, reason, ops = send_and_wait(driver, cast_self_body(spell_id), spell_id, timeout)
        if go or reason != SPELL_FAILED_NO_POWER or waited >= retries:
            return go, reason, ops, waited
        waited += 1
        driver.poll(9.0)
    return go, reason, ops, waited


def snapshot(driver: ScriptedDriver, guid: int) -> str:
    obj = driver.world.get(guid)
    if obj is None:
        return "absent"
    return (
        f"health={obj.values.get(UNIT_FIELD_HEALTH)} "
        f"max={obj.values.get(UNIT_FIELD_MAXHEALTH)} "
        f"power1={obj.values.get(UNIT_FIELD_POWER1)}"
    )


def live_dummy(driver: ScriptedDriver, entry: int, counters: list[int], exclude: int | None = None) -> int | None:
    """First counter of `entry` tracked with health > 0, else None."""
    for counter in counters:
        guid = raw_guid(entry, counter)
        if guid == exclude:
            continue
        if driver.target_health(guid) not in (None, 0):
            return guid
    return None


def ensure_battle_stance(
    driver: ScriptedDriver,
    guid: int,
) -> tuple[bool, int | None, list[int]]:
    """Enter Battle Stance (2457) so stance-gated Warrior rows are
    cast under fork-legal conditions, and verify its aura landed."""
    if aura_present(driver, guid, 2457):
        return True, None, []
    go, reason, ops = send_and_wait(driver, cast_self_body(2457), 2457, target_guid=guid)
    return bool(go and wait_for_aura(driver, guid, 2457)), reason, ops


def expected_failure_verdict(
    row: dict, go: bool, reason: int | None, packets: list[int], target_guid: int, attempts: int = 1
) -> dict:
    """Verdict for a matrix `expect_cast_failure` row.

    This row can never PASS. A shapeshift rejection is the pilot
    gate's expected FAIL; a landed cast is an unexpected engine FAIL.
    """
    spell_label = row.get("name") or f"spell {row.get('spell_id')}"
    if go:
        return make_verdict(
            row,
            "FAIL",
            attempts=attempts,
            phase="prepare",
            evidence=f"{spell_label} landed from the wrong form; "
            "stance/shape gate missing (engine finding, not a pass)",
            packets=packets,
            target_guid=target_guid,
        )
    expected = reason in (SPELL_FAILED_ONLY_SHAPESHIFT, SPELL_FAILED_NOT_SHAPESHIFT)
    if expected:
        evidence = f"expected wrong-form rejection for {spell_label}: CAST_RESULT({reason}) after {attempts} attempt(s)"
    elif reason == SPELL_FAILED_OUT_OF_RANGE:
        evidence = f"wrong-form premise untested for {spell_label}: all {attempts} attempt(s) ended OUT_OF_RANGE(89)"
    elif reason is None:
        evidence = f"no bound cast answer for {spell_label}"
    else:
        evidence = f"wrong-form cast for {spell_label} failed for an unclassified reason CAST_RESULT({reason})"
    return make_verdict(
        row,
        "FAIL",
        attempts=attempts,
        phase="prepare",
        reason=(f"CAST_RESULT({reason})" if reason is not None else None),
        evidence=evidence,
        packets=packets,
        target_guid=target_guid,
        expected_failure=expected,
    )


def _reset_target_ok(item: dict | None, require_dead: bool) -> bool:
    if not item:
        return False
    pre_hp = item.get("pre_hp")
    post_hp = item.get("post_hp")
    if pre_hp is None or post_hp is None:
        return False
    health_ok = post_hp == 0 if require_dead else post_hp < pre_hp
    return health_ok and bool(item.get("aura_seen") or item.get("damage_logs"))


def reset_evidence_ok(first: dict | None, second: dict | None) -> bool:
    """A reset is proven only by attributed damage to two counters."""
    if not first or not second or first.get("guid") == second.get("guid"):
        return False
    return _reset_target_ok(first, require_dead=True) and _reset_target_ok(second, require_dead=False)


def heal_jump_in_band(jump: float, band: list[int] | tuple[int, int]) -> bool:
    return float(band[0]) <= jump <= float(band[1])


def single_outcome_verdict(row: dict, observed: dict) -> dict:
    """Apply a row's generated observable outcome to one repro cast."""
    go = bool(observed.get("go"))
    reason = observed.get("reason")
    packets = observed.get("packets", [])
    target_guid = observed.get("target_guid")
    attempts = int(observed.get("attempts", 1))
    expected = row.get("expected", {})
    kind = expected.get("kind")
    pre_hp = observed.get("pre_hp")
    post_hp = observed.get("post_hp")
    health_fell = pre_hp is not None and post_hp is not None and post_hp < pre_hp
    damage_logs = observed.get("damage_logs", [])
    damage_total = sum(int(log.get("damage", 0)) for log in damage_logs)
    pre = observed.get("pre", "")
    post = observed.get("post", "")

    if kind == "expect_cast_failure":
        return expected_failure_verdict(row, go, reason, packets, target_guid, attempts=attempts)
    if kind == "resurrect_request":
        return make_verdict(
            row,
            "BLOCKED",
            attempts=attempts,
            reason="wire-target-unsupported",
            evidence="single repro has no corpse-target scenario; use the pilot classification leg",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=target_guid,
        )
    if kind == "heal_delta_target":
        return make_verdict(
            row,
            "BLOCKED",
            attempts=attempts,
            reason="scenario-missing-ally",
            evidence="single repro has no damaged-ally scenario; use the pilot heal leg",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=target_guid,
        )
    if not go:
        return make_verdict(
            row,
            "FAIL",
            attempts=attempts,
            phase="prepare",
            reason=(f"CAST_RESULT({reason})" if reason is not None else None),
            evidence=f"no bound SPELL_GO for spell {row.get('spell_id')} (reason={reason})",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=target_guid,
        )

    if kind in ("aura_on_caster", "form_change"):
        outcome_ok = bool(observed.get("caster_aura"))
        detail = f"caster_aura={observed.get('caster_aura')}"
    elif kind == "aura_on_target":
        outcome_ok = bool(observed.get("target_aura"))
        detail = f"target_aura={observed.get('target_aura')}"
        if (
            not outcome_ok
            and observed.get("pre_hp") not in (None, 0)
            and observed.get("post_hp") == 0
            and damage_logs
            and 2 not in (row.get("effect_ids") or [])
        ):
            # A damaging aura killed the 100-hp dummy before the
            # aura check ran: attributed damage logs prove the aura
            # applied (all damage rides the periodic aura; rows with
            # a direct-damage effect keep the aura-only gate).
            outcome_ok = True
            detail += f"; dummy died with {len(damage_logs)} attributed logs"
    elif kind == "health_delta_target":
        attribution = bool(observed.get("target_aura") or damage_logs)
        if not expected.get("aura"):
            attribution = True
        outcome_ok = health_fell and attribution
        detail = (
            f"health {pre_hp}->{post_hp}, target_aura={observed.get('target_aura')}, damage_logs={len(damage_logs)}"
        )
    elif kind == "damage_log":
        outcome_ok = health_fell and bool(damage_logs)
        detail = f"health {pre_hp}->{post_hp}, matching damage={damage_total}"
    elif kind == "object_created":
        if "object_seen" not in observed and not damage_logs:
            return make_verdict(
                row,
                "BLOCKED",
                attempts=attempts,
                reason="effect-marker-only",
                evidence="single repro cannot observe expected kind object_created (no dummy-in-radius setup)",
                pre=pre,
                post=post,
                packets=packets,
                target_guid=target_guid,
            )
        outcome_ok = bool(observed.get("object_seen")) or (health_fell and bool(damage_logs))
        detail = (
            f"object_seen={observed.get('object_seen')}, health {pre_hp}->{post_hp}, matching damage={damage_total}"
        )
    elif kind == "totem_summon":
        if "totem_seen" not in observed:
            return make_verdict(
                row,
                "BLOCKED",
                attempts=attempts,
                reason="effect-marker-only",
                evidence="single repro cannot observe expected kind totem_summon (no totem latch)",
                pre=pre,
                post=post,
                packets=packets,
                target_guid=target_guid,
            )
        outcome_ok = bool(observed.get("totem_seen"))
        detail = f"totem_seen={observed.get('totem_seen')}"
    elif kind == "item_consumed":
        if "reagent_before" not in observed:
            return make_verdict(
                row,
                "BLOCKED",
                attempts=attempts,
                reason="effect-marker-only",
                evidence="single repro cannot observe expected kind item_consumed (no reagent count read)",
                pre=pre,
                post=post,
                packets=packets,
                target_guid=target_guid,
            )
        before = observed.get("reagent_before")
        after = observed.get("reagent_after")
        outcome_ok = before is not None and after is not None and after < before
        detail = f"reagent {before}->{after}"
    elif kind == "dual_outcome":
        return make_verdict(
            row,
            "BLOCKED",
            attempts=attempts,
            reason="script-evidence-unknown",
            evidence="T25 dual row: needs both the friendly heal leg "
            "and the hostile damage leg; no single-bucket home until "
            "both legs pass",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=target_guid,
        )
    else:
        return make_verdict(
            row,
            "BLOCKED",
            attempts=attempts,
            reason="effect-marker-only",
            evidence=f"single repro cannot observe expected kind {kind}",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=target_guid,
        )
    if kind in ("health_delta_target", "damage_log", "aura_on_target") and "tick_band" in expected:
        # Periodic-damage rows additionally latch their 590 ticks in
        # the oracle band (hostile leg observes tick_amounts; paths
        # without a latch ride the base gate and say so).
        amounts = observed.get("tick_amounts")
        if amounts is None:
            detail += "; no tick latch on this path (band un-enforced: single repro)"
        else:
            tick_ok, tick_detail = tick_band_check(
                expected, amounts, observed.get("post_hp") != 0, observed.get("pre_hp")
            )
            outcome_ok = outcome_ok and tick_ok
            detail += "; " + tick_detail
    return make_verdict(
        row,
        "PASS" if outcome_ok else "FAIL",
        attempts=attempts,
        phase=None if outcome_ok else "apply",
        evidence=("observable outcome: " if outcome_ok else "SPELL_GO without required observable outcome: ") + detail,
        pre=pre,
        post=post,
        packets=packets,
        target_guid=target_guid,
    )


# --------------------------------------------------------------------------
# Pilot legs (§7)
# --------------------------------------------------------------------------


class Pilot:
    def __init__(self) -> None:
        self.verdicts: dict[str, list[dict]] = {}
        self.manifest_extra: dict = {
            "mode": "pilot",
            "matrix_classes": [],
            "entry_327": {},
            "accounts": {},
        }

    def record(self, class_name: str, v: dict) -> None:
        self.verdicts.setdefault(class_name, []).append(v)
        status = v["verdict"]
        extra = v.get("reason") or v.get("phase") or ""
        print(f"ROW {class_name} {v['spell_id']} {v['bucket']}: {status} {extra} | {v['evidence']}")

    def matrix_row(self, class_name: str, spell_id: int) -> dict:
        doc = json.loads(MATRIX_PATH.read_text())
        for r in doc["rows"]:
            if r["class"] == class_name and r["spell_id"] == spell_id:
                return r
        raise AssertionError(f"matrix lacks {class_name}/{spell_id}")

    # -- leg 1: Warrior -----------------------------------------------------
    def leg_warrior(self, guid: int) -> None:
        row = self.matrix_row("Warrior", 5242)
        client, driver = login_as("SPELLCAMP01", self.account_id("SPELLCAMP01"), guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Warrior")))
            pre = snapshot(driver, guid)
            go, reason, ops = send_and_wait(driver, cast_self_body(5242), 5242)
            driver.poll(2.0)
            post = snapshot(driver, guid)
            if go and aura_present(driver, guid, 5242):
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "PASS",
                        evidence="bound SPELL_GO + aura 5242 in self fields 47-94",
                        pre=pre,
                        post=post,
                        packets=ops,
                        target_guid=guid,
                    ),
                )
            else:
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="prepare" if not go else "apply",
                        reason=(f"CAST_RESULT({reason})" if reason is not None else None),
                        evidence=f"go={go} reason={reason} aura5242={aura_present(driver, guid, 5242)}",
                        pre=pre,
                        post=post,
                        packets=ops,
                        target_guid=guid,
                    ),
                )

            row = self.matrix_row("Warrior", 6546)
            dummy = live_dummy(driver, 705, WOLF_COUNTERS)
            if dummy is None:
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "BLOCKED",
                        reason="dummy-unavailable",
                        evidence="no 705 counter with health>0 in WorldState",
                    ),
                )
                return
            stance_ok, stance_reason, stance_ops = ensure_battle_stance(driver, guid)
            if not stance_ok:
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="prepare",
                        reason=(f"CAST_RESULT({stance_reason})" if stance_reason is not None else None),
                        evidence="Battle Stance setup did not verify aura 2457; Rend not cast under invalid conditions",
                        packets=stance_ops,
                        target_guid=dummy,
                    ),
                )
                return
            driver.approach_guid(dummy, 2.0)
            pre_hp = driver.target_health(dummy)
            pre = f"dummy guid={dummy} health={pre_hp}; stance aura=2457"
            go, reason, ops = cast_unit_faced(driver, 6546, dummy)
            ops = stance_ops + ops
            attempts = 1
            while go and attempts < 5 and not aura_present(driver, dummy, 6546):
                driver.poll(1.6)
                attempts += 1
                if not aura_present(driver, dummy, 6546):
                    go, reason, ops2 = cast_unit_faced(driver, 6546, dummy)
                    ops += ops2
            driver.poll(4.0)  # Rend ticks
            post_hp = driver.target_health(dummy)
            dot = aura_present(driver, dummy, 6546)
            if go and dot and pre_hp is not None and post_hp is not None and post_hp < pre_hp:
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "PASS",
                        attempts=attempts,
                        evidence="Battle Stance aura 2457 verified, then "
                        f"DoT aura 6546 on dummy guid={dummy} + health "
                        f"{pre_hp}->{post_hp}",
                        pre=pre,
                        post=f"dummy guid={dummy} health={post_hp}",
                        packets=ops,
                        target_guid=dummy,
                    ),
                )
            else:
                self.record(
                    "Warrior",
                    make_verdict(
                        row,
                        "FAIL",
                        attempts=attempts,
                        phase="apply" if go else "prepare",
                        reason=(f"CAST_RESULT({reason})" if not go and reason is not None else None),
                        evidence=f"stance_ok=True go={go} dot={dot} health {pre_hp}->{post_hp}",
                        pre=pre,
                        post=f"dummy guid={dummy} health={post_hp}",
                        packets=ops,
                        target_guid=dummy,
                    ),
                )
        finally:
            smoke.logout(client, "warrior-pilot")

    # -- leg 2: Mage --------------------------------------------------------
    def leg_mage(self, guid: int) -> None:
        row = self.matrix_row("Mage", 168)
        client, driver = login_as("SPELLCAMP07", self.account_id("SPELLCAMP07"), guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Mage")))
            pre = snapshot(driver, guid)
            go, reason, ops = send_and_wait(driver, cast_self_body(168), 168)
            driver.poll(1.5)
            post = snapshot(driver, guid)
            if go and aura_present(driver, guid, 168):
                seeded_count = len(pilot_spells_for("Mage"))
                smoke.logout(client, "mage-168-persist")
                total_rows = int(
                    smoke.sql_scalar("characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid}")
                )
                rows_168 = int(
                    smoke.sql_scalar(
                        "characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid} AND spell=168"
                    )
                )
                client, driver = login_as("SPELLCAMP07", self.account_id("SPELLCAMP07"), guid)
                drift_gate(driver, guid, seeded_count)
                persist_ok = total_rows == seeded_count and rows_168 == 1
                self.record(
                    "Mage",
                    make_verdict(
                        row,
                        "PASS" if persist_ok else "FAIL",
                        phase=None if persist_ok else "persist",
                        evidence="bound SPELL_GO + aura 168 on self; after "
                        f"logout character_spell total={total_rows} "
                        f"(seeded={seeded_count}), spell168 rows={rows_168}",
                        pre=pre,
                        post=post,
                        packets=ops,
                        target_guid=guid,
                    ),
                )
            else:
                self.record(
                    "Mage",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="prepare" if not go else "apply",
                        reason=(f"CAST_RESULT({reason})" if reason is not None else None),
                        evidence=f"go={go} aura168={aura_present(driver, guid, 168)}",
                        pre=pre,
                        post=post,
                        packets=ops,
                        target_guid=guid,
                    ),
                )

            row = self.matrix_row("Mage", 133)
            dummy = live_dummy(driver, 705, WOLF_COUNTERS)
            if dummy is None:
                self.record(
                    "Mage",
                    make_verdict(
                        row,
                        "BLOCKED",
                        reason="dummy-unavailable",
                        evidence="no 705 counter with health>0 in WorldState",
                    ),
                )
                return
            driver.approach_guid(dummy, 13.0)
            pre_hp = driver.target_health(dummy)
            pre = f"dummy guid={dummy} health={pre_hp}"
            mark = event_mark(driver)
            go, reason, _ops = cast_unit_faced(driver, 133, dummy, timeout=15.0)
            driver.poll(6.0)  # flight + damage log
            window = events_since(driver, mark)
            flight_ops = [op for op, _ in window]
            start_seen = SMSG_SPELL_START in flight_ops
            logs = matching_damage_logs(window, 133, dummy, guid)
            damage_total = sum(log["damage"] for log in logs)
            post_hp = driver.target_health(dummy)
            if go and start_seen and logs and pre_hp is not None and post_hp is not None and post_hp < pre_hp:
                self.record(
                    "Mage",
                    make_verdict(
                        row,
                        "PASS",
                        evidence="bound SPELL_START+SPELL_GO, then opcode "
                        f"592 body spell=133 target={dummy} attacker={guid} "
                        f"damage={damage_total}; dummy health "
                        f"{pre_hp}->{post_hp}",
                        pre=pre,
                        post=f"dummy guid={dummy} health={post_hp}",
                        packets=flight_ops,
                        target_guid=dummy,
                    ),
                )
            else:
                self.record(
                    "Mage",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="flight" if go else "prepare",
                        reason=(f"CAST_RESULT({reason})" if not go and reason is not None else None),
                        evidence=f"go={go} start={start_seen} "
                        f"matching_logs={len(logs)} damage={damage_total} "
                        f"health {pre_hp}->{post_hp}",
                        pre=pre,
                        post=f"dummy guid={dummy} health={post_hp}",
                        packets=flight_ops,
                        target_guid=dummy,
                    ),
                )
        finally:
            smoke.logout(client, "mage-pilot")

    # -- legs 3-5: Druid ----------------------------------------------------
    def leg_druid(self, guid: int, ally_guid: int) -> None:
        client, driver = login_as("SPELLCAMP09", self.account_id("SPELLCAMP09"), guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Druid")))
            # Healing leg needs a damaged ally: ally was SQL-damaged
            # while offline (saved low health persists through load,
            # which only clamps downward-overshoot to the max).
            ally_client, ally_driver = login_as("SPELLCAMP01", self.account_id("SPELLCAMP01"), ally_guid)
            try:
                # Regen-baselined heal measurement on the ally's own
                # socket (its updates are authoritative; the caster's
                # cross-client view of ally health proved stale).
                # Rank 3 (oracle band ~204-254: MaxLevel=19 clamps the
                # fork level term to min(60,19)-14=5, value =
                # 194+5*1.9+irand(1,49)) separates completely from the
                # measured regen-tick noise (+34/1.5s): no-heal windows
                # top out at +34, a landed heal starts at 204-34=170.
                row = self.matrix_row("Druid", 5187)
                ally_driver.poll(1.0)
                t0 = time.monotonic()
                h0 = ally_driver.self_health()
                ally_driver.poll(3.0)
                t1 = time.monotonic()
                h1 = ally_driver.self_health()
                pre = f"ally guid={ally_guid} health={h0}->{h1}"
                go, reason, ops = cast_unit_faced(driver, 5187, ally_guid, timeout=12.0)
                if not go and reason == SPELL_FAILED_NO_POWER:
                    driver.poll(9.0)
                    go, reason, ops = cast_unit_faced(driver, 5187, ally_guid, timeout=12.0)
                ally_driver.poll(4.0)
                t2 = time.monotonic()
                h2 = ally_driver.self_health()
                if go and h0 is not None and h1 is not None and h2 is not None and t1 > t0:
                    rate = (h1 - h0) / (t1 - t0)
                    jump = h2 - (h1 + rate * (t2 - t1))
                    heal_band = row["expected"].get("heal_band", [204, 254])
                    jump_band = row["expected"].get("observed_jump_band", [170, 288])
                    # Oracle heal is 204-254; a discrete regen tick
                    # can phase-shift the baselined jump by one tick
                    # (+34), hence the generated observed band. The
                    # check is bounded on both sides.
                    if heal_jump_in_band(jump, jump_band):
                        self.record(
                            "Druid",
                            make_verdict(
                                row,
                                "PASS",
                                evidence="Healing Touch rank 3 bound GO + "
                                "regen-baselined jump "
                                f"{jump:.0f} within observed band "
                                f"{jump_band[0]}-{jump_band[1]} (oracle heal "
                                f"band {heal_band[0]}-{heal_band[1]}, rate "
                                f"{rate:.1f}/s, {h0}->{h1} pre, got {h2})",
                                pre=pre,
                                post=f"ally health={h2}",
                                packets=ops,
                                target_guid=ally_guid,
                            ),
                        )
                    else:
                        self.record(
                            "Druid",
                            make_verdict(
                                row,
                                "FAIL",
                                phase="apply",
                                evidence=f"GO but jump {jump:.0f} outside "
                                f"observed band {jump_band[0]}-{jump_band[1]} "
                                f"(oracle heal band {heal_band[0]}-"
                                f"{heal_band[1]}, rate {rate:.1f}/s, "
                                f"{h0}->{h1} pre, got {h2})",
                                pre=pre,
                                post=f"ally health={h2}",
                                packets=ops,
                                target_guid=ally_guid,
                            ),
                        )
                else:
                    self.record(
                        "Druid",
                        make_verdict(
                            row,
                            "FAIL",
                            phase="prepare",
                            reason=(f"CAST_RESULT({reason})" if not go and reason is not None else None),
                            evidence=f"cast failed or missing update: go={go} reason={reason} h={h0}->{h1}->{h2}",
                            pre=pre,
                            post=f"ally health={h2 or '?'}",
                            packets=ops,
                            target_guid=ally_guid,
                        ),
                    )
            finally:
                smoke.logout(ally_client, "druid-ally")

            # Wrong-form leg first: Growl (bear-form gated) from
            # caster form (guaranteed shapeshift-free this early).
            # Expected: shapeshift CAST_RESULT failure, recorded as
            # the gate's expected FAIL/prepare. A landed cast is an
            # unexpected engine FAIL, never PASS.
            row = self.matrix_row("Druid", 6795)
            if aura_present(driver, guid, 5487) or aura_present(driver, guid, 768):
                self.record(
                    "Druid",
                    make_verdict(
                        row,
                        "BLOCKED",
                        reason="scenario-missing-ally",
                        evidence="already shapeshifted; wrong-form premise unusable",
                        target_guid=guid,
                    ),
                )
            else:
                candidates = [
                    raw_guid(705, counter)
                    for counter in WOLF_COUNTERS
                    if driver.target_health(raw_guid(705, counter)) not in (None, 0)
                ]
                if not candidates:
                    self.record(
                        "Druid",
                        make_verdict(
                            row, "BLOCKED", reason="dummy-unavailable", evidence="no live 705 for wrong-form Growl"
                        ),
                    )
                else:
                    # Roaming wolves can drift out of Growl range
                    # between approach and cast. Re-approach and then
                    # move to another live counter on OUT_OF_RANGE(89)
                    # so range cannot mask the stance gate under test.
                    attempts = 0
                    go, reason, all_ops = False, None, []
                    dummy = candidates[0]
                    for candidate in candidates:
                        dummy = candidate
                        for _retry in range(2):
                            attempts += 1
                            driver.approach_guid(dummy, 1.0)
                            go, reason, ops = cast_unit_faced(driver, 6795, dummy)
                            all_ops += ops
                            if go or reason != SPELL_FAILED_OUT_OF_RANGE:
                                break
                            driver.poll(1.0)
                        if go or reason != SPELL_FAILED_OUT_OF_RANGE:
                            break
                    self.record("Druid", expected_failure_verdict(row, go, reason, all_ops, dummy, attempts=attempts))

            # Form legs: Bear then Cat replaces it. Cat is followed
            # by logout/login (§7.4) and the relog aura state is
            # recorded in its verdict either way.
            for spell, aura, label in ((5487, 5487, "bear"), (768, 768, "cat")):
                row = self.matrix_row("Druid", spell)
                pre = snapshot(driver, guid)
                cancel_note = ""
                if spell == 768 and aura_present(driver, guid, 5487):
                    # Stock-shaped form switch: Cat Form carries
                    # SPELL_ATTR_NOT_SHAPESHIFT, so the fork gate
                    # rejects a direct Bear -> Cat cast. Cancel
                    # Bear (CMSG_CANCEL_AURA 310) first.
                    driver.client.send_packet(310, struct.pack("<I", 5487))
                    driver.poll(1.5)
                    cancel_note = f"; cancelled Bear first (bear_aura_after_cancel={aura_present(driver, guid, 5487)})"
                go, reason, ops, waited = cast_self_retry_power(driver, spell)
                _ = label
                driver.poll(1.5)
                post = snapshot(driver, guid)
                if go and aura_present(driver, guid, aura):
                    relog_note = ""
                    relog_bear = False
                    if spell == 768:
                        bear_before_relog = aura_present(driver, guid, 5487)
                        smoke.logout(client, "druid-form-persist")
                        client, driver = login_as("SPELLCAMP09", self.account_id("SPELLCAMP09"), guid)
                        drift_gate(driver, guid, len(pilot_spells_for("Druid")))
                        relog_bear = aura_present(driver, guid, 5487)
                        relog_cat = aura_present(driver, guid, 768)
                        relog_note = (
                            f"; before relog bear={bear_before_relog}, after relog bear={relog_bear} cat={relog_cat}"
                        )
                        post = snapshot(driver, guid)
                    if spell == 768 and (aura_present(driver, guid, 5487) or relog_bear):
                        self.record(
                            "Druid",
                            make_verdict(
                                row,
                                "FAIL",
                                phase="apply",
                                evidence="cat aura present but bear aura stuck" + relog_note,
                                pre=pre,
                                post=post,
                                packets=ops,
                                target_guid=guid,
                            ),
                        )
                    else:
                        self.record(
                            "Druid",
                            make_verdict(
                                row,
                                "PASS",
                                evidence=f"bound SPELL_GO + form aura {aura} "
                                "on self"
                                + (" (bear gone)" if spell == 768 else "")
                                + (f" after {waited} mana wait(s)" if waited else "")
                                + cancel_note
                                + relog_note,
                                pre=pre,
                                post=post,
                                packets=ops,
                                target_guid=guid,
                            ),
                        )
                else:
                    self.record(
                        "Druid",
                        make_verdict(
                            row,
                            "FAIL",
                            phase="prepare" if not go else "apply",
                            reason=(f"CAST_RESULT({reason})" if reason is not None else None),
                            evidence=f"go={go} aura{aura}={aura_present(driver, guid, aura)}",
                            pre=pre,
                            post=post,
                            packets=ops,
                            target_guid=guid,
                        ),
                    )

            # Corpse-bucket classification: fork-shaped corpse packet
            # at a live ally guid (not a corpse object). The parser
            # now returns corpse targets, so the cast must read the
            # fork's BAD_TARGETS for a guid with no corpse behind
            # it; the real resurrect scenario rides run-class.
            row = self.matrix_row("Druid", 20484)
            go, reason, ops = send_and_wait(driver, cast_corpse_body(20484, ally_guid), 20484, timeout=6.0)
            driver.poll(0.5)
            alive = True
            try:
                driver.client.send_packet(CMSG_SET_ACTIVE_MOVER, struct.pack("<Q", guid))
                driver.poll(0.5)
            except Exception:
                alive = False
            if not go and reason in (SPELL_FAILED_BAD_TARGETS, SPELL_FAILED_NOT_SHAPESHIFT):
                self.record(
                    "Druid",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="prepare",
                        reason=f"CAST_RESULT({reason})",
                        expected_failure=True,
                        evidence="corpse-mask cast at a non-corpse guid is "
                        "rejected fork-exact (BAD_TARGETS, or NOT_SHAPESHIFT "
                        "from the persisted Cat Form, checked first); socket "
                        f"alive={alive}",
                        packets=ops,
                        target_guid=ally_guid,
                    ),
                )
            elif not go and reason is None:
                self.record(
                    "Druid",
                    make_verdict(
                        row,
                        "BLOCKED",
                        reason="wire-target-unsupported",
                        evidence="corpse-mask cast: no bound SPELL_GO or "
                        "CAST_RESULT (parser discards corpse GUIDs); socket "
                        f"alive={alive}",
                        packets=ops,
                        target_guid=ally_guid,
                    ),
                )
            elif go:
                self.record(
                    "Druid",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="apply",
                        evidence="corpse-target cast reached bound SPELL_GO "
                        "against a live stand-in; parser behavior changed, "
                        "but this scenario has no resurrect-request evidence "
                        "and cannot PASS",
                        packets=ops,
                        target_guid=ally_guid,
                    ),
                )
            else:
                self.record(
                    "Druid",
                    make_verdict(
                        row,
                        "FAIL",
                        phase="prepare",
                        reason=f"CAST_RESULT({reason})",
                        evidence="corpse cast answered CAST_RESULT; parser or gate changed — needs §8 triage",
                        packets=ops,
                        target_guid=ally_guid,
                    ),
                )

            # Dummy-reset leg (§7.6, Druid-local equivalent): kill
            # one wolf with Moonfire, then prove a different counter
            # is targetable by damaging it in the same session. The
            # architecture's Fireball wording belongs to the Mage
            # session; this leg's evidence standard is the same:
            # attributed health/aura/damage-log outcomes per GUID.
            row = self.matrix_row("Druid", 8921)
            reset = self.dummy_reset_leg(driver, guid)
            first = reset.get("first") or {}
            second = reset.get("second") or {}

            def describe(item: dict) -> str:
                if not item:
                    return "missing"
                damage = sum(log["damage"] for log in item.get("damage_logs", []))
                return (
                    f"guid={item.get('guid')} health "
                    f"{item.get('pre_hp')}->{item.get('post_hp')} "
                    f"aura8921={item.get('aura_seen')} "
                    f"matching_damage={damage}"
                )

            reset_ok = bool(reset.get("ok"))
            self.record(
                "Druid",
                make_verdict(
                    row,
                    "PASS" if reset_ok else "FAIL",
                    attempts=int(reset.get("attempts", 1)),
                    phase=None if reset_ok else "apply",
                    evidence="dummy-reset leg: first " + describe(first) + "; second " + describe(second),
                    pre=(f"first guid={first.get('guid')} health={first.get('pre_hp')}" if first else "no first dummy"),
                    post=(
                        f"second guid={second.get('guid')} health={second.get('post_hp')}"
                        if second
                        else "no second dummy"
                    ),
                    packets=reset.get("packets", []),
                    target_guid=first.get("guid"),
                ),
            )
        finally:
            smoke.logout(client, "druid-pilot")

    def moonfire_evidence(
        self, driver: ScriptedDriver, caster_guid: int, dummy: int, *, kill: bool, max_casts: int = 6
    ) -> dict:
        driver.approach_guid(dummy, 13.0)
        pre_hp = driver.target_health(dummy)
        evidence = {
            "guid": dummy,
            "pre_hp": pre_hp,
            "post_hp": pre_hp,
            "aura_seen": False,
            "damage_logs": [],
            "packets": [],
            "attempts": 0,
            "go": False,
            "reason": None,
        }
        for _ in range(max_casts):
            if driver.target_health(dummy) == 0:
                break
            mark = event_mark(driver)
            go, reason, ops = cast_unit_faced(driver, 8921, dummy, timeout=15.0)
            evidence["attempts"] += 1
            evidence["go"] = go
            evidence["reason"] = reason
            evidence["packets"] += ops
            for _poll in range(3):
                driver.poll(1.0)
                evidence["aura_seen"] = bool(evidence["aura_seen"] or aura_present(driver, dummy, 8921))
            window = events_since(driver, mark)
            evidence["packets"] += [op for op, _ in window]
            evidence["damage_logs"] += matching_damage_logs(window, 8921, dummy, caster_guid)
            evidence["post_hp"] = driver.target_health(dummy)
            if kill and evidence["post_hp"] == 0:
                break
            if (
                not kill
                and evidence["post_hp"] is not None
                and pre_hp is not None
                and evidence["post_hp"] < pre_hp
                and (evidence["aura_seen"] or evidence["damage_logs"])
            ):
                break
            if not go:
                driver.poll(1.6)
        evidence["post_hp"] = driver.target_health(dummy)
        return evidence

    def dummy_reset_leg(self, driver: ScriptedDriver, guid: int) -> dict:
        # Leave cat form first (Moonfire is caster-form-only): the
        # stock client cancels shapeshift with CMSG_CANCEL_AURA (310).
        if aura_present(driver, guid, 768) or aura_present(driver, guid, 5487):
            form = 768 if aura_present(driver, guid, 768) else 5487
            driver.client.send_packet(310, struct.pack("<I", form))
            driver.poll(1.5)
        first_guid = live_dummy(driver, 705, WOLF_COUNTERS)
        if first_guid is None:
            return {"ok": False, "first": None, "second": None, "packets": [], "attempts": 0}
        first = self.moonfire_evidence(driver, guid, first_guid, kill=True)
        second_guid = live_dummy(driver, 705, WOLF_COUNTERS, exclude=first_guid)
        second = None
        if second_guid is not None:
            second = self.moonfire_evidence(driver, guid, second_guid, kill=False, max_casts=3)
        return {
            "ok": reset_evidence_ok(first, second),
            "first": first,
            "second": second,
            "packets": first["packets"] + (second or {}).get("packets", []),
            "attempts": first["attempts"] + (second or {}).get("attempts", 0),
        }

    def account_id(self, account: str) -> int:
        found = smoke.sql_scalar("realmd", f"SELECT id FROM account WHERE username='{account}'")
        return int(found)


def guid_of(name: str) -> int:
    found = smoke.sql_scalar("characters", f"SELECT guid FROM characters WHERE name='{name}'")
    if not found:
        raise AssertionError(f"no character named {name}; run fixtures first")
    return int(found)


def acquire_lock() -> None:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    if LOCK_PATH.exists():
        try:
            pid = int(LOCK_PATH.read_text().split()[0])
            os.kill(pid, 0)
            raise SystemExit(f"campaign lock held by live pid {pid}; refusing to run")
        except (OSError, ValueError):
            pass
    LOCK_PATH.write_text(f"{os.getpid()} {time.time()}\n")


def release_lock() -> None:
    try:
        LOCK_PATH.unlink()
    except FileNotFoundError:
        pass


def run_leg_with_log(class_name: str, leg: object) -> str:
    """Run one class leg capturing its raw stdout (§6 run-<class>.log)."""
    import contextlib
    import io

    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        leg()
    text = buf.getvalue()
    print(text, end="")
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    with open(EVIDENCE_DIR / f"run-{class_name}.log", "w") as fh:
        fh.write(text)
    return text


EXPECTED_PILOT_COUNTS = {"Warrior": 2, "Mage": 2, "Druid": 6}

CLASSIFY_BLOCKED_REASONS = {
    "script-evidence-unknown",
    "effect-marker-only",
    "no-party",
    "no-pet",
    "fixture-gear",
    "fixture-reagent",
    "target-not-dead",
    # Corpse-target rows ride classify-only until the RESURRECT wire
    # seam lands (the pilot gate's pinned Rebirth, generalized).
    "wire-target-unsupported",
    # Totem rows with a cast-shaped expectation stay uncast until
    # the totem engine slice lands (live failing-cast evidence per
    # row, e.g. Sentry 6495 dest-cast answers CAST_RESULT(145)).
    "engine-totem",
}


def class_gate_failures(
    all_verdicts: dict[str, list[dict]],
    expected_counts: dict[str, int],
) -> list[tuple[str, int, str]]:
    """Fan-out gate for run_class rows. A PASS row is gate-compatible
    only with non-empty evidence and a bound SPELL_GO (306) in its
    packets: GO is necessary but never sufficient, and a PASS without
    the bound GO is a vacuous claim. FAIL rows stop the gate except
    the expected-failure shapeshift form; BLOCKED rows are compatible
    only for classify-only buckets (left uncast by design)."""
    failures: list[tuple[str, int, str]] = []
    for class_name, expected in expected_counts.items():
        rows = all_verdicts.get(class_name, [])
        if len(rows) != expected:
            failures.append((class_name, 0, f"row count {len(rows)} != expected {expected}"))
        for verdict in rows:
            status = verdict["verdict"]
            if status == "PASS":
                if not verdict.get("evidence"):
                    failures.append((class_name, int(verdict["spell_id"]), "PASS with empty evidence"))
                elif SMSG_SPELL_GO not in (verdict.get("packets") or []):
                    failures.append(
                        (class_name, int(verdict["spell_id"]), "PASS with no bound SPELL_GO (306) in packets")
                    )
            elif status == "FAIL" and verdict.get("expected_failure"):
                continue
            elif status == "BLOCKED" and verdict.get("reason") in CLASSIFY_BLOCKED_REASONS:
                continue
            elif status == "PASSIVE-VERIFIED":
                continue
            else:
                failures.append((class_name, int(verdict["spell_id"]), f"{status}: {verdict.get('evidence')}"))
    return failures


def pilot_gate_failures(
    all_verdicts: dict[str, list[dict]],
    expected_counts: dict[str, int] | None = None,
) -> list[tuple[str, int, str]]:
    """Rows that must stop fan-out. Only a generated expected
    shapeshift FAIL and the pinned corpse-target BLOCKED are gate-
    compatible non-PASS classifications."""
    expected_counts = expected_counts or EXPECTED_PILOT_COUNTS
    failures: list[tuple[str, int, str]] = []
    for class_name, expected in expected_counts.items():
        rows = all_verdicts.get(class_name, [])
        if len(rows) != expected:
            failures.append((class_name, 0, f"row count {len(rows)} != expected {expected}"))
        for verdict in rows:
            if verdict["verdict"] == "PASS":
                continue
            if verdict["verdict"] == "FAIL" and verdict.get("expected_failure"):
                continue
            if (
                verdict["verdict"] == "BLOCKED"
                and verdict["spell_id"] == 20484
                and verdict.get("reason") == "wire-target-unsupported"
            ):
                continue
            failures.append((class_name, int(verdict["spell_id"]), f"{verdict['verdict']}: {verdict['evidence']}"))
    return failures


def run_pilot() -> int:
    if not MATRIX_PATH.exists():
        print("matrix missing; run spell_matrix_gen.py --pilot first", file=sys.stderr)
        return 2
    acquire_lock()
    pilot = Pilot()
    pilot.manifest_extra["started"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    pilot.manifest_extra["setup_spells"] = {name: list(spells) for name, spells in SETUP_SPELLS.items()}
    try:
        pilot.manifest_extra["db_hostname"] = smoke.sql_scalar("realmd", "SELECT @@hostname")
        entry = smoke.sql_scalar("mangos", "SELECT COUNT(*) FROM creature_template WHERE entry=327")
        spawns = smoke.sql_scalar("mangos", "SELECT COUNT(*) FROM creature WHERE id=327")
        pilot.manifest_extra["entry_327"] = {"template": entry, "spawns": spawns}
        warrior = guid_of("Campwarrior")
        mage = guid_of("Campmage")
        druid = guid_of("Campdruid")
        pilot.manifest_extra["accounts"] = {
            "SPELLCAMP01": pilot.account_id("SPELLCAMP01"),
            "SPELLCAMP07": pilot.account_id("SPELLCAMP07"),
            "SPELLCAMP09": pilot.account_id("SPELLCAMP09"),
        }
        pilot.manifest_extra["guids"] = {"Campwarrior": warrior, "Campmage": mage, "Campdruid": druid}
        pilot.manifest_extra["matrix_classes"] = ["Warrior", "Mage", "Druid"]

        seed_character(warrior, pilot_spells_for("Warrior"))
        run_leg_with_log("Warrior", lambda: pilot.leg_warrior(warrior))

        seed_character(mage, pilot_spells_for("Mage"))
        run_leg_with_log("Mage", lambda: pilot.leg_mage(mage))

        # Ally must be offline for the SQL damage write, then online
        # beside the druid at a damaged value.
        seed_character(warrior, pilot_spells_for("Warrior"), pos=ALLY_POS)
        smoke.sql_scalar("characters", f"UPDATE characters SET health=300 WHERE guid={warrior}")
        seed_character(druid, pilot_spells_for("Druid"), pos=HEAL_CASTER_POS)
        run_leg_with_log("Druid", lambda: pilot.leg_druid(druid, warrior))
    finally:
        for name in ("Campwarrior", "Campmage", "Campdruid"):
            try:
                smoke.ensure_offline(guid_of(name))
            except Exception as exc:
                print(f"WARN offline restore {name}: {exc}")
        release_lock()
    for class_name, verdicts in pilot.verdicts.items():
        write_results(class_name, verdicts)
    write_summary(pilot.verdicts)
    doc = json.loads(MATRIX_PATH.read_text())
    pilot.manifest_extra["measured_notes"] = [
        "heal leg escalated 5185->5187 mid-pilot: rank-1 oracle band "
        "is 40-54 (MaxLevel=5 clamp per calculate_effect_value_bp) but "
        "bench regen ticks a deterministic +34/1.5s, so one window "
        "cannot separate a rank-1 heal from a phase-aligned tick "
        "(triple-cast jumps 49/23/29 mean 34); rank 3 band ~204-254 "
        "(MaxLevel=19 clamp) separates completely. This pilot casts "
        "pinned 5187 (rank 3/11), not last rank 25297; generated rows "
        "report rank metadata explicitly",
        "bench login restores health above the saved value "
        "(SQL 300 loaded at ~600 in the /tmp/health_probe.py run), "
        "then ~17/s out-of-combat regen at 60: heal legs must be "
        "regen-baselined, raw deltas over-attribute",
        "cross-client WorldState health for the ally proved stale "
        "(caster view 27->35 while ally self-view rose hundreds); "
        "the heal leg reads the ally's own socket",
        "CMSG_CHAR_CREATE before CMSG_CHAR_ENUM always answers "
        "SERVER_LIMIT(52): the actor defaults the session count to 10 "
        "and only the enum burst lowers it (fixtures ensure sends "
        "enum first)",
    ]
    write_manifest(
        {
            **pilot.manifest_extra,
            "talent_builds": doc.get("talent_builds", {}),
            "per_class_counts": doc.get("per_class_counts", {}),
            "matrix_mode": doc.get("mode"),
            "g1_skillraceclassinfo": doc.get("g1_skillraceclassinfo", ""),
            "g2_talent_builds": doc.get("g2_talent_builds", ""),
            "caveats": [
                doc.get("g3_isspellvalid", ""),
                doc.get("g4_script_evidence", ""),
                doc.get("g5_trainer_equivalence", ""),
            ],
        }
    )
    per_class = doc.get("per_class_counts", {})
    unexpected = pilot_gate_failures(
        pilot.verdicts, {name: per_class.get(name, EXPECTED_PILOT_COUNTS[name]) for name in EXPECTED_PILOT_COUNTS}
    )
    if unexpected:
        print(f"PILOT-LEG-FAILED {unexpected}")
        return 1
    print("SPELL-CAMPAIGN-PILOT-PASS")
    return 0


def run_single(spell_id: int, char_name: str) -> int:
    """Single-spell repro for the §8 fix loop.

    Uses the generated row's observable-outcome verdict path. A bound
    SPELL_GO is necessary but never sufficient for PASS.
    """
    if not MATRIX_PATH.exists():
        print("matrix missing; run spell_matrix_gen.py --pilot first", file=sys.stderr)
        return 2
    guid = guid_of(char_name)
    doc = json.loads(MATRIX_PATH.read_text())
    rows = [r for r in doc["rows"] if r["spell_id"] == spell_id]
    for fanout_class in ("Paladin", "Druid"):
        try:
            extra = matrix_doc_for(fanout_class).get("rows", [])
        except (OSError, ValueError):
            extra = []
        seen = {(r.get("class"), r.get("spell_id")) for r in rows}
        rows += [r for r in extra if r["spell_id"] == spell_id and (r.get("class"), r.get("spell_id")) not in seen]
    if not rows:
        print(f"spell {spell_id} not in pilot matrix", file=sys.stderr)
        return 2
    row = [r for r in rows if r["class"] in ("Warrior", "Mage", "Druid", "Paladin")][0]
    account = CLASS_ACCOUNT[row["class"]]
    expected = row.get("expected", {})
    bucket = row.get("bucket")
    if expected.get("kind") == "learned_spell":
        have = smoke.sql_scalar(
            "characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid} AND spell={spell_id}"
        )
        verified = int(have or 0) > 0
        verdict = make_verdict(
            row,
            "PASSIVE-VERIFIED" if verified else "PASSIVE-UNVERIFIED",
            evidence=f"character_spell rows for {spell_id}={have}",
        )
        print(json.dumps(verdict, indent=1))
        return 0 if verified else 1
    if bucket == "friendly":
        print(
            json.dumps(
                make_verdict(
                    row,
                    "BLOCKED",
                    reason="scenario-missing-ally",
                    evidence="single repro has no damaged-ally session; use run-class for the ally leg",
                ),
                indent=1,
            )
        )
        return 1
    if bucket == "party_duel":
        print(
            json.dumps(
                make_verdict(row, "BLOCKED", reason="no-party", evidence="single repro has no party; use run-class"),
                indent=1,
            )
        )
        return 1
    if bucket in ("ground_area", "dual"):
        print(
            json.dumps(
                single_outcome_verdict(
                    row,
                    {
                        "go": False,
                        "reason": None,
                        "packets": [],
                        "target_guid": None,
                    },
                ),
                indent=1,
            )
        )
        return 1
    if bucket == "corpse":
        print(
            json.dumps(
                make_verdict(
                    row,
                    "BLOCKED",
                    reason="wire-target-unsupported",
                    evidence="single repro has no corpse-target scenario",
                ),
                indent=1,
            )
        )
        return 1
    if expected.get("kind") == "none_marker":
        print(
            json.dumps(
                make_verdict(
                    row,
                    "BLOCKED",
                    reason="effect-marker-only",
                    evidence=row.get("expected", {}).get("note", "classify-only"),
                    pre="",
                    post="",
                ),
                indent=1,
            )
        )
        return 1
    if bucket == "item_reagent_ammo":
        targets = set(row.get("target_a", []) + row.get("target_b", []))
        if targets & {35, 37, 57, 58, 61}:
            print(
                json.dumps(
                    make_verdict(row, "BLOCKED", reason="no-party", evidence="single repro has no raid/party target"),
                    indent=1,
                )
            )
            return 1
    acquire_lock()
    try:
        client, driver = login_as(
            account, int(smoke.sql_scalar("realmd", f"SELECT id FROM account WHERE username='{account}'")), guid
        )
        try:
            pre = snapshot(driver, guid)
            target: int | None = guid
            pre_hp = None
            if row["bucket"] == "hostile_dummy" or (expected.get("kind") == "expect_cast_failure"):
                target = live_dummy(driver, 705, WOLF_COUNTERS)
                if target is None:
                    verdict = make_verdict(
                        row,
                        "BLOCKED",
                        reason="dummy-unavailable",
                        evidence="no 705 counter with health>0 in WorldState",
                        pre=pre,
                    )
                    print(json.dumps(verdict, indent=1))
                    return 1
                standoff = 2.0 if row.get("range_index") == 2 else 13.0
                driver.approach_guid(target, standoff)
                pre_hp = driver.target_health(target)
                pre = f"target guid={target} health={pre_hp}; caster {pre}"
                if spell_id == 6546:
                    stance_ok, stance_reason, stance_ops = ensure_battle_stance(driver, guid)
                    if not stance_ok:
                        verdict = make_verdict(
                            row,
                            "FAIL",
                            phase="prepare",
                            reason=(f"CAST_RESULT({stance_reason})" if stance_reason is not None else None),
                            evidence="Battle Stance setup did not verify aura 2457; Rend not cast",
                            pre=pre,
                            packets=stance_ops,
                            target_guid=target,
                        )
                        print(json.dumps(verdict, indent=1))
                        return 1
                mark = event_mark(driver)
                go, reason, _ops = cast_unit_faced(driver, spell_id, target, timeout=15.0)
            else:
                mark = event_mark(driver)
                go, reason, _ops = send_and_wait(
                    driver, cast_self_body(spell_id), spell_id, timeout=15.0, target_guid=guid
                )
            driver.poll(6.0 if float(row.get("speed") or 0) > 0 else 4.0)
            window = events_since(driver, mark)
            post_hp = driver.target_health(target) if target is not None and target != guid else None
            post = snapshot(driver, guid)
            if target is not None and target != guid:
                post = f"target guid={target} health={post_hp}; caster {post}"
            expected_aura = int(expected.get("aura", spell_id))
            verdict = single_outcome_verdict(
                row,
                {
                    "go": go,
                    "reason": reason,
                    "caster_aura": aura_present(driver, guid, expected_aura),
                    "target_guid": target,
                    "target_aura": (aura_present(driver, target, expected_aura) if target is not None else False),
                    "pre_hp": pre_hp,
                    "post_hp": post_hp,
                    "damage_logs": (matching_damage_logs(window, spell_id, target, guid) if target is not None else []),
                    "tick_amounts": (
                        matching_periodic_ticks(window, spell_id, target, guid) if target is not None else None
                    ),
                    "pre": pre,
                    "post": post,
                    "packets": [op for op, _ in window],
                },
            )
            print(json.dumps(verdict, indent=1))
            return 0 if (verdict["verdict"] == "PASS" or verdict.get("expected_failure")) else 1
        finally:
            smoke.logout(client, f"run-{spell_id}")
    finally:
        release_lock()


def wait_for_aura(driver: ScriptedDriver, guid: int, spell_id: int, timeout_s: float = 8.0) -> bool:
    """Poll until the aura is observable. A single post-cast check
    races slow applies under bench load (one Berserker Stance entry
    bound GO with the aura arriving late; one Recklessness with the
    holder still missing at +5 s); a truly missing aura still reads
    absent after the timeout, so this only converts timing flakes,
    never verdicts."""
    waited = 0.0
    while waited < timeout_s:
        if aura_present(driver, guid, spell_id):
            return True
        driver.poll(0.5)
        waited += 0.5
    return aura_present(driver, guid, spell_id)


def cancel_auras(
    driver: ScriptedDriver, guid: int, spell_ids: list[int], force: tuple[int, ...] = ()
) -> dict[int, bool]:
    """Cancel cancelable auras (CMSG_CANCEL_AURA 310) and poll gone.
    Forced ids are sent even with no visible holder: the fork
    interrupts a matching channeled cast slot-side, which is how a
    holder-less channel (Hurricane's radius) is stopped."""
    cleared: dict[int, bool] = {}
    for spell_id in spell_ids:
        if spell_id in force or aura_present(driver, guid, spell_id):
            driver.client.send_packet(310, struct.pack("<I", spell_id))
            driver.poll(1.5)
            cleared[spell_id] = not aura_present(driver, guid, spell_id)
    return cleared


def cast_unit_retry_power(
    driver: ScriptedDriver,
    spell_id: int,
    target: int,
    timeout: float = 10.0,
    retries: int = POWER_RETRIES,
) -> tuple[bool, int | None, list[int], int]:
    """Unit-cast twin of cast_self_retry_power (NO_POWER pacing)."""
    waited = 0
    go, reason, ops = cast_unit_faced(driver, spell_id, target, timeout)
    while not go and reason == SPELL_FAILED_NO_POWER and waited < retries:
        waited += 1
        driver.poll(9.0)
        go, reason, ops = cast_unit_faced(driver, spell_id, target, timeout)
    return go, reason, ops, waited


def cooldown_wait_s(recovery_ms: object, category_ms: object, margin_s: float = 5.0, cap_s: float = 600.0) -> float:
    """Pure full-cooldown wait: max(recovery, category) plus margin, capped.

    The matrix carries only recoveryTime, but same-chain ranks share
    the fork's categoryRecoveryTime (Wyvern Sting 24132/24133 share
    category 1111 at 120 s) which also rejects with NOT_READY. The
    live lookup below feeds this; the cap matches not_ready_retry so
    one row cannot stall the campaign.
    """
    try:
        wait_s = max(float(recovery_ms or 0), float(category_ms or 0)) / 1000.0 + margin_s
    except (TypeError, ValueError):
        wait_s = margin_s
    return min(max(wait_s, margin_s), cap_s)


def live_cooldown_wait_s(spell_id: int, cap_s: float = 600.0) -> float:
    """Full fork cooldown wait for a NOT_READY row (live bench oracle).

    Resolves max(recoveryTime, categoryRecoveryTime) over the
    spell_template builds and waits it out plus margin. Bench-only
    read on a rare path (first NOT_READY per row); failure falls
    back to the margin so the row still retries once. The cap keeps
    one row from stalling the campaign; callers that can prove an
    oracle-bounded longer wait (in-session category cooldowns) pass
    cap_s=inf explicitly.
    """
    try:
        row = smoke.sql_scalar(
            "mangos",
            "SELECT COALESCE(MAX(recoveryTime),0),COALESCE(MAX(categoryRecoveryTime),0) "
            f"FROM spell_template WHERE entry={int(spell_id)}",
        )
        parts = str(row).split("\t")
        return cooldown_wait_s(parts[0] if len(parts) > 0 else 0, parts[1] if len(parts) > 1 else 0, cap_s=cap_s)
    except Exception:
        # Bench oracle unreachable: still retry once after the
        # margin rather than failing the row on a read error.
        return cooldown_wait_s(0, 0, cap_s=cap_s)


def live_trigger_holder(spell_id: int) -> int | None:
    """Holder spell id for a trigger-first row's main-target aura
    (live bench oracle): the row's effectTriggerSpell1 when that
    triggered spell's first effect applies an aura. None when the
    trigger is not an aura applier or the oracle is unreadable, so
    the caller keeps its own (vacuous) expectation and fails
    loudly instead of checking a wrong holder."""
    try:
        trig = smoke.sql_scalar(
            "mangos",
            "SELECT effectTriggerSpell1 FROM spell_template "
            f"WHERE entry={int(spell_id)} AND build=("
            f"SELECT MAX(build) FROM spell_template s WHERE s.entry={int(spell_id)} AND s.build<=5875)",
        )
        trigger_id = int(str(trig).split("\t")[0])
        if not trigger_id:
            return None
        eff = smoke.sql_scalar(
            "mangos",
            "SELECT effect1 FROM spell_template "
            f"WHERE entry={trigger_id} AND build=("
            f"SELECT MAX(build) FROM spell_template s WHERE s.entry={trigger_id} AND s.build<=5875)",
        )
        if int(str(eff).split("\t")[0]) != 6:
            return None
        return trigger_id
    except Exception:
        return None


def live_heal_band(spell_id: int, caster_level: int = 60) -> tuple[int, int] | None:
    """Fork-exact direct-heal amount band (live bench oracle).

    Mirrors `calculate_effect_value_bp` over the row spell's
    first-effect coefficients at the best build <= 5875: level
    clamp, spell-level shift, dice range. Widened by 1 each side
    for dither. Raw formula only: no healing-bonus tail (the
    fixtures cast naked) and no crit multiplier (unimplemented
    in the engine); a geared or critting caster reads outside
    the band and fails loudly instead of passing silently.
    None when the oracle is unreadable.
    """
    import math as _math

    try:
        row = smoke.sql_scalar(
            "mangos",
            "SELECT effectBasePoints1,effectDieSides1,effectBaseDice1,"
            "effectDicePerLevel1,effectRealPointsPerLevel1,spellLevel,baseLevel,maxLevel "
            f"FROM spell_template WHERE entry={int(spell_id)} AND build=("
            f"SELECT MAX(build) FROM spell_template s WHERE s.entry={int(spell_id)} AND s.build<=5875)",
        )
        parts = str(row).split("\t")
        bp, sides, dice = int(parts[0]), int(parts[1]), int(parts[2])
        dpl, rppl = float(parts[3]), float(parts[4])
        slevel, blevel, mlevel = int(parts[5]), int(parts[6]), int(parts[7])
    except Exception:
        return None
    level = caster_level
    if mlevel > 0 and level > mlevel:
        level = mlevel
    elif level < blevel:
        level = blevel
    level -= slevel
    base = bp + level * rppl
    random_points = int(sides + level * dpl)
    if random_points in (0, 1):
        lo = hi = base + dice
    else:
        lo = base + min(random_points, dice)
        hi = base + max(random_points, dice)
    return (_math.floor(lo) - 1, _math.ceil(hi) + 1)


def not_ready_retry(
    driver: ScriptedDriver,
    guid: int,
    spell_id: int,
    cooldown_ms: object,
    ops: list[int],
) -> tuple[bool, int | None, list[int], str]:
    """Second-leg second chance after NOT_READY (shared by blessing
    legs): the full DBC cooldown plus margin is waited out, so the
    recast proves both the cooldown (NOT_READY observed first) and
    its expiry (the Freedom pattern, extended to long rows like
    Innervate 29166). The wait comes from the live oracle
    (recovery + category): category-only cooldowns like
    Blessing of Protection (recovery 0, category 300 s) read
    30 s from the matrix recovery alone and stayed NOT_READY.
    Capped at 600 s; longer cooldowns stay FAIL.
    Returns (go, reason, ops, evidence_note)."""
    wait_s = live_cooldown_wait_s(spell_id, cap_s=600.0)
    driver.poll(wait_s)
    go, reason, chunk, _waited = cast_unit_retry_power(driver, spell_id, guid, timeout=10.0)
    return go, reason, ops + chunk, f" cooldown_wait={wait_s:.1f}s"


def bag_reagent_count(player_guid: int, entry: int) -> int | None:
    """Read-only reagent count in the fixture's bags (no writes)."""
    try:
        return int(
            smoke.sql_scalar(
                "characters",
                "SELECT COALESCE(SUM(ii.count),0) FROM character_inventory ci "
                "JOIN item_instance ii ON ii.guid=ci.item_guid "
                f"WHERE ci.guid={player_guid} AND ii.item_id={entry}",
            )
        )
    except (TypeError, ValueError):
        return None


def equipped_item_class(
    player_guid: int,
    slot: int = 16,
) -> tuple[int, int] | None:
    """(class, subclass) of the equipped item in an equipment slot
    (default offhand/16, where a shield sits), or None."""
    try:
        row = smoke.sql_scalar(
            "mangos",
            "SELECT CONCAT_WS(',',class,subclass) FROM item_template "
            f"WHERE entry=(SELECT ci.item_id FROM "
            f"characters.character_inventory ci WHERE ci.guid={player_guid} "
            f"AND ci.bag=0 AND ci.slot={slot})",
        )
    except (TypeError, ValueError):
        return None
    if not row or "," not in str(row):
        return None
    first, second = str(row).split(",", 1)
    return int(first), int(second)


def dummy_world_state(driver: ScriptedDriver, guid: int) -> tuple[int | None, int | None, bool]:
    """(health, max_health, has_aura) for a dummy guid, for the
    clean-reuse gate."""
    obj = driver.world.get(guid)
    if obj is None:
        return (None, None, False)
    max_health = obj.values.get(UNIT_FIELD_MAXHEALTH)
    has_aura = any(obj.values.get(field) for field in range(AURA_FIELD_FIRST, AURA_FIELD_LAST + 1))
    return (driver.target_health(guid), max_health, has_aura)


def pick_dummy(
    driver: ScriptedDriver,
    used: set[int],
) -> tuple[int | None, bool]:
    """Live dummy per spell: round-robin 705/707 counters, preferring
    an unused one. Falls back to reusing a used dummy only when it is
    observably clean (alive, full health, aura-free), since DoTs, CC,
    and prior damage all leave a health deficit or an aura holder.
    Returns (guid, reused); (None, False) when no counter is live."""
    order = []
    for entry in (705, 707):
        counters = WOLF_COUNTERS if entry == 705 else TROGG_COUNTERS
        for counter in counters:
            order.append(raw_guid(entry, counter))
    for guid in order:
        if guid in used:
            continue
        if driver.target_health(guid) not in (None, 0):
            return (guid, False)
    for guid in order:
        if guid not in used:
            continue
        health, max_health, has_aura = dummy_world_state(driver, guid)
        if dummy_reusable(health, max_health, has_aura):
            return (guid, True)
    return (None, False)


def wait_for_dummy(driver: ScriptedDriver, used: set[int], timeout_s: float = 30.0) -> tuple[int | None, bool, float]:
    """Pool pick with a respawn wait. Bench dummies respawn in 2-3 s,
    but a DoT-heavy class (Warlock: 49 hostile + 9 ground rows) can
    hold all 10 counters dead/damaged/aurad at once; failing the row
    instantly then is pool contention, not an engine finding. Poll
    until a counter is live/clean or the budget runs out."""
    waited = 0.0
    while True:
        dummy, reused = pick_dummy(driver, used)
        if dummy is not None or waited >= timeout_s:
            return (dummy, reused, waited)
        driver.poll(2.0)
        waited += 2.0


def self_alive(driver: ScriptedDriver, guid: int) -> bool:
    obj = driver.world.get(guid)
    if obj is None:
        return False
    health = obj.values.get(UNIT_FIELD_HEALTH)
    return bool(health)


def self_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    applied_auras: list[int],
    pre_cast_setup=None,
) -> dict:
    """Solo self cast with NO_POWER pacing (spec §5). An optional
    pre_cast_setup(driver) runs first (Warrior Bloodrage rage
    top-up); its note joins the pre evidence like the hostile
    leg's setup."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    pre = snapshot(driver, guid)
    if pre_cast_setup is not None:
        pre += pre_cast_setup(driver)
    mark = event_mark(driver)
    go, reason, ops, waited = cast_self_retry_power(driver, spell_id)
    if not go and reason == SPELL_FAILED_NOT_READY:
        # Shared category cooldown (Warrior Shield Wall 871 arms
        # category 132/1800 s, so Recklessness 1719 reads NOT_READY
        # even on a fresh character): wait out the live fork
        # cooldown in capped chunks and retry, like the hostile
        # leg's single wait but extended to the oracle total. The
        # total is oracle-bounded (max recovery/category + margin),
        # never open-ended; longer category cooldowns stay FAIL.
        total_s = live_cooldown_wait_s(spell_id, cap_s=float("inf"))
        waited_s = 0.0
        chunks = 0
        while not go and reason == SPELL_FAILED_NOT_READY and waited_s < total_s and chunks < 6:
            chunk = min(600.0, total_s - waited_s)
            driver.poll(chunk)
            waited_s += chunk
            chunks += 1
            go, reason, chunk_ops, _waited = cast_self_retry_power(driver, spell_id)
            ops += chunk_ops
        if not go and reason == SPELL_FAILED_NOT_READY:
            return make_verdict(
                row,
                "FAIL",
                phase="prepare",
                reason=f"CAST_RESULT({reason})",
                evidence=f"cooldown-blocked after +{waited_s:.0f}s oracle waits",
                pre=pre,
                post=snapshot(driver, guid),
                packets=ops,
                target_guid=guid,
            )
    if not go and reason == SPELL_FAILED_NO_POWER:
        return make_verdict(
            row,
            "FAIL",
            phase="prepare",
            reason=f"CAST_RESULT({reason})",
            evidence=f"power-starved after {waited} 9s waits",
            pre=pre,
            post=snapshot(driver, guid),
            packets=ops,
            target_guid=guid,
        )
    # Latch the aura in slices from the cast return: a
    # 1-charge proc aura (Touch of Weakness) is consumed by
    # the first dummy swing inside one blind poll, while a
    # slow holder can also arrive seconds after SPELL_GO
    # (Shield Block read absent at +2 s twice).
    aura_id = int(expected.get("aura", spell_id))
    caster_aura = False
    if go:
        # Check before any poll first (the aura update can
        # ride the GO batch), then in 50 ms slices: a
        # 1-charge proc aura can be consumed by the first
        # dummy swing inside one 250 ms slice.
        caster_aura = aura_present(driver, guid, aura_id)
        for _ in range(40):
            if caster_aura:
                break
            driver.poll(0.05)
            if aura_present(driver, guid, aura_id):
                caster_aura = True
                break
        if not caster_aura:
            caster_aura = wait_for_aura(driver, guid, aura_id)
    window = events_since(driver, mark)
    if caster_aura and aura_id not in applied_auras:
        applied_auras.append(aura_id)
    post = snapshot(driver, guid)
    packets = ops + [op for op, _ in window]
    note = f" after {waited} mana wait(s)" if waited else ""
    verdict = single_outcome_verdict(
        row,
        {
            "go": go,
            "reason": reason,
            "caster_aura": caster_aura,
            "target_guid": guid,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": pre,
            "post": post,
            "packets": packets,
        },
    )
    if verdict["verdict"] == "PASS":
        verdict["evidence"] += note
    return verdict


def cast_form_entry(driver: ScriptedDriver, form_spell: int):
    """Form-entry cast with one cooldown wait: forms with a
    category cooldown (Stealth: 10 s) answer NOT_READY on
    re-entry inside the window a previous row opened, so wait
    the live oracle cooldown out and retry once (like the
    hostile/self legs' NOT_READY arms)."""
    go, reason, ops, waited = cast_self_retry_power(driver, form_spell)
    if not go and reason == SPELL_FAILED_NOT_READY:
        driver.poll(live_cooldown_wait_s(form_spell))
        go2, reason2, ops2, waited2 = cast_self_retry_power(driver, form_spell)
        return go2, reason2, ops + ops2, waited + waited2
    return go, reason, ops, waited


def form_leg(row: dict, driver: ScriptedDriver, guid: int) -> dict:
    """Shapeshift entry leg for Druid `form`-bucket rows.

    Pre-clears any lingering form aura, casts the form from caster
    form, verifies its aura on self (the pilot Bear/Cat pattern),
    then cancels it and verifies it is gone so later caster-form rows
    gate cleanly. A stuck form is an apply-phase FAIL, never PASS:
    the row's observable outcome includes the clean exit.
    """
    spell_id = row["spell_id"]
    pre = snapshot(driver, guid)
    cleared = cancel_auras(driver, guid, list(ALL_FORM_SPELLS))
    mark = event_mark(driver)
    go, reason, ops, waited = cast_form_entry(driver, spell_id)
    aura_ok = go and wait_for_aura(driver, guid, spell_id)
    post = snapshot(driver, guid)
    packets = ops + [op for op, _ in events_since(driver, mark)]
    if not go:
        return make_verdict(
            row,
            "FAIL",
            phase="prepare",
            reason=(f"CAST_RESULT({reason})" if reason is not None else None),
            evidence=f"form {spell_id} did not reach SPELL_GO (reason={reason}); pre-cleared {cleared}",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=guid,
        )
    if not aura_ok:
        return make_verdict(
            row,
            "FAIL",
            phase="apply",
            evidence=f"bound SPELL_GO for form {spell_id} without the form aura on self; pre-cleared {cleared}",
            pre=pre,
            post=post,
            packets=packets,
            target_guid=guid,
        )
    note = f" after {waited} mana wait(s)" if waited else ""
    gone = cancel_auras(driver, guid, [spell_id])
    if gone.get(spell_id):
        verdict = make_verdict(
            row,
            "PASS",
            evidence=f"bound SPELL_GO + form aura {spell_id} on self, cancelled after (pre-cleared {cleared}){note}",
            pre=pre,
            post=snapshot(driver, guid),
            packets=packets,
            target_guid=guid,
        )
        return verdict
    return make_verdict(
        row,
        "FAIL",
        phase="apply",
        evidence=f"form aura {spell_id} present but stuck after cancel (pre-cleared {cleared}){note}",
        pre=pre,
        post=snapshot(driver, guid),
        packets=packets,
        target_guid=guid,
    )


def low_health_setup(driver: ScriptedDriver, dummy: int) -> str:
    """pre_cast_setup for HEALTHLESS_20 rows (Execute): enter
    Battle Stance, top up Rend's rage cost, and Rend (6546) the
    dummy repeatedly as the aura expires until it falls strictly
    below 20% health so the fork TargetAuraState gate opens.
    Rend is stance-gated, so the predamage cast is only
    fork-legal after the stance verifies. The loop breaks on the
    first alive sub-20% read so the next Rend tick cannot spend
    the scenario: the dummy staying alive is the scenario's job
    (one Rend's total is well under a fresh counter's pool)."""
    stance_ok, stance_reason, _stance_ops = ensure_battle_stance(driver, driver.own_guid)
    if not stance_ok:
        return f"; predamage stance FAILED reason={stance_reason}"
    rage_note = ""
    go, reason = False, None
    casts = 0
    waited = 0.0
    hp = max_hp = None
    while waited < 180.0:
        hp, max_hp, _aura = dummy_world_state(driver, dummy)
        if hp is not None and max_hp and 0 < hp * 5 < max_hp:
            break
        if hp == 0:
            break
        if not aura_present(driver, dummy, 6546):
            rage_note = warrior_rage_setup(100)(driver, dummy)
            driver.approach_guid(dummy, 2.0)
            go, reason, _ops = cast_unit_faced(driver, 6546, dummy, timeout=10.0)
            casts += 1
            if not go:
                break
        driver.poll(1.0)
        waited += 1.0
    return rage_note + f"; predamage rend casts={casts} go={go} reason={reason} hp {hp}/{max_hp} after {waited:.0f}s"


def combo_builder_setup(builder_id: int):
    """pre_cast_setup factory: land one combo builder (lowest rank,
    e.g. Claw 1082) on the dummy so a finisher row passes the fork
    NO_COMBO_POINTS cast gate with a point on that target."""

    def setup(driver: ScriptedDriver, dummy: int) -> str:
        # Builders are melee abilities: close first or the cast
        # reads OUT_OF_RANGE from a ranged standoff and the
        # finisher then reads NO_COMBO_POINTS.
        driver.approach_guid(dummy, 2.0)
        go, reason, _ops = cast_unit_faced(driver, builder_id, dummy)
        driver.poll(1.6)
        return f"; combo builder {builder_id} go={go} reason={reason}"

    return setup


def immolate_setup(immolate_id: int):
    """pre_cast_setup factory: land Immolate on the dummy so a
    Conflagrate row passes the fork TARGET_AURASTATE gate (the
    fork consumes the Immolate; the setup runs per row, and a
    failed setup rides the row's honest 103)."""

    def setup(driver: ScriptedDriver, dummy: int) -> str:
        if not aura_present(driver, dummy, immolate_id):
            go, reason, _ops = cast_unit_faced(driver, immolate_id, dummy)
            # Outlast the setup's own cast time plus the GCD it
            # triggers: the Conflagrate that follows would otherwise
            # eat NOT_READY from the still-running global cooldown.
            driver.poll(3.5)
            if not (go and aura_present(driver, dummy, immolate_id)):
                return f"; immolate {immolate_id} setup FAILED go={go} reason={reason}"
        return f"; immolate {immolate_id} setup verified"

    return setup


#: Bloodrage setup spell: the fork-legal rage source for Warrior
#: rows. Its Effect-30 energize grants rage through the generic
#: engine lane (the engine's Bloodrage arm itself only flags
#: in-combat); the row itself stays marker-only and never casts as
#: a verdict row.
WARRIOR_RAGE_SPELL = 2687
#: Combat-maintenance taps for Bloodrage cooldown waits. The
#: tap must deal damage: a damage-free shout aura lands but
#: never aggros the dummy (live probe: caster never hit, rage
#: decayed to zero through the wait). Rend serves Battle and
#: Defensive, Hamstring serves Berserker (and Battle); a
#: stanceless caster enters Battle first. Execute itself is
#: health-gated and can no longer serve as the tap.
WARRIOR_TAP_REND = 6546
WARRIOR_TAP_HAMSTRING = 1715
WARRIOR_TAP_COST = 100
# Rage lives in the second power slot (field 24), like the Bear
# arm's POWER2=24: UNIT_FIELD_POWER1 (23) is the mana slot, which a
# compressed update omits for a rage user (reads None). Reading 23
# made every setup see 0 rage and burn Bloodrage blindly while the
# real pool was invisible.
WARRIOR_RAGE_FIELD = 24


def warrior_rage_cycles(pool: int, cost: int, grant: int = 100) -> int:
    """Pure Bloodrage-cycle math: casts needed to reach `cost` from
    `pool` when each cast grants `grant` power (Bloodrage 2687 =
    +100 power/10 rage at DBC basePoints 99+1). Zero when the pool
    already covers the cost."""
    if pool >= cost:
        return 0
    short = cost - pool
    return (short + grant - 1) // grant


def warrior_rage_setup(cost: int):
    """pre_cast_setup factory: top rage up via Bloodrage (2687) so a
    rage-cost Warrior row passes the fork NO_POWER gate.

    Check-first on the real rage field (WARRIOR_RAGE_FIELD): when
    the pool already covers the cost (combat income from the
    engaged dummy plus earlier Bloodrages) no Bloodrage is burned —
    its 60 s cooldown is then still up for a later starving row.
    Otherwise loop: cast Bloodrage, and when it answers NOT_READY
    (still on its 60 s cooldown) wait out the live cooldown before
    the next cycle, until the pool covers the cost (buffered by 50
    power) or 8 cycles are spent. A pool that never fills rides
    the row's honest NO_POWER, never a pass.

    Combat maintenance: rage drains 2/tick out of combat
    (fork-exact), and a 60 s cooldown wait with nothing
    referencing the warrior wipes the pool faster than Bloodrage
    refills it. So when Bloodrage answers NOT_READY (its 60 s
    cooldown is live), the setup first taps a dummy with a
    damaging stance-legal ability (Rend in Battle/Defensive,
    Hamstring in Berserker, Battle entry first when
    stanceless): small damage — it cannot quickly kill the
    counter but keeps it aggroed and swinging, holding combat
    (and its damage-taken income) up through the wait. The
    tap drains the pool, but
    the coming wait would decay ~650 power off an unreferenced
    pool anyway, so firing it is free or better whenever the
    wait is certain — which the NOT_READY itself proves.
    Dummy-less legs borrow a spare counter the same way (serial
    rows never contend for it); a missing spare skips the tap
    and the row rides ambient combat as before."""

    def setup(driver: ScriptedDriver, dummy: int | None = None) -> str:
        guid = driver.own_guid
        cycles = 0
        casts = 0
        waits = 0.0
        engaged = False
        taps = 0
        # Let any incoming GCD expire before the first cast: a
        # GCD-masked Bloodrage would read NOT_READY and burn a
        # 65 s cooldown wait needlessly.
        driver.poll(2.0)
        waits += 2.0
        while cycles < 8:
            driver.poll(1.0)
            waits += 1.0
            obj = driver.world.get(guid) if guid is not None else None
            rage = (obj.values.get(WARRIOR_RAGE_FIELD) or 0) if obj else 0
            if rage >= cost + 50:
                return (
                    f"; rage {rage}>={cost} (no Bloodrage)"
                    if cycles == 0
                    else (
                        f"; rage {rage}/{cost} after {casts} bloodrage(s) +{waits:.0f}s waits"
                        + ("; tap-engaged" if engaged else "")
                    )
                )
            go, reason, _o, _w = cast_self_retry_power(driver, WARRIOR_RAGE_SPELL)
            if go:
                casts += 1
                driver.poll(2.0)
                waits += 2.0
            elif reason == SPELL_FAILED_NOT_READY:
                if not engaged and taps < 4 and rage >= WARRIOR_TAP_COST:
                    tap_target = dummy
                    if tap_target is None:
                        tap_target = live_dummy(driver, 705, WOLF_COUNTERS) or live_dummy(driver, 707, TROGG_COUNTERS)
                    if tap_target is not None:
                        # Close to melee first: the tap is a
                        # melee ability, so a cast from a ranged
                        # standoff would only read OUT_OF_RANGE
                        # (the pool is about to decay through the
                        # wait anyway).
                        t0 = time.monotonic()
                        driver.approach_guid(tap_target, 2.0)
                        waits += time.monotonic() - t0
                        taps += 1
                        # Stance-legal damaging tap: Rend in
                        # Battle/Defensive, Hamstring in
                        # Berserker, Battle entry first when
                        # stanceless (the stanceless rows this
                        # setup serves cast legally from Battle:
                        # their stance mask is 0). The tap aura
                        # on the dummy, not the GO alone, proves
                        # the engagement.
                        if aura_present(driver, guid, 2457) or aura_present(driver, guid, 71):
                            tap_spell = WARRIOR_TAP_REND
                        elif aura_present(driver, guid, 2458):
                            tap_spell = WARRIOR_TAP_HAMSTRING
                        else:
                            stance_ok, _stance_reason, _stance_ops = ensure_battle_stance(driver, guid)
                            tap_spell = WARRIOR_TAP_REND if stance_ok else None
                            if stance_ok:
                                # Outlast the stance-entry GCD
                                # before the tap cast.
                                driver.poll(1.6)
                                waits += 1.6
                        if tap_spell is not None:
                            tap_go, _tap_reason, _t = cast_unit_faced(driver, tap_spell, tap_target, timeout=10.0)
                            if tap_go and wait_for_aura(driver, tap_target, tap_spell):
                                engaged = True
                        driver.poll(2.0)
                        waits += 2.0
                wait_s = live_cooldown_wait_s(WARRIOR_RAGE_SPELL)
                driver.poll(wait_s)
                waits += wait_s
            else:
                driver.poll(2.0)
                waits += 2.0
            cycles += 1
        obj = driver.world.get(guid) if guid is not None else None
        rage = (obj.values.get(WARRIOR_RAGE_FIELD) or 0) if obj else 0
        return f"; bloodrage short: rage {rage}/{cost} after {casts} cast(s) +{waits:.0f}s waits"

    return setup


def warrior_rage_setup_for_row(class_name: str, row: dict):
    """Bloodrage setup for one rage-cost Warrior row, else None.

    Pure routing over the matrix row (the setup itself is live):
    Bloodrage itself, non-rage rows, and other classes never take
    the setup. Shared by the run-class legs; the form_gated leg
    applies the same rule via the entered stance."""
    if class_name != "Warrior":
        return None
    if int(row.get("spell_id") or 0) == WARRIOR_RAGE_SPELL:
        return None
    if int(row.get("power_type") or 0) != 1:
        return None
    cost = int(row.get("mana_cost") or 0)
    if cost <= 0:
        return None
    return warrior_rage_setup(cost)


def form_gated_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    used_dummies: set[int],
    applied_auras: list[int],
    combo_builder: int | None = None,
) -> dict:
    """Stance-gated ability leg: enter the required shapeshift form,
    verify its aura, cast the row through its natural self/hostile
    leg, then exit the form so caster-form rows gate cleanly. A form
    that will not enter is a prepare FAIL; a form stuck after the row
    downgrades a PASS to an apply FAIL (the form_leg contract)."""
    spell_id = row["spell_id"]
    form_spell = required_form_spell(row)
    pre = snapshot(driver, guid)
    cleared = cancel_auras(driver, guid, list(ALL_FORM_SPELLS))
    go, reason, ops, _waited = cast_form_entry(driver, form_spell)
    if not (go and wait_for_aura(driver, guid, form_spell)):
        # One retry from a verified-bare caster form: a GO without
        # the aura is a slow apply (or a stale exit cancel) far more
        # often than a dead gate, but a second miss fails loudly.
        cleared2 = cancel_auras(driver, guid, list(ALL_FORM_SPELLS))
        bare = not any(aura_present(driver, guid, s) for s in ALL_FORM_SPELLS)
        go2, reason2, ops2, _waited2 = cast_self_retry_power(driver, form_spell)
        ops += ops2
        if not (go2 and wait_for_aura(driver, guid, form_spell)):
            return make_verdict(
                row,
                "FAIL",
                phase="prepare",
                reason=(f"CAST_RESULT({reason2})" if reason2 is not None else None),
                evidence=f"form entry {form_spell} did not verify "
                f"(go={go}/{go2} reason={reason}/{reason2} cleared={cleared}/{cleared2} bare={bare}); "
                f"gated spell {spell_id} not cast under invalid conditions (not a pass)",
                pre=pre,
                post=snapshot(driver, guid),
                packets=ops,
                target_guid=guid,
            )
        go, reason = go2, reason2
    # Bear abilities spend rage, which does not regenerate out of
    # combat. The fork-legal source is Enrage (5229, itself a matrix
    # row): periodic energize ~20/s (2 rage) for 10 s. It must be cast
    # after the dummy is approached (aura window is short) and the
    # dummy engaged with a free Growl pull so damage-taken rage adds
    # up while the energize ticks; hostile_leg runs this setup right
    # before its cast loop.
    setup = None
    if (
        form_spell in (5487, 9634)
        and spell_id != 5229
        and int(row.get("power_type") or 0) == 1
        and int(row.get("mana_cost") or 0) > 0
    ):
        rage_cost = int(row.get("mana_cost") or 0)

        def setup(driver: ScriptedDriver, dummy: int) -> str:
            cast_unit_faced(driver, 6795, dummy)
            go, _r, _o, _w = cast_self_retry_power(driver, 5229)
            # Wait on the observed rage field (POWER2=24), not a
            # fixed sleep: Swipe needs the full Enrage window.
            waited = 0.0
            rage = 0
            while waited < 14.0:
                driver.poll(0.5)
                waited += 0.5
                obj = driver.world.get(guid)
                rage = (obj.values.get(24) or 0) if obj else 0
                if rage >= rage_cost:
                    break
            return f"; growl pull + enrage go={go}, rage {rage}/{rage_cost} after {waited:.1f}s"

    if setup is None and form_spell in WARRIOR_STANCE_SPELLS:
        # Warrior stance rows spend rage, which does not regen out
        # of combat: top up via Bloodrage before the row's natural
        # leg (hostile or self), mirroring the Bear Enrage arm.
        setup = warrior_rage_setup_for_row(row.get("class", ""), row)
    override = None
    if setup is None and row.get("needs_combo_points") and combo_builder:
        setup = combo_builder_setup(combo_builder)
        override = finisher_dummy(used_dummies)
    if row.get("requires_low_health_target"):
        rage_first = setup

        def setup(driver: ScriptedDriver, dummy: int, _first=rage_first) -> str:
            note = _first(driver, dummy) if _first is not None else ""
            return note + low_health_setup(driver, dummy)

    targets = {int(t) for t in (row.get("target_a") or [])}
    if targets & {6, 22}:
        verdict = hostile_leg(
            row, driver, guid, used_dummies, applied_auras, pre_cast_setup=setup, dummy_override=override
        )
    else:
        verdict = self_leg(row, driver, guid, applied_auras, pre_cast_setup=setup)
    _gone = cancel_auras(driver, guid, [form_spell])
    # Exited means the form aura is gone, however it left: a
    # stealth opener breaks Stealth itself, so the cancel finds
    # no holder to report while the exit is still clean. A form
    # aura still present after the cancel is the stuck case.
    exited = not aura_present(driver, guid, form_spell)
    verdict["evidence"] += f"; form {form_spell} setup verified, exited={exited}"
    if verdict["verdict"] == "PASS" and not exited:
        verdict["verdict"] = "FAIL"
        verdict["phase"] = "apply"
        verdict["evidence"] += "; form stuck after cancel"
    return verdict


def approach_behind(driver: ScriptedDriver, dummy: int, standoff: float = 2.0) -> str:
    """Walk to the point behind the dummy (its facing reversed)
    and face it, for from-behind-only rows (Backstab/Garrote).
    The dummy's tracked orientation is read live; a missing
    position falls back to the plain approach and the cast
    then reads its honest NOT_BEHIND."""
    pos = driver.world.position(dummy)
    if pos is None:
        driver.approach_guid(dummy, standoff)
        return "; behind approach: dummy position unknown"
    behind_x = pos[0] - math.cos(pos[3]) * standoff
    behind_y = pos[1] - math.sin(pos[3]) * standoff
    driver.walk_to(behind_x, behind_y, pos[2])
    driver.face(pos[0], pos[1])
    driver.poll(0.5)
    return f"; behind approach to ({behind_x:.1f},{behind_y:.1f}) facing dummy o={pos[3]:.2f}"


def ack_teleports(driver: ScriptedDriver, timeout: float = 12.0) -> bool:
    """Consume server near-teleports like the stock client:
    adopt each `MSG_MOVE_TELEPORT_ACK` destination and reply
    with the client ack (death probe shape)."""
    acked = False
    deadline = time.monotonic() + timeout
    seen = 0
    while time.monotonic() < deadline:
        driver.poll(0.25)
        while seen < len(driver.events):
            opcode, body = driver.events[seen]
            seen += 1
            if opcode != MSG_MOVE_TELEPORT_ACK:
                continue
            raw, off = smoke.parse_packed_guid(body, 0)
            counter = struct.unpack_from("<I", body, off)[0]
            _flags, _time, x, y, z, o = struct.unpack_from("<IIffff", body, off + 4)
            driver.position = (x, y, z, o)
            ack = struct.pack("<QII", raw, counter, int(time.time() * 1000) & 0xFFFF_FFFF)
            driver.client.send_packet(MSG_MOVE_TELEPORT_ACK, ack)
            acked = True
        if acked:
            driver.poll(0.5)
            return True
    return acked


def corpse_guid_for(driver: ScriptedDriver, owner_guid: int) -> int | None:
    """Tracked corpse guid owned by `owner_guid`, else the
    SQL corpse row (death probe resolution order)."""
    for obj in driver.world.objects.values():
        if obj.type_id != TYPEID_CORPSE:
            continue
        owner = obj.values.get(CORPSE_FIELD_OWNER, 0) | (obj.values.get(CORPSE_FIELD_OWNER + 1, 0) << 32)
        if owner == owner_guid:
            return obj.guid
    low = smoke.sql_scalar("characters", f"SELECT guid FROM corpse WHERE player_guid={owner_guid} LIMIT 1")
    if low and low != "NULL":
        return (0xF101 << 48) | int(low)
    return None


def hostile_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    used_dummies: set[int],
    applied_auras: list[int],
    pre_cast_setup=None,
    dummy_override: tuple | None = None,
) -> dict:
    """Hostile cast on a fresh live dummy (never reused), with seal
    setup-verify for Judgement rows and a 5-recast retry cap.
    `dummy_override` pins (guid, fallback xyz) instead of the
    705/707 rotation (finisher bears: the builder must land on the
    finisher's own target and the wolf dies to it)."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    kind = expected.get("kind")
    name = row.get("name") or ""
    pre = snapshot(driver, guid)
    setup_note = ""
    if name.startswith("Judgement") and kind in ("aura_on_target", "damage_log"):
        if not aura_present(driver, guid, 20154):
            mark0 = event_mark(driver)
            setup_go, setup_reason, _sops, _waited = cast_self_retry_power(driver, 20154)
            driver.poll(1.5)
            if not (setup_go and aura_present(driver, guid, 20154)):
                return make_verdict(
                    row,
                    "FAIL",
                    phase="prepare",
                    reason=(f"CAST_RESULT({setup_reason})" if not setup_go and setup_reason is not None else None),
                    evidence="seal setup 20154 did not verify; judgement "
                    "not cast under invalid conditions (not a pass)",
                    pre=pre,
                    post=snapshot(driver, guid),
                    packets=[op for op, _ in events_since(driver, mark0)],
                    target_guid=None,
                )
            applied_auras.append(20154)
        setup_note = "; seal 20154 setup verified"
    fallback = None
    if dummy_override is not None:
        dummy, fallback = dummy_override
        reused = dummy in used_dummies
        dummy_waited = 0.0
    else:
        dummy, reused, dummy_waited = wait_for_dummy(driver, used_dummies)
    if dummy is None:
        return make_verdict(
            row, "BLOCKED", reason="dummy-unavailable", evidence="no live 705/707 counter left in WorldState", pre=pre
        )
    used_dummies.add(dummy)
    reuse_note = "; reused observably-clean dummy (alive, full health, aura-free)" if reused else ""
    if dummy_waited:
        reuse_note += f"; pool wait +{dummy_waited:.0f}s"
    # Self-range rows (range_index 1, areaselect-at-caster 22) only
    # reach a dummy standing inside the effect radius: at the ranged
    # standoff the cast lands on nobody (Demoralizing Roar at 13 yd
    # vs its 10 yd radius reads as a landed-but-empty GO).
    standoff = 2.0 if (row.get("range_index") in (1, 2) or 22 in (row.get("target_a") or [])) else 13.0
    driver.approach_guid(dummy, standoff, fallback=fallback, attempts=6 if fallback else 3)
    if driver.target_health(dummy) in (None, 0):
        return make_verdict(
            row,
            "BLOCKED",
            reason="dummy-unavailable",
            evidence=f"pinned dummy guid={dummy} not live after approach (dead or never visible)",
            pre=pre,
            target_guid=dummy,
        )
    pre_hp = driver.target_health(dummy)
    pre = f"dummy guid={dummy} health={pre_hp}; {pre}{setup_note}"
    if pre_cast_setup is not None:
        setup_note += pre_cast_setup(driver, dummy)
        pre += setup_note
    if row.get("requires_behind_target"):
        # From-behind-only rows: take the behind position after
        # any setup (which may have moved the caster), then cast.
        pre += approach_behind(driver, dummy)
    attempts = 0
    go, reason, ops = False, None, []
    post_hp, target_aura, logs = pre_hp, False, []
    min_hp = pre_hp
    aura_id = int(expected.get("aura", spell_id))
    if aura_id == spell_id and (row.get("effect_ids") or [None])[0] == 64:
        # Vacuous derivation fallback (no direct aura on the row)
        # with a trigger first effect: the main-target observable
        # is the triggered spell's holder (Intimidating Shout
        # 5246's 20511 stun; the area fear/surround explicitly
        # exempts the main target via the fork script). Resolved
        # live so a non-aura trigger fails loudly below instead.
        aura_id = live_trigger_holder(spell_id) or aura_id
    range_step = standoff
    landed_go = False
    cooldown_waited_s = 0.0
    cooldown_note = ""
    range_misses = 0
    while attempts < 7:
        attempts += 1
        mark = event_mark(driver)
        go, reason, cast_ops, waited = cast_unit_retry_power(driver, spell_id, dummy, timeout=15.0)
        ops += cast_ops
        landed_go = landed_go or go
        if not go and reason == SPELL_FAILED_NOT_READY and not landed_go and not cooldown_waited_s:
            # Shared category cooldown (multi-rank chains: Wyvern
            # Sting 24132 arms category 1111/120 s, so 24133 reads
            # NOT_READY): the matrix carries only recoveryTime, so
            # resolve the full fork cooldown live from the bench
            # oracle and wait it out once within the same cap,
            # like the blessing legs (a landed GO never waits:
            # the cooldown_erased arm below retains it).
            cooldown_waited_s = live_cooldown_wait_s(spell_id)
            driver.poll(cooldown_waited_s)
            cooldown_note = f"; cooldown wait +{cooldown_waited_s:.0f}s"
            continue
        if not go and reason == SPELL_FAILED_NOT_BEHIND:
            # The dummy turned between the behind approach and
            # the cast: re-take the behind position and retry
            # within the same cap.
            pre += approach_behind(driver, dummy)
            continue
        if not go and reason == SPELL_FAILED_OUT_OF_RANGE:
            # Roaming dummy drifted out of range between approach and
            # cast (pilot Growl pattern): step closer and retry within
            # the same cap instead of failing the row. Past two
            # consecutive misses the approach itself is not closing
            # (unpathable counter): take a different counter like
            # the dead-dummy arm instead of casting at geometry
            # that never resolves.
            range_misses += 1
            if range_misses >= 3:
                driver.poll(1.6)
                fresh, fresh_reused = pick_dummy(driver, used_dummies)
                if fresh is None:
                    return make_verdict(
                        row,
                        "BLOCKED",
                        reason="dummy-unavailable",
                        evidence="out of range of picked dummy and no live 705/707 counter remains",
                        pre=pre,
                        post=snapshot(driver, guid),
                        packets=ops,
                        target_guid=dummy,
                    )
                dummy, reused = fresh, fresh_reused
                used_dummies.add(dummy)
                reuse_note = "; re-picked after out-of-range geometry" + (
                    "; reused observably-clean dummy (alive, full health, aura-free)" if reused else ""
                )
                range_step = standoff
                range_misses = 0
                driver.approach_guid(dummy, range_step)
                driver.poll(1.0)
                pre_hp = driver.target_health(dummy)
                pre = f"dummy guid={dummy} health={pre_hp}; {pre}"
                continue
            range_step = max(2.0, range_step / 2.0)
            driver.approach_guid(dummy, range_step)
            driver.poll(1.0)
            continue
        if not go and reason == SPELL_FAILED_TARGETS_DEAD:
            # The picked dummy died under us (stale WorldState or a
            # neighbor's AoE, e.g. Hurricane's radius): take a
            # different counter within the same cap. The dead one
            # stays in used_dummies, never reused.
            driver.poll(1.6)
            fresh, fresh_reused = pick_dummy(driver, used_dummies)
            if fresh is None:
                return make_verdict(
                    row,
                    "BLOCKED",
                    reason="dummy-unavailable",
                    evidence="picked dummy died and no live 705/707 counter remains",
                    pre=pre,
                    post=snapshot(driver, guid),
                    packets=ops,
                    target_guid=dummy,
                )
            dummy, reused = fresh, fresh_reused
            used_dummies.add(dummy)
            reuse_note = "; re-picked after a dead dummy" + (
                "; reused observably-clean dummy (alive, full health, aura-free)" if reused else ""
            )
            range_step = standoff
            driver.approach_guid(dummy, range_step)
            driver.poll(1.0)
            pre_hp = driver.target_health(dummy)
            pre = f"dummy guid={dummy} health={pre_hp}; {pre}"
            continue
        # Poll in slices and latch the aura: short auras (Growl's 3 s
        # taunt, Bash's stun) expire inside one long poll and would
        # read as never applied, then the recast dies on cooldown.
        # Track the low-water hp too: fast regen can erase a small
        # hit before the post read, and a kill respawns the dummy
        # (full health) before the post read, so the single final
        # read alone cannot tell landed damage from a clean miss.
        # Aura rows hold the window open past the hp gate: an 8 s
        # stun applied 4+ s late under bench load still verifies.
        observe_s = 6.0 if float(row.get("speed") or 0) > 0 else 4.0
        if kind in ("aura_on_target", "aura_on_caster"):
            observe_s = max(observe_s, 8.0)
        target_aura = False
        caster_aura = False
        sliced = 0.0
        min_hp = pre_hp
        while sliced < observe_s:
            driver.poll(0.5)
            sliced += 0.5
            probe_hp = driver.target_health(dummy)
            if probe_hp is not None and (min_hp is None or probe_hp < min_hp):
                min_hp = probe_hp
            if aura_present(driver, dummy, aura_id):
                target_aura = True
                if kind == "aura_on_target":
                    break
            if aura_present(driver, guid, aura_id):
                # Caster-side aura on a hostile cast (Slice and
                # Dice: combo dummy effect on the unit target,
                # haste aura on the caster).
                caster_aura = True
                if kind == "aura_on_caster":
                    break
        window = events_since(driver, mark)
        ops += [op for op, _ in window]
        post_hp = driver.target_health(dummy)
        if min_hp is not None and (post_hp is None or min_hp < post_hp):
            post_hp = min_hp
        logs = matching_damage_logs(window, spell_id, dummy, guid)
        if not go:
            break
        if kind == "aura_on_target" and target_aura:
            break
        if kind == "aura_on_caster" and caster_aura:
            break
        if kind == "damage_log" and logs and post_hp is not None and pre_hp is not None and post_hp < pre_hp:
            break
        if kind not in ("aura_on_target", "aura_on_caster", "damage_log"):
            break
        driver.poll(1.6)
    # A landed cast whose outcome simply had not manifested inside
    # the short window must not be erased by a recast dying on the
    # spell's own cooldown (Curse of Doom: GO, 60 s tick pending,
    # recast reads NOT_READY). Restore the landed GO for the latch
    # and the verdict; the cooldown rejection is scenario noise.
    cooldown_erased = ""
    if landed_go and not go and reason == SPELL_FAILED_NOT_READY:
        go = True
        cooldown_erased = "; landed cast retained over cooldown-blocked recast"
    # Periodic-damage rows latch 590 ticks after the base outcome:
    # DoT ticks land seconds apart, so the short aura/damage window
    # above rarely holds the in-band ticks the band gate needs. The
    # horizon follows the aura's own duration (Curse of Doom ticks
    # once at 60 s), capped so one row cannot stall the campaign;
    # rows that produce their ticks exit as soon as the gate is met.
    tick_amounts: list[int] | None = None
    tick_note = ""
    if expected.get("tick_band") and go:
        tick_amounts = matching_periodic_ticks(events_since(driver, mark), spell_id, dummy, guid)
        latched_s = 0.0
        band = expected["tick_band"]
        horizon_s = 12.0
        duration_ms = expected.get("tick_duration_ms") or 0
        if duration_ms > 0:
            horizon_s = min(max(12.0, duration_ms / 1000.0 + 5.0), 70.0)
        want = min(2, int(expected.get("tick_count_expected") or 2))
        died = min_hp == 0
        while (
            len([amount for amount in tick_amounts if band[0] <= amount <= band[1]]) < want
            and latched_s < horizon_s
            and not died
            and driver.target_health(dummy) not in (None, 0)
        ):
            driver.poll(2.0)
            latched_s += 2.0
            latch_hp = driver.target_health(dummy)
            if latch_hp is not None and (min_hp is None or latch_hp < min_hp):
                min_hp = latch_hp
                died = min_hp == 0
            tick_amounts = matching_periodic_ticks(events_since(driver, mark), spell_id, dummy, guid)
        post_hp = driver.target_health(dummy)
        if min_hp is not None and (post_hp is None or min_hp < post_hp):
            post_hp = min_hp
        if latched_s:
            tick_note = f"; tick latch +{latched_s:.0f}s"
    post = f"dummy guid={dummy} health={post_hp}"
    consumed = ""
    if name.startswith("Judgement") and go and not aura_present(driver, guid, 20154):
        consumed = "; seal 20154 consumed by the judgement"
        if 20154 in applied_auras:
            applied_auras.remove(20154)
    verdict = single_outcome_verdict(
        row,
        {
            "go": go,
            "reason": reason,
            "caster_aura": caster_aura,
            "target_guid": dummy,
            "target_aura": target_aura,
            "pre_hp": pre_hp,
            "post_hp": post_hp,
            "damage_logs": logs,
            "tick_amounts": tick_amounts,
            "pre": pre,
            "post": post,
            "packets": ops,
            "attempts": attempts,
        },
    )
    verdict["evidence"] += (
        f" after {attempts} attempt(s)" + consumed + reuse_note + cooldown_erased + cooldown_note + tick_note
    )
    if dummy_override is not None:
        # Pinned dummies live far from the arena (finisher bears):
        # walk back so later rows still see the 705/707 counters
        # and the ally position. Without this every following
        # arena row reads dummy-unavailable / OUT_OF_RANGE.
        driver.walk_to(*CASTER_POS)
    return verdict


def blessing_leg(
    row: dict, driver: ScriptedDriver, guid: int, ally_driver: ScriptedDriver, ally_guid: int, applied_auras: list[int]
) -> dict:
    """Blessing ally leg + self leg with aura-bounce reset
    tracking. PASS only when both legs verify. The ally leg runs
    first so the row's own cooldown (Innervate 29166: 360 s) can
    never fail the second leg with NOT_READY."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    aura_id = int(expected.get("aura", spell_id))
    pre = f"caster {snapshot(driver, guid)}; ally {ally_guid}"
    mark = event_mark(driver)
    # The ally parks ~37 yd from the caster spot: close to blessing
    # range before the friendly leg, and re-approach once on
    # OUT_OF_RANGE like the pilot Growl leg.
    ops2: list[int] = []
    go_ally, reason_ally = False, None
    cooldown_waited = 0.0
    standoff = 10.0
    for _ in range(2):
        driver.approach_guid(ally_guid, standoff)
        go_ally, reason_ally, chunk, _ally_waited = cast_unit_retry_power(driver, spell_id, ally_guid, timeout=10.0)
        ops2 += chunk
        if go_ally or reason_ally != SPELL_FAILED_OUT_OF_RANGE:
            break
        standoff = 5.0
        driver.poll(1.0)
    if not go_ally and reason_ally == SPELL_FAILED_NOT_READY:
        # Cooldown-gated blessing (e.g. Freedom 1044, RecoveryTime
        # 20s, when a previous row left the same cooldown behind):
        # wait it out once and retry. Capped at 30s; longer
        # cooldowns stay FAIL.
        cooldown_waited = min(float(row.get("cooldown_ms") or 1500) / 1000.0, 30.0)
        driver.poll(max(cooldown_waited, 2.0))
        driver.approach_guid(ally_guid, 5.0)
        go_ally, reason_ally, chunk = cast_unit_faced(driver, spell_id, ally_guid, timeout=10.0)
        ops2 += chunk
    ally_driver.poll(2.0)
    driver.poll(1.0)
    ally_aura = aura_present(ally_driver, ally_guid, aura_id)
    face_guid(driver, guid)
    go_self, reason_self, ops1, _self_waited = cast_unit_retry_power(driver, spell_id, guid, timeout=10.0)
    self_cooldown_note = ""
    if not go_self and reason_self == SPELL_FAILED_NOT_READY:
        go_self, reason_self, ops1, self_cooldown_note = not_ready_retry(
            driver, guid, spell_id, row.get("cooldown_ms"), ops1
        )
    driver.poll(2.0)
    self_aura = aura_present(driver, guid, aura_id)
    if self_aura and aura_id not in applied_auras:
        applied_auras.append(aura_id)
    packets = ops1 + ops2 + [op for op, _ in events_since(driver, mark)]
    detail = (
        f"self leg go={go_self} reason={reason_self} "
        f"aura={self_aura}; ally leg go={go_ally} "
        f"reason={reason_ally} aura={ally_aura}"
    )
    if cooldown_waited:
        detail += f" cooldown_wait={cooldown_waited:.1f}s"
    detail += self_cooldown_note
    if go_self and self_aura and go_ally and ally_aura:
        return make_verdict(
            row,
            "PASS",
            evidence="self + friendly legs verify: " + detail,
            pre=pre,
            post=f"ally aura={ally_aura}",
            packets=packets,
            target_guid=ally_guid,
        )
    phase = None
    if not (go_self and go_ally):
        phase = "prepare"
    else:
        phase = "apply"
    return make_verdict(
        row,
        "FAIL",
        phase=phase,
        reason=(f"CAST_RESULT({reason_self}/{reason_ally})" if not (go_self and go_ally) else None),
        evidence="blessing legs incomplete: " + detail,
        pre=pre,
        post=f"ally aura={ally_aura}",
        packets=packets,
        target_guid=ally_guid,
    )


def resurrect_leg(row: dict, driver: ScriptedDriver, guid: int, ally) -> tuple[dict, object]:
    """Corpse-target resurrect scenario (Rebirth and kin).

    The ally victim is reseeded at level 1 with 5 hp so a
    Coldridge trogg kills it promptly (the death probe's
    shape), releases spirit, and the caster casts on the
    corpse object. PASS observability: the bound SPELL_GO,
    SMSG_RESURRECT_REQUEST on the ally's own socket, and
    the ally alive with health after it accepts
    (CMSG_RESURRECT_RESPONSE). The ally session is always
    logged out at the end; the caller reseeds it for later
    legs, so the returned ally handle is None.
    """
    spell_id = row["spell_id"]
    ally_guid = guid_of("Campwarrior")
    ally_account = int(smoke.sql_scalar("realmd", "SELECT id FROM account WHERE username='SPELLCAMP01'"))
    if ally is not None:
        try:
            smoke.logout(ally[0], "resurrect-ally-reset")
        except Exception:
            pass
    seed_character(ally_guid, seed_spells_for("Warrior"), pos=ALLY_POS)
    smoke.sql_scalar("characters", f"UPDATE characters SET level=1, health=5 WHERE guid={ally_guid}")
    ally_client, ally_driver = login_as("SPELLCAMP01", ally_account, ally_guid)
    pre = f"victim ally {ally_guid} level1/5hp"
    # Readiness: the stance cast can outrun the ally's own
    # object landing in its world state right after a
    # relogin (full runs read stance go=False/reason=None
    # there while subsets pass). Wait for the self object.
    ready_deadline = time.monotonic() + 15.0
    while time.monotonic() < ready_deadline and ally_driver.world.get(ally_guid) is None:
        ally_driver.poll(0.5)
    caster_moved = [False]

    def fail(phase: str, evidence: str, packets: list[int], target_guid: int | None = None):
        if caster_moved[0]:
            # The caster walked to the far death spot for
            # the corpse cast: walk back to the arena or
            # every following row starts out of dummy/
            # ally visibility (and in bear aggro range).
            driver.walk_to(*CASTER_POS)
        try:
            smoke.logout(ally_client, "resurrect-victim")
        except Exception:
            pass
        return (
            make_verdict(
                row, "FAIL", phase=phase, evidence=evidence, pre=pre, packets=packets, target_guid=target_guid
            ),
            None,
        )

    stance_ok, stance_reason, _stance_ops = ensure_battle_stance(ally_driver, ally_guid)
    for _retry in range(2):
        if stance_ok:
            break
        ally_driver.poll(2.0)
        stance_ok, stance_reason, _stance_ops = ensure_battle_stance(ally_driver, ally_guid)
    pull_notes = [f"stance go={stance_ok} reason={stance_reason}"]

    def pull(target: int, label: str) -> bool:
        go, reason, _ops = cast_unit_faced(ally_driver, 1715, target, timeout=10.0)
        if not go and reason == SPELL_FAILED_NO_POWER:
            # Rage decayed on the walk over: Bloodrage
            # (seeded setup spell) refills one Hamstring.
            send_and_wait(ally_driver, cast_self_body(2687), 2687, timeout=10.0)
            ally_driver.poll(1.6)
            go, reason, _ops = cast_unit_faced(ally_driver, 1715, target, timeout=10.0)
        pull_notes.append(f"{label}:go={go}/r{reason}")
        return go

    # Pull a finisher bear with Hamstring: arena troggs
    # cannot finish the victim (proximity alone never
    # aggroed; one trogg ties level-1 regen; several
    # still left it at 2 hp after 90 s). A level-9 bear
    # kills a 5 hp level-1 victim in a few swings.
    pulled = False
    for counter, xyz in FINISHER_DUMMIES:
        if ally_driver.self_health() == 0:
            break
        bear = raw_guid(1196, counter)
        ally_driver.approach_guid(bear, 2.5, fallback=xyz)
        if ally_driver.target_health(bear) in (0, None):
            pull_notes.append(f"bear{counter}:untracked")
            continue
        if pull(bear, f"bear{counter}"):
            pulled = True
            break
    if not pulled and ally_driver.self_health() != 0:
        # Bear fallback: multi-pull arena troggs (the
        # second resurrect row in a run can find every
        # bear dead, evaded, or untracked after the
        # first kill).
        for _ in range(4):
            if ally_driver.self_health() == 0:
                break
            trogg = live_dummy(ally_driver, 707, TROGG_COUNTERS)
            if trogg is None:
                pull_notes.append("trogg:none")
                break
            ally_driver.approach_guid(trogg, 2.5)
            pulled = pull(trogg, "trogg") or pulled
    pre += "; pulls " + ",".join(pull_notes)
    if not pulled and ally_driver.self_health() != 0:
        return fail("prepare", "no live bear or trogg to kill the victim ally", [])
    deadline = time.monotonic() + 90.0
    while time.monotonic() < deadline and ally_driver.self_health() not in (0, None):
        ally_driver.poll(0.5)
        driver.poll(0.1)
    if ally_driver.self_health() != 0:
        return fail("apply", f"victim ally survived the kill pull at {ally_driver.self_health()} hp", [])
    death_pos = ally_driver.position[:3]
    ally_driver.client.send_packet(CMSG_REPOP_REQUEST, b"")
    if not ack_teleports(ally_driver):
        return fail("apply", "no graveyard teleport after repop request", [])
    driver.walk_to(*death_pos)
    caster_moved[0] = True
    driver.poll(1.0)
    corpse_guid = corpse_guid_for(driver, ally_guid)
    if corpse_guid is None:
        return fail("prepare", "victim corpse not resolvable on the caster socket or SQL row", [])
    mark = event_mark(ally_driver)
    go, reason, ops = send_and_wait(driver, cast_corpse_body(spell_id, corpse_guid), spell_id, timeout=25.0)
    ally_driver.poll(2.0)
    requests = [body for op, body in events_since(ally_driver, mark) if op == SMSG_RESURRECT_REQUEST]
    if not go:
        return fail("prepare", f"no bound SPELL_GO for spell {spell_id} (reason={reason})", ops, corpse_guid)
    if not requests:
        return fail("apply", "bound SPELL_GO but no SMSG_RESURRECT_REQUEST on the victim socket", ops, corpse_guid)
    ally_driver.client.send_packet(CMSG_RESURRECT_RESPONSE, struct.pack("<QB", guid, 1))
    ack_teleports(ally_driver)
    deadline = time.monotonic() + 20.0
    hp = ally_driver.self_health()
    while time.monotonic() < deadline and not (hp and hp > 0):
        ally_driver.poll(0.5)
        driver.poll(0.1)
        hp = ally_driver.self_health()
    if not hp:
        return fail("apply", "victim accepted but never came alive", ops, corpse_guid)
    try:
        smoke.logout(ally_client, "resurrect-victim")
    except Exception:
        pass
    driver.walk_to(*CASTER_POS)
    verdict = make_verdict(
        row,
        "PASS",
        evidence=f"bound SPELL_GO + SMSG_RESURRECT_REQUEST on victim socket; accepted, alive at {hp} hp",
        pre=pre,
        post=f"victim alive hp={hp}",
        packets=ops,
        target_guid=corpse_guid,
    )
    return verdict, None


def heal_leg(row: dict, driver: ScriptedDriver, guid: int, ally_driver: ScriptedDriver, ally_guid: int) -> dict:
    """Regen-baselined heal on the ally's own socket (Druid-5187
    lesson). No oracle band exists for Paladin heals, so the gate is
    rise-over-baseline above one discrete regen tick (+34): a bounded,
    band-less check, stated as such in the evidence."""
    spell_id = row["spell_id"]
    # The ally parks out of short-range heal reach; close to 10 yd
    # first (blessing legs already do), with one closer retry on
    # OUT_OF_RANGE instead of failing the row.
    driver.approach_guid(ally_guid, 10.0)
    ally_driver.poll(1.0)
    t0 = time.monotonic()
    h0 = ally_driver.self_health()
    ally_driver.poll(3.0)
    t1 = time.monotonic()
    h1 = ally_driver.self_health()
    pre = f"ally guid={ally_guid} health={h0}->{h1}"
    mark = event_mark(driver)
    go, reason, ops, waited = cast_unit_retry_power(driver, spell_id, ally_guid, timeout=12.0)
    if not go and reason == SPELL_FAILED_OUT_OF_RANGE:
        driver.approach_guid(ally_guid, 4.0)
        driver.poll(1.0)
        go, reason, ops2, waited = cast_unit_retry_power(driver, spell_id, ally_guid, timeout=12.0)
        ops += ops2
    ally_driver.poll(4.0)
    t2 = time.monotonic()
    h2 = ally_driver.self_health()
    # Flush the caster's own socket: cast-completion packets (the
    # heal log rides here) arrive while only the ally is polled.
    driver.poll(1.0)
    window = [op for op, _ in events_since(driver, mark)]
    if not go:
        phase = "prepare"
        if reason == SPELL_FAILED_NO_POWER:
            detail = f"power-starved after {waited} 9s waits"
        else:
            detail = f"no bound SPELL_GO (reason={reason})"
        return make_verdict(
            row,
            "FAIL",
            phase=phase,
            reason=(f"CAST_RESULT({reason})" if reason is not None else None),
            evidence=detail,
            pre=pre,
            post=f"ally health={h2 or '?'}",
            packets=ops + window,
            target_guid=ally_guid,
        )
    jump, rise_ok = heal_rise_ok(h0, h1, h2, t0, t1, t2)
    if h0 is None or h1 is None or h2 is None or t1 <= t0:
        return make_verdict(
            row,
            "FAIL",
            phase="apply",
            evidence=f"GO but ally health unreadable {h0}->{h1}->{h2}",
            pre=pre,
            post=f"ally health={h2 or '?'}",
            packets=ops + window,
            target_guid=ally_guid,
        )
    rate = (h1 - h0) / (t1 - t0) if t1 > t0 else 0.0
    if rise_ok:
        return make_verdict(
            row,
            "PASS",
            evidence=f"bound GO + regen-baselined jump {jump:.0f} "
            f"> 50 (no oracle band; band-less rise gate, rate "
            f"{rate:.1f}/s, {h0}->{h1} pre, got {h2})",
            pre=pre,
            post=f"ally health={h2}",
            packets=ops + window,
            target_guid=ally_guid,
        )
    # Sub-noise heals (rank-1 heals inside the +34 regen-tick
    # quantum, e.g. Healing Wave r1 ≈ 35-45) cannot separate from
    # a phase-aligned tick by health delta. The engine emits the
    # fork's SMSG_SPELLHEALLOG with the formula-derived amount,
    # which regen never produces: bind it against the live oracle
    # band. Health must still not have dropped over the window.
    heal_logs = matching_heal_logs(events_since(driver, mark), spell_id, ally_guid, guid)
    band = live_heal_band(spell_id)
    logged = [log["amount"] for log in heal_logs if band is not None and band[0] <= log["amount"] <= band[1]]
    if logged and h2 >= h0:
        return make_verdict(
            row,
            "PASS",
            evidence=f"bound GO + heal log {logged[0]} in oracle band "
            f"[{band[0]}-{band[1]}] (sub-noise heal; rate "
            f"{rate:.1f}/s, {h0}->{h1} pre, got {h2})",
            pre=pre,
            post=f"ally health={h2}",
            packets=ops + window,
            target_guid=ally_guid,
        )
    detail = (
        f"GO but jump {jump:.0f} <= 50 above baseline (no oracle band; rate {rate:.1f}/s, {h0}->{h1} pre, got {h2})"
    )
    if heal_logs:
        detail += f"; heal logs (out of band {band}): {[(log['spell_id'], log['amount']) for log in heal_logs]}"
    else:
        detail += f"; no heal log for spell {spell_id} (band {band})"
    return make_verdict(
        row,
        "FAIL",
        phase="apply",
        evidence=detail,
        pre=pre,
        post=f"ally health={h2}",
        packets=ops + window,
        target_guid=ally_guid,
    )


def self_heal_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    client: object,
    seed_all: list[int],
    account: str,
    account_id: int,
) -> tuple[dict, tuple | None]:
    """Self-targeted heal rows (Warrior Bloodthirst restore ranks):
    a full-health caster shows no delta, so the caster relogs
    SQL-damaged (health=300, the ally-leg pattern) and casts on
    itself. Verification mirrors heal_leg: regen-baselined rise
    first, then the sub-noise heal-log path against the live oracle
    band (these restores are 10-20, far below the rise gate). The
    old session is gone, so the second item is the fresh session
    whenever the relog succeeds (even when the row fails); None
    only when the relog itself fails and the caller must stop."""
    spell_id = row["spell_id"]
    try:
        smoke.logout(client, "warrior-selfheal")
    except Exception:
        pass
    try:
        client.sock.close()
    except Exception:
        pass
    for _ in range(10):
        try:
            online = smoke.sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
        except RuntimeError:
            online = "1"
        if online != "1":
            break
        time.sleep(8.0)
    seed_character(guid, seed_all, pos=CASTER_POS)
    smoke.sql_scalar("characters", f"UPDATE characters SET health=300 WHERE guid={guid}")
    try:
        new_client, driver = login_as(account, account_id, guid)
    except Exception as exc:
        return (
            make_verdict(
                row,
                "FAIL",
                phase="prepare",
                evidence=f"self-heal relog failed: {exc}",
                pre="",
                post="",
                packets=[],
                target_guid=guid,
            ),
            None,
        )
    session: tuple | None = (new_client, driver)
    driver.poll(1.0)
    t0 = time.monotonic()
    h0 = driver.self_health()
    driver.poll(3.0)
    t1 = time.monotonic()
    h1 = driver.self_health()
    pre = f"self guid={guid} health={h0}->{h1} (SQL-damaged)"
    mark = event_mark(driver)
    go, reason, ops, waited = cast_self_retry_power(driver, spell_id, timeout=12.0)
    if not go:
        phase = "prepare"
        if reason == SPELL_FAILED_NO_POWER:
            detail = f"power-starved after {waited} 9s waits"
        else:
            detail = f"no bound SPELL_GO (reason={reason})"
        return (
            make_verdict(
                row,
                "FAIL",
                phase=phase,
                reason=(f"CAST_RESULT({reason})" if reason is not None else None),
                evidence=detail,
                pre=pre,
                post=f"self health={driver.self_health()}",
                packets=ops,
                target_guid=guid,
            ),
            session,
        )
    driver.poll(4.0)
    t2 = time.monotonic()
    h2 = driver.self_health()
    driver.poll(1.0)
    window = [op for op, _ in events_since(driver, mark)]
    jump, rise_ok = heal_rise_ok(h0, h1, h2, t0, t1, t2)
    rate = (h1 - h0) / (t1 - t0) if h0 is not None and h1 is not None and t1 > t0 else 0.0
    if rise_ok:
        return (
            make_verdict(
                row,
                "PASS",
                evidence=f"bound GO + self heal rise {jump:.0f} > 50 (rate {rate:.1f}/s, {h0}->{h1} pre, got {h2})",
                pre=pre,
                post=f"self health={h2}",
                packets=ops + window,
                target_guid=guid,
            ),
            session,
        )
    heal_logs = matching_heal_logs(events_since(driver, mark), spell_id, guid, guid)
    band = live_heal_band(spell_id)
    logged = [log["amount"] for log in heal_logs if band is not None and band[0] <= log["amount"] <= band[1]]
    if logged and h2 is not None and h0 is not None and h2 >= h0:
        return (
            make_verdict(
                row,
                "PASS",
                evidence=f"bound GO + self heal log {logged[0]} in oracle band "
                f"[{band[0]}-{band[1]}] (sub-noise heal; rate "
                f"{rate:.1f}/s, {h0}->{h1} pre, got {h2})",
                pre=pre,
                post=f"self health={h2}",
                packets=ops + window,
                target_guid=guid,
            ),
            session,
        )
    detail = f"GO but self jump {jump:.0f} <= 50 (rate {rate:.1f}/s, {h0}->{h1} pre, got {h2})"
    if heal_logs:
        detail += f"; heal logs (out of band {band}): {[(log['spell_id'], log['amount']) for log in heal_logs]}"
    else:
        detail += f"; no heal log for spell {spell_id} (band {band})"
    return (
        make_verdict(
            row,
            "FAIL",
            phase="apply",
            evidence=detail,
            pre=pre,
            post=f"self health={h2}",
            packets=ops + window,
            target_guid=guid,
        ),
        session,
    )


def teleport_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    client: object,
    seed_all: list[int],
    account: str,
    account_id: int,
) -> tuple[dict, tuple | None]:
    """Cross-map teleport row (Teleport: Moonglade): dest-cast at the
    caster's feet, PASS iff the server starts the far transfer (bound
    SPELL_GO plus TRANSFER_PENDING/NEW_WORLD in the window). The
    driver never ACKs the worldport, so the session cannot continue
    on the new map: the leg reseeds and fresh-logins back on the
    bench map and returns the new session. The second item is None
    only when the row fails (old session still usable).
    """
    spell_id = row["spell_id"]
    pos = driver.position
    pre = snapshot(driver, guid)
    mark = event_mark(driver)
    go, reason, ops = False, None, []
    waited = 0
    while True:
        go, reason, cast_ops = send_and_wait(
            driver, cast_dest_body(spell_id, pos[0], pos[1], pos[2]), spell_id, timeout=15.0, target_guid=guid
        )
        ops += cast_ops
        if go or reason != SPELL_FAILED_NO_POWER or waited >= POWER_RETRIES:
            break
        waited += 1
        driver.poll(9.0)
    driver.poll(6.0)
    window = [op for op, _ in events_since(driver, mark)]
    packets = ops + window
    transferred = SMSG_TRANSFER_PENDING in window or SMSG_NEW_WORLD in window
    if not go or not transferred:
        phase = None if go else "prepare"
        if go:
            detail = "SPELL_GO without transfer packets: the engine did not start the far transfer"
            why = None
        elif reason == SPELL_FAILED_NO_POWER:
            detail = f"power-starved after {waited} 9s waits; teleport never cast"
            why = f"CAST_RESULT({reason})"
        else:
            detail = f"no bound SPELL_GO (reason={reason})"
            why = f"CAST_RESULT({reason})" if reason is not None else None
        return (
            make_verdict(
                row,
                "FAIL",
                phase=phase,
                reason=why,
                evidence=detail,
                pre=pre,
                post=snapshot(driver, guid),
                packets=packets,
                target_guid=guid,
            ),
            None,
        )
    try:
        smoke.logout(client, "Druid-teleport")
    except Exception:
        pass
    # The wedged session never ACKs the worldport, so the server only
    # releases the character once the socket goes away: close it here
    # (logout alone cannot finish) and wait for the online flag to
    # clear before reseeding.
    try:
        client.sock.close()
    except Exception:
        pass
    for _ in range(10):
        try:
            online = smoke.sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
        except RuntimeError:
            online = "1"
        if online != "1":
            break
        time.sleep(8.0)
    seed_character(guid, seed_all, pos=CASTER_POS)
    new_client, new_driver = login_as(account, account_id, guid)
    drift_gate(new_driver, guid, len(seed_all))
    return (
        make_verdict(
            row,
            "PASS",
            evidence=f"bound GO + far-transfer packets {sorted(set(packets))}; reseeded to bench map, session fresh",
            pre=pre,
            post=snapshot(new_driver, guid),
            packets=packets,
            target_guid=guid,
        ),
        (new_client, new_driver),
    )


def ground_leg(
    row: dict,
    driver: ScriptedDriver,
    guid: int,
    used_dummies: set[int],
    applied_auras: list[int],
    pre_cast_setup=None,
) -> dict:
    """Ground cast at the caster's feet with a dummy inside the
    radius; outcome is dummy damage attributed to the spell. A
    landed cast registers for the next row's cancel so a channeled
    radius spell (Hurricane) cannot hold the slot open into the
    following row (the cancel-aura channel interrupt). An optional
    pre_cast_setup(driver) runs after the approach (Warrior
    Bloodrage rage top-up for the shout rows)."""
    spell_id = row["spell_id"]
    pre = snapshot(driver, guid)
    dummy, reused, dummy_waited = wait_for_dummy(driver, used_dummies)
    if dummy is None:
        return make_verdict(
            row, "BLOCKED", reason="dummy-unavailable", evidence="no live dummy for the radius leg", pre=pre
        )
    used_dummies.add(dummy)
    if reused:
        pre = f"{pre}; reused observably-clean dummy (alive, full health, aura-free)"
    if dummy_waited:
        pre = f"{pre}; pool wait +{dummy_waited:.0f}s"
    driver.approach_guid(dummy, 2.0)
    pre_hp = driver.target_health(dummy)
    pre = f"dummy guid={dummy} health={pre_hp}; caster {pre}"
    if pre_cast_setup is not None:
        # The setup gets the row dummy so a tap engages it in
        # place: tapping a far spare would walk the caster out of
        # the row dummy's radius and the dest cast (taken below)
        # would land on nobody (Piercing Howl at 25 yd).
        pre += pre_cast_setup(driver, dummy)
    # Face the dummy at cast time: it chases into melee during
    # the approach and can end beside or behind the caster,
    # which empties frontal-cone enumeration (Cone of Cold saw
    # SPELL_GO with no victims) even though radius legs pass.
    face_guid(driver, dummy)
    # Fresh position: the setup may have moved (spare tap for a
    # dummy-less leg), so the dest must center on where the
    # caster stands now, not where the approach left it.
    pos = driver.position
    mark = event_mark(driver)
    pre_gameobjects = {
        g for g, o in driver.world.objects.items() if o.type_id in (TYPEID_GAMEOBJECT, TYPEID_DYNAMICOBJECT)
    }
    # One cast with the dest-target sender (mask 0x0040 + xyz at the
    # caster's feet), with NO_POWER pacing, not a self cast plus a
    # dest cast.
    go, reason, ops = False, None, []
    waited = 0
    cooldown_waited = False
    while True:
        go, reason, cast_ops = send_and_wait(
            driver, cast_dest_body(spell_id, pos[0], pos[1], pos[2]), spell_id, timeout=15.0, target_guid=guid
        )
        ops += cast_ops
        if not go and reason == SPELL_FAILED_NOT_READY and not cooldown_waited:
            # Long category cooldowns shared across ranks
            # (Lightwell: 600 s) wait out once, in chunks
            # that keep the session polling.
            cooldown_waited = True
            remaining = live_cooldown_wait_s(spell_id, cap_s=900.0)
            pre += f"; cooldown wait +{remaining:.0f}s"
            while remaining > 0:
                chunk = min(30.0, remaining)
                driver.poll(chunk)
                remaining -= chunk
            continue
        if go or reason != SPELL_FAILED_NO_POWER or waited >= POWER_RETRIES:
            break
        waited += 1
        driver.poll(9.0)
    if not go and reason == SPELL_FAILED_NO_POWER:
        return make_verdict(
            row,
            "FAIL",
            phase="prepare",
            reason=f"CAST_RESULT({reason})",
            evidence=f"power-starved after {waited} 9s waits",
            pre=pre,
            post=snapshot(driver, guid),
            packets=ops,
            target_guid=dummy,
        )
    # Sliced low-water hp like the hostile leg: a kill respawns
    # the dummy (full health) and fast regen erases small hits,
    # so the single final read cannot tell landed damage apart
    # from a clean miss. Latch short auras the same way: a 6 s
    # daze (Piercing Howl) expires inside one long poll and a
    # single post-window check reads never-applied.
    want_aura_id = None
    if (row.get("expected") or {}).get("kind") in ("aura_on_target", "aura_on_caster"):
        # aura_on_caster must arm the latch too: Blink's 1 s
        # mechanic-immunity aura on the caster is the outcome,
        # and without the id the sliced caster poll below
        # never runs (caster_aura=False on a landed Blink).
        want_aura_id = int((row.get("expected") or {}).get("aura", spell_id))
    min_hp = pre_hp
    target_aura = False
    object_seen = False
    caster_aura = False
    sliced = 0.0
    while sliced < 6.0:
        # Quarter-second slices: a fear aura that breaks on
        # the first dummy swing (or a field update batch
        # that lands late) can come and go inside one
        # 1 s slice in full-run load.
        driver.poll(0.25)
        sliced += 0.25
        probe_hp = driver.target_health(dummy)
        if probe_hp is not None and (min_hp is None or probe_hp < min_hp):
            min_hp = probe_hp
        if want_aura_id is not None and not target_aura and aura_present(driver, dummy, want_aura_id):
            target_aura = True
        if want_aura_id is not None and not caster_aura and aura_present(driver, guid, want_aura_id):
            caster_aura = True
        if not object_seen:
            # Summoned-object outcome (Lightwell GO, Blizzard
            # dynobject): a new gameobject/dynamicobject
            # tracked near the caster that was not there
            # before the cast.
            for obj_guid, obj in driver.world.objects.items():
                if (
                    obj.type_id not in (TYPEID_GAMEOBJECT, TYPEID_DYNAMICOBJECT)
                    or obj_guid in pre_gameobjects
                    or obj.position is None
                ):
                    continue
                if math.dist(obj.position[:3], driver.position[:3]) <= 15.0:
                    object_seen = True
                    break
    window = events_since(driver, mark)
    ops += [op for op, _ in window]
    post_hp = driver.target_health(dummy)
    if min_hp is not None and (post_hp is None or min_hp < post_hp):
        post_hp = min_hp
    logs = matching_damage_logs(window, spell_id, dummy, guid)
    if go and spell_id not in applied_auras:
        applied_auras.append(spell_id)
    # Instant area-aura rows (Howl of Terror) land fear on the
    # in-radius dummy instead of a dynobject: the latched aura
    # above is the outcome, with the old post-window check as a
    # fallback for server-slow applies landing after the slices.
    if not target_aura and want_aura_id is not None:
        target_aura = aura_present(driver, dummy, want_aura_id)
    if target_aura and want_aura_id is not None and want_aura_id not in applied_auras:
        applied_auras.append(want_aura_id)
    if caster_aura and want_aura_id is not None and want_aura_id not in applied_auras:
        applied_auras.append(want_aura_id)
    return single_outcome_verdict(
        row,
        {
            "go": go,
            "reason": reason,
            "caster_aura": caster_aura,
            "target_guid": dummy,
            "target_aura": target_aura,
            "pre_hp": pre_hp,
            "post_hp": post_hp,
            "damage_logs": logs,
            "object_seen": object_seen,
            "pre": pre,
            "post": f"dummy guid={dummy} health={post_hp}",
            "packets": ops,
        },
    )


def totem_leg(row: dict, driver: ScriptedDriver, guid: int, applied_auras: list[int]) -> dict:
    """Summon-totem rows: self cast, then latch the new totem
    creature by its UNIT_CREATED_BY_SPELL field near the caster
    in sliced polls (the create-object batch can lag the GO)."""
    spell_id = row["spell_id"]
    pre = snapshot(driver, guid)
    pre_units = set(driver.world.objects)
    mark = event_mark(driver)
    go, reason, ops, _waited = cast_self_retry_power(driver, spell_id, timeout=15.0)
    if not go and reason == SPELL_FAILED_NOT_READY:
        # Shared category cooldown from the chain's earlier rank
        # (Mana Tide 17359 reads NOT_READY behind 16190's 300 s
        # category): wait out the live fork cooldown in capped
        # chunks like the self leg, then recast.
        total_s = live_cooldown_wait_s(spell_id, cap_s=float("inf"))
        waited_s = 0.0
        chunks = 0
        while not go and reason == SPELL_FAILED_NOT_READY and waited_s < total_s and chunks < 6:
            chunk = min(600.0, total_s - waited_s)
            driver.poll(chunk)
            waited_s += chunk
            chunks += 1
            go, reason, chunk_ops, _waited = cast_self_retry_power(driver, spell_id, timeout=15.0)
            ops += chunk_ops
    totem_seen = False
    sliced = 0.0
    while go and sliced < 6.0 and not totem_seen:
        driver.poll(0.25)
        sliced += 0.25
        for obj_guid, obj in driver.world.objects.items():
            if (
                obj.type_id == TYPEID_UNIT
                and obj_guid not in pre_units
                and obj.values.get(UNIT_CREATED_BY_SPELL) == spell_id
                and obj.position is not None
                and math.dist(obj.position[:3], driver.position[:3]) <= 15.0
            ):
                totem_seen = True
                break
    window = [op for op, _ in events_since(driver, mark)]
    if go and spell_id not in applied_auras:
        applied_auras.append(spell_id)
    return single_outcome_verdict(
        row,
        {
            "go": go,
            "reason": reason,
            "caster_aura": False,
            "target_guid": guid,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "totem_seen": totem_seen,
            "pre": pre,
            "post": snapshot(driver, guid),
            "packets": ops + window,
        },
    )


def item_leg(row: dict, driver: ScriptedDriver, guid: int, used_dummies: set[int], applied_auras: list[int]) -> dict:
    """Reagent rows verify the count decremented; equipped-item rows
    (Holy Shield) verify the aura with a fixture-gear gate."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    pre = snapshot(driver, guid)
    reagent = expected.get("reagent")
    if reagent:
        before = bag_reagent_count(guid, reagent)
        if before is None:
            return make_verdict(
                row,
                "BLOCKED",
                reason="effect-marker-only",
                evidence="reagent count unreadable; no decrement leg",
                pre=pre,
            )
        if before <= 0:
            return make_verdict(
                row,
                "BLOCKED",
                reason="fixture-reagent",
                evidence=f"reagent {reagent} absent from bags; the fixture seed did not cover this row",
                pre=pre,
            )
        # Hostile-unit reagent rows (Blind, Enslave Demon)
        # cannot cast on the caster: stage a dummy and cast
        # there; the decrement observable is unchanged. A
        # dummy rejected BAD_TARGETS means the spell needs a
        # target kind the dummy pool cannot stage (a demon
        # for Enslave) — an honest scenario block, converted
        # below.
        cast_target = guid
        dummy = None
        if 6 in (row.get("target_a") or []):
            dummy, _reused, _waited_dummy = wait_for_dummy(driver, used_dummies)
            if dummy is None:
                return make_verdict(
                    row,
                    "BLOCKED",
                    reason="dummy-unavailable",
                    evidence="no live dummy for the hostile-unit reagent leg",
                    pre=pre,
                )
            used_dummies.add(dummy)
            driver.approach_guid(dummy, 2.0)
            cast_target = dummy
        mark = event_mark(driver)
        auras_before = aura_ids_on(driver, guid)
        go, reason, ops, _waited = cast_unit_retry_power(driver, spell_id, cast_target, timeout=15.0)
        if not go and dummy is not None and reason == SPELL_FAILED_BAD_TARGETS:
            return make_verdict(
                row,
                "BLOCKED",
                reason="scenario-target",
                evidence="dummy target rejected BAD_TARGETS; the spell needs a target "
                "kind the dummy pool cannot stage (e.g. a demon for Enslave Demon)",
                pre=pre,
                packets=ops,
                target_guid=dummy,
            )
        driver.poll(4.0)
        after = bag_reagent_count(guid, reagent)
        window = [op for op, _ in events_since(driver, mark)]
        alive = self_alive(driver, guid)
        detail = "" if alive else "; caster died casting (death reset next)"
        verdict = single_outcome_verdict(
            row,
            {
                "go": go,
                "reason": reason,
                "caster_aura": False,
                "target_guid": guid,
                "target_aura": False,
                "pre_hp": None,
                "post_hp": None,
                "damage_logs": [],
                "reagent_before": before,
                "reagent_after": after,
                "pre": pre,
                "post": snapshot(driver, guid) + detail,
                "packets": ops + window,
            },
        )
        if go:
            # Register for the inter-row cancel: a landed
            # reagent spell can leave a controlling aura on
            # the caster (Divine Intervention cast on self
            # stuns via its triggered immunity holder 19753,
            # not the cast spell's own id, and every
            # following row read FAILED_STUNNED). Track the
            # cast spell plus every newly appeared holder.
            for new_id in [spell_id, *sorted(aura_ids_on(driver, guid) - auras_before)]:
                if new_id not in applied_auras:
                    applied_auras.append(new_id)
        if verdict["verdict"] == "PASS":
            verdict["evidence"] += detail
        return verdict
    want_class = int(row.get("equipped_item_class", -1))
    if want_class == 2:
        # Weapon rows (Hunter ranged/melee): the fixture seeds a
        # class-2 weapon in the ranged slot (fork
        # EQUIPMENT_SLOT_RANGED = 17). Melee rows ride the same
        # gate: the Rust engine has no weapon check, and the
        # fixture also seeds a mainhand sword for fork honesty.
        gear = equipped_item_class(guid, slot=EQUIPMENT_SLOT_RANGED)
        if gear is None or gear[0] != 2:
            return make_verdict(
                row,
                "BLOCKED",
                reason="fixture-gear",
                evidence=f"ranged slot is not weapon-class (class 2): {gear}; hunter fixture lacks a ranged weapon",
                pre=pre,
            )
        verdict = self_leg(row, driver, guid, applied_auras)
        verdict["evidence"] += f"; ranged weapon equipped {gear}"
        return verdict
    if want_class != 4:
        return make_verdict(
            row,
            "BLOCKED",
            reason="fixture-gear",
            evidence=f"equipped-item class {want_class} has no fixture gate; scenario-inapplicable",
            pre=pre,
        )
    gear = equipped_item_class(guid)
    if gear is None or gear[0] != 4:
        return make_verdict(
            row,
            "BLOCKED",
            reason="fixture-gear",
            evidence=f"offhand is not armor-class (class 4): {gear}; starter lacks a shield",
            pre=pre,
        )
    verdict = self_leg(row, driver, guid, applied_auras)
    verdict["evidence"] += f"; shield equipped {gear}"
    return verdict


#: Fork summon vocabulary (`SpellEffects.h`): the pet scenario's
#: setup rows carry one of these; LEARN_PET_EFFECTS rides along so a
#: future learn-pet row dispatches the same way.
PET_SUMMON_EFFECT_IDS = {28, 56, 112}
PET_LEARN_EFFECT_IDS = {57}
#: Fork `SPELL_EFFECT_SUMMON_DEAD_PET` (`SpellDefines.h:320`):
#: Revive Pet needs a dead pet, which no live leg scenarios.
PET_REVIVE_EFFECT_IDS = {109}
#: Fork summon-object effects (`SpellDefines.h`): trap placements
#: carry 104 (GAMEOBJECT). Rows with these effects plus no
#: observable outcome (no aura, no damage/heal/script) create an
#: object the harness cannot observe (object_seen unsupported).
BARE_OBJECT_EFFECT_IDS = {76, 86, 104, 105, 106, 107}
#: Fork hostile targets (`SpellDefines.h`), mirroring the matrix
#: generator's HOSTILE_TARGETS for the item-bucket routing below.
HOSTILE_TARGET_IDS = {6, 2, 15, 16, 24, 28, 36, 54}


def class_summon_spell(rows: list[dict]) -> int | None:
    """Lowest reagent-free summon row for a class, or None.

    Pure function of matrix rows: the pet scenario exists only when
    the matrix carries a castable summon (pet bucket, summon effect,
    pet_created outcome, no reagents a reagent-less engine cannot
    consume). Reagent-gated summons cannot create the scenario (their
    fixture-reagent BLOCKED is honest engine evidence), so they are
    never returned here."""
    candidates = [
        int(row["spell_id"])
        for row in rows
        if row.get("bucket") == "pet"
        and set(row.get("effect_ids", []) or []) & (PET_SUMMON_EFFECT_IDS | PET_LEARN_EFFECT_IDS)
        and not (row.get("campaign", "") or "").startswith("classify_only")
        and (row.get("expected") or {}).get("kind") == "pet_created"
        and not any(row.get("reagents", []) or [])
    ]
    return min(candidates) if candidates else None


def find_summoned_pet(
    before_guids: set[int],
    candidates: list[tuple],
    caster_pos: tuple[float, ...],
    max_dist_yd: float = 20.0,
) -> int | None:
    """New-unit pet discovery (pure part of the summon leg).

    candidates are (guid, x, y, health, entry) tracker tuples; the
    pet is the nearest new alive object within max_dist_yd of the
    caster. A pre-existing guid is never returned, so a stale world
    entry cannot verify a failed summon."""
    best: tuple[float, int] | None = None
    for guid, x, y, health, _entry in candidates:
        if guid in before_guids:
            continue
        if health is None or health <= 0:
            continue
        dist = math.hypot(float(x) - float(caster_pos[0]), float(y) - float(caster_pos[1]))
        if dist <= max_dist_yd and (best is None or dist < best[0]):
            best = (dist, int(guid))
    return best[1] if best is not None else None


def find_active_pet(objects, caster_pos: tuple[float, ...], entry: int, max_dist_yd: float = 20.0) -> int | None:
    """Already-out pet discovery (pure part of the fixture-summon reuse).

    The engine auto-loads the slot-0 saved pet at world entry, so a
    Hunter Call Pet cast with the pet already out is a silent no-op
    (GO, nothing new to track). The scenario pet is the nearest
    alive unit of the fixture entry near the caster carrying
    campaign-pet health (level-60 pet rows hold thousands of HP;
    wild same-entry wolves and 100-hp arena dummies must never
    verify as the scenario pet).
    """
    best: tuple[float, int] | None = None
    for tracked_guid, obj in objects.items():
        if getattr(obj, "entry", None) != entry:
            continue
        health = getattr(obj, "health", None)
        if health is None or health <= 1000:
            continue
        pos = getattr(obj, "position", None)
        if pos is None:
            continue
        dist = math.hypot(float(pos[0]) - float(caster_pos[0]), float(pos[1]) - float(caster_pos[1]))
        if dist <= max_dist_yd and (best is None or dist < best[0]):
            best = (dist, int(tracked_guid))
    return best[1] if best is not None else None


def pet_summon_leg(row: dict, driver: ScriptedDriver, guid: int) -> dict:
    """Pet-scenario setup: self-cast the class summon row, then PASS
    only when a new alive unit object tracks near the caster (the
    pet). GO without a tracked pet is an apply FAIL with the
    summon-failure evidence the pet rows' BLOCKED then cites."""
    spell_id = row["spell_id"]
    pre = snapshot(driver, guid)
    before = set(driver.world.objects)
    caster_pos = (driver.position[0], driver.position[1])
    mark = event_mark(driver)
    # Summon casts run the full DBC cast time (Summon Imp's GO lands
    # ~10.15 s after START on the bench): the 10 s default would time
    # out just before GO and abandon a pet that materializes right
    # after. The 25 s budget covers any 10 s summon plus latency.
    go, reason, ops, waited = cast_self_retry_power(driver, spell_id, timeout=25.0)
    if not go and reason == SPELL_FAILED_NO_POWER:
        return make_verdict(
            row,
            "FAIL",
            phase="prepare",
            reason=f"CAST_RESULT({reason})",
            evidence=f"power-starved after {waited} 9s waits; summon never cast",
            pre=pre,
            post=snapshot(driver, guid),
            packets=ops,
            target_guid=None,
        )
    if not go:
        return make_verdict(
            row,
            "FAIL",
            phase="prepare",
            reason=(f"CAST_RESULT({reason})" if reason is not None else None),
            evidence=f"no bound SPELL_GO for summon {spell_id} (reason={reason})",
            pre=pre,
            post=snapshot(driver, guid),
            packets=ops + [op for op, _ in events_since(driver, mark)],
            target_guid=None,
        )
    pet_guid: int | None = None
    pet_entry: int | None = None
    pet_dist = 0.0
    waited_s = 0.0
    while pet_guid is None and waited_s < 8.0:
        driver.poll(1.0)
        waited_s += 1.0
        candidates = []
        for tracked_guid, obj in driver.world.objects.items():
            if obj.position is None:
                continue
            candidates.append((tracked_guid, obj.position[0], obj.position[1], obj.health, obj.entry))
        pet_guid = find_summoned_pet(before, candidates, caster_pos)
        if pet_guid is not None:
            found = driver.world.get(pet_guid)
            pet_entry = found.entry if found is not None else None
            if found is not None and found.position is not None:
                pet_dist = math.hypot(
                    float(found.position[0]) - float(caster_pos[0]),
                    float(found.position[1]) - float(caster_pos[1]),
                )
    packets = ops + [op for op, _ in events_since(driver, mark)]
    if pet_guid is None:
        return make_verdict(
            row,
            "FAIL",
            phase="apply",
            evidence=f"bound SPELL_GO for summon {spell_id} but no new alive unit tracked "
            f"within 20 yd after {waited_s:.0f}s; pet scenario not established",
            pre=pre,
            post=snapshot(driver, guid),
            packets=packets,
            target_guid=None,
        )
    note = f" after {waited} mana wait(s)" if waited else ""
    # The summon's own GCD is still running when the pet tracks: the
    # next cast in the same flow (a pet-bucket row, or the row after
    # a summon row) would eat NOT_READY. Outlast it here.
    driver.poll(2.0)
    return make_verdict(
        row,
        "PASS",
        evidence=f"bound SPELL_GO + pet object tracked guid={pet_guid} entry={pet_entry} "
        f"dist={pet_dist:.1f}yd health={driver.target_health(pet_guid)}{note} +2s GCD settle",
        pre=pre,
        post=snapshot(driver, guid),
        packets=packets,
        target_guid=pet_guid,
    )


def pet_leg(row: dict, driver: ScriptedDriver, guid: int, pet_guid: int, applied_auras: list[int]) -> dict:
    """Pet-bucket row cast on the summoned pet (Health Funnel style).

    The summon leg establishes the pet first; this leg only casts on
    it. A pet that vanished between setup and cast is a no-pet
    BLOCKED, never a cast at a dead guid."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    if driver.target_health(pet_guid) is None:
        return make_verdict(
            row,
            "BLOCKED",
            reason="no-pet",
            evidence=f"pet guid={pet_guid} no longer tracked; re-summon path runs before the next pet row",
            pre=snapshot(driver, guid),
            target_guid=None,
        )
    pre = f"pet guid={pet_guid} health={driver.target_health(pet_guid)}; caster {snapshot(driver, guid)}"
    aura_id = int(expected.get("aura", spell_id))
    mark = event_mark(driver)
    go, reason, ops, _waited = cast_unit_retry_power(driver, spell_id, pet_guid, timeout=10.0)
    driver.poll(2.0)
    post_hp = driver.target_health(pet_guid)
    target_aura = aura_present(driver, pet_guid, aura_id)
    if target_aura and aura_id not in applied_auras:
        applied_auras.append(aura_id)
    window = events_since(driver, mark)
    verdict = single_outcome_verdict(
        row,
        {
            "go": go,
            "reason": reason,
            "caster_aura": False,
            "target_guid": pet_guid,
            "target_aura": target_aura,
            "pre_hp": driver.target_health(pet_guid),
            "post_hp": post_hp,
            "damage_logs": matching_damage_logs(window, spell_id, pet_guid, guid),
            "tick_amounts": matching_periodic_ticks(window, spell_id, pet_guid, guid),
            "pre": pre,
            "post": f"pet guid={pet_guid} health={post_hp}",
            "packets": ops + [op for op, _ in window],
        },
    )
    return verdict


def pet_cast_leg(row: dict, driver: ScriptedDriver, guid: int, pet_guid: int) -> dict:
    """Pet-cast row (TARGET_UNIT_CASTER_MASTER, e.g. Sacrifice):
    order the pet to cast on its master via CMSG_PET_ACTION (373),
    then verify the master-side outcome. The voidwalker must be
    the active pet for Sacrifice; ensure_pet's imp cannot cast it,
    so this leg summons the voidwalker first when needed (697;
    the engine has no reagent gate yet, recorded in evidence)."""
    spell_id = row["spell_id"]
    expected = row.get("expected", {})
    aura_id = int(expected.get("aura", spell_id))
    pre = snapshot(driver, guid)
    note = ""
    pet_obj = driver.world.get(pet_guid)
    if pet_obj is None or pet_obj.entry != 1860:  # Voidwalker entry
        before = set(driver.world.objects)
        go, reason, _ops, _w = cast_self_retry_power(driver, 697, timeout=25.0)
        driver.poll(2.0)
        found = None
        waited = 0.0
        while found is None and waited < 8.0:
            driver.poll(1.0)
            waited += 1.0
            for g, obj in driver.world.objects.items():
                if g not in before and obj.entry == 1860 and (obj.health or 0) > 0:
                    found = g
                    break
        if not go or found is None:
            return make_verdict(
                row,
                "FAIL",
                phase="prepare",
                reason=(f"CAST_RESULT({reason})" if reason is not None else None),
                evidence=f"voidwalker summon 697 for Sacrifice did not verify (go={go} reason={reason} found={found})",
                pre=pre,
                post=snapshot(driver, guid),
                packets=[],
                target_guid=None,
            )
        pet_guid = found
        note = "; voidwalker summoned for the sacrifice"
    # Teach the rank (grimoire wrapper; a fresh voidwalker knows
    # only Torment). Already-known ranks simply no-op in the fork
    # learn path, so this is safe to repeat per row.
    teach_id = {7812: 7885, 19443: 19447}.get(int(spell_id))
    if teach_id:
        tgo, treason, _tops, _tw = cast_self_retry_power(driver, teach_id)
        driver.poll(1.0)
        note += f"; teach {teach_id} go={tgo} reason={treason}"
    mark = event_mark(driver)
    # CMSG_PET_ACTION: pet guid (raw u64), data (flag<<24 | spell),
    # target guid (raw u64). ACT_ENABLED (0xC1) orders the cast.
    data = (0xC1 << 24) | int(spell_id)
    driver.client.send_packet(373, struct.pack("<QIQ", pet_guid, data, guid))
    caster_aura = False
    waited = 0.0
    while waited < 6.0:
        driver.poll(0.5)
        waited += 0.5
        if aura_present(driver, guid, aura_id):
            caster_aura = True
            break
    pet_after = driver.target_health(pet_guid)
    window = events_since(driver, mark)
    packets = [op for op, _ in window]
    fail_code = None
    for op, body in window:
        if op == 312 and len(body) >= 5:  # SMSG_PET_CAST_FAILED
            fail_code = body[4]
    verdict = single_outcome_verdict(
        row,
        {
            "go": caster_aura or 306 in packets,
            "reason": fail_code,
            "caster_aura": caster_aura,
            "target_guid": guid,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": pre,
            "post": snapshot(driver, guid),
            "packets": packets,
        },
    )
    verdict["evidence"] += f"; pet_cast via CMSG_PET_ACTION pet={pet_guid} health_after={pet_after}{note}"
    return verdict


def pre_dispatch(row: dict, summon_spell: int | None = None) -> tuple[str, str | None]:
    """Pure per-row dispatch: matrix row -> (action, payload).

    "blocked" carries a closed-enum BLOCKED reason (the row is never
    cast, and can never PASS); "passive" means a learned_spell
    character_spell check without casting; "cast" carries the leg name
    ("friendly", "hostile", "ground", "item", "form", "form_gated",
    "pet", "totem", or "self"). Every bucket any fan-out matrix contains maps
    here; pet rows ride the pet leg only when a summon scenario
    exists for the class (summon_spell from class_summon_spell),
    otherwise they stay honestly BLOCKED no-pet. Actual totems
    ride the totem leg (summoned-creature latch), while
    stance-gated form_totem rows ride form_gated. Ranged-weapon
    item rows verify on the dummy (hostile for unit targets, ground
    for area-only rows like Volley); bare summon-object placements
    and dead-pet revives stay BLOCKED. Mirrors the
    run_class branch order so the self-test can assert totality over a
    real artifact."""
    campaign = row.get("campaign", "")
    kind = (row.get("expected") or {}).get("kind")
    bucket = row.get("bucket")
    if campaign.startswith("classify_only"):
        if campaign == "classify_only_script" or kind == "dual_outcome":
            return ("blocked", "script-evidence-unknown")
        return ("blocked", "effect-marker-only")
    if kind == "dual_outcome":
        return ("blocked", "script-evidence-unknown")
    if kind == "learned_spell":
        return ("passive", None)
    if (row.get("power_type") or 0) == 2 and (row.get("mana_cost") or 0) > 0:
        # Pet-resource (focus) spells: no player caster holds focus,
        # so the engine honestly answers NO_POWER, and pet autocasts
        # have no harness leg. Without a pet scenario they stay
        # uncast (Shaman Lightning Breath 24844/25012 are wind
        # serpent autocasts, not player spells).
        return ("blocked", "no-pet")
    if PET_REVIVE_EFFECT_IDS & set(row.get("effect_ids") or []):
        # Revive Pet: no dead-pet scenario in a live class run.
        return ("blocked", "target-not-dead")
    if bucket == "party_duel":
        return ("blocked", "no-party")
    if bucket == "corpse":
        # Corpse-target resurrect rows ride the dedicated
        # dead-ally scenario (kill, release, cast on the
        # corpse object, accept over the wire).
        return ("cast", "resurrect")
    if bucket == "pet":
        effects = set(row.get("effect_ids", []) or [])
        if effects & (PET_SUMMON_EFFECT_IDS | PET_LEARN_EFFECT_IDS):
            # The summon row itself creates the scenario.
            return ("cast", "pet")
        if 27 in (row.get("target_a") or []):
            # TARGET_UNIT_CASTER_MASTER: cast BY the pet on its
            # master (Sacrifice); rides the pet-cast leg when a
            # summon scenario exists.
            if summon_spell is not None:
                return ("cast", "pet_cast")
            return ("blocked", "no-pet")
        if summon_spell is not None:
            return ("cast", "pet")
        return ("blocked", "no-pet")
    if bucket == "form_totem":
        if kind == "totem_summon":
            return ("cast", "totem")
        if required_form_spell(row) is not None:
            return ("cast", "form_gated")
        if kind == "aura_on_caster":
            # Totem rows whose observable is a caster aura
            # (Sentry Totem's vision): the self leg latches
            # the aura; the summoned totem is incidental.
            # Stance-gated rows keep form_gated precedence.
            return ("cast", "self")
        return ("blocked", "engine-totem")
    if bucket == "form":
        return ("cast", "form")
    if bucket == "friendly":
        return ("cast", "friendly")
    if bucket == "self" and 25 in (row.get("target_a") or []):
        # Self-bucketed but unit-targeted (Mind Vision: aura
        # effects on the selected unit plus the caster): a
        # bare self cast reads BAD_IMPLICIT_TARGETS, so ride
        # the hostile leg on a dummy (its caster-aura latch
        # verifies the channel side).
        return ("cast", "hostile")
    if bucket == "self" and kind == "heal_delta_target":
        # Self-targeted heals (Warrior Bloodthirst restore rows):
        # the single-repro self leg has no damaged-caster scenario,
        # so they ride the relog self-heal leg, which SQL-damages
        # the caster offline first like the ally heal legs.
        return ("cast", "self_heal")
    if bucket == "hostile_dummy":
        return ("cast", "hostile")
    if bucket == "ground_area":
        if 6 in (row.get("target_a") or []):
            # Mixed unit-target + areaselect rows (Warrior
            # Intimidating Shout 5246: trigger on the hostile unit
            # target plus the area fear): the dest-only ground leg
            # sends no unit target, so the engine honestly answers
            # BAD_IMPLICIT_TARGETS. They ride the hostile leg, whose
            # targ-22 standoff closes to 2 yd so the area component
            # still lands. No other fan-out matrix has a
            # ground-bucket row with a hostile unit target.
            return ("cast", "hostile")
        if kind == "heal_delta_target":
            # Unit-targeted heals filed under ground_area by the
            # target-45 first-match order (Shaman Chain Heal /
            # Healing Wave): the dest leg would miscast a
            # unit-targeted spell, so they ride the ally heal leg
            # like friendly heals. Only the single-target
            # component verifies there; chain jumps have no fork
            # code and stay explicitly unverified.
            return ("cast", "friendly")
        effects = set(row.get("effect_ids") or [])
        if (
            kind == "object_created"
            and effects & BARE_OBJECT_EFFECT_IDS
            and not (effects & (PET_SUMMON_EFFECT_IDS | PET_LEARN_EFFECT_IDS))
            and not any(row.get("aura_ids") or [])
        ):
            # Bare summon-object placements (Hunter traps): the
            # trap object is wire-invisible (object_seen
            # unsupported) and the trigger damage attributes to the
            # effect spell, whose own rows verify it. Stays uncast.
            return ("blocked", "effect-marker-only")
        return ("cast", "ground")
    if bucket == "item_reagent_ammo":
        if kind in ("damage_log", "aura_on_target") and set(row.get("target_a") or []) & HOSTILE_TARGET_IDS:
            # Weapon/damage rows verify on the dummy, not the
            # caster: unit-targeted rows ride hostile, area-only
            # rows (Volley) ride the ground dest leg. Reagent
            # rows join them now that the fixture seeds the bags
            # and the engine consumes (Soul Fire/Shadowburn sat
            # fixture-gear BLOCKED behind the old reagent-free
            # condition; the landed cast itself proves the gate
            # passed, sibling item rows prove consumption).
            if 6 in (row.get("target_a") or []):
                return ("cast", "hostile")
            return ("cast", "ground")
        return ("cast", "item")
    return ("cast", "self")


# Honest BLOCKED evidence for buckets no live scenario can execute.
UNSCENARIOED_EVIDENCE = {
    "wire-target-unsupported": "no corpse-target scenario in a live class "
    "run; the parser discards corpse GUIDs (correction 2)",
    "no-pet": "no pet scenario for this class in a live run; nothing to observe",
    "engine-totem": "totem/stance hardware scenario not implemented in this harness; engine-totem",
    "effect-marker-only": "no wire-observable outcome for this row shape in a live run "
    "(bare trap placements, marker-only rows); left uncast",
    "target-not-dead": "no dead-pet scenario in a live class run; revive rows stay uncast",
}


def run_class(class_name: str, only: list[int] | None = None, matrix_path: pathlib.Path | str | None = None) -> int:
    """Data-driven full-class run: every generated cast row for the
    class, serially, one harness process under RUNNING.lock. Rows
    sorted by spell_id; classify_only rows ride as BLOCKED without a
    cast. `matrix_path` overrides the class's default artifact (used by
    `--matrix` and tests). Returns 0 only when the class gate holds."""
    if matrix_path is not None:
        matrix_file = pathlib.Path(matrix_path)
        if not matrix_file.exists():
            print(f"run-class {class_name}: matrix file missing: {matrix_file}", file=sys.stderr)
            return 2
        doc = json.loads(matrix_file.read_text())
    else:
        # matrix_doc_for prefers the per-class fan-out artifact when
        # the generator wrote one (Warrior is a pilot class, so
        # matrix_path_for alone would record the pilot matrix while
        # the run executes fan-out rows): record the same file the
        # rows come from.
        doc = matrix_doc_for(class_name)
        try:
            import spell_matrix_gen as _gen  # noqa: PLC0415

            _per_class = _gen.class_matrix_path(class_name)
            matrix_file = _per_class if _per_class.exists() else matrix_path_for(class_name)
        except (ImportError, AttributeError):
            matrix_file = matrix_path_for(class_name)
    rows = sorted((r for r in doc.get("rows", []) if r["class"] == class_name), key=lambda r: r["spell_id"])
    if only:
        wanted = set(only)
        rows = [r for r in rows if r["spell_id"] in wanted]
        missing = wanted - {r["spell_id"] for r in rows}
        if missing:
            print(f"run-class {class_name}: spells not in matrix: {sorted(missing)}", file=sys.stderr)
            return 2
    if not rows:
        print(f"run-class {class_name}: no matrix rows; run spell_matrix_gen.py --full first", file=sys.stderr)
        return 2
    try:
        account = CLASS_ACCOUNT[class_name]
        char_name = CLASS_CHAR[class_name]
    except KeyError:
        print(f"run-class {class_name}: no §2 fixture; want one of {sorted(CLASS_ACCOUNT)}", file=sys.stderr)
        return 2
    seed_all = seed_spells_for(class_name)
    combo_builder = doc.get("full_combo_builder_spell")
    # Pet scenario: the class summon row (reagent-free, pet_created)
    # casts first in-session; pet-bucket rows cast on the tracked
    # pet. Without one the pet rows stay BLOCKED no-pet.
    class_rows = [r for r in doc.get("rows", []) if r.get("class") == class_name]
    by_spell_id = {int(r["spell_id"]): r for r in class_rows}
    summon_spell = class_summon_spell(class_rows)
    if class_name == "Hunter" and summon_spell is None:
        # Call Pet 883 is family-0 (outside the matrix, seeded via
        # SETUP_SPELLS): the fixture summon for the Hunter pet
        # scenario. Not a matrix row, so its verdict is setup
        # evidence only and is never recorded (row counts must
        # equal the matrix).
        summon_spell = HUNTER_PET_SETUP_SPELL
    acquire_lock()
    verdicts: list[dict] = []
    manifest_extra: dict = {
        "mode": "run-class",
        "class": class_name,
        "matrix_file": str(matrix_file),
        "matrix_mode": doc.get("mode"),
        "matrix_counts": doc.get("per_class_counts", {}),
        "talent_build": ((doc.get("talent_builds") or {}).get("selected") or {}).get(class_name),
        "fixture_note": (
            "union-seeded single-race fixture; not a legal "
            "single-race spellbook claim where union exceeds "
            "any one race (Paladin Human==Dwarf==160, "
            "numerically equal)"
        ),
        "seed_count": len(seed_all),
        "talent_seed_checks": [],
    }
    try:
        guid = guid_of(char_name)
        account_id = int(smoke.sql_scalar("realmd", f"SELECT id FROM account WHERE username='{account}'"))

        def fresh_login(pos: tuple | None = None):
            seed_character(guid, seed_all, pos=pos or CASTER_POS, hunter_pet=(class_name == "Hunter"))
            if class_name == "Hunter":
                seed_hunter_gear(guid)
            if class_name == "Warrior":
                seed_warrior_gear(guid)
            if class_name in ("Mage", "Priest"):
                seed_caster_gear(guid)
            seed_reagents(guid, rows)
            client, driver = login_as(account, account_id, guid)
            drift_gate(driver, guid, len(seed_all))
            return client, driver

        client, driver = fresh_login()
        pet_guid: int | None = None

        def ensure_pet(active_row: dict) -> dict | None:
            """Summon-once pet setup. Returns a verdict to record for
            the active row, or None when the tracked pet is ready and
            the caller may proceed (summon rows: nothing more to
            cast after a ready pet is verified). The summon verdict
            is recorded at the first row that needs it, in row order
            when the summon row itself is reached. The Hunter
            fixture summon (Call Pet, not a matrix row) is setup
            evidence only: its verdict is never recorded."""
            nonlocal pet_guid
            fixture_summon = summon_spell is not None and summon_spell not in by_spell_id
            is_summon_row = (
                summon_spell is not None and not fixture_summon and int(active_row["spell_id"]) == summon_spell
            )
            if not is_summon_row and pet_guid is not None and (driver.target_health(pet_guid) or 0) > 0:
                return None
            pet_guid = None
            if summon_spell is None or (not fixture_summon and summon_spell not in by_spell_id):
                return make_verdict(
                    active_row,
                    "BLOCKED",
                    reason="no-pet",
                    evidence="no castable summon row for this class in the matrix; pet rows stay uncast",
                    pre=snapshot(driver, guid),
                    target_guid=None,
                )
            if fixture_summon:
                # The engine auto-loads the slot-0 saved pet at
                # world entry, so by the first pet row the fixture
                # pet is usually already out (a Call Pet cast then
                # is a silent no-op GO with nothing new to track).
                # Reuse it directly, parked passive like a
                # summoned one; only cast when nothing qualifies.
                existing = find_active_pet(
                    driver.world.objects,
                    (driver.position[0], driver.position[1]),
                    HUNTER_PET_ENTRY if class_name == "Hunter" else -1,
                )
                if existing is not None:
                    pet_guid = existing
                    driver.client.send_packet(373, struct.pack("<QIQ", pet_guid, 0x06 << 24, 0))
                    driver.poll(0.4)
                    return None
                summon_row = {
                    "class": class_name,
                    "spell_id": summon_spell,
                    "name": "Call Pet",
                    "bucket": "pet",
                    "expected": {"kind": "pet_created"},
                }
            else:
                summon_row = by_spell_id[summon_spell]
            summon_verdict = pet_summon_leg(summon_row, driver, guid)
            if summon_verdict["verdict"] == "PASS" and summon_verdict.get("target_guid"):
                pet_guid = int(summon_verdict["target_guid"])
                # A defensive pet assists on every caster target and
                # kills 100 hp dummies mid-observation (its Firebolt
                # damage is unattributed in the caster's logs, and
                # the fast respawn then reads as full health). Park
                # it passive: ACT_REACTION (0x06) / REACT_PASSIVE (0)
                # via CMSG_PET_ACTION, like a player clicking Passive.
                driver.client.send_packet(373, struct.pack("<QIQ", pet_guid, 0x06 << 24, 0))
                driver.poll(0.4)
            if fixture_summon:
                # Setup evidence only (not a matrix row): proceed
                # to the pet row on PASS, else BLOCKED no-pet.
                if pet_guid is None:
                    return make_verdict(
                        active_row,
                        "BLOCKED",
                        reason="no-pet",
                        evidence=f"fixture summon {summon_spell} failed: {summon_verdict['evidence'][:200]}",
                        pre=snapshot(driver, guid),
                        target_guid=None,
                    )
                return None
            if not is_summon_row:
                verdicts.append(summon_verdict)
                print(
                    f"ROW {class_name} {summon_spell} pet: "
                    f"{summon_verdict['verdict']} {summon_verdict.get('reason') or ''} | "
                    f"{summon_verdict['evidence'][:120]}"
                )
                if pet_guid is None:
                    return make_verdict(
                        active_row,
                        "BLOCKED",
                        reason="no-pet",
                        evidence=f"summon {summon_spell} failed: {summon_verdict['evidence'][:200]}",
                        pre=snapshot(driver, guid),
                        target_guid=None,
                    )
                return None
            return summon_verdict

        try:
            # Talent seed check: every selected known spell learned.
            for known in talent_known_spells(class_name):
                have = smoke.sql_scalar(
                    "characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid} AND spell={known}"
                )
                manifest_extra["talent_seed_checks"].append({"spell_id": known, "learned": int(have or 0) > 0})
            used_dummies: set[int] = set()
            applied_auras: list[int] = []
            ally: tuple | None = None

            def need_ally(heal_rows: bool, force: bool = False) -> tuple:
                """Second client (Campwarrior) for friendly legs; the
                ally is SQL-damaged offline first when heals need it.
                Heal legs pass force=True: a regen-topped ally makes
                every heal after the first unobservable, so each heal
                row re-damages a freshly logged-in ally."""
                nonlocal ally
                if ally is not None and not force:
                    return ally
                if ally is not None:
                    old_client, _old_driver, _old_guid = ally
                    try:
                        smoke.logout(old_client, f"{class_name}-ally")
                    except Exception as exc:
                        print(f"WARN ally relog logout: {exc}")
                    ally = None
                ally_guid = guid_of("Campwarrior")
                ally_seed = seed_spells_for("Warrior") or pilot_spells_for("Warrior")
                seed_character(ally_guid, ally_seed, pos=ALLY_POS)
                if heal_rows:
                    smoke.sql_scalar("characters", f"UPDATE characters SET health=300 WHERE guid={ally_guid}")
                ally_id = int(smoke.sql_scalar("realmd", "SELECT id FROM account WHERE username='SPELLCAMP01'"))
                ally_client, ally_driver = login_as("SPELLCAMP01", ally_id, ally_guid)
                ally = (ally_client, ally_driver, ally_guid)
                return ally

            def cancel_tracked(target_guid: int, force: tuple[int, ...] = ()) -> None:
                pending = list(applied_auras)
                for forced in force:
                    if forced not in pending:
                        pending.append(forced)
                if pending:
                    cancel_auras(driver, target_guid, pending, force=force)
                    applied_auras.clear()
                driver.poll(1.6)

            for row in rows:
                spell_id = row["spell_id"]
                bucket = row.get("bucket")
                expected = row.get("expected", {})
                kind = expected.get("kind")
                campaign = row.get("campaign", "")
                if not self_alive(driver, guid):
                    # The dead session still holds the online flag, so
                    # reseeding first would trip ensure_offline: retire
                    # the client (logout, drop the socket, wait for the
                    # flag to clear) the way the teleport wedge path
                    # does, then log back in fresh.
                    try:
                        smoke.logout(client, f"{class_name}-dead")
                    except Exception as exc:
                        print(f"WARN dead-session logout: {exc}")
                    try:
                        client.sock.close()
                    except Exception:
                        pass
                    for _ in range(10):
                        try:
                            online = smoke.sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
                        except RuntimeError:
                            online = "1"
                        if online != "1":
                            break
                        time.sleep(8.0)
                    client, driver = fresh_login()
                    used_dummies.clear()
                    applied_auras.clear()
                    # A dead master dismisses the pet: force a
                    # re-summon before the next pet row.
                    pet_guid = None
                if class_name == "Warrior" and self_alive(driver, guid):
                    # Wounded-warrior top-up: ambient dummy
                    # retaliation grinds health down across the run
                    # (a row died mid-setup at 48 hp), and a long
                    # rage setup at low health dies the same way.
                    # Relog SQL-topped-up like the heal legs; rage
                    # persists across the relog, stances are entered
                    # per row afterwards, and the dummy pool is
                    # world state, so nothing else resets.
                    driver.poll(1.0)
                    wobj = driver.world.get(guid)
                    hp = wobj.values.get(UNIT_FIELD_HEALTH) if wobj else None
                    maxhp = wobj.values.get(UNIT_FIELD_MAXHEALTH) if wobj else None
                    if hp is not None and maxhp and hp < 1200:
                        try:
                            smoke.logout(client, f"{class_name}-topup")
                        except Exception as exc:
                            print(f"WARN topup logout: {exc}")
                        try:
                            client.sock.close()
                        except Exception:
                            pass
                        for _ in range(10):
                            try:
                                online = smoke.sql_scalar(
                                    "characters", f"SELECT online FROM characters WHERE guid={guid}"
                                )
                            except RuntimeError:
                                online = "1"
                            if online != "1":
                                break
                            time.sleep(8.0)
                        smoke.sql_scalar("characters", f"UPDATE characters SET health={int(maxhp)} WHERE guid={guid}")
                        client, driver = login_as(account, account_id, guid)
                        print(f"ROW {class_name} {spell_id}: topped up {hp}->{maxhp}")
                # A landed ground cast may still be channeling with
                # no holder on the caster (Hurricane's radius): force
                # its cancel so the next row never meets a busy slot.
                # Other buckets ride applied_auras alone (a forced
                # cancel there could strip a seal a later leg needs).
                force: tuple[int, ...] = ()
                if verdicts and verdicts[-1]["verdict"] == "PASS" and verdicts[-1].get("bucket") == "ground_area":
                    force = (int(verdicts[-1]["spell_id"]),)
                cancel_tracked(guid, force)
                # A previous row's channel (Health Funnel, Hellfire,
                # Drain Life) otherwise occupies the cast slot and
                # the next cast dies SPELL_IN_PROGRESS (97) even
                # though its own scenario is fine. The stock client
                # cancels with CMSG_CANCEL_CHANNELLING (315) /
                # CMSG_CANCEL_CAST (303) naming the spell; both are
                # no-ops when the slot is already free.
                if verdicts:
                    prev_spell = int(verdicts[-1]["spell_id"])
                    driver.client.send_packet(315, struct.pack("<I", prev_spell))
                    driver.client.send_packet(303, struct.pack("<I", prev_spell))
                    driver.poll(0.4)
                if campaign.startswith("classify_only"):
                    reason = (
                        "script-evidence-unknown"
                        if campaign == "classify_only_script" or kind == "dual_outcome"
                        else "effect-marker-only"
                    )
                    verdicts.append(
                        make_verdict(
                            row,
                            "BLOCKED",
                            reason=reason,
                            evidence=expected.get("note", "classify-only bucket; left uncast"),
                            target_guid=None,
                        )
                    )
                    print(f"ROW {class_name} {spell_id} {bucket}: BLOCKED {reason} | classify_only")
                    continue
                if kind == "dual_outcome":
                    verdicts.append(
                        single_outcome_verdict(row, {"go": False, "reason": None, "packets": [], "target_guid": None})
                    )
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: BLOCKED script-evidence-unknown | dual needs both legs"
                    )
                    continue
                if kind == "learned_spell":
                    have = smoke.sql_scalar(
                        "characters", f"SELECT COUNT(*) FROM character_spell WHERE guid={guid} AND spell={spell_id}"
                    )
                    learned = int(have or 0) > 0
                    aura_ok = aura_present(driver, guid, spell_id)
                    status = passive_learn_verdict(row.get("passive", False), learned, aura_ok)
                    note = "passive spell: no aura holder by design" if row.get("passive", False) else "aura gated"
                    verdicts.append(make_verdict(row, status, evidence=f"learned={learned} aura={aura_ok}; {note}"))
                    print(f"ROW {class_name} {spell_id} {bucket}: {verdicts[-1]['verdict']} | learned={learned}")
                    continue
                if bucket == "party_duel":
                    verdicts.append(
                        make_verdict(
                            row,
                            "BLOCKED",
                            reason="no-party",
                            evidence="solo character with no party; scenario-inapplicable",
                        )
                    )
                    print(f"ROW {class_name} {spell_id} {bucket}: BLOCKED no-party | solo repro")
                    continue
                action, payload = pre_dispatch(row, summon_spell)
                if action == "blocked" and payload in UNSCENARIOED_EVIDENCE:
                    verdicts.append(
                        make_verdict(
                            row, "BLOCKED", reason=payload, evidence=UNSCENARIOED_EVIDENCE[payload], target_guid=None
                        )
                    )
                    print(f"ROW {class_name} {spell_id} {bucket}: BLOCKED {payload} | no live scenario")
                    continue
                if action == "cast" and payload in ("pet", "pet_cast"):
                    pet_verdict = ensure_pet(row)
                    if pet_verdict is not None:
                        verdicts.append(pet_verdict)
                        print(
                            f"ROW {class_name} {spell_id} {bucket}: "
                            f"{pet_verdict['verdict']} "
                            f"{pet_verdict.get('reason') or ''} | "
                            f"{pet_verdict['evidence'][:120]}"
                        )
                        continue
                    if summon_spell is not None and int(spell_id) == summon_spell:
                        # Summon verified by ensure_pet; its verdict is
                        # already recorded, nothing more to cast.
                        continue
                    assert pet_guid is not None
                    if payload == "pet_cast":
                        verdicts.append(pet_cast_leg(row, driver, guid, pet_guid))
                        # Sacrifice consumes the pet; force a
                        # re-summon before the next pet row.
                        pet_guid = None
                    else:
                        verdicts.append(pet_leg(row, driver, guid, pet_guid, applied_auras))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if action == "cast" and payload == "totem":
                    verdicts.append(totem_leg(row, driver, guid, applied_auras))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if action == "cast" and payload == "form_gated":
                    verdicts.append(
                        form_gated_leg(row, driver, guid, used_dummies, applied_auras, combo_builder=combo_builder)
                    )
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if bucket == "form":
                    verdicts.append(form_leg(row, driver, guid))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if action == "cast" and payload == "resurrect":
                    verdict, ally = resurrect_leg(row, driver, guid, ally)
                    verdicts.append(verdict)
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                # Friendly bucket rows always dispatch ("cast",
                # "friendly"); ground-bucket unit heals (Shaman
                # Chain/Healing Wave) are routed there by
                # pre_dispatch, so the payload owns this branch.
                if action == "cast" and payload == "friendly":
                    heal_row = kind == "heal_delta_target"
                    ally_client, ally_driver, ally_guid = need_ally(heal_row, force=heal_row)
                    if kind == "heal_delta_target":
                        verdicts.append(heal_leg(row, driver, guid, ally_driver, ally_guid))
                    else:
                        verdicts.append(blessing_leg(row, driver, guid, ally_driver, ally_guid, applied_auras))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if bucket == "hostile_dummy":
                    finisher_setup = (
                        combo_builder_setup(combo_builder) if row.get("needs_combo_points") and combo_builder else None
                    )
                    if row.get("requires_low_health_target"):
                        finisher_setup = low_health_setup
                    if finisher_setup is None and (row.get("name") or "").startswith("Conflagrate"):
                        immolate_id = min(
                            (r["spell_id"] for r in class_rows if r.get("name") == "Immolate"),
                            default=None,
                        )
                        if immolate_id is not None:
                            finisher_setup = immolate_setup(immolate_id)
                    override = finisher_dummy(used_dummies) if finisher_setup else None
                    if finisher_setup is None:
                        # Warrior rage top-up (None for every other
                        # class and every non-rage row).
                        finisher_setup = warrior_rage_setup_for_row(class_name, row)
                    verdicts.append(
                        hostile_leg(
                            row,
                            driver,
                            guid,
                            used_dummies,
                            applied_auras,
                            pre_cast_setup=finisher_setup,
                            dummy_override=override,
                        )
                    )
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if bucket == "ground_area" and kind == "teleport":
                    verdict, session = teleport_leg(row, driver, guid, client, seed_all, account, account_id)
                    verdicts.append(verdict)
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    if session is not None:
                        client, driver = session
                        used_dummies.clear()
                        applied_auras.clear()
                    continue
                if bucket == "ground_area":
                    if action == "cast" and payload == "hostile":
                        # Mixed unit-target + areaselect rows (Warrior
                        # Intimidating Shout): the unit target rides
                        # the hostile leg, whose targ-22 standoff
                        # closes to 2 yd for the area component.
                        verdicts.append(
                            hostile_leg(
                                row,
                                driver,
                                guid,
                                used_dummies,
                                applied_auras,
                                pre_cast_setup=warrior_rage_setup_for_row(class_name, row),
                            )
                        )
                    else:
                        verdicts.append(
                            ground_leg(
                                row,
                                driver,
                                guid,
                                used_dummies,
                                applied_auras,
                                pre_cast_setup=warrior_rage_setup_for_row(class_name, row),
                            )
                        )
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if bucket == "item_reagent_ammo":
                    if action == "cast" and payload == "hostile":
                        # Ranged-weapon unit rows (Hunter) and
                        # reagent-free weapon rows (Warrior/Rogue):
                        # the pre_dispatch routes them off the item
                        # leg to the dummy, like hostile-bucket rows.
                        # Combo finishers filed here (Rogue
                        # Eviscerate/Kidney/Rupture/Expose) take the
                        # builder setup + pinned finisher dummy,
                        # same as hostile-bucket finishers.
                        item_setup = warrior_rage_setup_for_row(class_name, row)
                        item_override = None
                        if item_setup is None and row.get("needs_combo_points") and combo_builder:
                            item_setup = combo_builder_setup(combo_builder)
                            item_override = finisher_dummy(used_dummies)
                        verdicts.append(
                            hostile_leg(
                                row,
                                driver,
                                guid,
                                used_dummies,
                                applied_auras,
                                pre_cast_setup=item_setup,
                                dummy_override=item_override,
                            )
                        )
                    elif action == "cast" and payload == "ground":
                        # Area-only weapon rows (Hunter Volley):
                        # the dest leg, like ground-bucket rows.
                        verdicts.append(
                            ground_leg(
                                row,
                                driver,
                                guid,
                                used_dummies,
                                applied_auras,
                                pre_cast_setup=warrior_rage_setup_for_row(class_name, row),
                            )
                        )
                    else:
                        verdicts.append(item_leg(row, driver, guid, used_dummies, applied_auras))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if action == "cast" and payload == "hostile" and bucket == "self":
                    # Self-bucketed unit-target rows (Mind
                    # Vision) rerouted by pre_dispatch: ride
                    # the hostile leg on a dummy.
                    verdicts.append(hostile_leg(row, driver, guid, used_dummies, applied_auras))
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                if action == "cast" and payload == "self_heal":
                    verdict, session = self_heal_leg(row, driver, guid, client, seed_all, account, account_id)
                    verdicts.append(verdict)
                    if session is not None:
                        client, driver = session
                        used_dummies.clear()
                        applied_auras.clear()
                    print(
                        f"ROW {class_name} {spell_id} {bucket}: "
                        f"{verdicts[-1]['verdict']} "
                        f"{verdicts[-1].get('reason') or ''} | "
                        f"{verdicts[-1]['evidence'][:120]}"
                    )
                    continue
                # self default: solo self cast (unlisted buckets;
                # corpse/unenterable form_totem rows are BLOCKED above
                # via pre_dispatch and pet rows rode the pet leg,
                # stance-gated rows the form_gated leg).
                verdicts.append(
                    self_leg(
                        row, driver, guid, applied_auras, pre_cast_setup=warrior_rage_setup_for_row(class_name, row)
                    )
                )
                print(
                    f"ROW {class_name} {spell_id} {bucket}: "
                    f"{verdicts[-1]['verdict']} "
                    f"{verdicts[-1].get('reason') or ''} | "
                    f"{verdicts[-1]['evidence'][:120]}"
                )
        finally:
            if ally is not None:
                try:
                    smoke.logout(ally[0], f"{class_name}-ally")
                except Exception as exc:
                    print(f"WARN ally logout: {exc}")
            try:
                smoke.logout(client, f"{class_name}-run")
            except Exception as exc:
                print(f"WARN logout: {exc}")
    finally:
        release_lock()
    cluster = cluster_failures(verdicts)
    manifest_extra["failure_clusters"] = cluster
    manifest_extra["verdict_counts"] = {
        status: sum(1 for v in verdicts if v["verdict"] == status)
        for status in sorted({v["verdict"] for v in verdicts})
    }
    write_results(class_name, verdicts)
    run_leg_line = f"run-class {class_name}: {len(verdicts)} rows, " + ", ".join(
        f"{k}={v}" for k, v in sorted(manifest_extra["verdict_counts"].items())
    )
    print(run_leg_line)
    (EVIDENCE_DIR / f"manifest-{class_name}.json").write_text(
        json.dumps({**manifest_extra, "git_head": subprocess_head(), "git_dirty": subprocess_dirty()}, indent=1) + "\n"
    )
    append_summary(class_name, verdicts, cluster)
    gate = class_gate_failures({class_name: verdicts}, {class_name: len(verdicts)})
    if gate:
        print(f"CLASS-GATE-FAILED {gate}")
        return 1
    print(f"SPELL-CAMPAIGN-RUN-CLASS-{class_name.upper()}-PASS")
    return 0


def cluster_failures(
    verdicts: list[dict],
) -> list[dict]:
    """Group FAIL rows by (phase, CAST_RESULT reason, bucket) so a
    shared gate shows as one cluster before spell-specific triage."""
    groups: dict[tuple, list[int]] = {}
    for v in verdicts:
        if v["verdict"] != "FAIL":
            continue
        key = (v.get("phase"), v.get("reason"), v.get("bucket"))
        groups.setdefault(key, []).append(int(v["spell_id"]))
    return [
        {"phase": phase, "reason": reason, "bucket": bucket, "count": len(spells), "spells": sorted(spells)}
        for (phase, reason, bucket), spells in sorted(groups.items(), key=lambda item: (-len(item[1]), str(item[0])))
    ]


def append_summary(class_name: str, verdicts: list[dict], cluster: list[dict]) -> pathlib.Path:
    """Append one class section to summary.md (never rewrite pilot
    sections)."""
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    path = EVIDENCE_DIR / "summary.md"
    if not path.exists():
        path.write_text("# Spell campaign summary\n\n")
    counts: dict[str, int] = {}
    for v in verdicts:
        counts[v["verdict"]] = counts.get(v["verdict"], 0) + 1
    lines = [f"## {class_name} run-class ({len(verdicts)} rows)", ""]
    lines.append("counts: " + ", ".join(f"{k}={counts.get(k, 0)}" for k in sorted(counts)))
    for v in verdicts:
        if v["verdict"] in ("FAIL", "BLOCKED"):
            lines.append(
                f"- {v['spell_id']} {v.get('name') or ''} "
                f"{v['bucket']} target={v.get('target_guid')}: "
                f"{v['verdict']} {v.get('reason') or ''} "
                f"{v.get('phase') or ''} | {v['evidence']}"
            )
    if cluster:
        lines.append("")
        lines.append("failure clusters:")
        for entry in cluster:
            lines.append(
                f"- phase={entry['phase']} reason={entry['reason']} "
                f"bucket={entry['bucket']} count={entry['count']} "
                f"spells={entry['spells']}"
            )
    lines.append("")
    with open(path, "a") as fh:
        fh.write("\n".join(lines))
    return path


def run_self_test() -> int:
    # Verdict constructor gates.
    row = {"class": "Warrior", "spell_id": 5242, "bucket": "self"}
    v = make_verdict(row, "PASS", evidence="e")
    assert v["verdict"] == "PASS" and v["reason"] is None
    v = make_verdict(row, "BLOCKED", reason="wire-target-unsupported")
    assert v["packets"] == []
    for bad in ("NOPE",):
        try:
            make_verdict(row, bad)
        except AssertionError:
            pass
        else:
            raise AssertionError(f"verdict {bad} accepted")
    try:
        make_verdict(row, "BLOCKED", reason="not-a-reason")
    except AssertionError:
        pass
    else:
        raise AssertionError("bad BLOCKED reason accepted")
    try:
        make_verdict(row, "FAIL", phase="launch")
    except AssertionError:
        pass
    else:
        raise AssertionError("bad FAIL phase accepted")
    # Packet builders match the shapes world_actor parses.
    assert char_create_body("Campwarrior", 1, 1) == (b"Campwarrior\x00" + bytes([1, 1, 0, 0, 0, 0, 0, 0, 0]))
    assert cast_self_body(5242) == struct.pack("<IH", 5242, 0)
    assert cast_unit_body(133, 0xF1300002C1000158) == (
        struct.pack("<IH", 133, 0x0002) + smoke.pack_guid(0xF1300002C1000158)
    )
    dest = cast_dest_body(133, 1.0, 2.0, 3.0)
    assert dest == struct.pack("<IH", 133, 0x0040) + struct.pack("<fff", 1.0, 2.0, 3.0)
    corpse = cast_corpse_body(20484, 0xF1300002C1000158)
    assert corpse == (struct.pack("<IH", 20484, 0x0200) + smoke.pack_guid(0xF1300002C1000158))
    assert raw_guid(705, 344) == 0xF1300002C1000158
    # Packet verdicts bind to spell/caster/target bodies, not opcode
    # presence alone (the shipped send_and_wait accepted any 306/304).
    caster = 900017
    dummy = 0xF1300002C1000158
    assert parse_cast_result_body(struct.pack("<IB", 6795, 86)) == (6795, 86)
    assert parse_cast_result_body(struct.pack("<IBB", 6795, 2, 86)) == (6795, 86)
    assert parse_cast_result_body(b"\x01") is None
    go_body = (
        smoke.pack_guid(caster)
        + smoke.pack_guid(dummy)
        + struct.pack("<BIII", 0, 6795, 0, 12345)
        + bytes([1])
        + struct.pack("<Q", dummy)
        + bytes([0])
    )
    go = parse_spell_go_body(go_body)
    assert go == {
        "caster_guid": caster,
        "target_guid": dummy,
        "cast_counter": 0,
        "spell_id": 6795,
        "hits": [dummy],
        "misses": [],
    }, go
    assert spell_go_matches(go, 6795, caster, dummy)
    assert not spell_go_matches(go, 133, caster, dummy)
    assert not spell_go_matches(go, 6795, 900016, dummy)
    dmg_body = smoke.pack_guid(dummy) + smoke.pack_guid(caster) + struct.pack("<III", 133, 46, 0)
    assert parse_damage_log_body(dmg_body) == {
        "target_guid": dummy,
        "attacker_guid": caster,
        "spell_id": 133,
        "damage": 46,
    }
    window = [
        (592, dmg_body),
        (592, smoke.pack_guid(dummy) + smoke.pack_guid(900016) + struct.pack("<III", 133, 999, 0)),
    ]
    assert matching_damage_logs(window, 133, dummy, caster)[0]["damage"] == 46
    assert matching_damage_logs(window, 133, dummy, 900015) == []
    # Periodic ticks ride 590, not 592: a Consecration-style tick
    # must attribute the same way, and 592 bodies still match.
    tick_body = smoke.pack_guid(dummy) + smoke.pack_guid(caster) + struct.pack("<IIIIIII", 20924, 1, 3, 12, 2, 0, 0)
    assert parse_periodicaura_log_body(tick_body) == {
        "target_guid": dummy,
        "attacker_guid": caster,
        "spell_id": 20924,
        "damage": 12,
    }, parse_periodicaura_log_body(tick_body)
    assert parse_periodicaura_log_body(b"\x01") is None
    tick_window = [(590, tick_body), (592, dmg_body)]
    assert matching_damage_logs(tick_window, 20924, dummy, caster)[0]["damage"] == 12
    assert matching_damage_logs(tick_window, 20924, dummy, 900015) == []
    assert matching_damage_logs(tick_window, 133, dummy, caster)[0]["damage"] == 46
    # 590-only latch: direct 592 hits never count as ticks.
    assert matching_periodic_ticks(tick_window, 20924, dummy, caster) == [12]
    assert matching_periodic_ticks(tick_window, 133, dummy, caster) == []
    assert matching_periodic_ticks(tick_window, 20924, dummy, 900016) == []
    # Heal logs ride 336: victim, caster, spell, amount, crit.
    # Regen never emits logs, so attribution is exact.
    ally = 900015
    heal_body = smoke.pack_guid(ally) + smoke.pack_guid(caster) + struct.pack("<II", 331, 40) + bytes([0])
    assert parse_heal_log_body(heal_body) == {
        "target_guid": ally,
        "caster_guid": caster,
        "spell_id": 331,
        "amount": 40,
        "crit": 0,
    }
    assert parse_heal_log_body(b"\x01") is None
    heal_window = [
        (336, heal_body),
        (336, smoke.pack_guid(ally) + smoke.pack_guid(900016) + struct.pack("<II", 331, 999) + bytes([0])),
        (592, dmg_body),
    ]
    assert matching_heal_logs(heal_window, 331, ally, caster)[0]["amount"] == 40
    assert matching_heal_logs(heal_window, 331, ally, 900015) == []
    assert matching_heal_logs(heal_window, 133, ally, caster) == []
    # Tick-band gate: in-band PASS, out-of-band FAIL, GO-only FAIL.
    # Band 40 vs 100 hp: two ticks do not kill, so the 2-tick
    # minimum is enforceable here.
    tick_row = {
        "class": "Warlock",
        "spell_id": 25311,
        "bucket": "hostile_dummy",
        "expected": {
            "kind": "aura_on_target",
            "aura": 25311,
            "tick_band": [40.0, 40.0],
            "tick_period_ms": 3000,
            "tick_duration_ms": 18000,
            "tick_count_expected": 6,
        },
    }

    def tick_observed(amounts, target_aura=True, post_hp=100, logs=None):
        return {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": dummy,
            "target_aura": target_aura,
            "pre_hp": 100,
            "post_hp": post_hp,
            "damage_logs": logs if logs is not None else [{"damage": 40}],
            "tick_amounts": amounts,
            "packets": [306, 590],
        }

    in_band = single_outcome_verdict(tick_row, tick_observed([40, 40]))
    assert in_band["verdict"] == "PASS", in_band
    assert "ticks 2/6" in in_band["evidence"], in_band
    out_of_band = single_outcome_verdict(tick_row, tick_observed([40, 50]))
    assert out_of_band["verdict"] == "FAIL" and out_of_band["phase"] == "apply", out_of_band
    short = single_outcome_verdict(tick_row, tick_observed([40]))
    assert short["verdict"] == "FAIL" and short["phase"] == "apply", short
    # Lethal-pair band (last-rank Drain Soul: 91/tick vs a 100 hp
    # dummy): one in-band tick on a living target passes, because a
    # second tick cannot land on a live dummy by construction.
    drain_row = {
        "class": "Warlock",
        "spell_id": 11675,
        "bucket": "hostile_dummy",
        "expected": {
            "kind": "aura_on_target",
            "aura": 11675,
            "tick_band": [91.0, 91.0],
            "tick_period_ms": 3000,
            "tick_duration_ms": 15000,
            "tick_count_expected": 5,
        },
    }
    lethal_one = single_outcome_verdict(drain_row, tick_observed([91], post_hp=9))
    assert lethal_one["verdict"] == "PASS", lethal_one
    go_only_tick = single_outcome_verdict(
        tick_row,
        {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": dummy,
            "target_aura": False,
            "pre_hp": 100,
            "post_hp": 100,
            "damage_logs": [],
            "tick_amounts": [],
            "packets": [306],
        },
    )
    assert go_only_tick["verdict"] == "FAIL", go_only_tick
    # A target that died before any tick could land leaves the band
    # un-enforced (kill proves the damage); a dead target with an
    # out-of-band tick still FAILs.
    dead_clean = single_outcome_verdict(tick_row, tick_observed([], post_hp=0))
    assert dead_clean["verdict"] == "PASS", dead_clean
    assert "un-enforced" in dead_clean["evidence"], dead_clean
    dead_bad = single_outcome_verdict(tick_row, tick_observed([50], post_hp=0))
    assert dead_bad["verdict"] == "FAIL", dead_bad
    # Paths without a latch ride the base gate and say so.
    no_latch = dict(tick_observed([137, 137]))
    del no_latch["tick_amounts"]
    unlatch = single_outcome_verdict(tick_row, no_latch)
    assert unlatch["verdict"] == "PASS" and "un-enforced" in unlatch["evidence"], unlatch
    # damage_log tick rows gate the same way (Immolate shape).
    immolate_row = {
        "class": "Warlock",
        "spell_id": 25309,
        "bucket": "hostile_dummy",
        "expected": {
            "kind": "damage_log",
            "opcode": 592,
            "tick_band": [102.0, 102.0],
            "tick_period_ms": 3000,
            "tick_duration_ms": 15000,
            "tick_count_expected": 5,
        },
    }
    immolate_ok = single_outcome_verdict(immolate_row, tick_observed([102, 102], post_hp=40, logs=[{"damage": 200}]))
    assert immolate_ok["verdict"] == "PASS", immolate_ok
    immolate_bad = single_outcome_verdict(immolate_row, tick_observed([102, 10], post_hp=40, logs=[{"damage": 200}]))
    assert immolate_bad["verdict"] == "FAIL", immolate_bad
    # Single-tick auras (Curse of Doom: one 3200 tick at 60 s) pass
    # on their one in-band tick while alive; zero ticks still FAILs.
    doom_row = {
        "class": "Warlock",
        "spell_id": 603,
        "bucket": "hostile_dummy",
        "expected": {
            "kind": "aura_on_target",
            "aura": 603,
            "tick_band": [3200.0, 3200.0],
            "tick_period_ms": 60000,
            "tick_duration_ms": 60000,
            "tick_count_expected": 1,
        },
    }
    doom_ok = single_outcome_verdict(doom_row, tick_observed([3200], post_hp=100, logs=[{"damage": 3200}]))
    assert doom_ok["verdict"] == "PASS", doom_ok
    assert "ticks 1/1" in doom_ok["evidence"], doom_ok
    doom_short = single_outcome_verdict(doom_row, tick_observed([], post_hp=100, logs=[{"damage": 3200}]))
    assert doom_short["verdict"] == "FAIL", doom_short
    print("SELFTEST periodic tick latch + band gate ok")
    # An expected-failure row is never PASS. A rejection is the gate's
    # expected FAIL; a landed wrong-form cast is an unexpected FAIL.
    fail_row = {"class": "Druid", "spell_id": 6795, "bucket": "form", "expected": {"kind": "expect_cast_failure"}}
    rejected = expected_failure_verdict(fail_row, False, 86, [304], dummy, attempts=2)
    assert rejected["verdict"] == "FAIL" and rejected["phase"] == "prepare"
    assert rejected["expected_failure"] is True
    assert rejected["target_guid"] == dummy
    landed = expected_failure_verdict(fail_row, True, None, [306], dummy, attempts=1)
    assert landed["verdict"] == "FAIL" and landed["expected_failure"] is False
    out_of_range = expected_failure_verdict(fail_row, False, 89, [304], dummy, attempts=3)
    assert out_of_range["verdict"] == "FAIL"
    assert out_of_range["expected_failure"] is False
    # Dummy reset needs attributed first-kill and second-target damage;
    # the mere existence of another live counter is not evidence.
    first = {"guid": dummy, "pre_hp": 100, "post_hp": 0, "damage_logs": [{"damage": 46}], "aura_seen": False}
    second = {"guid": raw_guid(705, 345), "pre_hp": 100, "post_hp": 54, "damage_logs": [], "aura_seen": True}
    assert reset_evidence_ok(first, second)
    assert not reset_evidence_ok(first, {**second, "post_hp": 100})
    assert not reset_evidence_ok(first, {**second, "guid": dummy})
    assert not reset_evidence_ok({**first, "damage_logs": []}, second)
    # Single-spell repro uses the row's observable outcome; SPELL_GO
    # alone (the shipped run_single rule) cannot produce PASS.
    fireball = {
        "class": "Mage",
        "spell_id": 133,
        "bucket": "hostile_dummy",
        "expected": {"kind": "damage_log", "opcode": 592},
    }
    unobserved = single_outcome_verdict(
        fireball,
        {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": dummy,
            "target_aura": False,
            "pre_hp": 100,
            "post_hp": 54,
            "damage_logs": [],
            "packets": [306],
        },
    )
    assert unobserved["verdict"] == "FAIL", unobserved
    observed = single_outcome_verdict(
        fireball,
        {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": dummy,
            "target_aura": False,
            "pre_hp": 100,
            "post_hp": 54,
            "damage_logs": [{"damage": 46}],
            "packets": [306, 592],
        },
    )
    assert observed["verdict"] == "PASS", observed
    # Pilot gate: only the expected shapeshift FAIL and pinned corpse
    # BLOCKED are compatible; a core-row BLOCKED stops the gate.
    gate_rows = {
        "Warrior": [
            make_verdict({"class": "Warrior", "spell_id": 5242, "bucket": "self"}, "PASS", evidence="ok"),
            make_verdict({"class": "Warrior", "spell_id": 6546, "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
        "Mage": [
            make_verdict({"class": "Mage", "spell_id": 168, "bucket": "self"}, "PASS", evidence="ok"),
            make_verdict({"class": "Mage", "spell_id": 133, "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
        "Druid": [
            make_verdict({"class": "Druid", "spell_id": 5187, "bucket": "friendly"}, "PASS", evidence="ok"),
            rejected,
            make_verdict({"class": "Druid", "spell_id": 5487, "bucket": "form"}, "PASS", evidence="ok"),
            make_verdict({"class": "Druid", "spell_id": 768, "bucket": "form"}, "PASS", evidence="ok"),
            make_verdict(
                {"class": "Druid", "spell_id": 20484, "bucket": "corpse"},
                "BLOCKED",
                reason="wire-target-unsupported",
                evidence="ok",
            ),
            make_verdict({"class": "Druid", "spell_id": 8921, "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
    }
    assert pilot_gate_failures(gate_rows) == []
    gate_rows["Warrior"][1] = make_verdict(
        {"class": "Warrior", "spell_id": 6546, "bucket": "hostile_dummy"},
        "BLOCKED",
        reason="dummy-unavailable",
        evidence="no dummy",
    )
    assert pilot_gate_failures(gate_rows)
    gate_rows["Druid"][1] = landed
    assert any(spell == 6795 for _, spell, _ in pilot_gate_failures(gate_rows))
    # Anti-vacuity: GO alone (or Started alone) is never PASS, for
    # every expected kind including the Paladin fan-out kinds.
    all_kinds = [
        "aura_on_caster",
        "aura_on_target",
        "health_delta_target",
        "heal_delta_target",
        "damage_log",
        "object_created",
        "pet_created",
        "form_change",
        "item_consumed",
        "learned_spell",
        "teleport",
        "dual_outcome",
        "none_marker",
    ]
    for kind in all_kinds:
        kind_row = {"class": "Paladin", "spell_id": 20293, "bucket": "self", "expected": {"kind": kind, "aura": 20293}}
        vacuous = single_outcome_verdict(
            kind_row,
            {
                "go": True,
                "reason": None,
                "caster_aura": False,
                "target_guid": caster,
                "target_aura": False,
                "pre_hp": 100,
                "post_hp": 100,
                "damage_logs": [],
                "pre": "pre",
                "post": "post",
                "packets": [306],
            },
        )
        assert vacuous["verdict"] != "PASS", (kind, vacuous)
    started_only = single_outcome_verdict(
        fireball,
        {
            "go": False,
            "reason": None,
            "caster_aura": False,
            "target_guid": dummy,
            "target_aura": False,
            "pre_hp": 100,
            "post_hp": 100,
            "damage_logs": [],
            "packets": [305],
        },
    )
    assert started_only["verdict"] == "FAIL"
    assert started_only["phase"] == "prepare"
    # Class gate: a PASS row without evidence or without a bound
    # SPELL_GO (306) in its packets stops the class gate.
    gate_ok = [make_verdict(row, "PASS", evidence="bound GO + aura", packets=[306, 592], target_guid=dummy)]
    assert class_gate_failures({"Paladin": gate_ok}, {"Paladin": 1}) == []
    gate_no_evidence = [make_verdict(row, "PASS", packets=[306], target_guid=dummy)]
    assert class_gate_failures({"Paladin": gate_no_evidence}, {"Paladin": 1})
    gate_no_go = [make_verdict(row, "PASS", evidence="stale claim", packets=[592], target_guid=dummy)]
    assert class_gate_failures({"Paladin": gate_no_go}, {"Paladin": 1})
    gate_count = [make_verdict(row, "PASS", evidence="ok", packets=[306], target_guid=dummy)]
    assert class_gate_failures({"Paladin": gate_count}, {"Paladin": 2})
    # Documented blockers stay gate-compatible: engine-totem rows
    # (Sentry 6495) are left uncast with live failing-cast evidence
    # until the totem engine slice lands.
    gate_totem = [make_verdict(row, "BLOCKED", reason="engine-totem", evidence="no live scenario")]
    assert class_gate_failures({"Shaman": gate_totem}, {"Shaman": 1}) == []
    gate_scenario = [make_verdict(row, "BLOCKED", reason="dummy-unavailable", evidence="x")]
    assert class_gate_failures({"Shaman": gate_scenario}, {"Shaman": 1})
    print("SELFTEST anti-vacuity + class gate ok")
    # Per-class matrix paths: pilot classes stay on the pilot matrix;
    # fan-out classes resolve to their own artifact (never MATRIX_PATH).
    assert matrix_path_for("Warrior") == MATRIX_PATH
    assert matrix_path_for("Mage") == MATRIX_PATH
    assert matrix_path_for("Druid") == MATRIX_PATH
    paladin_path = matrix_path_for("Paladin")
    assert paladin_path != MATRIX_PATH
    assert paladin_path.name == "spell_campaign_matrix-Paladin.json"
    # Generic per-row dispatch: every Paladin bucket maps to a cast leg
    # or to an honest closed-enum BLOCKED reason; nothing unmapped can
    # reach a cast sender.
    dispatch_table = [
        (
            {
                "class": "Paladin",
                "spell_id": 10293,
                "bucket": "self",
                "expected": {"kind": "aura_on_caster", "aura": 10293},
            },
            ("cast", "self"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 1038,
                "bucket": "party_duel",
                "expected": {"kind": "aura_on_target", "aura": 1038},
            },
            ("blocked", "no-party"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 1044,
                "bucket": "friendly",
                "expected": {"kind": "aura_on_target", "aura": 1044},
            },
            ("cast", "friendly"),
        ),
        (
            {"class": "Paladin", "spell_id": 10310, "bucket": "friendly", "expected": {"kind": "heal_delta_target"}},
            ("cast", "friendly"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 10308,
                "bucket": "hostile_dummy",
                "expected": {"kind": "aura_on_target", "aura": 10308},
            },
            ("cast", "hostile"),
        ),
        (
            {"class": "Paladin", "spell_id": 20924, "bucket": "ground_area", "expected": {"kind": "object_created"}},
            ("cast", "ground"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 19752,
                "bucket": "item_reagent_ammo",
                "expected": {"kind": "item_consumed", "reagent": 21177},
            },
            ("cast", "item"),
        ),
        (
            {"class": "Paladin", "spell_id": 20048, "bucket": "passive", "expected": {"kind": "learned_spell"}},
            ("passive", None),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 1152,
                "bucket": "friendly",
                "campaign": "classify_only",
                "expected": {"kind": "none_marker", "note": "dispel"},
            },
            ("blocked", "effect-marker-only"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 20271,
                "bucket": "hostile_dummy",
                "campaign": "classify_only_script",
                "expected": {"kind": "damage_log", "opcode": 592},
            },
            ("blocked", "script-evidence-unknown"),
        ),
        (
            {
                "class": "Paladin",
                "spell_id": 20930,
                "bucket": "dual",
                "campaign": "classify_only",
                "expected": {"kind": "dual_outcome"},
            },
            ("blocked", "script-evidence-unknown"),
        ),
        (
            {"class": "Paladin", "spell_id": 20930, "bucket": "dual", "expected": {"kind": "dual_outcome"}},
            ("blocked", "script-evidence-unknown"),
        ),
        (
            {"class": "Paladin", "spell_id": 0, "bucket": "corpse", "expected": {"kind": "resurrect_request"}},
            ("cast", "resurrect"),
        ),
        (
            {
                "class": "Priest",
                "spell_id": 2096,
                "bucket": "self",
                "target_a": [25, 1, 25],
                "expected": {"kind": "aura_on_caster", "aura": 2096},
            },
            ("cast", "hostile"),
        ),
        (
            {"class": "Paladin", "spell_id": 0, "bucket": "pet", "expected": {"kind": "none_marker"}},
            ("blocked", "no-pet"),
        ),
        (
            {"class": "Paladin", "spell_id": 0, "bucket": "form_totem", "expected": {"kind": "none_marker"}},
            ("blocked", "engine-totem"),
        ),
    ]
    for synth_row, want in dispatch_table:
        got = pre_dispatch(synth_row)
        assert got == want, (synth_row, got, want)
        if got[0] == "blocked":
            assert got[1] in BLOCKED_REASONS, got
    for reason in UNSCENARIOED_EVIDENCE:
        assert reason in BLOCKED_REASONS, reason
    print("SELFTEST per-class dispatch table ok")
    # Druid fan-out dispatch: form rows ride the form leg; stance-gated
    # non-forms ride the form_gated leg (enter form, cast, exit);
    # actual totem rows stay engine-totem; corpse rows (Rebirth)
    # ride the resurrect leg and never reach the item leg; taught
    # caster-form rows ride the proven
    # friendly/hostile/ground/item legs; form passives ride the
    # learned_spell check.
    assert required_form_spell({"stances": 144, "totem": [0, 0]}) == 9634
    assert required_form_spell({"stances": 16, "totem": [0, 0]}) == 5487
    assert required_form_spell({"stances": 1, "totem": [0, 0]}) == 768
    assert required_form_spell({"stances": 1 << 30, "totem": [0, 0]}) == 24858
    assert required_form_spell({"stances": 144, "totem": [5174, 0]}) is None
    assert required_form_spell({"stances": 0, "totem": [0, 0]}) is None
    # Warrior stance bits resolve after the Druid table: singleton
    # bits to their stance, dual-stance rows preferring Battle, and
    # totem rows staying unenterable even with stance bits.
    assert required_form_spell({"stances": 65536, "totem": [0, 0]}) == 2457
    assert required_form_spell({"stances": 131072, "totem": [0, 0]}) == 71
    assert required_form_spell({"stances": 262144, "totem": [0, 0]}) == 2458
    assert required_form_spell({"stances": 196608, "totem": [0, 0]}) == 2457
    assert required_form_spell({"stances": 327680, "totem": [0, 0]}) == 2457
    assert required_form_spell({"stances": 65536, "totem": [5174, 0]}) is None
    # Druid bits still win when both families are set (disjoint in
    # practice; the order pins Druid behavior unchanged).
    assert required_form_spell({"stances": 144 | 65536, "totem": [0, 0]}) == 9634
    druid_dispatch = [
        ({"class": "Druid", "spell_id": 9634, "bucket": "form", "expected": {"kind": "form_change"}}, ("cast", "form")),
        (
            {
                "class": "Druid",
                "spell_id": 6795,
                "bucket": "form_totem",
                "stances": 144,
                "totem": [0, 0],
                "expected": {"kind": "aura_on_target", "aura": 6795},
            },
            ("cast", "form_gated"),
        ),
        (
            {
                "class": "Shaman",
                "spell_id": 0,
                "bucket": "form_totem",
                "stances": 0,
                "totem": [5174, 0],
                "expected": {"kind": "object_created"},
            },
            ("blocked", "engine-totem"),
        ),
        (
            {"class": "Druid", "spell_id": 20748, "bucket": "corpse", "expected": {"kind": "resurrect_request"}},
            ("cast", "resurrect"),
        ),
        (
            {"class": "Druid", "spell_id": 25297, "bucket": "friendly", "expected": {"kind": "heal_delta_target"}},
            ("cast", "friendly"),
        ),
        (
            {
                "class": "Druid",
                "spell_id": 25299,
                "bucket": "friendly",
                "expected": {"kind": "aura_on_target", "aura": 25299},
            },
            ("cast", "friendly"),
        ),
        (
            {
                "class": "Druid",
                "spell_id": 9835,
                "bucket": "hostile_dummy",
                "expected": {"kind": "damage_log", "opcode": 592},
            },
            ("cast", "hostile"),
        ),
        (
            {"class": "Druid", "spell_id": 18960, "bucket": "ground_area", "expected": {"kind": "teleport"}},
            ("cast", "ground"),
        ),
        (
            {
                "class": "Druid",
                "spell_id": 21850,
                "bucket": "item_reagent_ammo",
                "expected": {"kind": "item_consumed", "reagent": 17026},
            },
            ("cast", "item"),
        ),
        (
            {
                "class": "Druid",
                "spell_id": 22812,
                "bucket": "self",
                "expected": {"kind": "aura_on_caster", "aura": 22812},
            },
            ("cast", "self"),
        ),
        (
            {"class": "Druid", "spell_id": 3025, "bucket": "passive", "expected": {"kind": "learned_spell"}},
            ("passive", None),
        ),
    ]
    for synth_row, want in druid_dispatch:
        got = pre_dispatch(synth_row)
        assert got == want, (synth_row, got, want)
        if got[0] == "blocked":
            assert got[1] in BLOCKED_REASONS, got
    print("SELFTEST Druid dispatch table ok")
    # Warrior dispatch: stance entries ride the form leg;
    # stance-gated rows ride form_gated; movement/script-dummy
    # rows stay honestly BLOCKED; the focus-cost Thunderstomp
    # rows are marker-only (the classify_only gate fires first);
    # reagent-free weapon rows verify on the dummy.
    warrior_dispatch = [
        (
            {"class": "Warrior", "spell_id": 2457, "bucket": "form", "expected": {"kind": "form_change"}},
            ("cast", "form"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 772,
                "bucket": "form_totem",
                "stances": 196608,
                "totem": [0, 0],
                "expected": {"kind": "aura_on_target", "aura": 772},
            },
            ("cast", "form_gated"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 871,
                "bucket": "form_totem",
                "stances": 131072,
                "totem": [0, 0],
                "expected": {"kind": "aura_on_caster", "aura": 871},
            },
            ("cast", "form_gated"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 5308,
                "bucket": "form_totem",
                "expected": {"kind": "none_marker"},
                "campaign": "classify_only",
            },
            ("blocked", "effect-marker-only"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 100,
                "bucket": "form_totem",
                "expected": {"kind": "none_marker"},
                "campaign": "classify_only",
            },
            ("blocked", "effect-marker-only"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 26090,
                "bucket": "ground_area",
                "power_type": 2,
                "mana_cost": 60,
                "expected": {"kind": "none_marker"},
                "campaign": "classify_only",
            },
            ("blocked", "effect-marker-only"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 23923,
                "bucket": "item_reagent_ammo",
                "target_a": [6, 6, 0],
                "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
                "equipped_item_class": 4,
                "expected": {"kind": "damage_log", "opcode": 592},
            },
            ("cast", "hostile"),
        ),
        (
            {
                "class": "Warrior",
                "spell_id": 20647,
                "bucket": "hostile_dummy",
                "power_type": 1,
                "mana_cost": 1,
                "expected": {"kind": "damage_log", "opcode": 592},
            },
            ("cast", "hostile"),
        ),
        # Mixed unit-target + areaselect ground rows ride hostile:
        # the dest-only ground leg sends no unit target (Intimidating
        # Shout 5246 honestly answers BAD_IMPLICIT_TARGETS there).
        (
            {
                "class": "Warrior",
                "spell_id": 5246,
                "bucket": "ground_area",
                "power_type": 1,
                "mana_cost": 250,
                "target_a": [6, 22, 22],
                "expected": {"kind": "aura_on_target", "aura": 7},
            },
            ("cast", "hostile"),
        ),
        # Pure-areaselect ground rows keep the dest leg.
        (
            {
                "class": "Warrior",
                "spell_id": 1160,
                "bucket": "ground_area",
                "power_type": 1,
                "mana_cost": 100,
                "target_a": [22, 0, 0],
                "expected": {"kind": "aura_on_target", "aura": 99},
            },
            ("cast", "ground"),
        ),
        # Self-targeted heals ride the relog self-heal leg: the solo
        # self leg has no damaged-caster scenario (Bloodthirst 23880).
        (
            {
                "class": "Warrior",
                "spell_id": 23880,
                "bucket": "self",
                "power_type": 1,
                "mana_cost": 0,
                "target_a": [1, 0, 0],
                "expected": {"kind": "heal_delta_target"},
            },
            ("cast", "self_heal"),
        ),
    ]
    for synth_row, want in warrior_dispatch:
        got = pre_dispatch(synth_row)
        assert got == want, (synth_row, got, want)
        if got[0] == "blocked":
            assert got[1] in BLOCKED_REASONS, got
    # Rage-setup routing: rage-cost Warrior rows take the
    # Bloodrage setup everywhere; Bloodrage itself, non-rage
    # rows, and other classes never do.
    assert warrior_rage_setup_for_row("Warrior", {"spell_id": 772, "power_type": 1, "mana_cost": 100}) is not None
    assert warrior_rage_setup_for_row("Warrior", {"spell_id": 20647, "power_type": 1, "mana_cost": 1}) is not None
    assert warrior_rage_setup_for_row("Warrior", {"spell_id": 2687, "power_type": 0, "mana_cost": 0}) is None
    assert warrior_rage_setup_for_row("Warrior", {"spell_id": 871, "power_type": 1, "mana_cost": 0}) is None
    assert warrior_rage_setup_for_row("Warrior", {"spell_id": 2458, "power_type": 1, "mana_cost": 0}) is None
    assert warrior_rage_setup_for_row("Druid", {"spell_id": 772, "power_type": 1, "mana_cost": 100}) is None
    assert warrior_rage_setup_for_row("Paladin", {"spell_id": 20154, "power_type": 0, "mana_cost": 60}) is None

    # Low-health setup: Rend is stance-gated, so Battle Stance and a
    # Rend-sized rage top-up must both land before the predamage cast;
    # a stance miss must stop before Rend instead of riding a doomed
    # full-health Execute.
    class _LowHealthDriver:
        own_guid = 900015

        def __init__(self, calls):
            self.calls = calls
            self.polls = []

        def approach_guid(self, dummy, standoff):
            self.calls.append(("approach", dummy, standoff))

        def poll(self, seconds):
            self.polls.append(seconds)

    def _run_low_health(stance_ok):
        calls = []
        originals = {
            name: globals()[name]
            for name in (
                "ensure_battle_stance",
                "warrior_rage_setup",
                "cast_unit_faced",
                "dummy_world_state",
                "aura_present",
            )
        }
        states = iter([(100, 100, False), (85, 100, True), (85, 100, False), (19, 100, True)])
        aura_states = iter([False, True, False])

        def fake_ensure(driver, guid):
            calls.append(("stance", guid))
            return stance_ok, None if stance_ok else 86, []

        def fake_rage_factory(cost):
            def fake_rage(driver, dummy=None):
                calls.append(("rage", cost, dummy))
                return "; fake rage"

            return fake_rage

        def fake_cast(driver, spell_id, dummy, timeout=0.0):
            calls.append(("rend", spell_id, dummy))
            return True, None, []

        def fake_state(driver, dummy):
            calls.append(("state", dummy))
            return next(states, (19, 100, True))

        def fake_aura(driver, guid, spell_id):
            calls.append(("aura", guid, spell_id))
            return next(aura_states, True)

        globals()["ensure_battle_stance"] = fake_ensure
        globals()["warrior_rage_setup"] = fake_rage_factory
        globals()["cast_unit_faced"] = fake_cast
        globals()["dummy_world_state"] = fake_state
        globals()["aura_present"] = fake_aura
        try:
            note = low_health_setup(_LowHealthDriver(calls), 4242)
        finally:
            globals().update(originals)
        return calls, note

    low_calls, low_note = _run_low_health(True)
    assert low_calls[0] == ("stance", 900015), low_calls
    assert [call for call in low_calls if call[0] == "rage"] == [("rage", 100, 4242)] * 2, low_calls
    assert [call for call in low_calls if call[0] == "approach"] == [("approach", 4242, 2.0)] * 2, low_calls
    assert [call for call in low_calls if call[0] == "rend"] == [("rend", 6546, 4242)] * 2, low_calls
    assert "casts=2" in low_note and "hp 19/100" in low_note, low_note
    low_calls, low_note = _run_low_health(False)
    assert low_calls == [("stance", 900015)], low_calls
    assert "stance FAILED" in low_note and "reason=86" in low_note, low_note
    # Battle Stance verification rides the aura-wait helper: a bound
    # GO whose aura lands after the first read still verifies.
    stance_calls = []
    stance_originals = {name: globals()[name] for name in ("aura_present", "send_and_wait", "wait_for_aura")}

    def fake_aura_absent(driver, guid, spell_id):
        stance_calls.append(("aura", guid, spell_id))
        return False

    def fake_stance_cast(driver, body, spell_id, target_guid=None):
        stance_calls.append(("cast", spell_id, target_guid))
        return True, None, [306]

    def fake_wait_aura(driver, guid, spell_id, timeout_s=8.0):
        stance_calls.append(("wait", guid, spell_id))
        return True

    globals()["aura_present"] = fake_aura_absent
    globals()["send_and_wait"] = fake_stance_cast
    globals()["wait_for_aura"] = fake_wait_aura
    try:
        stance_result = ensure_battle_stance(_LowHealthDriver([]), 900015)
    finally:
        globals().update(stance_originals)
    assert stance_result == (True, None, [306]), stance_result
    assert stance_calls == [("aura", 900015, 2457), ("cast", 2457, 900015), ("wait", 900015, 2457)], stance_calls
    # Rage-setup combat tap: Rend/Hamstring are stance-gated, so a
    # stanceless row's tap never lands and every Bloodrage grant
    # decays through the cooldown wait (live 4-row NO_POWER
    # cluster); a damage-free shout aura lands but never aggros
    # the dummy (live probe: rage decayed 680->0, caster never
    # hit). The tap therefore enters Battle Stance when
    # stanceless, then Rends: damaging, stance-legal, engaging.
    from types import SimpleNamespace as _TapNS

    tap_calls = []
    tap_obj = _TapNS(values={WARRIOR_RAGE_FIELD: 0})
    tap_driver = _TapNS(
        own_guid=900015,
        world=_TapNS(get=lambda guid: tap_obj),
        poll=lambda seconds: None,
        approach_guid=lambda dummy, standoff: tap_calls.append(("approach", dummy, standoff)),
    )
    tap_originals = {
        name: globals()[name]
        for name in (
            "cast_self_retry_power",
            "cast_unit_faced",
            "send_and_wait",
            "live_cooldown_wait_s",
            "wait_for_aura",
            "ensure_battle_stance",
        )
    }
    bloodrage_results = iter([(True, None, [], 0), (False, SPELL_FAILED_NOT_READY, [], 0)])

    def fake_bloodrage(driver, spell_id, timeout=10.0, retries=POWER_RETRIES):
        tap_calls.append(("bloodrage", spell_id))
        result = next(bloodrage_results, (False, SPELL_FAILED_NOT_READY, [], 0))
        if result[0]:
            tap_obj.values[WARRIOR_RAGE_FIELD] = 100
        return result

    def fake_unit_cast(driver, spell_id, dummy, timeout=0.0):
        tap_calls.append(("unit", spell_id, dummy))
        tap_obj.values[WARRIOR_RAGE_FIELD] = 260
        return True, None, []

    def fake_tap_cast(driver, body, spell_id, timeout=10.0, target_guid=None):
        tap_calls.append(("tap", spell_id, body))
        return True, None, [306]

    def fake_tap_stance(driver, guid):
        tap_calls.append(("stance", guid))
        return True, None, []

    def fake_tap_aura(driver, guid, spell_id, timeout_s=8.0):
        tap_calls.append(("wait-aura", guid, spell_id))
        return True

    globals()["cast_self_retry_power"] = fake_bloodrage
    globals()["cast_unit_faced"] = fake_unit_cast
    globals()["send_and_wait"] = fake_tap_cast
    globals()["live_cooldown_wait_s"] = lambda spell_id, cap_s=65.0: 65.0
    globals()["wait_for_aura"] = fake_tap_aura
    globals()["ensure_battle_stance"] = fake_tap_stance
    try:
        tap_note = warrior_rage_setup(200)(tap_driver, 4242)
    finally:
        globals().update(tap_originals)
    assert tap_calls == [
        ("bloodrage", WARRIOR_RAGE_SPELL),
        ("bloodrage", WARRIOR_RAGE_SPELL),
        ("approach", 4242, 2.0),
        ("stance", 900015),
        ("unit", 6546, 4242),
        ("wait-aura", 4242, 6546),
    ], tap_calls
    assert "rage 260/200" in tap_note and "tap-engaged" in tap_note, tap_note
    print("SELFTEST Warrior dispatch table ok")
    # Pet dispatch: the summon row itself always rides the pet leg;
    # other pet rows ride it only when a summon scenario exists for
    # the class, else they stay honestly BLOCKED no-pet.
    summon_row = {
        "class": "Warlock",
        "spell_id": 688,
        "bucket": "pet",
        "effect_ids": [56, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "expected": {"kind": "pet_created"},
    }
    funnel_row = {
        "class": "Warlock",
        "spell_id": 11695,
        "bucket": "pet",
        "effect_ids": [6, 6, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "expected": {"kind": "aura_on_target", "aura": 11695},
    }
    assert pre_dispatch(summon_row) == ("cast", "pet")
    assert pre_dispatch(summon_row, 688) == ("cast", "pet")
    assert pre_dispatch(funnel_row) == ("blocked", "no-pet")
    assert pre_dispatch(funnel_row, 688) == ("cast", "pet")
    assert pre_dispatch(funnel_row, None) == ("blocked", "no-pet")
    scripted_pet = dict(funnel_row, campaign="classify_only_script")
    assert pre_dispatch(scripted_pet, 688) == ("blocked", "script-evidence-unknown")
    # TARGET_UNIT_CASTER_MASTER rows (Sacrifice) are cast BY the pet:
    # they dispatch to the pet-cast leg, never the on-pet leg, and
    # stay no-pet BLOCKED without a summon scenario.
    sacrifice_row = {
        "class": "Warlock",
        "spell_id": 7812,
        "bucket": "pet",
        "effect_ids": [6, 1, 0],
        "target_a": [27, 1, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "expected": {"kind": "aura_on_caster", "aura": 7812},
    }
    assert pre_dispatch(sacrifice_row, 688) == ("cast", "pet_cast")
    assert pre_dispatch(sacrifice_row) == ("blocked", "no-pet")
    # Summon-spell selection: lowest reagent-free pet_created summon
    # wins; reagent-gated or classify_only summons never create the
    # scenario (their fixture-reagent BLOCKED is the honest path).
    assert class_summon_spell([summon_row, funnel_row]) == 688
    assert class_summon_spell([funnel_row]) is None
    assert class_summon_spell([]) is None
    reagent_summon = dict(
        summon_row,
        spell_id=691,
        bucket="item_reagent_ammo",
        reagents=[6265, 0, 0, 0, 0, 0, 0, 0],
        expected={"kind": "item_consumed", "reagent": 6265},
    )
    assert class_summon_spell([reagent_summon, funnel_row]) is None
    assert class_summon_spell([dict(summon_row, campaign="classify_only")]) is None
    assert class_summon_spell([dict(summon_row, spell_id=712), summon_row]) == 688
    # Pet discovery: nearest new alive unit near the caster; stale,
    # dead, and far objects never verify a summon.
    # Stale (10), far (11), and dead (12) objects: nothing new and
    # alive near the caster, so no pet.
    cands = [(10, 0.0, 0.0, 100, 416), (11, 100.0, 100.0, 100, 416), (12, 1.0, 1.0, 0, 416)]
    assert find_summoned_pet({10}, cands, (0.0, 0.0)) is None
    assert find_summoned_pet({10}, [(10, 0.0, 0.0, 100, 416)], (0.0, 0.0)) is None
    assert find_summoned_pet(set(), [(12, 1.0, 1.0, 0, 416)], (0.0, 0.0)) is None
    assert find_summoned_pet(set(), [(11, 100.0, 100.0, 100, 416)], (0.0, 0.0)) is None
    assert find_summoned_pet(set(), [(20, 5.0, 0.0, 100, 416), (21, 2.0, 0.0, 80, 416)], (0.0, 0.0)) == 21
    assert find_summoned_pet({20, 21}, [(20, 5.0, 0.0, 100, 416), (21, 2.0, 0.0, 80, 416)], (0.0, 0.0)) is None
    # Already-out pet discovery: the login-autoloaded fixture pet
    # (right entry, campaign health, near caster) reuses; wild
    # same-entry wolves, 100-hp dummies, wrong entries, far and
    # position-less objects never qualify.
    from types import SimpleNamespace as _NS

    _objs = {
        1: _NS(entry=69, health=3052, position=(1.0, 1.0, 0.0, 0.0)),
        2: _NS(entry=69, health=300, position=(0.5, 0.5, 0.0, 0.0)),
        3: _NS(entry=705, health=3052, position=(0.5, 0.5, 0.0, 0.0)),
        4: _NS(entry=69, health=3052, position=(100.0, 100.0, 0.0, 0.0)),
        5: _NS(entry=69, health=0, position=(1.0, 1.0, 0.0, 0.0)),
        6: _NS(entry=69, health=3052, position=None),
    }
    assert find_active_pet(_objs, (0.0, 0.0), 69) == 1
    assert find_active_pet({2: _objs[2]}, (0.0, 0.0), 69) is None
    assert find_active_pet({3: _objs[3]}, (0.0, 0.0), 69) is None
    assert find_active_pet({4: _objs[4]}, (0.0, 0.0), 69) is None
    assert find_active_pet({5: _objs[5], 6: _objs[6]}, (0.0, 0.0), 69) is None
    assert find_active_pet(_objs, (0.0, 0.0), 416) is None
    # Full-cooldown wait: category dominates (Wyvern 0/120000 ->
    # 125 s), recovery dominates when larger, the cap binds long
    # rows, and garbage falls back to the margin.
    assert cooldown_wait_s(0, 120000) == 125.0
    assert cooldown_wait_s(20000, 0) == 25.0
    assert cooldown_wait_s(3600000, 3600000) == 600.0
    assert cooldown_wait_s("x", None) == 5.0
    print("SELFTEST pet dispatch + summon selection + discovery ok")
    # GO-only anti-vacuity for the form_change mapping: a bound GO
    # without the form aura on the caster is an apply FAIL, while the
    # same GO with the aura observed is PASS.
    form_row = {"class": "Druid", "spell_id": 9634, "bucket": "form", "expected": {"kind": "form_change"}}
    go_alone_form = single_outcome_verdict(
        form_row,
        {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": caster,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": "pre",
            "post": "post",
            "packets": [306],
        },
    )
    assert go_alone_form["verdict"] == "FAIL", go_alone_form
    assert go_alone_form["phase"] == "apply", go_alone_form
    go_observed_form = single_outcome_verdict(
        form_row,
        {
            "go": True,
            "reason": None,
            "caster_aura": True,
            "target_guid": caster,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": "pre",
            "post": "post",
            "packets": [306],
        },
    )
    assert go_observed_form["verdict"] == "PASS", go_observed_form
    print("SELFTEST GO-alone cannot PASS form row ok")
    # SPELL_GO alone cannot PASS a generic row: a bound GO with no
    # observed outcome is an apply-phase FAIL, while the same GO with
    # the aura observed is PASS. The discriminator is the observable,
    # never the GO.
    generic_row = {
        "class": "Paladin",
        "spell_id": 10293,
        "bucket": "self",
        "expected": {"kind": "aura_on_caster", "aura": 10293},
    }
    go_alone = single_outcome_verdict(
        generic_row,
        {
            "go": True,
            "reason": None,
            "caster_aura": False,
            "target_guid": caster,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": "pre",
            "post": "post",
            "packets": [306],
        },
    )
    assert go_alone["verdict"] == "FAIL", go_alone
    assert go_alone["phase"] == "apply", go_alone
    go_observed = single_outcome_verdict(
        generic_row,
        {
            "go": True,
            "reason": None,
            "caster_aura": True,
            "target_guid": caster,
            "target_aura": False,
            "pre_hp": None,
            "post_hp": None,
            "damage_logs": [],
            "pre": "pre",
            "post": "post",
            "packets": [306],
        },
    )
    assert go_observed["verdict"] == "PASS", go_observed
    print("SELFTEST GO-alone cannot PASS generic row ok")
    # Settlable-seed castability (the live 145 regression): every
    # castable row's spell is in the artifact seed, and the seed
    # talent cost settles at or under the level-60 cap, else fork
    # login wipes the talent book and every talent cast fails 145.
    if paladin_path.exists():
        paladin_seed_doc = json.loads(paladin_path.read_text())
        paladin_seed = set(paladin_seed_doc.get("full_seed_spell_ids", {}).get("Paladin", []))
        assert paladin_seed, "Paladin artifact seed empty"
        assert paladin_seed_doc.get("full_seed_talent_cost", 99) <= 51, paladin_seed_doc.get("full_seed_talent_cost")
        castable = [
            r
            for r in paladin_seed_doc.get("rows", [])
            if r.get("class") == "Paladin"
            and not (r.get("campaign", "") or "").startswith("classify_only")
            and (r.get("expected") or {}).get("kind") not in ("none_marker", "dual_outcome", "learned_spell")
        ]
        assert castable, "no castable Paladin rows"
        for cast_row in castable:
            assert cast_row["spell_id"] in paladin_seed, cast_row["spell_id"]
            assert isinstance(cast_row.get("cooldown_ms"), int), cast_row
        print(
            f"SELFTEST Paladin seed castability ok "
            f"({len(castable)} cast rows seeded, "
            f"cost={paladin_seed_doc.get('full_seed_talent_cost')})"
        )
    else:
        print("SELFTEST Paladin artifact absent; skipping castability")
    # Pure heal-rise gate: regen-baselined rise over one discrete
    # tick; a full ally (jump 0) is a scenario FAIL, never PASS.
    assert heal_rise_ok(2609.0, 2609.0, 2609.0, 0.0, 3.0, 7.0)[1] is False
    rise, ok = heal_rise_ok(300.0, 310.0, 900.0, 0.0, 3.0, 7.0)
    assert ok and rise > 50, (rise, ok)
    rise2, ok2 = heal_rise_ok(300.0, 300.0, 340.0, 0.0, 3.0, 7.0)
    assert not ok2 and rise2 <= 50, (rise2, ok2)
    # Clean-dummy reuse predicate: only alive, full-health, aura-free
    # dummies are reusable; DoT/CC/dead holders never are.
    assert dummy_reusable(100, 100, False) is True
    assert dummy_reusable(100, 100, True) is False
    assert dummy_reusable(54, 100, False) is False
    assert dummy_reusable(0, 100, False) is False
    assert dummy_reusable(None, 100, False) is False
    assert dummy_reusable(100, None, False) is False
    # Passive learned rule: passive spells have no aura holder by
    # design, so learned alone verifies; non-passive rows still need
    # the aura.
    assert passive_learn_verdict(True, True, False) == "PASSIVE-VERIFIED"
    assert passive_learn_verdict(True, False, False) == "PASSIVE-UNVERIFIED"
    assert passive_learn_verdict(False, True, True) == "PASSIVE-VERIFIED"
    assert passive_learn_verdict(False, True, False) == "PASSIVE-UNVERIFIED"
    print("SELFTEST seed castability + heal + dummy + passive ok")
    # Totality over the real per-class artifact when present: every
    # row dispatches to a known cast leg, the passive check, or a
    # closed-enum BLOCKED reason.
    if paladin_path.exists():
        paladin_doc = json.loads(paladin_path.read_text())
        paladin_rows = [r for r in paladin_doc.get("rows", []) if r.get("class") == "Paladin"]
        assert paladin_rows, "Paladin artifact has no Paladin rows"
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in paladin_rows:
            act, pay = pre_dispatch(real_row)
            if act == "cast":
                assert pay in ("self", "friendly", "hostile", "ground", "item"), (real_row, pay)
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        assert {"self", "friendly", "hostile", "ground", "item"} <= seen_legs, seen_legs
        assert "script-evidence-unknown" in seen_blocked, seen_blocked
        assert "effect-marker-only" in seen_blocked, seen_blocked
        # Single raid-target blessings ride the friendly leg
        # now; no Paladin row needs a party scenario.
        assert "no-party" not in seen_blocked, seen_blocked
        print(
            f"SELFTEST Paladin artifact dispatch totality ok "
            f"({len(paladin_rows)} rows, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Paladin artifact absent; skipping totality")
    # Totality over the real Druid fan-out artifact when present:
    # every row dispatches to a cast leg, the passive check, or a
    # closed-enum BLOCKED reason; the form leg is exercised and the
    # stance/corpse BLOCKED reasons are present.
    druid_path = matrix_path_for("Druid")
    try:
        import spell_matrix_gen as gen  # noqa: PLC0415

        druid_path = gen.class_matrix_path("Druid")
    except (ImportError, AttributeError):
        pass
    if druid_path.exists():
        druid_doc = json.loads(druid_path.read_text())
        druid_rows = [r for r in druid_doc.get("rows", []) if r.get("class") == "Druid"]
        assert druid_rows, "Druid artifact has no Druid rows"
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in druid_rows:
            act, pay = pre_dispatch(real_row)
            if act == "cast":
                assert pay in ("self", "friendly", "hostile", "ground", "item", "form", "form_gated", "resurrect"), (
                    real_row,
                    pay,
                )
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        assert {"self", "friendly", "hostile", "ground", "item", "form", "form_gated", "resurrect"} <= seen_legs, (
            seen_legs
        )
        # Druid has no totems: stance-gated rows ride form_gated, so
        # engine-totem must not appear for this class at all.
        assert "engine-totem" not in seen_blocked, seen_blocked
        assert "wire-target-unsupported" not in seen_blocked, seen_blocked
        assert "effect-marker-only" in seen_blocked, seen_blocked
        assert not [r for r in druid_rows if (r.get("expected") or {}).get("kind") == "expect_cast_failure"], (
            "full Druid matrix must not invent expect_cast_failure rows"
        )
        print(
            f"SELFTEST Druid artifact dispatch totality ok "
            f"({len(druid_rows)} rows, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Druid artifact absent; skipping totality")
    # Totality over the Warlock fan-out artifact: the summon scenario
    # exists (reagent-free Summon Imp), so pet rows dispatch to the
    # pet leg and no-pet never appears; every other row dispatches to
    # a cast leg, the passive check, or a closed-enum BLOCKED reason.
    warlock_path = matrix_path_for("Warlock")
    try:
        import spell_matrix_gen as gen  # noqa: PLC0415

        warlock_path = gen.class_matrix_path("Warlock")
    except (ImportError, AttributeError):
        pass
    if warlock_path.exists():
        warlock_doc = json.loads(warlock_path.read_text())
        warlock_rows = [r for r in warlock_doc.get("rows", []) if r.get("class") == "Warlock"]
        assert warlock_rows, "Warlock artifact has no Warlock rows"
        summon = class_summon_spell(warlock_rows)
        assert summon == 688, summon
        assert warlock_doc.get("full_talent_build") == "smruin", warlock_doc.get("full_talent_build")
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in warlock_rows:
            act, pay = pre_dispatch(real_row, summon)
            if act == "cast":
                assert pay in ("self", "friendly", "hostile", "ground", "item", "pet", "pet_cast"), (
                    real_row,
                    pay,
                )
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        assert "pet" in seen_legs, seen_legs
        assert "no-pet" not in seen_blocked, seen_blocked
        assert "hostile" in seen_legs and "self" in seen_legs, seen_legs
        # Reagent-gated rows ride the item leg (their fixture-reagent
        # BLOCKED is a runtime verdict, not a dispatch reason).
        assert "item" in seen_legs, seen_legs
        assert "effect-marker-only" in seen_blocked, seen_blocked
        summon_rows = [r for r in warlock_rows if int(r["spell_id"]) == summon]
        assert len(summon_rows) == 1 and summon_rows[0]["expected"]["kind"] == "pet_created", summon_rows
        print(
            f"SELFTEST Warlock artifact dispatch totality ok "
            f"({len(warlock_rows)} rows, summon={summon}, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Warlock artifact absent; skipping totality")
    # Hunter fan-out dispatch: reagent-free weapon rows verify on
    # the dummy (unit targets ride hostile, area-only Volley rides
    # ground); bare trap placements and Revive Pet stay BLOCKED;
    # pet rows ride the pet leg only with the Call Pet fixture
    # summon (883), else no-pet.
    aimed_row = {
        "class": "Hunter",
        "spell_id": 19434,
        "bucket": "item_reagent_ammo",
        "effect_ids": [121, 0, 0],
        "target_a": [6, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "equipped_item_class": 2,
        "expected": {"kind": "damage_log", "opcode": 592},
    }
    assert pre_dispatch(aimed_row) == ("cast", "hostile")
    sting_row = {
        "class": "Hunter",
        "spell_id": 1978,
        "bucket": "item_reagent_ammo",
        "effect_ids": [6, 0, 0],
        "target_a": [6, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "equipped_item_class": 2,
        "expected": {"kind": "aura_on_target", "aura": 1978},
    }
    assert pre_dispatch(sting_row) == ("cast", "hostile")
    volley_row = {
        "class": "Hunter",
        "spell_id": 1510,
        "bucket": "item_reagent_ammo",
        "effect_ids": [6, 0, 0],
        "target_a": [28, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "equipped_item_class": 2,
        "expected": {"kind": "aura_on_target", "aura": 1510},
    }
    assert pre_dispatch(volley_row) == ("cast", "ground")
    soulfire_row = {
        "class": "Warlock",
        "spell_id": 6353,
        "bucket": "item_reagent_ammo",
        "effect_ids": [2, 0, 0],
        "target_a": [6, 0, 0],
        "reagents": [6265, 0, 0, 0, 0, 0, 0, 0],
        "equipped_item_class": -1,
        "expected": {"kind": "item_consumed", "reagent": 6265},
    }
    assert pre_dispatch(soulfire_row) == ("cast", "item")
    trap_row = {
        "class": "Hunter",
        "spell_id": 1499,
        "bucket": "ground_area",
        "effect_ids": [104, 0, 0],
        "aura_ids": [0, 0, 0],
        "expected": {"kind": "object_created"},
    }
    assert pre_dispatch(trap_row) == ("blocked", "effect-marker-only")
    revive_row = {
        "class": "Hunter",
        "spell_id": 982,
        "bucket": "ground_area",
        "effect_ids": [109, 0, 0],
        "aura_ids": [0, 0, 0],
        "expected": {"kind": "object_created"},
    }
    assert pre_dispatch(revive_row) == ("blocked", "target-not-dead")
    mend_row = {
        "class": "Hunter",
        "spell_id": 136,
        "bucket": "pet",
        "effect_ids": [10, 0, 0],
        "target_a": [5, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "expected": {"kind": "aura_on_target", "aura": 136},
    }
    assert pre_dispatch(mend_row) == ("blocked", "no-pet")
    assert pre_dispatch(mend_row, HUNTER_PET_SETUP_SPELL) == ("cast", "pet")
    assert pre_dispatch(mend_row, None) == ("blocked", "no-pet")
    # Fixture vocabulary: Call Pet is family-0 (outside the matrix),
    # so the matrix carries no summon row for Hunter; the fixture
    # constants pin the setup the pet scenario needs.
    assert HUNTER_PET_SETUP_SPELL == 883
    assert HUNTER_PET_SETUP_SPELL in SETUP_SPELLS["Hunter"]
    assert HUNTER_PET_ENTRY == 69
    assert HUNTER_PET_SPELL == 2649
    assert [entry for _guid, entry, _bag, _slot, _count in HUNTER_GEAR_ROWS] == [5596, 2512, 1161]
    assert EQUIPMENT_SLOT_RANGED == 17
    assert EQUIPMENT_SLOT_MAINHAND == 15
    # Warrior fixture vocabulary: all stance entries plus the
    # Bloodrage rage source seed as setup; the shield fixture pins
    # the fork-honest offhand for shield-gated rows.
    assert SETUP_SPELLS["Warrior"] == (2457, 71, 2458, 2687)
    assert WARRIOR_STANCE_SPELLS == (2457, 71, 2458)
    assert WARRIOR_RAGE_SPELL == 2687
    assert WARRIOR_TAP_REND == 6546
    assert WARRIOR_TAP_HAMSTRING == 1715
    assert WARRIOR_TAP_COST == 100
    # Rage is the second power slot (field 24): field 23 is mana and
    # reads None for a rage user, which once blinded every setup.
    assert WARRIOR_RAGE_FIELD == 24
    assert UNIT_FIELD_POWER1 == 23
    assert warrior_rage_cycles(0, 100) == 1
    assert warrior_rage_cycles(100, 100) == 0
    assert warrior_rage_cycles(0, 300) == 3
    assert warrior_rage_cycles(250, 300) == 1
    assert warrior_rage_cycles(0, 0) == 0
    assert [entry for _guid, entry, _bag, _slot, _count in WARRIOR_GEAR_ROWS] == [1161, 1166]
    assert [slot for _guid, _entry, _bag, slot, _count in WARRIOR_GEAR_ROWS] == [
        EQUIPMENT_SLOT_MAINHAND,
        EQUIPMENT_SLOT_OFFHAND,
    ]
    assert EQUIPMENT_SLOT_OFFHAND == 16
    for reason in ("effect-marker-only", "target-not-dead"):
        assert reason in BLOCKED_REASONS, reason
        assert reason in UNSCENARIOED_EVIDENCE, reason
    print("SELFTEST Hunter dispatch table ok")
    # Shaman dispatch: ground-bucket unit heals ride the ally heal
    # leg (Chain Heal shape); actual totems stay engine-totem;
    # Ghost Wolf rides the form leg; Ancestral Spirit rides the
    # resurrect leg; Reincarnation rides the item leg.
    chain_row = {
        "class": "Shaman",
        "spell_id": 1064,
        "bucket": "ground_area",
        "effect_ids": [10, 0, 0],
        "target_a": [45, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "expected": {"kind": "heal_delta_target"},
    }
    assert pre_dispatch(chain_row) == ("cast", "friendly")
    sentry_row = {
        "class": "Shaman",
        "spell_id": 6495,
        "bucket": "form_totem",
        "stances": 0,
        "totem": [5178, 0],
        "expected": {"kind": "aura_on_caster", "aura": 6495},
    }
    # Sentry's observable is its caster vision aura: it rides
    # the self leg now that the totem engine exists.
    assert pre_dispatch(sentry_row) == ("cast", "self")
    fire_nova_row = {
        "class": "Shaman",
        "spell_id": 1535,
        "bucket": "form_totem",
        "stances": 0,
        "totem": [5176, 0],
        "expected": {"kind": "totem_summon"},
    }
    assert pre_dispatch(fire_nova_row) == ("cast", "totem")
    earthbind_row = {
        "class": "Shaman",
        "spell_id": 2484,
        "bucket": "form_totem",
        "stances": 0,
        "totem": [5175, 0],
        "campaign": "classify_only",
        "expected": {"kind": "none_marker"},
    }
    assert pre_dispatch(earthbind_row) == ("blocked", "effect-marker-only")
    windfury_row = {
        "class": "Shaman",
        "spell_id": 8232,
        "bucket": "self",
        "effect_ids": [54, 0, 0],
        "target_a": [0, 0, 0],
        "reagents": [0, 0, 0, 0, 0, 0, 0, 0],
        "campaign": "classify_only",
        "expected": {"kind": "none_marker"},
    }
    assert pre_dispatch(windfury_row) == ("blocked", "effect-marker-only")
    ghost_row = {
        "class": "Shaman",
        "spell_id": 2645,
        "bucket": "form",
        "expected": {"kind": "form_change"},
    }
    assert pre_dispatch(ghost_row) == ("cast", "form")
    breath_row = {
        "class": "Shaman",
        "spell_id": 24844,
        "bucket": "hostile_dummy",
        "power_type": 2,
        "mana_cost": 50,
        "expected": {"kind": "damage_log", "opcode": 592},
    }
    assert pre_dispatch(breath_row) == ("blocked", "no-pet")
    ancestral_row = {
        "class": "Shaman",
        "spell_id": 2008,
        "bucket": "corpse",
        "expected": {"kind": "resurrect_request"},
    }
    assert pre_dispatch(ancestral_row) == ("cast", "resurrect")
    reincarnation_row = {
        "class": "Shaman",
        "spell_id": 21169,
        "bucket": "item_reagent_ammo",
        "effect_ids": [94, 0, 0],
        "target_a": [0, 0, 0],
        "reagents": [17030, 0, 0, 0, 0, 0, 0, 0],
        "equipped_item_class": -1,
        "expected": {"kind": "item_consumed", "reagent": 17030},
    }
    assert pre_dispatch(reincarnation_row) == ("cast", "item")
    for reason in ("engine-totem", "wire-target-unsupported"):
        assert reason in BLOCKED_REASONS, reason
        assert reason in UNSCENARIOED_EVIDENCE, reason
    print("SELFTEST Shaman dispatch table ok")
    # Rogue dispatch: stealth-gated openers (Stances bit 29 =
    # FORM_STEALTH 30 in the fork SharedDefines) enter via Stealth
    # and ride the form-gated leg; Stealth itself rides the form leg.
    garrote_row = {
        "class": "Rogue",
        "spell_id": 703,
        "bucket": "form_totem",
        "stances": 1 << 29,
        "totem": [0, 0],
        "expected": {"kind": "aura_on_target", "aura": 703},
    }
    assert required_form_spell(garrote_row) == 1784
    assert pre_dispatch(garrote_row) == ("cast", "form_gated")
    stealth_row = {
        "class": "Rogue",
        "spell_id": 1784,
        "bucket": "form",
        "expected": {"kind": "form_change"},
    }
    assert pre_dispatch(stealth_row) == ("cast", "form")
    # Combo builder setup closes to melee first: from a ranged
    # standoff the builder reads OUT_OF_RANGE and the finisher
    # then honestly reads NO_COMBO_POINTS (live Slice and Dice).
    combo_calls = []
    combo_driver = _TapNS(
        approach_guid=lambda dummy, standoff: combo_calls.append(("approach", dummy, standoff)),
        poll=lambda seconds: combo_calls.append(("poll", seconds)),
    )
    combo_original = globals()["cast_unit_faced"]

    def fake_builder_cast(driver, spell_id, dummy, timeout=0.0):
        combo_calls.append(("cast", spell_id, dummy))
        return True, None, []

    globals()["cast_unit_faced"] = fake_builder_cast
    try:
        combo_note = combo_builder_setup(1752)(combo_driver, 4242)
    finally:
        globals()["cast_unit_faced"] = combo_original
    assert combo_calls[:2] == [("approach", 4242, 2.0), ("cast", 1752, 4242)], combo_calls
    assert "go=True" in combo_note, combo_note
    # Form entry waits out a live form cooldown (Stealth: 10 s
    # category) instead of failing NOT_READY on re-entry.
    form_calls = []
    form_driver = _TapNS(poll=lambda seconds: form_calls.append(("poll", seconds)))
    form_originals = {name: globals()[name] for name in ("cast_self_retry_power", "live_cooldown_wait_s")}
    form_results = iter([(False, SPELL_FAILED_NOT_READY, [], 0), (True, None, [306], 0)])

    def fake_form_cast(driver, spell_id, timeout=10.0, retries=POWER_RETRIES):
        form_calls.append(("cast", spell_id))
        return next(form_results)

    globals()["cast_self_retry_power"] = fake_form_cast
    globals()["live_cooldown_wait_s"] = lambda spell_id, cap_s=600.0: 12.0
    try:
        form_go, form_reason, _form_ops, _form_waited = cast_form_entry(form_driver, 1784)
    finally:
        globals().update(form_originals)
    assert form_go and form_reason is None
    assert form_calls == [("cast", 1784), ("poll", 12.0), ("cast", 1784)], form_calls
    print("SELFTEST Rogue dispatch table ok")
    # Totality over the real Hunter fan-out artifact when present:
    # every row dispatches to a cast leg, the passive check, or a
    # closed-enum BLOCKED reason, with the Call Pet fixture summon.
    # Without it the pet rows honestly stay no-pet.
    hunter_path = matrix_path_for("Hunter")
    if hunter_path.exists():
        hunter_doc = json.loads(hunter_path.read_text())
        hunter_rows = [r for r in hunter_doc.get("rows", []) if r.get("class") == "Hunter"]
        assert hunter_rows, "Hunter artifact has no Hunter rows"
        assert class_summon_spell(hunter_rows) is None
        assert hunter_doc.get("full_talent_build") == "mm", hunter_doc.get("full_talent_build")
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in hunter_rows:
            act, pay = pre_dispatch(real_row, HUNTER_PET_SETUP_SPELL)
            if act == "cast":
                assert pay in ("self", "hostile", "ground", "item", "pet"), (real_row, pay)
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        # Hunter has no reagent-gated rows, so every item-bucket
        # row either routes to a dummy leg (hostile/ground) or is
        # classify_only-blocked: the item leg is unused, and the
        # ranged fixture exists for fork honesty, not gating.
        assert {"self", "hostile", "ground", "pet"} <= seen_legs, seen_legs
        assert "item" not in seen_legs, seen_legs
        assert "no-pet" not in seen_blocked, seen_blocked
        assert "effect-marker-only" in seen_blocked, seen_blocked
        assert "target-not-dead" in seen_blocked, seen_blocked
        for real_row in hunter_rows:
            if real_row.get("bucket") == "pet" and not (real_row.get("campaign", "") or "").startswith("classify_only"):
                assert pre_dispatch(real_row, HUNTER_PET_SETUP_SPELL) == ("cast", "pet"), real_row
                assert pre_dispatch(real_row) == ("blocked", "no-pet"), real_row
        # Seed castability (the live 145 regression): every castable
        # row's spell is in the artifact seed, the Call Pet setup
        # spell is seeded, and talent cost settles at the cap.
        hunter_seed = set(hunter_doc.get("full_seed_spell_ids", {}).get("Hunter", []))
        assert hunter_seed, "Hunter artifact seed empty"
        assert hunter_doc.get("full_seed_talent_cost", 99) <= 51, hunter_doc.get("full_seed_talent_cost")
        assert HUNTER_PET_SETUP_SPELL in seed_spells_for("Hunter")
        castable = [
            r
            for r in hunter_rows
            if not (r.get("campaign", "") or "").startswith("classify_only")
            and (r.get("expected") or {}).get("kind") not in ("none_marker", "dual_outcome", "learned_spell")
        ]
        assert castable, "no castable Hunter rows"
        for cast_row in castable:
            assert cast_row["spell_id"] in hunter_seed, cast_row["spell_id"]
            assert isinstance(cast_row.get("cooldown_ms"), int), cast_row
        print(
            f"SELFTEST Hunter artifact dispatch totality ok "
            f"({len(hunter_rows)} rows, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Hunter artifact absent; skipping totality")
    # Totality over the real Shaman fan-out artifact when present:
    # every row dispatches to a cast leg, the passive check, or a
    # closed-enum BLOCKED reason. Ground-bucket unit heals ride
    # the ally heal leg, so the ground leg is unused; actual
    # totems ride the totem leg; the only no-pet rows are the two
    # focus-cost wind-serpent autocasts, and no stance rows ride
    # form_gated.
    shaman_path = matrix_path_for("Shaman")
    if shaman_path.exists():
        shaman_doc = json.loads(shaman_path.read_text())
        shaman_rows = [r for r in shaman_doc.get("rows", []) if r.get("class") == "Shaman"]
        assert shaman_rows, "Shaman artifact has no Shaman rows"
        assert class_summon_spell(shaman_rows) is None
        assert shaman_doc.get("full_talent_build") == "enhance", shaman_doc.get("full_talent_build")
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in shaman_rows:
            act, pay = pre_dispatch(real_row)
            if act == "cast":
                assert pay in ("self", "friendly", "hostile", "item", "form", "resurrect", "totem"), (real_row, pay)
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        assert {"self", "friendly", "hostile", "item", "form", "resurrect", "totem"} <= seen_legs, seen_legs
        assert "ground" not in seen_legs, seen_legs
        assert "form_gated" not in seen_legs, seen_legs
        focus_blocked = sorted(r["spell_id"] for r in shaman_rows if pre_dispatch(r) == ("blocked", "no-pet"))
        assert focus_blocked == [24844, 25012], focus_blocked
        assert "engine-totem" not in seen_blocked, seen_blocked
        assert "wire-target-unsupported" not in seen_blocked, seen_blocked
        assert "effect-marker-only" in seen_blocked, seen_blocked
        for real_row in shaman_rows:
            if real_row.get("bucket") == "ground_area" and not (real_row.get("campaign", "") or "").startswith(
                "classify_only"
            ):
                assert (real_row.get("expected") or {}).get("kind") == "heal_delta_target", real_row
                assert pre_dispatch(real_row) == ("cast", "friendly"), real_row
        shaman_seed = set(shaman_doc.get("full_seed_spell_ids", {}).get("Shaman", []))
        assert shaman_seed, "Shaman artifact seed empty"
        assert shaman_doc.get("full_seed_talent_cost", 99) <= 51, shaman_doc.get("full_seed_talent_cost")
        castable = [
            r
            for r in shaman_rows
            if not (r.get("campaign", "") or "").startswith("classify_only")
            and (r.get("expected") or {}).get("kind") not in ("none_marker", "dual_outcome", "learned_spell")
        ]
        assert castable, "no castable Shaman rows"
        for cast_row in castable:
            assert cast_row["spell_id"] in shaman_seed, cast_row["spell_id"]
            assert isinstance(cast_row.get("cooldown_ms"), int), cast_row
        print(
            f"SELFTEST Shaman artifact dispatch totality ok "
            f"({len(shaman_rows)} rows, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Shaman artifact absent; skipping totality")
    # Totality over the real Warrior fan-out artifact when present:
    # every row dispatches to a cast leg, the passive check, or a
    # closed-enum BLOCKED reason. Stance-gated rows ride
    # form_gated (never engine-totem); the three stance entries
    # ride form; Execute-dummy/Overpower/Charge/Intercept rows stay
    # honestly BLOCKED (no movement or threshold scenario exists);
    # the focus-cost Thunderstomp rows are marker-only; no item
    # row reaches the item leg (all verify on the dummy), so the
    # shield fixture is fork honesty, never a gate.
    try:
        import spell_matrix_gen as _gen  # noqa: PLC0415

        _warrior_fanout = _gen.class_matrix_path("Warrior")
    except (ImportError, AttributeError):
        _warrior_fanout = None
    # Warrior is a pilot class, so matrix_path_for falls back to
    # the pilot matrix: totality needs the fan-out artifact itself,
    # and skips until the generator writes it.
    if _warrior_fanout is not None and _warrior_fanout.exists():
        warrior_doc = json.loads(_warrior_fanout.read_text())
        warrior_rows = [r for r in warrior_doc.get("rows", []) if r.get("class") == "Warrior"]
        assert warrior_rows, "Warrior artifact has no Warrior rows"
        assert class_summon_spell(warrior_rows) is None
        assert warrior_doc.get("full_talent_build") == "fury", warrior_doc.get("full_talent_build")
        seen_legs: set[str] = set()
        seen_blocked: set[str] = set()
        for real_row in warrior_rows:
            act, pay = pre_dispatch(real_row)
            if act == "cast":
                assert pay in ("self", "hostile", "ground", "form", "form_gated", "self_heal"), (real_row, pay)
                seen_legs.add(pay)
            elif act == "blocked":
                assert pay in BLOCKED_REASONS, (real_row, pay)
                seen_blocked.add(pay)
            else:
                assert act == "passive" and pay is None, real_row
        assert {"self", "hostile", "ground", "form", "form_gated", "self_heal"} <= seen_legs, seen_legs
        assert "item" not in seen_legs, seen_legs
        assert seen_blocked == {"effect-marker-only"}, seen_blocked
        for real_row in warrior_rows:
            if real_row.get("bucket") == "form":
                assert real_row["spell_id"] in WARRIOR_STANCE_SPELLS, real_row
                assert pre_dispatch(real_row) == ("cast", "form"), real_row
            if real_row.get("bucket") == "form_totem" and not (real_row.get("campaign", "") or "").startswith(
                "classify_only"
            ):
                assert pre_dispatch(real_row) == ("cast", "form_gated"), real_row
                assert required_form_spell(real_row) in WARRIOR_STANCE_SPELLS, real_row
        # Movement/threshold rows the harness cannot scenario stay
        # BLOCKED: Charge/Intercept teleports, Execute dummies
        # (NEEDS-HOST, no <20% gate in the engine), Overpower
        # (no aura outcome), Whirlwind, Thunderstomps.
        marker_blocked = sorted(
            r["spell_id"] for r in warrior_rows if pre_dispatch(r) == ("blocked", "effect-marker-only")
        )
        # Rank coverage keeps only the first + last Execute ranks
        # (5308 + 20662 share one signature), so the interior
        # ranks 20658/20660/20661 never reach the cast set.
        assert marker_blocked == [
            100,
            1680,
            2687,
            5308,
            7384,
            11578,
            11585,
            20252,
            20617,
            20662,
            26090,
            26188,
        ], marker_blocked
        # Seed castability (the live 145 regression): every castable
        # row's spell is seeded, all stance entries plus Bloodrage
        # seed as setup, and talent cost settles at the cap.
        warrior_seed = set(warrior_doc.get("full_seed_spell_ids", {}).get("Warrior", []))
        assert warrior_seed, "Warrior artifact seed empty"
        assert warrior_doc.get("full_seed_talent_cost", 99) <= 51, warrior_doc.get("full_seed_talent_cost")
        assert set(SETUP_SPELLS["Warrior"]) <= warrior_seed, SETUP_SPELLS["Warrior"]
        castable = [
            r
            for r in warrior_rows
            if not (r.get("campaign", "") or "").startswith("classify_only")
            and (r.get("expected") or {}).get("kind") not in ("none_marker", "dual_outcome", "learned_spell")
        ]
        assert castable, "no castable Warrior rows"
        for cast_row in castable:
            assert cast_row["spell_id"] in warrior_seed, cast_row["spell_id"]
            assert isinstance(cast_row.get("cooldown_ms"), int), cast_row
        print(
            f"SELFTEST Warrior artifact dispatch totality ok "
            f"({len(warrior_rows)} rows, legs={sorted(seen_legs)}, "
            f"blocked={sorted(seen_blocked)})"
        )
    else:
        print("SELFTEST Warrior artifact absent; skipping totality")
    # Artifact writers round-trip.
    tmp = [make_verdict(row, "PASS", evidence="t", packets=[306, 169, 306])]
    assert tmp[0]["packets"] == [169, 306]
    # Per-class raw log capture writes run-<class>.log verbatim.
    import tempfile as _tempfile

    global EVIDENCE_DIR
    real_dir = EVIDENCE_DIR
    with _tempfile.TemporaryDirectory() as tmpdir:
        EVIDENCE_DIR = pathlib.Path(tmpdir)
        text = run_leg_with_log("Selftest", lambda: print("ROW line"))
        assert text == "ROW line\n"
        assert (EVIDENCE_DIR / "run-Selftest.log").read_text() == text
    EVIDENCE_DIR = real_dir
    print("SELFTEST verdicts + packets + guid math ok")
    print("SPELL-CAMPAIGN-SELFTEST-PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "command", choices=["preflight", "fixtures", "pilot", "run", "run-class"], nargs="?", default="pilot"
    )
    parser.add_argument("class_arg", nargs="?", help="class for run-class (e.g. Paladin)")
    parser.add_argument("--spell", type=int, default=None)
    parser.add_argument("--char", default=None)
    parser.add_argument("--class", dest="class_name", default="Paladin", help="class fanned out by run-class")
    parser.add_argument(
        "--matrix",
        default=None,
        help="run-class matrix artifact override "
        "(default: the class's own artifact, "
        "spell_campaign_matrix-<class>.json for fan-out "
        "classes, the pilot matrix for pilot classes)",
    )
    parser.add_argument(
        "--only",
        type=int,
        action="append",
        default=None,
        help="restrict run-class to these spell ids (repeatable; smoke subsets)",
    )
    parser.add_argument("--self-test", action="store_true", help="offline invariant checks, no server needed")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    if args.command == "preflight":
        return preflight()
    if args.command == "fixtures":
        guard_loopback()
        return fixtures_ensure()
    if args.command == "run":
        if args.spell is None or args.char is None:
            print("run needs --spell <id> --char <name>", file=sys.stderr)
            return 2
        guard_loopback()
        return run_single(args.spell, args.char)
    if args.command == "run-class":
        guard_loopback()
        holder: list[int] = []
        which = args.class_arg or args.class_name

        def _leg() -> None:
            holder.append(run_class(which, args.only, matrix_path=args.matrix))

        run_leg_with_log(which, _leg)
        return holder[0] if holder else 2
    guard_loopback()
    return run_pilot()


if __name__ == "__main__":
    sys.exit(main())
