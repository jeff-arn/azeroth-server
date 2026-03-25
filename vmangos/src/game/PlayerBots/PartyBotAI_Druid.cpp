/*
* This program is free software; you can redistribute it and/or modify
* it under the terms of the GNU General Public License as published by
* the Free Software Foundation; either version 2 of the License, or
* (at your option) any later version.
*
* This program is distributed in the hope that it will be useful,
* but WITHOUT ANY WARRANTY; without even the implied warranty of
* MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
* GNU General Public License for more details.
*
* You should have received a copy of the GNU General Public License
* along with this program; if not, write to the Free Software
* Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA
*/

#include "PartyBotAI.h"
#include "Player.h"

bool PartyBotAI::EnterCombatDruidForm()
{
    if (m_spells.druid.pCatForm &&
        GetRole() == ROLE_MELEE_DPS &&
        CanTryToCastSpell(me, m_spells.druid.pCatForm))
    {
        if (DoCastSpell(me, m_spells.druid.pCatForm) == SPELL_CAST_OK)
            return true;
    }

    if (m_spells.druid.pBearForm &&
       (m_role == ROLE_TANK || GetRole() == ROLE_MELEE_DPS) &&
        CanTryToCastSpell(me, m_spells.druid.pBearForm))
    {
        if (DoCastSpell(me, m_spells.druid.pBearForm) == SPELL_CAST_OK)
            return true;
    }

    if (m_spells.druid.pMoonkinForm &&
        GetRole() == ROLE_RANGE_DPS &&
        CanTryToCastSpell(me, m_spells.druid.pMoonkinForm))
    {
        if (DoCastSpell(me, m_spells.druid.pMoonkinForm) == SPELL_CAST_OK)
            return true;
    }

    return false;
}

void PartyBotAI::UpdateOutOfCombatAI_Druid()
{
    // Make sure bot leaves combat form if his role is changed to healer.
    if (GetRole() == ROLE_HEALER && me->GetShapeshiftForm() != FORM_NONE &&
        me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
    {
        me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);
        return;
    }

    if (m_spells.druid.pGiftoftheWild)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.druid.pGiftoftheWild))
        {
            if (CanTryToCastSpell(pTarget, m_spells.druid.pGiftoftheWild))
            {
                if (DoCastSpell(pTarget, m_spells.druid.pGiftoftheWild) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }
    else if (m_spells.druid.pMarkoftheWild)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.druid.pMarkoftheWild))
        {
            if (CanTryToCastSpell(pTarget, m_spells.druid.pMarkoftheWild))
            {
                if (DoCastSpell(pTarget, m_spells.druid.pMarkoftheWild) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    // Thorns: prioritize the tank, then any buff target
    if (m_spells.druid.pThorns)
    {
        Player* pThornsTarget = nullptr;
        if (Player* pTank = FindPartyTank())
        {
            if (CanTryToCastSpell(pTank, m_spells.druid.pThorns))
                pThornsTarget = pTank;
        }
        if (!pThornsTarget)
            pThornsTarget = SelectBuffTarget(m_spells.druid.pThorns);

        if (pThornsTarget && CanTryToCastSpell(pThornsTarget, m_spells.druid.pThorns))
        {
            if (DoCastSpell(pThornsTarget, m_spells.druid.pThorns) == SPELL_CAST_OK)
            {
                m_isBuffing = true;
                me->ClearTarget();
                return;
            }
        }
    }

    if (m_spells.druid.pNaturesGrasp &&
        CanTryToCastSpell(me, m_spells.druid.pNaturesGrasp))
    {
        if (DoCastSpell(me, m_spells.druid.pNaturesGrasp) == SPELL_CAST_OK)
            return;
    }

    // Rebirth (combat rez): most important Druid utility — check even out of combat
    if (m_spells.druid.pRebirth)
    {
        if (Player* pDeadTarget = SelectResurrectionTarget())
        {
            // Must be in caster form to cast Rebirth
            if (me->GetShapeshiftForm() != FORM_NONE &&
                me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
            {
                me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);
                return;
            }
            if (CanTryToCastSpell(pDeadTarget, m_spells.druid.pRebirth))
            {
                if (DoCastSpell(pDeadTarget, m_spells.druid.pRebirth) == SPELL_CAST_OK)
                    return;
            }
        }
    }

    if (m_isBuffing &&
       (!m_spells.druid.pMarkoftheWild ||
        !me->HasGCD(m_spells.druid.pMarkoftheWild)))
    {
        m_isBuffing = false;
    }

    if (me->GetShapeshiftForm() == FORM_NONE)
    {
        if (EnterCombatDruidForm())
            return;

        if ((me->GetPowerPercent(POWER_MANA) > 80.0f) &&
            FindAndHealInjuredAlly())
            return;
    }
    else if (me->GetShapeshiftForm() == FORM_CAT)
    {
        if (EnterStealthIfNeeded(m_spells.druid.pProwl))
            return;
    }

    if (me->GetVictim())
        UpdateInCombatAI_Druid();
}

void PartyBotAI::UpdateInCombatAI_Druid()
{
    // === REBIRTH — highest priority for all specs (combat rez a dead party member) ===
    if (m_spells.druid.pRebirth)
    {
        if (Player* pDeadTarget = SelectResurrectionTarget())
        {
            // Must shift to caster form to cast Rebirth
            if (me->GetShapeshiftForm() != FORM_NONE &&
                me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
            {
                me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);
                return;
            }
            if (CanTryToCastSpell(pDeadTarget, m_spells.druid.pRebirth))
            {
                if (DoCastSpell(pDeadTarget, m_spells.druid.pRebirth) == SPELL_CAST_OK)
                {
                    // Follow up with Nature's Swiftness + emergency heal if possible
                    if (m_spells.druid.pNaturesSwiftness &&
                        CanTryToCastSpell(me, m_spells.druid.pNaturesSwiftness))
                    {
                        DoCastSpell(me, m_spells.druid.pNaturesSwiftness);
                    }
                    return;
                }
            }
        }
    }

    // === INNERVATE — mana recovery for self or a healer ===
    if (m_spells.druid.pInnervate &&
        me->GetShapeshiftForm() == FORM_NONE)
    {
        // Self: very low mana
        if (me->GetPowerPercent(POWER_MANA) < 20.0f &&
            CanTryToCastSpell(me, m_spells.druid.pInnervate))
        {
            if (DoCastSpell(me, m_spells.druid.pInnervate) == SPELL_CAST_OK)
                return;
        }

        // Healer party member: mana running low — help them out
        if (me->GetPowerPercent(POWER_MANA) >= 20.0f)
        {
            if (Group* pGroup = me->GetGroup())
            {
                for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
                {
                    if (Player* pMember = itr->getSource())
                    {
                        if (pMember == me || !pMember->IsAlive())
                            continue;
                        if (pMember->GetPowerType() != POWER_MANA)
                            continue;
                        // Only target designated healers (class-based heuristic: Priest/Druid/Shaman/Paladin)
                        uint8 cls = pMember->GetClass();
                        bool isHealer = (cls == CLASS_PRIEST || cls == CLASS_DRUID ||
                                         cls == CLASS_SHAMAN || cls == CLASS_PALADIN);
                        if (isHealer && pMember->GetPowerPercent(POWER_MANA) < 30.0f &&
                            CanTryToCastSpell(pMember, m_spells.druid.pInnervate))
                        {
                            if (DoCastSpell(pMember, m_spells.druid.pInnervate) == SPELL_CAST_OK)
                                return;
                        }
                    }
                }
            }
        }
    }

    ShapeshiftForm const form = me->GetShapeshiftForm();

    // === BARKSKIN — survivability in caster/moonkin form ===
    if (m_spells.druid.pBarkskin &&
        (form == FORM_NONE || form == FORM_MOONKIN) &&
        (me->GetHealthPercent() < 50.0f) &&
        CanTryToCastSpell(me, m_spells.druid.pBarkskin))
    {
        if (DoCastSpell(me, m_spells.druid.pBarkskin) == SPELL_CAST_OK)
            return;
    }

    // === HEALER ROLE (caster form) ===
    if (GetRole() == ROLE_HEALER)
    {
        // Hibernate CC on Beast/Dragonkin attacker
        if (m_spells.druid.pHibernate &&
            !me->GetAttackers().empty())
        {
            Unit* pAttacker = *me->GetAttackers().begin();
            if (CanTryToCastSpell(pAttacker, m_spells.druid.pHibernate) &&
                CanUseCrowdControl(m_spells.druid.pHibernate, pAttacker))
            {
                if (DoCastSpell(pAttacker, m_spells.druid.pHibernate) == SPELL_CAST_OK)
                    return;
            }
        }

        // Nature's Swiftness emergency: use on tank when critical, then direct heal
        if (m_spells.druid.pNaturesSwiftness)
        {
            if (Player* pTank = FindPartyTank())
            {
                if (pTank->GetHealthPercent() < 35.0f &&
                    CanTryToCastSpell(me, m_spells.druid.pNaturesSwiftness))
                {
                    if (DoCastSpell(me, m_spells.druid.pNaturesSwiftness) == SPELL_CAST_OK)
                    {
                        // Follow up with the best available direct heal (instant due to NS)
                        HealInjuredTargetDirect(pTank);
                        return;
                    }
                }
            }
        }

        // HoT priority: apply periodic heals first (mana efficient)
        if (Unit* pTarget = SelectPeriodicHealTarget(80.0f, 90.0f))
            if (HealInjuredTargetPeriodic(pTarget))
                return;

        // Direct heal for more urgent injuries
        if (Unit* pTarget = SelectHealTarget(60.0f, 70.0f))
            if (HealInjuredTargetDirect(pTarget))
                return;

        // Innervate self when critically OOM in combat
        if (m_spells.druid.pInnervate &&
           (me->GetHealthPercent() > 40.0f) &&
           (me->GetPowerPercent(POWER_MANA) < 10.0f) &&
            CanTryToCastSpell(me, m_spells.druid.pInnervate))
        {
            if (DoCastSpell(me, m_spells.druid.pInnervate) == SPELL_CAST_OK)
                return;
        }

        if (FindAndPreHealTarget())
            return;

        // Emergency Bear Form: if being attacked and no tank is peeling the mob
        if (!me->GetAttackers().empty())
        {
            bool tankPresent = false;
            if (Player* pTank = FindPartyTank())
                tankPresent = pTank->IsAlive();

            if (!tankPresent)
            {
                // Shift to Bear Form, Bash the attacker, then shift back on next tick
                if (m_spells.druid.pBearForm && form == FORM_NONE &&
                    CanTryToCastSpell(me, m_spells.druid.pBearForm))
                {
                    if (DoCastSpell(me, m_spells.druid.pBearForm) == SPELL_CAST_OK)
                        return;
                }
                if ((form == FORM_BEAR || form == FORM_DIREBEAR) &&
                    m_spells.druid.pBash)
                {
                    Unit* pAttacker = *me->GetAttackers().begin();
                    if (CanTryToCastSpell(pAttacker, m_spells.druid.pBash))
                    {
                        if (DoCastSpell(pAttacker, m_spells.druid.pBash) == SPELL_CAST_OK)
                            return;
                    }
                }
            }
        }

        return;
    }

    Unit* pVictim = me->GetVictim();
    if (!pVictim)
        return;

    // Unroot and shift back if rooted and out of melee range (non-tank)
    if (form != FORM_NONE &&
        me->HasUnitState(UNIT_STATE_ROOT) &&
        me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT) &&
        (m_role != ROLE_TANK || !me->CanReachWithMeleeAutoAttack(pVictim)))
        me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);

    switch (form)
    {
        case FORM_CAT:
        {
            if (me->HasDistanceCasterMovement())
                me->SetCasterChaseDistance(0.0f);

            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
                && !me->CanReachWithMeleeAutoAttack(pVictim))
            {
                me->GetMotionMaster()->MoveChase(pVictim);
            }

            // Stealth openers
            if (me->HasAuraType(SPELL_AURA_MOD_STEALTH))
            {
                if (m_spells.druid.pPounce &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pPounce))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pPounce) == SPELL_CAST_OK)
                        return;
                }
                if (m_spells.druid.pRavage &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pRavage))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pRavage) == SPELL_CAST_OK)
                        return;
                }
                if (m_spells.druid.pTigersFury &&
                    CanTryToCastSpell(me, m_spells.druid.pTigersFury))
                {
                    if (DoCastSpell(me, m_spells.druid.pTigersFury) == SPELL_CAST_OK)
                        return;
                }
                return;
            }

            // Cower for threat management — use at high threat before pulling aggro
            if (m_spells.druid.pCower &&
                GetThreatRatio(pVictim) >= 0.80f &&
                CanTryToCastSpell(me, m_spells.druid.pCower))
            {
                if (DoCastSpell(me, m_spells.druid.pCower) == SPELL_CAST_OK)
                    return;
            }

            // Deterministic finisher at 5 combo points
            if (me->GetComboPoints() >= 5)
            {
                // Rip: bleed DoT on bosses/long-lived targets with HP to spare
                if (m_spells.druid.pRip &&
                    pVictim->GetHealthPercent() > 30.0f &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pRip))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pRip) == SPELL_CAST_OK)
                        return;
                }
                // Ferocious Bite: burst on trash or low-HP targets
                if (m_spells.druid.pFerociousBite &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pFerociousBite))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pFerociousBite) == SPELL_CAST_OK)
                        return;
                }
            }

            // Faerie Fire (Feral): free armor debuff + threat, keep up on target
            if (m_spells.druid.pFaerieFireFeral &&
                CanTryToCastSpell(pVictim, m_spells.druid.pFaerieFireFeral))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pFaerieFireFeral) == SPELL_CAST_OK)
                    return;
            }

            // Rake: bleed DoT, keep up
            if (m_spells.druid.pRake &&
                CanTryToCastSpell(pVictim, m_spells.druid.pRake))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pRake) == SPELL_CAST_OK)
                    return;
            }

            // Gap close on moving targets
            if (!me->CanReachWithMeleeAutoAttack(pVictim))
            {
                if (m_spells.druid.pDash &&
                    pVictim->IsMoving() &&
                    CanTryToCastSpell(me, m_spells.druid.pDash))
                {
                    if (DoCastSpell(me, m_spells.druid.pDash) == SPELL_CAST_OK)
                        return;
                }
            }

            // Shred: primary CP builder (from behind)
            if (m_spells.druid.pShred &&
                CanTryToCastSpell(pVictim, m_spells.druid.pShred))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pShred) == SPELL_CAST_OK)
                    return;
            }

            // Claw: fallback CP builder
            if (m_spells.druid.pClaw &&
                CanTryToCastSpell(pVictim, m_spells.druid.pClaw))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pClaw) == SPELL_CAST_OK)
                    return;
            }

            // Emergency: if tank dies, shift to Bear Form and become emergency tank
            if (m_role == ROLE_MELEE_DPS)
            {
                bool tankAlive = false;
                if (Player* pTank = FindPartyTank())
                    tankAlive = pTank->IsAlive();

                if (!tankAlive && GetAttackersInRangeCount(10.0f) > 0 &&
                    m_spells.druid.pBearForm &&
                    CanTryToCastSpell(me, m_spells.druid.pBearForm))
                {
                    if (DoCastSpell(me, m_spells.druid.pBearForm) == SPELL_CAST_OK)
                        return;
                }
            }

            break;
        }
        case FORM_BEAR:
        case FORM_DIREBEAR:
        {
            if (me->HasDistanceCasterMovement())
                me->SetCasterChaseDistance(0.0f);

            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
                && !me->CanReachWithMeleeAutoAttack(pVictim))
            {
                me->GetMotionMaster()->MoveChase(pVictim);
            }

            // Growl: taunt if a mob is attacking a party member (especially healer)
            if (m_spells.druid.pGrowl)
            {
                if (Group* pGroup = me->GetGroup())
                {
                    for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
                    {
                        if (Player* pMember = itr->getSource())
                        {
                            if (pMember == me || !pMember->IsAlive())
                                continue;
                            if (!pMember->GetAttackers().empty())
                            {
                                Unit* pThreatenedBy = *pMember->GetAttackers().begin();
                                if (pThreatenedBy == pVictim &&
                                    CanTryToCastSpell(pVictim, m_spells.druid.pGrowl))
                                {
                                    if (DoCastSpell(pVictim, m_spells.druid.pGrowl) == SPELL_CAST_OK)
                                        return;
                                }
                            }
                        }
                    }
                }
            }

            // Challenging Roar: AoE taunt emergency (multiple mobs on party members)
            if (m_spells.druid.pChallengingRoar &&
                GetAttackersInRangeCount(10.0f) >= 2 &&
                CanTryToCastSpell(me, m_spells.druid.pChallengingRoar))
            {
                if (DoCastSpell(me, m_spells.druid.pChallengingRoar) == SPELL_CAST_OK)
                    return;
            }

            // Feral Charge: close gap and interrupt
            if (m_spells.druid.pFeralCharge &&
                CanTryToCastSpell(pVictim, m_spells.druid.pFeralCharge))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pFeralCharge) == SPELL_CAST_OK)
                    return;
            }

            // Bash: interrupt spell casts
            if (m_spells.druid.pBash &&
                CanTryToCastSpell(pVictim, m_spells.druid.pBash))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pBash) == SPELL_CAST_OK)
                    return;
            }

            // Frenzied Regeneration: emergency self-heal at low HP
            if (m_spells.druid.pFrenziedRegeneration &&
                (me->GetHealthPercent() < 30.0f) &&
                CanTryToCastSpell(me, m_spells.druid.pFrenziedRegeneration))
            {
                if (DoCastSpell(me, m_spells.druid.pFrenziedRegeneration) == SPELL_CAST_OK)
                    return;
            }

            // Faerie Fire (Feral): armor debuff + threat, keep up on target
            if (m_spells.druid.pFaerieFireFeral &&
                CanTryToCastSpell(pVictim, m_spells.druid.pFaerieFireFeral))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pFaerieFireFeral) == SPELL_CAST_OK)
                    return;
            }

            // Demoralizing Roar: AP debuff for damage reduction
            if (m_spells.druid.pDemoralizingRoar &&
                CanTryToCastSpell(pVictim, m_spells.druid.pDemoralizingRoar))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pDemoralizingRoar) == SPELL_CAST_OK)
                    return;
            }

            // Swipe: AoE threat when rage is abundant or multiple attackers
            if ((me->GetPower(POWER_RAGE) > 80) ||
                (GetAttackersInRangeCount(10.0f) > 1))
            {
                if (m_spells.druid.pSwipe &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pSwipe))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pSwipe) == SPELL_CAST_OK)
                        return;
                }
            }

            // Maul: primary rage dump and threat builder
            if (m_spells.druid.pMaul &&
                CanTryToCastSpell(pVictim, m_spells.druid.pMaul))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pMaul) == SPELL_CAST_OK)
                    return;
            }

            // Enrage: rage generation when low on rage
            if (m_spells.druid.pEnrage &&
                me->GetPower(POWER_RAGE) < 20 &&
                CanTryToCastSpell(me, m_spells.druid.pEnrage))
            {
                if (DoCastSpell(me, m_spells.druid.pEnrage) == SPELL_CAST_OK)
                    return;
            }

            break;
        }
        case FORM_NONE:
        case FORM_MOONKIN:
        {
            // Range positioning
            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE &&
                me->GetDistance(pVictim) > 30.0f)
            {
                me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
            }
            else if (pVictim->CanReachWithMeleeAutoAttack(me) &&
                    (pVictim->GetVictim() == me) &&
                    !me->HasUnitState(UNIT_STATE_ROOT) &&
                    (me->GetMotionMaster()->GetCurrentMovementGeneratorType() != DISTANCING_MOTION_TYPE))
            {
                // Entangling Roots to escape melee attacker
                if (m_spells.druid.pEntanglingRoots &&
                    CanTryToCastSpell(pVictim, m_spells.druid.pEntanglingRoots))
                {
                    if (DoCastSpell(pVictim, m_spells.druid.pEntanglingRoots) == SPELL_CAST_OK)
                        return;
                }
                me->SetCasterChaseDistance(25.0f);
                if (RunAwayFromTarget(pVictim))
                    return;
            }

            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == DISTANCING_MOTION_TYPE)
                return;

            // Faerie Fire: free armor debuff, no cooldown — keep up on target always
            if (m_spells.druid.pFaerieFire &&
                CanTryToCastSpell(pVictim, m_spells.druid.pFaerieFire))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pFaerieFire) == SPELL_CAST_OK)
                    return;
            }

            // Insect Swarm: DoT + reduces target hit chance
            if (m_spells.druid.pInsectSwarm &&
                CanTryToCastSpell(pVictim, m_spells.druid.pInsectSwarm))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pInsectSwarm) == SPELL_CAST_OK)
                    return;
            }

            // Moonfire: instant DoT
            if (m_spells.druid.pMoonfire &&
                CanTryToCastSpell(pVictim, m_spells.druid.pMoonfire))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pMoonfire) == SPELL_CAST_OK)
                    return;
            }

            // Hurricane: AoE channel for 3+ enemies
            if (m_spells.druid.pHurricane &&
               (me->GetEnemyCountInRadiusAround(pVictim, 10.0f) > 2) &&
                CanTryToCastSpell(pVictim, m_spells.druid.pHurricane))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pHurricane) == SPELL_CAST_OK)
                    return;
            }

            // Starfire: long cast, high damage — use on targets with enough HP to justify it
            if (m_spells.druid.pStarfire &&
               (pVictim->GetHealthPercent() > 50.0f) &&
                CanTryToCastSpell(pVictim, m_spells.druid.pStarfire))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pStarfire) == SPELL_CAST_OK)
                    return;
            }

            // Wrath: fast filler
            if (m_spells.druid.pWrath &&
                CanTryToCastSpell(pVictim, m_spells.druid.pWrath))
            {
                if (DoCastSpell(pVictim, m_spells.druid.pWrath) == SPELL_CAST_OK)
                    return;
            }

            break;
        }
    }
}
