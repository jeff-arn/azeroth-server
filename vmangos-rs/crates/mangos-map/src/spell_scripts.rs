//! Fork `SpellScript` / `AuraScript` host (`ScriptMgr.h:138-215`,
//! Tier 3 Slice 2 step 4): dispatch by
//! `spell_template.script_name` at the cast-pipeline seams the
//! fork calls the virtuals from. Wired hooks are `AuraScript::OnProc`,
//! `OnCheckCast`, `OnInit` (per-cast roll into `script_effect_pick`),
//! `OnEffectExecute` (per effect inside `apply_effects`, i.e.
//! `DoAllEffectOnTarget`; `false` skips the effect, the `spell->damage`
//! mutation rides the `script_damage` channel into the school and weapon
//! damage arms), `OnHit`, `OnSuccessfulStart`, `OnCast`,
//! `OnSuccessfulDispel`, `OnSuccessfulFinish`, `OnAfterHit`,
//! `OnSetTargetMap`, `OnCheckTarget`, `AuraScript::OnBeforeApply` and
//! `OnAfterApply` (holder-level in `add_aura_holder` / `remove_aura_at`,
//! fork `ApplyModifier` order), `OnPeriodicDummy`, `OnPeriodicTrigger`,
//! `OnPeriodicTickEnd` and `OnSummon`. Arms whose hook has no dispatcher
//! yet stay `dead_code` (`NO-DISPATCHER-YET`); their seam notes are
//! proposals, not lanes.

use crate::map::Map;
use mangos_entities::aura::AuraHolder;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::entry::SpellEntry;

use crate::procs::ProcHandlerCtx;
use crate::spells::{AuraRemoveMode, CastItemCtx, CastOptions, CastTarget, PeriodicTick};

mod aura;
mod check_cast;
mod effect_execute;
mod hit_finish;
mod summon;
mod target_map;

impl Map {
    // ---- helpers (impl-Map style, after the arms) ----
    /// `Unit::IsFriendlyTo` subset (no faction/reaction system:
    /// the same player-vs-creature split the spell core uses in
    /// `is_hostile_pair`).
    pub(crate) fn script_is_friendly(a: ObjectGuid, b: ObjectGuid) -> bool {
        !Self::is_hostile_pair(a, b)
    }

    /// `WorldObject::IsFacingTarget` (`Object.cpp:1849`):
    /// under `NO_FACING_CHECKS_DISTANCE` (1.4 to center;
    /// positions here are centers) always facing, else the
    /// PI front arc.
    pub(crate) fn is_facing_target(&self, owner: ObjectGuid, target: ObjectGuid) -> bool {
        let (Some(a), Some(b)) = (self.unit_position(owner), self.unit_position(target)) else {
            return false;
        };
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        if dx * dx + dy * dy < 1.4 * 1.4 {
            return true;
        }
        self.has_in_arc(owner, target, std::f32::consts::PI)
    }

    /// Unit target of a cast (`SpellTargets::getUnitTarget`)
    /// for script hooks; non-unit cast targets have none.
    pub(crate) fn script_unit_target(target: CastTarget) -> Option<ObjectGuid> {
        match target {
            CastTarget::Unit(guid) => Some(guid),
            _ => None,
        }
    }

    /// Shared Immolate-from-caster scan (`spell_warlock.cpp:65-74,87-96`):
    /// victim holder with a `SPELL_AURA_PERIODIC_DAMAGE` effect whose spell
    /// `IsFitToFamily<SPELLFAMILY_WARLOCK, CF_WARLOCK_IMMOLATE>()`
    /// (`SPELLFAMILY_WARLOCK=5`, `DBCEnums.h:199`; `CF_WARLOCK_IMMOLATE` bit 2 =
    /// mask `0x4`, `SpellClassMask.h:97`) and whose caster is `caster`.
    /// Returns the holder's spell id (the `RemoveAurasByCasterSpell` key).
    fn warlock_conflagrate_immolate_id(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> Option<u32> {
        let victim_unit = self.unit_base(victim)?;
        for holder in &victim_unit.auras {
            if holder.caster != caster {
                continue;
            }
            let aura_spell = self.stores.spells.get(holder.spell_id)?;
            if aura_spell.spell_family_name != 5 || (aura_spell.spell_family_flags & 0x4) == 0 {
                continue;
            }
            let has_periodic_damage = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_DAMAGE);
            if has_periodic_damage {
                return Some(holder.spell_id);
            }
        }
        None
    }

    /// Holder scan backing the Dire Bear Form check above
    /// (fork `Unit::HasAura(9634)`; `unit_base` per `motion.rs:591`).
    fn unit_has_aura_spell(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.unit_base(guid)
            .is_some_and(|u| u.auras.iter().any(|h| h.spell_id == spell_id))
    }
}
