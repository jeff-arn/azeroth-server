#!/usr/bin/env python3
"""Generate spline/MoveSpline/packet goldens for mangos-movement.

Independent oracle: a from-source Python reimplementation of
vmangos Movement/spline (spline.cpp evaluators + G3D Vector3/4 +
Matrix4 op order, MoveSpline.cpp initializers, util.cpp fall
formulas, packet_builder.cpp) with f32 rounding after every op.
Outputs tests/spline_oracle.json and tests/spline_oracle.rs with
f32 bit patterns; the Rust suite must match bit-for-bit.
"""

import json
import math
import struct

OUT_JSON = "vmangos-rs/crates/mangos-movement/tests/spline_oracle.json"
OUT_RS = "vmangos-rs/crates/mangos-movement/tests/spline_oracle.rs"

STEPS_PER_SEGMENT = 3
GRAVITY = 19.29110527038574  # f64 in util.cpp

CATMULL = [
    [-0.5, 1.5, -1.5, 0.5],
    [1.0, -2.5, 2.0, -0.5],
    [-0.5, 0.0, 0.5, 0.0],
    [0.0, 1.0, 0.0, 0.0],
]


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def bits(v):
    return struct.unpack("<I", struct.pack("<f", v))[0]


def v3(p):
    return (f32(p[0]), f32(p[1]), f32(p[2]))


def vadd(a, b):
    return (f32(a[0] + b[0]), f32(a[1] + b[1]), f32(a[2] + b[2]))


def vsub(a, b):
    return (f32(a[0] - b[0]), f32(a[1] - b[1]), f32(a[2] - b[2]))


def vmul(a, s):
    return (f32(a[0] * s), f32(a[1] * s), f32(a[2] * s))


def vlen(a):
    s = f32(f32(f32(a[0] * a[0]) + f32(a[1] * a[1])) + f32(a[2] * a[2]))
    return f32(math.sqrt(s))


def vlerp(a, b, alpha):
    return vadd(a, vmul(vsub(b, a), alpha))


def c_evaluate(verts, t, m, deriv=False):
    if deriv:
        tvec = (f32(f32(3.0 * t) * t), f32(2.0 * t), 1.0, 0.0)
    else:
        tvec = (f32(f32(t * t) * t), f32(t * t), t, 1.0)
    w = []
    for i in range(4):
        acc = 0.0
        for j in range(4):
            acc = f32(acc + f32(tvec[j] * f32(m[j][i])))
        w.append(acc)
    return vadd(
        vadd(vadd(vmul(verts[0], w[0]), vmul(verts[1], w[1])), vmul(verts[2], w[2])),
        vmul(verts[3], w[3]),
    )


def init_catmull_storage(controls, cyclic, cyclic_point=0):
    count = len(controls)
    real = count + (3 if cyclic else 2)
    pts = [(0.0, 0.0, 0.0)] * real
    lo = 1
    hi = lo + count - 1
    for i, c in enumerate(controls):
        pts[lo + i] = c
    if cyclic:
        if cyclic_point == 0:
            pts[0] = controls[count - 1]
        else:
            pts[0] = vlerp(controls[0], controls[1], -1.0)
        pts[hi + 1] = controls[cyclic_point]
        pts[hi + 2] = controls[cyclic_point + 1]
    else:
        pts[0] = vlerp(controls[0], controls[1], -1.0)
        pts[hi + 1] = controls[count - 1]
    return pts, lo, hi + (1 if cyclic else 0)


def seg_length(pts, idx, mode):
    if mode == "linear":
        return vlen(vsub(pts[idx], pts[idx + 1]))
    p = pts[idx - 1 : idx + 3]
    cur = p[1]
    length = 0.0  # f64 accumulator
    for i in range(1, STEPS_PER_SEGMENT + 1):
        nxt = c_evaluate(p, f32(i / STEPS_PER_SEGMENT), CATMULL)
        length += vlen(vsub(nxt, cur))
        cur = nxt
    return f32(length)


def evaluate(pts, idx, u, mode, deriv=False):
    if mode == "linear":
        if deriv:
            return vsub(pts[idx + 1], pts[idx])
        return vadd(pts[idx], vmul(vsub(pts[idx + 1], pts[idx]), u))
    return c_evaluate(pts[idx - 1 : idx + 3], u, CATMULL, deriv)


# ---- util.cpp fall formulas (f64 gravity promotions) ----
TERMINAL_VEL = f32(60.148003)
TERMINAL_SAVE_VEL = f32(7.0)
TERMINAL_LEN = f32(f32(TERMINAL_VEL * TERMINAL_VEL) / (2.0 * GRAVITY))
TERMINAL_SAVE_LEN = f32(f32(TERMINAL_SAVE_VEL * TERMINAL_SAVE_VEL) / (2.0 * GRAVITY))
TERMINAL_TIME = f32(TERMINAL_VEL / GRAVITY)


def fall_time(path_length, safe):
    if path_length < 0.0:
        return 0.0
    if safe:
        if path_length >= TERMINAL_SAVE_LEN:
            return f32(f32(f32(path_length - TERMINAL_SAVE_LEN) / TERMINAL_SAVE_VEL) + TERMINAL_SAVE_VEL / GRAVITY)
        return f32(math.sqrt(f32(f32(2.0 * path_length) / GRAVITY)))
    if path_length >= TERMINAL_LEN:
        return f32(f32(f32(path_length - TERMINAL_LEN) / TERMINAL_VEL) + TERMINAL_TIME)
    return f32(math.sqrt(f32(f32(2.0 * path_length) / GRAVITY)))


def fall_elevation(t):
    if t > TERMINAL_TIME:
        return f32(f32(TERMINAL_VEL * f32(t - TERMINAL_TIME)) + TERMINAL_LEN)
    return f32(f32(t * t) * GRAVITY * 0.5)


def fall_elevation_v(t, safe, start_velocity):
    term_vel = TERMINAL_SAVE_VEL if safe else TERMINAL_VEL
    if start_velocity > term_vel:
        start_velocity = term_vel
    terminal_time = f32(TERMINAL_TIME - start_velocity / GRAVITY)
    if t > terminal_time:
        return f32(
            f32(f32(TERMINAL_VEL * f32(t - terminal_time)) + f32(start_velocity * terminal_time))
            + GRAVITY * terminal_time * terminal_time * 0.5
        )
    return f32(t * (start_velocity + t * GRAVITY * 0.5))


def timestamps(pts, lo, hi, mode, velocity, falling):
    lens = [0] * (hi + 1)
    if falling:
        start_elevation = pts[lo][2]
        for i in range(lo, hi):
            val = int(f32(fall_time(f32(start_elevation - pts[i + 1][2]), False) * 1000.0))
            lens[i + 1] = val if val >= 0 else 0x7FFFFFFF
    else:
        vinv = f32(1000.0 / velocity)
        t = 1  # minimal_duration
        for i in range(lo, hi):
            t = int(f32(f32(t) + f32(seg_length(pts, i, mode) * vinv)))
            lens[i + 1] = t if t >= 0 else 0x7FFFFFFF
    return lens


def lround_pack(v):
    p = f32(v * 4.0)
    if p >= 0:
        return math.floor(p + 0.5)
    return math.ceil(p - 0.5)


def pack_xyz(x, y, z):
    return (lround_pack(x) & 0x7FF) | ((lround_pack(y) & 0x7FF) << 11) | ((lround_pack(z) & 0x3FF) << 22)


def packet(pts, lo, hi, cyclist, flags, spline_id, lens, max_points=80):
    """WriteMonsterMove(firstPoint=0) body (no opcode/guid)."""
    out = bytearray()
    wire_flags = flags
    wire_flags = (wire_flags | 0x200000) if cyclist else (wire_flags & ~0x200000)
    wire_flags &= 0xFFFFFFFF
    out += struct.pack("<fff", *pts[lo])
    out += struct.pack("<I", spline_id)
    out += b"\x00"  # MonsterMoveNormal
    out += struct.pack("<I", (wire_flags & ~0x70001 | 0x100) & 0xFFFFFFFF)
    dur_pos = len(out)
    out += struct.pack("<i", lens[hi])
    if flags & 0x200:  # Mask_CatmullRom
        count = len(pts) - 3
        if flags & 0x100000:  # cyclic
            out += struct.pack("<I", count + 1)
            out += struct.pack("<fff", *pts[1])
            for i in range(count):
                out += struct.pack("<fff", *pts[1 + i])
        else:
            out += struct.pack("<I", count)
            for i in range(count):
                out += struct.pack("<fff", *pts[2 + i])
    else:
        start = 0
        last_idx = len(pts) - 3
        written = -1
        if last_idx - start > max_points:
            last_idx = start + max_points
            written = last_idx
        destination = pts[1 + last_idx]
        out += struct.pack("<I", last_idx - start + 1)
        out += struct.pack("<fff", *destination)
        if last_idx > 1:
            for i in range(start, last_idx):
                off = vsub(destination, pts[1 + i])
                if abs(off[0]) < 0.25 and abs(off[1]) < 0.25 and abs(off[2]) < 0.25:
                    z = f32(off[2] + 0.51) if off[2] < 0 else f32(off[2] + 0.26)
                    off = (off[0], off[1], z)
                out += struct.pack("<I", pack_xyz(*off))
        last_node = written if written >= 0 else hi
        out[dur_pos : dur_pos + 4] = struct.pack("<I", (lens[last_node] - lens[0]) & 0xFFFFFFFF)
    return bytes(out)


US = [0.0, 0.25, 0.5, 0.75, 1.0]

CASES = [
    {
        "name": "curved_catmull",
        "controls": [
            (0.0, 0.0, 10.0),
            (37.5, -12.25, 14.0),
            (80.3, 44.7, 9.5),
            (120.0, -60.125, 22.75),
            (200.6, 0.0, 5.25),
        ],
        "velocity": 7.0,
        "mode": "catmull",
        "cyclic": False,
        "flags": 0x200,
        "spline_id": 11,
    },
    {
        "name": "curved_linear",
        "controls": [
            (0.0, 0.0, 10.0),
            (37.5, -12.25, 14.0),
            (80.3, 44.7, 9.5),
            (120.0, -60.125, 22.75),
            (200.6, 0.0, 5.25),
        ],
        "velocity": 7.0,
        "mode": "linear",
        "cyclic": False,
        "flags": 0,
        "spline_id": 12,
    },
    {
        "name": "cyclic_catmull",
        "controls": [(0.0, 0.0, 0.0), (50.0, 0.0, 3.0), (50.0, 50.0, 0.0), (0.0, 50.0, -3.0)],
        "velocity": 5.5,
        "mode": "catmull",
        "cyclic": True,
        "flags": 0x200 | 0x100000,
        "spline_id": 13,
    },
    {
        "name": "falling_linear",
        "controls": [(0.0, 0.0, 100.0), (10.0, 0.0, 60.0), (25.0, 5.0, 20.0), (40.0, 5.0, 0.0)],
        "velocity": 9.0,
        "mode": "linear",
        "cyclic": False,
        "flags": 0x2,
        "spline_id": 14,
    },
]


def run_case(case):
    controls = [v3(c) for c in case["controls"]]
    pts, lo, hi = init_catmull_storage(controls, case["cyclic"])
    falling = bool(case["flags"] & 0x2)
    lens = timestamps(pts, lo, hi, case["mode"], f32(case["velocity"]), falling)
    segs = []
    for idx in range(lo, hi):
        segs.append(
            {
                "index": idx,
                "seg_length_bits": bits(seg_length(pts, idx, case["mode"])),
                "eval_bits": [tuple(bits(c) for c in evaluate(pts, idx, f32(u), case["mode"])) for u in US],
                "deriv_bits": [tuple(bits(c) for c in evaluate(pts, idx, f32(u), case["mode"], True)) for u in US],
            }
        )
    pkt = packet(pts, lo, hi, case["cyclic"], case["flags"], case["spline_id"], lens)
    return {
        "name": case["name"],
        "controls_bits": [tuple(bits(c) for c in p) for p in controls],
        "velocity_bits": bits(f32(case["velocity"])),
        "mode": case["mode"],
        "cyclic": case["cyclic"],
        "flags": case["flags"],
        "spline_id": case["spline_id"],
        "first": lo,
        "last": hi,
        "storage_bits": [tuple(bits(c) for c in p) for p in pts],
        "lengths": lens,
        "segments": segs,
        "packet_hex": pkt.hex(),
    }


def fall_probes():
    probes = []
    for p in [0.0, 0.5, 3.0, 50.0, 93.7, 93.8, 200.0, 1000.0]:
        for safe in (False, True):
            probes.append(
                {
                    "kind": "time",
                    "a_bits": bits(f32(p)),
                    "safe": safe,
                    "b_bits": 0,
                    "result_bits": bits(fall_time(f32(p), safe)),
                }
            )
    for t in [0.1, 1.0, 3.0, 3.2, 10.0]:
        probes.append(
            {
                "kind": "elev",
                "a_bits": bits(f32(t)),
                "safe": False,
                "b_bits": 0,
                "result_bits": bits(fall_elevation(f32(t))),
            }
        )
    for t, safe, v0 in [
        (0.5, False, 0.0),
        (5.0, False, 10.0),
        (5.0, True, 10.0),
        (2.0, True, 0.0),
        (1.0, False, 100.0),
        (4.0, True, 3.0),
    ]:
        probes.append(
            {
                "kind": "elev_v",
                "a_bits": bits(f32(t)),
                "safe": safe,
                "b_bits": bits(f32(v0)),
                "result_bits": bits(fall_elevation_v(f32(t), safe, f32(v0))),
            }
        )
    return probes


def hx(v):
    return f"0x{v >> 16:04X}_{v & 0xFFFF:04X}"


def tupl(t):
    return "(" + ", ".join(hx(v) for v in t) + ")"


def emit_rs(data):
    lines = [
        "// Generated by vmangos-rs/scripts/spline_oracle_gen.py — an independent",
        "// Python reimplementation of vmangos Movement/spline. Do not edit.",
        "",
        "pub struct SplineCase {",
        "    pub name: &'static str,",
        "    pub controls_bits: &'static [(u32, u32, u32)],",
        "    pub velocity_bits: u32,",
        "    pub catmull: bool,",
        "    pub cyclic: bool,",
        "    pub flags: u32,",
        "    pub spline_id: u32,",
        "    pub first: i32,",
        "    pub last: i32,",
        "    pub storage_bits: &'static [(u32, u32, u32)],",
        "    pub lengths: &'static [i32],",
        "    pub seg_lengths_bits: &'static [u32],",
        "    pub eval_bits: &'static [(u32, u32, u32)],",
        "    pub deriv_bits: &'static [(u32, u32, u32)],",
        "    pub packet_hex: &'static str,",
        "}",
        "",
        "pub const CASES: &[SplineCase] = &[",
    ]
    for c in data["cases"]:
        eval_flat = [t for s in c["segments"] for t in s["eval_bits"]]
        deriv_flat = [t for s in c["segments"] for t in s["deriv_bits"]]
        lines.append("    SplineCase {")
        lines.append(f'        name: "{c["name"]}",')
        lines.append("        controls_bits: &[" + ", ".join(tupl(t) for t in c["controls_bits"]) + "],")
        lines.append(f"        velocity_bits: {hx(c['velocity_bits'])},")
        lines.append(f"        catmull: {str(c['mode'] == 'catmull').lower()},")
        lines.append(f"        cyclic: {str(c['cyclic']).lower()},")
        lines.append(f"        flags: {hx(c['flags'])},")
        lines.append(f"        spline_id: {c['spline_id']},")
        lines.append(f"        first: {c['first']},")
        lines.append(f"        last: {c['last']},")
        lines.append("        storage_bits: &[" + ", ".join(tupl(t) for t in c["storage_bits"]) + "],")
        lines.append("        lengths: &[" + ", ".join(str(v) for v in c["lengths"]) + "],")
        lines.append("        seg_lengths_bits: &[" + ", ".join(hx(s["seg_length_bits"]) for s in c["segments"]) + "],")
        lines.append("        eval_bits: &[")
        lines.extend("            " + tupl(t) + "," for t in eval_flat)
        lines.append("        ],")
        lines.append("        deriv_bits: &[")
        lines.extend("            " + tupl(t) + "," for t in deriv_flat)
        lines.append("        ],")
        lines.append(f'        packet_hex: "{c["packet_hex"]}",')
        lines.append("    },")
    lines.append("];")
    lines.append("")
    lines.append("// (kind, a_bits, safe, b_bits, result_bits); kind: 0=time 1=elev 2=elev_v")
    lines.append("pub const FALL_PROBES: &[(u8, u32, bool, u32, u32)] = &[")
    kind = {"time": 0, "elev": 1, "elev_v": 2}
    for p in data["fall_probes"]:
        lines.append(
            f"    ({kind[p['kind']]}, {hx(p['a_bits'])}, {str(p['safe']).lower()}, "
            f"{hx(p['b_bits'])}, {hx(p['result_bits'])}),"
        )
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


def main():
    data = {"cases": [run_case(c) for c in CASES], "fall_probes": fall_probes()}
    with open(OUT_JSON, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=1)
    with open(OUT_RS, "w", encoding="utf-8") as fh:
        fh.write(emit_rs(data))
    print(f"cases={len(data['cases'])} probes={len(data['fall_probes'])}")


if __name__ == "__main__":
    main()
