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

bool PartyBotAI::ShouldEnterStealth() const
{
    if (me->IsMounted())
        return false;

    if (me->GetVictim() || me->InBattleGround() || me->IsFFAPvP())
        return true;

    if (me->GetHealthPercent() < 10.0f)
        return true;

    if (Player* pLeader = GetPartyLeader())
    {
        if (pLeader->IsDead() || pLeader->IsFeigningDeathSuccessfully() ||
            pLeader->HasAuraType(SPELL_AURA_MOD_STEALTH) ||
            pLeader->HasAuraType(SPELL_AURA_MOD_INVISIBILITY))
            return true;
    }

    return false;
}

bool PartyBotAI::EnterStealthIfNeeded(SpellEntry const* pStealthSpell)
{
    if (pStealthSpell)
    {
        bool const shouldStealth = ShouldEnterStealth();

        if (me->HasAura(pStealthSpell->Id))
        {
            if (!shouldStealth)
                me->RemoveAurasDueToSpellByCancel(pStealthSpell->Id);
        }
        else
        {
            if (shouldStealth &&
                CanTryToCastSpell(me, pStealthSpell) &&
                DoCastSpell(me, pStealthSpell) == SPELL_CAST_OK)
                return true;
        }
    }

    return false;
}

void PartyBotAI::UpdateOutOfCombatAI_Rogue()
{
    if (m_spells.rogue.pMainHandPoison &&
        CanTryToCastSpell(me, m_spells.rogue.pMainHandPoison))
    {
        if (CastWeaponBuff(m_spells.rogue.pMainHandPoison, EQUIPMENT_SLOT_MAINHAND) == SPELL_CAST_OK)
            return;
    }

    if (m_spells.rogue.pOffHandPoison &&
        CanTryToCastSpell(me, m_spells.rogue.pOffHandPoison))
    {
        if (CastWeaponBuff(m_spells.rogue.pOffHandPoison, EQUIPMENT_SLOT_OFFHAND) == SPELL_CAST_OK)
            return;
    }

    if (EnterStealthIfNeeded(m_spells.rogue.pStealth))
        return;

    if (me->GetVictim())
        UpdateInCombatAI_Rogue();
}

void PartyBotAI::UpdateInCombatAI_Rogue()
{
    // === INTERRUPT — highest priority: Kick immediately when target is casting ===
    // Reserve 25 energy so Kick is always available when needed.
    if (Unit* pVictim = me->GetVictim())
    {
        if (m_spells.rogue.pKick &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.rogue.pKick))
        {
            if (DoCastSpell(pVictim, m_spells.rogue.pKick) == SPELL_CAST_OK)
                return;
        }
    }

    // === THREAT MANAGEMENT — Vanish emergency dump at >=90% threat ===
    if (m_spells.rogue.pVanish && me->GetVictim() &&
        GetThreatRatio(me->GetVictim()) >= 0.90f &&
        CanTryToCastSpell(me, m_spells.rogue.pVanish))
    {
        if (DoCastSpell(me, m_spells.rogue.pVanish) == SPELL_CAST_OK)
            return;
    }

    // === FEINT — proactive threat management at >=70% threat ===
    if (m_spells.rogue.pFeint && me->GetVictim() &&
        GetThreatRatio(me->GetVictim()) >= 0.70f &&
        CanTryToCastSpell(me, m_spells.rogue.pFeint))
    {
        if (DoCastSpell(me, m_spells.rogue.pFeint) == SPELL_CAST_OK)
            return;
    }

    if (Unit* pVictim = me->GetVictim())
    {
        // === STEALTH OPENERS ===
        if (me->HasAuraType(SPELL_AURA_MOD_STEALTH))
        {
            // Premeditation first to front-load combo points
            if (m_spells.rogue.pPremeditation &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pPremeditation))
            {
                DoCastSpell(pVictim, m_spells.rogue.pPremeditation);
            }

            if (pVictim->IsCaster())
            {
                // Garrote silences casters — prevents spell casts during fight
                if (m_spells.rogue.pGarrote &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pGarrote))
                {
                    if (DoCastSpell(pVictim, m_spells.rogue.pGarrote) == SPELL_CAST_OK)
                        return;
                }
            }
            else
            {
                // Ambush (dagger) for highest burst from stealth vs melee targets
                if (m_spells.rogue.pAmbush &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pAmbush))
                {
                    if (DoCastSpell(pVictim, m_spells.rogue.pAmbush) == SPELL_CAST_OK)
                        return;
                }

                // Cheap Shot as stun opener when Ambush is unavailable
                if (m_spells.rogue.pCheapShot &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pCheapShot))
                {
                    if (DoCastSpell(pVictim, m_spells.rogue.pCheapShot) == SPELL_CAST_OK)
                        return;
                }
            }
        }
        else
        {
            // === EMERGENCY SURVIVAL ===

            // Vanish at <10% HP to escape — use Preparation to reset if needed
            if (m_spells.rogue.pVanish &&
                (me->GetHealthPercent() < 10.0f))
            {
                if (m_spells.rogue.pPreparation &&
                    !me->IsSpellReady(m_spells.rogue.pVanish->Id) &&
                    CanTryToCastSpell(me, m_spells.rogue.pPreparation))
                {
                    if (DoCastSpell(me, m_spells.rogue.pPreparation) == SPELL_CAST_OK)
                        return;
                }

                if (CanTryToCastSpell(me, m_spells.rogue.pVanish))
                {
                    if (DoCastSpell(me, m_spells.rogue.pVanish) == SPELL_CAST_OK)
                    {
                        if (RunAwayFromTarget(pVictim))
                            return;
                    }
                }
            }

            // Evasion when HP <80% and multiple melee attackers are on us
            if (m_spells.rogue.pEvasion &&
               (me->GetHealthPercent() < 80.0f) &&
               (GetAttackersInRangeCount(10.0f) >= 2) &&
                CanTryToCastSpell(me, m_spells.rogue.pEvasion))
            {
                if (DoCastSpell(me, m_spells.rogue.pEvasion) == SPELL_CAST_OK)
                    return;
            }

            // === FINISHERS at 5 combo points — DETERMINISTIC priority ===
            if (me->GetComboPoints() >= 5)
            {
                // Priority 1: Slice and Dice if not active and target has HP left
                if (m_spells.rogue.pSliceAndDice &&
                    !me->HasAura(m_spells.rogue.pSliceAndDice->Id) &&
                    pVictim->GetHealthPercent() > 10.0f &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pSliceAndDice))
                {
                    // Cold Blood before finisher if available
                    if (m_spells.rogue.pColdBlood &&
                        CanTryToCastSpell(me, m_spells.rogue.pColdBlood))
                    {
                        DoCastSpell(me, m_spells.rogue.pColdBlood);
                    }

                    if (DoCastSpell(pVictim, m_spells.rogue.pSliceAndDice) == SPELL_CAST_OK)
                        return;
                }

                // Priority 2: Kidney Shot when target is casting or multiple mobs
                if (m_spells.rogue.pKidneyShot &&
                    !pVictim->IsImmuneToMechanic(MECHANIC_STUN) &&
                    (pVictim->IsNonMeleeSpellCasted() || GetAttackersInRangeCount(10.0f) >= 2) &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pKidneyShot))
                {
                    // Cold Blood before finisher if available
                    if (m_spells.rogue.pColdBlood &&
                        CanTryToCastSpell(me, m_spells.rogue.pColdBlood))
                    {
                        DoCastSpell(me, m_spells.rogue.pColdBlood);
                    }

                    if (DoCastSpell(pVictim, m_spells.rogue.pKidneyShot) == SPELL_CAST_OK)
                        return;
                }

                // Priority 3: Eviscerate — default damage finisher
                if (m_spells.rogue.pEviscerate &&
                    CanTryToCastSpell(pVictim, m_spells.rogue.pEviscerate))
                {
                    // Cold Blood before finisher if available
                    if (m_spells.rogue.pColdBlood &&
                        CanTryToCastSpell(me, m_spells.rogue.pColdBlood))
                    {
                        DoCastSpell(me, m_spells.rogue.pColdBlood);
                    }

                    if (DoCastSpell(pVictim, m_spells.rogue.pEviscerate) == SPELL_CAST_OK)
                        return;
                }
            }

            // === BLADE FLURRY — activate on 2+ enemies in range ===
            if (m_spells.rogue.pBladeFlurry &&
                GetAttackersInRangeCount(10.0f) >= 2 &&
                CanTryToCastSpell(me, m_spells.rogue.pBladeFlurry))
            {
                if (DoCastSpell(me, m_spells.rogue.pBladeFlurry) == SPELL_CAST_OK)
                    return;
            }

            // === BLIND — crowd control a different attacker ===
            if (m_spells.rogue.pBlind)
            {
                if (Unit* pTarget = SelectAttackerDifferentFrom(pVictim))
                {
                    if (CanTryToCastSpell(pTarget, m_spells.rogue.pBlind) &&
                        CanUseCrowdControl(m_spells.rogue.pBlind, pTarget))
                    {
                        if (DoCastSpell(pTarget, m_spells.rogue.pBlind) == SPELL_CAST_OK)
                        {
                            me->AttackStop();
                            AttackStart(pVictim);
                            return;
                        }
                    }
                }
            }

            // === ADRENALINE RUSH — use when energy is depleted ===
            if (m_spells.rogue.pAdrenalineRush &&
               !me->GetPower(POWER_ENERGY) &&
                CanTryToCastSpell(me, m_spells.rogue.pAdrenalineRush))
            {
                if (DoCastSpell(me, m_spells.rogue.pAdrenalineRush) == SPELL_CAST_OK)
                    return;
            }

            // === COMBO POINT BUILDERS — priority order ===

            // Backstab — highest damage builder, requires being behind target
            if (m_spells.rogue.pBackstab &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pBackstab))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pBackstab) == SPELL_CAST_OK)
                    return;
            }

            // Ghostly Strike — strong builder with dodge buff
            if (m_spells.rogue.pGhostlyStrike &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pGhostlyStrike))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pGhostlyStrike) == SPELL_CAST_OK)
                    return;
            }

            // Hemorrhage — applies bleed debuff for party melee benefit
            if (m_spells.rogue.pHemorrhage &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pHemorrhage))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pHemorrhage) == SPELL_CAST_OK)
                    return;
            }

            // Sinister Strike — reliable default filler
            if (m_spells.rogue.pSinisterStrike &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pSinisterStrike))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pSinisterStrike) == SPELL_CAST_OK)
                    return;
            }

            // Riposte — parry proc counter-attack
            if (m_spells.rogue.pRiposte &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pRiposte))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pRiposte) == SPELL_CAST_OK)
                    return;
            }

            // === SPRINT — close gap on ranged targets ===
            if (m_spells.rogue.pSprint &&
               !me->HasUnitState(UNIT_STATE_ROOT) &&
               !me->CanReachWithMeleeAutoAttack(pVictim) &&
                CanTryToCastSpell(me, m_spells.rogue.pSprint))
            {
                if (DoCastSpell(me, m_spells.rogue.pSprint) == SPELL_CAST_OK)
                    return;
            }

            // === GOUGE — backup interrupt when Kick is on cooldown ===
            if (m_spells.rogue.pGouge &&
                pVictim->IsNonMeleeSpellCasted() &&
                CanTryToCastSpell(pVictim, m_spells.rogue.pGouge))
            {
                if (DoCastSpell(pVictim, m_spells.rogue.pGouge) == SPELL_CAST_OK)
                    return;
            }
        }
    }
}
