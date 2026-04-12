#include "BotRoleConfig.h"
#include "BotBehaviors_Shared.h"
#include "BotBehaviors_ClassUnique.h"
#include "CombatBotBaseAI.h"
#include "PartyBotAI.h"

// Role weight presets: Tank / Healer / Melee DPS / Ranged DPS
static float GetWeight(CombatBotRoles role, float tank, float healer, float melee, float ranged)
{
    switch (role)
    {
        case ROLE_TANK:       return tank;
        case ROLE_HEALER:     return healer;
        case ROLE_MELEE_DPS:  return melee;
        case ROLE_RANGE_DPS:  return ranged;
        default:              return melee; // fallback
    }
}

std::unique_ptr<BotBehaviorSet> BuildBehaviorSet(
    uint8 classId, CombatBotRoles role, bool inCombat,
    CombatBotBaseAI* ai, PartyBotAI* partyAI)
{
    auto set = std::make_unique<BotBehaviorSet>();

    if (inCombat)
    {
        // === Shared combat behaviors ===
        set->Add(std::make_unique<BehaviorSelfPreserve>(ai),     GetWeight(role, 1.0f, 1.2f, 1.0f, 1.0f));
        set->Add(std::make_unique<BehaviorInterrupt>(ai),        GetWeight(role, 1.0f, 0.8f, 1.0f, 1.0f));

        if (partyAI)
        {
            set->Add(std::make_unique<BehaviorTaunt>(partyAI),   GetWeight(role, 1.5f, 0.0f, 0.0f, 0.0f));
            set->Add(std::make_unique<BehaviorCrowdControl>(partyAI), GetWeight(role, 0.3f, 0.5f, 0.8f, 1.0f));
            set->Add(std::make_unique<BehaviorKite>(partyAI),    GetWeight(role, 0.0f, 1.0f, 0.0f, 1.2f));
            set->Add(std::make_unique<BehaviorDispel>(partyAI),  GetWeight(role, 0.8f, 1.2f, 0.5f, 0.5f));
        }

        set->Add(std::make_unique<BehaviorHealParty>(ai),        GetWeight(role, 0.3f, 1.5f, 0.0f, 0.0f));
        set->Add(std::make_unique<BehaviorEmergencyHeal>(ai),    GetWeight(role, 0.0f, 0.0f, 0.5f, 0.5f));
        set->Add(std::make_unique<BehaviorTankThreat>(ai),       GetWeight(role, 1.5f, 0.0f, 0.0f, 0.0f));
        set->Add(std::make_unique<BehaviorThreatDump>(ai),       GetWeight(role, 0.0f, 1.0f, 1.2f, 1.2f));
        set->Add(std::make_unique<BehaviorExecuteTarget>(ai),    GetWeight(role, 1.0f, 0.0f, 1.2f, 1.0f));
        set->Add(std::make_unique<BehaviorApplyDots>(ai),        GetWeight(role, 0.3f, 0.0f, 0.8f, 1.0f));
        set->Add(std::make_unique<BehaviorAoeDamage>(ai),        GetWeight(role, 0.5f, 0.0f, 1.0f, 1.0f));
        set->Add(std::make_unique<BehaviorDirectDamage>(ai),     GetWeight(role, 0.5f, 0.2f, 1.0f, 1.0f));
        set->Add(std::make_unique<BehaviorSelfBuff>(ai),         GetWeight(role, 0.8f, 0.8f, 0.8f, 0.8f));
        set->Add(std::make_unique<BehaviorPosition>(ai),         GetWeight(role, 0.8f, 0.8f, 0.8f, 1.0f));

        if (partyAI)
            set->Add(std::make_unique<BehaviorWandFiller>(partyAI), GetWeight(role, 0.0f, 1.0f, 0.0f, 0.0f));

        // === Class-unique combat behaviors ===
        switch (classId)
        {
            case CLASS_WARRIOR:
                set->Add(std::make_unique<BehaviorWarriorStanceDance>(ai), 1.0f);
                set->Add(std::make_unique<BehaviorWarriorRageDump>(ai), 1.0f);
                break;
            case CLASS_ROGUE:
                set->Add(std::make_unique<BehaviorRogueStealthOpener>(ai), 1.0f);
                set->Add(std::make_unique<BehaviorRogueComboFinisher>(ai), 1.0f);
                break;
            case CLASS_HUNTER:
                set->Add(std::make_unique<BehaviorHunterPetManage>(ai), 1.0f);
                set->Add(std::make_unique<BehaviorHunterAspectSwap>(ai), 1.0f);
                if (partyAI)
                    set->Add(std::make_unique<BehaviorHunterDeadZone>(partyAI), 1.0f);
                break;
            case CLASS_PALADIN:
                set->Add(std::make_unique<BehaviorPaladinSealJudge>(ai), 1.0f);
                break;
            case CLASS_SHAMAN:
                set->Add(std::make_unique<BehaviorShamanTotemManage>(ai), 1.0f);
                break;
            case CLASS_PRIEST:
                set->Add(std::make_unique<BehaviorPriestShadowformToggle>(ai), 1.0f);
                if (partyAI)
                    set->Add(std::make_unique<BehaviorPriestShieldLogic>(partyAI), 1.0f);
                break;
            case CLASS_WARLOCK:
                if (partyAI)
                    set->Add(std::make_unique<BehaviorWarlockPetSummon>(partyAI), 1.0f);
                set->Add(std::make_unique<BehaviorWarlockCurseManage>(ai), 1.0f);
                set->Add(std::make_unique<BehaviorWarlockLifeTap>(ai), 1.0f);
                break;
            case CLASS_DRUID:
                set->Add(std::make_unique<BehaviorDruidFormShift>(ai), 1.0f);
                if (partyAI)
                    set->Add(std::make_unique<BehaviorDruidRebirth>(partyAI), 1.0f);
                set->Add(std::make_unique<BehaviorDruidInnervate>(ai), 1.0f);
                break;
        }
    }
    else
    {
        // === OOC behaviors ===
        // Buffing, pet management, weapon maintenance
        set->Add(std::make_unique<BehaviorSelfBuff>(ai), 1.0f);
        set->Add(std::make_unique<BehaviorPartyBuff>(ai), 1.0f);
        set->Add(std::make_unique<BehaviorHealParty>(ai), GetWeight(role, 0.3f, 1.5f, 0.0f, 0.0f));
        set->Add(std::make_unique<BehaviorPosition>(ai), 0.5f);

        if (partyAI)
            set->Add(std::make_unique<BehaviorDispel>(partyAI), 1.0f);

        switch (classId)
        {
            case CLASS_ROGUE:
                set->Add(std::make_unique<BehaviorRoguePoisonWeapons>(ai), 1.0f);
                if (partyAI)
                {
                    set->Add(std::make_unique<BehaviorRogueStealth>(partyAI), 1.0f);
                    set->Add(std::make_unique<BehaviorRogueStealthOpener>(ai), 1.0f);
                }
                break;
            case CLASS_HUNTER:
                set->Add(std::make_unique<BehaviorHunterPetManage>(ai), 1.0f);
                set->Add(std::make_unique<BehaviorHunterAspectSwap>(ai), 1.0f);
                break;
            case CLASS_PALADIN:
                set->Add(std::make_unique<BehaviorPaladinBlessing>(ai), 1.0f);
                break;
            case CLASS_WARLOCK:
                if (partyAI)
                    set->Add(std::make_unique<BehaviorWarlockPetSummon>(partyAI), 1.0f);
                break;
            case CLASS_DRUID:
                set->Add(std::make_unique<BehaviorDruidFormShift>(ai), 1.0f);
                if (partyAI)
                    set->Add(std::make_unique<BehaviorDruidRebirth>(partyAI), 1.0f);
                set->Add(std::make_unique<BehaviorDruidInnervate>(ai), 1.0f);
                break;
            case CLASS_PRIEST:
                set->Add(std::make_unique<BehaviorPriestShadowformToggle>(ai), 1.0f);
                break;
        }
    }

    return set;
}
