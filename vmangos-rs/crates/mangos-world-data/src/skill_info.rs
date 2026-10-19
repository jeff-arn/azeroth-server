//! Skill DBC products for `Player::_LoadSkills` field
//! population: `SkillLine.dbc` categories,
//! `SkillRaceClassInfo.dbc` rows (`SkillRaceClassInfoBySkill`),
//! and the `SkillTiers.dbc` id set behind
//! `GetSkillRangeType` (`ObjectMgr.cpp:10067-10081`).
//! Built by the mangosd binary from generic DBC rows, like
//! [`crate::player_info::RaceClassDbc`].

use std::collections::{HashMap, HashSet};

/// `SKILL_CATEGORY_ARMOR` (`SharedDefines.h:1120`).
pub const SKILL_CATEGORY_ARMOR: i32 = 8;
/// `SKILL_CATEGORY_LANGUAGES` (`SharedDefines.h:1122`).
pub const SKILL_CATEGORY_LANGUAGES: i32 = 10;

/// `SkillRangeType` (`ObjectMgr.h`): how a skill's
/// value/max are fixed up at load.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillRange {
    /// Tiered (profession) skill: saved value/max kept.
    Rank,
    /// Armor category: fixed 1..1.
    Mono,
    /// Language category: fixed 300..300.
    Language,
    /// Everything else: max is `level * 5`.
    Level,
}

/// One `SkillRaceClassInfoEntry` subset
/// (`DBCStructure.h:499-509`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RaceClassSkill {
    pub skill_id: u32,
    pub race_mask: u32,
    pub class_mask: u32,
    pub flags: u32,
    pub req_level: u32,
    pub skill_tier_id: u32,
}

/// The skill DBC fields `_LoadSkills` consumes.
#[derive(Clone, Debug, Default)]
pub struct SkillDbc {
    /// `SkillLine` id -> `categoryId`.
    pub categories: HashMap<u32, i32>,
    /// `SkillRaceClassInfo` rows, file order (C++
    /// `equal_range` returns the first mask match).
    pub race_class: Vec<RaceClassSkill>,
    /// Skill tier ids present in `SkillTiers.dbc`.
    pub tiers: HashSet<u32>,
}

impl SkillDbc {
    /// `sSkillLineStore.LookupEntry` existence + category.
    #[must_use]
    pub fn category(&self, skill: u32) -> Option<i32> {
        self.categories.get(&skill).copied()
    }

    /// `GetSkillRaceClassInfo` (`DBCStores.cpp:581-595`):
    /// first row whose nonzero race/class masks contain
    /// the character's race/class bits.
    #[must_use]
    pub fn race_class_info(&self, skill: u32, race: u32, class: u32) -> Option<&RaceClassSkill> {
        self.race_class.iter().find(|entry| {
            if entry.skill_id != skill {
                return false;
            }
            if entry.race_mask != 0 && (race == 0 || entry.race_mask & (1 << (race - 1)) == 0) {
                return false;
            }
            if entry.class_mask != 0 && (class == 0 || entry.class_mask & (1 << (class - 1)) == 0) {
                return false;
            }
            true
        })
    }

    /// `GetSkillRangeType`: tiered skills keep saved
    /// ranges; armor is mono, languages fixed, the rest
    /// level-scaled.
    #[must_use]
    pub fn range_type(&self, category: i32, rc: &RaceClassSkill) -> SkillRange {
        if self.tiers.contains(&rc.skill_tier_id) {
            return SkillRange::Rank;
        }
        match category {
            SKILL_CATEGORY_ARMOR => SkillRange::Mono,
            SKILL_CATEGORY_LANGUAGES => SkillRange::Language,
            _ => SkillRange::Level,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbc() -> SkillDbc {
        SkillDbc {
            categories: HashMap::from([(109, SKILL_CATEGORY_LANGUAGES), (43, 6)]),
            race_class: vec![
                RaceClassSkill {
                    skill_id: 109,
                    race_mask: 0b1010_1110,
                    class_mask: 0,
                    ..Default::default()
                },
                RaceClassSkill {
                    skill_id: 43,
                    race_mask: 0,
                    class_mask: 1,
                    skill_tier_id: 3,
                    ..Default::default()
                },
            ],
            tiers: HashSet::from([3]),
        }
    }

    #[test]
    fn race_class_masks_match_cpp() {
        let dbc = dbc();
        // Orc (race 2 -> bit 1) is in the Orcish mask.
        assert!(dbc.race_class_info(109, 2, 1).is_some());
        // Human (race 1 -> bit 0) is not.
        assert!(dbc.race_class_info(109, 1, 1).is_none());
        // Class mask 1 allows class 1 only.
        assert!(dbc.race_class_info(43, 2, 1).is_some());
        assert!(dbc.race_class_info(43, 2, 5).is_none());
        assert!(dbc.race_class_info(999, 2, 1).is_none());
    }

    #[test]
    fn range_types_match_cpp() {
        let dbc = dbc();
        let lang = dbc.race_class_info(109, 2, 1).expect("orcish");
        assert_eq!(
            dbc.range_type(SKILL_CATEGORY_LANGUAGES, lang),
            SkillRange::Language
        );
        let tiered = dbc.race_class_info(43, 2, 1).expect("tiered");
        assert_eq!(dbc.range_type(6, tiered), SkillRange::Rank);
        assert_eq!(dbc.range_type(SKILL_CATEGORY_ARMOR, lang), SkillRange::Mono);
        assert_eq!(dbc.range_type(6, lang), SkillRange::Level);
    }
}
