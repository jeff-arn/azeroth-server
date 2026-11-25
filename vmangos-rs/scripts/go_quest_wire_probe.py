#!/usr/bin/env python3
"""Scripted wire probe: questgiver gameobject flow (Tier 3 step 6).

Expects the bench fixture (BOT_Zulrath, guid 3) pre-positioned at the
Control Console spawn 13167 (entry 4141, map 1 at 825.53/-2686.73/91.6111)
by the caller. Sends CMSG_GAMEOBJ_USE and requires
SMSG_QUESTGIVER_QUEST_LIST carrying the GO guid and title.

Prints LIVE-GO-QUEST-WIRE-PASS on success. Run from the repository root.
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402

CMSG_GAMEOBJ_USE = 177
SMSG_QUESTGIVER_QUEST_LIST = 389

# GO guid wire raw: high 0xF110, entry 4141, counter 13167.
GO_RAW = (0xF110 << 48) | (4141 << 24) | 13167


def main() -> int:
    client, _pos = smoke.login_client("BOT_Zulrath", 101, 3)
    try:
        client.send_packet(CMSG_GAMEOBJ_USE, struct.pack("<Q", GO_RAW))
        body, _ = smoke.read_until_opcode(
            client, SMSG_QUESTGIVER_QUEST_LIST, 10, "GO-QUEST-LIST"
        )
        (guid,) = struct.unpack_from("<Q", body, 0)
        if guid != GO_RAW:
            raise AssertionError(f"list guid {guid:#x} != {GO_RAW:#x}")
        end = body.index(b"\x00", 8)
        title = body[8:end].decode("utf-8", "replace")
        if title != "Control Console":
            raise AssertionError(f"list title {title!r}")
        count = body[end + 9]
        print(f"GO-QUEST-LIST title={title!r} quests={count}")
    finally:
        smoke.logout(client, "A")
    print("LIVE-GO-QUEST-WIRE-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
