//! The per-object values word array + mirror, ported from
//! `Object` (`vmangos/src/game/Objects/Object.h:175-245,424-433`,
//! `Object.cpp:942-972,1035-1313,2806-2810`).
//!
//! Semantics replicated exactly:
//!
//! - One `u32` word array plus a same-size **mirror** ("last
//!   broadcast" state). Dirty = `mirror[i] != words[i]`; the mirror
//!   is global, not per-viewer — dirtiness is consumed once per tick
//!   for all viewers, and late viewers depend on CREATE blocks.
//! - Setters write only on change and report it (`bool` return);
//!   the C++ side calls `MarkForClientUpdate()` at that point,
//!   which needs world/map state and therefore stays with the
//!   entity layer — callers must mark on `true`.
//! - [`Values::set_u64`] poisons both mirror halves with `value + 1`
//!   so both GUID halves always reach the wire (client bug
//!   workaround documented at `Object.cpp:1120-1133`).
//! - Bounds: the C++ `MANGOS_ASSERT(index < m_valuesCount)` becomes
//!   ordinary slice-index panics here (documented per method).
//!   Byte/short offsets assert `< 4` / `< 2` like the C++ getters;
//!   the C++ *setters* check `> 4` / `> 2` and log-and-return,
//!   admitting one out-of-range offset into UB — that case panics
//!   here instead (recorded deviation, unreachable in practice).
//! - Float comparison in [`Values::set_float`] is `!=`, so NaN
//!   always reports changed, as in C++.

use mangos_guid::ObjectGuid;
use mangos_protocol::update_mask::UpdateMask;

/// `m_uint32Values` + `m_uint32Values_mirror` (`Object.h:424-433`),
/// sized by the leaf type's `*_END` at construction
/// (`_InitValues`, `Object.cpp:212-221`).
#[derive(Clone, Debug)]
pub struct Values {
    words: Box<[u32]>,
    mirror: Box<[u32]>,
}

impl Values {
    /// Zeroed store of `count` words (`m_valuesCount`).
    #[must_use]
    pub fn new(count: u16) -> Self {
        Self {
            words: vec![0; count as usize].into_boxed_slice(),
            mirror: vec![0; count as usize].into_boxed_slice(),
        }
    }

    /// `m_valuesCount`.
    #[must_use]
    pub fn count(&self) -> u16 {
        self.words.len() as u16
    }

    /// Raw words (create-block nonzero test, `Object.cpp:1085`).
    #[must_use]
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// Raw mirror words.
    #[must_use]
    pub fn mirror(&self) -> &[u32] {
        &self.mirror
    }

    /// Any word differs from its mirror (a values block
    /// would be non-empty).
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.words != self.mirror
    }

    /// `ClearUpdateMask` mirror half (`Object.cpp:942-954`): copy
    /// words over the mirror. The client-update-list removal half
    /// belongs to the entity layer.
    pub fn clear_update_mask(&mut self) {
        self.mirror.copy_from_slice(&self.words);
    }

    /// `ForceValuesUpdateAtIndex` (`Object.cpp:2806-2810`): poison
    /// the mirror so the word broadcasts. The C++ also queues a
    /// delayed mark; the entity layer does that.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn force_update_at(&mut self, index: u16) {
        self.mirror[index as usize] = self.words[index as usize].wrapping_add(1);
    }

    /// Sync one mirror word to its value word (the inline mirror
    /// writes in `DirectSendPublicValueUpdate`, `Object.cpp:385`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn sync_mirror_word(&mut self, index: u16) {
        self.mirror[index as usize] = self.words[index as usize];
    }

    /// `LoadValues` (`Object.cpp:956-972`): parse exactly `count`
    /// space-separated integers (`atol` semantics per token).
    /// Returns `false` without writing on a token-count mismatch.
    /// The mirror is not touched, as in C++.
    pub fn load_values(&mut self, data: &str) -> bool {
        let tokens: Vec<&str> = data.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.len() != self.words.len() {
            return false;
        }
        for (i, tok) in tokens.iter().enumerate() {
            self.words[i] = atol(tok);
        }
        true
    }

    /// `_LoadIntoDataField` (`Object.cpp:1035-1050`): load `count`
    /// tokens at `start`, poisoning each mirror word with `+1` so
    /// the range broadcasts. Empty input or a token-count mismatch
    /// is a silent no-op, as in C++.
    ///
    /// # Panics
    ///
    /// Panics if the target range leaves the store (C++ writes out
    /// of bounds).
    pub fn load_into_data_field(&mut self, data: &str, start: u32, count: u32) {
        if data.is_empty() {
            return;
        }
        let tokens: Vec<&str> = data.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.len() != count as usize {
            return;
        }
        for (i, tok) in tokens.iter().enumerate() {
            let idx = (start as usize) + i;
            self.words[idx] = strtoul(tok);
            self.mirror[idx] = self.words[idx].wrapping_add(1);
        }
    }

    /// `GetUInt32Value` (`Object.h:184-188`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn get_u32(&self, index: u16) -> u32 {
        self.words[index as usize]
    }

    /// `GetInt32Value` (`Object.h:178-182`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn get_i32(&self, index: u16) -> i32 {
        i32::from_ne_bytes(self.get_u32(index).to_ne_bytes())
    }

    /// `GetUInt64Value` (`Object.h:190-194`): two LE words.
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count` (C++ asserts the same
    /// strict bound).
    #[must_use]
    pub fn get_u64(&self, index: u16) -> u64 {
        assert!(
            (index as usize) + 1 < self.words.len(),
            "u64 value index {index} out of range"
        );
        u64::from(self.words[index as usize]) | (u64::from(self.words[index as usize + 1]) << 32)
    }

    /// `GetGuidValue` (`Object.h:223`). Note: reads the raw words
    /// with no player clamp, exactly as the reinterpret cast does.
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    #[must_use]
    pub fn get_guid(&self, index: u16) -> ObjectGuid {
        ObjectGuid::from_raw(self.get_u64(index))
    }

    /// `GetFloatValue` (`Object.h:196-200`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn get_f32(&self, index: u16) -> f32 {
        f32::from_bits(self.get_u32(index))
    }

    /// `GetByteValue` (`Object.h:202-207`): lane `offset` (0..4),
    /// little-endian.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    #[must_use]
    pub fn get_byte(&self, index: u16, offset: u8) -> u8 {
        assert!(offset < 4, "byte offset {offset} out of range");
        (self.get_u32(index) >> (u32::from(offset) * 8)) as u8
    }

    /// `GetUInt16Value` (`Object.h:209-214`): lane `offset` (0..2).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 2`.
    #[must_use]
    pub fn get_u16(&self, index: u16, offset: u8) -> u16 {
        assert!(offset < 2, "short offset {offset} out of range");
        (self.get_u32(index) >> (u32::from(offset) * 16)) as u16
    }

    /// `GetInt16Value` (`Object.h:216-221`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 2`.
    #[must_use]
    pub fn get_i16(&self, index: u16, offset: u8) -> i16 {
        i16::from_ne_bytes(self.get_u16(index, offset).to_ne_bytes())
    }

    /// `HasFlag` (`Object.h:259-263`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn has_flag(&self, index: u16, flag: u32) -> bool {
        self.get_u32(index) & flag != 0
    }

    /// `HasByteFlag` (`Object.h:283-288`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    #[must_use]
    pub fn has_byte_flag(&self, index: u16, offset: u8, flag: u8) -> bool {
        self.get_byte(index, offset) & flag != 0
    }

    /// `HasShortFlag` (`Object.h:306-310`); the C++ flag parameter
    /// is `uint8` (narrower than the setters'), kept as-is.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn has_short_flag(&self, index: u16, high_part: bool, flag: u8) -> bool {
        self.get_u16(index, u8::from(high_part)) & u16::from(flag) != 0
    }

    /// `HasFlag64` (`Object.h:335-339`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    #[must_use]
    pub fn has_flag64(&self, index: u16, flag: u64) -> bool {
        self.get_u64(index) & flag != 0
    }

    /// `SetUInt32Value` (`Object.cpp:1101-1110`). Returns `true`
    /// when the word changed (C++ then marks for client update).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_u32(&mut self, index: u16, value: u32) -> bool {
        let word = &mut self.words[index as usize];
        if *word == value {
            return false;
        }
        *word = value;
        true
    }

    /// `SetInt32Value` (`Object.cpp:1090-1099`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_i32(&mut self, index: u16, value: i32) -> bool {
        self.set_u32(index, u32::from_ne_bytes(value.to_ne_bytes()))
    }

    /// `SetUInt64Value` (`Object.cpp:1112-1136`): writes both words
    /// and poisons both mirror halves (`+1`) so the pair always
    /// broadcasts together.
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn set_u64(&mut self, index: u16, value: u64) -> bool {
        if self.get_u64(index) == value {
            return false;
        }
        let i = index as usize;
        let first = value as u32;
        let second = (value >> 32) as u32;
        self.words[i] = first;
        self.words[i + 1] = second;
        self.mirror[i] = first.wrapping_add(1);
        self.mirror[i + 1] = second.wrapping_add(1);
        true
    }

    /// `SetGuidValue` (`Object.h:232`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn set_guid(&mut self, index: u16, value: ObjectGuid) -> bool {
        self.set_u64(index, value.raw())
    }

    /// `SetFloatValue` (`Object.cpp:1138-1147`): `!=` comparison,
    /// so NaN always counts as changed.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[allow(clippy::float_cmp)] // C++ compares with != (Object.cpp:1142); NaN must count as changed
    pub fn set_f32(&mut self, index: u16, value: f32) -> bool {
        if self.get_f32(index) == value {
            return false;
        }
        self.words[index as usize] = value.to_bits();
        true
    }

    /// `SetByteValue` (`Object.cpp:1149-1165`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4` (the C++
    /// setter's `offset > 4` check admits offset 4 into UB; this
    /// port asserts like the getter instead).
    pub fn set_byte(&mut self, index: u16, offset: u8, value: u8) -> bool {
        assert!(offset < 4, "byte offset {offset} out of range");
        let shift = u32::from(offset) * 8;
        if ((self.words[index as usize] >> shift) as u8) == value {
            return false;
        }
        let word = &mut self.words[index as usize];
        *word &= !(0xFFu32 << shift);
        *word |= u32::from(value) << shift;
        true
    }

    /// `SetUInt16Value` (`Object.cpp:1167-1183`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 2`.
    pub fn set_u16(&mut self, index: u16, offset: u8, value: u16) -> bool {
        assert!(offset < 2, "short offset {offset} out of range");
        let shift = u32::from(offset) * 16;
        if ((self.words[index as usize] >> shift) as u16) == value {
            return false;
        }
        let word = &mut self.words[index as usize];
        *word &= !(0xFFFFu32 << shift);
        *word |= u32::from(value) << shift;
        true
    }

    /// `SetInt16Value` (`Object.h:231`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 2`.
    pub fn set_i16(&mut self, index: u16, offset: u8, value: i16) -> bool {
        self.set_u16(index, offset, u16::from_ne_bytes(value.to_ne_bytes()))
    }

    /// `SetStatFloatValue` (`Object.cpp:1185-1191`): negatives
    /// clamp to 0 (NaN passes through, `value < 0` is false).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_stat_f32(&mut self, index: u16, value: f32) -> bool {
        self.set_f32(index, if value < 0.0 { 0.0 } else { value })
    }

    /// `SetStatInt32Value` (`Object.cpp:1193-1199`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_stat_i32(&mut self, index: u16, value: i32) -> bool {
        self.set_u32(index, if value < 0 { 0 } else { value as u32 })
    }

    /// `ApplyModUInt32Value` (`Object.cpp:1201-1208`): the current
    /// word is read as `int32`, adjusted, and clamped at 0.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_u32(&mut self, index: u16, val: i32, apply: bool) -> bool {
        let mut cur = self.get_i32(index);
        cur = cur.wrapping_add(if apply { val } else { val.wrapping_neg() });
        if cur < 0 {
            cur = 0;
        }
        self.set_u32(index, u32::from_ne_bytes(cur.to_ne_bytes()))
    }

    /// `ApplyModInt32Value` (`Object.cpp:1210-1215`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_i32(&mut self, index: u16, val: i32, apply: bool) -> bool {
        let cur = self
            .get_i32(index)
            .wrapping_add(if apply { val } else { val.wrapping_neg() });
        self.set_i32(index, cur)
    }

    /// `ApplyModSignedFloatValue` (`Object.cpp:1217-1222`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_signed_f32(&mut self, index: u16, val: f32, apply: bool) -> bool {
        let cur = self.get_f32(index) + if apply { val } else { -val };
        self.set_f32(index, cur)
    }

    /// `ApplyModPositiveFloatValue` (`Object.cpp:1224-1231`):
    /// signed mod clamped at 0.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_positive_f32(&mut self, index: u16, val: f32, apply: bool) -> bool {
        let mut cur = self.get_f32(index) + if apply { val } else { -val };
        if cur < 0.0 {
            cur = 0.0;
        }
        self.set_f32(index, cur)
    }

    /// `ApplyPercentModFloatValue` (`Object.h:243-248`): a `-100%`
    /// mod is rewritten to `-99.9%` before applying.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[allow(clippy::float_cmp)] // C++ tests val != -100.0f exactly (Object.h:246)
    pub fn apply_percent_mod_f32(&mut self, index: u16, val: f32, apply: bool) -> bool {
        let val = if val == -100.0 { -99.9 } else { val };
        let cur = self.get_f32(index)
            * if apply {
                (100.0 + val) / 100.0
            } else {
                100.0 / (100.0 + val)
            };
        self.set_f32(index, cur)
    }

    /// `SetFlag` (`Object.cpp:1233-1244`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_flag(&mut self, index: u16, flag: u32) -> bool {
        let new = self.get_u32(index) | flag;
        self.set_u32(index, new)
    }

    /// `RemoveFlag` (`Object.cpp:1246-1257`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn remove_flag(&mut self, index: u16, flag: u32) -> bool {
        let new = self.get_u32(index) & !flag;
        self.set_u32(index, new)
    }

    /// `ToggleFlag` (`Object.h:252-258`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn toggle_flag(&mut self, index: u16, flag: u32) -> bool {
        if self.has_flag(index, flag) {
            self.remove_flag(index, flag)
        } else {
            self.set_flag(index, flag)
        }
    }

    /// `ApplyModFlag` (`Object.h:265-271`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_flag(&mut self, index: u16, flag: u32, apply: bool) -> bool {
        if apply {
            self.set_flag(index, flag)
        } else {
            self.remove_flag(index, flag)
        }
    }

    /// `SetByteFlag` (`Object.cpp:1259-1274`): no-op when the flag
    /// byte already intersects.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    pub fn set_byte_flag(&mut self, index: u16, offset: u8, flag: u8) -> bool {
        if self.has_byte_flag(index, offset, flag) {
            return false;
        }
        let shift = u32::from(offset) * 8;
        self.words[index as usize] |= u32::from(flag) << shift;
        true
    }

    /// `RemoveByteFlag` (`Object.cpp:1276-1291`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    pub fn remove_byte_flag(&mut self, index: u16, offset: u8, flag: u8) -> bool {
        if !self.has_byte_flag(index, offset, flag) {
            return false;
        }
        let shift = u32::from(offset) * 8;
        self.words[index as usize] &= !(u32::from(flag) << shift);
        true
    }

    /// `ToggleByteFlag` (`Object.h:276-282`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    pub fn toggle_byte_flag(&mut self, index: u16, offset: u8, flag: u8) -> bool {
        if self.has_byte_flag(index, offset, flag) {
            self.remove_byte_flag(index, offset, flag)
        } else {
            self.set_byte_flag(index, offset, flag)
        }
    }

    /// `ApplyModByteFlag` (`Object.h:290-296`); the C++ flag
    /// parameter is `uint32`, truncated to a byte at the call.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or `offset >= 4`.
    pub fn apply_mod_byte_flag(&mut self, index: u16, offset: u8, flag: u32, apply: bool) -> bool {
        if apply {
            self.set_byte_flag(index, offset, flag as u8)
        } else {
            self.remove_byte_flag(index, offset, flag as u8)
        }
    }

    /// `SetShortFlag` (`Object.cpp:1293-1302`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set_short_flag(&mut self, index: u16, high_part: bool, flag: u16) -> bool {
        let shift = if high_part { 16 } else { 0 };
        if (self.words[index as usize] >> shift) as u16 & flag != 0 {
            return false;
        }
        self.words[index as usize] |= u32::from(flag) << shift;
        true
    }

    /// `RemoveShortFlag` (`Object.cpp:1304-1313`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn remove_short_flag(&mut self, index: u16, high_part: bool, flag: u16) -> bool {
        let shift = if high_part { 16 } else { 0 };
        if (self.words[index as usize] >> shift) as u16 & flag == 0 {
            return false;
        }
        self.words[index as usize] &= !(u32::from(flag) << shift);
        true
    }

    /// `ToggleShortFlag` (`Object.h:299-305`); flag parameter is
    /// `uint8` in C++.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn toggle_short_flag(&mut self, index: u16, high_part: bool, flag: u8) -> bool {
        if self.has_short_flag(index, high_part, flag) {
            self.remove_short_flag(index, high_part, u16::from(flag))
        } else {
            self.set_short_flag(index, high_part, u16::from(flag))
        }
    }

    /// `ApplyModShortFlag` (`Object.h:312-318`); flag parameter is
    /// `uint32` in C++, truncated to `uint16` at the call.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn apply_mod_short_flag(
        &mut self,
        index: u16,
        high_part: bool,
        flag: u32,
        apply: bool,
    ) -> bool {
        if apply {
            self.set_short_flag(index, high_part, flag as u16)
        } else {
            self.remove_short_flag(index, high_part, flag as u16)
        }
    }

    /// `SetFlag64` (`Object.h:320-325`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn set_flag64(&mut self, index: u16, flag: u64) -> bool {
        let new = self.get_u64(index) | flag;
        self.set_u64(index, new)
    }

    /// `RemoveFlag64` (`Object.h:327-332`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn remove_flag64(&mut self, index: u16, flag: u64) -> bool {
        let new = self.get_u64(index) & !flag;
        self.set_u64(index, new)
    }

    /// `ToggleFlag64` (`Object.h:341-347`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn toggle_flag64(&mut self, index: u16, flag: u64) -> bool {
        if self.has_flag64(index, flag) {
            self.remove_flag64(index, flag)
        } else {
            self.set_flag64(index, flag)
        }
    }

    /// `ApplyModFlag64` (`Object.h:349-355`).
    ///
    /// # Panics
    ///
    /// Panics unless `index + 1 < count`.
    pub fn apply_mod_flag64(&mut self, index: u16, flag: u64, apply: bool) -> bool {
        if apply {
            self.set_flag64(index, flag)
        } else {
            self.remove_flag64(index, flag)
        }
    }

    /// `_SetCreateBits` (`Object.cpp:1077-1088`): bit set iff the
    /// word is nonzero and its field flags intersect `visible`.
    /// `flags` is the per-type flags array (`update_field_flags`).
    ///
    /// # Panics
    ///
    /// Panics if `flags` is shorter than the store.
    #[must_use]
    pub fn create_bits(&self, flags: &[u16], visible: u16) -> UpdateMask {
        let mut mask = UpdateMask::new();
        mask.set_count(u32::from(self.count()));
        for (i, &word) in self.words.iter().enumerate() {
            if word != 0 && flags[i] & visible != 0 {
                mask.set_bit(i as u32);
            }
        }
        mask
    }

    /// `_SetUpdateBits` (`Object.cpp:1064-1075`): bit set iff the
    /// word differs from the mirror and is visible.
    ///
    /// # Panics
    ///
    /// Panics if `flags` is shorter than the store.
    #[must_use]
    pub fn update_bits(&self, flags: &[u16], visible: u16) -> UpdateMask {
        let mut mask = UpdateMask::new();
        mask.set_count(u32::from(self.count()));
        for (i, &word) in self.words.iter().enumerate() {
            if self.mirror[i] != word && flags[i] & visible != 0 {
                mask.set_bit(i as u32);
            }
        }
        mask
    }

    /// `MarkUpdateFieldsWithFlagForUpdate` (`Object.cpp:1052-1062`):
    /// add bits for every field carrying `flag`, skipping zero
    /// words unless `including_empty`.
    ///
    /// # Panics
    ///
    /// Panics if `flags` is shorter than the store, or if `mask`
    /// is narrower (same as C++ writing out of bounds).
    pub fn mark_fields_with_flag(
        &self,
        mask: &mut UpdateMask,
        flags: &[u16],
        flag: u16,
        including_empty: bool,
    ) {
        for (i, &word) in self.words.iter().enumerate() {
            if (including_empty || word != 0) && flags[i] & flag != 0 {
                mask.set_bit(i as u32);
            }
        }
    }
}

/// C `atol` as used by `LoadValues`: optional sign + decimal prefix,
/// no digits = 0, result wrapped into `u32` on assignment.
fn atol(tok: &str) -> u32 {
    let (neg, digits) = parse_decimal_prefix(tok);
    let mag = digits as u32;
    if neg { mag.wrapping_neg() } else { mag }
}

/// C `strtoul(..., 10)` as used by `_LoadIntoDataField`: optional
/// sign + decimal prefix; a negative result wraps modulo 2^32
/// (the C++ stores into `uint32`; 64-bit `unsigned long` overflow
/// behaviour is unreachable from DB data and not replicated).
fn strtoul(tok: &str) -> u32 {
    atol(tok)
}

/// Shared prefix parse: skips leading ASCII whitespace (`atol`
/// does; tokens are pre-split so this only matters for tabs),
/// one optional `+`/`-`, then decimal digits. Returns
/// (negative, magnitude) with magnitude saturating at `u32::MAX`
/// growth via `i64` headroom.
fn parse_decimal_prefix(tok: &str) -> (bool, u64) {
    let bytes = tok.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let mut mag: u64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        mag = mag
            .saturating_mul(10)
            .saturating_add(u64::from(bytes[i] - b'0'));
        i += 1;
    }
    (neg, mag)
}
