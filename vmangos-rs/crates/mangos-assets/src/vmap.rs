//! `VMap` geometry: parsers + ray queries mirroring
//! `vmangos/src/game/vmap/{BIH,WorldModel,MapTree,ModelInstance}.*`.
//!
//! File formats (little-endian, `VMAP_7.0` magic):
//! - `.vmo` (`WorldModel::readFile`): magic, `WMOD`, root id, then an
//!   optional `GMOD` section with `GroupModel`s and the group BIH.
//! - `.vmtree` (`StaticMapTree::InitMap`): magic, tiled flag, `NODE`
//!   BIH over model instances, `GOBJ` global spawns (non-tiled maps).
//! - `.vmtile` (`LoadMapTile`): magic, spawn count, spawn records.
//!
//! Ray traversal copies `BIH::intersectRay` step for step (same f32
//! operation order, same G3D fuzzy-compare epsilon), and the triangle
//! test copies `IntersectTriangle` (Möller, `EPS = 1e-5`), so query
//! results are bit-identical to the C++ for the same inputs.

/// 3-component f32 vector (`G3D::Vector3` subset, same op order).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub const fn axis(self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    #[must_use]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// `Vector3::magnitude`.
    #[must_use]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }
}

/// `G3D::Matrix3` subset (row-major `elt[row][col]`,
/// column-vector multiply as in G3D).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub elt: [[f32; 3]; 3],
}

impl Mat3 {
    /// `Matrix3::zero()`.
    pub const ZERO: Self = Self { elt: [[0.0; 3]; 3] };

    #[must_use]
    pub const fn from_rows(r0: [f32; 3], r1: [f32; 3], r2: [f32; 3]) -> Self {
        Self { elt: [r0, r1, r2] }
    }

    /// `Matrix3::fromEulerAnglesZYX(yaw, pitch, roll)`
    /// = `Rz(yaw) * Ry(pitch) * Rx(roll)`.
    #[must_use]
    pub fn from_euler_angles_zyx(yaw: f32, pitch: f32, roll: f32) -> Self {
        let (cy, sy) = (yaw.cos(), yaw.sin());
        let z = Self::from_rows([cy, -sy, 0.0], [sy, cy, 0.0], [0.0, 0.0, 1.0]);
        let (cp, sp) = (pitch.cos(), pitch.sin());
        let y = Self::from_rows([cp, 0.0, sp], [0.0, 1.0, 0.0], [-sp, 0.0, cp]);
        let (cr, sr) = (roll.cos(), roll.sin());
        let x = Self::from_rows([1.0, 0.0, 0.0], [0.0, cr, -sr], [0.0, sr, cr]);
        z.mul_mat(y.mul_mat(x))
    }

    /// Matrix product `self * rhs`.
    #[must_use]
    pub fn mul_mat(self, rhs: Self) -> Self {
        let mut out = Self::ZERO;
        for (i, row) in out.elt.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.elt[i][0] * rhs.elt[0][j]
                    + self.elt[i][1] * rhs.elt[1][j]
                    + self.elt[i][2] * rhs.elt[2][j];
            }
        }
        out
    }

    /// `Matrix3 * Vector3`.
    #[must_use]
    pub fn mul_vec(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.elt[0][0] * v.x + self.elt[0][1] * v.y + self.elt[0][2] * v.z,
            self.elt[1][0] * v.x + self.elt[1][1] * v.y + self.elt[1][2] * v.z,
            self.elt[2][0] * v.x + self.elt[2][1] * v.y + self.elt[2][2] * v.z,
        )
    }

    /// `Matrix3::inverse(1e-06)`: cofactor inverse,
    /// `ZERO` when `|det| <= 1e-6` (the C++
    /// two-arg form leaving the zero matrix).
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn inverse(self) -> Self {
        let e = self.elt;
        let mut inv = Self::ZERO;
        inv.elt[0][0] = e[1][1] * e[2][2] - e[1][2] * e[2][1];
        inv.elt[0][1] = e[0][2] * e[2][1] - e[0][1] * e[2][2];
        inv.elt[0][2] = e[0][1] * e[1][2] - e[0][2] * e[1][1];
        inv.elt[1][0] = e[1][2] * e[2][0] - e[1][0] * e[2][2];
        inv.elt[1][1] = e[0][0] * e[2][2] - e[0][2] * e[2][0];
        inv.elt[1][2] = e[0][2] * e[1][0] - e[0][0] * e[1][2];
        inv.elt[2][0] = e[1][0] * e[2][1] - e[1][1] * e[2][0];
        inv.elt[2][1] = e[0][1] * e[2][0] - e[0][0] * e[2][1];
        inv.elt[2][2] = e[0][0] * e[1][1] - e[0][1] * e[1][0];
        let det = e[0][0] * inv.elt[0][0] + e[0][1] * inv.elt[1][0] + e[0][2] * inv.elt[2][0];
        if det.abs() <= 1e-06 {
            return Self::ZERO;
        }
        let inv_det = 1.0 / det;
        for row in &mut inv.elt {
            for cell in row {
                *cell *= inv_det;
            }
        }
        inv
    }
}

/// Axis-aligned box (`G3D::AABox` low/high).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AABox {
    pub lo: Vec3,
    pub hi: Vec3,
}

impl AABox {
    /// `AABox::contains` (inclusive on all axes).
    #[must_use]
    pub fn contains(&self, p: Vec3) -> bool {
        p.x >= self.lo.x
            && p.x <= self.hi.x
            && p.y >= self.lo.y
            && p.y <= self.hi.y
            && p.z >= self.lo.z
            && p.z <= self.hi.z
    }
}

/// `G3D::Ray` subset. Direction is used as given (callers pass unit
/// directions, as in the C++); the inverse is `1.0 / component`.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
    pub inv_direction: Vec3,
}

impl Ray {
    #[must_use]
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction,
            inv_direction: Vec3::new(1.0 / direction.x, 1.0 / direction.y, 1.0 / direction.z),
        }
    }

    /// `Ray::intersectionTime(AABox)` via
    /// `CollisionDetection::collisionTimeForMoving
    /// PointFixedAABox`: 0 when the origin is
    /// inside, the hit distance (Euclidean
    /// `|location - origin|`, as in G3D), +inf on
    /// miss.
    #[must_use]
    pub fn intersection_time_aabox(&self, b: AABox) -> f32 {
        let o = [self.origin.x, self.origin.y, self.origin.z];
        let d = [self.direction.x, self.direction.y, self.direction.z];
        let lo = [b.lo.x, b.lo.y, b.lo.z];
        let hi = [b.hi.x, b.hi.y, b.hi.z];
        let mut inside = true;
        let mut loc = o;
        let mut max_t = [-1.0f32; 3];
        for i in 0..3 {
            if o[i] < lo[i] {
                loc[i] = lo[i];
                inside = false;
                // IR(dir): any nonzero bit pattern.
                if d[i].to_bits() != 0 {
                    max_t[i] = (lo[i] - o[i]) / d[i];
                }
            } else if o[i] > hi[i] {
                loc[i] = hi[i];
                inside = false;
                if d[i].to_bits() != 0 {
                    max_t[i] = (hi[i] - o[i]) / d[i];
                }
            }
        }
        if inside {
            return 0.0;
        }
        let mut which = 0usize;
        if max_t[1] > max_t[which] {
            which = 1;
        }
        if max_t[2] > max_t[which] {
            which = 2;
        }
        if max_t[which] < 0.0 {
            return f32::INFINITY;
        }
        for i in 0..3 {
            if i != which {
                loc[i] = o[i] + max_t[which] * d[i];
                if loc[i] < lo[i] || loc[i] > hi[i] {
                    return f32::INFINITY;
                }
            }
        }
        (Vec3::new(loc[0], loc[1], loc[2]) - self.origin).length()
    }
}

/// `G3D::fuzzyNe(float, float)` with `fuzzyEpsilon32 = 1e-5`.
#[allow(clippy::float_cmp)] // exact-equality shortcut is G3D's own first branch
fn fuzzy_ne(a: f32, b: f32) -> bool {
    if a == b {
        return false;
    }
    let aa = a.abs() + 1.0;
    let eps = if aa == f32::INFINITY {
        0.00001
    } else {
        0.00001 * aa
    };
    (a - b).abs() > eps
}

/// Failure modes of vmap parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmapError {
    /// Fewer bytes than a record requires.
    Truncated,
    /// File magic is not `VMAP_7.0`.
    BadMagic,
    /// A chunk tag (`WMOD`/`GMOD`/...) does not match.
    BadChunk(&'static str),
    /// Declared sizes overflow `usize` arithmetic.
    SizeOverflow,
}

impl std::fmt::Display for VmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated vmap file"),
            Self::BadMagic => write!(f, "bad vmap magic (want VMAP_7.0)"),
            Self::BadChunk(c) => write!(f, "bad vmap chunk (want {c})"),
            Self::SizeOverflow => write!(f, "vmap declared sizes overflow"),
        }
    }
}

impl std::error::Error for VmapError {}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], VmapError> {
        let out = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or(VmapError::Truncated)?;
        self.pos += n;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, VmapError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, VmapError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| VmapError::Truncated)?,
        ))
    }

    pub fn u32(&mut self) -> Result<u32, VmapError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| VmapError::Truncated)?,
        ))
    }

    pub fn f32(&mut self) -> Result<f32, VmapError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn vec3(&mut self) -> Result<Vec3, VmapError> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    pub fn aabox(&mut self) -> Result<AABox, VmapError> {
        Ok(AABox {
            lo: self.vec3()?,
            hi: self.vec3()?,
        })
    }

    pub fn u32s(&mut self, n: u32) -> Result<Vec<u32>, VmapError> {
        let bytes = self.take((n as usize).checked_mul(4).ok_or(VmapError::SizeOverflow)?)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }

    pub fn f32s(&mut self, n: usize) -> Result<Vec<f32>, VmapError> {
        let bytes = self.take(n.checked_mul(4).ok_or(VmapError::SizeOverflow)?)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }

    pub fn chunk(&mut self, want: &'static str) -> Result<(), VmapError> {
        if self.take(4)? == want.as_bytes() {
            Ok(())
        } else {
            Err(VmapError::BadChunk(want))
        }
    }

    pub fn magic(&mut self) -> Result<(), VmapError> {
        if self.take(8)? == b"VMAP_7.0" {
            Ok(())
        } else {
            Err(VmapError::BadMagic)
        }
    }
}

/// Bounding interval hierarchy (`BIH`), file form + ray traversal.
#[derive(Clone, Debug, Default)]
pub struct Bih {
    tree: Vec<u32>,
    objects: Vec<u32>,
    bounds: AABox,
}

impl Bih {
    /// The empty tree the C++ default-constructs (dummy leaf).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            tree: vec![3 << 30, 0, 0],
            objects: Vec::new(),
            bounds: AABox::default(),
        }
    }

    /// `BIH::readFromFile`.
    ///
    /// # Errors
    ///
    /// [`VmapError`] on truncation/overflow.
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, VmapError> {
        let lo = r.vec3()?;
        let hi = r.vec3()?;
        let tree_size = r.u32()?;
        let tree = r.u32s(tree_size)?;
        let count = r.u32()?;
        let objects = r.u32s(count)?;
        Ok(Self {
            tree,
            objects,
            bounds: AABox { lo, hi },
        })
    }

    /// `primCount`.
    #[must_use]
    pub fn prim_count(&self) -> usize {
        self.objects.len()
    }

    /// Overall bounds.
    #[must_use]
    pub const fn bounds(&self) -> AABox {
        self.bounds
    }

    /// Serialized node words (as stored in the file).
    #[must_use]
    pub fn tree(&self) -> &[u32] {
        &self.tree
    }

    /// Primitive index table (as stored in the file).
    #[must_use]
    pub fn objects(&self) -> &[u32] {
        &self.objects
    }

    /// `BIH::intersectRay`: walks the tree, invoking
    /// `callback(entry, max_dist)` for leaf primitives; the callback
    /// may shrink `max_dist` and returns whether it hit.
    #[allow(
        clippy::redundant_else,
        clippy::similar_names,
        clippy::too_many_lines,
        clippy::if_not_else,
        clippy::cast_possible_wrap
    )]
    pub fn intersect_ray(
        &self,
        ray: &Ray,
        callback: &mut dyn FnMut(u32, &mut f32) -> bool,
        max_dist: &mut f32,
        stop_at_first: bool,
    ) {
        let mut interval_min = -1.0f32;
        let mut interval_max = -1.0f32;
        let (org, dir, inv) = (ray.origin, ray.direction, ray.inv_direction);
        for i in 0..3 {
            if fuzzy_ne(dir.axis(i), 0.0) {
                let mut t1 = (self.bounds.lo.axis(i) - org.axis(i)) * inv.axis(i);
                let mut t2 = (self.bounds.hi.axis(i) - org.axis(i)) * inv.axis(i);
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                if t1 > interval_min {
                    interval_min = t1;
                }
                if t2 < interval_max || interval_max < 0.0 {
                    interval_max = t2;
                }
                if interval_max <= 0.0 || interval_min >= *max_dist {
                    return;
                }
            }
        }
        if interval_min > interval_max {
            return;
        }
        interval_min = interval_min.max(0.0);
        interval_max = interval_max.min(*max_dist);

        let mut offset_front = [0u32; 3];
        let mut offset_back = [0u32; 3];
        let mut offset_front3 = [0u32; 3];
        let mut offset_back3 = [0u32; 3];
        for i in 0..3 {
            offset_front[i] = dir.axis(i).to_bits() >> 31;
            offset_back[i] = offset_front[i] ^ 1;
            offset_front3[i] = offset_front[i] * 3;
            offset_back3[i] = offset_back[i] * 3;
            offset_front[i] += 1;
            offset_back[i] += 1;
        }

        let mut stack: Vec<(u32, f32, f32)> = Vec::with_capacity(64);
        let mut node: u32 = 0;
        'outer: loop {
            loop {
                let tn = self.tree[node as usize];
                let axis = ((tn >> 30) & 3) as usize;
                let bvh2 = tn & (1 << 29) != 0;
                let offset = tn & !(7 << 29);
                if !bvh2 {
                    if axis < 3 {
                        let tf = (f32::from_bits(self.tree[(node + offset_front[axis]) as usize])
                            - org.axis(axis))
                            * inv.axis(axis);
                        let tb = (f32::from_bits(self.tree[(node + offset_back[axis]) as usize])
                            - org.axis(axis))
                            * inv.axis(axis);
                        if tf < interval_min && tb > interval_max {
                            break;
                        }
                        let back = offset + offset_back3[axis];
                        node = back;
                        if tf < interval_min {
                            if tb >= interval_min {
                                interval_min = tb;
                            }
                            continue;
                        }
                        node = offset + offset_front3[axis];
                        if tb > interval_max {
                            if tf <= interval_max {
                                interval_max = tf;
                            }
                            continue;
                        }
                        stack.push((
                            back,
                            if tb >= interval_min { tb } else { interval_min },
                            interval_max,
                        ));
                        if tf <= interval_max {
                            interval_max = tf;
                        }
                        continue;
                    }
                    // leaf
                    let mut n = self.tree[(node + 1) as usize] as i32;
                    let mut off = offset;
                    while n > 0 {
                        let hit = callback(self.objects[off as usize], max_dist);
                        if stop_at_first && hit {
                            return;
                        }
                        n -= 1;
                        off += 1;
                    }
                    break;
                } else {
                    if axis > 2 {
                        return;
                    }
                    let tf = (f32::from_bits(self.tree[(node + offset_front[axis]) as usize])
                        - org.axis(axis))
                        * inv.axis(axis);
                    let tb = (f32::from_bits(self.tree[(node + offset_back[axis]) as usize])
                        - org.axis(axis))
                        * inv.axis(axis);
                    node = offset;
                    if tf >= interval_min {
                        interval_min = tf;
                    }
                    if tb <= interval_max {
                        interval_max = tb;
                    }
                    if interval_min > interval_max {
                        break;
                    }
                }
            }
            loop {
                let Some((saved_node, tnear, tfar)) = stack.pop() else {
                    return;
                };
                interval_min = tnear;
                if *max_dist < interval_min {
                    continue;
                }
                node = saved_node;
                interval_max = tfar;
                continue 'outer;
            }
        }
    }
}

/// `IntersectTriangle` (Möller; `WorldModel.cpp:35-82`).
/// On a closer hit, stores `t` into `distance` and returns true.
#[allow(clippy::many_single_char_names)]
pub fn intersect_triangle(p0: Vec3, p1: Vec3, p2: Vec3, ray: &Ray, distance: &mut f32) -> bool {
    const EPS: f32 = 1e-5;
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let p = ray.direction.cross(e2);
    let a = e1.dot(p);
    if a.abs() < EPS {
        return false;
    }
    let f = 1.0 / a;
    let s = ray.origin - p0;
    let u = f * s.dot(p);
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = f * ray.direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = f * e2.dot(q);
    if t > 0.0 && t < *distance {
        *distance = t;
        return true;
    }
    false
}

/// One mesh triangle (vertex indices).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeshTriangle {
    pub idx0: u32,
    pub idx1: u32,
    pub idx2: u32,
}

/// `WmoLiquid` heightfield.
#[derive(Clone, Debug)]
pub struct WmoLiquid {
    tiles_x: u32,
    tiles_y: u32,
    corner: Vec3,
    liquid_type: u32,
    heights: Vec<f32>,
    flags: Vec<u8>,
}

impl WmoLiquid {
    fn read(r: &mut Reader<'_>) -> Result<Self, VmapError> {
        let tiles_x = r.u32()?;
        let tiles_y = r.u32()?;
        let corner = r.vec3()?;
        let liquid_type = r.u32()?;
        let heights = r.f32s(
            ((tiles_x + 1) as usize)
                .checked_mul((tiles_y + 1) as usize)
                .ok_or(VmapError::SizeOverflow)?,
        )?;
        let flags = r
            .take(
                (tiles_x as usize)
                    .checked_mul(tiles_y as usize)
                    .ok_or(VmapError::SizeOverflow)?,
            )?
            .to_vec();
        Ok(Self {
            tiles_x,
            tiles_y,
            corner,
            liquid_type,
            heights,
            flags,
        })
    }

    /// `GetType`.
    #[must_use]
    pub const fn liquid_type(&self) -> u32 {
        self.liquid_type
    }

    /// `GetLiquidHeight` (`WorldModel.cpp`): two-triangle tesselation
    /// of the liquid tile, `None` outside or on disabled tiles.
    #[must_use]
    #[allow(clippy::similar_names, clippy::cast_precision_loss)]
    pub fn liquid_height(&self, pos: Vec3) -> Option<f32> {
        const LIQUID_TILE_SIZE: f32 = 533.333 / 128.0; // VMapDefinitions.h:22
        let tx_f = (pos.x - self.corner.x) / LIQUID_TILE_SIZE;
        let tx = tx_f as u32;
        if tx_f < 0.0 || tx >= self.tiles_x {
            return None;
        }
        let ty_f = (pos.y - self.corner.y) / LIQUID_TILE_SIZE;
        let ty = ty_f as u32;
        if ty_f < 0.0 || ty >= self.tiles_y {
            return None;
        }
        let (tx, ty) = (tx as usize, ty as usize);
        if self.flags[tx + ty * self.tiles_x as usize] & 0x0F == 0x0F {
            return None;
        }
        let dx = tx_f - tx as f32;
        let dy = ty_f - ty as f32;
        let row_offset = self.tiles_x as usize + 1;
        let h = |x: usize, y: usize| self.heights[x + y * row_offset];
        let (sx, sy) = if dx > dy {
            (h(tx + 1, ty) - h(tx, ty), h(tx + 1, ty + 1) - h(tx + 1, ty))
        } else {
            (h(tx + 1, ty + 1) - h(tx, ty + 1), h(tx, ty + 1) - h(tx, ty))
        };
        Some(h(tx, ty) + dx * sx + dy * sy)
    }
}

/// One WMO group / M2 model (`GroupModel`).
#[derive(Clone, Debug, Default)]
pub struct GroupModel {
    bound: AABox,
    mogp_flags: u32,
    group_wmo_id: u32,
    vertices: Vec<Vec3>,
    triangles: Vec<MeshTriangle>,
    mesh_tree: Bih,
    liquid: Option<WmoLiquid>,
}

impl GroupModel {
    fn read(r: &mut Reader<'_>) -> Result<Self, VmapError> {
        let bound = r.aabox()?;
        let mogp_flags = r.u32()?;
        let group_wmo_id = r.u32()?;
        r.chunk("VERT")?;
        let _chunk_size = r.u32()?;
        let count = r.u32()?;
        let mut model = Self {
            bound,
            mogp_flags,
            group_wmo_id,
            ..Self::default()
        };
        if count == 0 {
            return Ok(model);
        }
        let mut vertices = Vec::with_capacity(count as usize);
        for _ in 0..count {
            vertices.push(r.vec3()?);
        }
        model.vertices = vertices;

        r.chunk("TRIM")?;
        let _chunk_size = r.u32()?;
        let tri_count = r.u32()?;
        let mut triangles = Vec::with_capacity(tri_count as usize);
        for _ in 0..tri_count {
            triangles.push(MeshTriangle {
                idx0: r.u32()?,
                idx1: r.u32()?,
                idx2: r.u32()?,
            });
        }
        model.triangles = triangles;

        r.chunk("MBIH")?;
        model.mesh_tree = Bih::read(r)?;

        r.chunk("LIQU")?;
        let chunk_size = r.u32()?;
        if chunk_size > 0 {
            model.liquid = Some(WmoLiquid::read(r)?);
        }
        Ok(model)
    }

    /// `GetBound`.
    #[must_use]
    pub const fn bound(&self) -> AABox {
        self.bound
    }

    /// `GetMogpFlags`.
    #[must_use]
    pub const fn mogp_flags(&self) -> u32 {
        self.mogp_flags
    }

    /// `GetWmoID`.
    #[must_use]
    pub const fn wmo_id(&self) -> u32 {
        self.group_wmo_id
    }

    /// Raw vertices (model-local).
    #[must_use]
    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices
    }

    /// Raw triangles (index triples).
    #[must_use]
    pub fn triangles(&self) -> &[MeshTriangle] {
        &self.triangles
    }

    /// `GroupModel::IntersectRay` — returns the number of triangle
    /// hits and shrinks `distance` to the closest.
    #[must_use]
    pub fn intersect_ray(&self, ray: &Ray, distance: &mut f32, stop_at_first: bool) -> u32 {
        if self.triangles.is_empty() {
            return 0;
        }
        let mut hits = 0u32;
        let triangles = &self.triangles;
        let vertices = &self.vertices;
        self.mesh_tree.intersect_ray(
            ray,
            &mut |entry, dist| {
                let tri = triangles[entry as usize];
                let hit = intersect_triangle(
                    vertices[tri.idx0 as usize],
                    vertices[tri.idx1 as usize],
                    vertices[tri.idx2 as usize],
                    ray,
                    dist,
                );
                if hit {
                    hits += 1;
                }
                // C++ GModelRayCallback returns the running hit count
                // (nonzero from the first hit on).
                hits != 0
            },
            distance,
            stop_at_first,
        );
        hits
    }

    /// `GetLiquidLevel`.
    #[must_use]
    pub fn liquid_level(&self, pos: Vec3) -> Option<f32> {
        self.liquid.as_ref().and_then(|l| l.liquid_height(pos))
    }

    /// `GetLiquidType` (0 when the group has no liquid).
    #[must_use]
    pub fn liquid_type(&self) -> u32 {
        self.liquid.as_ref().map_or(0, WmoLiquid::liquid_type)
    }

    /// Serialized mesh BIH (for parity tooling).
    #[must_use]
    pub const fn mesh_tree(&self) -> &Bih {
        &self.mesh_tree
    }
}

/// `MOD_M2` model flag (`ModelInstance.h`).
pub const MOD_M2: u32 = 1;

/// Converted M2/WMO in model-local space (`WorldModel`, `.vmo`).
#[derive(Clone, Debug, Default)]
pub struct WorldModel {
    root_wmo_id: u32,
    group_models: Vec<GroupModel>,
    group_tree: Bih,
    model_flags: u32,
}

impl WorldModel {
    /// Parse a `.vmo` file (`WorldModel::readFile`).
    ///
    /// # Errors
    ///
    /// [`VmapError`] on bad magic/chunks or truncation.
    pub fn parse(data: &[u8]) -> Result<Self, VmapError> {
        let mut r = Reader::new(data);
        r.magic()?;
        r.chunk("WMOD")?;
        let _chunk_size = r.u32()?;
        let root_wmo_id = r.u32()?;
        let mut model = Self {
            root_wmo_id,
            group_models: Vec::new(),
            group_tree: Bih::empty(),
            model_flags: 0,
        };
        // C++ tolerates a missing GMOD (readChunk failure just skips).
        if r.remaining() >= 4 {
            // Peek without consuming on mismatch is not possible with
            // the C++ stream either; it would fail subsequent reads.
            // Match it: attempt the chunk; on mismatch, error.
            r.chunk("GMOD")?;
            let count = r.u32()?;
            let mut groups = Vec::with_capacity(count as usize);
            for _ in 0..count {
                groups.push(GroupModel::read(&mut r)?);
            }
            model.group_models = groups;
            r.chunk("GBIH")?;
            model.group_tree = Bih::read(&mut r)?;
        }
        Ok(model)
    }

    /// `RootWMOID`.
    #[must_use]
    pub const fn root_wmo_id(&self) -> u32 {
        self.root_wmo_id
    }

    /// Group models in file order.
    #[must_use]
    pub fn group_models(&self) -> &[GroupModel] {
        &self.group_models
    }

    /// Serialized group BIH (for parity tooling).
    #[must_use]
    pub const fn group_tree(&self) -> &Bih {
        &self.group_tree
    }

    /// `setModelFlags` (spawn flags are applied after load in C++).
    pub fn set_model_flags(&mut self, flags: u32) {
        self.model_flags = flags;
    }

    /// `getModelFlags`.
    #[must_use]
    pub const fn model_flags(&self) -> u32 {
        self.model_flags
    }

    /// `WorldModel::IntersectRay`.
    #[must_use]
    #[allow(
        clippy::redundant_else,
        clippy::similar_names,
        clippy::too_many_lines,
        clippy::if_not_else,
        clippy::cast_possible_wrap
    )]
    pub fn intersect_ray(
        &self,
        ray: &Ray,
        distance: &mut f32,
        stop_at_first: bool,
        ignore_m2: bool,
    ) -> bool {
        if ignore_m2 && self.model_flags & MOD_M2 != 0 {
            return false;
        }
        if self.group_models.len() == 1 {
            return self.group_models[0].intersect_ray(ray, distance, stop_at_first) != 0;
        }
        let mut hit = false;
        let groups = &self.group_models;
        self.group_tree.intersect_ray(
            ray,
            &mut |entry, dist| {
                if groups[entry as usize].intersect_ray(ray, dist, stop_at_first) != 0 {
                    hit = true;
                }
                hit
            },
            distance,
            stop_at_first,
        );
        hit
    }
}

/// Spawn flags (`ModelInstance.h`): bounds follow in the file.
pub const MOD_HAS_BOUND: u32 = 1 << 2;

/// One model spawn record (`ModelSpawn` as serialized).
#[derive(Clone, Debug)]
pub struct ModelSpawn {
    pub flags: u32,
    pub adt_id: u16,
    pub id: u32,
    pub pos: Vec3,
    pub rot: Vec3,
    pub scale: f32,
    pub bound: Option<AABox>,
    pub name: String,
}

impl ModelSpawn {
    /// `ModelSpawn::readFromFile`; `Ok(None)` at clean EOF (the C++
    /// returns false when the first read gets nothing).
    ///
    /// # Errors
    ///
    /// [`VmapError::Truncated`] on partial records or overlong names.
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Option<Self>, VmapError> {
        if r.remaining() == 0 {
            return Ok(None);
        }
        let flags = r.u32()?;
        let adt_id = r.u16()?;
        let id = r.u32()?;
        let pos = r.vec3()?;
        let rot = r.vec3()?;
        let scale = r.f32()?;
        let bound = if flags & MOD_HAS_BOUND != 0 {
            Some(r.aabox()?)
        } else {
            None
        };
        let name_len = r.u32()?;
        if name_len > 500 {
            return Err(VmapError::Truncated);
        }
        let name = String::from_utf8_lossy(r.take(name_len as usize)?).into_owned();
        Ok(Some(Self {
            flags,
            adt_id,
            id,
            pos,
            rot,
            scale,
            bound,
            name,
        }))
    }
}

/// Parsed `.vmtree` header + instance BIH + global spawns
/// (`StaticMapTree::InitMap` file content; tile loading/query
/// assembly lands with the Phase 3 world layer).
#[derive(Clone, Debug)]
pub struct MapTreeFile {
    pub tiled: bool,
    pub tree: Bih,
    /// `(spawn, referencedVal)` pairs (non-tiled maps only).
    pub global_spawns: Vec<(ModelSpawn, u32)>,
}

impl MapTreeFile {
    /// Parse a `.vmtree` file.
    ///
    /// # Errors
    ///
    /// [`VmapError`] on bad magic/chunks or truncation.
    pub fn parse(data: &[u8]) -> Result<Self, VmapError> {
        let mut r = Reader::new(data);
        r.magic()?;
        let tiled = r.u8()? != 0;
        r.chunk("NODE")?;
        let tree = Bih::read(&mut r)?;
        r.chunk("GOBJ")?;
        let mut global_spawns = Vec::new();
        if !tiled {
            while let Some(spawn) = ModelSpawn::read(&mut r)? {
                let referenced = r.u32()?;
                global_spawns.push((spawn, referenced));
            }
        }
        Ok(Self {
            tiled,
            tree,
            global_spawns,
        })
    }
}

/// Parse a `.vmtile` file into `(spawn, referencedVal)` pairs
/// (`StaticMapTree::LoadMapTile` file content).
///
/// # Errors
///
/// [`VmapError`] on bad magic or truncation.
pub fn parse_tile_file(data: &[u8]) -> Result<Vec<(ModelSpawn, u32)>, VmapError> {
    let mut r = Reader::new(data);
    r.magic()?;
    let num_spawns = r.u32()?;
    let mut out = Vec::with_capacity(num_spawns as usize);
    for _ in 0..num_spawns {
        let spawn = ModelSpawn::read(&mut r)?.ok_or(VmapError::Truncated)?;
        let referenced = r.u32()?;
        out.push((spawn, referenced));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_hit_and_miss() {
        let ray = Ray::new(Vec3::new(0.25, 0.25, 2.0), Vec3::new(0.0, 0.0, -1.0));
        let mut dist = f32::INFINITY;
        assert!(intersect_triangle(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            &ray,
            &mut dist
        ));
        assert_eq!(dist.to_bits(), 2.0f32.to_bits());
        let ray2 = Ray::new(Vec3::new(2.0, 2.0, 2.0), Vec3::new(0.0, 0.0, -1.0));
        let mut dist2 = f32::INFINITY;
        assert!(!intersect_triangle(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            &ray2,
            &mut dist2
        ));
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(
            WorldModel::parse(b"VMAP_6.0rest").unwrap_err(),
            VmapError::BadMagic
        );
        assert_eq!(WorldModel::parse(b"VM").unwrap_err(), VmapError::Truncated);
    }
}
