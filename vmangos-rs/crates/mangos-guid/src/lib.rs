//! `ObjectGuid` value semantics, byte-compatible with the C++ originals in
//! `vmangos/src/game/ObjectGuid.h` / `ObjectGuid.cpp` and the packed-GUID
//! codec in `vmangos/src/shared/ByteBuffer.h` (`appendPackGUID` /
//! `readPackGUID`).
//!
//! Layout (frozen by the 1.12.1 client, packets, and DB rows):
//! `u64 = high:16 << 48 | entry:24 << 24 | counter:24` for entry-bearing
//! highs, or `high:16 << 48 | counter:32` for the rest. Database columns
//! store only [`ObjectGuid::counter`].
//!
//! Deliberate deviations from C++, forced by layering: the C++
//! `ObjectGuidGenerator` logs and stops the server on overflow
//! (`ObjectGuid.cpp:86-90`); [`GuidGenerator`] here returns `None`
//! instead and the world layer (Phase 3+) decides the shutdown policy.
//! DB loading (`LoadFromDB`) is not ported; it belongs to `mangos-db`.

pub mod timer;

use std::collections::VecDeque;

/// The 16-bit high part of a GUID (`enum HighGuid`, `ObjectGuid.h:66-78`).
/// A newtype rather than an enum because wire input can carry any `u16`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct HighGuid(pub u16);

impl HighGuid {
    pub const ITEM: Self = Self(0x4000);
    /// Same wire value as [`Self::ITEM`] (`ObjectGuid.h:69`).
    pub const CONTAINER: Self = Self(0x4000);
    pub const PLAYER: Self = Self(0x0000);
    pub const GAMEOBJECT: Self = Self(0xF110);
    pub const TRANSPORT: Self = Self(0xF120);
    pub const UNIT: Self = Self(0xF130);
    pub const PET: Self = Self(0xF140);
    pub const DYNAMIC_OBJECT: Self = Self(0xF100);
    pub const CORPSE: Self = Self(0xF101);
    pub const MO_TRANSPORT: Self = Self(0x1FC0);

    /// `ObjectGuid::HasEntry` (`ObjectGuid.h:216-233`): unknown highs
    /// default to entry-bearing, exactly as the C++ `default:` arm does.
    #[must_use]
    pub const fn has_entry(self) -> bool {
        !matches!(
            self,
            Self::ITEM | Self::PLAYER | Self::DYNAMIC_OBJECT | Self::CORPSE | Self::MO_TRANSPORT
        )
    }

    /// `ObjectGuid::GetMaxCounter` (`ObjectGuid.h:158`).
    #[must_use]
    pub const fn max_counter(self) -> u32 {
        if self.has_entry() {
            0x00FF_FFFF
        } else {
            0xFFFF_FFFF
        }
    }

    /// `ObjectGuid::GetTypeName` (`ObjectGuid.cpp:29-54`).
    #[must_use]
    pub const fn type_name(self) -> &'static str {
        match self {
            Self::ITEM => "Item",
            Self::PLAYER => "Player",
            Self::GAMEOBJECT => "Gameobject",
            Self::TRANSPORT => "Transport",
            Self::UNIT => "Creature",
            Self::PET => "Pet",
            Self::DYNAMIC_OBJECT => "DynObject",
            Self::CORPSE => "Corpse",
            Self::MO_TRANSPORT => "MoTransport",
            _ => "<unknown>",
        }
    }
}

/// `enum TypeID` (`ObjectGuid.h:33-43`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum TypeId {
    Object = 0,
    Item = 1,
    Container = 2,
    Unit = 3,
    Player = 4,
    GameObject = 5,
    DynamicObject = 6,
    Corpse = 7,
}

impl TypeId {
    /// `ObjectGuid::GetTypeId` (`ObjectGuid.h:176-201`). Preserves the C++
    /// quirk that `HIGHGUID_TRANSPORT` has no arm and maps to
    /// [`TypeId::Object`].
    #[must_use]
    pub const fn from_high(high: HighGuid) -> Self {
        match high {
            HighGuid::ITEM => Self::Item,
            HighGuid::UNIT | HighGuid::PET => Self::Unit,
            HighGuid::PLAYER => Self::Player,
            HighGuid::GAMEOBJECT | HighGuid::MO_TRANSPORT => Self::GameObject,
            HighGuid::DYNAMIC_OBJECT => Self::DynamicObject,
            HighGuid::CORPSE => Self::Corpse,
            _ => Self::Object,
        }
    }
}

/// A frozen-layout object GUID: `#[repr(transparent)]` over the raw `u64`
/// the client, packets, and DB rows already use.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ObjectGuid(u64);

impl ObjectGuid {
    pub const EMPTY: Self = Self(0);

    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// `ObjectGuid(HighGuid, uint32 counter)` (`ObjectGuid.h:130`):
    /// counter 0 collapses to the empty GUID.
    #[must_use]
    pub const fn new(high: HighGuid, counter: u32) -> Self {
        if counter == 0 {
            Self::EMPTY
        } else {
            Self(counter as u64 | ((high.0 as u64) << 48))
        }
    }

    /// `ObjectGuid(HighGuid, uint32 entry, uint32 counter)`
    /// (`ObjectGuid.h:129`): counter 0 collapses to the empty GUID.
    /// Entry and counter are placed unmasked, as in C++; callers must
    /// keep entry < 2^24 and counter within [`HighGuid::max_counter`].
    #[must_use]
    pub const fn with_entry(high: HighGuid, entry: u32, counter: u32) -> Self {
        if counter == 0 {
            Self::EMPTY
        } else {
            Self(counter as u64 | ((entry as u64) << 24) | ((high.0 as u64) << 48))
        }
    }

    /// Like the C++ `operator>>` (`ObjectGuid.cpp:176-182`): player GUIDs
    /// read from the wire are clamped to their low counter.
    #[must_use]
    pub const fn from_wire_raw(raw: u64) -> Self {
        Self(clamp_player_guid(raw))
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// `GetHigh` (`ObjectGuid.h:150`).
    #[must_use]
    pub const fn high(self) -> HighGuid {
        HighGuid(((self.0 >> 48) & 0xFFFF) as u16)
    }

    /// `GetEntry` (`ObjectGuid.h:153`): 0 for non-entry highs.
    #[must_use]
    pub const fn entry(self) -> u32 {
        if self.high().has_entry() {
            ((self.0 >> 24) & 0x00FF_FFFF) as u32
        } else {
            0
        }
    }

    /// `GetCounter` (`ObjectGuid.h:154-156`). This low part is what DB
    /// columns store.
    #[must_use]
    pub const fn counter(self) -> u32 {
        counter_of(self.0, self.high().has_entry())
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_creature(self) -> bool {
        matches!(self.high(), HighGuid::UNIT)
    }

    #[must_use]
    pub const fn is_pet(self) -> bool {
        matches!(self.high(), HighGuid::PET)
    }

    #[must_use]
    pub const fn is_creature_or_pet(self) -> bool {
        self.is_creature() || self.is_pet()
    }

    #[must_use]
    pub const fn is_player(self) -> bool {
        !self.is_empty() && matches!(self.high(), HighGuid::PLAYER)
    }

    #[must_use]
    pub const fn is_unit(self) -> bool {
        self.is_creature_or_pet() || self.is_player()
    }

    #[must_use]
    pub const fn is_item(self) -> bool {
        matches!(self.high(), HighGuid::ITEM)
    }

    #[must_use]
    pub const fn is_game_object(self) -> bool {
        matches!(self.high(), HighGuid::GAMEOBJECT)
    }

    #[must_use]
    pub const fn is_dynamic_object(self) -> bool {
        matches!(self.high(), HighGuid::DYNAMIC_OBJECT)
    }

    #[must_use]
    pub const fn is_corpse(self) -> bool {
        matches!(self.high(), HighGuid::CORPSE)
    }

    #[must_use]
    pub const fn is_transport(self) -> bool {
        matches!(self.high(), HighGuid::TRANSPORT)
    }

    #[must_use]
    pub const fn is_mo_transport(self) -> bool {
        matches!(self.high(), HighGuid::MO_TRANSPORT)
    }

    #[must_use]
    pub const fn type_id(self) -> TypeId {
        TypeId::from_high(self.high())
    }

    /// `GetTypeName` on the instance (`ObjectGuid.h:212`): `"None"` when
    /// empty.
    #[must_use]
    pub const fn type_name(self) -> &'static str {
        if self.is_empty() {
            "None"
        } else {
            self.high().type_name()
        }
    }

    /// `ByteBuffer::appendPackGUID` (`ByteBuffer.h:440-458`): one mask
    /// byte, then each non-zero little-endian byte in order.
    pub fn write_packed(self, out: &mut Vec<u8>) {
        let start = out.len();
        out.push(0);
        let mut guid = self.0;
        let mut i = 0u8;
        while guid != 0 {
            if guid & 0xFF != 0 {
                out[start] |= 1 << i;
                out.push((guid & 0xFF) as u8);
            }
            guid >>= 8;
            i += 1;
        }
    }

    /// `ByteBuffer::readPackGUID` (`ByteBuffer.h:377-394`) without the
    /// player clamp. Returns the raw value and bytes consumed. Where C++
    /// throws on truncated input, this returns
    /// [`PackedGuidError::Truncated`].
    ///
    /// # Errors
    /// Returns [`PackedGuidError::Truncated`] if `bytes` ends before the
    /// mask's promised bytes.
    pub fn read_packed_raw(bytes: &[u8]) -> Result<(u64, usize), PackedGuidError> {
        let (&mark, rest) = bytes.split_first().ok_or(PackedGuidError::Truncated)?;
        let mut guid = 0u64;
        let mut consumed = 1usize;
        for i in 0..8 {
            if mark & (1 << i) != 0 {
                let &byte = rest.get(consumed - 1).ok_or(PackedGuidError::Truncated)?;
                guid |= u64::from(byte) << (i * 8);
                consumed += 1;
            }
        }
        Ok((guid, consumed))
    }

    /// The `PackedGuidReader` path (`ObjectGuid.cpp:190-196`):
    /// [`Self::read_packed_raw`] plus the player clamp.
    ///
    /// # Errors
    /// Returns [`PackedGuidError::Truncated`] on truncated input.
    pub fn read_packed(bytes: &[u8]) -> Result<(Self, usize), PackedGuidError> {
        let (raw, consumed) = Self::read_packed_raw(bytes)?;
        Ok((Self::from_wire_raw(raw), consumed))
    }
}

impl From<ObjectGuid> for u64 {
    fn from(guid: ObjectGuid) -> Self {
        guid.0
    }
}

impl std::fmt::Display for ObjectGuid {
    /// `ObjectGuid::GetString` (`ObjectGuid.cpp:56-73`) minus the player
    /// name lookup, which needs the world layer.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (", self.type_name())?;
        if self.high().has_entry() {
            let label = if self.is_pet() { "Petnumber" } else { "Entry" };
            write!(f, "{label}: {} ", self.entry())?;
        }
        write!(f, "Guid: {})", self.counter())
    }
}

/// Failure modes of packed-GUID decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackedGuidError {
    /// Input ended before the mask byte's promised bytes arrived.
    Truncated,
}

impl std::fmt::Display for PackedGuidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated packed GUID"),
        }
    }
}

impl std::error::Error for PackedGuidError {}

const fn counter_of(raw: u64, has_entry: bool) -> u32 {
    if has_entry {
        (raw & 0x00FF_FFFF) as u32
    } else {
        (raw & 0xFFFF_FFFF) as u32
    }
}

/// `ObjectGuid::ClampPlayerGuid` (`ObjectGuid.cpp:202-208`): a player
/// GUID read from an untrusted packet is reduced to its low counter so
/// `low + k * (u32::MAX + 1)` cannot alias another hash bucket.
#[must_use]
pub const fn clamp_player_guid(raw: u64) -> u64 {
    if ((raw >> 48) & 0xFFFF) as u16 == HighGuid::PLAYER.0 {
        counter_of(raw, false) as u64
    } else {
        raw
    }
}

/// Counter allocator with freed-GUID reuse, mirroring
/// `ObjectGuidGenerator` (`ObjectGuid.h:281-300`, `ObjectGuid.cpp:77-106`)
/// except overflow yields `None` instead of stopping the server.
#[derive(Debug)]
pub struct GuidGenerator {
    high: HighGuid,
    next: u32,
    freed: VecDeque<u32>,
}

impl GuidGenerator {
    /// C++ default start is 1 (`ObjectGuid.h:285`).
    #[must_use]
    pub fn new(high: HighGuid) -> Self {
        Self::with_start(high, 1)
    }

    #[must_use]
    pub fn with_start(high: HighGuid, start: u32) -> Self {
        Self {
            high,
            next: start,
            freed: VecDeque::new(),
        }
    }

    /// `SetMaxUsedGuid` (`ObjectGuid.cpp:159-168`): resume after the
    /// largest counter already in the DB. Returns `false` where C++
    /// aborts startup (`val == u32::MAX`).
    pub fn set_max_used(&mut self, val: u32) -> bool {
        if val == u32::MAX {
            return false;
        }
        self.next = val + 1;
        true
    }

    /// `GetNextAfterMaxUsed` (`ObjectGuid.h:294`).
    #[must_use]
    pub const fn next_after_max_used(&self) -> u32 {
        self.next
    }

    /// `FreeGuid` (`ObjectGuid.h:295`).
    pub fn free(&mut self, counter: u32) {
        self.freed.push_back(counter);
    }

    /// `Generate` (`ObjectGuid.cpp:78-92`): freed counters are reused
    /// first (FIFO), even at the overflow threshold.
    pub fn generate(&mut self) -> Option<u32> {
        if let Some(counter) = self.freed.pop_front() {
            return Some(counter);
        }
        if self.next >= self.high.max_counter() - 1 {
            return None;
        }
        let counter = self.next;
        self.next += 1;
        Some(counter)
    }

    /// `GenerateRange` (`ObjectGuid.cpp:95-106`): reserves 100 counters,
    /// returning `(first, last)` with `last` exclusive, as in C++ where
    /// `last` is the next counter after the block.
    pub fn generate_range(&mut self) -> Option<(u32, u32)> {
        const RANGE: u32 = 100;
        if self.next >= self.high.max_counter() - RANGE {
            return None;
        }
        let first = self.next;
        self.next += RANGE;
        Some((first, self.next))
    }

    /// The gap scan of `LoadFromDB` (`ObjectGuid.cpp:110-157`), pure
    /// part: seed the freed pool from the ascending-sorted set of
    /// counters already used in the DB. Call after
    /// [`Self::set_max_used`] with the table's max. `used` must be
    /// sorted ascending and duplicate-free (as the `ORDER BY` query
    /// returns it); values are consumed window by window exactly as
    /// the SQL loop does:
    /// - window size `max(pool_size * 2, 1000)`, windows start at 1;
    /// - a window with no used counters stops the scan entirely
    ///   (the C++ `goto end` on a null result);
    /// - only gaps *between* consecutive used counters in a window
    ///   are collected — the tail after a window's last row is not;
    /// - scanning stops when the pool is full or when a window's
    ///   upper bound reaches `next` (the do-while condition).
    ///
    /// The DB queries themselves stay with the Phase 3 world layer.
    /// Returns how many counters were seeded (the C++ log line count).
    pub fn seed_freed_from_used(&mut self, used: &[u32], pool_size: u32) -> usize {
        if pool_size == 0 {
            return 0;
        }
        let pool_size = pool_size as usize;
        let window = (pool_size * 2).max(1000) as u32;
        let mut seeded = 0usize;
        let mut lower: u32 = 1;
        'scan: loop {
            let upper = lower - 1 + window;
            let rows: Vec<u32> = used
                .iter()
                .copied()
                .filter(|g| (lower..=upper).contains(g))
                .collect();
            if rows.is_empty() {
                break;
            }
            let mut last = lower - 1;
            for current in rows {
                for gap in (last + 1)..current {
                    self.freed.push_back(gap);
                    seeded += 1;
                    if self.freed.len() >= pool_size {
                        break 'scan;
                    }
                }
                last = current;
            }
            lower += window;
            if upper >= self.next {
                break;
            }
        }
        seeded
    }
}

#[cfg(test)]
mod gap_scan_tests {
    use super::*;

    fn seeded_pool(genr: &GuidGenerator) -> Vec<u32> {
        genr.freed.iter().copied().collect()
    }

    #[test]
    fn gaps_between_used_counters_are_seeded() {
        let mut genr = GuidGenerator::new(HighGuid::PLAYER);
        assert!(genr.set_max_used(10));
        let n = genr.seed_freed_from_used(&[1, 2, 5, 10], 100);
        assert_eq!(n, 6);
        assert_eq!(seeded_pool(&genr), vec![3, 4, 6, 7, 8, 9]);
    }

    #[test]
    fn tail_after_last_row_in_window_is_not_collected() {
        let mut genr = GuidGenerator::new(HighGuid::PLAYER);
        assert!(genr.set_max_used(2000));
        // Window is 1000 (pool 100 -> 2*100 < 1000). Used ends at 3;
        // gaps 4..=1000 are the window tail and must NOT be seeded.
        let n = genr.seed_freed_from_used(&[1, 3], 100);
        assert_eq!(n, 1);
        assert_eq!(seeded_pool(&genr), vec![2]);
    }

    #[test]
    fn empty_window_stops_scan() {
        let mut genr = GuidGenerator::new(HighGuid::PLAYER);
        assert!(genr.set_max_used(5000));
        // Window 1 (1..=1000) has rows; window 2 (1001..=2000) is
        // empty -> scan ends, rows in later windows never examined.
        let n = genr.seed_freed_from_used(&[2, 3000, 3002], 100);
        assert_eq!(n, 1);
        assert_eq!(seeded_pool(&genr), vec![1]);
    }

    #[test]
    fn pool_size_caps_seeding() {
        let mut genr = GuidGenerator::new(HighGuid::PLAYER);
        assert!(genr.set_max_used(10));
        let n = genr.seed_freed_from_used(&[10], 3);
        assert_eq!(n, 3);
        assert_eq!(seeded_pool(&genr), vec![1, 2, 3]);
        assert_eq!(genr.generate(), Some(1));
    }

    #[test]
    fn zero_pool_disables_seeding() {
        let mut genr = GuidGenerator::new(HighGuid::PLAYER);
        assert!(genr.set_max_used(10));
        assert_eq!(genr.seed_freed_from_used(&[5], 0), 0);
        assert_eq!(genr.generate(), Some(11));
    }
}
