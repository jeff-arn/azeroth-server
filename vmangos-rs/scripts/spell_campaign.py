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
* `fixtures` -- idempotent `ensure` of the pilot accounts/characters
  (Warrior/Mage/Druid): account rows and the `CMSG_CHAR_CREATE`
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
import os
import pathlib
import select
import socket
import struct
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402
import phase3_scripted_login as p3  # noqa: E402
from scripted_driver import (  # noqa: E402
    ScriptedDriver,
    SMSG_CAST_RESULT,
    SMSG_SPELL_GO,
    UNIT_FIELD_HEALTH,
    UNIT_FIELD_LEVEL,
    UNIT_FIELD_MAXHEALTH,
)
UNIT_FIELD_POWER1 = 23
UNIT_FIELD_MAXPOWER1 = 30
SPELL_FAILED_NO_POWER = 77
SPELL_FAILED_ONLY_SHAPESHIFT = 86
SPELL_FAILED_NOT_SHAPESHIFT = 61
SPELL_FAILED_OUT_OF_RANGE = 89

SCRIPT_DIR = pathlib.Path(__file__).parent
EVIDENCE_DIR = pathlib.Path("docs/rust-migration-evidence/spell-campaign")
MATRIX_PATH = SCRIPT_DIR / "spell_campaign_matrix.json"
LOCK_PATH = EVIDENCE_DIR / "RUNNING.lock"

CMSG_CHAR_CREATE = 54
SMSG_CHAR_CREATE = 58
CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CHAR_CREATE_SUCCESS = 46
SMSG_SPELL_START = 305
SMSG_SPELLNONMELEEDAMAGELOG = 592
CMSG_SET_ACTIVE_MOVER = 618

AURA_FIELD_FIRST = 47
AURA_FIELD_LAST = 102

# (account, char name, race, class) per architecture §2 fixture table.
PILOT_FIXTURES = [
    ("SPELLCAMP01", "Campwarrior", 1, 1),   # Human Warrior
    ("SPELLCAMP07", "Campmage", 1, 8),      # Human Mage
    ("SPELLCAMP09", "Campdruid", 4, 11),    # NightElf Druid
]

# Coldridge arena state (map 0, beside the proven 705/707 spawns).
CASTER_POS = (-6295.0, 315.0, 377.5)
# Heal-ally spot: ~15 yd from the nearest wolf counter so gray-mob
# aggro cannot maul the damaged ally mid-leg (z=380 falls to ground).
HEAL_CASTER_POS = (-6270.0, 292.0, 380.0)
ALLY_POS = (-6268.0, 290.0, 380.0)
WOLF_COUNTERS = [344, 345, 346, 347, 1525, 1526]
TROGG_COUNTERS = [335, 340, 341, 342]

VERDICTS = {"PASS", "FAIL", "BLOCKED", "PASSIVE-VERIFIED",
            "PASSIVE-UNVERIFIED"}
BLOCKED_REASONS = {
    "scenario-missing-ally", "no-pet", "no-party",
    "wire-target-unsupported", "engine-totem", "fixture-gear",
    "fixture-reagent", "dummy-immune", "dummy-unavailable",
    "area-or-quest-gated", "effect-marker-only",
    "script-evidence-unknown", "target-not-dead",
}


def raw_guid(entry: int, counter: int) -> int:
    """Raw creature GUID the bench world DB spawns use."""
    return (0xF130 << 48) | (entry << 24) | counter


def char_create_body(name: str, race: int, cls: int) -> bytes:
    """Wire shape `world_actor.rs::on_char_create` parses."""
    return (name.encode() + b"\x00"
            + struct.pack("<9B", race, cls, 0, 0, 0, 0, 0, 0, 0))


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

def make_verdict(row: dict, verdict: str, attempts: int = 1,
                 reason: str | None = None, phase: str | None = None,
                 evidence: str = "", pre: str = "", post: str = "",
                 packets: list[int] | None = None,
                 target_guid: int | None = None,
                 expected_failure: bool = False) -> dict:
    assert verdict in VERDICTS, verdict
    if verdict == "BLOCKED":
        assert reason in BLOCKED_REASONS, reason
    if verdict == "FAIL":
        assert phase in {"prepare", "go", "flight", "apply", "persist"}, phase
    if expected_failure:
        assert verdict == "FAIL" and phase == "prepare", (
            verdict, phase)
    return {
        "class": row.get("class"), "spell_id": row.get("spell_id"),
        "name": row.get("name", ""), "bucket": row.get("bucket"),
        "rank": row.get("rank"), "rank_count": row.get("rank_count"),
        "is_last_rank": row.get("is_last_rank"),
        "attempts": attempts,
        "verdict": verdict, "reason": reason, "phase": phase,
        "expected_failure": expected_failure,
        "target_guid": target_guid,
        "evidence": evidence, "pre": pre, "post": post,
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
        lines.append("counts: " + ", ".join(
            f"{k}={counts.get(k, 0)}" for k in sorted(counts)))
        lines.append("bucket counts: " + ", ".join(
            f"{bucket}/{verdict}={count}"
            for (bucket, verdict), count in sorted(bucket_counts.items())))
        for v in verdicts:
            if v["verdict"] in ("FAIL", "BLOCKED"):
                lines.append(
                    f"- {v['spell_id']} {v.get('name') or ''} "
                    f"rank {v.get('rank')}/{v.get('rank_count')} "
                    f"{v['bucket']} target={v.get('target_guid')}: "
                    f"{v['verdict']} "
                    f"{v.get('reason') or ''} {v.get('phase') or ''} "
                    f"expected_failure={v.get('expected_failure')} "
                    f"| {v['evidence']}")
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
        "binary_build": "unknown: bench mangosd was already running; "
        "this harness neither builds nor starts it",
        "world_host": p3.HOST, "world_port": p3.PORT,
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

    r = subprocess.run(["git", "rev-parse", "--short", "HEAD"],
                       capture_output=True, text=True)
    return r.stdout.strip()


def subprocess_dirty() -> bool:
    import subprocess

    r = subprocess.run(["git", "status", "--porcelain"],
                       capture_output=True, text=True)
    return bool(r.stdout.strip())


# --------------------------------------------------------------------------
# Guards and preflight (§1, §7)
# --------------------------------------------------------------------------

def guard_loopback() -> None:
    host = p3.HOST
    assert host in ("127.0.0.1", "localhost"), \
        f"refusing non-loopback world host {host!r}"
    resolved = socket.gethostbyname(host)
    assert resolved.startswith("127."), f"refusing {host=}->{resolved=}"
    db_host = os.environ.setdefault("MYSQL_HOST", "127.0.0.1")
    assert socket.gethostbyname(db_host).startswith("127."), \
        f"refusing non-loopback MYSQL_HOST={db_host!r}"
    print(f"GUARD-OK world={host}:{p3.PORT} resolved={resolved} "
          f"mysql_host={db_host}")


def preflight() -> int:
    import subprocess as _subprocess

    guard_loopback()
    for probe, marker in (
            ("scripted_driver.py", "SCRIPTED-DRIVER-SELFTEST-PASS"),
            ("two_client_smoke.py", "TWO-CLIENT-SELFTEST-PASS")):
        r = _subprocess.run(
            [sys.executable, str(SCRIPT_DIR / probe), "--self-test"],
            capture_output=True, text=True)
        if r.returncode != 0 or marker not in (r.stdout + r.stderr):
            print(f"PREFLIGHT-FAIL {probe}:\n{r.stdout}\n{r.stderr}")
            return 2
        print(f"PREFLIGHT-OK {probe} {marker}")
    hostname = smoke.sql_scalar("realmd", "SELECT @@hostname")
    print(f"DB-OK @@hostname={hostname}")
    row = smoke.sql_scalar(
        "mangos",
        "SELECT COUNT(*) FROM creature_template WHERE entry=327")
    spawns = smoke.sql_scalar(
        "mangos", "SELECT COUNT(*) FROM creature WHERE id=327")
    print(f"PROBE-ENTRY-327 template={row} spawns={spawns} "
          f"(informational; pilot runs on 705/707 regardless)")
    return 0


# --------------------------------------------------------------------------
# Fixtures (§2)
# --------------------------------------------------------------------------

def ensure_account(username: str) -> int:
    found = smoke.sql_scalar(
        "realmd",
        f"SELECT id FROM account WHERE username='{username}'")
    if found:
        return int(found)
    smoke.sql_scalar(
        "realmd", f"INSERT INTO account (username) VALUES ('{username}')")
    found = smoke.sql_scalar(
        "realmd", f"SELECT id FROM account WHERE username='{username}'")
    print(f"ACCOUNT-CREATED {username} id={found}")
    return int(found)


def ensure_character(account: str, account_id: int, name: str,
                     race: int, cls: int) -> int:
    found = smoke.sql_scalar(
        "characters", f"SELECT guid FROM characters WHERE name='{name}'")
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
    body, _ = smoke.read_until_opcode(client, SMSG_CHAR_CREATE, 10,
                                      f"create {name}")
    code = body[0] if body else None
    client.sock.close()
    if code != CHAR_CREATE_SUCCESS:
        raise AssertionError(f"CMSG_CHAR_CREATE {name} refused: code={code}")
    found = smoke.sql_scalar(
        "characters", f"SELECT guid FROM characters WHERE name='{name}'")
    if not found:
        raise AssertionError(f"character {name} missing after code 46")
    print(f"CHAR-CREATED {name} guid={found} race={race} class={cls}")
    return int(found)


def seed_character(guid: int, spells: list[int],
                   pos: tuple[float, float, float] = CASTER_POS) -> None:
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
        f"WHERE guid={guid}")
    smoke.sql_scalar(
        "characters", f"DELETE FROM character_spell WHERE guid={guid}")
    for spell in spells:
        smoke.sql_scalar(
            "characters",
            f"INSERT INTO character_spell (guid,spell,active,disabled) "
            f"VALUES ({guid},{spell},1,0)")
    smoke.sql_scalar(
        "characters", f"DELETE FROM character_spell_cooldown WHERE guid={guid}")
    smoke.sql_scalar(
        "characters", f"DELETE FROM character_aura WHERE guid={guid}")
    smoke.sql_scalar(
        "characters", f"DELETE FROM corpse WHERE player_guid={guid}")
    print(f"SEEDED guid={guid} spells={len(spells)} pos=({x},{y},{z})")


SETUP_SPELLS: dict[str, tuple[int, ...]] = {
    # Battle Stance is scenario setup for stance-gated Rend, not a
    # verdict row. It is seeded and counted explicitly.
    "Warrior": (2457,),
}


def pilot_spells_for(class_name: str) -> list[int]:
    doc = json.loads(MATRIX_PATH.read_text())
    spells = {r["spell_id"] for r in doc["rows"]
              if r["class"] == class_name}
    spells.update(SETUP_SPELLS.get(class_name, ()))
    return sorted(spells)


def drift_gate(driver: ScriptedDriver, guid: int,
               expect_spells: int) -> None:
    level = driver.world.get(guid).values.get(UNIT_FIELD_LEVEL) \
        if driver.world.get(guid) else None
    if level != 60:
        raise AssertionError(f"fixture drift: level={level} want 60")
    have = smoke.sql_scalar(
        "characters",
        f"SELECT COUNT(*) FROM character_spell WHERE guid={guid}")
    if int(have) != expect_spells:
        raise AssertionError(
            f"fixture drift: character_spell={have} want {expect_spells}")
    health = driver.self_health()
    max_health = driver.world.get(driver.own_guid).values.get(
        UNIT_FIELD_MAXHEALTH) if driver.own_guid else None
    if health != max_health or not health:
        raise AssertionError(
            f"fixture drift: health={health} max={max_health}")
    print(f"DRIFT-OK guid={guid} level=60 spells={have} "
          f"health={health}")


def fixtures_ensure() -> int:
    guids: dict[str, int] = {}
    for account, name, race, cls in PILOT_FIXTURES:
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
        "caster_guid": caster_guid, "target_guid": target_guid,
        "cast_counter": cast_counter, "spell_id": spell_id,
        "hits": hits, "misses": misses,
    }


def spell_go_matches(parsed: dict | None, spell_id: int,
                     caster_guid: int | None,
                     target_guid: int | None = None) -> bool:
    if parsed is None or parsed["spell_id"] != spell_id:
        return False
    if caster_guid is not None and parsed["caster_guid"] != caster_guid:
        return False
    if target_guid is not None:
        return (parsed["target_guid"] == target_guid
                or target_guid in parsed["hits"])
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
        "target_guid": target_guid, "attacker_guid": attacker_guid,
        "spell_id": spell_id, "damage": damage,
    }


def event_mark(driver: ScriptedDriver) -> int:
    return len(driver.events)


def events_since(driver: ScriptedDriver, mark: int) -> list[tuple[int, bytes]]:
    return driver.events[mark:]


def matching_damage_logs(events: list[tuple[int, bytes]], spell_id: int,
                         target_guid: int,
                         attacker_guid: int | None = None) -> list[dict]:
    logs = []
    for opcode, body in events:
        if opcode != SMSG_SPELLNONMELEEDAMAGELOG:
            continue
        parsed = parse_damage_log_body(body)
        if parsed is None:
            continue
        if parsed["spell_id"] != spell_id:
            continue
        if parsed["target_guid"] != target_guid:
            continue
        if (attacker_guid is not None
                and parsed["attacker_guid"] != attacker_guid):
            continue
        logs.append(parsed)
    return logs


def send_and_wait(driver: ScriptedDriver, body: bytes, spell_id: int,
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
            if spell_go_matches(parsed, spell_id, driver.own_guid,
                                target_guid):
                return True, None, ops
        if opcode == SMSG_CAST_RESULT:
            parsed = parse_cast_result_body(pkt)
            if parsed is not None and parsed[0] == spell_id:
                return False, parsed[1], ops
    return False, None, ops


def login_as(account: str, account_id: int, guid: int,
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
            if verify is not None and seen_update \
                    and time.monotonic() - last_packet_at >= 1.5:
                break
            continue
        op, body = client.read_packet()
        last_packet_at = time.monotonic()
        driver.observe(op, body)
        if op == smoke.SMSG_LOGIN_VERIFY_WORLD:
            verify = body
        if op in (smoke.SMSG_UPDATE_OBJECT,
                  smoke.SMSG_COMPRESSED_UPDATE_OBJECT):
            seen_update = True
    if verify is None:
        raise AssertionError(f"{account}: no SMSG_LOGIN_VERIFY_WORLD")
    map_id, x, y, z, orientation = struct.unpack("<Iffff", verify)
    driver.position = (x, y, z, orientation)
    client.send_packet(smoke.CMSG_SET_ACTIVE_MOVER, struct.pack("<Q", guid))
    print(f"LOGIN-OK account={account} guid={guid} map={map_id} "
          f"pos=({x:.2f},{y:.2f},{z:.2f})")
    return client, driver


def power1(driver: ScriptedDriver, guid: int) -> int | None:
    obj = driver.world.get(guid)
    return None if obj is None else obj.values.get(UNIT_FIELD_POWER1)


def face_guid(driver: ScriptedDriver, guid: int) -> None:
    pos = driver.world.position(guid)
    if pos is not None:
        driver.face(pos[0], pos[1])


def cast_unit_faced(driver: ScriptedDriver, spell_id: int, target: int,
                    timeout: float = 10.0) -> tuple[bool, int | None,
                                                   list[int]]:
    """Face the target first (1.12.1 casts require facing); otherwise
    identical to `send_and_wait`."""
    face_guid(driver, target)
    return send_and_wait(driver, cast_unit_body(spell_id, target), spell_id,
                         timeout, target_guid=target)


def cast_self_retry_power(driver: ScriptedDriver, spell_id: int,
                          timeout: float = 10.0,
                          retries: int = 3) -> tuple[bool, int | None,
                                                   list[int], int]:
    """Self cast with the `cast_until_dead` mana pacing treatment:
    NO_POWER(77) waits 9 s (five-second rule + regen tick) and retries.
    Returns (go, reason, ops, power_retries)."""
    waited = 0
    for _ in range(retries + 1):
        go, reason, ops = send_and_wait(driver, cast_self_body(spell_id),
                                        spell_id, timeout)
        if go or reason != SPELL_FAILED_NO_POWER or waited >= retries:
            return go, reason, ops, waited
        waited += 1
        driver.poll(9.0)
    return go, reason, ops, waited


def snapshot(driver: ScriptedDriver, guid: int) -> str:
    obj = driver.world.get(guid)
    if obj is None:
        return "absent"
    return (f"health={obj.values.get(UNIT_FIELD_HEALTH)} "
            f"max={obj.values.get(UNIT_FIELD_MAXHEALTH)} "
            f"power1={obj.values.get(UNIT_FIELD_POWER1)}")


def live_dummy(driver: ScriptedDriver, entry: int,
               counters: list[int],
               exclude: int | None = None) -> int | None:
    """First counter of `entry` tracked with health > 0, else None."""
    for counter in counters:
        guid = raw_guid(entry, counter)
        if guid == exclude:
            continue
        if driver.target_health(guid) not in (None, 0):
            return guid
    return None


def ensure_battle_stance(driver: ScriptedDriver, guid: int,
                         ) -> tuple[bool, int | None, list[int]]:
    """Enter Battle Stance (2457) so stance-gated Warrior rows are
    cast under fork-legal conditions, and verify its aura landed."""
    if aura_present(driver, guid, 2457):
        return True, None, []
    go, reason, ops = send_and_wait(
        driver, cast_self_body(2457), 2457, target_guid=guid)
    driver.poll(1.5)
    return bool(go and aura_present(driver, guid, 2457)), reason, ops


def expected_failure_verdict(row: dict, go: bool, reason: int | None,
                             packets: list[int], target_guid: int,
                             attempts: int = 1) -> dict:
    """Verdict for a matrix `expect_cast_failure` row.

    This row can never PASS. A shapeshift rejection is the pilot
    gate's expected FAIL; a landed cast is an unexpected engine FAIL.
    """
    spell_label = row.get("name") or f"spell {row.get('spell_id')}"
    if go:
        return make_verdict(
            row, "FAIL", attempts=attempts, phase="prepare",
            evidence=f"{spell_label} landed from the wrong form; "
            "stance/shape gate missing (engine finding, not a pass)",
            packets=packets, target_guid=target_guid)
    expected = reason in (SPELL_FAILED_ONLY_SHAPESHIFT,
                          SPELL_FAILED_NOT_SHAPESHIFT)
    if expected:
        evidence = (
            f"expected wrong-form rejection for {spell_label}: "
            f"CAST_RESULT({reason}) after {attempts} attempt(s)")
    elif reason == SPELL_FAILED_OUT_OF_RANGE:
        evidence = (
            f"wrong-form premise untested for {spell_label}: "
            f"all {attempts} attempt(s) ended OUT_OF_RANGE(89)")
    elif reason is None:
        evidence = f"no bound cast answer for {spell_label}"
    else:
        evidence = (
            f"wrong-form cast for {spell_label} failed for an "
            f"unclassified reason CAST_RESULT({reason})")
    return make_verdict(
        row, "FAIL", attempts=attempts, phase="prepare",
        reason=(f"CAST_RESULT({reason})" if reason is not None else None),
        evidence=evidence, packets=packets, target_guid=target_guid,
        expected_failure=expected)


def _reset_target_ok(item: dict | None, require_dead: bool) -> bool:
    if not item:
        return False
    pre_hp = item.get("pre_hp")
    post_hp = item.get("post_hp")
    if pre_hp is None or post_hp is None:
        return False
    health_ok = post_hp == 0 if require_dead else post_hp < pre_hp
    return health_ok and bool(item.get("aura_seen")
                              or item.get("damage_logs"))


def reset_evidence_ok(first: dict | None, second: dict | None) -> bool:
    """A reset is proven only by attributed damage to two counters."""
    if not first or not second or first.get("guid") == second.get("guid"):
        return False
    return (_reset_target_ok(first, require_dead=True)
            and _reset_target_ok(second, require_dead=False))


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
    health_fell = (pre_hp is not None and post_hp is not None
                   and post_hp < pre_hp)
    damage_logs = observed.get("damage_logs", [])
    damage_total = sum(int(log.get("damage", 0)) for log in damage_logs)
    pre = observed.get("pre", "")
    post = observed.get("post", "")

    if kind == "expect_cast_failure":
        return expected_failure_verdict(
            row, go, reason, packets, target_guid, attempts=attempts)
    if kind == "resurrect_request":
        return make_verdict(
            row, "BLOCKED", attempts=attempts,
            reason="wire-target-unsupported",
            evidence="single repro has no corpse-target scenario; "
            "use the pilot classification leg",
            pre=pre, post=post, packets=packets,
            target_guid=target_guid)
    if kind == "heal_delta_target":
        return make_verdict(
            row, "BLOCKED", attempts=attempts,
            reason="scenario-missing-ally",
            evidence="single repro has no damaged-ally scenario; "
            "use the pilot heal leg",
            pre=pre, post=post, packets=packets,
            target_guid=target_guid)
    if not go:
        return make_verdict(
            row, "FAIL", attempts=attempts, phase="prepare",
            reason=(f"CAST_RESULT({reason})"
                    if reason is not None else None),
            evidence=f"no bound SPELL_GO for spell {row.get('spell_id')} "
            f"(reason={reason})", pre=pre, post=post, packets=packets,
            target_guid=target_guid)

    if kind in ("aura_on_caster", "form_change"):
        outcome_ok = bool(observed.get("caster_aura"))
        detail = f"caster_aura={observed.get('caster_aura')}"
    elif kind == "aura_on_target":
        outcome_ok = bool(observed.get("target_aura"))
        detail = f"target_aura={observed.get('target_aura')}"
    elif kind == "health_delta_target":
        attribution = bool(observed.get("target_aura") or damage_logs)
        if not expected.get("aura"):
            attribution = True
        outcome_ok = health_fell and attribution
        detail = (f"health {pre_hp}->{post_hp}, "
                  f"target_aura={observed.get('target_aura')}, "
                  f"damage_logs={len(damage_logs)}")
    elif kind == "damage_log":
        outcome_ok = health_fell and bool(damage_logs)
        detail = (f"health {pre_hp}->{post_hp}, "
                  f"matching damage={damage_total}")
    else:
        return make_verdict(
            row, "BLOCKED", attempts=attempts,
            reason="effect-marker-only",
            evidence=f"single repro cannot observe expected kind {kind}",
            pre=pre, post=post, packets=packets,
            target_guid=target_guid)
    return make_verdict(
        row, "PASS" if outcome_ok else "FAIL", attempts=attempts,
        phase=None if outcome_ok else "apply",
        evidence=("observable outcome: " if outcome_ok else
                  "SPELL_GO without required observable outcome: ")
        + detail, pre=pre, post=post, packets=packets,
        target_guid=target_guid)


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
        print(f"ROW {class_name} {v['spell_id']} {v['bucket']}: "
              f"{status} {extra} | {v['evidence']}")

    def matrix_row(self, class_name: str, spell_id: int) -> dict:
        doc = json.loads(MATRIX_PATH.read_text())
        for r in doc["rows"]:
            if r["class"] == class_name and r["spell_id"] == spell_id:
                return r
        raise AssertionError(f"matrix lacks {class_name}/{spell_id}")

    # -- leg 1: Warrior -----------------------------------------------------
    def leg_warrior(self, guid: int) -> None:
        row = self.matrix_row("Warrior", 5242)
        client, driver = login_as("SPELLCAMP01", self.account_id("SPELLCAMP01"),
                                  guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Warrior")))
            pre = snapshot(driver, guid)
            go, reason, ops = send_and_wait(driver, cast_self_body(5242), 5242)
            driver.poll(2.0)
            post = snapshot(driver, guid)
            if go and aura_present(driver, guid, 5242):
                self.record("Warrior", make_verdict(
                    row, "PASS", evidence="bound SPELL_GO + aura "
                    f"5242 in self fields 47-94", pre=pre, post=post,
                    packets=ops, target_guid=guid))
            else:
                self.record("Warrior", make_verdict(
                    row, "FAIL", phase="prepare" if not go else "apply",
                    reason=(f"CAST_RESULT({reason})" if reason is not None
                            else None),
                    evidence=f"go={go} reason={reason} "
                    f"aura5242={aura_present(driver, guid, 5242)}",
                    pre=pre, post=post, packets=ops, target_guid=guid))

            row = self.matrix_row("Warrior", 6546)
            dummy = live_dummy(driver, 705, WOLF_COUNTERS)
            if dummy is None:
                self.record("Warrior", make_verdict(
                    row, "BLOCKED", reason="dummy-unavailable",
                    evidence="no 705 counter with health>0 in WorldState"))
                return
            stance_ok, stance_reason, stance_ops = ensure_battle_stance(
                driver, guid)
            if not stance_ok:
                self.record("Warrior", make_verdict(
                    row, "FAIL", phase="prepare",
                    reason=(f"CAST_RESULT({stance_reason})"
                            if stance_reason is not None else None),
                    evidence="Battle Stance setup did not verify aura "
                    "2457; Rend not cast under invalid conditions",
                    packets=stance_ops, target_guid=dummy))
                return
            driver.approach_guid(dummy, 2.0)
            pre_hp = driver.target_health(dummy)
            pre = f"dummy guid={dummy} health={pre_hp}; stance aura=2457"
            go, reason, ops = cast_unit_faced(driver, 6546, dummy)
            ops = stance_ops + ops
            attempts = 1
            while go and attempts < 5 and not aura_present(
                    driver, dummy, 6546):
                driver.poll(1.6)
                attempts += 1
                if not aura_present(driver, dummy, 6546):
                    go, reason, ops2 = cast_unit_faced(driver, 6546, dummy)
                    ops += ops2
            driver.poll(4.0)  # Rend ticks
            post_hp = driver.target_health(dummy)
            dot = aura_present(driver, dummy, 6546)
            if (go and dot and pre_hp is not None and post_hp is not None
                    and post_hp < pre_hp):
                self.record("Warrior", make_verdict(
                    row, "PASS", attempts=attempts,
                    evidence="Battle Stance aura 2457 verified, then "
                    f"DoT aura 6546 on dummy guid={dummy} + health "
                    f"{pre_hp}->{post_hp}", pre=pre,
                    post=f"dummy guid={dummy} health={post_hp}",
                    packets=ops, target_guid=dummy))
            else:
                self.record("Warrior", make_verdict(
                    row, "FAIL", attempts=attempts,
                    phase="apply" if go else "prepare",
                    reason=(f"CAST_RESULT({reason})"
                            if not go and reason is not None else None),
                    evidence=f"stance_ok=True go={go} dot={dot} health "
                    f"{pre_hp}->{post_hp}", pre=pre,
                    post=f"dummy guid={dummy} health={post_hp}",
                    packets=ops, target_guid=dummy))
        finally:
            smoke.logout(client, "warrior-pilot")

    # -- leg 2: Mage --------------------------------------------------------
    def leg_mage(self, guid: int) -> None:
        row = self.matrix_row("Mage", 168)
        client, driver = login_as("SPELLCAMP07", self.account_id("SPELLCAMP07"),
                                  guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Mage")))
            pre = snapshot(driver, guid)
            go, reason, ops = send_and_wait(driver, cast_self_body(168), 168)
            driver.poll(1.5)
            post = snapshot(driver, guid)
            if go and aura_present(driver, guid, 168):
                seeded_count = len(pilot_spells_for("Mage"))
                smoke.logout(client, "mage-168-persist")
                total_rows = int(smoke.sql_scalar(
                    "characters",
                    f"SELECT COUNT(*) FROM character_spell WHERE guid={guid}"))
                rows_168 = int(smoke.sql_scalar(
                    "characters",
                    f"SELECT COUNT(*) FROM character_spell WHERE guid={guid} "
                    f"AND spell=168"))
                client, driver = login_as(
                    "SPELLCAMP07", self.account_id("SPELLCAMP07"), guid)
                drift_gate(driver, guid, seeded_count)
                persist_ok = total_rows == seeded_count and rows_168 == 1
                self.record("Mage", make_verdict(
                    row, "PASS" if persist_ok else "FAIL",
                    phase=None if persist_ok else "persist",
                    evidence="bound SPELL_GO + aura 168 on self; after "
                    f"logout character_spell total={total_rows} "
                    f"(seeded={seeded_count}), spell168 rows={rows_168}",
                    pre=pre, post=post, packets=ops, target_guid=guid))
            else:
                self.record("Mage", make_verdict(
                    row, "FAIL", phase="prepare" if not go else "apply",
                    reason=(f"CAST_RESULT({reason})" if reason is not None
                            else None),
                    evidence=f"go={go} aura168="
                    f"{aura_present(driver, guid, 168)}", pre=pre, post=post,
                    packets=ops, target_guid=guid))

            row = self.matrix_row("Mage", 133)
            dummy = live_dummy(driver, 705, WOLF_COUNTERS)
            if dummy is None:
                self.record("Mage", make_verdict(
                    row, "BLOCKED", reason="dummy-unavailable",
                    evidence="no 705 counter with health>0 in WorldState"))
                return
            driver.approach_guid(dummy, 13.0)
            pre_hp = driver.target_health(dummy)
            pre = f"dummy guid={dummy} health={pre_hp}"
            mark = event_mark(driver)
            go, reason, _ops = cast_unit_faced(driver, 133, dummy,
                                               timeout=15.0)
            driver.poll(6.0)  # flight + damage log
            window = events_since(driver, mark)
            flight_ops = [op for op, _ in window]
            start_seen = SMSG_SPELL_START in flight_ops
            logs = matching_damage_logs(window, 133, dummy, guid)
            damage_total = sum(log["damage"] for log in logs)
            post_hp = driver.target_health(dummy)
            if go and start_seen and logs and pre_hp is not None \
                    and post_hp is not None and post_hp < pre_hp:
                self.record("Mage", make_verdict(
                    row, "PASS",
                    evidence="bound SPELL_START+SPELL_GO, then opcode "
                    f"592 body spell=133 target={dummy} attacker={guid} "
                    f"damage={damage_total}; dummy health "
                    f"{pre_hp}->{post_hp}", pre=pre,
                    post=f"dummy guid={dummy} health={post_hp}",
                    packets=flight_ops, target_guid=dummy))
            else:
                self.record("Mage", make_verdict(
                    row, "FAIL", phase="flight" if go else "prepare",
                    reason=(f"CAST_RESULT({reason})"
                            if not go and reason is not None else None),
                    evidence=f"go={go} start={start_seen} "
                    f"matching_logs={len(logs)} damage={damage_total} "
                    f"health {pre_hp}->{post_hp}", pre=pre,
                    post=f"dummy guid={dummy} health={post_hp}",
                    packets=flight_ops, target_guid=dummy))
        finally:
            smoke.logout(client, "mage-pilot")

    # -- legs 3-5: Druid ----------------------------------------------------
    def leg_druid(self, guid: int, ally_guid: int) -> None:
        client, driver = login_as("SPELLCAMP09", self.account_id("SPELLCAMP09"),
                                  guid)
        try:
            drift_gate(driver, guid, len(pilot_spells_for("Druid")))
            # Healing leg needs a damaged ally: ally was SQL-damaged
            # while offline (saved low health persists through load,
            # which only clamps downward-overshoot to the max).
            ally_client, ally_driver = login_as(
                "SPELLCAMP01", self.account_id("SPELLCAMP01"), ally_guid)
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
                go, reason, ops = cast_unit_faced(
                    driver, 5187, ally_guid, timeout=12.0)
                if not go and reason == SPELL_FAILED_NO_POWER:
                    driver.poll(9.0)
                    go, reason, ops = cast_unit_faced(
                        driver, 5187, ally_guid, timeout=12.0)
                ally_driver.poll(4.0)
                t2 = time.monotonic()
                h2 = ally_driver.self_health()
                if go and h0 is not None and h1 is not None \
                        and h2 is not None and t1 > t0:
                    rate = (h1 - h0) / (t1 - t0)
                    jump = h2 - (h1 + rate * (t2 - t1))
                    heal_band = row["expected"].get(
                        "heal_band", [204, 254])
                    jump_band = row["expected"].get(
                        "observed_jump_band", [170, 288])
                    # Oracle heal is 204-254; a discrete regen tick
                    # can phase-shift the baselined jump by one tick
                    # (+34), hence the generated observed band. The
                    # check is bounded on both sides.
                    if heal_jump_in_band(jump, jump_band):
                        self.record("Druid", make_verdict(
                            row, "PASS",
                            evidence="Healing Touch rank 3 bound GO + "
                            "regen-baselined jump "
                            f"{jump:.0f} within observed band "
                            f"{jump_band[0]}-{jump_band[1]} (oracle heal "
                            f"band {heal_band[0]}-{heal_band[1]}, rate "
                            f"{rate:.1f}/s, {h0}->{h1} pre, got {h2})",
                            pre=pre, post=f"ally health={h2}",
                            packets=ops, target_guid=ally_guid))
                    else:
                        self.record("Druid", make_verdict(
                            row, "FAIL", phase="apply",
                            evidence=f"GO but jump {jump:.0f} outside "
                            f"observed band {jump_band[0]}-{jump_band[1]} "
                            f"(oracle heal band {heal_band[0]}-"
                            f"{heal_band[1]}, rate {rate:.1f}/s, "
                            f"{h0}->{h1} pre, got {h2})",
                            pre=pre, post=f"ally health={h2}",
                            packets=ops, target_guid=ally_guid))
                else:
                    self.record("Druid", make_verdict(
                        row, "FAIL", phase="prepare",
                        reason=(f"CAST_RESULT({reason})"
                                if not go and reason is not None else None),
                        evidence=f"cast failed or missing update: go={go} "
                        f"reason={reason} h={h0}->{h1}->{h2}",
                        pre=pre,
                        post=f"ally health={h2 or '?'}", packets=ops,
                        target_guid=ally_guid))
            finally:
                smoke.logout(ally_client, "druid-ally")

            # Wrong-form leg first: Growl (bear-form gated) from
            # caster form (guaranteed shapeshift-free this early).
            # Expected: shapeshift CAST_RESULT failure, recorded as
            # the gate's expected FAIL/prepare. A landed cast is an
            # unexpected engine FAIL, never PASS.
            row = self.matrix_row("Druid", 6795)
            if aura_present(driver, guid, 5487) or aura_present(
                    driver, guid, 768):
                self.record("Druid", make_verdict(
                    row, "BLOCKED", reason="scenario-missing-ally",
                    evidence="already shapeshifted; wrong-form premise "
                    "unusable", target_guid=guid))
            else:
                candidates = [
                    raw_guid(705, counter) for counter in WOLF_COUNTERS
                    if driver.target_health(raw_guid(705, counter))
                    not in (None, 0)]
                if not candidates:
                    self.record("Druid", make_verdict(
                        row, "BLOCKED", reason="dummy-unavailable",
                        evidence="no live 705 for wrong-form Growl"))
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
                            go, reason, ops = cast_unit_faced(
                                driver, 6795, dummy)
                            all_ops += ops
                            if go or reason != SPELL_FAILED_OUT_OF_RANGE:
                                break
                            driver.poll(1.0)
                        if go or reason != SPELL_FAILED_OUT_OF_RANGE:
                            break
                    self.record("Druid", expected_failure_verdict(
                        row, go, reason, all_ops, dummy,
                        attempts=attempts))

            # Form legs: Bear then Cat replaces it. Cat is followed
            # by logout/login (§7.4) and the relog aura state is
            # recorded in its verdict either way.
            for spell, aura, label in ((5487, 5487, "bear"),
                                       (768, 768, "cat")):
                row = self.matrix_row("Druid", spell)
                pre = snapshot(driver, guid)
                cancel_note = ""
                if spell == 768 and aura_present(driver, guid, 5487):
                    # Stock-shaped form switch: Cat Form carries
                    # SPELL_ATTR_NOT_SHAPESHIFT, so the fork gate
                    # rejects a direct Bear -> Cat cast. Cancel
                    # Bear (CMSG_CANCEL_AURA 310) first.
                    driver.client.send_packet(
                        310, struct.pack("<I", 5487))
                    driver.poll(1.5)
                    cancel_note = (
                        "; cancelled Bear first "
                        f"(bear_aura_after_cancel="
                        f"{aura_present(driver, guid, 5487)})")
                go, reason, ops, waited = cast_self_retry_power(
                    driver, spell)
                _ = label
                driver.poll(1.5)
                post = snapshot(driver, guid)
                if go and aura_present(driver, guid, aura):
                    relog_note = ""
                    relog_bear = False
                    if spell == 768:
                        bear_before_relog = aura_present(
                            driver, guid, 5487)
                        smoke.logout(client, "druid-form-persist")
                        client, driver = login_as(
                            "SPELLCAMP09",
                            self.account_id("SPELLCAMP09"), guid)
                        drift_gate(driver, guid,
                                   len(pilot_spells_for("Druid")))
                        relog_bear = aura_present(driver, guid, 5487)
                        relog_cat = aura_present(driver, guid, 768)
                        relog_note = (
                            f"; before relog bear={bear_before_relog}, "
                            f"after relog bear={relog_bear} "
                            f"cat={relog_cat}")
                        post = snapshot(driver, guid)
                    if (spell == 768
                            and (aura_present(driver, guid, 5487)
                                 or relog_bear)):
                        self.record("Druid", make_verdict(
                            row, "FAIL", phase="apply",
                            evidence="cat aura present but bear aura "
                            "stuck" + relog_note,
                            pre=pre, post=post, packets=ops,
                            target_guid=guid))
                    else:
                        self.record("Druid", make_verdict(
                            row, "PASS",
                            evidence=f"bound SPELL_GO + form aura {aura} "
                            "on self"
                            + (" (bear gone)" if spell == 768 else "")
                            + (f" after {waited} mana wait(s)"
                               if waited else "")
                            + cancel_note
                            + relog_note,
                            pre=pre, post=post, packets=ops,
                            target_guid=guid))
                else:
                    self.record("Druid", make_verdict(
                        row, "FAIL", phase="prepare" if not go else "apply",
                        reason=(f"CAST_RESULT({reason})"
                                if reason is not None else None),
                        evidence=f"go={go} aura{aura}="
                        f"{aura_present(driver, guid, aura)}", pre=pre,
                        post=post, packets=ops, target_guid=guid))

            # Corpse-bucket classification: fork-shaped corpse packet.
            # The parser skips-but-discards corpse GUIDs, so no SPELL_GO
            # is expected. A live ally guid stands in for the corpse
            # guid because the discard happens before any state check;
            # recorded either way, never silently passed.
            row = self.matrix_row("Druid", 20484)
            go, reason, ops = send_and_wait(
                driver, cast_corpse_body(20484, ally_guid), 20484,
                timeout=6.0)
            driver.poll(0.5)
            alive = True
            try:
                driver.client.send_packet(
                    CMSG_SET_ACTIVE_MOVER, struct.pack("<Q", guid))
                driver.poll(0.5)
            except Exception:
                alive = False
            if not go and reason is None:
                self.record("Druid", make_verdict(
                    row, "BLOCKED", reason="wire-target-unsupported",
                    evidence="corpse-mask cast: no bound SPELL_GO or "
                    "CAST_RESULT (parser discards corpse GUIDs); socket "
                    f"alive={alive}", packets=ops,
                    target_guid=ally_guid))
            elif go:
                self.record("Druid", make_verdict(
                    row, "FAIL", phase="apply",
                    evidence="corpse-target cast reached bound SPELL_GO "
                    "against a live stand-in; parser behavior changed, "
                    "but this scenario has no resurrect-request evidence "
                    "and cannot PASS", packets=ops,
                    target_guid=ally_guid))
            else:
                self.record("Druid", make_verdict(
                    row, "FAIL", phase="prepare",
                    reason=f"CAST_RESULT({reason})",
                    evidence="corpse cast answered CAST_RESULT; parser or "
                    "gate changed — needs §8 triage", packets=ops,
                    target_guid=ally_guid))

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
                damage = sum(log["damage"]
                             for log in item.get("damage_logs", []))
                return (f"guid={item.get('guid')} health "
                        f"{item.get('pre_hp')}->{item.get('post_hp')} "
                        f"aura8921={item.get('aura_seen')} "
                        f"matching_damage={damage}")

            reset_ok = bool(reset.get("ok"))
            self.record("Druid", make_verdict(
                row, "PASS" if reset_ok else "FAIL",
                attempts=int(reset.get("attempts", 1)),
                phase=None if reset_ok else "apply",
                evidence="dummy-reset leg: first "
                + describe(first) + "; second " + describe(second),
                pre=(f"first guid={first.get('guid')} "
                     f"health={first.get('pre_hp')}"
                     if first else "no first dummy"),
                post=(f"second guid={second.get('guid')} "
                      f"health={second.get('post_hp')}"
                      if second else "no second dummy"),
                packets=reset.get("packets", []),
                target_guid=first.get("guid")))
        finally:
            smoke.logout(client, "druid-pilot")

    def moonfire_evidence(self, driver: ScriptedDriver, caster_guid: int,
                          dummy: int, *, kill: bool,
                          max_casts: int = 6) -> dict:
        driver.approach_guid(dummy, 13.0)
        pre_hp = driver.target_health(dummy)
        evidence = {
            "guid": dummy, "pre_hp": pre_hp, "post_hp": pre_hp,
            "aura_seen": False, "damage_logs": [], "packets": [],
            "attempts": 0, "go": False, "reason": None,
        }
        for _ in range(max_casts):
            if driver.target_health(dummy) == 0:
                break
            mark = event_mark(driver)
            go, reason, ops = cast_unit_faced(
                driver, 8921, dummy, timeout=15.0)
            evidence["attempts"] += 1
            evidence["go"] = go
            evidence["reason"] = reason
            evidence["packets"] += ops
            for _poll in range(3):
                driver.poll(1.0)
                evidence["aura_seen"] = bool(
                    evidence["aura_seen"]
                    or aura_present(driver, dummy, 8921))
            window = events_since(driver, mark)
            evidence["packets"] += [op for op, _ in window]
            evidence["damage_logs"] += matching_damage_logs(
                window, 8921, dummy, caster_guid)
            evidence["post_hp"] = driver.target_health(dummy)
            if kill and evidence["post_hp"] == 0:
                break
            if (not kill and evidence["post_hp"] is not None
                    and pre_hp is not None
                    and evidence["post_hp"] < pre_hp
                    and (evidence["aura_seen"]
                         or evidence["damage_logs"])):
                break
            if not go:
                driver.poll(1.6)
        evidence["post_hp"] = driver.target_health(dummy)
        return evidence

    def dummy_reset_leg(self, driver: ScriptedDriver,
                        guid: int) -> dict:
        # Leave cat form first (Moonfire is caster-form-only): the
        # stock client cancels shapeshift with CMSG_CANCEL_AURA (310).
        if aura_present(driver, guid, 768) or aura_present(driver, guid, 5487):
            form = 768 if aura_present(driver, guid, 768) else 5487
            driver.client.send_packet(310, struct.pack("<I", form))
            driver.poll(1.5)
        first_guid = live_dummy(driver, 705, WOLF_COUNTERS)
        if first_guid is None:
            return {"ok": False, "first": None, "second": None,
                    "packets": [], "attempts": 0}
        first = self.moonfire_evidence(driver, guid, first_guid, kill=True)
        second_guid = live_dummy(
            driver, 705, WOLF_COUNTERS, exclude=first_guid)
        second = None
        if second_guid is not None:
            second = self.moonfire_evidence(
                driver, guid, second_guid, kill=False, max_casts=3)
        return {
            "ok": reset_evidence_ok(first, second),
            "first": first, "second": second,
            "packets": first["packets"] + (second or {}).get("packets", []),
            "attempts": first["attempts"] + (second or {}).get("attempts", 0),
        }

    def account_id(self, account: str) -> int:
        found = smoke.sql_scalar(
            "realmd", f"SELECT id FROM account WHERE username='{account}'")
        return int(found)


def guid_of(name: str) -> int:
    found = smoke.sql_scalar(
        "characters", f"SELECT guid FROM characters WHERE name='{name}'")
    if not found:
        raise AssertionError(f"no character named {name}; run fixtures first")
    return int(found)


def acquire_lock() -> None:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    if LOCK_PATH.exists():
        try:
            pid = int(LOCK_PATH.read_text().split()[0])
            os.kill(pid, 0)
            raise SystemExit(
                f"campaign lock held by live pid {pid}; refusing to run")
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


def pilot_gate_failures(all_verdicts: dict[str, list[dict]],
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
            failures.append((
                class_name, 0,
                f"row count {len(rows)} != expected {expected}"))
        for verdict in rows:
            if verdict["verdict"] == "PASS":
                continue
            if (verdict["verdict"] == "FAIL"
                    and verdict.get("expected_failure")):
                continue
            if (verdict["verdict"] == "BLOCKED"
                    and verdict["spell_id"] == 20484
                    and verdict.get("reason") == "wire-target-unsupported"):
                continue
            failures.append((
                class_name, int(verdict["spell_id"]),
                f"{verdict['verdict']}: {verdict['evidence']}"))
    return failures


def run_pilot() -> int:
    if not MATRIX_PATH.exists():
        print("matrix missing; run spell_matrix_gen.py --pilot first",
              file=sys.stderr)
        return 2
    acquire_lock()
    pilot = Pilot()
    pilot.manifest_extra["started"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    pilot.manifest_extra["setup_spells"] = {
        name: list(spells) for name, spells in SETUP_SPELLS.items()}
    try:
        pilot.manifest_extra["db_hostname"] = smoke.sql_scalar(
            "realmd", "SELECT @@hostname")
        entry = smoke.sql_scalar(
            "mangos",
            "SELECT COUNT(*) FROM creature_template WHERE entry=327")
        spawns = smoke.sql_scalar(
            "mangos", "SELECT COUNT(*) FROM creature WHERE id=327")
        pilot.manifest_extra["entry_327"] = {
            "template": entry, "spawns": spawns}
        warrior = guid_of("Campwarrior")
        mage = guid_of("Campmage")
        druid = guid_of("Campdruid")
        pilot.manifest_extra["accounts"] = {
            "SPELLCAMP01": pilot.account_id("SPELLCAMP01"),
            "SPELLCAMP07": pilot.account_id("SPELLCAMP07"),
            "SPELLCAMP09": pilot.account_id("SPELLCAMP09"),
        }
        pilot.manifest_extra["guids"] = {
            "Campwarrior": warrior, "Campmage": mage, "Campdruid": druid}
        pilot.manifest_extra["matrix_classes"] = ["Warrior", "Mage", "Druid"]

        seed_character(warrior, pilot_spells_for("Warrior"))
        run_leg_with_log("Warrior", lambda: pilot.leg_warrior(warrior))

        seed_character(mage, pilot_spells_for("Mage"))
        run_leg_with_log("Mage", lambda: pilot.leg_mage(mage))

        # Ally must be offline for the SQL damage write, then online
        # beside the druid at a damaged value.
        seed_character(warrior, pilot_spells_for("Warrior"), pos=ALLY_POS)
        smoke.sql_scalar(
            "characters",
            f"UPDATE characters SET health=300 WHERE guid={warrior}")
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
    write_manifest({**pilot.manifest_extra,
                    "talent_builds": doc.get("talent_builds", {}),
                    "per_class_counts": doc.get("per_class_counts", {}),
                    "matrix_mode": doc.get("mode"),
                    "g1_skillraceclassinfo": doc.get(
                        "g1_skillraceclassinfo", ""),
                    "g2_talent_builds": doc.get("g2_talent_builds", ""),
                    "caveats": [doc.get("g3_isspellvalid", ""),
                                doc.get("g4_script_evidence", ""),
                                doc.get("g5_trainer_equivalence", "")]})
    unexpected = pilot_gate_failures(
        pilot.verdicts, doc.get("per_class_counts", {}))
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
        print("matrix missing; run spell_matrix_gen.py --pilot first",
              file=sys.stderr)
        return 2
    guid = guid_of(char_name)
    doc = json.loads(MATRIX_PATH.read_text())
    rows = [r for r in doc["rows"] if r["spell_id"] == spell_id]
    if not rows:
        print(f"spell {spell_id} not in pilot matrix", file=sys.stderr)
        return 2
    row = rows[0]
    account = {"Warrior": "SPELLCAMP01", "Mage": "SPELLCAMP07",
               "Druid": "SPELLCAMP09"}[row["class"]]
    expected = row.get("expected", {})
    acquire_lock()
    try:
        client, driver = login_as(account, int(smoke.sql_scalar(
            "realmd",
            f"SELECT id FROM account WHERE username='{account}'")), guid)
        try:
            pre = snapshot(driver, guid)
            target: int | None = guid
            pre_hp = None
            if row["bucket"] == "hostile_dummy" or (
                    expected.get("kind") == "expect_cast_failure"):
                target = live_dummy(driver, 705, WOLF_COUNTERS)
                if target is None:
                    verdict = make_verdict(
                        row, "BLOCKED", reason="dummy-unavailable",
                        evidence="no 705 counter with health>0 in "
                        "WorldState", pre=pre)
                    print(json.dumps(verdict, indent=1))
                    return 1
                standoff = 2.0 if row.get("range_index") == 2 else 13.0
                driver.approach_guid(target, standoff)
                pre_hp = driver.target_health(target)
                pre = f"target guid={target} health={pre_hp}; caster {pre}"
                if spell_id == 6546:
                    stance_ok, stance_reason, stance_ops = (
                        ensure_battle_stance(driver, guid))
                    if not stance_ok:
                        verdict = make_verdict(
                            row, "FAIL", phase="prepare",
                            reason=(f"CAST_RESULT({stance_reason})"
                                    if stance_reason is not None else None),
                            evidence="Battle Stance setup did not verify "
                            "aura 2457; Rend not cast",
                            pre=pre, packets=stance_ops,
                            target_guid=target)
                        print(json.dumps(verdict, indent=1))
                        return 1
                mark = event_mark(driver)
                go, reason, _ops = cast_unit_faced(
                    driver, spell_id, target, timeout=15.0)
            else:
                mark = event_mark(driver)
                go, reason, _ops = send_and_wait(
                    driver, cast_self_body(spell_id), spell_id,
                    timeout=15.0, target_guid=guid)
            driver.poll(6.0 if float(row.get("speed") or 0) > 0 else 4.0)
            window = events_since(driver, mark)
            post_hp = (driver.target_health(target)
                       if target is not None and target != guid else None)
            post = snapshot(driver, guid)
            if target is not None and target != guid:
                post = f"target guid={target} health={post_hp}; caster {post}"
            expected_aura = int(expected.get("aura", spell_id))
            verdict = single_outcome_verdict(row, {
                "go": go, "reason": reason,
                "caster_aura": aura_present(driver, guid, expected_aura),
                "target_guid": target,
                "target_aura": (aura_present(driver, target, expected_aura)
                                if target is not None else False),
                "pre_hp": pre_hp, "post_hp": post_hp,
                "damage_logs": (matching_damage_logs(
                    window, spell_id, target, guid)
                    if target is not None else []),
                "pre": pre, "post": post,
                "packets": [op for op, _ in window],
            })
            print(json.dumps(verdict, indent=1))
            return 0 if (verdict["verdict"] == "PASS"
                         or verdict.get("expected_failure")) else 1
        finally:
            smoke.logout(client, f"run-{spell_id}")
    finally:
        release_lock()


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
    assert char_create_body("Campwarrior", 1, 1) == (
        b"Campwarrior\x00" + bytes([1, 1, 0, 0, 0, 0, 0, 0, 0]))
    assert cast_self_body(5242) == struct.pack("<IH", 5242, 0)
    assert cast_unit_body(133, 0xF1300002C1000158) == (
        struct.pack("<IH", 133, 0x0002)
        + smoke.pack_guid(0xF1300002C1000158))
    dest = cast_dest_body(133, 1.0, 2.0, 3.0)
    assert dest == struct.pack("<IH", 133, 0x0040) + struct.pack(
        "<fff", 1.0, 2.0, 3.0)
    corpse = cast_corpse_body(20484, 0xF1300002C1000158)
    assert corpse == (struct.pack("<IH", 20484, 0x0200)
                      + smoke.pack_guid(0xF1300002C1000158))
    assert raw_guid(705, 344) == 0xF1300002C1000158
    # Packet verdicts bind to spell/caster/target bodies, not opcode
    # presence alone (the shipped send_and_wait accepted any 306/304).
    caster = 900017
    dummy = 0xF1300002C1000158
    assert parse_cast_result_body(struct.pack("<IB", 6795, 86)) == (6795, 86)
    assert parse_cast_result_body(
        struct.pack("<IBB", 6795, 2, 86)) == (6795, 86)
    assert parse_cast_result_body(b"\x01") is None
    go_body = (
        smoke.pack_guid(caster) + smoke.pack_guid(dummy)
        + struct.pack("<BIII", 0, 6795, 0, 12345)
        + bytes([1]) + struct.pack("<Q", dummy) + bytes([0]))
    go = parse_spell_go_body(go_body)
    assert go == {
        "caster_guid": caster, "target_guid": dummy,
        "cast_counter": 0, "spell_id": 6795, "hits": [dummy],
        "misses": [],
    }, go
    assert spell_go_matches(go, 6795, caster, dummy)
    assert not spell_go_matches(go, 133, caster, dummy)
    assert not spell_go_matches(go, 6795, 900016, dummy)
    dmg_body = (
        smoke.pack_guid(dummy) + smoke.pack_guid(caster)
        + struct.pack("<III", 133, 46, 0))
    assert parse_damage_log_body(dmg_body) == {
        "target_guid": dummy, "attacker_guid": caster,
        "spell_id": 133, "damage": 46,
    }
    window = [(592, dmg_body), (592, smoke.pack_guid(dummy)
                                + smoke.pack_guid(900016)
                                + struct.pack("<III", 133, 999, 0))]
    assert matching_damage_logs(window, 133, dummy, caster)[0]["damage"] == 46
    assert matching_damage_logs(window, 133, dummy, 900015) == []
    # An expected-failure row is never PASS. A rejection is the gate's
    # expected FAIL; a landed wrong-form cast is an unexpected FAIL.
    fail_row = {"class": "Druid", "spell_id": 6795, "bucket": "form",
                "expected": {"kind": "expect_cast_failure"}}
    rejected = expected_failure_verdict(
        fail_row, False, 86, [304], dummy, attempts=2)
    assert rejected["verdict"] == "FAIL" and rejected["phase"] == "prepare"
    assert rejected["expected_failure"] is True
    assert rejected["target_guid"] == dummy
    landed = expected_failure_verdict(
        fail_row, True, None, [306], dummy, attempts=1)
    assert landed["verdict"] == "FAIL" and landed["expected_failure"] is False
    out_of_range = expected_failure_verdict(
        fail_row, False, 89, [304], dummy, attempts=3)
    assert out_of_range["verdict"] == "FAIL"
    assert out_of_range["expected_failure"] is False
    # Dummy reset needs attributed first-kill and second-target damage;
    # the mere existence of another live counter is not evidence.
    first = {"guid": dummy, "pre_hp": 100, "post_hp": 0,
             "damage_logs": [{"damage": 46}], "aura_seen": False}
    second = {"guid": raw_guid(705, 345), "pre_hp": 100, "post_hp": 54,
              "damage_logs": [], "aura_seen": True}
    assert reset_evidence_ok(first, second)
    assert not reset_evidence_ok(first, {**second, "post_hp": 100})
    assert not reset_evidence_ok(first, {**second, "guid": dummy})
    assert not reset_evidence_ok(
        {**first, "damage_logs": []}, second)
    # Single-spell repro uses the row's observable outcome; SPELL_GO
    # alone (the shipped run_single rule) cannot produce PASS.
    fireball = {"class": "Mage", "spell_id": 133,
                "bucket": "hostile_dummy",
                "expected": {"kind": "damage_log", "opcode": 592}}
    unobserved = single_outcome_verdict(fireball, {
        "go": True, "reason": None, "caster_aura": False,
        "target_guid": dummy, "target_aura": False,
        "pre_hp": 100, "post_hp": 54, "damage_logs": [],
        "packets": [306],
    })
    assert unobserved["verdict"] == "FAIL", unobserved
    observed = single_outcome_verdict(fireball, {
        "go": True, "reason": None, "caster_aura": False,
        "target_guid": dummy, "target_aura": False,
        "pre_hp": 100, "post_hp": 54,
        "damage_logs": [{"damage": 46}], "packets": [306, 592],
    })
    assert observed["verdict"] == "PASS", observed
    # Pilot gate: only the expected shapeshift FAIL and pinned corpse
    # BLOCKED are compatible; a core-row BLOCKED stops the gate.
    gate_rows = {
        "Warrior": [
            make_verdict({"class": "Warrior", "spell_id": 5242,
                          "bucket": "self"}, "PASS", evidence="ok"),
            make_verdict({"class": "Warrior", "spell_id": 6546,
                          "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
        "Mage": [
            make_verdict({"class": "Mage", "spell_id": 168,
                          "bucket": "self"}, "PASS", evidence="ok"),
            make_verdict({"class": "Mage", "spell_id": 133,
                          "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
        "Druid": [
            make_verdict({"class": "Druid", "spell_id": 5187,
                          "bucket": "friendly"}, "PASS", evidence="ok"),
            rejected,
            make_verdict({"class": "Druid", "spell_id": 5487,
                          "bucket": "form"}, "PASS", evidence="ok"),
            make_verdict({"class": "Druid", "spell_id": 768,
                          "bucket": "form"}, "PASS", evidence="ok"),
            make_verdict({"class": "Druid", "spell_id": 20484,
                          "bucket": "corpse"}, "BLOCKED",
                         reason="wire-target-unsupported", evidence="ok"),
            make_verdict({"class": "Druid", "spell_id": 8921,
                          "bucket": "hostile_dummy"}, "PASS", evidence="ok"),
        ],
    }
    assert pilot_gate_failures(gate_rows) == []
    gate_rows["Warrior"][1] = make_verdict(
        {"class": "Warrior", "spell_id": 6546,
         "bucket": "hostile_dummy"}, "BLOCKED",
        reason="dummy-unavailable", evidence="no dummy")
    assert pilot_gate_failures(gate_rows)
    gate_rows["Druid"][1] = landed
    assert any(spell == 6795 for _, spell, _ in
               pilot_gate_failures(gate_rows))
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
    parser.add_argument("command",
                        choices=["preflight", "fixtures", "pilot", "run"],
                        nargs="?", default="pilot")
    parser.add_argument("--spell", type=int, default=None)
    parser.add_argument("--char", default=None)
    parser.add_argument("--self-test", action="store_true",
                        help="offline invariant checks, no server needed")
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
    guard_loopback()
    return run_pilot()


if __name__ == "__main__":
    sys.exit(main())
