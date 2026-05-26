#include "GuildBotMgr.h"
#include "Config/Config.h"
#include "Database/DatabaseEnv.h"
#include "Guild.h"
#include "GuildMgr.h"
#include "Item.h"
#include "ItemPrototype.h"
#include "Log.h"
#include "ObjectAccessor.h"
#include "ObjectMgr.h"
#include "PartyBotAI.h"
#include "Player.h"
#include "PlayerBotAI.h"
#include "PlayerBotMgr.h"
#include "World.h"

INSTANTIATE_SINGLETON_1(GuildBotMgr);

#include <cmath>
#include <sstream>

GuildBotMgr::GuildBotMgr() : m_presenceEnabled(false), m_presenceTimer(0), m_presenceCheckInterval(300000), m_presencePeakCount(14), m_presenceOffPeakCount(3), m_presencePeakHour(20), m_presenceTimezoneOffset(0), m_enabled(false), m_guildId(0), m_accountId(98), m_levelSyncTimer(0) {}

GuildBotMgr::~GuildBotMgr() {}

void GuildBotMgr::Load()
{
    m_enabled = sConfig.GetBoolDefault("GuildBot.Enable", true);
    if (!m_enabled)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Disabled.");
        return;
    }

    m_accountId = sConfig.GetIntDefault("GuildBot.AccountId", 98);

    // Presence system config
    m_presenceEnabled = sConfig.GetBoolDefault("GuildBot.Presence.Enable", true);
    m_presenceCheckInterval = sConfig.GetIntDefault("GuildBot.Presence.CheckInterval", 300) * 1000;
    m_presencePeakHour = sConfig.GetIntDefault("GuildBot.Presence.PeakHour", 20);
    m_presencePeakCount = sConfig.GetIntDefault("GuildBot.Presence.PeakCount", 14);
    m_presenceOffPeakCount = sConfig.GetIntDefault("GuildBot.Presence.OffPeakCount", 3);
    m_presenceTimezoneOffset = sConfig.GetIntDefault("GuildBot.Presence.TimezoneOffset", 0);
    m_presenceTimer = m_presenceCheckInterval - 30000; // first check ~30s after startup

    LoadRoster();
    LoadSpecWeights();

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Loaded %u guild bots, %u spec weight entries. Presence: %s (peak=%u, offpeak=%u, tz=%d)", (uint32)m_roster.size(), (uint32)m_specWeights.size(), m_presenceEnabled ? "ON" : "OFF", m_presencePeakCount, m_presenceOffPeakCount, m_presenceTimezoneOffset);
}

void GuildBotMgr::LoadRoster()
{
    m_roster.clear();
    m_guidToIndex.clear();

    //                                          0      1       2     3      4     5     6              7       8           9          10
    std::unique_ptr<QueryResult> result(CharacterDatabase.Query("SELECT r.guid, c.name, c.race, c.class, r.spec, r.role, r.raid_assigned, r.active, r.gear_score, r.gear_tier, r.personality "
                                                                "FROM guild_bot_roster r "
                                                                "JOIN characters c ON r.guid = c.guid "
                                                                "ORDER BY r.guid"));

    if (!result)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "[GuildBotMgr] No guild bots found in guild_bot_roster!");
        return;
    }

    // Get guild ID from the first bot's guild membership
    do
    {
        Field* fields = result->Fetch();

        GuildBotInfo info;
        info.guid = fields[0].GetUInt32();
        info.name = fields[1].GetCppString();
        info.race = fields[2].GetUInt8();
        info.cls = fields[3].GetUInt8();
        info.spec = fields[4].GetCppString();
        info.raidAssigned = fields[6].GetBool();
        info.active = fields[7].GetBool();
        info.gearScore = fields[8].GetFloat();
        info.hasLootUpgrades = false; // will be detected below
        info.online = false;
        info.personality = BotPersonalityFromString(fields[10].GetCppString());

        std::string tierStr = fields[9].GetCppString();
        if (tierStr == "fresh_60")
            info.gearTier = GUILD_BOT_GEAR_FRESH_60;
        else if (tierStr == "pre_raid_bis")
            info.gearTier = GUILD_BOT_GEAR_PRE_RAID_BIS;
        else
            info.gearTier = GUILD_BOT_GEAR_DUNGEON_GEARED;

        std::string roleStr = fields[5].GetCppString();
        if (roleStr == "tank")
            info.role = GUILD_BOT_ROLE_TANK;
        else if (roleStr == "healer")
            info.role = GUILD_BOT_ROLE_HEALER;
        else
            info.role = GUILD_BOT_ROLE_DPS;

        m_guidToIndex[info.guid] = m_roster.size();
        m_roster.push_back(info);
    }
    while (result->NextRow());

    // Find guild: look for a guild that has a human player member (account < 100)
    DetectPlayerGuild();
}

void GuildBotMgr::DetectPlayerGuild()
{
    // Check if bots are already in a guild
    if (!m_roster.empty())
    {
        std::unique_ptr<QueryResult> guildResult(CharacterDatabase.PQuery("SELECT guild_id FROM guild_member WHERE guid = %u", m_roster[0].guid));
        if (guildResult)
        {
            m_guildId = guildResult->Fetch()[0].GetUInt32();
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Bots already in guild %u.", m_guildId);
            return;
        }
    }

    // No guild yet — find a guild owned by a human player
    std::unique_ptr<QueryResult> result(CharacterDatabase.Query("SELECT g.guild_id FROM guild g "
                                                                "JOIN guild_member gm ON g.guild_id = gm.guild_id "
                                                                "JOIN characters c ON gm.guid = c.guid "
                                                                "WHERE c.account < 100 "
                                                                "ORDER BY g.guild_id ASC LIMIT 1"));

    if (!result)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] No player guild found. Bots will join when a player creates a guild.");
        m_guildId = 0;
        return;
    }

    m_guildId = result->Fetch()[0].GetUInt32();
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Found player guild %u. Adding bots...", m_guildId);

    // Add all roster bots to this guild as rank 2 (Raider)
    PopulateBotsIntoGuild();
}

void GuildBotMgr::PopulateBotsIntoGuild()
{
    if (m_guildId == 0)
        return;

    Guild* guild = sGuildMgr.GetGuildById(m_guildId);
    if (!guild)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "[GuildBotMgr] Guild %u not found in GuildMgr.", m_guildId);
        return;
    }

    uint32 added = 0;
    for (auto const& bot : m_roster)
    {
        ObjectGuid botGuid(HIGHGUID_PLAYER, bot.guid);

        // Use Guild::AddMember to update both DB and in-memory roster
        GuildAddStatus status = guild->AddMember(botGuid, 2);
        if (status == GuildAddStatus::ALREADY_IN_GUILD)
            continue;
        if (status != GuildAddStatus::OK)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "[GuildBotMgr] Failed to add bot %u to guild: status %u", bot.guid, (uint32)status);
            continue;
        }

        // Set player note based on role
        std::string note;
        switch (bot.role)
        {
        case GUILD_BOT_ROLE_TANK:
            note = "Tank";
            break;
        case GUILD_BOT_ROLE_HEALER:
            note = "Healer";
            break;
        case GUILD_BOT_ROLE_DPS:
            note = "DPS";
            break;
        }
        if (MemberSlot* slot = guild->GetMemberSlot(botGuid))
            slot->SetPNOTE(note);

        ++added;
    }

    if (added > 0)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Added %u bots to guild %u.", added, m_guildId);
        guild->Roster(); // Broadcast updated roster to online members
    }
}

void GuildBotMgr::Update(uint32 diff)
{
    if (!m_enabled || m_guildId == 0)
        return;

    m_levelSyncTimer += diff;
    if (m_levelSyncTimer >= LEVEL_SYNC_INTERVAL)
    {
        m_levelSyncTimer = 0;
        SyncBotLevels();
    }

    // Presence system: simulate login/logout for guild bots
    if (m_presenceEnabled)
    {
        m_presenceTimer += diff;
        if (m_presenceTimer >= m_presenceCheckInterval)
        {
            m_presenceTimer = 0;
            UpdatePresence();
        }
    }
}

void GuildBotMgr::SyncBotLevels()
{
    if (m_guildId == 0 || m_roster.empty())
        return;

    // Find the guild master's level
    Guild* guild = sGuildMgr.GetGuildById(m_guildId);
    if (!guild)
        return;

    uint32 leaderGuid = guild->GetLeaderGuid().GetCounter();

    // Get leader level from DB (they may be offline)
    uint32 leaderLevel = 0;
    if (Player* leader = sObjectAccessor.FindPlayer(ObjectGuid(HIGHGUID_PLAYER, leaderGuid)))
    {
        leaderLevel = leader->GetLevel();
    }
    else
    {
        std::unique_ptr<QueryResult> result(CharacterDatabase.PQuery("SELECT level FROM characters WHERE guid = %u", leaderGuid));
        if (result)
            leaderLevel = (*result)[0].GetUInt32();
    }

    if (leaderLevel == 0)
        return;

    uint32 updated = 0;
    for (auto const& bot : m_roster)
    {
        // Target level: leader level ±3, clamped to [1, 60]
        int32 offset = (int32)(rand() % 7) - 3; // -3 to +3
        uint32 targetLevel = std::max(1, std::min(60, (int32)leaderLevel + offset));

        // Get current bot level
        std::unique_ptr<QueryResult> result(CharacterDatabase.PQuery("SELECT level FROM characters WHERE guid = %u", bot.guid));
        if (!result)
            continue;

        uint32 currentLevel = (*result)[0].GetUInt32();
        if (currentLevel == targetLevel)
            continue;

        CharacterDatabase.PExecute("UPDATE characters SET level = %u WHERE guid = %u", targetLevel, bot.guid);

        // Update in-memory guild roster too
        if (MemberSlot* slot = guild->GetMemberSlot(ObjectGuid(HIGHGUID_PLAYER, bot.guid)))
            slot->Level = targetLevel;

        ++updated;
    }

    if (updated > 0)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[GuildBotMgr] Synced %u bot levels to GM level %u (±3).", updated, leaderLevel);
        guild->Roster(); // Refresh roster for online players
    }
}

void GuildBotMgr::LoadSpecWeights()
{
    m_specWeights.clear();

    std::unique_ptr<QueryResult> result(CharacterDatabase.Query("SELECT class, spec, stamina, strength, agility, intellect, spirit, "
                                                                "attack_power, spell_power, healing_power, hit_rating, crit_rating, "
                                                                "defense, dodge, parry, block, mp5 "
                                                                "FROM guild_bot_spec_weights"));

    if (!result)
        return;

    do
    {
        Field* fields = result->Fetch();

        uint8 cls = fields[0].GetUInt8();
        std::string spec = fields[1].GetCppString();

        GuildBotSpecWeights w;
        w.stamina = fields[2].GetFloat();
        w.strength = fields[3].GetFloat();
        w.agility = fields[4].GetFloat();
        w.intellect = fields[5].GetFloat();
        w.spirit = fields[6].GetFloat();
        w.attackPower = fields[7].GetFloat();
        w.spellPower = fields[8].GetFloat();
        w.healingPower = fields[9].GetFloat();
        w.hitRating = fields[10].GetFloat();
        w.critRating = fields[11].GetFloat();
        w.defense = fields[12].GetFloat();
        w.dodge = fields[13].GetFloat();
        w.parry = fields[14].GetFloat();
        w.block = fields[15].GetFloat();
        w.mp5 = fields[16].GetFloat();

        m_specWeights[MakeSpecKey(cls, spec)] = w;
    }
    while (result->NextRow());
}

std::string GuildBotMgr::MakeSpecKey(uint8 cls, std::string const& spec) const
{
    std::ostringstream ss;
    ss << (int)cls << ":" << spec;
    return ss.str();
}

// --- Roster Access ---

GuildBotInfo const* GuildBotMgr::GetBotInfo(uint32 guid) const
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return nullptr;
    return &m_roster[it->second];
}

GuildBotSpecWeights const* GuildBotMgr::GetSpecWeights(uint8 cls, std::string const& spec) const
{
    auto it = m_specWeights.find(MakeSpecKey(cls, spec));
    if (it == m_specWeights.end())
        return nullptr;
    return &it->second;
}

bool GuildBotMgr::IsGuildBot(uint32 guid) const { return m_guidToIndex.find(guid) != m_guidToIndex.end(); }

uint32 GuildBotMgr::GetOnlineGuildBotCount() const
{
    uint32 count = 0;
    for (auto const& bot : m_roster)
        if (bot.online)
            ++count;
    return count;
}

void GuildBotMgr::DetectAndPopulateGuild() { DetectPlayerGuild(); }

// --- Raid Assembly ---

bool GuildBotMgr::AssembleRaid(Player* pLeader)
{
    if (!pLeader || !m_enabled)
        return false;

    // If no guild detected yet, try again (player may have just created one)
    if (m_guildId == 0)
    {
        DetectPlayerGuild();
        if (m_guildId == 0)
            return false;
    }

    uint32 spawned = 0;

    for (auto& bot : m_roster)
    {
        if (!bot.raidAssigned || !bot.active)
            continue;

        if (bot.online)
            continue;

        // Check if character is already logged in
        if (sObjectAccessor.FindPlayerNotInWorld(ObjectGuid(HIGHGUID_PLAYER, bot.guid)))
            continue;

        float x, y, z;
        pLeader->GetNearPoint(pLeader, x, y, z, 0, 5.0f, frand(0.0f, 6.28f));

        PartyBotAI* pAI = new PartyBotAI(pLeader, pLeader->GetMapId(), pLeader->GetMap()->GetInstanceId(), x, y, z, pLeader->GetOrientation());

        // Set role based on guild roster
        switch (bot.role)
        {
        case GUILD_BOT_ROLE_TANK:
            pAI->m_role = ROLE_TANK;
            break;
        case GUILD_BOT_ROLE_HEALER:
            pAI->m_role = ROLE_HEALER;
            break;
        case GUILD_BOT_ROLE_DPS:
            // Auto-detect melee vs ranged based on class
            switch (bot.cls)
            {
            case 1: // Warrior
            case 4: // Rogue
                pAI->m_role = ROLE_MELEE_DPS;
                break;
            case 2: // Paladin (Ret)
                pAI->m_role = (bot.spec == "retribution") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
                break;
            case 11: // Druid (Feral Cat)
                pAI->m_role = (bot.spec == "feral_cat") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
                break;
            case 7: // Shaman (Enhancement)
                pAI->m_role = (bot.spec == "enhancement") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
                break;
            default:
                pAI->m_role = ROLE_RANGE_DPS;
                break;
            }
            break;
        }

        if (sPlayerBotMgr.AddBot(bot.guid, false, pAI))
        {
            bot.online = true;
            ++spawned;
        }
        else
        {
            delete pAI;
        }
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Assembled raid: %u guild bots spawned for %s.", spawned, pLeader->GetName());

    return spawned > 0;
}

bool GuildBotMgr::DisbandRaid(Player* pLeader)
{
    if (!pLeader || !m_enabled)
        return false;

    uint32 dismissed = 0;

    for (auto& bot : m_roster)
    {
        if (!bot.online)
            continue;

        if (sPlayerBotMgr.DeleteBot(bot.guid))
        {
            bot.online = false;
            ++dismissed;
        }
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Disbanded raid: %u guild bots dismissed.", dismissed);

    return dismissed > 0;
}

// --- Roster Management ---

void GuildBotMgr::SetRaidAssigned(uint32 guid, bool assigned)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].raidAssigned = assigned;
    CharacterDatabase.PExecute("UPDATE guild_bot_roster SET raid_assigned = %u WHERE guid = %u", assigned ? 1 : 0, guid);
}

void GuildBotMgr::SetBotActive(uint32 guid, bool active)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].active = active;
    CharacterDatabase.PExecute("UPDATE guild_bot_roster SET active = %u WHERE guid = %u", active ? 1 : 0, guid);
}

void GuildBotMgr::SetBotSpec(uint32 guid, std::string const& spec, GuildBotRole role)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].spec = spec;
    m_roster[it->second].role = role;
    CharacterDatabase.PExecute("UPDATE guild_bot_roster SET spec = '%s', role = '%s' WHERE guid = %u", spec.c_str(), role == GUILD_BOT_ROLE_TANK ? "tank" : (role == GUILD_BOT_ROLE_HEALER ? "healer" : "dps"), guid);
}

// --- Gear Evaluation ---

float GuildBotMgr::EvaluateUpgrade(uint32 botGuid, uint32 newItemId, uint8 slot) const
{
    GuildBotInfo const* info = GetBotInfo(botGuid);
    if (!info)
        return -1.0f;

    GuildBotSpecWeights const* weights = GetSpecWeights(info->cls, info->spec);
    if (!weights)
        return -1.0f;

    ItemPrototype const* pNewProto = sObjectMgr.GetItemPrototype(newItemId);
    if (!pNewProto)
        return -1.0f;

    // Check armor class restrictions
    if (pNewProto->Class == ITEM_CLASS_ARMOR && pNewProto->SubClass > 0)
    {
        uint32 maxArmorType = 0;
        switch (info->cls)
        {
        case 1:
        case 2:
            maxArmorType = ITEM_SUBCLASS_ARMOR_PLATE;
            break; // Warrior, Paladin
        case 3:
        case 7:
            maxArmorType = ITEM_SUBCLASS_ARMOR_MAIL;
            break; // Hunter, Shaman
        case 4:
        case 11:
            maxArmorType = ITEM_SUBCLASS_ARMOR_LEATHER;
            break; // Rogue, Druid
        case 5:
        case 8:
        case 9:
            maxArmorType = ITEM_SUBCLASS_ARMOR_CLOTH;
            break; // Priest, Mage, Warlock
        default:
            maxArmorType = ITEM_SUBCLASS_ARMOR_CLOTH;
            break;
        }
        // Allow cloth for anyone, but only up to their max type
        if (pNewProto->SubClass > maxArmorType)
            return -1.0f;
    }

    // Score the new item based on stat weights
    auto scoreItem = [weights](ItemPrototype const* proto) -> float
    {
        float score = 0.0f;
        for (int i = 0; i < MAX_ITEM_PROTO_STATS; ++i)
        {
            if (proto->ItemStat[i].ItemStatValue == 0)
                continue;

            float value = (float)proto->ItemStat[i].ItemStatValue;
            switch (proto->ItemStat[i].ItemStatType)
            {
            case ITEM_MOD_STAMINA:
                score += value * weights->stamina;
                break;
            case ITEM_MOD_STRENGTH:
                score += value * weights->strength;
                break;
            case ITEM_MOD_AGILITY:
                score += value * weights->agility;
                break;
            case ITEM_MOD_INTELLECT:
                score += value * weights->intellect;
                break;
            case ITEM_MOD_SPIRIT:
                score += value * weights->spirit;
                break;
            default:
                break;
            }
        }
        // Add armor value for tanks
        if (weights->defense > 0 || weights->dodge > 0)
            score += proto->Armor * 0.01f * (weights->defense + weights->dodge);
        return score;
    };

    float newScore = scoreItem(pNewProto);

    // Get currently equipped item in same slot for comparison
    Player* pBot = sObjectAccessor.FindPlayer(ObjectGuid(HIGHGUID_PLAYER, botGuid));
    if (pBot)
    {
        if (Item* pOldItem = pBot->GetItemByPos(INVENTORY_SLOT_BAG_0, slot))
        {
            float oldScore = scoreItem(pOldItem->GetProto());
            return newScore - oldScore; // Positive = upgrade
        }
    }

    // No item in slot — any item is an upgrade
    return newScore;
}

void GuildBotMgr::OnBotEquipItem(uint32 botGuid, uint32 itemId, uint8 slot)
{
    // Mark that this bot has received loot — gear tier changes should not wipe their gear
    auto it = m_guidToIndex.find(botGuid);
    if (it != m_guidToIndex.end())
        m_roster[it->second].hasLootUpgrades = true;

    RecalculateGearScore(botGuid);
}

void GuildBotMgr::RecalculateGearScore(uint32 botGuid)
{
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return;

    GuildBotInfo& info = m_roster[it->second];
    GuildBotSpecWeights const* weights = GetSpecWeights(info.cls, info.spec);
    if (!weights)
        return;

    Player* pBot = sObjectAccessor.FindPlayer(ObjectGuid(HIGHGUID_PLAYER, botGuid));
    if (!pBot)
        return;

    float totalScore = 0.0f;
    for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
    {
        if (Item* pItem = pBot->GetItemByPos(INVENTORY_SLOT_BAG_0, slot))
        {
            ItemPrototype const* proto = pItem->GetProto();
            for (int i = 0; i < MAX_ITEM_PROTO_STATS; ++i)
            {
                if (proto->ItemStat[i].ItemStatValue == 0)
                    continue;
                float value = (float)proto->ItemStat[i].ItemStatValue;
                switch (proto->ItemStat[i].ItemStatType)
                {
                case ITEM_MOD_STAMINA:
                    totalScore += value * weights->stamina;
                    break;
                case ITEM_MOD_STRENGTH:
                    totalScore += value * weights->strength;
                    break;
                case ITEM_MOD_AGILITY:
                    totalScore += value * weights->agility;
                    break;
                case ITEM_MOD_INTELLECT:
                    totalScore += value * weights->intellect;
                    break;
                case ITEM_MOD_SPIRIT:
                    totalScore += value * weights->spirit;
                    break;
                default:
                    break;
                }
            }
        }
    }

    info.gearScore = totalScore;
    CharacterDatabase.PExecute("UPDATE guild_bot_roster SET gear_score = %f WHERE guid = %u", totalScore, botGuid);
}

void GuildBotMgr::SetGearTier(uint32 botGuid, GuildBotGearTier tier)
{
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].gearTier = tier;

    char const* tierStr = "dungeon_geared";
    switch (tier)
    {
    case GUILD_BOT_GEAR_FRESH_60:
        tierStr = "fresh_60";
        break;
    case GUILD_BOT_GEAR_PRE_RAID_BIS:
        tierStr = "pre_raid_bis";
        break;
    default:
        break;
    }

    CharacterDatabase.PExecute("UPDATE guild_bot_roster SET gear_tier = '%s' WHERE guid = %u", tierStr, botGuid);
}

bool GuildBotMgr::HasLootUpgrades(uint32 botGuid) const
{
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return false;
    return m_roster[it->second].hasLootUpgrades;
}

uint32 GuildBotMgr::GetAutoEquipOptionForTier(GuildBotGearTier tier) const
{
    switch (tier)
    {
    case GUILD_BOT_GEAR_FRESH_60:
        return PLAYER_BOT_AUTO_EQUIP_RANDOM_GEAR;
    case GUILD_BOT_GEAR_PRE_RAID_BIS:
        return PLAYER_BOT_AUTO_EQUIP_PREMADE_GEAR;
    case GUILD_BOT_GEAR_DUNGEON_GEARED:
    default:
        return PLAYER_BOT_AUTO_EQUIP_PREMADE_GEAR;
    }
}

void GuildBotMgr::ApplyGearTier(uint32 botGuid)
{
    // Called by PartyBotAI during initialization when a guild bot spawns.
    // If the bot has received loot upgrades from raiding, their gear is sacred — do not touch.
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return;

    GuildBotInfo& info = m_roster[it->second];

    if (info.hasLootUpgrades)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[GuildBotMgr] Bot %s (GUID %u) has loot upgrades — gear tier change skipped.", info.name.c_str(), botGuid);
        return;
    }

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "[GuildBotMgr] Bot %s (GUID %u) will equip gear tier %u on spawn.", info.name.c_str(), botGuid, (uint32)info.gearTier);
}

// ============================================================================
// Guild Bot Online Presence
// ============================================================================

bool GuildBotMgr::IsPresenceBot(uint32 guid) const
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return false;
    return m_roster[it->second].online;
}

uint32 GuildBotMgr::GetTargetOnlineCount() const
{
    // Get current server hour adjusted for timezone
    time_t now = time(nullptr);
    struct tm* tmNow = gmtime(&now);
    int hour = (tmNow->tm_hour + m_presenceTimezoneOffset + 24) % 24;

    // Smooth curve: peak at m_presencePeakHour, trough 12 hours later
    // Uses cosine curve between off-peak and peak counts
    int distFromPeak = abs(hour - (int)m_presencePeakHour);
    if (distFromPeak > 12)
        distFromPeak = 24 - distFromPeak;

    // 0 = at peak, 12 = at trough
    float t = static_cast<float>(distFromPeak) / 12.0f; // 0.0 to 1.0
    // Cosine interpolation: smooth transition
    float factor = (1.0f + cosf(t * 3.14159f)) / 2.0f; // 1.0 at peak, 0.0 at trough

    uint32 target = static_cast<uint32>(m_presenceOffPeakCount + factor * (m_presencePeakCount - m_presenceOffPeakCount));

    // Cap to roster size
    return std::min(target, static_cast<uint32>(m_roster.size()));
}

void GuildBotMgr::LoginPresenceBot(GuildBotInfo& bot)
{
    if (bot.online)
        return;

    // Check if already logged in
    if (sObjectAccessor.FindPlayerNotInWorld(ObjectGuid(HIGHGUID_PLAYER, bot.guid)))
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Presence login skip (already in world): %s (GUID %u)", bot.name.c_str(), bot.guid);
        return;
    }

    // Spawn as CrowdBotAI in a capital city (idle presence, no combat)
    CrowdBotAI* pAI = new CrowdBotAI();

    if (sPlayerBotMgr.AddBot(bot.guid, false, pAI))
    {
        bot.online = true;
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Presence login: %s (GUID %u)", bot.name.c_str(), bot.guid);
    }
    else
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Presence login FAILED: %s (GUID %u) — AddBot returned false", bot.name.c_str(), bot.guid);
        delete pAI;
    }
}

void GuildBotMgr::LogoutPresenceBot(GuildBotInfo& bot)
{
    if (!bot.online)
        return;

    if (sPlayerBotMgr.DeleteBot(bot.guid))
    {
        bot.online = false;
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Presence logout: %s", bot.name.c_str());
    }
}

void GuildBotMgr::UpdatePresence()
{
    if (!m_presenceEnabled || m_roster.empty())
        return;

    uint32 target = GetTargetOnlineCount();

    // Count currently online guild bots
    uint32 currentPresence = 0;
    for (auto const& bot : m_roster)
    {
        if (bot.online)
            ++currentPresence;
    }

    if (currentPresence < target)
    {
        uint32 toLogin = target - currentPresence;

        // Collect offline bots eligible for presence login.
        // raidAssigned only means "in the raid plan" — bots are available for
        // presence unless they're currently online (e.g., actively assembled).
        std::vector<size_t> offlineBots;
        for (size_t i = 0; i < m_roster.size(); ++i)
        {
            if (!m_roster[i].online && m_roster[i].active)
                offlineBots.push_back(i);
        }

        for (uint32 i = 0; i < toLogin && !offlineBots.empty(); ++i)
        {
            uint32 idx = urand(0, static_cast<uint32>(offlineBots.size()) - 1);
            LoginPresenceBot(m_roster[offlineBots[idx]]);
            offlineBots.erase(offlineBots.begin() + idx);
        }
    }
    else if (currentPresence > target)
    {
        uint32 toLogout = currentPresence - target;

        // Collect online bots eligible for presence logout
        std::vector<size_t> onlineBots;
        for (size_t i = 0; i < m_roster.size(); ++i)
        {
            if (m_roster[i].online)
                onlineBots.push_back(i);
        }

        for (uint32 i = 0; i < toLogout && !onlineBots.empty(); ++i)
        {
            uint32 idx = urand(0, static_cast<uint32>(onlineBots.size()) - 1);
            LogoutPresenceBot(m_roster[onlineBots[idx]]);
            onlineBots.erase(onlineBots.begin() + idx);
        }
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Presence update: target=%u, current=%u", target, currentPresence);
}

// ============================================================================
// Party Invite: Convert presence bot <-> party bot
// ============================================================================

bool GuildBotMgr::ConvertPresenceToParty(uint32 botGuid, ObjectGuid leaderGuid)
{
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return false;

    GuildBotInfo& info = m_roster[it->second];
    if (!info.online)
        return false;

    Player* pLeader = sObjectAccessor.FindPlayer(leaderGuid);
    if (!pLeader)
        return false;

    // Get bot's current position before despawn
    Player* pBot = sObjectAccessor.FindPlayer(ObjectGuid(HIGHGUID_PLAYER, botGuid));
    float x, y, z, o;
    uint32 mapId = 0;
    uint32 instanceId = 0;
    if (pBot)
    {
        x = pBot->GetPositionX();
        y = pBot->GetPositionY();
        z = pBot->GetPositionZ();
        o = pBot->GetOrientation();
        mapId = pBot->GetMapId();
        instanceId = pBot->GetMap() ? pBot->GetMap()->GetInstanceId() : 0;
    }
    else
    {
        // Fallback to leader position
        x = pLeader->GetPositionX();
        y = pLeader->GetPositionY();
        z = pLeader->GetPositionZ();
        o = pLeader->GetOrientation();
        mapId = pLeader->GetMapId();
        instanceId = pLeader->GetMap() ? pLeader->GetMap()->GetInstanceId() : 0;
    }

    // Despawn the CrowdBotAI bot and remove its entry so AddBot gets a fresh account ID
    sPlayerBotMgr.DeleteBot(botGuid);
    sPlayerBotMgr.RemoveBotEntry(botGuid);
    info.online = false;

    // Respawn as PartyBotAI
    PartyBotAI* pAI = new PartyBotAI(pLeader, mapId, instanceId, x, y, z, o);

    // Set role from guild roster
    switch (info.role)
    {
    case GUILD_BOT_ROLE_TANK:
        pAI->m_role = ROLE_TANK;
        break;
    case GUILD_BOT_ROLE_HEALER:
        pAI->m_role = ROLE_HEALER;
        break;
    case GUILD_BOT_ROLE_DPS:
        switch (info.cls)
        {
        case 1:
        case 4: // Warrior, Rogue
            pAI->m_role = ROLE_MELEE_DPS;
            break;
        case 2: // Paladin
            pAI->m_role = (info.spec == "retribution") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
            break;
        case 7: // Shaman
            pAI->m_role = (info.spec == "enhancement") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
            break;
        case 11: // Druid
            pAI->m_role = (info.spec == "feral_cat") ? ROLE_MELEE_DPS : ROLE_RANGE_DPS;
            break;
        default:
            pAI->m_role = ROLE_RANGE_DPS;
            break;
        }
        break;
    }

    if (sPlayerBotMgr.AddBot(botGuid, false, pAI))
    {
        info.online = true;
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] %s converted to PartyBot for %s", info.name.c_str(), pLeader->GetName());
        return true;
    }
    else
    {
        delete pAI;
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] %s failed to convert to PartyBot", info.name.c_str());
        return false;
    }
}

void GuildBotMgr::ConvertPartyToPresence(uint32 botGuid)
{
    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return;

    GuildBotInfo& info = m_roster[it->second];

    // Despawn the PartyBotAI bot and remove entry for clean respawn
    if (info.online)
    {
        sPlayerBotMgr.DeleteBot(botGuid);
        sPlayerBotMgr.RemoveBotEntry(botGuid);
        info.online = false;
    }

    // Respawn as CrowdBotAI (presence mode)
    if (m_presenceEnabled)
    {
        LoginPresenceBot(info);
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] %s reverted to presence bot", info.name.c_str());
}

void GuildBotMgr::OnBotRemovedFromGroup(uint32 botGuid)
{
    if (!IsGuildBot(botGuid))
        return;

    auto it = m_guidToIndex.find(botGuid);
    if (it == m_guidToIndex.end())
        return;

    GuildBotInfo& info = m_roster[it->second];

    // Only revert if bot is online (avoid double-fire from RemoveMember + Disband)
    if (!info.online)
        return;

    // Despawn the PartyBotAI bot
    sPlayerBotMgr.DeleteBot(botGuid);
    sPlayerBotMgr.RemoveBotEntry(botGuid);
    info.online = false;

    // Don't respawn immediately — the player object is still being cleaned up.
    // The presence system will pick this bot up on its next cycle (~30s-5min).
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] %s removed from group, will rejoin via presence", info.name.c_str());
}
