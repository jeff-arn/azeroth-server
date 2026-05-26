#ifndef MANGOS_BOT_COMBAT_EVENT_BUFFER_H
#define MANGOS_BOT_COMBAT_EVENT_BUFFER_H

#include <mutex>
#include <unordered_map>
#include <vector>
#include "Common.h"

// ============================================================
// Combat Event Ring Buffer + Pattern Learning System
// ============================================================

enum CombatEventType : uint8
{
    COMBAT_EVENT_DAMAGE = 0,
    COMBAT_EVENT_CAST_START = 1,
    COMBAT_EVENT_DEATH = 2,
};

struct CombatEvent
{
    uint32 timestamp; // WorldTimer::getMSTime()
    uint8 eventType; // CombatEventType
    uint64 sourceGuid;
    uint64 targetGuid;
    uint32 spellId;
    uint32 damage;
    uint32 creatureEntry; // Source creature entry (0 for players)
};

class CombatEventBuffer
{
public:
    void Push(CombatEvent const& e);

    // Scan backwards from newest event, calling visitor(CombatEvent const&)
    // until visitor returns false or all 256 entries are visited.
    template <typename Fn>
    void ScanBackwards(Fn&& visitor) const
    {
        uint8 idx = m_writeIdx;
        for (uint16 i = 0; i < 256; ++i)
        {
            idx--; // wraps naturally for uint8
            if (m_events[idx].timestamp == 0)
                break; // uninitialized slot
            if (!visitor(m_events[idx]))
                break;
        }
    }

private:
    CombatEvent m_events[256] = {};
    uint8 m_writeIdx = 0;
};

// ============================================================
// Learned Patterns
// ============================================================

enum PatternResponse : uint8
{
    RESPONSE_INTERRUPT = 0,
    RESPONSE_MOVE_AWAY = 1,
    RESPONSE_SPREAD = 2,
    RESPONSE_SAVE_CD = 3,
    // Future extensibility:
    RESPONSE_MOVE_TO_POSITION = 4, // safety dance waypoints
    RESPONSE_STACK = 5, // stack for shared damage
    RESPONSE_SWAP_SIDE = 6, // polarity
};

struct LearnedPattern
{
    uint32 creatureEntry;
    uint32 triggerSpellId;
    uint8 responseType; // PatternResponse
    float responseParam; // e.g. avoidance range in yards
    uint8 confidence; // 0-254 = learned, 255 = permanent (hand-crafted)
    uint32 lastSeen; // unix timestamp
    bool isPermanent; // hand-crafted override, cannot decay
};

// ============================================================
// Pattern Manager Singleton
// ============================================================

class BotPatternMgr
{
public:
    static BotPatternMgr& Instance();

    // --- Recording (called from hooks in Unit.cpp / Spell.cpp) ---
    void RecordDamage(Unit* source, Unit* victim, uint32 damage, uint32 spellId, uint32 mapId, uint32 creatureEntry);
    void RecordCastStart(Unit* caster, Unit* target, SpellEntry const* spell, uint32 mapId, uint32 creatureEntry);
    void RecordDeath(Unit* killer, Unit* victim, uint32 mapId, uint32 creatureEntry);

    // --- Queries (called from EvaluateSituation) ---
    LearnedPattern const* GetActivePattern(uint32 creatureEntry, uint32 spellId) const;
    bool HasPatternsForCreature(uint32 creatureEntry) const;

    // --- Confidence management ---
    void DecayPatterns(); // called periodically (~5 min)

    // --- DB persistence ---
    void LoadFromDB();
    void SaveToDB();

    // --- Hand-crafted overrides ---
    void RegisterPermanentPattern(LearnedPattern const& p);
    void InitEncounterOverrides(); // migrates boss override registry

    // --- Activation (only process events when party bots exist) ---
    void SetActive(bool active) { m_active = active; }
    bool IsActive() const { return m_active; }

private:
    BotPatternMgr() = default;

    void DetectPatterns(uint32 creatureEntry, uint64 victimGuid, uint32 damage);
    void AddOrUpdatePattern(LearnedPattern const& p);

    CombatEventBuffer m_buffer;
    std::unordered_map<uint32 /*creatureEntry*/, std::vector<LearnedPattern>> m_patterns;
    bool m_active = false;
};

#define sBotPatternMgr BotPatternMgr::Instance()

#endif
