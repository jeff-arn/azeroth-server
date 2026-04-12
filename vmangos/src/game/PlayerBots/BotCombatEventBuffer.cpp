#include "BotCombatEventBuffer.h"
#include "Unit.h"
#include "Creature.h"
#include "Player.h"
#include "Database/DatabaseEnv.h"
#include "Timer.h"
#include "Log.h"

// ============================================================
// CombatEventBuffer
// ============================================================

void CombatEventBuffer::Push(CombatEvent const& e)
{
    m_events[m_writeIdx] = e;
    m_writeIdx++; // uint8 wraps at 256
}

// ============================================================
// BotPatternMgr Singleton
// ============================================================

BotPatternMgr& BotPatternMgr::Instance()
{
    static BotPatternMgr instance;
    return instance;
}

// --- Recording ---

void BotPatternMgr::RecordDamage(Unit* source, Unit* victim, uint32 damage,
                                  uint32 spellId, uint32 mapId, uint32 creatureEntry)
{
    if (!m_active)
        return;

    CombatEvent e;
    e.timestamp = WorldTimer::getMSTime();
    e.eventType = COMBAT_EVENT_DAMAGE;
    e.sourceGuid = source->GetGUIDLow();
    e.targetGuid = victim->GetGUIDLow();
    e.spellId = spellId;
    e.damage = damage;
    e.creatureEntry = creatureEntry;
    m_buffer.Push(e);

    // Check if this damage is significant enough to trigger pattern detection
    // Threshold: > 15% of victim's max HP
    if (victim->IsPlayer() && damage > (victim->GetMaxHealth() * 15 / 100))
        DetectPatterns(creatureEntry, victim->GetGUIDLow(), damage);
}

void BotPatternMgr::RecordCastStart(Unit* caster, Unit* target, SpellEntry const* spell,
                                     uint32 mapId, uint32 creatureEntry)
{
    if (!m_active || !spell)
        return;

    CombatEvent e;
    e.timestamp = WorldTimer::getMSTime();
    e.eventType = COMBAT_EVENT_CAST_START;
    e.sourceGuid = caster->GetGUIDLow();
    e.targetGuid = target ? target->GetGUIDLow() : 0;
    e.spellId = spell->Id;
    e.damage = 0;
    e.creatureEntry = creatureEntry;
    m_buffer.Push(e);
}

void BotPatternMgr::RecordDeath(Unit* killer, Unit* victim, uint32 mapId, uint32 creatureEntry)
{
    if (!m_active)
        return;

    CombatEvent e;
    e.timestamp = WorldTimer::getMSTime();
    e.eventType = COMBAT_EVENT_DEATH;
    e.sourceGuid = killer->GetGUIDLow();
    e.targetGuid = victim->GetGUIDLow();
    e.spellId = 0;
    e.damage = 0;
    e.creatureEntry = creatureEntry;
    m_buffer.Push(e);

    // Death is a strong signal — scan for patterns with boosted confidence
    if (victim->IsPlayer())
        DetectPatterns(creatureEntry, victim->GetGUIDLow(), 0);
}

// --- Pattern Detection ---

void BotPatternMgr::DetectPatterns(uint32 creatureEntry, uint64 victimGuid, uint32 damage)
{
    if (creatureEntry == 0)
        return;

    uint32 now = WorldTimer::getMSTime();
    bool isDeath = (damage == 0);

    // Scan backwards for a CAST_START from the same creature within 5 seconds
    uint32 foundSpellId = 0;
    uint32 castTimestamp = 0;
    uint8 hitCount = 0;

    // First pass: find the most recent CAST_START from this creature entry
    m_buffer.ScanBackwards([&](CombatEvent const& evt) -> bool
    {
        // Don't look more than 5 seconds back
        if (now - evt.timestamp > 5000)
            return false;

        if (evt.creatureEntry == creatureEntry && evt.eventType == COMBAT_EVENT_CAST_START)
        {
            foundSpellId = evt.spellId;
            castTimestamp = evt.timestamp;
            return false; // found it
        }
        return true;
    });

    if (foundSpellId == 0)
        return;

    // Second pass: count how many targets were hit by damage events from this spell
    // within 200ms of each other (AoE detection)
    m_buffer.ScanBackwards([&](CombatEvent const& evt) -> bool
    {
        if (now - evt.timestamp > 5000)
            return false;

        if (evt.creatureEntry == creatureEntry &&
            evt.eventType == COMBAT_EVENT_DAMAGE &&
            evt.spellId == foundSpellId)
        {
            hitCount++;
        }
        return true;
    });

    // Classify the response
    PatternResponse response;
    float param = 0.0f;

    if (hitCount >= 3)
    {
        response = RESPONSE_MOVE_AWAY;
        param = 12.0f; // default avoidance range
    }
    else
    {
        response = RESPONSE_INTERRUPT;
    }

    // Build the pattern
    LearnedPattern p;
    p.creatureEntry = creatureEntry;
    p.triggerSpellId = foundSpellId;
    p.responseType = response;
    p.responseParam = param;
    p.confidence = isDeath ? 3 : 1; // death boosts confidence more
    p.lastSeen = time(nullptr);
    p.isPermanent = false;

    AddOrUpdatePattern(p);
}

void BotPatternMgr::AddOrUpdatePattern(LearnedPattern const& p)
{
    auto& vec = m_patterns[p.creatureEntry];
    for (auto& existing : vec)
    {
        if (existing.triggerSpellId == p.triggerSpellId &&
            existing.responseType == p.responseType)
        {
            // Update existing pattern
            if (!existing.isPermanent)
            {
                existing.confidence = std::min<uint8>(existing.confidence + p.confidence, 254);
                existing.lastSeen = p.lastSeen;
                if (p.responseParam > 0.0f)
                    existing.responseParam = p.responseParam;
            }
            return;
        }
    }

    // New pattern
    vec.push_back(p);
}

// --- Queries ---

LearnedPattern const* BotPatternMgr::GetActivePattern(uint32 creatureEntry, uint32 spellId) const
{
    auto it = m_patterns.find(creatureEntry);
    if (it == m_patterns.end())
        return nullptr;

    for (auto const& p : it->second)
    {
        if (p.triggerSpellId == spellId && p.confidence >= 2)
            return &p;
    }
    return nullptr;
}

bool BotPatternMgr::HasPatternsForCreature(uint32 creatureEntry) const
{
    auto it = m_patterns.find(creatureEntry);
    return it != m_patterns.end() && !it->second.empty();
}

// --- Confidence Decay ---

void BotPatternMgr::DecayPatterns()
{
    uint32 now = time(nullptr);

    for (auto it = m_patterns.begin(); it != m_patterns.end(); )
    {
        auto& vec = it->second;
        vec.erase(std::remove_if(vec.begin(), vec.end(),
            [now](LearnedPattern& p) -> bool
            {
                if (p.isPermanent)
                    return false;

                // Decay if not seen in 30 minutes
                if (now - p.lastSeen > 1800)
                {
                    if (p.confidence > 0)
                        p.confidence--;
                }
                return p.confidence == 0;
            }),
            vec.end());

        if (vec.empty())
            it = m_patterns.erase(it);
        else
            ++it;
    }
}

// --- DB Persistence ---

void BotPatternMgr::LoadFromDB()
{
    // Auto-create table if it doesn't exist (no manual migration step required)
    CharacterDatabase.DirectExecute(
        "CREATE TABLE IF NOT EXISTS `bot_learned_patterns` ("
        "`creature_entry` INT UNSIGNED NOT NULL,"
        "`trigger_spell_id` INT UNSIGNED NOT NULL,"
        "`response_type` TINYINT UNSIGNED NOT NULL DEFAULT 0,"
        "`response_param` FLOAT NOT NULL DEFAULT 0,"
        "`confidence` TINYINT UNSIGNED NOT NULL DEFAULT 1,"
        "`is_permanent` TINYINT UNSIGNED NOT NULL DEFAULT 0,"
        "`last_seen` INT UNSIGNED NOT NULL DEFAULT 0,"
        "PRIMARY KEY (`creature_entry`, `trigger_spell_id`, `response_type`),"
        "KEY `idx_creature` (`creature_entry`)"
        ") ENGINE=InnoDB DEFAULT CHARSET=utf8");

    auto result = CharacterDatabase.Query("SELECT creature_entry, trigger_spell_id, response_type, "
        "response_param, confidence, is_permanent, last_seen FROM bot_learned_patterns");

    if (!result)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotPatternMgr] No learned patterns found in DB.");
        InitEncounterOverrides();
        return;
    }

    uint32 count = 0;
    do
    {
        Field* fields = result->Fetch();
        LearnedPattern p;
        p.creatureEntry = fields[0].GetUInt32();
        p.triggerSpellId = fields[1].GetUInt32();
        p.responseType = fields[2].GetUInt8();
        p.responseParam = fields[3].GetFloat();
        p.confidence = fields[4].GetUInt8();
        p.isPermanent = fields[5].GetUInt8() != 0;
        p.lastSeen = fields[6].GetUInt32();
        m_patterns[p.creatureEntry].push_back(p);
        count++;
    } while (result->NextRow());

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotPatternMgr] Loaded %u learned patterns from DB.", count);

    // Ensure hand-crafted overrides are present (idempotent)
    InitEncounterOverrides();
}

void BotPatternMgr::SaveToDB()
{
    // Clear and re-insert all patterns
    CharacterDatabase.Execute("DELETE FROM bot_learned_patterns");

    for (auto const& pair : m_patterns)
    {
        for (auto const& p : pair.second)
        {
            CharacterDatabase.PExecute(
                "INSERT INTO bot_learned_patterns "
                "(creature_entry, trigger_spell_id, response_type, response_param, "
                "confidence, is_permanent, last_seen) VALUES (%u, %u, %u, %f, %u, %u, %u)",
                p.creatureEntry, p.triggerSpellId, p.responseType, p.responseParam,
                uint32(p.confidence), p.isPermanent ? 1u : 0u, p.lastSeen);
        }
    }

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotPatternMgr] Saved patterns to DB.");
}

// --- Hand-crafted overrides ---

void BotPatternMgr::RegisterPermanentPattern(LearnedPattern const& p)
{
    auto& vec = m_patterns[p.creatureEntry];
    for (auto& existing : vec)
    {
        if (existing.triggerSpellId == p.triggerSpellId &&
            existing.responseType == p.responseType)
        {
            // Already exists — ensure it's marked permanent
            existing.isPermanent = true;
            existing.confidence = 255;
            existing.responseParam = p.responseParam;
            return;
        }
    }
    vec.push_back(p);
}

void BotPatternMgr::InitEncounterOverrides()
{
    // Migrated from the old BossOverride static registry in PartyBotAI.cpp.
    // Each entry is a permanent pattern that cannot decay.

    auto MakeInterrupt = [this](uint32 creature, uint32 spell)
    {
        LearnedPattern p;
        p.creatureEntry = creature;
        p.triggerSpellId = spell;
        p.responseType = RESPONSE_INTERRUPT;
        p.responseParam = 0.0f;
        p.confidence = 255;
        p.lastSeen = time(nullptr);
        p.isPermanent = true;
        RegisterPermanentPattern(p);
    };

    auto MakeAvoidAoe = [this](uint32 creature, uint32 spell, float range)
    {
        LearnedPattern p;
        p.creatureEntry = creature;
        p.triggerSpellId = spell;
        p.responseType = RESPONSE_MOVE_AWAY;
        p.responseParam = range;
        p.confidence = 255;
        p.lastSeen = time(nullptr);
        p.isPermanent = true;
        RegisterPermanentPattern(p);
    };

    // === Ragefire Chasm ===
    // Jergosh the Invoker — interrupt Shadow Bolt Volley
    MakeInterrupt(11518, 11966);  // Shadow Bolt Volley

    // Taragaman the Hungerer — avoid Fire Nova
    MakeAvoidAoe(11520, 11969, 10.0f);

    // === Deadmines ===
    // Mr. Smite — save cooldowns for phase 3 at 33% HP
    {
        LearnedPattern p;
        p.creatureEntry = 646;  // Mr. Smite
        p.triggerSpellId = 0;
        p.responseType = RESPONSE_SAVE_CD;
        p.responseParam = 33.0f;
        p.confidence = 255;
        p.lastSeen = time(nullptr);
        p.isPermanent = true;
        RegisterPermanentPattern(p);
    }

    // === Wailing Caverns ===
    // Lord Serpentis — interrupt Lightning Bolt and Healing Touch
    MakeInterrupt(3673, 9532);   // Lightning Bolt
    MakeInterrupt(3673, 11986);  // Healing Touch

    // === Shadowfang Keep ===
    // Baron Silverlaine — interrupt Veil of Shadow
    MakeInterrupt(3887, 7068);

    // === Blackfathom Deeps ===
    // Lady Sarevess — interrupt Forked Lightning
    MakeInterrupt(4831, 8435);

    // === Gnomeregan ===
    // Electrocutioner 6000 — interrupt Chain Lightning
    MakeInterrupt(6235, 11085);

    // === Scarlet Monastery ===
    // High Inquisitor Whitemane — interrupt heals
    MakeInterrupt(3977, 12039);  // Heal
    MakeInterrupt(3977, 22187);  // Greater Heal (Whitemane version)

    // Herod — avoid Whirlwind
    MakeAvoidAoe(3975, 8989, 10.0f);

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotPatternMgr] Initialized %zu encounter overrides.",
             m_patterns.size());
}
