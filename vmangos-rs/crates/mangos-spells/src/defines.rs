//! Spell defines used by the Slice 5 subset
//! (`SpellDefines.h`, `SpellAuraDefines.h`,
//! `SharedDefines.h`). Values are wire/DB constants —
//! keep them exact.

/// `enum SpellEffects` (`SpellDefines.h:211+`), subset.
pub mod effect {
    pub const NONE: u32 = 0;
    pub const INSTAKILL: u32 = 1;
    pub const THREAT: u32 = 63;
    pub const DISPEL: u32 = 38;
    pub const LEARN_SPELL: u32 = 36;
    pub const SUMMON_PET: u32 = 56;
    pub const LEARN_PET_SPELL: u32 = 57;
    pub const CREATE_ITEM: u32 = 24;
    pub const TELEPORT_UNITS: u32 = 5;
    pub const LEAP: u32 = 29;
    pub const TELEPORT_UNITS_FACE_CASTER: u32 = 43;
    pub const KNOCK_BACK: u32 = 98;
    pub const PLAYER_PULL: u32 = 124;
    pub const SCHOOL_DAMAGE: u32 = 2;
    pub const DUMMY: u32 = 3;
    pub const SCRIPT_EFFECT: u32 = 77;
    pub const ENVIRONMENTAL_DAMAGE: u32 = 7;
    pub const POWER_DRAIN: u32 = 8;
    pub const HEALTH_LEECH: u32 = 9;
    pub const POWER_BURN: u32 = 62;
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
    pub const WEAPON_PERCENT_DAMAGE: u32 = 31;
    pub const NORMALIZED_WEAPON_DMG: u32 = 121;
    /// No handler registered at 1.12.1 (null table slot);
    /// zero bench rows use them (verified by SQL).
    pub const APPLY_AREA_AURA_PET: u32 = 119;
    pub const APPLY_AREA_AURA_FRIEND: u32 = 128;
    pub const APPLY_AREA_AURA_RAID: u32 = 132;
    pub const APPLY_AREA_AURA_OWNER: u32 = 133;
    pub const TOTAL: u32 = 134;
}

/// `enum AuraType` (`SpellAuraDefines.h:45+`), subset.
pub mod aura {
    pub const NONE: u32 = 0;
    pub const PERIODIC_DAMAGE: u32 = 3;
    pub const MOD_DAMAGE_DONE: u32 = 13;
    pub const MOD_DAMAGE_TAKEN: u32 = 14;
    pub const MOD_RESISTANCE: u32 = 22;
    pub const MOD_CRIT_PERCENT: u32 = 52;
    pub const SCHOOL_ABSORB: u32 = 69;
    pub const MOD_DAMAGE_PERCENT_DONE: u32 = 79;
    pub const MOD_DAMAGE_PERCENT_TAKEN: u32 = 87;
    pub const MOD_HEALING_DONE: u32 = 135;
    pub const MOD_HEALING_DONE_PERCENT: u32 = 136;
    pub const MOD_CONFUSE: u32 = 5;
    pub const DAMAGE_SHIELD: u32 = 15;
    pub const MOD_SCALE: u32 = 61;
    pub const MOD_PACIFY: u32 = 25;
    pub const MOD_SILENCE: u32 = 27;
    pub const MOD_PACIFY_SILENCE: u32 = 60;
    pub const PERIODIC_HEAL: u32 = 8;
    pub const DUMMY: u32 = 4;
    pub const PERIODIC_LEECH: u32 = 53;
    pub const MOD_DAMAGE_DONE_CREATURE: u32 = 59;
    pub const MOD_DAMAGE_DONE_VERSUS: u32 = 168;
    pub const MOD_FLAT_SPELL_DAMAGE_VERSUS: u32 = 180;
    pub const OVERRIDE_CLASS_SCRIPTS: u32 = 112;
    pub const MOD_SPELL_DAMAGE_OF_STAT_PERCENT: u32 = 174;
    pub const MOD_SPELL_HEALING_OF_STAT_PERCENT: u32 = 175;
    pub const MOD_HEALING: u32 = 115;
    pub const MOD_HEALING_PCT: u32 = 118;
    pub const EFFECT_IMMUNITY: u32 = 37;
    pub const STATE_IMMUNITY: u32 = 38;
    pub const SCHOOL_IMMUNITY: u32 = 39;
    pub const DAMAGE_IMMUNITY: u32 = 40;
    pub const DISPEL_IMMUNITY: u32 = 41;
    pub const MECHANIC_IMMUNITY: u32 = 77;
    pub const MECHANIC_IMMUNITY_MASK: u32 = 147;
    pub const MOD_ATTACKSPEED: u32 = 9;
    pub const MOD_SKILL: u32 = 30;
    pub const MOD_SKILL_TALENT: u32 = 98;
    pub const MOD_MELEE_HASTE: u32 = 138;
    pub const MOD_RANGED_HASTE: u32 = 140;
    pub const MOD_RANGED_AMMO_HASTE: u32 = 141;
    pub const PERIODIC_TRIGGER_SPELL: u32 = 23;
    pub const PERIODIC_ENERGIZE: u32 = 24;
    pub const MOD_TAUNT: u32 = 11;
    pub const MOD_STUN: u32 = 12;
    pub const MOD_ROOT: u32 = 26;
    pub const MOD_STAT: u32 = 29;
    pub const PROC_TRIGGER_SPELL: u32 = 42;
    pub const PROC_TRIGGER_DAMAGE: u32 = 43;
    pub const MOD_PERCENT_STAT: u32 = 80;
    pub const MOD_TOTAL_STAT_PERCENTAGE: u32 = 137;
    pub const MOD_INCREASE_SPEED: u32 = 31;
    pub const MOD_DECREASE_SPEED: u32 = 33;
    pub const MOD_INCREASE_SWIM_SPEED: u32 = 107;
    pub const MOD_SPEED_ALWAYS: u32 = 129;
    pub const MOD_MOUNTED_SPEED_ALWAYS: u32 = 130;
    pub const MOD_SPEED_NOT_STACK: u32 = 171;
    pub const MOD_MOUNTED_SPEED_NOT_STACK: u32 = 172;
    pub const MOD_INCREASE_MOUNTED_SPEED: u32 = 32;
    pub const MOUNTED: u32 = 78;
    pub const MOD_FEAR: u32 = 7;
    pub const MOD_INCREASE_HEALTH: u32 = 34;
    pub const MOD_INCREASE_ENERGY: u32 = 35;
    pub const MOD_SHAPESHIFT: u32 = 36;
    pub const MOD_STEALTH: u32 = 16;
    pub const MOD_INCREASE_ENERGY_PERCENT: u32 = 132;
    pub const TRANSFORM: u32 = 56;
    pub const MOD_REGEN: u32 = 84;
    pub const GHOST: u32 = 95;
    pub const WATER_WALK: u32 = 104;
    pub const FEATHER_FALL: u32 = 105;
    pub const HOVER: u32 = 106;
    pub const MOD_POWER_REGEN: u32 = 85;
    pub const MOD_HEALTH_REGEN_PERCENT: u32 = 88;
    pub const PERIODIC_DAMAGE_PERCENT: u32 = 89;
    pub const INTERRUPT_REGEN: u32 = 94;
    pub const MOD_ATTACK_POWER: u32 = 99;
    pub const ADD_FLAT_MODIFIER: u32 = 107;
    pub const ADD_PCT_MODIFIER: u32 = 108;
    pub const MOD_POWER_REGEN_PERCENT: u32 = 110;
    pub const MOD_REGEN_DURING_COMBAT: u32 = 116;
    pub const MOD_INCREASE_HEALTH_PERCENT: u32 = 133;
    pub const MOD_MANA_REGEN_INTERRUPT: u32 = 134;
    pub const MOD_HEALTH_REGEN_IN_COMBAT: u32 = 161;
    pub const MOD_ATTACK_POWER_PCT: u32 = 166;
    pub const RESIST_PUSHBACK: u32 = 149;
    pub const MOD_INVISIBILITY: u32 = 18;
    pub const MOD_CASTING_SPEED_NOT_STACK: u32 = 65;
    pub const MOD_POWER_COST_SCHOOL_PCT: u32 = 72;
    pub const MOD_POWER_COST_SCHOOL: u32 = 73;
    pub const REFLECT_SPELLS_SCHOOL: u32 = 74;
    pub const ADD_TARGET_TRIGGER: u32 = 109;
    pub const MOD_MECHANIC_RESISTANCE: u32 = 117;
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

/// `enum SpellInterruptFlags` (`SpellDefines.h:528+`),
/// subset.
pub mod spell_interrupt_flag {
    pub const DAMAGE_PUSHBACK: u32 = 0x02;
    pub const DAMAGE_CANCELS: u32 = 0x10;
}

/// `enum ProcFlags` (`SpellDefines.h:1008+`).
pub mod proc_flag {
    pub const NONE: u32 = 0x0000_0000;
    pub const HEARTBEAT: u32 = 0x0000_0001;
    pub const KILL: u32 = 0x0000_0002;
    pub const DEAL_MELEE_SWING: u32 = 0x0000_0004;
    pub const TAKE_MELEE_SWING: u32 = 0x0000_0008;
    pub const DEAL_MELEE_ABILITY: u32 = 0x0000_0010;
    pub const TAKE_MELEE_ABILITY: u32 = 0x0000_0020;
    pub const DEAL_RANGED_ATTACK: u32 = 0x0000_0040;
    pub const TAKE_RANGED_ATTACK: u32 = 0x0000_0080;
    pub const DEAL_RANGED_ABILITY: u32 = 0x0000_0100;
    pub const TAKE_RANGED_ABILITY: u32 = 0x0000_0200;
    pub const DEAL_HELPFUL_ABILITY: u32 = 0x0000_0400;
    pub const TAKE_HELPFUL_ABILITY: u32 = 0x0000_0800;
    pub const DEAL_HARMFUL_ABILITY: u32 = 0x0000_1000;
    pub const TAKE_HARMFUL_ABILITY: u32 = 0x0000_2000;
    pub const DEAL_HELPFUL_SPELL: u32 = 0x0000_4000;
    pub const TAKE_HELPFUL_SPELL: u32 = 0x0000_8000;
    pub const DEAL_HARMFUL_SPELL: u32 = 0x0001_0000;
    pub const TAKE_HARMFUL_SPELL: u32 = 0x0002_0000;
    pub const DEAL_HARMFUL_PERIODIC: u32 = 0x0004_0000;
    pub const TAKE_HARMFUL_PERIODIC: u32 = 0x0008_0000;
    pub const TAKEN_ANY_DAMAGE: u32 = 0x0010_0000;
    pub const ON_TRAP_ACTIVATION: u32 = 0x0020_0000;
    pub const MAIN_HAND_WEAPON_SWING: u32 = 0x0040_0000;
    pub const OFF_HAND_WEAPON_SWING: u32 = 0x0080_0000;
}

/// `enum ProcFlagsEx` (`SpellDefines.h:1063+`).
pub mod proc_ex {
    pub const NONE: u32 = 0x000_0000;
    pub const NORMAL_HIT: u32 = 0x000_0001;
    pub const CRITICAL_HIT: u32 = 0x000_0002;
    pub const MISS: u32 = 0x000_0004;
    pub const RESIST: u32 = 0x000_0008;
    pub const DODGE: u32 = 0x000_0010;
    pub const PARRY: u32 = 0x000_0020;
    pub const BLOCK: u32 = 0x000_0040;
    pub const EVADE: u32 = 0x000_0080;
    pub const IMMUNE: u32 = 0x000_0100;
    pub const DEFLECT: u32 = 0x000_0200;
    pub const ABSORB: u32 = 0x000_0400;
    pub const REFLECT: u32 = 0x000_0800;
    pub const INTERRUPT: u32 = 0x000_1000;
    pub const EX_TRIGGER_ALWAYS: u32 = 0x001_0000;
    pub const NO_PERIODIC: u32 = 0x002_0000;
    pub const PERIODIC_POSITIVE: u32 = 0x004_0000;
    pub const CAST_END: u32 = 0x008_0000;
    /// `PROX_EX_NO_DAMAGE_MASK`.
    pub const NO_DAMAGE_MASK: u32 =
        MISS | RESIST | DODGE | PARRY | BLOCK | EVADE | IMMUNE | DEFLECT | ABSORB | REFLECT;
}

/// `enum SpellModOp` (`SpellDefines.h:564+`), subset
/// wired in the map spell engine.
pub mod spellmod {
    pub const DAMAGE: u32 = 0;
    pub const DURATION: u32 = 1;
    pub const RANGE: u32 = 5;
    pub const RADIUS: u32 = 6;
    pub const CRITICAL_CHANCE: u32 = 7;
    pub const ALL_EFFECTS: u32 = 8;
    pub const NOT_LOSE_CASTING_TIME: u32 = 9;
    pub const CASTING_TIME: u32 = 10;
    pub const COOLDOWN: u32 = 11;
    pub const COST: u32 = 14;
    pub const CHANCE_OF_SUCCESS: u32 = 18;
    pub const CRIT_DAMAGE_BONUS: u32 = 15;
    pub const GLOBAL_COOLDOWN: u32 = 21;
    pub const DOT: u32 = 22;
    pub const HASTE: u32 = 23;
    pub const SPELL_BONUS_DAMAGE: u32 = 24;
    pub const MULTIPLE_VALUE: u32 = 27;
    pub const MAX: u32 = 29;
}

/// `enum AuraInterruptFlags` (`SpellDefines.h:538+`),
/// subset (the channel-interrupt column reads the
/// same enum).
pub mod aura_interrupt_flag {
    pub const DAMAGE_CANCELS: u32 = 0x0000_0002;
    pub const TURNING_CANCELS: u32 = 0x0000_0010;
    pub const DISMOUNT_CANCELS: u32 = 0x0000_0040;
    pub const INVULNERABILITY_BUFF_CANCELS: u32 = 0x0020_0000;
    pub const DAMAGE_CHANNEL_DURATION: u32 = 0x0000_4000;
    pub const MOUNT_CANCELS: u32 = 0x0002_0000;
    pub const SHAPESHIFTING_CANCELS: u32 = 0x0000_8000;
}

/// `enum SpellTargetType` (`SpellDefines.h:25+`), subset.
pub mod target {
    pub const NONE: u32 = 0;
    pub const UNIT_CASTER: u32 = 1;
    pub const UNIT_ENEMY: u32 = 6;
    pub const ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE: u32 = 20;
    pub const ENUM_UNITS_ENEMY_IN_CONE: u32 = 24;
    pub const UNIT_FRIEND: u32 = 21;
    pub const UNIT: u32 = 25;
    pub const UNIT_PARTY: u32 = 35;
    pub const UNIT_FRIEND_AND_PARTY: u32 = 37;
    pub const LOCATION_CASTER_DEST: u32 = 18;
    pub const LOCATION_UNIT_MINION_POSITION: u32 = 32;
    pub const LOCATION_CASTER_SRC: u32 = 22;
    pub const ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC: u32 = 15;
    pub const ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC: u32 = 16;
    pub const ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION: u32 = 28;
    pub const ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION: u32 = 29;
    pub const ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC: u32 = 30;
    pub const ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC: u32 = 31;
    pub const ENEMY_AOE_AT_SRC_LOC_SCRIPT: u32 = 7;
    pub const LOCATION_CASTER_HOME_BIND: u32 = 9;
    pub const LOCATION_DATABASE: u32 = 17;
    pub const LOCATION_CASTER_FRONT_RIGHT: u32 = 41;
    pub const LOCATION_CASTER_BACK_RIGHT: u32 = 42;
    pub const LOCATION_CASTER_BACK_LEFT: u32 = 43;
    pub const LOCATION_CASTER_FRONT_LEFT: u32 = 44;
    pub const LOCATION_CASTER_FRONT: u32 = 47;
    pub const LOCATION_CASTER_BACK: u32 = 48;
    pub const LOCATION_CASTER_LEFT: u32 = 49;
    pub const LOCATION_CASTER_RIGHT: u32 = 50;
    pub const LOCATION_CASTER_FRONT_LEAP: u32 = 55;
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
    pub const FAILED_FIZZLE: u8 = 29;
    pub const FAILED_CASTER_DEAD: u8 = 19;
    pub const FAILED_IMMUNE: u8 = 34;
    pub const FAILED_LINE_OF_SIGHT: u8 = 42;
    pub const FAILED_NOT_READY: u8 = 60;
    pub const FAILED_NOT_KNOWN: u8 = 56;
    pub const FAILED_NO_MOUNTS_ALLOWED: u8 = 75;
    pub const FAILED_INTERRUPTED: u8 = 35;
    pub const FAILED_NO_POWER: u8 = 77;
    pub const FAILED_OUT_OF_RANGE: u8 = 89;
    pub const FAILED_SILENCED: u8 = 96;
    pub const FAILED_CONFUSED: u8 = 22;
    pub const FAILED_FLEEING: u8 = 30;
    pub const FAILED_PACIFIED: u8 = 90;
    pub const FAILED_STUNNED: u8 = 100;
    pub const FAILED_SPELL_IN_PROGRESS: u8 = 97;
    pub const FAILED_TARGETS_DEAD: u8 = 101;
    pub const FAILED_TARGET_AURASTATE: u8 = 103;
    pub const FAILED_UNKNOWN: u8 = 145;
}
