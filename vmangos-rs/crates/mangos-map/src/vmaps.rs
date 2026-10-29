//! Runtime vmap collision host (`VMapManager2` +
//! `StaticMapTree` + `ModelInstance`,
//! `vmangos/src/game/vmap/`), built on the
//! `mangos-assets` vmap parser.
//!
//! Deviations from the fork, recorded in
//! TIER3-PROGRESS Slice 1: tiles load lazily
//! around query points (the `NavHost` pattern)
//! instead of on grid activation, and loaded
//! tiles are never unloaded. The dynamic
//! gameobject tree is not modeled.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use mangos_assets::gridmap::INVALID_HEIGHT_VALUE;
use mangos_assets::vmap::{
    AABox, Bih, MOD_M2, MapTreeFile, Mat3, ModelSpawn, Ray, Vec3, WorldModel, parse_tile_file,
};

/// `SIZE_OF_GRIDS`.
#[allow(clippy::excessive_precision)]
const SIZE_OF_GRIDS: f32 = 533.333_33;
/// `0.5 * MAX_NUMBER_OF_GRIDS * SIZE_OF_GRIDS`
/// (`convertPositionToInternalRep` midpoint).
#[allow(clippy::excessive_precision)]
const INTERNAL_MID: f32 = 0.5 * 64.0 * 533.333_333_33;

/// `convertPositionToInternalRep` (self-inverse).
fn convert(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(INTERNAL_MID - x, INTERNAL_MID - y, z)
}

/// Fork tile pair for a world position (same
/// derivation as `Heights::tile_pair`; vmtile
/// filenames print `(map, tx, ty)`).
#[allow(clippy::cast_possible_truncation)]
fn tile_pair(x: f32, y: f32) -> (i32, i32) {
    let tx = (32.0 - y / SIZE_OF_GRIDS) as i32;
    let ty = (32.0 - x / SIZE_OF_GRIDS) as i32;
    (tx, ty)
}

/// `ModelInstance`: one placed world model.
/// `setModelFlags` lands on the instance here
/// (the fork writes it onto the shared model per
/// spawn; per-file type consistency makes the
/// observable result identical).
struct ModelInstance {
    flags: u32,
    pos: Vec3,
    bound: Option<AABox>,
    inv_rot: Mat3,
    scale: f32,
    inv_scale: f32,
    model: Option<Arc<WorldModel>>,
}

impl ModelInstance {
    fn new(spawn: &ModelSpawn, model: Option<Arc<WorldModel>>) -> Self {
        // ModelInstance ctor: rotation degrees ->
        // radians in (y, x, z) component order.
        let rot = Mat3::from_euler_angles_zyx(
            std::f32::consts::PI * spawn.rot.y / 180.0,
            std::f32::consts::PI * spawn.rot.x / 180.0,
            std::f32::consts::PI * spawn.rot.z / 180.0,
        );
        Self {
            flags: spawn.flags,
            pos: spawn.pos,
            bound: spawn.bound,
            inv_rot: rot.inverse(),
            scale: spawn.scale,
            inv_scale: 1.0 / spawn.scale,
            model,
        }
    }

    /// `ModelInstance::intersectRay`. A missing
    /// bound mirrors the fork's NaN default `AABox`
    /// (intersection time 0), i.e. no early-out.
    fn intersect_ray(
        &self,
        ray: &Ray,
        distance: &mut f32,
        stop_at_first: bool,
        ignore_m2: bool,
    ) -> bool {
        let Some(model) = self.model.as_ref() else {
            return false;
        };
        if let Some(bound) = self.bound {
            let t = ray.intersection_time_aabox(bound);
            if t > *distance {
                return false;
            }
        }
        if ignore_m2 && self.flags & MOD_M2 != 0 {
            return false;
        }
        let p = self.inv_rot.mul_vec(ray.origin - self.pos) * self.inv_scale;
        let dir = self.inv_rot.mul_vec(ray.direction);
        let inv_ray = Ray::new(p, dir);
        let mut dist = *distance * self.inv_scale;
        let hit = model.intersect_ray(&inv_ray, &mut dist, stop_at_first, ignore_m2);
        if hit {
            *distance = dist * self.scale;
        }
        hit
    }
}

/// `StaticMapTree`: prebuilt instance BIH plus
/// the lazily filled instance slots. All query
/// coordinates here are vmap-internal.
struct StaticMapTree {
    tiled: bool,
    tree: Bih,
    values: Vec<Option<ModelInstance>>,
    loaded_tiles: HashSet<(u32, u32)>,
}

impl StaticMapTree {
    /// `InitMap`: parse the `.vmtree`; non-tiled
    /// global spawns are materialized immediately
    /// (the fork's fake tile load).
    fn init(
        data: &[u8],
        acquire: &mut impl FnMut(&str) -> Option<Arc<WorldModel>>,
    ) -> Option<Self> {
        let file = MapTreeFile::parse(data).ok()?;
        let count = file.tree.prim_count();
        let mut out = Self {
            tiled: file.tiled,
            tree: file.tree,
            values: (0..count).map(|_| None).collect(),
            loaded_tiles: HashSet::new(),
        };
        for (spawn, referenced) in &file.global_spawns {
            out.fill_spawn(spawn, *referenced, acquire);
        }
        Some(out)
    }

    fn fill_spawn(
        &mut self,
        spawn: &ModelSpawn,
        referenced: u32,
        acquire: &mut impl FnMut(&str) -> Option<Arc<WorldModel>>,
    ) {
        let slot = referenced as usize;
        if slot >= self.values.len() {
            return;
        }
        // Fork: first writer wins (`getUnloaded`
        // guard); failed acquires still occupy
        // the slot with a null model.
        if self.values[slot].is_none() {
            let model = acquire(&spawn.name);
            self.values[slot] = Some(ModelInstance::new(spawn, model));
        }
    }

    /// `LoadMapTile` (no unload counterpart).
    fn load_tile(
        &mut self,
        dir: &std::path::Path,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        acquire: &mut impl FnMut(&str) -> Option<Arc<WorldModel>>,
    ) {
        if !self.loaded_tiles.insert((tile_x, tile_y)) {
            return;
        }
        if !self.tiled {
            return;
        }
        let path = dir.join(format!("{map_id:03}_{tile_x:02}_{tile_y:02}.vmtile"));
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let Ok(spawns) = parse_tile_file(&bytes) else {
            return;
        };
        for (spawn, referenced) in &spawns {
            self.fill_spawn(spawn, *referenced, acquire);
        }
    }

    /// `StaticMapTree::getIntersectionTime` over
    /// loaded instances; shrinks `max_dist` on hit.
    fn get_intersection_time(
        &self,
        ray: &Ray,
        max_dist: &mut f32,
        stop_at_first: bool,
        ignore_m2: bool,
    ) -> bool {
        let mut first = false;
        let values = &self.values;
        self.tree.intersect_ray(
            ray,
            &mut |entry, dist| {
                let hit = values
                    .get(entry as usize)
                    .and_then(Option::as_ref)
                    .is_some_and(|inst| inst.intersect_ray(ray, dist, stop_at_first, ignore_m2));
                first = first || hit;
                hit
            },
            max_dist,
            stop_at_first,
        );
        first
    }

    /// `StaticMapTree::isInLineOfSight`.
    fn is_in_line_of_sight(&self, pos1: Vec3, pos2: Vec3, ignore_m2: bool) -> bool {
        let dir = pos2 - pos1;
        let mut max_dist = dir.length();
        if max_dist < 1e-10 {
            return true;
        }
        let unit = dir * (1.0 / max_dist);
        let ray = Ray::new(pos1, unit);
        !self.get_intersection_time(&ray, &mut max_dist, true, ignore_m2)
    }

    /// `StaticMapTree::getObjectHitPos`: on hit,
    /// the point on src→dest at hit distance plus
    /// `modify_dist`, clamped at the source for a
    /// negative modify overshoot.
    fn get_object_hit_pos(&self, src: Vec3, dest: Vec3, modify_dist: f32) -> Option<Vec3> {
        let dir = dest - src;
        let mut max_dist = dir.length();
        if max_dist < 1e-10 {
            return None;
        }
        let unit = dir * (1.0 / max_dist);
        let ray = Ray::new(src, unit);
        if !self.get_intersection_time(&ray, &mut max_dist, false, false) {
            return None;
        }
        let mut at = max_dist + modify_dist;
        if at < 0.0 {
            at = 0.0;
        }
        Some(src + unit * at)
    }

    /// `StaticMapTree::getHeight`: vertical ray,
    /// +inf on miss.
    fn get_height(&self, pos: Vec3, max_search_dist: f32) -> f32 {
        let (dir, sign) = if max_search_dist >= 0.0 {
            (Vec3::new(0.0, 0.0, -1.0), -1.0)
        } else {
            (Vec3::new(0.0, 0.0, 1.0), 1.0)
        };
        let ray = Ray::new(pos, dir);
        let mut dist = max_search_dist.abs();
        if self.get_intersection_time(&ray, &mut dist, false, false) {
            pos.z + sign * dist
        } else {
            f32::INFINITY
        }
    }
}

/// Per-map vmap host (`VMapManager2` instance-tree
/// slice): lazy tree init, lazy 3×3 tile loading
/// around query endpoints, model file cache.
/// Public API takes world coordinates; the fork's
/// enable gates apply (`vmap.enableLOS` /
/// `vmap.enableHeight`).
pub struct VmapHost {
    map_id: u32,
    dir: Option<PathBuf>,
    tree: Option<StaticMapTree>,
    tree_failed: bool,
    models: HashMap<String, Option<Arc<WorldModel>>>,
    enable_los: bool,
    enable_height: bool,
}

impl VmapHost {
    #[must_use]
    pub fn new(map_id: u32) -> Self {
        Self {
            map_id,
            dir: None,
            tree: None,
            tree_failed: false,
            models: HashMap::new(),
            enable_los: false,
            enable_height: false,
        }
    }

    pub fn set_dir(&mut self, dir: Option<PathBuf>, enable_los: bool, enable_height: bool) {
        self.dir = dir;
        self.enable_los = enable_los;
        self.enable_height = enable_height;
        self.tree = None;
        self.tree_failed = false;
        self.models.clear();
    }

    #[must_use]
    pub const fn los_enabled(&self) -> bool {
        self.enable_los
    }

    #[must_use]
    pub const fn height_enabled(&self) -> bool {
        self.enable_height
    }

    fn ensure_tree(&mut self) -> Option<&mut StaticMapTree> {
        if self.tree.is_none() && !self.tree_failed {
            let path = self
                .dir
                .as_ref()
                .map(|dir| dir.join(format!("{:03}.vmtree", self.map_id)));
            let data = path.and_then(|p| std::fs::read(p).ok());
            // Split borrows: swap the model cache out
            // for the init callback.
            let mut models = std::mem::take(&mut self.models);
            let dir = self.dir.clone();
            let mut acquire = |name: &str| -> Option<Arc<WorldModel>> {
                if let Some(hit) = models.get(name) {
                    return hit.clone();
                }
                let loaded = dir.as_ref().and_then(|d| {
                    let bytes = std::fs::read(d.join(format!("{name}.vmo"))).ok()?;
                    WorldModel::parse(&bytes).ok().map(Arc::new)
                });
                models.insert(name.to_owned(), loaded.clone());
                loaded
            };
            let tree = data.and_then(|bytes| StaticMapTree::init(&bytes, &mut acquire));
            self.models = models;
            match tree {
                Some(t) => self.tree = Some(t),
                None => self.tree_failed = true,
            }
        }
        self.tree.as_mut()
    }

    /// Load the 3×3 tiles around a world position
    /// (fork loads on grid activation; queries here
    /// only originate near active objects).
    fn ensure_tiles(&mut self, x: f32, y: f32) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let (tx, ty) = tile_pair(x, y);
        let mut models = std::mem::take(&mut self.models);
        let mut acquire = |name: &str| -> Option<Arc<WorldModel>> {
            if let Some(hit) = models.get(name) {
                return hit.clone();
            }
            let loaded = {
                let bytes = std::fs::read(dir.join(format!("{name}.vmo"))).ok()?;
                WorldModel::parse(&bytes).ok().map(Arc::new)
            };
            models.insert(name.to_owned(), loaded.clone());
            loaded
        };
        if let Some(tree) = self.tree.as_mut() {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let (gx, gy) = (tx + dx, ty + dy);
                    if (0..64).contains(&gx) && (0..64).contains(&gy) {
                        #[allow(clippy::cast_sign_loss)]
                        tree.load_tile(&dir, self.map_id, gx as u32, gy as u32, &mut acquire);
                    }
                }
            }
        }
        self.models = models;
    }

    fn prepare(&mut self, points: &[(f32, f32)]) -> Option<&StaticMapTree> {
        self.ensure_tree()?;
        for &(x, y) in points {
            self.ensure_tiles(x, y);
        }
        self.tree.as_ref()
    }

    /// `VMapManager2::getHeight`: `INVALID_HEIGHT_VALUE`
    /// when height calc is disabled, no tree, or
    /// no hit within search distance.
    #[must_use]
    pub fn vmap_height(&mut self, x: f32, y: f32, z: f32, max_search: f32) -> f32 {
        if !self.enable_height {
            return INVALID_HEIGHT_VALUE;
        }
        let Some(tree) = self.prepare(&[(x, y)]) else {
            return INVALID_HEIGHT_VALUE;
        };
        let h = tree.get_height(convert(x, y, z), max_search);
        if h < f32::INFINITY {
            h
        } else {
            INVALID_HEIGHT_VALUE
        }
    }

    /// `VMapManager2::isInLineOfSight`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn is_in_line_of_sight(
        &mut self,
        x1: f32,
        y1: f32,
        z1: f32,
        x2: f32,
        y2: f32,
        z2: f32,
        ignore_m2: bool,
    ) -> bool {
        if !self.enable_los {
            return true;
        }
        let Some(tree) = self.prepare(&[(x1, y1), (x2, y2)]) else {
            return true;
        };
        let p1 = convert(x1, y1, z1);
        let p2 = convert(x2, y2, z2);
        if p1 == p2 {
            return true;
        }
        tree.is_in_line_of_sight(p1, p2, ignore_m2)
    }

    /// Static half of `Map::GetLosHitPosition`
    /// (`VMapManager2::getObjectHitPos`): `None`
    /// when no hit (destination unchanged).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn get_object_hit_pos(
        &mut self,
        x1: f32,
        y1: f32,
        z1: f32,
        x2: f32,
        y2: f32,
        z2: f32,
        modify_dist: f32,
    ) -> Option<(f32, f32, f32)> {
        if !self.enable_los {
            return None;
        }
        let tree = self.prepare(&[(x1, y1), (x2, y2)])?;
        let hit = tree.get_object_hit_pos(convert(x1, y1, z1), convert(x2, y2, z2), modify_dist)?;
        let out = convert(hit.x, hit.y, hit.z);
        Some((out.x, out.y, out.z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Data-driven host check against the bench
    /// vmap extract (map 0, Goldshire tiles):
    /// the largest building bound near Goldshire
    /// must block a chord through its center,
    /// report a hit position between the
    /// endpoints, and give a model height above
    /// its footprint; open ocean stays clear.
    #[test]
    #[ignore = "needs MANGOS_VMAPS_DIR bench data"]
    fn bench_vmap_host_queries() {
        let dir = PathBuf::from(std::env::var_os("MANGOS_VMAPS_DIR").expect("MANGOS_VMAPS_DIR"));
        let mut host = VmapHost::new(0);
        host.set_dir(Some(dir), true, true);
        assert!(host.prepare(&[(-9456.0, 43.0)]).is_some(), "tree loads");
        let (filled, candidates) = {
            let tree = host.tree.as_ref().expect("tree");
            let filled = tree.values.iter().filter(|v| v.is_some()).count();
            // Building-sized bounds only: zone-scale
            // WMOs have open air at bound mid-height.
            let mut candidates: Vec<AABox> = tree
                .values
                .iter()
                .flatten()
                .filter(|inst| inst.model.is_some())
                .filter_map(|inst| inst.bound)
                .filter(|b| {
                    let fp = (b.hi.x - b.lo.x).min(b.hi.y - b.lo.y);
                    (8.0..=60.0).contains(&fp) && (4.0..=40.0).contains(&(b.hi.z - b.lo.z))
                })
                .collect();
            candidates.sort_by(|a, b| {
                let sa = (a.hi.x - a.lo.x).min(a.hi.y - a.lo.y);
                let sb = (b.hi.x - b.lo.x).min(b.hi.y - b.lo.y);
                sb.total_cmp(&sa)
            });
            (filled, candidates)
        };
        assert!(filled > 50, "instances loaded: {filled}");
        assert!(!candidates.is_empty(), "building-sized bounds exist");
        // A near-floor chord through a walled
        // building's center must block for at least
        // one candidate (some bounds are open
        // structures like gazebos/bridges).
        let mut verified: Option<AABox> = None;
        for b in candidates.iter().take(24) {
            let c_i = Vec3::new(
                f32::midpoint(b.lo.x, b.hi.x),
                f32::midpoint(b.lo.y, b.hi.y),
                b.lo.z + 2.0,
            );
            let c = convert(c_i.x, c_i.y, c_i.z);
            let along_x = (b.hi.x - b.lo.x) >= (b.hi.y - b.lo.y);
            let half = if along_x {
                (b.hi.x - b.lo.x) * 0.5
            } else {
                (b.hi.y - b.lo.y) * 0.5
            };
            let (ax, ay) = if along_x {
                (half + 12.0, 0.0)
            } else {
                (0.0, half + 12.0)
            };
            if !host.is_in_line_of_sight(c.x - ax, c.y - ay, c.z, c.x + ax, c.y + ay, c.z, false) {
                verified = Some(*b);
                break;
            }
        }
        let b = verified.expect("some building near Goldshire blocks its floor chord");
        // Same near-floor chord for the hit-pos and
        // height assertions (convert flips signs,
        // extents preserved).
        let center_i = Vec3::new(
            f32::midpoint(b.lo.x, b.hi.x),
            f32::midpoint(b.lo.y, b.hi.y),
            b.lo.z + 2.0,
        );
        let c = convert(center_i.x, center_i.y, center_i.z);
        let along_x = (b.hi.x - b.lo.x) >= (b.hi.y - b.lo.y);
        let half = if along_x {
            (b.hi.x - b.lo.x) * 0.5
        } else {
            (b.hi.y - b.lo.y) * 0.5
        };
        let (ax, ay) = if along_x {
            (half + 12.0, 0.0)
        } else {
            (0.0, half + 12.0)
        };
        let (x1, y1, x2, y2) = (c.x - ax, c.y - ay, c.x + ax, c.y + ay);
        assert!(
            !host.is_in_line_of_sight(x1, y1, c.z, x2, y2, c.z, false),
            "chord through the building must be blocked"
        );
        let (hx, hy, _hz) = host
            .get_object_hit_pos(x1, y1, c.z, x2, y2, c.z, -1.0)
            .expect("hit position");
        let along = if along_x { hx } else { hy };
        let (lo_e, hi_e) = if along_x { (x1, x2) } else { (y1, y2) };
        assert!(
            along > lo_e && along < hi_e,
            "hit {along} must lie between {lo_e} and {hi_e}"
        );
        // Height over the footprint from far above
        // lands on the model, not below its base.
        let h = host.vmap_height(c.x, c.y, c.z + 200.0, 400.0);
        assert!(
            h > INVALID_HEIGHT_VALUE && h >= b.lo.z - 1.0,
            "model height {h} vs bound base {}",
            b.lo.z
        );
        // Open ocean west of Westfall: clear LoS,
        // no model height.
        assert!(host.is_in_line_of_sight(-11_000.0, 800.0, 100.0, -10_980.0, 800.0, 100.0, false));
        assert!(
            (host.vmap_height(-11_000.0, 800.0, 100.0, 500.0) - INVALID_HEIGHT_VALUE).abs()
                < f32::EPSILON
        );
        println!("LIVE-VMAP-PASS filled={filled}");
    }
}
