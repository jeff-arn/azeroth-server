#include "GuildBotMgr.h"
#include "PlayerBotMgr.h"
#include "PartyBotAI.h"
#include "World.h"
#include "ObjectMgr.h"
#include "ObjectAccessor.h"
#include "Database/DatabaseEnv.h"
#include "Config/Config.h"
#include "Log.h"
#include "Player.h"
#include "Item.h"
#include "ItemPrototype.h"

INSTANTIATE_SINGLETON_1(GuildBotMgr);

#include <sstream>
#include <cmath>

GuildBotMgr::GuildBotMgr()
    : m_enabled(false), m_guildId(0), m_accountId(98)
{
}

GuildBotMgr::~GuildBotMgr()
{
}

void GuildBotMgr::Load()
{
    m_enabled = sConfig.GetBoolDefault("GuildBot.Enable", true);
    if (!m_enabled)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Disabled.");
        return;
    }

    m_accountId = sConfig.GetIntDefault("GuildBot.AccountId", 98);

    LoadRoster();
    LoadSpecWeights();

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Loaded %u guild bots, %u spec weight entries.",
        (uint32)m_roster.size(), (uint32)m_specWeights.size());
}

void GuildBotMgr::LoadRoster()
{
    m_roster.clear();
    m_guidToIndex.clear();

    //                                          0      1       2     3      4     5     6              7       8
    std::unique_ptr<QueryResult> result(CharacterDatabase.Query(
        "SELECT r.guid, c.name, c.race, c.class, r.spec, r.role, r.raid_assigned, r.active, r.gear_score "
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
        info.guid         = fields[0].GetUInt32();
        info.name         = fields[1].GetCppString();
        info.race         = fields[2].GetUInt8();
        info.cls          = fields[3].GetUInt8();
        info.spec         = fields[4].GetCppString();
        info.raidAssigned = fields[6].GetBool();
        info.active       = fields[7].GetBool();
        info.gearScore    = fields[8].GetFloat();
        info.online       = false;

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

    // Get guild ID from first bot
    if (!m_roster.empty())
    {
        std::unique_ptr<QueryResult> guildResult(CharacterDatabase.PQuery(
            "SELECT guild_id FROM guild_member WHERE guid = %u", m_roster[0].guid));
        if (guildResult)
            m_guildId = guildResult->Fetch()[0].GetUInt32();
    }
}

void GuildBotMgr::LoadSpecWeights()
{
    m_specWeights.clear();

    std::unique_ptr<QueryResult> result(CharacterDatabase.Query(
        "SELECT class, spec, stamina, strength, agility, intellect, spirit, "
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
        w.stamina      = fields[2].GetFloat();
        w.strength     = fields[3].GetFloat();
        w.agility      = fields[4].GetFloat();
        w.intellect    = fields[5].GetFloat();
        w.spirit       = fields[6].GetFloat();
        w.attackPower  = fields[7].GetFloat();
        w.spellPower   = fields[8].GetFloat();
        w.healingPower = fields[9].GetFloat();
        w.hitRating    = fields[10].GetFloat();
        w.critRating   = fields[11].GetFloat();
        w.defense      = fields[12].GetFloat();
        w.dodge        = fields[13].GetFloat();
        w.parry        = fields[14].GetFloat();
        w.block        = fields[15].GetFloat();
        w.mp5          = fields[16].GetFloat();

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

bool GuildBotMgr::IsGuildBot(uint32 guid) const
{
    return m_guidToIndex.find(guid) != m_guidToIndex.end();
}

uint32 GuildBotMgr::GetOnlineGuildBotCount() const
{
    uint32 count = 0;
    for (auto const& bot : m_roster)
        if (bot.online)
            ++count;
    return count;
}

// --- Raid Assembly ---

bool GuildBotMgr::AssembleRaid(Player* pLeader)
{
    if (!pLeader || !m_enabled)
        return false;

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

        PartyBotAI* pAI = new PartyBotAI(pLeader,
            pLeader->GetMapId(), pLeader->GetMap()->GetInstanceId(),
            x, y, z, pLeader->GetOrientation());

        // Set role based on guild roster
        switch (bot.role)
        {
            case GUILD_BOT_ROLE_TANK:   pAI->m_role = ROLE_TANK; break;
            case GUILD_BOT_ROLE_HEALER: pAI->m_role = ROLE_HEALER; break;
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

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Assembled raid: %u guild bots spawned for %s.",
        spawned, pLeader->GetName());

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

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[GuildBotMgr] Disbanded raid: %u guild bots dismissed.",
        dismissed);

    return dismissed > 0;
}

// --- Roster Management ---

void GuildBotMgr::SetRaidAssigned(uint32 guid, bool assigned)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].raidAssigned = assigned;
    CharacterDatabase.PExecute(
        "UPDATE guild_bot_roster SET raid_assigned = %u WHERE guid = %u",
        assigned ? 1 : 0, guid);
}

void GuildBotMgr::SetBotActive(uint32 guid, bool active)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].active = active;
    CharacterDatabase.PExecute(
        "UPDATE guild_bot_roster SET active = %u WHERE guid = %u",
        active ? 1 : 0, guid);
}

void GuildBotMgr::SetBotSpec(uint32 guid, std::string const& spec, GuildBotRole role)
{
    auto it = m_guidToIndex.find(guid);
    if (it == m_guidToIndex.end())
        return;

    m_roster[it->second].spec = spec;
    m_roster[it->second].role = role;
    CharacterDatabase.PExecute(
        "UPDATE guild_bot_roster SET spec = '%s', role = '%s' WHERE guid = %u",
        spec.c_str(),
        role == GUILD_BOT_ROLE_TANK ? "tank" : (role == GUILD_BOT_ROLE_HEALER ? "healer" : "dps"),
        guid);
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
            case 1: case 2: maxArmorType = ITEM_SUBCLASS_ARMOR_PLATE; break;  // Warrior, Paladin
            case 3: case 7: maxArmorType = ITEM_SUBCLASS_ARMOR_MAIL; break;   // Hunter, Shaman
            case 4: case 11: maxArmorType = ITEM_SUBCLASS_ARMOR_LEATHER; break; // Rogue, Druid
            case 5: case 8: case 9: maxArmorType = ITEM_SUBCLASS_ARMOR_CLOTH; break; // Priest, Mage, Warlock
            default: maxArmorType = ITEM_SUBCLASS_ARMOR_CLOTH; break;
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
                case ITEM_MOD_STAMINA:       score += value * weights->stamina; break;
                case ITEM_MOD_STRENGTH:      score += value * weights->strength; break;
                case ITEM_MOD_AGILITY:       score += value * weights->agility; break;
                case ITEM_MOD_INTELLECT:     score += value * weights->intellect; break;
                case ITEM_MOD_SPIRIT:        score += value * weights->spirit; break;
                default: break;
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
            return newScore - oldScore;  // Positive = upgrade
        }
    }

    // No item in slot — any item is an upgrade
    return newScore;
}

void GuildBotMgr::OnBotEquipItem(uint32 botGuid, uint32 itemId, uint8 slot)
{
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
                    case ITEM_MOD_STAMINA:   totalScore += value * weights->stamina; break;
                    case ITEM_MOD_STRENGTH:  totalScore += value * weights->strength; break;
                    case ITEM_MOD_AGILITY:   totalScore += value * weights->agility; break;
                    case ITEM_MOD_INTELLECT: totalScore += value * weights->intellect; break;
                    case ITEM_MOD_SPIRIT:    totalScore += value * weights->spirit; break;
                    default: break;
                }
            }
        }
    }

    info.gearScore = totalScore;
    CharacterDatabase.PExecute(
        "UPDATE guild_bot_roster SET gear_score = %f WHERE guid = %u",
        totalScore, botGuid);
}
