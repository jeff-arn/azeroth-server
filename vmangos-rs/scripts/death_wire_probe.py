#!/usr/bin/env python3
"""Scripted wire probe: the player death loop end to end.

Regression for the Tier 4 owner canary report
(2026-11-12): a saved ghost re-entered the world
unplayable and could not reclaim at the corpse.
Loopback covers the packet shapes in-process; this
probe drives the real client flow against the
running bench mangosd with the bench gnome
(Probegnome, guid 900014) and the scripted driver:

1. Die to a Coldridge wolf at 5 hp, release spirit
   (`CMSG_REPOP_REQUEST`), ack the graveyard
   teleport, require a `MSG_CORPSE_QUERY` answer
   naming the death spot, walk the ghost back and
   reclaim (`CMSG_RECLAIM_CORPSE`) past the 30 s
   delay.
2. Die again, release, and resurrect at the
   graveyard spirit healer
   (`CMSG_SPIRIT_HEALER_ACTIVATE`, entry 6491).
3. Die a third time, release, log out as a ghost,
   require the saved ghost flag + corpse row in
   the DB, log back in as that saved ghost, and
   reclaim — the exact state Ruikanis was stuck in
   on the droplet.

Prints LIVE-DEATH-WIRE-PASS on success. Run from
the repository root against the running bench
mangosd. Fixture placements and health are bench
DB edits between logins; every gameplay packet is
the real client flow.
"""

from __future__ import annotations

import pathlib
import struct
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402
from scripted_driver import ScriptedDriver  # noqa: E402

GUID = 900014
CMSG_REPOP_REQUEST = 346
CMSG_RECLAIM_CORPSE = 466
MSG_CORPSE_QUERY = 534
CMSG_SPIRIT_HEALER_ACTIVATE = 540
MSG_MOVE_TELEPORT_ACK = 199
SMSG_CORPSE_RECLAIM_DELAY = 617
TYPEID_CORPSE = 7
CORPSE_FIELD_OWNER = 6
SPIRIT_HEALER_ENTRY = 6491

# Coldridge kill spots: Rockjaw Trogg (707) and
# Burly Rockjaw Trogg (724) spawns first (hostile,
# from bench world DB spawn rows), then Ragged Young
# Wolf (705) spawns from the starter-quest routes.
# Tuples are (spawn guid, entry, x, y, z).
KILL_SPOTS = [
    (342, 707, -6324.98, 294.269, 379.937),
    (341, 707, -6303.39, 270.471, 380.585),
    (340, 707, -6331.14, 268.768, 380.622),
    (336, 724, -6298.93, 243.345, 390.887),
    (335, 707, -6322.07, 214.021, 392.046),
    (344, 705, -6287.2, 328.1, 376.5),
    (1526, 705, -6295.2, 321.0, 376.8),
    (1525, 705, -6298.1, 302.0, 376.7),
    (345, 705, -6309.4, 304.2, 378.1),
    (347, 705, -6329.8, 288.5, 380.1),
    (346, 705, -6278.2, 276.6, 382.9),
]


def sql(stmt: str) -> str:
    return smoke.sql_scalar("characters", stmt)


def set_alive_pos(x: float, y: float, z: float, health: int) -> None:
    sql(
        "UPDATE characters SET map=0, "
        f"position_x={x}, position_y={y}, position_z={z}, orientation=0, "
        f"health={health}, death_expire_time=0, "
        "character_flags=character_flags & ~8192, online=0 "
        f"WHERE guid={GUID}"
    )


def reset_fixture() -> None:
    sql(f"DELETE FROM corpse WHERE player_guid={GUID}")
    sql(f"DELETE FROM character_aura WHERE guid={GUID} AND spell IN (8326,20584)")


def login_driver() -> tuple[smoke.Client, ScriptedDriver]:
    client, position = smoke.login_client("TESTUSER", 142, GUID)
    driver = ScriptedDriver(client, position, own_guid=GUID)
    driver.drain_idle(idle_secs=0.3, max_secs=2)
    return client, driver


def ack_teleports(driver: ScriptedDriver, timeout: float = 12.0) -> bool:
    """Consume server near-teleports like the stock client.

    Parses each MSG_MOVE_TELEPORT_ACK (packed guid +
    counter + MovementInfo), adopts the destination
    as the driver position, and replies with the
    client ack (raw guid + counter + time; the fork's
    `ByteBuffer >> ObjectGuid` reads a plain u64).
    Returns True when at least one teleport landed.
    """
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


FIREBALL = 133


def creature_raw(counter: int, entry: int) -> int:
    return (0xF130 << 48) | (entry << 24) | counter


def die_to_wolves(driver: ScriptedDriver) -> tuple[float, float, float]:
    """Pull a Coldridge hostile with one Fireball, then let it finish.

    Troggs are first because they also proximity-aggro;
    wolves are neutral, so the starter-quest probe's
    Fireball pull is used for every spot.
    """
    for counter, entry, sx, sy, sz in KILL_SPOTS:
        raw = creature_raw(counter, entry)
        driver.approach_guid(raw, 2.5, fallback=(sx, sy, sz))
        if driver.self_health() == 0:
            return driver.position[:3]
        target_pos = driver.world.position(raw)
        if target_pos is None or driver.target_health(raw) == 0:
            print(f"wolf spot {counter}: no living target tracked")
            continue
        driver.face(target_pos[0], target_pos[1])
        outcome = driver.cast_spell(FIREBALL, raw)
        print(
            f"wolf spot {counter}: pull ok={outcome.ok} reason={outcome.result_reason} "
            f"target hp={driver.target_health(raw)} self hp={driver.self_health()}"
        )
        deadline = time.monotonic() + 30.0
        while time.monotonic() < deadline:
            if driver.self_health() == 0:
                return driver.position[:3]
            if driver.target_health(raw) == 0:
                break
            driver.face(target_pos[0], target_pos[1])
            driver.poll(0.5)
        if driver.self_health() == 0:
            return driver.position[:3]
        print(f"wolf spot {counter}: still alive at {driver.self_health()} hp")
    raise AssertionError(f"gnome survived the wolf spots at {driver.self_health()} hp")


def release_and_query(driver: ScriptedDriver) -> tuple[tuple[float, float, float], float]:
    """Release spirit, ack the graveyard teleport, query the corpse.

    Returns the corpse position and the server-sent
    reclaim delay in seconds (30 s default when the
    packet was missed).
    """
    driver.client.send_packet(CMSG_REPOP_REQUEST, b"")
    if not ack_teleports(driver):
        raise AssertionError("no graveyard teleport after repop request")
    delay_secs = 30.0
    for opcode, body in driver.events:
        if opcode == SMSG_CORPSE_RECLAIM_DELAY and len(body) >= 4:
            delay_ms = struct.unpack_from("<I", body)[0]
            delay_secs = delay_ms / 1000.0
            print(f"RECLAIM-DELAY {delay_ms} ms")
    driver.client.send_packet(MSG_CORPSE_QUERY, b"")
    body, _skipped = driver.wait_for(MSG_CORPSE_QUERY, 10.0, "corpse query reply")
    if len(body) != 21 or body[0] != 1:
        raise AssertionError(f"corpse not found by query: {body.hex()}")
    x, y, z = struct.unpack_from("<fff", body, 5)
    return (x, y, z), delay_secs


def corpse_raw(driver: ScriptedDriver, corpse_pos: tuple[float, float, float]) -> int:
    """Own corpse guid: tracked corpse by owner field, else SQL row.

    Other players' corpses/bones can be tracked too,
    so never take the first TYPEID_CORPSE blind.
    """
    candidates = []
    for obj in driver.world.objects.values():
        if obj.type_id != TYPEID_CORPSE:
            continue
        owner = obj.values.get(CORPSE_FIELD_OWNER, 0) | (obj.values.get(CORPSE_FIELD_OWNER + 1, 0) << 32)
        candidates.append((obj.guid, owner, obj.position))
    print(f"corpse candidates: {[(hex(g), o, p) for g, o, p in candidates]}")
    for guid, owner, _pos in candidates:
        if owner == GUID:
            return guid
    if candidates:

        def dist(item: tuple) -> float:
            pos = item[2]
            if pos is None:
                return float("inf")
            return sum((a - b) ** 2 for a, b in zip(pos[:3], corpse_pos))

        return min(candidates, key=dist)[0]
    low = sql(f"SELECT guid FROM corpse WHERE player_guid={GUID} LIMIT 1")
    if not low or low == "NULL":
        raise AssertionError("no corpse row for reclaim")
    return (0xF101 << 48) | int(low)


def wait_alive(driver: ScriptedDriver, label: str, timeout: float = 20.0) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        health = driver.self_health()
        if health is not None and health > 1:
            return health
        driver.poll(0.5)
    raise AssertionError(f"{label}: still at {driver.self_health()} hp")


def phase_reclaim() -> None:
    reset_fixture()
    set_alive_pos(-6322.0, 290.0, 380.0, health=5)
    client, driver = login_driver()
    try:
        death_pos = die_to_wolves(driver)
        repop_at = time.monotonic()
        corpse_pos, delay_secs = release_and_query(driver)
        dist = sum((a - b) ** 2 for a, b in zip(corpse_pos, death_pos)) ** 0.5
        if dist > 15.0:
            raise AssertionError(f"corpse query pos {corpse_pos} vs death {death_pos}")
        print(f"RECLAIM-PATH corpse at {corpse_pos}, ghost at {driver.position}")
        driver.walk_to(corpse_pos[0], corpse_pos[1], corpse_pos[2])
        # Wait out the server-sent reclaim delay from repop.
        while time.monotonic() - repop_at < delay_secs + 2.0:
            driver.face(corpse_pos[0] + 1.0, corpse_pos[1])
            driver.poll(1.0)
        raw = corpse_raw(driver, corpse_pos)
        print(f"RECLAIM send corpse={raw:#x} from ghost pos {driver.position}")
        driver.client.send_packet(CMSG_RECLAIM_CORPSE, struct.pack("<Q", raw))
        health = wait_alive(driver, "reclaim")
        print(f"RECLAIM ok (health {health})")
    finally:
        smoke.logout(client, "death-reclaim")


def phase_spirit() -> None:
    reset_fixture()
    set_alive_pos(-6322.0, 290.0, 380.0, health=5)
    client, driver = login_driver()
    try:
        die_to_wolves(driver)
        _pos, _delay = release_and_query(driver)
        healers = []
        deadline = time.monotonic() + 20.0
        while time.monotonic() < deadline and not healers:
            driver.poll(0.5)
            healers = [o for o in driver.world.by_entry(SPIRIT_HEALER_ENTRY) if o.position]
        if not healers:
            raise AssertionError("no spirit healer visible at the graveyard")
        healer = healers[0]
        driver.approach_guid(healer.guid, 3.0, fallback=healer.position[:3])
        driver.client.send_packet(CMSG_SPIRIT_HEALER_ACTIVATE, struct.pack("<Q", healer.guid))
        health = wait_alive(driver, "spirit resurrect")
        print(f"SPIRIT-RESURRECT ok (health {health})")
    finally:
        smoke.logout(client, "death-spirit")


def phase_saved_ghost() -> None:
    reset_fixture()
    set_alive_pos(-6322.0, 290.0, 380.0, health=5)
    client, driver = login_driver()
    death_pos = die_to_wolves(driver)
    repop_at = time.monotonic()
    corpse_pos, delay_secs = release_and_query(driver)
    smoke.logout(client, "death-ghost-logout")
    time.sleep(2.0)
    flags = int(sql(f"SELECT character_flags & 8192 FROM characters WHERE guid={GUID}"))
    corpses = sql(f"SELECT COUNT(*) FROM corpse WHERE player_guid={GUID}")
    if flags == 0 or corpses == "0":
        raise AssertionError(f"saved ghost state flags={flags} corpses={corpses}")
    print(f"SAVED-GHOST persisted (death {death_pos})")
    client, driver = login_driver()
    try:
        if driver.self_health() is not None and driver.self_health() > 1:
            # Ghost login restores 1 hp; anything higher was not a ghost.
            print(f"note: relog health {driver.self_health()}")
        driver.client.send_packet(MSG_CORPSE_QUERY, b"")
        body, _skipped = driver.wait_for(MSG_CORPSE_QUERY, 10.0, "saved-ghost query")
        if len(body) != 21 or body[0] != 1:
            raise AssertionError(f"saved-ghost corpse not found: {body.hex()}")
        driver.walk_to(corpse_pos[0], corpse_pos[1], corpse_pos[2])
        while time.monotonic() - repop_at < delay_secs + 2.0:
            driver.face(corpse_pos[0] + 1.0, corpse_pos[1])
            driver.poll(1.0)
        raw = corpse_raw(driver, corpse_pos)
        print(f"SAVED-GHOST reclaim send corpse={raw:#x} from {driver.position}")
        driver.client.send_packet(CMSG_RECLAIM_CORPSE, struct.pack("<Q", raw))
        health = wait_alive(driver, "saved-ghost reclaim")
        print(f"SAVED-GHOST-RECLAIM ok (health {health})")
    finally:
        smoke.logout(client, "death-saved-ghost")


def main() -> int:
    phase_reclaim()
    if sql(f"SELECT character_flags & 8192 FROM characters WHERE guid={GUID}") != "0":
        raise AssertionError("ghost flag still saved after reclaim logout")
    phase_spirit()
    phase_saved_ghost()
    if sql(f"SELECT character_flags & 8192 FROM characters WHERE guid={GUID}") != "0":
        raise AssertionError("ghost flag still saved after saved-ghost reclaim")
    print("LIVE-DEATH-WIRE-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
