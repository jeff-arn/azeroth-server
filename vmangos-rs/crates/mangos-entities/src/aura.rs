//! Aura holder state (`SpellAuraHolder` + `Aura`
//! subset): one holder per (spell, caster) application
//! on a unit, up to three effect auras. Slot allocation
//! and update-field writes live in `mangos-map` (they
//! need the values mirror); this module is pure state.

use mangos_guid::ObjectGuid;

/// `MAX_AURAS` / `MAX_POSITIVE_AURAS`
/// (`SpellAuraDefines.h:25-26`).
pub const MAX_AURAS: u8 = 48;
pub const MAX_POSITIVE_AURAS: u8 = 32;
/// `NULL_AURA_SLOT`.
pub const NULL_AURA_SLOT: u8 = 0xFF;

/// One effect's aura (`class Aura` subset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuraEffectState {
    /// `m_modifier.m_auraname`.
    pub aura_type: u32,
    /// `m_modifier.m_amount` (bonus-baked at apply for
    /// periodic damage at 1.12.1).
    pub amount: i32,
    /// `m_currentBasePoints`.
    pub base_points: i32,
    /// `EffectAmplitude` (`m_modifier.periodictime`),
    /// 0 = not periodic.
    pub period_ms: u32,
    /// `m_periodicTimer`.
    pub periodic_timer: i32,
    /// `m_periodicTick`.
    pub periodic_ticks: u32,
    /// `m_miscvalue`.
    pub misc_value: i32,
    /// `SpellModifier::charges` for
    /// `ADD_FLAT_MODIFIER`/`ADD_PCT_MODIFIER`
    /// effects (0 = infinite, -1 = exhausted);
    /// unused (0) for other types.
    pub mod_charges: i32,
}

impl AuraEffectState {
    #[must_use]
    pub fn is_periodic(&self) -> bool {
        self.period_ms > 0
            && matches!(
                self.aura_type,
                mangos_spells::defines::aura::PERIODIC_DAMAGE
                    | mangos_spells::defines::aura::PERIODIC_HEAL
                    | mangos_spells::defines::aura::PERIODIC_ENERGIZE
                    | mangos_spells::defines::aura::PERIODIC_TRIGGER_SPELL
                    | mangos_spells::defines::aura::PERIODIC_DAMAGE_PERCENT
            )
    }
}

/// `class SpellAuraHolder` subset.
#[derive(Clone, Debug, PartialEq)]
pub struct AuraHolder {
    pub spell_id: u32,
    pub caster: ObjectGuid,
    pub caster_level: u32,
    /// Remaining duration ms; -1 = permanent
    /// (`m_duration` counts down in the holder update).
    pub duration_ms: i32,
    /// `m_maxDuration`.
    pub max_duration_ms: i32,
    /// `m_stackAmount`.
    pub stack: u32,
    /// Reserved visible slot (`NULL_AURA_SLOT` when no
    /// slot was free or not yet assigned).
    pub slot: u8,
    /// Holder positivity (slot range + cancelable).
    pub positive: bool,
    pub cancelable: bool,
    pub effects: [Option<AuraEffectState>; 3],
    /// Remaining proc charges (`m_procCharges`; 0 =
    /// uncharged holder, never charge-gated).
    pub proc_charges: i32,
    /// Map-clock apply time (`GetAuraApplyTime`),
    /// stamped by `add_aura_holder`.
    pub apply_time_ms: u32,
}

impl AuraHolder {
    #[must_use]
    pub fn new(
        spell_id: u32,
        caster: ObjectGuid,
        caster_level: u32,
        duration_ms: i32,
        positive: bool,
        cancelable: bool,
    ) -> AuraHolder {
        AuraHolder {
            spell_id,
            caster,
            caster_level,
            duration_ms,
            max_duration_ms: duration_ms,
            stack: 1,
            slot: NULL_AURA_SLOT,
            positive,
            cancelable,
            effects: [None, None, None],
            proc_charges: 0,
            apply_time_ms: 0,
        }
    }

    /// Effect-index flag nibble bits for `AURAFLAGS`
    /// (which effect slots carry auras).
    #[must_use]
    pub fn effect_flags(&self) -> u32 {
        let mut flags = 0u32;
        if self.effects[0].is_some() {
            flags |= 0x08; // AFLAG_EFF_INDEX_0
        }
        if self.effects[1].is_some() {
            flags |= 0x04; // AFLAG_EFF_INDEX_1
        }
        if self.effects[2].is_some() {
            flags |= 0x02; // AFLAG_EFF_INDEX_2
        }
        if self.positive && self.cancelable {
            flags |= 0x01; // AFLAG_CANCELABLE
        }
        flags
    }
}

/// Save-side aura snapshot (`AuraSaveStruct`):
/// the `character_aura` row shape without the
/// owner guid. Kept here (not in `mangos-db`) so
/// `mangos-map` can produce it without a DB
/// dependency; `mangosd` converts to the DB row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavableAura {
    pub caster_guid: u64,
    pub item_guid: u32,
    pub spell: u32,
    pub stacks: i32,
    pub charges: i32,
    pub base_points: [f32; 3],
    pub periodic_time: [i32; 3],
    pub max_duration: i32,
    pub duration: i32,
    pub effect_index_mask: u8,
}

/// Save-side cooldown snapshot (one
/// `character_spell_cooldown` row sans guid).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CooldownSnapshot {
    pub spell: u32,
    pub spell_expire_time: i64,
    pub category: u32,
    pub category_expire_time: i64,
    pub item_id: u32,
}
