//! Behaviour trait + the weighted fallthrough
//! runner (`BotBehavior.h` / `BotBehavior.cpp`
//! `RunBestBehavior`, ported line-for-line
//! including `[BotAI]` debug formats so traces
//! diff against C++).
//!
//! Deliberate deviations:
//! - No silent 32-candidate truncation (sets stay
//!   well under it; `.claude/rules/bots.md` calls
//!   the cap out as a gotcha).
//! - Ties keep registration order (stable sort);
//!   C++ `std::sort` leaves them unspecified.
//! - `Execute` also receives the host: C++
//!   behaviours reach world verbs through the
//!   AI pointer captured at construction; the Rust
//!   host is that surface, passed per tick.

use crate::context::SituationContext;
use crate::host::BotHost;
use crate::spell_kit::SpellKit;

/// One scored behaviour (`BotBehavior`). `Send`
/// because behaviour sets live on the world actor
/// task.
pub trait BotBehavior: Send {
    /// `GetName` (debug label).
    fn name(&self) -> &'static str;

    /// `Score`: 0 = irrelevant, 100 = life-or-death.
    /// The fork's `Score(ctx)` reads live world
    /// state through the AI pointer captured at
    /// construction (group scans, aura checks);
    /// the host parameter is that surface.
    fn score(&self, ctx: &SituationContext, host: &dyn BotHost) -> f32;

    /// `Execute`: `true` consumes the tick.
    fn execute(&mut self, ctx: &SituationContext, kit: &SpellKit, host: &mut dyn BotHost) -> bool;
}

/// Debug trace sink (`.partybot debug` output).
pub trait BehaviorDebug {
    /// One formatted debug line.
    fn line(&mut self, text: String);
}

impl BehaviorDebug for () {
    fn line(&mut self, _text: String) {}
}

struct Entry {
    behavior: Box<dyn BotBehavior>,
    role_weight: f32,
}

/// The per-AI behaviour registry (`BotBehaviorSet`).
#[derive(Default)]
pub struct BehaviorSet {
    entries: Vec<Entry>,
}

impl BehaviorSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `Add` with its role weight (<= 0 disables).
    pub fn add(&mut self, behavior: Box<dyn BotBehavior>, role_weight: f32) {
        self.entries.push(Entry {
            behavior,
            role_weight,
        });
    }

    /// `Size`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no behaviour is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `RunBestBehavior`.
    pub fn run(
        &mut self,
        bot_name: &str,
        ctx: &SituationContext,
        kit: &SpellKit,
        host: &mut dyn BotHost,
        debug: &mut dyn BehaviorDebug,
        debug_enabled: bool,
    ) -> bool {
        let mut candidates: Vec<(usize, f32)> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.role_weight <= 0.0 {
                continue;
            }
            let raw = entry.behavior.score(ctx, host);
            if raw <= 0.0 {
                continue;
            }
            let weighted = raw * entry.role_weight;
            if debug_enabled {
                debug.line(format!(
                    "[BotAI] {bot_name} | {:<20} raw={raw:.1} weight={:.1} \u{2192} {weighted:.1}",
                    entry.behavior.name(),
                    entry.role_weight,
                ));
            }
            candidates.push((index, weighted));
        }

        if candidates.is_empty() {
            if debug_enabled {
                debug.line(format!(
                    "[BotAI] {bot_name} \u{2192} NO BEHAVIOR SCORED (all zero) target={} hp={:.0}% mana={:.0}% role={} inCombat={}",
                    if ctx.current_target.is_some() { "yes" } else { "NO" },
                    ctx.health_pct,
                    ctx.mana_pct,
                    ctx.role.as_u8(),
                    u8::from(ctx.in_combat),
                ));
            }
            return false;
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (index, score) in candidates {
            let name = self.entries[index].behavior.name();
            let result = self.entries[index].behavior.execute(ctx, kit, host);
            if debug_enabled {
                debug.line(format!(
                    "[BotAI] {bot_name} \u{2192} {name} ({score:.1}) {}",
                    if result { "OK" } else { "FAILED" },
                ));
            }
            if result {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{BotChatKind, CastOutcome, UnitView};
    use mangos_guid::ObjectGuid;

    struct FakeHost;

    impl BotHost for FakeHost {
        fn me(&self) -> ObjectGuid {
            ObjectGuid::EMPTY
        }
        fn unit(&self, _guid: ObjectGuid) -> Option<UnitView> {
            None
        }
        fn party_members(&self) -> Vec<ObjectGuid> {
            Vec::new()
        }
        fn can_try_cast(&self, _target: Option<ObjectGuid>, _spell_id: u32) -> bool {
            true
        }
        fn cast_spell(&mut self, _target: Option<ObjectGuid>, _spell_id: u32) -> CastOutcome {
            CastOutcome::Ok
        }
        fn move_chase(&mut self, _target: ObjectGuid, _distance: f32, _angle: f32) {}
        fn move_follow(&mut self, _target: ObjectGuid, _distance: f32, _angle: f32) {}
        fn move_point(&mut self, _x: f32, _y: f32, _z: f32) {}
        fn stop_moving(&mut self) {}
        fn set_target(&mut self, _target: Option<ObjectGuid>) {}
        fn say(&mut self, _kind: BotChatKind, _text: &str) {}
        fn whisper(&mut self, _target: ObjectGuid, _text: &str) {}
        fn attackers_of(&self, _guid: ObjectGuid) -> Vec<ObjectGuid> {
            Vec::new()
        }
        fn has_aura(&self, _guid: ObjectGuid, _spell_id: u32) -> bool {
            false
        }
        fn has_periodic_heal_aura(&self, _guid: ObjectGuid) -> bool {
            false
        }
        fn knows_spell(&self, _spell_id: u32) -> bool {
            false
        }
        fn hostile_count_around(&self, _center: ObjectGuid, _radius: f32) -> u8 {
            0
        }
        fn incoming_damage(&self, _guid: ObjectGuid) -> i32 {
            0
        }
        fn party_tank(&self) -> Option<ObjectGuid> {
            None
        }
        fn party_leader(&self) -> Option<ObjectGuid> {
            None
        }
        fn threat_of(&self, _enemy: ObjectGuid, _who: ObjectGuid) -> f32 {
            0.0
        }
        fn motion_kind(&self) -> crate::host::MotionKind {
            crate::host::MotionKind::Idle
        }
        fn attack_stop(&mut self) {}
        fn can_reach_melee(&self, _target: ObjectGuid) -> bool {
            false
        }
        fn dispel_target(&self, _spell_id: u32) -> Option<ObjectGuid> {
            None
        }
        fn cc_marks(&self) -> Vec<u8> {
            Vec::new()
        }
        fn marked_target(&self, _icon: u8) -> Option<ObjectGuid> {
            None
        }
        fn is_marked(&self, _guid: ObjectGuid) -> bool {
            false
        }
        fn auto_mark_cc(&mut self, _target: ObjectGuid) -> bool {
            false
        }
        fn is_ccd(&self, _guid: ObjectGuid) -> bool {
            false
        }
        fn clear_target(&mut self) {}
        fn has_aura_type(&self, _guid: ObjectGuid, _aura_type: u32) -> bool {
            false
        }
        fn cancel_aura(&mut self, _spell_id: u32) {}
        fn remove_shapeshift(&mut self) {}
        fn pet(&self) -> Option<ObjectGuid> {
            None
        }
        fn pet_attack(&mut self, _target: ObjectGuid) {}
        fn summon_pet_if_needed(&mut self) {}
        fn cast_weapon_buff(&mut self, _spell_id: u32, _offhand: bool) -> bool {
            false
        }
        fn totem_present(&self, _slot: u32) -> bool {
            false
        }
        fn note_totem_cast(&mut self, _slot: u32) {}
        fn face(&mut self, _target: ObjectGuid) {}
        fn pick_index(&mut self, len: usize) -> usize {
            let _ = len;
            0
        }
    }

    struct Fixed {
        name: &'static str,
        score: f32,
        act: bool,
    }

    impl BotBehavior for Fixed {
        fn name(&self) -> &'static str {
            self.name
        }
        fn score(&self, _ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
            self.score
        }
        fn execute(
            &mut self,
            _ctx: &SituationContext,
            _kit: &SpellKit,
            _host: &mut dyn BotHost,
        ) -> bool {
            self.act
        }
    }

    struct Lines(Vec<String>);
    impl BehaviorDebug for Lines {
        fn line(&mut self, text: String) {
            self.0.push(text);
        }
    }

    #[test]
    fn fallthrough_and_weighting() {
        let mut set = BehaviorSet::new();
        set.add(
            Box::new(Fixed {
                name: "A",
                score: 10.0,
                act: false,
            }),
            1.0,
        );
        set.add(
            Box::new(Fixed {
                name: "B",
                score: 20.0,
                act: true,
            }),
            0.25, // weighted 5 < A: A tries first, falls through
        );
        set.add(
            Box::new(Fixed {
                name: "C",
                score: 0.0,
                act: true,
            }),
            1.0,
        );
        let mut lines = Lines(Vec::new());
        let acted = set.run(
            "Testbot",
            &SituationContext::default(),
            &SpellKit::default(),
            &mut FakeHost,
            &mut lines,
            true,
        );
        assert!(acted);
        assert!(
            lines
                .0
                .iter()
                .any(|l| l == "[BotAI] Testbot \u{2192} B (5.0) OK"),
            "{:?}",
            lines.0
        );
        assert!(
            lines
                .0
                .iter()
                .any(|l| l == "[BotAI] Testbot \u{2192} A (10.0) FAILED"),
            "{:?}",
            lines.0
        );
    }

    #[test]
    fn zero_weight_disables() {
        let mut set = BehaviorSet::new();
        set.add(
            Box::new(Fixed {
                name: "Only",
                score: 50.0,
                act: true,
            }),
            0.0,
        );
        let mut lines = Lines(Vec::new());
        assert!(!set.run(
            "Testbot",
            &SituationContext::default(),
            &SpellKit::default(),
            &mut FakeHost,
            &mut lines,
            true
        ));
        assert!(
            lines.0.iter().any(|l| l.contains("NO BEHAVIOR SCORED")),
            "{:?}",
            lines.0
        );
    }
}
