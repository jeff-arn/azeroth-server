#ifndef _BOT_CHAT_MANAGER_H
#define _BOT_CHAT_MANAGER_H

#include "Policies/Singleton.h"
#include "SharedDefines.h"
#include "Timer.h"
#include "ObjectGuid.h"

#include <vector>
#include <string>
#include <ctime>

class Player;

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

private:
    // Trade chat advertising
    void UpdateTradeChat();
    Player* FindRandomCityBot() const;
    const char* PickTradeMessage(Player* bot) const;

    // Whisper responses
    const char* PickWhisperResponse(const std::string& incomingMessage) const;
    void SendWhisperToPlayer(Player* bot, ObjectGuid targetGuid, const char* message);

    // Timers
    ShortTimeTracker m_tradeChatTimer;
    uint32 m_tradeChatIntervalMin;  // ms
    uint32 m_tradeChatIntervalMax;  // ms

    bool m_enabled;
    bool m_loaded;
};

#define sBotChatMgr MaNGOS::Singleton<BotChatManager>::Instance()

#endif
