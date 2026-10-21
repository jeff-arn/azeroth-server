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
//! Recorded subset deviations: the fork's hard-coded
//! per-spell proc cases (the `UnitAuraProcHandler`
//! custom switches and dummy-aura procs) are not
//! ported — aura types without a handler here return
//! OK like `HandleNULLProc` (charges still burn);
//! equipment-subclass proc gates and the
//! outdoors-only gate are skipped (no item-proto /
//! outdoor plumbing at proc sites); `spell_chain`
//! rank fill uses the DB table only.

use mangos_entities::aura::AuraHolder;
use mangos_guid::ObjectGuid;
use mangos_spells::defines::{aura as aura_type, dmg_class, effect, proc_ex, proc_flag, spellmod};
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
                && self.has_cooldown(owner, &aura_spell)
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
                    CooldownEntry::timed(self.now_ms + spe.cooldown),
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
                    t if CAN_T_TRIGGER_TYPES.contains(&t) => ProcResult::CantTrigger,
                    // HandleNULLProc and the unported
                    // custom handlers: OK, charges burn.
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
        if self.has_cooldown(owner, &entry) {
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
                skip_known: true,
            },
        );
        self.in_proc_cast = was_in_proc;
        if outcome == CastOutcome::Started {
            if cooldown > 0 {
                self.cooldowns.insert(
                    (owner, trigger_id),
                    CooldownEntry::timed(self.now_ms + cooldown),
                );
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
        let value =
            aura_spell.calculate_effect_value(eff_index, holder.caster_level, 0, self.spell_rng());
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
