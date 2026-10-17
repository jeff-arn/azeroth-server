#!/usr/bin/env python3
"""Independent SMSG_CHAR_ENUM oracle for the Rust enum builder.

Reimplements Player::BuildEnumData (Player.cpp:1627-1727) +
HandleCharEnum (CharacterHandler.cpp:135-162) from the C++
source, reading the dev-bench databases with raw SQL — no Rust
code involved. Output: JSON {account_id, body_hex, count,
max_level} written outside git (snapshot character data).

Usage: char_enum_oracle_gen.py <account_id> <out.json>
Creds come from vmangos-rs/.env.test (DEV_MYSQL_PASSWORD).
"""

import json
import pathlib
import re
import struct
import subprocess
import sys

WOW_PATCH = 10
CHAR_FLAG_GHOST = 0x2000


def load_pw() -> str:
    env = pathlib.Path("vmangos-rs/.env.test").read_text()
    return re.search(r"DEV_MYSQL_PASSWORD=(\S+)", env).group(1).strip("'\"")


PW = load_pw()


def query(db: str, sql: str) -> list[list[str]]:
    r = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{PW}", "-N", "-B", db, "-e", sql],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr)
    return [line.split("\t") for line in r.stdout.splitlines() if line]


def u32(v: str) -> int:
    return int(v) & 0xFFFFFFFF if v not in ("NULL", "") else 0


def build(account_id: int) -> dict:
    pairs = {(int(r[0]), int(r[1])) for r in query("mangos", "SELECT race, class FROM playercreateinfo")}
    items = {
        int(r[0]): (int(r[1]), int(r[2]))
        for r in query(
            "mangos",
            "SELECT entry, display_id, inventory_type FROM item_template t1 "
            f"WHERE patch=(SELECT max(patch) FROM item_template t2 "
            f"WHERE t1.entry=t2.entry AND patch <= {WOW_PATCH})",
        )
    }
    families = {
        int(r[0]): int(r[1])
        for r in query(
            "mangos",
            "SELECT entry, pet_family FROM creature_template t1 "
            f"WHERE patch=(SELECT max(patch) FROM creature_template t2 "
            f"WHERE t1.entry=t2.entry AND patch <= {WOW_PATCH})",
        )
    }
    rows = query(
        "characters",
        "SELECT c.guid, c.name, c.race, c.class, c.gender, c.skin, c.face, "
        "c.hair_style, c.hair_color, c.facial_hair, c.level, c.zone, c.map, "
        "c.position_x, c.position_y, c.position_z, gm.guild_id, "
        "c.character_flags, c.played_time_total, cp.entry, cp.display_id, "
        "cp.level, c.equipment_cache FROM characters c "
        "LEFT JOIN character_pet cp ON c.guid=cp.owner_guid AND cp.slot='0' "
        "LEFT JOIN guild_member gm ON c.guid=gm.guid "
        f"WHERE c.account={account_id} ORDER BY c.create_time, c.guid LIMIT 0,10",
    )

    body = bytearray([0])
    count = 0
    max_level = 0
    for f in rows:
        level = u32(f[10])
        max_level = max(max_level, level)
        race, cls = u32(f[2]), u32(f[3])
        if (race, cls) not in pairs:
            continue
        body += struct.pack("<Q", u32(f[0]))
        body += f[1].encode() + b"\x00"
        body += bytes([race, cls, u32(f[4]), u32(f[5]), u32(f[6]), u32(f[7]), u32(f[8]), u32(f[9]), level])
        body += struct.pack("<II", u32(f[11]), u32(f[12]))
        body += struct.pack("<fff", float(f[13]), float(f[14]), float(f[15]))
        body += struct.pack("<I", u32(f[16]))
        flags = u32(f[17])
        body += struct.pack("<I", flags)
        body += bytes([0 if u32(f[18]) != 0 else 1])

        pet_display = pet_level = pet_family = 0
        if not (flags & CHAR_FLAG_GHOST) and cls in (3, 9):
            entry = u32(f[19])
            if entry in families:
                pet_display, pet_level = u32(f[20]), u32(f[21])
                pet_family = families[entry]
        body += struct.pack("<III", pet_display, pet_level, pet_family)

        tokens = [t for t in f[22].split(" ") if t] if f[22] != "NULL" else []
        for slot in range(20):
            item_id = int(tokens[slot * 2]) if slot * 2 < len(tokens) else 0
            display, inv = items.get(item_id, (0, 0)) if item_id in items else (0, 0)
            if item_id in items:
                display, inv = items[item_id]
                body += struct.pack("<I", display) + bytes([inv])
            else:
                body += struct.pack("<I", 0) + b"\x00"
        count += 1

    body[0] = count
    return {"account_id": account_id, "count": count, "max_level": max_level, "body_hex": body.hex()}


def main() -> None:
    account_id = int(sys.argv[1])
    out = pathlib.Path(sys.argv[2])
    out.write_text(json.dumps(build(account_id), indent=1) + "\n")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
