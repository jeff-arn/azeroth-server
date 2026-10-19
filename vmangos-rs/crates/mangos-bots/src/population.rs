//! Random-bot population control (`PlayerBotMgr`
//! decision core): `playerbot` rows with login
//! chances, peak-hour cosine scaling, weighted
//! random add/remove. Pure state; host performs
//! bot login/logout.

use std::collections::HashMap;

/// `RandomBot.*` + `PlayerBot.*` config subset.
#[derive(Clone, Debug)]
pub struct PopulationConfig {
    pub enable_random_bots: bool,
    pub min_bots: u32,
    pub max_bots: u32,
    pub refresh_ms: u64,
    pub update_ms: u64,
    pub debug: bool,
    pub peak_hour: u32,
    pub off_peak_percent: u32,
    pub timezone_offset: i32,
}

impl Default for PopulationConfig {
    fn default() -> Self {
        Self {
            enable_random_bots: false,
            min_bots: 3,
            max_bots: 10,
            refresh_ms: 60_000,
            update_ms: 10_000,
            debug: false,
            peak_hour: 20,
            off_peak_percent: 50,
            timezone_offset: -4,
        }
    }
}

/// `PB_STATE_*`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BotEntryState {
    Offline = 0,
    Loading = 1,
    Online = 2,
}

/// One `playerbot` row (`PlayerBotEntry` subset).
#[derive(Clone, Debug)]
pub struct BotEntry {
    pub guid: u32,
    pub chance: u32,
    pub ai: String,
    pub state: BotEntryState,
    pub custom: bool,
    pub chat_bot: bool,
}

/// `PlayerBotStats`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PopulationStats {
    pub online_count: u32,
    pub loading_count: u32,
    pub total_bots: u32,
    pub online_chat: u32,
    pub conf_max_online: u32,
    pub conf_min_online: u32,
}

/// One population decision for the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PopulationAction {
    Login(u32),
    Logout(u32),
}

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn urand(pick: &mut Pick<'_>, lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + pick(0, hi - lo) }
}

/// Population manager core.
pub struct PopulationCore {
    pub config: PopulationConfig,
    entries: Vec<BotEntry>,
    by_guid: HashMap<u32, usize>,
    total_chance: u32,
    update_deadline_ms: u64,
    refresh_deadline_ms: u64,
}

impl PopulationCore {
    /// `Load` steps 2/4/5/7 (rows supplied by host;
    /// config clamped against row count like fork).
    #[must_use]
    pub fn load(mut config: PopulationConfig, rows: Vec<(u32, u32, String)>) -> Self {
        let mut entries = Vec::with_capacity(rows.len());
        let mut by_guid = HashMap::new();
        let mut total_chance = 0u32;
        for (guid, chance, ai) in rows {
            by_guid.insert(guid, entries.len());
            total_chance = total_chance.saturating_add(chance);
            entries.push(BotEntry {
                guid,
                chance,
                ai,
                state: BotEntryState::Offline,
                custom: false,
                chat_bot: false,
            });
        }
        let total = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        if config.min_bots >= total && total > 0 {
            config.min_bots = total - 1;
        }
        if config.max_bots > total {
            config.max_bots = total;
        }
        if config.max_bots <= config.min_bots {
            config.max_bots = config.min_bots + 1;
        }
        Self {
            config,
            entries,
            by_guid,
            total_chance,
            update_deadline_ms: 0,
            refresh_deadline_ms: 0,
        }
    }

    #[must_use]
    pub fn stats(&self) -> PopulationStats {
        let mut stats = PopulationStats {
            total_bots: u32::try_from(self.entries.len()).unwrap_or(u32::MAX),
            conf_max_online: self.config.max_bots,
            conf_min_online: self.config.min_bots,
            ..PopulationStats::default()
        };
        for entry in &self.entries {
            match entry.state {
                BotEntryState::Online => {
                    if entry.chat_bot {
                        stats.online_chat += 1;
                    } else if !entry.custom {
                        stats.online_count += 1;
                    }
                }
                BotEntryState::Loading => stats.loading_count += 1,
                BotEntryState::Offline => {}
            }
        }
        stats
    }

    /// `GetScaledBotTarget` (host supplies the
    /// offset-adjusted local hour like the fork's
    /// `gmtime` + `RandomBot.TimezoneOffset`).
    #[must_use]
    pub fn scaled_target(&self, local_hour: u32) -> u32 {
        let hour = i32::try_from(local_hour % 24).unwrap_or(0);
        let peak = i32::try_from(self.config.peak_hour).unwrap_or(20);
        let mut dist = (hour - peak).abs();
        if dist > 12 {
            dist = 24 - dist;
        }
        #[allow(clippy::cast_precision_loss)]
        let t = dist as f32 / 12.0;
        // Fork literally uses 3.14159; keep numeric parity.
        #[allow(clippy::approx_constant, clippy::manual_midpoint)]
        let factor = (1.0 + (t * 3.14159).cos()) / 2.0;
        #[allow(clippy::cast_precision_loss)]
        let off_peak = self.config.off_peak_percent as f32 / 100.0;
        let scaled = off_peak + factor * (1.0 - off_peak);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let target = (self.config.max_bots as f32 * scaled) as u32;
        target.max(1)
    }

    /// `AddRandomBot` weighted walk (fork compares
    /// before subtracting; mirrored exactly).
    pub fn add_random(&mut self, pick: &mut Pick<'_>) -> Option<u32> {
        let mut alea = urand(pick, 0, self.total_chance);
        for entry in &mut self.entries {
            if entry.state != BotEntryState::Offline || entry.custom {
                continue;
            }
            let chance = entry.chance;
            if chance >= alea {
                entry.state = BotEntryState::Loading;
                return Some(entry.guid);
            }
            alea = alea.wrapping_sub(chance);
        }
        None
    }

    /// `DeleteRandomBot` (fork rolls 0..=online and
    /// matches the running 1-based count).
    pub fn delete_random(&mut self, pick: &mut Pick<'_>) -> Option<u32> {
        let online = self.stats().online_count;
        if online < 1 {
            return None;
        }
        let id_delete = urand(pick, 0, online);
        let mut passed = 0u32;
        for entry in &mut self.entries {
            if !entry.custom && !entry.chat_bot && entry.state == BotEntryState::Online {
                passed += 1;
                if passed == id_delete {
                    entry.state = BotEntryState::Offline;
                    return Some(entry.guid);
                }
            }
        }
        None
    }

    /// `AddOrRemoveBot`.
    pub fn add_or_remove(
        &mut self,
        local_hour: u32,
        pick: &mut Pick<'_>,
    ) -> Option<PopulationAction> {
        let target = self.scaled_target(local_hour);
        let alea = urand(pick, target * 9 / 10, target);
        let online = self.stats().online_count;
        if alea > online {
            self.add_random(pick).map(PopulationAction::Login)
        } else if online > target {
            self.delete_random(pick).map(PopulationAction::Logout)
        } else {
            None
        }
    }

    /// Periodic gate (`Update`): refresh decision
    /// every `RandomBot.Refresh` once enabled.
    pub fn update(
        &mut self,
        now_ms: u64,
        local_hour: u32,
        pick: &mut Pick<'_>,
    ) -> Vec<PopulationAction> {
        if now_ms < self.update_deadline_ms {
            return Vec::new();
        }
        self.update_deadline_ms = now_ms.saturating_add(self.config.update_ms);
        if !self.config.enable_random_bots || now_ms < self.refresh_deadline_ms {
            return Vec::new();
        }
        self.refresh_deadline_ms = now_ms.saturating_add(self.config.refresh_ms);
        self.add_or_remove(local_hour, pick).into_iter().collect()
    }

    /// Initial population at load (`Load` step 6).
    pub fn initial_bots(&mut self, pick: &mut Pick<'_>) -> Vec<PopulationAction> {
        if !self.config.enable_random_bots {
            return Vec::new();
        }
        let mut actions = Vec::new();
        for _ in 0..self.config.min_bots {
            match self.add_random(pick) {
                Some(guid) => actions.push(PopulationAction::Login(guid)),
                None => break,
            }
        }
        actions
    }

    /// `AddAllBots`: every offline entry starts loading.
    pub fn add_all(&mut self) -> Vec<u32> {
        let mut guids = Vec::new();
        for entry in &mut self.entries {
            if entry.state == BotEntryState::Offline {
                entry.state = BotEntryState::Loading;
                guids.push(entry.guid);
            }
        }
        guids
    }

    /// All entry GUIDs (`DeleteAll` sweep order source).
    #[must_use]
    pub fn entry_guids(&self) -> Vec<u32> {
        self.entries.iter().map(|e| e.guid).collect()
    }

    /// Host outcome reports.
    pub fn on_login(&mut self, guid: u32) {
        if let Some(entry) = self.entry_mut(guid) {
            entry.state = BotEntryState::Online;
        }
    }

    pub fn on_logout(&mut self, guid: u32) {
        if let Some(entry) = self.entry_mut(guid) {
            entry.state = BotEntryState::Offline;
        }
    }

    /// Mark a manually added bot (`AddBot`) loading;
    /// inserts a temporary entry like the fork.
    pub fn mark_manual_add(&mut self, guid: u32) {
        if let Some(entry) = self.entry_mut(guid) {
            entry.state = BotEntryState::Loading;
        } else {
            self.by_guid.insert(guid, self.entries.len());
            self.entries.push(BotEntry {
                guid,
                chance: 10,
                ai: String::new(),
                state: BotEntryState::Loading,
                custom: false,
                chat_bot: false,
            });
        }
    }

    /// Apply director chance writes locally.
    pub fn set_chance(&mut self, guid: u32, chance: u32) {
        let Some(idx) = self.by_guid.get(&guid).copied() else {
            return;
        };
        let Some(entry) = self.entries.get_mut(idx) else {
            return;
        };
        let old = entry.chance;
        entry.chance = chance;
        self.total_chance = self.total_chance.saturating_sub(old).saturating_add(chance);
    }

    /// `Start` / stop (`.bot start|stop` flips the
    /// random-bots switch; stop has no fork body
    /// beyond the mgr flag path used here).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enable_random_bots = enabled;
    }

    fn entry_mut(&mut self, guid: u32) -> Option<&mut BotEntry> {
        let idx = *self.by_guid.get(&guid)?;
        self.entries.get_mut(idx)
    }
}
