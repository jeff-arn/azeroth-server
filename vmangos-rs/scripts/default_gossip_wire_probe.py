#!/usr/bin/env python3
"""Scripted wire probe: default (non-script) gossip menu.

Regression for the Tier 4 Coldridge gnome probe:
`CMSG_GOSSIP_HELLO` to a plain questgiver+gossip NPC
used to answer `SMSG_GOSSIP_COMPLETE` because only the
script arm was modeled. Logs in the bench gnome
(Probegnome, guid 900014) next to Sten Stoutarm
(entry 658, spawn guid 351), sends GOSSIP_HELLO and
requires an SMSG_GOSSIP_MESSAGE with the fork menu
(text 4937 from gossip_menu 4047, zero gossip items,
quests 179 + 233 available). Then plays the real
client's npc-text cache step: sends CMSG_NPC_TEXT_QUERY
for 4937 and requires an SMSG_NPC_TEXT_UPDATE whose
first option carries the broadcast text (the stock
client withholds GOSSIP_SHOW until this arrives;
without a handler the gossip frame never opens).

Prints LIVE-DEFAULT-GOSSIP-WIRE-PASS on success. Run
from the repository root against the running bench
mangosd. The character position and starter-quest
rows are restored after (the starter-quest probe
leaves this fixture with 179/233 rewarded, which
would otherwise offer follow-up 3114 here).
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402

CMSG_GOSSIP_HELLO = 379
SMSG_GOSSIP_MESSAGE = 381
CMSG_NPC_TEXT_QUERY = 383
SMSG_NPC_TEXT_UPDATE = 384

# Unit guid wire raw: high 0xF130, entry 658, counter 351.
NPC_RAW = (0xF130 << 48) | (658 << 24) | 351
GUID = 900014
QUEST_COLS = [
    "guid", "quest", "status", "rewarded", "explored", "timer",
    "mob_count1", "mob_count2", "mob_count3", "mob_count4",
    "item_count1", "item_count2", "item_count3", "item_count4",
    "reward_choice",
]
STARTER_QUESTS = (179, 233, 3114)


def quest_snapshot() -> dict[int, str]:
    expr = ",".join(QUEST_COLS)
    rows = {}
    for quest in STARTER_QUESTS:
        value = smoke.sql_scalar(
            "characters",
            f"SELECT CONCAT_WS(',',{expr}) FROM character_queststatus "
            f"WHERE guid={GUID} AND quest={quest}",
        )
        if value:
            rows[quest] = value
    return rows


def restore_quests(rows: dict[int, str]) -> None:
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_queststatus WHERE guid={GUID} "
        f"AND quest IN ({','.join(str(q) for q in STARTER_QUESTS)})",
    )
    cols = ",".join(QUEST_COLS)
    for value in rows.values():
        smoke.sql_scalar(
            "characters",
            f"INSERT INTO character_queststatus ({cols}) VALUES ({value})",
        )


def main() -> int:
    old = smoke.sql_scalar(
        "characters",
        "SELECT CONCAT(map,' ',position_x,' ',position_y,' ',position_z) "
        f"FROM characters WHERE guid={GUID}",
    )
    omap, ox, oy, oz = old.split()
    saved_quests = quest_snapshot()
    smoke.sql_scalar(
        "characters",
        f"DELETE FROM character_queststatus WHERE guid={GUID} "
        f"AND quest IN ({','.join(str(q) for q in STARTER_QUESTS)})",
    )
    smoke.sql_scalar(
        "characters",
        "UPDATE characters SET map=0, position_x=-6216.0, position_y=328.2, "
        f"position_z=383.7, online=0 WHERE guid={GUID}",
    )
    try:
        client, _pos = smoke.login_client("TESTUSER", 142, GUID)
        try:
            client.send_packet(CMSG_GOSSIP_HELLO, struct.pack("<Q", NPC_RAW))
            body, _ = smoke.read_until_opcode(
                client, SMSG_GOSSIP_MESSAGE, 10, "GOSSIP-MENU"
            )
            guid, text_id, count = struct.unpack_from("<QII", body, 0)
            if guid != NPC_RAW:
                raise AssertionError(f"menu guid {guid:#x} != {NPC_RAW:#x}")
            if text_id != 4937:
                raise AssertionError(f"menu text {text_id} != 4937")
            if count != 0:
                raise AssertionError(f"gossip items {count} != 0")
            off = 16
            (quest_count,) = struct.unpack_from("<I", body, off)
            off += 4
            quests = []
            for _ in range(quest_count):
                qid, _qicon, _qlvl = struct.unpack_from("<III", body, off)
                off += 12
                end = body.index(b"\x00", off)
                quests.append((qid, body[off:end].decode("utf-8", "replace")))
                off = end + 1
            if quests != [
                (179, "Dwarven Outfitters"),
                (233, "Coldridge Valley Mail Delivery"),
            ]:
                raise AssertionError(f"quests {quests!r}")
            if off != len(body):
                raise AssertionError(f"trailing bytes: consumed {off}/{len(body)}")
            print(f"GOSSIP-MENU text=4937 quests={quests!r}")

            # Real-client cache step (HandleNpcTextQueryOpcode).
            client.send_packet(
                CMSG_NPC_TEXT_QUERY, struct.pack("<IQ", 4937, NPC_RAW)
            )
            body, _ = smoke.read_until_opcode(
                client, SMSG_NPC_TEXT_UPDATE, 10, "NPC-TEXT-UPDATE"
            )
            (update_id,) = struct.unpack_from("<I", body, 0)
            if update_id != 4937:
                raise AssertionError(f"npc text update id {update_id} != 4937")
            off = 4
            first_male = None
            for i in range(8):
                (prob,) = struct.unpack_from("<f", body, off)
                off += 4
                end = body.index(b"\x00", off)
                male = body[off:end].decode("utf-8", "replace")
                off = end + 1
                end = body.index(b"\x00", off)
                off = end + 1 + 28  # female text + language + 3*(delay,emote)
                if i == 0:
                    if prob != 1.0:
                        raise AssertionError(f"option0 probability {prob} != 1.0")
                    first_male = male
            if off != len(body):
                raise AssertionError(
                    f"npc text trailing bytes: consumed {off}/{len(body)}"
                )
            if not first_male.startswith("Ah, well aren't you a sturdy-looking"):
                raise AssertionError(f"option0 text {first_male!r}")
            print(f"NPC-TEXT update ok option0={first_male[:40]!r}...")
        finally:
            smoke.logout(client, "defaultgossip")
    finally:
        smoke.sql_scalar(
            "characters",
            f"UPDATE characters SET map={omap}, position_x={ox}, "
            f"position_y={oy}, position_z={oz}, online=0 WHERE guid={GUID}",
        )
        restore_quests(saved_quests)
    print("LIVE-DEFAULT-GOSSIP-WIRE-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
