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
#include "CreatureAI.h"

enum PartyBotSpells_Hunter
{
    PB_SPELL_AUTO_SHOT_H = 75,
};

void PartyBotAI::UpdateOutOfCombatAI_Hunter()
{
    // Aspect of the Hawk (default ranged aspect out of combat)
    if (m_spells.hunter.pAspectOfTheHawk &&
        CanTryToCastSpell(me, m_spells.hunter.pAspectOfTheHawk))
    {
        if (DoCastSpell(me, m_spells.hunter.pAspectOfTheHawk) == SPELL_CAST_OK)
            return;
    }

    // Summon pet when idle
    SummonPetIfNeeded();

    if (Unit* pVictim = me->GetVictim())
    {
        // Hunter's Mark before the pull
        if (m_spells.hunter.pHuntersMark &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pHuntersMark))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pHuntersMark) == SPELL_CAST_OK)
                return;
        }

        // Send pet in at the start of combat
        if (Pet* pPet = me->GetPet())
        {
            if (!pPet->GetVictim())
            {
                pPet->GetCharmInfo()->SetIsCommandAttack(true);
                pPet->AI()->AttackStart(pVictim);
            }
        }

        UpdateInCombatAI_Hunter();
    }
}

void PartyBotAI::UpdateInCombatAI_Hunter()
{
    // === PROACTIVE FEIGN DEATH at >=80% threat ratio ===
    if (m_spells.hunter.pFeignDeath && me->GetVictim() &&
        GetThreatRatio(me->GetVictim()) >= 0.80f &&
        CanTryToCastSpell(me, m_spells.hunter.pFeignDeath))
    {
        if (DoCastSpell(me, m_spells.hunter.pFeignDeath) == SPELL_CAST_OK)
            return;
    }

    if (Unit* pVictim = me->GetVictim())
    {
        bool const highThreat = ShouldThrottleDPS(pVictim);
        float const combatDist = me->GetCombatDistance(pVictim);

        // === PET MANAGEMENT: keep pet on target ===
        if (Pet* pPet = me->GetPet())
        {
            if (!pPet->GetVictim())
            {
                pPet->GetCharmInfo()->SetIsCommandAttack(true);
                pPet->AI()->AttackStart(pVictim);
            }
        }

        // === DEAD ZONE / MELEE RANGE HANDLING (<8 yards) ===
        if (combatDist < 8.0f || pVictim->CanReachWithMeleeAutoAttack(me))
        {
            // Emergency Feign Death at low HP when in melee
            if (m_spells.hunter.pFeignDeath &&
               (me->GetHealthPercent() < 20.0f) &&
                CanTryToCastSpell(me, m_spells.hunter.pFeignDeath))
            {
                if (DoCastSpell(me, m_spells.hunter.pFeignDeath) == SPELL_CAST_OK)
                    return;
            }

            // Disengage to escape melee
            if (m_spells.hunter.pDisengage &&
                CanTryToCastSpell(pVictim, m_spells.hunter.pDisengage))
            {
                if (DoCastSpell(pVictim, m_spells.hunter.pDisengage) == SPELL_CAST_OK)
                    return;
            }

            // Aspect of the Monkey while stuck in melee
            if (m_spells.hunter.pAspectOfTheMonkey &&
                CanTryToCastSpell(me, m_spells.hunter.pAspectOfTheMonkey))
            {
                if (DoCastSpell(me, m_spells.hunter.pAspectOfTheMonkey) == SPELL_CAST_OK)
                    return;
            }

            // Wing Clip to slow the target so we can escape
            if (m_spells.hunter.pWingClip &&
                CanTryToCastSpell(pVictim, m_spells.hunter.pWingClip))
            {
                DoCastSpell(pVictim, m_spells.hunter.pWingClip);
            }

            // Mongoose Bite (melee proc, use while stuck in melee)
            if (m_spells.hunter.pMongooseBite &&
                CanTryToCastSpell(pVictim, m_spells.hunter.pMongooseBite))
            {
                DoCastSpell(pVictim, m_spells.hunter.pMongooseBite);
            }

            // Raptor Strike (melee filler while stuck in dead zone)
            if (m_spells.hunter.pRaptorStrike &&
                CanTryToCastSpell(pVictim, m_spells.hunter.pRaptorStrike))
            {
                DoCastSpell(pVictim, m_spells.hunter.pRaptorStrike);
            }

            // Run away from target if not assigned melee role
            if (!me->HasUnitState(UNIT_STATE_ROOT) &&
               (GetRole() != ROLE_MELEE_DPS) &&
                me->GetMotionMaster()->GetCurrentMovementGeneratorType() != DISTANCING_MOTION_TYPE)
            {
                if (!me->IsStopped())
                    me->StopMoving();
                me->GetMotionMaster()->Clear();
                RunAwayFromTarget(pVictim);
            }

            // Nothing more to do until we escape the dead zone
            return;
        }

        // === RANGE POSITIONING: maintain 25-30 yard sweet spot ===
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
            && me->GetDistance(pVictim) > 30.0f)
        {
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        }

        // Swap back to Aspect of the Hawk at range
        if (m_spells.hunter.pAspectOfTheHawk &&
            CanTryToCastSpell(me, m_spells.hunter.pAspectOfTheHawk))
        {
            if (DoCastSpell(me, m_spells.hunter.pAspectOfTheHawk) == SPELL_CAST_OK)
                return;
        }

        // === FREEZING TRAP: CC a secondary attacker ===
        if (m_spells.hunter.pFreezingTrap && CanUseCrowdControl(m_spells.hunter.pFreezingTrap, pVictim))
        {
            if (Unit* pOtherAttacker = SelectAttackerDifferentFrom(pVictim))
            {
                if (CanTryToCastSpell(pOtherAttacker, m_spells.hunter.pFreezingTrap) &&
                    CanUseCrowdControl(m_spells.hunter.pFreezingTrap, pOtherAttacker))
                {
                    if (DoCastSpell(pOtherAttacker, m_spells.hunter.pFreezingTrap) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // === CONCUSSIVE SHOT on casters to interrupt their cast ===
        if (m_spells.hunter.pConcussiveShot &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pConcussiveShot))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pConcussiveShot) == SPELL_CAST_OK)
                return;
        }

        // === VOLLEY: AoE when 3+ enemies are clustered ===
        if (!highThreat &&
            m_spells.hunter.pVolley &&
           (me->GetEnemyCountInRadiusAround(pVictim, 10.0f) >= 2) &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pVolley))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pVolley) == SPELL_CAST_OK)
                return;
        }

        // === SHOT ROTATION (Marksmanship priority) ===

        // Hunter's Mark (keep up — increases ranged AP against target)
        if (m_spells.hunter.pHuntersMark &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pHuntersMark))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pHuntersMark) == SPELL_CAST_OK)
                return;
        }

        // Aimed Shot (highest-priority ranged shot — skip when threat is high)
        if (!highThreat &&
            m_spells.hunter.pAimedShot &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pAimedShot))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pAimedShot) == SPELL_CAST_OK)
                return;
        }

        // Multi-Shot (AoE shot — only use on 3+ targets to save mana, skip when threat is high)
        if (!highThreat &&
            m_spells.hunter.pMultiShot &&
           (me->GetEnemyCountInRadiusAround(pVictim, 8.0f) >= 2) &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pMultiShot))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pMultiShot) == SPELL_CAST_OK)
                return;
        }

        // Serpent Sting (DoT — keep up for long fights / bosses)
        if (m_spells.hunter.pSerpentSting &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pSerpentSting))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pSerpentSting) == SPELL_CAST_OK)
                return;
        }

        // Arcane Shot (filler between auto shots — safe to use even at moderate threat)
        if (!highThreat &&
            m_spells.hunter.pArcaneShot &&
            CanTryToCastSpell(pVictim, m_spells.hunter.pArcaneShot))
        {
            if (DoCastSpell(pVictim, m_spells.hunter.pArcaneShot) == SPELL_CAST_OK)
                return;
        }

        // === AUTO SHOT: maintain ranged auto-attack ===
        // Only fire when not moving and not casting another spell, and outside dead zone
        if (me->HasSpell(PB_SPELL_AUTO_SHOT_H) &&
            !me->IsMoving() &&
            !me->IsNonMeleeSpellCasted())
        {
            switch (me->CastSpell(pVictim, PB_SPELL_AUTO_SHOT_H, false))
            {
                case SPELL_FAILED_NEED_AMMO:
                case SPELL_FAILED_NO_AMMO:
                {
                    AddHunterAmmo();
                    break;
                }
            }
        }
    }
}
