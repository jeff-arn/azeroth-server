#ifndef _BOT_SERVICE_MGR_H
#define _BOT_SERVICE_MGR_H

#include "Policies/Singleton.h"
#include "ObjectGuid.h"
#include "Timer.h"

#include <map>
#include <string>
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
// Service request types.
// ============================================================================
enum BotServiceType : uint8
{
    BOT_SERVICE_NONE    = 0,
    BOT_SERVICE_PORTAL  = 1,
    // Future: BOT_SERVICE_ENCHANT, BOT_SERVICE_TRANSMUTE, etc.
};

enum BotServiceState : uint8
{
    SERVICE_STATE_ASKED_DESTINATION = 0,  // Bot asked "which city?"
    SERVICE_STATE_WAITING_FOR_TRADE = 1,  // Bot said "meet me, 1g"
    SERVICE_STATE_TRADE_IN_PROGRESS = 2,  // Trade window open
    SERVICE_STATE_COMPLETE          = 3,  // Service delivered
};

struct BotServiceRequest
{
    ObjectGuid playerGuid;
    ObjectGuid botGuid;
    BotServiceType serviceType;
    BotServiceState state;
    uint32 portalSpellId;      // which portal to cast
    time_t createdAt;
    time_t lastUpdate;

    static constexpr uint32 PORTAL_TIP_MIN = 10000;  // 1g minimum tip in copper
    static constexpr uint32 TIMEOUT_SECS   = 300;    // 5 min timeout
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

    // Called after a trade with a bot completes successfully.
    // Delivers the service (casts portal, etc.)
    void OnTradeCompleted(Player* bot, Player* player, uint32 goldReceived);

    // === Query ===

    // Returns the pending request for this bot+player pair, or nullptr.
    BotServiceRequest* GetPendingRequest(ObjectGuid botGuid, ObjectGuid playerGuid);

    // Is this bot a mage that can provide portal services?
    static bool IsMageBot(Player* bot);

private:
    // Resolve portal destination from player's whisper text.
    uint32 ResolvePortalSpell(Player* bot, const std::string& message) const;

    // Send a whisper from bot to player.
    void BotWhisper(Player* bot, ObjectGuid targetGuid, const char* message);

    // Cast portal and clean up.
    void DeliverPortal(Player* bot, Player* player, uint32 spellId);

    // Clean up expired requests.
    void CleanupExpiredRequests();

    // Key: botGuid.GetCounter() ^ playerGuid.GetCounter() (unique pair)
    using RequestKey = uint64;
    static RequestKey MakeKey(ObjectGuid botGuid, ObjectGuid playerGuid);

    std::map<RequestKey, BotServiceRequest> m_requests;

    ShortTimeTracker m_cleanupTimer;
    bool m_enabled;
    bool m_loaded;
};

#define sBotServiceMgr MaNGOS::Singleton<BotServiceMgr>::Instance()

#endif
