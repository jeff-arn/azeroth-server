#!/usr/bin/env python3
"""Two-client scripted smoke: shared world + movement relay.

Proves, over real sockets against the running Rust mangosd,
that two sessions on different accounts can enter the same
map, see each other, and receive each other's relayed
movement. This is the session/socket counterpart of the
in-process map test
``mangos-map/tests/live_map.rs::bench_map_visibility_and_relay``
(which drives ``Map`` directly and needs live DB env vars);
this script additionally covers world-actor routing and
the AuthCrypt packet path in both directions.

Single-client world behavior (casts, attacks, emotes,
gameobject use) is already covered by
``phase4_scripted_world.py`` and deliberately not repeated
here. Login-train completeness is ``login_train_audit.py``;
this script only drains the train loosely until the first
update burst.

Bench fixtures: BOT_Zulrath/Zulrath (guid 3) and
BOT_Morka/Morka (guid 4) share the exact Durotar spawn
point on map 1, so mutual visibility needs no teleporting.
TESTUSER/Groktar is avoided on purpose (the real-client
track uses it); override with --account-a/--guid-a and
--account-b/--guid-b (or the TWO_CLIENT_* env vars) with
any two offline, co-located characters on distinct
accounts. Two sessions of one account cannot coexist, so
the accounts must differ. Both characters must be marked
offline or the smoke refuses to run (TWO_CLIENT_FORCE=1
overrides at collision risk).

Heartbeat bodies carry the full 28-byte flagless
MovementInfo (flags, client time, x/y/z/o, fall time);
a shorter body is rejected by ``MovementInfo::read`` and
silently dropped, which would make the relay assertion
meaningless.

Prints TWO-CLIENT-SMOKE-PASS on success. Exit codes:
0 = pass, 1 = smoke failure, 2 = setup/connection error.
``--self-test`` checks the packed-GUID/relay parsers
offline and prints TWO-CLIENT-SELFTEST-PASS.

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

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import phase3_scripted_login as p3  # noqa: E402
from phase3_scripted_login import Client, bench_password  # noqa: E402

CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
CMSG_LOGOUT_REQUEST = 75
SMSG_LOGOUT_COMPLETE = 77
SMSG_UPDATE_OBJECT = 169
MSG_MOVE_HEARTBEAT = 238
SMSG_COMPRESSED_UPDATE_OBJECT = 502
SMSG_LOGIN_VERIFY_WORLD = 566


def sql_scalar(db: str, stmt: str) -> str:
    completed = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{bench_password()}", "-N", "-B", db, "-e", stmt],
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"mysql query failed for {db}")
    return completed.stdout.strip()


def ensure_offline(guid: int) -> None:
    if os.environ.get("TWO_CLIENT_FORCE") == "1":
        return
    online = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
    if online == "1":
        raise RuntimeError(f"character guid {guid} is marked online; aborting scripted login")


def pack_guid(guid: int) -> bytes:
    """ObjectGuid packed form: mask byte + present bytes LE."""
    raw = guid.to_bytes(8, "little")
    mask = 0
    body = bytearray()
    for index, byte in enumerate(raw):
        if byte:
            mask |= 1 << index
            body.append(byte)
    return bytes([mask]) + bytes(body)


def parse_packed_guid(body: bytes, offset: int = 0) -> tuple[int, int]:
    """Return (guid, next offset) for a packed GUID at offset."""
    mask = body[offset]
    offset += 1
    guid = 0
    for index in range(8):
        if mask & (1 << index):
            guid |= body[offset] << (index * 8)
            offset += 1
    return guid, offset


def parse_relay(body: bytes) -> tuple[int, float, float, float, float]:
    """Relayed movement: packed mover GUID + MovementInfo
    (flags, server time, x, y, z, o, ...)."""
    guid, offset = parse_packed_guid(body)
    _flags, _time, x, y, z, orientation = struct.unpack_from("<IIffff", body, offset)
    return guid, x, y, z, orientation


def heartbeat_body(client_time: int, x: float, y: float, z: float, orientation: float) -> bytes:
    """Full 28-byte flagless MovementInfo, fall time included."""
    return struct.pack("<IIffffI", 0, client_time, x, y, z, orientation, 0)


def drain_until_idle(client: Client, idle_secs: float, max_secs: float) -> list[tuple[int, bytes]]:
    """Read packets until the stream is quiet; returns them
    all. Only calls read_packet when select reports data,
    so a timeout never lands mid-packet."""
    seen: list[tuple[int, bytes]] = []
    client.sock.settimeout(5)
    deadline = time.time() + max_secs
    last_packet_at = time.time()
    while time.time() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if ready:
            seen.append(client.read_packet())
            last_packet_at = time.time()
        elif seen and time.time() - last_packet_at >= idle_secs:
            break
    return seen


def read_until_opcode(
    client: Client, wanted: int, timeout_secs: float, label: str
) -> tuple[bytes, list[tuple[int, bytes]]]:
    """Read until `wanted` arrives; returns its body plus
    everything skipped on the way."""
    skipped: list[tuple[int, bytes]] = []
    client.sock.settimeout(5)
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if not ready:
            continue
        opcode, body = client.read_packet()
        if opcode == wanted:
            return body, skipped
        skipped.append((opcode, body))
    raise AssertionError(f"timeout waiting for {label}; skipped opcodes {[op for op, _ in skipped]}")


def login_client(account: str, account_id: int, guid: int) -> tuple[Client, tuple[float, float, float, float]]:
    """One scripted login through the first update burst.

    The phase3 Client reads its account/host/port from
    module globals at auth time, so point them at this
    account first; the session key (K_HEX) is shared.
    """
    p3.ACCOUNT = account
    p3.ACCOUNT_ID = account_id
    p3.setup_account()
    client = Client()
    client.auth()
    client.send_packet(CMSG_CHAR_ENUM, b"")
    opcode, _body = client.read_packet()
    if opcode != SMSG_CHAR_ENUM:
        raise AssertionError(f"{account}: expected SMSG_CHAR_ENUM, got {opcode}")
    client.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", guid))
    packets = drain_until_idle(client, idle_secs=1.5, max_secs=15)
    verify = next((body for op, body in packets if op == SMSG_LOGIN_VERIFY_WORLD), None)
    if verify is None:
        raise AssertionError(f"{account}: no SMSG_LOGIN_VERIFY_WORLD in {[op for op, _ in packets]}")
    map_id, x, y, z, orientation = struct.unpack("<Iffff", verify)
    if not any(op in (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT) for op, _ in packets):
        raise AssertionError(f"{account}: no update burst in {[op for op, _ in packets]}")
    print(f"LOGIN-OK account={account} guid={guid} map={map_id} pos=({x:.2f},{y:.2f},{z:.2f})")
    return client, (x, y, z, orientation)


def logout(client: Client, label: str) -> None:
    client.send_packet(CMSG_LOGOUT_REQUEST, b"")
    read_until_opcode(client, SMSG_LOGOUT_COMPLETE, timeout_secs=10, label=f"{label} logout")
    client.sock.close()
    print(f"LOGOUT-OK {label}")


def assert_relay(
    mover: Client,
    watcher: Client,
    mover_guid: int,
    position: tuple[float, float, float, float],
    dx: float,
    dy: float,
    label: str,
) -> tuple[float, float, float, float]:
    """Move the mover and require the watcher to receive the
    relay with the mover's GUID at the new position. Player
    GUIDs on the wire are the bare counter at 1.12.1."""
    x, y, z, orientation = position
    new_position = (x + dx, y + dy, z, orientation)
    mover.send_packet(
        MSG_MOVE_HEARTBEAT,
        heartbeat_body(2000, new_position[0], new_position[1], new_position[2], new_position[3]),
    )
    deadline = time.time() + 10
    while time.time() < deadline:
        body, _skipped = read_until_opcode(watcher, MSG_MOVE_HEARTBEAT, 10, label)
        guid, rx, ry, rz, _ro = parse_relay(body)
        if guid != mover_guid:
            continue  # some other mover's relay; keep waiting
        if abs(rx - new_position[0]) > 0.01 or abs(ry - new_position[1]) > 0.01 or abs(rz - z) > 0.01:
            raise AssertionError(f"{label}: relay pos ({rx},{ry},{rz}) != sent {new_position}")
        print(f"{label} relay guid={guid} pos=({rx:.2f},{ry:.2f},{rz:.2f})")
        return new_position
    raise AssertionError(f"{label}: no relay for guid {mover_guid} before deadline")


def run_smoke(args: argparse.Namespace) -> int:
    p3.HOST = args.host
    p3.PORT = args.port
    try:
        if args.account_a == args.account_b:
            raise RuntimeError("the two accounts must differ; one account cannot hold two world sessions")
        ensure_offline(args.guid_a)
        ensure_offline(args.guid_b)
    except Exception as exc:
        print(f"TWO-CLIENT-SETUP-ERROR: {exc}")
        return 2

    client_a: Client | None = None
    client_b: Client | None = None
    try:
        client_a, position_a = login_client(args.account_a, args.account_id_a, args.guid_a)
        client_b, position_b = login_client(args.account_b, args.account_id_b, args.guid_b)
        # B's arrival generates visibility creates for A;
        # quiet both streams before asserting on relays.
        drain_until_idle(client_a, idle_secs=1.0, max_secs=5)
        drain_until_idle(client_b, idle_secs=1.0, max_secs=5)

        online_a = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={args.guid_a}")
        online_b = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={args.guid_b}")
        if (online_a, online_b) != ("1", "1"):
            print(f"TWO-CLIENT-FAIL presence online_a={online_a} online_b={online_b}, expected both 1")
            return 1
        print("PRESENCE-BOTH-ONLINE")

        position_a = assert_relay(client_a, client_b, args.guid_a, position_a, 4.0, 0.0, "RELAY-A-TO-B")
        # The mover must not receive its own relay.
        echoed = drain_until_idle(client_a, idle_secs=1.0, max_secs=3)
        own = [body for op, body in echoed if op == MSG_MOVE_HEARTBEAT and parse_relay(body)[0] == args.guid_a]
        if own:
            print("TWO-CLIENT-FAIL mover received its own movement relay")
            return 1
        print("NO-SELF-RELAY-A")

        position_b = assert_relay(client_b, client_a, args.guid_b, position_b, 0.0, 4.0, "RELAY-B-TO-A")
        _ = (position_a, position_b)
    except Exception as exc:
        print(f"TWO-CLIENT-FAIL: {exc}")
        return 1
    finally:
        for client, label in ((client_b, "B"), (client_a, "A")):
            if client is None:
                continue
            try:
                logout(client, label)
            except Exception:
                try:
                    client.sock.close()
                except Exception:
                    pass

    try:
        online_a = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={args.guid_a}")
        online_b = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={args.guid_b}")
        if (online_a, online_b) != ("0", "0"):
            print(f"TWO-CLIENT-FAIL presence after logout online_a={online_a} online_b={online_b}, expected both 0")
            return 1
        print("PRESENCE-BOTH-OFFLINE")
    except Exception as exc:
        print(f"TWO-CLIENT-SETUP-ERROR: presence check failed: {exc}")
        return 2
    print("TWO-CLIENT-SMOKE-PASS")
    return 0


def run_self_test() -> int:
    for guid in (0, 3, 4, 255, 0x0100, 100_000, 0xF100_0000_0000_BEEF):
        packed = pack_guid(guid)
        parsed, next_offset = parse_packed_guid(packed + b"tail")
        assert parsed == guid and next_offset == len(packed), (guid, parsed)
    print("SELFTEST packed-guid roundtrip")

    body = pack_guid(3) + struct.pack("<IIffffI", 0, 1100, -614.5, -4251.67, 38.718, 1.25, 0)
    guid, x, y, z, orientation = parse_relay(body)
    assert guid == 3
    assert abs(x + 614.5) < 0.001 and abs(y + 4251.67) < 0.01 and abs(z - 38.718) < 0.001
    assert abs(orientation - 1.25) < 0.001
    print("SELFTEST relay-body parse")

    assert len(heartbeat_body(1000, 1.0, 2.0, 3.0, 0.5)) == 28
    print("SELFTEST heartbeat-body is the full 28-byte flagless MovementInfo")

    print("TWO-CLIENT-SELFTEST-PASS")
    return 0


def _env_int(name: str, default: int) -> int:
    return int(os.environ.get(name, str(default)))


def main() -> int:
    parser = argparse.ArgumentParser(description="Two-client scripted world smoke (see module docstring).")
    parser.add_argument("--self-test", action="store_true", help="run offline parser tests, no server needed")
    parser.add_argument("--account-a", default=os.environ.get("TWO_CLIENT_ACCOUNT_A", "BOT_Zulrath"))
    parser.add_argument("--account-id-a", type=int, default=_env_int("TWO_CLIENT_ACCOUNT_ID_A", 101))
    parser.add_argument("--guid-a", type=int, default=_env_int("TWO_CLIENT_GUID_A", 3))
    parser.add_argument("--account-b", default=os.environ.get("TWO_CLIENT_ACCOUNT_B", "BOT_Morka"))
    parser.add_argument("--account-id-b", type=int, default=_env_int("TWO_CLIENT_ACCOUNT_ID_B", 102))
    parser.add_argument("--guid-b", type=int, default=_env_int("TWO_CLIENT_GUID_B", 4))
    parser.add_argument("--host", default=os.environ.get("TWO_CLIENT_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=_env_int("TWO_CLIENT_PORT", 18099))
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    return run_smoke(args)


if __name__ == "__main__":
    sys.exit(main())
