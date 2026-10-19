//! Bot islands that need no Host API (architecture document
//! §12): formation math (`BotFormation`), the combat-event ring
//! buffer and pattern learning (`BotCombatEventBuffer`), and the
//! premade talent tables. The behaviour runner, chat/services,
//! director, and guild/AH systems land here in Phase 4 behind the
//! Bot Host API; nothing in this phase's modules touches the
//! world except through the small query traits.

pub mod ah_bot;
pub mod behavior;
pub mod behaviors_class;
pub mod behaviors_shared;
pub mod breadcrumb;
pub mod chat;
pub mod chat_pools;
pub mod class_spells;
pub mod class_spells_table;
pub mod combat_events;
pub mod context;
pub mod director;
pub mod follow;
pub mod formation;
pub mod guild_bots;
pub mod healing;
pub mod host;
pub mod patterns;
pub mod population;
pub mod role;
pub mod role_config;
pub mod services;
pub mod spell_kit;
pub mod talent_builds;
pub mod talents;

pub use behavior::{BehaviorDebug, BehaviorSet, BotBehavior};
pub use breadcrumb::{BreadcrumbPoint, BreadcrumbTrail};
pub use class_spells::{ClassSpells, HealSpell, KnownSpellInfo, resolve_class_spells};
pub use combat_events::{CombatEvent, CombatEventBuffer, CombatEventType};
pub use context::SituationContext;
pub use follow::FollowLeader;
pub use formation::{FormationHost, FormationManager, FormationSlot, FormationState};
pub use host::{
    AURA_MOD_INVISIBILITY, AURA_MOD_SHAPESHIFT, AURA_MOD_STEALTH, BotChatKind, BotHost,
    CastOutcome, MotionKind, UnitView,
};
pub use patterns::{LearnedPattern, PatternResponse, PatternStore};
pub use role::{BotNiche, BotRole, auto_assign_role};
pub use spell_kit::SpellKit;
pub use talent_builds::TALENT_BUILDS;
pub use talents::TalentBuild;

impl From<BotRole> for Role {
    fn from(role: BotRole) -> Self {
        match role {
            BotRole::Invalid => Self::Invalid,
            BotRole::MeleeDps => Self::MeleeDps,
            BotRole::RangeDps => Self::RangeDps,
            BotRole::Tank => Self::Tank,
            BotRole::Healer => Self::Healer,
        }
    }
}

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
