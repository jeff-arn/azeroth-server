#include "BotSituationContext.h"
#include "BotFormation.h"
#include "BotCombatEventBuffer.h"
#include "CombatBotBaseAI.h"
#include "PartyBotAI.h"
#include "Player.h"
#include "Group.h"
#include "Creature.h"
#include "Spell.h"

void EvaluateSituation(CombatBotBaseAI* ai, SituationContext& ctx)
{
    Player* me = ai->me;
    if (!me)
        return;

    PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(ai);

    // === Self ===
    ctx.healthPct = me->GetHealthPercent();
    ctx.manaPct = me->GetPowerPercent(POWER_MANA);
    ctx.ragePct = (me->GetPowerType() == POWER_RAGE) ? (me->GetPower(POWER_RAGE) * 100.0f / me->GetMaxPower(POWER_RAGE)) : 0.0f;
    ctx.energyPct = (me->GetPowerType() == POWER_ENERGY) ? (me->GetPower(POWER_ENERGY) * 100.0f / me->GetMaxPower(POWER_ENERGY)) : 0.0f;
    ctx.powerType = me->GetPowerType();
    ctx.meleeAttackerCount = ai->GetAttackersInRangeCount(10.0f);
    ctx.isBeingMeleed = ctx.meleeAttackerCount > 0;
    ctx.currentForm = me->GetShapeshiftForm();
    ctx.comboPoints = me->GetComboPoints();
    ctx.playerClass = me->GetClass();
    ctx.hasShieldEquipped = ai->IsWearingShield(me);
    ctx.isCasting = me->IsNonMeleeSpellCasted(false, false, true);
    ctx.isMoving = me->IsMoving();
    ctx.role = ai->m_role;

    // === Target ===
    ctx.currentTarget = me->GetVictim();
    if (ctx.currentTarget)
    {
        ctx.targetHealthPct = ctx.currentTarget->GetHealthPercent();
        ctx.distanceToTarget = me->GetCombatDistance(ctx.currentTarget);
        ctx.targetIsPlayer = ctx.currentTarget->IsPlayer();
        ctx.targetCreatureEntry = 0;

        if (Creature* pCreature = ctx.currentTarget->ToCreature())
            ctx.targetCreatureEntry = pCreature->GetEntry();

        // Check if target is casting an interruptible spell
        ctx.targetIsCasting = false;
        ctx.targetCastSpellId = 0;
        if (Spell* pSpell = ctx.currentTarget->GetCurrentSpell(CURRENT_GENERIC_SPELL))
        {
            if (pSpell->m_spellInfo)
            {
                ctx.targetIsCasting = true;
                ctx.targetCastSpellId = pSpell->m_spellInfo->Id;
            }
        }
        else if (Spell* pChannel = ctx.currentTarget->GetCurrentSpell(CURRENT_CHANNELED_SPELL))
        {
            if (pChannel->m_spellInfo)
            {
                ctx.targetIsCasting = true;
                ctx.targetCastSpellId = pChannel->m_spellInfo->Id;
            }
        }

        // === Threat ===
        if (partyAI)
        {
            ctx.threatRatio = partyAI->GetThreatRatio(ctx.currentTarget);
            ctx.shouldThrottle = partyAI->ShouldThrottleDPS(ctx.currentTarget);
        }
    }

    // === Party ===
    ctx.hasTank = false;
    ctx.tankIsAlive = false;
    ctx.tankHealthPct = 0.0f;
    ctx.tankUnit = nullptr;
    ctx.lowestAllyHealthPct = 100.0f;
    ctx.mostInjuredAlly = nullptr;
    ctx.alivePartyMembers = 0;
    ctx.deadPartyMembers = 0;
    ctx.anyAllyNeedsHeal = false;
    ctx.tankNeedsHeal = false;
    ctx.criticalHealNeeded = false;
    ctx.healerAvailable = false;

    Player* pTankPlayer = nullptr;
    if (partyAI)
    {
        pTankPlayer = partyAI->FindPartyTank();
        if (pTankPlayer)
        {
            ctx.hasTank = true;
            ctx.tankIsAlive = pTankPlayer->IsAlive();
            ctx.tankHealthPct = pTankPlayer->IsAlive() ? pTankPlayer->GetHealthPercent() : 0.0f;
            ctx.tankUnit = pTankPlayer;
            if (ctx.tankIsAlive && ctx.tankHealthPct < 80.0f)
                ctx.tankNeedsHeal = true;
        }
    }

    if (Group* pGroup = me->GetGroup())
    {
        for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                if (!pMember->IsAlive())
                {
                    ctx.deadPartyMembers++;
                    continue;
                }

                ctx.alivePartyMembers++;

                float hp = pMember->GetHealthPercent();
                if (hp < ctx.lowestAllyHealthPct && pMember != me)
                {
                    ctx.lowestAllyHealthPct = hp;
                    ctx.mostInjuredAlly = pMember;
                }

                // Role-aware heal thresholds: tank is healed more aggressively.
                if (pMember != me)
                {
                    bool isTank = (pTankPlayer && pMember == pTankPlayer);
                    if (isTank && hp < 80.0f)
                        ctx.anyAllyNeedsHeal = true;
                    else if (!isTank && hp < 50.0f)
                        ctx.anyAllyNeedsHeal = true;
                }
                if (hp < 30.0f && pMember != me)
                    ctx.criticalHealNeeded = true;

                // Check if a healer is available (alive with mana)
                if (pMember != me &&
                    CombatBotBaseAI::IsHealerClass(pMember->GetClass()) &&
                    pMember->GetPowerPercent(POWER_MANA) > 10.0f)
                {
                    ctx.healerAvailable = true;
                }
            }
        }
    }

    // Detect loose adds: mobs attacking party that aren't the tank's target
    // and aren't already crowd-controlled.
    ctx.hasLooseAdds = false;
    if (Group* pGroup = me->GetGroup())
    {
        Unit* pTankTarget = (pTankPlayer && pTankPlayer->IsAlive()) ? pTankPlayer->GetVictim() : nullptr;
        for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr && !ctx.hasLooseAdds; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                for (const auto pAttacker : pMember->GetAttackers())
                {
                    if (pAttacker != pTankTarget &&
                        !pAttacker->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL))
                    {
                        ctx.hasLooseAdds = true;
                        break;
                    }
                }
            }
        }
    }

    // === Encounter ===
    ctx.isBossEncounter = false;
    ctx.activePattern = nullptr;

    if (ctx.targetCreatureEntry > 0)
    {
        if (ctx.targetIsCasting && ctx.targetCastSpellId)
            ctx.activePattern = sBotPatternMgr.GetActivePattern(ctx.targetCreatureEntry, ctx.targetCastSpellId);

        // Check boss status from creature rank or pattern existence
        if (Creature* pCreature = ctx.currentTarget ? ctx.currentTarget->ToCreature() : nullptr)
        {
            if (pCreature->GetCreatureInfo() && pCreature->GetCreatureInfo()->rank >= 3)
                ctx.isBossEncounter = true;
        }

        if (sBotPatternMgr.HasPatternsForCreature(ctx.targetCreatureEntry))
            ctx.isBossEncounter = true;
    }

    // === Formation ===
    ctx.hasFormationTarget = false;
    ctx.hasFleeTarget = false;

    if (partyAI && partyAI->m_formationSlotIndex >= 0)
    {
        FormationState* fs = sBotFormationMgr.GetFormation(partyAI->m_leaderGuid);
        if (fs && !fs->trail.IsEmpty())
        {
            float fx, fy, fz;
            if (fs->CalculateSlotPosition(partyAI->m_formationSlotIndex, me->GetMap(), me,
                                          fx, fy, fz))
            {
                ctx.formationTargetX = fx;
                ctx.formationTargetY = fy;
                ctx.formationTargetZ = fz;
                ctx.distToFormationPos = me->GetDistance(fx, fy, fz);
                ctx.hasFormationTarget = true;
            }

            // Precompute flee point only when being meleed (avoid costly hostile scans).
            if (ctx.isBeingMeleed)
            {
                float flX, flY, flZ;
                if (fs->CalculateFleePoint(me, me->GetMap(), flX, flY, flZ))
                {
                    ctx.fleeTargetX = flX;
                    ctx.fleeTargetY = flY;
                    ctx.fleeTargetZ = flZ;
                    ctx.hasFleeTarget = true;
                }
            }
        }
    }
}
