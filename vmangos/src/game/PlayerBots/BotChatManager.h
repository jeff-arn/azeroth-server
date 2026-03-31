#ifndef _BOT_CHAT_MANAGER_H
#define _BOT_CHAT_MANAGER_H

#include "Policies/Singleton.h"
#include "SharedDefines.h"
#include "Timer.h"
#include "ObjectGuid.h"

#include <vector>
#include <string>
#include <ctime>
#include <unordered_map>

class Player;
class Group;

// ============================================================================
// Personality system — affects chat style and frequency.
// ============================================================================

enum BotPersonality : uint8
{
    BOT_PERSONALITY_GENERIC      = 0,  // world/city bots — no personality filter
    BOT_PERSONALITY_ENTHUSIASTIC = 1,  // "nice!!", "let's goooo", exclamation marks
    BOT_PERSONALITY_CHILL        = 2,  // "yeah sure", "nah", "lol k"
    BOT_PERSONALITY_SARCASTIC    = 3,  // "oh great another wipe", dry humor
    BOT_PERSONALITY_QUIET        = 4,  // speaks rarely (~30% normal rate), short messages
};

BotPersonality BotPersonalityFromString(const std::string& str);
const char* BotPersonalityToString(BotPersonality p);

struct PersonalityMessage
{
    const char* text;
    BotPersonality personality;  // BOT_PERSONALITY_GENERIC = usable by all
};

// ============================================================================
// Spam control — per-bot cooldowns by chat type.
// ============================================================================

enum ChatCooldownType : uint8
{
    CHAT_COOLDOWN_TRADE   = 0,
    CHAT_COOLDOWN_GUILD   = 1,
    CHAT_COOLDOWN_PARTY   = 2,
    CHAT_COOLDOWN_SAY     = 3,
    CHAT_COOLDOWN_GENERAL = 4,
    CHAT_COOLDOWN_MAX     = 5,
};

struct BotChatCooldown
{
    time_t lastChat[CHAT_COOLDOWN_MAX];
    BotChatCooldown() { memset(lastChat, 0, sizeof(lastChat)); }
};

// ============================================================================

struct BotChatMessage
{
    const char* text;
    uint8 minLevel;  // minimum bot level to use this message
};

class BotChatManager
{
public:
    BotChatManager();

    void Load();
    void Update(uint32 diff);

    // Called when a player whispers a bot. Returns true if handled.
    bool HandleWhisper(Player* bot, ObjectGuid senderGuid, const std::string& message);

    // Personality
    BotPersonality GetPersonality(Player* bot) const;

    // Spam control
    bool CanChat(uint32 botGuid, ChatCooldownType type) const;
    void RecordChat(uint32 botGuid, ChatCooldownType type);

    // Party/raid chat (Phase 2)
    void SendPartyChat(Player* bot, const char* message);
    void SendPartyEmote(Player* bot, uint32 emoteId);
    void OnGroupMemberJoined(Player* bot, Group* group);
    void OnGroupMemberLevelUp(Player* leveledPlayer);
    void OnBossKilled(Group* group, uint32 creatureEntry);
    void OnPartyWipe(Group* group);

    // Trade chat monitoring (Phase 3)
    void OnTradeChatMessage(ObjectGuid senderGuid, const std::string& message);

    // General channel for crowd bots (Phase 4)
    void SendGeneralChat(Player* bot, const char* message);

    // Whisper sending (used by BotServiceMgr too)
    void SendWhisperToPlayer(Player* bot, ObjectGuid targetGuid, const char* message);

    // City detection
    static bool IsInCity(Player* player);

private:
    // Trade chat advertising
    void UpdateTradeChat();
    Player* FindRandomCityBot() const;
    const char* PickTradeMessage(Player* bot) const;

    // Whisper responses
    const char* PickWhisperResponse(const std::string& incomingMessage) const;

    // Guild chat
    void UpdateGuildChat();
    Player* FindRandomOnlineGuildBot() const;
    const char* PickGuildChatMessage(BotPersonality personality) const;
    void SendGuildChat(Player* bot, const char* message);

    // Pending delayed whispers (Phase 3)
    struct PendingWhisper
    {
        ObjectGuid botGuid;
        ObjectGuid playerGuid;
        uint32 delayMs;
        std::string message;
    };
    std::vector<PendingWhisper> m_pendingWhispers;
    void UpdatePendingWhispers(uint32 diff);

    // Spam control internals
    void CleanupCooldowns();
    uint32 GetCooldownForType(ChatCooldownType type) const;
    float GetFrequencyScale() const;
    uint32 GetOnlineBotCount() const;

    std::unordered_map<uint32, BotChatCooldown> m_cooldowns;
    uint32 m_cooldownTradeChat;
    uint32 m_cooldownGuildChat;
    uint32 m_cooldownPartyChat;
    uint32 m_cooldownSayChat;
    uint32 m_cooldownGeneralChat;
    uint32 m_cooldownScaleThreshold;

    // Timers
    ShortTimeTracker m_tradeChatTimer;
    uint32 m_tradeChatIntervalMin;  // ms
    uint32 m_tradeChatIntervalMax;  // ms

    ShortTimeTracker m_guildChatTimer;
    uint32 m_guildChatIntervalMin;
    uint32 m_guildChatIntervalMax;

    ShortTimeTracker m_cooldownCleanupTimer;

    // Zone-population-driven General chat
    static constexpr float GENERAL_MSGS_PER_MIN_PER_BOT = 0.09f;  // tuned: 50 bots ≈ 4.5 msgs/min
    void UpdateGeneralChat(uint32 diff);

    bool m_enabled;
    bool m_loaded;
};

#define sBotChatMgr MaNGOS::Singleton<BotChatManager>::Instance()

#endif
