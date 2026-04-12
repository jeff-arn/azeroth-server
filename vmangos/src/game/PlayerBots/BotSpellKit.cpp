#include "BotSpellKit.h"
#include "CombatBotBaseAI.h"
#include "Player.h"

BotSpellKit::BotSpellKit(uint8 classId, CombatBotBaseAI* ai)
    : m_classId(classId), m_ai(ai)
{
}

SpellEntry const* BotSpellKit::GetInterrupt() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pPummel;
        case CLASS_ROGUE:   return m_ai->m_spells.rogue.pKick;
        case CLASS_MAGE:    return m_ai->m_spells.mage.pCounterspell;
        case CLASS_SHAMAN:  return m_ai->m_spells.shaman.pEarthShock;
        case CLASS_PRIEST:  return m_ai->m_spells.priest.pSilence;
        case CLASS_DRUID:   return m_ai->m_spells.druid.pBash;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetInterruptAlt() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pShieldBash;
        case CLASS_ROGUE:   return m_ai->m_spells.rogue.pGouge;
        case CLASS_HUNTER:  return m_ai->m_spells.hunter.pConcussiveShot;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetFiller() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR:
            if (m_ai->m_role == ROLE_TANK)
            {
                // Tank priority: Shield Slam > Revenge > Sunder Armor
                if (m_ai->m_spells.warrior.pShieldSlam) return m_ai->m_spells.warrior.pShieldSlam;
                if (m_ai->m_spells.warrior.pRevenge) return m_ai->m_spells.warrior.pRevenge;
                return m_ai->m_spells.warrior.pSunderArmor;
            }
            // DPS: Bloodthirst > Mortal Strike > Heroic Strike
            if (m_ai->m_spells.warrior.pBloodthirst) return m_ai->m_spells.warrior.pBloodthirst;
            if (m_ai->m_spells.warrior.pMortalStrike) return m_ai->m_spells.warrior.pMortalStrike;
            return m_ai->m_spells.warrior.pHeroicStrike;
        case CLASS_ROGUE:
            // Backstab > Sinister Strike
            if (m_ai->m_spells.rogue.pBackstab) return m_ai->m_spells.rogue.pBackstab;
            return m_ai->m_spells.rogue.pSinisterStrike;
        case CLASS_HUNTER:
            if (m_ai->m_spells.hunter.pAimedShot) return m_ai->m_spells.hunter.pAimedShot;
            return m_ai->m_spells.hunter.pArcaneShot;
        case CLASS_MAGE:
            if (m_ai->m_spells.mage.pFrostbolt) return m_ai->m_spells.mage.pFrostbolt;
            if (m_ai->m_spells.mage.pFireball) return m_ai->m_spells.mage.pFireball;
            return m_ai->m_spells.mage.pScorch;
        case CLASS_PRIEST:
            if (m_ai->m_spells.priest.pMindFlay) return m_ai->m_spells.priest.pMindFlay;
            if (m_ai->m_spells.priest.pMindBlast) return m_ai->m_spells.priest.pMindBlast;
            return m_ai->m_spells.priest.pSmite;
        case CLASS_WARLOCK:
            return m_ai->m_spells.warlock.pShadowBolt;
        case CLASS_PALADIN:
            if (m_ai->m_role == ROLE_TANK)
            {
                if (m_ai->m_spells.paladin.pHolyShield) return m_ai->m_spells.paladin.pHolyShield;
                return m_ai->m_spells.paladin.pConsecration;
            }
            return m_ai->m_spells.paladin.pJudgement;
        case CLASS_SHAMAN:
            if (m_ai->m_spells.shaman.pStormstrike) return m_ai->m_spells.shaman.pStormstrike;
            return m_ai->m_spells.shaman.pLightningBolt;
        case CLASS_DRUID:
            switch (m_ai->me->GetShapeshiftForm())
            {
                case FORM_CAT:
                    if (m_ai->m_spells.druid.pShred) return m_ai->m_spells.druid.pShred;
                    return m_ai->m_spells.druid.pClaw;
                case FORM_BEAR:
                case FORM_DIREBEAR:
                    return m_ai->m_spells.druid.pMaul;
                default:
                    return m_ai->m_spells.druid.pWrath;
            }
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetExecuteAbility() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pExecute;
        case CLASS_WARLOCK: return m_ai->m_spells.warlock.pShadowburn;
        case CLASS_PALADIN: return m_ai->m_spells.paladin.pHammerOfWrath;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetAoeSpell() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pWhirlwind;
        case CLASS_MAGE:
            if (m_ai->m_spells.mage.pBlizzard) return m_ai->m_spells.mage.pBlizzard;
            return m_ai->m_spells.mage.pArcaneExplosion;
        case CLASS_WARLOCK: return m_ai->m_spells.warlock.pRainOfFire;
        case CLASS_PRIEST:  return m_ai->m_spells.priest.pHolyNova;
        case CLASS_PALADIN: return m_ai->m_spells.paladin.pConsecration;
        case CLASS_HUNTER:  return m_ai->m_spells.hunter.pVolley;
        case CLASS_DRUID:   return m_ai->m_spells.druid.pHurricane;
        case CLASS_SHAMAN:  return m_ai->m_spells.shaman.pChainLightning;
        default: return nullptr;
    }
}

void BotSpellKit::GetDotSpells(std::vector<SpellEntry const*>& out) const
{
    out.clear();
    switch (m_classId)
    {
        case CLASS_WARLOCK:
            if (m_ai->m_spells.warlock.pCorruption) out.push_back(m_ai->m_spells.warlock.pCorruption);
            if (m_ai->m_spells.warlock.pImmolate)   out.push_back(m_ai->m_spells.warlock.pImmolate);
            if (m_ai->m_spells.warlock.pSiphonLife)  out.push_back(m_ai->m_spells.warlock.pSiphonLife);
            break;
        case CLASS_PRIEST:
            if (m_ai->m_spells.priest.pShadowWordPain) out.push_back(m_ai->m_spells.priest.pShadowWordPain);
            if (m_ai->m_spells.priest.pDevouringPlague) out.push_back(m_ai->m_spells.priest.pDevouringPlague);
            break;
        case CLASS_DRUID:
            if (m_ai->m_spells.druid.pMoonfire)    out.push_back(m_ai->m_spells.druid.pMoonfire);
            if (m_ai->m_spells.druid.pInsectSwarm)  out.push_back(m_ai->m_spells.druid.pInsectSwarm);
            if (m_ai->m_spells.druid.pRake)         out.push_back(m_ai->m_spells.druid.pRake);
            break;
        case CLASS_HUNTER:
            if (m_ai->m_spells.hunter.pSerpentSting) out.push_back(m_ai->m_spells.hunter.pSerpentSting);
            break;
        case CLASS_SHAMAN:
            if (m_ai->m_spells.shaman.pFlameShock) out.push_back(m_ai->m_spells.shaman.pFlameShock);
            break;
        case CLASS_ROGUE:
            if (m_ai->m_spells.rogue.pGarrote) out.push_back(m_ai->m_spells.rogue.pGarrote);
            if (m_ai->m_spells.rogue.pRupture) out.push_back(m_ai->m_spells.rogue.pRupture);
            break;
        case CLASS_WARRIOR:
            if (m_ai->m_spells.warrior.pRend) out.push_back(m_ai->m_spells.warrior.pRend);
            break;
        default:
            break;
    }
}

SpellEntry const* BotSpellKit::GetCrowdControl() const
{
    return m_ai->GetCrowdControlSpell();
}

SpellEntry const* BotSpellKit::GetRoot() const
{
    switch (m_classId)
    {
        case CLASS_MAGE:  return m_ai->m_spells.mage.pFrostNova;
        case CLASS_DRUID: return m_ai->m_spells.druid.pEntanglingRoots;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetThreatDump() const
{
    switch (m_classId)
    {
        case CLASS_ROGUE:   return m_ai->m_spells.rogue.pFeint;
        case CLASS_PRIEST:  return m_ai->m_spells.priest.pFade;
        case CLASS_DRUID:   return m_ai->m_spells.druid.pCower;
        case CLASS_HUNTER:  return m_ai->m_spells.hunter.pFeignDeath;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetSelfBuff() const
{
    switch (m_classId)
    {
        case CLASS_MAGE:    return m_ai->m_spells.mage.pIceArmor;
        case CLASS_WARLOCK: return m_ai->m_spells.warlock.pDemonArmor;
        case CLASS_PRIEST:  return m_ai->m_spells.priest.pInnerFire;
        case CLASS_SHAMAN:  return m_ai->m_spells.shaman.pLightningShield;
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pBattleShout;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetPartyBuff() const
{
    switch (m_classId)
    {
        case CLASS_MAGE:
            if (m_ai->m_spells.mage.pArcaneBrilliance) return m_ai->m_spells.mage.pArcaneBrilliance;
            return m_ai->m_spells.mage.pArcaneIntellect;
        case CLASS_PRIEST:
            if (m_ai->m_spells.priest.pPrayerofFortitude) return m_ai->m_spells.priest.pPrayerofFortitude;
            return m_ai->m_spells.priest.pPowerWordFortitude;
        case CLASS_DRUID:
            if (m_ai->m_spells.druid.pGiftoftheWild) return m_ai->m_spells.druid.pGiftoftheWild;
            return m_ai->m_spells.druid.pMarkoftheWild;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetTaunt() const
{
    if (!m_ai->m_spellListTaunt.empty())
        return m_ai->m_spellListTaunt.front();
    return nullptr;
}

SpellEntry const* BotSpellKit::GetAoeTaunt() const
{
    switch (m_classId)
    {
        case CLASS_WARRIOR: return m_ai->m_spells.warrior.pChallengingShout;
        case CLASS_DRUID:   return m_ai->m_spells.druid.pChallengingRoar;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetEmergencyAbility() const
{
    switch (m_classId)
    {
        case CLASS_MAGE:    return m_ai->m_spells.mage.pIceBlock;
        case CLASS_WARRIOR:
            if (m_ai->m_spells.warrior.pLastStand) return m_ai->m_spells.warrior.pLastStand;
            return m_ai->m_spells.warrior.pShieldWall;
        case CLASS_PALADIN: return m_ai->m_spells.paladin.pDivineShield;
        case CLASS_ROGUE:   return m_ai->m_spells.rogue.pVanish;
        case CLASS_HUNTER:  return m_ai->m_spells.hunter.pFeignDeath;
        case CLASS_WARLOCK: return m_ai->m_spells.warlock.pDeathCoil;
        case CLASS_PRIEST:  return m_ai->m_spells.priest.pPowerWordShield;
        case CLASS_DRUID:   return m_ai->m_spells.druid.pBarkskin;
        default: return nullptr;
    }
}

SpellEntry const* BotSpellKit::GetEscapeAbility() const
{
    switch (m_classId)
    {
        case CLASS_MAGE:   return m_ai->m_spells.mage.pBlink;
        case CLASS_ROGUE:  return m_ai->m_spells.rogue.pSprint;
        case CLASS_DRUID:  return m_ai->m_spells.druid.pDash;
        case CLASS_HUNTER: return m_ai->m_spells.hunter.pDisengage;
        default: return nullptr;
    }
}
