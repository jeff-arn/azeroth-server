#!/usr/bin/env python3
"""Phase 4 Slice 3 scripted world evidence against the dev bench.

Reuses the Phase 3 login client. Two sessions of the same
bench account cannot coexist, so the second client logs in
after the first logs out; the movement-relay check runs
with one client in world plus server-side assertions via
packet flow:

  login -> verify world -> object-update train (init-self
  + creature/GO creates) -> character online flag set ->
  heartbeat movement accepted (no kick, session alive) ->
  logout -> LOGOUT_COMPLETE + online flag cleared.

Expects mangosd on 127.0.0.1:18099 with the bench conf.
Prints SCRIPTED-WORLD-PASS on success.
"""

import pathlib
import struct
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from phase3_scripted_login import (  # noqa: E402
    ACCOUNT_ID,
    GUID,
    Client,
    bench_password,
    setup_account,
)

CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
SMSG_LOGIN_VERIFY_WORLD = 566
SMSG_ACCOUNT_DATA_MD5 = 521
SMSG_FRIEND_LIST = 103
SMSG_IGNORE_LIST = 107
SMSG_MESSAGECHAT = 150
SMSG_UPDATE_OBJECT = 169
SMSG_COMPRESSED_UPDATE_OBJECT = 502
MSG_MOVE_HEARTBEAT = 238
CMSG_LOGOUT_REQUEST = 75
SMSG_LOGOUT_COMPLETE = 77
CMSG_PING = 476
SMSG_PONG = 477
SMSG_MONSTER_MOVE = 221
CMSG_FORCE_RUN_SPEED_CHANGE_ACK = 227
CMSG_CAST_SPELL = 302
CMSG_CANCEL_CAST = 303
SMSG_CAST_RESULT = 304
SMSG_SPELL_GO = 306
CMSG_SET_SELECTION = 317
CMSG_ATTACKSWING = 321
CMSG_ATTACKSTOP = 322
CMSG_GAMEOBJ_USE = 177
CMSG_EMOTE = 258
CMSG_TEXT_EMOTE = 260
SMSG_TEXT_EMOTE = 261


def sql_scalar(db: str, stmt: str) -> str:
    r = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{bench_password()}", "-N", "-B", db, "-e", stmt],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr)
    return r.stdout.strip()


def read_until(client: Client, want: set[int], tally: dict[int, int]) -> tuple[int, bytes]:
    while True:
        op, body = client.read_packet()
        tally[op] = tally.get(op, 0) + 1
        if op in want:
            return op, body


def main() -> None:
    setup_account()
    client = Client()
    client.auth()
    client.send_packet(CMSG_CHAR_ENUM, b"")
    op, _ = client.read_packet()
    assert op == SMSG_CHAR_ENUM, op

    client.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", GUID))
    op, body = client.read_packet()
    assert op == SMSG_LOGIN_VERIFY_WORLD, op
    map_id, x, y, z, o = struct.unpack("<Iffff", body)
    print(f"VERIFY-WORLD map={map_id} pos=({x:.2f},{y:.2f},{z:.2f}) o={o:.2f}")
    assert map_id == 0

    tally: dict[int, int] = {}
    for want in (SMSG_ACCOUNT_DATA_MD5, SMSG_FRIEND_LIST, SMSG_IGNORE_LIST):
        read_until(client, {want}, tally)
    read_until(client, {SMSG_MESSAGECHAT}, tally)
    read_until(client, {SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT}, tally)
    print(f"OBJECT-UPDATES tally={ {k: v for k, v in sorted(tally.items())} }")
    assert tally.get(SMSG_UPDATE_OBJECT, 0) + tally.get(SMSG_COMPRESSED_UPDATE_OBJECT, 0) >= 1

    online = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={GUID}")
    assert online == "1", f"presence online={online}"
    print("PRESENCE-ONLINE-1")

    # Slice 4: nearby random/waypoint creatures must start
    # server-moving within ~15s (random gen first step is
    # at 1s; waypoint starts immediately).
    import time as _time

    deadline = _time.time() + 15.0
    monster = 0
    while _time.time() < deadline and monster == 0:
        try:
            op, _body = client.read_packet()
        except TimeoutError:
            break
        tally[op] = tally.get(op, 0) + 1
        if op == SMSG_MONSTER_MOVE:
            monster += 1
    assert monster >= 1, f"no MONSTER_MOVE observed; tally={tally}"
    print("MONSTER-MOVE-OBSERVED")

    # Heartbeat a small step east; relay has no watcher, but
    # the session must stay alive and answer ping. The body is
    # the full 28-byte flagless MovementInfo including fall
    # time; MovementInfo::read rejects anything shorter.
    move = struct.pack("<IIffffI", 0, 1000, x + 3.0, y, z, o, 0)
    client.send_packet(MSG_MOVE_HEARTBEAT, move)
    client.send_packet(CMSG_PING, struct.pack("<II", 0xABCD, 40))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xABCD)
    print("MOVEMENT-ACCEPTED + PONG")

    # Slice 4 ACK loop: a forged force-speed ACK (nothing
    # pending) must be dropped without killing the
    # session. Wire shape: raw u64 guid, counter,
    # MovementInfo (28-byte flagless base), flat speed.
    # Player GUID high word for 1.12.1 is 0x0000 (counter
    # only), matching the bench fixture's enum bytes.
    ack = struct.pack("<QI", GUID, 77) + move + struct.pack("<f", 99.0)
    client.send_packet(CMSG_FORCE_RUN_SPEED_CHANGE_ACK, ack)
    client.send_packet(CMSG_PING, struct.pack("<II", 0xBEEF, 40))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xBEEF)
    print("BOGUS-ACK-DROPPED + PONG")

    # Slice 5: cast pipeline over the wire. Unknown spell
    # drops silently; known Battle Shout (5242) at 0 rage
    # must answer CAST_RESULT with FAILED_NO_POWER (77) —
    # resource generation is a later slice, so a
    # successful cast is not reachable from the client
    # yet (live_spells.rs covers success paths in-map).
    client.send_packet(CMSG_CAST_SPELL, struct.pack("<IH", 999999, 0))
    client.send_packet(CMSG_PING, struct.pack("<II", 0xCA57, 40))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xCA57)
    assert tally.get(SMSG_CAST_RESULT, 0) == 0, tally
    print("UNKNOWN-SPELL-DROPPED + PONG")

    client.send_packet(CMSG_CAST_SPELL, struct.pack("<IH", 5242, 0))
    op, body = read_until(client, {SMSG_CAST_RESULT}, tally)
    spell_id, reason = struct.unpack("<IB", body[:5])
    assert (spell_id, reason) == (5242, 77), (spell_id, reason)
    print("CAST-RESULT-NO-POWER")

    client.send_packet(CMSG_CANCEL_CAST, struct.pack("<I", 5242))
    client.send_packet(CMSG_PING, struct.pack("<II", 0xCA58, 40))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xCA58)
    print("CANCEL-CAST-TOLERATED + PONG")

    # Slice 6: attack handlers. A bogus target guid must be
    # a silent no-op (real combat evidence is live_ai.rs,
    # which parses creature guids from the world state).
    bogus = struct.pack("<Q", 0xF10000000000BEEF)
    client.send_packet(CMSG_SET_SELECTION, bogus)
    client.send_packet(CMSG_ATTACKSWING, bogus)
    client.send_packet(CMSG_ATTACKSTOP, b"")
    client.send_packet(CMSG_PING, struct.pack("<II", 0xCA59, 40))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xCA59)
    print("ATTACK-SWING-BOGUS-TOLERATED + PONG")

    # Slice 6b: text emote with a bogus target must not
    # crash the session; the SMSG_TEXT_EMOTE broadcast
    # still reaches the sender (listen range includes
    # self). A hardcoded wave emote is tolerated too.
    client.send_packet(CMSG_TEXT_EMOTE, struct.pack("<IIQ", 431, 0, 0xF10000000000BEEF))
    op, body = read_until(client, {SMSG_TEXT_EMOTE}, tally)
    assert struct.unpack_from("<I", body, 8)[0] == 431, body
    print("TEXT-EMOTE-ECHO")
    client.send_packet(CMSG_EMOTE, struct.pack("<I", 3))
    client.send_packet(CMSG_PING, struct.pack("<II", 0xCA60, 41))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xCA60)
    print("EMOTE-TOLERATED + PONG")

    # Slice 7: gameobject use with a bogus guid is a
    # silent no-op (real GO script evidence is
    # live_deadmines.rs).
    client.send_packet(CMSG_GAMEOBJ_USE, struct.pack("<Q", 0xF12000000000BEEF))
    client.send_packet(CMSG_PING, struct.pack("<II", 0xCA61, 42))
    op, body = read_until(client, {SMSG_PONG}, tally)
    assert body == struct.pack("<I", 0xCA61)
    print("GAMEOBJ-USE-BOGUS-TOLERATED + PONG")

    client.send_packet(CMSG_LOGOUT_REQUEST, b"")
    op, _ = read_until(client, {SMSG_LOGOUT_COMPLETE}, tally)
    print("LOGOUT-COMPLETE")
    online = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={GUID}")
    assert online == "0", f"presence after logout={online}"
    print("PRESENCE-ONLINE-0")

    # Enum still works after logout (session back at char screen).
    client.send_packet(CMSG_CHAR_ENUM, b"")
    op, _ = client.read_packet()
    assert op == SMSG_CHAR_ENUM, op
    print("ENUM-AFTER-LOGOUT")
    _ = ACCOUNT_ID
    print("SCRIPTED-WORLD-PASS")


if __name__ == "__main__":
    sys.exit(main())
