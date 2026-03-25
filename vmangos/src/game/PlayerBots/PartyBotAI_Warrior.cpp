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

void PartyBotAI::UpdateOutOfCombatAI_Warrior()
{
    // Tanks: stay in Defensive Stance, DPS: Battle Stance
    if (m_role == ROLE_TANK)
    {
        if (m_spells.warrior.pDefensiveStance &&
            CanTryToCastSpell(me, m_spells.warrior.pDefensiveStance))
        {
            DoCastSpell(me, m_spells.warrior.pDefensiveStance);
        }
    }
    else
    {
        if (m_spells.warrior.pBattleStance &&
            CanTryToCastSpell(me, m_spells.warrior.pBattleStance))
        {
            if (DoCastSpell(me, m_spells.warrior.pBattleStance) == SPELL_CAST_OK)
                return;
        }
    }

    // Bloodrage before pulls to bank rage (rage decays out of combat)
    if (m_spells.warrior.pBloodrage &&
        me->GetPower(POWER_RAGE) < 10 &&
        me->GetVictim() &&
        CanTryToCastSpell(me, m_spells.warrior.pBloodrage))
    {
        DoCastSpell(me, m_spells.warrior.pBloodrage);
    }

    // Maintain Battle Shout
    if (m_spells.warrior.pBattleShout &&
       !me->HasAura(m_spells.warrior.pBattleShout->Id))
    {
        if (CanTryToCastSpell(me, m_spells.warrior.pBattleShout))
            DoCastSpell(me, m_spells.warrior.pBattleShout);
        else if (m_spells.warrior.pBloodrage &&
            (me->GetPower(POWER_RAGE) < 10) &&
            CanTryToCastSpell(me, m_spells.warrior.pBloodrage))
        {
            DoCastSpell(me, m_spells.warrior.pBloodrage);
        }
    }

    // Charge on pull (DPS should NOT charge — only tank)
    if (Unit* pVictim = me->GetVictim())
    {
        if (m_role == ROLE_TANK && m_spells.warrior.pCharge &&
            CanTryToCastSpell(pVictim, m_spells.warrior.pCharge))
        {
            if (DoCastSpell(pVictim, m_spells.warrior.pCharge) == SPELL_CAST_OK)
                return;
        }
    }
}

void PartyBotAI::UpdateInCombatAI_Warrior()
{
    if (Unit* pVictim = me->GetVictim())
    {
        uint32 const enemyCount = GetAttackersInRangeCount(10.0f);
        uint32 const rage = me->GetPower(POWER_RAGE);
        bool const hasShield = IsWearingShield(me);
        bool const inDefStance = (me->GetShapeshiftForm() == FORM_DEFENSIVESTANCE);

        // === INTERRUPT (both tank and DPS — always highest priority) ===
        if (pVictim->IsNonMeleeSpellCasted(false, false, true))
        {
            if (m_spells.warrior.pPummel &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pPummel))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pPummel) == SPELL_CAST_OK)
                    return;
            }
            if (m_spells.warrior.pShieldBash && hasShield &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pShieldBash))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pShieldBash) == SPELL_CAST_OK)
                    return;
            }
        }

        // === EMERGENCY RESPONSES (both tank and DPS) ===
        if (m_spells.warrior.pLastStand &&
            me->GetHealthPercent() < 20.0f &&
            CanTryToCastSpell(me, m_spells.warrior.pLastStand))
        {
            if (DoCastSpell(me, m_spells.warrior.pLastStand) == SPELL_CAST_OK)
                return;
        }
        if (m_spells.warrior.pShieldWall && hasShield &&
            me->GetHealthPercent() < 20.0f &&
            CanTryToCastSpell(me, m_spells.warrior.pShieldWall))
        {
            if (DoCastSpell(me, m_spells.warrior.pShieldWall) == SPELL_CAST_OK)
                return;
        }

        // ============================================================
        // PROTECTION WARRIOR (TANK) ROTATION
        // Per CLASS_GROUP_BEHAVIOR_GUIDE: Shield Slam > Revenge > Sunder > Shield Block
        // ============================================================
        if (m_role == ROLE_TANK)
        {
            // Ensure Defensive Stance
            if (m_spells.warrior.pDefensiveStance && !inDefStance &&
                CanTryToCastSpell(me, m_spells.warrior.pDefensiveStance))
            {
                DoCastSpell(me, m_spells.warrior.pDefensiveStance);
            }

            // AoE emergency: Challenging Shout when multiple mobs are loose
            if (m_spells.warrior.pChallengingShout && enemyCount >= 3)
            {
                uint8 looseMobs = 0;
                for (auto const& pAttacker : me->GetAttackers())
                    if (pAttacker->GetVictim() != me)
                        looseMobs++;
                if (looseMobs >= 2 && CanTryToCastSpell(me, m_spells.warrior.pChallengingShout))
                {
                    if (DoCastSpell(me, m_spells.warrior.pChallengingShout) == SPELL_CAST_OK)
                        return;
                }
            }

            // Shield Block on cooldown (guarantees Revenge procs)
            if (m_spells.warrior.pShieldBlock && hasShield &&
                !me->GetAttackers().empty() &&
                CanTryToCastSpell(me, m_spells.warrior.pShieldBlock))
            {
                if (DoCastSpell(me, m_spells.warrior.pShieldBlock) == SPELL_CAST_OK)
                    return;
            }

            // Priority 1: Shield Slam (highest threat per rage)
            if (m_spells.warrior.pShieldSlam && hasShield &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pShieldSlam))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pShieldSlam) == SPELL_CAST_OK)
                    return;
            }

            // Priority 2: Revenge (proc-based, very high threat, only 5 rage)
            if (m_spells.warrior.pRevenge &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pRevenge))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pRevenge) == SPELL_CAST_OK)
                    return;
            }

            // Priority 3: Sunder Armor (threat + armor debuff, apply/maintain 5 stacks)
            if (m_spells.warrior.pSunderArmor &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pSunderArmor))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pSunderArmor) == SPELL_CAST_OK)
                    return;
            }

            // Demoralizing Shout (keep up — AoE threat + damage reduction)
            if (m_spells.warrior.pDemoralizingShout &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pDemoralizingShout))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pDemoralizingShout) == SPELL_CAST_OK)
                    return;
            }

            // Battle Shout (maintain — AoE threat per buffed party member)
            if (m_spells.warrior.pBattleShout &&
               !me->HasAura(m_spells.warrior.pBattleShout->Id) &&
                CanTryToCastSpell(me, m_spells.warrior.pBattleShout))
            {
                if (DoCastSpell(me, m_spells.warrior.pBattleShout) == SPELL_CAST_OK)
                    return;
            }

            // Concussion Blow (stun for control)
            if (m_spells.warrior.pConcussionBlow &&
               (pVictim->IsNonMeleeSpellCasted() || me->GetHealthPercent() < 40.0f) &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pConcussionBlow))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pConcussionBlow) == SPELL_CAST_OK)
                    return;
            }

            // Rage dump: Heroic Strike when rage > 50 (single), Cleave (AoE)
            if (rage > 50)
            {
                if (enemyCount > 1 && m_spells.warrior.pCleave &&
                    CanTryToCastSpell(pVictim, m_spells.warrior.pCleave))
                {
                    if (DoCastSpell(pVictim, m_spells.warrior.pCleave) == SPELL_CAST_OK)
                        return;
                }
                else if (m_spells.warrior.pHeroicStrike &&
                    CanTryToCastSpell(pVictim, m_spells.warrior.pHeroicStrike))
                {
                    if (DoCastSpell(pVictim, m_spells.warrior.pHeroicStrike) == SPELL_CAST_OK)
                        return;
                }
            }
        }
        // ============================================================
        // ARMS/FURY WARRIOR (DPS) ROTATION
        // Per CLASS_GROUP_BEHAVIOR_GUIDE: BT/MS > WW > Execute > HS dump
        // ============================================================
        else
        {
            // Prefer Berserker Stance for DPS (unless low HP)
            if (me->GetHealthPercent() < 20.0f)
            {
                if (m_spells.warrior.pDefensiveStance &&
                    CanTryToCastSpell(me, m_spells.warrior.pDefensiveStance))
                    DoCastSpell(me, m_spells.warrior.pDefensiveStance);
            }
            else if (m_spells.warrior.pBerserkerStance &&
                CanTryToCastSpell(me, m_spells.warrior.pBerserkerStance))
            {
                DoCastSpell(me, m_spells.warrior.pBerserkerStance);
            }

            // Execute phase (target < 20% HP) — dump all rage
            if (m_spells.warrior.pExecute &&
                pVictim->GetHealthPercent() < 20.0f &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pExecute))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pExecute) == SPELL_CAST_OK)
                    return;
            }

            // DPS cooldowns
            if (me->GetHealthPercent() > 60.0f && pVictim->GetHealthPercent() > 40.0f)
            {
                if (m_spells.warrior.pDeathWish &&
                    CanTryToCastSpell(me, m_spells.warrior.pDeathWish))
                {
                    if (DoCastSpell(me, m_spells.warrior.pDeathWish) == SPELL_CAST_OK)
                        return;
                }
                if (m_spells.warrior.pRecklessness &&
                    CanTryToCastSpell(me, m_spells.warrior.pRecklessness))
                {
                    if (DoCastSpell(me, m_spells.warrior.pRecklessness) == SPELL_CAST_OK)
                        return;
                }
            }

            // AoE: Sweeping Strikes + Whirlwind
            if (enemyCount > 2)
            {
                if (m_spells.warrior.pSweepingStrikes &&
                    CanTryToCastSpell(me, m_spells.warrior.pSweepingStrikes))
                {
                    if (DoCastSpell(me, m_spells.warrior.pSweepingStrikes) == SPELL_CAST_OK)
                        return;
                }
            }

            // Priority 1: Bloodthirst (Fury) or Mortal Strike (Arms)
            if (m_spells.warrior.pBloodthirst &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pBloodthirst))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pBloodthirst) == SPELL_CAST_OK)
                    return;
            }
            if (m_spells.warrior.pMortalStrike &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pMortalStrike))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pMortalStrike) == SPELL_CAST_OK)
                    return;
            }

            // Priority 2: Whirlwind
            if (m_spells.warrior.pWhirlwind &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pWhirlwind))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pWhirlwind) == SPELL_CAST_OK)
                    return;
            }

            // Overpower (proc on dodge)
            if (m_spells.warrior.pOverpower &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pOverpower))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pOverpower) == SPELL_CAST_OK)
                    return;
            }

            // Hamstring (only if target is fleeing)
            if (m_spells.warrior.pHamstring &&
                pVictim->IsMoving() &&
               !pVictim->HasUnitState(UNIT_STATE_ROOT) &&
               !pVictim->HasAuraType(SPELL_AURA_MOD_DECREASE_SPEED) &&
                CanTryToCastSpell(pVictim, m_spells.warrior.pHamstring))
            {
                if (DoCastSpell(pVictim, m_spells.warrior.pHamstring) == SPELL_CAST_OK)
                    return;
            }

            // Rage dump: Heroic Strike at rage > 50 (single), Cleave (AoE)
            if (rage > 50)
            {
                if (enemyCount > 1 && m_spells.warrior.pCleave &&
                    CanTryToCastSpell(pVictim, m_spells.warrior.pCleave))
                {
                    if (DoCastSpell(pVictim, m_spells.warrior.pCleave) == SPELL_CAST_OK)
                        return;
                }
                else if (m_spells.warrior.pHeroicStrike &&
                    CanTryToCastSpell(pVictim, m_spells.warrior.pHeroicStrike))
                {
                    if (DoCastSpell(pVictim, m_spells.warrior.pHeroicStrike) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Chase if not in melee range (both tank and DPS)
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
            && !me->CanReachWithMeleeAutoAttack(pVictim))
        {
            me->GetMotionMaster()->MoveChase(pVictim);
        }
    }
    else // no victim
    {
        // Maintain Battle Shout
        if (m_spells.warrior.pBattleShout &&
            CanTryToCastSpell(me, m_spells.warrior.pBattleShout))
        {
            if (DoCastSpell(me, m_spells.warrior.pBattleShout) == SPELL_CAST_OK)
                return;
        }
    }
}
