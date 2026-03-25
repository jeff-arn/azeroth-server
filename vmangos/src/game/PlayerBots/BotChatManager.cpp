#include "BotChatManager.h"
#include "BotServiceMgr.h"
#include "Policies/SingletonImp.h"
#include "Player.h"
#include "ObjectAccessor.h"
#include "ObjectMgr.h"
#include "World.h"
#include "Chat/Chat.h"
#include "Chat/Channel.h"
#include "Chat/ChannelMgr.h"
#include "PlayerBotMgr.h"
#include "Config/Config.h"
#include "Log.h"
#include "Util.h"

INSTANTIATE_SINGLETON_1(BotChatManager);

// ============================================================================
// Trade chat message pools — bots advertise services and goods.
// ============================================================================

static const char* TRADE_MESSAGES_ENCHANTER[] = {
    "WTS enchants, your mats + tip. [Crusader] [+30 SP] [+55 Healing] and more. PST",
    "Enchanting services - all enchants available. Your mats, tips appreciated",
    "WTS [Enchant Weapon - Crusader] your mats, 5g tip. PST",
    "WTS enchants! +15 agi gloves, +4 stats chest, Crusader, and more. PST",
    "Enchanter LF work, have all recipes. Mats + tip",
};
static const uint32 TRADE_ENCHANTER_COUNT = sizeof(TRADE_MESSAGES_ENCHANTER) / sizeof(TRADE_MESSAGES_ENCHANTER[0]);

static const char* TRADE_MESSAGES_ALCHEMIST[] = {
    "WTS Arcanite transmute, CD up. 3g PST",
    "Alchemist here, transmute CD available. 3g. PST",
    "WTS transmute - Arcanite bar, CD ready. PST",
};
static const uint32 TRADE_ALCHEMIST_COUNT = sizeof(TRADE_MESSAGES_ALCHEMIST) / sizeof(TRADE_MESSAGES_ALCHEMIST[0]);

static const char* TRADE_MESSAGES_CRAFTER[] = {
    "WTS Mooncloth CD, your mats + 3g tip. PST",
    "LF Mooncloth CD, have mats + tip",
    "WTS BS crafting - Arcanite Reaper, Annihilator, more. Your mats + tip",
    "Tailor LF work, have Mooncloth CD available",
};
static const uint32 TRADE_CRAFTER_COUNT = sizeof(TRADE_MESSAGES_CRAFTER) / sizeof(TRADE_MESSAGES_CRAFTER[0]);

static const char* TRADE_MESSAGES_GENERAL[] = {
    "WTB [Black Lotus] paying well, PST",
    "WTS [Greater Fire Protection Potion] x5, 4g each. PST",
    "WTB [Arcane Crystal] paying 20g each, PST",
    "WTS [Elixir of the Mongoose] x10, 3g each PST",
    "WTS various herbs and pots, check AH or PST for deals",
    "WTB [Righteous Orb] paying 5g, PST",
    "WTS [Flask of the Titans] 50g PST",
    "LF enchanter with Crusader, have mats + tip",
    "WTB [Large Brilliant Shard] x5, paying 6g each",
    "WTS [Arcanite Bar] 30g, PST",
};
static const uint32 TRADE_GENERAL_COUNT = sizeof(TRADE_MESSAGES_GENERAL) / sizeof(TRADE_MESSAGES_GENERAL[0]);

// Low-level trade messages for lower level bots.
static const char* TRADE_MESSAGES_LOWLEVEL[] = {
    "WTS [Mithril Ore] stack, 2g50s PST",
    "WTB silk cloth, paying 80s per stack",
    "WTS leather and hides, check AH",
    "LF someone to run me through SM, will pay",
};
static const uint32 TRADE_LOWLEVEL_COUNT = sizeof(TRADE_MESSAGES_LOWLEVEL) / sizeof(TRADE_MESSAGES_LOWLEVEL[0]);

// ============================================================================
// Whisper response pools — keyword matched.
// ============================================================================

struct WhisperPattern
{
    const char* keyword;     // substring to match (case-insensitive)
    const char* responses[4];
    uint32 responseCount;
};

static const WhisperPattern WHISPER_PATTERNS[] = {
    { "hi",      { "hey", "hey there", "yo", "sup" }, 4 },
    { "hello",   { "hey!", "hello", "hi there", "hey what's up" }, 4 },
    { "hey",     { "hey", "what's up", "yo", "hey there" }, 4 },
    { "thanks",  { "np", "np!", "anytime", "no problem" }, 4 },
    { "thank",   { "np", "you're welcome", "sure thing", "anytime" }, 4 },
    { "ty",      { "np", "yw", "anytime", "np!" }, 4 },
    { "price",   { "check the AH, good deals up", "depends, what are you looking for?", "PST me what you need" }, 3 },
    { "enchant", { "what enchant do you need?", "I can do most enchants, what do you need?", "sure, your mats + tip?" }, 3 },
    { "crusader",{ "yeah I can do Crusader, 5g tip + your mats", "got Crusader, meet me in IF?" }, 2 },
    { "portal",  { "where do you need a port?", "sure, 1g for a portal", "which city?" }, 3 },
    { "transmute",{"CD is up, 3g. meet in IF?", "sure, got your mats?", "yeah transmute ready, 3g" }, 3 },
    { "inv",     { "sure, one sec", "yeah give me a sec", "omw" }, 3 },
    { "invite",  { "sure, one sec", "yeah give me a sec", "incoming" }, 3 },
    { "wtb",     { "check the AH, I put some stuff up", "how much you paying?", "PST me what you need" }, 3 },
    { "wts",     { "how much?", "what's the price?", "interested, how much?" }, 3 },
    { "help",    { "what do you need?", "sure, what's up?", "how can I help?" }, 3 },
    { "lfg",     { "what are you running?", "which dungeon?", "I might be down, what instance?" }, 3 },
    { "lol",     { "lol", "haha", "xD", ":P" }, 4 },
    { "gg",      { "gg", "gg!", "good stuff" }, 3 },
    { "bye",     { "cya", "later", "peace", "take it easy" }, 4 },
};
static const uint32 WHISPER_PATTERN_COUNT = sizeof(WHISPER_PATTERNS) / sizeof(WHISPER_PATTERNS[0]);

static const char* WHISPER_FALLBACK[] = {
    "?", "hmm?", "what's up?", "sorry, busy atm", "one sec",
};
static const uint32 WHISPER_FALLBACK_COUNT = sizeof(WHISPER_FALLBACK) / sizeof(WHISPER_FALLBACK[0]);

// ============================================================================
// Capital city zone IDs for detecting city bots.
// ============================================================================
static const uint32 CITY_ZONES[] = {
    1519,  // Stormwind
    1537,  // Ironforge
    1657,  // Darnassus
    1637,  // Orgrimmar
    1638,  // Thunder Bluff
    1497,  // Undercity
};
static const uint32 CITY_ZONE_COUNT = sizeof(CITY_ZONES) / sizeof(CITY_ZONES[0]);

static bool IsInCity(Player* player)
{
    uint32 zone = player->GetZoneId();
    for (uint32 i = 0; i < CITY_ZONE_COUNT; ++i)
        if (zone == CITY_ZONES[i])
            return true;
    return false;
}

// ============================================================================
// Implementation
// ============================================================================

BotChatManager::BotChatManager()
    : m_tradeChatTimer(0)
    , m_tradeChatIntervalMin(120000)  // 2 min
    , m_tradeChatIntervalMax(300000)  // 5 min
    , m_enabled(false)
    , m_loaded(false)
{
}

void BotChatManager::Load()
{
    m_enabled = sConfig.GetBoolDefault("BotChat.Enable", true);
    m_tradeChatIntervalMin = sConfig.GetIntDefault("BotChat.TradeChat.IntervalMin", 120) * 1000;
    m_tradeChatIntervalMax = sConfig.GetIntDefault("BotChat.TradeChat.IntervalMax", 300) * 1000;

    // Set initial timer with random delay so it doesn't fire immediately on startup.
    m_tradeChatTimer.Reset(urand(m_tradeChatIntervalMin, m_tradeChatIntervalMax));

    m_loaded = true;
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotChat: Loaded. Trade chat interval: %u-%us.",
        m_tradeChatIntervalMin / 1000, m_tradeChatIntervalMax / 1000);
}

void BotChatManager::Update(uint32 diff)
{
    if (!m_loaded || !m_enabled)
        return;

    m_tradeChatTimer.Update(diff);
    if (m_tradeChatTimer.Passed())
    {
        m_tradeChatTimer.Reset(urand(m_tradeChatIntervalMin, m_tradeChatIntervalMax));
        UpdateTradeChat();
    }
}

void BotChatManager::UpdateTradeChat()
{
    Player* bot = FindRandomCityBot();
    if (!bot)
        return;

    const char* message = PickTradeMessage(bot);
    if (!message)
        return;

    // Get the Trade channel for the bot's faction.
    ChannelMgr* mgr = channelMgr(bot->GetTeam());
    if (!mgr)
        return;

    Channel* tradeChannel = mgr->GetJoinChannel("Trade");
    if (!tradeChannel)
        return;

    // Send to trade chat using skipCheck to bypass membership requirement.
    tradeChannel->Say(bot->GetObjectGuid(), message, LANG_UNIVERSAL, /*skipCheck=*/true);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotChat: %s says in Trade: %s",
        bot->GetName(), message);
}

Player* BotChatManager::FindRandomCityBot() const
{
    // Collect all online bots that are in capital cities.
    std::vector<Player*> cityBots;

    // Bot accounts are >= 100 per CLAUDE.md convention.
    HashMapHolder<Player>::MapType& players = sObjectAccessor.GetPlayers();
    for (auto& pair : players)
    {
        Player* player = pair.second;
        if (!player || !player->IsInWorld() || !player->IsAlive())
            continue;

        if (!player->GetSession() || player->GetSession()->GetAccountId() < 100)
            continue;

        if (IsInCity(player))
            cityBots.push_back(player);
    }

    if (cityBots.empty())
        return nullptr;

    return cityBots[urand(0, static_cast<uint32>(cityBots.size()) - 1)];
}

const char* BotChatManager::PickTradeMessage(Player* bot) const
{
    uint32 level = bot->GetLevel();

    // Lower level bots use generic messages.
    if (level < 40)
        return TRADE_MESSAGES_LOWLEVEL[urand(0, TRADE_LOWLEVEL_COUNT - 1)];

    // Mage bots sometimes advertise portals specifically.
    if (bot->GetClass() == CLASS_MAGE && urand(1, 100) <= 50)
    {
        // Extern declarations for portal trade messages in BotServiceMgr.cpp
        extern const char* TRADE_MESSAGES_PORTALS[];
        extern const uint32 TRADE_PORTALS_COUNT;
        extern const char* TRADE_MESSAGES_PORTALS_HORDE[];
        extern const uint32 TRADE_PORTALS_HORDE_COUNT;

        if (bot->GetTeam() == ALLIANCE)
            return TRADE_MESSAGES_PORTALS[urand(0, TRADE_PORTALS_COUNT - 1)];
        else
            return TRADE_MESSAGES_PORTALS_HORDE[urand(0, TRADE_PORTALS_HORDE_COUNT - 1)];
    }

    // Level 60 bots use a weighted mix of message types.
    uint32 roll = urand(1, 100);
    if (roll <= 40)
        return TRADE_MESSAGES_GENERAL[urand(0, TRADE_GENERAL_COUNT - 1)];
    else if (roll <= 60)
        return TRADE_MESSAGES_ENCHANTER[urand(0, TRADE_ENCHANTER_COUNT - 1)];
    else if (roll <= 80)
        return TRADE_MESSAGES_ALCHEMIST[urand(0, TRADE_ALCHEMIST_COUNT - 1)];
    else
        return TRADE_MESSAGES_CRAFTER[urand(0, TRADE_CRAFTER_COUNT - 1)];
}

// ============================================================================
// Whisper handling
// ============================================================================

bool BotChatManager::HandleWhisper(Player* bot, ObjectGuid senderGuid, const std::string& message)
{
    if (!m_enabled || !bot)
        return false;

    // Route to service manager first (portal requests, enchants, etc.)
    if (sBotServiceMgr.HandleServiceWhisper(bot, senderGuid, message))
        return true;

    // Generic whisper response.
    const char* response = PickWhisperResponse(message);
    SendWhisperToPlayer(bot, senderGuid, response);
    return true;
}

const char* BotChatManager::PickWhisperResponse(const std::string& incomingMessage) const
{
    // Convert to lowercase for matching.
    std::string lower = incomingMessage;
    for (auto& c : lower)
        c = static_cast<char>(tolower(c));

    // Try keyword matching in order (first match wins).
    for (uint32 i = 0; i < WHISPER_PATTERN_COUNT; ++i)
    {
        if (lower.find(WHISPER_PATTERNS[i].keyword) != std::string::npos)
        {
            return WHISPER_PATTERNS[i].responses[
                urand(0, WHISPER_PATTERNS[i].responseCount - 1)];
        }
    }

    // Fallback.
    return WHISPER_FALLBACK[urand(0, WHISPER_FALLBACK_COUNT - 1)];
}

void BotChatManager::SendWhisperToPlayer(Player* bot, ObjectGuid targetGuid, const char* message)
{
    Player* target = sObjectAccessor.FindPlayer(targetGuid);
    if (!target || !target->GetSession())
        return;

    // Build whisper packet from bot to player.
    WorldPacket data;
    ChatHandler::BuildChatPacket(data, CHAT_MSG_WHISPER, message,
        LANG_UNIVERSAL, bot->GetChatTag(), bot->GetObjectGuid(), bot->GetName());
    target->GetSession()->SendPacket(&data);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotChat: %s whispers to %s: %s",
        bot->GetName(), target->GetName(), message);
}
