#!/usr/bin/env python3
"""Generate the Rust 5875 update-field table for mangos-protocol.

Source of truth: vmangos/src/game/Objects/UpdateFields_1_12_1.cpp
(the `g_updateFieldsData` rows with absolute offsets). The sibling
header's `// 0x...` comments are known-stale for CONTAINER/PLAYER
(they understate by 6) and are never read; only .cpp rows count.

This script asserts its census (324 rows, the eight *_END offsets,
per-mask offset contiguity, closed UF_TYPE/UF_FLAG/TYPEMASK
vocabularies) so drift in the C++ source fails loudly.

Output: vmangos-rs/crates/mangos-protocol/src/update_field_table.rs
(checked-in generated data; regenerate, never hand-edit).
Run from the repository root, then `cargo fmt -p mangos-protocol`.
"""

import re
import sys

SOURCE = "vmangos/src/game/Objects/UpdateFields_1_12_1.cpp"
OUT = "vmangos-rs/crates/mangos-protocol/src/update_field_table.rs"

EXPECTED_ROWS = 324
EXPECTED_ENDS = {
    "OBJECT_END": 0x6,
    "ITEM_END": 0x30,
    "CONTAINER_END": 0x7A,
    "UNIT_END": 0xBC,
    "PLAYER_END": 0x502,
    "GAMEOBJECT_END": 0x1A,
    "DYNAMICOBJECT_END": 0x10,
    "CORPSE_END": 0x26,
}
# Each mask group's first row starts where its parent block ends
# (all object types share the OBJECT block at offsets 0..6).
EXPECTED_STARTS = {
    "TYPEMASK_OBJECT": 0x0,
    "TYPEMASK_ITEM": 0x6,
    "TYPEMASK_CONTAINER": 0x30,
    "TYPEMASK_UNIT": 0x6,
    "TYPEMASK_PLAYER": 0xBC,
    "TYPEMASK_GAMEOBJECT": 0x6,
    "TYPEMASK_DYNAMICOBJECT": 0x6,
    "TYPEMASK_CORPSE": 0x6,
}

TYPEMASKS = {
    "TYPEMASK_OBJECT",
    "TYPEMASK_ITEM",
    "TYPEMASK_CONTAINER",
    "TYPEMASK_UNIT",
    "TYPEMASK_PLAYER",
    "TYPEMASK_GAMEOBJECT",
    "TYPEMASK_DYNAMICOBJECT",
    "TYPEMASK_CORPSE",
}
VALUE_TYPES = {
    "UF_TYPE_NONE": "None",
    "UF_TYPE_INT": "Int",
    "UF_TYPE_TWO_SHORT": "TwoShort",
    "UF_TYPE_FLOAT": "Float",
    "UF_TYPE_GUID": "Guid",
    "UF_TYPE_BYTES": "Bytes",
    "UF_TYPE_BYTES2": "Bytes2",
}
FLAGS = {
    "UF_FLAG_NONE": "NONE",
    "UF_FLAG_PUBLIC": "PUBLIC",
    "UF_FLAG_PRIVATE": "PRIVATE",
    "UF_FLAG_OWNER_ONLY": "OWNER_ONLY",
    "UF_FLAG_UNK1": "UNK1",
    "UF_FLAG_UNK2": "UNK2",
    "UF_FLAG_SPECIAL_INFO": "SPECIAL_INFO",
    "UF_FLAG_GROUP_ONLY": "GROUP_ONLY",
    "UF_FLAG_UNK5": "UNK5",
    "UF_FLAG_DYNAMIC": "DYNAMIC",
}

ROW = re.compile(
    r'^\s*\{(TYPEMASK_[A-Z]+),\s*"([A-Z0-9_]+)",\s*(0x[0-9A-Fa-f]+|\d+),\s*(\d+),\s*'
    r"(UF_TYPE_[A-Z0-9_]+),\s*([A-Z0-9_ +]+)\},?\s*$"
)


def fail(msg):
    print(f"update_fields_gen: {msg}", file=sys.stderr)
    sys.exit(1)


def main():
    text = open(SOURCE, encoding="utf-8").read()

    m = re.search(r"g_updateFieldsData\s*=\s*\{\{", text)
    if not m:
        fail("g_updateFieldsData array not found")
    declared = re.search(r"array<UpdateFieldData,\s*(\d+)>", text)
    if not declared:
        fail("declared array size not found")

    rows = []
    for lineno, line in enumerate(text.splitlines(), 1):
        rm = ROW.match(line)
        if not rm:
            if line.strip().startswith("{TYPEMASK_"):
                fail(f"unparsed field row at line {lineno}: {line.strip()}")
            continue
        mask, name, offset, size, vtype, flags = rm.groups()
        if mask not in TYPEMASKS:
            fail(f"line {lineno}: unknown mask {mask}")
        if vtype not in VALUE_TYPES:
            fail(f"line {lineno}: unknown value type {vtype}")
        flag_names = [f.strip() for f in flags.split("+")]
        for f in flag_names:
            if f not in FLAGS:
                fail(f"line {lineno}: unknown flag {f}")
        rows.append(
            {
                "mask": mask,
                "name": name,
                "offset": int(offset, 0),
                "size": int(size),
                "vtype": vtype,
                "flags": flag_names,
            }
        )

    if len(rows) != EXPECTED_ROWS:
        fail(f"expected {EXPECTED_ROWS} rows, parsed {len(rows)}")
    if int(declared.group(1)) != len(rows):
        fail(f"declared size {declared.group(1)} != parsed rows {len(rows)}")

    names = [r["name"] for r in rows]
    if len(set(names)) != len(names):
        dupes = sorted({n for n in names if names.count(n) > 1})
        fail(f"duplicate field names: {dupes}")

    ends = {r["name"]: r["offset"] for r in rows if r["name"].endswith("_END")}
    if ends != EXPECTED_ENDS:
        fail(f"END offsets mismatch: got {ends}")

    # Per-mask contiguity: rows tile [start, END) with no gaps, which
    # is what makes the C++ flags-array fill total.
    for mask, start in EXPECTED_STARTS.items():
        group = [r for r in rows if r["mask"] == mask]
        if not group:
            fail(f"no rows for {mask}")
        cursor = start
        for r in group:
            if r["offset"] != cursor:
                fail(f"{mask}: {r['name']} at {r['offset']:#x}, cursor at {cursor:#x}")
            cursor += r["size"]
        end_name = mask.removeprefix("TYPEMASK_") + "_END"
        if ends[end_name] != cursor:
            fail(f"{mask}: tiles end at {cursor:#x}, {end_name} is {ends[end_name]:#x}")

    out = []
    out.append("// @generated by vmangos-rs/scripts/update_fields_gen.py from")
    out.append("// vmangos/src/game/Objects/UpdateFields_1_12_1.cpp (build 5875).")
    out.append("// Regenerate with that script; never hand-edit.")
    out.append("")
    out.append("use crate::update_fields::{TypeMask, UpdateFieldData, UpdateFieldFlags, UpdateFieldType};")
    out.append("")
    for name in [
        "OBJECT_END",
        "ITEM_END",
        "CONTAINER_END",
        "UNIT_END",
        "PLAYER_END",
        "GAMEOBJECT_END",
        "DYNAMICOBJECT_END",
        "CORPSE_END",
    ]:
        label = {
            "GAMEOBJECT_END": "GameObject",
            "DYNAMICOBJECT_END": "DynamicObject",
        }.get(name, name.removesuffix("_END").title())
        out.append(f"/// Word count of the `{label}` values block (5875).")
        out.append(f"pub const {name}: u16 = {ends[name]};")
    out.append("")
    out.append("/// `g_updateFieldsData` (`UpdateFields_1_12_1.cpp`), 5875 shape:")
    out.append(f"/// {len(rows)} rows (316 fields + 8 `*_END` markers).")
    out.append(f"pub static UPDATE_FIELDS_5875: [UpdateFieldData; {len(rows)}] = [")
    for r in rows:
        flags_expr = " | ".join(f"UpdateFieldFlags::{FLAGS[f]}" for f in r["flags"])
        mask_short = r["mask"].removeprefix("TYPEMASK_")
        out.append(
            "    UpdateFieldData { "
            f"object_type_mask: TypeMask::{mask_short}, "
            f'name: "{r["name"]}", '
            f"offset: {r['offset']}, "
            f"size: {r['size']}, "
            f"value_type: UpdateFieldType::{VALUE_TYPES[r['vtype']]}, "
            f"flags: UpdateFieldFlags({flags_expr}) }},"
        )
    out.append("];")
    out.append("")
    out.append("/// Named field offsets (the `E*Fields` enums of")
    out.append("/// `UpdateFields_1_12_1.h`, at the .cpp rows' absolute offsets).")
    out.append("pub mod offsets {")
    for r in rows:
        out.append(f"    pub const {r['name']}: u16 = {r['offset']};")
    out.append("}")
    out.append("")

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out))
    print(f"rows={len(rows)} ends={len(ends)} wrote {OUT}")


if __name__ == "__main__":
    main()
