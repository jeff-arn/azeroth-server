//! Auction house bot (`AuctionHouseBotMgr` core):
//! item pools by category, weighted population
//! planning with seller-type pricing, fair market
//! value, and the bot-buyer scan. Pure state; the
//! host creates/removes real auctions and items.
//!
//! Float rolls ride the integer `Pick` at 1e4-1e6
//! resolution: distributions match the fork, roll
//! sequences do not (different RNG streams).

/// `AHBotCategory`.
pub const CAT_TRADE_GOODS: usize = 0;
pub const CAT_COUNT: usize = 6;

/// `AHBotCategory` names (`auctionhousebot.category`).
pub const CATEGORY_NAMES: [&str; CAT_COUNT] = [
    "trade_goods",
    "crafting",
    "consumables",
    "recipes",
    "equipment",
    "leveling",
];

/// Seller-name pool for generated identities.
pub const SELLER_NAMES: [&str; 30] = [
    "Grimjaw",
    "Thornweave",
    "Silkfingers",
    "Copperbell",
    "Ironhide",
    "Dustwalker",
    "Ashgrip",
    "Stonebrew",
    "Goldleaf",
    "Moonpetal",
    "Flamestitch",
    "Frostpick",
    "Brighthammer",
    "Shadowloom",
    "Steelclaw",
    "Embershank",
    "Mistweaver",
    "Oakbarrel",
    "Windsorrow",
    "Duskmantle",
    "Boulderback",
    "Sunthread",
    "Ravenforge",
    "Deepmine",
    "Wildroot",
    "Silversting",
    "Coalhand",
    "Mossgrip",
    "Starforge",
    "Rustblade",
];

/// `AHBotSellerType`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SellerType {
    Casual = 0,
    Dedicated = 1,
    Crafter = 2,
    Opportunist = 3,
}

/// `AuctionHouseBotConfig`.
#[derive(Clone, Debug)]
pub struct AhBotConfig {
    pub enable: bool,
    pub ah_fid: u32,
    pub item_count: u32,
    pub refresh_interval_secs: u64,
    pub seller_account_id: u32,
    pub seller_count: u32,
    pub quota_min: [u32; CAT_COUNT],
    pub quota_max: [u32; CAT_COUNT],
    pub buyer_interval_secs: u64,
    pub buyer_budget_per_cycle: u32,
}

impl Default for AhBotConfig {
    fn default() -> Self {
        Self {
            enable: false,
            ah_fid: 120,
            item_count: 200,
            refresh_interval_secs: 259_200,
            seller_account_id: 99,
            seller_count: 12,
            quota_min: [35, 15, 20, 3, 5, 10],
            quota_max: [40, 20, 25, 5, 10, 15],
            buyer_interval_secs: 1800,
            buyer_budget_per_cycle: 500_000,
        }
    }
}

/// `AuctionHouseBotEntry` (one `auctionhousebot` row).
#[derive(Clone, Copy, Debug)]
pub struct AhBotEntry {
    pub item: u32,
    pub min_stack: u32,
    pub max_stack: u32,
    pub bid: u32,
    pub buyout: u32,
    pub weight: f32,
    pub category: u8,
    pub price_variance_pct: u32,
}

/// `AHBotSellerIdentity`.
#[derive(Clone, Debug)]
pub struct SellerIdentity {
    pub character_guid: u32,
    pub name: String,
    pub seller_type: SellerType,
}

/// One planned bot listing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewListing {
    pub item_id: u32,
    pub stack_size: u32,
    pub bid: u32,
    pub buyout: u32,
    pub seller_guid: u32,
}

/// One player auction for the buyer scan.
#[derive(Clone, Copy, Debug)]
pub struct BuyerCandidate {
    pub auction_id: u32,
    pub item_id: u32,
    pub stack_size: u32,
    pub buyout: u32,
    pub owner_guid: u32,
}

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn urand(pick: &mut Pick<'_>, lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + pick(0, hi - lo) }
}

fn frand(pick: &mut Pick<'_>, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return lo;
    }
    #[allow(clippy::cast_precision_loss)]
    let t = pick(0, 1_000_000) as f32 / 1_000_000.0;
    lo + t * (hi - lo)
}

/// `ParseCategory`.
#[must_use]
pub fn parse_category(text: &str) -> u8 {
    for (idx, name) in CATEGORY_NAMES.iter().enumerate() {
        if text == *name {
            return u8::try_from(idx).unwrap_or(0);
        }
    }
    0
}

/// AH bot core state.
pub struct AhBotCore {
    pub config: AhBotConfig,
    pub loaded: bool,
    items: Vec<AhBotEntry>,
    sellers: Vec<SellerIdentity>,
    category_items: [Vec<usize>; CAT_COUNT],
    category_weights: [Vec<f32>; CAT_COUNT],
    category_total: [f32; CAT_COUNT],
    last_refresh_s: i64,
    last_buyer_s: i64,
}

impl AhBotCore {
    /// `Load` (rows + sellers supplied by the host;
    /// `house_found` mirrors the missing-house early
    /// return that leaves the manager unloaded).
    #[must_use]
    pub fn load(
        config: AhBotConfig,
        items: Vec<AhBotEntry>,
        mut sellers: Vec<SellerIdentity>,
        last_refresh_s: i64,
        house_found: bool,
    ) -> Self {
        // Seller-type distribution: 40/25/20/15.
        let total = sellers.len().max(1);
        for (idx, seller) in sellers.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let pct = idx as f32 / total as f32;
            seller.seller_type = if pct < 0.40 {
                SellerType::Casual
            } else if pct < 0.65 {
                SellerType::Dedicated
            } else if pct < 0.85 {
                SellerType::Crafter
            } else {
                SellerType::Opportunist
            };
        }
        let mut core = Self {
            config,
            loaded: house_found,
            items,
            sellers,
            category_items: Default::default(),
            category_weights: Default::default(),
            category_total: [0.0; CAT_COUNT],
            last_refresh_s,
            last_buyer_s: 0,
        };
        core.build_pools();
        core
    }

    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn seller_count(&self) -> usize {
        self.sellers.len()
    }

    /// Per-category (count, total weight) for boot logs.
    #[must_use]
    pub fn category_stats(&self) -> Vec<(&'static str, usize, f32)> {
        (0..CAT_COUNT)
            .map(|cat| {
                (
                    CATEGORY_NAMES[cat],
                    self.category_items[cat].len(),
                    self.category_total[cat],
                )
            })
            .collect()
    }

    fn build_pools(&mut self) {
        for cat in 0..CAT_COUNT {
            self.category_items[cat].clear();
            self.category_weights[cat].clear();
            self.category_total[cat] = 0.0;
        }
        for (idx, entry) in self.items.iter().enumerate() {
            let cat = usize::from(entry.category).min(CAT_COUNT - 1);
            let cat = if usize::from(entry.category) >= CAT_COUNT {
                CAT_TRADE_GOODS
            } else {
                cat
            };
            self.category_items[cat].push(idx);
            self.category_total[cat] += entry.weight;
            self.category_weights[cat].push(self.category_total[cat]);
        }
    }

    /// `GetFairBuyoutForItem`.
    #[must_use]
    pub fn fair_buyout(&self, item_id: u32, stack_size: u32) -> u32 {
        for entry in &self.items {
            if entry.item == item_id {
                let per_unit = entry.buyout / entry.min_stack.max(1);
                return per_unit.saturating_mul(stack_size);
            }
        }
        0
    }

    /// `IsBotOwned`.
    #[must_use]
    pub fn is_bot_owned(&self, owner_guid: u32) -> bool {
        self.sellers.iter().any(|s| s.character_guid == owner_guid)
    }

    /// `GetPurchaseProbability` (percent).
    #[must_use]
    pub fn purchase_probability(listing_buyout: u32, fair_buyout: u32) -> f32 {
        if fair_buyout == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = listing_buyout as f32 / fair_buyout as f32;
        if ratio <= 0.8 {
            95.0
        } else if ratio <= 1.0 {
            80.0
        } else if ratio <= 1.2 {
            50.0
        } else if ratio <= 1.5 {
            20.0
        } else if ratio <= 2.0 {
            5.0
        } else if ratio <= 5.0 {
            1.0
        } else {
            0.0
        }
    }

    /// Whether `Update` wants a full refresh now.
    #[must_use]
    pub fn needs_refresh(&self, now_s: i64, force: bool) -> bool {
        self.loaded
            && (self.config.enable || force)
            && (force
                || self.last_refresh_s == 0
                || u64::try_from(now_s - self.last_refresh_s).unwrap_or(0)
                    >= self.config.refresh_interval_secs)
    }

    /// Whether the 80% restock arm applies.
    #[must_use]
    pub fn needs_restock(&self, current_count: u32) -> bool {
        self.loaded && current_count < self.config.item_count * 80 / 100
    }

    pub fn mark_refreshed(&mut self, now_s: i64) {
        self.last_refresh_s = now_s;
    }

    /// Whether the buyer scan is due.
    #[must_use]
    pub fn buyer_due(&self, now_s: i64) -> bool {
        self.loaded
            && u64::try_from(now_s - self.last_buyer_s).unwrap_or(u64::MAX)
                >= self.config.buyer_interval_secs
    }

    pub fn mark_buyer_run(&mut self, now_s: i64) {
        self.last_buyer_s = now_s;
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.loaded && !self.items.is_empty() && !self.sellers.is_empty()
    }

    fn pick_weighted_item(&self, cat: usize, pick: &mut Pick<'_>) -> usize {
        if self.category_items[cat].is_empty() {
            return 0;
        }
        let roll = frand(pick, 0.0, self.category_total[cat]);
        let weights = &self.category_weights[cat];
        let idx = weights
            .partition_point(|w| *w < roll)
            .min(weights.len().saturating_sub(1));
        self.category_items[cat][idx]
    }

    fn roll_seller_type(pick: &mut Pick<'_>) -> SellerType {
        match urand(pick, 1, 100) {
            0..=40 => SellerType::Casual,
            41..=65 => SellerType::Dedicated,
            66..=85 => SellerType::Crafter,
            _ => SellerType::Opportunist,
        }
    }

    fn pick_seller(&self, seller_type: SellerType, pick: &mut Pick<'_>) -> &SellerIdentity {
        let matching: Vec<usize> = self
            .sellers
            .iter()
            .enumerate()
            .filter(|(_, s)| s.seller_type == seller_type)
            .map(|(idx, _)| idx)
            .collect();
        if matching.is_empty() {
            let idx = usize::try_from(urand(
                pick,
                0,
                u32::try_from(self.sellers.len())
                    .unwrap_or(1)
                    .saturating_sub(1),
            ))
            .unwrap_or(0);
            &self.sellers[idx.min(self.sellers.len() - 1)]
        } else {
            let idx = usize::try_from(urand(
                pick,
                0,
                u32::try_from(matching.len()).unwrap_or(1).saturating_sub(1),
            ))
            .unwrap_or(0);
            &self.sellers[matching[idx.min(matching.len() - 1)]]
        }
    }

    fn roll_stack_size(entry: &AhBotEntry, seller_type: SellerType, pick: &mut Pick<'_>) -> u32 {
        if entry.min_stack == entry.max_stack {
            return entry.min_stack;
        }
        match seller_type {
            SellerType::Casual => urand(
                pick,
                entry.min_stack,
                entry.max_stack.min(entry.min_stack + 2),
            ),
            SellerType::Dedicated => {
                let min = if entry.max_stack > 2 {
                    entry.max_stack - 2
                } else {
                    entry.min_stack
                };
                urand(pick, min, entry.max_stack)
            }
            _ => urand(pick, entry.min_stack, entry.max_stack),
        }
    }

    fn apply_price_jitter(
        entry: &AhBotEntry,
        stack_size: u32,
        seller_type: SellerType,
        pick: &mut Pick<'_>,
    ) -> (u32, u32) {
        let seller_mod = match seller_type {
            SellerType::Casual => 0.05,
            SellerType::Dedicated => -0.07,
            SellerType::Crafter => 0.15,
            SellerType::Opportunist => 0.25,
        };
        #[allow(clippy::cast_precision_loss)]
        let variance = entry.price_variance_pct as f32;
        let jitter = frand(pick, -variance, variance) / 100.0;
        let mut price_mod = 1.0 + seller_mod + jitter;
        if price_mod < 0.5 {
            price_mod = 0.5;
        }
        #[allow(clippy::cast_precision_loss)]
        let stack_ratio = stack_size as f32 / entry.min_stack.max(1) as f32;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let buyout = ((entry.buyout as f32 * stack_ratio * price_mod).round() as u32).max(1);
        let bid_pct = frand(pick, 0.70, 0.90);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let bid = ((buyout as f32 * bid_pct).round() as u32).max(1);
        (bid, buyout)
    }

    /// `PopulateAuctions` planning (host creates the
    /// auctions; unknown item templates are the
    /// host's `AddItem` early return).
    #[allow(clippy::needless_range_loop)]
    pub fn plan_populate(&self, current_count: u32, pick: &mut Pick<'_>) -> Vec<NewListing> {
        if current_count >= self.config.item_count || !self.ready() {
            return Vec::new();
        }
        let remaining = self.config.item_count - current_count;
        let mut targets = [0u32; CAT_COUNT];
        let mut total_quota = 0u32;
        for cat in 0..CAT_COUNT {
            if self.category_items[cat].is_empty() {
                continue;
            }
            let pct = urand(pick, self.config.quota_min[cat], self.config.quota_max[cat]);
            targets[cat] = remaining * pct / 100;
            total_quota += targets[cat];
        }
        if total_quota < remaining && !self.category_items[CAT_TRADE_GOODS].is_empty() {
            targets[CAT_TRADE_GOODS] += remaining - total_quota;
        }
        let mut plan = Vec::new();
        for cat in 0..CAT_COUNT {
            for _ in 0..targets[cat] {
                let entry = self.items[self.pick_weighted_item(cat, pick)];
                let seller_type = Self::roll_seller_type(pick);
                let seller = self.pick_seller(seller_type, pick);
                let stack = Self::roll_stack_size(&entry, seller_type, pick);
                let (bid, buyout) = Self::apply_price_jitter(&entry, stack, seller_type, pick);
                plan.push(NewListing {
                    item_id: entry.item,
                    stack_size: stack,
                    bid,
                    buyout,
                    seller_guid: seller.character_guid,
                });
            }
        }
        plan
    }

    /// `UpdateBotBuyers` planning: shuffled candidate
    /// walk under the per-cycle budget. Returns
    /// (auction ids to buy out, copper spent).
    pub fn plan_buyer_purchases(
        &self,
        mut candidates: Vec<BuyerCandidate>,
        pick: &mut Pick<'_>,
    ) -> (Vec<u32>, u32) {
        if self.sellers.is_empty() {
            return (Vec::new(), 0);
        }
        candidates.retain(|c| c.buyout != 0 && !self.is_bot_owned(c.owner_guid));
        for i in (1..candidates.len()).rev() {
            let j = usize::try_from(urand(pick, 0, u32::try_from(i).unwrap_or(0)))
                .unwrap_or(0)
                .min(i);
            candidates.swap(i, j);
        }
        let mut budget = self.config.buyer_budget_per_cycle;
        let mut bought = Vec::new();
        for candidate in candidates {
            if budget == 0 {
                break;
            }
            let fair = self.fair_buyout(candidate.item_id, candidate.stack_size);
            if fair == 0 {
                continue;
            }
            let prob = Self::purchase_probability(candidate.buyout, fair);
            if prob <= 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let roll = urand(pick, 0, 10_000) as f32 / 100.0;
            if roll > prob || candidate.buyout > budget {
                continue;
            }
            budget -= candidate.buyout;
            bought.push(candidate.auction_id);
        }
        (bought, self.config.buyer_budget_per_cycle - budget)
    }

    /// Random seller identity as buyout buyer.
    pub fn random_seller_guid(&self, pick: &mut Pick<'_>) -> u32 {
        if self.sellers.is_empty() {
            return 0;
        }
        let idx = usize::try_from(urand(
            pick,
            0,
            u32::try_from(self.sellers.len())
                .unwrap_or(1)
                .saturating_sub(1),
        ))
        .unwrap_or(0);
        self.sellers[idx.min(self.sellers.len() - 1)].character_guid
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn entry(item: u32, category: u8) -> AhBotEntry {
        AhBotEntry {
            item,
            min_stack: 1,
            max_stack: 1,
            bid: 900,
            buyout: 1000,
            weight: 1.0,
            category,
            price_variance_pct: 0,
        }
    }

    fn core() -> AhBotCore {
        let mut config = AhBotConfig::default();
        config.enable = true;
        let sellers = vec![SellerIdentity {
            character_guid: 900_001,
            name: "Grimjaw".to_owned(),
            seller_type: SellerType::Casual,
        }];
        AhBotCore::load(config, vec![entry(2589, 0)], sellers, 0, true)
    }

    #[test]
    fn fair_buyout_scales_by_stack() {
        let core = core();
        assert_eq!(core.fair_buyout(2589, 1), 1000);
        assert_eq!(core.fair_buyout(2589, 20), 20_000);
        assert_eq!(core.fair_buyout(999_999, 1), 0);
    }

    #[test]
    fn purchase_probability_bands_match_fork() {
        assert_eq!(AhBotCore::purchase_probability(80, 100), 95.0);
        assert_eq!(AhBotCore::purchase_probability(100, 100), 80.0);
        assert_eq!(AhBotCore::purchase_probability(120, 100), 50.0);
        assert_eq!(AhBotCore::purchase_probability(150, 100), 20.0);
        assert_eq!(AhBotCore::purchase_probability(200, 100), 5.0);
        assert_eq!(AhBotCore::purchase_probability(500, 100), 1.0);
        assert_eq!(AhBotCore::purchase_probability(600, 100), 0.0);
        assert_eq!(AhBotCore::purchase_probability(10, 0), 0.0);
    }

    #[test]
    fn parse_category_defaults_to_trade_goods() {
        assert_eq!(parse_category("recipes"), 3);
        assert_eq!(parse_category("nonsense"), 0);
    }

    #[test]
    fn refresh_and_restock_gates() {
        let mut core = core();
        assert!(core.ready());
        assert!(core.needs_refresh(1_000_000, true));
        assert!(core.needs_refresh(1_000_000, false));
        core.mark_refreshed(1_000_000);
        assert!(!core.needs_refresh(1_000_100, false));
        assert!(core.needs_refresh(1_000_100 + 259_200, false));
        assert!(core.needs_restock(159));
        assert!(!core.needs_restock(160));
    }

    #[test]
    fn buyer_respects_budget_and_skips_bot_owned() {
        let core = core();
        let candidates = vec![
            BuyerCandidate {
                auction_id: 7,
                item_id: 2589,
                stack_size: 1,
                buyout: 800,
                owner_guid: 42,
            },
            BuyerCandidate {
                auction_id: 8,
                item_id: 2589,
                stack_size: 1,
                buyout: 800,
                owner_guid: 900_001,
            },
        ];
        let mut pick = |_lo: u32, _hi: u32| 0u32;
        let (bought, spent) = core.plan_buyer_purchases(candidates, &mut pick);
        assert_eq!(bought, vec![7]);
        assert_eq!(spent, 800);
    }

    #[test]
    fn unloaded_house_never_refreshes() {
        let sellers = vec![SellerIdentity {
            character_guid: 900_001,
            name: "Grimjaw".to_owned(),
            seller_type: SellerType::Casual,
        }];
        let core = AhBotCore::load(
            AhBotConfig::default(),
            vec![entry(2589, 0)],
            sellers,
            0,
            false,
        );
        assert!(!core.ready());
        assert!(!core.needs_refresh(1_000_000, true));
    }
}
