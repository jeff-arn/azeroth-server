//! Pattern learning (`BotPatternMgr` in
//! `PlayerBots/BotCombatEventBuffer.{h,cpp}`), minus the
//! singleton, unit pointers, and DB persistence (the world layer
//! feeds events as plain data and owns `bot_learned_patterns`
//! SQL through [`PatternStore::all_patterns`] /
//! [`PatternStore::load_pattern`]).

use std::collections::HashMap;

use crate::combat_events::{CombatEvent, CombatEventBuffer, CombatEventType};

/// Detection window: casts older than this don't count.
const DETECT_WINDOW_MS: u32 = 5000;
/// Decay step window (30 minutes without a sighting).
const DECAY_AFTER_SECS: u32 = 1800;
/// Learned confidence cap (255 is reserved for permanent).
const LEARNED_CONFIDENCE_CAP: u8 = 254;
/// Minimum confidence for `GetActivePattern`.
const ACTIVE_MIN_CONFIDENCE: u8 = 2;
/// Hits at/above which a spell is classified as `AoE`.
const AOE_HIT_THRESHOLD: u8 = 3;

/// `PatternResponse`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum PatternResponse {
    #[default]
    Interrupt = 0,
    MoveAway = 1,
    Spread = 2,
    SaveCd = 3,
    MoveToPosition = 4,
    Stack = 5,
    SwapSide = 6,
}

/// One learned (or hand-crafted) encounter response
/// (`LearnedPattern` / `bot_learned_patterns` row).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LearnedPattern {
    pub creature_entry: u32,
    pub trigger_spell_id: u32,
    pub response_type: PatternResponse,
    /// e.g. avoidance range in yards
    pub response_param: f32,
    /// 0–254 learned, 255 permanent (hand-crafted).
    pub confidence: u8,
    /// unix timestamp
    pub last_seen: u32,
    /// hand-crafted override, cannot decay.
    pub permanent: bool,
}

/// Inputs common to the three record hooks, as plain data.
#[derive(Clone, Copy, Debug)]
pub struct RecordContext {
    /// `WorldTimer::getMSTime()`.
    pub now_ms: u32,
    /// `time(nullptr)`.
    pub now_unix: u32,
    pub creature_entry: u32,
}

/// The pattern store + event buffer (`BotPatternMgr` state).
#[derive(Clone, Debug, Default)]
pub struct PatternStore {
    buffer: CombatEventBuffer,
    patterns: HashMap<u32, Vec<LearnedPattern>>,
    active: bool,
}

impl PatternStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `SetActive` / `IsActive` (only process events when party
    /// bots exist).
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// All stored patterns, for DB persistence and inspection.
    pub fn all_patterns(&self) -> impl Iterator<Item = &LearnedPattern> {
        self.patterns.values().flat_map(|v| v.iter())
    }

    /// Pattern count across all creatures.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.values().map(Vec::len).sum()
    }

    /// Creature keys with at least one pattern.
    #[must_use]
    pub fn creature_count(&self) -> usize {
        self.patterns.len()
    }

    /// Insert a row loaded from `bot_learned_patterns` (the C++
    /// `LoadFromDB` loop appends unconditionally).
    pub fn load_pattern(&mut self, p: LearnedPattern) {
        self.patterns.entry(p.creature_entry).or_default().push(p);
    }

    /// `RecordDamage` (the C++ `mapId` parameter is unused
    /// there and dropped here).
    #[allow(clippy::too_many_arguments)]
    pub fn record_damage(
        &mut self,
        ctx: RecordContext,
        source_guid_low: u64,
        victim_guid_low: u64,
        victim_is_player: bool,
        victim_max_health: u32,
        damage: u32,
        spell_id: u32,
    ) {
        if !self.active {
            return;
        }
        self.buffer.push(CombatEvent {
            timestamp: ctx.now_ms,
            event_type: CombatEventType::Damage,
            source_guid_low,
            target_guid_low: victim_guid_low,
            spell_id,
            damage,
            creature_entry: ctx.creature_entry,
        });

        // Significant damage: > 15% of victim's max HP (u32 math).
        if victim_is_player && damage > victim_max_health * 15 / 100 {
            self.detect_patterns(ctx, damage);
        }
    }

    /// `RecordCastStart` (target may be absent → guid low 0).
    pub fn record_cast_start(
        &mut self,
        ctx: RecordContext,
        caster_guid_low: u64,
        target_guid_low: u64,
        spell_id: u32,
    ) {
        if !self.active {
            return;
        }
        self.buffer.push(CombatEvent {
            timestamp: ctx.now_ms,
            event_type: CombatEventType::CastStart,
            source_guid_low: caster_guid_low,
            target_guid_low,
            spell_id,
            damage: 0,
            creature_entry: ctx.creature_entry,
        });
    }

    /// `RecordDeath`.
    pub fn record_death(
        &mut self,
        ctx: RecordContext,
        killer_guid_low: u64,
        victim_guid_low: u64,
        victim_is_player: bool,
    ) {
        if !self.active {
            return;
        }
        self.buffer.push(CombatEvent {
            timestamp: ctx.now_ms,
            event_type: CombatEventType::Death,
            source_guid_low: killer_guid_low,
            target_guid_low: victim_guid_low,
            spell_id: 0,
            damage: 0,
            creature_entry: ctx.creature_entry,
        });

        // Death is a strong signal — boosted confidence.
        if victim_is_player {
            self.detect_patterns(ctx, 0);
        }
    }

    /// `DetectPatterns` (`damage == 0` means the death path).
    fn detect_patterns(&mut self, ctx: RecordContext, damage: u32) {
        let creature_entry = ctx.creature_entry;
        if creature_entry == 0 {
            return;
        }
        let now = ctx.now_ms;
        let is_death = damage == 0;

        // First pass: most recent CAST_START from this creature
        // within 5 seconds.
        let mut found_spell = 0u32;
        self.buffer.scan_backwards(|evt| {
            if now.wrapping_sub(evt.timestamp) > DETECT_WINDOW_MS {
                return false;
            }
            if evt.creature_entry == creature_entry && evt.event_type == CombatEventType::CastStart
            {
                found_spell = evt.spell_id;
                return false;
            }
            true
        });
        if found_spell == 0 {
            return;
        }

        // Second pass: count damage hits from that spell in the
        // window (u8 counter, wrapping — as in C++).
        let mut hit_count: u8 = 0;
        self.buffer.scan_backwards(|evt| {
            if now.wrapping_sub(evt.timestamp) > DETECT_WINDOW_MS {
                return false;
            }
            if evt.creature_entry == creature_entry
                && evt.event_type == CombatEventType::Damage
                && evt.spell_id == found_spell
            {
                hit_count = hit_count.wrapping_add(1);
            }
            true
        });

        let (response, param) = if hit_count >= AOE_HIT_THRESHOLD {
            (PatternResponse::MoveAway, 12.0) // default avoidance range
        } else {
            (PatternResponse::Interrupt, 0.0)
        };

        self.add_or_update_pattern(LearnedPattern {
            creature_entry,
            trigger_spell_id: found_spell,
            response_type: response,
            response_param: param,
            confidence: if is_death { 3 } else { 1 },
            last_seen: ctx.now_unix,
            permanent: false,
        });
    }

    /// `AddOrUpdatePattern`.
    pub fn add_or_update_pattern(&mut self, p: LearnedPattern) {
        let vec = self.patterns.entry(p.creature_entry).or_default();
        for existing in &mut *vec {
            if existing.trigger_spell_id == p.trigger_spell_id
                && existing.response_type == p.response_type
            {
                if !existing.permanent {
                    existing.confidence = (u16::from(existing.confidence) + u16::from(p.confidence))
                        .min(u16::from(LEARNED_CONFIDENCE_CAP))
                        as u8;
                    existing.last_seen = p.last_seen;
                    if p.response_param > 0.0 {
                        existing.response_param = p.response_param;
                    }
                }
                return;
            }
        }
        vec.push(p);
    }

    /// `GetActivePattern`: first match with confidence ≥ 2.
    #[must_use]
    pub fn active_pattern(&self, creature_entry: u32, spell_id: u32) -> Option<&LearnedPattern> {
        self.patterns
            .get(&creature_entry)?
            .iter()
            .find(|p| p.trigger_spell_id == spell_id && p.confidence >= ACTIVE_MIN_CONFIDENCE)
    }

    /// `HasPatternsForCreature`.
    #[must_use]
    pub fn has_patterns_for(&self, creature_entry: u32) -> bool {
        self.patterns
            .get(&creature_entry)
            .is_some_and(|v| !v.is_empty())
    }

    /// `DecayPatterns(now)`: −1 confidence per call for patterns
    /// unseen for 30 minutes; zero-confidence learned patterns
    /// are erased (in the same call that brings them to zero,
    /// as the C++ mutating `remove_if` does). Permanent patterns
    /// never decay.
    pub fn decay_patterns(&mut self, now_unix: u32) {
        self.patterns.retain(|_, vec| {
            vec.retain_mut(|p| {
                if p.permanent {
                    return true;
                }
                if now_unix.wrapping_sub(p.last_seen) > DECAY_AFTER_SECS && p.confidence > 0 {
                    p.confidence -= 1;
                }
                p.confidence != 0
            });
            !vec.is_empty()
        });
    }

    /// `RegisterPermanentPattern`: an existing (spell, response)
    /// pair is upgraded in place (permanent, confidence 255,
    /// param overwritten; `last_seen` NOT refreshed).
    pub fn register_permanent_pattern(&mut self, p: LearnedPattern) {
        let vec = self.patterns.entry(p.creature_entry).or_default();
        for existing in &mut *vec {
            if existing.trigger_spell_id == p.trigger_spell_id
                && existing.response_type == p.response_type
            {
                existing.permanent = true;
                existing.confidence = 255;
                existing.response_param = p.response_param;
                return;
            }
        }
        vec.push(p);
    }

    /// `InitEncounterOverrides`: the hand-crafted registry
    /// migrated from the old boss override table.
    pub fn init_encounter_overrides(&mut self, now_unix: u32) {
        for (creature, spell) in [
            (11518, 11966), // Jergosh the Invoker — Shadow Bolt Volley
            (3673, 9532),   // Lord Serpentis — Lightning Bolt
            (3673, 11986),  // Lord Serpentis — Healing Touch
            (3887, 7068),   // Baron Silverlaine — Veil of Shadow
            (4831, 8435),   // Lady Sarevess — Forked Lightning
            (6235, 11085),  // Electrocutioner 6000 — Chain Lightning
            (3977, 12039),  // Whitemane — Heal
            (3977, 22187),  // Whitemane — Greater Heal
        ] {
            self.register_permanent_pattern(LearnedPattern {
                creature_entry: creature,
                trigger_spell_id: spell,
                response_type: PatternResponse::Interrupt,
                response_param: 0.0,
                confidence: 255,
                last_seen: now_unix,
                permanent: true,
            });
        }
        for (creature, spell, range) in [
            (11520, 11969, 10.0), // Taragaman the Hungerer — Fire Nova
            (3975, 8989, 10.0),   // Herod — Whirlwind
        ] {
            self.register_permanent_pattern(LearnedPattern {
                creature_entry: creature,
                trigger_spell_id: spell,
                response_type: PatternResponse::MoveAway,
                response_param: range,
                confidence: 255,
                last_seen: now_unix,
                permanent: true,
            });
        }
        // Mr. Smite — save cooldowns for phase 3 at 33% HP.
        self.register_permanent_pattern(LearnedPattern {
            creature_entry: 646,
            trigger_spell_id: 0,
            response_type: PatternResponse::SaveCd,
            response_param: 33.0,
            confidence: 255,
            last_seen: now_unix,
            permanent: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(now_ms: u32) -> RecordContext {
        RecordContext {
            now_ms,
            now_unix: 1_000_000,
            creature_entry: 999,
        }
    }

    fn active_store() -> PatternStore {
        let mut s = PatternStore::new();
        s.set_active(true);
        s
    }

    /// Cast at t=1000, big damage at `t`, returns store.
    fn cast_then_hit(t: u32, damage: u32) -> PatternStore {
        let mut s = active_store();
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        s.record_damage(ctx(t), 501, 601, true, 1000, damage, 4242);
        s
    }

    #[test]
    fn inactive_store_records_nothing() {
        let mut s = PatternStore::new(); // inactive by default
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        s.record_damage(ctx(2000), 501, 601, true, 1000, 500, 4242);
        assert_eq!(s.pattern_count(), 0);
    }

    #[test]
    fn damage_threshold_is_fifteen_percent_strict() {
        // 15% of 1000 = 150; damage == 150 does not trigger, 151 does.
        assert_eq!(cast_then_hit(2000, 150).pattern_count(), 0);
        assert_eq!(cast_then_hit(2000, 151).pattern_count(), 1);
        // Non-player victims never trigger detection.
        let mut s = active_store();
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        s.record_damage(ctx(2000), 501, 601, false, 1000, 9999, 4242);
        assert_eq!(s.pattern_count(), 0);
    }

    #[test]
    fn confidence_accumulates_until_active() {
        let mut s = cast_then_hit(2000, 500);
        // confidence 1: recorded but not active yet
        assert!(s.active_pattern(999, 4242).is_none());
        // second sighting: confidence 2 -> active Interrupt
        s.record_damage(ctx(3000), 501, 601, true, 1000, 500, 4242);
        let p = s.active_pattern(999, 4242).unwrap();
        assert_eq!(p.response_type, PatternResponse::Interrupt);
        assert_eq!(p.confidence, 2);
        assert!(s.has_patterns_for(999));
        assert!(!s.has_patterns_for(1234));
    }

    #[test]
    fn cast_outside_window_not_detected() {
        assert_eq!(cast_then_hit(6001, 500).pattern_count(), 0); // 5001 ms later
        assert_eq!(cast_then_hit(6000, 500).pattern_count(), 1); // exactly 5000
    }

    #[test]
    fn aoe_classified_at_three_hits() {
        let mut s = active_store();
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        // Two hits on other players first (below detect threshold each).
        s.record_damage(ctx(1500), 501, 602, true, 100_000, 500, 4242);
        s.record_damage(ctx(1500), 501, 603, true, 100_000, 500, 4242);
        // Triggering hit: third damage event with the spell.
        s.record_damage(ctx(1600), 501, 601, true, 1000, 500, 4242);
        let p = s.all_patterns().next().unwrap();
        assert_eq!(p.response_type, PatternResponse::MoveAway);
        assert_eq!(p.response_param.to_bits(), 12.0f32.to_bits());
    }

    #[test]
    fn death_boosts_confidence_to_three() {
        let mut s = active_store();
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        s.record_death(ctx(2000), 501, 601, true);
        let p = s.active_pattern(999, 4242).unwrap();
        assert_eq!(p.confidence, 3, "active immediately after a death");
    }

    #[test]
    fn full_buffer_still_detects_with_cast_as_oldest() {
        // Buffer exactly full: cast + 254 filler hits + trigger =
        // 256 events; cast survives as the oldest slot, 255 hits
        // counted (the C++ u8 counter cannot wrap: the cast event
        // itself occupies one slot).
        let mut s = active_store();
        s.record_cast_start(ctx(1000), 501, 601, 4242);
        for _ in 0..254 {
            s.record_damage(ctx(1500), 501, 602, true, 100_000, 500, 4242);
        }
        s.record_damage(ctx(1600), 501, 601, true, 1000, 500, 4242);
        let p = s.all_patterns().next().unwrap();
        assert_eq!(p.response_type, PatternResponse::MoveAway);
    }

    #[test]
    fn update_caps_confidence_and_keeps_param_when_zero() {
        let mut s = active_store();
        s.add_or_update_pattern(LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::MoveAway,
            response_param: 15.0,
            confidence: 250,
            last_seen: 10,
            permanent: false,
        });
        s.add_or_update_pattern(LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::MoveAway,
            response_param: 0.0, // must NOT overwrite
            confidence: 10,
            last_seen: 20,
            permanent: false,
        });
        let p = s.all_patterns().next().unwrap();
        assert_eq!(p.confidence, 254);
        assert_eq!(p.response_param.to_bits(), 15.0f32.to_bits());
        assert_eq!(p.last_seen, 20);
    }

    #[test]
    fn decay_steps_and_erases_but_never_permanent() {
        let mut s = active_store();
        let base = LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::Interrupt,
            response_param: 0.0,
            confidence: 2,
            last_seen: 1000,
            permanent: false,
        };
        s.load_pattern(base);
        s.load_pattern(LearnedPattern {
            trigger_spell_id: 2,
            confidence: 255,
            permanent: true,
            ..base
        });

        s.decay_patterns(1000 + 1800); // exactly 30 min: not > 1800
        assert_eq!(
            s.all_patterns()
                .find(|p| p.trigger_spell_id == 1)
                .unwrap()
                .confidence,
            2
        );
        s.decay_patterns(1000 + 1801);
        assert_eq!(
            s.all_patterns()
                .find(|p| p.trigger_spell_id == 1)
                .unwrap()
                .confidence,
            1
        );
        s.decay_patterns(1000 + 1802); // hits 0 and erases in the same call
        assert!(s.all_patterns().all(|p| p.trigger_spell_id != 1));
        assert_eq!(s.pattern_count(), 1, "permanent survives");
        assert_eq!(s.creature_count(), 1);
    }

    #[test]
    fn encounter_overrides_match_cpp_registry() {
        let mut s = PatternStore::new();
        s.init_encounter_overrides(42);
        assert_eq!(s.pattern_count(), 11);
        assert_eq!(s.creature_count(), 9);
        let smite = s.all_patterns().find(|p| p.creature_entry == 646).unwrap();
        assert_eq!(smite.response_type, PatternResponse::SaveCd);
        assert_eq!(smite.response_param.to_bits(), 33.0f32.to_bits());
        let herod = s.active_pattern(3975, 8989).unwrap();
        assert_eq!(herod.response_type, PatternResponse::MoveAway);
        assert_eq!(herod.confidence, 255);
        assert!(herod.permanent);
        // idempotent re-init (LoadFromDB calls it again)
        s.init_encounter_overrides(43);
        assert_eq!(s.pattern_count(), 11);
    }

    #[test]
    fn permanent_upgrade_does_not_refresh_last_seen() {
        let mut s = active_store();
        s.load_pattern(LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::Interrupt,
            response_param: 0.0,
            confidence: 3,
            last_seen: 111,
            permanent: false,
        });
        // Learned updates on a later-permanent are ignored after upgrade.
        s.register_permanent_pattern(LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::Interrupt,
            response_param: 0.0,
            confidence: 255,
            last_seen: 222,
            permanent: true,
        });
        s.add_or_update_pattern(LearnedPattern {
            creature_entry: 999,
            trigger_spell_id: 1,
            response_type: PatternResponse::Interrupt,
            response_param: 99.0,
            confidence: 5,
            last_seen: 333,
            permanent: false,
        });
        let p = s.all_patterns().next().unwrap();
        assert!(p.permanent);
        assert_eq!(p.confidence, 255);
        assert_eq!(p.last_seen, 111);
        assert_eq!(p.response_param.to_bits(), 0.0f32.to_bits());
    }
}
