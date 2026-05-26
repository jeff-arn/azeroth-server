#ifndef MANGOS_BOT_SITUATION_CONTEXT_H
#define MANGOS_BOT_SITUATION_CONTEXT_H

#include "Common.h"
#include "SharedDefines.h"

class Unit;
class CombatBotBaseAI;
struct LearnedPattern;

struct SituationContext
{
    // Self
    float healthPct = 100.0f;
    float manaPct = 100.0f;
    float ragePct = 0.0f;
    float energyPct = 100.0f;
    uint8 powerType = 0; // Powers enum value
    bool isBeingMeleed = false;
    uint8 meleeAttackerCount = 0;
    ShapeshiftForm currentForm = FORM_NONE;
    uint8 comboPoints = 0;
    uint8 playerClass = 0;
    bool hasShieldEquipped = false;
    bool isCasting = false;
    bool isMoving = false;

    // Threat
    float threatRatio = 0.0f;
    bool shouldThrottle = false;

    // Target
    Unit* currentTarget = nullptr;
    float targetHealthPct = 100.0f;
    float distanceToTarget = 0.0f;
    bool targetIsCasting = false;
    uint32 targetCastSpellId = 0;
    uint32 targetCreatureEntry = 0;
    bool targetIsPlayer = false;

    // Party
    float tankHealthPct = 100.0f;
    bool tankIsAlive = false;
    bool hasTank = false;
    Unit* tankUnit = nullptr;
    float lowestAllyHealthPct = 100.0f;
    Unit* mostInjuredAlly = nullptr;
    uint8 alivePartyMembers = 0;
    uint8 deadPartyMembers = 0;
    bool anyAllyNeedsHeal = false; // tank < 80% OR non-tank < 50%
    bool tankNeedsHeal = false; // tank < 80%
    bool criticalHealNeeded = false; // anyone < 30%
    bool healerAvailable = false;
    bool hasLooseAdds = false; // party has un-CC'd mobs that aren't the tank's target

    // Encounter
    LearnedPattern const* activePattern = nullptr;
    bool isBossEncounter = false;

    // Role (cached from bot)
    uint8 role = 0; // CombatBotRoles value

    // Formation (populated when formation system is active)
    float formationTargetX = 0.0f;
    float formationTargetY = 0.0f;
    float formationTargetZ = 0.0f;
    float distToFormationPos = 0.0f;
    bool hasFormationTarget = false;
    float fleeTargetX = 0.0f;
    float fleeTargetY = 0.0f;
    float fleeTargetZ = 0.0f;
    bool hasFleeTarget = false;
};

// Fills a SituationContext from the bot's current state.
// Must be called once per AI tick, before behavior scoring.
void EvaluateSituation(CombatBotBaseAI* ai, SituationContext& ctx);

#endif
