//! Production `.map` parity suite. `#[ignore]`d; gated on
//! `MANGOS_MAPS_DIR` (dev bench: `~/azeroth-dev-fixtures/datadir/maps`).
//! Goldens come from the independent Python oracle
//! `vmangos-rs/scripts/gridmap_oracle_gen.py`, which reimplements the
//! C++ sampling arithmetic with per-operation f32 rounding — digest
//! equality here means bit-identical heights/areas/liquid levels
//! across the probe lattice of every production tile.

use std::path::PathBuf;

use mangos_assets::gridmap::{GridMap, SIZE_OF_GRIDS};
use sha2::{Digest, Sha256};

mod gridmap_oracle;
use gridmap_oracle::TILES;

fn maps_dir() -> PathBuf {
    std::env::var_os("MANGOS_MAPS_DIR")
        .expect("MANGOS_MAPS_DIR must be set for production gridmap parity tests")
        .into()
}

fn hex_of(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    digest.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Mirrors `probe_points` in the oracle generator exactly.
#[allow(clippy::cast_precision_loss)]
fn probe_points(gx: u32, gy: u32) -> Vec<(f32, f32)> {
    let steps = [0.0f32, 7.25, 31.5, 63.75, 90.125, 127.0];
    let mut out = Vec::new();
    for lx in steps {
        for ly in steps {
            let cx = gx as f32 * 128.0 + lx;
            let cy = gy as f32 * 128.0 + ly;
            let x = SIZE_OF_GRIDS * (32.0 - cx / 128.0);
            let y = SIZE_OF_GRIDS * (32.0 - cy / 128.0);
            out.push((x, y));
        }
    }
    out
}

#[test]
#[ignore = "requires MANGOS_MAPS_DIR with production map fixtures"]
fn all_tiles_height_area_liquid_parity() {
    let dir = maps_dir();
    assert_eq!(TILES.len(), 2429, "golden covers every production tile");
    for golden in TILES {
        let bytes = std::fs::read(dir.join(golden.name)).expect("tile present");
        let map =
            GridMap::parse(&bytes).unwrap_or_else(|e| panic!("{}: parse failed: {e}", golden.name));
        let gx: u32 = golden.name[3..5].parse().expect("gx in name");
        let gy: u32 = golden.name[5..7].parse().expect("gy in name");

        let (mut hh, mut ha, mut hl, mut ht) =
            (Sha256::new(), Sha256::new(), Sha256::new(), Sha256::new());
        for (x, y) in probe_points(gx, gy) {
            hh.update(map.height(x, y).to_le_bytes());
            ha.update(map.area(x, y).to_le_bytes());
            hl.update(map.liquid_level(x, y).to_le_bytes());
            ht.update([map.terrain_type(x, y)]);
        }
        assert_eq!(
            hex_of(&hh.finalize()),
            golden.heights_sha256,
            "{} heights",
            golden.name
        );
        assert_eq!(
            hex_of(&ha.finalize()),
            golden.areas_sha256,
            "{} areas",
            golden.name
        );
        assert_eq!(
            hex_of(&hl.finalize()),
            golden.liquid_sha256,
            "{} liquid",
            golden.name
        );
        assert_eq!(
            hex_of(&ht.finalize()),
            golden.terrain_sha256,
            "{} terrain",
            golden.name
        );
    }
}
