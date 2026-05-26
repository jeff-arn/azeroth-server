#ifndef _BOT_DIRECTOR_H
#define _BOT_DIRECTOR_H

#include "Common.h"
#include "Policies/Singleton.h"
#include "Timer.h"

#include <unordered_map>
#include <vector>

enum BotDirectorZoneType : uint8
{
    ZONE_TYPE_CAPITAL = 0,
    ZONE_TYPE_FRIENDLY = 1,
    ZONE_TYPE_CONTESTED = 2,
    ZONE_TYPE_UNKNOWN = 3,
};

struct BotDirectorZoneInfo
{
    uint32 zoneId;
    const char* name;
    uint32 mapId;
    float x, y, z;
    uint8 levelMin;
    uint8 levelMax;
    BotDirectorZoneType type;
    // Capital cities store faction; friendly/contested don't need it
    const char* faction; // "alliance" or "horde" (capitals only)
};

struct BotAssignment
{
    uint32 guid;
    uint32 targetMap;
    float targetX, targetY, targetZ;
    uint32 targetZone;
    float radius;
    bool aggressive;
    uint32 targetLevel;
    uint32 faction; // 469=Alliance, 67=Horde
};

class BotDirector
{
public:
    BotDirector();

    void Load();
    void Update(uint32 diff);

    // Called by DynamicBotAI to get its assignment (returns nullptr if none)
    BotAssignment const* GetAssignment(uint32 guid) const;

    bool IsEnabled() const { return m_enabled; }

private:
    void RunDirectorCycle();
    BotDirectorZoneType ClassifyZone(uint32 zoneId, BotDirectorZoneInfo const** outInfo) const;

    // Assignment storage: guid -> assignment
    std::unordered_map<uint32, BotAssignment> m_assignments;

    // Chance updates
    void UpdateBotChances() const;

    bool m_enabled;
    uint32 m_updateInterval; // ms
    ShortTimeTracker m_timer;
};

#define sBotDirector MaNGOS::Singleton<BotDirector>::Instance()

#endif
