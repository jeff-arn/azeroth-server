//! Trainer spell lists (`TrainerSpellData`, plus
//! `ObjectMgr::LoadTrainers` / `LoadTrainerTemplates`
//! / `LoadTrainerGreetings`, `ObjectMgr.cpp:9959`,
//! `10216-10410`).
//!
//! Loader order and gates are fork-exact where the
//! substrate exists: templates load first; each row's
//! wrapper spell must exist and carry
//! `SPELL_EFFECT_LEARN_SPELL` at effect index 0;
//! creature lists additionally require a loaded
//! trainer-flagged template and reject spells the
//! creature's trainer template already teaches.
//! `req_level` defaults to the wrapper's `spellLevel`
//! when the row leaves it 0. A list's `trainer_type`
//! becomes 2 (profession) when any taught spell is a
//! profession spell.
//!
//! Recorded remainders:
//! - UNMODELED: `IsSpellValid` and the talent-wrapper
//!   skip (`GetTalentSpellCost`) — no talent cost
//!   table on the Rust side yet (talent slice owns it).
//! - UNMODELED: greeting locales beyond the default
//!   content (the session locale is always default).

use std::collections::HashMap;

use mysql::prelude::Queryable;

use mangos_db::DbError;
use mangos_spells::SpellStore;
use mangos_spells::defines::effect;

use crate::creature_template::CreatureTemplates;
use crate::load_report::LoadReport;
use crate::skill_info::SkillDbc;

/// `UNIT_NPC_FLAG_TRAINER` (`UnitDefines.h`).
const NPC_FLAG_TRAINER: u32 = 0x0000_0010;
/// `SKILL_CATEGORY_PROFESSION` (`SharedDefines.h:1123`).
const SKILL_CATEGORY_PROFESSION: i32 = 11;
/// `SKILL_FISHING` / `SKILL_COOKING` / `SKILL_FIRST_AID`
/// (secondary professions in `IsProfessionSkill`).
const SECONDARY_PROFESSIONS: [u32; 3] = [356, 185, 129];

/// One `TrainerSpell` (`CreatureDefines.h`): the
/// wrapper (learn) spell plus its gates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrainerSpell {
    pub spell: u32,
    pub spell_cost: u32,
    pub req_skill: u32,
    pub req_skill_value: u32,
    pub req_level: u32,
}

/// One trainer's list (`TrainerSpellData`): spells in
/// wrapper-id order (fork `std::map`) plus the derived
/// trainer type (0 class/other, 2 profession).
#[derive(Clone, Debug, Default)]
pub struct TrainerSpellData {
    pub spells: Vec<TrainerSpell>,
    pub trainer_type: u32,
}

impl TrainerSpellData {
    /// `TrainerSpellData::Find`.
    #[must_use]
    pub fn find(&self, spell: u32) -> Option<&TrainerSpell> {
        self.spells.iter().find(|row| row.spell == spell)
    }
}

/// Loaded trainer lists and greetings.
#[derive(Clone, Debug, Default)]
pub struct TrainerStores {
    by_creature: HashMap<u32, TrainerSpellData>,
    by_template: HashMap<u32, TrainerSpellData>,
    greetings: HashMap<u32, String>,
}

impl TrainerStores {
    /// Builds a store from in-memory lists (test
    /// fixtures; no fork validation is applied).
    #[must_use]
    pub fn from_maps_for_tests(
        by_creature: HashMap<u32, TrainerSpellData>,
        by_template: HashMap<u32, TrainerSpellData>,
        greetings: HashMap<u32, String>,
    ) -> Self {
        Self {
            by_creature,
            by_template,
            greetings,
        }
    }

    /// `GetNpcTrainerSpells(entry)`.
    #[must_use]
    pub fn creature_spells(&self, entry: u32) -> Option<&TrainerSpellData> {
        self.by_creature.get(&entry)
    }

    /// `GetNpcTrainerTemplateSpells(trainer_id)`.
    #[must_use]
    pub fn template_spells(&self, trainer_id: u32) -> Option<&TrainerSpellData> {
        self.by_template.get(&trainer_id)
    }

    /// `GetTrainerGreetingLocale(entry)` default
    /// content.
    #[must_use]
    pub fn greeting(&self, entry: u32) -> Option<&str> {
        self.greetings.get(&entry).map(String::as_str)
    }

    /// Total loaded trainer spells across both maps.
    #[must_use]
    pub fn spell_count(&self) -> usize {
        self.by_creature
            .values()
            .map(|d| d.spells.len())
            .sum::<usize>()
            + self
                .by_template
                .values()
                .map(|d| d.spells.len())
                .sum::<usize>()
    }
}

/// `IsProfessionSkill` (`SpellMgr.h:248`).
fn is_profession_skill(skills: &SkillDbc, skill: u32) -> bool {
    skills.category(skill) == Some(SKILL_CATEGORY_PROFESSION)
        || SECONDARY_PROFESSIONS.contains(&skill)
}

/// `IsProfessionSpell(spell)` for the loader's
/// trainer-type derivation.
fn is_profession_spell(spells: &SpellStore, skills: &SkillDbc, spell_id: u32) -> bool {
    spells
        .profession_skill_of_spell(spell_id)
        .is_some_and(|skill| is_profession_skill(skills, skill))
}

struct RawRow {
    entry: u32,
    spell: u32,
    spell_cost: u32,
    req_skill: u32,
    req_skill_value: u32,
    req_level: u32,
}

/// One `LoadTrainers(table, isTemplates)` pass.
#[allow(clippy::too_many_arguments)]
fn load_trainer_table(
    conn: &mut mysql::Conn,
    table: &str,
    is_templates: bool,
    client_build: u32,
    spells: &SpellStore,
    skills: &SkillDbc,
    templates: &CreatureTemplates,
    template_lists: &HashMap<u32, TrainerSpellData>,
    report: &mut LoadReport,
) -> Result<HashMap<u32, TrainerSpellData>, DbError> {
    let rows: Vec<(u32, u32, u32, u32, u32, u32)> = conn.query_map(
        format!(
            "SELECT entry, spell, spellcost, reqskill, reqskillvalue, reqlevel \
             FROM {table} WHERE {client_build} BETWEEN build_min AND build_max"
        ),
        |(entry, spell, spell_cost, req_skill, req_skill_value, req_level)| {
            (
                entry,
                spell,
                spell_cost,
                req_skill,
                req_skill_value,
                req_level,
            )
        },
    )?;
    let mut lists: HashMap<u32, TrainerSpellData> = HashMap::new();
    for (entry, spell, spell_cost, req_skill, req_skill_value, req_level) in rows {
        let row = RawRow {
            entry,
            spell,
            spell_cost,
            req_skill,
            req_skill_value,
            req_level,
        };
        let Some(wrapper) = spells.get(row.spell) else {
            report.error(format!(
                "{table} entry {} spell {}: wrapper spell does not exist",
                row.entry, row.spell
            ));
            continue;
        };
        if wrapper.effects.first().map_or(0, |e| e.effect) != effect::LEARN_SPELL {
            report.error(format!(
                "{table} entry {} spell {}: not a learn spell",
                row.entry, row.spell
            ));
            continue;
        }
        if !is_templates {
            let Some(info) = templates.get(row.entry) else {
                report.error(format!(
                    "{table} entry {}: no such creature template",
                    row.entry
                ));
                continue;
            };
            if info.npc_flags & NPC_FLAG_TRAINER == 0 {
                report.error(format!(
                    "{table} entry {}: creature lacks trainer flag",
                    row.entry
                ));
                continue;
            }
            if info.trainer_id != 0
                && template_lists
                    .get(&info.trainer_id)
                    .is_some_and(|list| list.find(row.spell).is_some())
            {
                report.error(format!(
                    "{table} entry {} spell {}: already in trainer template {}",
                    row.entry, row.spell, info.trainer_id
                ));
                continue;
            }
        }
        let req_level = if row.req_level != 0 {
            row.req_level
        } else {
            wrapper.spell_level
        };
        let trigger = wrapper.effects.first().map_or(0, |e| e.trigger_spell);
        let data = lists.entry(row.entry).or_default();
        // Fork `spellList[spell]`: a duplicate row
        // overwrites in place.
        if let Some(existing) = data.spells.iter_mut().find(|s| s.spell == row.spell) {
            *existing = TrainerSpell {
                spell: row.spell,
                spell_cost: row.spell_cost,
                req_skill: row.req_skill,
                req_skill_value: row.req_skill_value,
                req_level,
            };
        } else {
            data.spells.push(TrainerSpell {
                spell: row.spell,
                spell_cost: row.spell_cost,
                req_skill: row.req_skill,
                req_skill_value: row.req_skill_value,
                req_level,
            });
        }
        if is_profession_spell(spells, skills, trigger) {
            data.trainer_type = 2;
        }
    }
    for data in lists.values_mut() {
        data.spells.sort_unstable_by_key(|row| row.spell);
    }
    Ok(lists)
}

/// Loads all trainer data: templates, creature lists,
/// greetings (`LoadTrainerTemplates` + `LoadTrainers`
/// + `LoadTrainerGreetings`).
///
/// # Errors
///
/// Returns the first query error.
pub fn load_trainer_stores(
    conn: &mut mysql::Conn,
    client_build: u32,
    spells: &SpellStore,
    skills: &SkillDbc,
    templates: &CreatureTemplates,
    report: &mut LoadReport,
) -> Result<TrainerStores, DbError> {
    let by_template = load_trainer_table(
        conn,
        "npc_trainer_template",
        true,
        client_build,
        spells,
        skills,
        templates,
        &HashMap::new(),
        report,
    )?;
    let by_creature = load_trainer_table(
        conn,
        "npc_trainer",
        false,
        client_build,
        spells,
        skills,
        templates,
        &by_template,
        report,
    )?;
    let greeting_rows: Vec<(u32, String)> = conn.query_map(
        "SELECT entry, content_default FROM npc_trainer_greeting",
        |(entry, content)| (entry, content),
    )?;
    let mut greetings = HashMap::new();
    for (entry, content) in greeting_rows {
        if templates.get(entry).is_none() {
            report.error(format!(
                "npc_trainer_greeting entry {entry}: no such creature template"
            ));
            continue;
        }
        greetings.insert(entry, content);
    }
    Ok(TrainerStores {
        by_creature,
        by_template,
        greetings,
    })
}
