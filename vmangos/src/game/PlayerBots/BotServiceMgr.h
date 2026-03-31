#ifndef _BOT_SERVICE_MGR_H
#define _BOT_SERVICE_MGR_H

#include "Policies/Singleton.h"
#include "ObjectGuid.h"
#include "Timer.h"

#include <map>
#include <unordered_map>
#include <string>
#include <vector>
#include <ctime>

class Player;

// ============================================================================
// Portal spell IDs by destination.
// ============================================================================
enum PortalSpellId : uint32
{
    PORTAL_SPELL_STORMWIND    = 10059,
    PORTAL_SPELL_IRONFORGE    = 11416,
    PORTAL_SPELL_DARNASSUS    = 11419,
    PORTAL_SPELL_ORGRIMMAR    = 11417,
    PORTAL_SPELL_UNDERCITY    = 11418,
    PORTAL_SPELL_THUNDERBLUFF = 11420,
};

// ============================================================================
// Enchant definitions — spell ID, keyword, tip tier.
// ============================================================================
enum EnchantTipTier : uint8
{
    ENCHANT_TIP_COMMON   = 0,  // 0-1g (trainer-learned)
    ENCHANT_TIP_MIDTIER  = 1,  // 2-5g (rep-gated/uncommon)
    ENCHANT_TIP_RARE     = 2,  // 5-10g (world drop, e.g., Crusader)
    ENCHANT_TIP_ULTRARARE = 3, // 10-20g (very rare, e.g., +30 SP, +55 Healing)
};

struct EnchantDefinition
{
    uint32 spellId;
    const char* keyword;       // match in whisper (e.g., "crusader")
    const char* displayName;   // friendly name for bot responses
    EnchantTipTier tipTier;
    bool isRareRecipe;         // not all bots know this
};

// ============================================================================
// Service request types.
// ============================================================================
enum BotServiceType : uint8
{
    BOT_SERVICE_NONE      = 0,
    BOT_SERVICE_PORTAL    = 1,
    BOT_SERVICE_ENCHANT   = 2,
    BOT_SERVICE_TRANSMUTE = 3,
    BOT_SERVICE_MOONCLOTH = 4,
    BOT_SERVICE_BUY       = 5,  // bot buys items from player
};

enum BotServiceState : uint8
{
    SERVICE_STATE_ASKED_DESTINATION = 0,  // Bot asked "which city?" / "which enchant?"
    SERVICE_STATE_WAITING_FOR_TRADE = 1,  // Bot said "trade me"
    SERVICE_STATE_TRADE_IN_PROGRESS = 2,  // Trade window open
    SERVICE_STATE_COMPLETE          = 3,  // Service delivered
};

struct BotServiceRequest
{
    ObjectGuid playerGuid;
    ObjectGuid botGuid;
    BotServiceType serviceType;
    BotServiceState state;
    uint32 spellId;            // portal spell, enchant spell, or transmute spell
    time_t createdAt;
    time_t lastUpdate;

    static constexpr uint32 PORTAL_TIP_MIN    = 10000;   // 1g in copper
    static constexpr uint32 ENCHANT_TIP_MIN   = 10000;   // 1g minimum
    static constexpr uint32 TRANSMUTE_TIP_MIN = 20000;   // 2g minimum
    static constexpr uint32 MOONCLOTH_TIP_MIN = 20000;   // 2g minimum
    static constexpr uint32 TIMEOUT_SECS      = 300;     // 5 min timeout

    // For BOT_SERVICE_BUY: tracks what the bot wants to buy
    uint32 buyItemId;       // item entry ID
    uint32 buyMaxQuantity;  // max stack to buy
    uint32 buyPriceCopper;  // price per unit in copper
};

// ============================================================================
// BotServiceMgr — manages stateful service interactions between bots and
// players. Singleton, updated from the world loop.
// ============================================================================
class BotServiceMgr
{
public:
    BotServiceMgr();

    void Load();
    void Update(uint32 diff);

    // === Whisper-driven service flow ===

    // Called from BotChatManager when a player whispers a bot.
    // Returns true if this is a service-related whisper that was handled.
    bool HandleServiceWhisper(Player* bot, ObjectGuid playerGuid,
                              const std::string& message);

    // === Trade handling ===

    // Called when a player initiates trade with a bot.
    // Returns true if the bot should accept (has a pending service).
    bool ShouldAcceptTrade(Player* bot, Player* player);

    // Called when the trade window is open and the bot should set up its side.
    // For enchanting, this sets the enchant spell on the bot's trade data.
    void OnTradeWindowOpen(Player* bot, Player* player);

    // Called after a trade with a bot completes successfully.
    // Delivers the service (casts portal, gives transmute product, etc.)
    void OnTradeCompleted(Player* bot, Player* player, uint32 goldReceived);

    // === Query ===

    BotServiceRequest* GetPendingRequest(ObjectGuid botGuid, ObjectGuid playerGuid);

    // Bot class checks.
    static bool IsMageBot(Player* bot);
    static bool IsEnchanterBot(Player* bot);
    static bool IsAlchemistBot(Player* bot);
    static bool IsTailorBot(Player* bot);

    // Create a service request proactively (from trade chat monitoring).
    // tradeChatMsg is the original message for parsing destination/details.
    void CreateProactiveRequest(Player* bot, ObjectGuid playerGuid, BotServiceType type,
                                const std::string& tradeChatMsg = "");

    // Check if a player already has an active request with any bot.
    bool HasActiveRequest(ObjectGuid playerGuid) const;

    // Clear all requests for a player (called on logout).
    void ClearRequestsForPlayer(ObjectGuid playerGuid);

    // === WTB Buy system ===

    // Called when a player whispers a bot about selling an item the bot wants.
    bool HandleBuyWhisper(Player* bot, ObjectGuid playerGuid, const std::string& message);

    // Get the buy price for an item (based on AH fair market value, discounted)
    uint32 GetBuyPriceForItem(uint32 itemId, uint32 quantity) const;

private:
    // === Portal helpers ===
    uint32 ResolvePortalSpell(Player* bot, const std::string& message) const;
    void DeliverPortal(Player* bot, Player* player, uint32 spellId);

    // === Enchant helpers ===
    EnchantDefinition const* ResolveEnchant(const std::string& message) const;
    bool BotKnowsEnchant(Player* bot, EnchantDefinition const* enchant) const;
    void DeliverEnchant(Player* bot, Player* player, uint32 spellId);
    uint32 GetEnchantTipMin(EnchantTipTier tier) const;

    // === Transmute/Mooncloth helpers ===
    void DeliverTransmute(Player* bot, Player* player);
    void DeliverMooncloth(Player* bot, Player* player);

    // === Shared helpers ===
    void BotWhisper(Player* bot, ObjectGuid targetGuid, const char* message);
    void CleanupExpiredRequests();

    using RequestKey = uint64;
    static RequestKey MakeKey(ObjectGuid botGuid, ObjectGuid playerGuid);

    std::map<RequestKey, BotServiceRequest> m_requests;

    // === WTB Buy helpers ===
    struct WtbItemDef
    {
        uint32 itemId;
        const char* keyword;       // match in whisper
        const char* displayName;   // for bot chat
        uint32 maxQuantity;
    };
    static const WtbItemDef s_wtbItems[];
    static const uint32 s_wtbItemCount;

    WtbItemDef const* ResolveWtbItem(const std::string& message) const;

    // Per-bot buy budget (resets daily)
    struct BotBuyBudget
    {
        uint32 remainingCopper;
        time_t lastResetTime;
    };
    std::unordered_map<uint32, BotBuyBudget> m_buyBudgets;
    uint32 m_buyBudgetPerDay;  // config: copper per bot per day
    void ResetBudgetIfNeeded(uint32 botGuid);

    ShortTimeTracker m_cleanupTimer;
    bool m_enabled;
    bool m_loaded;
};

#define sBotServiceMgr MaNGOS::Singleton<BotServiceMgr>::Instance()

#endif
