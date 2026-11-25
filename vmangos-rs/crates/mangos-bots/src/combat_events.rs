//! `CombatEventBuffer` (`PlayerBots/BotCombatEventBuffer.h`):
//! the 256-slot combat-event ring buffer feeding pattern
//! detection. GUIDs are stored as raw lows, as in the C++
//! (`GetGUIDLow()` widened to uint64 fields).

/// `CombatEventType`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum CombatEventType {
    #[default]
    Damage = 0,
    CastStart = 1,
    Death = 2,
}

/// One recorded combat event (`CombatEvent`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CombatEvent {
    /// `WorldTimer::getMSTime()` at record time (0 = empty slot).
    pub timestamp: u32,
    pub event_type: CombatEventType,
    pub source_guid_low: u64,
    pub target_guid_low: u64,
    pub spell_id: u32,
    pub damage: u32,
    /// Source creature entry (0 for players).
    pub creature_entry: u32,
}

/// Fixed 256-entry ring buffer; the write index is a wrapping
/// u8 exactly as in C++.
#[derive(Clone, Debug)]
pub struct CombatEventBuffer {
    events: [CombatEvent; 256],
    write_idx: u8,
}

impl Default for CombatEventBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl CombatEventBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: [CombatEvent::default(); 256],
            write_idx: 0,
        }
    }

    /// `Push`.
    pub fn push(&mut self, e: CombatEvent) {
        self.events[self.write_idx as usize] = e;
        self.write_idx = self.write_idx.wrapping_add(1); // wraps at 256
    }

    /// `ScanBackwards`: visit from newest to oldest until the
    /// visitor returns false, an uninitialized slot
    /// (timestamp 0) is hit, or all 256 entries were visited.
    pub fn scan_backwards(&self, mut visitor: impl FnMut(&CombatEvent) -> bool) {
        let mut idx = self.write_idx;
        for _ in 0..256 {
            idx = idx.wrapping_sub(1);
            let event = &self.events[idx as usize];
            if event.timestamp == 0 {
                break;
            }
            if !visitor(event) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: u32, spell: u32) -> CombatEvent {
        CombatEvent {
            timestamp: ts,
            spell_id: spell,
            ..CombatEvent::default()
        }
    }

    #[test]
    fn scan_newest_first_and_stops_at_empty() {
        let mut b = CombatEventBuffer::new();
        let mut seen = Vec::new();
        b.scan_backwards(|e| {
            seen.push(e.spell_id);
            true
        });
        assert!(seen.is_empty());

        b.push(ev(100, 1));
        b.push(ev(200, 2));
        b.push(ev(300, 3));
        b.scan_backwards(|e| {
            seen.push(e.spell_id);
            true
        });
        assert_eq!(seen, vec![3, 2, 1]);
    }

    #[test]
    fn visitor_false_stops_scan() {
        let mut b = CombatEventBuffer::new();
        for i in 0..5 {
            b.push(ev(100 + i, i));
        }
        let mut seen = Vec::new();
        b.scan_backwards(|e| {
            seen.push(e.spell_id);
            e.spell_id != 3
        });
        assert_eq!(seen, vec![4, 3]);
    }

    #[test]
    fn wrap_overwrites_oldest() {
        let mut b = CombatEventBuffer::new();
        for i in 0..300u32 {
            b.push(ev(1000 + i, i));
        }
        let mut seen = Vec::new();
        b.scan_backwards(|e| {
            seen.push(e.spell_id);
            true
        });
        assert_eq!(seen.len(), 256);
        assert_eq!(seen[0], 299);
        assert_eq!(seen[255], 44); // 300 - 256
    }

    #[test]
    fn timestamp_zero_event_terminates_scan() {
        // An event recorded at ms-time 0 makes everything older
        // invisible, exactly as in C++.
        let mut b = CombatEventBuffer::new();
        b.push(ev(100, 1));
        b.push(ev(0, 2));
        b.push(ev(200, 3));
        let mut seen = Vec::new();
        b.scan_backwards(|e| {
            seen.push(e.spell_id);
            true
        });
        assert_eq!(seen, vec![3]);
    }
}
