//! Guild bot roster/presence (`GuildBotMgr` core):
//! roster + spec weights, presence target curve,
//! raid composition, gear scoring. Pure state;
//! host performs logins, guild writes, level sync.

use std::collections::HashMap;

use crate::BotRole;
use crate::chat::BotPersonality;

/// `GuildBot.*` config subset.
#[derive(Clone, Debug)]
pub struct GuildBotConfig {
    pub enabled: bool,
    pub account_id: u32,
    pub presence_enabled: bool,
    pub presence_check_interval_ms: u64,
    pub presence_peak_hour: u32,
    pub presence_peak_count: u32,
    pub presence_off_peak_count: u32,
    pub presence_timezone_offset: i32,
}

impl Default for GuildBotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            account_id: 98,
            presence_enabled: true,
            presence_check_interval_ms: 300_000,
            presence_peak_hour: 20,
            presence_peak_count: 14,
            presence_off_peak_count: 3,
            presence_timezone_offset: 0,
        }
    }
}

/// `GuildBotRole`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuildBotRole {
    Tank = 0,
    Healer = 1,
    Dps = 2,
}

/// `GuildBotGearTier`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GearTier {
    Fresh60 = 0,
    DungeonGeared = 1,
    PreRaidBis = 2,
}

/// `GuildBotInfo` (field-for-field mirror).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct GuildBotInfo {
    pub guid: u32,
    pub name: String,
    pub race: u8,
    pub class_id: u8,
    pub spec: String,
    pub role: GuildBotRole,
    pub raid_assigned: bool,
    pub active: bool,
    pub gear_score: f32,
    pub gear_tier: GearTier,
    pub has_loot_upgrades: bool,
    pub online: bool,
    pub personality: BotPersonality,
}

/// `GuildBotSpecWeights`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpecWeights {
    pub stamina: f32,
    pub strength: f32,
    pub agility: f32,
    pub intellect: f32,
    pub spirit: f32,
    pub attack_power: f32,
    pub spell_power: f32,
    pub healing_power: f32,
    pub hit_rating: f32,
    pub crit_rating: f32,
    pub defense: f32,
    pub dodge: f32,
    pub parry: f32,
    pub block: f32,
    pub mp5: f32,
}

/// Item stat ids scored by the fork's gear math
/// (`ITEM_MOD_*` subset it actually reads).
pub mod item_mod {
    pub const STAMINA: u32 = 7;
    pub const STRENGTH: u32 = 4;
    pub const AGILITY: u32 = 3;
    pub const INTELLECT: u32 = 5;
    pub const SPIRIT: u32 = 6;
}

/// `BotPersonalityFromString`.
#[must_use]
pub fn personality_from_str(text: &str) -> BotPersonality {
    match text {
        "enthusiastic" => BotPersonality::Enthusiastic,
        "chill" => BotPersonality::Chill,
        "sarcastic" => BotPersonality::Sarcastic,
        "quiet" => BotPersonality::Quiet,
        _ => BotPersonality::Generic,
    }
}

/// Roster-role → behaviour role (fork's assemble
/// switch, shared by raid assembly + conversion).
#[must_use]
pub fn behavior_role(info: &GuildBotInfo) -> BotRole {
    match info.role {
        GuildBotRole::Tank => BotRole::Tank,
        GuildBotRole::Healer => BotRole::Healer,
        GuildBotRole::Dps => match info.class_id {
            1 | 4 => BotRole::MeleeDps,
            2 => {
                if info.spec == "retribution" {
                    BotRole::MeleeDps
                } else {
                    BotRole::RangeDps
                }
            }
            7 => {
                if info.spec == "enhancement" {
                    BotRole::MeleeDps
                } else {
                    BotRole::RangeDps
                }
            }
            11 => {
                if info.spec == "feral_cat" {
                    BotRole::MeleeDps
                } else {
                    BotRole::RangeDps
                }
            }
            _ => BotRole::RangeDps,
        },
    }
}

/// One presence/level-sync decision for the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuildBotAction {
    LoginPresence(u32),
    LogoutPresence(u32),
    SyncLevels,
}

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn urand(pick: &mut Pick<'_>, lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + pick(0, hi - lo) }
}

const LEVEL_SYNC_INTERVAL_MS: u64 = 5 * 60 * 1000;

/// `GuildBotMgr` core state.
pub struct GuildBotCore {
    pub config: GuildBotConfig,
    pub guild_id: u32,
    roster: Vec<GuildBotInfo>,
    by_guid: HashMap<u32, usize>,
    weights: HashMap<String, SpecWeights>,
    presence_deadline_ms: u64,
    level_sync_deadline_ms: u64,
    started: bool,
}

impl GuildBotCore {
    #[must_use]
    pub fn load(
        config: GuildBotConfig,
        roster: Vec<GuildBotInfo>,
        weights: Vec<(u8, String, SpecWeights)>,
        guild_id: u32,
    ) -> Self {
        let by_guid = roster
            .iter()
            .enumerate()
            .map(|(idx, info)| (info.guid, idx))
            .collect();
        Self {
            config,
            guild_id,
            roster,
            by_guid,
            weights: weights
                .into_iter()
                .map(|(cls, spec, w)| (format!("{cls}:{spec}"), w))
                .collect(),
            presence_deadline_ms: 0,
            level_sync_deadline_ms: 0,
            started: false,
        }
    }

    #[must_use]
    pub fn roster(&self) -> &[GuildBotInfo] {
        &self.roster
    }

    #[must_use]
    pub fn weights_count(&self) -> usize {
        self.weights.len()
    }

    /// `GetBotInfo`.
    #[must_use]
    pub fn bot_info(&self, guid: u32) -> Option<&GuildBotInfo> {
        self.by_guid
            .get(&guid)
            .and_then(|idx| self.roster.get(*idx))
    }

    /// `IsGuildBot`.
    #[must_use]
    pub fn is_guild_bot(&self, guid: u32) -> bool {
        self.by_guid.contains_key(&guid)
    }

    /// `IsPresenceBot` (roster `online` flag).
    #[must_use]
    pub fn is_presence_bot(&self, guid: u32) -> bool {
        self.bot_info(guid).is_some_and(|info| info.online)
    }

    /// `GetOnlineGuildBotCount`.
    #[must_use]
    pub fn online_count(&self) -> u32 {
        u32::try_from(self.roster.iter().filter(|info| info.online).count()).unwrap_or(u32::MAX)
    }

    pub fn set_online(&mut self, guid: u32, online: bool) {
        if let Some(info) = self
            .by_guid
            .get(&guid)
            .copied()
            .and_then(|idx| self.roster.get_mut(idx))
        {
            info.online = online;
        }
    }

    /// `GetSpecWeights`.
    #[must_use]
    pub fn spec_weights(&self, class_id: u8, spec: &str) -> Option<SpecWeights> {
        self.weights.get(&format!("{class_id}:{spec}")).copied()
    }

    /// `GetTargetOnlineCount` (host supplies the
    /// offset-adjusted local hour).
    #[must_use]
    pub fn target_online_count(&self, local_hour: u32) -> u32 {
        let hour = i32::try_from(local_hour % 24).unwrap_or(0);
        let peak = i32::try_from(self.config.presence_peak_hour).unwrap_or(20);
        let mut dist = (hour - peak).abs();
        if dist > 12 {
            dist = 24 - dist;
        }
        #[allow(clippy::cast_precision_loss)]
        let t = dist as f32 / 12.0;
        // Fork literally uses 3.14159; keep numeric parity.
        #[allow(clippy::approx_constant, clippy::manual_midpoint)]
        let factor = (1.0 + (t * 3.14159).cos()) / 2.0;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let target = (self.config.presence_off_peak_count as f32
            + factor
                * (self.config.presence_peak_count as f32
                    - self.config.presence_off_peak_count as f32)) as u32;
        target.min(u32::try_from(self.roster.len()).unwrap_or(u32::MAX))
    }

    /// `Update`: presence churn + level-sync tick
    /// (first presence check ~30s after start,
    /// like the fork's preset timer).
    pub fn update(
        &mut self,
        now_ms: u64,
        local_hour: u32,
        pick: &mut Pick<'_>,
    ) -> Vec<GuildBotAction> {
        if !self.config.enabled || self.guild_id == 0 {
            return Vec::new();
        }
        if !self.started {
            self.started = true;
            self.presence_deadline_ms = now_ms.saturating_add(30_000);
            self.level_sync_deadline_ms = now_ms.saturating_add(LEVEL_SYNC_INTERVAL_MS);
        }
        let mut actions = Vec::new();
        if now_ms >= self.level_sync_deadline_ms {
            self.level_sync_deadline_ms = now_ms.saturating_add(LEVEL_SYNC_INTERVAL_MS);
            if !self.roster.is_empty() {
                actions.push(GuildBotAction::SyncLevels);
            }
        }
        if self.config.presence_enabled && now_ms >= self.presence_deadline_ms {
            self.presence_deadline_ms =
                now_ms.saturating_add(self.config.presence_check_interval_ms);
            actions.extend(self.update_presence(local_hour, pick));
        }
        actions
    }

    /// `UpdatePresence`.
    fn update_presence(&mut self, local_hour: u32, pick: &mut Pick<'_>) -> Vec<GuildBotAction> {
        if self.roster.is_empty() {
            return Vec::new();
        }
        let target = self.target_online_count(local_hour);
        let current = self.online_count();
        let mut actions = Vec::new();
        if current < target {
            let mut offline: Vec<u32> = self
                .roster
                .iter()
                .filter(|info| !info.online && info.active)
                .map(|info| info.guid)
                .collect();
            for _ in 0..(target - current) {
                if offline.is_empty() {
                    break;
                }
                let idx = usize::try_from(urand(
                    pick,
                    0,
                    u32::try_from(offline.len()).unwrap_or(1).saturating_sub(1),
                ))
                .unwrap_or(0)
                .min(offline.len() - 1);
                actions.push(GuildBotAction::LoginPresence(offline.remove(idx)));
            }
        } else if current > target {
            let mut online: Vec<u32> = self
                .roster
                .iter()
                .filter(|info| info.online)
                .map(|info| info.guid)
                .collect();
            for _ in 0..(current - target) {
                if online.is_empty() {
                    break;
                }
                let idx = usize::try_from(urand(
                    pick,
                    0,
                    u32::try_from(online.len()).unwrap_or(1).saturating_sub(1),
                ))
                .unwrap_or(0)
                .min(online.len() - 1);
                actions.push(GuildBotAction::LogoutPresence(online.remove(idx)));
            }
        }
        actions
    }

    /// Raid composition counts (`.guildraid check`):
    /// tanks, healers, melee, ranged, then Mark /
    /// Fortitude / Kings / `Trueshot` coverage flags.
    #[must_use]
    pub fn raid_composition(&self) -> (u32, u32, u32, u32, bool, bool, bool, bool) {
        let (mut tanks, mut healers, mut melee, mut ranged) = (0, 0, 0, 0);
        let (mut mark, mut fort, mut kings, mut trueshot) = (false, false, false, false);
        for info in self.roster.iter().filter(|i| i.raid_assigned && i.active) {
            match behavior_role(info) {
                BotRole::Tank => tanks += 1,
                BotRole::Healer => healers += 1,
                BotRole::MeleeDps => melee += 1,
                _ => ranged += 1,
            }
            match info.class_id {
                11 => mark = true,
                5 => fort = true,
                2 => kings = true,
                3 => trueshot = true,
                _ => {}
            }
        }
        (tanks, healers, melee, ranged, mark, fort, kings, trueshot)
    }
}

/// Fork gear scoring for one item (`scoreItem`):
/// primary stats × weights + tank armor term.
#[must_use]
pub fn score_item(weights: &SpecWeights, stats: &[(u32, i32)], armor: u32) -> f32 {
    let mut score = 0.0f32;
    for (stat_type, value) in stats {
        if *value == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let value = *value as f32;
        score += value
            * match *stat_type {
                item_mod::STAMINA => weights.stamina,
                item_mod::STRENGTH => weights.strength,
                item_mod::AGILITY => weights.agility,
                item_mod::INTELLECT => weights.intellect,
                item_mod::SPIRIT => weights.spirit,
                _ => 0.0,
            };
    }
    if weights.defense > 0.0 || weights.dodge > 0.0 {
        #[allow(clippy::cast_precision_loss)]
        {
            score += armor as f32 * 0.01 * (weights.defense + weights.dodge);
        }
    }
    score
}
