//! `BreadcrumbTrail` (`PlayerBots/BotFormation.{h,cpp}`): ring
//! buffer of leader path waypoints with arc-length lookups.

/// One recorded leader position (`BreadcrumbPoint`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BreadcrumbPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
    pub timestamp: u32,
}

fn dist_sq(a: (f32, f32, f32), b: &BreadcrumbPoint) -> f32 {
    let dx = a.0 - b.x;
    let dy = a.1 - b.y;
    let dz = a.2 - b.z;
    dx * dx + dy * dy + dz * dz
}

fn distance(a: &BreadcrumbPoint, b: &BreadcrumbPoint) -> f32 {
    dist_sq((a.x, a.y, a.z), b).sqrt()
}

/// `Lerp`: orientation taken from `a`; timestamp is 0 (the C++
/// leaves the default in place).
fn lerp(a: &BreadcrumbPoint, b: &BreadcrumbPoint, t: f32) -> BreadcrumbPoint {
    BreadcrumbPoint {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
        orientation: a.orientation,
        timestamp: 0,
    }
}

/// Ring buffer of 40 leader breadcrumbs, sampled at most once a
/// second and only after ≥2 yd of movement.
#[derive(Clone, Debug)]
pub struct BreadcrumbTrail {
    points: [BreadcrumbPoint; Self::MAX_CRUMBS as usize],
    head: u32,
    count: u32,
    last_record_time: u32,
}

impl Default for BreadcrumbTrail {
    fn default() -> Self {
        Self::new()
    }
}

impl BreadcrumbTrail {
    /// `MAX_CRUMBS`.
    pub const MAX_CRUMBS: u32 = 40;
    /// `MIN_RECORD_DIST_SQ` (2 yd minimum movement, squared).
    pub const MIN_RECORD_DIST_SQ: f32 = 4.0;
    /// `RECORD_INTERVAL_MS` (match bot tick rate).
    pub const RECORD_INTERVAL_MS: u32 = 1000;

    #[must_use]
    pub fn new() -> Self {
        Self {
            points: [BreadcrumbPoint::default(); Self::MAX_CRUMBS as usize],
            head: 0,
            count: 0,
            last_record_time: 0,
        }
    }

    /// Raw ring index for "steps back from head".
    fn ring_index(&self, steps_back: u32) -> usize {
        ((self.head + Self::MAX_CRUMBS - 1 - steps_back) % Self::MAX_CRUMBS) as usize
    }

    fn at(&self, steps_back: u32) -> BreadcrumbPoint {
        self.points[self.ring_index(steps_back)]
    }

    /// `RecordPoint`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_point(&mut self, x: f32, y: f32, z: f32, o: f32, now: u32) {
        if now.wrapping_sub(self.last_record_time) < Self::RECORD_INTERVAL_MS {
            return;
        }
        if self.count > 0 {
            let last = self.at(0);
            if dist_sq((x, y, z), &last) < Self::MIN_RECORD_DIST_SQ {
                return;
            }
        }
        self.points[self.head as usize] = BreadcrumbPoint {
            x,
            y,
            z,
            orientation: o,
            timestamp: now,
        };
        self.head = (self.head + 1) % Self::MAX_CRUMBS;
        if self.count < Self::MAX_CRUMBS {
            self.count += 1;
        }
        self.last_record_time = now;
    }

    /// `Clear`.
    pub fn clear(&mut self) {
        self.head = 0;
        self.count = 0;
        self.last_record_time = 0;
    }

    /// `Size()`.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.count
    }

    /// `IsEmpty()`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `GetPoint(offsetFromHead)`: 0 = newest.
    #[must_use]
    pub fn get_point(&self, offset_from_head: u32) -> Option<BreadcrumbPoint> {
        if offset_from_head >= self.count {
            return None;
        }
        Some(self.at(offset_from_head))
    }

    /// `GetPointAtDistance`: walk the trail backward by
    /// arc-length and interpolate to the exact distance. A trail
    /// shorter than the distance yields the oldest point.
    #[must_use]
    pub fn get_point_at_distance(&self, target_dist: f32) -> Option<BreadcrumbPoint> {
        if self.count == 0 {
            return None;
        }
        if self.count == 1 || target_dist <= 0.0 {
            return Some(self.at(0));
        }
        Some(self.walk_back(0, target_dist))
    }

    /// `GetPointFurtherBack`: from `start_index`, walk further
    /// back by `additional_dist` yards.
    #[must_use]
    pub fn get_point_further_back(
        &self,
        start_index: u32,
        additional_dist: f32,
    ) -> Option<BreadcrumbPoint> {
        if start_index >= self.count {
            return None;
        }
        if additional_dist <= 0.0 {
            return Some(self.at(start_index));
        }
        Some(self.walk_back(start_index, additional_dist))
    }

    fn walk_back(&self, start: u32, dist: f32) -> BreadcrumbPoint {
        let mut accum = 0.0f32;
        let mut i = start;
        while i < self.count - 1 {
            let curr = self.at(i);
            let next = self.at(i + 1);
            let seg_len = distance(&curr, &next);
            if accum + seg_len >= dist {
                let t = if seg_len > 0.001 {
                    (dist - accum) / seg_len
                } else {
                    0.0
                };
                return lerp(&curr, &next, t);
            }
            accum += seg_len;
            i += 1;
        }
        // Trail is shorter than the distance — oldest point.
        self.at(self.count - 1)
    }

    /// `FindNearestIndex`: offset-from-head of the closest point
    /// (first wins on ties; 0 for an empty trail, as in C++).
    #[must_use]
    pub fn find_nearest_index(&self, x: f32, y: f32, z: f32) -> u32 {
        let mut best = 0u32;
        let mut best_sq = f32::MAX;
        for i in 0..self.count {
            let sq = dist_sq((x, y, z), &self.at(i));
            if sq < best_sq {
                best_sq = sq;
                best = i;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records a straight +x trail: points at x = 0, 3, 6, ... (3 yd apart).
    #[allow(clippy::cast_precision_loss)]
    fn straight_trail(n: u32) -> BreadcrumbTrail {
        let mut t = BreadcrumbTrail::new();
        for i in 0..n {
            t.record_point(3.0 * i as f32, 0.0, 0.0, 0.0, 1000 * (i + 1));
        }
        t
    }

    #[test]
    fn record_gates_on_interval_and_distance() {
        let mut t = BreadcrumbTrail::new();
        t.record_point(0.0, 0.0, 0.0, 0.0, 1000);
        assert_eq!(t.size(), 1);
        // too soon
        t.record_point(10.0, 0.0, 0.0, 0.0, 1999);
        assert_eq!(t.size(), 1);
        // on time but only 1 yd moved
        t.record_point(1.0, 0.0, 0.0, 0.0, 2000);
        assert_eq!(t.size(), 1);
        // exactly 2 yd: distSq == 4.0 is NOT < 4.0, so it records
        t.record_point(2.0, 0.0, 0.0, 0.0, 2000);
        assert_eq!(t.size(), 2);
        assert_eq!(t.get_point(0).unwrap().x.to_bits(), 2.0f32.to_bits());
        assert_eq!(t.get_point(1).unwrap().x.to_bits(), 0.0f32.to_bits());
        assert!(t.get_point(2).is_none());
    }

    #[test]
    fn wrap_keeps_newest_order() {
        let t = straight_trail(45); // 5 overwritten
        assert_eq!(t.size(), 40);
        assert_eq!(t.get_point(0).unwrap().x.to_bits(), 132.0f32.to_bits()); // 3*44
        assert_eq!(t.get_point(39).unwrap().x.to_bits(), 15.0f32.to_bits()); // 3*5
    }

    #[test]
    fn point_at_distance_interpolates_and_clamps() {
        let t = straight_trail(6); // newest x=15, 15 yd of trail
        assert_eq!(
            t.get_point_at_distance(0.0).unwrap().x.to_bits(),
            15.0f32.to_bits()
        );
        assert_eq!(
            t.get_point_at_distance(3.0).unwrap().x.to_bits(),
            12.0f32.to_bits()
        );
        // midpoint of segment 15 -> 12
        assert_eq!(
            t.get_point_at_distance(1.5).unwrap().x.to_bits(),
            13.5f32.to_bits()
        );
        // beyond the trail: oldest point
        assert_eq!(
            t.get_point_at_distance(100.0).unwrap().x.to_bits(),
            0.0f32.to_bits()
        );
        // further back from index 1 (x=12) by 6 yd -> x=6
        assert_eq!(
            t.get_point_further_back(1, 6.0).unwrap().x.to_bits(),
            6.0f32.to_bits()
        );
        assert!(t.get_point_further_back(6, 1.0).is_none());
        assert_eq!(
            t.get_point_further_back(2, 0.0).unwrap().x.to_bits(),
            9.0f32.to_bits()
        );
    }

    #[test]
    fn nearest_index_first_wins() {
        let t = straight_trail(4); // x: 9,6,3,0 from head
        assert_eq!(t.find_nearest_index(9.4, 0.0, 0.0), 0);
        assert_eq!(t.find_nearest_index(1.0, 0.0, 0.0), 3);
        // tie between index 1 (6) and 2 (3) at x=4.5: index 1 was scanned first
        assert_eq!(t.find_nearest_index(4.5, 0.0, 0.0), 1);
        assert_eq!(BreadcrumbTrail::new().find_nearest_index(0.0, 0.0, 0.0), 0);
    }

    #[test]
    fn clear_resets() {
        let mut t = straight_trail(3);
        t.clear();
        assert!(t.is_empty());
        assert!(t.get_point_at_distance(1.0).is_none());
    }
}
