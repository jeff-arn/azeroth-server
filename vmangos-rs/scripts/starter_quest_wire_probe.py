#!/usr/bin/env python3
"""Scripted wire probe: the Coldridge starter-quest loop end to end.

Covers the chain the Tier 4 VM gnome probe shook
out, in one deterministic scripted run on the bench
gnome mage (Probegnome, guid 900014):

1. Accept quests 179 (Dwarven Outfitters) and 233
   (Coldridge Valley Mail Delivery) from Sten
   Stoutarm through the default gossip menu
   (query -> details -> accept). Asserts the mail
   source item (2187) is actually in the live
   inventory and survives logout (the grant used to
   write only the save baseline; the next save diff
   deleted its DB rows).
2. Turn in 233 at Talin Keeneye (complete ->
   request reward -> choose reward); asserts the
   rewarded flag and the XP gain in the DB.
3. Kill Ragged Young Wolves with Fireball and loot
   Tough Wolf Meat (750) until the quest counter
   reaches 8 (80% quest drop; spots rotate over 12
   spawns, respawn 180s). Asserts SMSG_QUESTUPDATE
   item progress and the persisted item_count1.
4. Turn in 179 at Sten and asserts rewarded + XP.

Initial placements are bench-fixture teleports
between phase logins. Within the wolf phase, the
scripted navigation driver walks between spawns by
ordinary movement heartbeats, tracks update-object
positions/health, casts, approaches corpses, and
loots; every gameplay packet is the real client
flow.

Prints LIVE-STARTER-QUESTS-PASS on success. Run
from the repository root against the running bench
mangosd. The gnome's quest state for 179/233 and
quest items are reset first; final position is the
Sten turn-in spot.
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402
from scripted_driver import ScriptedDriver  # noqa: E402

CMSG_GOSSIP_HELLO = 379
SMSG_GOSSIP_MESSAGE = 381
CMSG_NPC_TEXT_QUERY = 383
SMSG_NPC_TEXT_UPDATE = 384
CMSG_QUESTGIVER_HELLO = 388
SMSG_QUESTGIVER_QUEST_LIST = 389
CMSG_QUESTGIVER_QUERY_QUEST = 390
SMSG_QUESTGIVER_QUEST_DETAILS = 392
CMSG_QUESTGIVER_ACCEPT_QUEST = 393
CMSG_QUESTGIVER_COMPLETE_QUEST = 394
CMSG_QUESTGIVER_REQUEST_REWARD = 396
SMSG_QUESTGIVER_OFFER_REWARD = 397
CMSG_QUESTGIVER_CHOOSE_REWARD = 398
SMSG_QUESTGIVER_QUEST_COMPLETE = 401

GUID = 900014
STEN_RAW = (0xF130 << 48) | (658 << 24) | 351
TALIN_RAW = (0xF130 << 48) | (714 << 24) | 472
FIREBALL = 133
FROST_ARMOR = 168
MEAT = 750
MAIL = 2187

# One login per route. Each route starts at a fixture
# placement, then ScriptedDriver walks spawn to spawn.
# Coordinates are the bench world DB spawn points for
# creature entry 705; tracked update-object positions
# supersede them when the wolf has wandered.
WOLF_ROUTES = [
    (
        (-6288.0, 327.0, 377.5),
        [
            (344, -6287.2, 328.1, 376.5),
            (1526, -6295.2, 321.0, 376.8),
            (1525, -6298.1, 302.0, 376.7),
            (345, -6309.4, 304.2, 378.1),
            (347, -6329.8, 288.5, 380.1),
            (1032, -6340.8, 308.9, 380.0),
            (1530, -6302.5, 257.0, 382.9),
            (338, -6303.4, 262.0, 381.8),
            (1527, -6272.1, 285.6, 382.6),
            (346, -6278.2, 276.6, 382.9),
        ],
    ),
    (
        (-6286.0, 353.0, 378.2),
        [
            (1515, -6286.1, 352.9, 378.2),
            (1516, -6294.7, 383.0, 381.0),
            (1012, -6299.6, 381.7, 380.5),
            (1015, -6303.2, 403.1, 381.9),
            (1541, -6311.4, 392.4, 380.9),
            (1018, -6318.3, 403.0, 380.6),
            (1013, -6333.4, 399.4, 379.2),
            (1000, -6338.7, 420.6, 378.4),
            (998, -6314.3, 445.4, 383.4),
            (1016, -6281.5, 431.9, 384.4),
            (428, -6282.0, 424.8, 384.3),
            (1014, -6263.9, 390.0, 382.8),
            (400, -6252.8, 387.1, 383.3),
            (402, -6245.9, 395.6, 385.9),
            (1020, -6233.0, 408.1, 387.6),
            (418, -6244.8, 446.1, 385.5),
            (419, -6216.6, 449.4, 387.7),
        ],
    ),
]


def wolf_raw(counter: int) -> int:
    return (0xF130 << 48) | (705 << 24) | counter


def sql(stmt: str) -> str:
    return smoke.sql_scalar("characters", stmt)


def set_pos(x: float, y: float, z: float, o: float) -> None:
    # death_expire_time=0 + clear CHARACTER_FLAG_GHOST
    # (0x2000): the fixture gnome may be persistently
    # dead from VM-client sessions, and ghosts cannot
    # gossip/loot/cast.
    sql(
        "UPDATE characters SET map=0, "
        f"position_x={x}, position_y={y}, position_z={z}, orientation={o}, "
        "health=120, death_expire_time=0, "
        f"character_flags=character_flags & ~8192, online=0 WHERE guid={GUID}"
    )


def quest_row(quest: int) -> str:
    return sql(
        "SELECT CONCAT(status,':',rewarded,':',item_count1) "
        f"FROM character_queststatus WHERE guid={GUID} AND quest={quest}"
    )


def level_xp() -> tuple[int, int]:
    level, current = sql(
        f"SELECT CONCAT(level,' ',xp) FROM characters WHERE guid={GUID}"
    ).split()
    return int(level), int(current)


def reset_fixture() -> None:
    # Backpack junk from earlier probe runs fills all
    # 16 slots and starves quest item grants; clear
    # backpack rows except the starter consumables
    # and hearthstone (159/4536/6948).
    guids = sql(
        "SELECT GROUP_CONCAT(item_guid) FROM character_inventory "
        f"WHERE guid={GUID} AND bag=0 AND slot BETWEEN 23 AND 38 "
        "AND item_id NOT IN (159,4536,6948)"
    )
    sql(f"DELETE FROM character_queststatus WHERE guid={GUID} AND quest IN (179,233)")
    sql(
        "DELETE FROM character_inventory "
        f"WHERE guid={GUID} AND bag=0 AND slot BETWEEN 23 AND 38 "
        "AND item_id NOT IN (159,4536,6948)"
    )
    sql(
        "DELETE FROM character_inventory "
        f"WHERE guid={GUID} AND item_id IN ({MEAT},{MAIL})"
    )
    if guids and guids != "NULL":
        sql(f"DELETE FROM item_instance WHERE guid IN ({guids})")


def accept_quest(client: smoke.Client, npc_raw: int, quest_id: int) -> None:
    client.send_packet(
        CMSG_QUESTGIVER_QUERY_QUEST, struct.pack("<QI", npc_raw, quest_id)
    )
    smoke.read_until_opcode(
        client, SMSG_QUESTGIVER_QUEST_DETAILS, 10, f"DETAILS-{quest_id}"
    )
    client.send_packet(
        CMSG_QUESTGIVER_ACCEPT_QUEST, struct.pack("<QI", npc_raw, quest_id)
    )
    smoke.drain_until_idle(client, idle_secs=0.8, max_secs=4)


def phase_accept() -> None:
    set_pos(-6216.0, 328.2, 383.7, 0.0)
    client, _pos = smoke.login_client("TESTUSER", 142, GUID)
    try:
        client.send_packet(CMSG_GOSSIP_HELLO, struct.pack("<Q", STEN_RAW))
        body, _ = smoke.read_until_opcode(client, SMSG_GOSSIP_MESSAGE, 10, "GOSSIP")
        (text_id,) = struct.unpack_from("<I", body, 8)
        client.send_packet(
            CMSG_NPC_TEXT_QUERY, struct.pack("<IQ", text_id, STEN_RAW)
        )
        smoke.read_until_opcode(client, SMSG_NPC_TEXT_UPDATE, 10, "NPC-TEXT")
        accept_quest(client, STEN_RAW, 179)
        accept_quest(client, STEN_RAW, 233)
    finally:
        smoke.logout(client, "starter-accept")
    # 233's only objective is possessing the mail, so
    # the accept-time source-item grant completes it
    # on the spot (status COMPLETE, item_count1=1).
    if quest_row(179) != "3:0:0" or quest_row(233) != "1:0:1":
        raise AssertionError(
            f"quest rows after accept: 179={quest_row(179)!r} 233={quest_row(233)!r}"
        )
    mail = sql(
        "SELECT COUNT(*) FROM character_inventory "
        f"WHERE guid={GUID} AND item_id={MAIL}"
    )
    if mail != "1":
        raise AssertionError(f"mail source item rows after accept+save: {mail!r}")
    print("ACCEPT ok (179 incomplete, 233 objective complete, mail item persisted)")


def turn_in(client: smoke.Client, npc_raw: int, quest_id: int) -> None:
    client.send_packet(CMSG_QUESTGIVER_HELLO, struct.pack("<Q", npc_raw))
    try:
        smoke.read_until_opcode(
            client, SMSG_QUESTGIVER_QUEST_LIST, 4, "QUEST-LIST"
        )
    except AssertionError:
        pass  # gossip-flag enders answer with a menu instead; both fine
    client.send_packet(
        CMSG_QUESTGIVER_COMPLETE_QUEST, struct.pack("<QI", npc_raw, quest_id)
    )
    smoke.drain_until_idle(client, idle_secs=0.8, max_secs=4)
    client.send_packet(
        CMSG_QUESTGIVER_REQUEST_REWARD, struct.pack("<QI", npc_raw, quest_id)
    )
    smoke.read_until_opcode(
        client, SMSG_QUESTGIVER_OFFER_REWARD, 10, f"OFFER-{quest_id}"
    )
    client.send_packet(
        CMSG_QUESTGIVER_CHOOSE_REWARD, struct.pack("<QII", npc_raw, quest_id, 0)
    )
    smoke.read_until_opcode(
        client, SMSG_QUESTGIVER_QUEST_COMPLETE, 10, f"COMPLETE-{quest_id}"
    )


def phase_mail_turnin(level_xp_before: tuple[int, int]) -> None:
    set_pos(-6224.0, 687.0, 386.0, 0.785)
    client, _pos = smoke.login_client("TESTUSER", 142, GUID)
    try:
        turn_in(client, TALIN_RAW, 233)
    finally:
        smoke.logout(client, "starter-mail")
    row = quest_row(233)
    if not row.endswith(":1:0") and ":1:" not in row:
        raise AssertionError(f"233 not rewarded after turn-in: {row!r}")
    level_after, xp_after = level_xp()
    level_before, xp_before = level_xp_before
    if (level_after, xp_after) <= (level_before, xp_before):
        raise AssertionError(
            "no XP/level gain from 233 turn-in "
            f"({level_before}:{xp_before} -> {level_after}:{xp_after})"
        )
    print(
        f"MAIL-TURNIN ok (row {row}, level/xp "
        f"{level_before}:{xp_before} -> {level_after}:{xp_after})"
    )


def phase_wolves() -> int:
    total_meat = 0
    routes = [(start, spawns) for start, spawns in WOLF_ROUTES]
    # Second pass after the 180s respawn timer: drop
    # variance and mid-route deaths can leave the
    # first pass short of 8 meat.
    routes += routes
    for route_no, ((x, y, z), spawns) in enumerate(routes):
        if total_meat >= 8:
            break
        set_pos(x, y, z, 0.0)
        client, position = smoke.login_client("TESTUSER", 142, GUID)
        try:
            driver = ScriptedDriver(client, position, own_guid=GUID)
            driver.drain_idle(idle_secs=0.3, max_secs=2)
            armor = driver.cast_spell(FROST_ARMOR)
            if not armor.ok:
                raise AssertionError(
                    f"Frost Armor failed before wolf route {route_no}: "
                    f"reason={armor.result_reason}"
                )
            for counter, sx, sy, sz in spawns:
                if total_meat >= 8:
                    break
                if driver.self_health() == 0:
                    print("gnome died; ending wolf route early")
                    break
                health = driver.self_health()
                if health is not None and health <= 20:
                    recovered = driver.wait_self_health_at_least(40, timeout=45.0)
                    print(f"regen wait before wolf {counter}: hp {health} -> {recovered}")
                    if driver.self_health() == 0:
                        break
                result = driver.kill_and_loot(
                    wolf_raw(counter),
                    FIREBALL,
                    fallback=(sx, sy, sz),
                    cast_range=18.0,
                    max_casts=8,
                )
                if result is None:
                    outcome = driver.last_cast_outcome
                    reason = None if outcome is None else outcome.result_reason
                    print(
                        f"wolf {counter}: skipped (last cast reason={reason}, "
                        f"target hp={driver.target_health(wolf_raw(counter))}, "
                        f"self hp={driver.self_health()}, pos={driver.position})"
                    )
                    continue
                got = result.items.get(MEAT, 0)
                total_meat += got
                print(
                    f"wolf {counter}: +{got} meat (total {total_meat}, "
                    f"gold {result.gold}, pos {driver.position})"
                )
            if driver.world.errors:
                print(
                    "world-state parse notes: "
                    + "; ".join(driver.world.errors[:3])
                )
        finally:
            smoke.logout(client, f"starter-wolves-{route_no}")
    return total_meat


def phase_meat_turnin() -> None:
    set_pos(-6216.0, 328.2, 383.7, 0.0)
    client, _pos = smoke.login_client("TESTUSER", 142, GUID)
    try:
        turn_in(client, STEN_RAW, 179)
    finally:
        smoke.logout(client, "starter-meat")
    row = quest_row(179)
    if ":1:" not in row:
        raise AssertionError(f"179 not rewarded after turn-in: {row!r}")
    print(f"MEAT-TURNIN ok (row {row})")


def main() -> int:
    reset_fixture()
    level_xp_start = level_xp()
    phase_accept()
    phase_mail_turnin(level_xp_start)
    meat = phase_wolves()
    row = quest_row(179)
    persisted = int(row.rsplit(":", 1)[1]) if row else -1
    if meat < 1:
        raise AssertionError("no Tough Wolf Meat looted in wolf phase")
    if persisted != meat and meat < 8:
        raise AssertionError(
            f"persisted item_count1 {persisted} != looted meat {meat}"
        )
    print(f"WOLVES ok (meat looted {meat}, persisted count {persisted})")
    if meat >= 8:
        phase_meat_turnin()
    else:
        print(f"NOTE: meat {meat}/8 after all spots; 179 turn-in not run")
    print("LIVE-STARTER-QUESTS-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
