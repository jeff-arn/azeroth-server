#include "BotChatManager.h"
#include "BotServiceMgr.h"
#include "GuildBotMgr.h"
#include "Group.h"
#include "Opcodes.h"
#include "Policies/SingletonImp.h"
#include "Player.h"
#include "ObjectAccessor.h"
#include "ObjectMgr.h"
#include "World.h"
#include "Chat/Chat.h"
#include "Chat/Channel.h"
#include "Chat/ChannelMgr.h"
#include "Guild/Guild.h"
#include "Guild/GuildMgr.h"
#include "PlayerBotMgr.h"
#include "Config/Config.h"
#include "Log.h"
#include "Util.h"

INSTANTIATE_SINGLETON_1(BotChatManager);

// ============================================================================
// Personality helpers
// ============================================================================

BotPersonality BotPersonalityFromString(const std::string& str)
{
    if (str == "enthusiastic") return BOT_PERSONALITY_ENTHUSIASTIC;
    if (str == "chill")        return BOT_PERSONALITY_CHILL;
    if (str == "sarcastic")    return BOT_PERSONALITY_SARCASTIC;
    if (str == "quiet")        return BOT_PERSONALITY_QUIET;
    return BOT_PERSONALITY_GENERIC;
}

const char* BotPersonalityToString(BotPersonality p)
{
    switch (p)
    {
        case BOT_PERSONALITY_ENTHUSIASTIC: return "enthusiastic";
        case BOT_PERSONALITY_CHILL:        return "chill";
        case BOT_PERSONALITY_SARCASTIC:    return "sarcastic";
        case BOT_PERSONALITY_QUIET:        return "quiet";
        default:                           return "generic";
    }
}

// Helper: pick a message from a PersonalityMessage array, filtered by personality.
// Falls back to GENERIC messages if no personality-specific match exists.
static const char* PickPersonalityMessage(const PersonalityMessage* pool, uint32 count, BotPersonality personality)
{
    // Collect matching messages: exact personality match + generic fallbacks
    std::vector<const char*> matches;
    std::vector<const char*> generics;

    for (uint32 i = 0; i < count; ++i)
    {
        if (pool[i].personality == personality)
            matches.push_back(pool[i].text);
        else if (pool[i].personality == BOT_PERSONALITY_GENERIC)
            generics.push_back(pool[i].text);
    }

    // Prefer personality-specific messages (70% of the time if available)
    if (!matches.empty() && (generics.empty() || urand(1, 100) <= 70))
        return matches[urand(0, static_cast<uint32>(matches.size()) - 1)];

    if (!generics.empty())
        return generics[urand(0, static_cast<uint32>(generics.size()) - 1)];

    // Shouldn't happen if pools are well-formed, but fall back to first entry
    return (count > 0) ? pool[0].text : "";
}

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

bool BotChatManager::IsInCity(Player* player)
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
    , m_cooldownCleanupTimer(0)
    , m_cooldownTradeChat(300)
    , m_cooldownGuildChat(180)
    , m_cooldownPartyChat(30)
    , m_cooldownSayChat(120)
    , m_cooldownGeneralChat(120)
    , m_cooldownScaleThreshold(30)
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

    m_guildChatIntervalMin = sConfig.GetIntDefault("BotChat.GuildChat.IntervalMin", 300) * 1000;
    m_guildChatIntervalMax = sConfig.GetIntDefault("BotChat.GuildChat.IntervalMax", 900) * 1000;
    m_guildChatTimer.Reset(urand(m_guildChatIntervalMin, m_guildChatIntervalMax));

    // Spam control cooldowns (seconds)
    m_cooldownTradeChat      = sConfig.GetIntDefault("BotChat.Cooldown.TradeChat", 300);
    m_cooldownGuildChat      = sConfig.GetIntDefault("BotChat.Cooldown.GuildChat", 180);
    m_cooldownPartyChat      = sConfig.GetIntDefault("BotChat.Cooldown.PartyChat", 30);
    m_cooldownSayChat        = sConfig.GetIntDefault("BotChat.Cooldown.SayChat", 120);
    m_cooldownGeneralChat    = sConfig.GetIntDefault("BotChat.Cooldown.GeneralChat", 120);
    m_cooldownScaleThreshold = sConfig.GetIntDefault("BotChat.Cooldown.ScaleThreshold", 30);

    m_cooldownCleanupTimer.Reset(60000);  // cleanup every 60s

    m_loaded = true;
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotChat: Loaded. Trade chat interval: %u-%us, Guild chat interval: %u-%us.",
        m_tradeChatIntervalMin / 1000, m_tradeChatIntervalMax / 1000,
        m_guildChatIntervalMin / 1000, m_guildChatIntervalMax / 1000);
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

    m_guildChatTimer.Update(diff);
    if (m_guildChatTimer.Passed())
    {
        m_guildChatTimer.Reset(urand(m_guildChatIntervalMin, m_guildChatIntervalMax));
        UpdateGuildChat();
    }

    // Zone-population-driven General chat — runs every Update tick.
    // Uses actual diff to calculate probability correctly regardless of Update frequency.
    UpdateGeneralChat(diff);

    // Process pending delayed whispers
    UpdatePendingWhispers(diff);

    // Periodic cooldown cleanup
    m_cooldownCleanupTimer.Update(diff);
    if (m_cooldownCleanupTimer.Passed())
    {
        m_cooldownCleanupTimer.Reset(60000);
        CleanupCooldowns();
    }
}

void BotChatManager::UpdateTradeChat()
{
    Player* bot = FindRandomCityBot();
    if (!bot)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotChat] Trade chat tick: no city bots found.");
        return;
    }

    const char* message = PickTradeMessage(bot);
    if (!message)
        return;

    // Get the Trade channel for the bot's faction.
    ChannelMgr* mgr = channelMgr(bot->GetTeam());
    if (!mgr)
        return;

    Channel* tradeChannel = mgr->GetJoinChannel("Trade - City");
    if (!tradeChannel)
        return;

    // Send to trade chat using skipCheck to bypass membership requirement.
    tradeChannel->Say(bot->GetObjectGuid(), message, LANG_UNIVERSAL, /*skipCheck=*/true);

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotChat] %s says in Trade: %s",
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

    // Try WTB buy service (bot wants to buy items from player)
    if (sBotServiceMgr.HandleBuyWhisper(bot, senderGuid, message))
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

// ============================================================================
// Guild chat
// ============================================================================

static const PersonalityMessage GUILD_CHAT_IDLE[] = {
    // Generic — usable by any personality
    { "anyone up for a dungeon later?",                              BOT_PERSONALITY_GENERIC },
    { "just finished farming, got some good drops",                  BOT_PERSONALITY_GENERIC },
    { "how's everyone doing today",                                  BOT_PERSONALITY_GENERIC },
    { "anyone need help with anything?",                             BOT_PERSONALITY_GENERIC },
    { "been working on my pre-raid gear, getting close",             BOT_PERSONALITY_GENERIC },
    { "quiet day on the server",                                     BOT_PERSONALITY_GENERIC },
    { "anyone seen any good BoE drops lately?",                      BOT_PERSONALITY_GENERIC },
    { "I need to farm some gold tonight",                            BOT_PERSONALITY_GENERIC },
    { "thinking about running Strat later if anyone's interested",   BOT_PERSONALITY_GENERIC },
    { "finally got my last piece from BRD",                          BOT_PERSONALITY_GENERIC },
    { "who's available for raid this week?",                         BOT_PERSONALITY_GENERIC },
    { "what consumables should I bring for next raid?",              BOT_PERSONALITY_GENERIC },
    // Enthusiastic
    { "LET'S GO boys who's ready to run some dungeons!!",            BOT_PERSONALITY_ENTHUSIASTIC },
    { "just got an epic drop I'm so hyped right now!!!",             BOT_PERSONALITY_ENTHUSIASTIC },
    { "this guild is the best, love you guys!",                      BOT_PERSONALITY_ENTHUSIASTIC },
    { "I am SO ready for raid night, been farming consumes all week", BOT_PERSONALITY_ENTHUSIASTIC },
    { "grats grats grats to everyone on progression!!",              BOT_PERSONALITY_ENTHUSIASTIC },
    { "who wants to grind some rep?? I'm down for anything!",        BOT_PERSONALITY_ENTHUSIASTIC },
    // Chill
    { "sup",                                                         BOT_PERSONALITY_CHILL },
    { "yeah just chillin, farming some stuff",                       BOT_PERSONALITY_CHILL },
    { "anyone wanna run something idc what",                         BOT_PERSONALITY_CHILL },
    { "lol k",                                                       BOT_PERSONALITY_CHILL },
    { "just vibing in IF",                                           BOT_PERSONALITY_CHILL },
    { "might do a dungeon later, we'll see",                         BOT_PERSONALITY_CHILL },
    // Sarcastic
    { "oh good another day of farming thorium, living the dream",    BOT_PERSONALITY_SARCASTIC },
    { "can't wait to wipe on trash again",                           BOT_PERSONALITY_SARCASTIC },
    { "sure would be nice if something actually dropped for once",   BOT_PERSONALITY_SARCASTIC },
    { "another day another 50 silver repair bill",                   BOT_PERSONALITY_SARCASTIC },
    { "who needs sleep when you can farm fire resist gear",          BOT_PERSONALITY_SARCASTIC },
    { "I'm sure this UBRS run will be the one where dal'rends drop", BOT_PERSONALITY_SARCASTIC },
    // Quiet — uses generic pool but at reduced frequency (handled at callsite)
    { "hey",                                                         BOT_PERSONALITY_QUIET },
    { "o/",                                                          BOT_PERSONALITY_QUIET },
    { "mhm",                                                         BOT_PERSONALITY_QUIET },
};
static const uint32 GUILD_IDLE_COUNT = sizeof(GUILD_CHAT_IDLE) / sizeof(GUILD_CHAT_IDLE[0]);

void BotChatManager::UpdateGuildChat()
{
    if (!sGuildBotMgr.IsEnabled())
        return;

    Player* bot = FindRandomOnlineGuildBot();
    if (!bot)
        return;

    BotPersonality personality = GetPersonality(bot);

    // Quiet bots only speak 30% of the time
    if (personality == BOT_PERSONALITY_QUIET && urand(1, 100) > 30)
        return;

    // Check per-bot cooldown
    if (!CanChat(bot->GetGUIDLow(), CHAT_COOLDOWN_GUILD))
        return;

    const char* message = PickGuildChatMessage(personality);
    if (!message)
        return;

    RecordChat(bot->GetGUIDLow(), CHAT_COOLDOWN_GUILD);
    SendGuildChat(bot, message);
}

Player* BotChatManager::FindRandomOnlineGuildBot() const
{
    std::vector<Player*> onlineBots;

    auto const& roster = sGuildBotMgr.GetRoster();
    for (auto const& info : roster)
    {
        if (!info.online)
            continue;

        Player* p = sObjectAccessor.FindPlayer(ObjectGuid(HIGHGUID_PLAYER, info.guid));
        if (p && p->IsInWorld())
            onlineBots.push_back(p);
    }

    if (onlineBots.empty())
        return nullptr;

    return onlineBots[urand(0, static_cast<uint32>(onlineBots.size()) - 1)];
}

const char* BotChatManager::PickGuildChatMessage(BotPersonality personality) const
{
    return PickPersonalityMessage(GUILD_CHAT_IDLE, GUILD_IDLE_COUNT, personality);
}

void BotChatManager::SendGuildChat(Player* bot, const char* message)
{
    uint32 guildId = sGuildBotMgr.GetGuildId();
    if (!guildId)
        return;

    Guild* guild = sGuildMgr.GetGuildById(guildId);
    if (!guild)
        return;

    guild->BroadcastToGuild(bot->GetSession(), message, LANG_UNIVERSAL);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotChat: %s says in guild: %s",
        bot->GetName(), message);
}

// ============================================================================
// Personality
// ============================================================================

BotPersonality BotChatManager::GetPersonality(Player* bot) const
{
    if (!bot)
        return BOT_PERSONALITY_GENERIC;

    // Guild bots have personality stored in roster
    GuildBotInfo const* info = sGuildBotMgr.GetBotInfo(bot->GetGUIDLow());
    if (info)
        return info->personality;

    // All other bots (crowd, dynamic, party) use generic
    return BOT_PERSONALITY_GENERIC;
}

// ============================================================================
// Spam control
// ============================================================================

uint32 BotChatManager::GetCooldownForType(ChatCooldownType type) const
{
    switch (type)
    {
        case CHAT_COOLDOWN_TRADE:   return m_cooldownTradeChat;
        case CHAT_COOLDOWN_GUILD:   return m_cooldownGuildChat;
        case CHAT_COOLDOWN_PARTY:   return m_cooldownPartyChat;
        case CHAT_COOLDOWN_SAY:     return m_cooldownSayChat;
        case CHAT_COOLDOWN_GENERAL: return m_cooldownGeneralChat;
        default:                    return 60;
    }
}

float BotChatManager::GetFrequencyScale() const
{
    if (m_cooldownScaleThreshold == 0)
        return 1.0f;

    uint32 botCount = GetOnlineBotCount();
    if (botCount <= m_cooldownScaleThreshold)
        return 1.0f;

    return static_cast<float>(botCount) / static_cast<float>(m_cooldownScaleThreshold);
}

uint32 BotChatManager::GetOnlineBotCount() const
{
    uint32 count = 0;
    HashMapHolder<Player>::MapType const& players = sObjectAccessor.GetPlayers();
    for (auto const& pair : players)
    {
        Player* player = pair.second;
        if (player && player->GetSession() && player->GetSession()->GetAccountId() >= 100)
            ++count;
    }
    return count;
}

bool BotChatManager::CanChat(uint32 botGuid, ChatCooldownType type) const
{
    auto it = m_cooldowns.find(botGuid);
    if (it == m_cooldowns.end())
        return true;

    time_t now = time(nullptr);
    time_t lastChat = it->second.lastChat[type];
    if (lastChat == 0)
        return true;

    uint32 baseCooldown = GetCooldownForType(type);
    float scale = GetFrequencyScale();
    uint32 effectiveCooldown = static_cast<uint32>(baseCooldown * scale);

    return (now - lastChat) >= effectiveCooldown;
}

void BotChatManager::RecordChat(uint32 botGuid, ChatCooldownType type)
{
    m_cooldowns[botGuid].lastChat[type] = time(nullptr);
}

void BotChatManager::CleanupCooldowns()
{
    time_t now = time(nullptr);
    auto it = m_cooldowns.begin();
    while (it != m_cooldowns.end())
    {
        // Remove entries where all timestamps are older than 10 minutes
        bool stale = true;
        for (uint8 i = 0; i < CHAT_COOLDOWN_MAX; ++i)
        {
            if (it->second.lastChat[i] != 0 && (now - it->second.lastChat[i]) < 600)
            {
                stale = false;
                break;
            }
        }

        if (stale)
            it = m_cooldowns.erase(it);
        else
            ++it;
    }
}

// ============================================================================
// Party/Raid Chat + Emotes
// ============================================================================

static const PersonalityMessage PARTY_GREET[] = {
    { "hey",                     BOT_PERSONALITY_GENERIC },
    { "o/",                      BOT_PERSONALITY_GENERIC },
    { "sup",                     BOT_PERSONALITY_GENERIC },
    { "hey all",                 BOT_PERSONALITY_GENERIC },
    { "ready when you are",      BOT_PERSONALITY_GENERIC },
    { "lets gooo!!",             BOT_PERSONALITY_ENTHUSIASTIC },
    { "hype! ready to go!",      BOT_PERSONALITY_ENTHUSIASTIC },
    { "yo",                      BOT_PERSONALITY_CHILL },
    { "heya",                    BOT_PERSONALITY_CHILL },
    { "oh good, more people",    BOT_PERSONALITY_SARCASTIC },
    { "hi",                      BOT_PERSONALITY_QUIET },
};
static const uint32 PARTY_GREET_COUNT = sizeof(PARTY_GREET) / sizeof(PARTY_GREET[0]);

static const PersonalityMessage PARTY_GRATZ[] = {
    { "grats!",                  BOT_PERSONALITY_GENERIC },
    { "gz",                      BOT_PERSONALITY_GENERIC },
    { "nice!",                   BOT_PERSONALITY_GENERIC },
    { "grats dude",              BOT_PERSONALITY_GENERIC },
    { "GRATS!!!",                BOT_PERSONALITY_ENTHUSIASTIC },
    { "woooo grats!!",           BOT_PERSONALITY_ENTHUSIASTIC },
    { "gz",                      BOT_PERSONALITY_CHILL },
    { "nice gz",                 BOT_PERSONALITY_CHILL },
    { "only took forever",       BOT_PERSONALITY_SARCASTIC },
    { "gz",                      BOT_PERSONALITY_QUIET },
};
static const uint32 PARTY_GRATZ_COUNT = sizeof(PARTY_GRATZ) / sizeof(PARTY_GRATZ[0]);

static const PersonalityMessage PARTY_BOSS_KILL[] = {
    { "nice kill",               BOT_PERSONALITY_GENERIC },
    { "gg",                      BOT_PERSONALITY_GENERIC },
    { "good fight",              BOT_PERSONALITY_GENERIC },
    { "down!",                   BOT_PERSONALITY_GENERIC },
    { "LETS GOOO!! GG!!!",       BOT_PERSONALITY_ENTHUSIASTIC },
    { "YESSS nice kill!!",       BOT_PERSONALITY_ENTHUSIASTIC },
    { "ez",                      BOT_PERSONALITY_CHILL },
    { "gg",                      BOT_PERSONALITY_CHILL },
    { "finally lol",             BOT_PERSONALITY_SARCASTIC },
    { "wow that only took 3 tries", BOT_PERSONALITY_SARCASTIC },
    { "gg",                      BOT_PERSONALITY_QUIET },
};
static const uint32 PARTY_BOSS_KILL_COUNT = sizeof(PARTY_BOSS_KILL) / sizeof(PARTY_BOSS_KILL[0]);

static const PersonalityMessage PARTY_WIPE[] = {
    { "oof",                     BOT_PERSONALITY_GENERIC },
    { "rip",                     BOT_PERSONALITY_GENERIC },
    { "that was rough",          BOT_PERSONALITY_GENERIC },
    { "we'll get it next time!", BOT_PERSONALITY_ENTHUSIASTIC },
    { "again! we got this!",     BOT_PERSONALITY_ENTHUSIASTIC },
    { "F",                       BOT_PERSONALITY_CHILL },
    { "rip",                     BOT_PERSONALITY_CHILL },
    { "shocking",                BOT_PERSONALITY_SARCASTIC },
    { "well that was fun",       BOT_PERSONALITY_SARCASTIC },
    { "who could have possibly seen that coming", BOT_PERSONALITY_SARCASTIC },
};
static const uint32 PARTY_WIPE_COUNT = sizeof(PARTY_WIPE) / sizeof(PARTY_WIPE[0]);

static const char* PARTY_OOM[] = {
    "oom",
    "oom need mana",
    "low mana",
    "need to drink",
};
static const uint32 PARTY_OOM_COUNT = sizeof(PARTY_OOM) / sizeof(PARTY_OOM[0]);

static const char* PARTY_PULLING[] = {
    "pulling",
    "inc",
    "going in",
};
static const uint32 PARTY_PULLING_COUNT = sizeof(PARTY_PULLING) / sizeof(PARTY_PULLING[0]);

void BotChatManager::SendPartyChat(Player* bot, const char* message)
{
    if (!bot || !message)
        return;

    Group* group = bot->GetGroup();
    if (!group)
        return;

    WorldPacket data;
    ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, message,
        LANG_UNIVERSAL, bot->GetChatTag(), bot->GetObjectGuid(), bot->GetName());
    group->BroadcastPacket(&data, true);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotChat: %s says in party: %s",
        bot->GetName(), message);
}

void BotChatManager::SendPartyEmote(Player* bot, uint32 emoteId)
{
    if (!bot)
        return;
    bot->HandleEmote(emoteId);
}

void BotChatManager::OnGroupMemberJoined(Player* bot, Group* group)
{
    if (!m_enabled || !bot || !group)
        return;

    // Only bots greet
    if (!bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return;

    BotPersonality personality = GetPersonality(bot);
    if (personality == BOT_PERSONALITY_QUIET && urand(1, 100) > 30)
        return;

    if (!CanChat(bot->GetGUIDLow(), CHAT_COOLDOWN_PARTY))
        return;

    const char* msg = PickPersonalityMessage(PARTY_GREET, PARTY_GREET_COUNT, personality);
    RecordChat(bot->GetGUIDLow(), CHAT_COOLDOWN_PARTY);
    SendPartyChat(bot, msg);
    SendPartyEmote(bot, 3);  // EMOTE_ONESHOT_WAVE
}

void BotChatManager::OnGroupMemberLevelUp(Player* leveledPlayer)
{
    if (!m_enabled || !leveledPlayer)
        return;

    Group* group = leveledPlayer->GetGroup();
    if (!group)
        return;

    // Pick 1-2 random bot group members to say gratz
    std::vector<Player*> bots;
    for (GroupReference* itr = group->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        Player* member = itr->getSource();
        if (!member || member == leveledPlayer)
            continue;
        if (!member->GetSession() || member->GetSession()->GetAccountId() < 100)
            continue;
        if (!member->IsInWorld() || member->IsDead())
            continue;
        if (CanChat(member->GetGUIDLow(), CHAT_COOLDOWN_PARTY))
            bots.push_back(member);
    }

    if (bots.empty())
        return;

    // Shuffle and pick 1-2
    uint32 count = std::min(static_cast<uint32>(bots.size()), urand(1, 2));
    for (uint32 i = 0; i < count; ++i)
    {
        uint32 idx = urand(0, static_cast<uint32>(bots.size()) - 1);
        Player* bot = bots[idx];
        BotPersonality personality = GetPersonality(bot);

        if (personality == BOT_PERSONALITY_QUIET && urand(1, 100) > 30)
            continue;

        const char* msg = PickPersonalityMessage(PARTY_GRATZ, PARTY_GRATZ_COUNT, personality);
        RecordChat(bot->GetGUIDLow(), CHAT_COOLDOWN_PARTY);
        SendPartyChat(bot, msg);

        // Remove from pool to avoid duplicate
        bots.erase(bots.begin() + idx);
        if (bots.empty())
            break;
    }
}

void BotChatManager::OnBossKilled(Group* group, uint32 /*creatureEntry*/)
{
    if (!m_enabled || !group)
        return;

    // 1-3 random bots react
    std::vector<Player*> bots;
    for (GroupReference* itr = group->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        Player* member = itr->getSource();
        if (!member || !member->GetSession() || member->GetSession()->GetAccountId() < 100)
            continue;
        if (!member->IsInWorld() || member->IsDead())
            continue;
        if (CanChat(member->GetGUIDLow(), CHAT_COOLDOWN_PARTY))
            bots.push_back(member);
    }

    uint32 count = std::min(static_cast<uint32>(bots.size()), urand(1, 3));
    for (uint32 i = 0; i < count && !bots.empty(); ++i)
    {
        uint32 idx = urand(0, static_cast<uint32>(bots.size()) - 1);
        Player* bot = bots[idx];
        BotPersonality personality = GetPersonality(bot);

        const char* msg = PickPersonalityMessage(PARTY_BOSS_KILL, PARTY_BOSS_KILL_COUNT, personality);
        RecordChat(bot->GetGUIDLow(), CHAT_COOLDOWN_PARTY);
        SendPartyChat(bot, msg);
        SendPartyEmote(bot, 4);  // EMOTE_ONESHOT_CHEER

        bots.erase(bots.begin() + idx);
    }
}

void BotChatManager::OnPartyWipe(Group* group)
{
    if (!m_enabled || !group)
        return;

    // 1-2 bots react to wipe
    std::vector<Player*> bots;
    for (GroupReference* itr = group->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        Player* member = itr->getSource();
        if (!member || !member->GetSession() || member->GetSession()->GetAccountId() < 100)
            continue;
        if (!member->IsInWorld())
            continue;
        bots.push_back(member);
    }

    uint32 count = std::min(static_cast<uint32>(bots.size()), urand(1, 2));
    for (uint32 i = 0; i < count && !bots.empty(); ++i)
    {
        uint32 idx = urand(0, static_cast<uint32>(bots.size()) - 1);
        Player* bot = bots[idx];
        BotPersonality personality = GetPersonality(bot);

        const char* msg = PickPersonalityMessage(PARTY_WIPE, PARTY_WIPE_COUNT, personality);
        SendPartyChat(bot, msg);
        SendPartyEmote(bot, 18);  // EMOTE_ONESHOT_CRY

        bots.erase(bots.begin() + idx);
    }
}

// ============================================================================
// Trade chat monitoring (Phase 3 stub)
// ============================================================================

// Trade chat trigger keywords that map to bot services
struct TradeChatTrigger
{
    const char* keyword;
    BotServiceType serviceType;
    uint8 requiredClass;  // 0 = any, CLASS_MAGE, etc.
};

static const TradeChatTrigger TRADE_TRIGGERS[] = {
    { "wtb port",     BOT_SERVICE_PORTAL,    CLASS_MAGE },
    { "lf port",      BOT_SERVICE_PORTAL,    CLASS_MAGE },
    { "need port",    BOT_SERVICE_PORTAL,    CLASS_MAGE },
    { "lfm port",     BOT_SERVICE_PORTAL,    CLASS_MAGE },
    { "wtb enchant",  BOT_SERVICE_ENCHANT,   0 },
    { "lf enchant",   BOT_SERVICE_ENCHANT,   0 },
    { "need enchant", BOT_SERVICE_ENCHANT,   0 },
    { "lf chanter",   BOT_SERVICE_ENCHANT,   0 },
    { "wtb transmute",BOT_SERVICE_TRANSMUTE, 0 },
    { "lf alch",      BOT_SERVICE_TRANSMUTE, 0 },
    { "wtb mooncloth",BOT_SERVICE_MOONCLOTH, 0 },
    { "lf tailor",    BOT_SERVICE_MOONCLOTH, 0 },
};
static const uint32 TRADE_TRIGGER_COUNT = sizeof(TRADE_TRIGGERS) / sizeof(TRADE_TRIGGERS[0]);

void BotChatManager::OnTradeChatMessage(ObjectGuid senderGuid, const std::string& message)
{
    if (!m_enabled)
        return;

    // Only react to human player messages
    Player* sender = sObjectAccessor.FindPlayer(senderGuid);
    if (!sender || !sender->GetSession() || sender->GetSession()->GetAccountId() >= 100)
        return;

    // Don't create duplicate requests — one bot response per player at a time
    if (sBotServiceMgr.HasActiveRequest(senderGuid))
        return;

    // Lowercase for matching
    std::string lower = message;
    for (auto& c : lower)
        c = static_cast<char>(tolower(c));

    // Check for service request keywords
    for (uint32 i = 0; i < TRADE_TRIGGER_COUNT; ++i)
    {
        if (lower.find(TRADE_TRIGGERS[i].keyword) == std::string::npos)
            continue;

        // Find a suitable city bot in the same zone as the player
        Player* bot = nullptr;
        std::vector<Player*> candidates;
        uint32 playerZone = sender->GetZoneId();

        HashMapHolder<Player>::MapType& players = sObjectAccessor.GetPlayers();
        for (auto& pair : players)
        {
            Player* p = pair.second;
            if (!p || !p->IsInWorld() || !p->IsAlive())
                continue;
            if (!p->GetSession() || p->GetSession()->GetAccountId() < 100)
                continue;
            if (p->GetZoneId() != playerZone)
                continue;

            // Class/profession check
            if (TRADE_TRIGGERS[i].requiredClass != 0 &&
                p->GetClass() != TRADE_TRIGGERS[i].requiredClass)
                continue;
            if (TRADE_TRIGGERS[i].serviceType == BOT_SERVICE_ENCHANT &&
                !BotServiceMgr::IsEnchanterBot(p))
                continue;
            if (TRADE_TRIGGERS[i].serviceType == BOT_SERVICE_TRANSMUTE &&
                !BotServiceMgr::IsAlchemistBot(p))
                continue;
            if (TRADE_TRIGGERS[i].serviceType == BOT_SERVICE_MOONCLOTH &&
                !BotServiceMgr::IsTailorBot(p))
                continue;

            if (CanChat(p->GetGUIDLow(), CHAT_COOLDOWN_TRADE))
                candidates.push_back(p);
        }

        if (candidates.empty())
            return;

        bot = candidates[urand(0, static_cast<uint32>(candidates.size()) - 1)];

        // Build a service-specific proactive whisper and create the service request
        PendingWhisper pw;
        pw.botGuid = bot->GetObjectGuid();
        pw.playerGuid = senderGuid;
        pw.delayMs = urand(5000, 20000);

        // Build location string from bot's zone
        std::string location = "in town";
        {
            AreaEntry const* area = sAreaStorage.LookupEntry<AreaEntry>(bot->GetZoneId());
            if (area && area->Name)
                location = "at " + std::string(area->Name);
        }

        switch (TRADE_TRIGGERS[i].serviceType)
        {
            case BOT_SERVICE_PORTAL:
            {
                // Try to resolve destination from the trade chat message
                sBotServiceMgr.CreateProactiveRequest(bot, senderGuid, BOT_SERVICE_PORTAL, message);
                BotServiceRequest* req = sBotServiceMgr.GetPendingRequest(bot->GetObjectGuid(), senderGuid);
                if (req && req->state == SERVICE_STATE_WAITING_FOR_TRADE)
                {
                    // Destination was parsed from trade message — go straight to "trade me"
                    char buf[256];
                    snprintf(buf, sizeof(buf), "yeah I can port you, 1g. I'm %s, trade me", location.c_str());
                    pw.message = buf;
                }
                else
                {
                    char buf[256];
                    snprintf(buf, sizeof(buf), "hey I can do ports, where do you need? 1g. I'm %s", location.c_str());
                    pw.message = buf;
                }
                break;
            }
            case BOT_SERVICE_ENCHANT:
            {
                sBotServiceMgr.CreateProactiveRequest(bot, senderGuid, BOT_SERVICE_ENCHANT, message);
                BotServiceRequest* req = sBotServiceMgr.GetPendingRequest(bot->GetObjectGuid(), senderGuid);
                if (req && req->state == SERVICE_STATE_WAITING_FOR_TRADE)
                {
                    char buf[256];
                    snprintf(buf, sizeof(buf), "yeah I can do that, your mats + tip. I'm %s, trade me", location.c_str());
                    pw.message = buf;
                }
                else
                {
                    char buf[256];
                    snprintf(buf, sizeof(buf), "hey I can enchant, what do you need? I'm %s", location.c_str());
                    pw.message = buf;
                }
                break;
            }
            case BOT_SERVICE_TRANSMUTE:
            {
                char buf[256];
                snprintf(buf, sizeof(buf), "transmute CD is up, 3g. I'm %s, trade me", location.c_str());
                pw.message = buf;
                sBotServiceMgr.CreateProactiveRequest(bot, senderGuid, BOT_SERVICE_TRANSMUTE, message);
                break;
            }
            case BOT_SERVICE_MOONCLOTH:
            {
                char buf[256];
                snprintf(buf, sizeof(buf), "mooncloth CD is up, 3g. I'm %s, trade me", location.c_str());
                pw.message = buf;
                sBotServiceMgr.CreateProactiveRequest(bot, senderGuid, BOT_SERVICE_MOONCLOTH, message);
                break;
            }
            default:
                pw.message = "hey I can help with that, PST me";
                break;
        }

        m_pendingWhispers.push_back(pw);
        RecordChat(bot->GetGUIDLow(), CHAT_COOLDOWN_TRADE);

        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL,
            "BotChat: %s will whisper %s about trade request (delay %ums)",
            bot->GetName(), sender->GetName(), pw.delayMs);
        return;  // only one bot responds per message
    }
}

void BotChatManager::SendMinimapPing(Player* bot, Player* target)
{
    if (!bot || !target || !target->GetSession())
        return;

    WorldPacket data(SMSG_GOSSIP_POI, (4 + 4 + 4 + 4 + 4 + 40));
    data << uint32(99);    // Flags (standard POI flags)
    data << float(bot->GetPositionX());
    data << float(bot->GetPositionY());
    data << uint32(6);     // Icon: red flag marker
    data << uint32(0);     // Data
    data << bot->GetName();
    target->GetSession()->SendPacket(&data);
}

void BotChatManager::UpdatePendingWhispers(uint32 diff)
{
    if (m_pendingWhispers.empty())
        return;

    auto it = m_pendingWhispers.begin();
    while (it != m_pendingWhispers.end())
    {
        if (it->delayMs <= diff)
        {
            Player* bot = sObjectAccessor.FindPlayer(it->botGuid);
            Player* target = sObjectAccessor.FindPlayer(it->playerGuid);
            if (bot)
            {
                SendWhisperToPlayer(bot, it->playerGuid, it->message.c_str());
                // Send minimap marker so the player can find the bot
                if (target)
                    SendMinimapPing(bot, target);
            }
            it = m_pendingWhispers.erase(it);
        }
        else
        {
            it->delayMs -= diff;
            ++it;
        }
    }
}

// ============================================================================
// General channel chat — zone-population driven
// Target: ~4-5 msgs/min in IF at peak (~50 bots).
// Rate per zone = botCount * GENERAL_MSGS_PER_MIN_PER_BOT.
// Each 5s tick rolls probability per zone: botCount * 0.09 * (5/60) = botCount * 0.0075.
// ============================================================================

static const char* GENERAL_IDLE_MESSAGES[] = {
    "anyone know where the enchanting trainer is?",
    "brb bio",
    "lfg BRD",
    "lfm UBRS need tank and heals",
    "I need to farm more gold",
    "heading to IF anyone need anything?",
    "who's got the cheapest ports",
    "finally done with that quest",
    "so bored",
    "anyone selling flask of titans?",
    "LF healer Strat live pst",
    "just hit 60 what do I do now",
    "anybody running DM tonight?",
    "server seems pretty busy today",
    "how much is arcanite bar going for?",
    "any guilds recruiting?",
    "where do I get the ony attune quest",
    "LF tank BRD full run",
    "lfm Scholo need dps",
    "WTB runecloth 3g a stack",
    "anyone wanna do LBRS?",
    "what level can I start BRD",
    "need 1 more for strat UD",
    "did they nerf DM:E lasher farming?",
    "how much hit do warriors need for raids",
    "lfm DM:W tribute run, need lock",
};
static const uint32 GENERAL_IDLE_COUNT = sizeof(GENERAL_IDLE_MESSAGES) / sizeof(GENERAL_IDLE_MESSAGES[0]);

void BotChatManager::UpdateGeneralChat(uint32 diff)
{
    // Group online crowd bots by city zone
    std::unordered_map<uint32, std::vector<Player*>> zoneBots;

    HashMapHolder<Player>::MapType& players = sObjectAccessor.GetPlayers();
    for (auto& pair : players)
    {
        Player* p = pair.second;
        if (!p || !p->IsInWorld() || !p->IsAlive())
            continue;
        if (!p->GetSession() || p->GetSession()->GetAccountId() < 100)
            continue;
        if (!IsInCity(p))
            continue;
        zoneBots[p->GetZoneId()].push_back(p);
    }

    if (zoneBots.empty())
        return;

    // For each city zone, roll probability based on bot count and actual elapsed time.
    // Probability = botCount * MSGS_PER_MIN_PER_BOT * (diff_ms / 60000)
    float tickFraction = static_cast<float>(diff) / 60000.0f;

    for (auto& kv : zoneBots)
    {
        uint32 botCount = static_cast<uint32>(kv.second.size());
        float expectedMsgs = botCount * GENERAL_MSGS_PER_MIN_PER_BOT * tickFraction;

        // Determine how many messages to send this tick.
        // Integer part fires guaranteed, fractional part is a probability roll.
        uint32 msgsToSend = static_cast<uint32>(expectedMsgs);
        float fractional = expectedMsgs - msgsToSend;
        if (urand(0, 999) < static_cast<uint32>(fractional * 1000.0f))
            ++msgsToSend;

        if (msgsToSend == 0)
            continue;

        std::vector<Player*>& bots = kv.second;
        for (uint32 m = 0; m < msgsToSend; ++m)
        {
            // Pick a random bot in this zone that isn't on cooldown
            uint32 startIdx = urand(0, static_cast<uint32>(bots.size()) - 1);
            Player* speaker = nullptr;
            for (uint32 i = 0; i < bots.size(); ++i)
            {
                uint32 idx = (startIdx + i) % bots.size();
                if (CanChat(bots[idx]->GetGUIDLow(), CHAT_COOLDOWN_GENERAL))
                {
                    speaker = bots[idx];
                    break;
                }
            }

            if (!speaker)
                break;  // no available speakers left in this zone

            const char* msg = GENERAL_IDLE_MESSAGES[urand(0, GENERAL_IDLE_COUNT - 1)];
            RecordChat(speaker->GetGUIDLow(), CHAT_COOLDOWN_GENERAL);
            SendGeneralChat(speaker, msg);
        }
    }
}

void BotChatManager::SendGeneralChat(Player* bot, const char* message)
{
    if (!bot || !message)
        return;

    // Build zone-specific channel name: "General - Ironforge", "General - Stormwind City", etc.
    uint32 zoneId = bot->GetZoneId();
    AreaEntry const* area = sAreaStorage.LookupEntry<AreaEntry>(zoneId);
    if (!area)
        return;

    std::string channelName = "General - " + std::string(area->Name);

    ChannelMgr* mgr = channelMgr(bot->GetTeam());
    if (!mgr)
        return;

    Channel* generalChannel = mgr->GetJoinChannel(channelName.c_str());
    if (!generalChannel)
        return;

    generalChannel->Say(bot->GetObjectGuid(), message, LANG_UNIVERSAL, /*skipCheck=*/true);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotChat: %s says in General (%s): %s",
        bot->GetName(), area->Name, message);
}
