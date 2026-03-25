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

enum PartyBotSpells_Warlock
{
    PB_SPELL_SHOOT_WAND_W = 5019,
};

void PartyBotAI::UpdateOutOfCombatAI_Warlock()
{
    // === BUFF: Detect Invisibility on party ===
    if (m_spells.warlock.pDetectInvisibility)
    {
        if (Player* pTarget = SelectBuffTarget(m_spells.warlock.pDetectInvisibility))
        {
            if (CanTryToCastSpell(pTarget, m_spells.warlock.pDetectInvisibility))
            {
                if (DoCastSpell(pTarget, m_spells.warlock.pDetectInvisibility) == SPELL_CAST_OK)
                {
                    m_isBuffing = true;
                    me->ClearTarget();
                    return;
                }
            }
        }
    }

    // === BUFF: Demon Armor self-buff ===
    if (m_spells.warlock.pDemonArmor &&
        CanTryToCastSpell(me, m_spells.warlock.pDemonArmor))
    {
        if (DoCastSpell(me, m_spells.warlock.pDemonArmor) == SPELL_CAST_OK)
        {
            m_isBuffing = true;
            me->ClearTarget();
            return;
        }
    }

    if (m_isBuffing &&
       (!m_spells.warlock.pDetectInvisibility ||
        !me->HasGCD(m_spells.warlock.pDetectInvisibility)))
    {
        m_isBuffing = false;
    }

    if (Unit* pVictim = me->GetVictim())
    {
        // On entering combat: send pet to attack target
        if (Pet* pPet = me->GetPet())
        {
            if (!pPet->GetVictim())
            {
                pPet->GetCharmInfo()->SetIsCommandAttack(true);
                pPet->AI()->AttackStart(pVictim);
            }
        }

        UpdateInCombatAI_Warlock();
    }
    else
        SummonPetIfNeeded();
}

void PartyBotAI::UpdateInCombatAI_Warlock()
{
    if (Unit* pVictim = me->GetVictim())
    {
        bool const highThreat = ShouldThrottleDPS(pVictim);

        // === RANGE POSITIONING: stay at 25 yards ===
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE
            && me->GetDistance(pVictim) > 30.0f)
        {
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        }

        // === EMERGENCY: Death Coil when being attacked or target casting ===
        // Prioritize self-defense with Death Coil before anything else
        if (m_spells.warlock.pDeathCoil &&
           !me->GetAttackers().empty() &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pDeathCoil))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pDeathCoil) == SPELL_CAST_OK)
                return;
        }

        // === EMERGENCY: Howl of Terror when 2+ melee attackers ===
        if (m_spells.warlock.pHowlofTerror &&
            GetAttackersInRangeCount(10.0f) >= 2 &&
            CanTryToCastSpell(me, m_spells.warlock.pHowlofTerror))
        {
            if (DoCastSpell(me, m_spells.warlock.pHowlofTerror) == SPELL_CAST_OK)
                return;
        }

        // === SELF-DEFENSE: Fear when primary target is hitting you ===
        // Only as last-resort when no other peel is available
        if (m_spells.warlock.pFear &&
            pVictim->GetVictim() == me &&
            me->GetAttackers().size() == 1 &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pFear))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pFear) == SPELL_CAST_OK)
                return;
        }

        // === CC: Banish a second attacker to reduce incoming damage ===
        if (m_spells.warlock.pBanish &&
            me->GetAttackers().size() > 1)
        {
            if (Unit* pAttacker = SelectAttackerDifferentFrom(pVictim))
            {
                if (CanUseCrowdControl(m_spells.warlock.pBanish, pAttacker) &&
                    CanTryToCastSpell(pAttacker, m_spells.warlock.pBanish))
                {
                    if (DoCastSpell(pAttacker, m_spells.warlock.pBanish) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // === EXECUTE: Shadowburn at <10% HP (instant, shard on kill) ===
        if (m_spells.warlock.pShadowburn &&
           (pVictim->GetHealthPercent() < 10.0f) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pShadowburn))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pShadowburn) == SPELL_CAST_OK)
                return;
        }

        // === AoE: Rain of Fire when 3+ enemies grouped ===
        if (m_spells.warlock.pRainOfFire &&
           (me->GetEnemyCountInRadiusAround(pVictim, 10.0f) > 2) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pRainOfFire))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pRainOfFire) == SPELL_CAST_OK)
                return;
        }

        // === DEMONIC SACRIFICE (keep existing logic) ===
        if (m_spells.warlock.pDemonicSacrifice)
        {
            if (Pet* pPet = me->GetPet())
            {
                if (pPet->IsAlive() &&
                    CanTryToCastSpell(pPet, m_spells.warlock.pDemonicSacrifice))
                {
                    if (DoCastSpell(pPet, m_spells.warlock.pDemonicSacrifice) == SPELL_CAST_OK)
                        return;
                }
            }
        }

        // === INTELLIGENT CURSE PRIORITY ===
        // Curse of Tongues on caster mobs (cripples their cast speed)
        if (m_spells.warlock.pCurseofTongues &&
           (pVictim->GetPowerType() == POWER_MANA) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseofTongues))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseofTongues) == SPELL_CAST_OK)
                return;
        }

        // Curse of the Elements (best for shadow/fire group DPS, highest priority debuff)
        if (m_spells.warlock.pCurseoftheElements &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseoftheElements))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseoftheElements) == SPELL_CAST_OK)
                return;
        }

        // Curse of Shadow (second best for shadow damage amplification)
        if (m_spells.warlock.pCurseofShadow &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseofShadow))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseofShadow) == SPELL_CAST_OK)
                return;
        }

        // Curse of Doom on bosses with very high HP (long fights, big damage tick)
        if (m_spells.warlock.pCurseofDoom &&
           (pVictim->GetHealth() > (uint32)(me->GetMaxHealth() * 5)) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseofDoom))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseofDoom) == SPELL_CAST_OK)
                return;
        }

        // Curse of Recklessness (increases attack power of target, removes fear immunity —
        // use to prevent fleeing and as filler when other curses are unavailable)
        if (m_spells.warlock.pCurseofRecklessness &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseofRecklessness))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseofRecklessness) == SPELL_CAST_OK)
                return;
        }

        // Curse of Agony (filler DPS curse — apply if nothing better is available)
        if (m_spells.warlock.pCurseofAgony &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCurseofAgony))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCurseofAgony) == SPELL_CAST_OK)
                return;
        }

        // === DoT PRIORITY: Corruption first (instant cast), then Immolate ===

        // Corruption (instant, apply first — no cast time risk)
        if (m_spells.warlock.pCorruption &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pCorruption))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pCorruption) == SPELL_CAST_OK)
                return;
        }

        // Immolate (fire DoT + upfront burst — apply before Conflagrate)
        if (m_spells.warlock.pImmolate &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pImmolate))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pImmolate) == SPELL_CAST_OK)
                return;
        }

        // Conflagrate (consumes Immolate for burst — only fires if Immolate is on target,
        // CanTryToCastSpell handles the Immolate requirement check)
        if (m_spells.warlock.pConflagrate &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pConflagrate))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pConflagrate) == SPELL_CAST_OK)
                return;
        }

        // Siphon Life (self-heal DoT — apply on longer-lived targets when not full HP)
        if (m_spells.warlock.pSiphonLife &&
           (me->GetHealthPercent() < 80.0f) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pSiphonLife))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pSiphonLife) == SPELL_CAST_OK)
                return;
        }

        // === DRAIN LIFE: self-heal filler when HP <40% ===
        if (m_spells.warlock.pDrainLife &&
           (me->GetHealthPercent() < 40.0f) &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pDrainLife))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pDrainLife) == SPELL_CAST_OK)
                return;
        }

        // === SHADOW BOLT: primary nuke — threat-aware ===
        // (Searing Pain is intentionally excluded: generates 2x threat)
        if (m_spells.warlock.pShadowBolt &&
            !highThreat &&
            CanTryToCastSpell(pVictim, m_spells.warlock.pShadowBolt))
        {
            if (DoCastSpell(pVictim, m_spells.warlock.pShadowBolt) == SPELL_CAST_OK)
                return;
        }

        // === LIFE TAP: mana restore with safety thresholds ===
        // Only tap when mana is very low AND health is safe — never when health <40%
        if (m_spells.warlock.pLifeTap &&
           (me->GetPowerPercent(POWER_MANA) < 20.0f) &&
           (me->GetHealthPercent() > 50.0f) &&
           (GetAttackersInRangeCount(10.0f) == 0) &&
            CanTryToCastSpell(me, m_spells.warlock.pLifeTap))
        {
            if (DoCastSpell(me, m_spells.warlock.pLifeTap) == SPELL_CAST_OK)
                return;
        }

        // === WAND: use when OOM and Life Tap is unsafe (<5% mana) ===
        if (me->HasSpell(PB_SPELL_SHOOT_WAND_W) &&
           !me->IsMoving() &&
           (me->GetPowerPercent(POWER_MANA) < 5.0f) &&
           !me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
            me->CastSpell(pVictim, PB_SPELL_SHOOT_WAND_W, false);
    }
}
