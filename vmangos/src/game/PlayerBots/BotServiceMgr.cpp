#include "BotServiceMgr.h"
#include "Policies/SingletonImp.h"
#include "Player.h"
#include "ObjectAccessor.h"
#include "World.h"
#include "Chat/Chat.h"
#include "Spell.h"
#include "Config/Config.h"
#include "Log.h"
#include "Util.h"

INSTANTIATE_SINGLETON_1(BotServiceMgr);

// ============================================================================
// Portal destination lookup tables.
// ============================================================================
struct PortalDestination
{
    const char* keyword;     // substring to match in whisper
    uint32 allianceSpell;    // spell for Alliance bots
    uint32 hordeSpell;       // spell for Horde bots
};

static const PortalDestination PORTAL_DESTINATIONS[] = {
    { "stormwind", PORTAL_SPELL_STORMWIND,    0 },
    { "sw",        PORTAL_SPELL_STORMWIND,    0 },
    { "storm",     PORTAL_SPELL_STORMWIND,    0 },
    { "ironforge", PORTAL_SPELL_IRONFORGE,    0 },
    { "if",        PORTAL_SPELL_IRONFORGE,    0 },
    { "darnassus", PORTAL_SPELL_DARNASSUS,    0 },
    { "darn",      PORTAL_SPELL_DARNASSUS,    0 },
    { "orgrimmar", 0, PORTAL_SPELL_ORGRIMMAR    },
    { "org",       0, PORTAL_SPELL_ORGRIMMAR    },
    { "undercity", 0, PORTAL_SPELL_UNDERCITY    },
    { "uc",        0, PORTAL_SPELL_UNDERCITY    },
    { "thunder",   0, PORTAL_SPELL_THUNDERBLUFF },
    { "tb",        0, PORTAL_SPELL_THUNDERBLUFF },
};
static const uint32 PORTAL_DEST_COUNT = sizeof(PORTAL_DESTINATIONS) / sizeof(PORTAL_DESTINATIONS[0]);

// ============================================================================
// Bot responses for the portal service flow.
// ============================================================================
static const char* PORTAL_ASK_DESTINATION[] = {
    "where do you need a port?",
    "sure, which city?",
    "where to?",
    "got ports, where do you need?",
};
static const uint32 PORTAL_ASK_DEST_COUNT = sizeof(PORTAL_ASK_DESTINATION) / sizeof(PORTAL_ASK_DESTINATION[0]);

static const char* PORTAL_CONFIRM[] = {
    "sure, trade me 1g and I'll open it up",
    "np, 1g tip. trade me when you're ready",
    "yeah I got that, trade me 1g",
    "coming right up, just trade me 1g tip",
};
static const uint32 PORTAL_CONFIRM_COUNT = sizeof(PORTAL_CONFIRM) / sizeof(PORTAL_CONFIRM[0]);

static const char* PORTAL_WRONG_FACTION[] = {
    "sorry, I can only port to Alliance cities",
    "I don't have that port, only Alliance cities",
    "sorry, I can only port to Horde cities",
    "can't do that one, only Horde cities",
};

static const char* PORTAL_THANKS[] = {
    "np, safe travels",
    "enjoy, cya",
    "anytime!",
    "gl out there",
};
static const uint32 PORTAL_THANKS_COUNT = sizeof(PORTAL_THANKS) / sizeof(PORTAL_THANKS[0]);

static const char* PORTAL_LOW_TIP[] = {
    "need at least 1g for the port",
    "1g min for a portal",
    "gonna need a bit more, 1g for ports",
};
static const uint32 PORTAL_LOW_TIP_COUNT = sizeof(PORTAL_LOW_TIP) / sizeof(PORTAL_LOW_TIP[0]);

// ============================================================================
// Mage portal trade chat ads (picked up by BotChatManager).
// ============================================================================
extern const char* TRADE_MESSAGES_PORTALS[] = {
    "WTS portals [Stormwind] [Ironforge] [Darnassus] 1g PST",
    "Mage LF port tips, 1g any city. PST",
    "WTS ports to any city, 1g. PST",
    "Portals up, 1g tip. SW/IF/Darn. PST",
};
extern const uint32 TRADE_PORTALS_COUNT = sizeof(TRADE_MESSAGES_PORTALS) / sizeof(TRADE_MESSAGES_PORTALS[0]);

extern const char* TRADE_MESSAGES_PORTALS_HORDE[] = {
    "WTS portals [Orgrimmar] [Undercity] [Thunder Bluff] 1g PST",
    "Mage LF port tips, 1g any city. PST",
    "WTS ports to any city, 1g. PST",
    "Portals up, 1g tip. Org/UC/TB. PST",
};
extern const uint32 TRADE_PORTALS_HORDE_COUNT = sizeof(TRADE_MESSAGES_PORTALS_HORDE) / sizeof(TRADE_MESSAGES_PORTALS_HORDE[0]);

// ============================================================================
// Implementation
// ============================================================================

BotServiceMgr::BotServiceMgr()
    : m_cleanupTimer(30000)  // cleanup check every 30s
    , m_enabled(false)
    , m_loaded(false)
{
}

void BotServiceMgr::Load()
{
    m_enabled = sConfig.GetBoolDefault("BotService.Enable", true);
    m_loaded = true;
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotServiceMgr: Loaded. Enabled: %s",
        m_enabled ? "yes" : "no");
}

void BotServiceMgr::Update(uint32 diff)
{
    if (!m_loaded || !m_enabled)
        return;

    m_cleanupTimer.Update(diff);
    if (m_cleanupTimer.Passed())
    {
        m_cleanupTimer.Reset(30000);
        CleanupExpiredRequests();
    }
}

// ============================================================================
// Whisper-driven portal service flow.
// ============================================================================

bool BotServiceMgr::HandleServiceWhisper(Player* bot, ObjectGuid playerGuid,
                                          const std::string& message)
{
    if (!m_enabled || !bot)
        return false;

    // Only mage bots can provide portal services.
    if (!IsMageBot(bot))
        return false;

    std::string lower = message;
    for (auto& c : lower)
        c = static_cast<char>(tolower(c));

    RequestKey key = MakeKey(bot->GetObjectGuid(), playerGuid);

    // Check if we have an existing request.
    auto it = m_requests.find(key);

    if (it != m_requests.end())
    {
        BotServiceRequest& req = it->second;

        if (req.state == SERVICE_STATE_ASKED_DESTINATION)
        {
            // Player is responding with a city name.
            uint32 spellId = ResolvePortalSpell(bot, lower);
            if (spellId)
            {
                req.portalSpellId = spellId;
                req.state = SERVICE_STATE_WAITING_FOR_TRADE;
                req.lastUpdate = time(nullptr);
                BotWhisper(bot, playerGuid,
                    PORTAL_CONFIRM[urand(0, PORTAL_CONFIRM_COUNT - 1)]);
                return true;
            }
            else
            {
                // Check if they said a city for the wrong faction.
                bool isAlliance = (bot->GetTeam() == ALLIANCE);
                BotWhisper(bot, playerGuid,
                    isAlliance ? PORTAL_WRONG_FACTION[0] : PORTAL_WRONG_FACTION[2]);
                return true;
            }
        }

        // If we're waiting for trade and they whisper again, just remind them.
        if (req.state == SERVICE_STATE_WAITING_FOR_TRADE)
        {
            BotWhisper(bot, playerGuid, "just trade me when you're ready");
            return true;
        }
    }

    // New request: player mentions "portal" or "port" to a mage bot.
    if (lower.find("portal") != std::string::npos ||
        lower.find("port") != std::string::npos)
    {
        // Check if they also specified a destination in the same message.
        uint32 spellId = ResolvePortalSpell(bot, lower);
        if (spellId)
        {
            // They gave us everything in one message — skip to waiting for trade.
            BotServiceRequest req;
            req.playerGuid = playerGuid;
            req.botGuid = bot->GetObjectGuid();
            req.serviceType = BOT_SERVICE_PORTAL;
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            req.portalSpellId = spellId;
            req.createdAt = time(nullptr);
            req.lastUpdate = time(nullptr);
            m_requests[key] = req;

            BotWhisper(bot, playerGuid,
                PORTAL_CONFIRM[urand(0, PORTAL_CONFIRM_COUNT - 1)]);
            return true;
        }

        // No destination specified — ask which city.
        BotServiceRequest req;
        req.playerGuid = playerGuid;
        req.botGuid = bot->GetObjectGuid();
        req.serviceType = BOT_SERVICE_PORTAL;
        req.state = SERVICE_STATE_ASKED_DESTINATION;
        req.portalSpellId = 0;
        req.createdAt = time(nullptr);
        req.lastUpdate = time(nullptr);
        m_requests[key] = req;

        BotWhisper(bot, playerGuid,
            PORTAL_ASK_DESTINATION[urand(0, PORTAL_ASK_DEST_COUNT - 1)]);
        return true;
    }

    return false;  // Not a service-related whisper.
}

// ============================================================================
// Trade handling.
// ============================================================================

bool BotServiceMgr::ShouldAcceptTrade(Player* bot, Player* player)
{
    if (!m_enabled)
        return false;

    BotServiceRequest* req = GetPendingRequest(bot->GetObjectGuid(), player->GetObjectGuid());
    if (!req || req->state != SERVICE_STATE_WAITING_FOR_TRADE)
        return false;

    req->state = SERVICE_STATE_TRADE_IN_PROGRESS;
    req->lastUpdate = time(nullptr);
    return true;
}

void BotServiceMgr::OnTradeCompleted(Player* bot, Player* player, uint32 goldReceived)
{
    if (!m_enabled)
        return;

    RequestKey key = MakeKey(bot->GetObjectGuid(), player->GetObjectGuid());
    auto it = m_requests.find(key);
    if (it == m_requests.end())
        return;

    BotServiceRequest& req = it->second;

    if (req.serviceType == BOT_SERVICE_PORTAL)
    {
        if (goldReceived < BotServiceRequest::PORTAL_TIP_MIN)
        {
            // Not enough gold — whisper and don't deliver.
            BotWhisper(bot, player->GetObjectGuid(),
                PORTAL_LOW_TIP[urand(0, PORTAL_LOW_TIP_COUNT - 1)]);
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            req.lastUpdate = time(nullptr);
            return;
        }

        // Deliver the portal!
        DeliverPortal(bot, player, req.portalSpellId);

        // Send thanks and clean up.
        BotWhisper(bot, player->GetObjectGuid(),
            PORTAL_THANKS[urand(0, PORTAL_THANKS_COUNT - 1)]);

        m_requests.erase(it);
    }
}

// ============================================================================
// Helpers.
// ============================================================================

BotServiceRequest* BotServiceMgr::GetPendingRequest(ObjectGuid botGuid, ObjectGuid playerGuid)
{
    RequestKey key = MakeKey(botGuid, playerGuid);
    auto it = m_requests.find(key);
    if (it != m_requests.end())
        return &it->second;
    return nullptr;
}

bool BotServiceMgr::IsMageBot(Player* bot)
{
    if (!bot)
        return false;
    // Must be a bot (account >= 100) and a mage (class 8).
    if (!bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return false;
    return bot->GetClass() == CLASS_MAGE;
}

uint32 BotServiceMgr::ResolvePortalSpell(Player* bot, const std::string& message) const
{
    bool isAlliance = (bot->GetTeam() == ALLIANCE);

    for (uint32 i = 0; i < PORTAL_DEST_COUNT; ++i)
    {
        if (message.find(PORTAL_DESTINATIONS[i].keyword) != std::string::npos)
        {
            uint32 spellId = isAlliance
                ? PORTAL_DESTINATIONS[i].allianceSpell
                : PORTAL_DESTINATIONS[i].hordeSpell;
            if (spellId)
                return spellId;
            // Found a keyword but wrong faction — return 0, caller handles.
            return 0;
        }
    }
    return 0;
}

void BotServiceMgr::BotWhisper(Player* bot, ObjectGuid targetGuid, const char* message)
{
    Player* target = sObjectAccessor.FindPlayer(targetGuid);
    if (!target || !target->GetSession())
        return;

    WorldPacket data;
    ChatHandler::BuildChatPacket(data, CHAT_MSG_WHISPER, message,
        LANG_UNIVERSAL, bot->GetChatTag(), bot->GetObjectGuid(), bot->GetName());
    target->GetSession()->SendPacket(&data);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotService: %s whispers to %s: %s",
        bot->GetName(), target->GetName(), message);
}

void BotServiceMgr::DeliverPortal(Player* bot, Player* player, uint32 spellId)
{
    if (!spellId)
        return;

    SpellEntry const* spellEntry = sSpellMgr.GetSpellEntry(spellId);
    if (!spellEntry)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "BotService: Unknown portal spell %u", spellId);
        return;
    }

    // Cast the portal spell. Bots inherit Player and can cast spells directly.
    bot->CastSpell(bot, spellEntry, false);

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotService: %s casts Portal (spell %u) for %s",
        bot->GetName(), spellId, player->GetName());
}

void BotServiceMgr::CleanupExpiredRequests()
{
    time_t now = time(nullptr);
    for (auto it = m_requests.begin(); it != m_requests.end(); )
    {
        if (now - it->second.lastUpdate > BotServiceRequest::TIMEOUT_SECS)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotService: Expired request (type %u) cleaned up",
                it->second.serviceType);
            it = m_requests.erase(it);
        }
        else
            ++it;
    }
}

BotServiceMgr::RequestKey BotServiceMgr::MakeKey(ObjectGuid botGuid, ObjectGuid playerGuid)
{
    // Combine the two counter values into a single 64-bit key.
    return (static_cast<uint64>(botGuid.GetCounter()) << 32) |
            static_cast<uint64>(playerGuid.GetCounter());
}
