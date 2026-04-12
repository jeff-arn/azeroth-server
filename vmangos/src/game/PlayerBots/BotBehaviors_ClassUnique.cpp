#include "BotBehaviors_ClassUnique.h"
#include "BotSituationContext.h"
#include "BotSpellKit.h"
#include "CombatBotBaseAI.h"
#include "PartyBotAI.h"
#include "Player.h"
#include "Pet.h"
#include "Group.h"
#include "CreatureAI.h"

// Warlock pet summon spell IDs
enum WarlockPetSpellIds
{
    PB_SPELL_SUMMON_IMP         = 688,
    PB_SPELL_SUMMON_VOIDWALKER  = 697,
    PB_SPELL_SUMMON_FELHUNTER   = 691,
    PB_SPELL_SUMMON_SUCCUBUS    = 712,
};

// ============================================================
// WARRIOR
// ============================================================

float BehaviorWarriorStanceDance::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_WARRIOR)
        return 0.0f;

    if (ctx.role == ROLE_TANK)
    {
        // Tank should be in Defensive Stance
        if (m_ai->m_spells.warrior.pDefensiveStance &&
            m_ai->me->GetShapeshiftForm() != FORM_DEFENSIVESTANCE)
            return 88.0f;
    }
    else
    {
        // DPS should be in Battle or Berserker Stance
        if (m_ai->me->GetShapeshiftForm() == FORM_DEFENSIVESTANCE)
            return 88.0f;
    }
    return 0.0f;
}

bool BehaviorWarriorStanceDance::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (ctx.role == ROLE_TANK && m_ai->m_spells.warrior.pDefensiveStance)
    {
        if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.warrior.pDefensiveStance))
            return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.warrior.pDefensiveStance) == SPELL_CAST_OK;
    }
    else if (m_ai->m_spells.warrior.pBerserkerStance)
    {
        if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.warrior.pBerserkerStance))
            return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.warrior.pBerserkerStance) == SPELL_CAST_OK;
    }
    else if (m_ai->m_spells.warrior.pBattleStance)
    {
        if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.warrior.pBattleStance))
            return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.warrior.pBattleStance) == SPELL_CAST_OK;
    }
    return false;
}

float BehaviorWarriorRageDump::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_WARRIOR)
        return 0.0f;
    if (ctx.ragePct < 50.0f || !ctx.currentTarget)
        return 0.0f;
    return 45.0f;
}

bool BehaviorWarriorRageDump::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Cleave if multiple targets, else Heroic Strike
    if (m_ai->GetAttackersInRangeCount(10.0f) > 1 && m_ai->m_spells.warrior.pCleave)
    {
        if (m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warrior.pCleave))
            return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warrior.pCleave) == SPELL_CAST_OK;
    }
    if (m_ai->m_spells.warrior.pHeroicStrike)
    {
        if (m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warrior.pHeroicStrike))
            return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warrior.pHeroicStrike) == SPELL_CAST_OK;
    }
    return false;
}

// ============================================================
// ROGUE
// ============================================================

float BehaviorRogueStealthOpener::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_ROGUE)
        return 0.0f;
    if (!m_ai->me->HasAuraType(SPELL_AURA_MOD_STEALTH))
        return 0.0f;

    // Use ctx.currentTarget (from GetVictim) or fall back to visual target
    // (set by AttackStart stealth path which can't call Attack())
    Unit* target = ctx.currentTarget;
    if (!target)
    {
        ObjectGuid guid = m_ai->me->GetTargetGuid();
        if (guid.IsUnit())
            target = m_ai->me->GetMap()->GetUnit(guid);
    }
    if (!target)
        return 0.0f;

    return 92.0f;
}

bool BehaviorRogueStealthOpener::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Resolve target — may not be in ctx.currentTarget if stealthed
    Unit* target = ctx.currentTarget;
    if (!target)
    {
        ObjectGuid guid = m_ai->me->GetTargetGuid();
        if (guid.IsUnit())
            target = m_ai->me->GetMap()->GetUnit(guid);
    }
    if (!target)
        return false;

    // Must be behind the target for openers (Garrote, Ambush)
    if (!m_ai->me->IsBehindTarget(target, false))
    {
        // Calculate a point 2yd behind the target (opposite its facing)
        float behindAngle = target->GetOrientation() + M_PI_F; // opposite of target's facing
        float x = target->GetPositionX() + 2.0f * cos(behindAngle);
        float y = target->GetPositionY() + 2.0f * sin(behindAngle);
        float z = target->GetPositionZ();
        m_ai->me->GetMotionMaster()->MovePoint(0, x, y, z);
        return true;
    }

    // In position — face the target before opening
    m_ai->me->SetFacingToObject(target);

    // Must also be in melee range
    if (!m_ai->me->CanReachWithMeleeAutoAttack(target))
    {
        m_ai->me->GetMotionMaster()->MoveChase(target, 1.0f, 3.0f);
        return true;
    }

    // Premeditation first
    if (m_ai->m_spells.rogue.pPremeditation &&
        m_ai->CanTryToCastSpell(target, m_ai->m_spells.rogue.pPremeditation))
    {
        m_ai->DoCastSpell(target, m_ai->m_spells.rogue.pPremeditation);
    }

    // Opener priority: Cheap Shot (stun) > Garrote (bleed) > Ambush (burst)
    if (m_ai->m_spells.rogue.pCheapShot && m_ai->CanTryToCastSpell(target, m_ai->m_spells.rogue.pCheapShot))
        return m_ai->DoCastSpell(target, m_ai->m_spells.rogue.pCheapShot) == SPELL_CAST_OK;
    if (m_ai->m_spells.rogue.pGarrote && m_ai->CanTryToCastSpell(target, m_ai->m_spells.rogue.pGarrote))
        return m_ai->DoCastSpell(target, m_ai->m_spells.rogue.pGarrote) == SPELL_CAST_OK;
    if (m_ai->m_spells.rogue.pAmbush && m_ai->CanTryToCastSpell(target, m_ai->m_spells.rogue.pAmbush))
        return m_ai->DoCastSpell(target, m_ai->m_spells.rogue.pAmbush) == SPELL_CAST_OK;

    return false;
}

float BehaviorRogueComboFinisher::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_ROGUE || ctx.comboPoints < 5)
        return 0.0f;
    return 70.0f;
}

bool BehaviorRogueComboFinisher::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // SnD > Kidney Shot (if target casting) > Eviscerate
    if (m_ai->m_spells.rogue.pSliceAndDice &&
        !m_ai->me->HasAura(m_ai->m_spells.rogue.pSliceAndDice->Id) &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.rogue.pSliceAndDice))
    {
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.rogue.pSliceAndDice) == SPELL_CAST_OK;
    }
    if (ctx.targetIsCasting && m_ai->m_spells.rogue.pKidneyShot &&
        m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.rogue.pKidneyShot))
    {
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.rogue.pKidneyShot) == SPELL_CAST_OK;
    }
    if (m_ai->m_spells.rogue.pEviscerate && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.rogue.pEviscerate))
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.rogue.pEviscerate) == SPELL_CAST_OK;

    return false;
}

float BehaviorRogueStealth::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_ROGUE)
        return 0.0f;
    if (!m_ai->m_spells.rogue.pStealth)
        return 0.0f;
    // Already stealthed
    if (m_ai->me->HasAuraType(SPELL_AURA_MOD_STEALTH))
        return 0.0f;
    // Only stealth when the group has a target (about to pull)
    if (!m_ai->ShouldEnterStealth())
        return 0.0f;

    return 60.0f;
}

bool BehaviorRogueStealth::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    return m_ai->EnterStealthIfNeeded(m_ai->m_spells.rogue.pStealth);
}

float BehaviorRoguePoisonWeapons::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_ROGUE)
        return 0.0f;
    // Only score if we actually have poison spells
    if (!m_ai->m_spells.rogue.pMainHandPoison && !m_ai->m_spells.rogue.pOffHandPoison)
        return 0.0f;
    return 50.0f;
}

bool BehaviorRoguePoisonWeapons::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (m_ai->m_spells.rogue.pMainHandPoison)
    {
        if (m_ai->CastWeaponBuff(m_ai->m_spells.rogue.pMainHandPoison, EQUIPMENT_SLOT_MAINHAND) == SPELL_CAST_OK)
            return true;
    }
    if (m_ai->m_spells.rogue.pOffHandPoison)
    {
        if (m_ai->CastWeaponBuff(m_ai->m_spells.rogue.pOffHandPoison, EQUIPMENT_SLOT_OFFHAND) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// HUNTER
// ============================================================

float BehaviorHunterPetManage::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_HUNTER)
        return 0.0f;

    // Pet not summoned or dead
    if (!m_ai->me->GetPet())
        return 55.0f;

    // Pet alive but not attacking current target
    if (Pet* pPet = m_ai->me->GetPet())
    {
        if (ctx.currentTarget && !pPet->GetVictim())
            return 55.0f;
    }
    return 0.0f;
}

bool BehaviorHunterPetManage::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (!m_ai->me->GetPet())
    {
        m_ai->SummonPetIfNeeded();
        return true;
    }

    if (Pet* pPet = m_ai->me->GetPet())
    {
        if (ctx.currentTarget && !pPet->GetVictim())
        {
            pPet->GetCharmInfo()->SetIsCommandAttack(true);
            pPet->AI()->AttackStart(ctx.currentTarget);
            return true;
        }
    }
    return false;
}

float BehaviorHunterAspectSwap::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_HUNTER)
        return 0.0f;

    // Check if we need to swap aspects
    bool inMelee = ctx.distanceToTarget < 8.0f;
    if (inMelee && m_ai->m_spells.hunter.pAspectOfTheMonkey &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheMonkey))
        return 45.0f;
    if (!inMelee && m_ai->m_spells.hunter.pAspectOfTheHawk &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheHawk))
        return 45.0f;

    return 0.0f;
}

bool BehaviorHunterAspectSwap::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    bool inMelee = ctx.distanceToTarget < 8.0f;
    if (inMelee && m_ai->m_spells.hunter.pAspectOfTheMonkey)
    {
        if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheMonkey))
            return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheMonkey) == SPELL_CAST_OK;
    }
    else if (m_ai->m_spells.hunter.pAspectOfTheHawk)
    {
        if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheHawk))
            return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.hunter.pAspectOfTheHawk) == SPELL_CAST_OK;
    }
    return false;
}

float BehaviorHunterDeadZone::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_HUNTER)
        return 0.0f;
    if (!ctx.currentTarget)
        return 0.0f;

    // In dead zone: < 8 yards or being melee'd
    if (ctx.distanceToTarget < 8.0f || (ctx.currentTarget->CanReachWithMeleeAutoAttack(m_ai->me)))
        return 88.0f;

    return 0.0f;
}

bool BehaviorHunterDeadZone::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Wing Clip to slow (instant, enables escape)
    if (m_ai->m_spells.hunter.pWingClip && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pWingClip))
        m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pWingClip);

    // Mongoose Bite (melee filler while retreating)
    if (m_ai->m_spells.hunter.pMongooseBite && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pMongooseBite))
        m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pMongooseBite);

    // Raptor Strike (melee filler)
    if (m_ai->m_spells.hunter.pRaptorStrike && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pRaptorStrike))
        m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.hunter.pRaptorStrike);

    // Run away to re-establish range
    if (!m_ai->me->HasUnitState(UNIT_STATE_ROOT) &&
        m_ai->me->GetMotionMaster()->GetCurrentMovementGeneratorType() != DISTANCING_MOTION_TYPE)
    {
        if (!m_ai->me->IsStopped())
            m_ai->me->StopMoving();
        m_ai->me->GetMotionMaster()->Clear();
        m_ai->RunAwayFromTarget(ctx.currentTarget);
    }
    return true;
}

// ============================================================
// PALADIN
// ============================================================

float BehaviorPaladinBlessing::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_PALADIN)
        return 0.0f;

    // OOC buffing — check if any party member needs a blessing
    if (m_ai->m_spells.paladin.pBlessingBuff)
    {
        if (Player* pTarget = m_ai->SelectBuffTarget(m_ai->m_spells.paladin.pBlessingBuff))
            return 50.0f;
    }
    return 0.0f;
}

bool BehaviorPaladinBlessing::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (m_ai->m_spells.paladin.pBlessingBuff)
    {
        if (Player* pTarget = m_ai->SelectBuffTarget(m_ai->m_spells.paladin.pBlessingBuff))
        {
            if (m_ai->CanTryToCastSpell(pTarget, m_ai->m_spells.paladin.pBlessingBuff))
                return m_ai->DoCastSpell(pTarget, m_ai->m_spells.paladin.pBlessingBuff) == SPELL_CAST_OK;
        }
    }
    return false;
}

float BehaviorPaladinSealJudge::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_PALADIN || !ctx.currentTarget)
        return 0.0f;

    // Seal + Judgement maintenance
    if (m_ai->m_spells.paladin.pSeal && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.paladin.pSeal))
        return 55.0f;
    if (m_ai->m_spells.paladin.pJudgement && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.paladin.pJudgement))
        return 55.0f;

    return 0.0f;
}

bool BehaviorPaladinSealJudge::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Apply seal if missing
    if (m_ai->m_spells.paladin.pSeal && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.paladin.pSeal))
    {
        if (m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.paladin.pSeal) == SPELL_CAST_OK)
            return true;
    }
    // Judge the seal
    if (m_ai->m_spells.paladin.pJudgement && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.paladin.pJudgement))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.paladin.pJudgement) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// SHAMAN
// ============================================================

float BehaviorShamanTotemManage::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_SHAMAN)
        return 0.0f;
    return 60.0f;
}

bool BehaviorShamanTotemManage::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Delegate to existing totem management
    return m_ai->SummonShamanTotems();
}

// ============================================================
// PRIEST
// ============================================================

float BehaviorPriestShadowformToggle::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_PRIEST)
        return 0.0f;
    if (ctx.role == ROLE_HEALER)
        return 0.0f;
    if (!m_ai->m_spells.priest.pShadowform)
        return 0.0f;

    // Enter Shadowform if not already in it
    if (m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.priest.pShadowform))
        return 65.0f;

    return 0.0f;
}

bool BehaviorPriestShadowformToggle::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (m_ai->m_spells.priest.pShadowform && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.priest.pShadowform))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.priest.pShadowform) == SPELL_CAST_OK;
    return false;
}

float BehaviorPriestShieldLogic::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_PRIEST || ctx.role != ROLE_HEALER)
        return 0.0f;
    if (!m_ai->m_spells.priest.pPowerWordShield)
        return 0.0f;

    // Shield an ally under attack
    if (Player* pTarget = m_ai->SelectShieldTarget())
        return 72.0f;

    return 0.0f;
}

bool BehaviorPriestShieldLogic::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pShield = m_ai->m_spells.priest.pPowerWordShield;
    if (!pShield)
        return false;

    if (Player* pTarget = m_ai->SelectShieldTarget())
    {
        // Warrior tank exception: don't shield unless < 15% HP (blocks rage)
        if (pTarget->GetClass() == CLASS_WARRIOR)
        {
            Player* pTank = m_ai->FindPartyTank();
            if (pTank == pTarget && pTarget->GetHealthPercent() > 15.0f)
                return false;
        }

        if (m_ai->CanTryToCastSpell(pTarget, pShield))
            return m_ai->DoCastSpell(pTarget, pShield) == SPELL_CAST_OK;
    }
    return false;
}

// ============================================================
// WARLOCK
// ============================================================

float BehaviorWarlockPetSummon::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_WARLOCK)
        return 0.0f;

    // No pet at all — need to summon
    if (!m_ai->me->GetPetGuid() && !m_ai->me->GetCharmGuid())
        return 55.0f;

    // Pet exists but hasn't been taught grimoire spells yet
    if (Pet* pPet = m_ai->me->GetPet())
    {
        if (pPet->GetGUIDLow() != m_ai->m_lastTaughtPetGuid)
            return 55.0f;
    }

    return 0.0f;
}

bool BehaviorWarlockPetSummon::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // If pet exists, teach grimoire spells
    if (Pet* pPet = m_ai->me->GetPet())
    {
        if (pPet->GetGUIDLow() != m_ai->m_lastTaughtPetGuid)
        {
            m_ai->TeachWarlockPetSpells();
            m_ai->m_lastTaughtPetGuid = pPet->GetGUIDLow();
            return true;
        }
        return false;
    }

    // No pet — summon one. Skip Voidwalker when a tank is present.
    bool hasTank = (m_ai->FindPartyTank() != nullptr);
    std::vector<uint32> vSummons;

    if (m_ai->me->HasSpell(PB_SPELL_SUMMON_IMP))
        vSummons.push_back(PB_SPELL_SUMMON_IMP);
    if (m_ai->me->HasSpell(PB_SPELL_SUMMON_VOIDWALKER) && !hasTank)
        vSummons.push_back(PB_SPELL_SUMMON_VOIDWALKER);
    if (m_ai->me->HasSpell(PB_SPELL_SUMMON_FELHUNTER))
        vSummons.push_back(PB_SPELL_SUMMON_FELHUNTER);
    if (m_ai->me->HasSpell(PB_SPELL_SUMMON_SUCCUBUS))
        vSummons.push_back(PB_SPELL_SUMMON_SUCCUBUS);

    if (!vSummons.empty())
    {
        uint32 spell = vSummons[urand(0, vSummons.size() - 1)];
        m_ai->me->CastSpell(m_ai->me, spell, true);
        return true;
    }
    return false;
}

float BehaviorWarlockCurseManage::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_WARLOCK || !ctx.currentTarget)
        return 0.0f;

    // Check if any curse is already on target
    SpellEntry const* curseSpells[] = {
        m_ai->m_spells.warlock.pCurseofTongues, m_ai->m_spells.warlock.pCurseoftheElements,
        m_ai->m_spells.warlock.pCurseofShadow, m_ai->m_spells.warlock.pCurseofDoom,
        m_ai->m_spells.warlock.pCurseofRecklessness, m_ai->m_spells.warlock.pCurseofAgony,
    };
    for (auto const* pCurse : curseSpells)
    {
        if (pCurse && ctx.currentTarget->HasAura(pCurse->Id))
            return 0.0f; // Already have a curse
    }

    return 52.0f;
}

bool BehaviorWarlockCurseManage::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Curse priority: Tongues (casters) > Elements > Shadow > Doom (bosses) > Agony
    if (m_ai->m_spells.warlock.pCurseofTongues &&
        ctx.currentTarget->GetPowerType() == POWER_MANA &&
        m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofTongues))
    {
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofTongues) == SPELL_CAST_OK;
    }
    if (m_ai->m_spells.warlock.pCurseoftheElements && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseoftheElements))
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseoftheElements) == SPELL_CAST_OK;
    if (m_ai->m_spells.warlock.pCurseofShadow && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofShadow))
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofShadow) == SPELL_CAST_OK;
    if (m_ai->m_spells.warlock.pCurseofAgony && m_ai->CanTryToCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofAgony))
        return m_ai->DoCastSpell(ctx.currentTarget, m_ai->m_spells.warlock.pCurseofAgony) == SPELL_CAST_OK;

    return false;
}

float BehaviorWarlockLifeTap::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_WARLOCK)
        return 0.0f;
    if (!m_ai->m_spells.warlock.pLifeTap)
        return 0.0f;
    if (ctx.manaPct > 30.0f || ctx.healthPct < 60.0f)
        return 0.0f;
    if (ctx.meleeAttackerCount > 0)
        return 0.0f;
    return 48.0f;
}

bool BehaviorWarlockLifeTap::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (m_ai->m_spells.warlock.pLifeTap && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.warlock.pLifeTap))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.warlock.pLifeTap) == SPELL_CAST_OK;
    return false;
}

// ============================================================
// DRUID
// ============================================================

float BehaviorDruidFormShift::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_DRUID)
        return 0.0f;

    // Healer should be in caster form
    if (ctx.role == ROLE_HEALER && ctx.currentForm != FORM_NONE)
        return 85.0f;

    // Tank should be in bear
    if (ctx.role == ROLE_TANK && ctx.currentForm != FORM_BEAR && ctx.currentForm != FORM_DIREBEAR)
    {
        if (m_ai->m_spells.druid.pBearForm)
            return 80.0f;
    }

    // Melee DPS should be in cat (or bear if no cat form)
    if (ctx.role == ROLE_MELEE_DPS && ctx.currentForm != FORM_CAT)
    {
        if (m_ai->m_spells.druid.pCatForm)
            return 75.0f;
        if (m_ai->m_spells.druid.pBearForm && ctx.currentForm != FORM_BEAR && ctx.currentForm != FORM_DIREBEAR)
            return 75.0f;
    }

    // Range DPS could be moonkin
    if (ctx.role == ROLE_RANGE_DPS && ctx.currentForm != FORM_MOONKIN && m_ai->m_spells.druid.pMoonkinForm)
        return 70.0f;

    return 0.0f;
}

bool BehaviorDruidFormShift::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Remove current form first if needed
    if (ctx.role == ROLE_HEALER && ctx.currentForm != FORM_NONE)
    {
        m_ai->me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);
        return true;
    }

    if (ctx.role == ROLE_TANK && m_ai->m_spells.druid.pBearForm &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.druid.pBearForm))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.druid.pBearForm) == SPELL_CAST_OK;

    if (ctx.role == ROLE_MELEE_DPS && m_ai->m_spells.druid.pCatForm &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.druid.pCatForm))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.druid.pCatForm) == SPELL_CAST_OK;

    if (ctx.role == ROLE_RANGE_DPS && m_ai->m_spells.druid.pMoonkinForm &&
        m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.druid.pMoonkinForm))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.druid.pMoonkinForm) == SPELL_CAST_OK;

    return false;
}

float BehaviorDruidRebirth::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_DRUID || !m_ai->m_spells.druid.pRebirth)
        return 0.0f;
    if (ctx.deadPartyMembers == 0)
        return 0.0f;

    // Check for a valid resurrection target
    if (Player* pTarget = m_ai->SelectResurrectionTarget())
        return 93.0f;

    return 0.0f;
}

bool BehaviorDruidRebirth::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Must shift to caster form first
    if (ctx.currentForm != FORM_NONE && m_ai->me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
    {
        m_ai->me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);
        return true;
    }

    if (Player* pTarget = m_ai->SelectResurrectionTarget())
    {
        if (m_ai->CanTryToCastSpell(pTarget, m_ai->m_spells.druid.pRebirth))
            return m_ai->DoCastSpell(pTarget, m_ai->m_spells.druid.pRebirth) == SPELL_CAST_OK;
    }
    return false;
}

float BehaviorDruidInnervate::Score(SituationContext const& ctx) const
{
    if (ctx.playerClass != CLASS_DRUID || !m_ai->m_spells.druid.pInnervate)
        return 0.0f;

    // Self: very low mana
    if (ctx.manaPct < 20.0f)
        return 72.0f;

    // Check party healers
    if (Group* pGroup = m_ai->me->GetGroup())
    {
        for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                if (pMember == m_ai->me || !pMember->IsAlive())
                    continue;
                uint8 cls = pMember->GetClass();
                if ((cls == CLASS_PRIEST || cls == CLASS_DRUID || cls == CLASS_SHAMAN || cls == CLASS_PALADIN) &&
                    pMember->GetPowerPercent(POWER_MANA) < 30.0f)
                    return 72.0f;
            }
        }
    }
    return 0.0f;
}

bool BehaviorDruidInnervate::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Self first if OOM
    if (ctx.manaPct < 20.0f && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.druid.pInnervate))
        return m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.druid.pInnervate) == SPELL_CAST_OK;

    // Then party healers
    if (Group* pGroup = m_ai->me->GetGroup())
    {
        for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                if (pMember == m_ai->me || !pMember->IsAlive())
                    continue;
                uint8 cls = pMember->GetClass();
                if ((cls == CLASS_PRIEST || cls == CLASS_DRUID || cls == CLASS_SHAMAN || cls == CLASS_PALADIN) &&
                    pMember->GetPowerPercent(POWER_MANA) < 30.0f &&
                    m_ai->CanTryToCastSpell(pMember, m_ai->m_spells.druid.pInnervate))
                {
                    return m_ai->DoCastSpell(pMember, m_ai->m_spells.druid.pInnervate) == SPELL_CAST_OK;
                }
            }
        }
    }
    return false;
}
