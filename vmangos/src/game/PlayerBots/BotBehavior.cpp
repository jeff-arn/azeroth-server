#include "BotBehavior.h"
#include "BotSituationContext.h"
#include "BotSpellKit.h"
#include "CombatBotBaseAI.h"
#include "Log.h"
#include "Player.h"

void BotBehaviorSet::Add(std::unique_ptr<BotBehavior> b, float roleWeight)
{
    Entry e;
    e.behavior = std::move(b);
    e.roleWeight = roleWeight;
    m_entries.push_back(std::move(e));
}

bool BotBehaviorSet::RunBestBehavior(SituationContext const& ctx, BotSpellKit& kit)
{
    // Score all behaviors and sort by weighted score descending.
    // Then try each in order until one succeeds.
    struct Candidate
    {
        BotBehavior* behavior;
        float score;
    };

    // Use a small inline vector — typically < 20 behaviors
    Candidate candidates[32];
    uint8 count = 0;

    for (auto const& entry : m_entries)
    {
        if (entry.roleWeight <= 0.0f)
            continue;

        float raw = entry.behavior->Score(ctx);
        if (raw <= 0.0f)
            continue;

        float weighted = raw * entry.roleWeight;

        if (m_debugTarget)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotAI] %s | %-20s raw=%.1f weight=%.1f → %.1f", m_debugTarget->GetName(), entry.behavior->GetName(), raw, entry.roleWeight, weighted);
        }

        if (count < 32)
        {
            candidates[count].behavior = entry.behavior.get();
            candidates[count].score = weighted;
            count++;
        }
    }

    if (count == 0)
    {
        if (m_debugTarget)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotAI] %s → NO BEHAVIOR SCORED (all zero) target=%s hp=%.0f%% mana=%.0f%% role=%u inCombat=%u", m_debugTarget->GetName(), ctx.currentTarget ? "yes" : "NO", ctx.healthPct, ctx.manaPct, ctx.role, m_debugTarget->IsInCombat() ? 1u : 0u);
        }
        return false;
    }

    // Sort descending by score
    std::sort(candidates, candidates + count, [](Candidate const& a, Candidate const& b) { return a.score > b.score; });

    // Try each in order until one succeeds
    for (uint8 i = 0; i < count; ++i)
    {
        bool result = candidates[i].behavior->Execute(ctx, kit);

        if (m_debugTarget)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[BotAI] %s → %s (%.1f) %s", m_debugTarget->GetName(), candidates[i].behavior->GetName(), candidates[i].score, result ? "OK" : "FAILED");
        }

        if (result)
            return true;
    }

    return false;
}
