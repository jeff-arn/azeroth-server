//! Quest template subset for character load/save parity
//! (`ObjectMgr::LoadQuests` derivations): the required-item
//! objectives and the time limit are all the Phase 6
//! load/save path reads. `QUEST_SPECIAL_FLAG_DELIVER` is
//! computed when any `ReqItemId` is set
//! (`ObjectMgr.cpp:5602`), `QUEST_SPECIAL_FLAG_TIMED`
//! when `LimitTime` is nonzero (`ObjectMgr.cpp:5953`).

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// One `quest_template` row, load/save-relevant fields only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestTemplateLite {
    pub req_item_ids: [u32; 4],
    pub req_item_counts: [u32; 4],
    pub limit_time: u32,
}

impl QuestTemplateLite {
    /// `QUEST_SPECIAL_FLAG_DELIVER` (computed when any
    /// `ReqItemId` is nonzero).
    #[must_use]
    pub fn is_deliver(&self) -> bool {
        self.req_item_ids.iter().any(|id| *id != 0)
    }

    /// `QUEST_SPECIAL_FLAG_TIMED` (computed when
    /// `LimitTime` is nonzero).
    #[must_use]
    pub fn is_timed(&self) -> bool {
        self.limit_time != 0
    }
}

/// `quest_template` subset by entry.
#[derive(Clone, Debug, Default)]
pub struct QuestTemplatesLite {
    by_entry: HashMap<u32, QuestTemplateLite>,
}

impl QuestTemplatesLite {
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, QuestTemplateLite)>) -> Self {
        Self {
            by_entry: entries.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    #[must_use]
    pub fn get(&self, quest_id: u32) -> Option<&QuestTemplateLite> {
        self.by_entry.get(&quest_id)
    }
}

/// Loads the `quest_template` columns the load/save path
/// reads. Many world databases carry per-patch duplicate
/// quest rows; callers pass the progression patch exactly
/// as for `map_template`, but `quest_template` has no
/// `patch` column in this schema, so all rows load.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_quest_templates_lite(conn: &mut mysql::Conn) -> Result<QuestTemplatesLite, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `LimitTime`, `ReqItemId1`, `ReqItemId2`, `ReqItemId3`, \
         `ReqItemId4`, `ReqItemCount1`, `ReqItemCount2`, `ReqItemCount3`, \
         `ReqItemCount4` FROM `quest_template`",
        (),
    )?;
    let entries = rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        (
            f.u32(0),
            QuestTemplateLite {
                limit_time: f.u32(1),
                req_item_ids: [f.u32(2), f.u32(3), f.u32(4), f.u32(5)],
                req_item_counts: [f.u32(6), f.u32(7), f.u32(8), f.u32(9)],
            },
        )
    });
    Ok(QuestTemplatesLite::from_entries(entries))
}

/// `QUEST_SPECIAL_FLAG_REPEATABLE` (`SpecialFlags` bit 0).
pub const QUEST_SPECIAL_FLAG_REPEATABLE: u32 = 0x001;
/// `QUEST_FLAGS_HIDDEN_REWARDS`.
pub const QUEST_FLAGS_HIDDEN_REWARDS: u32 = 0x0200;
/// `QUEST_FLAGS_AUTO_REWARDED`.
pub const QUEST_FLAGS_AUTO_REWARDED: u32 = 0x0400;

/// One full `quest_template` row (Tier 1 Slice 1a):
/// every field the questgiver accept/progress/
/// turn-in flow and its packets read. Locale text
/// joins (`locales_quest`) are not loaded; packets
/// send the base-locale strings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestTemplate {
    pub entry: u32,
    pub method: u32,
    pub zone_or_sort: i32,
    pub min_level: u32,
    pub max_level: u32,
    pub quest_level: u32,
    pub quest_type: u32,
    pub required_classes: u32,
    pub required_races: u32,
    pub required_skill: u32,
    pub required_skill_value: u32,
    pub required_condition: u32,
    pub rep_objective_faction: u32,
    pub rep_objective_value: i32,
    pub required_min_rep_faction: u32,
    pub required_min_rep_value: i32,
    pub required_max_rep_faction: u32,
    pub required_max_rep_value: i32,
    pub suggested_players: u32,
    pub limit_time: u32,
    pub quest_flags: u32,
    pub special_flags: u32,
    pub prev_quest_id: i32,
    pub next_quest_id: i32,
    pub exclusive_group: i32,
    pub breadcrumb_for_quest_id: u32,
    pub next_quest_in_chain: u32,
    pub src_item_id: u32,
    pub src_item_count: u32,
    pub src_spell: u32,
    pub title: String,
    pub details: String,
    pub objectives: String,
    pub offer_reward_text: String,
    pub request_items_text: String,
    pub end_text: String,
    pub objective_texts: [String; 4],
    pub req_item_ids: [u32; 4],
    pub req_item_counts: [u32; 4],
    pub req_source_ids: [u32; 4],
    pub req_source_counts: [u32; 4],
    pub req_creature_or_go_ids: [i32; 4],
    pub req_creature_or_go_counts: [u32; 4],
    pub req_spell_cast: [u32; 4],
    pub rew_choice_item_ids: [u32; 6],
    pub rew_choice_item_counts: [u32; 6],
    pub rew_item_ids: [u32; 4],
    pub rew_item_counts: [u32; 4],
    pub rew_rep_factions: [u32; 5],
    pub rew_rep_values: [i32; 5],
    pub rew_rep_spillover_mask: u32,
    pub rew_xp: u32,
    pub rew_or_req_money: i32,
    pub rew_money_max_level: u32,
    pub rew_spell: u32,
    pub rew_spell_cast: u32,
    pub rew_mail_template_id: i32,
    pub rew_mail_delay_secs: u32,
    pub rew_mail_money: u32,
    pub point_map_id: u32,
    pub point_x: f32,
    pub point_y: f32,
    pub point_opt: u32,
    pub details_emotes: [u32; 4],
    pub details_emote_delays: [u32; 4],
    pub incomplete_emote: u32,
    pub complete_emote: u32,
    pub offer_reward_emotes: [u32; 4],
    pub offer_reward_emote_delays: [u32; 4],
    pub start_script: u32,
    pub complete_script: u32,
}

impl QuestTemplate {
    /// `Quest::IsAutoComplete` (`QuestMethod == 0`).
    #[must_use]
    pub fn is_auto_complete(&self) -> bool {
        self.method == 0
    }

    /// `Quest::IsActive` subset: the fork computes
    /// patch activity at load; this store already
    /// selects the max row at or below the world
    /// patch, so only the disabled method (1) is
    /// inactive here.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.method != 1
    }

    /// `Quest::IsRepeatable`.
    #[must_use]
    pub fn is_repeatable(&self) -> bool {
        self.special_flags & QUEST_SPECIAL_FLAG_REPEATABLE != 0
    }

    /// `QUEST_SPECIAL_FLAG_DELIVER` (any `ReqItemId` set).
    #[must_use]
    pub fn is_deliver(&self) -> bool {
        self.req_item_ids.iter().any(|id| *id != 0)
    }

    /// Nonzero choice-reward count (`GetRewChoiceItemsCount`,
    /// computed at load in the fork).
    #[must_use]
    pub fn rew_choice_items_count(&self) -> usize {
        self.rew_choice_item_ids
            .iter()
            .filter(|id| **id != 0)
            .count()
    }

    /// Nonzero fixed-reward count (`GetRewItemsCount`).
    #[must_use]
    pub fn rew_items_count(&self) -> usize {
        self.rew_item_ids.iter().filter(|id| **id != 0).count()
    }

    /// `Quest::XPValue` (`QuestDef.cpp:176`): full XP
    /// within 5 levels, then 0.8/0.6/0.4/0.2/0.1 bands,
    /// `ceilf` throughout. The `Rate.XP.Quest` config
    /// multiplier applies at the reward site.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn xp_value(&self, player_level: u32) -> u32 {
        if self.rew_xp == 0 {
            return 0;
        }
        let full = self.rew_xp as f32;
        let scaled = if player_level <= self.quest_level + 5 {
            full
        } else {
            match player_level - self.quest_level {
                6 => full * 0.8,
                7 => full * 0.6,
                8 => full * 0.4,
                9 => full * 0.2,
                _ => full * 0.1,
            }
        };
        scaled.ceil() as u32
    }

    /// `Quest::GetRewOrReqMoney`: positive values scale
    /// by the drop-money rate, negative pass through.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn rew_or_req_money(&self, rate_drop_money: f32) -> i32 {
        if self.rew_or_req_money <= 0 {
            return self.rew_or_req_money;
        }
        (self.rew_or_req_money as f32 * rate_drop_money) as i32
    }
}

/// Full `quest_template` rows by entry.
#[derive(Clone, Debug, Default)]
pub struct QuestTemplates {
    by_entry: HashMap<u32, QuestTemplate>,
}

impl QuestTemplates {
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = QuestTemplate>) -> Self {
        Self {
            by_entry: entries.into_iter().map(|q| (q.entry, q)).collect(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    #[must_use]
    pub fn get(&self, quest_id: u32) -> Option<&QuestTemplate> {
        self.by_entry.get(&quest_id)
    }

    /// All loaded templates (exclusive-group and
    /// chain scans at accept time).
    pub fn iter(&self) -> impl Iterator<Item = &QuestTemplate> {
        self.by_entry.values()
    }
}

/// Loads full `quest_template` rows, keeping the
/// highest `patch <= wow_patch` row per entry (the
/// progression rule the other template loaders use).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::too_many_lines)] // one wide-row loader
pub fn load_quest_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
) -> Result<QuestTemplates, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `patch`, `Method`, `ZoneOrSort`, `MinLevel`, \
         `MaxLevel`, `QuestLevel`, `Type`, `RequiredClasses`, \
         `RequiredRaces`, `RequiredSkill`, `RequiredSkillValue`, \
         `RequiredCondition`, `RepObjectiveFaction`, \
         `RepObjectiveValue`, `RequiredMinRepFaction`, \
         `RequiredMinRepValue`, `RequiredMaxRepFaction`, \
         `RequiredMaxRepValue`, `SuggestedPlayers`, `LimitTime`, \
         `QuestFlags`, `SpecialFlags`, `PrevQuestId`, `NextQuestId`, \
         `ExclusiveGroup`, `BreadcrumbForQuestId`, \
         `NextQuestInChain`, `SrcItemId`, `SrcItemCount`, `SrcSpell`, \
         `Title`, `Details`, `Objectives`, `OfferRewardText`, \
         `RequestItemsText`, `EndText`, `ObjectiveText1`, \
         `ObjectiveText2`, `ObjectiveText3`, `ObjectiveText4`, \
         `ReqItemId1`, `ReqItemId2`, `ReqItemId3`, `ReqItemId4`, \
         `ReqItemCount1`, `ReqItemCount2`, `ReqItemCount3`, \
         `ReqItemCount4`, `ReqSourceId1`, `ReqSourceId2`, \
         `ReqSourceId3`, `ReqSourceId4`, `ReqSourceCount1`, \
         `ReqSourceCount2`, `ReqSourceCount3`, `ReqSourceCount4`, \
         `ReqCreatureOrGOId1`, `ReqCreatureOrGOId2`, \
         `ReqCreatureOrGOId3`, `ReqCreatureOrGOId4`, \
         `ReqCreatureOrGOCount1`, `ReqCreatureOrGOCount2`, \
         `ReqCreatureOrGOCount3`, `ReqCreatureOrGOCount4`, \
         `ReqSpellCast1`, `ReqSpellCast2`, `ReqSpellCast3`, \
         `ReqSpellCast4`, `RewChoiceItemId1`, `RewChoiceItemId2`, \
         `RewChoiceItemId3`, `RewChoiceItemId4`, `RewChoiceItemId5`, \
         `RewChoiceItemId6`, `RewChoiceItemCount1`, \
         `RewChoiceItemCount2`, `RewChoiceItemCount3`, \
         `RewChoiceItemCount4`, `RewChoiceItemCount5`, \
         `RewChoiceItemCount6`, `RewItemId1`, `RewItemId2`, \
         `RewItemId3`, `RewItemId4`, `RewItemCount1`, \
         `RewItemCount2`, `RewItemCount3`, `RewItemCount4`, \
         `RewRepFaction1`, `RewRepFaction2`, `RewRepFaction3`, \
         `RewRepFaction4`, `RewRepFaction5`, `RewRepValue1`, \
         `RewRepValue2`, `RewRepValue3`, `RewRepValue4`, \
         `RewRepValue5`, `RewRepSpilloverMask`, `RewXP`, \
         `RewOrReqMoney`, `RewMoneyMaxLevel`, `RewSpell`, \
         `RewSpellCast`, `RewMailTemplateId`, `RewMailDelaySecs`, \
         `RewMailMoney`, `PointMapId`, `PointX`, `PointY`, \
         `PointOpt`, `DetailsEmote1`, `DetailsEmote2`, \
         `DetailsEmote3`, `DetailsEmote4`, `DetailsEmoteDelay1`, \
         `DetailsEmoteDelay2`, `DetailsEmoteDelay3`, \
         `DetailsEmoteDelay4`, `IncompleteEmote`, `CompleteEmote`, \
         `OfferRewardEmote1`, `OfferRewardEmote2`, \
         `OfferRewardEmote3`, `OfferRewardEmote4`, \
         `OfferRewardEmoteDelay1`, `OfferRewardEmoteDelay2`, \
         `OfferRewardEmoteDelay3`, `OfferRewardEmoteDelay4`, \
         `StartScript`, `CompleteScript` FROM `quest_template` \
         WHERE `patch` <= ? ORDER BY `entry`, `patch`",
        (wow_patch,),
    )?;
    // Ascending patch order: later rows overwrite, so
    // the highest patch <= wow_patch wins per entry.
    let mut by_entry: HashMap<u32, QuestTemplate> = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let mut i = 0usize;
        let mut next = || {
            let v = i;
            i += 1;
            v
        };
        let entry = f.u32(next());
        let _patch = f.u32(next());
        let template = QuestTemplate {
            entry,
            method: f.u32(next()),
            zone_or_sort: f.i32(next()),
            min_level: f.u32(next()),
            max_level: f.u32(next()),
            quest_level: f.u32(next()),
            quest_type: f.u32(next()),
            required_classes: f.u32(next()),
            required_races: f.u32(next()),
            required_skill: f.u32(next()),
            required_skill_value: f.u32(next()),
            required_condition: f.u32(next()),
            rep_objective_faction: f.u32(next()),
            rep_objective_value: f.i32(next()),
            required_min_rep_faction: f.u32(next()),
            required_min_rep_value: f.i32(next()),
            required_max_rep_faction: f.u32(next()),
            required_max_rep_value: f.i32(next()),
            suggested_players: f.u32(next()),
            limit_time: f.u32(next()),
            quest_flags: f.u32(next()),
            special_flags: f.u32(next()),
            prev_quest_id: f.i32(next()),
            next_quest_id: f.i32(next()),
            exclusive_group: f.i32(next()),
            breadcrumb_for_quest_id: f.u32(next()),
            next_quest_in_chain: f.u32(next()),
            src_item_id: f.u32(next()),
            src_item_count: f.u32(next()),
            src_spell: f.u32(next()),
            title: f.string(next()),
            details: f.string(next()),
            objectives: f.string(next()),
            offer_reward_text: f.string(next()),
            request_items_text: f.string(next()),
            end_text: f.string(next()),
            objective_texts: [
                f.string(next()),
                f.string(next()),
                f.string(next()),
                f.string(next()),
            ],
            req_item_ids: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            req_item_counts: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            req_source_ids: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            req_source_counts: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            req_creature_or_go_ids: [f.i32(next()), f.i32(next()), f.i32(next()), f.i32(next())],
            req_creature_or_go_counts: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            req_spell_cast: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            rew_choice_item_ids: [
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
            ],
            rew_choice_item_counts: [
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
            ],
            rew_item_ids: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            rew_item_counts: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            rew_rep_factions: [
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
                f.u32(next()),
            ],
            rew_rep_values: [
                f.i32(next()),
                f.i32(next()),
                f.i32(next()),
                f.i32(next()),
                f.i32(next()),
            ],
            rew_rep_spillover_mask: f.u32(next()),
            rew_xp: f.u32(next()),
            rew_or_req_money: f.i32(next()),
            rew_money_max_level: f.u32(next()),
            rew_spell: f.u32(next()),
            rew_spell_cast: f.u32(next()),
            rew_mail_template_id: f.i32(next()),
            rew_mail_delay_secs: f.u32(next()),
            rew_mail_money: f.u32(next()),
            point_map_id: f.u32(next()),
            point_x: f.f32(next()),
            point_y: f.f32(next()),
            point_opt: f.u32(next()),
            details_emotes: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            details_emote_delays: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            incomplete_emote: f.u32(next()),
            complete_emote: f.u32(next()),
            offer_reward_emotes: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            offer_reward_emote_delays: [f.u32(next()), f.u32(next()), f.u32(next()), f.u32(next())],
            start_script: f.u32(next()),
            complete_script: f.u32(next()),
        };
        by_entry.insert(entry, template);
    }
    Ok(QuestTemplates { by_entry })
}

/// Questgiver relations (`creature_questrelation`,
/// `creature_involvedrelation`, and the gameobject
/// pair): which quests an object entry starts
/// (questrelation) and ends (involvedrelation).
#[derive(Clone, Debug, Default)]
pub struct QuestRelations {
    creature_starts: HashMap<u32, Vec<u32>>,
    creature_ends: HashMap<u32, Vec<u32>>,
    gameobject_starts: HashMap<u32, Vec<u32>>,
    gameobject_ends: HashMap<u32, Vec<u32>>,
}

impl QuestRelations {
    /// Quests the creature entry can start
    /// (`Creature::HasQuest` source set).
    #[must_use]
    pub fn creature_starts(&self, entry: u32) -> &[u32] {
        self.creature_starts.get(&entry).map_or(&[], Vec::as_slice)
    }

    /// Quests the creature entry can end
    /// (`Creature::HasInvolvedQuest` source set).
    #[must_use]
    pub fn creature_ends(&self, entry: u32) -> &[u32] {
        self.creature_ends.get(&entry).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn gameobject_starts(&self, entry: u32) -> &[u32] {
        self.gameobject_starts
            .get(&entry)
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn gameobject_ends(&self, entry: u32) -> &[u32] {
        self.gameobject_ends.get(&entry).map_or(&[], Vec::as_slice)
    }
}

/// Loads the four quest-relation tables
/// (`ObjectMgr::LoadQuestRelations` family; the
/// patch column is not present on these tables in
/// this schema, matching the Lite loader's note).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_quest_relations(conn: &mut mysql::Conn) -> Result<QuestRelations, DbError> {
    fn load_pairs(conn: &mut mysql::Conn, sql: &str) -> Result<HashMap<u32, Vec<u32>>, DbError> {
        let rows: Vec<mysql::Row> = conn.exec(sql, ())?;
        let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
        for row in rows {
            let f = mangos_db::row::Fields::from_row(row);
            map.entry(f.u32(0)).or_default().push(f.u32(1));
        }
        Ok(map)
    }
    Ok(QuestRelations {
        creature_starts: load_pairs(conn, "SELECT `id`, `quest` FROM `creature_questrelation`")?,
        creature_ends: load_pairs(
            conn,
            "SELECT `id`, `quest` FROM `creature_involvedrelation`",
        )?,
        gameobject_starts: load_pairs(
            conn,
            "SELECT `id`, `quest` FROM `gameobject_questrelation`",
        )?,
        gameobject_ends: load_pairs(
            conn,
            "SELECT `id`, `quest` FROM `gameobject_involvedrelation`",
        )?,
    })
}

/// Loads `areatrigger_tavern` ids (`ObjectMgr::LoadTavernAreaTriggers`):
/// client-reported trigger ids that put the player into
/// `REST_TYPE_IN_TAVERN`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_tavern_triggers(
    conn: &mut mysql::Conn,
) -> Result<std::collections::HashSet<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec("SELECT `id` FROM `areatrigger_tavern`", ())?;
    Ok(rows
        .into_iter()
        .map(|row| mangos_db::row::Fields::from_row(row).u32(0))
        .collect())
}
