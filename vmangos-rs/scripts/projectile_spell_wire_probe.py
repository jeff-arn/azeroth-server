#!/usr/bin/env python3
"""Scripted wire probe: projectile spell flight (Fireball).

Regression for the Tier 4 Coldridge gnome probe:
spells with a projectile speed (`speed > 0`) were
rejected at prepare with SPELL_FAILED_UNKNOWN, so
every starter nuke (Fireball, Frostbolt, Shadow
Bolt, ...) was uncastable. Logs in the bench gnome
mage (Probegnome, guid 900014) placed ~13 yd from
Ragged Young Wolf spawn 344, casts Fireball (133)
at it and requires SPELL_START + SPELL_GO at launch
and a SPELLNONMELEEDAMAGELOG after the flight time
(distance/speed), plus the wolf's AI reaction.

Prints LIVE-PROJECTILE-SPELL-PASS on success. Run
from the repository root against the running bench
mangosd. The character position is restored after.
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402

CMSG_CAST_SPELL = 302
SMSG_CAST_RESULT = 304
SMSG_SPELL_START = 305
SMSG_SPELL_GO = 306
SMSG_AI_REACTION = 316
SMSG_SPELLNONMELEEDAMAGELOG = 592

GUID = 900014
FIREBALL = 133
# Unit guid wire raw: high 0xF130, entry 705, counter 344.
WOLF_RAW = (0xF130 << 48) | (705 << 24) | 344


def packed_guid(raw: int) -> bytes:
    out = bytearray([0])
    for i in range(8):
        byte = (raw >> (8 * i)) & 0xFF
        if byte:
            out[0] |= 1 << i
            out.append(byte)
    return bytes(out)


def main() -> int:
    old = smoke.sql_scalar(
        "characters",
        "SELECT CONCAT(map,' ',position_x,' ',position_y,' ',position_z,"
        f"' ',orientation) FROM characters WHERE guid={GUID}",
    )
    omap, ox, oy, oz, oo = old.split()
    smoke.sql_scalar(
        "characters",
        "UPDATE characters SET map=0, position_x=-6300.0, "
        "position_y=328.0, position_z=378.0, orientation=0.0, "
        f"health=51, online=0 WHERE guid={GUID}",
    )
    try:
        client, _pos = smoke.login_client("TESTUSER", 142, GUID)
        try:
            # Wire shape the actor parses: spell id,
            # target mask (unit), packed target guid.
            body = struct.pack("<IH", FIREBALL, 0x0002) + packed_guid(WOLF_RAW)
            client.send_packet(CMSG_CAST_SPELL, body)
            _go, skipped = smoke.read_until_opcode(
                client, SMSG_SPELL_GO, 10, "SPELL-GO"
            )
            launch_ops = [op for op, _ in skipped]
            if SMSG_CAST_RESULT in launch_ops:
                raise AssertionError(f"cast failed at launch: {skipped!r}")
            if SMSG_SPELL_START not in launch_ops:
                raise AssertionError(f"no SPELL_START before GO: {launch_ops!r}")
            packets = smoke.drain_until_idle(client, idle_secs=1.5, max_secs=6)
            flight_ops = [op for op, _ in packets]
            if SMSG_SPELLNONMELEEDAMAGELOG not in flight_ops:
                raise AssertionError(
                    f"projectile never landed (no damage log): {flight_ops!r}"
                )
            if SMSG_AI_REACTION not in flight_ops:
                raise AssertionError(f"wolf never reacted: {flight_ops!r}")
            print("SPELL-GO + damage log + AI reaction ok")
        finally:
            smoke.logout(client, "projectile")
    finally:
        smoke.sql_scalar(
            "characters",
            f"UPDATE characters SET map={omap}, position_x={ox}, "
            f"position_y={oy}, position_z={oz}, orientation={oo}, "
            f"online=0 WHERE guid={GUID}",
        )
    print("LIVE-PROJECTILE-SPELL-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
