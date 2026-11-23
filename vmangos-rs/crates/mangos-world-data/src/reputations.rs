//! Faction templates and the initial reputation state
//! projection (`ObjectMgr::LoadFactions`,
//! `ReputationMgr::Initialize`, `ReputationMgr::LoadFromDB`,
//! and `SendInitialReputations`).

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_db::character_load::ReputationRow;
use mysql::prelude::Queryable;

use crate::load_report::LoadReport;

/// Client build whose `faction` rows are selected
/// (`SUPPORTED_CLIENT_BUILD`).
const SUPPORTED_CLIENT_BUILD: u32 = 5875;

/// `FACTION_FLAG_*` (`ReputationMgr.h`).
pub const FACTION_FLAG_VISIBLE: u32 = 0x01;
pub const FACTION_FLAG_AT_WAR: u32 = 0x02;
pub const FACTION_FLAG_HIDDEN: u32 = 0x04;
pub const FACTION_FLAG_INVISIBLE_FORCED: u32 = 0x08;
pub const FACTION_FLAG_PEACE_FORCED: u32 = 0x10;
pub const FACTION_FLAG_INACTIVE: u32 = 0x20;

/// Stored `character_reputation.flags` bits
/// (`ReputationMgr::LoadFromDB`).
pub const FACTION_DB_VISIBLE: i32 = 0x01;
pub const FACTION_DB_ATWAR: i32 = 0x02;
pub const FACTION_DB_INACTIVE: i32 = 0x04;

/// `SMSG_INITIALIZE_FACTIONS` always carries 64 slots.
pub const INITIAL_FACTION_SLOTS: usize = 64;

/// The `faction` fields reputation initialization consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactionEntry {
    pub id: u32,
    pub reputation_index: i32,
    pub race_masks: [u32; 4],
    pub class_masks: [u32; 4],
    pub base_values: [i32; 4],
    pub flags: [u32; 4],
    /// `team` (`DBCStructure.h:318`): the team/parent
    /// faction id the reputation team-award half
    /// (`RewardReputation`) resolves through
    /// (`GetFactionEntry(pFactionEntry->team)`).
    pub team: u32,
}

impl FactionEntry {
    /// `FactionEntry::GetIndexFitTo`: first slot matching
    /// race and (empty or matching) class masks.
    #[must_use]
    pub fn index_fit_to(&self, race_mask: u32, class_mask: u32) -> Option<usize> {
        (0..4).find(|&index| {
            self.race_masks[index] & race_mask != 0
                && (self.class_masks[index] == 0 || self.class_masks[index] & class_mask != 0)
        })
    }
}

/// `FACTION_TEMPLATE_FLAG_ATTACK_PVP_ACTIVE_PLAYERS`
/// (`DBCEnums.h:81`): contested-guard faction.
pub const FACTION_TEMPLATE_FLAG_ATTACK_PVP_ACTIVE_PLAYERS: u32 = 0x0000_1000;

/// One `faction_template` row
/// (`FactionTemplateEntry`, `DBCStructure.h:342-352`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactionTemplateEntry {
    pub id: u32,
    /// Reputation faction this template maps to
    /// (`faction_id` column; `GetFactionId`).
    pub faction: u32,
    pub faction_flags: u32,
    pub our_mask: u32,
    pub friendly_mask: u32,
    pub hostile_mask: u32,
    pub enemy_factions: [u32; 4],
    pub friend_factions: [u32; 4],
}

impl FactionTemplateEntry {
    /// `IsFriendlyTo` (`DBCStructure.h:358-380`).
    #[must_use]
    pub fn is_friendly_to(&self, entry: &FactionTemplateEntry) -> bool {
        if entry.faction != 0 {
            for enemy in self.enemy_factions {
                if enemy == entry.faction {
                    return false;
                }
            }
            for friend in self.friend_factions {
                if friend == entry.faction {
                    return true;
                }
            }
        }
        self.friendly_mask & entry.our_mask != 0 || self.our_mask & entry.our_mask != 0
    }

    /// `IsHostileTo` (`DBCStructure.h:381-390`).
    #[must_use]
    pub fn is_hostile_to(&self, entry: &FactionTemplateEntry) -> bool {
        if entry.faction != 0 {
            for enemy in self.enemy_factions {
                if enemy == entry.faction {
                    return true;
                }
            }
            for friend in self.friend_factions {
                if friend == entry.faction {
                    return false;
                }
            }
        }
        self.hostile_mask & entry.our_mask != 0
    }

    /// `IsNeutralToAll`.
    #[must_use]
    pub fn is_neutral_to_all(&self) -> bool {
        self.enemy_factions.iter().all(|&f| f == 0)
            && self.hostile_mask == 0
            && self.friendly_mask == 0
    }

    /// `IsContestedGuardFaction`.
    #[must_use]
    pub fn is_contested_guard_faction(&self) -> bool {
        self.faction_flags & FACTION_TEMPLATE_FLAG_ATTACK_PVP_ACTIVE_PLAYERS != 0
    }
}

/// Loaded `faction` rows by entry and reputation-list index.
#[derive(Clone, Debug, Default)]
pub struct FactionStore {
    by_id: HashMap<u32, FactionEntry>,
    by_reputation_index: HashMap<i32, FactionEntry>,
    templates: HashMap<u32, FactionTemplateEntry>,
}

impl FactionStore {
    #[must_use]
    pub fn get(&self, faction_id: u32) -> Option<FactionEntry> {
        self.by_id.get(&faction_id).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    fn from_entries(entries: impl IntoIterator<Item = FactionEntry>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_reputation_index = HashMap::new();
        for entry in entries {
            by_id.insert(entry.id, entry);
            if entry.reputation_index >= 0 {
                by_reputation_index.insert(entry.reputation_index, entry);
            }
        }
        Self {
            by_id,
            by_reputation_index,
            templates: HashMap::new(),
        }
    }

    /// Attaches `faction_template` rows (the DB half
    /// of `LoadFactions`; also the test-fixture seam).
    pub fn set_templates(&mut self, templates: impl IntoIterator<Item = FactionTemplateEntry>) {
        self.templates = templates.into_iter().map(|t| (t.id, t)).collect();
    }

    /// `sFactionTemplateStore.LookupEntry(id)`.
    #[must_use]
    pub fn template(&self, template_id: u32) -> Option<FactionTemplateEntry> {
        self.templates.get(&template_id).copied()
    }

    /// Number of loaded faction templates.
    #[must_use]
    pub fn template_len(&self) -> usize {
        self.templates.len()
    }

    /// `WorldObject::GetFactionId` for a unit carrying
    /// `template_id` in `UNIT_FIELD_FACTIONTEMPLATE`:
    /// the template's reputation faction, else 0.
    #[must_use]
    pub fn faction_id_for_template(&self, template_id: u32) -> u32 {
        self.template(template_id).map_or(0, |t| t.faction)
    }

    /// `ReputationMgr::Initialize` plus `LoadFromDB`, reduced
    /// to the 64 `(flags, standing)` pairs on the wire.
    /// Standings are the stored deltas, exactly as
    /// `SendInitialReputations` sends `FactionState.Standing`.
    #[must_use]
    pub fn initial_slots(
        &self,
        race: u8,
        class: u8,
        rows: &[ReputationRow],
    ) -> [(u8, i32); INITIAL_FACTION_SLOTS] {
        let race_mask = race_mask(race);
        let class_mask = class_mask(class);
        let rows_by_faction: HashMap<u32, &ReputationRow> =
            rows.iter().map(|row| (row.faction, row)).collect();
        let mut states: HashMap<u32, (u32, i32)> = HashMap::new();

        for entry in self.by_id.values() {
            if entry.reputation_index >= 0 {
                states.insert(
                    entry.id,
                    runtime_state(
                        entry,
                        race_mask,
                        class_mask,
                        rows_by_faction.get(&entry.id).copied(),
                    ),
                );
            }
        }

        let mut slots = [(0u8, 0i32); INITIAL_FACTION_SLOTS];
        for entry in self.by_reputation_index.values() {
            let Ok(index) = usize::try_from(entry.reputation_index) else {
                continue;
            };
            if index >= INITIAL_FACTION_SLOTS {
                continue;
            }
            let (flags, standing) = states.get(&entry.id).copied().unwrap_or((0, 0));
            slots[index] = (flags as u8, standing);
        }
        slots
    }
}

/// One faction's `(flags, standing)` runtime state
/// (`ReputationMgr::Initialize` defaults overlaid with
/// one `LoadFromDB` row): shared by the login projection
/// (`FactionStore::initial_slots`) and the kill path, so
/// both compute the same flags from the same inputs.
#[must_use]
pub fn runtime_state(
    entry: &FactionEntry,
    race_mask: u32,
    class_mask: u32,
    row: Option<&ReputationRow>,
) -> (u32, i32) {
    let mut flags = default_flags(entry, race_mask, class_mask);
    let mut standing = 0;
    let Some(row) = row else {
        return (flags, standing);
    };
    standing = row.standing;

    if row.flags & FACTION_DB_VISIBLE != 0 && flags & FACTION_FLAG_INVISIBLE_FORCED == 0 {
        flags |= FACTION_FLAG_VISIBLE;
        flags &= !FACTION_FLAG_HIDDEN;
    }
    if row.flags & FACTION_DB_INACTIVE != 0 {
        flags |= FACTION_FLAG_INACTIVE;
    }
    if row.flags & FACTION_DB_ATWAR != 0 {
        set_at_war(&mut flags, true);
    } else if row.flags & FACTION_DB_VISIBLE != 0 {
        set_at_war(&mut flags, false);
    }

    let base = entry
        .index_fit_to(race_mask, class_mask)
        .map_or(0, |index| entry.base_values[index]);
    if base.saturating_add(standing) < -3000 {
        set_at_war(&mut flags, true);
    }
    (flags, standing)
}

#[must_use]
pub fn race_mask(race: u8) -> u32 {
    if race == 0 {
        0
    } else {
        1u32.checked_shl(u32::from(race - 1)).unwrap_or(0)
    }
}

#[must_use]
pub fn class_mask(class: u8) -> u32 {
    if class == 0 {
        0
    } else {
        1u32.checked_shl(u32::from(class - 1)).unwrap_or(0)
    }
}

fn default_flags(entry: &FactionEntry, race_mask: u32, class_mask: u32) -> u32 {
    entry
        .index_fit_to(race_mask, class_mask)
        .map_or(0, |index| entry.flags[index])
}

fn set_at_war(flags: &mut u32, on: bool) {
    if *flags & FACTION_FLAG_PEACE_FORCED != 0 {
        return;
    }
    if on {
        *flags |= FACTION_FLAG_AT_WAR;
    } else {
        *flags &= !FACTION_FLAG_AT_WAR;
    }
}

/// Loads `ObjectMgr::LoadFactions` for build 5875.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_factions(conn: &mut mysql::Conn) -> Result<FactionStore, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `reputation_list_id`, \
         `base_rep_race_mask1`, `base_rep_race_mask2`, `base_rep_race_mask3`, \
         `base_rep_race_mask4`, `base_rep_class_mask1`, `base_rep_class_mask2`, \
         `base_rep_class_mask3`, `base_rep_class_mask4`, `base_rep_value1`, \
         `base_rep_value2`, `base_rep_value3`, `base_rep_value4`, \
         `reputation_flags1`, `reputation_flags2`, `reputation_flags3`, `reputation_flags4`, \
         `team` \
         FROM `faction` t1 \
         WHERE `build` = (SELECT max(`build`) FROM `faction` t2 \
                          WHERE t1.`id` = t2.`id` AND `build` <= ?)",
        (SUPPORTED_CLIENT_BUILD,),
    )?;
    let mut store = FactionStore::from_entries(rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        FactionEntry {
            id: f.u32(0),
            reputation_index: f.i32(1),
            race_masks: [f.u32(2), f.u32(3), f.u32(4), f.u32(5)],
            class_masks: [f.u32(6), f.u32(7), f.u32(8), f.u32(9)],
            base_values: [f.i32(10), f.i32(11), f.i32(12), f.i32(13)],
            flags: [f.u32(14), f.u32(15), f.u32(16), f.u32(17)],
            team: f.u32(18),
        }
    }));
    // The `faction_template` half of `LoadFactions`
    // (progressive on `build`, as above).
    let template_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `id`, `faction_id`, `faction_flags`, `our_mask`, `friendly_mask`,          `hostile_mask`, `enemy_faction1`, `enemy_faction2`, `enemy_faction3`,          `enemy_faction4`, `friend_faction1`, `friend_faction2`, `friend_faction3`,          `friend_faction4` FROM `faction_template` t1          WHERE `build` = (SELECT max(`build`) FROM `faction_template` t2                           WHERE t1.`id` = t2.`id` AND `build` <= ?)",
        (SUPPORTED_CLIENT_BUILD,),
    )?;
    store.set_templates(template_rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        FactionTemplateEntry {
            id: f.u32(0),
            faction: f.u32(1),
            faction_flags: f.u32(2),
            our_mask: f.u32(3),
            friendly_mask: f.u32(4),
            hostile_mask: f.u32(5),
            enemy_factions: [f.u32(6), f.u32(7), f.u32(8), f.u32(9)],
            friend_factions: [f.u32(10), f.u32(11), f.u32(12), f.u32(13)],
        }
    }));
    Ok(store)
}

/// `MAX_SPILLOVER_FACTIONS` (`SharedDefines.h:130`).
pub const MAX_SPILLOVER_FACTIONS: usize = 4;

/// `ReputationOnKillEntry` (`ObjectMgr.h:290-301`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReputationOnKillEntry {
    pub repfaction1: u32,
    pub repfaction2: u32,
    pub is_teamaward1: bool,
    pub reputation_max_cap1: u32,
    pub repvalue1: i32,
    pub is_teamaward2: bool,
    pub reputation_max_cap2: u32,
    pub repvalue2: i32,
    pub team_dependent: bool,
}

/// `RepRewardRate` (`ObjectMgr.h:283-288`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepRewardRate {
    pub quest_rate: f32,
    pub creature_rate: f32,
    pub spell_rate: f32,
}

/// `RepSpilloverTemplate` (`ObjectMgr.h:303-308`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepSpilloverTemplate {
    pub faction: [u32; MAX_SPILLOVER_FACTIONS],
    pub faction_rate: [f32; MAX_SPILLOVER_FACTIONS],
    pub faction_rank: [u32; MAX_SPILLOVER_FACTIONS],
}

/// `m_RepOnKillMap` / `m_RepRewardRateMap` /
/// `m_RepSpilloverTemplateMap` (`ObjectMgr.h:1467-1469`).
#[derive(Clone, Debug, Default)]
pub struct ReputationKillStores {
    on_kill: HashMap<u32, ReputationOnKillEntry>,
    reward_rates: HashMap<u32, RepRewardRate>,
    spillover: HashMap<u32, RepSpilloverTemplate>,
}

impl ReputationKillStores {
    /// `GetReputationOnKillEntry`: `None` on miss, exactly
    /// like the fork's null return (the kill then grants
    /// no reputation).
    #[must_use]
    pub fn get_on_kill(&self, creature_entry: u32) -> Option<ReputationOnKillEntry> {
        self.on_kill.get(&creature_entry).copied()
    }

    /// `GetRepRewardRate`: `None` on miss (no multiplier,
    /// fork keeps `percent` unchanged).
    #[must_use]
    pub fn get_reward_rate(&self, faction_id: u32) -> Option<RepRewardRate> {
        self.reward_rates.get(&faction_id).copied()
    }

    /// `GetRepSpilloverTemplate`: `None` on miss (no
    /// spillover arms).
    #[must_use]
    pub fn get_spillover(&self, faction_id: u32) -> Option<RepSpilloverTemplate> {
        self.spillover.get(&faction_id).copied()
    }

    #[must_use]
    pub fn on_kill_len(&self) -> usize {
        self.on_kill.len()
    }

    #[must_use]
    pub fn reward_rate_len(&self) -> usize {
        self.reward_rates.len()
    }

    #[must_use]
    pub fn spillover_len(&self) -> usize {
        self.spillover.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.on_kill.is_empty() && self.reward_rates.is_empty() && self.spillover.is_empty()
    }

    /// Test seam mirroring `FactionStore::from_entries`.
    #[must_use]
    pub fn from_parts_for_tests(
        on_kill: Vec<(u32, ReputationOnKillEntry)>,
        reward_rates: Vec<(u32, RepRewardRate)>,
        spillover: Vec<(u32, RepSpilloverTemplate)>,
    ) -> Self {
        Self {
            on_kill: on_kill.into_iter().collect(),
            reward_rates: reward_rates.into_iter().collect(),
            spillover: spillover.into_iter().collect(),
        }
    }
}

/// `MAX_REPUTATION_RANK` (`SharedDefines.h:128`).
pub const MAX_REPUTATION_RANK: u32 = 8;

/// All three reputation-gain stores in fork boot order
/// (`World.cpp:1486-1492`: reward-rate, on-kill,
/// spillover). Empty tables yield empty stores and boot
/// continues, exactly like the fork's log-and-return.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_reputation_kill_stores(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    creature_templates: &crate::creature_template::CreatureTemplates,
    factions: &FactionStore,
    report: &mut LoadReport,
) -> Result<ReputationKillStores, DbError> {
    // `ObjectMgr::LoadAllIdentifiers`: every
    // `creature_template` entry, including rows above the
    // active patch (those skip silently below).
    let id_rows: Vec<mysql::Row> =
        conn.exec("SELECT DISTINCT `entry` FROM `creature_template`", ())?;
    let creature_entries: std::collections::HashSet<u32> = id_rows
        .into_iter()
        .map(|row| mangos_db::row::Fields::from_row(row).u32(0))
        .collect();
    let reward_rates = load_rep_reward_rates(conn, factions, report)?;
    let on_kill = load_reputation_on_kill(
        conn,
        wow_patch,
        &creature_entries,
        creature_templates,
        factions,
        report,
    )?;
    let spillover = load_rep_spillover_templates(conn, factions, report)?;
    Ok(ReputationKillStores {
        on_kill,
        reward_rates,
        spillover,
    })
}

/// `ObjectMgr::LoadReputationRewardRate`
/// (`ObjectMgr.cpp:8440-8505`): no patch gate; rows with
/// an unknown faction or any negative rate are skipped.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_rep_reward_rates(
    conn: &mut mysql::Conn,
    factions: &FactionStore,
    report: &mut LoadReport,
) -> Result<HashMap<u32, RepRewardRate>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `faction`, `quest_rate`, `creature_rate`, `spell_rate` \
         FROM `reputation_reward_rate`",
        (),
    )?;
    let mut map = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let faction_id = f.u32(0);
        let rate = RepRewardRate {
            quest_rate: f.f32(1),
            creature_rate: f.f32(2),
            spell_rate: f.f32(3),
        };
        if factions.get(faction_id).is_none() {
            report.error(format!(
                "Faction (faction.dbc) {faction_id} does not exist but is used in \
                 `reputation_reward_rate`, skipping"
            ));
            continue;
        }
        if rate.quest_rate < 0.0 || rate.creature_rate < 0.0 || rate.spell_rate < 0.0 {
            report.error(format!(
                "Table reputation_reward_rate has invalid rate for faction {faction_id}, skipping"
            ));
            continue;
        }
        map.insert(faction_id, rate);
    }
    Ok(map)
}

/// `ObjectMgr::LoadReputationOnKill`
/// (`ObjectMgr.cpp:8507-8583`): patch-gated per creature;
/// rows with an unknown creature or faction are skipped
/// (silently when the creature id exists at another
/// patch, mirroring `IsExistingCreatureId`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
#[allow(clippy::implicit_hasher)] // callers use default sets like the other loaders
pub fn load_reputation_on_kill(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    creature_entries: &std::collections::HashSet<u32>,
    creature_templates: &crate::creature_template::CreatureTemplates,
    factions: &FactionStore,
    report: &mut LoadReport,
) -> Result<HashMap<u32, ReputationOnKillEntry>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `creature_id`, `RewOnKillRepFaction1`, `RewOnKillRepFaction2`, \
         `IsTeamAward1`, `MaxStanding1`, `RewOnKillRepValue1`, \
         `IsTeamAward2`, `MaxStanding2`, `RewOnKillRepValue2`, `TeamDependent` \
         FROM `creature_onkill_reputation` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `creature_onkill_reputation` t2 \
                          WHERE t1.`creature_id` = t2.`creature_id` AND `patch` <= ?)",
        (wow_patch,),
    )?;
    let mut map = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let creature_id = f.u32(0);
        let entry = ReputationOnKillEntry {
            repfaction1: f.u32(1),
            repfaction2: f.u32(2),
            is_teamaward1: f.u8(3) != 0,
            reputation_max_cap1: f.u32(4),
            repvalue1: f.i32(5),
            is_teamaward2: f.u8(6) != 0,
            reputation_max_cap2: f.u32(7),
            repvalue2: f.i32(8),
            team_dependent: f.u8(9) != 0,
        };
        if creature_templates.get(creature_id).is_none() {
            if !creature_entries.contains(&creature_id) {
                report.error(format!(
                    "Table `creature_onkill_reputation` has data for nonexistent creature entry \
                     ({creature_id}), skipped"
                ));
            }
            continue;
        }
        if entry.repfaction1 != 0 && factions.get(entry.repfaction1).is_none() {
            report.error(format!(
                "Faction (faction.dbc) {} does not exist but is used in \
                 `creature_onkill_reputation`, skipping",
                entry.repfaction1
            ));
            continue;
        }
        if entry.repfaction2 != 0 && factions.get(entry.repfaction2).is_none() {
            report.error(format!(
                "Faction (faction.dbc) {} does not exist but is used in \
                 `creature_onkill_reputation`, skipping",
                entry.repfaction2
            ));
            continue;
        }
        map.insert(creature_id, entry);
    }
    Ok(map)
}

/// `ObjectMgr::LoadReputationSpilloverTemplate`
/// (`ObjectMgr.cpp:8585-8693`): per-slot validation uses
/// an inner continue (the row still loads); the redundant
/// whole-row faction checks run after.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_rep_spillover_templates(
    conn: &mut mysql::Conn,
    factions: &FactionStore,
    report: &mut LoadReport,
) -> Result<HashMap<u32, RepSpilloverTemplate>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `faction`, `faction1`, `rate_1`, `rank_1`, \
         `faction2`, `rate_2`, `rank_2`, `faction3`, `rate_3`, `rank_3`, \
         `faction4`, `rate_4`, `rank_4` \
         FROM `reputation_spillover_template`",
        (),
    )?;
    let mut map = HashMap::new();
    for row in rows {
        let f = mangos_db::row::Fields::from_row(row);
        let faction_id = f.u32(0);
        if factions.get(faction_id).is_none() {
            report.error(format!(
                "Faction (faction.dbc) {faction_id} does not exist but is used in \
                 `reputation_spillover_template`, skipping"
            ));
            continue;
        }
        let mut template = RepSpilloverTemplate {
            faction: [0; MAX_SPILLOVER_FACTIONS],
            faction_rate: [0.0; MAX_SPILLOVER_FACTIONS],
            faction_rank: [0; MAX_SPILLOVER_FACTIONS],
        };
        for slot in 0..MAX_SPILLOVER_FACTIONS {
            let base = 1 + slot * 3;
            template.faction[slot] = f.u32(base);
            template.faction_rate[slot] = f.f32(base + 1);
            template.faction_rank[slot] = f.u32(base + 2);
            if template.faction[slot] != 0 {
                let Some(spill) = factions.get(template.faction[slot]) else {
                    report.error(format!(
                        "Spillover faction (faction.dbc) {} does not exist but is used in \
                         `reputation_spillover_template` for faction {faction_id}, skipping",
                        template.faction[slot]
                    ));
                    template.faction[slot] = 0;
                    continue;
                };
                if spill.reputation_index < 0 {
                    report.error(format!(
                        "Spillover faction (faction.dbc) {} for faction {faction_id} in \
                         `reputation_spillover_template` cannot be listed for client, skipping",
                        template.faction[slot]
                    ));
                    template.faction[slot] = 0;
                    continue;
                }
                if template.faction_rank[slot] >= MAX_REPUTATION_RANK {
                    report.error(format!(
                        "Rank {} used in `reputation_spillover_template` for spillover faction {} \
                         is not valid, skipping",
                        template.faction_rank[slot], template.faction[slot]
                    ));
                    template.faction[slot] = 0;
                }
            }
        }
        // Redundant whole-row checks (`ObjectMgr.cpp:8660-83`):
        // every surviving slot re-resolves by construction,
        // so this only re-reports a genuinely missing faction.
        let mut skip_row = false;
        for slot in 0..MAX_SPILLOVER_FACTIONS {
            if template.faction[slot] != 0 && factions.get(template.faction[slot]).is_none() {
                report.error(format!(
                    "Faction (faction.dbc) {} does not exist but is used in \
                     `reputation_spillover_template`, skipping",
                    template.faction[slot]
                ));
                skip_row = true;
            }
        }
        if skip_row {
            continue;
        }
        map.insert(faction_id, template);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, reputation_index: i32, flags: u32, base: i32) -> FactionEntry {
        FactionEntry {
            id,
            reputation_index,
            race_masks: [0xFFFF_FFFF, 0, 0, 0],
            class_masks: [0, 0, 0, 0],
            base_values: [base, 0, 0, 0],
            flags: [flags, 0, 0, 0],
            team: 0,
        }
    }

    #[test]
    fn initial_slots_apply_defaults_and_saved_flags() {
        let store = FactionStore::from_entries([
            entry(72, 6, 0, 0),
            entry(47, 8, FACTION_FLAG_PEACE_FORCED, -5000),
            entry(999, -1, FACTION_FLAG_VISIBLE, 0),
        ]);
        let rows = [
            ReputationRow {
                faction: 72,
                standing: 125,
                flags: FACTION_DB_VISIBLE | FACTION_DB_INACTIVE,
            },
            ReputationRow {
                faction: 47,
                standing: 0,
                flags: FACTION_DB_VISIBLE | FACTION_DB_ATWAR,
            },
        ];

        let slots = store.initial_slots(1, 1, &rows);
        assert_eq!(
            slots[6],
            ((FACTION_FLAG_VISIBLE | FACTION_FLAG_INACTIVE) as u8, 125)
        );
        assert_eq!(
            slots[8].0,
            (FACTION_FLAG_PEACE_FORCED | FACTION_FLAG_VISIBLE) as u8
        );
        assert_eq!(slots[8].1, 0);
    }

    #[test]
    fn initial_slots_force_war_at_hostile_threshold() {
        let store = FactionStore::from_entries([entry(72, 0, FACTION_FLAG_VISIBLE, -2900)]);
        let rows = [ReputationRow {
            faction: 72,
            standing: -101,
            flags: FACTION_DB_VISIBLE,
        }];
        let slots = store.initial_slots(2, 1, &rows);
        assert_eq!(
            slots[0],
            ((FACTION_FLAG_VISIBLE | FACTION_FLAG_AT_WAR) as u8, -101)
        );
    }
}
