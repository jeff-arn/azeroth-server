//! `MoveSpline` + flags + init args + fall formulas
//! (`MoveSpline.{h,cpp}`, `MoveSplineFlag.h`, `MoveSplineInitArgs.h`,
//! `util.cpp`). State machine and timestamp computation mirror the
//! C++ float/int conversions exactly (including `util.cpp`'s f64
//! gravity promotions).

use crate::spline::{EvaluationMode, Spline, Vec3};

/// `MoveSplineFlag` — bitfield wrapper around the raw u32.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MoveSplineFlag(pub u32);

impl MoveSplineFlag {
    pub const NONE: u32 = 0x0000_0000;
    pub const DONE: u32 = 0x0000_0001;
    pub const FALLING: u32 = 0x0000_0002;
    pub const RUNMODE: u32 = 0x0000_0100;
    pub const FLYING: u32 = 0x0000_0200;
    pub const NO_SPLINE: u32 = 0x0000_0400;
    pub const FINAL_POINT: u32 = 0x0001_0000;
    pub const FINAL_TARGET: u32 = 0x0002_0000;
    pub const FINAL_ANGLE: u32 = 0x0004_0000;
    pub const CYCLIC: u32 = 0x0010_0000;
    pub const ENTER_CYCLE: u32 = 0x0020_0000;
    pub const FROZEN: u32 = 0x0040_0000;

    pub const MASK_FINAL_FACING: u32 = Self::FINAL_POINT | Self::FINAL_TARGET | Self::FINAL_ANGLE;
    pub const MASK_NO_MONSTER_MOVE: u32 = Self::MASK_FINAL_FACING | Self::DONE;
    pub const MASK_CATMULL_ROM: u32 = Self::FLYING;

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    #[must_use]
    pub const fn done(self) -> bool {
        self.has(Self::DONE)
    }

    #[must_use]
    pub const fn falling(self) -> bool {
        self.has(Self::FALLING)
    }

    #[must_use]
    pub const fn cyclic(self) -> bool {
        self.has(Self::CYCLIC)
    }

    #[must_use]
    pub const fn final_point(self) -> bool {
        self.has(Self::FINAL_POINT)
    }

    #[must_use]
    pub const fn final_target(self) -> bool {
        self.has(Self::FINAL_TARGET)
    }

    #[must_use]
    pub const fn final_angle(self) -> bool {
        self.has(Self::FINAL_ANGLE)
    }

    /// `isSmooth()` — Catmull-Rom interpolation selected.
    #[must_use]
    pub const fn is_smooth(self) -> bool {
        self.0 & Self::MASK_CATMULL_ROM != 0
    }

    /// `isFacing()`.
    #[must_use]
    pub const fn is_facing(self) -> bool {
        self.0 & Self::MASK_FINAL_FACING != 0
    }

    pub fn enable_facing_point(&mut self) {
        self.0 = (self.0 & !Self::MASK_FINAL_FACING) | Self::FINAL_POINT;
    }

    pub fn enable_facing_angle(&mut self) {
        self.0 = (self.0 & !Self::MASK_FINAL_FACING) | Self::FINAL_ANGLE;
    }

    pub fn enable_facing_target(&mut self) {
        self.0 = (self.0 & !Self::MASK_FINAL_FACING) | Self::FINAL_TARGET;
    }
}

impl std::ops::BitAnd<u32> for MoveSplineFlag {
    type Output = u32;
    fn bitand(self, rhs: u32) -> u32 {
        self.0 & rhs
    }
}

/// The `FacingInfo` union, as an enum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FacingInfo {
    Angle(f32),
    Target(u64),
    Point(Vec3),
}

impl Default for FacingInfo {
    fn default() -> Self {
        Self::Angle(0.0)
    }
}

/// `MoveSplineInitArgs` (minus the `Unit*`-bound validation).
#[derive(Clone, Debug)]
pub struct MoveSplineInitArgs {
    pub path: Vec<Vec3>,
    pub facing: FacingInfo,
    pub flags: MoveSplineFlag,
    pub path_idx_offset: i32,
    pub velocity: f32,
    pub spline_id: u32,
    pub transport_guid: u32,
    pub uninterruptible: bool,
}

impl Default for MoveSplineInitArgs {
    fn default() -> Self {
        Self {
            path: Vec::new(),
            facing: FacingInfo::default(),
            flags: MoveSplineFlag::default(),
            path_idx_offset: 0,
            velocity: 0.0,
            spline_id: 0,
            transport_guid: 0,
            uninterruptible: false,
        }
    }
}

/// `Location` (position + orientation).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Location {
    pub pos: Vec3,
    pub orientation: f32,
}

const MINIMAL_DURATION: i32 = 1;

/// `CommonInitializer`.
struct CommonInitializer {
    velocity_inv: f32,
    time: i32,
}

impl CommonInitializer {
    fn next(&mut self, spline: &Spline, i: i32) -> i32 {
        // C++: int32 += float — float addition, truncation back.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        {
            self.time = (self.time as f32 + spline.seg_length(i) * self.velocity_inv) as i32;
        }
        self.time
    }
}

/// Fall physics constants (`util.cpp`; gravity is f64 there).
pub const GRAVITY: f64 = 19.291_105_270_385_74;
pub const TERMINAL_VELOCITY: f32 = 60.148_003;
pub const TERMINAL_SAVEFALL_VELOCITY: f32 = 7.0;

fn terminal_length() -> f32 {
    ((TERMINAL_VELOCITY * TERMINAL_VELOCITY) as f64 / (2.0 * GRAVITY)) as f32
}

fn terminal_savefall_length() -> f32 {
    ((TERMINAL_SAVEFALL_VELOCITY * TERMINAL_SAVEFALL_VELOCITY) as f64 / (2.0 * GRAVITY)) as f32
}

fn terminal_fall_time() -> f32 {
    (f64::from(TERMINAL_VELOCITY) / GRAVITY) as f32
}

/// `computeFallTime`.
#[must_use]
pub fn compute_fall_time(path_length: f32, is_safe_fall: bool) -> f32 {
    if path_length < 0.0 {
        return 0.0;
    }
    if is_safe_fall {
        let tsl = terminal_savefall_length();
        if path_length >= tsl {
            (f64::from((path_length - tsl) / TERMINAL_SAVEFALL_VELOCITY)
                + f64::from(TERMINAL_SAVEFALL_VELOCITY) / GRAVITY) as f32
        } else {
            (((2.0 * path_length) as f64 / GRAVITY) as f32).sqrt()
        }
    } else if path_length >= terminal_length() {
        (path_length - terminal_length()) / TERMINAL_VELOCITY + terminal_fall_time()
    } else {
        (((2.0 * path_length) as f64 / GRAVITY) as f32).sqrt()
    }
}

/// `computeFallElevation(t)` (single-argument overload).
#[must_use]
pub fn compute_fall_elevation(t_passed: f32) -> f32 {
    if t_passed > terminal_fall_time() {
        TERMINAL_VELOCITY * (t_passed - terminal_fall_time()) + terminal_length()
    } else {
        (f64::from(t_passed * t_passed) * GRAVITY * 0.5) as f32
    }
}

/// `computeFallElevation(t, isSafeFall, start_velocity)`.
#[must_use]
pub fn compute_fall_elevation_with_velocity(
    t_passed: f32,
    is_safe_fall: bool,
    mut start_velocity: f32,
) -> f32 {
    let term_vel = if is_safe_fall {
        TERMINAL_SAVEFALL_VELOCITY
    } else {
        TERMINAL_VELOCITY
    };
    if start_velocity > term_vel {
        start_velocity = term_vel;
    }
    let terminal_time =
        (f64::from(terminal_fall_time()) - f64::from(start_velocity) / GRAVITY) as f32;
    if t_passed > terminal_time {
        (f64::from(TERMINAL_VELOCITY * (t_passed - terminal_time) + start_velocity * terminal_time)
            + GRAVITY * f64::from(terminal_time) * f64::from(terminal_time) * 0.5) as f32
    } else {
        (f64::from(t_passed) * (f64::from(start_velocity) + f64::from(t_passed) * GRAVITY * 0.5))
            as f32
    }
}

/// `MSToSec`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn ms_to_sec(ms: i32) -> f32 {
    ms as f32 / 1000.0
}

/// `MoveSpline::UpdateResult`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateResult {
    None = 0x01,
    Arrived = 0x02,
    NextCycle = 0x04,
    NextSegment = 0x08,
}

/// The moving point on a spline (`MoveSpline`).
#[derive(Clone, Debug)]
pub struct MoveSpline {
    spline: Spline,
    facing: FacingInfo,
    id: u32,
    transport_guid: u32,
    flags: MoveSplineFlag,
    time_passed: i32,
    point_idx: i32,
    last_point_sent_idx: i32,
    point_idx_offset: i32,
    uninterruptible: bool,
}

impl Default for MoveSpline {
    fn default() -> Self {
        Self::new()
    }
}

impl MoveSpline {
    /// Default-constructed (done flag set, as in C++).
    #[must_use]
    pub fn new() -> Self {
        Self {
            spline: Spline::new(),
            facing: FacingInfo::default(),
            id: 0,
            transport_guid: 0,
            flags: MoveSplineFlag(MoveSplineFlag::DONE),
            time_passed: 0,
            point_idx: 0,
            last_point_sent_idx: -1,
            point_idx_offset: 0,
            uninterruptible: false,
        }
    }

    /// `Initialize` + `init_spline`.
    pub fn initialize(&mut self, args: &MoveSplineInitArgs) {
        self.flags = args.flags;
        self.facing = args.facing;
        self.id = args.spline_id;
        self.point_idx_offset = args.path_idx_offset;
        self.time_passed = 0;
        self.transport_guid = args.transport_guid;
        self.last_point_sent_idx = -1;
        self.uninterruptible = args.uninterruptible;

        let mode = if args.flags.is_smooth() {
            EvaluationMode::CatmullRom
        } else {
            EvaluationMode::Linear
        };
        if args.flags.cyclic() {
            self.spline.init_cyclic_spline(&args.path, mode, 0);
        } else {
            self.spline.init_spline(&args.path, mode);
        }

        if self.flags.falling() {
            let start_elevation = self.spline.point(self.spline.first()).z;
            self.spline.init_lengths_with(&mut |s, i| {
                #[allow(clippy::cast_possible_truncation)]
                {
                    (compute_fall_time(start_elevation - s.point(i + 1).z, false) * 1000.0) as i32
                }
            });
        } else {
            let mut init = CommonInitializer {
                velocity_inv: 1000.0 / args.velocity,
                time: MINIMAL_DURATION,
            };
            self.spline.init_lengths_with(&mut |s, i| init.next(s, i));
        }

        if self.spline.length() < MINIMAL_DURATION {
            let len = if self.spline.is_cyclic() { 1000 } else { 1 };
            self.spline.set_length(self.spline.last(), len);
        }
        self.point_idx = self.spline.first();
    }

    /// `Initialized()`.
    #[must_use]
    pub fn initialized(&self) -> bool {
        !self.spline.is_empty()
    }

    /// `GetId()`.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// `GetTransportGuid()`.
    #[must_use]
    pub const fn transport_guid(&self) -> u32 {
        self.transport_guid
    }

    /// Current spline flags.
    #[must_use]
    pub const fn flags(&self) -> MoveSplineFlag {
        self.flags
    }

    /// Current facing info.
    #[must_use]
    pub const fn facing(&self) -> FacingInfo {
        self.facing
    }

    /// The underlying spline (`_Spline()`).
    #[must_use]
    pub const fn spline(&self) -> &Spline {
        &self.spline
    }

    /// `Finalized()`.
    #[must_use]
    pub fn finalized(&self) -> bool {
        self.flags.done()
    }

    /// `isCyclic()`.
    #[must_use]
    pub fn is_cyclic(&self) -> bool {
        self.flags.cyclic()
    }

    /// `IsUninterruptible()`.
    #[must_use]
    pub const fn is_uninterruptible(&self) -> bool {
        self.uninterruptible
    }

    /// `Duration()`.
    #[must_use]
    pub fn duration(&self) -> i32 {
        self.spline.length()
    }

    /// `Duration(first, last)`.
    #[must_use]
    pub fn duration_between(&self, first: i32, last: i32) -> i32 {
        self.spline.length_between(first, last)
    }

    /// `timePassed()`.
    #[must_use]
    pub const fn time_passed(&self) -> i32 {
        self.time_passed
    }

    /// `CountSplinePoints()` (returns `spline.last()`, as in C++).
    #[must_use]
    pub fn count_spline_points(&self) -> i32 {
        self.spline.last()
    }

    /// `FinalDestination()`.
    #[must_use]
    pub fn final_destination(&self) -> Vec3 {
        if self.initialized() {
            self.spline.point(self.spline.last())
        } else {
            Vec3::ZERO
        }
    }

    /// `CurrentDestination()`.
    #[must_use]
    pub fn current_destination(&self) -> Vec3 {
        if self.initialized() {
            self.spline.point(self.point_idx + 1)
        } else {
            Vec3::ZERO
        }
    }

    /// `getLastPointSent` / `setLastPointSent` (packet chunking cursor).
    #[must_use]
    pub const fn last_point_sent(&self) -> i32 {
        self.last_point_sent_idx
    }

    pub fn set_last_point_sent(&mut self, i: i32) {
        self.last_point_sent_idx = i;
    }

    fn next_timestamp(&self) -> i32 {
        self.spline.length_at(self.point_idx + 1)
    }

    fn segment_time_elapsed(&self) -> i32 {
        self.next_timestamp() - self.time_passed
    }

    fn finalize(&mut self) {
        self.flags.0 |= MoveSplineFlag::DONE;
        self.point_idx = self.spline.last() - 1;
        self.time_passed = self.duration();
    }

    /// `updateState(difftime)` — C++ is a do-while: `_updateState`
    /// runs at least once even for a zero diff. Returns the last
    /// result (C++ feeds each to a handler / drops it).
    pub fn update_state(&mut self, mut ms_time_diff: i32) -> UpdateResult {
        debug_assert!(self.initialized());
        loop {
            let result = self.update_state_step(&mut ms_time_diff);
            if ms_time_diff <= 0 {
                return result;
            }
        }
    }

    fn update_state_step(&mut self, ms_time_diff: &mut i32) -> UpdateResult {
        if self.finalized() {
            *ms_time_diff = 0;
            return UpdateResult::Arrived;
        }
        let mut result = UpdateResult::None;
        let minimal_diff = (*ms_time_diff).min(self.segment_time_elapsed());
        debug_assert!(minimal_diff >= 0);
        self.time_passed += minimal_diff;
        *ms_time_diff -= minimal_diff;

        if self.time_passed >= self.next_timestamp() {
            self.point_idx += 1;
            if self.point_idx < self.spline.last() {
                result = UpdateResult::NextSegment;
            } else if self.spline.is_cyclic() {
                self.point_idx = self.spline.first();
                self.time_passed %= self.duration();
                result = UpdateResult::NextSegment;
            } else {
                self.finalize();
                *ms_time_diff = 0;
                result = UpdateResult::Arrived;
            }
        }
        result
    }

    /// `computeFallElevation(float&)` — note the C++ ignores the
    /// incoming elevation entirely.
    fn fall_elevation(&self) -> f32 {
        let z_now = self.spline.point(self.spline.first()).z
            - compute_fall_elevation(ms_to_sec(self.time_passed));
        let final_z = self.final_destination().z;
        if z_now < final_z { final_z } else { z_now }
    }

    fn compute_position_at(&self, last_index: i32, desired_time: i32) -> Location {
        debug_assert!(desired_time >= self.spline.length_at(last_index));
        debug_assert!(desired_time <= self.spline.length_at(last_index + 1));
        let seg_time = self.spline.length_between(last_index, last_index + 1);
        #[allow(clippy::cast_precision_loss)]
        let u = if seg_time > 0 {
            (desired_time - self.spline.length_at(last_index)) as f32 / seg_time as f32
        } else {
            1.0
        };
        let mut pos = self.spline.evaluate_percent_at(last_index, u);
        if self.flags.falling() {
            pos.z = self.fall_elevation();
        }
        // C++: done+facing Target leaves orientation at the
        // Location default (0); only Angle/Point set it.
        let orientation = if self.flags.done() && self.flags.is_facing() {
            match self.facing {
                FacingInfo::Angle(a) if self.flags.final_angle() => a,
                FacingInfo::Point(f) if self.flags.final_point() => {
                    (f.y - pos.y).atan2(f.x - pos.x)
                }
                _ => 0.0,
            }
        } else {
            let hermite = self.spline.evaluate_derivative_at(self.point_idx, u);
            hermite.y.atan2(hermite.x)
        };
        Location { pos, orientation }
    }

    /// `ComputePosition()`.
    #[must_use]
    pub fn compute_position(&self) -> Location {
        debug_assert!(self.initialized());
        self.compute_position_at(self.point_idx, self.time_passed)
    }

    /// `ComputePositionAfterTime(duration)`.
    #[must_use]
    pub fn compute_position_after_time(&self, duration: i32) -> Location {
        debug_assert!(self.initialized());
        let mut last_index = self.point_idx;
        let time_passed = self.time_passed.max(self.spline.length_at(last_index));
        for i in (self.point_idx + 1)..=self.spline.last() {
            if self.spline.length_at(i) - time_passed > duration {
                break;
            }
            last_index = i;
        }
        if last_index == self.spline.last() {
            return Location {
                pos: self.final_destination(),
                orientation: 0.0,
            };
        }
        self.compute_position_at(last_index, duration + time_passed)
    }

    /// `currentPathIdx()`.
    #[must_use]
    pub fn current_path_idx(&self) -> i32 {
        let mut point = self.point_idx_offset + self.point_idx - self.spline.first()
            + i32::from(self.finalized());
        if self.is_cyclic() {
            point %= self.spline.last() - self.spline.first();
        }
        point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_args() -> MoveSplineInitArgs {
        MoveSplineInitArgs {
            path: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(20.0, 0.0, 0.0),
            ],
            velocity: 10.0,
            spline_id: 7,
            ..MoveSplineInitArgs::default()
        }
    }

    #[test]
    fn common_initializer_timestamps() {
        // velocityInv = 100; time starts at 1; segs are 10 long each:
        // timestamps: [0, 0(lo=1 default), 1+1000, 1+2000] at idx 2,3.
        let mut ms = MoveSpline::new();
        ms.initialize(&linear_args());
        assert_eq!(ms.duration(), 2001);
        assert_eq!(ms.spline().length_at(2), 1001);
        assert_eq!(ms.spline().length_at(1), 0);
    }

    #[test]
    fn update_arrives_and_finalizes() {
        let mut ms = MoveSpline::new();
        ms.initialize(&linear_args());
        assert_eq!(ms.update_state(1001), UpdateResult::NextSegment);
        assert_eq!(ms.update_state(5000), UpdateResult::Arrived);
        assert!(ms.finalized());
        assert_eq!(ms.time_passed(), 2001);
    }

    #[test]
    fn fall_time_below_terminal() {
        // sqrt(2*10/19.29110527038574) in f32.
        let expected = ((20.0f64 / GRAVITY) as f32).sqrt();
        assert_eq!(compute_fall_time(10.0, false).to_bits(), expected.to_bits());
        assert_eq!(compute_fall_time(-1.0, false).to_bits(), 0.0f32.to_bits());
    }
}
