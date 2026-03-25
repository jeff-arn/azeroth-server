#ifndef _AUCTION_HOUSE_BOT_MGR_H
#define _AUCTION_HOUSE_BOT_MGR_H

#include "Policies/Singleton.h"
#include "SharedDefines.h"

#include <vector>
#include <memory>
#include <string>
#include <ctime>

class Player;

enum AHBotCategory : uint8
{
    AHBOT_CAT_TRADE_GOODS = 0,
    AHBOT_CAT_CRAFTING    = 1,
    AHBOT_CAT_CONSUMABLES = 2,
    AHBOT_CAT_RECIPES     = 3,
    AHBOT_CAT_EQUIPMENT   = 4,
    AHBOT_CAT_LEVELING    = 5,
    AHBOT_CAT_COUNT       = 6
};

enum AHBotSellerType : uint8
{
    AHBOT_SELLER_CASUAL      = 0,  // 1-2 stacks, slight markup
    AHBOT_SELLER_DEDICATED   = 1,  // bulk, slight discount
    AHBOT_SELLER_CRAFTER     = 2,  // finished products, markup
    AHBOT_SELLER_OPPORTUNIST = 3,  // rare items, high markup
    AHBOT_SELLER_COUNT       = 4
};

struct AuctionHouseBotEntry
{
    uint32 item;
    uint32 minStack;
    uint32 maxStack;
    uint32 bid;
    uint32 buyout;
    float  weight;
    uint8  category;
    uint8  priceVariancePct;
};

struct AHBotSellerIdentity
{
    uint32 characterGuid;
    std::string name;
    AHBotSellerType type;
};

struct AuctionHouseBotConfig
{
    bool   enable;
    uint32 ahfid;
    uint32 itemcount;
    uint32 refreshIntervalSecs;
    uint32 sellerAccountId;
    uint32 sellerCount;

    uint8 quotaMin[AHBOT_CAT_COUNT];
    uint8 quotaMax[AHBOT_CAT_COUNT];
};

class AuctionHouseBotMgr
{
    public:
        AuctionHouseBotMgr() = default;
        ~AuctionHouseBotMgr();

        void Load();
        void Update(bool force = false);

    protected:
        // Data
        std::vector<AuctionHouseBotEntry> m_items;
        std::vector<AHBotSellerIdentity>  m_sellers;
        std::unique_ptr<AuctionHouseBotConfig> m_config;
        AuctionHouseEntry const* m_auctionHouseEntry = nullptr;
        bool m_loaded = false;
        time_t m_lastRefreshTime = 0;

        // Per-category item pools (indices into m_items)
        std::vector<uint32> m_categoryItems[AHBOT_CAT_COUNT];
        std::vector<float>  m_categoryWeights[AHBOT_CAT_COUNT];
        float               m_categoryTotalWeight[AHBOT_CAT_COUNT] = {};

        // Methods
        void BuildCategoryPools();
        void LoadSellerIdentities();
        void CreateSellerIdentities();
        void WipeAllBotAuctions(AuctionHouseObject* ah);
        void PopulateAuctions(AuctionHouseObject* ah);
        void AddItem(AuctionHouseBotEntry const& e,
                     uint32 stackSize,
                     uint32 bidPrice,
                     uint32 buyoutPrice,
                     AHBotSellerIdentity const& seller,
                     AuctionHouseObject* ah);

        // Helpers
        uint32 PickWeightedItem(AHBotCategory cat) const;
        uint32 RollStackSize(AuctionHouseBotEntry const& e, AHBotSellerType sellerType) const;
        void   ApplyPriceJitter(uint32& bid, uint32& buyout,
                                AuctionHouseBotEntry const& e,
                                uint32 stackSize,
                                AHBotSellerType sellerType) const;
        AHBotSellerType RollSellerType() const;
        AHBotSellerIdentity const& PickSeller(AHBotSellerType type) const;
        uint32 ComputeCategoryTarget(AHBotCategory cat) const;
};

#define sAuctionHouseBotMgr MaNGOS::Singleton< AuctionHouseBotMgr >::Instance()
#endif
