#!/usr/bin/env python3
"""Canonical raw-state dump of one character's save surface.

Dumps every table the fork `Player::SaveToDB` family writes
for one guid, sorted by primary key, into
`section|col=value|...` lines (mysql batch text, NULL kept).
Two dumps around a login/logout cycle diff cleanly; that is
the Phase 6 differential save oracle.

Usage: character_state_dump.py <db> <guid_low> <out.txt>
Run from the repo root (reads vmangos-rs/.env.test).
"""

import pathlib
import re
import subprocess
import sys


def pw() -> str:
    env = pathlib.Path("vmangos-rs/.env.test").read_text()
    return re.search(r"DEV_MYSQL_PASSWORD=(\S+)", env).group(1).strip("'\"")


DB = sys.argv[1]
PW = pw()


def query(sql: str) -> list[list[str]]:
    r = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{PW}", "-N", "-B", DB, "-e", sql],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr)
    return [line.split("\t") for line in r.stdout.splitlines() if line]


def columns(table: str) -> list[str]:
    return [r[0] for r in query(f"SHOW COLUMNS FROM `{table}`")]


def main() -> None:
    guid = int(sys.argv[2])
    out_path = pathlib.Path(sys.argv[3])
    lines: list[str] = []

    def dump(section: str, table: str, where: str, order: str) -> None:
        cols = columns(table)
        for r in query(f"SELECT * FROM `{table}` WHERE {where} ORDER BY {order}"):
            body = "".join(f"|{c}={v}" for c, v in zip(cols, r))
            lines.append(section + body)

    dump("characters", "characters", f"guid={guid}", "guid")
    dump("aura", "character_aura", f"guid={guid}", "spell, effect_index_mask")
    dump("spell", "character_spell", f"guid={guid}", "spell")
    dump("cooldown", "character_spell_cooldown", f"guid={guid}", "spell, item_id")
    dump("skill", "character_skills", f"guid={guid}", "skill")
    dump("forgotten", "character_forgotten_skills", f"guid={guid}", "skill")
    dump("quest", "character_queststatus", f"guid={guid}", "quest")
    dump("reputation", "character_reputation", f"guid={guid}", "faction")
    dump("honor_cp", "character_honor_cp", f"guid={guid}", "date, type, victim_id, cp")
    dump("action", "character_action", f"guid={guid}", "button")
    dump("homebind", "character_homebind", f"guid={guid}", "guid")
    dump("bgdata", "character_battleground_data", f"guid={guid}", "guid")
    dump("instance", "character_instance", f"guid={guid}", "instance")
    # Inventory joins item_instance; loot hangs off item guids.
    inv_cols = columns("character_inventory")
    item_cols = columns("item_instance")
    rows = query(
        "SELECT ci.*, ii.* FROM character_inventory ci "
        "JOIN item_instance ii ON ii.guid = ci.item_guid "
        f"WHERE ci.guid={guid} ORDER BY ci.bag, ci.slot, ci.item_guid"
    )
    for r in rows:
        body = "".join(f"|{c}={v}" for c, v in zip(inv_cols + item_cols, r))
        lines.append("inventory" + body)
    loot_cols = columns("item_loot")
    rows = query(f"SELECT il.* FROM item_loot il WHERE il.owner_guid={guid} ORDER BY il.guid, il.item_id")
    for r in rows:
        body = "".join(f"|{c}={v}" for c, v in zip(loot_cols, r))
        lines.append("loot" + body)

    out_path.write_text("\n".join(lines) + "\n")
    print(f"{len(lines)} lines -> {out_path}")


if __name__ == "__main__":
    main()
