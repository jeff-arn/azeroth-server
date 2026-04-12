#ifndef MANGOS_BOT_BEHAVIOR_H
#define MANGOS_BOT_BEHAVIOR_H

#include "Common.h"
#include <vector>
#include <memory>

struct SituationContext;
class BotSpellKit;
class Player;

// Base class for all bot behavior modules.
// Each behavior represents one tactical concern (interrupt, kite, heal, etc.)
// and knows how to score its own urgency against the current situation.
class BotBehavior
{
public:
    virtual ~BotBehavior() = default;

    // How important is this behavior right now? 0 = irrelevant, 100 = life-or-death.
    // Called every tick. Must be fast.
    virtual float Score(SituationContext const& ctx) const = 0;

    // Attempt to execute the behavior. Returns true if an action was taken
    // (spell cast, movement started), consuming the tick.
    virtual bool Execute(SituationContext const& ctx, BotSpellKit& kit) = 0;

    // Behavior identity for debugging / logging.
    virtual char const* GetName() const = 0;
};

// Holds a set of behaviors and their role-specific weight multipliers.
// Runs scoring each tick and executes the winning behavior.
class BotBehaviorSet
{
public:
    struct Entry
    {
        std::unique_ptr<BotBehavior> behavior;
        float roleWeight;
    };

    void Add(std::unique_ptr<BotBehavior> b, float roleWeight = 1.0f);

    // Score all behaviors, execute the highest-scoring one.
    // Returns true if any behavior executed (consumed the tick).
    bool RunBestBehavior(SituationContext const& ctx, BotSpellKit& kit);

    // Number of registered behaviors.
    size_t Size() const { return m_entries.size(); }

    // Debug: set to a Player* to enable per-tick logging for that bot.
    // When non-null, logs all non-zero scores (LOG_LVL_DETAIL) and
    // the winning behavior + result (LOG_LVL_MINIMAL) each tick.
    // Set via .botdebug command or programmatically.
    void SetDebugTarget(Player* p) { m_debugTarget = p; }
    Player* GetDebugTarget() const { return m_debugTarget; }

private:
    std::vector<Entry> m_entries;
    Player* m_debugTarget = nullptr;
};

#endif
