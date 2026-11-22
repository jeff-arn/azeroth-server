//! Scarlet Monastery (Tirisfal) creature scripts.
//!
//! Constants, per-script state, and `Map` bodies for
//! Arcanist Doan, High Inquisitor Fairbanks,
//! Houndmaster Loksey, and Interrogator Vishas. Shared
//! content-script dispatch and encounter constants stay
//! in the parent `content` module.

#[allow(clippy::wildcard_imports)]
use super::*;

// ===== lane cai_tirisfal (consts + state) =====
// Scratch transcription: Scarlet Monastery (Tirisfal) CreatureAI scripts -> Rust (Tier 3).
// Fork files: vmangos/src/scripts/eastern_kingdoms/tirisfal_glades/scarlet_monastery/
//   {boss_arcanist_doan, boss_herod, boss_high_inquisitor_fairbanks,
//   boss_houndmaster_loksey, boss_interrogator_vishas}.cpp (+ scarlet_monastery.h).
// Mirror: content.rs CreatureScriptAi/for_script, HerodAi/ScarletTrainee
//   patterns, script_combat_drive, do_cast_spell_if_can, instance_* accessors.
//
// Per-script census {script_name, suggested variant, state struct, hooks}
// - boss_arcanist_doan -> ArcanistDoan -> ArcanistDoanAi,
//   hooks: reset / aggro / update.
// - boss_herod -> Herod -> HerodAi (ALREADY IN content.rs, cpp 36-241;
//   not retranscribed): hooks reset / aggro / killed_unit / just_summoned /
//   enter_evade (pre-base) / just_died / update.
// - mob_scarlet_trainee -> ScarletTrainee -> ScarletTraineeAi (ALREADY IN
//   content.rs, cpp 245-351; not retranscribed): hooks reset / update /
//   movement_inform.
// - boss_high_inquisitor_fairbanks -> HighInquisitorFairbanks ->
//   HighInquisitorFairbanksAi, hooks: reset / spell_hit (NO-DISPATCHER-YET) /
//   update.
// - boss_houndmaster_loksey -> HoundmasterLoksey -> HoundmasterLokseyAi,
//   hooks: reset / aggro / update.
// - boss_interrogator_vishas -> InterrogatorVishas -> InterrogatorVishasAi,
//   hooks: reset / aggro / killed_unit / just_died / update.
// - go_herod_lever -> GO kind (cpp 357-375), already ported as GoAiState::
//   HerodLever in content.rs; not transcribed (see other-kinds census).
//
// Reused from content.rs (NOT redefined): GO_STATE_ACTIVE/GO_STATE_READY,
//   SM_DATA_VORREL (scarlet_monastery.h DATA_VORREL), SM_SPELL_AB_EFFECT
//   (28441, fairbanks SpellHit gate), SHEATH_UNARMED, STAND_STATE_STAND,
//   CastTarget, CastOptions, Position, offsets, script_combat_drive,
//   do_cast_spell_if_can, do_script_text, cast_spell, instance_get_data64,
//   unit_victim, unit_is_alive, unit_position, object_position,
//   object_get_field, object_set_field, unit_set_byte, bot_has_aura,
//   bot_casting_spell, do_flee, select_random_hostile (lane convention;
//   NOT yet defined in-tree: NEEDS-MAP-HELPER, integrator adds).
//
// Dropped (no const/field emitted; unused = clippy error at integration):
// - boss_herod.cpp GO_HEROD_DOOR (101854): already HL_GO_HEROD_DOOR in
//   content.rs; herod bodies already ported.
// - vishas `m_pInstance` member: instance access is map-level
//   (`instance_get_data64`); no per-script field.
// - fairbanks `IsWithinLOSInMap` gate and `SetFacingToObject`: no Map
//   helper; kept as NEEDS-MAP-HELPER comments at the fork position.

// ---- `boss_arcanist_doan` (`boss_arcanist_doan.cpp` enum, cpp 26-36) ----
const DOAN_SAY_AGGRO: i32 = 6199;
const DOAN_SAY_BURN_IN_FIRE: i32 = 6200;
const DOAN_SPELL_POLYMORPH: u32 = 13_323;
const DOAN_SPELL_AOE_SILENCE: u32 = 8988;
const DOAN_SPELL_ARCANE_EXPLOSION: u32 = 9433;
const DOAN_SPELL_FIRE_AOE: u32 = 9435;
const DOAN_SPELL_ARCANE_BUBBLE: u32 = 9438;

/// `boss_arcanist_doanAI` state (fork member order, cpp 42-46).
pub(crate) struct ArcanistDoanAi {
    polymorph_timer_ms: u32,
    aoe_silence_timer_ms: u32,
    arcane_explosion_timer_ms: u32,
    can_detonate: bool,
    shielded: bool,
}

impl ArcanistDoanAi {
    pub(super) fn new() -> Self {
        Self {
            polymorph_timer_ms: 20_000,
            aoe_silence_timer_ms: 15_000,
            arcane_explosion_timer_ms: 3000,
            can_detonate: false,
            shielded: false,
        }
    }
}

// ---- `boss_high_inquisitor_fairbanks` (cpp 26-34) ----
const FAIRBANKS_SPELL_CURSE_OF_BLOOD: u32 = 8282;
const FAIRBANKS_SPELL_DISPEL_MAGIC: u32 = 15_090;
const FAIRBANKS_SPELL_FEAR: u32 = 12_096;
const FAIRBANKS_SPELL_HEAL: u32 = 12_039;
const FAIRBANKS_SPELL_POWER_WORD_SHIELD: u32 = 11_647;
const FAIRBANKS_SPELL_SLEEP: u32 = 8399;
/// Transform-visual cast in `SpellHit` (cpp 65; spell 28443, triggered).
const FAIRBANKS_SPELL_TRANSFORM: u32 = 28_443;
// `UNIT_NPC_FLAG_GOSSIP` (`UnitDefines.h:653`); read-modify-write through
// `UNIT_NPC_FLAGS` (NAGMARA precedent in cai_brd.rs).
const FAIRBANKS_UNIT_NPC_FLAG_GOSSIP: u32 = 0x01;

/// `boss_high_inquisitor_fairbanksAI` state (fork member order, cpp 40-48).
pub(crate) struct HighInquisitorFairbanksAi {
    curse_of_blood_timer_ms: u32,
    dispel_magic_timer_ms: u32,
    fear_timer_ms: u32,
    heal_timer_ms: u32,
    sleep_timer_ms: u32,
    dispel_timer_ms: u32,
    power_word_shield: bool,
    ashbringer: bool,
}

impl HighInquisitorFairbanksAi {
    pub(super) fn new() -> Self {
        Self {
            curse_of_blood_timer_ms: 10_000,
            dispel_magic_timer_ms: 30_000,
            fear_timer_ms: 40_000,
            heal_timer_ms: 30_000,
            sleep_timer_ms: 30_000,
            dispel_timer_ms: 20_000,
            power_word_shield: false,
            ashbringer: false,
        }
    }
}

// ---- `boss_houndmaster_loksey` (cpp 26-31) ----
const LOKSEY_SAY_AGGRO: i32 = 2655;
const LOKSEY_SPELL_SUMMON_SCARLET_HOUND: u32 = 17_164;
const LOKSEY_SPELL_BLOODLUST: u32 = 6742;

/// `boss_houndmaster_lokseyAI` state (cpp 37).
pub(crate) struct HoundmasterLokseyAi {
    bloodlust_timer_ms: u32,
}

impl HoundmasterLokseyAi {
    pub(super) fn new() -> Self {
        Self {
            bloodlust_timer_ms: 20_000,
        }
    }
}

// ---- `boss_interrogator_vishas` (cpp 26-36) ----
const VISHAS_SAY_AGGRO: i32 = 6204;
const VISHAS_SAY_HEALTH1: i32 = 6206;
const VISHAS_SAY_HEALTH2: i32 = 6207;
const VISHAS_SAY_KILL: i32 = 6205;
const VISHAS_SAY_TRIGGER_VORREL: i32 = 1376;
const VISHAS_SPELL_SHADOW_WORD_PAIN: u32 = 2767;

/// `boss_interrogator_vishasAI` state (fork member order, cpp 48-51).
pub(crate) struct InterrogatorVishasAi {
    yell30: bool,
    yell60: bool,
    shadow_word_pain_timer_ms: u32,
}

impl InterrogatorVishasAi {
    pub(super) fn new() -> Self {
        Self {
            yell30: false,
            yell60: false,
            shadow_word_pain_timer_ms: 5000,
        }
    }
}

impl Map {
    // ===== lane cai_tirisfal (bodies) =====
    fn arcanist_doan_state_mut(&mut self, guid: ObjectGuid) -> Option<&mut ArcanistDoanAi> {
        self.ai.get_mut(&guid).and_then(|ai| match &mut ai.script {
            Some(CreatureScriptAi::ArcanistDoan(s)) => Some(s),
            _ => None,
        })
    }

    fn high_inquisitor_fairbanks_state_mut(
        &mut self,
        guid: ObjectGuid,
    ) -> Option<&mut HighInquisitorFairbanksAi> {
        self.ai.get_mut(&guid).and_then(|ai| match &mut ai.script {
            Some(CreatureScriptAi::HighInquisitorFairbanks(s)) => Some(s),
            _ => None,
        })
    }

    fn houndmaster_loksey_state_mut(
        &mut self,
        guid: ObjectGuid,
    ) -> Option<&mut HoundmasterLokseyAi> {
        self.ai.get_mut(&guid).and_then(|ai| match &mut ai.script {
            Some(CreatureScriptAi::HoundmasterLoksey(s)) => Some(s),
            _ => None,
        })
    }

    fn interrogator_vishas_state_mut(
        &mut self,
        guid: ObjectGuid,
    ) -> Option<&mut InterrogatorVishasAi> {
        self.ai.get_mut(&guid).and_then(|ai| match &mut ai.script {
            Some(CreatureScriptAi::InterrogatorVishas(s)) => Some(s),
            _ => None,
        })
    }

    /// `boss_arcanist_doanAI::Reset` (cpp 48-55).
    pub(super) fn arcanist_doan_reset(&mut self, guid: ObjectGuid) {
        if let Some(s) = self.arcanist_doan_state_mut(guid) {
            s.polymorph_timer_ms = 20_000;
            s.aoe_silence_timer_ms = 15_000;
            s.arcane_explosion_timer_ms = 3000;
            s.can_detonate = false;
            s.shielded = false;
        }
    }

    /// `boss_arcanist_doanAI::Aggro` (cpp 57).
    pub(super) fn arcanist_doan_aggro(&mut self, guid: ObjectGuid, _who: ObjectGuid) {
        self.do_script_text(DOAN_SAY_AGGRO, guid, None, -1);
    }

    /// `boss_arcanist_doanAI::UpdateAI` (cpp 59-116).
    pub(super) fn arcanist_doan_update(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let Some(victim) = self.script_combat_drive(guid, true) else {
            return;
        };
        // Detonate the bubble (fork ignores the cast result; the aura
        // check below gates further casts while shielded).
        let detonate = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::ArcanistDoan(s)) => Some(s.shielded && s.can_detonate),
                _ => None,
            })
            .unwrap_or(false);
        if detonate {
            self.do_script_text(DOAN_SAY_BURN_IN_FIRE, guid, None, -1);
            self.do_cast_spell_if_can(guid, DOAN_SPELL_FIRE_AOE, CastTarget::SelfCast);
            if let Some(s) = self.arcanist_doan_state_mut(guid) {
                s.can_detonate = false;
            }
        }
        // `HasAura(SPELL_ARCANEBUBBLE)` (fork): no bubble interaction
        // while the shield aura is up.
        if self.bot_has_aura(guid, DOAN_SPELL_ARCANE_BUBBLE) {
            return;
        }
        // Shield at <=50% (fork `IsNonMeleeSpellCasted` ~ active cast;
        // fork sets both flags unconditionally after the cast attempt).
        let shielded = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::ArcanistDoan(s)) => Some(s.shielded),
                _ => None,
            })
            .unwrap_or(true);
        if !shielded
            && self.creature_health_percent(guid) <= 50.0
            && self.bot_casting_spell(guid).is_none()
        {
            self.do_cast_spell_if_can(guid, DOAN_SPELL_ARCANE_BUBBLE, CastTarget::SelfCast);
            if let Some(s) = self.arcanist_doan_state_mut(guid) {
                s.can_detonate = true;
                s.shielded = true;
            }
        }
        // Polymorph a random target (fork resets the timer even with no
        // target / failed cast).
        let poly_due = {
            let Some(s) = self.arcanist_doan_state_mut(guid) else {
                return;
            };
            if s.polymorph_timer_ms < diff_ms {
                true
            } else {
                s.polymorph_timer_ms -= diff_ms;
                false
            }
        };
        if poly_due {
            // NEEDS-MAP-HELPER: fork `SelectAttackingTarget(ATTACKING_TARGET_RANDOM, 1)`.
            if let Some(target) = self.select_random_hostile(guid) {
                self.do_cast_spell_if_can(guid, DOAN_SPELL_POLYMORPH, CastTarget::Unit(target));
            }
            if let Some(s) = self.arcanist_doan_state_mut(guid) {
                s.polymorph_timer_ms = 20_000;
            }
        }
        // AoE silence on the victim.
        let silence_due = {
            let Some(s) = self.arcanist_doan_state_mut(guid) else {
                return;
            };
            if s.aoe_silence_timer_ms < diff_ms {
                true
            } else {
                s.aoe_silence_timer_ms -= diff_ms;
                false
            }
        };
        if silence_due {
            self.do_cast_spell_if_can(guid, DOAN_SPELL_AOE_SILENCE, CastTarget::Unit(victim));
            let next = self.rng.urand(15_000, 20_000);
            if let Some(s) = self.arcanist_doan_state_mut(guid) {
                s.aoe_silence_timer_ms = next;
            }
        }
        // Arcane explosion on the victim.
        let explosion_due = {
            let Some(s) = self.arcanist_doan_state_mut(guid) else {
                return;
            };
            if s.arcane_explosion_timer_ms < diff_ms {
                true
            } else {
                s.arcane_explosion_timer_ms -= diff_ms;
                false
            }
        };
        if explosion_due {
            self.do_cast_spell_if_can(guid, DOAN_SPELL_ARCANE_EXPLOSION, CastTarget::Unit(victim));
            if let Some(s) = self.arcanist_doan_state_mut(guid) {
                s.arcane_explosion_timer_ms = 8000;
            }
        }
    }

    /// `boss_high_inquisitor_fairbanksAI::Reset` (cpp 49-59).
    pub(super) fn high_inquisitor_fairbanks_reset(&mut self, guid: ObjectGuid) {
        if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
            s.curse_of_blood_timer_ms = 10_000;
            s.dispel_magic_timer_ms = 30_000;
            s.fear_timer_ms = 40_000;
            s.heal_timer_ms = 30_000;
            s.sleep_timer_ms = 30_000;
            s.dispel_timer_ms = 20_000;
            s.power_word_shield = false;
            s.ashbringer = false;
        }
    }

    /// `boss_high_inquisitor_fairbanksAI::SpellHit` (cpp 61-71).
    /// NO-DISPATCHER-YET (no `SpellHit` dispatcher in content.rs).
    pub(super) fn high_inquisitor_fairbanks_spell_hit(
        &mut self,
        guid: ObjectGuid,
        caster: ObjectGuid,
        spell_id: u32,
    ) {
        if spell_id != SM_SPELL_AB_EFFECT {
            return;
        }
        let ashbringer = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::HighInquisitorFairbanks(s)) => Some(s.ashbringer),
                _ => None,
            })
            .unwrap_or(true);
        if ashbringer || caster.is_empty() {
            return;
        }
        // NEEDS-MAP-HELPER: fork `me->IsWithinLOSInMap(pCaster)` (cpp 63):
        // no LOS helper on Map.
        // `SetSheath(SHEATH_STATE_UNARMED)` via the worn-bytes helper
        // (smite equip precedent).
        self.unit_set_byte(guid, offsets::UNIT_FIELD_BYTES_2, 0, SHEATH_UNARMED);
        // `CastSpell(me, 28443, true)`: triggered self cast.
        let _ = self.cast_spell(
            guid,
            FAIRBANKS_SPELL_TRANSFORM,
            CastTarget::SelfCast,
            CastOptions {
                triggered: true,
                skip_known: true,
                ..Default::default()
            },
        );
        // NEEDS-MAP-HELPER: `SetFacingToObject(pCaster)` (cpp 67): no
        // facing helper on Map.
        let flags = self.object_get_field(guid, offsets::UNIT_NPC_FLAGS);
        let _ = self.object_set_field(
            guid,
            u32::from(offsets::UNIT_NPC_FLAGS),
            flags | FAIRBANKS_UNIT_NPC_FLAG_GOSSIP,
        );
        if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
            s.ashbringer = true;
        }
    }

    /// `boss_high_inquisitor_fairbanksAI::UpdateAI` (cpp 73-137).
    #[allow(clippy::too_many_lines)]
    pub(super) fn high_inquisitor_fairbanks_update(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let Some(victim) = self.script_combat_drive(guid, true) else {
            return;
        };
        // Heal at <=25% (fork `else` decrements even when the health /
        // cast gate is closed; transcribed exactly).
        let low_health = self.creature_health_percent(guid) <= 25.0;
        let casting = self.bot_casting_spell(guid).is_some();
        let heal_due = {
            let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) else {
                return;
            };
            if low_health && !casting && s.heal_timer_ms < diff_ms {
                true
            } else {
                s.heal_timer_ms -= diff_ms;
                false
            }
        };
        if heal_due {
            self.do_cast_spell_if_can(guid, FAIRBANKS_SPELL_HEAL, CastTarget::SelfCast);
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.heal_timer_ms = 30_000;
            }
        }
        // Fear a random target.
        let fear_due = {
            let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) else {
                return;
            };
            if s.fear_timer_ms < diff_ms {
                true
            } else {
                s.fear_timer_ms -= diff_ms;
                false
            }
        };
        if fear_due {
            // NEEDS-MAP-HELPER: fork `SelectAttackingTarget(ATTACKING_TARGET_RANDOM, 1)`.
            if let Some(target) = self.select_random_hostile(guid) {
                self.do_cast_spell_if_can(guid, FAIRBANKS_SPELL_FEAR, CastTarget::Unit(target));
            }
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.fear_timer_ms = 40_000;
            }
        }
        // Sleep the top-aggro target.
        let sleep_due = {
            let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) else {
                return;
            };
            if s.sleep_timer_ms < diff_ms {
                true
            } else {
                s.sleep_timer_ms -= diff_ms;
                false
            }
        };
        if sleep_due {
            // NEEDS-MAP-HELPER: fork `SelectAttackingTarget(ATTACKING_TARGET_TOPAGGRO, 0)`;
            // transcribed as the drive victim (highest threat in practice).
            self.do_cast_spell_if_can(guid, FAIRBANKS_SPELL_SLEEP, CastTarget::Unit(victim));
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.sleep_timer_ms = 30_000;
            }
        }
        // Shield at <=25%, once (fork sets the flag unconditionally
        // after the cast attempt).
        let shielded = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::HighInquisitorFairbanks(s)) => Some(s.power_word_shield),
                _ => None,
            })
            .unwrap_or(true);
        if !shielded && self.creature_health_percent(guid) <= 25.0 {
            self.do_cast_spell_if_can(
                guid,
                FAIRBANKS_SPELL_POWER_WORD_SHIELD,
                CastTarget::SelfCast,
            );
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.power_word_shield = true;
            }
        }
        // Dispel a random target. FORK BUG (cpp 117-125): the due branch
        // resets `DispelMagic_Timer` while the check and the `else`
        // decrement run against `Dispel_Timer`, so `Dispel_Timer` never
        // ticks down; transcribed exactly.
        let dispel_due = {
            let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) else {
                return;
            };
            if s.dispel_timer_ms < diff_ms {
                true
            } else {
                s.dispel_magic_timer_ms -= diff_ms;
                false
            }
        };
        if dispel_due {
            // NEEDS-MAP-HELPER: fork `SelectAttackingTarget(ATTACKING_TARGET_RANDOM, 0)`.
            if let Some(target) = self.select_random_hostile(guid) {
                self.do_cast_spell_if_can(
                    guid,
                    FAIRBANKS_SPELL_DISPEL_MAGIC,
                    CastTarget::Unit(target),
                );
            }
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.dispel_magic_timer_ms = 30_000;
            }
        }
        // Curse of Blood on the victim.
        let curse_due = {
            let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) else {
                return;
            };
            if s.curse_of_blood_timer_ms < diff_ms {
                true
            } else {
                s.curse_of_blood_timer_ms -= diff_ms;
                false
            }
        };
        if curse_due {
            self.do_cast_spell_if_can(
                guid,
                FAIRBANKS_SPELL_CURSE_OF_BLOOD,
                CastTarget::Unit(victim),
            );
            if let Some(s) = self.high_inquisitor_fairbanks_state_mut(guid) {
                s.curse_of_blood_timer_ms = 25_000;
            }
        }
    }

    /// `boss_houndmaster_lokseyAI::Reset` (cpp 39).
    pub(super) fn houndmaster_loksey_reset(&mut self, guid: ObjectGuid) {
        if let Some(s) = self.houndmaster_loksey_state_mut(guid) {
            s.bloodlust_timer_ms = 20_000;
        }
    }

    /// `boss_houndmaster_lokseyAI::Aggro` (cpp 41-45).
    pub(super) fn houndmaster_loksey_aggro(&mut self, guid: ObjectGuid, _who: ObjectGuid) {
        self.do_script_text(LOKSEY_SAY_AGGRO, guid, None, -1);
        self.do_cast_spell_if_can(
            guid,
            LOKSEY_SPELL_SUMMON_SCARLET_HOUND,
            CastTarget::SelfCast,
        );
    }

    /// `boss_houndmaster_lokseyAI::UpdateAI` (cpp 47-61).
    pub(super) fn houndmaster_loksey_update(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let Some(_victim) = self.script_combat_drive(guid, true) else {
            return;
        };
        let bloodlust_due = {
            let Some(s) = self.houndmaster_loksey_state_mut(guid) else {
                return;
            };
            if s.bloodlust_timer_ms < diff_ms {
                true
            } else {
                s.bloodlust_timer_ms -= diff_ms;
                false
            }
        };
        if bloodlust_due {
            self.do_cast_spell_if_can(guid, LOKSEY_SPELL_BLOODLUST, CastTarget::SelfCast);
            if let Some(s) = self.houndmaster_loksey_state_mut(guid) {
                s.bloodlust_timer_ms = 20_000;
            }
        }
    }

    /// `boss_interrogator_vishasAI::Reset` (cpp 52-57).
    pub(super) fn interrogator_vishas_reset(&mut self, guid: ObjectGuid) {
        if let Some(s) = self.interrogator_vishas_state_mut(guid) {
            s.yell30 = false;
            s.yell60 = false;
            s.shadow_word_pain_timer_ms = 5000;
        }
    }

    /// `boss_interrogator_vishasAI::Aggro` (cpp 59).
    pub(super) fn interrogator_vishas_aggro(&mut self, guid: ObjectGuid, _who: ObjectGuid) {
        self.do_script_text(VISHAS_SAY_AGGRO, guid, None, -1);
    }

    /// `boss_interrogator_vishasAI::KilledUnit` (cpp 61).
    pub(super) fn interrogator_vishas_killed_unit(
        &mut self,
        guid: ObjectGuid,
        _victim: ObjectGuid,
    ) {
        self.do_script_text(VISHAS_SAY_KILL, guid, None, -1);
    }

    /// `boss_interrogator_vishasAI::JustDied` (cpp 63-71): Vorrel speaks
    /// his trigger line. `m_pInstance` null-guard ~ missing instance data.
    pub(super) fn interrogator_vishas_just_died(&mut self, guid: ObjectGuid) {
        let _ = guid;
        let Some(vorrel) = self.instance_get_data64(SM_DATA_VORREL) else {
            return;
        };
        // NEEDS-MAP-HELPER: fork `GetMap()->GetUnit` (any unit type);
        // creature-only lookup here (Vorrel is `SM_NPC_VORREL_SENGUTZ`).
        if !self.creatures.contains_key(&vorrel) {
            return;
        }
        self.do_script_text(VISHAS_SAY_TRIGGER_VORREL, vorrel, None, -1);
    }

    /// `boss_interrogator_vishasAI::UpdateAI` (cpp 73-101).
    pub(super) fn interrogator_vishas_update(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let Some(victim) = self.script_combat_drive(guid, true) else {
            return;
        };
        // Health yells, once each.
        let yell60 = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::InterrogatorVishas(s)) => Some(s.yell60),
                _ => None,
            })
            .unwrap_or(true);
        if !yell60 && self.creature_health_percent(guid) <= 60.0 {
            self.do_script_text(VISHAS_SAY_HEALTH1, guid, None, -1);
            if let Some(s) = self.interrogator_vishas_state_mut(guid) {
                s.yell60 = true;
            }
        }
        let yell30 = self
            .ai
            .get(&guid)
            .and_then(|ai| match &ai.script {
                Some(CreatureScriptAi::InterrogatorVishas(s)) => Some(s.yell30),
                _ => None,
            })
            .unwrap_or(true);
        if !yell30 && self.creature_health_percent(guid) <= 30.0 {
            self.do_script_text(VISHAS_SAY_HEALTH2, guid, None, -1);
            if let Some(s) = self.interrogator_vishas_state_mut(guid) {
                s.yell30 = true;
            }
        }
        // Shadow Word: Pain on the victim.
        let pain_due = {
            let Some(s) = self.interrogator_vishas_state_mut(guid) else {
                return;
            };
            if s.shadow_word_pain_timer_ms < diff_ms {
                true
            } else {
                s.shadow_word_pain_timer_ms -= diff_ms;
                false
            }
        };
        if pain_due {
            self.do_cast_spell_if_can(
                guid,
                VISHAS_SPELL_SHADOW_WORD_PAIN,
                CastTarget::Unit(victim),
            );
            let next = self.rng.urand(5000, 15_000);
            if let Some(s) = self.interrogator_vishas_state_mut(guid) {
                s.shadow_word_pain_timer_ms = next;
            }
        }
    }
}
