#include "AuctionHouseBotMgr.h"
#include "AuctionHouseMgr.h"
#include "Chat.h"
#include "Config/Config.h"
#include "Database/DatabaseEnv.h"
#include "Item.h"
#include "Log.h"
#include "ObjectMgr.h"
#include "Policies/SingletonImp.h"
#include "Util.h"

#include <algorithm>
#include <cmath>

INSTANTIATE_SINGLETON_1(AuctionHouseBotMgr);

static const char* AHBOT_CATEGORY_NAMES[] = {"trade_goods", "crafting", "consumables", "recipes", "equipment", "leveling"};

static AHBotCategory ParseCategory(const std::string& str)
{
    for (uint8 i = 0; i < AHBOT_CAT_COUNT; ++i)
        if (str == AHBOT_CATEGORY_NAMES[i])
            return static_cast<AHBotCategory>(i);
    return AHBOT_CAT_TRADE_GOODS;
}

// Pool of lore-appropriate seller names for fake AH characters.
static const char* SELLER_NAMES[] = {"Grimjaw", "Thornweave", "Silkfingers", "Copperbell", "Ironhide", "Dustwalker", "Ashgrip", "Stonebrew", "Goldleaf", "Moonpetal", "Flamestitch", "Frostpick", "Brighthammer", "Shadowloom", "Steelclaw", "Embershank", "Mistweaver", "Oakbarrel", "Windsorrow", "Duskmantle", "Boulderback", "Sunthread", "Ravenforge", "Deepmine", "Wildroot", "Silversting", "Coalhand", "Mossgrip", "Starforge", "Rustblade"};
static const uint32 SELLER_NAME_COUNT = sizeof(SELLER_NAMES) / sizeof(SELLER_NAMES[0]);

AuctionHouseBotMgr::~AuctionHouseBotMgr()
{
    m_items.clear();
    m_sellers.clear();
    if (m_config)
        m_config.reset();
}

void AuctionHouseBotMgr::Load()
{
    m_items.clear();
    m_sellers.clear();
    m_loaded = false;

    for (uint8 i = 0; i < AHBOT_CAT_COUNT; ++i)
    {
        m_categoryItems[i].clear();
        m_categoryWeights[i].clear();
        m_categoryTotalWeight[i] = 0.0f;
    }

    if (m_config)
        m_config.reset();

    // Load items from DB.
    std::unique_ptr<QueryResult> result(WorldDatabase.Query("SELECT `item`, `min_stack`, `max_stack`, `bid`, `buyout`, `weight`, `category`, `price_variance_pct` "
                                                            "FROM `auctionhousebot`"));

    if (!result)
    {
        BarGoLink bar(1);
        bar.step();
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "");
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, ">> Loaded 0 AuctionHouseBot items");
        return;
    }

    uint32 count = 0;
    BarGoLink bar(result->GetRowCount());

    do
    {
        bar.step();
        Field* fields = result->Fetch();
        AuctionHouseBotEntry e;
        e.item = fields[0].GetUInt32();
        e.minStack = fields[1].GetUInt32();
        e.maxStack = fields[2].GetUInt32();
        e.bid = fields[3].GetUInt32();
        e.buyout = fields[4].GetUInt32();
        e.weight = fields[5].GetFloat();
        e.category = ParseCategory(fields[6].GetCppString());
        e.priceVariancePct = fields[7].GetUInt8();

        if (e.maxStack < e.minStack)
            e.maxStack = e.minStack;

        m_items.push_back(e);
        ++count;
    }
    while (result->NextRow());

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "");
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, ">> Loaded %u AuctionHouseBot items", count);

    // Load config.
    m_config = std::make_unique<AuctionHouseBotConfig>();
    m_config->enable = sConfig.GetBoolDefault("AHBot.Enable", false);
    m_config->ahfid = sConfig.GetIntDefault("AHBot.ah.fid", 120);
    m_config->itemcount = sConfig.GetIntDefault("AHBot.itemcount", 200);
    m_config->refreshIntervalSecs = sConfig.GetIntDefault("AHBot.RefreshInterval", 259200);
    m_config->sellerAccountId = sConfig.GetIntDefault("AHBot.SellerAccountId", 99);
    m_config->sellerCount = sConfig.GetIntDefault("AHBot.SellerCount", 12);

    m_config->quotaMin[AHBOT_CAT_TRADE_GOODS] = sConfig.GetIntDefault("AHBot.Quota.TradeGoods.Min", 35);
    m_config->quotaMax[AHBOT_CAT_TRADE_GOODS] = sConfig.GetIntDefault("AHBot.Quota.TradeGoods.Max", 40);
    m_config->quotaMin[AHBOT_CAT_CRAFTING] = sConfig.GetIntDefault("AHBot.Quota.Crafting.Min", 15);
    m_config->quotaMax[AHBOT_CAT_CRAFTING] = sConfig.GetIntDefault("AHBot.Quota.Crafting.Max", 20);
    m_config->quotaMin[AHBOT_CAT_CONSUMABLES] = sConfig.GetIntDefault("AHBot.Quota.Consumables.Min", 20);
    m_config->quotaMax[AHBOT_CAT_CONSUMABLES] = sConfig.GetIntDefault("AHBot.Quota.Consumables.Max", 25);
    m_config->quotaMin[AHBOT_CAT_RECIPES] = sConfig.GetIntDefault("AHBot.Quota.Recipes.Min", 3);
    m_config->quotaMax[AHBOT_CAT_RECIPES] = sConfig.GetIntDefault("AHBot.Quota.Recipes.Max", 5);
    m_config->quotaMin[AHBOT_CAT_EQUIPMENT] = sConfig.GetIntDefault("AHBot.Quota.Equipment.Min", 5);
    m_config->quotaMax[AHBOT_CAT_EQUIPMENT] = sConfig.GetIntDefault("AHBot.Quota.Equipment.Max", 10);
    m_config->quotaMin[AHBOT_CAT_LEVELING] = sConfig.GetIntDefault("AHBot.Quota.Leveling.Min", 10);
    m_config->quotaMax[AHBOT_CAT_LEVELING] = sConfig.GetIntDefault("AHBot.Quota.Leveling.Max", 15);

    m_buyerIntervalSecs = sConfig.GetIntDefault("AHBot.Buyer.IntervalSecs", 1800);
    m_buyerBudgetPerCycle = sConfig.GetIntDefault("AHBot.Buyer.BudgetPerCycle", 500000);

    m_auctionHouseEntry = sAuctionMgr.GetAuctionHouseEntry(m_config->ahfid);
    if (!m_auctionHouseEntry)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot::Load() : No auction house for faction %u.", m_config->ahfid);
        return;
    }

    BuildCategoryPools();
    LoadSellerIdentities();

    // Load last refresh time from DB.
    std::unique_ptr<QueryResult> stateResult(WorldDatabase.Query("SELECT `value` FROM `auctionhousebot_state` WHERE `key_name` = 'last_refresh_time'"));
    if (stateResult)
        m_lastRefreshTime = static_cast<time_t>(stateResult->Fetch()[0].GetUInt64());
    else
        m_lastRefreshTime = 0;

    m_loaded = true;
}

void AuctionHouseBotMgr::BuildCategoryPools()
{
    for (uint8 i = 0; i < AHBOT_CAT_COUNT; ++i)
    {
        m_categoryItems[i].clear();
        m_categoryWeights[i].clear();
        m_categoryTotalWeight[i] = 0.0f;
    }

    for (uint32 idx = 0; idx < m_items.size(); ++idx)
    {
        uint8 cat = m_items[idx].category;
        if (cat >= AHBOT_CAT_COUNT)
            cat = AHBOT_CAT_TRADE_GOODS;

        m_categoryItems[cat].push_back(idx);
        m_categoryTotalWeight[cat] += m_items[idx].weight;
        m_categoryWeights[cat].push_back(m_categoryTotalWeight[cat]);
    }

    for (uint8 i = 0; i < AHBOT_CAT_COUNT; ++i)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Category '%s' has %u items, total weight %.1f", AHBOT_CATEGORY_NAMES[i], (uint32)m_categoryItems[i].size(), m_categoryTotalWeight[i]);
    }
}

void AuctionHouseBotMgr::LoadSellerIdentities()
{
    m_sellers.clear();

    uint32 accountId = m_config->sellerAccountId;
    uint32 targetCount = m_config->sellerCount;

    // Query existing seller characters for this account.
    std::unique_ptr<QueryResult> result(CharacterDatabase.PQuery("SELECT `guid`, `name` FROM `characters` WHERE `account` = %u AND `guid` >= 900001 AND `guid` <= 900100", accountId));

    if (result)
    {
        do
        {
            Field* fields = result->Fetch();
            AHBotSellerIdentity seller;
            seller.characterGuid = fields[0].GetUInt32();
            seller.name = fields[1].GetCppString();
            seller.type = static_cast<AHBotSellerType>(m_sellers.size() % AHBOT_SELLER_COUNT);
            m_sellers.push_back(seller);
        }
        while (result->NextRow());
    }

    if (m_sellers.size() < targetCount)
        CreateSellerIdentities();

    // Assign seller types by distribution: 40% casual, 25% dedicated, 20% crafter, 15% opportunist.
    for (size_t i = 0; i < m_sellers.size(); ++i)
    {
        float pct = static_cast<float>(i) / static_cast<float>(m_sellers.size());
        if (pct < 0.40f)
            m_sellers[i].type = AHBOT_SELLER_CASUAL;
        else if (pct < 0.65f)
            m_sellers[i].type = AHBOT_SELLER_DEDICATED;
        else if (pct < 0.85f)
            m_sellers[i].type = AHBOT_SELLER_CRAFTER;
        else
            m_sellers[i].type = AHBOT_SELLER_OPPORTUNIST;
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: %u seller identities loaded.", (uint32)m_sellers.size());
}

void AuctionHouseBotMgr::CreateSellerIdentities()
{
    uint32 accountId = m_config->sellerAccountId;
    uint32 targetCount = m_config->sellerCount;
    uint32 baseGuid = 900001;

    for (uint32 i = 0; i < SELLER_NAME_COUNT && m_sellers.size() < targetCount; ++i)
    {
        uint32 guid = baseGuid + i;
        const char* name = SELLER_NAMES[i];

        // Check if name is already taken.
        std::unique_ptr<QueryResult> nameCheck(CharacterDatabase.PQuery("SELECT `guid` FROM `characters` WHERE `name` = '%s'", name));
        if (nameCheck)
            continue;

        // Check if guid is already used.
        std::unique_ptr<QueryResult> guidCheck(CharacterDatabase.PQuery("SELECT `guid` FROM `characters` WHERE `guid` = %u", guid));
        if (guidCheck)
        {
            // Already exists, should have been loaded above.
            continue;
        }

        // Create a minimal character row. Race 1 = Human, Class 1 = Warrior, level 60.
        CharacterDatabase.PExecute("INSERT INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, "
                                   "`zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`) "
                                   "VALUES (%u, %u, '%s', 1, 1, 0, 60, 0, 0, 0, 0, 0, 0)",
                                   guid, accountId, name);

        AHBotSellerIdentity seller;
        seller.characterGuid = guid;
        seller.name = name;
        seller.type = AHBOT_SELLER_CASUAL;
        m_sellers.push_back(seller);

        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Created seller '%s' (GUID %u).", name, guid);
    }
}

void AuctionHouseBotMgr::Update(bool force /* = false */)
{
    if (!m_loaded)
        return;

    ASSERT(m_config);
    ASSERT(m_auctionHouseEntry);

    if (!(m_config->enable || force))
        return;

    if (m_items.empty() || m_sellers.empty())
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "AHBot::Update() : No items or sellers configured.");
        return;
    }

    AuctionHouseObject* auctionHouse = sAuctionMgr.GetAuctionsMap(m_auctionHouseEntry);
    if (!auctionHouse)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "AHBot::Update() : No auction house for faction %u.", m_config->ahfid);
        return;
    }

    time_t now = time(nullptr);
    bool needsRefresh = force || (m_lastRefreshTime == 0) || (static_cast<uint32>(now - m_lastRefreshTime) >= m_config->refreshIntervalSecs);

    if (needsRefresh)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Performing full AH refresh...");
        WipeAllBotAuctions(auctionHouse);
        PopulateAuctions(auctionHouse);

        m_lastRefreshTime = now;
        WorldDatabase.PExecute("UPDATE `auctionhousebot_state` SET `value` = " UI64FMTD " WHERE `key_name` = 'last_refresh_time'", static_cast<uint64>(m_lastRefreshTime));

        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Refresh complete. AH now has %u listings.", auctionHouse->GetCount());
    }
    else
    {
        // Top up if items have been bought by players since last refresh.
        uint32 currentCount = auctionHouse->GetCount();
        if (currentCount < m_config->itemcount * 80 / 100) // Restock if below 80% of target.
        {
            uint32 toAdd = m_config->itemcount - currentCount;
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Restocking %u items (current: %u, target: %u).", toAdd, currentCount, m_config->itemcount);
            PopulateAuctions(auctionHouse);
        }
    }

    // Run bot buyer scan periodically.
    if (static_cast<uint32>(now - m_lastBuyerRunTime) >= m_buyerIntervalSecs)
    {
        UpdateBotBuyers(auctionHouse);
        m_lastBuyerRunTime = now;
    }
}

void AuctionHouseBotMgr::WipeAllBotAuctions(AuctionHouseObject* ah)
{
    uint32 accountId = m_config->sellerAccountId;

    // Collect bot auction IDs first to avoid modifying map while iterating.
    // Match both the configured seller account AND legacy owner=0 auctions from the old AH bot.
    std::vector<uint32> toRemove;
    AuctionHouseObject::AuctionEntryMap* auctions = ah->GetAuctions();
    for (auto const& itr : *auctions)
    {
        AuctionEntry* entry = itr.second;
        if (entry->ownerAccount == accountId || entry->owner == 0)
            toRemove.push_back(entry->Id);
    }

    for (uint32 auctionId : toRemove)
    {
        AuctionEntry* entry = ah->GetAuction(auctionId);
        if (!entry)
            continue;

        // Remove the item from the AH item manager.
        Item* item = sAuctionMgr.GetAItem(entry->itemGuidLow);
        if (item)
        {
            item->DeleteFromDB();
            sAuctionMgr.RemoveAItem(entry->itemGuidLow);
            delete item;
        }

        entry->DeleteFromDB();
        ah->RemoveAuction(entry);
        delete entry;
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot: Wiped %u bot-owned auctions.", (uint32)toRemove.size());
}

void AuctionHouseBotMgr::PopulateAuctions(AuctionHouseObject* ah)
{
    uint32 currentCount = ah->GetCount();
    uint32 targetTotal = m_config->itemcount;

    if (currentCount >= targetTotal)
        return;

    uint32 remaining = targetTotal - currentCount;

    // Compute per-category targets.
    uint32 categoryTargets[AHBOT_CAT_COUNT] = {};
    uint32 totalQuota = 0;

    for (uint8 cat = 0; cat < AHBOT_CAT_COUNT; ++cat)
    {
        if (m_categoryItems[cat].empty())
        {
            categoryTargets[cat] = 0;
            continue;
        }
        uint32 pct = urand(m_config->quotaMin[cat], m_config->quotaMax[cat]);
        categoryTargets[cat] = remaining * pct / 100;
        totalQuota += categoryTargets[cat];
    }

    // If quotas don't add up to remaining due to rounding, distribute leftover to trade_goods.
    if (totalQuota < remaining && !m_categoryItems[AHBOT_CAT_TRADE_GOODS].empty())
        categoryTargets[AHBOT_CAT_TRADE_GOODS] += (remaining - totalQuota);

    for (uint8 cat = 0; cat < AHBOT_CAT_COUNT; ++cat)
    {
        if (categoryTargets[cat] == 0 || m_categoryItems[cat].empty())
            continue;

        for (uint32 i = 0; i < categoryTargets[cat]; ++i)
        {
            uint32 itemIdx = PickWeightedItem(static_cast<AHBotCategory>(cat));
            AuctionHouseBotEntry const& e = m_items[itemIdx];

            AHBotSellerType sellerType = RollSellerType();
            AHBotSellerIdentity const& seller = PickSeller(sellerType);

            uint32 stackSize = RollStackSize(e, sellerType);
            uint32 bidPrice = e.bid;
            uint32 buyoutPrice = e.buyout;
            ApplyPriceJitter(bidPrice, buyoutPrice, e, stackSize, sellerType);

            AddItem(e, stackSize, bidPrice, buyoutPrice, seller, ah);
        }
    }
}

uint32 AuctionHouseBotMgr::PickWeightedItem(AHBotCategory cat) const
{
    if (m_categoryItems[cat].empty())
        return 0;

    float roll = frand(0.0f, m_categoryTotalWeight[cat]);

    // Binary search for the item whose cumulative weight includes this roll.
    auto const& weights = m_categoryWeights[cat];
    auto it = std::lower_bound(weights.begin(), weights.end(), roll);
    uint32 idx = static_cast<uint32>(std::distance(weights.begin(), it));

    if (idx >= m_categoryItems[cat].size())
        idx = static_cast<uint32>(m_categoryItems[cat].size()) - 1;

    return m_categoryItems[cat][idx];
}

uint32 AuctionHouseBotMgr::RollStackSize(AuctionHouseBotEntry const& e, AHBotSellerType sellerType) const
{
    if (e.minStack == e.maxStack)
        return e.minStack;

    switch (sellerType)
    {
    case AHBOT_SELLER_CASUAL:
        {
            // Casual: small stacks (1-2 units above min, or min itself).
            uint32 maxForCasual = std::min(e.maxStack, e.minStack + 2u);
            return urand(e.minStack, maxForCasual);
        }
    case AHBOT_SELLER_DEDICATED:
        {
            // Dedicated: large stacks near max.
            uint32 minForDedicated = (e.maxStack > 2) ? (e.maxStack - 2) : e.minStack;
            return urand(minForDedicated, e.maxStack);
        }
    default:
        return urand(e.minStack, e.maxStack);
    }
}

void AuctionHouseBotMgr::ApplyPriceJitter(uint32& bid, uint32& buyout, AuctionHouseBotEntry const& e, uint32 stackSize, AHBotSellerType sellerType) const
{
    // Seller type price modifier.
    float sellerMod = 0.0f;
    switch (sellerType)
    {
    case AHBOT_SELLER_CASUAL:
        sellerMod = 0.05f;
        break; // +5%
    case AHBOT_SELLER_DEDICATED:
        sellerMod = -0.07f;
        break; // -7%
    case AHBOT_SELLER_CRAFTER:
        sellerMod = 0.15f;
        break; // +15%
    case AHBOT_SELLER_OPPORTUNIST:
        sellerMod = 0.25f;
        break; // +25%
    default:
        break;
    }

    // Random jitter within the item's variance percentage.
    float jitter = frand(-static_cast<float>(e.priceVariancePct), static_cast<float>(e.priceVariancePct)) / 100.0f;

    float priceMod = 1.0f + sellerMod + jitter;
    if (priceMod < 0.5f)
        priceMod = 0.5f;

    // Scale price by stack size relative to the base stack size.
    // DB prices are for a full stack at min_stack, so scale proportionally.
    float stackRatio = static_cast<float>(stackSize) / static_cast<float>(e.minStack);

    buyout = static_cast<uint32>(std::round(e.buyout * stackRatio * priceMod));
    if (buyout < 1)
        buyout = 1;

    // Bid is 70-90% of buyout.
    float bidPct = frand(0.70f, 0.90f);
    bid = static_cast<uint32>(std::round(buyout * bidPct));
    if (bid < 1)
        bid = 1;
}

AHBotSellerType AuctionHouseBotMgr::RollSellerType() const
{
    uint32 roll = urand(1, 100);
    if (roll <= 40)
        return AHBOT_SELLER_CASUAL;
    if (roll <= 65)
        return AHBOT_SELLER_DEDICATED;
    if (roll <= 85)
        return AHBOT_SELLER_CRAFTER;
    return AHBOT_SELLER_OPPORTUNIST;
}

AHBotSellerIdentity const& AuctionHouseBotMgr::PickSeller(AHBotSellerType type) const
{
    // Try to find a seller of the requested type.
    std::vector<uint32> matching;
    for (uint32 i = 0; i < m_sellers.size(); ++i)
    {
        if (m_sellers[i].type == type)
            matching.push_back(i);
    }

    if (!matching.empty())
        return m_sellers[matching[urand(0, static_cast<uint32>(matching.size()) - 1)]];

    // Fallback: any seller.
    return m_sellers[urand(0, static_cast<uint32>(m_sellers.size()) - 1)];
}

uint32 AuctionHouseBotMgr::ComputeCategoryTarget(AHBotCategory cat) const
{
    uint32 pct = urand(m_config->quotaMin[cat], m_config->quotaMax[cat]);
    return m_config->itemcount * pct / 100;
}

void AuctionHouseBotMgr::AddItem(AuctionHouseBotEntry const& e, uint32 stackSize, uint32 bidPrice, uint32 buyoutPrice, AHBotSellerIdentity const& seller, AuctionHouseObject* ah)
{
    ASSERT(m_auctionHouseEntry);

    ItemPrototype const* prototype = sObjectMgr.GetItemPrototype(e.item);
    if (!prototype)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot::AddItem() : Item %u does not exist.", e.item);
        return;
    }

    Item* item = Item::CreateItem(e.item, 1);
    if (!item)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot::AddItem() : Cannot create item %u.", e.item);
        return;
    }

    uint32 randomPropertyId = Item::GenerateItemRandomPropertyId(e.item);
    if (randomPropertyId != 0)
        item->SetItemRandomProperties(randomPropertyId);

    item->SetCount(stackSize);

    // All bot auctions last 48 hours (aligned with 72h refresh cycle).
    uint32 etime = 172800;

    uint32 dep = sAuctionMgr.GetAuctionDeposit(m_auctionHouseEntry, etime, item);

    AuctionEntry* auctionEntry = new AuctionEntry;
    auctionEntry->Id = sObjectMgr.GenerateAuctionID();
    auctionEntry->auctionHouseEntry = m_auctionHouseEntry;
    auctionEntry->itemGuidLow = item->GetGUIDLow();
    auctionEntry->itemTemplate = item->GetEntry();
    auctionEntry->owner = seller.characterGuid;
    auctionEntry->ownerAccount = m_config->sellerAccountId;
    auctionEntry->startbid = bidPrice;
    auctionEntry->buyout = buyoutPrice;
    auctionEntry->bidder = 0;
    auctionEntry->bid = 0;
    auctionEntry->deposit = dep;
    auctionEntry->depositTime = time(nullptr);
    auctionEntry->expireTime = static_cast<time_t>(etime) + time(nullptr);

    item->SaveToDB();

    sAuctionMgr.AddAItem(item);
    ah->AddAuction(auctionEntry);
    auctionEntry->SaveToDB();
}

// ============================================================================
// Bot buyer system — purchases player-listed auctions at fair prices.
// ============================================================================

void AuctionHouseBotMgr::UpdateBotBuyers(AuctionHouseObject* ah)
{
    if (!ah || m_sellers.empty())
        return;

    AuctionHouseObject::AuctionEntryMap* auctions = ah->GetAuctions();
    if (!auctions || auctions->empty())
        return;

    uint32 budgetRemaining = m_buyerBudgetPerCycle;
    uint32 purchaseCount = 0;

    // Collect candidate auctions (player-listed, has buyout, not already bid on).
    std::vector<AuctionEntry*> candidates;
    for (auto& pair : *auctions)
    {
        AuctionEntry* entry = pair.second;
        if (!entry || entry->buyout == 0)
            continue;

        // Skip bot-owned auctions.
        if (IsBotOwned(entry->owner))
            continue;

        // Skip auctions already bid on.
        if (entry->bidder != 0)
            continue;

        candidates.push_back(entry);
    }

    // Shuffle to avoid always buying the same items first.
    if (candidates.size() > 1)
    {
        for (uint32 i = static_cast<uint32>(candidates.size()) - 1; i > 0; --i)
        {
            uint32 j = urand(0, i);
            std::swap(candidates[i], candidates[j]);
        }
    }

    for (AuctionEntry* entry : candidates)
    {
        if (budgetRemaining == 0)
            break;

        // Get the item count from the actual item.
        Item* item = sAuctionMgr.GetAItem(entry->itemGuidLow);
        if (!item)
            continue;

        uint32 stackSize = item->GetCount();
        uint32 fairBuyout = GetFairBuyoutForItem(entry->itemTemplate, stackSize);
        if (fairBuyout == 0)
            continue; // Unknown item — don't buy.

        float prob = GetPurchaseProbability(entry->buyout, fairBuyout);
        if (prob <= 0.0f)
            continue;

        // Roll the dice.
        float roll = static_cast<float>(urand(0, 10000)) / 100.0f; // 0.00 - 100.00
        if (roll > prob)
            continue;

        // Can we afford it?
        if (entry->buyout > budgetRemaining)
            continue;

        // Execute the buyout! The buyer is a random bot seller identity.
        AHBotSellerIdentity const& buyer = m_sellers[urand(0, static_cast<uint32>(m_sellers.size()) - 1)];

        uint32 buyoutPrice = entry->buyout;

        // Update the entry with the buyer info before sending mail.
        entry->bidder = buyer.characterGuid;
        entry->bid = buyoutPrice;

        // Send gold to the player seller via mail (the auction system handles this).
        sAuctionMgr.SendAuctionSuccessfulMail(entry);

        // Remove auction and item from AH.
        sAuctionMgr.RemoveAItem(entry->itemGuidLow);
        entry->DeleteFromDB();
        ah->RemoveAuction(entry);

        // Delete the item instance (it was "purchased" by the bot, effectively consumed).
        if (item)
        {
            item->DeleteFromDB();
            delete item;
        }

        delete entry;

        budgetRemaining -= buyoutPrice;
        ++purchaseCount;
    }

    if (purchaseCount > 0)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "AHBot Buyer: Purchased %u player auctions (budget used: %ug %us %uc).", purchaseCount, (m_buyerBudgetPerCycle - budgetRemaining) / 10000, ((m_buyerBudgetPerCycle - budgetRemaining) % 10000) / 100, (m_buyerBudgetPerCycle - budgetRemaining) % 100);
    }
}

uint32 AuctionHouseBotMgr::GetFairBuyoutForItem(uint32 itemId, uint32 stackSize) const
{
    // Look up the item in our bot item table. Use the per-unit buyout price.
    for (auto const& e : m_items)
    {
        if (e.item == itemId)
        {
            // The stored buyout is per-stack at minStack, so calculate per-unit.
            uint32 perUnit = e.buyout / std::max(e.minStack, 1u);
            return perUnit * stackSize;
        }
    }
    return 0; // Unknown item.
}

float AuctionHouseBotMgr::GetPurchaseProbability(uint32 listingBuyout, uint32 fairBuyout) const
{
    if (fairBuyout == 0)
        return 0.0f;

    float ratio = static_cast<float>(listingBuyout) / static_cast<float>(fairBuyout);

    // Price at or below fair market value — very likely to buy.
    if (ratio <= 0.8f)
        return 95.0f; // bargain
    if (ratio <= 1.0f)
        return 80.0f; // at market
    if (ratio <= 1.2f)
        return 50.0f; // slightly over
    if (ratio <= 1.5f)
        return 20.0f; // overpriced
    if (ratio <= 2.0f)
        return 5.0f; // significantly overpriced
    if (ratio <= 5.0f)
        return 1.0f; // absurd
    return 0.0f; // Peacebloom for 1000g — never
}

bool AuctionHouseBotMgr::IsBotOwned(uint32 ownerGuid) const
{
    for (auto const& seller : m_sellers)
    {
        if (seller.characterGuid == ownerGuid)
            return true;
    }
    return false;
}

// Chat command handlers.

bool ChatHandler::HandleAHBotUpdateCommand(char* args)
{
    sAuctionHouseBotMgr.Update(true);
    SendSysMessage("[AHBot] Update finished.");
    return true;
}

bool ChatHandler::HandleAHBotReloadCommand(char* args)
{
    sAuctionHouseBotMgr.Load();
    sAuctionHouseBotMgr.Update(true);
    SendSysMessage("[AHBot] Reload finished.");
    return true;
}
