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

enum PartyBotSpells_Priest
{
    PB_SPELL_SHOOT_WAND_P = 5019,
};

void PartyBotAI::UpdateOutOfCombatAI_Priest()
{
    // === BUFF PRIORITY: Fortitude > Spirit > Shadow Protection > Inner Fire ===

    if (m_spells.priest.pPrayerofFortitude)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pPrayerofFortitude))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pPrayerofFortitude))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pPrayerofFortitude) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }
    else if (m_spells.priest.pPowerWordFortitude)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pPowerWordFortitude))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pPowerWordFortitude))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pPowerWordFortitude) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    if (m_spells.priest.pPrayerofSpirit)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pPrayerofSpirit))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pPrayerofSpirit))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pPrayerofSpirit) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }
    else if (m_spells.priest.pDivineSpirit)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pDivineSpirit))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pDivineSpirit))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pDivineSpirit) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    if (m_spells.priest.pPrayerofShadowProtection)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pPrayerofShadowProtection))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pPrayerofShadowProtection))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pPrayerofShadowProtection) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }
    else if (m_spells.priest.pShadowProtection)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.priest.pShadowProtection))
        {
            if (CanTryToCastSpell(pTarget, m_spells.priest.pShadowProtection))
            {
                if (DoCastSpell(pTarget, m_spells.priest.pShadowProtection) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    if (m_spells.priest.pInnerFire &&
        CanTryToCastSpell(me, m_spells.priest.pInnerFire))
    {
        if (DoCastSpell(me, m_spells.priest.pInnerFire) == SPELL_CAST_OK)
        {
            m_isBuffing = true;
            me->ClearTarget();
            return;
        }
    }

    if (m_isBuffing &&
       (!m_spells.priest.pPowerWordFortitude ||
        !me->HasGCD(m_spells.priest.pPowerWordFortitude)))
    {
        m_isBuffing = false;
    }

    // Pre-pull: Shield tank before combat starts (skip warriors — prevents rage generation)
    if (GetRole() == ROLE_HEALER && me->GetVictim())
    {
        if (m_spells.priest.pPowerWordShield)
        {
            if (Player* pTank = FindPartyTank())
            {
                if (pTank->GetClass() != CLASS_WARRIOR &&
                    CanTryToCastSpell(pTank, m_spells.priest.pPowerWordShield))
                {
                    if (DoCastSpell(pTank, m_spells.priest.pPowerWordShield) == SPELL_CAST_OK)
                        return;
                }
            }
        }
    }

    // Heal between pulls
    if (m_role == ROLE_HEALER && FindAndHealInjuredAlly())
        return;

    if (me->GetVictim())
        UpdateInCombatAI_Priest();
}

void PartyBotAI::UpdateInCombatAI_Priest()
{
    // Priests never melee auto-attack — cancel it each tick.
    me->AttackStop();

    // === HOLY/DISCIPLINE PRIEST (Healer) ===
    if (GetRole() == ROLE_HEALER)
    {
        // Shield self if being attacked
        if (!me->GetAttackers().empty())
        {
            if (m_spells.priest.pPowerWordShield &&
                CanTryToCastSpell(me, m_spells.priest.pPowerWordShield))
            {
                if (DoCastSpell(me, m_spells.priest.pPowerWordShield) == SPELL_CAST_OK)
                    return;
            }

            // Fade to dump threat
            if (m_spells.priest.pFade &&
                CanTryToCastSpell(me, m_spells.priest.pFade))
            {
                if (DoCastSpell(me, m_spells.priest.pFade) == SPELL_CAST_OK)
                    return;
            }

            // Shackle Undead attacker as last resort CC
            if (m_spells.priest.pShackleUndead)
            {
                Unit* pAttacker = *me->GetAttackers().begin();
                if ((pAttacker->GetHealth() > me->GetHealth()) &&
                    CanTryToCastSpell(pAttacker, m_spells.priest.pShackleUndead) &&
                    CanUseCrowdControl(m_spells.priest.pShackleUndead, pAttacker))
                {
                    if (DoCastSpell(pAttacker, m_spells.priest.pShackleUndead) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Inner Focus before expensive heals when mana is getting low
        if (m_spells.priest.pInnerFocus &&
           (me->GetPowerPercent(POWER_MANA) < 50.0f) &&
            CanTryToCastSpell(me, m_spells.priest.pInnerFocus))
        {
            DoCastSpell(me, m_spells.priest.pInnerFocus);
        }

        // === HP-THRESHOLD HEALING on tank ===
        if (Player* pTank = FindPartyTank())
        {
            float tankHp = pTank->GetHealthPercent();

            // CRITICAL (<30%): Shield + Greater Heal
            // Warriors only get shielded below 15% — shielding blocks rage generation
            if (tankHp < 30.0f)
            {
                if (m_spells.priest.pPowerWordShield &&
                    (pTank->GetClass() != CLASS_WARRIOR || tankHp < 15.0f) &&
                    CanTryToCastSpell(pTank, m_spells.priest.pPowerWordShield))
                {
                    DoCastSpell(pTank, m_spells.priest.pPowerWordShield);
                }

                if (m_spells.priest.pGreaterHeal &&
                    CanTryToCastSpell(pTank, m_spells.priest.pGreaterHeal))
                {
                    if (DoCastSpell(pTank, m_spells.priest.pGreaterHeal) == SPELL_CAST_OK)
                        return;
                }
            }
            // MODERATE (30-60%): Flash Heal (fast, 1.5s cast)
            else if (tankHp < 60.0f)
            {
                if (m_spells.priest.pFlashHeal &&
                    CanTryToCastSpell(pTank, m_spells.priest.pFlashHeal))
                {
                    if (DoCastSpell(pTank, m_spells.priest.pFlashHeal) == SPELL_CAST_OK)
                        return;
                }
            }
            // LIGHT (60-80%): Renew (HoT, mana efficient)
            else if (tankHp < 80.0f)
            {
                if (m_spells.priest.pRenew &&
                    CanTryToCastSpell(pTank, m_spells.priest.pRenew))
                {
                    if (DoCastSpell(pTank, m_spells.priest.pRenew) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Shield allies being attacked (skip warrior tanks — blocks rage generation)
        if (m_spells.priest.pPowerWordShield)
        {
            if (Player* pTarget = SelectShieldTarget())
            {
                Player* pTank = FindPartyTank();
                bool isWarriorTank = (pTank && pTarget == pTank && pTarget->GetClass() == CLASS_WARRIOR);
                if (!isWarriorTank &&
                    CanTryToCastSpell(pTarget, m_spells.priest.pPowerWordShield))
                {
                    if (DoCastSpell(pTarget, m_spells.priest.pPowerWordShield) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Prayer of Healing when 3+ party members are injured
        if (m_spells.priest.pPrayerOfHealing &&
            me->GetPowerPercent(POWER_MANA) > 30.0f)
        {
            // Count injured party members in range
            uint8 injuredCount = 0;
            if (Group* pGroup = me->GetGroup())
            {
                for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
                {
                    if (Player* pMember = itr->getSource())
                    {
                        if (pMember->IsAlive() && pMember->GetHealthPercent() < 60.0f &&
                            me->GetDistance(pMember) < 30.0f)
                            injuredCount++;
                    }
                }
            }
            if (injuredCount >= 3 &&
                CanTryToCastSpell(me, m_spells.priest.pPrayerOfHealing))
            {
                // Use Inner Focus before Prayer of Healing if available
                if (m_spells.priest.pInnerFocus &&
                    CanTryToCastSpell(me, m_spells.priest.pInnerFocus))
                {
                    DoCastSpell(me, m_spells.priest.pInnerFocus);
                }

                if (DoCastSpell(me, m_spells.priest.pPrayerOfHealing) == SPELL_CAST_OK)
                    return;
            }
        }

        // Fallback: heal any injured ally (direct then periodic)
        if (Unit* pTarget = SelectHealTarget(60.0f, 80.0f))
            if (HealInjuredTargetDirect(pTarget))
                return;

        if (Unit* pTarget = SelectPeriodicHealTarget(80.0f, 90.0f))
            if (HealInjuredTargetPeriodic(pTarget))
                return;

        if (FindAndPreHealTarget())
            return;

        // Wand for mana regen during low-damage phases (5-second rule)
        if (me->HasSpell(PB_SPELL_SHOOT_WAND_P) && me->GetVictim() &&
           !me->IsMoving() &&
           (me->GetPowerPercent(POWER_MANA) < 80.0f) &&
           !me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
        {
            me->CastSpell(me->GetVictim(), PB_SPELL_SHOOT_WAND_P, false);
        }

        return;
    }

    // === SHADOW PRIEST (DPS) ===
    if (Unit* pVictim = me->GetVictim())
    {
        bool const highThreat = ShouldThrottleDPS(pVictim);

        // Enter Shadowform
        if (m_spells.priest.pShadowform &&
            CanTryToCastSpell(me, m_spells.priest.pShadowform))
        {
            if (DoCastSpell(me, m_spells.priest.pShadowform) == SPELL_CAST_OK)
                return;
        }

        // Shield self if being attacked
        if (m_spells.priest.pPowerWordShield &&
            !me->GetAttackers().empty() &&
            CanTryToCastSpell(me, m_spells.priest.pPowerWordShield))
        {
            if (DoCastSpell(me, m_spells.priest.pPowerWordShield) == SPELL_CAST_OK)
                return;
        }

        // Fade when we have aggro
        if (!me->GetAttackers().empty())
        {
            if (m_spells.priest.pFade &&
                CanTryToCastSpell(me, m_spells.priest.pFade))
            {
                if (DoCastSpell(me, m_spells.priest.pFade) == SPELL_CAST_OK)
                    return;
            }
        }

        // Silence interrupt (highest priority)
        if (m_spells.priest.pSilence &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.priest.pSilence))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pSilence) == SPELL_CAST_OK)
                return;
        }

        // Vampiric Embrace (apply FIRST before any damage)
        if (m_spells.priest.pVampiricEmbrace &&
            CanTryToCastSpell(pVictim, m_spells.priest.pVampiricEmbrace))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pVampiricEmbrace) == SPELL_CAST_OK)
                return;
        }

        // Shadow Word: Pain (keep up, lower threat than Mind Blast)
        if (m_spells.priest.pShadowWordPain &&
            CanTryToCastSpell(pVictim, m_spells.priest.pShadowWordPain))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pShadowWordPain) == SPELL_CAST_OK)
                return;
        }

        // Mind Blast (high burst, HIGH threat — skip entirely when threat is high)
        if (!highThreat && m_spells.priest.pMindBlast &&
            CanTryToCastSpell(pVictim, m_spells.priest.pMindBlast))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pMindBlast) == SPELL_CAST_OK)
                return;
        }

        // Devouring Plague (Undead racial)
        if (m_spells.priest.pDevouringPlague &&
            CanTryToCastSpell(pVictim, m_spells.priest.pDevouringPlague))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pDevouringPlague) == SPELL_CAST_OK)
                return;
        }

        // Mind Flay (filler — safe to use at high threat since it's low threat)
        if (m_spells.priest.pMindFlay &&
           (!GetAttackersInRangeCount(10.0f) || me->HasAuraType(SPELL_AURA_SCHOOL_ABSORB)) &&
            CanTryToCastSpell(pVictim, m_spells.priest.pMindFlay))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pMindFlay) == SPELL_CAST_OK)
                return;
        }

        // Range positioning
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
            && me->GetDistance(pVictim) > 30.0f)
        {
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        }

        // Mana Burn (situational, target has mana)
        if (m_spells.priest.pManaBurn &&
           (pVictim->GetPowerType() == POWER_MANA) &&
           !highThreat &&
            CanTryToCastSpell(pVictim, m_spells.priest.pManaBurn))
        {
            if (DoCastSpell(pVictim, m_spells.priest.pManaBurn) == SPELL_CAST_OK)
                return;
        }

        // Holy form fallback (if not in Shadowform for some reason)
        if (me->GetShapeshiftForm() == FORM_NONE)
        {
            if (m_spells.priest.pHolyNova &&
                GetAttackersInRangeCount(10.0f) > 2 &&
                CanTryToCastSpell(me, m_spells.priest.pHolyNova))
            {
                if (DoCastSpell(me, m_spells.priest.pHolyNova) == SPELL_CAST_OK)
                    return;
            }

            if (m_spells.priest.pSmite &&
                CanTryToCastSpell(pVictim, m_spells.priest.pSmite))
            {
                if (DoCastSpell(pVictim, m_spells.priest.pSmite) == SPELL_CAST_OK)
                    return;
            }
        }

        // Wand when OOM
        if (me->HasSpell(PB_SPELL_SHOOT_WAND_P) &&
           !me->IsMoving() &&
           (me->GetPowerPercent(POWER_MANA) < 10.0f) &&
           !me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
            me->CastSpell(pVictim, PB_SPELL_SHOOT_WAND_P, false);
    }
}
