#include "BotServiceMgr.h"
#include "Policies/SingletonImp.h"
#include "Player.h"
#include "ObjectAccessor.h"
#include "World.h"
#include "Chat/Chat.h"
#include "Spell.h"
#include "TradeData.h"
#include "Config/Config.h"
#include "Log.h"
#include "Util.h"

INSTANTIATE_SINGLETON_1(BotServiceMgr);

// ============================================================================
// Portal destination lookup tables.
// ============================================================================
struct PortalDestination
{
    const char* keyword;
    uint32 allianceSpell;
    uint32 hordeSpell;
};

static const PortalDestination PORTAL_DESTINATIONS[] = {
    { "stormwind", PORTAL_SPELL_STORMWIND,    0 },
    { "sw",        PORTAL_SPELL_STORMWIND,    0 },
    { "storm",     PORTAL_SPELL_STORMWIND,    0 },
    { "ironforge", PORTAL_SPELL_IRONFORGE,    0 },
    { "if",        PORTAL_SPELL_IRONFORGE,    0 },
    { "darnassus", PORTAL_SPELL_DARNASSUS,    0 },
    { "darn",      PORTAL_SPELL_DARNASSUS,    0 },
    { "orgrimmar", 0, PORTAL_SPELL_ORGRIMMAR    },
    { "org",       0, PORTAL_SPELL_ORGRIMMAR    },
    { "undercity", 0, PORTAL_SPELL_UNDERCITY    },
    { "uc",        0, PORTAL_SPELL_UNDERCITY    },
    { "thunder",   0, PORTAL_SPELL_THUNDERBLUFF },
    { "tb",        0, PORTAL_SPELL_THUNDERBLUFF },
};
static const uint32 PORTAL_DEST_COUNT = sizeof(PORTAL_DESTINATIONS) / sizeof(PORTAL_DESTINATIONS[0]);

// ============================================================================
// Enchant definitions — all enchants bots can provide.
// ============================================================================
static const EnchantDefinition ENCHANT_LIST[] = {
    // Common (trainer-learned) — 0-1g tip
    { 13890, "minor speed",     "Minor Speed (boots)",        ENCHANT_TIP_COMMON,   false },
    { 13945, "greater stamina", "Greater Stamina (bracers)",  ENCHANT_TIP_COMMON,   false },
    { 20025, "greater stats",   "Greater Stats (chest)",      ENCHANT_TIP_COMMON,   false },
    // Mid-tier (rep/uncommon) — 2-5g tip
    { 20012, "greater agility", "Greater Agility (gloves)",   ENCHANT_TIP_MIDTIER,  false },
    { 20011, "superior stamina","Superior Stamina (bracers)", ENCHANT_TIP_MIDTIER,  false },
    { 20014, "greater resistance","Greater Resistance (cloak)",ENCHANT_TIP_MIDTIER, false },
    { 20017, "greater stamina shield","Greater Stamina (shield)",ENCHANT_TIP_MIDTIER,false },
    // Rare (world drop) — 5-10g tip
    { 20034, "crusader",        "Crusader (weapon)",          ENCHANT_TIP_RARE,     true  },
    { 23800, "agility weapon",  "Agility (weapon)",           ENCHANT_TIP_RARE,     true  },
    // Ultra-rare — 10-20g tip
    { 22749, "spell power",     "Spell Power (weapon)",       ENCHANT_TIP_ULTRARARE,true  },
    { 22750, "healing power",   "Healing Power (weapon)",     ENCHANT_TIP_ULTRARARE,true  },
    { 23802, "healing bracer",  "Healing Power (bracers)",    ENCHANT_TIP_ULTRARARE,true  },
};
static const uint32 ENCHANT_LIST_COUNT = sizeof(ENCHANT_LIST) / sizeof(ENCHANT_LIST[0]);

// Transmute spell IDs.
static const uint32 SPELL_TRANSMUTE_ARCANITE = 17187;
static const uint32 SPELL_MOONCLOTH          = 18560;

// ============================================================================
// Response message pools.
// ============================================================================

// --- Portal responses ---
static const char* PORTAL_ASK_DESTINATION[] = {
    "where do you need a port?", "sure, which city?",
    "where to?", "got ports, where do you need?",
};
static const uint32 PORTAL_ASK_DEST_COUNT = 4;

static const char* PORTAL_CONFIRM[] = {
    "sure, trade me 1g and I'll open it up",
    "np, 1g tip. trade me when you're ready",
    "yeah I got that, trade me 1g",
    "coming right up, just trade me 1g tip",
};
static const uint32 PORTAL_CONFIRM_COUNT = 4;

static const char* PORTAL_WRONG_FACTION[] = {
    "sorry, I can only port to Alliance cities",
    "I don't have that port, only Alliance cities",
    "sorry, I can only port to Horde cities",
    "can't do that one, only Horde cities",
};

static const char* SERVICE_THANKS[] = {
    "np, safe travels", "enjoy, cya", "anytime!", "gl out there",
    "np!", "sure thing", "anytime, cya",
};
static const uint32 SERVICE_THANKS_COUNT = 7;

static const char* SERVICE_LOW_TIP[] = {
    "gonna need a bit more for that", "need a bit more gold",
    "tip's a bit low for that one",
};
static const uint32 SERVICE_LOW_TIP_COUNT = 3;

// --- Enchant responses ---
static const char* ENCHANT_ASK_WHICH[] = {
    "what enchant do you need?", "which enchant?",
    "I can do most enchants, what do you need?",
};
static const uint32 ENCHANT_ASK_WHICH_COUNT = 3;

static const char* ENCHANT_CONFIRM[] = {
    "sure, put your item in the trade window and tip. I'll enchant it",
    "got it, trade me with your item + tip",
    "yeah I can do that, trade me",
};
static const uint32 ENCHANT_CONFIRM_COUNT = 3;

static const char* ENCHANT_DONT_KNOW[] = {
    "sorry, I don't have that recipe",
    "don't know that one, sorry",
    "I don't have that enchant",
};
static const uint32 ENCHANT_DONT_KNOW_COUNT = 3;

// --- Transmute responses ---
static const char* TRANSMUTE_CONFIRM[] = {
    "CD is up, trade me your mats + 3g tip",
    "sure, transmute ready. your mats + 3g",
    "yeah transmute available, trade me mats + tip",
};
static const uint32 TRANSMUTE_CONFIRM_COUNT = 3;

static const char* TRANSMUTE_ON_CD[] = {
    "sorry, my transmute is on cooldown",
    "CD isn't up yet, try again later",
    "transmute on CD, sorry",
};
static const uint32 TRANSMUTE_ON_CD_COUNT = 3;

// --- Mooncloth responses ---
static const char* MOONCLOTH_CONFIRM[] = {
    "Mooncloth CD is up, trade me your felcloth + 3g tip",
    "sure, bring 2 felcloth + 3g. trade me",
    "yeah I can make mooncloth, your mats + tip",
};
static const uint32 MOONCLOTH_CONFIRM_COUNT = 3;

static const char* WAIT_FOR_TRADE[] = {
    "just trade me when you're ready",
    "trade me whenever",
    "ready when you are, just trade me",
};
static const uint32 WAIT_FOR_TRADE_COUNT = 3;

// --- Trade chat ads (extern for BotChatManager) ---
extern const char* TRADE_MESSAGES_PORTALS[] = {
    "WTS portals [Stormwind] [Ironforge] [Darnassus] 1g PST",
    "Mage LF port tips, 1g any city. PST",
    "WTS ports to any city, 1g. PST",
    "Portals up, 1g tip. SW/IF/Darn. PST",
};
extern const uint32 TRADE_PORTALS_COUNT = sizeof(TRADE_MESSAGES_PORTALS) / sizeof(TRADE_MESSAGES_PORTALS[0]);

extern const char* TRADE_MESSAGES_PORTALS_HORDE[] = {
    "WTS portals [Orgrimmar] [Undercity] [Thunder Bluff] 1g PST",
    "Mage LF port tips, 1g any city. PST",
    "WTS ports to any city, 1g. PST",
    "Portals up, 1g tip. Org/UC/TB. PST",
};
extern const uint32 TRADE_PORTALS_HORDE_COUNT = sizeof(TRADE_MESSAGES_PORTALS_HORDE) / sizeof(TRADE_MESSAGES_PORTALS_HORDE[0]);

// ============================================================================
// WTB item definitions — items bots advertise wanting to buy.
// Prices come from AH fair market value at runtime.
// ============================================================================

const BotServiceMgr::WtbItemDef BotServiceMgr::s_wtbItems[] = {
    { 13463, "lotus",           "[Black Lotus]",          5  },
    { 12363, "arcane crystal",  "[Arcane Crystal]",       5  },
    { 12811, "righteous orb",   "[Righteous Orb]",        5  },
    { 14344, "large brilliant", "[Large Brilliant Shard]", 5 },
    { 4625,  "firebloom",       "[Firebloom]",            20 },
    { 13467, "icecap",          "[Icecap]",               20 },
    { 8838,  "sungrass",        "[Sungrass]",             20 },
    { 13468, "black lotus",     "[Black Lotus]",          5  },
    { 7076,  "essence of earth","[Essence of Earth]",     5  },
    { 7078,  "essence of fire", "[Essence of Fire]",      5  },
    { 7080,  "essence of water","[Essence of Water]",     5  },
    { 7082,  "essence of air",  "[Essence of Air]",       5  },
    { 12359, "thorium bar",     "[Thorium Bar]",          20 },
    { 12655, "mooncloth",       "[Mooncloth]",            5  },
    { 12360, "arcanite bar",    "[Arcanite Bar]",         3  },
};
const uint32 BotServiceMgr::s_wtbItemCount = sizeof(s_wtbItems) / sizeof(s_wtbItems[0]);

// ============================================================================
// Implementation
// ============================================================================

BotServiceMgr::BotServiceMgr()
    : m_cleanupTimer(30000)
    , m_buyBudgetPerDay(2000000)  // 200g in copper
    , m_enabled(false)
    , m_loaded(false)
{
}

void BotServiceMgr::Load()
{
    m_enabled = sConfig.GetBoolDefault("BotService.Enable", true);
    m_buyBudgetPerDay = sConfig.GetIntDefault("BotService.Buy.BudgetPerDay", 200) * 10000;  // gold -> copper
    m_loaded = true;
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotServiceMgr: Loaded. Enabled: %s, Buy budget: %ug/day",
        m_enabled ? "yes" : "no", m_buyBudgetPerDay / 10000);
}

void BotServiceMgr::Update(uint32 diff)
{
    if (!m_loaded || !m_enabled)
        return;

    m_cleanupTimer.Update(diff);
    if (m_cleanupTimer.Passed())
    {
        m_cleanupTimer.Reset(30000);
        CleanupExpiredRequests();
    }
}

// ============================================================================
// Whisper-driven service flow — routes to appropriate service handler.
// ============================================================================

bool BotServiceMgr::HandleServiceWhisper(Player* bot, ObjectGuid playerGuid,
                                          const std::string& message)
{
    if (!m_enabled || !bot)
        return false;

    std::string lower = message;
    for (auto& c : lower)
        c = static_cast<char>(tolower(c));

    RequestKey key = MakeKey(bot->GetObjectGuid(), playerGuid);

    // Check if we have an existing request (continuation of a flow).
    auto it = m_requests.find(key);
    if (it != m_requests.end())
    {
        BotServiceRequest& req = it->second;

        if (req.state == SERVICE_STATE_ASKED_DESTINATION)
        {
            // Portal: player specifying a city.
            if (req.serviceType == BOT_SERVICE_PORTAL)
            {
                uint32 spellId = ResolvePortalSpell(bot, lower);
                if (spellId)
                {
                    req.spellId = spellId;
                    req.state = SERVICE_STATE_WAITING_FOR_TRADE;
                    req.lastUpdate = time(nullptr);
                    BotWhisper(bot, playerGuid, PORTAL_CONFIRM[urand(0, PORTAL_CONFIRM_COUNT - 1)]);
                    return true;
                }
                bool isAlliance = (bot->GetTeam() == ALLIANCE);
                BotWhisper(bot, playerGuid, isAlliance ? PORTAL_WRONG_FACTION[0] : PORTAL_WRONG_FACTION[2]);
                return true;
            }

            // Enchant: player specifying which enchant.
            if (req.serviceType == BOT_SERVICE_ENCHANT)
            {
                EnchantDefinition const* enchant = ResolveEnchant(lower);
                if (enchant)
                {
                    if (BotKnowsEnchant(bot, enchant))
                    {
                        req.spellId = enchant->spellId;
                        req.state = SERVICE_STATE_WAITING_FOR_TRADE;
                        req.lastUpdate = time(nullptr);
                        BotWhisper(bot, playerGuid, ENCHANT_CONFIRM[urand(0, ENCHANT_CONFIRM_COUNT - 1)]);
                        return true;
                    }
                    BotWhisper(bot, playerGuid, ENCHANT_DONT_KNOW[urand(0, ENCHANT_DONT_KNOW_COUNT - 1)]);
                    m_requests.erase(it);
                    return true;
                }
                BotWhisper(bot, playerGuid, "which enchant specifically?");
                return true;
            }
        }

        // Waiting for trade — remind them.
        if (req.state == SERVICE_STATE_WAITING_FOR_TRADE)
        {
            BotWhisper(bot, playerGuid, WAIT_FOR_TRADE[urand(0, WAIT_FOR_TRADE_COUNT - 1)]);
            return true;
        }
    }

    // === New request detection ===

    // Portal (mage only).
    if (IsMageBot(bot) &&
        (lower.find("portal") != std::string::npos || lower.find("port") != std::string::npos))
    {
        uint32 spellId = ResolvePortalSpell(bot, lower);
        BotServiceRequest req;
        req.playerGuid = playerGuid;
        req.botGuid = bot->GetObjectGuid();
        req.serviceType = BOT_SERVICE_PORTAL;
        req.createdAt = time(nullptr);
        req.lastUpdate = time(nullptr);

        if (spellId)
        {
            req.spellId = spellId;
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            m_requests[key] = req;
            BotWhisper(bot, playerGuid, PORTAL_CONFIRM[urand(0, PORTAL_CONFIRM_COUNT - 1)]);
        }
        else
        {
            req.spellId = 0;
            req.state = SERVICE_STATE_ASKED_DESTINATION;
            m_requests[key] = req;
            BotWhisper(bot, playerGuid, PORTAL_ASK_DESTINATION[urand(0, PORTAL_ASK_DEST_COUNT - 1)]);
        }
        return true;
    }

    // Enchanting (any bot can potentially be an enchanter — we check class later).
    if (IsEnchanterBot(bot) &&
        (lower.find("enchant") != std::string::npos || lower.find("chant") != std::string::npos))
    {
        // Check if they specified an enchant in the same message.
        EnchantDefinition const* enchant = ResolveEnchant(lower);
        BotServiceRequest req;
        req.playerGuid = playerGuid;
        req.botGuid = bot->GetObjectGuid();
        req.serviceType = BOT_SERVICE_ENCHANT;
        req.createdAt = time(nullptr);
        req.lastUpdate = time(nullptr);

        if (enchant && BotKnowsEnchant(bot, enchant))
        {
            req.spellId = enchant->spellId;
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            m_requests[key] = req;
            BotWhisper(bot, playerGuid, ENCHANT_CONFIRM[urand(0, ENCHANT_CONFIRM_COUNT - 1)]);
        }
        else if (enchant)
        {
            BotWhisper(bot, playerGuid, ENCHANT_DONT_KNOW[urand(0, ENCHANT_DONT_KNOW_COUNT - 1)]);
        }
        else
        {
            req.spellId = 0;
            req.state = SERVICE_STATE_ASKED_DESTINATION;
            m_requests[key] = req;
            BotWhisper(bot, playerGuid, ENCHANT_ASK_WHICH[urand(0, ENCHANT_ASK_WHICH_COUNT - 1)]);
        }
        return true;
    }

    // Transmute (alchemist only).
    if (IsAlchemistBot(bot) &&
        (lower.find("transmute") != std::string::npos || lower.find("arcanite") != std::string::npos))
    {
        BotServiceRequest req;
        req.playerGuid = playerGuid;
        req.botGuid = bot->GetObjectGuid();
        req.serviceType = BOT_SERVICE_TRANSMUTE;
        req.spellId = SPELL_TRANSMUTE_ARCANITE;
        req.createdAt = time(nullptr);
        req.lastUpdate = time(nullptr);

        // Check cooldown (use a simple time-based check — bot tracks internally).
        // For now, transmute is always available. Future: track per-bot CD.
        req.state = SERVICE_STATE_WAITING_FOR_TRADE;
        m_requests[key] = req;
        BotWhisper(bot, playerGuid, TRANSMUTE_CONFIRM[urand(0, TRANSMUTE_CONFIRM_COUNT - 1)]);
        return true;
    }

    // Mooncloth (tailor only).
    if (IsTailorBot(bot) &&
        (lower.find("mooncloth") != std::string::npos))
    {
        BotServiceRequest req;
        req.playerGuid = playerGuid;
        req.botGuid = bot->GetObjectGuid();
        req.serviceType = BOT_SERVICE_MOONCLOTH;
        req.spellId = SPELL_MOONCLOTH;
        req.createdAt = time(nullptr);
        req.lastUpdate = time(nullptr);
        req.state = SERVICE_STATE_WAITING_FOR_TRADE;
        m_requests[key] = req;
        BotWhisper(bot, playerGuid, MOONCLOTH_CONFIRM[urand(0, MOONCLOTH_CONFIRM_COUNT - 1)]);
        return true;
    }

    // Keyword "crusader" directly to any enchanter bot.
    if (IsEnchanterBot(bot) && lower.find("crusader") != std::string::npos)
    {
        EnchantDefinition const* enchant = ResolveEnchant(lower);
        if (enchant && BotKnowsEnchant(bot, enchant))
        {
            BotServiceRequest req;
            req.playerGuid = playerGuid;
            req.botGuid = bot->GetObjectGuid();
            req.serviceType = BOT_SERVICE_ENCHANT;
            req.spellId = enchant->spellId;
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            req.createdAt = time(nullptr);
            req.lastUpdate = time(nullptr);
            m_requests[key] = req;
            BotWhisper(bot, playerGuid, ENCHANT_CONFIRM[urand(0, ENCHANT_CONFIRM_COUNT - 1)]);
            return true;
        }
        BotWhisper(bot, playerGuid, ENCHANT_DONT_KNOW[urand(0, ENCHANT_DONT_KNOW_COUNT - 1)]);
        return true;
    }

    return false;
}

// ============================================================================
// Trade handling.
// ============================================================================

bool BotServiceMgr::ShouldAcceptTrade(Player* bot, Player* player)
{
    if (!m_enabled)
        return false;

    BotServiceRequest* req = GetPendingRequest(bot->GetObjectGuid(), player->GetObjectGuid());
    if (!req || req->state != SERVICE_STATE_WAITING_FOR_TRADE)
        return false;

    req->state = SERVICE_STATE_TRADE_IN_PROGRESS;
    req->lastUpdate = time(nullptr);
    return true;
}

void BotServiceMgr::OnTradeWindowOpen(Player* bot, Player* player)
{
    if (!m_enabled || !bot || !player)
        return;

    BotServiceRequest* req = GetPendingRequest(bot->GetObjectGuid(), player->GetObjectGuid());
    if (!req)
        return;

    // For enchanting services, set the enchant spell on the bot's trade data.
    if (req->serviceType == BOT_SERVICE_ENCHANT && req->spellId)
    {
        TradeData* botTrade = bot->GetTradeData();
        if (botTrade)
        {
            botTrade->SetSpell(req->spellId);
            sLog.Out(LOG_BASIC, LOG_LVL_DETAIL,
                "BotService: %s sets enchant spell %u on trade", bot->GetName(), req->spellId);
        }
    }
}

void BotServiceMgr::OnTradeCompleted(Player* bot, Player* player, uint32 goldReceived)
{
    if (!m_enabled)
        return;

    RequestKey key = MakeKey(bot->GetObjectGuid(), player->GetObjectGuid());
    auto it = m_requests.find(key);
    if (it == m_requests.end())
        return;

    BotServiceRequest& req = it->second;

    // Handle buy-side trades differently
    if (req.serviceType == BOT_SERVICE_BUY)
    {
        // Bot was buying items from player. The trade completed, items transferred.
        // Deduct from bot budget.
        ResetBudgetIfNeeded(bot->GetGUIDLow());
        auto budgetIt = m_buyBudgets.find(bot->GetGUIDLow());
        if (budgetIt != m_buyBudgets.end())
        {
            uint32 cost = goldReceived;  // goldReceived is actually what the BOT paid (from bot's trade side)
            if (cost <= budgetIt->second.remainingCopper)
                budgetIt->second.remainingCopper -= cost;
            else
                budgetIt->second.remainingCopper = 0;
        }

        BotWhisper(bot, player->GetObjectGuid(), "thanks, pleasure doing business");

        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
            "BotService: %s bought items from %s for %u copper",
            bot->GetName(), player->GetName(), goldReceived);

        m_requests.erase(it);
        return;
    }

    // Check minimum tip based on service type.
    uint32 minTip = 0;
    switch (req.serviceType)
    {
        case BOT_SERVICE_PORTAL:    minTip = BotServiceRequest::PORTAL_TIP_MIN; break;
        case BOT_SERVICE_ENCHANT:   minTip = GetEnchantTipMin(ENCHANT_TIP_COMMON); break;
        case BOT_SERVICE_TRANSMUTE: minTip = BotServiceRequest::TRANSMUTE_TIP_MIN; break;
        case BOT_SERVICE_MOONCLOTH: minTip = BotServiceRequest::MOONCLOTH_TIP_MIN; break;
        default: break;
    }

    if (goldReceived < minTip)
    {
        BotWhisper(bot, player->GetObjectGuid(),
            SERVICE_LOW_TIP[urand(0, SERVICE_LOW_TIP_COUNT - 1)]);
        req.state = SERVICE_STATE_WAITING_FOR_TRADE;
        req.lastUpdate = time(nullptr);
        return;
    }

    // Deliver the service.
    switch (req.serviceType)
    {
        case BOT_SERVICE_PORTAL:
            DeliverPortal(bot, player, req.spellId);
            break;
        case BOT_SERVICE_ENCHANT:
            // Enchant was already applied via the trade window spell system.
            // The trade handler cast the spell on the item in slot 6.
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
                "BotService: %s enchanted item for %s (spell %u)",
                bot->GetName(), player->GetName(), req.spellId);
            break;
        case BOT_SERVICE_TRANSMUTE:
            DeliverTransmute(bot, player);
            break;
        case BOT_SERVICE_MOONCLOTH:
            DeliverMooncloth(bot, player);
            break;
        default:
            break;
    }

    BotWhisper(bot, player->GetObjectGuid(),
        SERVICE_THANKS[urand(0, SERVICE_THANKS_COUNT - 1)]);

    m_requests.erase(it);
}

// ============================================================================
// Enchant helpers.
// ============================================================================

EnchantDefinition const* BotServiceMgr::ResolveEnchant(const std::string& message) const
{
    for (uint32 i = 0; i < ENCHANT_LIST_COUNT; ++i)
    {
        if (message.find(ENCHANT_LIST[i].keyword) != std::string::npos)
            return &ENCHANT_LIST[i];
    }
    return nullptr;
}

bool BotServiceMgr::BotKnowsEnchant(Player* bot, EnchantDefinition const* enchant) const
{
    if (!enchant)
        return false;
    // Common and mid-tier enchants: all enchanter bots know them.
    if (!enchant->isRareRecipe)
        return true;
    // Rare recipes: use bot GUID as seed for deterministic "recipe knowledge."
    // ~60% of enchanter bots know Crusader, ~30% know SP/Healing.
    uint32 seed = bot->GetGUIDLow() + enchant->spellId;
    switch (enchant->tipTier)
    {
        case ENCHANT_TIP_RARE:     return (seed % 100) < 60;
        case ENCHANT_TIP_ULTRARARE:return (seed % 100) < 30;
        default: return true;
    }
}

uint32 BotServiceMgr::GetEnchantTipMin(EnchantTipTier tier) const
{
    switch (tier)
    {
        case ENCHANT_TIP_COMMON:    return 0;       // free is ok
        case ENCHANT_TIP_MIDTIER:   return 20000;   // 2g
        case ENCHANT_TIP_RARE:      return 50000;   // 5g
        case ENCHANT_TIP_ULTRARARE: return 100000;  // 10g
    }
    return 0;
}

void BotServiceMgr::DeliverEnchant(Player* /*bot*/, Player* /*player*/, uint32 /*spellId*/)
{
    // Enchants are delivered via the trade window spell system automatically.
    // This is a no-op — the trade handler's spell casting handles it.
}

// ============================================================================
// Transmute/Mooncloth delivery.
// For simplicity, the bot "consumes" the mats from the trade and the product
// appears via the transmute/craft spell effect. In practice, the trade system
// handles mat transfer, and we cast the craft spell to generate the product
// which gets mailed or traded back. For now we just log it — the actual mat
// verification and product delivery requires deeper integration with the
// crafting system that we'll refine in testing.
// ============================================================================

void BotServiceMgr::DeliverTransmute(Player* bot, Player* player)
{
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
        "BotService: %s performs Arcanite transmute for %s",
        bot->GetName(), player->GetName());
    // Future: verify mats (1 Thorium Bar + 1 Arcane Crystal), craft Arcanite Bar,
    // trade product back to player. For now the trade itself handles the exchange.
}

void BotServiceMgr::DeliverMooncloth(Player* bot, Player* player)
{
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
        "BotService: %s crafts Mooncloth for %s",
        bot->GetName(), player->GetName());
    // Future: verify mats (2 Felcloth), craft Mooncloth, trade back.
}

// ============================================================================
// Portal delivery.
// ============================================================================

uint32 BotServiceMgr::ResolvePortalSpell(Player* bot, const std::string& message) const
{
    bool isAlliance = (bot->GetTeam() == ALLIANCE);
    for (uint32 i = 0; i < PORTAL_DEST_COUNT; ++i)
    {
        if (message.find(PORTAL_DESTINATIONS[i].keyword) != std::string::npos)
        {
            uint32 spellId = isAlliance
                ? PORTAL_DESTINATIONS[i].allianceSpell
                : PORTAL_DESTINATIONS[i].hordeSpell;
            return spellId;  // 0 if wrong faction
        }
    }
    return 0;
}

void BotServiceMgr::DeliverPortal(Player* bot, Player* player, uint32 spellId)
{
    if (!spellId)
        return;

    SpellEntry const* spellEntry = sSpellMgr.GetSpellEntry(spellId);
    if (!spellEntry)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "BotService: Unknown portal spell %u", spellId);
        return;
    }

    // Use triggered=true to bypass spell-known checks (CrowdBots don't learn spells)
    bot->CastSpell(bot, spellEntry, true);

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "BotService: %s casts Portal (spell %u) for %s",
        bot->GetName(), spellId, player->GetName());
}

// ============================================================================
// Bot class checks.
// Profession assignment is deterministic based on bot class:
// - Mages: always portals + enchanting
// - Priests/Warlocks: enchanting
// - Any caster (mage/priest/warlock/druid): alchemy
// - Mages/Priests/Warlocks: tailoring (mooncloth)
// ============================================================================

bool BotServiceMgr::IsMageBot(Player* bot)
{
    if (!bot || !bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return false;
    return bot->GetClass() == CLASS_MAGE;
}

bool BotServiceMgr::IsEnchanterBot(Player* bot)
{
    if (!bot || !bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return false;
    // Mages, Priests, and Warlocks can be enchanters.
    uint8 cls = bot->GetClass();
    return cls == CLASS_MAGE || cls == CLASS_PRIEST || cls == CLASS_WARLOCK;
}

bool BotServiceMgr::IsAlchemistBot(Player* bot)
{
    if (!bot || !bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return false;
    // Casters are alchemists.
    uint8 cls = bot->GetClass();
    return cls == CLASS_MAGE || cls == CLASS_PRIEST || cls == CLASS_WARLOCK || cls == CLASS_DRUID;
}

bool BotServiceMgr::IsTailorBot(Player* bot)
{
    if (!bot || !bot->GetSession() || bot->GetSession()->GetAccountId() < 100)
        return false;
    // Cloth-wearing classes are tailors.
    uint8 cls = bot->GetClass();
    return cls == CLASS_MAGE || cls == CLASS_PRIEST || cls == CLASS_WARLOCK;
}

// ============================================================================
// Shared helpers.
// ============================================================================

BotServiceRequest* BotServiceMgr::GetPendingRequest(ObjectGuid botGuid, ObjectGuid playerGuid)
{
    RequestKey key = MakeKey(botGuid, playerGuid);
    auto it = m_requests.find(key);
    if (it != m_requests.end())
        return &it->second;
    return nullptr;
}

void BotServiceMgr::BotWhisper(Player* bot, ObjectGuid targetGuid, const char* message)
{
    Player* target = sObjectAccessor.FindPlayer(targetGuid);
    if (!target || !target->GetSession())
        return;

    WorldPacket data;
    ChatHandler::BuildChatPacket(data, CHAT_MSG_WHISPER, message,
        LANG_UNIVERSAL, bot->GetChatTag(), bot->GetObjectGuid(), bot->GetName());
    target->GetSession()->SendPacket(&data);

    sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotService: %s whispers to %s: %s",
        bot->GetName(), target->GetName(), message);
}

void BotServiceMgr::CleanupExpiredRequests()
{
    time_t now = time(nullptr);
    for (auto it = m_requests.begin(); it != m_requests.end(); )
    {
        if (now - it->second.lastUpdate > BotServiceRequest::TIMEOUT_SECS)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_DETAIL, "BotService: Expired request (type %u) cleaned up",
                it->second.serviceType);
            it = m_requests.erase(it);
        }
        else
            ++it;
    }
}

BotServiceMgr::RequestKey BotServiceMgr::MakeKey(ObjectGuid botGuid, ObjectGuid playerGuid)
{
    return (static_cast<uint64>(botGuid.GetCounter()) << 32) |
            static_cast<uint64>(playerGuid.GetCounter());
}

void BotServiceMgr::CreateProactiveRequest(Player* bot, ObjectGuid playerGuid,
                                            BotServiceType type, const std::string& tradeChatMsg)
{
    if (!bot)
        return;

    RequestKey key = MakeKey(bot->GetObjectGuid(), playerGuid);

    BotServiceRequest req;
    req.playerGuid = playerGuid;
    req.botGuid = bot->GetObjectGuid();
    req.serviceType = type;
    req.spellId = 0;
    req.createdAt = time(nullptr);
    req.lastUpdate = time(nullptr);
    req.buyItemId = 0;
    req.buyMaxQuantity = 0;
    req.buyPriceCopper = 0;

    // For portals, try to parse the destination from the original trade chat message.
    // e.g., "WTB port to darn" → resolve "darn" → Darnassus portal spell.
    if (type == BOT_SERVICE_PORTAL && !tradeChatMsg.empty())
    {
        std::string lower = tradeChatMsg;
        for (auto& c : lower) c = static_cast<char>(tolower(c));

        uint32 spellId = ResolvePortalSpell(bot, lower);
        if (spellId)
        {
            req.spellId = spellId;
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            m_requests[key] = req;
            return;
        }
    }

    // Portal without resolved destination, or enchant: need clarification.
    // Transmute and mooncloth go straight to WAITING_FOR_TRADE.
    switch (type)
    {
        case BOT_SERVICE_PORTAL:
        case BOT_SERVICE_ENCHANT:
            req.state = SERVICE_STATE_ASKED_DESTINATION;
            break;
        default:
            req.state = SERVICE_STATE_WAITING_FOR_TRADE;
            break;
    }

    m_requests[key] = req;
}

bool BotServiceMgr::HasActiveRequest(ObjectGuid playerGuid) const
{
    uint32 playerCounter = playerGuid.GetCounter();
    for (auto const& kv : m_requests)
    {
        if (kv.second.playerGuid.GetCounter() == playerCounter)
            return true;
    }
    return false;
}

void BotServiceMgr::ClearRequestsForPlayer(ObjectGuid playerGuid)
{
    uint32 playerCounter = playerGuid.GetCounter();
    for (auto it = m_requests.begin(); it != m_requests.end(); )
    {
        if (it->second.playerGuid.GetCounter() == playerCounter)
            it = m_requests.erase(it);
        else
            ++it;
    }
}

// ============================================================================
// WTB Buy System
// ============================================================================

#include "AuctionHouse/AuctionHouseBotMgr.h"

BotServiceMgr::WtbItemDef const* BotServiceMgr::ResolveWtbItem(const std::string& message) const
{
    std::string lower = message;
    for (auto& c : lower)
        c = static_cast<char>(tolower(c));

    for (uint32 i = 0; i < s_wtbItemCount; ++i)
    {
        if (lower.find(s_wtbItems[i].keyword) != std::string::npos)
            return &s_wtbItems[i];
    }
    return nullptr;
}

uint32 BotServiceMgr::GetBuyPriceForItem(uint32 itemId, uint32 quantity) const
{
    // Use AH fair market value, discounted 10% (bots buy cheaper than AH)
    uint32 fairBuyout = sAuctionHouseBotMgr.GetFairBuyoutForItem(itemId, quantity);
    if (fairBuyout == 0)
        return 0;

    return static_cast<uint32>(fairBuyout * 0.9f);
}

void BotServiceMgr::ResetBudgetIfNeeded(uint32 botGuid)
{
    time_t now = time(nullptr);
    auto it = m_buyBudgets.find(botGuid);
    if (it == m_buyBudgets.end())
    {
        m_buyBudgets[botGuid] = { m_buyBudgetPerDay, now };
        return;
    }

    // Reset if more than 24 hours since last reset
    if (now - it->second.lastResetTime >= 86400)
    {
        it->second.remainingCopper = m_buyBudgetPerDay;
        it->second.lastResetTime = now;
    }
}

bool BotServiceMgr::HandleBuyWhisper(Player* bot, ObjectGuid playerGuid, const std::string& message)
{
    if (!m_enabled || !bot)
        return false;

    // Check if player is mentioning an item we want to buy
    WtbItemDef const* item = ResolveWtbItem(message);
    if (!item)
        return false;

    // Check budget
    ResetBudgetIfNeeded(bot->GetGUIDLow());
    auto budgetIt = m_buyBudgets.find(bot->GetGUIDLow());
    if (budgetIt == m_buyBudgets.end() || budgetIt->second.remainingCopper == 0)
    {
        BotWhisper(bot, playerGuid, "sorry, I'm tapped out for today");
        return true;
    }

    // Get price
    uint32 pricePerUnit = GetBuyPriceForItem(item->itemId, 1);
    if (pricePerUnit == 0)
    {
        BotWhisper(bot, playerGuid, "hmm, not sure what those are worth right now");
        return true;
    }

    // Create buy request
    RequestKey key = MakeKey(bot->GetObjectGuid(), playerGuid);
    BotServiceRequest& req = m_requests[key];
    req.botGuid = bot->GetObjectGuid();
    req.playerGuid = playerGuid;
    req.serviceType = BOT_SERVICE_BUY;
    req.state = SERVICE_STATE_WAITING_FOR_TRADE;
    req.spellId = 0;
    req.createdAt = time(nullptr);
    req.lastUpdate = time(nullptr);
    req.buyItemId = item->itemId;
    req.buyMaxQuantity = item->maxQuantity;
    req.buyPriceCopper = pricePerUnit;

    // Format price as gold/silver for the whisper
    uint32 totalPrice = pricePerUnit;
    uint32 gold = totalPrice / 10000;
    uint32 silver = (totalPrice % 10000) / 100;

    char priceMsg[256];
    if (silver > 0)
        snprintf(priceMsg, sizeof(priceMsg), "I'll buy %s for %ug %us each, trade me",
            item->displayName, gold, silver);
    else
        snprintf(priceMsg, sizeof(priceMsg), "I'll buy %s for %ug each, trade me",
            item->displayName, gold);

    BotWhisper(bot, playerGuid, priceMsg);
    return true;
}
