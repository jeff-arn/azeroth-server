//! Role identifiers (`CombatBotRoles` in
//! `SharedDefines.h`).

/// `enum CombatBotRoles` (wire values preserved:
/// the `[BotAI]` debug line prints `role=%u`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum BotRole {
    #[default]
    Invalid = 0,
    MeleeDps = 1,
    RangeDps = 2,
    Tank = 3,
    Healer = 4,
}

impl BotRole {
    /// Numeric value as in C++.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// From the numeric value (unknown → `Invalid`).
    #[must_use]
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::MeleeDps,
            2 => Self::RangeDps,
            3 => Self::Tank,
            4 => Self::Healer,
            _ => Self::Invalid,
        }
    }
}

/// Niche sub-roles used by the older island code
/// (talent/pattern modules); not part of the
/// fork's `CombatBotRoles`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BotNiche {
    #[default]
    None,
    Offtank,
    Offheal,
    CrowdControl,
    Buffer,
    Aoe,
}

/// `CombatBotBaseAI::AutoAssignRole` spell markers.
const SPELL_SHIELD_SLAM: u32 = 23922;
const SPELL_HOLY_SHIELD: u32 = 20925;
const SPELL_SANCTITY_AURA: u32 = 20218;
const SPELL_SHADOWFORM: u32 = 15473;
const SPELL_ELEMENTAL_MASTERY: u32 = 16166;
const SPELL_STORMSTRIKE: u32 = 17364;
const SPELL_MOONKIN_FORM: u32 = 24858;
const SPELL_LEADER_OF_THE_PACK: u32 = 17007;

/// `AutoAssignRole`: class + signature-spell role.
#[allow(clippy::match_same_arms)]
#[must_use]
pub fn auto_assign_role(class_id: u8, has_spell: &dyn Fn(u32) -> bool) -> BotRole {
    match class_id {
        1 => {
            if has_spell(SPELL_SHIELD_SLAM) {
                BotRole::Tank
            } else {
                BotRole::MeleeDps
            }
        }
        4 => BotRole::MeleeDps,
        3 | 8 | 9 => BotRole::RangeDps,
        2 => {
            if has_spell(SPELL_HOLY_SHIELD) {
                BotRole::Tank
            } else if has_spell(SPELL_SANCTITY_AURA) {
                BotRole::MeleeDps
            } else {
                BotRole::Healer
            }
        }
        5 => {
            if has_spell(SPELL_SHADOWFORM) {
                BotRole::RangeDps
            } else {
                BotRole::Healer
            }
        }
        7 => {
            if has_spell(SPELL_ELEMENTAL_MASTERY) {
                BotRole::RangeDps
            } else if has_spell(SPELL_STORMSTRIKE) {
                BotRole::MeleeDps
            } else {
                BotRole::Healer
            }
        }
        11 => {
            if has_spell(SPELL_MOONKIN_FORM) {
                BotRole::RangeDps
            } else if has_spell(SPELL_LEADER_OF_THE_PACK) {
                BotRole::MeleeDps
            } else {
                BotRole::Healer
            }
        }
        _ => BotRole::MeleeDps,
    }
}
