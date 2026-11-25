#!/usr/bin/env python3
"""VMap parity goldens from production vmap fixtures.

Independent oracle with three layers:
1. Parse digests for every .vmo/.vmtree/.vmtile (proves byte->value
   parser parity, including the serialized BIH trees).
2. Brute-force ray probes (exhaustive triangle scan, Moller test
   with per-operation f32 rounding) for models with <= 2000
   triangles total, capped at 120 sampled models — the Rust BIH
   traversal must find bit-identical nearest hits.
3. (Rust-side) BIH vs exhaustive scan on the large models.

Usage: vmap_oracle_gen.py <vmaps_dir> <out.json> [out.rs]
"""

import hashlib
import json
import os
import struct
import sys

EPS = 1e-5


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def f32bits(v):
    return struct.pack("<f", v)


class Reader:
    def __init__(self, data):
        self.d = data
        self.p = 0

    def take(self, n):
        out = self.d[self.p : self.p + n]
        assert len(out) == n, "truncated"
        self.p += n
        return out

    def u8(self):
        return self.take(1)[0]

    def u16(self):
        return struct.unpack("<H", self.take(2))[0]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def f32(self):
        return struct.unpack("<f", self.take(4))[0]

    def vec3(self):
        return struct.unpack("<3f", self.take(12))

    def chunk(self, want):
        got = self.take(4)
        assert got == want, (got, want)

    def magic(self):
        assert self.take(8) == b"VMAP_7.0"


def parse_bih(r):
    lo, hi = r.vec3(), r.vec3()
    tree = [r.u32() for _ in range(r.u32())]
    objects = [r.u32() for _ in range(r.u32())]
    return {"lo": lo, "hi": hi, "tree": tree, "objects": objects}


def bih_digest(bih):
    h = hashlib.sha256()
    for v in (*bih["lo"], *bih["hi"]):
        h.update(f32bits(v))
    h.update(struct.pack("<I", len(bih["tree"])))
    for t in bih["tree"]:
        h.update(struct.pack("<I", t))
    h.update(struct.pack("<I", len(bih["objects"])))
    for o in bih["objects"]:
        h.update(struct.pack("<I", o))
    return h.hexdigest()


def parse_liquid(r):
    tx, ty = r.u32(), r.u32()
    r.vec3()
    ltype = r.u32()
    r.take(4 * (tx + 1) * (ty + 1))
    r.take(tx * ty)
    return ltype


def parse_group(r):
    lo, hi = r.vec3(), r.vec3()
    flags, wmo_id = r.u32(), r.u32()
    r.chunk(b"VERT")
    r.u32()
    count = r.u32()
    group = {"bound": (lo, hi), "flags": flags, "wmo_id": wmo_id, "verts": [], "tris": [], "liquid": None}
    if count == 0:
        return group
    verts_raw = r.take(12 * count)
    group["verts_raw"] = verts_raw
    group["verts"] = [struct.unpack_from("<3f", verts_raw, 12 * i) for i in range(count)]
    r.chunk(b"TRIM")
    r.u32()
    tri_count = r.u32()
    tris_raw = r.take(12 * tri_count)
    group["tris_raw"] = tris_raw
    group["tris"] = [struct.unpack_from("<3I", tris_raw, 12 * i) for i in range(tri_count)]
    r.chunk(b"MBIH")
    group["bih"] = parse_bih(r)
    r.chunk(b"LIQU")
    chunk_size = r.u32()
    if chunk_size > 0:
        group["liquid"] = parse_liquid(r)
    return group


def parse_vmo(data):
    r = Reader(data)
    r.magic()
    r.chunk(b"WMOD")
    r.u32()
    root = r.u32()
    groups = []
    group_bih = None
    if r.p < len(data):
        r.chunk(b"GMOD")
        count = r.u32()
        groups = [parse_group(r) for _ in range(count)]
        r.chunk(b"GBIH")
        group_bih = parse_bih(r)
    return {"root": root, "groups": groups, "group_bih": group_bih}


def vmo_digest(model):
    h = hashlib.sha256()
    h.update(struct.pack("<I", model["root"]))
    h.update(struct.pack("<I", len(model["groups"])))
    total_tris = 0
    for g in model["groups"]:
        total_tris += len(g["tris"])
        for v in (*g["bound"][0], *g["bound"][1]):
            h.update(f32bits(v))
        h.update(struct.pack("<II", g["flags"], g["wmo_id"]))
        h.update(struct.pack("<II", len(g["verts"]), len(g["tris"])))
        if g["verts"]:
            h.update(hashlib.sha256(g["verts_raw"]).digest())
            h.update(hashlib.sha256(g["tris_raw"]).digest())
            h.update(bytes.fromhex(bih_digest(g["bih"])))
        h.update(struct.pack("<I", g["liquid"] or 0))
    if model["group_bih"]:
        h.update(bytes.fromhex(bih_digest(model["group_bih"])))
    return h.hexdigest(), total_tris


def sub(a, b):
    return (f32(a[0] - b[0]), f32(a[1] - b[1]), f32(a[2] - b[2]))


def cross(a, b):
    return (
        f32(f32(a[1] * b[2]) - f32(a[2] * b[1])),
        f32(f32(a[2] * b[0]) - f32(a[0] * b[2])),
        f32(f32(a[0] * b[1]) - f32(a[1] * b[0])),
    )


def dot(a, b):
    return f32(f32(f32(a[0] * b[0]) + f32(a[1] * b[1])) + f32(a[2] * b[2]))


def intersect_triangle(p0, p1, p2, origin, direction, dist):
    e1, e2 = sub(p1, p0), sub(p2, p0)
    p = cross(direction, e2)
    a = dot(e1, p)
    if abs(a) < EPS:
        return dist, False
    f = f32(1.0 / a)
    s = sub(origin, p0)
    u = f32(f * dot(s, p))
    if u < 0.0 or u > 1.0:
        return dist, False
    q = cross(s, e1)
    v = f32(f * dot(direction, q))
    if v < 0.0 or f32(u + v) > 1.0:
        return dist, False
    t = f32(f * dot(e2, q))
    if t > 0.0 and t < dist:
        return t, True
    return dist, False


def probe_rays(bounds_list):
    lo = tuple(min(b[0][i] for b in bounds_list) for i in range(3))
    hi = tuple(max(b[1][i] for b in bounds_list) for i in range(3))
    fracs = [0.13, 0.37, 0.61, 0.89]

    def mix(i, fr):
        # fr rounded to f32 first: the Rust side uses f32 literals.
        return f32(lo[i] + f32(f32(hi[i] - lo[i]) * f32(fr)))

    rays = []
    for fx in fracs:
        for fy in fracs:
            rays.append(((mix(0, fx), mix(1, fy), f32(hi[2] + 2.0)), (0.0, 0.0, -1.0)))
    for fz in fracs:
        z = mix(2, fz)
        rays.append(((f32(lo[0] - 2.0), mix(1, 0.5), z), (1.0, 0.0, 0.0)))
        rays.append(((mix(0, 0.5), f32(lo[1] - 2.0), z), (0.0, 1.0, 0.0)))
    return rays


def brute_force_digest(model):
    bounds = [g["bound"] for g in model["groups"] if g["tris"]]
    if not bounds:
        return None
    h = hashlib.sha256()
    for origin, direction in probe_rays(bounds):
        dist = float("inf")
        hit = False
        for g in model["groups"]:
            verts = g["verts"]
            for i0, i1, i2 in g["tris"]:
                dist, this = intersect_triangle(verts[i0], verts[i1], verts[i2], origin, direction, dist)
                hit = hit or this
        h.update(b"\x01" if hit else b"\x00")
        h.update(f32bits(dist))
    return h.hexdigest()


def parse_spawn(r):
    if r.p >= len(r.d):
        return None
    h = hashlib.sha256()
    flags = r.u32()
    h.update(struct.pack("<I", flags))
    h.update(r.take(2 + 4 + 12 + 12 + 4))
    if flags & 4:
        h.update(r.take(24))
    name_len = r.u32()
    h.update(struct.pack("<I", name_len))
    h.update(r.take(name_len))
    return h


def main():
    vdir, out_json = sys.argv[1], sys.argv[2]
    result = {"vmo": {}, "probes": {}, "spawns": {}, "trees": {}}
    tri_counts = {}
    for fname in sorted(os.listdir(vdir)):
        path = os.path.join(vdir, fname)
        if fname.endswith(".vmo"):
            model = parse_vmo(open(path, "rb").read())
            digest, total_tris = vmo_digest(model)
            result["vmo"][fname] = {"sha256": digest, "triangles": total_tris}
            tri_counts[fname] = (total_tris, model)
        elif fname.endswith(".vmtile"):
            r = Reader(open(path, "rb").read())
            r.magic()
            n = r.u32()
            h = hashlib.sha256()
            for _ in range(n):
                sh = parse_spawn(r)
                h.update(sh.digest())
                h.update(struct.pack("<I", r.u32()))
            result["spawns"][fname] = {"count": n, "sha256": h.hexdigest()}
        elif fname.endswith(".vmtree"):
            r = Reader(open(path, "rb").read())
            r.magic()
            tiled = r.u8()
            r.chunk(b"NODE")
            tree = parse_bih(r)
            entry = {"tiled": bool(tiled), "bih_sha256": bih_digest(tree), "spawns": None}
            r.chunk(b"GOBJ")
            if not tiled:
                h = hashlib.sha256()
                count = 0
                while True:
                    sh = parse_spawn(r)
                    if sh is None:
                        break
                    count += 1
                    h.update(sh.digest())
                    h.update(struct.pack("<I", r.u32()))
                entry["spawns"] = {"count": count, "sha256": h.hexdigest()}
            result["trees"][fname] = entry

    # Stratified probe sample: sort by triangle count, take up to 120
    # models with <= 2000 triangles spread across the range.
    small = sorted((t, name) for name, (t, _m) in tri_counts.items() if 0 < t <= 2000)
    if len(small) > 120:
        step = len(small) / 120.0
        small = [small[int(i * step)] for i in range(120)]
    for _t, name in small:
        digest = brute_force_digest(tri_counts[name][1])
        if digest:
            result["probes"][name] = digest

    with open(out_json, "w") as f:
        json.dump(result, f, indent=1, sort_keys=True)
    print(
        f"vmo: {len(result['vmo'])}, tiles: {len(result['spawns'])}, "
        f"trees: {len(result['trees'])}, probe models: {len(result['probes'])}"
    )

    if len(sys.argv) > 3:
        lines = [
            "// Generated by vmangos-rs/scripts/vmap_oracle_gen.py. Do not edit.",
            "",
            "pub struct VmoGolden {",
            "    pub name: &'static str,",
            "    pub sha256: &'static str,",
            "    pub triangles: u32,",
            "}",
            "",
            "pub const VMOS: &[VmoGolden] = &[",
        ]
        for name in sorted(result["vmo"]):
            v = result["vmo"][name]
            tris = f"{v['triangles']:,}".replace(",", "_")
            lines.append(f'    VmoGolden {{ name: "{name}", sha256: "{v["sha256"]}", triangles: {tris} }},')
        lines += [
            "];",
            "",
            "pub struct SpawnGolden {",
            "    pub name: &'static str,",
            "    pub count: u32,",
            "    pub sha256: &'static str,",
            "}",
            "",
            "pub const TILES: &[SpawnGolden] = &[",
        ]
        for name in sorted(result["spawns"]):
            v = result["spawns"][name]
            lines.append(f'    SpawnGolden {{ name: "{name}", count: {v["count"]}, sha256: "{v["sha256"]}" }},')
        lines += [
            "];",
            "",
            "pub struct TreeGolden {",
            "    pub name: &'static str,",
            "    pub tiled: bool,",
            "    pub bih_sha256: &'static str,",
            "    pub spawn_count: Option<u32>,",
            "    pub spawns_sha256: &'static str,",
            "}",
            "",
            "pub const TREES: &[TreeGolden] = &[",
        ]
        for name in sorted(result["trees"]):
            v = result["trees"][name]
            sc = f"Some({v['spawns']['count']})" if v["spawns"] else "None"
            ss = v["spawns"]["sha256"] if v["spawns"] else ""
            lines.append(
                f'    TreeGolden {{ name: "{name}", tiled: {str(v["tiled"]).lower()}, '
                f'bih_sha256: "{v["bih_sha256"]}", spawn_count: {sc}, spawns_sha256: "{ss}" }},'
            )
        lines += [
            "];",
            "",
            "pub struct ProbeGolden {",
            "    pub name: &'static str,",
            "    pub probe_sha256: &'static str,",
            "}",
            "",
            "pub const PROBES: &[ProbeGolden] = &[",
        ]
        for name in sorted(result["probes"]):
            lines.append(f'    ProbeGolden {{ name: "{name}", probe_sha256: "{result["probes"][name]}" }},')
        lines.append("];")
        with open(sys.argv[3], "w") as f:
            f.write("\n".join(lines) + "\n")
        print("rust golden written")


if __name__ == "__main__":
    main()
