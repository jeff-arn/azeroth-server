#ifndef MANGOS_BOT_ROLE_CONFIG_H
#define MANGOS_BOT_ROLE_CONFIG_H

#include <memory>
#include "BotBehavior.h"
#include "SharedDefines.h"

class CombatBotBaseAI;
class PartyBotAI;

// Factory: builds a BotBehaviorSet configured with the right subset
// of shared + class-unique behaviors and role-specific weights.
// inCombat = true for combat behaviors, false for OOC behaviors.
std::unique_ptr<BotBehaviorSet> BuildBehaviorSet(uint8 classId, CombatBotRoles role, bool inCombat, CombatBotBaseAI* ai, PartyBotAI* partyAI);

#endif
