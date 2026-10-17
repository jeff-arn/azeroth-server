//! Bot islands that need no Host API (architecture document
//! §12): formation math (`BotFormation`), the combat-event ring
//! buffer and pattern learning (`BotCombatEventBuffer`), and the
//! premade talent tables. The behaviour runner, chat/services,
//! director, and guild/AH systems land here in Phase 4 behind the
//! Bot Host API; nothing in this phase's modules touches the
//! world except through the small query traits.

pub mod breadcrumb;
pub mod combat_events;
pub mod formation;
pub mod patterns;
pub mod talent_builds;
pub mod talents;

pub use breadcrumb::{BreadcrumbPoint, BreadcrumbTrail};
pub use combat_events::{CombatEvent, CombatEventBuffer, CombatEventType};
pub use formation::{FormationHost, FormationManager, FormationSlot, FormationState};
pub use patterns::{LearnedPattern, PatternResponse, PatternStore};
pub use talent_builds::TALENT_BUILDS;
pub use talents::TalentBuild;

/// `CombatBotRoles` (`SharedDefines.h:104`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum Role {
    #[default]
    Invalid = 0,
    MeleeDps = 1,
    RangeDps = 2,
    Tank = 3,
    Healer = 4,
}

/// Playable classes (`SharedDefines.h` `Classes` values used by
/// the talent tables).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum Class {
    Warrior = 1,
    Paladin = 2,
    Hunter = 3,
    Rogue = 4,
    Priest = 5,
    Shaman = 7,
    Mage = 8,
    Warlock = 9,
    Druid = 11,
}

impl Class {
    /// `me->GetClass()` value.
    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }
}
