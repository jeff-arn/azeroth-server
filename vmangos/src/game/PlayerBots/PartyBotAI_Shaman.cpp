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

// Context-aware totem selection based on role, party composition, and encounter.
// Overrides the random-selection default from SummonShamanTotems().
bool PartyBotAI::SummonShamanTotemsContext()
{
    bool dropped = false;

    // --- AIR TOTEM ---
    if (!me->GetTotem(TOTEM_SLOT_AIR))
    {
        SpellEntry const* pAir = nullptr;
        // Melee DPS in party → Windfury; Healer → Tranquil Air if available
        if (m_spells.shaman.pWindfuryTotem && GetRole() != ROLE_HEALER)
            pAir = m_spells.shaman.pWindfuryTotem;
        else if (m_spells.shaman.pTranquilAirTotem && GetRole() == ROLE_HEALER)
            pAir = m_spells.shaman.pTranquilAirTotem;
        else
            pAir = m_spells.shaman.pAirTotem; // fallback random

        if (pAir && CanTryToCastSpell(me, pAir))
        {
            if (DoCastSpell(me, pAir) == SPELL_CAST_OK)
                return true;
        }
    }

    // --- EARTH TOTEM ---
    if (!me->GetTotem(TOTEM_SLOT_EARTH))
    {
        SpellEntry const* pEarth = nullptr;

        // Check boss overrides for fear mechanics → Tremor Totem
        if (m_spells.shaman.pTremorTotem)
        {
            if (BossOverride const* pOverride = GetCurrentBossOverride())
            {
                // If boss has any mechanics, Tremor is generally useful for fear bosses
                // We use Tremor as default earth totem during boss fights
                pEarth = m_spells.shaman.pTremorTotem;
            }
        }

        // If healer has aggro → Stoneclaw to peel
        if (!pEarth && m_spells.shaman.pStoneclawTotem &&
            GetRole() == ROLE_HEALER && !me->GetAttackers().empty())
        {
            pEarth = m_spells.shaman.pStoneclawTotem;
        }

        // Default: Strength of Earth for melee groups, Stoneskin otherwise
        if (!pEarth)
        {
            if (m_spells.shaman.pStrengthOfEarthTotem)
                pEarth = m_spells.shaman.pStrengthOfEarthTotem;
            else
                pEarth = m_spells.shaman.pEarthTotem; // fallback random
        }

        if (pEarth && CanTryToCastSpell(me, pEarth))
        {
            if (DoCastSpell(me, pEarth) == SPELL_CAST_OK)
                return true;
        }
    }

    // --- FIRE TOTEM ---
    if (!me->GetTotem(TOTEM_SLOT_FIRE))
    {
        SpellEntry const* pFire = nullptr;

        // Default: Searing Totem for passive DPS
        if (m_spells.shaman.pSearingTotem)
            pFire = m_spells.shaman.pSearingTotem;
        else
            pFire = m_spells.shaman.pFireTotem; // fallback random

        if (pFire && CanTryToCastSpell(me, pFire))
        {
            if (DoCastSpell(me, pFire) == SPELL_CAST_OK)
                return true;
        }
    }

    // --- WATER TOTEM ---
    if (!me->GetTotem(TOTEM_SLOT_WATER))
    {
        SpellEntry const* pWater = nullptr;

        // Healers/casters prefer Mana Spring; melee DPS prefer Healing Stream
        if (m_spells.shaman.pManaSpringTotem &&
            (GetRole() == ROLE_HEALER || GetRole() == ROLE_RANGE_DPS))
            pWater = m_spells.shaman.pManaSpringTotem;
        else if (m_spells.shaman.pHealingStreamTotem)
            pWater = m_spells.shaman.pHealingStreamTotem;
        else
            pWater = m_spells.shaman.pWaterTotem; // fallback random

        if (pWater && CanTryToCastSpell(me, pWater))
        {
            if (DoCastSpell(me, pWater) == SPELL_CAST_OK)
                return true;
        }
    }

    return false;
}

void PartyBotAI::UpdateOutOfCombatAI_Shaman()
{
    // Weapon buff
    if (m_spells.shaman.pWeaponBuff &&
        CanTryToCastSpell(me, m_spells.shaman.pWeaponBuff))
    {
        if (CastWeaponBuff(m_spells.shaman.pWeaponBuff, EQUIPMENT_SLOT_MAINHAND) == SPELL_CAST_OK)
            return;
    }

    // Lightning Shield (all specs keep this up)
    if (m_spells.shaman.pLightningShield &&
        CanTryToCastSpell(me, m_spells.shaman.pLightningShield))
    {
        if (DoCastSpell(me, m_spells.shaman.pLightningShield) == SPELL_CAST_OK)
            return;
    }

    // Healers: heal between pulls
    if (GetRole() == ROLE_HEALER && FindAndHealInjuredAlly())
        return;

    if (me->GetVictim())
    {
        // Drop totems as combat starts (context-aware)
        if (SummonShamanTotemsContext())
            return;

        UpdateInCombatAI_Shaman();
    }
}

void PartyBotAI::UpdateInCombatAI_Shaman()
{
    // === INTERRUPT — highest priority for ALL specs ===
    if (Unit* pVictim = me->GetVictim())
    {
        if (m_spells.shaman.pEarthShock &&
            pVictim->IsNonMeleeSpellCasted(false, false, true) &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pEarthShock))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pEarthShock) == SPELL_CAST_OK)
                return;
        }
    }

    // === MANA MANAGEMENT — all specs ===
    if (m_spells.shaman.pManaTideTotem &&
       (me->GetPowerPercent(POWER_MANA) < 40.0f) &&
        CanTryToCastSpell(me, m_spells.shaman.pManaTideTotem))
    {
        if (DoCastSpell(me, m_spells.shaman.pManaTideTotem) == SPELL_CAST_OK)
            return;
    }

    // === TOTEM MAINTENANCE ===
    if (SummonShamanTotemsContext())
        return;

    // === RESTORATION SHAMAN (Healer) ===
    if (GetRole() == ROLE_HEALER)
    {
        // Emergency: Nature's Swiftness + max rank heal when tank is critical
        if (m_spells.shaman.pNaturesSwiftness && m_spells.shaman.pHealingWave)
        {
            if (Player* pTank = FindPartyTank())
            {
                if (pTank->GetHealthPercent() < 35.0f &&
                    CanTryToCastSpell(me, m_spells.shaman.pNaturesSwiftness))
                {
                    if (DoCastSpell(me, m_spells.shaman.pNaturesSwiftness) == SPELL_CAST_OK)
                    {
                        // Follow up with Healing Wave on tank (instant due to NS)
                        if (CanTryToCastSpell(pTank, m_spells.shaman.pHealingWave))
                            DoCastSpell(pTank, m_spells.shaman.pHealingWave);
                        return;
                    }
                }
            }
        }

        // Chain Heal when multiple party members are injured
        if (m_spells.shaman.pChainHeal)
        {
            if (Unit* pTarget = SelectHealTarget(70.0f, 70.0f))
            {
                if (CanTryToCastSpell(pTarget, m_spells.shaman.pChainHeal))
                {
                    if (DoCastSpell(pTarget, m_spells.shaman.pChainHeal) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Lesser Healing Wave for quick top-ups or when moving
        if (m_spells.shaman.pLesserHealingWave)
        {
            if (Unit* pTarget = SelectHealTarget(50.0f, 70.0f))
            {
                if (CanTryToCastSpell(pTarget, m_spells.shaman.pLesserHealingWave))
                {
                    if (DoCastSpell(pTarget, m_spells.shaman.pLesserHealingWave) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // Standard healing fallback
        if (FindAndHealInjuredAlly(50.0f, 90.0f))
            return;

        if (FindAndPreHealTarget())
            return;

        // If healer has aggro, try Stoneclaw/Earthbind to peel
        if (!me->GetAttackers().empty())
        {
            if (m_spells.shaman.pStoneclawTotem &&
                !me->GetTotem(TOTEM_SLOT_EARTH) &&
                CanTryToCastSpell(me, m_spells.shaman.pStoneclawTotem))
            {
                if (DoCastSpell(me, m_spells.shaman.pStoneclawTotem) == SPELL_CAST_OK)
                    return;
            }

            // Frost Shock to slow melee attackers chasing healer
            if (m_spells.shaman.pFrostShock)
            {
                Unit* pAttacker = *me->GetAttackers().begin();
                if (pAttacker->CanReachWithMeleeAutoAttack(me) &&
                    CanTryToCastSpell(pAttacker, m_spells.shaman.pFrostShock))
                {
                    if (DoCastSpell(pAttacker, m_spells.shaman.pFrostShock) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        return;
    }

    // === ENHANCEMENT SHAMAN (Melee DPS) ===
    if (GetRole() == ROLE_MELEE_DPS)
    {
        if (Unit* pVictim = me->GetVictim())
        {
            // Threat check — Enhancement has no threat dump
            bool const highThreat = ShouldThrottleDPS(pVictim);

            // Stormstrike (highest priority melee ability)
            if (!highThreat && m_spells.shaman.pStormstrike &&
                CanTryToCastSpell(pVictim, m_spells.shaman.pStormstrike))
            {
                if (DoCastSpell(pVictim, m_spells.shaman.pStormstrike) == SPELL_CAST_OK)
                    return;
            }

            // Earth Shock for DPS (only when threat is safe — it's the highest threat shock)
            if (!highThreat && m_spells.shaman.pEarthShock &&
                !pVictim->IsNonMeleeSpellCasted(false, false, true) &&
                CanTryToCastSpell(pVictim, m_spells.shaman.pEarthShock))
            {
                if (DoCastSpell(pVictim, m_spells.shaman.pEarthShock) == SPELL_CAST_OK)
                    return;
            }

            // Flame Shock (DoT for longer fights, lower threat than Earth Shock)
            if (m_spells.shaman.pFlameShock &&
                CanTryToCastSpell(pVictim, m_spells.shaman.pFlameShock))
            {
                if (DoCastSpell(pVictim, m_spells.shaman.pFlameShock) == SPELL_CAST_OK)
                    return;
            }

            // Frost Shock on fleeing targets
            if (m_spells.shaman.pFrostShock &&
                pVictim->IsMoving() &&
                !pVictim->HasAuraType(SPELL_AURA_MOD_DECREASE_SPEED) &&
                CanTryToCastSpell(pVictim, m_spells.shaman.pFrostShock))
            {
                if (DoCastSpell(pVictim, m_spells.shaman.pFrostShock) == SPELL_CAST_OK)
                    return;
            }

            // Purge hostile buffs
            if (m_spells.shaman.pPurge &&
                IsValidDispelTarget(pVictim, m_spells.shaman.pPurge) &&
                CanTryToCastSpell(pVictim, m_spells.shaman.pPurge))
            {
                if (DoCastSpell(pVictim, m_spells.shaman.pPurge) == SPELL_CAST_OK)
                    return;
            }

            // Chase if not in melee range
            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
                && !me->CanReachWithMeleeAutoAttack(pVictim))
            {
                me->GetMotionMaster()->MoveChase(pVictim);
            }

            // Off-heal self if low
            if (me->GetHealthPercent() < 30.0f)
                HealInjuredTarget(me);
        }

        return;
    }

    // === ELEMENTAL SHAMAN (Ranged DPS) ===
    if (Unit* pVictim = me->GetVictim())
    {
        bool const highThreat = ShouldThrottleDPS(pVictim);

        // Range positioning
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
            && me->GetDistance(pVictim) > 30.0f)
        {
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        }

        // Elemental Mastery (instant next spell + guaranteed crit)
        if (!highThreat && m_spells.shaman.pElementalMastery &&
            CanTryToCastSpell(me, m_spells.shaman.pElementalMastery))
        {
            if (DoCastSpell(me, m_spells.shaman.pElementalMastery) == SPELL_CAST_OK)
                return;
        }

        // Chain Lightning (AoE, but skip if threat is high)
        if (!highThreat && m_spells.shaman.pChainLightning &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pChainLightning))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pChainLightning) == SPELL_CAST_OK)
                return;
        }

        // Purge hostile buffs
        if (m_spells.shaman.pPurge &&
            IsValidDispelTarget(pVictim, m_spells.shaman.pPurge) &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pPurge))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pPurge) == SPELL_CAST_OK)
                return;
        }

        // Flame Shock (DoT, apply before Lightning Bolt spam)
        if (m_spells.shaman.pFlameShock &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pFlameShock))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pFlameShock) == SPELL_CAST_OK)
                return;
        }

        // Lightning Bolt (primary nuke — skip if threat is too high)
        if (!highThreat && m_spells.shaman.pLightningBolt &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pLightningBolt))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pLightningBolt) == SPELL_CAST_OK)
                return;
        }

        // Frost Shock on fleeing targets
        if (m_spells.shaman.pFrostShock &&
            pVictim->IsMoving() &&
            CanTryToCastSpell(pVictim, m_spells.shaman.pFrostShock))
        {
            if (DoCastSpell(pVictim, m_spells.shaman.pFrostShock) == SPELL_CAST_OK)
                return;
        }
    }

    // Self-heal if low (all DPS specs)
    if (me->GetHealthPercent() < 20.0f)
        HealInjuredTarget(me);
}
