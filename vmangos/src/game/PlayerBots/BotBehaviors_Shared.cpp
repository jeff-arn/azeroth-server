#include "BotBehaviors_Shared.h"
#include "BotCombatEventBuffer.h"
#include "BotFormation.h"
#include "BotSituationContext.h"
#include "BotSpellKit.h"
#include "CombatBotBaseAI.h"
#include "Creature.h"
#include "Group.h"
#include "PartyBotAI.h"
#include "Player.h"

// Helper: is this role a caster that should not melee?
static bool IsCasterRole(uint8 role, uint8 playerClass, ShapeshiftForm form)
{
    if (role == ROLE_HEALER || role == ROLE_RANGE_DPS)
        return true;

    // Druid in caster/moonkin form
    if (playerClass == CLASS_DRUID && form != FORM_CAT && form != FORM_BEAR && form != FORM_DIREBEAR)
        return true;

    return false;
}

// ============================================================
// 1. BehaviorSelfPreserve
// ============================================================

float BehaviorSelfPreserve::Score(SituationContext const& ctx) const
{
    if (ctx.healthPct > 20.0f)
        return 0.0f;

    // Scale 90-95 based on how critical HP is
    return 90.0f + (20.0f - ctx.healthPct) * 0.25f;
}

bool BehaviorSelfPreserve::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pSpell = kit.GetEmergencyAbility();
    if (pSpell && m_ai->CanTryToCastSpell(m_ai->me, pSpell))
    {
        if (m_ai->DoCastSpell(m_ai->me, pSpell) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 2. BehaviorInterrupt
// ============================================================

float BehaviorInterrupt::Score(SituationContext const& ctx) const
{
    if (!ctx.targetIsCasting)
        return 0.0f;

    // Known dangerous spell from pattern system?
    if (ctx.activePattern && ctx.activePattern->responseType == RESPONSE_INTERRUPT)
        return 90.0f;

    // Generic: target is casting any interruptible spell
    return 85.0f;
}

bool BehaviorInterrupt::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (!ctx.currentTarget)
        return false;

    SpellEntry const* pInterrupt = kit.GetInterrupt();
    if (pInterrupt && m_ai->CanTryToCastSpell(ctx.currentTarget, pInterrupt))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pInterrupt) == SPELL_CAST_OK)
            return true;
    }

    // Try alternate interrupt
    SpellEntry const* pAlt = kit.GetInterruptAlt();
    if (pAlt && m_ai->CanTryToCastSpell(ctx.currentTarget, pAlt))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pAlt) == SPELL_CAST_OK)
            return true;
    }

    return false;
}

// ============================================================
// 3. BehaviorTaunt
// ============================================================

float BehaviorTaunt::Score(SituationContext const& ctx) const
{
    if (ctx.role != ROLE_TANK)
        return 0.0f;

    // Check if any party member is being attacked by our target
    if (Group* pGroup = m_ai->me->GetGroup())
    {
        for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                if (pMember == m_ai->me || !pMember->IsAlive())
                    continue;
                if (!pMember->GetAttackers().empty())
                    return 82.0f;
            }
        }
    }
    return 0.0f;
}

bool BehaviorTaunt::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Find the mob attacking a non-tank party member
    if (Group* pGroup = m_ai->me->GetGroup())
    {
        uint8 looseCount = 0;
        Unit* firstLoose = nullptr;

        for (auto itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* pMember = itr->getSource())
            {
                if (pMember == m_ai->me || !pMember->IsAlive())
                    continue;
                for (auto* attacker : pMember->GetAttackers())
                {
                    if (attacker->GetVictim() == pMember)
                    {
                        looseCount++;
                        if (!firstLoose)
                            firstLoose = attacker;
                    }
                }
            }
        }

        // AoE taunt if 2+ mobs loose
        if (looseCount >= 2)
        {
            SpellEntry const* pAoeTaunt = kit.GetAoeTaunt();
            if (pAoeTaunt && m_ai->CanTryToCastSpell(m_ai->me, pAoeTaunt))
            {
                if (m_ai->DoCastSpell(m_ai->me, pAoeTaunt) == SPELL_CAST_OK)
                    return true;
            }
        }

        // Single taunt
        if (firstLoose)
        {
            SpellEntry const* pTaunt = kit.GetTaunt();
            if (pTaunt && m_ai->CanTryToCastSpell(firstLoose, pTaunt))
            {
                if (m_ai->DoCastSpell(firstLoose, pTaunt) == SPELL_CAST_OK)
                    return true;
            }
        }
    }
    return false;
}

// ============================================================
// 4. BehaviorHealParty
// ============================================================

float BehaviorHealParty::Score(SituationContext const& ctx) const
{
    if (!ctx.anyAllyNeedsHeal && ctx.healthPct > 80.0f)
        return 0.0f;

    // Scale 75-85 based on how critical the deficit is
    if (ctx.criticalHealNeeded)
        return 85.0f;
    if (ctx.anyAllyNeedsHeal)
        return 78.0f;

    // Self-heal if own HP is low
    if (ctx.healthPct < 60.0f)
        return 76.0f;

    return 0.0f;
}

bool BehaviorHealParty::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Mana-aware, tank-prioritized healing.
    // Tank gets healed at generous thresholds; DPS only when genuinely hurt.
    // As mana drops, tighten thresholds to conserve for emergencies.

    if (ctx.manaPct < 30.0f)
    {
        // Emergency mana: only heal critical targets (< 30%).
        if (Unit* pTarget = m_ai->SelectHealTarget(30.0f, 30.0f))
        {
            if (m_ai->HealInjuredTargetDirect(pTarget))
                return true;
        }
        return false;
    }

    if (ctx.manaPct < 50.0f)
    {
        // Conserve mana: tank at 70%, non-tank only if < 40%.
        // HoT on tank only.
        if (ctx.tankUnit && ctx.tankHealthPct < 70.0f && !ctx.tankUnit->HasAuraType(SPELL_AURA_PERIODIC_HEAL))
        {
            if (m_ai->HealInjuredTargetPeriodic(ctx.tankUnit))
                return true;
        }
        if (Unit* pTarget = m_ai->SelectHealTarget(50.0f, 40.0f))
        {
            if (m_ai->HealInjuredTargetDirect(pTarget))
                return true;
        }
        // Heal tank at a more generous threshold than the rest.
        if (ctx.tankUnit && ctx.tankHealthPct < 70.0f)
        {
            if (m_ai->HealInjuredTargetDirect(ctx.tankUnit))
                return true;
        }
        return false;
    }

    // Normal mana (>= 50%): full healing logic.

    // HoT on tank if below 80% (proactive).
    if (ctx.tankUnit && ctx.tankHealthPct < 80.0f && !ctx.tankUnit->HasAuraType(SPELL_AURA_PERIODIC_HEAL))
    {
        if (m_ai->HealInjuredTargetPeriodic(ctx.tankUnit))
            return true;
    }

    // Direct heal: tank at 60%, non-tank at 50%.
    // SelectHealTarget returns lowest-HP member below threshold.
    // Check tank first with generous threshold, then group with strict.
    if (ctx.tankUnit && ctx.tankHealthPct < 60.0f)
    {
        if (m_ai->HealInjuredTargetDirect(ctx.tankUnit))
            return true;
    }
    if (Unit* pTarget = m_ai->SelectHealTarget(50.0f, 50.0f))
    {
        if (m_ai->HealInjuredTargetDirect(pTarget))
            return true;
    }

    // Pre-heal based on incoming damage (skip when conserving mana).
    if (m_ai->FindAndPreHealTarget())
        return true;

    return false;
}

// ============================================================
// 5. BehaviorEmergencyHeal
// ============================================================

float BehaviorEmergencyHeal::Score(SituationContext const& ctx) const
{
    if (!ctx.criticalHealNeeded)
        return 0.0f;

    // Only hybrids that have heals
    if (!CombatBotBaseAI::IsHealerClass(ctx.playerClass))
        return 0.0f;

    // Don't emergency heal if we ARE the healer (BehaviorHealParty handles that)
    if (ctx.role == ROLE_HEALER)
        return 0.0f;

    // Only if healer is unavailable (dead, OOM, or doesn't exist)
    if (ctx.healerAvailable)
        return 0.0f;

    return 80.0f;
}

bool BehaviorEmergencyHeal::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Use the existing healing system — find most injured target
    if (m_ai->FindAndHealInjuredAlly(60.0f, 30.0f))
        return true;

    return false;
}

// ============================================================
// 6. BehaviorTankThreat
// ============================================================

float BehaviorTankThreat::Score(SituationContext const& ctx) const
{
    if (ctx.role != ROLE_TANK)
        return 0.0f;
    if (!ctx.currentTarget)
        return 0.0f;

    return 75.0f;
}

bool BehaviorTankThreat::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (!ctx.currentTarget)
        return false;

    SpellEntry const* pFiller = kit.GetFiller();
    if (pFiller && m_ai->CanTryToCastSpell(ctx.currentTarget, pFiller))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pFiller) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 7. BehaviorThreatDump
// ============================================================

float BehaviorThreatDump::Score(SituationContext const& ctx) const
{
    if (ctx.role == ROLE_TANK)
        return 0.0f;
    if (ctx.threatRatio < 0.80f)
        return 0.0f;

    // Check if class actually has a threat dump ability
    switch (ctx.playerClass)
    {
    case CLASS_ROGUE: // Feint
    case CLASS_PRIEST: // Fade
    case CLASS_DRUID: // Cower
    case CLASS_HUNTER: // Feign Death
        break;
    default:
        return 0.0f;
    }

    // Scale 70-78, capped at 78
    return std::min(78.0f, 70.0f + (ctx.threatRatio - 0.80f) * 40.0f);
}

bool BehaviorThreatDump::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pDump = kit.GetThreatDump();
    if (!pDump)
        return false;

    // Some threat dumps target self (Fade), some target enemy (Feint)
    Unit* target = ctx.currentTarget ? ctx.currentTarget : m_ai->me;
    if (m_ai->CanTryToCastSpell(target, pDump))
    {
        if (m_ai->DoCastSpell(target, pDump) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 8. BehaviorCrowdControl
// ============================================================

float BehaviorCrowdControl::Score(SituationContext const& ctx) const
{
    if (!m_ai->GetCrowdControlSpell())
        return 0.0f;

    // Always handle manually-assigned CC marks.
    if (!m_ai->m_marksToCC.empty())
        return 68.0f;

    // Auto-CC: only DPS roles, and only when there are loose adds.
    if (ctx.role == ROLE_TANK || ctx.role == ROLE_HEALER)
        return 0.0f;
    if (ctx.hasLooseAdds)
        return 66.0f;

    return 0.0f;
}

bool BehaviorCrowdControl::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Try manually/auto-assigned marked targets first.
    if (m_ai->CrowdControlMarkedTargets())
        return true;

    // Auto-CC: find a loose add, mark it, and CC it.
    if (ctx.role != ROLE_TANK && ctx.role != ROLE_HEALER && ctx.hasLooseAdds)
    {
        if (Unit* pCCTarget = m_ai->SelectAutoCCTarget())
        {
            m_ai->AutoMarkForCC(pCCTarget);
            // Cast CC via the marked-target system (validates and casts).
            if (m_ai->CrowdControlMarkedTargets())
                return true;
        }
    }

    return false;
}

// ============================================================
// 9. BehaviorKite
// ============================================================

float BehaviorKite::Score(SituationContext const& ctx) const
{
    if (!ctx.isBeingMeleed)
        return 0.0f;
    if (!IsCasterRole(ctx.role, ctx.playerClass, ctx.currentForm))
        return 0.0f;

    // Healers: suppress kiting when the tank is dying and we're not in
    // serious danger ourselves.  Stand and heal instead.
    if (ctx.role == ROLE_HEALER && ctx.tankIsAlive && ctx.tankHealthPct < 30.0f && ctx.healthPct > 50.0f)
        return 0.0f;

    // Scale with HP: lower HP = more urgent to escape
    // At 100% HP: score 30 (low, keep casting)
    // At 30% HP: score 70 (high, need to move)
    return 30.0f + (100.0f - ctx.healthPct) * 0.57f;
}

bool BehaviorKite::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Try root first
    SpellEntry const* pRoot = kit.GetRoot();
    if (pRoot && ctx.currentTarget && m_ai->CanTryToCastSpell(ctx.currentTarget, pRoot))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pRoot) == SPELL_CAST_OK)
            return true;
    }

    // Formation-aware flee: run backward along cleared trail.
    if (ctx.hasFleeTarget)
    {
        m_ai->me->GetMotionMaster()->MovePoint(0, ctx.fleeTargetX, ctx.fleeTargetY, ctx.fleeTargetZ, MOVE_PATHFINDING);
        return true;
    }

    // Fallback: run toward tank (old behavior).
    if (ctx.currentTarget)
    {
        if (m_ai->RunAwayFromTarget(ctx.currentTarget))
            return true;
    }

    return false;
}

// ============================================================
// 10. BehaviorExecuteTarget
// ============================================================

float BehaviorExecuteTarget::Score(SituationContext const& ctx) const
{
    if (!ctx.currentTarget || ctx.targetHealthPct > 20.0f)
        return 0.0f;
    if (ctx.shouldThrottle)
        return 0.0f;
    if (!kit_has_execute(ctx))
        return 0.0f;
    return 65.0f;
}

bool BehaviorExecuteTarget::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pExec = kit.GetExecuteAbility();
    if (pExec && m_ai->CanTryToCastSpell(ctx.currentTarget, pExec))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pExec) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// Helper for Score — check without constructing the kit
bool BehaviorExecuteTarget::kit_has_execute(SituationContext const& ctx) const
{
    switch (ctx.playerClass)
    {
    case CLASS_WARRIOR:
    case CLASS_WARLOCK:
    case CLASS_PALADIN:
        return true;
    default:
        return false;
    }
}

// ============================================================
// 11. BehaviorApplyDots
// ============================================================

float BehaviorApplyDots::Score(SituationContext const& ctx) const
{
    if (!ctx.currentTarget || ctx.targetHealthPct < 30.0f)
        return 0.0f;
    if (ctx.shouldThrottle)
        return 0.0f;

    // Only score if the class actually has DoT spells
    switch (ctx.playerClass)
    {
    case CLASS_WARLOCK:
    case CLASS_PRIEST:
    case CLASS_DRUID:
    case CLASS_HUNTER:
    case CLASS_SHAMAN:
    case CLASS_ROGUE:
    case CLASS_WARRIOR:
        break;
    default:
        return 0.0f; // Mage, Paladin have no DoTs
    }

    return 55.0f;
}

bool BehaviorApplyDots::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    std::vector<SpellEntry const*> dots;
    kit.GetDotSpells(dots);

    for (auto const* pDot : dots)
    {
        if (pDot && m_ai->CanTryToCastSpell(ctx.currentTarget, pDot))
        {
            if (m_ai->DoCastSpell(ctx.currentTarget, pDot) == SPELL_CAST_OK)
                return true;
        }
    }
    return false;
}

// ============================================================
// 12. BehaviorAoeDamage
// ============================================================

float BehaviorAoeDamage::Score(SituationContext const& ctx) const
{
    if (!ctx.currentTarget || ctx.shouldThrottle)
        return 0.0f;

    uint8 enemyCount = m_ai->GetAttackersInRangeCount(10.0f);
    if (ctx.currentTarget)
    {
        // Use enemy count around target for ranged AoE
        enemyCount = std::max(enemyCount, (uint8)m_ai->me->GetEnemyCountInRadiusAround(ctx.currentTarget, 10.0f));
    }

    if (enemyCount < 3)
        return 0.0f;

    return 55.0f;
}

bool BehaviorAoeDamage::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pAoe = kit.GetAoeSpell();
    if (pAoe && ctx.currentTarget && m_ai->CanTryToCastSpell(ctx.currentTarget, pAoe))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pAoe) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 13. BehaviorDirectDamage
// ============================================================

float BehaviorDirectDamage::Score(SituationContext const& ctx) const
{
    if (!ctx.currentTarget)
        return 0.0f;
    if (ctx.shouldThrottle)
        return 0.0f;
    return 45.0f;
}

bool BehaviorDirectDamage::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pFiller = kit.GetFiller();
    if (pFiller && m_ai->CanTryToCastSpell(ctx.currentTarget, pFiller))
    {
        if (m_ai->DoCastSpell(ctx.currentTarget, pFiller) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 14. BehaviorSelfBuff
// ============================================================

float BehaviorSelfBuff::Score(SituationContext const& ctx) const
{
    SpellEntry const* pBuff = nullptr;
    switch (ctx.playerClass)
    {
    case CLASS_MAGE:
        pBuff = m_ai->m_spells.mage.pIceArmor;
        break;
    case CLASS_WARLOCK:
        pBuff = m_ai->m_spells.warlock.pDemonArmor;
        break;
    case CLASS_PRIEST:
        pBuff = m_ai->m_spells.priest.pInnerFire;
        break;
    case CLASS_SHAMAN:
        pBuff = m_ai->m_spells.shaman.pLightningShield;
        break;
    case CLASS_WARRIOR:
        pBuff = m_ai->m_spells.warrior.pBattleShout;
        break;
    default:
        return 0.0f;
    }

    if (!pBuff)
        return 0.0f;

    // Check if we already have the buff
    if (m_ai->me->HasAura(pBuff->Id))
        return 0.0f;

    return 40.0f;
}

bool BehaviorSelfBuff::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    SpellEntry const* pBuff = kit.GetSelfBuff();
    if (pBuff && m_ai->CanTryToCastSpell(m_ai->me, pBuff))
    {
        if (m_ai->DoCastSpell(m_ai->me, pBuff) == SPELL_CAST_OK)
            return true;
    }
    return false;
}

// ============================================================
// 15. BehaviorPosition
// ============================================================

float BehaviorPosition::Score(SituationContext const& ctx) const
{
    // Formation-aware: only when NOT in combat (no target). Combat uses MoveChase distances.
    if (!ctx.currentTarget && ctx.hasFormationTarget && ctx.distToFormationPos > BotFormationDefaults::MOVE_DEADZONE)
        return 35.0f;

    Unit* posTarget = ctx.currentTarget;
    bool isCaster = IsCasterRole(ctx.role, ctx.playerClass, ctx.currentForm);

    if (isCaster)
    {
        // For ranged/healers: position relative to mob (own target or tank's target)
        if (!posTarget && ctx.role == ROLE_HEALER)
        {
            if (PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(m_ai))
            {
                if (Player* pTank = partyAI->FindPartyTank())
                    posTarget = pTank->GetVictim();
            }
        }
        if (!posTarget)
            return 0.0f;

        float dist = m_ai->me->GetCombatDistance(posTarget);

        // Too far: need to close.  Healers always score when OOR
        // (post-kite recovery) so they walk back into heal range.
        if (dist > 30.0f)
        {
            if (ctx.role == ROLE_HEALER)
                return 35.0f;
            if (m_ai->me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE)
                return 35.0f;
        }

        // Too close: need to back away — but only if not already moving away
        if (dist < 10.0f && m_ai->me->GetMotionMaster()->GetCurrentMovementGeneratorType() != DISTANCING_MOTION_TYPE && m_ai->me->GetMotionMaster()->GetCurrentMovementGeneratorType() != CHASE_MOTION_TYPE)
            return 35.0f;
    }
    else
    {
        if (!posTarget)
            return 0.0f;

        // Melee: need to close gap
        if (!m_ai->me->CanReachWithMeleeAutoAttack(posTarget) && m_ai->me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE)
            return 35.0f;
    }

    return 0.0f;
}

bool BehaviorPosition::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Formation-aware: only when NOT in combat (no target). Combat uses MoveChase distances.
    if (!ctx.currentTarget && ctx.hasFormationTarget && ctx.distToFormationPos > BotFormationDefaults::MOVE_DEADZONE)
    {
        m_ai->me->GetMotionMaster()->MovePoint(0, ctx.formationTargetX, ctx.formationTargetY, ctx.formationTargetZ, MOVE_PATHFINDING);
        return true;
    }

    Unit* posTarget = ctx.currentTarget;
    bool isCaster = IsCasterRole(ctx.role, ctx.playerClass, ctx.currentForm);

    if (isCaster)
    {
        if (!posTarget && ctx.role == ROLE_HEALER)
        {
            if (PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(m_ai))
            {
                if (Player* pTank = partyAI->FindPartyTank())
                    posTarget = pTank->GetVictim();
            }
        }
        if (!posTarget)
            return false;

        m_ai->me->GetMotionMaster()->MoveChase(posTarget, 25.0f);
    }
    else
    {
        if (!posTarget)
            return false;

        m_ai->me->GetMotionMaster()->MoveChase(posTarget);
    }

    return true;
}

// ============================================================
// 16. BehaviorDispel
// ============================================================

float BehaviorDispel::Score(SituationContext const& ctx) const
{
    // Dispel is handled as a pre-step in the dispatcher (like pet commands)
    // because CheckForDispelTargets() is too expensive to call in both
    // Score and Execute. Score always returns 0.
    return 0.0f;
}

bool BehaviorDispel::Execute(SituationContext const& ctx, BotSpellKit& kit) { return m_ai->CheckForDispelTargets(); }

// ============================================================
// 17. BehaviorPartyBuff
// ============================================================

float BehaviorPartyBuff::Score(SituationContext const& ctx) const
{
    // Check if we actually have any party buff that needs applying.
    // Returns 0 if no buffs are available or all are already applied.
    auto HasBuffTarget = [this](SpellEntry const* pSpell) -> bool { return pSpell && m_ai->SelectBuffTarget(pSpell) != nullptr; };

    switch (ctx.playerClass)
    {
    case CLASS_DRUID:
        if (HasBuffTarget(m_ai->m_spells.druid.pGiftoftheWild))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.druid.pMarkoftheWild))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.druid.pThorns))
            return 52.0f;
        break;
    case CLASS_PRIEST:
        if (HasBuffTarget(m_ai->m_spells.priest.pPrayerofFortitude))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.priest.pPowerWordFortitude))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.priest.pPrayerofSpirit))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.priest.pDivineSpirit))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.priest.pPrayerofShadowProtection))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.priest.pShadowProtection))
            return 52.0f;
        break;
    case CLASS_MAGE:
        if (HasBuffTarget(m_ai->m_spells.mage.pArcaneBrilliance))
            return 52.0f;
        if (HasBuffTarget(m_ai->m_spells.mage.pArcaneIntellect))
            return 52.0f;
        break;
    case CLASS_WARLOCK:
        if (HasBuffTarget(m_ai->m_spells.warlock.pDetectInvisibility))
            return 52.0f;
        break;
    case CLASS_PALADIN:
        if (m_ai->m_spells.paladin.pAura && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.paladin.pAura))
            return 52.0f;
        break;
    default:
        break;
    }
    return 0.0f;
}

bool BehaviorPartyBuff::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    // Per-class party buff lists, checked in priority order via SelectBuffTarget().
    // SelectBuffTarget() returns nullptr if all party members already have the buff.
    auto TryBuff = [this](SpellEntry const* pSpell) -> bool
    {
        if (!pSpell)
            return false;
        if (Player* pTarget = m_ai->SelectBuffTarget(pSpell))
        {
            if (m_ai->CanTryToCastSpell(pTarget, pSpell))
            {
                if (m_ai->DoCastSpell(pTarget, pSpell) == SPELL_CAST_OK)
                {
                    m_ai->me->ClearTarget();
                    return true;
                }
            }
        }
        return false;
    };

    // Thorns-style buffs: prioritize the tank, then anyone
    auto TryBuffTankFirst = [this](SpellEntry const* pSpell) -> bool
    {
        if (!pSpell)
            return false;
        // Try tank first
        if (PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(m_ai))
        {
            if (Player* pTank = partyAI->FindPartyTank())
            {
                if (m_ai->CanTryToCastSpell(pTank, pSpell))
                {
                    if (m_ai->DoCastSpell(pTank, pSpell) == SPELL_CAST_OK)
                    {
                        m_ai->me->ClearTarget();
                        return true;
                    }
                }
            }
        }
        // Then anyone who needs it
        if (Player* pTarget = m_ai->SelectBuffTarget(pSpell))
        {
            if (m_ai->CanTryToCastSpell(pTarget, pSpell))
            {
                if (m_ai->DoCastSpell(pTarget, pSpell) == SPELL_CAST_OK)
                {
                    m_ai->me->ClearTarget();
                    return true;
                }
            }
        }
        return false;
    };

    switch (ctx.playerClass)
    {
    case CLASS_DRUID:
        // Gift of the Wild > Mark of the Wild
        if (TryBuff(m_ai->m_spells.druid.pGiftoftheWild))
            return true;
        if (TryBuff(m_ai->m_spells.druid.pMarkoftheWild))
            return true;
        // Thorns on tank
        if (TryBuffTankFirst(m_ai->m_spells.druid.pThorns))
            return true;
        break;

    case CLASS_PRIEST:
        // Prayer of Fortitude > Power Word: Fortitude
        if (TryBuff(m_ai->m_spells.priest.pPrayerofFortitude))
            return true;
        if (TryBuff(m_ai->m_spells.priest.pPowerWordFortitude))
            return true;
        // Prayer of Spirit > Divine Spirit
        if (TryBuff(m_ai->m_spells.priest.pPrayerofSpirit))
            return true;
        if (TryBuff(m_ai->m_spells.priest.pDivineSpirit))
            return true;
        // Shadow Protection
        if (TryBuff(m_ai->m_spells.priest.pPrayerofShadowProtection))
            return true;
        if (TryBuff(m_ai->m_spells.priest.pShadowProtection))
            return true;
        break;

    case CLASS_MAGE:
        // Arcane Brilliance > Arcane Intellect
        if (TryBuff(m_ai->m_spells.mage.pArcaneBrilliance))
            return true;
        if (TryBuff(m_ai->m_spells.mage.pArcaneIntellect))
            return true;
        break;

    case CLASS_WARLOCK:
        if (TryBuff(m_ai->m_spells.warlock.pDetectInvisibility))
            return true;
        break;

    case CLASS_PALADIN:
        // Aura
        if (m_ai->m_spells.paladin.pAura && m_ai->CanTryToCastSpell(m_ai->me, m_ai->m_spells.paladin.pAura))
        {
            if (m_ai->DoCastSpell(m_ai->me, m_ai->m_spells.paladin.pAura) == SPELL_CAST_OK)
                return true;
        }
        // Blessings handled by BehaviorPaladinBlessing (class-unique)
        break;

    default:
        break;
    }

    return false;
}

// ============================================================
// 18. BehaviorWandFiller
// ============================================================

static const uint32 PB_SPELL_SHOOT_WAND_ID = 5019;

float BehaviorWandFiller::Score(SituationContext const& ctx) const
{
    // Only for healers in combat with nothing else to do
    if (ctx.role != ROLE_HEALER)
        return 0.0f;
    if (!m_ai->me->HasSpell(PB_SPELL_SHOOT_WAND_ID))
        return 0.0f;
    if (ctx.isCasting || ctx.isMoving)
        return 0.0f;
    if (m_ai->me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
        return 0.0f;

    // Don't wand if the tank needs healing — let the tick fall through
    // to follow/position logic so the healer moves back into heal range.
    if (ctx.tankNeedsHeal || ctx.criticalHealNeeded)
        return 0.0f;

    // Need a target — use own victim or find tank's target
    if (ctx.currentTarget)
        return 5.0f;
    if (Player* pTank = m_ai->FindPartyTank())
    {
        if (pTank->GetVictim())
            return 5.0f;
    }

    return 0.0f;
}

bool BehaviorWandFiller::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    Unit* pTarget = ctx.currentTarget;
    if (!pTarget)
    {
        if (Player* pTank = m_ai->FindPartyTank())
            pTarget = pTank->GetVictim();
    }
    if (!pTarget)
        return false;

    m_ai->me->CastSpell(pTarget, PB_SPELL_SHOOT_WAND_ID, false);
    return true;
}

// ============================================================
// 19. BehaviorCasterAttackStop
// ============================================================

float BehaviorCasterAttackStop::Score(SituationContext const& ctx) const
{
    // This is a special behavior that doesn't consume the tick.
    // It just cancels auto-attack. Score is 0 — it's handled
    // separately in the dispatcher, not through scoring.
    return 0.0f;
}

bool BehaviorCasterAttackStop::Execute(SituationContext const& ctx, BotSpellKit& kit)
{
    if (IsCasterRole(ctx.role, ctx.playerClass, ctx.currentForm))
        m_ai->me->AttackStop();
    return false; // Does not consume the tick
}
