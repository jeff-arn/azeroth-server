//! Spell defines used by the Slice 5 subset
//! (`SpellDefines.h`, `SpellAuraDefines.h`,
//! `SharedDefines.h`). Values are wire/DB constants —
//! keep them exact.

/// `enum SpellEffects` (`SpellDefines.h:211+`), subset.
pub mod effect {
    pub const NONE: u32 = 0;
    pub const INSTAKILL: u32 = 1;
    pub const SCHOOL_DAMAGE: u32 = 2;
    pub const DUMMY: u32 = 3;
    pub const APPLY_AURA: u32 = 6;
    pub const HEAL: u32 = 10;
    pub const WEAPON_DAMAGE_NOSCHOOL: u32 = 17;
    pub const RESURRECT: u32 = 18;
    pub const ENCHANT_ITEM_TEMPORARY: u32 = 54;
    pub const RESURRECT_NEW: u32 = 113;
    pub const ATTACK_ME: u32 = 114;
    pub const PERSISTENT_AREA_AURA: u32 = 27;
    pub const ENERGIZE: u32 = 30;
    pub const TRIGGER_SPELL: u32 = 64;
    pub const APPLY_AREA_AURA_PARTY: u32 = 35;
    pub const WEAPON_DAMAGE: u32 = 58;
    /// No handler registered at 1.12.1 (null table slot);
    /// zero bench rows use them (verified by SQL).
    pub const APPLY_AREA_AURA_RAID: u32 = 132;
    pub const APPLY_AREA_AURA_OWNER: u32 = 133;
    pub const TOTAL: u32 = 134;
}

/// `enum AuraType` (`SpellAuraDefines.h:45+`), subset.
pub mod aura {
    pub const NONE: u32 = 0;
    pub const PERIODIC_DAMAGE: u32 = 3;
    pub const MOD_CONFUSE: u32 = 5;
    pub const PERIODIC_HEAL: u32 = 8;
    pub const PERIODIC_ENERGIZE: u32 = 24;
    pub const MOD_TAUNT: u32 = 11;
    pub const MOD_STUN: u32 = 12;
    pub const MOD_ROOT: u32 = 26;
    pub const MOD_STAT: u32 = 29;
    pub const MOD_INCREASE_SPEED: u32 = 31;
    pub const MOD_DECREASE_SPEED: u32 = 33;
    pub const MOD_INCREASE_HEALTH: u32 = 34;
    pub const TRANSFORM: u32 = 56;
    pub const MOD_REGEN: u32 = 84;
    pub const MOD_POWER_REGEN: u32 = 85;
    pub const MOD_HEALTH_REGEN_PERCENT: u32 = 88;
    pub const PERIODIC_DAMAGE_PERCENT: u32 = 89;
    pub const INTERRUPT_REGEN: u32 = 94;
    pub const MOD_ATTACK_POWER: u32 = 99;
    pub const MOD_POWER_REGEN_PERCENT: u32 = 110;
    pub const MOD_REGEN_DURING_COMBAT: u32 = 116;
    pub const MOD_INCREASE_HEALTH_PERCENT: u32 = 133;
    pub const MOD_MANA_REGEN_INTERRUPT: u32 = 134;
    pub const MOD_HEALTH_REGEN_IN_COMBAT: u32 = 161;
    pub const MOD_ATTACK_POWER_PCT: u32 = 166;
    /// True for the periodic family whose zero amplitude
    /// defaults to 5000 at load (`SpellMgr::LoadSpell`
    /// tail fixup).
    #[must_use]
    pub fn amplitude_defaults_5000(aura_type: u32) -> bool {
        // PERIODIC_DAMAGE/HEAL/TRIGGER_SPELL/ENERGIZE/
        // LEECH/HEALTH_FUNNEL/MANA_FUNNEL/MANA_LEECH/
        // DAMAGE_PERCENT/POWER_BURN_MANA.
        matches!(aura_type, 3 | 8 | 23 | 24 | 53 | 62 | 63 | 64 | 89 | 162)
    }
}

/// `enum SpellTargetType` (`SpellDefines.h:25+`), subset.
pub mod target {
    pub const NONE: u32 = 0;
    pub const UNIT_CASTER: u32 = 1;
    pub const UNIT_ENEMY: u32 = 6;
    pub const ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE: u32 = 20;
    pub const UNIT_FRIEND: u32 = 21;
    pub const UNIT_PARTY: u32 = 35;
    pub const UNIT_FRIEND_AND_PARTY: u32 = 37;
    pub const LOCATION_CASTER_DEST: u32 = 18;
    pub const LOCATION_CASTER_SRC: u32 = 22;
}

/// `enum Powers` (`SharedDefines.h:152`).
pub mod power {
    pub const MANA: u32 = 0;
    pub const RAGE: u32 = 1;
    pub const FOCUS: u32 = 2;
    pub const ENERGY: u32 = 3;
    pub const MAX: u32 = 4;
    pub const HEALTH: u32 = 0xFFFF_FFFE;
}

/// `enum SpellSchools` (`SpellDefines.h:715`).
pub mod school {
    pub const NORMAL: u32 = 0;
    pub const HOLY: u32 = 1;
    pub const FIRE: u32 = 2;
    pub const NATURE: u32 = 3;
    pub const FROST: u32 = 4;
    pub const SHADOW: u32 = 5;
    pub const ARCANE: u32 = 6;

    /// `SPELL_SCHOOL_MASK_NORMAL` bit for a school.
    #[must_use]
    pub const fn mask(school: u32) -> u32 {
        1 << school
    }
}

/// `enum SpellDamageClass` (`SpellDefines.h:188`).
pub mod dmg_class {
    pub const NONE: u32 = 0;
    pub const MAGIC: u32 = 1;
    pub const MELEE: u32 = 2;
    pub const RANGED: u32 = 3;
}

/// `SpellCastResult` subset (`SpellDefines.h:350+`,
/// counted from the header), the cast-fail reasons the
/// slice can emit. `SPELL_CAST_OK = 0xFF` is internal
/// only and never sent.
pub mod cast_result {
    pub const FAILED_AURA_BOUNCED: u8 = 7;
    pub const FAILED_BAD_IMPLICIT_TARGETS: u8 = 9;
    pub const FAILED_BAD_TARGETS: u8 = 10;
    pub const FAILED_CASTER_DEAD: u8 = 19;
    pub const FAILED_IMMUNE: u8 = 34;
    pub const FAILED_LINE_OF_SIGHT: u8 = 42;
    pub const FAILED_NOT_READY: u8 = 60;
    pub const FAILED_NO_MOUNTS_ALLOWED: u8 = 75;
    pub const FAILED_INTERRUPTED: u8 = 35;
    pub const FAILED_NO_POWER: u8 = 77;
    pub const FAILED_OUT_OF_RANGE: u8 = 89;
    pub const FAILED_SILENCED: u8 = 96;
    pub const FAILED_SPELL_IN_PROGRESS: u8 = 97;
    pub const FAILED_TARGETS_DEAD: u8 = 101;
    pub const FAILED_UNKNOWN: u8 = 145;
}
