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

// Helper: cast a blessing on the first party member that needs it.
// Returns true and early-outs if a cast succeeds.
static bool TryCastBlessingOnTarget(PartyBotAI* ai, Player* me, SpellEntry const* pBlessing)
{
    if (!pBlessing)
        return false;
    if (Player* pTarget = ai->SelectBuffTarget(pBlessing))
    {
        if (ai->CanTryToCastSpell(pTarget, pBlessing))
        {
            if (ai->DoCastSpell(pTarget, pBlessing) == SPELL_CAST_OK)
            {
                me->ClearTarget();
                return true;
            }
        }
    }
    return false;
}

void PartyBotAI::UpdateOutOfCombatAI_Paladin()
{
    // === AURA MAINTENANCE ===
    if (m_spells.paladin.pAura &&
        CanTryToCastSpell(me, m_spells.paladin.pAura))
    {
        if (DoCastSpell(me, m_spells.paladin.pAura) == SPELL_CAST_OK)
            return;
    }

    // === RIGHTEOUS FURY (Tank only) ===
    if (m_role == ROLE_TANK &&
        m_spells.paladin.pRighteousFury &&
        CanTryToCastSpell(me, m_spells.paladin.pRighteousFury))
    {
        if (DoCastSpell(me, m_spells.paladin.pRighteousFury) == SPELL_CAST_OK)
            return;
    }

    // === INTELLIGENT BLESSING SELECTION ===
    // Iterate party members and assign the most appropriate blessing per role.
    // Priority order per role:
    //   Tanks         -> Might (never Salvation)
    //   Melee DPS     -> Might, then Salvation
    //   Casters/Heals -> Wisdom, then Salvation
    // Fallback: Kings for everyone, then generic pBlessingBuff
    if (Group* pGroup = me->GetGroup())
    {
        for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            Player* pMember = itr->getSource();
            if (!pMember || !pMember->IsAlive())
                continue;

            uint8 memberClass = pMember->GetClass();
            bool isTank = false;

            // Identify the tank by asking the AI (if it's another bot) or by class heuristic
            // We consider Warriors and Paladins that could be tanking; use the generic
            // FindPartyTank() result as the reference rather than checking every member.
            if (Player* pFoundTank = FindPartyTank())
                isTank = (pFoundTank == pMember);

            SpellEntry const* pBestBlessing = nullptr;

            if (isTank)
            {
                // Tanks: Might > Kings > generic (never Salvation)
                if (m_spells.paladin.pBlessingOfMight)
                    pBestBlessing = m_spells.paladin.pBlessingOfMight;
                else if (m_spells.paladin.pBlessingOfKings)
                    pBestBlessing = m_spells.paladin.pBlessingOfKings;
                else
                    pBestBlessing = m_spells.paladin.pBlessingBuff;
            }
            else if (IsPhysicalDamageClass(memberClass))
            {
                // Melee DPS: Might first, then Salvation
                if (m_spells.paladin.pBlessingOfMight)
                    pBestBlessing = m_spells.paladin.pBlessingOfMight;
                else if (m_spells.paladin.pBlessingOfSalvation)
                    pBestBlessing = m_spells.paladin.pBlessingOfSalvation;
                else if (m_spells.paladin.pBlessingOfKings)
                    pBestBlessing = m_spells.paladin.pBlessingOfKings;
                else
                    pBestBlessing = m_spells.paladin.pBlessingBuff;
            }
            else
            {
                // Casters and healers: Wisdom first, then Salvation
                if (m_spells.paladin.pBlessingOfWisdom)
                    pBestBlessing = m_spells.paladin.pBlessingOfWisdom;
                else if (m_spells.paladin.pBlessingOfSalvation)
                    pBestBlessing = m_spells.paladin.pBlessingOfSalvation;
                else if (m_spells.paladin.pBlessingOfKings)
                    pBestBlessing = m_spells.paladin.pBlessingOfKings;
                else
                    pBestBlessing = m_spells.paladin.pBlessingBuff;
            }

            if (pBestBlessing && CanTryToCastSpell(pMember, pBestBlessing))
            {
                // Only cast if pMember actually needs it (SelectBuffTarget equivalent check)
                if (Player* pNeedsIt = SelectBuffTarget(pBestBlessing))
                {
                    if (pNeedsIt == pMember)
                    {
                        if (DoCastSpell(pMember, pBestBlessing) == SPELL_CAST_OK)
                        {
                            m_isBuffing = true;
                            me->ClearTarget();
                            return;
                        }
                    }
                }
            }
        }
    }

    // Secondary pass: SelectBuffTarget will find anyone that still needs any blessing
    if (TryCastBlessingOnTarget(this, me, m_spells.paladin.pBlessingOfMight))
    {
        m_isBuffing = true;
        return;
    }
    if (TryCastBlessingOnTarget(this, me, m_spells.paladin.pBlessingOfWisdom))
    {
        m_isBuffing = true;
        return;
    }
    if (TryCastBlessingOnTarget(this, me, m_spells.paladin.pBlessingOfKings))
    {
        m_isBuffing = true;
        return;
    }
    if (TryCastBlessingOnTarget(this, me, m_spells.paladin.pBlessingBuff))
    {
        m_isBuffing = true;
        return;
    }

    // Clear buffing flag once GCD expires
    if (m_isBuffing &&
       (!m_spells.paladin.pBlessingBuff ||
        !me->HasGCD(m_spells.paladin.pBlessingBuff)))
    {
        m_isBuffing = false;
    }

    // === HEAL BETWEEN PULLS (Healer) ===
    if (m_role == ROLE_HEALER && FindAndHealInjuredAlly())
        return;
}

void PartyBotAI::UpdateInCombatAI_Paladin()
{
    // =========================================================
    // === UNIVERSAL CHECKS (all specs) ===
    // =========================================================

    // Blessing of Freedom — remove roots/snares on self immediately
    if (m_spells.paladin.pBlessingOfFreedom &&
       (me->HasUnitState(UNIT_STATE_ROOT) || me->HasAuraType(SPELL_AURA_MOD_DECREASE_SPEED)) &&
        CanTryToCastSpell(me, m_spells.paladin.pBlessingOfFreedom))
    {
        if (DoCastSpell(me, m_spells.paladin.pBlessingOfFreedom) == SPELL_CAST_OK)
            return;
    }

    // Divine Shield — non-tank emergency escape at very low HP
    if (m_role != ROLE_TANK &&
        m_spells.paladin.pDivineShield &&
        me->GetHealthPercent() < 20.0f &&
        CanTryToCastSpell(me, m_spells.paladin.pDivineShield))
    {
        if (DoCastSpell(me, m_spells.paladin.pDivineShield) == SPELL_CAST_OK)
            return;
    }

    // Lay on Hands — critically low ally (any spec can do this)
    if (m_spells.paladin.pLayOnHands)
    {
        if (Unit* pCritical = me->FindLowestHpFriendlyUnit(40.0f, 100, true, me))
        {
            if (pCritical->GetHealthPercent() < 15.0f &&
                CanTryToCastSpell(pCritical, m_spells.paladin.pLayOnHands))
            {
                if (DoCastSpell(pCritical, m_spells.paladin.pLayOnHands) == SPELL_CAST_OK)
                    return;
            }
        }
    }

    // Blessing of Protection — low-HP non-physical ally to buy them time
    if (m_spells.paladin.pBlessingOfProtection)
    {
        if (Unit* pLow = me->FindLowestHpFriendlyUnit(30.0f, 70, true, me))
        {
            if (!IsPhysicalDamageClass(pLow->GetClass()) &&
                CanTryToCastSpell(pLow, m_spells.paladin.pBlessingOfProtection))
            {
                if (DoCastSpell(pLow, m_spells.paladin.pBlessingOfProtection) == SPELL_CAST_OK)
                    return;
            }
        }
    }

    // Blessing of Sacrifice — redirect damage from a hurt ally when we are healthy
    if (m_spells.paladin.pBlessingOfSacrifice &&
        me->GetHealthPercent() > 80.0f)
    {
        if (Unit* pLow = me->FindLowestHpFriendlyUnit(30.0f, 70, true, me))
        {
            if (CanTryToCastSpell(pLow, m_spells.paladin.pBlessingOfSacrifice))
            {
                if (DoCastSpell(pLow, m_spells.paladin.pBlessingOfSacrifice) == SPELL_CAST_OK)
                    return;
            }
        }
    }

    // =========================================================
    // === ROLE: HEALER ===
    // =========================================================
    if (GetRole() == ROLE_HEALER)
    {
        // Emergency: Lay on Hands on tank below 15%
        if (m_spells.paladin.pLayOnHands)
        {
            if (Player* pTank = FindPartyTank())
            {
                if (pTank->GetHealthPercent() < 15.0f &&
                    CanTryToCastSpell(pTank, m_spells.paladin.pLayOnHands))
                {
                    if (DoCastSpell(pTank, m_spells.paladin.pLayOnHands) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Self-preservation: Divine Favor + Holy Shock when taking damage at <50% HP
        if (m_spells.paladin.pHolyShock &&
            me->GetHealthPercent() < 50.0f &&
            !me->GetAttackers().empty() &&
            CanTryToCastSpell(me, m_spells.paladin.pHolyShock))
        {
            if (m_spells.paladin.pDivineFavor &&
                CanTryToCastSpell(me, m_spells.paladin.pDivineFavor))
            {
                DoCastSpell(me, m_spells.paladin.pDivineFavor);
            }
            if (DoCastSpell(me, m_spells.paladin.pHolyShock) == SPELL_CAST_OK)
                return;
        }

        // BoP self to drop physical aggro
        if (!me->GetAttackers().empty() &&
            m_spells.paladin.pBlessingOfProtection &&
            CanTryToCastSpell(me, m_spells.paladin.pBlessingOfProtection))
        {
            if (DoCastSpell(me, m_spells.paladin.pBlessingOfProtection) == SPELL_CAST_OK)
                return;
        }

        // Cleanse debuffs (high priority)
        if (CheckForDispelTargets())
            return;

        // === HP-THRESHOLD HEALING ON TANK ===
        if (Player* pTank = FindPartyTank())
        {
            float tankHp = pTank->GetHealthPercent();

            // CRITICAL (<30%): Divine Favor + direct heal for guaranteed result
            if (tankHp < 30.0f)
            {
                if (m_spells.paladin.pDivineFavor &&
                    CanTryToCastSpell(me, m_spells.paladin.pDivineFavor))
                {
                    DoCastSpell(me, m_spells.paladin.pDivineFavor);
                }
                if (HealInjuredTargetDirect(pTank))
                    return;
            }
            // MODERATE (30-60%): Flash of Light / Holy Light for quick top-up
            else if (tankHp < 60.0f)
            {
                if (HealInjuredTargetDirect(pTank))
                    return;
            }
            // LIGHT (60-80%): Efficient heal
            else if (tankHp < 80.0f)
            {
                if (HealInjuredTarget(pTank))
                    return;
            }
        }

        // Heal other injured allies
        if (FindAndHealInjuredAlly(70.0f, 85.0f))
            return;

        // Pre-heal (anticipate incoming damage)
        if (FindAndPreHealTarget())
            return;

        // Hammer of Justice — interrupt or stun a loose mob attacking the healer
        if (!me->GetAttackers().empty() &&
            m_spells.paladin.pHammerOfJustice)
        {
            Unit* pAttacker = *me->GetAttackers().begin();
            if (pAttacker &&
                CanTryToCastSpell(pAttacker, m_spells.paladin.pHammerOfJustice))
            {
                if (DoCastSpell(pAttacker, m_spells.paladin.pHammerOfJustice) == SPELL_CAST_OK)
                    return;
            }
        }

        return;
    }

    // =========================================================
    // === ROLE: TANK (Protection) ===
    // =========================================================
    if (GetRole() == ROLE_TANK)
    {
        // Holy Shield — keep up for consistent block and Holy damage
        if (m_spells.paladin.pHolyShield &&
            CanTryToCastSpell(me, m_spells.paladin.pHolyShield))
        {
            if (DoCastSpell(me, m_spells.paladin.pHolyShield) == SPELL_CAST_OK)
                return;
        }

        // Consecration — strong AoE threat when 2+ mobs are attacking
        if (m_spells.paladin.pConsecration &&
            GetAttackersInRangeCount(10.0f) >= 2 &&
            CanTryToCastSpell(me, m_spells.paladin.pConsecration))
        {
            if (DoCastSpell(me, m_spells.paladin.pConsecration) == SPELL_CAST_OK)
                return;
        }

        // Seal maintenance
        if (m_spells.paladin.pSeal &&
            !me->HasAura(m_spells.paladin.pSeal->Id) &&
            CanTryToCastSpell(me, m_spells.paladin.pSeal))
        {
            if (DoCastSpell(me, m_spells.paladin.pSeal) == SPELL_CAST_OK)
                return;
        }

        if (Unit* pVictim = me->GetVictim())
        {
            // Judgement — refresh threat and apply seal debuff
            bool const tankHasSeal = m_spells.paladin.pSeal &&
                                     me->HasAura(m_spells.paladin.pSeal->Id);
            if (m_spells.paladin.pJudgement &&
                tankHasSeal &&
                CanTryToCastSpell(pVictim, m_spells.paladin.pJudgement))
            {
                if (DoCastSpell(pVictim, m_spells.paladin.pJudgement) == SPELL_CAST_OK)
                    return;
            }

            // Hammer of Justice — interrupt caster mobs or stun
            if (m_spells.paladin.pHammerOfJustice &&
                (pVictim->IsNonMeleeSpellCasted(false, false, true) ||
                 GetAttackersInRangeCount(10.0f) >= 3) &&
                CanTryToCastSpell(pVictim, m_spells.paladin.pHammerOfJustice))
            {
                if (DoCastSpell(pVictim, m_spells.paladin.pHammerOfJustice) == SPELL_CAST_OK)
                    return;
            }

            // Exorcism — bonus damage vs undead
            if (m_spells.paladin.pExorcism &&
                pVictim->IsCreature() &&
                pVictim->GetCreatureType() == CREATURE_TYPE_UNDEAD &&
                CanTryToCastSpell(pVictim, m_spells.paladin.pExorcism))
            {
                if (DoCastSpell(pVictim, m_spells.paladin.pExorcism) == SPELL_CAST_OK)
                    return;
            }

            // Holy Wrath — AoE stun vs undead/demon, only when few attackers to avoid pushback
            if (m_spells.paladin.pHolyWrath &&
                pVictim->IsCreature() &&
               (pVictim->GetCreatureType() == CREATURE_TYPE_UNDEAD ||
                pVictim->GetCreatureType() == CREATURE_TYPE_DEMON) &&
                me->GetAttackers().size() < 3 &&
                CanTryToCastSpell(pVictim, m_spells.paladin.pHolyWrath))
            {
                if (DoCastSpell(pVictim, m_spells.paladin.pHolyWrath) == SPELL_CAST_OK)
                    return;
            }

            // Stay in melee range
            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE &&
                !me->CanReachWithMeleeAutoAttack(pVictim))
            {
                me->GetMotionMaster()->MoveChase(pVictim);
            }
        }

        return;
    }

    // =========================================================
    // === ROLE: DPS (Retribution) ===
    // =========================================================

    // Emergency self-heal via Lay on Hands
    if (m_spells.paladin.pLayOnHands &&
        me->GetHealthPercent() < 15.0f &&
        CanTryToCastSpell(me, m_spells.paladin.pLayOnHands))
    {
        if (DoCastSpell(me, m_spells.paladin.pLayOnHands) == SPELL_CAST_OK)
            return;
    }

    // Seal of Command maintenance (primary Ret seal)
    bool const hasSeal = m_spells.paladin.pSeal &&
                         me->HasAura(m_spells.paladin.pSeal->Id);
    if (!hasSeal &&
        m_spells.paladin.pSeal &&
        CanTryToCastSpell(me, m_spells.paladin.pSeal))
    {
        if (DoCastSpell(me, m_spells.paladin.pSeal) == SPELL_CAST_OK)
            return;
    }

    if (Unit* pVictim = me->GetVictim())
    {
        // Judgement — only when we have a seal up and mana to spare
        if (hasSeal &&
            m_spells.paladin.pJudgement &&
            me->GetPowerPercent(POWER_MANA) > 30.0f &&
            CanTryToCastSpell(pVictim, m_spells.paladin.pJudgement))
        {
            if (DoCastSpell(pVictim, m_spells.paladin.pJudgement) == SPELL_CAST_OK)
                return;
        }

        // Hammer of Wrath — execute range (<20% HP)
        if (m_spells.paladin.pHammerOfWrath &&
            pVictim->GetHealthPercent() < 20.0f &&
            CanTryToCastSpell(pVictim, m_spells.paladin.pHammerOfWrath))
        {
            if (DoCastSpell(pVictim, m_spells.paladin.pHammerOfWrath) == SPELL_CAST_OK)
                return;
        }

        // Consecration — mana-hungry, only use when flush and multiple targets
        if (m_spells.paladin.pConsecration &&
            me->GetPowerPercent(POWER_MANA) > 70.0f &&
            GetAttackersInRangeCount(10.0f) >= 2 &&
            CanTryToCastSpell(me, m_spells.paladin.pConsecration))
        {
            if (DoCastSpell(me, m_spells.paladin.pConsecration) == SPELL_CAST_OK)
                return;
        }

        // Holy Shock — instant damage on cooldown
        if (m_spells.paladin.pHolyShock &&
            CanTryToCastSpell(pVictim, m_spells.paladin.pHolyShock))
        {
            if (DoCastSpell(pVictim, m_spells.paladin.pHolyShock) == SPELL_CAST_OK)
                return;
        }

        // Exorcism — bonus Holy damage vs undead
        if (m_spells.paladin.pExorcism &&
            pVictim->IsCreature() &&
            pVictim->GetCreatureType() == CREATURE_TYPE_UNDEAD &&
            CanTryToCastSpell(pVictim, m_spells.paladin.pExorcism))
        {
            if (DoCastSpell(pVictim, m_spells.paladin.pExorcism) == SPELL_CAST_OK)
                return;
        }

        // Hammer of Justice — interrupt caster targets
        if (m_spells.paladin.pHammerOfJustice &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.paladin.pHammerOfJustice))
        {
            if (DoCastSpell(pVictim, m_spells.paladin.pHammerOfJustice) == SPELL_CAST_OK)
                return;
        }

        // Stay in melee range
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE &&
            !me->CanReachWithMeleeAutoAttack(pVictim))
        {
            me->GetMotionMaster()->MoveChase(pVictim);
        }
    }

    // Off-heal self when low (Ret has no other sustain)
    if (me->GetHealthPercent() < 30.0f)
        HealInjuredTarget(me);
}
