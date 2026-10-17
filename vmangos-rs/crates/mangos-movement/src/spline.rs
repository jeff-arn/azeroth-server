//! `SplineBase` + `Spline<int32>` (`vmangos/src/game/Movement/spline/`):
//! control-point storage with virtual end points, per-mode evaluation
//! (linear / Catmull-Rom / cubic Bezier), derivatives, segment lengths,
//! and cumulative length arrays. All arithmetic mirrors the C++ f32
//! operation order (G3D `Vector3`/`Vector4`/`Matrix4` semantics),
//! including `SegLengthCatmullRom`'s double accumulator over f32
//! step lengths.

/// 3-component f32 vector (G3D `Vector3` subset, same op order).
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
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// `Vector3::length`.
    #[must_use]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// `Vector3::lerp`: `this + (v - this) * alpha`.
    #[must_use]
    pub fn lerp(self, v: Self, alpha: f32) -> Self {
        self + (v - self) * alpha
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

/// `SplineBase::EvaluationMode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationMode {
    Linear,
    CatmullRom,
    Bezier3Unused,
    Uninitialized,
}

const STEPS_PER_SEGMENT: i32 = 3;

type Index = i32;

/// Catmull-Rom coefficient matrix, row-major (`s_catmullRomCoeffs`).
const CATMULL_ROM_COEFFS: [[f32; 4]; 4] = [
    [-0.5, 1.5, -1.5, 0.5],
    [1.0, -2.5, 2.0, -0.5],
    [-0.5, 0.0, 0.5, 0.0],
    [0.0, 1.0, 0.0, 0.0],
];

/// Cubic Bezier coefficient matrix, row-major (`s_Bezier3Coeffs`).
const BEZIER3_COEFFS: [[f32; 4]; 4] = [
    [-1.0, 3.0, -3.0, 1.0],
    [3.0, -6.0, 3.0, 0.0],
    [-3.0, 3.0, 0.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
];

/// `C_Evaluate`: weights = tvec * M (row vector), then the weighted
/// sum of the four vertices, in G3D accumulation order.
fn c_evaluate(verts: &[Vec3], t: f32, m: &[[f32; 4]; 4]) -> Vec3 {
    let tvec = [t * t * t, t * t, t, 1.0f32];
    let mut weights = [0.0f32; 4];
    for (i, w) in weights.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (j, tv) in tvec.iter().enumerate() {
            acc += *tv * m[j][i];
        }
        *w = acc;
    }
    verts[0] * weights[0] + verts[1] * weights[1] + verts[2] * weights[2] + verts[3] * weights[3]
}

/// `C_Evaluate_Derivative`.
fn c_evaluate_derivative(verts: &[Vec3], t: f32, m: &[[f32; 4]; 4]) -> Vec3 {
    let tvec = [3.0 * t * t, 2.0 * t, 1.0f32, 0.0f32];
    let mut weights = [0.0f32; 4];
    for (i, w) in weights.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (j, tv) in tvec.iter().enumerate() {
            acc += *tv * m[j][i];
        }
        *w = acc;
    }
    verts[0] * weights[0] + verts[1] * weights[1] + verts[2] * weights[2] + verts[3] * weights[3]
}

/// The spline with its cumulative i32 length array
/// (`SplineBase` + `Spline<int32>` fused).
#[derive(Clone, Debug)]
pub struct Spline {
    points: Vec<Vec3>,
    index_lo: Index,
    index_hi: Index,
    mode: EvaluationMode,
    cyclic: bool,
    lengths: Vec<i32>,
}

impl Default for Spline {
    fn default() -> Self {
        Self::new()
    }
}

impl Spline {
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            index_lo: 0,
            index_hi: 0,
            mode: EvaluationMode::Uninitialized,
            cyclic: false,
            lengths: Vec::new(),
        }
    }

    /// `init_spline`.
    ///
    /// # Panics
    ///
    /// Panics if `controls` has fewer than 2 points (the C++
    /// indexes `controls[1]` when building virtual end points).
    pub fn init_spline(&mut self, controls: &[Vec3], mode: EvaluationMode) {
        self.mode = mode;
        self.cyclic = false;
        self.initialize(controls, false, 0);
    }

    /// `init_cyclic_spline`.
    pub fn init_cyclic_spline(
        &mut self,
        controls: &[Vec3],
        mode: EvaluationMode,
        cyclic_point: Index,
    ) {
        self.mode = mode;
        self.cyclic = true;
        self.initialize(controls, true, cyclic_point);
    }

    fn initialize(&mut self, controls: &[Vec3], cyclic: bool, cyclic_point: Index) {
        match self.mode {
            // initializers[ModeLinear] is InitCatmullRom by design.
            EvaluationMode::Linear | EvaluationMode::CatmullRom => {
                self.init_catmull_rom(controls, cyclic, cyclic_point);
            }
            EvaluationMode::Bezier3Unused => self.init_bezier3(controls),
            EvaluationMode::Uninitialized => panic!("UninitializedSpline"),
        }
        self.lengths.clear();
    }

    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    fn init_catmull_rom(&mut self, controls: &[Vec3], cyclic: bool, cyclic_point: Index) {
        let count = controls.len() as Index;
        let real_size = count + if cyclic { 1 + 2 } else { 1 + 1 };
        self.points = vec![Vec3::ZERO; real_size as usize];
        let lo_index = 1;
        let high_index = lo_index + count - 1;
        self.points[lo_index as usize..(lo_index + count) as usize].copy_from_slice(controls);
        if cyclic {
            if cyclic_point == 0 {
                self.points[0] = controls[(count - 1) as usize];
            } else {
                self.points[0] = controls[0].lerp(controls[1], -1.0);
            }
            self.points[(high_index + 1) as usize] = controls[cyclic_point as usize];
            self.points[(high_index + 2) as usize] = controls[(cyclic_point + 1) as usize];
        } else {
            self.points[0] = controls[0].lerp(controls[1], -1.0);
            self.points[(high_index + 1) as usize] = controls[(count - 1) as usize];
        }
        self.index_lo = lo_index;
        self.index_hi = high_index + i32::from(cyclic);
    }

    fn init_bezier3(&mut self, controls: &[Vec3]) {
        let c = controls.len() / 3 * 3;
        let t = c / 3;
        self.points = controls[..c].to_vec();
        self.index_lo = 0;
        #[allow(clippy::cast_possible_wrap)]
        {
            self.index_hi = (t - 1) as Index;
        }
    }

    /// `first()`.
    #[must_use]
    pub const fn first(&self) -> i32 {
        self.index_lo
    }

    /// `last()`.
    #[must_use]
    pub const fn last(&self) -> i32 {
        self.index_hi
    }

    /// `empty()`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.index_lo == self.index_hi
    }

    /// `mode()`.
    #[must_use]
    pub const fn mode(&self) -> EvaluationMode {
        self.mode
    }

    /// `isCyclic()`.
    #[must_use]
    pub const fn is_cyclic(&self) -> bool {
        self.cyclic
    }

    /// `getPoints()` (storage incl. virtual end points).
    #[must_use]
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// `getPointCount()`.
    #[must_use]
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    pub fn point_count(&self) -> i32 {
        self.points.len() as i32
    }

    /// `getPoint(i)` — no bounds check in C++; panics here on misuse.
    #[must_use]
    pub fn point(&self, i: i32) -> Vec3 {
        self.points[i as usize]
    }

    /// `evaluate_percent(Idx, u)`.
    ///
    /// # Panics
    ///
    /// Panics on an uninitialized spline or out-of-range index,
    /// matching the C++ assert.
    #[must_use]
    pub fn evaluate_percent_at(&self, index: i32, u: f32) -> Vec3 {
        match self.mode {
            EvaluationMode::Linear => {
                let p = self.point(index);
                p + (self.point(index + 1) - p) * u
            }
            EvaluationMode::CatmullRom => {
                c_evaluate(&self.points[(index - 1) as usize..], u, &CATMULL_ROM_COEFFS)
            }
            EvaluationMode::Bezier3Unused => {
                c_evaluate(&self.points[(index * 3) as usize..], u, &BEZIER3_COEFFS)
            }
            EvaluationMode::Uninitialized => panic!("UninitializedSpline"),
        }
    }

    /// `evaluate_derivative(Idx, u)`.
    ///
    /// # Panics
    ///
    /// Panics on an uninitialized spline or out-of-range index,
    /// matching the C++ assert.
    #[must_use]
    pub fn evaluate_derivative_at(&self, index: i32, u: f32) -> Vec3 {
        match self.mode {
            EvaluationMode::Linear => self.point(index + 1) - self.point(index),
            EvaluationMode::CatmullRom => {
                c_evaluate_derivative(&self.points[(index - 1) as usize..], u, &CATMULL_ROM_COEFFS)
            }
            EvaluationMode::Bezier3Unused => {
                c_evaluate_derivative(&self.points[(index * 3) as usize..], u, &BEZIER3_COEFFS)
            }
            EvaluationMode::Uninitialized => panic!("UninitializedSpline"),
        }
    }

    /// `SegLength(i)`.
    ///
    /// # Panics
    ///
    /// Panics on an uninitialized spline or out-of-range index,
    /// matching the C++ assert.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn seg_length(&self, index: i32) -> f32 {
        match self.mode {
            EvaluationMode::Linear => (self.point(index) - self.point(index + 1)).length(),
            EvaluationMode::CatmullRom => self.seg_length_sampled(index - 1, true),
            EvaluationMode::Bezier3Unused => self.seg_length_sampled(index * 3, false),
            EvaluationMode::Uninitialized => panic!("UninitializedSpline"),
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn seg_length_sampled(&self, base: i32, catmull: bool) -> f32 {
        let coeffs = if catmull {
            &CATMULL_ROM_COEFFS
        } else {
            &BEZIER3_COEFFS
        };
        let p = &self.points[base as usize..];
        let mut cur_pos = if catmull {
            p[1]
        } else {
            c_evaluate(p, 0.0, coeffs)
        };
        let mut length: f64 = 0.0;
        let mut i = 1;
        while i <= STEPS_PER_SEGMENT {
            let next_pos = c_evaluate(p, i as f32 / STEPS_PER_SEGMENT as f32, coeffs);
            length += f64::from((next_pos - cur_pos).length());
            cur_pos = next_pos;
            i += 1;
        }
        length as f32
    }

    /// `initLengths()` (arc-length accumulation via [`Spline::seg_length`]).
    pub fn init_lengths(&mut self) {
        let mut i = self.index_lo;
        let mut length: i32 = 0;
        self.lengths = vec![0; (self.index_hi + 1) as usize];
        while i < self.index_hi {
            // C++: int32 += float — float addition, truncation back.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            {
                length = (length as f32 + self.seg_length(i)) as i32;
            }
            i += 1;
            self.lengths[i as usize] = length;
        }
    }

    /// `initLengths(cacher)`: fill timestamps from a callable
    /// invoked as `cacher(spline, i)` for `i` in `[first, last)`,
    /// with the C++ negative-overflow clamp to `i32::MAX`.
    pub fn init_lengths_with(&mut self, cacher: &mut dyn FnMut(&Self, i32) -> i32) {
        let mut i = self.index_lo;
        self.lengths = vec![0; (self.index_hi + 1) as usize];
        while i < self.index_hi {
            let mut new_length = cacher(self, i);
            if new_length < 0 {
                new_length = i32::MAX;
            }
            i += 1;
            self.lengths[i as usize] = new_length;
        }
    }

    /// `length()` — total.
    #[must_use]
    pub fn length(&self) -> i32 {
        self.lengths[self.index_hi as usize]
    }

    /// `length(Idx)` — cumulative length at a node.
    #[must_use]
    pub fn length_at(&self, idx: i32) -> i32 {
        self.lengths[idx as usize]
    }

    /// `length(first, last)`.
    #[must_use]
    pub fn length_between(&self, first: i32, last: i32) -> i32 {
        self.lengths[last as usize] - self.lengths[first as usize]
    }

    /// `set_length`.
    pub fn set_length(&mut self, i: i32, length: i32) {
        self.lengths[i as usize] = length;
    }

    fn compute_index_in_bounds_len(&self, length: i32) -> i32 {
        let mut i = self.index_lo;
        let n = self.index_hi;
        while i + 1 < n && self.lengths[(i + 1) as usize] < length {
            i += 1;
        }
        i
    }

    /// `computeIndex(t)`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn compute_index(&self, t: f32) -> (i32, f32) {
        debug_assert!((0.0..=1.0).contains(&t));
        let length = (t * self.length() as f32) as i32;
        let index = self.compute_index_in_bounds_len(length);
        let u =
            (length - self.length_at(index)) as f32 / self.length_between(index, index + 1) as f32;
        (index, u)
    }

    /// `computeIndexInBounds(t)`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn compute_index_in_bounds(&self, t: f32) -> i32 {
        debug_assert!((0.0..=1.0).contains(&t));
        self.compute_index_in_bounds_len((t * self.length() as f32) as i32)
    }

    /// `evaluate_percent(t)` over the whole spline.
    #[must_use]
    pub fn evaluate_percent(&self, t: f32) -> Vec3 {
        let (index, u) = self.compute_index(t);
        self.evaluate_percent_at(index, u)
    }

    /// `evaluate_derivative(t)` over the whole spline.
    #[must_use]
    pub fn evaluate_derivative(&self, t: f32) -> Vec3 {
        let (index, u) = self.compute_index(t);
        self.evaluate_derivative_at(index, u)
    }

    /// `clear`.
    pub fn clear(&mut self) {
        self.index_lo = 0;
        self.index_hi = 0;
        self.points.clear();
        self.lengths.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(v: Vec3) -> (u32, u32, u32) {
        (v.x.to_bits(), v.y.to_bits(), v.z.to_bits())
    }

    #[test]
    fn catmull_rom_initializer_virtual_points() {
        let controls = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 5.0, 0.0),
        ];
        let mut s = Spline::new();
        s.init_spline(&controls, EvaluationMode::CatmullRom);
        assert_eq!(s.first(), 1);
        assert_eq!(s.last(), 3);
        assert_eq!(s.point_count(), 5);
        // virtual first = p0.lerp(p1, -1) = (-10, 0, 0); last = p2
        assert_eq!(s.point(0), Vec3::new(-10.0, 0.0, 0.0));
        assert_eq!(s.point(4), controls[2]);
        // evaluation at u=0 of first segment is the first control point
        assert_eq!(bits(s.evaluate_percent_at(1, 0.0)), bits(controls[0]));
    }

    #[test]
    fn linear_mode_uses_catmull_initializer() {
        let controls = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(3.0, 4.0, 0.0)];
        let mut s = Spline::new();
        s.init_spline(&controls, EvaluationMode::Linear);
        assert_eq!(s.first(), 1);
        // segment length is the euclidean 5
        assert_eq!(s.seg_length(1).to_bits(), 5.0f32.to_bits());
    }
}
