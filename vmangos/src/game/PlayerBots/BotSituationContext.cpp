#include "BotSituationContext.h"
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
    ctx.lowestAllyHealthPct = 100.0f;
    ctx.mostInjuredAlly = nullptr;
    ctx.alivePartyMembers = 0;
    ctx.deadPartyMembers = 0;
    ctx.anyAllyNeedsHeal = false;
    ctx.criticalHealNeeded = false;
    ctx.healerAvailable = false;

    if (partyAI)
    {
        if (Player* pTank = partyAI->FindPartyTank())
        {
            ctx.hasTank = true;
            ctx.tankIsAlive = pTank->IsAlive();
            ctx.tankHealthPct = pTank->IsAlive() ? pTank->GetHealthPercent() : 0.0f;
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

                if (hp < 70.0f && pMember != me)
                    ctx.anyAllyNeedsHeal = true;
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
}
