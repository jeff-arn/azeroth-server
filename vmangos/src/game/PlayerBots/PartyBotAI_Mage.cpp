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

enum PartyBotSpells_Mage
{
    PB_SPELL_SHOOT_WAND_M = 5019,
};

void PartyBotAI::UpdateOutOfCombatAI_Mage()
{
    // === BUFF PRIORITY: Arcane Brilliance (group) > Arcane Intellect (individual) ===

    if (m_spells.mage.pArcaneBrilliance)
    {
        if (CanTryToCastSpell(me, m_spells.mage.pArcaneBrilliance))
        {
            if (DoCastSpell(me, m_spells.mage.pArcaneBrilliance) == SPELL_CAST_OK)
            {
                m_isBuffing = true;
                me->ClearTarget();
                return;
            }
        }
    }
    else if (m_spells.mage.pArcaneIntellect)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.mage.pArcaneIntellect))
        {
            if (CanTryToCastSpell(pTarget, m_spells.mage.pArcaneIntellect))
            {
                if (DoCastSpell(pTarget, m_spells.mage.pArcaneIntellect) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    // Ice Armor / Frost Armor self-buff
    if (m_spells.mage.pIceArmor &&
        CanTryToCastSpell(me, m_spells.mage.pIceArmor))
    {
        if (DoCastSpell(me, m_spells.mage.pIceArmor) == SPELL_CAST_OK)
        {
            m_isBuffing = true;
            me->ClearTarget();
            return;
        }
    }

    // Ice Barrier pre-combat absorb shield
    if (m_spells.mage.pIceBarrier &&
        CanTryToCastSpell(me, m_spells.mage.pIceBarrier))
    {
        if (DoCastSpell(me, m_spells.mage.pIceBarrier) == SPELL_CAST_OK)
        {
            m_isBuffing = true;
            me->ClearTarget();
            return;
        }
    }

    // Remove Lesser Curse on allies
    if (m_spells.mage.pRemoveLesserCurse)
    {
        if (Player* pTarget = SelectDispelTarget(m_spells.mage.pRemoveLesserCurse))
        {
            if (CanTryToCastSpell(pTarget, m_spells.mage.pRemoveLesserCurse))
            {
                if (DoCastSpell(pTarget, m_spells.mage.pRemoveLesserCurse) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    if (m_isBuffing &&
       (!m_spells.mage.pArcaneIntellect ||
        !me->HasGCD(m_spells.mage.pArcaneIntellect)))
    {
        m_isBuffing = false;
    }

    if (me->GetVictim())
        UpdateInCombatAI_Mage();
}

void PartyBotAI::UpdateInCombatAI_Mage()
{
    if (Unit* pVictim = me->GetVictim())
    {
        // Mages never melee auto-attack — cancel it each tick.
        // AttackStart enables melee for all classes; we suppress it here.
        me->AttackStop();

        // === HIGHEST PRIORITY: Counterspell interrupt ===
        if (m_spells.mage.pCounterspell &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.mage.pCounterspell))
        {
            if (DoCastSpell(pVictim, m_spells.mage.pCounterspell) == SPELL_CAST_OK)
                return;
        }

        // === EMERGENCY: Ice Block at <10% HP (drops all threat) ===
        if (m_spells.mage.pIceBlock &&
           (me->GetHealthPercent() < 10.0f) &&
            CanTryToCastSpell(me, m_spells.mage.pIceBlock))
        {
            if (DoCastSpell(me, m_spells.mage.pIceBlock) == SPELL_CAST_OK)
                return;
        }

        // === POLYMORPH CC MAINTENANCE: sheep a different attacker if >20% HP ===
        if (m_spells.mage.pPolymorph)
        {
            if (Unit* pCCTarget = SelectAttackerDifferentFrom(pVictim))
            {
                if (pCCTarget->GetHealthPercent() > 20.0f &&
                    CanTryToCastSpell(pCCTarget, m_spells.mage.pPolymorph) &&
                    CanUseCrowdControl(m_spells.mage.pPolymorph, pCCTarget))
                {
                    if (DoCastSpell(pCCTarget, m_spells.mage.pPolymorph) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // === RANGE POSITIONING: maintain 25-30 yards ===
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE &&
            me->GetDistance(pVictim) > 30.0f)
        {
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        }

        // === DEFENSIVE SEQUENCE: melee attackers in range ===
        if (GetAttackersInRangeCount(10.0f))
        {
            // Mana Shield if mana >20%
            if (m_spells.mage.pManaShield &&
               (me->GetPowerPercent(POWER_MANA) > 20.0f) &&
                CanTryToCastSpell(me, m_spells.mage.pManaShield))
            {
                if (DoCastSpell(me, m_spells.mage.pManaShield) == SPELL_CAST_OK)
                    return;
            }

            if (!me->HasUnitState(UNIT_STATE_CAN_NOT_MOVE))
            {
                // Frost Nova to root nearby enemies before blinking out
                if (m_spells.mage.pFrostNova &&
                   !pVictim->HasUnitState(UNIT_STATE_ROOT) &&
                   !pVictim->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL) &&
                    CanTryToCastSpell(me, m_spells.mage.pFrostNova))
                {
                    DoCastSpell(me, m_spells.mage.pFrostNova);
                }

                // Run away and set ranged kite distance
                if (RunAwayFromTarget(pVictim))
                {
                    me->SetCasterChaseDistance(25.0f);
                    return;
                }
            }

            // Blink away if rooted or snared
            if (m_spells.mage.pBlink &&
               (me->HasUnitState(UNIT_STATE_CAN_NOT_MOVE) ||
                me->HasAuraType(SPELL_AURA_MOD_DECREASE_SPEED)) &&
                CanTryToCastSpell(me, m_spells.mage.pBlink))
            {
                if (me->GetMotionMaster()->GetCurrentMovementGeneratorType())
                    me->GetMotionMaster()->Clear();

                if (DoCastSpell(me, m_spells.mage.pBlink) == SPELL_CAST_OK)
                    return;
            }
        }

        // === AOE: 2+ enemies near self ===
        if (me->GetEnemyCountInRadiusAround(me, 10.0f) > 1)
        {
            if (m_spells.mage.pConeofCold && !me->IsMoving() &&
                CanTryToCastSpell(me, m_spells.mage.pConeofCold))
            {
                if (DoCastSpell(pVictim, m_spells.mage.pConeofCold) == SPELL_CAST_OK)
                    return;
            }

            if (m_spells.mage.pBlastWave &&
                CanTryToCastSpell(me, m_spells.mage.pBlastWave))
            {
                if (DoCastSpell(me, m_spells.mage.pBlastWave) == SPELL_CAST_OK)
                    return;
            }

            if (m_spells.mage.pArcaneExplosion &&
                CanTryToCastSpell(me, m_spells.mage.pArcaneExplosion))
            {
                if (DoCastSpell(me, m_spells.mage.pArcaneExplosion) == SPELL_CAST_OK)
                    return;
            }
        }

        // === BLIZZARD: 3+ enemies grouped on target ===
        if (m_spells.mage.pBlizzard &&
           (me->GetEnemyCountInRadiusAround(pVictim, 10.0f) > 2) &&
            CanTryToCastSpell(pVictim, m_spells.mage.pBlizzard))
        {
            if (DoCastSpell(pVictim, m_spells.mage.pBlizzard) == SPELL_CAST_OK)
                return;
        }

        // Don't cast while actively distancing
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == DISTANCING_MOTION_TYPE)
            return;

        // === COOLDOWNS ===

        // Combustion (fire crit buff — activate before fire spells)
        if (m_spells.mage.pCombustion &&
            CanTryToCastSpell(me, m_spells.mage.pCombustion))
        {
            if (DoCastSpell(me, m_spells.mage.pCombustion) == SPELL_CAST_OK)
                return;
        }

        // Arcane Power (damage boost, mana >50%)
        if (m_spells.mage.pArcanePower &&
           (me->GetPowerPercent(POWER_MANA) > 50.0f) &&
            CanTryToCastSpell(me, m_spells.mage.pArcanePower))
        {
            if (DoCastSpell(me, m_spells.mage.pArcanePower) == SPELL_CAST_OK)
                return;
        }

        // Presence of Mind (instant next cast, mana >50%)
        if (m_spells.mage.pPresenceOfMind &&
           (me->GetPowerPercent(POWER_MANA) > 50.0f) &&
            CanTryToCastSpell(me, m_spells.mage.pPresenceOfMind))
        {
            if (DoCastSpell(me, m_spells.mage.pPresenceOfMind) == SPELL_CAST_OK)
                return;
        }

        // Pyroblast — only with Presence of Mind aura active OR opener at range
        if (m_spells.mage.pPyroblast &&
           ((m_spells.mage.pPresenceOfMind && me->HasAura(m_spells.mage.pPresenceOfMind->Id)) ||
            (!pVictim->IsInCombat() && (pVictim->GetMaxHealth() > me->GetMaxHealth()) && (me->GetDistance(pVictim) > 30.0f))) &&
            CanTryToCastSpell(pVictim, m_spells.mage.pPyroblast))
        {
            if (DoCastSpell(pVictim, m_spells.mage.pPyroblast) == SPELL_CAST_OK)
                return;
        }

        // === EXECUTE PHASE: Scorch at <20% target HP (fast cast) ===
        if (m_spells.mage.pScorch &&
           (pVictim->GetHealthPercent() < 20.0f) &&
            CanTryToCastSpell(pVictim, m_spells.mage.pScorch))
        {
            if (DoCastSpell(pVictim, m_spells.mage.pScorch) == SPELL_CAST_OK)
                return;
        }

        // === PRIMARY NUKES — threat-aware (skip if ShouldThrottleDPS) ===
        if (!ShouldThrottleDPS(pVictim))
        {
            // Frostbolt (primary nuke)
            if (m_spells.mage.pFrostbolt &&
                CanTryToCastSpell(pVictim, m_spells.mage.pFrostbolt))
            {
                if (DoCastSpell(pVictim, m_spells.mage.pFrostbolt) == SPELL_CAST_OK)
                    return;
            }

            // Fire Blast (instant, use between Frostbolts)
            if (m_spells.mage.pFireBlast &&
                CanTryToCastSpell(pVictim, m_spells.mage.pFireBlast))
            {
                if (DoCastSpell(pVictim, m_spells.mage.pFireBlast) == SPELL_CAST_OK)
                    return;
            }

            // Fireball (fire spec alternative)
            if (m_spells.mage.pFireball &&
                CanTryToCastSpell(pVictim, m_spells.mage.pFireball))
            {
                if (DoCastSpell(pVictim, m_spells.mage.pFireball) == SPELL_CAST_OK)
                    return;
            }
        }

        // === EVOCATION: mana recovery when <30% mana AND safe (no melee range) ===
        if (m_spells.mage.pEvocation &&
           (me->GetPowerPercent(POWER_MANA) < 30.0f) &&
           (GetAttackersInRangeCount(10.0f) == 0) &&
            CanTryToCastSpell(me, m_spells.mage.pEvocation))
        {
            if (DoCastSpell(me, m_spells.mage.pEvocation) == SPELL_CAST_OK)
                return;
        }

        // === WAND: when OOM (<5% mana) ===
        if (me->HasSpell(PB_SPELL_SHOOT_WAND_M) &&
           !me->IsMoving() &&
           (me->GetPowerPercent(POWER_MANA) < 5.0f) &&
           !me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
            me->CastSpell(pVictim, PB_SPELL_SHOOT_WAND_M, false);
    }
}
