#ifndef _GUILDBOTMGR_H
#define _GUILDBOTMGR_H

#include "Common.h"
#include "Policies/Singleton.h"
#include "ObjectGuid.h"

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
    bool online;       // true if currently spawned as a bot in-world
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

    // Guild info
    uint32 GetGuildId() const { return m_guildId; }
    bool IsGuildBot(uint32 guid) const;
    void DetectAndPopulateGuild();

    // Config
    bool IsEnabled() const { return m_enabled; }
    uint32 GetAccountId() const { return m_accountId; }

private:
    void LoadRoster();
    void LoadSpecWeights();
    void DetectPlayerGuild();
    void PopulateBotsIntoGuild();

    bool m_enabled;
    uint32 m_guildId;
    uint32 m_accountId;

    std::vector<GuildBotInfo> m_roster;
    std::unordered_map<uint32, size_t> m_guidToIndex;  // guid -> index into m_roster

    // Key: (class << 8) | hash of spec string — or just a combined string key
    std::unordered_map<std::string, GuildBotSpecWeights> m_specWeights;

    std::string MakeSpecKey(uint8 cls, std::string const& spec) const;
};

#define sGuildBotMgr MaNGOS::Singleton<GuildBotMgr>::Instance()

#endif
