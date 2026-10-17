//! Production-mmap parity: replay the oracle goldens
//! (`vmangos-rs/scripts/detour_oracle_gen.py` → independent C++
//! harness `scripts/detour_oracle.cpp`) through the Rust FFI and
//! compare statuses, poly refs, and point bit patterns exactly.
//!
//! Ignored by default; requires `MANGOS_MMAPS_DIR` pointing at a
//! data directory `mmaps/` (see vmangos-rs/.env.test).

use std::path::PathBuf;

use mangos_detour::{
    MAX_PATH_LENGTH, NavMesh, NavMeshQuery, PolyRef, Status, mover_filter_include, nav_terrain,
    target_allowed_flags,
};
use mangos_movement::Vec3;

#[path = "detour_oracle.rs"]
mod oracle;

use oracle::{NEAR, PARAMS_FILE, PATHS, TILE_FILES, TILES_OK};

fn vec3(bits: (u32, u32, u32)) -> Vec3 {
    Vec3::new(
        f32::from_bits(bits.0),
        f32::from_bits(bits.1),
        f32::from_bits(bits.2),
    )
}

fn bits(v: Vec3) -> (u32, u32, u32) {
    (v.x.to_bits(), v.y.to_bits(), v.z.to_bits())
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(std::env::var("MANGOS_MMAPS_DIR").expect("MANGOS_MMAPS_DIR not set"))
}

#[test]
#[ignore = "needs MANGOS_MMAPS_DIR production fixtures"]
fn production_tiles_load_and_queries_match_oracle() {
    let dir = fixture_dir();
    let params = std::fs::read(dir.join(PARAMS_FILE)).expect("params file");
    let mut mesh = NavMesh::from_mmap_params(&params).expect("navmesh init");

    let mut tiles_ok = 0;
    for file in TILE_FILES {
        let bytes = std::fs::read(dir.join(file)).expect("tile file");
        mesh.load_tile_file(&bytes)
            .unwrap_or_else(|e| panic!("tile {file}: {e:?}"));
        tiles_ok += 1;
    }
    assert_eq!(tiles_ok, TILES_OK);

    let query = NavMeshQuery::new(&mesh, mangos_detour::DEFAULT_MAX_NODES).expect("query init");

    // Creature-walk filter (createFilter with CanWalk+CanSwim).
    let walk = mover_filter_include(true, true, false);
    assert_eq!(
        walk,
        nav_terrain::GROUND | nav_terrain::WATER | nav_terrain::MAGMA | nav_terrain::SLIME
    );
    let end_include = walk | target_allowed_flags(true, true, false);

    for (i, (q, expected_ref, expected_closest, expected_dist)) in NEAR.iter().enumerate() {
        let got = query.find_walk_poly(vec3(*q), walk, 0, 10.0);
        match (*expected_ref, got) {
            (0, None) => {}
            (expected, Some((poly, closest, dist))) => {
                assert_eq!(poly.0, expected, "near[{i}] ref");
                assert_eq!(bits(closest), *expected_closest, "near[{i}] closest");
                assert_eq!(dist.to_bits(), *expected_dist, "near[{i}] dist");
            }
            (expected, got) => panic!("near[{i}]: expected ref {expected}, got {got:?}"),
        }
    }

    for (i, p) in PATHS.iter().enumerate() {
        let start = vec3(p.start);
        let end = vec3(p.end);
        let sref = query
            .find_walk_poly(start, walk, 0, 10.0)
            .map_or(PolyRef::INVALID, |(r, _, _)| r);
        let eref = query
            .find_walk_poly(end, end_include, 0, 10.0)
            .map_or(PolyRef::INVALID, |(r, _, _)| r);
        assert_eq!(sref.0, p.start_ref, "path[{i}] start ref");
        assert_eq!(eref.0, p.end_ref, "path[{i}] end ref");
        if p.status == 0 {
            continue; // oracle had no endpoint poly for this pair
        }
        let (status, refs) = query.find_path(sref, eref, start, end, walk, 0, MAX_PATH_LENGTH);
        assert_eq!(status, Status(p.status), "path[{i}] status");
        let got_refs: Vec<u64> = refs.iter().map(|r| r.0).collect();
        assert_eq!(got_refs, p.refs, "path[{i}] poly corridor");

        let (straight_status, points) =
            query.find_straight_path(start, end, &refs, MAX_PATH_LENGTH);
        assert_eq!(
            straight_status,
            Status(p.straight_status),
            "path[{i}] straight status"
        );
        let got_points: Vec<(u32, u32, u32)> = points.iter().map(|pt| bits(*pt)).collect();
        assert_eq!(got_points, p.points, "path[{i}] straight points");
    }
}
