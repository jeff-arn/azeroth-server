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
    float lowestAllyHealthPct = 100.0f;
    Unit* mostInjuredAlly = nullptr;
    uint8 alivePartyMembers = 0;
    uint8 deadPartyMembers = 0;
    bool anyAllyNeedsHeal = false;      // anyone < 70%
    bool criticalHealNeeded = false;    // anyone < 30%
    bool healerAvailable = false;

    // Encounter
    LearnedPattern const* activePattern = nullptr;
    bool isBossEncounter = false;

    // Role (cached from bot)
    uint8 role = 0; // CombatBotRoles value
};

// Fills a SituationContext from the bot's current state.
// Must be called once per AI tick, before behavior scoring.
void EvaluateSituation(CombatBotBaseAI* ai, SituationContext& ctx);

#endif
