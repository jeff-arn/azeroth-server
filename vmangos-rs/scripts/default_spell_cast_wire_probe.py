#!/usr/bin/env python3
"""Scripted wire probe: default (creation) spells are castable.

Regression for the Tier 4 Coldridge gnome probe: a
character created through the client (Probegnome,
guid 900014) has no `character_spell` rows — fork
`LearnDefaultSpells` adds race/class defaults as
dependent spells, which `_SaveSpells` never
persists. The login packet merged defaults into
SMSG_INITIAL_SPELLS, so the client spellbook looked
complete, but the cast gate only consulted persisted
rows and every starter spell failed with
SPELL_FAILED_UNKNOWN (145) in both scripted and real
clients.

Logs in the bench gnome, casts Frost Armor (168, a
gnome mage default with no persisted row) and
requires SMSG_SPELL_GO with no SMSG_CAST_RESULT,
then verifies after logout that the dependent spell
was still not persisted to `character_spell`.

Prints LIVE-DEFAULT-SPELL-CAST-PASS on success. Run
from the repository root against the running bench
mangosd.
"""

from __future__ import annotations

import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402

CMSG_CAST_SPELL = 302
SMSG_CAST_RESULT = 304
SMSG_SPELL_GO = 306

GUID = 900014
FROST_ARMOR = 168


def main() -> int:
    rows = smoke.sql_scalar(
        "characters",
        f"SELECT COUNT(*) FROM character_spell WHERE guid={GUID} AND spell={FROST_ARMOR}",
    )
    if rows != "0":
        raise AssertionError(
            f"fixture drift: character_spell has {rows} rows for spell "
            f"{FROST_ARMOR}; the probe requires a creation-only character"
        )
    client, _pos = smoke.login_client("TESTUSER", 142, GUID)
    try:
        # Phase-4 wire shape: spell id + cast counter;
        # no explicit targets resolves to self.
        client.send_packet(CMSG_CAST_SPELL, struct.pack("<IH", FROST_ARMOR, 0))
        _body, skipped = smoke.read_until_opcode(
            client, SMSG_SPELL_GO, 8, "SPELL-GO"
        )
        failures = [op for op, _ in skipped if op == SMSG_CAST_RESULT]
        if failures:
            raise AssertionError(f"cast answered SMSG_CAST_RESULT: {skipped!r}")
        print(f"SPELL-GO frost-armor ok (skipped {len(skipped)} packets)")
    finally:
        smoke.logout(client, "defaultspell")
    rows = smoke.sql_scalar(
        "characters",
        f"SELECT COUNT(*) FROM character_spell WHERE guid={GUID} AND spell={FROST_ARMOR}",
    )
    if rows != "0":
        raise AssertionError(
            "dependent default spell was persisted to character_spell "
            "(fork _SaveSpells skips dependent spells)"
        )
    print("LIVE-DEFAULT-SPELL-CAST-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
