#!/usr/bin/env python3
"""Scripted wire probe: gossip script path (Tier 3 Slice 2 step 5).

Logs in one bench client next to the dev-bench res-fixer
spawn (entry 13857, spawn guid 2530718, map 1 at
1660/-4370/31.25), sends CMSG_GOSSIP_HELLO and requires an
SMSG_GOSSIP_MESSAGE with the fork menu (text 6440, one chat
item), then selects option 0 and requires SMSG_GOSSIP_COMPLETE.

Prints LIVE-GOSSIP-WIRE-PASS on success. Run from the
repository root against the running bench mangosd.
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402

CMSG_GOSSIP_HELLO = 379
CMSG_GOSSIP_SELECT_OPTION = 380
SMSG_GOSSIP_MESSAGE = 381
SMSG_GOSSIP_COMPLETE = 382

# Unit guid wire raw: high 0xF130, entry 13857, counter 2530718.
NPC_RAW = (0xF130 << 48) | (13857 << 24) | 2530718


def parse_gossip_message(body: bytes) -> tuple[int, int, list[tuple[int, int, int, str]], int]:
    # Fork SendGossipMenu writes the raw u64 guid
    # (`data << ObjectGuid`), not a packed guid.
    guid, text_id, count = struct.unpack_from("<QII", body, 0)
    off = 16
    items = []
    for _ in range(count):
        index, icon, coded = struct.unpack_from("<IBB", body, off)
        off += 6
        end = body.index(b"\x00", off)
        text = body[off:end].decode("utf-8", "replace")
        off = end + 1
        items.append((index, icon, coded, text))
    (quest_count,) = struct.unpack_from("<I", body, off)
    return guid, text_id, items, quest_count


def main() -> int:
    client, _pos = smoke.login_client("BOT_Zulrath", 101, 3)
    try:
        client.send_packet(CMSG_GOSSIP_HELLO, struct.pack("<Q", NPC_RAW))
        body, _ = smoke.read_until_opcode(client, SMSG_GOSSIP_MESSAGE, 10, "GOSSIP-MENU")
        guid, text_id, items, quest_count = parse_gossip_message(body)
        if guid != NPC_RAW:
            raise AssertionError(f"menu guid {guid:#x} != {NPC_RAW:#x}")
        if text_id != 6440:
            raise AssertionError(f"menu text {text_id} != 6440")
        if len(items) != 1 or items[0][0] != 0 or items[0][1] != 0 or not items[0][3]:
            raise AssertionError(f"menu items {items!r}")
        if quest_count != 0:
            raise AssertionError(f"quest count {quest_count}")
        print(f"GOSSIP-MENU text=6440 item={items[0][3]!r}")

        client.send_packet(
            CMSG_GOSSIP_SELECT_OPTION, struct.pack("<QI", NPC_RAW, 0)
        )
        smoke.read_until_opcode(client, SMSG_GOSSIP_COMPLETE, 10, "GOSSIP-CLOSE")
        print("GOSSIP-CLOSE")
    finally:
        smoke.logout(client, "A")
    print("LIVE-GOSSIP-WIRE-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
