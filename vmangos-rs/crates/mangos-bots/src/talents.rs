//! Premade talent builds (`BotTalentBuild`/`s_talentBuilds` in
//! `CombatBotBaseAI.cpp`, data in generated `talent_builds.rs`)
//! and the pure selection half of `LearnPremadeSpecForClass`
//! (filtering; the random pick and talent application belong to
//! the world layer).

use crate::{Class, Role, TALENT_BUILDS};

/// One premade spec: an ordered list of (talent id, ranks) pairs
/// spent sequentially from level 10 (`BotTalentBuild`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TalentBuild {
    pub class: Class,
    pub role: Role,
    pub name: &'static str,
    /// `(talentId, ranksToLearn)` in learning order.
    pub points: &'static [(u32, u8)],
}

impl TalentBuild {
    /// Total ranks across all points (51 for a level-60 build).
    #[must_use]
    pub fn total_ranks(&self) -> u32 {
        self.points.iter().map(|&(_, r)| u32::from(r)).sum()
    }
}

/// Step 1 of `LearnPremadeSpecForClass`: all builds for a class,
/// in table order.
#[must_use]
pub fn builds_for_class(class: Class) -> Vec<&'static TalentBuild> {
    TALENT_BUILDS.iter().filter(|b| b.class == class).collect()
}

/// Role preference filter from `LearnPremadeSpecForClass`:
/// when a role is already assigned, only builds for that role
/// are candidates — unless none match, in which case all class
/// builds are (the caller then picks randomly and, if the role
/// was unset, adopts the chosen build's role).
#[must_use]
pub fn candidate_builds(class: Class, role: Role) -> Vec<&'static TalentBuild> {
    let matching = builds_for_class(class);
    if role != Role::Invalid {
        let role_builds: Vec<_> = matching
            .iter()
            .copied()
            .filter(|b| b.role == role)
            .collect();
        if !role_builds.is_empty() {
            return role_builds;
        }
    }
    matching
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_shape_matches_cpp() {
        assert_eq!(TALENT_BUILDS.len(), 26);
        for b in TALENT_BUILDS {
            assert_eq!(b.total_ranks(), 51, "{} {} ranks", b.class as u8, b.name);
            assert!(!b.points.is_empty());
        }
        // First row: Warrior fury starts with Cruelty 5 (talent 157).
        let first = TALENT_BUILDS[0];
        assert_eq!(first.class, Class::Warrior);
        assert_eq!(first.role, Role::MeleeDps);
        assert_eq!(first.name, "fury");
        assert_eq!(first.points[0], (157, 5));
    }

    #[test]
    fn class_counts_match_master_table() {
        let cases = [
            (Class::Warrior, 3),
            (Class::Paladin, 3),
            (Class::Hunter, 3),
            (Class::Rogue, 2),
            (Class::Priest, 2),
            (Class::Shaman, 3),
            (Class::Mage, 3),
            (Class::Warlock, 3),
            (Class::Druid, 4),
        ];
        for (class, n) in cases {
            assert_eq!(builds_for_class(class).len(), n, "class {}", class as u8);
        }
    }

    #[test]
    fn candidate_filtering_mirrors_learn_logic() {
        // Role assigned + matching build exists -> only those.
        let tanks = candidate_builds(Class::Warrior, Role::Tank);
        assert_eq!(tanks.len(), 1);
        assert_eq!(tanks[0].name, "prot");
        // Role assigned but no build for it -> all class builds.
        let healers = candidate_builds(Class::Warrior, Role::Healer);
        assert_eq!(healers.len(), 3);
        // No role -> all class builds.
        assert_eq!(candidate_builds(Class::Druid, Role::Invalid).len(), 4);
    }
}
