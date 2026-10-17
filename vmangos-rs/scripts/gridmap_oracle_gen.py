#!/usr/bin/env python3
"""GridMap parity goldens from production .map fixtures.

Independent oracle: reimplements vmangos GridMap height/area/liquid
sampling (float + flat planes, the only encodings present in the
production corpus) with per-operation f32 rounding, then digests a
fixed probe lattice per tile. The Rust suite recomputes the same
digests through mangos-assets APIs and compares bit-for-bit.

Usage: gridmap_oracle_gen.py <maps_dir> <out.json> [out.rs]
"""

import hashlib
import json
import os
import struct
import sys

SIZE_OF_GRIDS = struct.unpack("<f", struct.pack("<f", 533.33333))[0]
INVALID_HEIGHT_VALUE = -200000.0
HOLETAB_H = [0x1111, 0x2222, 0x4444, 0x8888]
HOLETAB_V = [0x000F, 0x00F0, 0x0F00, 0xF000]


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def f32bits(v):
    return struct.pack("<f", v)


class GridMap:
    def __init__(self, data):
        assert data[0:4] == b"MAPS" and data[4:8] == b"z1.4"
        vals = struct.unpack_from("<8I", data, 8)
        self.area_off, _a_sz, self.height_off, _h_sz, self.liquid_off, _l_sz, self.holes_off, _ho_sz = vals
        self.data = data
        self.holes = [0] * 256
        self.grid_area = 0
        self.area_map = None
        self.grid_height = INVALID_HEIGHT_VALUE
        self.v9 = self.v8 = None
        self.flat = True
        self.liquid_map = None
        self.liquid_level = 0.0
        self.liquid_global_flags = 0
        self.liquid_flags = None
        self.liquid_off_x = self.liquid_off_y = 0
        self.liquid_w = self.liquid_h = 0
        if self.area_off:
            self._load_area()
        if self.holes_off:
            self.holes = list(struct.unpack_from("<256H", data, self.holes_off))
        if self.height_off:
            self._load_heights()
        if self.liquid_off:
            self._load_liquid()

    def _load_area(self):
        fourcc, flags, grid_area = struct.unpack_from("<4sHH", self.data, self.area_off)
        assert fourcc == b"AREA"
        self.grid_area = grid_area
        if not flags & 1:
            self.area_map = list(struct.unpack_from("<256H", self.data, self.area_off + 8))

    def _load_heights(self):
        fourcc, flags, gh, _gmax = struct.unpack_from("<4sIff", self.data, self.height_off)
        assert fourcc == b"MHGT"
        self.grid_height = gh
        if not flags & 1:
            assert not flags & 2 and not flags & 4, "int-encoded heights not in corpus"
            off = self.height_off + 16
            n9 = 129 * 129
            self.v9 = struct.unpack_from(f"<{n9}f", self.data, off)
            self.v8 = struct.unpack_from(f"<{128 * 128}f", self.data, off + 4 * n9)
            self.flat = False

    def _load_liquid(self):
        off = self.liquid_off
        fourcc = self.data[off : off + 4]
        assert fourcc == b"MLIQ"
        flags, self.liquid_global_flags = self.data[off + 4], self.data[off + 5]
        _entry, self.liquid_off_x, self.liquid_off_y, self.liquid_w, self.liquid_h = struct.unpack_from(
            "<HBBBB", self.data, off + 6
        )
        (self.liquid_level,) = struct.unpack_from("<f", self.data, off + 12)
        pos = off + 16
        if not flags & 1:
            pos += 256 * 2  # liquid entries (unused by probes)
            self.liquid_flags = list(self.data[pos : pos + 256])
            pos += 256
        if not flags & 2:
            n = self.liquid_w * self.liquid_h
            self.liquid_map = struct.unpack_from(f"<{n}f", self.data, pos)

    def is_hole(self, row, col):
        cell_row, cell_col = row // 8, col // 8
        hole_row, hole_col = (row % 8) // 2, (col - cell_col * 8) // 2
        return (self.holes[cell_row * 16 + cell_col] & HOLETAB_H[hole_col] & HOLETAB_V[hole_row]) != 0

    def height(self, x, y):
        if self.flat:
            return self.grid_height
        x = f32(128.0 * f32(32.0 - f32(x / SIZE_OF_GRIDS)))
        y = f32(128.0 * f32(32.0 - f32(y / SIZE_OF_GRIDS)))
        xi_full, yi_full = int(x), int(y)
        x = f32(x - xi_full)
        y = f32(y - yi_full)
        xi, yi = xi_full & 127, yi_full & 127
        if self.is_hole(xi, yi):
            return INVALID_HEIGHT_VALUE
        v9, v8 = self.v9, self.v8
        if f32(x + y) < 1.0:
            if x > y:
                h1, h2 = v9[xi * 129 + yi], v9[(xi + 1) * 129 + yi]
                h5 = f32(2.0 * v8[xi * 128 + yi])
                a, b, c = f32(h2 - h1), f32(f32(h5 - h1) - h2), h1
            else:
                h1, h3 = v9[xi * 129 + yi], v9[xi * 129 + yi + 1]
                h5 = f32(2.0 * v8[xi * 128 + yi])
                a, b, c = f32(f32(h5 - h1) - h3), f32(h3 - h1), h1
        elif x > y:
            h2, h4 = v9[(xi + 1) * 129 + yi], v9[(xi + 1) * 129 + yi + 1]
            h5 = f32(2.0 * v8[xi * 128 + yi])
            a, b, c = f32(f32(h2 + h4) - h5), f32(h4 - h2), f32(h5 - h4)
        else:
            h3, h4 = v9[xi * 129 + yi + 1], v9[(xi + 1) * 129 + yi + 1]
            h5 = f32(2.0 * v8[xi * 128 + yi])
            a, b, c = f32(h4 - h3), f32(f32(h3 + h4) - h5), f32(h5 - h4)
        return f32(f32(f32(a * x) + f32(b * y)) + c)

    def area(self, x, y):
        if self.area_map is None:
            return self.grid_area
        lx = int(f32(16.0 * f32(32.0 - f32(x / SIZE_OF_GRIDS)))) & 15
        ly = int(f32(16.0 * f32(32.0 - f32(y / SIZE_OF_GRIDS)))) & 15
        return self.area_map[lx * 16 + ly]

    def liquid_level_at(self, x, y):
        if self.liquid_map is None:
            return self.liquid_level
        gx = f32(128.0 * f32(32.0 - f32(x / SIZE_OF_GRIDS)))
        gy = f32(128.0 * f32(32.0 - f32(y / SIZE_OF_GRIDS)))
        cx = (int(gx) & 127) - self.liquid_off_y
        cy = (int(gy) & 127) - self.liquid_off_x
        if cx < 0 or cx >= self.liquid_h or cy < 0 or cy >= self.liquid_w:
            return INVALID_HEIGHT_VALUE
        return self.liquid_map[cx * self.liquid_w + cy]

    def terrain_type(self, x, y):
        if self.liquid_flags is None:
            return self.liquid_global_flags
        lx = int(f32(16.0 * f32(32.0 - f32(x / SIZE_OF_GRIDS)))) & 15
        ly = int(f32(16.0 * f32(32.0 - f32(y / SIZE_OF_GRIDS)))) & 15
        return self.liquid_flags[lx * 16 + ly]


def probe_points(gx, gy):
    """World coordinates landing on this tile's probe lattice."""
    steps = [0.0, 7.25, 31.5, 63.75, 90.125, 127.0]
    for lx in steps:
        for ly in steps:
            cx = gx * 128 + lx
            cy = gy * 128 + ly
            x = f32(SIZE_OF_GRIDS * f32(32.0 - f32(cx / 128.0)))
            y = f32(SIZE_OF_GRIDS * f32(32.0 - f32(cy / 128.0)))
            yield x, y


def main():
    maps_dir, out_json = sys.argv[1], sys.argv[2]
    tiles = {}
    for fname in sorted(os.listdir(maps_dir)):
        if not fname.endswith(".map"):
            continue
        data = open(os.path.join(maps_dir, fname), "rb").read()
        gm = GridMap(data)
        gx, gy = int(fname[3:5]), int(fname[5:7])
        hh, ha, hl, ht = hashlib.sha256(), hashlib.sha256(), hashlib.sha256(), hashlib.sha256()
        for x, y in probe_points(gx, gy):
            hh.update(f32bits(gm.height(x, y)))
            ha.update(struct.pack("<H", gm.area(x, y)))
            hl.update(f32bits(gm.liquid_level_at(x, y)))
            ht.update(bytes([gm.terrain_type(x, y)]))
        tiles[fname] = {
            "heights_sha256": hh.hexdigest(),
            "areas_sha256": ha.hexdigest(),
            "liquid_sha256": hl.hexdigest(),
            "terrain_sha256": ht.hexdigest(),
        }
    with open(out_json, "w") as f:
        json.dump({"tiles": tiles}, f, indent=1, sort_keys=True)
    print(f"tiles: {len(tiles)} golden written")

    if len(sys.argv) > 3:
        lines = [
            "// Generated by vmangos-rs/scripts/gridmap_oracle_gen.py.",
            "// Do not edit.",
            "",
            "pub struct TileGolden {",
            "    pub name: &'static str,",
            "    pub heights_sha256: &'static str,",
            "    pub areas_sha256: &'static str,",
            "    pub liquid_sha256: &'static str,",
            "    pub terrain_sha256: &'static str,",
            "}",
            "",
            "pub const TILES: &[TileGolden] = &[",
        ]
        for name in sorted(tiles):
            t = tiles[name]
            lines.append(
                f'    TileGolden {{ name: "{name}", heights_sha256: "{t["heights_sha256"]}", '
                f'areas_sha256: "{t["areas_sha256"]}", liquid_sha256: "{t["liquid_sha256"]}", '
                f'terrain_sha256: "{t["terrain_sha256"]}" }},'
            )
        lines.append("];")
        with open(sys.argv[3], "w") as f:
            f.write("\n".join(lines) + "\n")
        print("rust golden written")


if __name__ == "__main__":
    main()
