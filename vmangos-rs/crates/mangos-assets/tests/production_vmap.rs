//! Production vmap parity suite. `#[ignore]`d; gated on
//! `MANGOS_VMAPS_DIR` (dev bench: `~/azeroth-dev-fixtures/datadir/vmaps`).
//! Goldens come from the independent Python oracle
//! `vmangos-rs/scripts/vmap_oracle_gen.py`: parse digests for every
//! `.vmo`/`.vmtile`/`.vmtree`, and brute-force (exhaustive-triangle)
//! ray-probe digests for a stratified small-model sample. A third
//! check compares BIH traversal against an exhaustive scan on the
//! largest production models (same triangle routine, independent
//! search strategy).

use std::path::PathBuf;

use mangos_assets::vmap::{
    Bih, MapTreeFile, ModelSpawn, Ray, Vec3, WorldModel, intersect_triangle, parse_tile_file,
};
use sha2::{Digest, Sha256};

mod vmap_oracle;
use vmap_oracle::{PROBES, TILES, TREES, VMOS};

fn vmaps_dir() -> PathBuf {
    std::env::var_os("MANGOS_VMAPS_DIR")
        .expect("MANGOS_VMAPS_DIR must be set for production vmap parity tests")
        .into()
}

fn hex_of(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    digest.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn bih_digest(bih: &Bih) -> [u8; 32] {
    let mut h = Sha256::new();
    let b = bih.bounds();
    for v in [b.lo.x, b.lo.y, b.lo.z, b.hi.x, b.hi.y, b.hi.z] {
        h.update(v.to_le_bytes());
    }
    h.update((bih.tree().len() as u32).to_le_bytes());
    for t in bih.tree() {
        h.update(t.to_le_bytes());
    }
    h.update((bih.objects().len() as u32).to_le_bytes());
    for o in bih.objects() {
        h.update(o.to_le_bytes());
    }
    h.finalize().into()
}

fn vmo_digest(model: &WorldModel) -> String {
    let mut h = Sha256::new();
    h.update(model.root_wmo_id().to_le_bytes());
    h.update((model.group_models().len() as u32).to_le_bytes());
    for g in model.group_models() {
        let b = g.bound();
        for v in [b.lo.x, b.lo.y, b.lo.z, b.hi.x, b.hi.y, b.hi.z] {
            h.update(v.to_le_bytes());
        }
        h.update(g.mogp_flags().to_le_bytes());
        h.update(g.wmo_id().to_le_bytes());
        h.update((g.vertices().len() as u32).to_le_bytes());
        h.update((g.triangles().len() as u32).to_le_bytes());
        if !g.vertices().is_empty() {
            let mut vh = Sha256::new();
            for v in g.vertices() {
                vh.update(v.x.to_le_bytes());
                vh.update(v.y.to_le_bytes());
                vh.update(v.z.to_le_bytes());
            }
            h.update(vh.finalize());
            let mut th = Sha256::new();
            for t in g.triangles() {
                th.update(t.idx0.to_le_bytes());
                th.update(t.idx1.to_le_bytes());
                th.update(t.idx2.to_le_bytes());
            }
            h.update(th.finalize());
            h.update(bih_digest(g.mesh_tree()));
        }
        h.update(g.liquid_type().to_le_bytes());
    }
    if !model.group_models().is_empty() {
        h.update(bih_digest(model.group_tree()));
    }
    hex_of(&h.finalize())
}

fn spawn_digest(spawn: &ModelSpawn) -> [u8; 32] {
    assert!(
        spawn.name.is_ascii(),
        "spawn name not ASCII: {:?}",
        spawn.name
    );
    let mut h = Sha256::new();
    h.update(spawn.flags.to_le_bytes());
    h.update(spawn.adt_id.to_le_bytes());
    h.update(spawn.id.to_le_bytes());
    for v in [spawn.pos, spawn.rot] {
        h.update(v.x.to_le_bytes());
        h.update(v.y.to_le_bytes());
        h.update(v.z.to_le_bytes());
    }
    h.update(spawn.scale.to_le_bytes());
    if let Some(b) = spawn.bound {
        for v in [b.lo.x, b.lo.y, b.lo.z, b.hi.x, b.hi.y, b.hi.z] {
            h.update(v.to_le_bytes());
        }
    }
    h.update((spawn.name.len() as u32).to_le_bytes());
    h.update(spawn.name.as_bytes());
    h.finalize().into()
}

/// Mirrors `probe_rays` in the oracle generator.
fn probe_rays(model: &WorldModel) -> Vec<(Vec3, Vec3)> {
    let mut lo = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut hi = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for g in model.group_models() {
        if g.triangles().is_empty() {
            continue;
        }
        let b = g.bound();
        lo = Vec3::new(lo.x.min(b.lo.x), lo.y.min(b.lo.y), lo.z.min(b.lo.z));
        hi = Vec3::new(hi.x.max(b.hi.x), hi.y.max(b.hi.y), hi.z.max(b.hi.z));
    }
    let fracs = [0.13f32, 0.37, 0.61, 0.89];
    let mix = |a: f32, b: f32, fr: f32| a + (b - a) * fr;
    let mut rays = Vec::new();
    for fx in fracs {
        for fy in fracs {
            rays.push((
                Vec3::new(mix(lo.x, hi.x, fx), mix(lo.y, hi.y, fy), hi.z + 2.0),
                Vec3::new(0.0, 0.0, -1.0),
            ));
        }
    }
    for fz in fracs {
        let z = mix(lo.z, hi.z, fz);
        rays.push((
            Vec3::new(lo.x - 2.0, mix(lo.y, hi.y, 0.5), z),
            Vec3::new(1.0, 0.0, 0.0),
        ));
        rays.push((
            Vec3::new(mix(lo.x, hi.x, 0.5), lo.y - 2.0, z),
            Vec3::new(0.0, 1.0, 0.0),
        ));
    }
    rays
}

#[test]
#[ignore = "requires MANGOS_VMAPS_DIR with production vmap fixtures"]
fn all_vmo_parse_digests() {
    let dir = vmaps_dir();
    assert_eq!(VMOS.len(), 3490);
    for golden in VMOS {
        let bytes = std::fs::read(dir.join(golden.name)).expect("vmo present");
        let model = WorldModel::parse(&bytes)
            .unwrap_or_else(|e| panic!("{}: parse failed: {e}", golden.name));
        let tris: usize = model
            .group_models()
            .iter()
            .map(|g| g.triangles().len())
            .sum();
        assert_eq!(tris as u32, golden.triangles, "{}", golden.name);
        assert_eq!(vmo_digest(&model), golden.sha256, "{}", golden.name);
    }
}

#[test]
#[ignore = "requires MANGOS_VMAPS_DIR with production vmap fixtures"]
fn all_tiles_and_trees_parse_digests() {
    let dir = vmaps_dir();
    assert_eq!(TILES.len(), 1250);
    for golden in TILES {
        let bytes = std::fs::read(dir.join(golden.name)).expect("tile present");
        let spawns = parse_tile_file(&bytes)
            .unwrap_or_else(|e| panic!("{}: parse failed: {e}", golden.name));
        assert_eq!(spawns.len() as u32, golden.count, "{}", golden.name);
        let mut h = Sha256::new();
        for (spawn, referenced) in &spawns {
            h.update(spawn_digest(spawn));
            h.update(referenced.to_le_bytes());
        }
        assert_eq!(hex_of(&h.finalize()), golden.sha256, "{}", golden.name);
    }

    assert_eq!(TREES.len(), 43);
    for golden in TREES {
        let bytes = std::fs::read(dir.join(golden.name)).expect("tree present");
        let tree = MapTreeFile::parse(&bytes)
            .unwrap_or_else(|e| panic!("{}: parse failed: {e}", golden.name));
        assert_eq!(tree.tiled, golden.tiled, "{}", golden.name);
        assert_eq!(
            hex_of(&bih_digest(&tree.tree)),
            golden.bih_sha256,
            "{}",
            golden.name
        );
        match golden.spawn_count {
            Some(count) => {
                assert_eq!(tree.global_spawns.len() as u32, count, "{}", golden.name);
                let mut h = Sha256::new();
                for (spawn, referenced) in &tree.global_spawns {
                    h.update(spawn_digest(spawn));
                    h.update(referenced.to_le_bytes());
                }
                assert_eq!(
                    hex_of(&h.finalize()),
                    golden.spawns_sha256,
                    "{}",
                    golden.name
                );
            }
            None => assert!(tree.global_spawns.is_empty(), "{}", golden.name),
        }
    }
}

#[test]
#[ignore = "requires MANGOS_VMAPS_DIR with production vmap fixtures"]
fn probe_rays_match_brute_force_oracle() {
    let dir = vmaps_dir();
    assert_eq!(PROBES.len(), 120);
    for golden in PROBES {
        let bytes = std::fs::read(dir.join(golden.name)).expect("probe model present");
        let model = WorldModel::parse(&bytes).expect("parse");
        let mut h = Sha256::new();
        for (origin, direction) in probe_rays(&model) {
            let ray = Ray::new(origin, direction);
            let mut dist = f32::INFINITY;
            let hit = model.intersect_ray(&ray, &mut dist, false, false);
            h.update([u8::from(hit)]);
            h.update(dist.to_le_bytes());
        }
        assert_eq!(
            hex_of(&h.finalize()),
            golden.probe_sha256,
            "{}",
            golden.name
        );
    }
}

#[test]
#[ignore = "requires MANGOS_VMAPS_DIR with production vmap fixtures"]
fn bih_traversal_matches_exhaustive_scan_on_large_models() {
    let dir = vmaps_dir();
    let mut by_size: Vec<&vmap_oracle::VmoGolden> = VMOS.iter().collect();
    by_size.sort_by_key(|g| std::cmp::Reverse(g.triangles));
    for golden in by_size.into_iter().take(8) {
        let bytes = std::fs::read(dir.join(golden.name)).expect("large model present");
        let model = WorldModel::parse(&bytes).expect("parse");
        for (origin, direction) in probe_rays(&model) {
            let ray = Ray::new(origin, direction);
            let mut dist_bih = f32::INFINITY;
            let hit_bih = model.intersect_ray(&ray, &mut dist_bih, false, false);

            let mut dist_scan = f32::INFINITY;
            let mut hit_scan = false;
            for g in model.group_models() {
                for tri in g.triangles() {
                    if intersect_triangle(
                        g.vertices()[tri.idx0 as usize],
                        g.vertices()[tri.idx1 as usize],
                        g.vertices()[tri.idx2 as usize],
                        &ray,
                        &mut dist_scan,
                    ) {
                        hit_scan = true;
                    }
                }
            }
            assert_eq!(
                hit_bih, hit_scan,
                "{} ray {origin:?}->{direction:?}",
                golden.name
            );
            assert_eq!(
                dist_bih.to_bits(),
                dist_scan.to_bits(),
                "{} ray {origin:?}->{direction:?}",
                golden.name
            );
        }
    }
}
