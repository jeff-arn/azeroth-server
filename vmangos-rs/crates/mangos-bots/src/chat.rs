//! Ambient bot chat (`BotChatManager` core):
//! config, per-bot personalities, per-channel
//! cooldowns, trade/guild interval timers on
//! absolute deadlines, delayed whispers, and
//! party event hooks. Pure state machine: the
//! host feeds bot snapshots and applies the
//! emitted [`ChatAction`]s. Zone general-chat
//! chains/debates land with the world-bot
//! director (E.6).

use std::collections::HashMap;

use mangos_guid::ObjectGuid;

use crate::chat_pools;
use crate::host::BotChatKind;

/// `BotChat.Enable` etc. (times in seconds, as
/// in the fork's config keys).
#[derive(Clone, Debug)]
pub struct ChatConfig {
    pub enabled: bool,
    pub trade_interval_min_s: u32,
    pub trade_interval_max_s: u32,
    pub guild_interval_min_s: u32,
    pub guild_interval_max_s: u32,
    pub cooldown_trade_s: u32,
    pub cooldown_guild_s: u32,
    pub cooldown_party_s: u32,
    pub cooldown_say_s: u32,
    pub cooldown_general_s: u32,
    pub cooldown_scale_threshold: u32,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trade_interval_min_s: 120,
            trade_interval_max_s: 300,
            guild_interval_min_s: 300,
            guild_interval_max_s: 900,
            cooldown_trade_s: 300,
            cooldown_guild_s: 180,
            cooldown_party_s: 30,
            cooldown_say_s: 120,
            cooldown_general_s: 120,
            cooldown_scale_threshold: 30,
        }
    }
}

/// `BotPersonality`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BotPersonality {
    Generic = 0,
    Enthusiastic = 1,
    Chill = 2,
    Sarcastic = 3,
    Quiet = 4,
}

/// One emitted chat side effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    Say {
        bot: ObjectGuid,
        kind: BotChatKind,
        text: String,
    },
    Whisper {
        bot: ObjectGuid,
        target: ObjectGuid,
        text: String,
    },
    /// Trade-channel line (`Channel::Say` with
    /// skipCheck on "Trade - City").
    TradeChannel {
        bot: ObjectGuid,
        text: String,
    },
    Emote {
        bot: ObjectGuid,
        emote_id: u32,
    },
}

/// Snapshot of one online bot for chat decisions.
#[derive(Clone, Copy, Debug)]
pub struct BotChatInfo {
    pub guid: ObjectGuid,
    pub class_id: u8,
    pub level: u32,
    pub is_alive: bool,
    pub in_city: bool,
    pub is_alliance: bool,
    pub guild_id: u32,
    pub party: Option<u64>,
}

const CD_TRADE: u8 = 0;
const CD_GUILD: u8 = 1;
const CD_PARTY: u8 = 2;

/// `BotChatManager` core state.
pub struct ChatCore {
    pub config: ChatConfig,
    /// (bot low, channel) -> next-allowed unix secs.
    cooldowns: HashMap<(u32, u8), u64>,
    personalities: HashMap<u32, BotPersonality>,
    pending_whispers: Vec<PendingWhisper>,
    trade_deadline_ms: u64,
    guild_deadline_ms: u64,
    cleanup_deadline_ms: u64,
    started: bool,
}

struct PendingWhisper {
    bot: ObjectGuid,
    target: ObjectGuid,
    text: String,
    due_ms: u64,
}

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn urand(pick: &mut Pick<'_>, lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + pick(0, hi - lo) }
}

impl ChatCore {
    #[must_use]
    pub fn new(config: ChatConfig) -> Self {
        Self {
            config,
            cooldowns: HashMap::new(),
            personalities: HashMap::new(),
            pending_whispers: Vec::new(),
            trade_deadline_ms: 0,
            guild_deadline_ms: 0,
            cleanup_deadline_ms: 0,
            started: false,
        }
    }

    /// `GetPersonality` (deterministic per guid).
    /// `GetPersonality`: guild-roster personality
    /// when the host registered one, else Generic
    /// (the fork never randomizes personalities).
    pub fn personality(&mut self, guid: ObjectGuid, _pick: &mut Pick<'_>) -> BotPersonality {
        self.personalities
            .get(&guid.counter())
            .copied()
            .unwrap_or(BotPersonality::Generic)
    }

    /// Register a roster personality (`guild_bot_roster`).
    pub fn set_personality(&mut self, guid_low: u32, personality: BotPersonality) {
        self.personalities.insert(guid_low, personality);
    }

    fn can_chat(&self, low: u32, channel: u8, now_s: u64, total_bots: usize) -> bool {
        let Some(until) = self.cooldowns.get(&(low, channel)) else {
            return true;
        };
        if now_s >= *until {
            return true;
        }
        // Scale relief: with few bots online the
        // fork shortens cooldowns proportionally.
        if total_bots > 0 && (total_bots as u64) < u64::from(self.config.cooldown_scale_threshold) {
            let base = self.channel_cooldown_s(channel);
            let scaled = base.saturating_mul(u32::try_from(total_bots).unwrap_or(1).max(1))
                / self.config.cooldown_scale_threshold.max(1);
            let started = until.saturating_sub(u64::from(base));
            return now_s >= started.saturating_add(u64::from(scaled));
        }
        false
    }

    fn channel_cooldown_s(&self, channel: u8) -> u32 {
        match channel {
            CD_TRADE => self.config.cooldown_trade_s,
            CD_GUILD => self.config.cooldown_guild_s,
            _ => self.config.cooldown_party_s,
        }
    }

    fn record(&mut self, low: u32, channel: u8, now_s: u64) {
        let cd = self.channel_cooldown_s(channel);
        self.cooldowns
            .insert((low, channel), now_s.saturating_add(u64::from(cd)));
    }

    /// `PickPersonalityMessage`: prefer matching
    /// personality, then generic, then any.
    pub fn pick_personality_line(
        pool: &[chat_pools::PersonalityLine],
        personality: BotPersonality,
        pick: &mut Pick<'_>,
    ) -> Option<&'static str> {
        if pool.is_empty() {
            return None;
        }
        let matching: Vec<&chat_pools::PersonalityLine> = pool
            .iter()
            .filter(|l| l.personality == personality as u8)
            .collect();
        let candidates: Vec<&chat_pools::PersonalityLine> = if matching.is_empty() {
            pool.iter().filter(|l| l.personality == 0).collect()
        } else {
            matching
        };
        let candidates = if candidates.is_empty() {
            pool.iter().collect()
        } else {
            candidates
        };
        let idx = usize::try_from(urand(
            pick,
            0,
            u32::try_from(candidates.len())
                .unwrap_or(1)
                .saturating_sub(1),
        ))
        .unwrap_or(0);
        Some(candidates[idx.min(candidates.len() - 1)].text)
    }

    fn pick_line(pool: &[&str], pick: &mut Pick<'_>) -> Option<String> {
        if pool.is_empty() {
            return None;
        }
        let idx = usize::try_from(urand(
            pick,
            0,
            u32::try_from(pool.len()).unwrap_or(1).saturating_sub(1),
        ))
        .unwrap_or(0);
        Some(pool[idx.min(pool.len() - 1)].to_owned())
    }

    /// `PickTradeMessage`.
    pub fn pick_trade_message(bot: &BotChatInfo, pick: &mut Pick<'_>) -> Option<String> {
        if bot.level < 40 {
            return Self::pick_line(chat_pools::TRADE_MESSAGES_LOWLEVEL, pick);
        }
        if bot.class_id == 8 && urand(pick, 1, 100) <= 50 {
            return if bot.is_alliance {
                Self::pick_line(chat_pools::TRADE_MESSAGES_PORTALS, pick)
            } else {
                Self::pick_line(chat_pools::TRADE_MESSAGES_PORTALS_HORDE, pick)
            };
        }
        let roll = urand(pick, 1, 100);
        if roll <= 40 {
            Self::pick_line(chat_pools::TRADE_MESSAGES_GENERAL, pick)
        } else if roll <= 60 {
            Self::pick_line(chat_pools::TRADE_MESSAGES_ENCHANTER, pick)
        } else if roll <= 80 {
            Self::pick_line(chat_pools::TRADE_MESSAGES_ALCHEMIST, pick)
        } else {
            Self::pick_line(chat_pools::TRADE_MESSAGES_CRAFTER, pick)
        }
    }

    /// `PickWhisperResponse`.
    #[must_use]
    pub fn pick_whisper_response(message: &str, pick: &mut Pick<'_>) -> String {
        let lower = message.to_ascii_lowercase();
        for (keyword, responses) in chat_pools::WHISPER_PATTERNS {
            if lower.contains(keyword) {
                return Self::pick_line(responses, pick).unwrap_or_default();
            }
        }
        Self::pick_line(chat_pools::WHISPER_FALLBACK, pick).unwrap_or_default()
    }

    /// Queues a delayed whisper (trade-chat
    /// proactive responses, 5-20s in the fork).
    pub fn queue_whisper(
        &mut self,
        bot: ObjectGuid,
        target: ObjectGuid,
        text: String,
        delay_ms: u64,
        now_ms: u64,
    ) {
        self.pending_whispers.push(PendingWhisper {
            bot,
            target,
            text,
            due_ms: now_ms.saturating_add(delay_ms),
        });
    }

    fn flush_due_whispers(&mut self, now_ms: u64) -> Vec<ChatAction> {
        let mut actions = Vec::new();
        let mut i = 0;
        while i < self.pending_whispers.len() {
            if self.pending_whispers[i].due_ms <= now_ms {
                let pw = self.pending_whispers.remove(i);
                actions.push(ChatAction::Whisper {
                    bot: pw.bot,
                    target: pw.target,
                    text: pw.text,
                });
            } else {
                i += 1;
            }
        }
        actions
    }

    /// `Update` (absolute deadlines; `now_ms` is
    /// a monotonic millisecond clock from the host).
    pub fn update(
        &mut self,
        now_ms: u64,
        bots: &[BotChatInfo],
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        if !self.config.enabled {
            return Vec::new();
        }
        if !self.started {
            self.started = true;
            self.trade_deadline_ms = now_ms.saturating_add(
                u64::from(urand(
                    pick,
                    self.config.trade_interval_min_s,
                    self.config.trade_interval_max_s,
                )) * 1000,
            );
            self.guild_deadline_ms = now_ms.saturating_add(
                u64::from(urand(
                    pick,
                    self.config.guild_interval_min_s,
                    self.config.guild_interval_max_s,
                )) * 1000,
            );
            self.cleanup_deadline_ms = now_ms.saturating_add(60_000);
        }
        let now_secs = now_ms / 1000;
        let mut actions = self.flush_due_whispers(now_ms);

        // Trade chat timer.
        if now_ms >= self.trade_deadline_ms {
            self.trade_deadline_ms = now_ms.saturating_add(
                u64::from(urand(
                    pick,
                    self.config.trade_interval_min_s,
                    self.config.trade_interval_max_s,
                )) * 1000,
            );
            let city_bots: Vec<&BotChatInfo> = bots
                .iter()
                .filter(|b| b.in_city && b.is_alive)
                .filter(|b| self.can_chat(b.guid.counter(), CD_TRADE, now_secs, bots.len()))
                .collect();
            if !city_bots.is_empty() {
                let idx = usize::try_from(urand(
                    pick,
                    0,
                    u32::try_from(city_bots.len())
                        .unwrap_or(1)
                        .saturating_sub(1),
                ))
                .unwrap_or(0);
                let bot = city_bots[idx.min(city_bots.len() - 1)];
                if let Some(text) = Self::pick_trade_message(bot, pick) {
                    self.record(bot.guid.counter(), CD_TRADE, now_secs);
                    actions.push(ChatAction::TradeChannel {
                        bot: bot.guid,
                        text,
                    });
                }
            }
        }

        // Guild idle chat timer (random guild with
        // an online bot speaks).
        if now_ms >= self.guild_deadline_ms {
            self.guild_deadline_ms = now_ms.saturating_add(
                u64::from(urand(
                    pick,
                    self.config.guild_interval_min_s,
                    self.config.guild_interval_max_s,
                )) * 1000,
            );
            let guild_bots: Vec<&BotChatInfo> = bots
                .iter()
                .filter(|b| b.guild_id != 0 && b.is_alive)
                .filter(|b| self.can_chat(b.guid.counter(), CD_GUILD, now_secs, bots.len()))
                .collect();
            if !guild_bots.is_empty() {
                let idx = usize::try_from(urand(
                    pick,
                    0,
                    u32::try_from(guild_bots.len())
                        .unwrap_or(1)
                        .saturating_sub(1),
                ))
                .unwrap_or(0);
                let bot = *guild_bots[idx.min(guild_bots.len() - 1)];
                let personality = self.personality(bot.guid, pick);
                if let Some(text) =
                    Self::pick_personality_line(chat_pools::GUILD_CHAT_IDLE, personality, pick)
                {
                    self.record(bot.guid.counter(), CD_GUILD, now_secs);
                    actions.push(ChatAction::Say {
                        bot: bot.guid,
                        kind: BotChatKind::Guild,
                        text: text.to_owned(),
                    });
                }
            }
        }

        // Cooldown cleanup.
        if now_ms >= self.cleanup_deadline_ms {
            self.cleanup_deadline_ms = now_ms.saturating_add(60_000);
            self.cooldowns.retain(|_, until| *until > now_secs);
        }

        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn party_react(
        &mut self,
        bots: &[BotChatInfo],
        pool: &[chat_pools::PersonalityLine],
        emote: Option<u32>,
        min_count: u32,
        max_count: u32,
        now_s: u64,
        skip_gates: bool,
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        let mut candidates: Vec<BotChatInfo> = bots
            .iter()
            .copied()
            .filter(|b| skip_gates || b.is_alive)
            .filter(|b| skip_gates || self.can_chat(b.guid.counter(), CD_PARTY, now_s, bots.len()))
            .collect();
        let count =
            urand(pick, min_count, max_count).min(u32::try_from(candidates.len()).unwrap_or(0));
        let mut actions = Vec::new();
        for _ in 0..count {
            if candidates.is_empty() {
                break;
            }
            let idx = usize::try_from(urand(
                pick,
                0,
                u32::try_from(candidates.len())
                    .unwrap_or(1)
                    .saturating_sub(1),
            ))
            .unwrap_or(0)
            .min(candidates.len() - 1);
            let bot = candidates.remove(idx);
            let personality = self.personality(bot.guid, pick);
            if personality == BotPersonality::Quiet && urand(pick, 1, 100) > 30 {
                continue;
            }
            if let Some(text) = Self::pick_personality_line(pool, personality, pick) {
                if !skip_gates {
                    self.record(bot.guid.counter(), CD_PARTY, now_s);
                }
                actions.push(ChatAction::Say {
                    bot: bot.guid,
                    kind: BotChatKind::Party,
                    text: text.to_owned(),
                });
                if let Some(emote_id) = emote {
                    actions.push(ChatAction::Emote {
                        bot: bot.guid,
                        emote_id,
                    });
                }
            }
        }
        actions
    }

    /// Direct party line under the party cooldown
    /// (`SendPartyChat` + `CanChat`/`RecordChat`).
    pub fn party_announce(
        &mut self,
        bot: ObjectGuid,
        text: &str,
        now_s: u64,
        total_bots: usize,
    ) -> Option<ChatAction> {
        if !self.config.enabled {
            return None;
        }
        if !self.can_chat(bot.counter(), CD_PARTY, now_s, total_bots) {
            return None;
        }
        self.record(bot.counter(), CD_PARTY, now_s);
        Some(ChatAction::Say {
            bot,
            kind: BotChatKind::Party,
            text: text.to_owned(),
        })
    }

    /// `OnGroupMemberJoined` (the joining bot greets).
    pub fn on_bot_joined_group(
        &mut self,
        bot: BotChatInfo,
        now_s: u64,
        total_bots: usize,
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        if !self.config.enabled {
            return Vec::new();
        }
        let personality = self.personality(bot.guid, pick);
        if personality == BotPersonality::Quiet && urand(pick, 1, 100) > 30 {
            return Vec::new();
        }
        if !self.can_chat(bot.guid.counter(), CD_PARTY, now_s, total_bots) {
            return Vec::new();
        }
        let Some(text) = Self::pick_personality_line(chat_pools::PARTY_GREET, personality, pick)
        else {
            return Vec::new();
        };
        self.record(bot.guid.counter(), CD_PARTY, now_s);
        vec![
            ChatAction::Say {
                bot: bot.guid,
                kind: BotChatKind::Party,
                text: text.to_owned(),
            },
            ChatAction::Emote {
                bot: bot.guid,
                emote_id: 3, // EMOTE_ONESHOT_WAVE
            },
        ]
    }

    /// `OnGroupMemberLevelUp`.
    pub fn on_member_level_up(
        &mut self,
        bots: &[BotChatInfo],
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        if !self.config.enabled {
            return Vec::new();
        }
        self.party_react(
            bots,
            chat_pools::PARTY_GRATZ,
            None,
            1,
            2,
            now_s,
            false,
            pick,
        )
    }

    /// `OnBossKilled`.
    pub fn on_boss_killed(
        &mut self,
        bots: &[BotChatInfo],
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        if !self.config.enabled {
            return Vec::new();
        }
        self.party_react(
            bots,
            chat_pools::PARTY_BOSS_KILL,
            Some(4), // EMOTE_ONESHOT_CHEER
            1,
            3,
            now_s,
            false,
            pick,
        )
    }

    /// `OnPartyWipe` (fork skips `CanChat`, records
    /// no cooldown, and reacts even while dead).
    pub fn on_party_wipe(
        &mut self,
        bots: &[BotChatInfo],
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Vec<ChatAction> {
        if !self.config.enabled {
            return Vec::new();
        }
        self.party_react(
            bots,
            chat_pools::PARTY_WIPE,
            Some(18), // EMOTE_ONESHOT_CRY
            1,
            2,
            now_s,
            true,
            pick,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(guid: u64) -> BotChatInfo {
        BotChatInfo {
            guid: ObjectGuid::from_raw(guid),
            class_id: 8,
            level: 60,
            is_alive: true,
            in_city: true,
            is_alliance: true,
            guild_id: 7,
            party: Some(1),
        }
    }

    #[test]
    fn timers_fire_on_deadlines() {
        let mut core = ChatCore::new(ChatConfig {
            trade_interval_min_s: 1,
            trade_interval_max_s: 1,
            guild_interval_min_s: 1,
            guild_interval_max_s: 1,
            ..ChatConfig::default()
        });
        let mut pick = |_lo: u32, _hi: u32| 0u32;
        let bots = [info(5)];
        assert!(core.update(0, &bots, &mut pick).is_empty());
        let actions = core.update(1_500, &bots, &mut pick);
        assert!(!actions.is_empty(), "trade/guild deadline should fire");
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, ChatAction::TradeChannel { .. }))
        );
    }
}
