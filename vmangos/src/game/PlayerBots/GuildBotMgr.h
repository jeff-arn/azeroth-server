#ifndef _GUILDBOTMGR_H
#define _GUILDBOTMGR_H

#include "Common.h"
#include "Policies/Singleton.h"
#include "ObjectGuid.h"
#include "BotChatManager.h"

#include <vector>
#include <string>
#include <unordered_map>

class Player;

enum GuildBotRole : uint8
{
    GUILD_BOT_ROLE_TANK   = 0,
    GUILD_BOT_ROLE_HEALER = 1,
    GUILD_BOT_ROLE_DPS    = 2,
};

enum GuildBotGearTier : uint8
{
    GUILD_BOT_GEAR_FRESH_60       = 0,
    GUILD_BOT_GEAR_DUNGEON_GEARED = 1,
    GUILD_BOT_GEAR_PRE_RAID_BIS   = 2,
};

struct GuildBotInfo
{
    uint32 guid;
    std::string name;
    uint8  race;
    uint8  cls;
    std::string spec;
    GuildBotRole role;
    bool raidAssigned;
    bool active;
    float gearScore;
    GuildBotGearTier gearTier;
    bool hasLootUpgrades;  // true if bot has received any items via loot (never wipe their gear)
    bool online;           // true if currently spawned as a bot in-world
    BotPersonality personality;
};

struct GuildBotSpecWeights
{
    float stamina;
    float strength;
    float agility;
    float intellect;
    float spirit;
    float attackPower;
    float spellPower;
    float healingPower;
    float hitRating;
    float critRating;
    float defense;
    float dodge;
    float parry;
    float block;
    float mp5;
};

class GuildBotMgr
{
public:
    GuildBotMgr();
    ~GuildBotMgr();

    void Load();

    // Roster access
    GuildBotInfo const* GetBotInfo(uint32 guid) const;
    std::vector<GuildBotInfo> const& GetRoster() const { return m_roster; }
    GuildBotSpecWeights const* GetSpecWeights(uint8 cls, std::string const& spec) const;

    // Raid assembly
    bool AssembleRaid(Player* pLeader);
    bool DisbandRaid(Player* pLeader);
    uint32 GetOnlineGuildBotCount() const;

    // Roster management
    void SetRaidAssigned(uint32 guid, bool assigned);
    void SetBotActive(uint32 guid, bool active);
    void SetBotSpec(uint32 guid, std::string const& spec, GuildBotRole role);

    // Gear management
    float EvaluateUpgrade(uint32 botGuid, uint32 newItemId, uint8 slot) const;
    void OnBotEquipItem(uint32 botGuid, uint32 itemId, uint8 slot);
    void RecalculateGearScore(uint32 botGuid);

    // Gear tier management
    void SetGearTier(uint32 botGuid, GuildBotGearTier tier);
    void ApplyGearTier(uint32 botGuid);  // Log/validate on spawn; no-op if bot has loot upgrades
    bool HasLootUpgrades(uint32 botGuid) const;
    uint32 GetAutoEquipOptionForTier(GuildBotGearTier tier) const;

    // Guild info
    uint32 GetGuildId() const { return m_guildId; }
    bool IsGuildBot(uint32 guid) const;
    void DetectAndPopulateGuild();

    // Update (called from World::Update)
    void Update(uint32 diff);

    // Presence: is this bot under presence control (not raid-assembled)?
    bool IsPresenceBot(uint32 guid) const;

    // Party invite: convert a presence bot to a party bot and back
    bool ConvertPresenceToParty(uint32 botGuid, ObjectGuid leaderGuid);
    void ConvertPartyToPresence(uint32 botGuid);
    void OnBotRemovedFromGroup(uint32 botGuid);

    // Config
    bool IsEnabled() const { return m_enabled; }
    uint32 GetAccountId() const { return m_accountId; }

private:
    void LoadRoster();
    void LoadSpecWeights();
    void DetectPlayerGuild();
    void PopulateBotsIntoGuild();
    void SyncBotLevels();

    // Presence system
    void UpdatePresence();
    uint32 GetTargetOnlineCount() const;
    void LoginPresenceBot(GuildBotInfo& bot);
    void LogoutPresenceBot(GuildBotInfo& bot);

    bool m_presenceEnabled;
    uint32 m_presenceTimer;
    uint32 m_presenceCheckInterval;  // ms
    uint32 m_presencePeakCount;
    uint32 m_presenceOffPeakCount;
    uint32 m_presencePeakHour;
    int32  m_presenceTimezoneOffset;

    bool m_enabled;
    uint32 m_guildId;
    uint32 m_accountId;
    uint32 m_levelSyncTimer;
    static const uint32 LEVEL_SYNC_INTERVAL = 5 * 60 * 1000;  // 5 minutes

    std::vector<GuildBotInfo> m_roster;
    std::unordered_map<uint32, size_t> m_guidToIndex;  // guid -> index into m_roster

    // Key: (class << 8) | hash of spec string — or just a combined string key
    std::unordered_map<std::string, GuildBotSpecWeights> m_specWeights;

    std::string MakeSpecKey(uint8 cls, std::string const& spec) const;
};

#define sGuildBotMgr MaNGOS::Singleton<GuildBotMgr>::Instance()

#endif
