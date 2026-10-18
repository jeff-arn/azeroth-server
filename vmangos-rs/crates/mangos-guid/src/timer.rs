//! The tick timers from `vmangos/src/shared/Timer.h:73-150`,
//! ported 1:1 per the Phase 4 tick contract: `Update(diff)`
//! accumulates (or counts down), `Passed()` is a plain
//! comparison, `Reset()` subtracts **one** interval, remainder
//! preserved. Interval 0 always passes. `diff` is real elapsed
//! milliseconds, never an assumed tick length.
//!
//! Signed (`i64`) forms mirror C++ `time_t` arithmetic; the
//! `Update` negative-clamp in [`IntervalTimer`] is part of the
//! original body, kept exactly.

/// `class IntervalTimer` (`Timer.h:73-98`): counts up.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IntervalTimer {
    interval: i64,
    current: i64,
}

impl IntervalTimer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, diff: i64) {
        self.current += diff;
        if self.current < 0 {
            self.current = 0;
        }
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.current >= self.interval
    }

    /// Subtracts one interval if passed; does nothing
    /// otherwise.
    pub fn reset(&mut self) {
        if self.current >= self.interval {
            self.current -= self.interval;
        }
    }

    pub fn set_current(&mut self, current: i64) {
        self.current = current;
    }

    pub fn set_interval(&mut self, interval: i64) {
        self.interval = interval;
    }

    #[must_use]
    pub fn interval(&self) -> i64 {
        self.interval
    }

    #[must_use]
    pub fn current(&self) -> i64 {
        self.current
    }
}

/// `class ShortIntervalTimer` (`Timer.h:100-123`): `u32` counts
/// up, wrapping add like C++ unsigned arithmetic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShortIntervalTimer {
    interval: u32,
    current: u32,
}

impl ShortIntervalTimer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, diff: u32) {
        self.current = self.current.wrapping_add(diff);
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.current >= self.interval
    }

    pub fn reset(&mut self) {
        if self.current >= self.interval {
            self.current -= self.interval;
        }
    }

    pub fn set_current(&mut self, current: u32) {
        self.current = current;
    }

    pub fn set_interval(&mut self, interval: u32) {
        self.interval = interval;
    }

    #[must_use]
    pub fn interval(&self) -> u32 {
        self.interval
    }

    #[must_use]
    pub fn current(&self) -> u32 {
        self.current
    }
}

/// `struct TimeTracker` (`Timer.h:125-136`): counts down to
/// expiry; `Passed()` at `<= 0`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimeTracker {
    expiry: i64,
}

impl TimeTracker {
    #[must_use]
    pub fn new(expiry: i64) -> Self {
        Self { expiry }
    }

    pub fn update(&mut self, diff: i64) {
        self.expiry -= diff;
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.expiry <= 0
    }

    pub fn reset(&mut self, interval: i64) {
        self.expiry = interval;
    }

    #[must_use]
    pub fn expiry(&self) -> i64 {
        self.expiry
    }
}

/// `struct ShortTimeTracker` (`Timer.h:138-150`): `i32` count
/// down.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShortTimeTracker {
    expiry: i32,
}

impl ShortTimeTracker {
    #[must_use]
    pub fn new(expiry: i32) -> Self {
        Self { expiry }
    }

    pub fn update(&mut self, diff: i32) {
        self.expiry -= diff;
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.expiry <= 0
    }

    pub fn reset(&mut self, interval: i32) {
        self.expiry = interval;
    }

    #[must_use]
    pub fn expiry(&self) -> i32 {
        self.expiry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_zero_always_passes() {
        let t = IntervalTimer::new();
        assert!(t.passed());
        assert!(ShortIntervalTimer::new().passed());
    }

    #[test]
    fn interval_reset_subtracts_one_interval_only() {
        let mut t = IntervalTimer::new();
        t.set_interval(100);
        t.update(250);
        assert!(t.passed());
        t.reset();
        assert_eq!(t.current(), 150);
        assert!(t.passed(), "still passed after one subtraction");
        t.reset();
        assert_eq!(t.current(), 50);
        assert!(!t.passed());
        t.reset(); // no-op when not passed
        assert_eq!(t.current(), 50);
    }

    #[test]
    fn interval_negative_diff_clamps_at_zero() {
        let mut t = IntervalTimer::new();
        t.set_interval(100);
        t.update(40);
        t.update(-90);
        assert_eq!(t.current(), 0);
    }

    #[test]
    fn short_interval_same_shape() {
        let mut t = ShortIntervalTimer::new();
        t.set_interval(500); // EventAI poll shape
        t.update(499);
        assert!(!t.passed());
        t.update(1);
        assert!(t.passed());
        t.reset();
        assert_eq!(t.current(), 0);
    }

    #[test]
    fn tracker_counts_down_past_zero() {
        let mut t = TimeTracker::new(100);
        assert!(!t.passed());
        t.update(100);
        assert!(t.passed());
        t.update(50);
        assert_eq!(t.expiry(), -50);
        t.reset(200);
        assert_eq!(t.expiry(), 200);
        assert!(!t.passed());
    }

    #[test]
    fn short_tracker_default_passed() {
        assert!(ShortTimeTracker::default().passed());
        let mut t = ShortTimeTracker::new(10);
        t.update(9);
        assert!(!t.passed());
        t.update(1);
        assert!(t.passed());
    }
}
