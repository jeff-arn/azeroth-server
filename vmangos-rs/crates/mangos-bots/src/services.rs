//! Whisper-driven bot services (`BotServiceMgr`
//! core): portal/enchant/transmute/mooncloth
//! request state machine, trade lifecycle hooks,
//! and the WTB buy side (price oracle injected by
//! the host — AH fair value lands with the AH bot
//! in Slice F). Pure state; host applies actions.

use std::collections::HashMap;

use mangos_guid::ObjectGuid;

use crate::chat::ChatAction;
use crate::chat_pools;

/// `BotServiceType`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceType {
    None = 0,
    Portal = 1,
    Enchant = 2,
    Transmute = 3,
    Mooncloth = 4,
    Buy = 5,
}

/// `BotServiceState`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceState {
    None = 0,
    AskedDestination = 1,
    WaitingForTrade = 2,
    TradeInProgress = 3,
}

/// `BotServiceRequest`.
#[derive(Clone, Copy, Debug)]
pub struct ServiceRequest {
    pub player: ObjectGuid,
    pub bot: ObjectGuid,
    pub service: ServiceType,
    pub state: ServiceState,
    pub spell_id: u32,
    pub created_s: u64,
    pub last_update_s: u64,
    pub buy_item_id: u32,
    pub buy_max_quantity: u32,
    pub buy_price_copper: u32,
}

const PORTAL_TIP_MIN: u32 = 10_000;
const ENCHANT_TIP_MIN_UNUSED: u32 = 10_000;
const TRANSMUTE_TIP_MIN: u32 = 20_000;
const MOONCLOTH_TIP_MIN: u32 = 20_000;
const TIMEOUT_SECS: u64 = 300;
const SPELL_TRANSMUTE_ARCANITE: u32 = 17187;
const SPELL_MOONCLOTH: u32 = 18560;

struct EnchantDef {
    spell_id: u32,
    keyword: &'static str,
    tier: u8, // 0 common, 1 mid, 2 rare, 3 ultrarare
    rare: bool,
}

/// `ENCHANT_LIST`.
const ENCHANT_LIST: &[EnchantDef] = &[
    EnchantDef {
        spell_id: 13890,
        keyword: "minor speed",
        tier: 0,
        rare: false,
    },
    EnchantDef {
        spell_id: 13945,
        keyword: "greater stamina",
        tier: 0,
        rare: false,
    },
    EnchantDef {
        spell_id: 20025,
        keyword: "greater stats",
        tier: 0,
        rare: false,
    },
    EnchantDef {
        spell_id: 20012,
        keyword: "greater agility",
        tier: 1,
        rare: false,
    },
    EnchantDef {
        spell_id: 20011,
        keyword: "superior stamina",
        tier: 1,
        rare: false,
    },
    EnchantDef {
        spell_id: 20014,
        keyword: "greater resistance",
        tier: 1,
        rare: false,
    },
    EnchantDef {
        spell_id: 20017,
        keyword: "greater stamina shield",
        tier: 1,
        rare: false,
    },
    EnchantDef {
        spell_id: 20034,
        keyword: "crusader",
        tier: 2,
        rare: true,
    },
    EnchantDef {
        spell_id: 23800,
        keyword: "agility weapon",
        tier: 2,
        rare: true,
    },
    EnchantDef {
        spell_id: 22749,
        keyword: "spell power",
        tier: 3,
        rare: true,
    },
    EnchantDef {
        spell_id: 22750,
        keyword: "healing power",
        tier: 3,
        rare: true,
    },
    EnchantDef {
        spell_id: 23802,
        keyword: "healing bracer",
        tier: 3,
        rare: true,
    },
];

/// `PORTAL_DESTINATIONS` (keyword, alliance, horde).
const PORTAL_DESTINATIONS: &[(&str, u32, u32)] = &[
    ("stormwind", 10059, 0),
    ("sw", 10059, 0),
    ("storm", 10059, 0),
    ("ironforge", 11416, 0),
    ("if", 11416, 0),
    ("darnassus", 11419, 0),
    ("darn", 11419, 0),
    ("orgrimmar", 0, 11417),
    ("org", 0, 11417),
    ("undercity", 0, 11418),
    ("uc", 0, 11418),
    ("thunder", 0, 11420),
    ("tb", 0, 11420),
];

struct WtbItem {
    item_id: u32,
    keyword: &'static str,
    display: &'static str,
    max_quantity: u32,
}

/// `s_wtbItems`.
const WTB_ITEMS: &[WtbItem] = &[
    WtbItem {
        item_id: 13463,
        keyword: "lotus",
        display: "[Black Lotus]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 12363,
        keyword: "arcane crystal",
        display: "[Arcane Crystal]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 12811,
        keyword: "righteous orb",
        display: "[Righteous Orb]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 14344,
        keyword: "large brilliant",
        display: "[Large Brilliant Shard]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 4625,
        keyword: "firebloom",
        display: "[Firebloom]",
        max_quantity: 20,
    },
    WtbItem {
        item_id: 13467,
        keyword: "icecap",
        display: "[Icecap]",
        max_quantity: 20,
    },
    WtbItem {
        item_id: 8838,
        keyword: "sungrass",
        display: "[Sungrass]",
        max_quantity: 20,
    },
    WtbItem {
        item_id: 13468,
        keyword: "black lotus",
        display: "[Black Lotus]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 7076,
        keyword: "essence of earth",
        display: "[Essence of Earth]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 7078,
        keyword: "essence of fire",
        display: "[Essence of Fire]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 7080,
        keyword: "essence of water",
        display: "[Essence of Water]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 7082,
        keyword: "essence of air",
        display: "[Essence of Air]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 12359,
        keyword: "thorium bar",
        display: "[Thorium Bar]",
        max_quantity: 20,
    },
    WtbItem {
        item_id: 12655,
        keyword: "mooncloth",
        display: "[Mooncloth]",
        max_quantity: 5,
    },
    WtbItem {
        item_id: 12360,
        keyword: "arcanite bar",
        display: "[Arcanite Bar]",
        max_quantity: 3,
    },
];

struct BuyBudget {
    remaining_copper: u32,
    last_reset_s: u64,
}

/// Bot facts the service core needs.
#[derive(Clone, Copy, Debug)]
pub struct ServiceBotInfo {
    pub guid: ObjectGuid,
    pub class_id: u8,
    pub is_alliance: bool,
}

/// `BotServiceMgr` core state.
pub struct ServiceCore {
    pub enabled: bool,
    pub buy_budget_per_day: u32,
    requests: HashMap<u64, ServiceRequest>,
    buy_budgets: HashMap<u32, BuyBudget>,
    cleanup_deadline_ms: u64,
}

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn pick_line(pool: &[&str], pick: &mut Pick<'_>) -> String {
    if pool.is_empty() {
        return String::new();
    }
    let idx = usize::try_from(pick(
        0,
        u32::try_from(pool.len()).unwrap_or(1).saturating_sub(1),
    ))
    .unwrap_or(0);
    pool[idx.min(pool.len() - 1)].to_owned()
}

fn key(bot: ObjectGuid, player: ObjectGuid) -> u64 {
    (u64::from(bot.counter()) << 32) | u64::from(player.counter())
}

fn whisper(bot: ObjectGuid, target: ObjectGuid, text: String) -> ChatAction {
    ChatAction::Whisper { bot, target, text }
}

impl ServiceCore {
    #[must_use]
    pub fn new(enabled: bool, buy_budget_gold_per_day: u32) -> Self {
        Self {
            enabled,
            buy_budget_per_day: buy_budget_gold_per_day.saturating_mul(10_000),
            requests: HashMap::new(),
            buy_budgets: HashMap::new(),
            cleanup_deadline_ms: 0,
        }
    }

    /// `IsMageBot` etc. (deterministic class map).
    #[must_use]
    pub fn is_enchanter(class_id: u8) -> bool {
        matches!(class_id, 8 | 5 | 9)
    }
    #[must_use]
    pub fn is_alchemist(class_id: u8) -> bool {
        matches!(class_id, 8 | 5 | 9 | 11)
    }
    #[must_use]
    pub fn is_tailor(class_id: u8) -> bool {
        matches!(class_id, 8 | 5 | 9)
    }

    fn resolve_portal(bot: &ServiceBotInfo, lower: &str) -> u32 {
        for (keyword, alliance, horde) in PORTAL_DESTINATIONS {
            if lower.contains(keyword) {
                return if bot.is_alliance { *alliance } else { *horde };
            }
        }
        0
    }

    fn resolve_enchant(lower: &str) -> Option<&'static EnchantDef> {
        ENCHANT_LIST.iter().find(|e| lower.contains(e.keyword))
    }

    fn bot_knows_enchant(bot_low: u32, enchant: &EnchantDef) -> bool {
        if !enchant.rare {
            return true;
        }
        let seed = bot_low.wrapping_add(enchant.spell_id);
        match enchant.tier {
            2 => seed % 100 < 90,
            3 => seed % 100 < 70,
            _ => true,
        }
    }

    fn enchant_tip_min(tier: u8) -> u32 {
        match tier {
            1 => 20_000,
            2 => 50_000,
            3 => 100_000,
            _ => 0,
        }
    }

    fn new_request(
        bot: ObjectGuid,
        player: ObjectGuid,
        service: ServiceType,
        state: ServiceState,
        spell_id: u32,
        now_s: u64,
    ) -> ServiceRequest {
        ServiceRequest {
            player,
            bot,
            service,
            state,
            spell_id,
            created_s: now_s,
            last_update_s: now_s,
            buy_item_id: 0,
            buy_max_quantity: 0,
            buy_price_copper: 0,
        }
    }

    /// `HandleServiceWhisper` (sell side). Returns
    /// `Some(actions)` when handled.
    pub fn handle_service_whisper(
        &mut self,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        message: &str,
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Option<Vec<ChatAction>> {
        if !self.enabled {
            return None;
        }
        let lower = message.to_ascii_lowercase();
        let k = key(bot.guid, player);
        if let Some(actions) = self.continue_request(k, bot, player, &lower, now_s, pick) {
            return Some(actions);
        }
        self.start_request(k, bot, player, &lower, now_s, pick)
    }

    /// Existing-flow continuation of
    /// `HandleServiceWhisper`.
    fn continue_request(
        &mut self,
        k: u64,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        lower: &str,
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Option<Vec<ChatAction>> {
        if let Some(req) = self.requests.get(&k).copied() {
            if req.state == ServiceState::AskedDestination {
                match req.service {
                    ServiceType::Portal => {
                        let spell_id = Self::resolve_portal(&bot, lower);
                        if spell_id != 0 {
                            if let Some(stored) = self.requests.get_mut(&k) {
                                stored.spell_id = spell_id;
                                stored.state = ServiceState::WaitingForTrade;
                                stored.last_update_s = now_s;
                            }
                            return Some(vec![whisper(
                                bot.guid,
                                player,
                                pick_line(chat_pools::PORTAL_CONFIRM, pick),
                            )]);
                        }
                        return Some(vec![whisper(
                            bot.guid,
                            player,
                            chat_pools::PORTAL_WRONG_FACTION[usize::from(!bot.is_alliance) * 2]
                                .to_owned(),
                        )]);
                    }
                    ServiceType::Enchant => {
                        if let Some(enchant) = Self::resolve_enchant(lower) {
                            if Self::bot_knows_enchant(bot.guid.counter(), enchant) {
                                if let Some(stored) = self.requests.get_mut(&k) {
                                    stored.spell_id = enchant.spell_id;
                                    stored.state = ServiceState::WaitingForTrade;
                                    stored.last_update_s = now_s;
                                }
                                return Some(vec![whisper(
                                    bot.guid,
                                    player,
                                    pick_line(chat_pools::ENCHANT_CONFIRM, pick),
                                )]);
                            }
                            self.requests.remove(&k);
                            return Some(vec![whisper(
                                bot.guid,
                                player,
                                pick_line(chat_pools::ENCHANT_DONT_KNOW, pick),
                            )]);
                        }
                        return Some(vec![whisper(
                            bot.guid,
                            player,
                            "which enchant specifically?".to_owned(),
                        )]);
                    }
                    _ => {}
                }
            }
            if req.state == ServiceState::WaitingForTrade {
                return Some(vec![whisper(
                    bot.guid,
                    player,
                    pick_line(chat_pools::WAIT_FOR_TRADE, pick),
                )]);
            }
        }
        None
    }

    /// New-request arm of `HandleServiceWhisper`.
    fn start_request(
        &mut self,
        k: u64,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        lower: &str,
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Option<Vec<ChatAction>> {
        if bot.class_id == 8 && (lower.contains("portal") || lower.contains("port")) {
            let spell_id = Self::resolve_portal(&bot, lower);
            let (state, text) = if spell_id != 0 {
                (
                    ServiceState::WaitingForTrade,
                    pick_line(chat_pools::PORTAL_CONFIRM, pick),
                )
            } else {
                (
                    ServiceState::AskedDestination,
                    pick_line(chat_pools::PORTAL_ASK_DESTINATION, pick),
                )
            };
            self.requests.insert(
                k,
                Self::new_request(
                    bot.guid,
                    player,
                    ServiceType::Portal,
                    state,
                    spell_id,
                    now_s,
                ),
            );
            return Some(vec![whisper(bot.guid, player, text)]);
        }

        if Self::is_enchanter(bot.class_id)
            && (lower.contains("enchant") || lower.contains("chant"))
        {
            let enchant = Self::resolve_enchant(lower);
            match enchant {
                Some(e) if Self::bot_knows_enchant(bot.guid.counter(), e) => {
                    self.requests.insert(
                        k,
                        Self::new_request(
                            bot.guid,
                            player,
                            ServiceType::Enchant,
                            ServiceState::WaitingForTrade,
                            e.spell_id,
                            now_s,
                        ),
                    );
                    return Some(vec![whisper(
                        bot.guid,
                        player,
                        pick_line(chat_pools::ENCHANT_CONFIRM, pick),
                    )]);
                }
                Some(_) => {
                    return Some(vec![whisper(
                        bot.guid,
                        player,
                        pick_line(chat_pools::ENCHANT_DONT_KNOW, pick),
                    )]);
                }
                None => {
                    self.requests.insert(
                        k,
                        Self::new_request(
                            bot.guid,
                            player,
                            ServiceType::Enchant,
                            ServiceState::AskedDestination,
                            0,
                            now_s,
                        ),
                    );
                    return Some(vec![whisper(
                        bot.guid,
                        player,
                        pick_line(chat_pools::ENCHANT_ASK_WHICH, pick),
                    )]);
                }
            }
        }

        self.start_craft_request(k, bot, player, lower, now_s, pick)
    }

    /// Transmute/mooncloth/crusader arms.
    fn start_craft_request(
        &mut self,
        k: u64,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        lower: &str,
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> Option<Vec<ChatAction>> {
        if Self::is_alchemist(bot.class_id)
            && (lower.contains("transmute") || lower.contains("arcanite"))
        {
            self.requests.insert(
                k,
                Self::new_request(
                    bot.guid,
                    player,
                    ServiceType::Transmute,
                    ServiceState::WaitingForTrade,
                    SPELL_TRANSMUTE_ARCANITE,
                    now_s,
                ),
            );
            return Some(vec![whisper(
                bot.guid,
                player,
                pick_line(chat_pools::TRANSMUTE_CONFIRM, pick),
            )]);
        }

        if Self::is_tailor(bot.class_id) && lower.contains("mooncloth") {
            self.requests.insert(
                k,
                Self::new_request(
                    bot.guid,
                    player,
                    ServiceType::Mooncloth,
                    ServiceState::WaitingForTrade,
                    SPELL_MOONCLOTH,
                    now_s,
                ),
            );
            return Some(vec![whisper(
                bot.guid,
                player,
                pick_line(chat_pools::MOONCLOTH_CONFIRM, pick),
            )]);
        }

        if Self::is_enchanter(bot.class_id) && lower.contains("crusader") {
            if let Some(enchant) = Self::resolve_enchant(lower) {
                if Self::bot_knows_enchant(bot.guid.counter(), enchant) {
                    self.requests.insert(
                        k,
                        Self::new_request(
                            bot.guid,
                            player,
                            ServiceType::Enchant,
                            ServiceState::WaitingForTrade,
                            enchant.spell_id,
                            now_s,
                        ),
                    );
                    return Some(vec![whisper(
                        bot.guid,
                        player,
                        pick_line(chat_pools::ENCHANT_CONFIRM, pick),
                    )]);
                }
            }
            return Some(vec![whisper(
                bot.guid,
                player,
                pick_line(chat_pools::ENCHANT_DONT_KNOW, pick),
            )]);
        }

        None
    }

    /// `HandleBuyWhisper` (price oracle = AH fair
    /// buyout × 0.9 host-side; 0 = unknown).
    pub fn handle_buy_whisper(
        &mut self,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        message: &str,
        now_s: u64,
        fair_buyout: &dyn Fn(u32, u32) -> u32,
    ) -> Option<Vec<ChatAction>> {
        if !self.enabled {
            return None;
        }
        let lower = message.to_ascii_lowercase();
        let item = WTB_ITEMS.iter().find(|i| lower.contains(i.keyword))?;
        self.reset_budget_if_needed(bot.guid.counter(), now_s);
        let remaining = self
            .buy_budgets
            .get(&bot.guid.counter())
            .map_or(0, |b| b.remaining_copper);
        if remaining == 0 {
            return Some(vec![whisper(
                bot.guid,
                player,
                "sorry, I'm tapped out for today".to_owned(),
            )]);
        }
        let fair = fair_buyout(item.item_id, 1);
        if fair == 0 {
            return Some(vec![whisper(
                bot.guid,
                player,
                "hmm, not sure what those are worth right now".to_owned(),
            )]);
        }
        let price = (f64::from(fair) * 0.9) as u32;
        let mut req = Self::new_request(
            bot.guid,
            player,
            ServiceType::Buy,
            ServiceState::WaitingForTrade,
            0,
            now_s,
        );
        req.buy_item_id = item.item_id;
        req.buy_max_quantity = item.max_quantity;
        req.buy_price_copper = price;
        self.requests.insert(key(bot.guid, player), req);
        let gold = price / 10_000;
        let silver = (price % 10_000) / 100;
        let text = if silver > 0 {
            format!(
                "I'll buy {} for {gold}g {silver}s each, trade me",
                item.display
            )
        } else {
            format!("I'll buy {} for {gold}g each, trade me", item.display)
        };
        Some(vec![whisper(bot.guid, player, text)])
    }

    fn reset_budget_if_needed(&mut self, bot_low: u32, now_s: u64) {
        let budget = self.buy_budgets.entry(bot_low).or_insert(BuyBudget {
            remaining_copper: self.buy_budget_per_day,
            last_reset_s: now_s,
        });
        if now_s.saturating_sub(budget.last_reset_s) >= 86_400 {
            budget.remaining_copper = self.buy_budget_per_day;
            budget.last_reset_s = now_s;
        }
    }

    /// `CreateProactiveRequest` (trade-chat triggers).
    pub fn create_proactive_request(
        &mut self,
        bot: ServiceBotInfo,
        player: ObjectGuid,
        service: ServiceType,
        trade_msg: &str,
        now_s: u64,
    ) {
        let lower = trade_msg.to_ascii_lowercase();
        let mut spell_id = 0;
        let mut state = match service {
            ServiceType::Portal | ServiceType::Enchant => ServiceState::AskedDestination,
            _ => ServiceState::WaitingForTrade,
        };
        if service == ServiceType::Portal && !lower.is_empty() {
            let resolved = Self::resolve_portal(&bot, &lower);
            if resolved != 0 {
                spell_id = resolved;
                state = ServiceState::WaitingForTrade;
            }
        }
        if service == ServiceType::Enchant && !lower.is_empty() {
            if let Some(enchant) = Self::resolve_enchant(&lower) {
                if Self::bot_knows_enchant(bot.guid.counter(), enchant) {
                    spell_id = enchant.spell_id;
                    state = ServiceState::WaitingForTrade;
                }
            }
        }
        self.requests.insert(
            key(bot.guid, player),
            Self::new_request(bot.guid, player, service, state, spell_id, now_s),
        );
    }

    /// `HasActiveRequest`.
    #[must_use]
    pub fn has_active_request(&self, player: ObjectGuid) -> bool {
        self.requests
            .values()
            .any(|r| r.player.counter() == player.counter())
    }

    /// `GetPendingRequest`.
    #[must_use]
    pub fn pending_request(&self, bot: ObjectGuid, player: ObjectGuid) -> Option<ServiceRequest> {
        self.requests.get(&key(bot, player)).copied()
    }

    /// `ShouldAcceptTrade` (+ state advance).
    pub fn should_accept_trade(&mut self, bot: ObjectGuid, player: ObjectGuid, now_s: u64) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(req) = self.requests.get_mut(&key(bot, player)) else {
            return false;
        };
        if req.state != ServiceState::WaitingForTrade {
            return false;
        }
        req.state = ServiceState::TradeInProgress;
        req.last_update_s = now_s;
        true
    }

    /// `OnTradeCompleted`. Returns (actions,
    /// delivered spell to cast by the host, if any).
    pub fn on_trade_completed(
        &mut self,
        bot: ObjectGuid,
        player: ObjectGuid,
        gold_received: u32,
        now_s: u64,
        pick: &mut Pick<'_>,
    ) -> (Vec<ChatAction>, Option<ServiceRequest>) {
        if !self.enabled {
            return (Vec::new(), None);
        }
        let k = key(bot, player);
        let Some(req) = self.requests.get(&k).copied() else {
            return (Vec::new(), None);
        };
        if req.service == ServiceType::Buy {
            self.reset_budget_if_needed(bot.counter(), now_s);
            if let Some(budget) = self.buy_budgets.get_mut(&bot.counter()) {
                budget.remaining_copper = budget.remaining_copper.saturating_sub(gold_received);
            }
            self.requests.remove(&k);
            return (
                vec![whisper(
                    bot,
                    player,
                    "thanks, pleasure doing business".to_owned(),
                )],
                Some(req),
            );
        }
        let min_tip = match req.service {
            ServiceType::Portal => PORTAL_TIP_MIN,
            // Fork uses the COMMON tier floor (0)
            // regardless of the resolved enchant.
            ServiceType::Enchant => Self::enchant_tip_min(0),
            ServiceType::Transmute => TRANSMUTE_TIP_MIN,
            ServiceType::Mooncloth => MOONCLOTH_TIP_MIN,
            _ => 0,
        };
        let _ = ENCHANT_TIP_MIN_UNUSED;
        if gold_received < min_tip {
            if let Some(stored) = self.requests.get_mut(&k) {
                stored.state = ServiceState::WaitingForTrade;
                stored.last_update_s = now_s;
            }
            return (
                vec![whisper(
                    bot,
                    player,
                    pick_line(chat_pools::SERVICE_LOW_TIP, pick),
                )],
                None,
            );
        }
        self.requests.remove(&k);
        (
            vec![whisper(
                bot,
                player,
                pick_line(chat_pools::SERVICE_THANKS, pick),
            )],
            Some(req),
        )
    }

    /// `Update` cleanup cadence (absolute deadline).
    pub fn update(&mut self, now_ms: u64, now_unix_s: u64) {
        if !self.enabled {
            return;
        }
        if self.cleanup_deadline_ms == 0 {
            self.cleanup_deadline_ms = now_ms.saturating_add(30_000);
            return;
        }
        if now_ms >= self.cleanup_deadline_ms {
            self.cleanup_deadline_ms = now_ms.saturating_add(30_000);
            self.requests
                .retain(|_, r| now_unix_s.saturating_sub(r.last_update_s) <= TIMEOUT_SECS);
        }
    }

    /// `ClearRequestsForPlayer`.
    pub fn clear_requests_for_player(&mut self, player: ObjectGuid) {
        self.requests
            .retain(|_, r| r.player.counter() != player.counter());
    }
}
