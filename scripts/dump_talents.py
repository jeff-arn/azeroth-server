#!/usr/bin/env python3
"""Parse VMaNGOS DBC files to dump talent data with spell names."""

import struct
import sys
from pathlib import Path

DBC_DIR = Path("/home/wow/wow-private-server/vmangos/run/bin/5875/dbc")

CLASS_NAMES = {
    1: "Warrior",
    2: "Paladin",
    3: "Hunter",
    4: "Rogue",
    5: "Priest",
    7: "Shaman",
    8: "Mage",
    9: "Warlock",
    11: "Druid",
}


def class_from_mask(mask):
    """Convert class bitmask to class ID (1-indexed, bit 0 = class 1)."""
    for i in range(1, 12):
        if mask & (1 << (i - 1)):
            return i
    return 0


def read_dbc(filepath):
    """Read a DBC file and return (records, string_block) where records is a list of uint32 tuples."""
    with open(filepath, "rb") as f:
        magic = f.read(4)
        if magic != b"WDBC":
            raise ValueError(f"Not a WDBC file: {filepath}")
        record_count, field_count, record_size, string_block_size = struct.unpack("<4I", f.read(16))

        fmt = f"<{field_count}I"
        records = []
        for _ in range(record_count):
            raw = f.read(record_size)
            # record_size might be > field_count*4 if there are padding bytes, but typically equal
            rec = struct.unpack_from(fmt, raw)
            records.append(rec)

        string_block = f.read(string_block_size)

    return records, string_block, field_count


def get_string(string_block, offset):
    """Extract a null-terminated string from the string block."""
    if offset == 0:
        return ""
    end = string_block.index(b"\x00", offset)
    return string_block[offset:end].decode("utf-8", errors="replace")


def main():
    # Parse TalentTab.dbc
    # Fields: 0=TalentTabID, 1-8=name(skip), 9=nameFlags(skip), 10=spellicon(skip),
    #         11=raceMask(skip), 12=ClassMask, 13=tabpage, 14=internalname(skip)
    tab_records, tab_strings, _ = read_dbc(DBC_DIR / "TalentTab.dbc")

    tabs = {}  # tab_id -> (class_id, tabpage, tab_name)
    for rec in tab_records:
        tab_id = rec[0]
        class_mask = rec[12]
        tabpage = rec[13]
        # Tab name is a localized string at fields 1-8, enUS at field 1
        tab_name = get_string(tab_strings, rec[1])
        class_id = class_from_mask(class_mask)
        tabs[tab_id] = (class_id, tabpage, tab_name)

    # Parse Talent.dbc
    # Fields: 0=TalentID, 1=TalentTab, 2=Row, 3=Col, 4-8=RankID[5],
    #         9-12=skip, 13=DependsOn, 14-15=skip, 16=DependsOnRank,
    #         17-18=skip, 19=flags(skip), 20=DependsOnSpell
    talent_records, _, _ = read_dbc(DBC_DIR / "Talent.dbc")

    # Collect all spell IDs we need names for (rank 1 of each talent)
    spell_ids_needed = set()
    talents = []
    for rec in talent_records:
        talent_id = rec[0]
        talent_tab = rec[1]
        row = rec[2]
        col = rec[3]
        rank_ids = [rec[4], rec[5], rec[6], rec[7], rec[8]]
        depends_on = rec[13]
        depends_on_rank = rec[16]

        num_ranks = sum(1 for r in rank_ids if r != 0)
        rank1_spell = rank_ids[0]

        if rank1_spell:
            spell_ids_needed.add(rank1_spell)

        talents.append(
            {
                "id": talent_id,
                "tab": talent_tab,
                "row": row,
                "col": col,
                "rank_ids": rank_ids,
                "num_ranks": num_ranks,
                "rank1_spell": rank1_spell,
                "depends_on": depends_on,
                "depends_on_rank": depends_on_rank,
            }
        )

    # Parse Spell.dbc - only need spell names for our spell IDs
    # Field 123 = SpellName (enUS string offset)
    print(f"Loading Spell.dbc (need {len(spell_ids_needed)} spell names)...", file=sys.stderr)
    spell_records, spell_strings, spell_field_count = read_dbc(DBC_DIR / "Spell.dbc")
    print(f"  Spell.dbc: {len(spell_records)} records, {spell_field_count} fields", file=sys.stderr)

    spell_names = {}
    for rec in spell_records:
        spell_id = rec[0]
        if spell_id in spell_ids_needed:
            name_offset = rec[120]
            spell_names[spell_id] = get_string(spell_strings, name_offset)

    print(f"  Resolved {len(spell_names)} spell names", file=sys.stderr)

    # Sort talents by class, tab, row, col
    def sort_key(t):
        tab_info = tabs.get(t["tab"], (99, 99, ""))
        return (tab_info[0], tab_info[1], t["row"], t["col"])

    talents.sort(key=sort_key)

    # Print header
    print(
        f"{'TalentID':>8}  {'Tab':>4}  {'Class':<10}  {'TreeName':<25}  {'Page':>4}  {'Row':>3}  {'Col':>3}  {'Ranks':>5}  {'Rank1SpellID':>12}  {'SpellName':<40}  {'DependsOn':>9}  {'DepRank':>7}"
    )
    print("-" * 180)

    current_class = None
    for t in talents:
        tab_info = tabs.get(t["tab"])
        if not tab_info:
            continue

        class_id, tabpage, tree_name = tab_info
        class_name = CLASS_NAMES.get(class_id, f"Unknown({class_id})")
        spell_name = spell_names.get(t["rank1_spell"], f"???({t['rank1_spell']})")

        if current_class != class_id:
            if current_class is not None:
                print()
            current_class = class_id

        depends_str = str(t["depends_on"]) if t["depends_on"] else ""
        dep_rank_str = str(t["depends_on_rank"]) if t["depends_on"] else ""

        print(
            f"{t['id']:>8}  {t['tab']:>4}  {class_name:<10}  {tree_name:<25}  {tabpage:>4}  {t['row']:>3}  {t['col']:>3}  {t['num_ranks']:>5}  {t['rank1_spell']:>12}  {spell_name:<40}  {depends_str:>9}  {dep_rank_str:>7}"
        )


if __name__ == "__main__":
    main()
