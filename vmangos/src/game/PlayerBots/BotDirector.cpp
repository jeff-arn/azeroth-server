#include "BotDirector.h"
#include "Config/Config.h"
#include "Database/DatabaseEnv.h"
#include "Log.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "PlayerBotMgr.h"
#include "Policies/SingletonImp.h"

#include <algorithm>
#include <random>

INSTANTIATE_SINGLETON_1(BotDirector);

// ============================================================================
// Zone data — ported from bot_director.py
// ============================================================================

static const BotDirectorZoneInfo CAPITAL_CITIES[] = {
    {1519, "Stormwind", 0, -8829.5f, 625.6f, 93.9f, 10, 60, ZONE_TYPE_CAPITAL, "alliance"},
    {1537, "Ironforge", 0, -4928.5f, -946.6f, 501.6f, 10, 60, ZONE_TYPE_CAPITAL, "alliance"},
};
static const size_t NUM_CAPITAL_CITIES = sizeof(CAPITAL_CITIES) / sizeof(CAPITAL_CITIES[0]);

static const BotDirectorZoneInfo FRIENDLY_ZONES[] = {
    {12, "Elwynn Forest", 0, -9449.0f, 64.0f, 56.0f, 1, 10, ZONE_TYPE_FRIENDLY, nullptr}, {40, "Westfall", 0, -10647.0f, 1112.0f, 35.0f, 10, 20, ZONE_TYPE_FRIENDLY, nullptr}, {38, "Loch Modan", 0, -5405.0f, -2894.0f, 341.0f, 10, 20, ZONE_TYPE_FRIENDLY, nullptr}, {44, "Redridge Mountains", 0, -9278.0f, -2250.0f, 68.0f, 15, 25, ZONE_TYPE_FRIENDLY, nullptr}, {148, "Darkshore", 1, 6463.0f, 683.0f, 9.0f, 10, 20, ZONE_TYPE_FRIENDLY, nullptr}, {10, "Duskwood", 0, -10531.0f, -1154.0f, 28.0f, 20, 30, ZONE_TYPE_FRIENDLY, nullptr}, {11, "Wetlands", 0, -3799.0f, -822.0f, 10.0f, 20, 30, ZONE_TYPE_FRIENDLY, nullptr},
};
static const size_t NUM_FRIENDLY_ZONES = sizeof(FRIENDLY_ZONES) / sizeof(FRIENDLY_ZONES[0]);

static const BotDirectorZoneInfo CONTESTED_ZONES[] = {
    {267, "Hillsbrad Foothills", 0, -40.7f, -957.0f, 56.3f, 20, 30, ZONE_TYPE_CONTESTED, nullptr},  {33, "Stranglethorn Vale", 0, -12412.8f, 171.2f, 3.4f, 30, 45, ZONE_TYPE_CONTESTED, nullptr},    {331, "Ashenvale", 1, 949.9f, -1435.7f, 62.7f, 18, 30, ZONE_TYPE_CONTESTED, nullptr},         {3, "Arathi Highlands", 0, -1263.0f, -2515.0f, 22.0f, 30, 40, ZONE_TYPE_CONTESTED, nullptr}, {4, "Badlands", 0, -6113.0f, -3205.0f, 255.0f, 35, 45, ZONE_TYPE_CONTESTED, nullptr}, {440, "Tanaris", 1, -7193.4f, -3793.5f, 9.8f, 40, 50, ZONE_TYPE_CONTESTED, nullptr},           {357, "Feralas", 1, -4808.0f, 1590.0f, 80.0f, 42, 50, ZONE_TYPE_CONTESTED, nullptr},  {361, "Felwood", 1, 5230.0f, -546.0f, 343.0f, 48, 55, ZONE_TYPE_CONTESTED, nullptr},
    {28, "Western Plaguelands", 0, 1741.0f, -1720.0f, 59.0f, 51, 58, ZONE_TYPE_CONTESTED, nullptr}, {139, "Eastern Plaguelands", 0, 2280.0f, -5310.0f, 82.0f, 53, 60, ZONE_TYPE_CONTESTED, nullptr}, {46, "Burning Steppes", 0, -7530.0f, -1052.0f, 229.0f, 50, 58, ZONE_TYPE_CONTESTED, nullptr}, {618, "Winterspring", 1, 6724.0f, -4606.0f, 721.0f, 53, 60, ZONE_TYPE_CONTESTED, nullptr},   {1377, "Silithus", 1, -6815.0f, 730.0f, 42.0f, 55, 60, ZONE_TYPE_CONTESTED, nullptr}, {490, "Un'Goro Crater", 1, -6204.0f, -1061.0f, -217.0f, 48, 55, ZONE_TYPE_CONTESTED, nullptr}, {405, "Desolace", 1, -1573.0f, 2860.0f, 76.0f, 30, 40, ZONE_TYPE_CONTESTED, nullptr}, {406, "Stonetalon Mountains", 1, 920.0f, 920.0f, 104.0f, 25, 35, ZONE_TYPE_CONTESTED, nullptr},
};
static const size_t NUM_CONTESTED_ZONES = sizeof(CONTESTED_ZONES) / sizeof(CONTESTED_ZONES[0]);

// Bot GUID pools
static const uint32 HORDE_BOT_GUID_MIN = 33;
static const uint32 HORDE_BOT_GUID_MAX = 52;
static const uint32 ALLIANCE_CITY_GUID_MIN = 53;
static const uint32 ALLIANCE_CITY_GUID_MAX = 72;
static const uint32 ALLIANCE_WORLD_GUID_MIN = 73;
static const uint32 ALLIANCE_WORLD_GUID_MAX = 92;

// ============================================================================
// Helpers
// ============================================================================

static uint32 RandLevel(uint32 min, uint32 max) { return urand(min, max); }

static uint32 CityBotLevel()
{
    // 40% chance level 60, 30% chance 40-59, 30% chance 10-39
    uint32 roll = urand(0, 99);
    if (roll < 40)
        return 60;
    if (roll < 70)
        return urand(40, 59);
    return urand(10, 39);
}

// ============================================================================
// BotDirector
// ============================================================================

BotDirector::BotDirector() : m_enabled(false), m_updateInterval(120000), m_timer(0) {}

void BotDirector::Load()
{
    m_enabled = sConfig.GetBoolDefault("BotDirector.Enable", true);
    m_updateInterval = sConfig.GetIntDefault("BotDirector.UpdateInterval", 120000);
    m_timer = ShortTimeTracker(0); // trigger immediately on first update

    if (m_enabled)
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotDirector] Loaded (interval: %u ms)", m_updateInterval);
    else
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotDirector] Disabled by config");
}

void BotDirector::Update(uint32 diff)
{
    if (!m_enabled)
        return;

    m_timer.Update(diff);
    if (m_timer.Passed())
    {
        m_timer.Reset(m_updateInterval);
        RunDirectorCycle();
    }
}

BotAssignment const* BotDirector::GetAssignment(uint32 guid) const
{
    auto it = m_assignments.find(guid);
    if (it != m_assignments.end())
        return &it->second;
    return nullptr;
}

BotDirectorZoneType BotDirector::ClassifyZone(uint32 zoneId, BotDirectorZoneInfo const** outInfo) const
{
    for (size_t i = 0; i < NUM_CAPITAL_CITIES; ++i)
    {
        if (CAPITAL_CITIES[i].zoneId == zoneId)
        {
            if (outInfo)
                *outInfo = &CAPITAL_CITIES[i];
            return ZONE_TYPE_CAPITAL;
        }
    }
    for (size_t i = 0; i < NUM_FRIENDLY_ZONES; ++i)
    {
        if (FRIENDLY_ZONES[i].zoneId == zoneId)
        {
            if (outInfo)
                *outInfo = &FRIENDLY_ZONES[i];
            return ZONE_TYPE_FRIENDLY;
        }
    }
    for (size_t i = 0; i < NUM_CONTESTED_ZONES; ++i)
    {
        if (CONTESTED_ZONES[i].zoneId == zoneId)
        {
            if (outInfo)
                *outInfo = &CONTESTED_ZONES[i];
            return ZONE_TYPE_CONTESTED;
        }
    }
    if (outInfo)
        *outInfo = nullptr;
    return ZONE_TYPE_UNKNOWN;
}

void BotDirector::RunDirectorCycle()
{
    // Collect online human players
    struct PlayerInfo
    {
        uint32 level;
        uint32 zone;
        std::string name;
    };

    std::vector<PlayerInfo> players;
    {
        HashMapHolder<Player>::MapType& allPlayers = sObjectAccessor.GetPlayers();
        for (auto const& pair : allPlayers)
        {
            Player* p = pair.second;
            if (!p || !p->IsInWorld())
                continue;
            // Only real players (account < 100 = human)
            if (p->GetSession()->GetAccountId() >= 100)
                continue;
            PlayerInfo info;
            info.level = p->GetLevel();
            info.zone = p->GetZoneId();
            info.name = p->GetName();
            players.push_back(info);
        }
    }

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector] Cycle: %u players online", (uint32)players.size());

    // Categorize players by zone type
    struct CategorizedPlayer
    {
        PlayerInfo info;
        BotDirectorZoneInfo const* zoneInfo;
    };

    std::vector<CategorizedPlayer> cityPlayers, friendlyPlayers, contestedPlayers;

    for (auto const& p : players)
    {
        BotDirectorZoneInfo const* zoneInfo = nullptr;
        BotDirectorZoneType type = ClassifyZone(p.zone, &zoneInfo);

        CategorizedPlayer cp;
        cp.info = p;
        cp.zoneInfo = zoneInfo;

        switch (type)
        {
        case ZONE_TYPE_CAPITAL:
            cityPlayers.push_back(cp);
            break;
        case ZONE_TYPE_CONTESTED:
            contestedPlayers.push_back(cp);
            break;
        default:
            friendlyPlayers.push_back(cp);
            break;
        }
    }

    // Build GUID pools and shuffle them
    std::vector<uint32> availableHorde, availableCity, availableWorld;
    for (uint32 g = HORDE_BOT_GUID_MIN; g <= HORDE_BOT_GUID_MAX; ++g)
        availableHorde.push_back(g);
    for (uint32 g = ALLIANCE_CITY_GUID_MIN; g <= ALLIANCE_CITY_GUID_MAX; ++g)
        availableCity.push_back(g);
    for (uint32 g = ALLIANCE_WORLD_GUID_MIN; g <= ALLIANCE_WORLD_GUID_MAX; ++g)
        availableWorld.push_back(g);

    std::shuffle(availableHorde.begin(), availableHorde.end(), std::mt19937(std::random_device()()));
    std::shuffle(availableCity.begin(), availableCity.end(), std::mt19937(std::random_device()()));
    std::shuffle(availableWorld.begin(), availableWorld.end(), std::mt19937(std::random_device()()));

    // New assignment map
    std::unordered_map<uint32, BotAssignment> newAssignments;

    auto popGuid = [](std::vector<uint32>& pool) -> uint32
    {
        if (pool.empty())
            return 0;
        uint32 g = pool.back();
        pool.pop_back();
        return g;
    };

    auto addAssignment = [&newAssignments](uint32 guid, BotDirectorZoneInfo const* zone, float radius, bool aggressive, uint32 level, uint32 faction)
    {
        if (!guid || !zone)
            return;
        BotAssignment a;
        a.guid = guid;
        a.targetMap = zone->mapId;
        a.targetX = zone->x;
        a.targetY = zone->y;
        a.targetZ = zone->z;
        a.targetZone = zone->zoneId;
        a.radius = radius;
        a.aggressive = aggressive;
        a.targetLevel = level;
        a.faction = faction;
        newAssignments[guid] = a;
    };

    // === Always-on city bots: 5 per capital city ===
    for (size_t i = 0; i < NUM_CAPITAL_CITIES; ++i)
    {
        BotDirectorZoneInfo const* zone = &CAPITAL_CITIES[i];
        uint32 numBots = std::min((uint32)5, (uint32)availableCity.size());
        for (uint32 j = 0; j < numBots; ++j)
        {
            uint32 guid = popGuid(availableCity);
            addAssignment(guid, zone, 80.0f, false, CityBotLevel(), 469);
        }
        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]   Always-on: %u city bots in %s", numBots, zone->name);
    }

    // === Extra city bots for players in capitals ===
    for (auto const& cp : cityPlayers)
    {
        if (!cp.zoneInfo)
            continue;
        uint32 numBots = std::min((uint32)5, (uint32)availableCity.size());
        for (uint32 j = 0; j < numBots; ++j)
        {
            uint32 guid = popGuid(availableCity);
            addAssignment(guid, cp.zoneInfo, 80.0f, false, CityBotLevel(), 469);
        }
        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]   Capital %s: extra %u city bots for %s", cp.zoneInfo->name, numBots, cp.info.name.c_str());
    }

    // === Friendly zones: alliance bots within zone level range ===
    for (auto const& cp : friendlyPlayers)
    {
        if (!cp.zoneInfo)
            continue;
        uint32 numBots = std::min((uint32)5, (uint32)availableWorld.size());
        for (uint32 j = 0; j < numBots; ++j)
        {
            uint32 guid = popGuid(availableWorld);
            uint32 level = RandLevel(cp.zoneInfo->levelMin, cp.zoneInfo->levelMax);
            addAssignment(guid, cp.zoneInfo, 150.0f, false, level, 469);
        }
        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]   Friendly %s: %u world bots for %s (zone lvl %u-%u)", cp.zoneInfo->name, numBots, cp.info.name.c_str(), cp.zoneInfo->levelMin, cp.zoneInfo->levelMax);
    }

    // === Contested zones: horde gankers + friendly alliance ===
    for (auto const& cp : contestedPlayers)
    {
        if (!cp.zoneInfo)
            continue;

        // 3-5 Horde enemies (aggressive)
        uint32 numHorde = std::min((uint32)5, (uint32)availableHorde.size());
        for (uint32 j = 0; j < numHorde; ++j)
        {
            uint32 guid = popGuid(availableHorde);
            uint32 level;
            // 10% chance of level 60 for tension
            if (urand(0, 99) < 10)
            {
                level = 60;
                sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]     !! Level 60 Horde ganker near %s!", cp.info.name.c_str());
            }
            else
            {
                level = RandLevel(cp.zoneInfo->levelMin, cp.zoneInfo->levelMax);
            }
            addAssignment(guid, cp.zoneInfo, 200.0f, true, level, 67);
        }

        // 2-3 friendly Alliance
        uint32 numFriendly = std::min((uint32)3, (uint32)availableWorld.size());
        for (uint32 j = 0; j < numFriendly; ++j)
        {
            uint32 guid = popGuid(availableWorld);
            uint32 level;
            // 10% chance of level 60 friendly hero
            if (urand(0, 99) < 10)
            {
                level = 60;
                sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]     ** Level 60 Alliance hero near %s!", cp.info.name.c_str());
            }
            else
            {
                level = RandLevel(cp.zoneInfo->levelMin, cp.zoneInfo->levelMax);
            }
            addAssignment(guid, cp.zoneInfo, 200.0f, false, level, 469);
        }

        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector]   Contested %s: %u horde + %u alliance for %s (zone lvl %u-%u)", cp.zoneInfo->name, numHorde, numFriendly, cp.info.name.c_str(), cp.zoneInfo->levelMin, cp.zoneInfo->levelMax);
    }

    // Swap in new assignments
    m_assignments = std::move(newAssignments);

    // Update playerbot chance values so assigned bots spawn
    UpdateBotChances();

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[BotDirector] Cycle complete: %u bots assigned for %u players", (uint32)m_assignments.size(), (uint32)players.size());
}

void BotDirector::UpdateBotChances() const
{
    // Assigned bots get high chance (30), unassigned get low chance (1)
    auto updateRange = [this](uint32 guidMin, uint32 guidMax)
    {
        for (uint32 g = guidMin; g <= guidMax; ++g)
        {
            uint32 chance = m_assignments.count(g) ? 30 : 1;
            CharacterDatabase.PExecute("UPDATE playerbot SET chance = %u WHERE char_guid = %u", chance, g);
        }
    };

    updateRange(HORDE_BOT_GUID_MIN, HORDE_BOT_GUID_MAX);
    updateRange(ALLIANCE_CITY_GUID_MIN, ALLIANCE_CITY_GUID_MAX);
    updateRange(ALLIANCE_WORLD_GUID_MIN, ALLIANCE_WORLD_GUID_MAX);
}
