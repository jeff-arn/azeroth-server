#!/usr/bin/env python3
"""Login-train completeness audit for the Rust mangosd.

Captures every SMSG from ``CMSG_PLAYER_LOGIN`` through the
initial object-update burst (plus a short tail window) and
checks the ordered milestones that the C++ server emits in
``WorldSession::HandlePlayerLogin``
(``CharacterHandler.cpp:422-632``) together with
``Player::SendInitialPacketsBeforeAddToMap``
(``Player.cpp:18905-18944``), ``Map::Add(Player*)``
(``Map.cpp:401-441``), and
``Player::SendInitialPacketsAfterAddToMap``
(``Player.cpp:18946-18981``):

  SMSG_LOGIN_VERIFY_WORLD       HandlePlayerLogin
  SMSG_ACCOUNT_DATA_MD5         SendAccountDataTimes
  SMSG_FRIEND_LIST              PlayerSocial::SendFriendList
  SMSG_IGNORE_LIST              PlayerSocial::SendIgnoreList
  SMSG_MESSAGECHAT (MOTD, >=1)  HandlePlayerLogin MOTD loop
  SMSG_SET_REST_START           SendInitialPacketsBeforeAddToMap
  SMSG_BINDPOINTUPDATE          SendInitialPacketsBeforeAddToMap
  SMSG_TUTORIAL_FLAGS           SendTutorialsData
  SMSG_INITIAL_SPELLS           SendInitialSpells
  SMSG_ACTION_BUTTONS           MasterPlayer (sent twice in C++:
                                once inside ...BeforeAddToMap and
                                again at CharacterHandler.cpp:602)
  SMSG_INITIALIZE_FACTIONS      ReputationMgr
  SMSG_LOGIN_SETTIMESPEED       SendInitialPacketsBeforeAddToMap
  update burst (169/502)        Map::Add: SendInitSelf/visibility
  SMSG_INIT_WORLD_STATES        UpdateZone (m_zoneUpdateId starts
                                at 0, so a fresh login always
                                enters a "new" zone)
  SMSG_WEATHER                  UpdateZone, only when the
                                ActivateWeather config is true
                                (the C++ default; the bench conf
                                does not disable it)

``HonorMgr::Update`` inside ``...BeforeAddToMap`` writes
update fields only and sends no packet, so it has no
milestone. Conditional train packets (guild MOTD, account
warning, corpse-reclaim delay, first-login cinematic,
timed enchant/item durations) are reported when present but
never required: they depend on character/account state, not
on the train itself.

Known gaps at audit-writing time (verified against
``world_actor.rs`` and ``player_login.rs``, which have no
builders/senders for them): SMSG_INITIAL_SPELLS,
SMSG_ACTION_BUTTONS, SMSG_INITIALIZE_FACTIONS,
SMSG_INIT_WORLD_STATES, SMSG_WEATHER. The default mode
reports them as KNOWN-GAP and passes; ``--strict`` fails
while any known gap remains. Missing or out-of-order
*required* packets always fail. MOTD line count and the
interleaving of map-driven packets (extra update bursts,
monster moves, aura-duration updates) are tolerated; the
INIT_WORLD_STATES/weather tail may land before or after the
first burst packet as long as both follow SETTIMESPEED.

Account-data *request* handling (CMSG_REQUEST_ACCOUNT_DATA
-> SMSG_UPDATE_ACCOUNT_DATA, MiscHandler.cpp:897) is not
part of the HandlePlayerLogin push train: the client asks
only after comparing the SMSG_ACCOUNT_DATA_MD5 digests.
The audit probes it separately after the train capture and
reports it as ADJACENT; it never affects the train verdict.
(Rust currently drops it in the world actor's execute stub;
the write side, CMSG_UPDATE_ACCOUNT_DATA, is a Phase 6 save
path and is never probed.)

Expects mangosd on 127.0.0.1:18099 with the bench conf.
Defaults to JEFF/Ruikanis (the phase3 fixture), deliberately
not TESTUSER/Groktar, which the real-client track uses.
Override with --account/--account-id/--guid/--host/--port
or the LOGIN_TRAIN_* environment variables. The character
must be offline; the audit refuses to log in otherwise
(set LOGIN_TRAIN_FORCE=1 to override, at your own risk of
colliding with a live session).

Exit codes: 0 = train OK (known gaps allowed unless
--strict), 1 = audit failure, 2 = setup/connection error.
Prints LOGIN-TRAIN-PASS, LOGIN-TRAIN-PASS-WITH-KNOWN-GAPS,
or LOGIN-TRAIN-FAIL. ``--self-test`` exercises the pure
ordering logic offline and prints LOGIN-TRAIN-SELFTEST-PASS.

Run from the repository root.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import select
import struct
import subprocess
import sys
import time
from dataclasses import dataclass, field

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import phase3_scripted_login as p3  # noqa: E402
from phase3_scripted_login import Client  # noqa: E402

CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
SMSG_CHARACTER_LOGIN_FAILED = 65
SMSG_LOGIN_SETTIMESPEED = 66
CMSG_LOGOUT_REQUEST = 75
SMSG_LOGOUT_COMPLETE = 77
SMSG_FRIEND_LIST = 103
SMSG_IGNORE_LIST = 107
SMSG_GUILD_EVENT = 146
SMSG_MESSAGECHAT = 150
SMSG_UPDATE_OBJECT = 169
SMSG_MONSTER_MOVE = 221
SMSG_TRIGGER_CINEMATIC = 250
SMSG_TUTORIAL_FLAGS = 253
SMSG_INITIALIZE_FACTIONS = 290
SMSG_ACTION_BUTTONS = 297
SMSG_INITIAL_SPELLS = 298
SMSG_UPDATE_AURA_DURATION = 311
SMSG_BINDPOINTUPDATE = 341
SMSG_NOTIFICATION = 459
SMSG_ITEM_TIME_UPDATE = 490
SMSG_ITEM_ENCHANT_TIME_UPDATE = 491
SMSG_COMPRESSED_UPDATE_OBJECT = 502
SMSG_ACCOUNT_DATA_MD5 = 521
CMSG_REQUEST_ACCOUNT_DATA = 522
SMSG_UPDATE_ACCOUNT_DATA = 524
SMSG_SET_REST_START = 542
SMSG_LOGIN_VERIFY_WORLD = 566
SMSG_CORPSE_RECLAIM_DELAY = 617
SMSG_INIT_WORLD_STATES = 706
SMSG_WEATHER = 756

OPCODE_NAMES = {
    SMSG_CHARACTER_LOGIN_FAILED: "SMSG_CHARACTER_LOGIN_FAILED",
    SMSG_LOGIN_SETTIMESPEED: "SMSG_LOGIN_SETTIMESPEED",
    SMSG_LOGOUT_COMPLETE: "SMSG_LOGOUT_COMPLETE",
    SMSG_FRIEND_LIST: "SMSG_FRIEND_LIST",
    SMSG_IGNORE_LIST: "SMSG_IGNORE_LIST",
    SMSG_GUILD_EVENT: "SMSG_GUILD_EVENT",
    SMSG_MESSAGECHAT: "SMSG_MESSAGECHAT",
    SMSG_UPDATE_OBJECT: "SMSG_UPDATE_OBJECT",
    SMSG_MONSTER_MOVE: "SMSG_MONSTER_MOVE",
    SMSG_TRIGGER_CINEMATIC: "SMSG_TRIGGER_CINEMATIC",
    SMSG_TUTORIAL_FLAGS: "SMSG_TUTORIAL_FLAGS",
    SMSG_INITIALIZE_FACTIONS: "SMSG_INITIALIZE_FACTIONS",
    SMSG_ACTION_BUTTONS: "SMSG_ACTION_BUTTONS",
    SMSG_INITIAL_SPELLS: "SMSG_INITIAL_SPELLS",
    SMSG_UPDATE_AURA_DURATION: "SMSG_UPDATE_AURA_DURATION",
    SMSG_BINDPOINTUPDATE: "SMSG_BINDPOINTUPDATE",
    SMSG_NOTIFICATION: "SMSG_NOTIFICATION",
    SMSG_ITEM_TIME_UPDATE: "SMSG_ITEM_TIME_UPDATE",
    SMSG_ITEM_ENCHANT_TIME_UPDATE: "SMSG_ITEM_ENCHANT_TIME_UPDATE",
    SMSG_COMPRESSED_UPDATE_OBJECT: "SMSG_COMPRESSED_UPDATE_OBJECT",
    SMSG_ACCOUNT_DATA_MD5: "SMSG_ACCOUNT_DATA_MD5",
    SMSG_UPDATE_ACCOUNT_DATA: "SMSG_UPDATE_ACCOUNT_DATA",
    SMSG_SET_REST_START: "SMSG_SET_REST_START",
    SMSG_LOGIN_VERIFY_WORLD: "SMSG_LOGIN_VERIFY_WORLD",
    SMSG_CORPSE_RECLAIM_DELAY: "SMSG_CORPSE_RECLAIM_DELAY",
    SMSG_INIT_WORLD_STATES: "SMSG_INIT_WORLD_STATES",
    SMSG_WEATHER: "SMSG_WEATHER",
}


def opcode_label(opcode: int) -> str:
    return OPCODE_NAMES.get(opcode, f"OPCODE-{opcode}")


REQUIRED = "required"
KNOWN_GAP = "known_gap"
CONDITIONAL = "conditional"


@dataclass(frozen=True)
class Milestone:
    name: str
    opcodes: tuple[int, ...]
    kind: str
    # Exact body length when the C++ builder has a fixed
    # size; otherwise a minimum plausible length.
    exact_len: int | None = None
    min_len: int = 0
    note: str = ""


# The strictly ordered chain up to SETTIMESPEED (C++ send
# order). MAX_ACTION_BUTTONS is 120 (Player.h:166), hence
# 480 bytes; INITIALIZE_FACTIONS is 4 + 64*5 = 324.
CHAIN: tuple[Milestone, ...] = (
    Milestone("SMSG_LOGIN_VERIFY_WORLD", (SMSG_LOGIN_VERIFY_WORLD,), REQUIRED, exact_len=20),
    Milestone("SMSG_ACCOUNT_DATA_MD5", (SMSG_ACCOUNT_DATA_MD5,), REQUIRED, exact_len=128),
    Milestone("SMSG_FRIEND_LIST", (SMSG_FRIEND_LIST,), REQUIRED, min_len=1),
    Milestone("SMSG_IGNORE_LIST", (SMSG_IGNORE_LIST,), REQUIRED, min_len=1),
    Milestone("MOTD(SMSG_MESSAGECHAT>=1)", (SMSG_MESSAGECHAT,), REQUIRED, min_len=1),
    Milestone("SMSG_SET_REST_START", (SMSG_SET_REST_START,), REQUIRED, exact_len=4),
    Milestone("SMSG_BINDPOINTUPDATE", (SMSG_BINDPOINTUPDATE,), REQUIRED, exact_len=20),
    Milestone("SMSG_TUTORIAL_FLAGS", (SMSG_TUTORIAL_FLAGS,), REQUIRED, exact_len=32),
    Milestone("SMSG_INITIAL_SPELLS", (SMSG_INITIAL_SPELLS,), KNOWN_GAP, min_len=5),
    Milestone(
        "SMSG_ACTION_BUTTONS",
        (SMSG_ACTION_BUTTONS,),
        KNOWN_GAP,
        exact_len=480,
        note="C++ sends it twice on this path; >=1 required",
    ),
    Milestone("SMSG_INITIALIZE_FACTIONS", (SMSG_INITIALIZE_FACTIONS,), KNOWN_GAP, exact_len=324),
    Milestone("SMSG_LOGIN_SETTIMESPEED", (SMSG_LOGIN_SETTIMESPEED,), REQUIRED, exact_len=8),
)

# After SETTIMESPEED the map burst and the AfterAddToMap
# zone packets may interleave in either order, but none of
# them may precede SETTIMESPEED.
TAIL: tuple[Milestone, ...] = (
    Milestone(
        "UPDATE-BURST(169|502)",
        (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT),
        REQUIRED,
        min_len=5,
    ),
    Milestone("SMSG_INIT_WORLD_STATES", (SMSG_INIT_WORLD_STATES,), KNOWN_GAP, min_len=10),
    Milestone(
        "SMSG_WEATHER",
        (SMSG_WEATHER,),
        KNOWN_GAP,
        exact_len=13,
        note="expected while ActivateWeather=true (C++ default)",
    ),
)

# State-dependent train packets: reported, never required.
CONDITIONALS: tuple[Milestone, ...] = (
    Milestone("SMSG_GUILD_EVENT", (SMSG_GUILD_EVENT,), CONDITIONAL, note="guilded characters only"),
    Milestone("SMSG_NOTIFICATION", (SMSG_NOTIFICATION,), CONDITIONAL, note="warning/GM/shutdown only"),
    Milestone("SMSG_CORPSE_RECLAIM_DELAY", (SMSG_CORPSE_RECLAIM_DELAY,), CONDITIONAL, note="dead with corpse only"),
    Milestone("SMSG_TRIGGER_CINEMATIC", (SMSG_TRIGGER_CINEMATIC,), CONDITIONAL, note="first login only"),
    Milestone(
        "SMSG_ITEM_ENCHANT_TIME_UPDATE", (SMSG_ITEM_ENCHANT_TIME_UPDATE,), CONDITIONAL, note="timed enchants only"
    ),
    Milestone("SMSG_ITEM_TIME_UPDATE", (SMSG_ITEM_TIME_UPDATE,), CONDITIONAL, note="duration items only"),
)


@dataclass
class AuditResult:
    failures: list[str] = field(default_factory=list)
    missing_required: list[str] = field(default_factory=list)
    missing_known_gaps: list[str] = field(default_factory=list)
    order_violations: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    present_conditionals: list[str] = field(default_factory=list)
    motd_lines: int = 0
    burst_packets: int = 0
    action_button_copies: int = 0

    def passed(self, strict: bool) -> bool:
        if self.failures or self.missing_required or self.order_violations:
            return False
        return not (strict and self.missing_known_gaps)


def _first_index(captured: list[tuple[int, int]], opcodes: tuple[int, ...]) -> int | None:
    for index, (opcode, _length) in enumerate(captured):
        if opcode in opcodes:
            return index
    return None


def _count(captured: list[tuple[int, int]], opcodes: tuple[int, ...]) -> int:
    return sum(1 for opcode, _length in captured if opcode in opcodes)


def _check_body(result: AuditResult, milestone: Milestone, captured: list[tuple[int, int]], index: int) -> None:
    _opcode, length = captured[index]
    if milestone.exact_len is not None and length != milestone.exact_len:
        result.failures.append(f"{milestone.name}: body is {length} bytes, expected {milestone.exact_len}")
    elif length < milestone.min_len:
        result.failures.append(f"{milestone.name}: body is {length} bytes, expected at least {milestone.min_len}")


def audit_packets(captured: list[tuple[int, int]], expect_weather: bool = True) -> AuditResult:
    """Pure ordering/completeness check over (opcode, body_len) pairs."""
    result = AuditResult()
    result.motd_lines = _count(captured, (SMSG_MESSAGECHAT,))
    result.burst_packets = _count(captured, (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT))
    result.action_button_copies = _count(captured, (SMSG_ACTION_BUTTONS,))

    if _first_index(captured, (SMSG_CHARACTER_LOGIN_FAILED,)) is not None:
        result.failures.append("SMSG_CHARACTER_LOGIN_FAILED received; no train follows a refused login")

    # Ordered chain: first occurrences of present milestones
    # must be strictly increasing.
    previous: tuple[str, int] | None = None
    for milestone in CHAIN:
        index = _first_index(captured, milestone.opcodes)
        if index is None:
            if milestone.kind == REQUIRED:
                result.missing_required.append(milestone.name)
            else:
                result.missing_known_gaps.append(milestone.name)
            continue
        _check_body(result, milestone, captured, index)
        if previous is not None and index <= previous[1]:
            result.order_violations.append(
                f"{milestone.name} (packet {index}) does not follow {previous[0]} (packet {previous[1]})"
            )
        else:
            previous = (milestone.name, index)
        copies = _count(captured, milestone.opcodes)
        if copies > 1 and milestone.opcodes not in ((SMSG_MESSAGECHAT,), (SMSG_ACTION_BUTTONS,)):
            result.warnings.append(f"{milestone.name} arrived {copies} times; C++ sends it once")
    if result.action_button_copies > 2:
        result.warnings.append(
            f"SMSG_ACTION_BUTTONS arrived {result.action_button_copies} times; C++ sends it twice on this path"
        )

    # Tail: unordered among themselves, all after SETTIMESPEED.
    speed_index = _first_index(captured, (SMSG_LOGIN_SETTIMESPEED,))
    for milestone in TAIL:
        if milestone.name == "SMSG_WEATHER" and not expect_weather:
            if _first_index(captured, milestone.opcodes) is not None:
                result.present_conditionals.append(milestone.name)
            continue
        index = _first_index(captured, milestone.opcodes)
        if index is None:
            if milestone.kind == REQUIRED:
                result.missing_required.append(milestone.name)
            else:
                result.missing_known_gaps.append(milestone.name)
            continue
        _check_body(result, milestone, captured, index)
        if speed_index is not None and index <= speed_index:
            result.order_violations.append(
                f"{milestone.name} (packet {index}) precedes SMSG_LOGIN_SETTIMESPEED (packet {speed_index})"
            )

    for milestone in CONDITIONALS:
        if _first_index(captured, milestone.opcodes) is not None:
            result.present_conditionals.append(milestone.name)
    return result


def print_report(
    captured: list[tuple[int, int]], result: AuditResult, strict: bool, expect_weather: bool = True
) -> None:
    sequence = ",".join(opcode_label(opcode) for opcode, _length in captured)
    print(f"CAPTURED {len(captured)} packets: {sequence}")
    for milestone in (*CHAIN, *TAIL):
        index = _first_index(captured, milestone.opcodes)
        if milestone.name == "SMSG_WEATHER" and not expect_weather:
            state = "CONDITIONAL-PRESENT" if index is not None else "CONDITIONAL-ABSENT (not expected)"
            print(f"{state} {milestone.name}")
        elif index is not None:
            print(f"MILESTONE-OK {milestone.name} at packet {index}")
        elif milestone.kind == REQUIRED:
            print(f"MILESTONE-MISSING {milestone.name} (required)")
        else:
            print(f"KNOWN-GAP {milestone.name} absent ({milestone.note or 'not implemented in Rust yet'})")
    for name in result.present_conditionals:
        if name != "SMSG_WEATHER":
            print(f"CONDITIONAL-PRESENT {name}")
    for violation in result.order_violations:
        print(f"ORDER-VIOLATION: {violation}")
    for failure in result.failures:
        print(f"FAILURE: {failure}")
    for warning in result.warnings:
        print(f"WARNING: {warning}")
    print(
        "TRAIN-SUMMARY "
        f"motd_lines={result.motd_lines} burst_packets={result.burst_packets} "
        f"action_button_copies={result.action_button_copies} "
        f"missing_required={result.missing_required} "
        f"known_gaps={result.missing_known_gaps} "
        f"conditionals_present={result.present_conditionals} strict={str(strict).lower()}"
    )


def _env_int(name: str, default: int) -> int:
    return int(os.environ.get(name, str(default)))


def sql_scalar(db: str, stmt: str) -> str:
    completed = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{p3.bench_password()}", "-N", "-B", db, "-e", stmt],
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"mysql query failed for {db}")
    return completed.stdout.strip()


def ensure_character_offline(guid: int) -> None:
    """Refuse to collide with a live session (same guard as
    update_burst_validator, with this tool's own force flag)."""
    if os.environ.get("LOGIN_TRAIN_FORCE") == "1":
        return
    online = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
    if online == "1":
        raise RuntimeError(f"character guid {guid} is marked online; aborting scripted login")


def capture_train(client: Client, max_secs: float, idle_secs: float) -> list[tuple[int, int]]:
    """Read every packet until the burst has been seen and
    the stream goes idle, or the capture window ends. The
    idle tail matters: INIT_WORLD_STATES follows the burst
    in C++, so stopping at the first update would miss it."""
    captured: list[tuple[int, int]] = []
    client.sock.settimeout(5)
    deadline = time.time() + max_secs
    last_packet_at = time.time()
    while time.time() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if ready:
            opcode, body = client.read_packet()
            captured.append((opcode, len(body)))
            last_packet_at = time.time()
            continue
        saw_verify = _first_index(captured, (SMSG_LOGIN_VERIFY_WORLD,)) is not None
        saw_burst = _first_index(captured, (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT)) is not None
        if saw_verify and saw_burst and time.time() - last_packet_at >= idle_secs:
            break
    return captured


def probe_account_data_request(client: Client, wait_secs: float = 2.0) -> bool:
    """Adjacent (non-train) probe: ask for account-data type
    0 and report whether SMSG_UPDATE_ACCOUNT_DATA answers.
    Read-only; never sends CMSG_UPDATE_ACCOUNT_DATA."""
    client.send_packet(CMSG_REQUEST_ACCOUNT_DATA, struct.pack("<I", 0))
    client.sock.settimeout(5)
    deadline = time.time() + wait_secs
    while time.time() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if not ready:
            continue
        opcode, _body = client.read_packet()
        if opcode == SMSG_UPDATE_ACCOUNT_DATA:
            return True
    return False


def logout_and_close(client: Client | None) -> None:
    if client is None:
        return
    try:
        client.send_packet(CMSG_LOGOUT_REQUEST, b"")
        client.sock.settimeout(5)
        deadline = time.time() + 3
        while time.time() < deadline:
            ready, _, _ = select.select([client.sock], [], [], 0.25)
            if not ready:
                continue
            opcode, _body = client.read_packet()
            if opcode == SMSG_LOGOUT_COMPLETE:
                break
    except Exception:
        pass
    finally:
        client.sock.close()


def run_live(args: argparse.Namespace) -> int:
    p3.ACCOUNT = args.account
    p3.ACCOUNT_ID = args.account_id
    p3.HOST = args.host
    p3.PORT = args.port
    try:
        ensure_character_offline(args.guid)
        p3.setup_account()
    except Exception as exc:
        print(f"LOGIN-TRAIN-SETUP-ERROR: {exc}")
        return 2

    client: Client | None = None
    logged_in = False
    try:
        client = Client()
        client.auth()
        client.send_packet(CMSG_CHAR_ENUM, b"")
        opcode, _body = client.read_packet()
        if opcode != SMSG_CHAR_ENUM:
            print(f"LOGIN-TRAIN-SETUP-ERROR: expected SMSG_CHAR_ENUM, got {opcode_label(opcode)}")
            return 2
        client.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", args.guid))
        logged_in = True
        captured = capture_train(client, args.capture_secs, args.idle_secs)
        if args.no_account_data_probe:
            print("ADJACENT-ACCOUNT-DATA-REQUEST: SKIPPED (--no-account-data-probe)")
        elif probe_account_data_request(client):
            print("ADJACENT-ACCOUNT-DATA-REQUEST: OK (SMSG_UPDATE_ACCOUNT_DATA answered; not part of the train)")
        else:
            print(
                "ADJACENT-ACCOUNT-DATA-REQUEST: GAP (no SMSG_UPDATE_ACCOUNT_DATA; "
                "client-initiated, not part of the HandlePlayerLogin train)"
            )
    except Exception as exc:
        print(f"LOGIN-TRAIN-SETUP-ERROR: {exc}")
        return 2
    finally:
        if logged_in:
            logout_and_close(client)
        elif client is not None:
            client.sock.close()

    expect_weather = not args.no_expect_weather
    result = audit_packets(captured, expect_weather=expect_weather)
    print_report(captured, result, strict=args.strict, expect_weather=expect_weather)
    if not result.passed(args.strict):
        if args.strict and not (result.failures or result.missing_required or result.order_violations):
            print("LOGIN-TRAIN-FAIL (strict: known gaps remain)")
        else:
            print("LOGIN-TRAIN-FAIL")
        return 1
    if result.missing_known_gaps:
        print("LOGIN-TRAIN-PASS-WITH-KNOWN-GAPS")
    else:
        print("LOGIN-TRAIN-PASS")
    return 0


# Correct body lengths for synthetic self-test trains.
_SELFTEST_LENS = {
    SMSG_LOGIN_VERIFY_WORLD: 20,
    SMSG_ACCOUNT_DATA_MD5: 128,
    SMSG_FRIEND_LIST: 1,
    SMSG_IGNORE_LIST: 1,
    SMSG_MESSAGECHAT: 30,
    SMSG_SET_REST_START: 4,
    SMSG_BINDPOINTUPDATE: 20,
    SMSG_TUTORIAL_FLAGS: 32,
    SMSG_INITIAL_SPELLS: 9,
    SMSG_ACTION_BUTTONS: 480,
    SMSG_INITIALIZE_FACTIONS: 324,
    SMSG_LOGIN_SETTIMESPEED: 8,
    SMSG_UPDATE_OBJECT: 100,
    SMSG_COMPRESSED_UPDATE_OBJECT: 100,
    SMSG_INIT_WORLD_STATES: 16,
    SMSG_WEATHER: 13,
}


def _synthetic(opcodes: list[int]) -> list[tuple[int, int]]:
    return [(opcode, _SELFTEST_LENS.get(opcode, 0)) for opcode in opcodes]


def run_self_test() -> int:
    full_cpp = _synthetic(
        [
            SMSG_LOGIN_VERIFY_WORLD,
            SMSG_ACCOUNT_DATA_MD5,
            SMSG_FRIEND_LIST,
            SMSG_IGNORE_LIST,
            SMSG_MESSAGECHAT,
            SMSG_MESSAGECHAT,
            SMSG_SET_REST_START,
            SMSG_BINDPOINTUPDATE,
            SMSG_TUTORIAL_FLAGS,
            SMSG_INITIAL_SPELLS,
            SMSG_ACTION_BUTTONS,
            SMSG_INITIALIZE_FACTIONS,
            SMSG_LOGIN_SETTIMESPEED,
            SMSG_ACTION_BUTTONS,  # C++ duplicate at CharacterHandler.cpp:602
            SMSG_UPDATE_OBJECT,
            SMSG_INIT_WORLD_STATES,
            SMSG_WEATHER,
        ]
    )
    result = audit_packets(full_cpp)
    assert result.passed(strict=True), f"full C++ train must pass strict: {result}"
    assert result.motd_lines == 2 and result.action_button_copies == 2
    print("SELFTEST full-cpp-train strict-pass")

    current_rust = _synthetic(
        [
            SMSG_LOGIN_VERIFY_WORLD,
            SMSG_ACCOUNT_DATA_MD5,
            SMSG_FRIEND_LIST,
            SMSG_IGNORE_LIST,
            SMSG_MESSAGECHAT,
            SMSG_MESSAGECHAT,
            SMSG_SET_REST_START,
            SMSG_BINDPOINTUPDATE,
            SMSG_TUTORIAL_FLAGS,
            SMSG_LOGIN_SETTIMESPEED,
            SMSG_UPDATE_OBJECT,
            SMSG_COMPRESSED_UPDATE_OBJECT,
        ]
    )
    result = audit_packets(current_rust)
    assert result.passed(strict=False), f"current Rust train must pass default: {result}"
    assert not result.passed(strict=True), "current Rust train must fail strict"
    assert result.missing_known_gaps == [
        "SMSG_INITIAL_SPELLS",
        "SMSG_ACTION_BUTTONS",
        "SMSG_INITIALIZE_FACTIONS",
        "SMSG_INIT_WORLD_STATES",
        "SMSG_WEATHER",
    ], result.missing_known_gaps
    print("SELFTEST current-rust-train default-pass strict-fail")

    result = audit_packets(current_rust, expect_weather=False)
    assert "SMSG_WEATHER" not in result.missing_known_gaps
    print("SELFTEST weather-downgrade")

    missing_tutorial = [(op, n) for op, n in current_rust if op != SMSG_TUTORIAL_FLAGS]
    result = audit_packets(missing_tutorial)
    assert not result.passed(strict=False) and result.missing_required == ["SMSG_TUTORIAL_FLAGS"], result
    print("SELFTEST missing-required fails")

    swapped = _synthetic(
        [
            SMSG_LOGIN_VERIFY_WORLD,
            SMSG_ACCOUNT_DATA_MD5,
            SMSG_IGNORE_LIST,
            SMSG_FRIEND_LIST,
            SMSG_MESSAGECHAT,
            SMSG_SET_REST_START,
            SMSG_BINDPOINTUPDATE,
            SMSG_TUTORIAL_FLAGS,
            SMSG_LOGIN_SETTIMESPEED,
            SMSG_UPDATE_OBJECT,
        ]
    )
    result = audit_packets(swapped)
    assert not result.passed(strict=False) and result.order_violations, result
    print("SELFTEST out-of-order fails")

    interleaved = _synthetic(
        [
            SMSG_LOGIN_VERIFY_WORLD,
            SMSG_ACCOUNT_DATA_MD5,
            SMSG_FRIEND_LIST,
            SMSG_IGNORE_LIST,
            SMSG_MESSAGECHAT,
            SMSG_SET_REST_START,
            SMSG_BINDPOINTUPDATE,
            SMSG_TUTORIAL_FLAGS,
            SMSG_INITIAL_SPELLS,
            SMSG_ACTION_BUTTONS,
            SMSG_INITIALIZE_FACTIONS,
            SMSG_LOGIN_SETTIMESPEED,
            SMSG_INIT_WORLD_STATES,
            SMSG_MONSTER_MOVE,
            SMSG_UPDATE_OBJECT,
            SMSG_UPDATE_AURA_DURATION,
            SMSG_WEATHER,
            SMSG_COMPRESSED_UPDATE_OBJECT,
        ]
    )
    result = audit_packets(interleaved)
    assert result.passed(strict=True), f"map interleaving must be tolerated: {result}"
    print("SELFTEST interleaving tolerated")

    early_burst = _synthetic(
        [
            SMSG_LOGIN_VERIFY_WORLD,
            SMSG_ACCOUNT_DATA_MD5,
            SMSG_FRIEND_LIST,
            SMSG_IGNORE_LIST,
            SMSG_MESSAGECHAT,
            SMSG_SET_REST_START,
            SMSG_BINDPOINTUPDATE,
            SMSG_TUTORIAL_FLAGS,
            SMSG_UPDATE_OBJECT,
            SMSG_LOGIN_SETTIMESPEED,
        ]
    )
    result = audit_packets(early_burst)
    assert not result.passed(strict=False) and result.order_violations, result
    print("SELFTEST burst-before-timespeed fails")

    bad_body = [(op, 16 if op == SMSG_TUTORIAL_FLAGS else n) for op, n in current_rust]
    result = audit_packets(bad_body)
    assert not result.passed(strict=False) and result.failures, result
    print("SELFTEST wrong-body-size fails")

    refused = _synthetic([SMSG_CHARACTER_LOGIN_FAILED])
    result = audit_packets(refused)
    assert not result.passed(strict=False) and result.failures and result.missing_required, result
    print("SELFTEST login-failed fails")

    print("LOGIN-TRAIN-SELFTEST-PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--strict", action="store_true", help="fail while known gaps remain")
    parser.add_argument("--self-test", action="store_true", help="run offline ordering tests, no server needed")
    parser.add_argument("--no-account-data-probe", action="store_true", help="skip the adjacent account-data probe")
    parser.add_argument(
        "--no-expect-weather",
        action="store_true",
        help="treat SMSG_WEATHER as conditional (servers with ActivateWeather=0)",
    )
    parser.add_argument("--account", default=os.environ.get("LOGIN_TRAIN_ACCOUNT", "JEFF"))
    parser.add_argument("--account-id", type=int, default=_env_int("LOGIN_TRAIN_ACCOUNT_ID", 5))
    parser.add_argument("--guid", type=int, default=_env_int("LOGIN_TRAIN_GUID", 1))
    parser.add_argument("--host", default=os.environ.get("LOGIN_TRAIN_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=_env_int("LOGIN_TRAIN_PORT", 18099))
    parser.add_argument("--capture-secs", type=float, default=float(os.environ.get("LOGIN_TRAIN_CAPTURE_SECS", "10")))
    parser.add_argument("--idle-secs", type=float, default=float(os.environ.get("LOGIN_TRAIN_IDLE_SECS", "2.0")))
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    return run_live(args)


if __name__ == "__main__":
    sys.exit(main())
