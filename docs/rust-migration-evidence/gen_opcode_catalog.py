#!/usr/bin/env python3
"""Per-opcode payload catalogue generator (Phase 0, section 3.5 / item #35).

Reads (read-only):
  vmangos/src/game/Server/Protocol/Opcodes.cpp      (StoreOpcode table)
  vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h  (enum -> numeric values)
  vmangos/src/game/**/*.cpp/.h                        (handler definitions)

Emits:
  22-opcode-catalog.csv  (one row per StoreOpcode registration)

Method: regex parse of StoreOpcode lines; handler located by searching for
`void WorldSession::<Handler>(`; handler body extracted by brace matching
from the definition's opening brace; ordered ByteBuffer read sequence
extracted by regex (operator>>, read<T>, read_skip<T>, ReadCString,
readPackGUID, ReadForCaster, etc.); loop/conditional enclosure flagged by
mapping control-statement brace ranges onto read offsets.
"""

import csv
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GAME = os.path.join(REPO, "vmangos/src/game")
OPCODES_CPP = os.path.join(GAME, "Server/Protocol/Opcodes.cpp")
ENUM_H = os.path.join(GAME, "Server/Protocol/Opcodes_1_12_1.h")

STUB_HANDLERS = {"Handle_NULL", "Handle_ServerSide", "Handle_Deprecated", "Handle_EarlyProccess"}

STORE_RE = re.compile(
    r"/\*(0x[0-9A-Fa-f]+)\*/\s*StoreOpcode\(\s*(\w+)\s*,\s*\"([^\"]+)\""
    r"\s*,\s*(\w+)\s*,\s*(\w+)\s*,"
    r"\s*(?:/\*.*?\*/\s*)?&WorldSession::(\w+)\s*\)"
)
IF_RE = re.compile(r"^\s*#\s*(if|ifdef|ifndef|elif|else|endif)\b(.*)$")
ENUM_RE = re.compile(r"^\s*(\w+)\s*(?:=\s*(\d+))?\s*,?\s*$")
DEF_RE = re.compile(
    r"void\s+WorldSession::(\w+)\s*\(\s*WorldPacket\s*&\s*"
    r"(?:(\w+)|/\*.*?\*/)?\s*\)"
)
# fallback when the packet parameter is unnamed (commented out): such
# bodies cannot reference it, but scan the usual names anyway (expect none)
PKT_FALLBACK = r"(?:recv_data|recvPacket|recv_packet|recvpacket)"

READ_METHOD_RE = re.compile(
    r"(\w+)\s*\.\s*(read|read_skip|ReadCString|readPackGUID|"
    r"ReadPackedGuid|ReadGuid|readGUID)\s*(<[^;{}]*?>)?\s*\("
)
READFOR_RE = re.compile(r"(\w+(?:\.\w+)*)\.ReadForCaster\s*\(")
CTRL_RE = re.compile(r"\b(for|while|if|switch|catch)\b")


def parse_registrations(path):
    regs, guard_stack, active = [], [], ""
    with open(path) as f:
        for ln, line in enumerate(f, 1):
            m = IF_RE.match(line)
            if m:
                kind, cond = m.group(1), m.group(2).strip()
                if kind in ("if", "ifdef", "ifndef"):
                    guard_stack.append(cond)
                elif kind == "endif":
                    guard_stack.pop()
                active = " && ".join(guard_stack)
                continue
            m = STORE_RE.search(line)
            if m:
                hexv, enum_name, str_name, status, proc, handler = m.groups()
                regs.append(
                    {
                        "line": ln,
                        "hex": hexv,
                        "dec": int(hexv, 16),
                        "enum": enum_name,
                        "name": str_name,
                        "status": status,
                        "processing": proc,
                        "handler": handler,
                        "guard": active,
                    }
                )
    return regs


def parse_enum(path):
    values, cur = {}, 0
    with open(path) as f:
        for line in f:
            line = line.split("//")[0].strip()
            if not line or line.startswith(("enum", "{", "}", "#")):
                continue
            m = ENUM_RE.match(line)
            if m and m.group(1) not in ("enum",):
                name, val = m.group(1), m.group(2)
                if val is not None:
                    cur = int(val)
                values[name] = cur
                cur += 1
    return values


def find_handler_defs():
    defs = {}
    for root, _, files in os.walk(GAME):
        for fn in files:
            if not fn.endswith((".cpp", ".h")):
                continue
            p = os.path.join(root, fn)
            try:
                with open(p, encoding="utf-8", errors="replace") as f:
                    text = f.read()
            except OSError:
                continue
            for m in DEF_RE.finditer(text):
                name = m.group(1)
                pkt = m.group(2) or PKT_FALLBACK
                line = text.count("\n", 0, m.start()) + 1
                # prefer .cpp definitions over .h declarations
                prev = defs.get(name)
                if prev is None or (prev[0].endswith(".h") and p.endswith(".cpp")):
                    defs[name] = (os.path.relpath(p, REPO), line, m.end(), text, pkt)
    return defs


def extract_body(text, def_end):
    """Brace-match the function body starting at def_end. Returns body str
    (without outer braces) and body start offset in text."""
    i = text.find("{", def_end)
    if i < 0:
        return "", def_end
    depth, j, start = 0, i, i + 1
    n = len(text)
    while j < n:
        c = text[j]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[start:j], start
        j += 1
    return text[start:], start


def control_ranges(body):
    """Map (start, end, kind) ranges of for/while/if/switch bodies."""
    out = []
    for m in CTRL_RE.finditer(body):
        kind = m.group(1)
        if kind in ("for", "while", "if", "switch", "catch"):
            # skip balanced condition parens
            p = body.find("(", m.end())
            if p < 0 or len(body[m.end() : p].strip()) > 1:
                continue
            depth, j = 0, p
            while j < len(body):
                if body[j] == "(":
                    depth += 1
                elif body[j] == ")":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            cond_end = j + 1
        else:
            cond_end = m.end()
        tail = body[cond_end : cond_end + 200]
        b = re.match(r"\s*\{", tail)
        if b:
            bs = cond_end + b.end() - 1
            depth, j = 0, bs
            while j < len(body):
                if body[j] == "{":
                    depth += 1
                elif body[j] == "}":
                    depth -= 1
                    if depth == 0:
                        out.append((m.start(), j + 1, kind))
                        break
                j += 1
        else:
            semi = body.find(";", cond_end)
            if semi > 0:
                out.append((m.start(), semi + 1, kind))
    return out


TYPE_BLACKLIST = {
    "if",
    "for",
    "while",
    "switch",
    "return",
    "sizeof",
    "new",
    "delete",
    "case",
    "else",
    "using",
    "typedef",
}


def declaration_types(body):
    """varname -> declared type, from simple declaration statements."""
    clean = re.sub(r"/\*.*?\*/", " ", body, flags=re.S)
    clean = re.sub(r"//[^\n]*", "", clean)
    body = clean
    types = {}
    for m in re.finditer(
        r"([A-Za-z_][\w:<>]*"
        r"(?:\s+[A-Za-z_][\w:<>]*)*(?:\s*[*&])?)\s+"
        r"([^;{}()]*?)\s*;",
        body,
    ):
        t = " ".join(m.group(1).split())
        base = t.split()[-1].strip("*&")
        if base in TYPE_BLACKLIST or "::" in t and "(" in t:
            continue
        if "(" in t or "->" in t or "." in t:
            continue
        for decl in m.group(2).split(","):
            decl = decl.strip()
            vm = re.match(r"[*&]?\s*(\w+)(\s*\[.*\])?\s*(=.*)?$", decl)
            if vm and vm.group(1) not in TYPE_BLACKLIST:
                types.setdefault(vm.group(1), t)
    return types


def extract_reads(body, pktvar):
    """Ordered read list: [(vartype, varexpr, flags)]."""
    work = re.sub(r"\bfor\s*\([^;]*;[^;]*;[^)]*\)", "for (...)", body)
    decls = declaration_types(body)
    ranges = control_ranges(body)
    reads = []

    def flags_at(pos):
        fl = []
        for s, e, k in ranges:
            if s < pos < e:
                fl.append("loop" if k in ("for", "while") else "cond")
        return sorted(set(fl))

    # operator>> chains on the packet variable
    pkt_pat = pktvar if pktvar.startswith("(?:") else re.escape(pktvar)
    for m in re.finditer(r"(?<!\w)" + pkt_pat + r"\s*((?:>>[^\n;]+)+)", work):
        chain = m.group(1)
        base = m.start()
        for tm in re.finditer(r">>\s*([^>;\n]+)", chain):
            raw = tm.group(1).strip()
            pos = base + tm.start()
            if "(" in raw and ")" in raw and re.search(r"\w+\s*\(", raw):
                reads.append((pos, "call", raw, flags_at(pos)))
                continue
            vm2 = re.match(r"^(\w+)$", raw)
            if vm2:
                # bare variable: type comes from declarations, if found
                v = vm2.group(1)
                reads.append((pos, decls.get(v, "?"), v, flags_at(pos)))
                continue
            vm = re.match(r"\(?\s*([\w:]+)\s*\)?\s*(\w+)$", raw)
            if vm and vm.group(2) not in ("true", "false"):
                v = vm.group(2)
                reads.append((pos, decls.get(v, vm.group(1)), v, flags_at(pos)))
            else:
                reads.append((pos, "?", raw, flags_at(pos)))
    # method-style reads
    for m in READ_METHOD_RE.finditer(work):
        obj, meth, tpl = m.group(1), m.group(2), m.group(3) or ""
        on_pkt = bool(re.fullmatch(pkt_pat, obj))
        if not on_pkt and meth in (
            "read",
            "read_skip",
            "readPackGUID",
            "ReadCString",
            "ReadPackedGuid",
            "ReadGuid",
            "readGUID",
        ):
            # helper-object read, not a packet read; ReadForCaster and
            # similar helpers are recorded separately below
            continue
        t = (meth + tpl.replace(" ", "")) if tpl else meth
        reads.append((m.start(), t, obj + "." + meth, flags_at(m.start())))
    call_raws = [v for _, t, v, _ in reads if t == "call"]
    for m in READFOR_RE.finditer(work):
        # skip when already captured as a >> chain call target
        if any(m.group(0) in c for c in call_raws):
            continue
        reads.append((m.start(), "targets-proxy", m.group(1) + ".ReadForCaster", flags_at(m.start())))
    reads.sort()
    return [(t, v, f) for _, t, v, f in reads], decls, ranges


def fmt_seq(reads):
    parts = []
    for t, v, fl in reads:
        s = "%s %s" % (t, v) if t != "call" else "call(%s)" % v
        if fl:
            s += "[%s]" % "+".join(fl)
        parts.append(s)
    return " | ".join(parts)


def main(out_csv):
    regs = parse_registrations(OPCODES_CPP)
    enum = parse_enum(ENUM_H)
    defs = find_handler_defs()
    body_cache, read_cache = {}, {}
    rows = []
    for r in regs:
        h = r["handler"]
        enum_ok = "n/a"
        if r["enum"] in enum:
            enum_ok = "match" if enum[r["enum"]] == r["dec"] else "MISMATCH:enum=%d" % enum[r["enum"]]
        elif r["enum"] not in ("MSG_NULL_ACTION",):
            enum_ok = "missing-from-1_12_1-enum"
        loc = defs.get(h)
        if loc is None:
            rows.append((r, "", "", "", "", "", enum_ok))
            continue
        rel, ln, dend, text, pktvar = loc
        key = (rel, ln)
        if key not in body_cache:
            b, _ = extract_body(text, dend)
            body_cache[key] = b
            try:
                read_cache[key] = extract_reads(b, pktvar)
            except re.error:
                read_cache[key] = ([], {}, [])
        body = body_cache[key]
        reads, _, _ = read_cache[key]
        nret = len(re.findall(r"\breturn\s*;", body))
        rpos = bool(re.search(r"\brpos\s*\(\s*\w+\.wpos\(\)", body))
        has_loop = any("loop" in f for _, _, f in reads) or bool(re.search(r"\b(for|while)\s*\(", body))
        has_cond = any("cond" in f for _, _, f in reads) or bool(re.search(r"\bif\s*\(", body))
        rows.append(
            (
                r,
                rel,
                ln,
                fmt_seq(reads),
                len(reads),
                "returns=%d;rpos_skip=%s" % (nret, rpos),
                enum_ok,
                has_loop,
                has_cond,
            )
        )

    with open(out_csv, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(
            [
                "opcode_hex",
                "opcode_dec",
                "enum_name",
                "reg_name",
                "status",
                "processing",
                "handler",
                "handler_file",
                "handler_line",
                "guard",
                "n_reads",
                "read_sequence",
                "has_loop",
                "has_conditional",
                "body_notes",
                "enum_check",
            ]
        )
        for row in rows:
            r = row[0]
            if len(row) == 7:
                _, rel, ln, seq, n, notes, ek = row
                w.writerow(
                    [
                        r["hex"],
                        r["dec"],
                        r["enum"],
                        r["name"],
                        r["status"],
                        r["processing"],
                        r["handler"],
                        rel,
                        ln,
                        r["guard"],
                        n,
                        seq,
                        "",
                        "",
                        notes,
                        ek,
                    ]
                )
            else:
                _, rel, ln, seq, n, notes, ek, hl, hc = row
                w.writerow(
                    [
                        r["hex"],
                        r["dec"],
                        r["enum"],
                        r["name"],
                        r["status"],
                        r["processing"],
                        r["handler"],
                        rel,
                        ln,
                        r["guard"],
                        n,
                        seq,
                        hl,
                        hc,
                        notes,
                        ek,
                    ]
                )

    # ---- reconciliation ----
    handlers = sorted({r["handler"] for r in regs})
    non_stub = [h for h in handlers if h not in STUB_HANDLERS]
    never = sum(1 for r in regs if r["status"] == "STATUS_NEVER")
    guarded = sum(1 for r in regs if r["guard"])
    missing = sorted({r["handler"] for r in regs if r["handler"] not in defs})
    mismatch = [(r["enum"], r["hex"]) for r in regs if r["enum"] in enum and enum[r["enum"]] != r["dec"]]
    print(
        "registrations=%d distinct_handlers=%d non_stub=%d "
        "status_never=%d guarded=%d" % (len(regs), len(handlers), len(non_stub), never, guarded)
    )
    print("handlers_missing_def=%d %s" % (len(missing), missing))
    print("enum_mismatches=%d %s" % (len(mismatch), mismatch))
    print("stub_rows=%d" % sum(1 for r in regs if r["handler"] in STUB_HANDLERS))
    print("wrote %s (%d rows)" % (out_csv, len(rows)))


if __name__ == "__main__":
    default_out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "22-opcode-catalog.csv")
    main(sys.argv[1] if len(sys.argv) > 1 else default_out)
