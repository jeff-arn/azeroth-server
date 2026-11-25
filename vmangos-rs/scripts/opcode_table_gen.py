#!/usr/bin/env python3
"""Generate the Rust opcode registry for mangos-protocol.

Source of truth: the C++ tables at their production-commit shape —
vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h (numeric ids) and
Opcodes.cpp (name/status/processing/handler registrations), with
preprocessor guards evaluated for SUPPORTED_CLIENT_BUILD =
CLIENT_BUILD_1_12_1. Method follows docs/rust-migration-evidence/
22-opcode-catalog.md; this script asserts its census so drift in
either source fails loudly instead of silently shifting the table.

Output: vmangos-rs/crates/mangos-protocol/src/opcode_table.rs
(checked-in generated data; regenerate, never hand-edit).
"""

import re

ENUM_HEADER = "vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h"
REGISTRATIONS = "vmangos/src/game/Server/Protocol/Opcodes.cpp"
OUT = "vmangos-rs/crates/mangos-protocol/src/opcode_table.rs"

# Build ordering used to evaluate #if SUPPORTED_CLIENT_BUILD guards.
BUILD_ORDER = {
    "CLIENT_BUILD_1_2_1": 4211,
    "CLIENT_BUILD_1_5_1": 4449,
    "CLIENT_BUILD_1_6_1": 4544,
    "CLIENT_BUILD_1_6_2": 4565,
    "CLIENT_BUILD_1_7_0": 4671,
    "CLIENT_BUILD_1_7_1": 4695,
    "CLIENT_BUILD_1_8_0": 4735,
    "CLIENT_BUILD_1_8_4": 4878,
    "CLIENT_BUILD_1_9_4": 5086,
    "CLIENT_BUILD_1_10_2": 5302,
    "CLIENT_BUILD_1_11_0": 5428,
    "CLIENT_BUILD_1_11_2": 5464,
    "CLIENT_BUILD_1_12_1": 5875,
}
TARGET = BUILD_ORDER["CLIENT_BUILD_1_12_1"]

STATUS = {
    "STATUS_AUTHED": "SessionStatus::Authed",
    "STATUS_LOGGEDIN": "SessionStatus::LoggedIn",
    "STATUS_TRANSFER": "SessionStatus::Transfer",
    "STATUS_LOGGEDIN_OR_RECENTLY_LOGGEDOUT": "SessionStatus::LoggedInOrRecentlyLoggedOut",
    "STATUS_NEVER": "SessionStatus::Never",
    "STATUS_UNHANDLED": "SessionStatus::Unhandled",
}

PROCESSING = {
    "PACKET_PROCESS_WORLD": "PacketProcessing::World",
    "PACKET_PROCESS_CHANNEL": "PacketProcessing::World",
    "PACKET_PROCESS_GROUP": "PacketProcessing::World",
    "PACKET_PROCESS_GUILD": "PacketProcessing::World",
    "PACKET_PROCESS_MAP": "PacketProcessing::Map",
    "PACKET_PROCESS_SELF_ITEMS": "PacketProcessing::Map",
    "PACKET_PROCESS_SPELLS": "PacketProcessing::Spells",
    "PACKET_PROCESS_MOVEMENT": "PacketProcessing::Movement",
    "PACKET_PROCESS_ASYNC": "PacketProcessing::Async",
    "PACKET_PROCESS_DB_QUERY": "PacketProcessing::Async",
    "PACKET_PROCESS_MAX_TYPE": "PacketProcessing::MaxType",
}


def parse_enum(text):
    m = re.search(r"enum OpcodesList\s*\{(.*?)\};", text, re.S)
    assert m, "Opcodes enum not found"
    values = {}
    current = -1
    for raw in m.group(1).splitlines():
        line = raw.split("//")[0].strip().rstrip(",")
        if not line:
            continue
        if "=" in line:
            name, _, num = line.partition("=")
            current = int(num.strip(), 0)
            values[name.strip()] = current
        else:
            current += 1
            values[line] = current
    return values


def guard_holds(expr, stack):
    expr = expr.strip()
    for part in expr.split("||"):
        part = part.strip()
        m = re.match(r"SUPPORTED_CLIENT_BUILD\s*(==|!=|>=|<=|>|<)\s*(CLIENT_BUILD_\w+)", part)
        if not m:
            stack.append(True)  # unknown guard form: catalog says all hold at 5875
            return
        op, name = m.groups()
        other = BUILD_ORDER[name]
        ok = {
            "==": TARGET == other,
            "!=": TARGET != other,
            ">=": TARGET >= other,
            "<=": TARGET <= other,
            ">": TARGET > other,
            "<": TARGET < other,
        }[op]
        if ok:
            stack.append(True)
            return
    stack.append(False)


def parse_registrations(text):
    rows = {}
    stack = []
    row_re = re.compile(
        r'StoreOpcode\((\w+),\s*"[^"]*",\s*(STATUS_\w+),\s*(PACKET_PROCESS_\w+),'
        r"\s*&WorldSession::(\w+)\);"
    )
    for raw in text.splitlines():
        line = re.sub(r"/\*.*?\*/", "", raw).strip()
        if line.startswith("#if"):
            guard_holds(line[3:], stack)
        elif line.startswith("#else"):
            stack[-1] = not stack[-1]
        elif line.startswith("#endif"):
            stack.pop()
        elif "StoreOpcode(" in line and all(stack):
            m = row_re.search(line)
            assert m, f"unparsed StoreOpcode line: {line}"
            name, status, processing, handler = m.groups()
            assert processing in PROCESSING, processing
            assert status in STATUS, status
            assert name not in rows, f"duplicate registration {name}"
            rows[name] = (status, processing, handler)
    return rows


def main():
    enum_values = parse_enum(open(ENUM_HEADER, encoding="utf-8").read())
    rows = parse_registrations(open(REGISTRATIONS, encoding="utf-8").read())

    num_types = enum_values["NUM_MSG_TYPES"]
    assert num_types == 828, num_types
    assert len(rows) == 826, f"registered rows: {len(rows)}"
    handlers = {h for _s, _p, h in rows.values()}
    assert len(handlers) == 276, f"distinct handlers: {len(handlers)}"
    never = sum(1 for s, _p, _h in rows.values() if s == "STATUS_NEVER")
    assert never == 525, f"STATUS_NEVER rows: {never}"

    by_id = {}
    for name, value in enum_values.items():
        if name != "NUM_MSG_TYPES":
            by_id[value] = name

    lines = [
        "// Generated by vmangos-rs/scripts/opcode_table_gen.py from",
        "// vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h + Opcodes.cpp.",
        "// Do not edit.",
        "",
        "use super::opcodes::OpcodeMeta;",
        "use super::world::{PacketProcessing, SessionStatus};",
        "",
        "/// Numeric opcode ids (1.12.1).",
        "pub mod opcode_id {",
    ]
    for value in sorted(by_id):
        lines.append(f"    pub const {by_id[value]}: u16 = {value};")
    lines.append("}")
    lines.append("")
    lines.append("/// All 828 slots indexed by opcode id; unregistered slots")
    lines.append("/// carry `registered: false` (C++ map lookup falls back to")
    lines.append("/// `emptyHandler`).")
    lines.append("pub static OPCODE_TABLE: [OpcodeMeta; 828] = [")
    for slot in range(828):
        name = by_id.get(slot)
        row = rows.get(name) if name else None
        if row:
            status, processing, handler = row
            lines.append(
                "    OpcodeMeta { "
                f'name: "{name}", status: {STATUS[status]}, '
                f"processing: {PROCESSING[processing]}, "
                f'handler_name: "{handler}", registered: true }},'
            )
        else:
            display = name if name else "<none>"
            lines.append(
                "    OpcodeMeta { "
                f'name: "{display}", status: SessionStatus::Unhandled, '
                "processing: PacketProcessing::MaxType, "
                'handler_name: "Handle_NULL", registered: false },'
            )
    lines.append("];")
    lines.append("")

    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
    print(f"rows={len(rows)} handlers={len(handlers)} never={never} slots=828")


if __name__ == "__main__":
    main()
