//! `CMSG_PING` overspeed accounting
//! (`WorldSocket::_HandlePing`, `WorldSocket.cpp:442-503`).
//!
//! The first ping only stamps the clock. Each later ping arriving
//! less than 27 s after the previous one increments the overspeed
//! counter; a gap of 27 s or more resets it. When the counter
//! exceeds a non-zero `MaxOverspeedPings` and the session is a
//! plain player, the C++ kicks. A ping without a session fails
//! regardless (handled by the caller).

use std::time::{Duration, Instant};

use crate::security;

/// Minimum well-behaved ping interval before a ping is "overspeed"
/// (`seconds.count() < 27`, `WorldSocket.cpp:458`).
pub const OVERSPEED_INTERVAL: Duration = Duration::from_secs(27);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PingAction {
    #[default]
    Pong,
    Kick,
}

#[derive(Debug, Default)]
pub struct PingTracker {
    last_ping: Option<Instant>,
    overspeed: u32,
}

impl PingTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one ping at `now` and decide its fate. `security`
    /// is the session's level; the kick applies only to
    /// `SEC_PLAYER` (`WorldSocket.cpp:470`).
    pub fn observe(&mut self, now: Instant, max_overspeed_pings: u32, security: u8) -> PingAction {
        let Some(previous) = self.last_ping.replace(now) else {
            return PingAction::Pong;
        };
        // C++ truncates the gap to whole seconds before comparing.
        let gap_seconds = now.saturating_duration_since(previous).as_secs();
        if gap_seconds < OVERSPEED_INTERVAL.as_secs() {
            self.overspeed += 1;
            if max_overspeed_pings != 0
                && self.overspeed > max_overspeed_pings
                && security == security::PLAYER
            {
                return PingAction::Kick;
            }
        } else {
            self.overspeed = 0;
        }
        PingAction::Pong
    }

    #[must_use]
    pub fn overspeed_count(&self) -> u32 {
        self.overspeed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_ping_never_kicks() {
        let mut tracker = PingTracker::new();
        let t0 = Instant::now();
        assert_eq!(tracker.observe(t0, 2, 0), PingAction::Pong);
    }

    #[test]
    fn overspeed_kick_after_limit() {
        let mut tracker = PingTracker::new();
        let t0 = Instant::now();
        assert_eq!(tracker.observe(t0, 2, 0), PingAction::Pong);
        assert_eq!(
            tracker.observe(t0 + Duration::from_secs(10), 2, 0),
            PingAction::Pong
        );
        assert_eq!(
            tracker.observe(t0 + Duration::from_secs(20), 2, 0),
            PingAction::Pong
        );
        assert_eq!(
            tracker.observe(t0 + Duration::from_secs(30), 2, 0),
            PingAction::Kick
        );
    }

    #[test]
    fn slow_gap_resets_and_staff_never_kicked() {
        let mut tracker = PingTracker::new();
        let t0 = Instant::now();
        tracker.observe(t0, 2, 0);
        tracker.observe(t0 + Duration::from_secs(5), 2, 0);
        assert_eq!(tracker.overspeed_count(), 1);
        tracker.observe(t0 + Duration::from_secs(40), 2, 0);
        assert_eq!(tracker.overspeed_count(), 0);

        let mut staff = PingTracker::new();
        staff.observe(t0, 2, 3);
        for i in 1..6 {
            assert_eq!(
                staff.observe(t0 + Duration::from_secs(i), 2, 3),
                PingAction::Pong
            );
        }
    }

    #[test]
    fn zero_limit_disables_kick() {
        let mut tracker = PingTracker::new();
        let t0 = Instant::now();
        tracker.observe(t0, 0, 0);
        for i in 1..6 {
            assert_eq!(
                tracker.observe(t0 + Duration::from_secs(i), 0, 0),
                PingAction::Pong
            );
        }
        assert_eq!(tracker.overspeed_count(), 5);
    }
}
