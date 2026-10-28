//! Proc system (`SpellCaster::ProcDamageAndSpell` +
//! `Unit::IsTriggeredAtSpellProcEvent` +
//! `Unit::HandleTriggers` subset): damage/heal events
//! are matched against aura holders' proc definitions
//! (`spell_proc_event` rows over DBC proc fields) and
//! executed through the 42/43 handlers, with holder
//! proc charges and the fork's `Spell.ProcDelay`
//! batching (attacker-side instant-target spells run
//! immediately; the rest lands after the delay).
//!
//! The fork's non-NULL `AuraProcHandler` table is
//! ported per aura type (dummy, fear/root damage
//! break, school/mechanic gates, haste, add-target
//! trigger, override class scripts); aura types
//! without a handler return OK like
//! `HandleNULLProc` (charges still burn).
//! Recorded subset deviations: holder cast-item
//! identity is not tracked, so proc casts pass no
//! cast item; equipment-subclass proc gates and the
//! outdoors-only gate are skipped (no item-proto /
//! outdoor plumbing at proc sites); `spell_chain`
//! rank fill uses the DB table only;
//! `SelectRandomUnfriendlyTarget` has no `LoS` or
//! valid-attack-target filter (no `LoS` service).

use mangos_entities::aura::AuraHolder;
use mangos_entities::unit::UNIT_STATE_CAN_NOT_REACT;
use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::defines::{
    aura as aura_type, dmg_class, effect, proc_ex, proc_flag, school, spellmod,
};
use mangos_spells::{ProcEventEntry, SpellEntry};

use crate::Map;
use crate::map::CooldownEntry;
use crate::spells::{CastOptions, CastOutcome, CastTarget};

const ATTR_EX3_NOT_A_PROC: u32 = 0x0000_0200;
const ATTR_EX3_SUPPRESS_CASTER_PROCS: u32 = 0x0001_0000;
const ATTR_EX3_SUPPRESS_TARGET_PROCS: u32 = 0x0002_0000;
const ATTR_EX3_INSTANT_TARGET_PROCS: u32 = 0x0008_0000;
const ATTR_EX3_CAN_PROC_FROM_PROCS: u32 = 0x0400_0000;
const ATTR_EX3_ONLY_PROC_ON_CASTER: u32 = 0x0800_0000;
const ATTR_EX2_PROC_COOLDOWN_ON_FAILURE: u32 = 0x0080_0000;
const ATTR_PROC_FAILURE_BURNS_CHARGE: u32 = 0x0000_0001;
/// `SPELL_ITEM_CLASS_WEAPON` gate anchor: aura types
/// that can never trigger (`HandleCantTrigger`).
const CAN_T_TRIGGER_TYPES: [u32; 2] = [85, aura_type::RESIST_PUSHBACK];

/// Weapon attack type (`WeaponAttackType` subset).
pub(crate) mod att_type {
    pub const BASE: u32 = 0;
    pub const OFF: u32 = 1;
    pub const RANGED: u32 = 2;
}

/// One proc event (`ProcSystemArguments` subset; item
/// and script fields omitted).
#[derive(Clone, Copy)]
pub(crate) struct ProcArgs<'a> {
    /// The other unit (`pVictim`); `None` for
    /// targetless cast-end events.
    pub target: Option<ObjectGuid>,
    pub attacker_flags: u32,
    pub victim_flags: u32,
    pub proc_ex: u32,
    /// Post-resist amount (`amount`).
    pub amount: u32,
    /// Pre-resist/absorb amount (`originalAmount`).
    pub original_amount: u32,
    pub att_type: u32,
    /// The spell that caused the event (none for
    /// melee swings).
    pub proc_spell_id: Option<u32>,
    /// The causing spell was cast from an aura
    /// (`IsTriggeredByAuraOrItem`).
    pub triggered_by_aura: bool,
    /// Map-clock time the event happened (fork
    /// `procTime`, for the apply-time guard).
    pub proc_time_ms: u32,
    /// Charged modifiers already dropped by the
    /// causing cast (`appliedSpellModifiers`).
    pub applied_mods: AppliedMods<'a>,
}

/// A delayed proc check (`m_pendingProcChecks`).
pub(crate) struct PendingProc {
    pub dealer: ObjectGuid,
    pub due_ms: u32,
    pub args: ProcArgsOwned,
}

/// Owned `ProcArgs` for the delay queue (the fork
/// stores the argument struct by value).
#[derive(Clone)]
pub(crate) struct ProcArgsOwned {
    pub target: Option<ObjectGuid>,
    pub attacker_flags: u32,
    pub victim_flags: u32,
    pub proc_ex: u32,
    pub amount: u32,
    pub original_amount: u32,
    pub att_type: u32,
    pub proc_spell_id: Option<u32>,
    pub triggered_by_aura: bool,
    pub proc_time_ms: u32,
    pub applied_mods: Vec<(u32, u8)>,
}

impl ProcArgsOwned {
    fn as_args(&self) -> ProcArgs<'_> {
        ProcArgs {
            target: self.target,
            attacker_flags: self.attacker_flags,
            victim_flags: self.victim_flags,
            proc_ex: self.proc_ex,
            amount: self.amount,
            original_amount: self.original_amount,
            att_type: self.att_type,
            proc_spell_id: self.proc_spell_id,
            triggered_by_aura: self.triggered_by_aura,
            proc_time_ms: self.proc_time_ms,
            applied_mods: &self.applied_mods,
        }
    }
}

impl ProcArgs<'_> {
    fn owned(self) -> ProcArgsOwned {
        ProcArgsOwned {
            target: self.target,
            attacker_flags: self.attacker_flags,
            victim_flags: self.victim_flags,
            proc_ex: self.proc_ex,
            amount: self.amount,
            original_amount: self.original_amount,
            att_type: self.att_type,
            proc_spell_id: self.proc_spell_id,
            triggered_by_aura: self.triggered_by_aura,
            proc_time_ms: self.proc_time_ms,
            applied_mods: self.applied_mods.to_vec(),
        }
    }
}

/// Holds a borrowed applied-mods slice (lifetime on
/// the args struct).
type AppliedMods<'a> = &'a [(u32, u8)];

/// `Spell::Spell` proc-flag computation (the
/// dmgclass/positivity switch, `Spell.cpp:910`)
/// plus `CanTriggerProcs` for this cast. Returns
/// (attacker flags, victim flags, can-trigger).
pub(crate) fn spell_proc_context(
    spell: &SpellEntry,
    triggered: bool,
    from_proc: bool,
) -> (u32, u32, bool) {
    let (mut attacker, mut victim) = match spell.dmg_class {
        dmg_class::MELEE => (proc_flag::DEAL_MELEE_ABILITY, proc_flag::TAKE_MELEE_ABILITY),
        dmg_class::RANGED => (
            proc_flag::DEAL_RANGED_ABILITY,
            proc_flag::TAKE_RANGED_ABILITY,
        ),
        dmg_class::MAGIC => {
            // SPELL_ATTR_IS_ABILITY.
            let is_ability = spell.attributes & 0x0000_0010 != 0;
            if spell.is_positive_spell() {
                if is_ability {
                    (
                        proc_flag::DEAL_HELPFUL_ABILITY,
                        proc_flag::TAKE_HELPFUL_ABILITY,
                    )
                } else {
                    (proc_flag::DEAL_HELPFUL_SPELL, proc_flag::TAKE_HELPFUL_SPELL)
                }
            } else if is_ability {
                (
                    proc_flag::DEAL_HARMFUL_ABILITY,
                    proc_flag::TAKE_HARMFUL_ABILITY,
                )
            } else {
                (proc_flag::DEAL_HARMFUL_SPELL, proc_flag::TAKE_HARMFUL_SPELL)
            }
        }
        _ => (proc_flag::NONE, proc_flag::NONE),
    };
    if spell.attributes_ex3 & ATTR_EX3_SUPPRESS_CASTER_PROCS != 0 {
        attacker = proc_flag::NONE;
    }
    if spell.attributes_ex3 & ATTR_EX3_SUPPRESS_TARGET_PROCS != 0 {
        victim = proc_flag::NONE;
    }
    // `CanTriggerProcs`: aura-triggered casts only
    // chain when NOT_A_PROC spells say so (the fork
    // family exception list is not ported).
    let can_trigger = if attacker == proc_flag::NONE && victim == proc_flag::NONE {
        false
    } else if !triggered || !from_proc {
        true
    } else {
        spell.attributes_ex3 & ATTR_EX3_NOT_A_PROC != 0
    };
    (attacker, victim, can_trigger)
}

/// `SpellMgr::IsSpellProcEventCanTriggeredBy` (the
/// fork does not check `spellFamilyMask` here; the
/// per-effect mask check lives in `HandleTriggers`).
fn can_triggered_by(
    spe: Option<&ProcEventEntry>,
    event_proc_flag: u32,
    proc_spell: Option<&SpellEntry>,
    proc_flag: u32,
    proc_extra: u32,
) -> bool {
    let event_proc_ex = spe.map_or(proc_ex::NONE, |s| s.proc_ex);
    if proc_flag & event_proc_flag == 0 {
        return false;
    }
    // Either procs only on cast end, or only on hit.
    if (proc_extra & proc_ex::CAST_END) != (event_proc_ex & proc_ex::CAST_END) {
        return false;
    }
    if event_proc_flag & (proc_flag::HEARTBEAT | proc_flag::KILL | proc_flag::ON_TRAP_ACTIVATION)
        != 0
    {
        return true;
    }
    if let Some(spe) = spe {
        match proc_spell {
            None => {
                // Melee attacks have Normal school
                // (mask bit 0).
                if spe.school_mask != 0 && spe.school_mask & 0x1 == 0 {
                    return false;
                }
            }
            Some(proc_spell) => {
                if spe.school_mask != 0 && spe.school_mask & (1u32 << proc_spell.school) == 0 {
                    return false;
                }
                if spe.spell_family_name != 0
                    && spe.spell_family_name != proc_spell.spell_family_name
                {
                    return false;
                }
            }
        }
    }
    if event_proc_ex == proc_ex::NONE {
        // No procs from periodic heals without an
        // extra requirement.
        if event_proc_flag & (proc_flag::DEAL_HARMFUL_PERIODIC | proc_flag::TAKE_HARMFUL_PERIODIC)
            != 0
            && proc_extra & proc_ex::PERIODIC_POSITIVE != 0
        {
            return false;
        }
        if proc_extra & (proc_ex::NORMAL_HIT | proc_ex::CRITICAL_HIT) != 0 {
            return true;
        }
    } else {
        if event_proc_ex & proc_ex::EX_TRIGGER_ALWAYS != 0 {
            return true;
        }
        if event_proc_ex & proc_ex::NO_PERIODIC != 0
            && (proc_flag & (proc_flag::DEAL_HARMFUL_PERIODIC | proc_flag::TAKE_HARMFUL_PERIODIC)
                != 0
                || proc_spell.is_some_and(applies_periodic_aura))
        {
            return false;
        }
        if event_proc_ex & proc_extra != 0 {
            return true;
        }
    }
    false
}

/// `IsSpellAppliesPeriodicAura` subset: any effect
/// applies a ticking aura.
fn applies_periodic_aura(spell: &SpellEntry) -> bool {
    spell.effects.iter().any(|e| {
        e.effect == effect::APPLY_AURA
            && matches!(
                e.apply_aura_name,
                aura_type::PERIODIC_DAMAGE
                    | aura_type::PERIODIC_HEAL
                    | aura_type::PERIODIC_ENERGIZE
                    | aura_type::PERIODIC_DAMAGE_PERCENT
            )
    })
}

/// `Aura::CanProcFrom` per-effect execution filter.
#[allow(clippy::too_many_arguments)]
fn can_proc_from(
    aura_spell: &SpellEntry,
    eff_index: usize,
    spe: Option<&ProcEventEntry>,
    proc_spell: &SpellEntry,
    event_proc_ex: u32,
    proc_ex: u32,
    active: bool,
    use_class_mask: bool,
) -> bool {
    let mut mask = aura_spell.effects[eff_index].item_type;
    if mask == 0 {
        mask = spe.map_or(0, |s| s.spell_family_mask[eff_index]);
    }
    if !use_class_mask || mask == 0 {
        if event_proc_ex & proc_ex::EX_TRIGGER_ALWAYS == 0 {
            if event_proc_ex == proc_ex::NONE {
                return proc_ex & (proc_ex::NORMAL_HIT | proc_ex::CRITICAL_HIT) != 0 && active;
            }
            if event_proc_ex & (proc_ex::NORMAL_HIT | proc_ex::CRITICAL_HIT) & proc_ex != 0
                && !active
                && event_proc_ex & proc_ex::NO_DAMAGE_MASK & proc_ex == 0
            {
                return false;
            }
        }
        true
    } else {
        mask & proc_spell.spell_family_flags != 0
    }
}

/// One matched proc awaiting execution
/// (`ProcTriggeredEntry` subset).
struct Triggered {
    spe: Option<ProcEventEntry>,
    /// Holder owner (executes the trigger).
    owner: ObjectGuid,
    /// Unit the trigger acts toward.
    target: Option<ObjectGuid>,
    /// Snapshot of the holder at match time (live
    /// holder re-found by spell/caster/apply time for
    /// charge drops).
    holder: AuraHolder,
    proc_flag: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum ProcProcess {
    All,
    Instant,
    Delayed,
}

#[derive(Clone, Copy, PartialEq)]
enum ProcResult {
    Ok,
    Failed,
    CantTrigger,
}

/// One per-effect proc handler invocation (the
/// `Unit::Handle*Proc` argument set).
struct ProcHandlerCtx<'a> {
    /// Aura owner (`this`).
    owner: ObjectGuid,
    /// The other unit (`pVictim`).
    victim: Option<ObjectGuid>,
    amount: u32,
    original_amount: u32,
    holder: &'a AuraHolder,
    eff_index: usize,
    aura_spell: &'a SpellEntry,
    proc_spell: Option<&'a SpellEntry>,
    proc_flag: u32,
    proc_ex: u32,
    cooldown: u32,
}

impl ProcHandlerCtx<'_> {
    /// `triggeredByAura->GetModifier()->m_amount`.
    fn trigger_amount(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        {
            self.holder.effects[self.eff_index].map_or(0.0, |s| s.amount as f32)
        }
    }

    /// `triggeredByAura->GetModifier()->m_miscvalue`.
    fn misc_value(&self) -> i32 {
        self.holder.effects[self.eff_index].map_or(0, |s| s.misc_value)
    }
}

impl Map {
    /// Raise a proc event (`ProcDamageAndSpell`):
    /// immediate when the delay is 0 or on kill,
    /// otherwise the instant-target attacker pass now
    /// and the rest after `Spell.ProcDelay`.
    pub(crate) fn proc_damage_and_spell(&mut self, dealer: ObjectGuid, args: ProcArgs<'_>) {
        if self.unit_position(dealer).is_none() {
            return;
        }
        if let Some(target) = args.target
            && target != dealer
            && self.unit_position(target).is_none()
        {
            return;
        }
        let delay = self.config.proc_delay_ms;
        if delay == 0 || args.attacker_flags & proc_flag::KILL != 0 {
            self.proc_damage_and_spell_real(dealer, &args, ProcProcess::All);
        } else {
            self.proc_damage_and_spell_real(dealer, &args, ProcProcess::Instant);
            self.pending_procs.push(PendingProc {
                dealer,
                due_ms: self.now_ms + delay,
                args: args.owned(),
            });
        }
    }

    /// Raise a damage/heal application as a proc
    /// event (fork does this inside `DealDamage` /
    /// `DealHeal` from the spell's proc info).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn raise_damage_procs(
        &mut self,
        dealer: ObjectGuid,
        victim: ObjectGuid,
        spell_id: Option<u32>,
        ctx: &ProcRaiseCtx,
        proc_ex_val: u32,
        amount: u32,
        original: u32,
    ) {
        if !ctx.can_trigger {
            return;
        }
        let mut victim_flags = ctx.victim_flags;
        if amount > 0 {
            victim_flags |= proc_flag::TAKEN_ANY_DAMAGE;
        }
        let args = ProcArgs {
            target: Some(victim),
            attacker_flags: ctx.attacker_flags,
            victim_flags,
            proc_ex: proc_ex_val,
            amount,
            original_amount: original,
            att_type: ctx.att_type,
            proc_spell_id: spell_id,
            triggered_by_aura: ctx.triggered_by_aura,
            proc_time_ms: self.now_ms,
            applied_mods: &ctx.applied_mods,
        };
        self.proc_damage_and_spell(dealer, args);
    }

    /// Flush due delayed proc checks
    /// (`ProcessPendingProcs` in the unit update).
    pub(crate) fn process_pending_procs(&mut self) {
        if self.pending_procs.is_empty() {
            return;
        }
        let now = self.now_ms;
        let mut pending = std::mem::take(&mut self.pending_procs);
        let mut keep = Vec::new();
        for entry in pending.drain(..) {
            if entry.due_ms > now {
                keep.push(entry);
                continue;
            }
            if self.unit_position(entry.dealer).is_none() {
                continue;
            }
            if let Some(target) = entry.args.target
                && self.unit_position(target).is_none()
            {
                continue;
            }
            let args = entry.args.as_args();
            self.proc_damage_and_spell_real(entry.dealer, &args, ProcProcess::Delayed);
        }
        keep.append(&mut self.pending_procs);
        self.pending_procs = keep;
    }

    /// Attacker pass then victim pass, then execution
    /// of the merged trigger list (victim side is
    /// skipped entirely in the instant pass).
    fn proc_damage_and_spell_real(
        &mut self,
        dealer: ObjectGuid,
        args: &ProcArgs<'_>,
        process: ProcProcess,
    ) {
        let mut triggered: Vec<Triggered> = Vec::new();
        if args.attacker_flags != 0 {
            self.proc_for(false, dealer, args.target, args, process, &mut triggered);
        }
        if process != ProcProcess::Instant
            && args.victim_flags != 0
            && let Some(victim) = args.target
            && self.unit_is_alive(victim)
        {
            self.proc_for(true, victim, Some(dealer), args, process, &mut triggered);
        }
        self.handle_triggers(args, triggered);
    }

    /// `Unit::ProcDamageAndSpellFor`: match one
    /// side's holders against the event.
    #[allow(clippy::too_many_lines)] // one ProcDamageAndSpellFor port
    fn proc_for(
        &mut self,
        is_victim: bool,
        owner: ObjectGuid,
        other: Option<ObjectGuid>,
        args: &ProcArgs<'_>,
        process: ProcProcess,
        triggered: &mut Vec<Triggered>,
    ) {
        // No victim-side procs on helpful spells dealt
        // by others.
        if is_victim
            && let Some(other_guid) = other
            && other_guid != owner
            && args
                .proc_spell_id
                .and_then(|id| self.stores.spells.get(id))
                .is_some_and(SpellEntry::is_positive_spell)
        {
            return;
        }
        let side_flag = if is_victim {
            args.victim_flags
        } else {
            args.attacker_flags
        };
        let proc_spell = args
            .proc_spell_id
            .and_then(|id| self.stores.spells.get(id).cloned());
        for holder in self.aura_holders(owner) {
            // An aura cannot proc off its own spell.
            if args.proc_spell_id == Some(holder.spell_id) {
                continue;
            }
            let Some(aura_spell) = self.stores.spells.get(holder.spell_id).cloned() else {
                continue;
            };
            // Delay-batch split (attacker side only).
            if !is_victim {
                let instant = aura_spell.attributes_ex3 & ATTR_EX3_INSTANT_TARGET_PROCS != 0;
                match process {
                    ProcProcess::Instant if !instant => continue,
                    ProcProcess::Delayed if instant => continue,
                    _ => {}
                }
            }
            // Cooldown-on-failure auras stop matching
            // while their own spell cools down.
            if aura_spell.attributes_ex2 & ATTR_EX2_PROC_COOLDOWN_ON_FAILURE != 0
                && self.has_cooldown(owner, &aura_spell, None)
            {
                continue;
            }
            // Auras applied after the event (by the
            // event's own side) cannot proc from it.
            let apply_guard = if is_victim {
                other == Some(holder.caster)
            } else {
                owner == holder.caster
            };
            if apply_guard && holder.apply_time_ms >= args.proc_time_ms {
                continue;
            }
            let (matched, spe, roll_failed) = self.is_triggered_at_proc_event(
                owner,
                other,
                &holder,
                &aura_spell,
                proc_spell.as_ref(),
                side_flag,
                args,
                is_victim,
            );
            if roll_failed
                && aura_spell.attributes_ex2 & ATTR_EX2_PROC_COOLDOWN_ON_FAILURE != 0
                && let Some(spe) = spe
                && spe.cooldown > 0
            {
                self.cooldowns.insert(
                    (owner, aura_spell.id),
                    CooldownEntry {
                        spell_id: aura_spell.id,
                        ..CooldownEntry::timed(self.now_ms + spe.cooldown)
                    },
                );
            }
            if matched {
                triggered.push(Triggered {
                    spe,
                    owner,
                    target: other,
                    holder,
                    proc_flag: side_flag,
                });
            }
        }
    }

    /// `Unit::IsTriggeredAtSpellProcEvent` general
    /// path (the fork's hard-coded per-spell cases
    /// and equipment-subclass gates are not ported).
    /// Returns (matched, event entry, chance-roll
    /// failed after all other gates passed).
    #[allow(clippy::too_many_arguments)]
    fn is_triggered_at_proc_event(
        &mut self,
        owner: ObjectGuid,
        other: Option<ObjectGuid>,
        holder: &AuraHolder,
        aura_spell: &SpellEntry,
        proc_spell: Option<&SpellEntry>,
        side_flag: u32,
        args: &ProcArgs<'_>,
        is_victim: bool,
    ) -> (bool, Option<ProcEventEntry>, bool) {
        let spe = self.stores.spells.proc_events.get(&aura_spell.id).copied();
        let event_flag = match spe {
            Some(s) if s.proc_flags != 0 => s.proc_flags,
            _ => aura_spell.proc_flags,
        };
        if event_flag == 0 {
            return (false, spe, false);
        }
        if !can_triggered_by(
            spe.as_ref(),
            event_flag,
            proc_spell,
            side_flag,
            args.proc_ex,
        ) {
            return (false, spe, false);
        }
        // On-kill procs need an honor/XP-worthy victim
        // for player owners.
        if event_flag & proc_flag::KILL != 0
            && self.players.contains_key(&owner)
            && !other.is_some_and(|victim| self.is_honor_or_xp_target(owner, victim))
        {
            return (false, spe, false);
        }
        // Periodic-damage auras do not proc off their
        // own ticks (fork family exception list not
        // ported).
        if args.triggered_by_aura
            && proc_spell.is_some()
            && aura_spell.attributes_ex3 & ATTR_EX3_CAN_PROC_FROM_PROCS == 0
            && proc_spell.is_some_and(|s| s.attributes_ex3 & ATTR_EX3_NOT_A_PROC == 0)
        {
            return (false, spe, false);
        }
        if aura_spell.attributes_ex3 & ATTR_EX3_ONLY_PROC_ON_CASTER != 0 && holder.caster != owner {
            return (false, spe, false);
        }
        // Chance: DBC, custom chance override, or PPM
        // on the dealer side; SPELLMOD_CHANCE_OF_SUCCESS
        // for players.
        let mut chance = match spe {
            Some(s) if s.custom_chance != 0.0 => s.custom_chance,
            #[allow(clippy::cast_precision_loss)]
            _ => aura_spell.proc_chance as f32,
        };
        if !is_victim
            && let Some(s) = spe
            && s.ppm_rate != 0.0
        {
            let speed = self.unit_base_attack_time(owner, args.att_type);
            #[allow(clippy::cast_precision_loss)]
            {
                chance = speed as f32 * s.ppm_rate / 600.0;
            }
        }
        if self.players.contains_key(&owner) {
            chance = self.apply_spell_mod(
                owner,
                aura_spell,
                spellmod::CHANCE_OF_SUCCESS,
                chance,
                false,
                None,
            );
        }
        if self.spell_rng().frand01() * 100.0 >= chance {
            return (false, spe, true);
        }
        (true, spe, false)
    }

    /// Melee swing proc raise (`AttackerStateUpdate`
    /// main-hand arm; off-hand swings are not modeled
    /// in this subset).
    pub(crate) fn raise_melee_procs(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        proc_ex_val: u32,
        amount: u32,
    ) {
        let ctx = ProcRaiseCtx {
            attacker_flags: proc_flag::DEAL_MELEE_SWING | proc_flag::MAIN_HAND_WEAPON_SWING,
            victim_flags: proc_flag::TAKE_MELEE_SWING,
            can_trigger: true,
            triggered_by_aura: false,
            att_type: att_type::BASE,
            applied_mods: Vec::new(),
        };
        self.raise_damage_procs(attacker, victim, None, &ctx, proc_ex_val, amount, amount);
    }

    /// Kill credit raised in `Kill` for the killer
    /// (on-kill procs / talents).
    pub(crate) fn raise_kill_proc(&mut self, killer: ObjectGuid, victim: ObjectGuid) {
        let args = ProcArgs {
            target: Some(victim),
            attacker_flags: proc_flag::KILL,
            victim_flags: proc_flag::NONE,
            proc_ex: proc_ex::NORMAL_HIT,
            amount: 0,
            original_amount: 0,
            att_type: att_type::BASE,
            proc_spell_id: None,
            triggered_by_aura: false,
            proc_time_ms: self.now_ms,
            applied_mods: &[],
        };
        self.proc_damage_and_spell(killer, args);
    }

    /// `IsHonorOrXPTarget` subset for on-kill procs.
    fn is_honor_or_xp_target(&self, owner: ObjectGuid, victim: ObjectGuid) -> bool {
        if victim.is_player() {
            return true;
        }
        let owner_level = self.unit_level(owner);
        self.unit_level(victim) > crate::xp::gray_level(owner_level)
    }

    /// Base attack time for PPM (`GetAttackTime`).
    fn unit_base_attack_time(&self, guid: ObjectGuid, attack_type: u32) -> u32 {
        use mangos_protocol::update_field_table::offsets;
        let index = if attack_type == att_type::OFF {
            offsets::UNIT_FIELD_BASEATTACKTIME + 1
        } else {
            offsets::UNIT_FIELD_BASEATTACKTIME
        };
        let value = self.unit_value_u32(guid, index);
        if value == 0 { 2000 } else { value }
    }

    /// Re-find a live holder matching a match-time
    /// snapshot (spell, caster, apply time).
    fn find_live_holder(&mut self, owner: ObjectGuid, snapshot: &AuraHolder) -> Option<usize> {
        self.unit_base_mut(owner).and_then(|u| {
            u.auras.iter().position(|h| {
                h.spell_id == snapshot.spell_id
                    && h.caster == snapshot.caster
                    && h.apply_time_ms == snapshot.apply_time_ms
            })
        })
    }

    /// `Unit::HandleTriggers`: execute matched procs
    /// in order, burning holder charges per the fork
    /// rules (failure only burns with
    /// `PROC_FAILURE_BURNS_CHARGE`).
    #[allow(clippy::too_many_lines)] // one HandleTriggers port
    fn handle_triggers(&mut self, args: &ProcArgs<'_>, triggered: Vec<Triggered>) {
        let proc_spell = args
            .proc_spell_id
            .and_then(|id| self.stores.spells.get(id).cloned());
        let mut removals: Vec<(ObjectGuid, u32)> = Vec::new();
        for trig in triggered {
            let owner = trig.owner;
            if self.find_live_holder(owner, &trig.holder).is_none() {
                continue;
            }
            let Some(aura_spell) = self.stores.spells.get(trig.holder.spell_id).cloned() else {
                continue;
            };
            // Generated from an ability/spell cast
            // (active even at amount 0).
            let generated = trig.proc_flag
                & (proc_flag::DEAL_HELPFUL_ABILITY
                    | proc_flag::DEAL_HELPFUL_SPELL
                    | proc_flag::DEAL_HARMFUL_ABILITY
                    | proc_flag::DEAL_HARMFUL_SPELL
                    | proc_flag::DEAL_MELEE_ABILITY
                    | proc_flag::DEAL_RANGED_ABILITY)
                != 0;
            let active = args.amount != 0 || generated;
            let burns = aura_spell.attributes & ATTR_PROC_FAILURE_BURNS_CHARGE != 0;
            let mut use_charges = trig.holder.proc_charges > 0;
            let mut any = false;
            let cooldown = trig.spe.map_or(0, |s| s.cooldown);
            for i in 0..3 {
                let Some(state) = trig.holder.effects[i] else {
                    continue;
                };
                // Modifiers already consumed by the
                // causing cast do not re-trigger.
                #[allow(clippy::cast_possible_truncation)]
                if state.mod_charges > 0
                    && !args.applied_mods.contains(&(trig.holder.spell_id, i as u8))
                {
                    continue;
                }
                if let Some(proc_spell_ref) = proc_spell.as_ref() {
                    let pass = match trig.spe {
                        Some(spe) if spe.spell_family_mask[i] != 0 => {
                            proc_spell_ref.spell_family_flags & spe.spell_family_mask[i] != 0
                        }
                        Some(spe) => can_proc_from(
                            &aura_spell,
                            i,
                            Some(&spe),
                            proc_spell_ref,
                            spe.proc_ex,
                            args.proc_ex,
                            active,
                            spe.school_mask == 0,
                        ),
                        None => can_proc_from(
                            &aura_spell,
                            i,
                            None,
                            proc_spell_ref,
                            proc_ex::NONE,
                            args.proc_ex,
                            active,
                            true,
                        ),
                    };
                    if !pass {
                        continue;
                    }
                }
                let ctx = ProcHandlerCtx {
                    owner,
                    victim: trig.target,
                    amount: args.amount,
                    original_amount: args.original_amount,
                    holder: &trig.holder,
                    eff_index: i,
                    aura_spell: &aura_spell,
                    proc_spell: proc_spell.as_ref(),
                    proc_flag: trig.proc_flag,
                    proc_ex: args.proc_ex,
                    cooldown,
                };
                let result = match state.aura_type {
                    aura_type::PROC_TRIGGER_SPELL => self.proc_trigger_spell(
                        owner,
                        trig.target,
                        trig.proc_flag,
                        &aura_spell,
                        i,
                        cooldown,
                    ),
                    aura_type::PROC_TRIGGER_DAMAGE => {
                        self.proc_trigger_damage(owner, trig.target, &trig.holder, &aura_spell, i)
                    }
                    aura_type::DUMMY => self.dummy_aura_proc(&ctx),
                    aura_type::OVERRIDE_CLASS_SCRIPTS => self.override_class_script_proc(&ctx),
                    aura_type::ADD_TARGET_TRIGGER => self.add_target_trigger_proc(&ctx),
                    aura_type::MOD_FEAR => self.remove_fear_by_damage_proc(&ctx),
                    aura_type::MOD_ROOT | aura_type::MOD_PACIFY_SILENCE => {
                        self.remove_by_damage_chance_proc(&ctx)
                    }
                    aura_type::MOD_DAMAGE_DONE => self.mod_damage_aura_proc(&ctx, state.misc_value),
                    aura_type::MOD_INVISIBILITY => self.invisibility_aura_proc(&ctx),
                    aura_type::MOD_RESISTANCE => Self::mod_resistance_aura_proc(&ctx),
                    aura_type::MOD_CASTING_SPEED_NOT_STACK => match ctx.proc_spell {
                        Some(spell) if spell.cast_time_ms(&self.stores.spells.tables) != 0 => {
                            ProcResult::Ok
                        }
                        _ => ProcResult::Failed,
                    },
                    aura_type::MOD_POWER_COST_SCHOOL | aura_type::MOD_POWER_COST_SCHOOL_PCT => {
                        match ctx.proc_spell {
                            Some(spell)
                                if (spell.mana_cost != 0 || spell.mana_cost_percentage != 0)
                                    && Self::school_mask_matches(state.misc_value, spell) =>
                            {
                                ProcResult::Ok
                            }
                            _ => ProcResult::Failed,
                        }
                    }
                    aura_type::REFLECT_SPELLS_SCHOOL => match ctx.proc_spell {
                        Some(spell) if Self::school_mask_matches(state.misc_value, spell) => {
                            ProcResult::Ok
                        }
                        _ => ProcResult::Failed,
                    },
                    aura_type::MECHANIC_IMMUNITY | aura_type::MOD_MECHANIC_RESISTANCE => {
                        match ctx.proc_spell {
                            Some(spell)
                                if i32::try_from(spell.mechanic) == Ok(state.misc_value) =>
                            {
                                ProcResult::Ok
                            }
                            _ => ProcResult::Failed,
                        }
                    }
                    aura_type::MOD_MELEE_HASTE => Self::haste_aura_proc(&ctx),
                    t if CAN_T_TRIGGER_TYPES.contains(&t) => ProcResult::CantTrigger,
                    // HandleNULLProc: OK, charges burn.
                    _ => ProcResult::Ok,
                };
                match result {
                    ProcResult::CantTrigger => continue,
                    ProcResult::Failed => {
                        if use_charges {
                            use_charges = burns;
                        }
                    }
                    ProcResult::Ok => {
                        if use_charges {
                            use_charges = !burns;
                        }
                    }
                }
                any = true;
            }
            if use_charges
                && any
                && let Some(idx) = self.find_live_holder(owner, &trig.holder)
                && let Some(unit) = self.unit_base_mut(owner)
            {
                unit.auras[idx].proc_charges -= 1;
                if unit.auras[idx].proc_charges == 0 {
                    removals.push((owner, trig.holder.spell_id));
                }
            }
        }
        for (unit, spell_id) in removals {
            self.remove_auras_due_to_spell(unit, spell_id);
        }
    }

    /// `HandleProcTriggerSpellAuraProc` default arm +
    /// `TriggerProccedSpell` (custom per-spell cases
    /// not ported).
    fn proc_trigger_spell(
        &mut self,
        owner: ObjectGuid,
        target: Option<ObjectGuid>,
        proc_flag_side: u32,
        aura_spell: &SpellEntry,
        eff_index: usize,
        cooldown: u32,
    ) -> ProcResult {
        let mut trigger_id = aura_spell.effects[eff_index].trigger_spell;
        // Lightning Shield: the fork overwrites the
        // (dummy) DBC trigger with the per-rank damage
        // spell (`UnitAuraProcHandler.cpp` shaman arm;
        // family 11, class mask 0x400, visual 37).
        if aura_spell.spell_family_name == 11
            && aura_spell.spell_family_flags & 0x400 != 0
            && aura_spell.spell_visual == 37
        {
            trigger_id = match aura_spell.id {
                324 => 26364,
                325 => 26365,
                905 => 26366,
                945 => 26367,
                8134 => 26369,
                10431 => 26370,
                10432 => 26363,
                _ => return ProcResult::Failed,
            };
        }
        let Some(entry) = self.stores.spells.get(trigger_id).cloned() else {
            return ProcResult::Failed;
        };
        if entry.effects.iter().all(|e| e.effect == effect::NONE) {
            return ProcResult::Failed;
        }
        let target =
            if entry.is_positive_spell() && proc_flag_side & proc_flag::DEAL_HELPFUL_SPELL == 0 {
                owner
            } else {
                match target {
                    Some(t) => t,
                    None => return ProcResult::Failed,
                }
            };
        if target != owner && !self.unit_is_alive(target) {
            return ProcResult::Failed;
        }
        // `IsSpellReady` on the triggered spell (this
        // is where the proc-event cooldown gates).
        if self.has_cooldown(owner, &entry, None) {
            return ProcResult::Failed;
        }
        let was_in_proc = self.in_proc_cast;
        self.in_proc_cast = true;
        let outcome = self.cast_spell(
            owner,
            trigger_id,
            CastTarget::Unit(target),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        self.in_proc_cast = was_in_proc;
        if outcome == CastOutcome::Started {
            if cooldown > 0 {
                // Proc-event internal cooldown: forced
                // `AddCooldown` on the triggered spell
                // (subject to the already-existing
                // guard, as after the fork cast's own
                // `SendSpellCooldown`).
                let mut mods = Vec::new();
                self.add_cooldowns(owner, &entry, &mut mods, Some(cooldown));
                self.remove_exhausted_spell_mods(owner, &mods);
            }
            ProcResult::Ok
        } else {
            ProcResult::Failed
        }
    }

    /// `HandleProcTriggerDamageAuraProc`: rolled
    /// effect value as spell damage on the proc
    /// target (no proc raising from this damage).
    fn proc_trigger_damage(
        &mut self,
        owner: ObjectGuid,
        target: Option<ObjectGuid>,
        holder: &AuraHolder,
        aura_spell: &SpellEntry,
        eff_index: usize,
    ) -> ProcResult {
        let Some(target) = target else {
            return ProcResult::Failed;
        };
        if !self.unit_is_alive(target) {
            return ProcResult::Failed;
        }
        let value = self.effect_value(aura_spell, eff_index, holder.caster_level);
        if value > 0.0 {
            let amount = mangos_spells::ditheru(value, self.spell_rng());
            #[allow(clippy::cast_possible_wrap)]
            self.school_damage(
                owner,
                aura_spell,
                target,
                amount as i32,
                #[allow(clippy::cast_possible_truncation)]
                {
                    eff_index as u32
                },
                None,
                None,
                None,
            );
        }
        ProcResult::Ok
    }

    /// `TriggerProccedSpell`
    /// (`UnitAuraProcHandler.cpp:503-521`): alive and
    /// ready gates, custom-spell cast when any base
    /// point is set (the fork engages zero-valued
    /// optionals for the unset slots), then the
    /// forced proc-event cooldown.
    fn trigger_procced_spell(
        &mut self,
        owner: ObjectGuid,
        target: Option<ObjectGuid>,
        basepoints: [i32; 3],
        triggered_spell_id: u32,
        cooldown: u32,
        aura_spell_id: u32,
    ) -> ProcResult {
        let Some(entry) = self.stores.spells.get(triggered_spell_id).cloned() else {
            return ProcResult::Failed;
        };
        let Some(target) = target else {
            return ProcResult::Failed;
        };
        if target != owner && !self.unit_is_alive(target) {
            return ProcResult::Failed;
        }
        if self.has_cooldown(owner, &entry, None) {
            return ProcResult::Failed;
        }
        let bp = if basepoints.iter().any(|b| *b != 0) {
            [
                Some(basepoints[0]),
                Some(basepoints[1]),
                Some(basepoints[2]),
            ]
        } else {
            [None; 3]
        };
        self.cast_custom_spell_aura(owner, target, triggered_spell_id, bp, aura_spell_id);
        if cooldown > 0 {
            let mut mods = Vec::new();
            self.add_cooldowns(owner, &entry, &mut mods, Some(cooldown));
            self.remove_exhausted_spell_mods(owner, &mods);
        }
        ProcResult::Ok
    }

    /// `CastCustomSpell` from a proc handler
    /// (triggered, aura-originated, no ready check;
    /// holder cast item unmodeled).
    pub(crate) fn cast_custom_spell(
        &mut self,
        caster: ObjectGuid,
        target: ObjectGuid,
        spell_id: u32,
        bp: [Option<i32>; 3],
    ) {
        self.cast_custom_spell_aura(caster, target, spell_id, bp, 0);
    }

    /// `cast_custom_spell` carrying the proccing
    /// aura's spell id (`m_triggeredByAuraSpell`).
    pub(crate) fn cast_custom_spell_aura(
        &mut self,
        caster: ObjectGuid,
        target: ObjectGuid,
        spell_id: u32,
        bp: [Option<i32>; 3],
        aura_spell_id: u32,
    ) {
        let was_in_proc = self.in_proc_cast;
        self.in_proc_cast = true;
        let _ = self.cast_spell(
            caster,
            spell_id,
            CastTarget::Unit(target),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: bp,
                triggered_by_aura_spell: aura_spell_id,
                triggered_by_spell: 0,
            },
        );
        self.in_proc_cast = was_in_proc;
    }

    /// `roll_chance_f`.
    pub(crate) fn roll_chance_f(&mut self, chance: f32) -> bool {
        self.spell_rng().frand01() * 100.0 < chance
    }

    /// `roll_chance_i`.
    pub(crate) fn roll_chance_i(&mut self, chance: i32) -> bool {
        self.spell_rng().irand(0, 99) < chance
    }

    /// `(misc & GetSchoolMask(spell->School)) != 0`.
    fn school_mask_matches(misc_value: i32, spell: &SpellEntry) -> bool {
        misc_value & (1i32 << spell.school) != 0
    }

    /// `GetFirstSchoolInMask` over `spell->School`:
    /// schools scan in enum order.
    fn first_school(spell: &SpellEntry) -> u32 {
        spell.school
    }

    /// `SelectRandomUnfriendlyTarget`
    /// (`Unit.cpp:9555`): random alive hostile unit
    /// within `radius` of the owner, excluding
    /// `except`. `LoS` and valid-attack-target filters
    /// unmodeled.
    fn select_random_unfriendly_target(
        &mut self,
        owner: ObjectGuid,
        except: Option<ObjectGuid>,
        radius: f32,
    ) -> Option<ObjectGuid> {
        let center = self.unit_position(owner)?;
        let mut targets = self.units_around(owner, center, radius, true);
        targets.retain(|g| Some(*g) != except && *g != owner);
        if targets.is_empty() {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_possible_wrap
        )]
        let idx = self.spell_rng().irand(0, targets.len() as i32 - 1) as usize;
        Some(targets[idx])
    }

    /// `HasInArc(target, arc = PI)` (`Object.cpp`):
    /// the target inside the owner's forward arc.
    pub(crate) fn has_in_arc(&self, owner: ObjectGuid, target: ObjectGuid, arc: f32) -> bool {
        let (Some(opos), Some(tpos)) = (self.unit_position(owner), self.unit_position(target))
        else {
            return false;
        };
        if (opos.x - tpos.x).abs() < f32::EPSILON && (opos.y - tpos.y).abs() < f32::EPSILON {
            return true;
        }
        let angle = (tpos.y - opos.y).atan2(tpos.x - opos.x);
        let mut diff = (angle - opos.o).abs() % std::f32::consts::TAU;
        if diff > std::f32::consts::PI {
            diff = std::f32::consts::TAU - diff;
        }
        diff <= arc / 2.0
    }

    /// `RemoveAuraHolderFromStack`: drop one stack of
    /// the owner's holder for `spell_id`, removing
    /// the holder at zero.
    pub(crate) fn remove_aura_holder_from_stack(&mut self, owner: ObjectGuid, spell_id: u32) {
        let Some(index) = self
            .aura_holders(owner)
            .iter()
            .position(|h| h.spell_id == spell_id)
        else {
            return;
        };
        let emptied = self.unit_base_mut(owner).is_some_and(|unit| {
            unit.auras[index].stack = unit.auras[index].stack.saturating_sub(1);
            unit.auras[index].stack == 0
        });
        if emptied {
            self.remove_aura_at(owner, index);
        }
    }

    /// `CalcArmorReducedDamage(victim, 100)` as the
    /// reconstitution divisor (Sweeping Strikes /
    /// Blade Flurry).
    fn armor_reduced_100(&self, owner: ObjectGuid, victim: ObjectGuid) -> f32 {
        mangos_spells::armor_reduced_damage(
            100.0,
            self.unit_resistance(victim, school::NORMAL),
            self.unit_level(owner),
        )
    }

    /// `HandleHasteAuraProc`
    /// (`UnitAuraProcHandler.cpp:523-530`): Flurry's
    /// last charge is not consumed on a crit.
    fn haste_aura_proc(ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        if ctx.aura_spell.spell_icon_id == 108
            && ctx.aura_spell.spell_visual == 2759
            && ctx.holder.proc_charges <= 1
            && ctx.proc_ex & proc_ex::CRITICAL_HIT != 0
        {
            return ProcResult::Failed;
        }
        ProcResult::Ok
    }

    /// `HandleModResistanceAuraProc`
    /// (`UnitAuraProcHandler.cpp:1833-1846`): Inner
    /// Fire burns charges only on real damage.
    fn mod_resistance_aura_proc(ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        // CF_PRIEST_INNER_FIRE: family 6, flag bit 1.
        if ctx.aura_spell.spell_family_name == 6
            && ctx.aura_spell.spell_family_flags & 0x2 != 0
            && ctx.amount == 0
        {
            return ProcResult::Failed;
        }
        ProcResult::Ok
    }

    /// `HandleInvisibilityAuraProc`
    /// (`UnitAuraProcHandler.cpp:1993-2000`).
    fn invisibility_aura_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        const ATTR_PASSIVE: u32 = 0x40;
        if ctx.aura_spell.attributes & ATTR_PASSIVE != 0 || !ctx.aura_spell.is_positive_spell() {
            return ProcResult::Failed;
        }
        self.remove_auras_due_to_spell(ctx.owner, ctx.aura_spell.id);
        ProcResult::Ok
    }

    /// `HandleRemoveByDamageChanceProc`
    /// (`UnitAuraProcHandler.cpp:1909-1923`).
    fn remove_by_damage_chance_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        let level = self.unit_level(ctx.owner);
        #[allow(clippy::cast_precision_loss)]
        let max_dmg = if level > 8 {
            25.0 * level as f32 - 150.0
        } else {
            50.0
        };
        #[allow(clippy::cast_precision_loss)]
        let chance = ctx.amount as f32 / max_dmg * 100.0;
        if self.roll_chance_f(chance) {
            self.remove_auras_by_caster_spell(ctx.owner, ctx.holder.spell_id, ctx.holder.caster);
            return ProcResult::Ok;
        }
        ProcResult::Failed
    }

    /// `HandleRemoveFearByDamageChanceProc`
    /// (`UnitAuraProcHandler.cpp:1925-1991`, 1.12
    /// build arms).
    fn remove_fear_by_damage_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        if ctx.original_amount == 0 {
            return ProcResult::Failed;
        }
        // MECHANIC_FEAR = 5, MECHANIC_TURN = 23.
        if !matches!(ctx.aura_spell.mechanic, 5 | 23) {
            return ProcResult::Failed;
        }
        let level = self.unit_level(ctx.owner);
        #[allow(clippy::cast_precision_loss)]
        let mut max_dmg = if level > 8 {
            25.0 * level as f32 - 150.0
        } else {
            50.0
        };
        if ctx.owner.is_player() {
            max_dmg *= 0.333;
        }
        if ctx.proc_flag & proc_flag::TAKE_HARMFUL_PERIODIC != 0 {
            max_dmg *= 3.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let chance = ctx.original_amount as f32 / max_dmg * 100.0;
        if self.roll_chance_f(chance) {
            self.remove_auras_by_caster_spell(ctx.owner, ctx.holder.spell_id, ctx.holder.caster);
            return ProcResult::Ok;
        }
        ProcResult::Failed
    }

    /// `HandleModDamageAuraProc`
    /// (`UnitAuraProcHandler.cpp:1848-1907`): school
    /// mask gate plus the Zandalarian Hero Charm
    /// stack burn.
    fn mod_damage_aura_proc(&mut self, ctx: &ProcHandlerCtx<'_>, misc_value: i32) -> ProcResult {
        let school_ok = match ctx.proc_spell {
            Some(spell) => Self::school_mask_matches(misc_value, spell),
            None => misc_value & 1 != 0, // SPELL_SCHOOL_MASK_NORMAL
        };
        if !school_ok {
            return ProcResult::Failed;
        }
        if ctx.aura_spell.id == 24659 {
            let Some(spell) = ctx.proc_spell else {
                return ProcResult::Failed;
            };
            let has_aura = |aura_t: u32| {
                spell
                    .effects
                    .iter()
                    .any(|e| e.effect == effect::APPLY_AURA && e.apply_aura_name == aura_t)
            };
            let totem_exception = matches!(
                (spell.spell_visual, spell.spell_icon_id),
                (319, 1647) | (221, 680 | 33) | (369, 37)
            );
            if !Self::is_direct_damage_spell(spell)
                && !Self::is_heal_spell(spell)
                && !has_aura(aura_type::PERIODIC_DAMAGE)
                && !has_aura(aura_type::PERIODIC_LEECH)
                && !has_aura(62) // PERIODIC_HEALTH_FUNNEL
                && !totem_exception
            {
                return ProcResult::Failed;
            }
            self.remove_aura_holder_from_stack(ctx.owner, 24659);
        }
        ProcResult::Ok
    }

    /// `SpellInternal::IsDirectDamageSpell`
    /// (`SpellMgr.cpp:3090` + `SpellEntry.h:474`).
    fn is_direct_damage_spell(spell: &SpellEntry) -> bool {
        spell.effects.iter().any(|e| {
            matches!(
                e.effect,
                effect::INSTAKILL
                    | effect::SCHOOL_DAMAGE
                    | effect::ENVIRONMENTAL_DAMAGE
                    | effect::HEALTH_LEECH
                    | effect::WEAPON_DAMAGE_NOSCHOOL
                    | effect::WEAPON_PERCENT_DAMAGE
                    | effect::WEAPON_DAMAGE
                    | effect::POWER_BURN
                    | effect::NORMALIZED_WEAPON_DMG
            )
        })
    }

    /// `SpellInternal::IsHealSpell`
    /// (`SpellMgr.cpp:3055-3088`).
    fn is_heal_spell(spell: &SpellEntry) -> bool {
        // CF_PALADIN_FLASH_OF_LIGHT2 bit 30,
        // CF_PALADIN_HOLY_LIGHT2 bit 31.
        if spell.spell_family_name == 10 && spell.spell_family_flags & 0xC000_0000 != 0 {
            return true;
        }
        spell.effects.iter().any(|e| match e.effect {
            effect::HEAL => true,
            effect::APPLY_AURA
            | effect::APPLY_AREA_AURA_PARTY
            | effect::APPLY_AREA_AURA_FRIEND
            | effect::APPLY_AREA_AURA_RAID
            | effect::APPLY_AREA_AURA_PET => e.apply_aura_name == aura_type::PERIODIC_HEAL,
            _ => false,
        })
    }

    /// `HandleAddTargetTriggerAuraProc`
    /// (`UnitAuraProcHandler.cpp:1788-1831`): the
    /// victim casts the trigger spell on itself
    /// (caster-side for the three self spells).
    fn add_target_trigger_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        let trigger_spell_id = ctx.aura_spell.effects[0].trigger_spell;
        #[allow(clippy::cast_precision_loss)]
        let mut chance = if ctx.aura_spell.effects[0].base_points == -1 {
            0.0
        } else {
            ctx.aura_spell.effects[0].base_points as f32
        };
        // CF_MAGE_BLIZZARD: family 3, flag bit 7.
        if ctx.aura_spell.spell_family_name == 3 && ctx.aura_spell.spell_family_flags & 0x80 != 0 {
            chance /= 8.0;
        }
        if !self.roll_chance_f(chance) {
            return ProcResult::Failed;
        }
        if trigger_spell_id != 0 {
            let self_cast = matches!(ctx.aura_spell.id, 17768 | 24392 | 14179);
            if self_cast {
                self.cast_custom_spell(ctx.owner, ctx.owner, trigger_spell_id, [None; 3]);
            } else {
                let Some(victim) = ctx.victim else {
                    return ProcResult::Failed;
                };
                if victim == ctx.owner {
                    return ProcResult::Failed;
                }
                self.cast_custom_spell(victim, victim, trigger_spell_id, [None; 3]);
            }
        }
        ProcResult::Ok
    }

    /// `HandleDummyAuraProc`
    /// (`UnitAuraProcHandler.cpp:532-1127`): family
    /// switch of per-spell dummy-aura customs.
    /// Cast-item identity and the Viscidus text
    /// emotes are unmodeled.
    #[allow(clippy::too_many_lines)] // one fork switch port
    fn dummy_aura_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        let owner = ctx.owner;
        let trigger_amount = ctx.trigger_amount();
        let mut triggered_spell_id = 0u32;
        let mut target = ctx.victim;
        let mut basepoints = [0i32; 3];

        match ctx.aura_spell.spell_family_name {
            0 => match ctx.aura_spell.id {
                // Illusion Passive: the creature
                // despawns on proc.
                7131 => {
                    if owner.is_creature() {
                        self.despawn_creature_now(owner, 0);
                    }
                    return ProcResult::Ok;
                }
                // Eye for an Eye.
                9799 | 25988 => {
                    match ctx.proc_spell {
                        Some(spell) if spell.dmg_class == dmg_class::MAGIC => {}
                        _ => return ProcResult::Failed,
                    }
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
                    {
                        basepoints[0] = mangos_spells::dither(
                            trigger_amount * ctx.original_amount as i32 as f32 / 100.0,
                            self.spell_rng(),
                        );
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    let half_max =
                        self.unit_value_u32(owner, offsets::UNIT_FIELD_MAXHEALTH) as i32 / 2;
                    if basepoints[0] > half_max {
                        basepoints[0] = half_max;
                    }
                    triggered_spell_id = 25997;
                }
                // Sweeping Strikes.
                12292 | 18765 => {
                    let Some(victim) = ctx.victim else {
                        return ProcResult::Failed;
                    };
                    if ctx.amount <= 1 {
                        return ProcResult::Failed;
                    }
                    if let Some(spell) = ctx.proc_spell
                        && matches!(spell.id, 26654 | 12723)
                    {
                        return ProcResult::Failed;
                    }
                    let radius = if ctx.proc_spell.is_some_and(|s| s.id == 1680) {
                        8.0
                    } else {
                        5.0 // ATTACK_DISTANCE
                    };
                    let Some(second) =
                        self.select_random_unfriendly_target(owner, Some(victim), radius)
                    else {
                        return ProcResult::Failed;
                    };
                    target = Some(second);
                    let reduced = self.armor_reduced_100(owner, victim);
                    let execute = ctx.proc_spell.is_some_and(|s| s.id == 20647);
                    if execute && self.health_pct(victim) <= 20.0 && self.health_pct(second) > 20.0
                    {
                        triggered_spell_id = 26654;
                    } else {
                        #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
                        {
                            basepoints[0] = mangos_spells::ditheru(
                                ctx.amount as f32 * 100.0 / reduced,
                                self.spell_rng(),
                            ) as i32;
                        }
                        triggered_spell_id = 12723;
                    }
                }
                // Retaliation.
                20230 => {
                    let Some(victim) = ctx.victim else {
                        return ProcResult::Failed;
                    };
                    if !self.has_in_arc(owner, victim, std::f32::consts::PI) {
                        return ProcResult::Failed;
                    }
                    if self
                        .unit_base(owner)
                        .is_some_and(|u| u.has_unit_state(UNIT_STATE_CAN_NOT_REACT))
                    {
                        return ProcResult::Failed;
                    }
                    triggered_spell_id = 22858;
                }
                // Twisted Reflection (boss spell).
                21063 => {
                    triggered_spell_id = 21064;
                }
                // Unstable Power / Restless Strength:
                // consume one stack of the paired aura.
                24658 => {
                    self.remove_aura_holder_from_stack(owner, 24659);
                    return ProcResult::Ok;
                }
                24661 => {
                    self.remove_aura_holder_from_stack(owner, 24662);
                    return ProcResult::Ok;
                }
                // Viscidus Freeze: physical hits charge
                // the freeze counter (text emotes at
                // 50/100 unmodeled).
                25937 => {
                    if let Some(spell) = ctx.proc_spell
                        && (spell.school != school::NORMAL || spell.id == 5019)
                    {
                        return ProcResult::Failed;
                    }
                    let mut amount_now = 0i32;
                    if let Some(idx) = self.find_live_holder(owner, ctx.holder)
                        && let Some(unit) = self.unit_base_mut(owner)
                        && let Some(state) = unit.auras[idx].effects[ctx.eff_index].as_mut()
                    {
                        state.amount += 1;
                        amount_now = state.amount;
                    }
                    if amount_now == 150 {
                        self.remove_auras_due_to_spell(owner, 25937);
                        triggered_spell_id = 25938;
                    }
                }
                // Adaptive Warding (Frostfire Regalia).
                28764 => {
                    let Some(spell) = ctx.proc_spell else {
                        return ProcResult::Failed;
                    };
                    let has_mage_armor = self.aura_holders(owner).iter().any(|h| {
                        h.effects
                            .iter()
                            .flatten()
                            .any(|e| e.aura_type == aura_type::MOD_MANA_REGEN_INTERRUPT)
                            && self.stores.spells.get(h.spell_id).is_some_and(|s| {
                                // CF_MAGE_MAGE_ARMOR: bit 28.
                                s.spell_family_name == 3 && s.spell_family_flags & 0x1000_0000 != 0
                            })
                    });
                    if !has_mage_armor {
                        return ProcResult::Failed;
                    }
                    triggered_spell_id = match Self::first_school(spell) {
                        school::FIRE => 28765,
                        school::NATURE => 28768,
                        school::FROST => 28766,
                        school::SHADOW => 28769,
                        school::ARCANE => 28770,
                        _ => return ProcResult::Failed,
                    };
                    target = Some(owner);
                }
                // Obsidian Armor.
                27539 => {
                    let Some(spell) = ctx.proc_spell else {
                        return ProcResult::Failed;
                    };
                    triggered_spell_id = match Self::first_school(spell) {
                        school::HOLY => 27536,
                        school::FIRE => 27533,
                        school::NATURE => 27538,
                        school::FROST => 27534,
                        school::SHADOW => 27535,
                        school::ARCANE => 27540,
                        _ => return ProcResult::Failed,
                    };
                    target = Some(owner);
                }
                _ => {}
            },
            3 => {
                // Magic Absorption (icon 459).
                if ctx.aura_spell.spell_icon_id == 459 {
                    if self
                        .unit_base(owner)
                        .map_or(0, mangos_entities::unit::UnitBase::power_type)
                        != 0
                    {
                        return ProcResult::Failed;
                    }
                    #[allow(clippy::cast_precision_loss)]
                    {
                        basepoints[0] = mangos_spells::dither(
                            trigger_amount
                                * self.unit_value_u32(owner, offsets::UNIT_FIELD_MAXPOWER1) as f32
                                / 100.0,
                            self.spell_rng(),
                        );
                    }
                    target = Some(owner);
                    triggered_spell_id = 29442;
                }
                // Master of Elements (icon 1920).
                if ctx.aura_spell.spell_icon_id == 1920 {
                    let Some(spell) = ctx.proc_spell else {
                        return ProcResult::Failed;
                    };
                    #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
                    let cost = spell.mana_cost as i32
                        + spell.mana_cost_percentage as i32
                            * self.unit_value_u32(owner, offsets::UNIT_FIELD_BASE_MANA) as i32
                            / 100;
                    #[allow(clippy::cast_precision_loss)]
                    {
                        basepoints[0] = mangos_spells::dither(
                            cost as f32 * trigger_amount / 100.0,
                            self.spell_rng(),
                        );
                    }
                    if basepoints[0] <= 0 {
                        return ProcResult::Failed;
                    }
                    target = Some(owner);
                    triggered_spell_id = 29077;
                }
            }
            6 => match ctx.aura_spell.id {
                // Vampiric Embrace: the priest (the
                // proc victim here) heals its group via
                // a self-cast custom spell.
                15286 => {
                    let Some(priest) = ctx.victim.filter(|v| self.unit_is_alive(*v)) else {
                        return ProcResult::Failed;
                    };
                    if ctx.holder.caster != priest {
                        return ProcResult::Failed;
                    }
                    #[allow(clippy::cast_precision_loss)]
                    let mut heal = mangos_spells::dither(
                        trigger_amount * ctx.amount as f32 / 100.0,
                        self.spell_rng(),
                    );
                    if heal < 1 {
                        heal = 1;
                    }
                    self.cast_custom_spell(priest, priest, 15290, [Some(heal), None, None]);
                    return ProcResult::Ok;
                }
                // Oracle Healing Bonus.
                26169 => {
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                    {
                        basepoints[0] = (ctx.amount as f32 * 0.1) as i32;
                    }
                    if basepoints[0] == 0 {
                        return ProcResult::Failed;
                    }
                    target = Some(owner);
                    triggered_spell_id = 26170;
                }
                // Greater Heal (Faith 4pc).
                28809 => {
                    triggered_spell_id = 28810;
                }
                _ => {}
            },
            7 => match ctx.aura_spell.id {
                // Healing Touch (Dreamwalker set).
                28719 => {
                    let Some(spell) = ctx.proc_spell else {
                        return ProcResult::Failed;
                    };
                    #[allow(clippy::cast_possible_wrap)]
                    {
                        basepoints[0] = spell.mana_cost as i32 * 30 / 100;
                    }
                    target = Some(owner);
                    triggered_spell_id = 28742;
                }
                // Healing Touch Refund (idol).
                28847 => {
                    target = Some(owner);
                    triggered_spell_id = 28848;
                }
                _ => {}
            },
            8 => match ctx.aura_spell.id {
                // Clean Escape.
                23582 => {
                    match ctx.proc_spell {
                        Some(spell) if spell.effects[0].effect != effect::NONE => {}
                        _ => return ProcResult::Failed,
                    }
                    triggered_spell_id = 23583;
                }
                // Blade Flurry.
                13877 => {
                    let Some(victim) = ctx.victim else {
                        return ProcResult::Failed;
                    };
                    if ctx.proc_spell.is_some_and(|s| s.id == 22482) {
                        return ProcResult::Failed;
                    }
                    let Some(second) =
                        self.select_random_unfriendly_target(owner, Some(victim), 5.0)
                    else {
                        return ProcResult::Failed;
                    };
                    target = Some(second);
                    let reduced = self.armor_reduced_100(owner, victim);
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
                    {
                        basepoints[0] = mangos_spells::ditheru(
                            ctx.amount as f32 * 100.0 / reduced,
                            self.spell_rng(),
                        ) as i32;
                    }
                    triggered_spell_id = 22482;
                }
                _ => {}
            },
            10 => {
                // Seal of Righteousness melee proc
                // (CF_PALADIN_SEALS bit 27, effect 0).
                if ctx.aura_spell.spell_family_flags & 0x0800_0000 != 0 && ctx.eff_index == 0 {
                    return self.seal_of_righteousness_proc(ctx, trigger_amount);
                }
                // Holy Power (Redemption set).
                if ctx.aura_spell.id == 28789 {
                    let Some(victim) = ctx.victim else {
                        return ProcResult::Failed;
                    };
                    triggered_spell_id = match self
                        .unit_base(victim)
                        .map_or(0, mangos_entities::unit::UnitBase::class)
                    {
                        2 | 5 | 7 | 11 => 28795,
                        8 | 9 => 28793,
                        3 | 4 => 28791,
                        1 => 28790,
                        _ => return ProcResult::Failed,
                    };
                }
            }
            11 => match ctx.aura_spell.id {
                // Totemic Power (Earthshatterer set).
                28823 => {
                    let Some(victim) = ctx.victim else {
                        return ProcResult::Failed;
                    };
                    triggered_spell_id = match self
                        .unit_base(victim)
                        .map_or(0, mangos_entities::unit::UnitBase::class)
                    {
                        2 | 5 | 7 | 11 => 28824,
                        8 | 9 => 28825,
                        3 | 4 => 28826,
                        1 => 28827,
                        _ => return ProcResult::Failed,
                    };
                }
                // Lesser Healing Wave (relic).
                28849 => {
                    target = Some(owner);
                    triggered_spell_id = 28850;
                }
                _ => {}
            },
            _ => {}
        }

        if triggered_spell_id == 0 {
            return ProcResult::Ok;
        }
        self.trigger_procced_spell(
            owner,
            target,
            basepoints,
            triggered_spell_id,
            ctx.cooldown,
            ctx.aura_spell.id,
        )
    }

    /// `GetHealthPercent`.
    fn health_pct(&self, guid: ObjectGuid) -> f32 {
        let max = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
        if max == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH) as f32 / max as f32 * 100.0
        }
    }

    /// Seal of Righteousness dummy proc
    /// (`UnitAuraProcHandler.cpp:960-1035`): weapon-
    /// speed-scaled holy damage with manual spell
    /// bonuses. The weapon-enchant combat spell
    /// (`CastItemCombatSpell`) is unmodeled.
    fn seal_of_righteousness_proc(
        &mut self,
        ctx: &ProcHandlerCtx<'_>,
        trigger_amount: f32,
    ) -> ProcResult {
        let owner = ctx.owner;
        let Some(victim) = ctx.victim.filter(|v| self.unit_is_alive(*v)) else {
            return ProcResult::Failed;
        };
        if !owner.is_player() {
            return ProcResult::Failed;
        }
        let spell_id = match ctx.holder.spell_id {
            20154 | 21084 => 25742,
            20287 => 25740,
            20288 => 25739,
            20289 => 25738,
            20290 => 25737,
            20291 => 25736,
            20292 => 25735,
            20293 => 25713,
            _ => return ProcResult::Failed,
        };
        // Main-hand proto delay in seconds (the
        // attack-time field de-hasted by the stored
        // pct; BASE_ATTACK_TIME when unset).
        let speed = self.unit_base(owner).map_or(2.0, |u| {
            let cur = f32::from_bits(
                u.world
                    .object
                    .values()
                    .get_u32(offsets::UNIT_FIELD_BASEATTACKTIME),
            );
            let pct = u.mod_attack_speed_pct[0];
            let base = if cur > 0.0 && pct != 0.0 {
                cur / pct
            } else {
                2000.0
            };
            base / 1000.0
        });
        let min_dmg = trigger_amount / 87.0;
        let max_dmg = trigger_amount / 25.0;
        let mut damage = (max_dmg - min_dmg) * ((speed - 1.5) / (4.0 - 1.5)) + min_dmg;
        // Improved Seal of Righteousness pct mods.
        for mod_spell in [20224u32, 20225, 20330, 20331, 20332] {
            let amount = self.aura_holders(owner).iter().find_map(|h| {
                if h.spell_id != mod_spell {
                    return None;
                }
                h.effects.iter().flatten().find_map(|e| {
                    (e.aura_type == aura_type::ADD_PCT_MODIFIER
                        && u32::try_from(e.misc_value) == Ok(spellmod::ALL_EFFECTS)
                        && e.amount > 0)
                        .then_some(e.amount)
                })
            });
            if let Some(amount) = amount {
                #[allow(clippy::cast_precision_loss)]
                {
                    damage += damage * amount as f32 / 100.0;
                }
            }
        }
        if damage >= 0.0 {
            damage = self.spell_damage_bonus_done(
                owner,
                victim,
                ctx.aura_spell,
                0,
                damage,
                false,
                1,
                None,
            );
            damage =
                self.spell_damage_bonus_taken(victim, ctx.aura_spell, 0, damage, false, 1, None);
        }
        let bp = mangos_spells::dither(damage, self.spell_rng());
        self.cast_custom_spell(owner, victim, spell_id, [Some(bp), None, None]);
        ProcResult::Ok
    }

    /// `HandleOverrideClassScriptAuraProc`
    /// (`UnitAuraProcHandler.cpp:1667-1758`).
    fn override_class_script_proc(&mut self, ctx: &ProcHandlerCtx<'_>) -> ProcResult {
        let Some(victim) = ctx.victim.filter(|v| self.unit_is_alive(*v)) else {
            return ProcResult::Failed;
        };
        let triggered_spell_id = match ctx.misc_value() {
            4309 => 17941,
            836 | 988 | 989 => {
                match ctx.proc_spell {
                    Some(spell) if spell.spell_visual == 259 => {}
                    _ => return ProcResult::Failed,
                }
                match ctx.misc_value() {
                    836 => 12484,
                    988 => 12485,
                    _ => 12486,
                }
            }
            4086 | 4087 => {
                #[allow(clippy::cast_possible_truncation)]
                if !self.roll_chance_i(ctx.trigger_amount() as i32) {
                    return ProcResult::Failed;
                }
                24406
            }
            3656 => match ctx.proc_spell {
                Some(spell) if spell.effects.iter().any(|e| e.effect == effect::HEAL) => 23402,
                _ => 0,
            },
            4533 => {
                // CF_DRUID_REJUVENATION: family 7,
                // flag bit 4.
                match ctx.proc_spell {
                    Some(spell)
                        if spell.spell_family_name == 7 && spell.spell_family_flags & 0x10 != 0 =>
                    {
                        match self
                            .unit_base(victim)
                            .map_or(0, mangos_entities::unit::UnitBase::power_type)
                        {
                            0 => 28722, // POWER_MANA
                            1 => 28723, // POWER_RAGE
                            3 => 28724, // POWER_ENERGY
                            _ => 0,
                        }
                    }
                    _ => 0,
                }
            }
            4537 => {
                // CF_DRUID_REGROWTH: family 7,
                // flag bit 6.
                match ctx.proc_spell {
                    Some(spell)
                        if spell.spell_family_name == 7 && spell.spell_family_flags & 0x40 != 0 =>
                    {
                        28750
                    }
                    _ => 0,
                }
            }
            _ => 0,
        };
        if triggered_spell_id == 0 {
            return ProcResult::Ok;
        }
        self.trigger_procced_spell(
            ctx.owner,
            Some(victim),
            [0; 3],
            triggered_spell_id,
            ctx.cooldown,
            ctx.aura_spell.id,
        )
    }
}

/// Proc raise context for one cast's damage/heal
/// applications (flags computed at prepare).
#[derive(Clone, Default)]
pub(crate) struct ProcRaiseCtx {
    pub attacker_flags: u32,
    pub victim_flags: u32,
    pub can_trigger: bool,
    pub triggered_by_aura: bool,
    pub att_type: u32,
    pub applied_mods: Vec<(u32, u8)>,
}
