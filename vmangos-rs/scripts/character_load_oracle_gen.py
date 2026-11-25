#!/usr/bin/env python3
"""Independent character-load oracle: dumps the 21 LoginQueryHolder
queries for one guid from the bench characters DB into a canonical
line format the Rust live test reproduces from its typed load.

Line format: `section|field=value|...`, floats as `bits:xxxxxxxx`
(f32 bit pattern), NULL as `NULL`, strings escaped (\\ -> \\\\,
| -> \\p, newline -> \\n). Field order = SELECT order.

Usage: character_load_oracle_gen.py <guid_low> <out.txt>
"""

import pathlib
import re
import struct
import subprocess
import sys


def pw() -> str:
    env = pathlib.Path("vmangos-rs/.env.test").read_text()
    return re.search(r"DEV_MYSQL_PASSWORD=(\S+)", env).group(1).strip("'\"")


PW = pw()


def query(sql: str) -> list[list[str]]:
    r = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{PW}", "-N", "-B", "characters", "-e", sql],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr)

    def unescape(v: str) -> str:
        # mysql -B escapes backslash first, then newline/tab.
        return (
            v.replace("\\\\", "\x00")
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r")
            .replace("\x00", "\\")
        )

    return [[unescape(field) for field in line.split("\t")] for line in r.stdout.splitlines() if line]


def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace("|", "\\p").replace("\n", "\\n")


def fbits(v: str) -> str:
    return "bits:%08x" % struct.unpack("<I", struct.pack("<f", float(v)))[0]


def emit(out: list[str], section: str, fields: list[tuple[str, str]]) -> None:
    out.append(section + "".join(f"|{k}={v}" for k, v in fields))


def main() -> None:
    guid = int(sys.argv[1])
    out_path = pathlib.Path(sys.argv[2])
    lines: list[str] = []

    rows = query(
        "SELECT guid, account, name, race, class, gender, level, xp, money, "
        "skin, face, hair_style, hair_color, facial_hair, bank_bag_slots, "
        "character_flags, position_x, position_y, position_z, map, "
        "orientation, known_taxi_mask, played_time_total, played_time_level, "
        "rest_bonus, logout_time, reset_talents_multiplier, "
        "reset_talents_time, transport_guid, transport_x, transport_y, "
        "transport_z, transport_o, extra_flags, stable_slots, "
        "death_expire_time, current_taxi_path, honor_rank_points, "
        "honor_highest_rank, honor_standing, honor_last_week_hk, "
        "honor_last_week_cp, honor_stored_hk, honor_stored_dk, "
        "watched_faction, drunk, health, power1, power2, power3, power4, "
        "power5, explored_zones, ammo_id, action_bars, world_phase_mask, "
        f"create_time, instance FROM characters WHERE guid={guid}"
    )
    if not rows:
        out_path.write_text("absent\n")
        print("absent")
        return
    f = rows[0]
    names = (
        "guid account name race class gender level xp money skin face "
        "hair_style hair_color facial_hair bank_bag_slots character_flags "
        "position_x position_y position_z map orientation known_taxi_mask "
        "played_time_total played_time_level rest_bonus logout_time "
        "reset_talents_multiplier reset_talents_time transport_guid "
        "transport_x transport_y transport_z transport_o extra_flags "
        "stable_slots death_expire_time current_taxi_path honor_rank_points "
        "honor_highest_rank honor_standing honor_last_week_hk "
        "honor_last_week_cp honor_stored_hk honor_stored_dk watched_faction "
        "drunk health power1 power2 power3 power4 power5 explored_zones "
        "ammo_id action_bars world_phase_mask create_time instance"
    ).split()
    float_idx = {16, 17, 18, 20, 24, 29, 30, 31, 32, 37, 41}
    lines.append("row" + "".join(f"|{n}={fbits(f[i]) if i in float_idx else esc(f[i])}" for i, n in enumerate(names)))

    g = query(f"SELECT group_id FROM group_member WHERE member_guid={guid}")
    lines.append(f"group|{g[0][0] if g else 'NULL'}")

    for r in query(
        "SELECT id, permanent, map, reset_time FROM character_instance "
        f"LEFT JOIN instance ON instance=id WHERE guid={guid}"
    ):
        emit(lines, "instance", list(zip(["id", "permanent", "map", "reset_time"], r)))

    for r in query(
        "SELECT caster_guid, item_guid, spell, stacks, charges, "
        "base_points0, base_points1, base_points2, periodic_time0, "
        "periodic_time1, periodic_time2, max_duration, duration, "
        f"effect_index_mask FROM character_aura WHERE guid={guid}"
    ):
        keys = [
            "caster",
            "item",
            "spell",
            "stacks",
            "charges",
            "bp0",
            "bp1",
            "bp2",
            "pt0",
            "pt1",
            "pt2",
            "max_dur",
            "dur",
            "mask",
        ]
        vals = [fbits(v) if i in (5, 6, 7) else v for i, v in enumerate(r)]
        emit(lines, "aura", list(zip(keys, vals)))

    for r in query(f"SELECT spell, active, disabled FROM character_spell WHERE guid={guid} ORDER BY spell"):
        emit(lines, "spell", list(zip(["spell", "active", "disabled"], r)))

    for r in query(
        "SELECT quest, status, rewarded, explored, timer, mob_count1, "
        "mob_count2, mob_count3, mob_count4, item_count1, item_count2, "
        f"item_count3, item_count4, reward_choice FROM character_queststatus WHERE guid={guid}"
    ):
        emit(lines, "quest", [(f"f{i}", v) for i, v in enumerate(r)])

    for r in query(f"SELECT victim_type, victim_id, cp, date, type FROM character_honor_cp WHERE guid={guid}"):
        emit(lines, "honorcp", [("vt", r[0]), ("vid", r[1]), ("cp", fbits(r[2])), ("date", r[3]), ("type", r[4])])

    for r in query(f"SELECT faction, standing, flags FROM character_reputation WHERE guid={guid}"):
        emit(lines, "rep", list(zip(["faction", "standing", "flags"], r)))

    def item_fields(r: list[str], guid_i: int, id_i: int, loot_i: int):
        return [
            ("creator", r[0]),
            ("gift", r[1]),
            ("count", r[2]),
            ("duration", r[3]),
            ("charges", esc(r[4])),
            ("flags", r[5]),
            ("ench", esc(r[6])),
            ("rprop", r[7]),
            ("dura", r[8]),
            ("text", r[9]),
            ("item_guid", r[guid_i]),
            ("item_id", r[id_i]),
            ("gen_loot", r[loot_i]),
        ]

    for r in query(
        "SELECT * FROM (SELECT creator_guid, gift_creator_guid, count, "
        "duration, charges, flags, enchantments, random_property_id, "
        "durability, text, bag, slot, item_guid, item_instance.item_id, "
        "generated_loot FROM character_inventory JOIN item_instance ON "
        f"character_inventory.item_guid=item_instance.guid WHERE character_inventory.guid={guid}) "
        "as t ORDER BY bag, slot"
    ):
        emit(lines, "inv", item_fields(r, 12, 13, 14) + [("bag", r[10]), ("slot", r[11])])

    for r in query(f"SELECT guid, item_id, amount, property FROM item_loot WHERE owner_guid={guid}"):
        emit(lines, "loot", list(zip(["guid", "item", "amount", "prop"], r)))

    for r in query(f"SELECT button, action, type FROM character_action WHERE guid={guid} ORDER BY button"):
        emit(lines, "action", list(zip(["button", "action", "type"], r)))

    for r in query(f"SELECT friend, flags FROM character_social WHERE guid={guid} LIMIT 255"):
        emit(lines, "social", list(zip(["friend", "flags"], r)))

    for r in query(f"SELECT map, zone, position_x, position_y, position_z FROM character_homebind WHERE guid={guid}"):
        emit(lines, "home", [("map", r[0]), ("zone", r[1]), ("x", fbits(r[2])), ("y", fbits(r[3])), ("z", fbits(r[4]))])

    for r in query(
        "SELECT spell, spell_expire_time, category, category_expire_time, "
        f"item_id FROM character_spell_cooldown WHERE guid={guid}"
    ):
        emit(lines, "cd", [(f"f{i}", v) for i, v in enumerate(r)])

    for r in query(f"SELECT guild_id, `rank` FROM guild_member WHERE guid={guid}"):
        emit(lines, "guild", list(zip(["id", "rank"], r)))

    for r in query(
        "SELECT instance_id, team, join_x, join_y, join_z, join_o, join_map "
        f"FROM character_battleground_data WHERE guid={guid}"
    ):
        emit(
            lines,
            "bg",
            [
                ("inst", r[0]),
                ("team", r[1]),
                ("x", fbits(r[2])),
                ("y", fbits(r[3])),
                ("z", fbits(r[4])),
                ("o", fbits(r[5])),
                ("map", r[6]),
            ],
        )

    for r in query(f"SELECT type, time, data FROM character_account_data WHERE guid={guid}"):
        emit(lines, "acctdata", [("type", r[0]), ("time", r[1]), ("data", esc(r[2]))])

    for r in query(f"SELECT skill, value, max FROM character_skills WHERE guid={guid}"):
        emit(lines, "skill", list(zip(["skill", "value", "max"], r)))

    for r in query(
        "SELECT id, message_type, sender_guid, receiver_guid, subject, "
        "item_text_id, expire_time, deliver_time, money, cod, checked, "
        f"stationery, mail_template_id, has_items FROM mail WHERE receiver_guid={guid} ORDER BY id DESC"
    ):
        vals = [esc(v) if i == 4 else v for i, v in enumerate(r)]
        emit(lines, "mail", [(f"f{i}", v) for i, v in enumerate(vals)])

    for r in query(
        "SELECT creator_guid, gift_creator_guid, count, duration, charges, "
        "flags, enchantments, random_property_id, durability, text, mail_id, "
        "item_guid, item_instance.item_id, generated_loot FROM mail_items "
        f"JOIN item_instance ON item_guid=guid WHERE receiver_guid={guid}"
    ):
        emit(lines, "mailitem", item_fields(r, 11, 12, 13) + [("mail_id", r[10])])

    for r in query(f"SELECT skill, value FROM character_forgotten_skills WHERE guid={guid}"):
        emit(lines, "forgotten", list(zip(["skill", "value"], r)))

    out_path.write_text("\n".join(lines) + "\n")
    print(f"{len(lines)} lines")


if __name__ == "__main__":
    main()
