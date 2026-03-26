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
#include "GuildBotMgr.h"
#include "BotServiceMgr.h"
#include "Player.h"
#include "CreatureAI.h"
#include "MotionMaster.h"
#include "ObjectMgr.h"
#include "PlayerBotMgr.h"
#include "Opcodes.h"
#include "World.h"
#include "WorldPacket.h"
#include "Spell.h"
#include "SpellAuras.h"
#include "Chat.h"
#include "TradeData.h"
#include <random>

enum PartyBotSpells
{
    PB_SPELL_FOOD = 1131,
    PB_SPELL_DRINK = 1137,
    PB_SPELL_AUTO_SHOT = 75,
    PB_SPELL_SHOOT_WAND = 5019,
    PB_SPELL_HONORLESS_TARGET = 2479,
};

#define PB_UPDATE_INTERVAL 1000
#define PB_MIN_FOLLOW_DIST 3.0f
#define PB_MAX_FOLLOW_DIST 6.0f
#define PB_MIN_FOLLOW_ANGLE 0.0f
#define PB_MAX_FOLLOW_ANGLE 6.0f

// ============================================================
// Boss Override Registry
// Populated with data from DUNGEON_RAID_GUIDE.md
// ============================================================
static std::unordered_map<uint32, BossOverride> s_bossOverrides;
static bool s_bossOverridesInitialized = false;

void InitBossOverrides()
{
    if (s_bossOverridesInitialized)
        return;
    s_bossOverridesInitialized = true;

    // --- Ragefire Chasm ---
    // Jergosh the Invoker: interrupt Shadow Bolt Volley
    {
        BossOverride o;
        o.creatureEntry = 11518;
        // Shadow Bolt Volley spell ID 15245 (generic NPC version)
        o.interruptSpellIds = {15245};
        s_bossOverrides[o.creatureEntry] = o;
    }
    // Taragaman: melee should avoid Fire Nova AoE
    {
        BossOverride o;
        o.creatureEntry = 11520;
        o.avoidAoeSpellIds = {11970}; // Fire Nova
        o.avoidAoeRange = 10.0f;
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Deadmines ---
    // Rhahk'Zor: frontal Slam - melee must be behind (spreadOut flag)
    {
        BossOverride o;
        o.creatureEntry = 644;
        o.spreadOut = true; // Ensures melee stays behind
        s_bossOverrides[o.creatureEntry] = o;
    }
    // Mr. Smite: save cooldowns for Phase 3 (<33% HP)
    {
        BossOverride o;
        o.creatureEntry = 646;
        o.saveCooldownsForPhase = true;
        o.cooldownPhaseHpPercent = 33.0f;
        s_bossOverrides[o.creatureEntry] = o;
    }
    // Edwin VanCleef: interrupt Sinister Strike not needed, but disable AoE
    // so CC on adds isn't broken
    {
        BossOverride o;
        o.creatureEntry = 639;
        o.disableAoe = true; // Don't break CC on adds
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Wailing Caverns ---
    // Lord Serpentis: interrupt Lightning Bolt, healing spells
    {
        BossOverride o;
        o.creatureEntry = 3673;
        o.interruptSpellIds = {9532, 23381}; // Lightning Bolt, Healing Touch (NPC versions)
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Shadowfang Keep ---
    // Baron Silverlaine: interrupt Veil of Shadow
    {
        BossOverride o;
        o.creatureEntry = 3887;
        o.interruptSpellIds = {7068}; // Veil of Shadow
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Blackfathom Deeps ---
    // Lady Sarevess: interrupt Forked Lightning
    {
        BossOverride o;
        o.creatureEntry = 4831;
        o.interruptSpellIds = {8435}; // Forked Lightning
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Stockades ---
    // Bazil Thredd: no special mechanics, simple fight
    // Dextren Ward: no special mechanics

    // --- Gnomeregan ---
    // Electrocutioner 6000: avoid Chain Lightning (spread out)
    {
        BossOverride o;
        o.creatureEntry = 6235;
        o.spreadOut = true;
        o.interruptSpellIds = {11824}; // Chain Lightning (NPC)
        s_bossOverrides[o.creatureEntry] = o;
    }

    // --- Scarlet Monastery ---
    // High Inquisitor Whitemane: interrupt heals
    {
        BossOverride o;
        o.creatureEntry = 3977;
        o.interruptSpellIds = {12039, 9474}; // Heal, Holy Smite
        s_bossOverrides[o.creatureEntry] = o;
    }
    // Herod: avoid Whirlwind (melee run out)
    {
        BossOverride o;
        o.creatureEntry = 3975;
        o.avoidAoeSpellIds = {8989}; // Whirlwind
        o.avoidAoeRange = 12.0f;
        s_bossOverrides[o.creatureEntry] = o;
    }
}

BossOverride const* GetBossOverride(uint32 creatureEntry)
{
    InitBossOverrides();
    auto it = s_bossOverrides.find(creatureEntry);
    return (it != s_bossOverrides.end()) ? &it->second : nullptr;
}

// ============================================================
// PartyBotAI Boss Override Helpers
// ============================================================
BossOverride const* PartyBotAI::GetCurrentBossOverride() const
{
    if (Unit* pVictim = me->GetVictim())
    {
        if (Creature* pCreature = pVictim->ToCreature())
            return GetBossOverride(pCreature->GetEntry());
    }
    return nullptr;
}

bool PartyBotAI::HandleBossInterrupt(Unit* pVictim, BossOverride const* pOverride)
{
    if (!pOverride || pOverride->interruptSpellIds.empty())
        return false;

    if (!pVictim->IsNonMeleeSpellCasted(false, false, true))
        return false;

    // Check if the spell being cast is one we must interrupt
    Spell* pSpell = pVictim->GetCurrentSpell(CURRENT_GENERIC_SPELL);
    if (!pSpell)
        pSpell = pVictim->GetCurrentSpell(CURRENT_CHANNELED_SPELL);
    if (!pSpell)
        return false;

    uint32 castingSpellId = pSpell->m_spellInfo->Id;
    bool shouldInterrupt = false;
    for (uint32 spellId : pOverride->interruptSpellIds)
    {
        if (castingSpellId == spellId)
        {
            shouldInterrupt = true;
            break;
        }
    }

    if (!shouldInterrupt)
        return false;

    // Try all available interrupts
    switch (me->GetClass())
    {
        case CLASS_WARRIOR:
            if (m_spells.warrior.pPummel && CanTryToCastSpell(pVictim, m_spells.warrior.pPummel))
                return DoCastSpell(pVictim, m_spells.warrior.pPummel) == SPELL_CAST_OK;
            if (m_spells.warrior.pShieldBash && CanTryToCastSpell(pVictim, m_spells.warrior.pShieldBash))
                return DoCastSpell(pVictim, m_spells.warrior.pShieldBash) == SPELL_CAST_OK;
            break;
        case CLASS_ROGUE:
            if (m_spells.rogue.pKick && CanTryToCastSpell(pVictim, m_spells.rogue.pKick))
                return DoCastSpell(pVictim, m_spells.rogue.pKick) == SPELL_CAST_OK;
            break;
        case CLASS_MAGE:
            if (m_spells.mage.pCounterspell && CanTryToCastSpell(pVictim, m_spells.mage.pCounterspell))
                return DoCastSpell(pVictim, m_spells.mage.pCounterspell) == SPELL_CAST_OK;
            break;
        case CLASS_SHAMAN:
            if (m_spells.shaman.pEarthShock && CanTryToCastSpell(pVictim, m_spells.shaman.pEarthShock))
                return DoCastSpell(pVictim, m_spells.shaman.pEarthShock) == SPELL_CAST_OK;
            break;
        case CLASS_PRIEST:
            if (m_spells.priest.pSilence && CanTryToCastSpell(pVictim, m_spells.priest.pSilence))
                return DoCastSpell(pVictim, m_spells.priest.pSilence) == SPELL_CAST_OK;
            break;
    }
    return false;
}

bool PartyBotAI::HandleBossAoeAvoidance(Unit* pVictim, BossOverride const* pOverride)
{
    if (!pOverride || pOverride->avoidAoeSpellIds.empty())
        return false;

    // Check if boss is casting an AoE we should avoid
    Spell* pSpell = pVictim->GetCurrentSpell(CURRENT_GENERIC_SPELL);
    if (!pSpell)
        return false;

    uint32 castingSpellId = pSpell->m_spellInfo->Id;
    for (uint32 spellId : pOverride->avoidAoeSpellIds)
    {
        if (castingSpellId == spellId)
        {
            // Only melee needs to run out
            if (m_role == ROLE_MELEE_DPS || m_role == ROLE_TANK)
            {
                if (me->GetDistance(pVictim) < pOverride->avoidAoeRange)
                {
                    // Run away from the boss temporarily
                    me->GetMotionMaster()->MoveDistance(pVictim, pOverride->avoidAoeRange + 3.0f);
                    return true;
                }
            }
            break;
        }
    }
    return false;
}



bool PartyBotAI::OnSessionLoaded(PlayerBotEntry* entry, WorldSession* sess)
{
    if (!m_race && !m_class)
    {
        sess->LoginPlayer(entry->playerGUID);
        return true;
    }

    return SpawnNewPlayer(sess, m_class, m_race, m_mapId, m_instanceId, m_x, m_y, m_z, m_o, sObjectAccessor.FindPlayer(m_cloneGuid));
}

void PartyBotAI::CloneFromPlayer(Player const* pPlayer)
{
    if (!pPlayer)
        return;

    if (pPlayer->GetLevel() != me->GetLevel())
    {
        me->GiveLevel(pPlayer->GetLevel());
        me->InitTalentForLevel();
        me->SetUInt32Value(PLAYER_XP, 0);
    }

    // Learn all of the target's spells.
    for (const auto& spell : pPlayer->GetSpellMap())
    {
        if (spell.second.disabled)
            continue;

        if (spell.second.state == PLAYERSPELL_REMOVED)
            continue;

        SpellEntry const* pSpellEntry = sSpellMgr.GetSpellEntry(spell.first);
        if (!pSpellEntry)
            continue;

        uint32 const firstRankId = sSpellMgr.GetFirstSpellInChain(spell.first);
        if (!me->HasSpell(spell.first))
            me->LearnSpell(spell.first, false, (firstRankId == spell.first && GetTalentSpellPos(firstRankId)));
    }

    me->GetHonorMgr().SetHighestRank(pPlayer->GetHonorMgr().GetHighestRank());
    me->GetHonorMgr().SetRank(pPlayer->GetHonorMgr().GetRank());

    // Unequip current gear
    for (int i = EQUIPMENT_SLOT_START; i < EQUIPMENT_SLOT_END; ++i)
        me->AutoUnequipItemFromSlot(i);

    // Copy gear from target.
    for (int i = EQUIPMENT_SLOT_START; i < EQUIPMENT_SLOT_END; ++i)
    {
        if (Item* pItem = pPlayer->GetItemByPos(INVENTORY_SLOT_BAG_0, i))
        {
            me->SatisfyItemRequirements(pItem->GetProto());
            me->StoreNewItemInBestSlots(pItem->GetEntry(), 1, pItem->GetEnchantmentId(EnchantmentSlot(0)));
        }   
    }
}

Player* PartyBotAI::GetPartyLeader() const
{
    Group* pGroup = me->GetGroup();
    if (!pGroup)
        return nullptr;

    if (Player* originalLeader = ObjectAccessor::FindPlayerNotInWorld(m_leaderGuid))
    {
        if (me->InBattleGround() == originalLeader->InBattleGround())
        {
            // In case the original spawner is not in the same group as the bots anymore.
            if (pGroup != originalLeader->GetGroup())
                return nullptr;

            // In case the current leader is the bot itself and it's not inside a Battleground.
            ObjectGuid currentLeaderGuid = pGroup->GetLeaderGuid();
            if (currentLeaderGuid == me->GetObjectGuid() && !me->InBattleGround())
                return nullptr;
        }

        return originalLeader;
    }
    return nullptr;
}

// Find the party member acting as tank.
Player* PartyBotAI::FindPartyTank() const
{
    Group* pGroup = me->GetGroup();
    if (!pGroup)
        return nullptr;

    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            if (pMember == me || !pMember->IsAlive() || !pMember->IsInWorld())
                continue;
            if (pMember->GetMap() != me->GetMap())
                continue;

            // Check by role: bots with ROLE_TANK, or players in tanking form/shield
            if (pMember->AI())
            {
                if (CombatBotBaseAI* pBotAI = dynamic_cast<CombatBotBaseAI*>(pMember->AI()))
                {
                    if (pBotAI->GetRole() == ROLE_TANK)
                        return pMember;
                }
            }
            if (IsTankingForm(pMember->GetShapeshiftForm()) || IsWearingShield(pMember))
                return pMember;
        }
    }
    return nullptr;
}

// SAFE FLEE: Always run toward the tank or party leader, never blindly away.
// If already near the safe target, stop — do not chain-pull by fleeing into packs.
bool PartyBotAI::RunAwayFromTarget(Unit* pEnemy)
{
    // Find a safe target to run toward: prefer tank, fall back to leader
    Player* pTank = FindPartyTank();
    Player* pLeader = GetPartyLeader();
    Unit* pSafeTarget = pTank ? static_cast<Unit*>(pTank) : static_cast<Unit*>(pLeader);

    if (!pSafeTarget || !pSafeTarget->IsAlive() || pSafeTarget == me)
        return false; // No safe target — stand still, accept death rather than chain-pull

    float dist = me->GetDistance(pSafeTarget);
    if (dist < 5.0f)
        return false; // Already near the tank/leader, stop running

    // Move toward the safe target (tank/leader position)
    me->GetMotionMaster()->MovePoint(0,
        pSafeTarget->GetPositionX(),
        pSafeTarget->GetPositionY(),
        pSafeTarget->GetPositionZ(),
        MOVE_PATHFINDING);
    return true;
}

// Returns this bot's threat as a ratio of the tank's threat on the same enemy.
// Returns 0.0 if threat data is unavailable.
float PartyBotAI::GetThreatRatio(Unit* pEnemy) const
{
    if (!pEnemy || !pEnemy->CanHaveThreatList())
        return 0.0f;

    float myThreat = pEnemy->GetThreatManager().getThreat(const_cast<Player*>(me));
    if (myThreat <= 0.0f)
        return 0.0f;

    // Find the tank's threat on this enemy
    Player* pTank = FindPartyTank();
    if (!pTank)
    {
        // No tank identified — use the enemy's current victim (highest threat holder)
        Unit* pVictim = pEnemy->GetVictim();
        if (!pVictim || pVictim == me)
            return 0.0f;
        float victimThreat = pEnemy->GetThreatManager().getThreat(pVictim);
        return victimThreat > 0.0f ? (myThreat / victimThreat) : 0.0f;
    }

    float tankThreat = pEnemy->GetThreatManager().getThreat(pTank);
    return tankThreat > 0.0f ? (myThreat / tankThreat) : 0.0f;
}

// Returns true if the bot should stop using damage abilities to avoid pulling aggro.
// Melee: throttle at 90% of tank threat, hard stop at 100%.
// Ranged: throttle at 90%, hard stop at 130%.
bool PartyBotAI::ShouldThrottleDPS(Unit* pEnemy) const
{
    if (m_role == ROLE_TANK || m_role == ROLE_HEALER)
        return false;

    float ratio = GetThreatRatio(pEnemy);
    if (ratio <= 0.0f)
        return false;

    // Per CLASS_GROUP_BEHAVIOR_GUIDE threat rules
    if (m_role == ROLE_MELEE_DPS)
        return ratio >= 0.90f;
    else // ROLE_RANGE_DPS
        return ratio >= 0.90f;
}

bool PartyBotAI::DrinkAndEat()
{
    if (m_isBuffing)
        return false;

    if (me->GetVictim())
        return false;

    // Per CLASS_GROUP_BEHAVIOR_GUIDE: eat at <60% HP, drink at <40% mana
    bool const needToEat = me->GetHealthPercent() < 60.0f;
    bool const needToDrink = (me->GetPowerType() == POWER_MANA) && (me->GetPowerPercent(POWER_MANA) < 40.0f);

    if (!needToEat && !needToDrink)
        return false;

    bool const isEating = me->HasAura(PB_SPELL_FOOD);
    bool const isDrinking = me->HasAura(PB_SPELL_DRINK);

    if (!isEating && needToEat)
    {
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType())
        {
            me->StopMoving();
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();
        }
        if (SpellEntry const* pSpellEntry = sSpellMgr.GetSpellEntry(PB_SPELL_FOOD))
        {
            me->CastSpell(me, pSpellEntry, true);
            me->RemoveSpellCooldown(*pSpellEntry);
        }
        return true;
    }

    if (!isDrinking && needToDrink)
    {
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType())
        {
            me->StopMoving();
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();
        }
        if (SpellEntry const* pSpellEntry = sSpellMgr.GetSpellEntry(PB_SPELL_DRINK))
        {
            me->CastSpell(me, pSpellEntry, true);
            me->RemoveSpellCooldown(*pSpellEntry);
        }
        return true;
    }

    return needToEat || needToDrink;
}

bool PartyBotAI::ShouldAutoRevive() const
{
    if (me->GetDeathState() == DEAD)
        return true;

    bool alivePlayerNearby = false;
    Group* pGroup = me->GetGroup();
    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            if (pMember == me)
                continue;

            if (pMember->IsInCombat())
                return false;

            if (pMember->IsAlive())
            {
                if (IsHealerClass(pMember->GetClass()))
                    return false;

                if (me->IsWithinDistInMap(pMember, 15.0f))
                    alivePlayerNearby = true;
            }
        }
    }

    return alivePlayerNearby;
}

bool PartyBotAI::CanTryToCastSpell(Unit const* pTarget, SpellEntry const* pSpellEntry) const
{
    if (!CombatBotBaseAI::CanTryToCastSpell(pTarget, pSpellEntry))
        return false;

    if (pSpellEntry->IsAreaOfEffectSpell() && !pSpellEntry->IsPositiveSpell() && !IsInDuel())
    {
        if (!m_marksToCC.empty())
            return false;

        // do not cast aoe if it will pull aggro
        if (m_role != ROLE_TANK)
        {
            float radius;
            if (pSpellEntry->EffectRadiusIndex[0])
                radius = Spells::GetSpellRadius(sSpellRadiusStore.LookupEntry(pSpellEntry->EffectRadiusIndex[0]));
            else if (pSpellEntry->EffectRadiusIndex[1])
                radius = Spells::GetSpellRadius(sSpellRadiusStore.LookupEntry(pSpellEntry->EffectRadiusIndex[1]));
            else if (pSpellEntry->EffectRadiusIndex[2])
                radius = Spells::GetSpellRadius(sSpellRadiusStore.LookupEntry(pSpellEntry->EffectRadiusIndex[2]));
            else
                radius = 10.0f;

            std::list<Unit*> targets;
            me->GetEnemyListInRadiusAround(pTarget, radius, targets);

            for (auto const& pEnemy : targets)
            {
                if (((pEnemy->GetLevel() + 5) > me->GetLevel()) &&
                    ((pEnemy->GetHealth() * 4) > me->GetHealth()) &&
                    pEnemy->GetVictim() && pEnemy->GetVictim() != me &&
                    pEnemy->IsValidAttackTarget(me) &&
                    pEnemy->CanHaveThreatList())
                {
                    float const myThreat = pEnemy->GetThreatManager().getThreat(me);
                    float const victimThreat = pEnemy->GetThreatManager().getThreat(pEnemy->GetVictim());
                    
                    if (victimThreat < (myThreat + me->GetMaxHealth()))
                        return false;
                }
            }
        }
    }

    return true;
}

bool PartyBotAI::CanUseCrowdControl(SpellEntry const* pSpellEntry, Unit* pTarget) const
{
    if (IsInDuel())
        return true;

    if (pSpellEntry->HasAuraInterruptFlag(AURA_INTERRUPT_DAMAGE_CANCELS) &&
        AreOthersOnSameTarget(pTarget->GetObjectGuid()))
        return false;

    if (pSpellEntry->HasSingleTargetAura())
    {
        auto const& singleAuras = me->GetSingleCastSpellTargets();
        if (singleAuras.find(pSpellEntry) != singleAuras.end())
            return false;
    }

    return true;
}

bool PartyBotAI::AttackStart(Unit* pVictim)
{
    m_isBuffing = false;

    if (me->IsMounted())
        me->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED);

    if (me->Attack(pVictim, true))
    {
        if (GetRole() == ROLE_RANGE_DPS &&
            me->GetPowerPercent(POWER_MANA) > 10.0f &&
            me->GetCombatDistance(pVictim) > 8.0f)
            me->SetCasterChaseDistance(25.0f);
        else if (me->HasDistanceCasterMovement())
            me->SetCasterChaseDistance(0.0f);

        me->GetMotionMaster()->MoveChase(pVictim, 1.0f, GetRole() == ROLE_MELEE_DPS ? 3.0f : 0.0f);
        return true;
    }

    return false;
}

Unit* PartyBotAI::GetMarkedTarget(RaidTargetIcon mark) const
{
    ObjectGuid targetGuid = me->GetGroup()->GetTargetWithIcon(mark);
    if (targetGuid.IsUnit())
        return me->GetMap()->GetUnit(targetGuid);

    return nullptr;
}

Unit* PartyBotAI::SelectAttackTarget(Player* pLeader) const
{
    if (IsInDuel())
    {
        if (me->m_duel->opponent && IsValidHostileTarget(me->m_duel->opponent))
            return me->m_duel->opponent;
    }
    else
    {
        // Stick to marked target in combat.
        if (me->IsInCombat() || pLeader->GetVictim())
        {
            for (auto markId : m_marksToFocus)
            {
                ObjectGuid targetGuid = me->GetGroup()->GetTargetWithIcon(markId);
                if (targetGuid.IsUnit())
                    if (Unit* pVictim = me->GetMap()->GetUnit(targetGuid))
                        if (IsValidHostileTarget(pVictim))
                            return pVictim;
            }
        }

        // Prefer the tank's target (per CLASS guide: always attack tank's current target)
        if (Player* pTank = FindPartyTank())
        {
            if (pTank != me && pTank->GetVictim())
            {
                if (IsValidHostileTarget(pTank->GetVictim()))
                    return pTank->GetVictim();
            }
        }

        // Fall back to leader's target
        if (Unit* pVictim = pLeader->GetVictim())
        {
            if (IsValidHostileTarget(pVictim))
                return pVictim;
        }
    }

    // Who is attacking me.
    for (const auto pAttacker : me->GetAttackers())
    {
        if (IsValidHostileTarget(pAttacker))
            return pAttacker;
    }

    if (!IsInDuel())
    {
        // Check if other group members are under attack.
        if (Unit* pPartyAttacker = SelectPartyAttackTarget())
            return pPartyAttacker;
    }

    // Assist pet if its in combat.
    if (Pet* pPet = me->GetPet())
    {
        if (Unit* pPetAttacker = pPet->GetAttackerForHelper())
            if (IsValidHostileTarget(pPetAttacker))
                return pPetAttacker;
    }

    return nullptr;
}

Unit* PartyBotAI::SelectPartyAttackTarget() const
{
    Group* pGroup = me->GetGroup();
    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            // We already checked self.
            if (pMember == me)
                continue;

            for (const auto pAttacker : pMember->GetAttackers())
            {
                if (IsValidHostileTarget(pAttacker) &&
                    me->IsWithinDist(pAttacker, 50.0f))
                    return pAttacker;
            }
        }
    }

    return nullptr;
}

Player* PartyBotAI::SelectResurrectionTarget() const
{
    if (IsInDuel())
        return nullptr;

    Group* pGroup = me->GetGroup();
    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            // Can't resurrect self.
            if (pMember == me)
                continue;

            if (pMember->GetDeathState() != CORPSE)
                continue;

            if (!me->IsWithinLOSInMap(pMember))
                continue;

            if (m_resurrectionSpell->IsTargetInRange(me, pMember))
                return pMember;
        }
    }

    return nullptr;
}

Player* PartyBotAI::SelectShieldTarget() const
{
    if (IsInDuel())
        return nullptr;

    Group* pGroup = me->GetGroup();
    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            // We already checked self.
            if (pMember == me)
                continue;

            if ((pMember->GetHealthPercent() < 90.0f) &&
                !pMember->GetAttackers().empty() &&
                !pMember->IsImmuneToMechanic(MECHANIC_SHIELD))
                return pMember;
        }
    }

    return nullptr;
}

bool PartyBotAI::CrowdControlMarkedTargets()
{
    SpellEntry const* pSpellEntry = GetCrowdControlSpell();
    if (!pSpellEntry)
        return false;

    for (auto mark : m_marksToCC)
    {
        if (Unit* pTarget = GetMarkedTarget(mark))
        {
            if (!pTarget->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL) &&
                IsValidHostileTarget(pTarget) && !AreOthersOnSameTarget(pTarget->GetObjectGuid()))
            {
                if (CanTryToCastSpell(pTarget, pSpellEntry))
                {
                    if (DoCastSpell(pTarget, pSpellEntry) == SPELL_CAST_OK)
                    {
                        me->ClearUnitState(UNIT_STATE_MELEE_ATTACKING);
                        return true;
                    }
                }
            }
        }
    }
    return false;
}

void PartyBotAI::AddToPlayerGroup()
{
    Player* pPlayer = ObjectAccessor::FindPlayer(m_leaderGuid);
    if (!pPlayer)
        return;

    Group* group = pPlayer->GetGroup();
    if (!group)
    {
        group = new Group;
        // new group: if can't add then delete
        if (!group->Create(pPlayer->GetObjectGuid(), pPlayer->GetName()))
        {
            delete group;
            return;
        }
        sObjectMgr.AddGroup(group);
    }

    if (me->GetGroup() != group)
    {
        if (me->GetGroup())
            me->RemoveFromGroup();

        group->AddMember(me->GetObjectGuid(), me->GetName());
    } 
}

void PartyBotAI::OnPacketReceived(WorldPacket const* packet)
{
    //printf("Bot received %s\n", LookupOpcodeName(packet->GetOpcode()));
    switch (packet->GetOpcode())
    {
        case SMSG_LEARNED_SPELL:
        case SMSG_SUPERCEDED_SPELL:
        case SMSG_REMOVED_SPELL:
        {
            if (m_initialized)
                m_resetSpellData = true;
            return;
        }
        case SMSG_DUEL_REQUESTED:
        {
            std::unique_ptr<WorldPacket> data = std::make_unique<WorldPacket>(CMSG_DUEL_ACCEPTED, 8);
            *data << me->GetObjectGuid();
            me->GetSession()->QueuePacket(std::move(data));
            return;
        }
        case SMSG_TRADE_STATUS:
        {
            // A player initiated trade with this bot.
            // Read the trade status from the packet.
            WorldPacket pktCopy(*packet);
            uint32 status;
            pktCopy >> status;

            if (status == TRADE_STATUS_BEGIN_TRADE)
            {
                // Check if we have a pending service request with this trader.
                if (me->GetTradeData())
                {
                    Player* trader = me->GetTradeData()->GetTrader();
                    if (trader && sBotServiceMgr.ShouldAcceptTrade(me, trader))
                    {
                        // Accept the trade initiation (open trade window).
                        std::unique_ptr<WorldPacket> beginData =
                            std::make_unique<WorldPacket>(CMSG_BEGIN_TRADE, 0);
                        me->GetSession()->QueuePacket(std::move(beginData));
                        m_pendingTradeAcceptTimer = 2000; // accept after 2s delay
                    }
                    else
                    {
                        // No pending service — decline trade.
                        me->TradeCancel(true, TRADE_STATUS_BUSY);
                    }
                }
            }
            else if (status == TRADE_STATUS_OPEN_WINDOW)
            {
                // Trade window is now open. Set up bot's side (e.g., enchant spell).
                if (me->GetTradeData())
                {
                    Player* trader = me->GetTradeData()->GetTrader();
                    if (trader)
                        sBotServiceMgr.OnTradeWindowOpen(me, trader);
                }
            }
            else if (status == TRADE_STATUS_TRADE_ACCEPT)
            {
                // The other player accepted. If we haven't yet, accept now.
                if (me->GetTradeData() && !me->GetTradeData()->IsAccepted())
                {
                    std::unique_ptr<WorldPacket> acceptData =
                        std::make_unique<WorldPacket>(CMSG_ACCEPT_TRADE, 4);
                    *acceptData << uint32(1);
                    me->GetSession()->QueuePacket(std::move(acceptData));
                }
            }
            else if (status == TRADE_STATUS_TRADE_COMPLETE)
            {
                // Trade completed — deliver the service.
                // Gold was already transferred by the trade system.
                // We need to figure out how much gold we received.
                // The service manager tracks the request and will deliver.
                // Note: the trade data is already cleaned up at this point,
                // so we fire the completion from the timer-based check.
            }
            return;
        }
    }

    CombatBotBaseAI::OnPacketReceived(packet);
}

void PartyBotAI::OnPlayerLogin()
{
    if (!m_initialized)
        me->SetFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_SPAWNING);
}

void PartyBotAI::UpdateAI(uint32 const diff)
{
    // Handle pending trade accept timer (for service trades like portals).
    if (m_pendingTradeAcceptTimer)
    {
        if (m_pendingTradeAcceptTimer <= diff)
        {
            m_pendingTradeAcceptTimer = 0;
            if (me->GetTradeData() && !me->GetTradeData()->IsAccepted())
            {
                std::unique_ptr<WorldPacket> acceptData =
                    std::make_unique<WorldPacket>(CMSG_ACCEPT_TRADE, 4);
                *acceptData << uint32(1);
                me->GetSession()->QueuePacket(std::move(acceptData));
            }
        }
        else
            m_pendingTradeAcceptTimer -= diff;
    }

    m_updateTimer.Update(diff);
    if (m_updateTimer.Passed())
        m_updateTimer.Reset(PB_UPDATE_INTERVAL);
    else
        return;

    if (!me->IsInWorld() || me->IsBeingTeleported())
        return;

    if (!m_initialized)
    {
        AddToPlayerGroup();

        if (m_race && m_class) // temporary character
        {
            if (m_level && m_level != me->GetLevel())
            {
                me->GiveLevel(m_level);
                me->InitTalentForLevel();
                me->SetUInt32Value(PLAYER_XP, 0);
            }

            if (!m_cloneGuid.IsEmpty())
            {
                CloneFromPlayer(sObjectAccessor.FindPlayer(m_cloneGuid));
                AutoAssignRole();
            }
            else
            {
                LearnPremadeSpecForClass();

                if (m_role == ROLE_INVALID)
                    AutoAssignRole();

                // Guild bots use gear tier setting; others use global config
                if (sGuildBotMgr.IsEnabled() && sGuildBotMgr.IsGuildBot(me->GetGUIDLow()))
                {
                    if (!sGuildBotMgr.HasLootUpgrades(me->GetGUIDLow()))
                    {
                        GuildBotInfo const* gbi = sGuildBotMgr.GetBotInfo(me->GetGUIDLow());
                        if (gbi)
                            AutoEquipGear(sGuildBotMgr.GetAutoEquipOptionForTier(gbi->gearTier));
                        else
                            AutoEquipGear(sWorld.getConfig(CONFIG_UINT32_PARTY_BOT_AUTO_EQUIP));
                    }
                    // else: bot has loot upgrades, keep their current gear (loaded from DB)
                }
                else
                    AutoEquipGear(sWorld.getConfig(CONFIG_UINT32_PARTY_BOT_AUTO_EQUIP));

                // fix client bug causing some item slots to not be visible
                if (Player* pLeader = GetPartyLeader())
                {
                    me->SetVisibility(VISIBILITY_OFF);
                    pLeader->UpdateVisibilityOf(pLeader, me);
                    me->SetVisibility(VISIBILITY_ON);
                }
            }
            me->UpdateSkillsToMaxSkillsForLevel();
        }
        else // loaded from db
        {
            if (m_role == ROLE_INVALID)
                AutoAssignRole();

            if (me->IsGameMaster())
                me->SetGameMaster(false);

            me->TeleportTo(m_mapId, m_x, m_y, m_z, m_o);
        }

        ResetSpellData();
        PopulateSpellData();
        AddAllSpellReagents();
        me->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_SPAWNING);
        SummonPetIfNeeded();
        me->SetHealthPercent(100.0f);
        me->SetPowerPercent(me->GetPowerType(), 100.0f);

        uint32 newzone, newarea;
        me->GetZoneAndAreaId(newzone, newarea);
        me->UpdateZone(newzone, newarea);

        m_initialized = true;
        return;
    }

    if (m_resetSpellData)
    {
        ResetSpellData();
        PopulateSpellData();
        m_resetSpellData = false;
    }

    Player* pLeader = GetPartyLeader();
    if (!pLeader)
    {
        botEntry->requestRemoval = true;
        return;
    }

    if (!pLeader->IsInWorld())
        return;

    if (pLeader->InBattleGround() &&
        !me->InBattleGround())
    {
        if (m_receivedBgInvite)
        {
            SendBattlefieldPortPacket();
            m_receivedBgInvite = false;
            return;
        }
        
        // Remain idle until we can join battleground.
        return;
    }

    if (pLeader->IsTaxiFlying())
    {
        if (me->GetMotionMaster()->GetCurrentMovementGeneratorType())
        {
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();
        }
        return;
    }

    if (me->HasUnitState(UNIT_STATE_FEIGN_DEATH) && me->HasAuraType(SPELL_AURA_FEIGN_DEATH) &&
       !me->IsInCombat() && (!me->GetPet() || !me->GetPet()->IsInCombat()) &&
       !me->SelectRandomUnfriendlyTarget(nullptr, 20.0f, false, true))
        me->RemoveSpellsCausingAura(SPELL_AURA_FEIGN_DEATH);

    if (me->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL))
    {
        BreakCrowdControlEffects();
        return;
    }

    if (me->IsDead())
    {
        if (me->InBattleGround())
        {
            if (me->GetDeathState() == CORPSE)
            {
                me->BuildPlayerRepop();
                me->RepopAtGraveyard();
            }
        }
        else
        {
            if (ShouldAutoRevive())
            {
                me->ResurrectPlayer(0.5f);
                me->SpawnCorpseBones();
                me->CastSpell(me, PB_SPELL_HONORLESS_TARGET, true);
            }
        }
        
        return;
    }

    if (me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
    {
        // Stop auto shot if no target.
        if (!me->GetVictim())
            me->InterruptSpell(CURRENT_AUTOREPEAT_SPELL, true);
        else if (me->GetClass() == CLASS_HUNTER)
        {
            if (me->GetCombatDistance(me->GetVictim()) < 8.0f)
                me->InterruptSpell(CURRENT_AUTOREPEAT_SPELL, true);
            else
                UpdateInCombatAI_Hunter();
        }

        return;
    }

    if (Spell* pCurrentSpell = me->GetCurrentSpell(CURRENT_GENERIC_SPELL))
    {
        // Interrupt pre casted heals if target is not injured.
        if (pCurrentSpell->getState() == SPELL_STATE_PREPARING &&
            pCurrentSpell->m_spellInfo->IsHealSpell())
        {
            if (Unit* pTarget = pCurrentSpell->m_targets.getUnitTarget())
            {
                if (pTarget->GetHealth() == pTarget->GetMaxHealth())
                {
                    me->InterruptSpell(CURRENT_GENERIC_SPELL, true);
                }
            }
        }
    }

    if (me->IsNonMeleeSpellCasted(false, false, true))
        return;

    if (me->GetTargetGuid() == me->GetObjectGuid())
        me->ClearTarget();

    if (!me->IsInCombat())
    {
        // Reset opener delay for next combat
        m_combatEngageTimer = 0;
        m_combatEngageDelayDone = false;

        if (DrinkAndEat())
        {
            if (!me->IsWithinDistInMap(pLeader, 100.0f))
            {
                me->SetHealth(me->GetMaxHealth());
                if (me->GetPowerType() == POWER_MANA)
                    me->SetPower(POWER_MANA, me->GetMaxPower(POWER_MANA));
            }
            else if (me->IsMounted())
                me->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED);
            return;
        }

        // Teleport to leader if too far away.
        if (!me->IsWithinDistInMap(pLeader, 100.0f) && !IsInDuel())
        {
            if (!me->IsStopped())
                me->StopMoving();
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();
            char name[128] = {};
            strcpy(name, pLeader->GetName());
            ChatHandler(me).HandleGonameCommand(name);
            return;
        }
    }

    if (me->GetStandState() != UNIT_STAND_STATE_STAND)
        me->SetStandState(UNIT_STAND_STATE_STAND);

    if (me->GetSheath() == SHEATH_STATE_UNARMED && !me->IsMounted())
        me->SetSheath(SHEATH_STATE_MELEE);

    if (!me->IsInCombat() && !me->IsMounted())
    {
        UpdateOutOfCombatAI();

        if (m_isBuffing)
            return;

        if (me->IsNonMeleeSpellCasted())
            return;
    }

    Unit* pVictim = me->GetVictim();

    if (GetRole() != ROLE_HEALER)
    {
        if (!pVictim || !IsValidHostileTarget(pVictim))
        {
            if (pVictim)
                me->AttackStop();

            if (Unit* pVictim = SelectAttackTarget(pLeader))
            {
                AttackStart(pVictim);
                return;
            }
        }
    }

    if (!me->IsInCombat())
    {
        // Mount if leader is mounted and we don't have a target.
        if (pLeader->IsMounted() && !me->GetVictim())
        {
            if (!me->IsMounted())
            {
                // Leave shapeshift before mounting.
                if (me->IsInDisallowedMountForm() &&
                    me->GetDisplayId() != me->GetNativeDisplayId() &&
                    me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
                    me->RemoveSpellsCausingAura(SPELL_AURA_MOD_SHAPESHIFT);

                auto auraList = pLeader->GetAurasByType(SPELL_AURA_MOUNTED);
                if (!auraList.empty())
                {
                    bool oldStateCastTime = me->HasCheatOption(PLAYER_CHEAT_NO_CAST_TIME);
                    bool oldStatePower = me->HasCheatOption(PLAYER_CHEAT_NO_POWER);
                    me->SetCheatOption(PLAYER_CHEAT_NO_CAST_TIME, true);
                    me->SetCheatOption(PLAYER_CHEAT_NO_POWER, true);
                    me->CastSpell(me, (*auraList.begin())->GetId(), true);
                    me->SetCheatOption(PLAYER_CHEAT_NO_CAST_TIME, oldStateCastTime);
                    me->SetCheatOption(PLAYER_CHEAT_NO_POWER, oldStatePower);
                } 
            }
        }
        else if (me->IsMounted())
            me->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED);
    }

    if (!me->IsMoving())
    {
        if (!pVictim)
        {
            if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() != FOLLOW_MOTION_TYPE)
                me->GetMotionMaster()->MoveFollow(pLeader, urand(PB_MIN_FOLLOW_DIST, PB_MAX_FOLLOW_DIST), frand(PB_MIN_FOLLOW_ANGLE, PB_MAX_FOLLOW_ANGLE));
        }
        else
        {
            if (!me->HasUnitState(UNIT_STATE_MELEE_ATTACKING) &&
               (GetRole() == ROLE_MELEE_DPS || m_role == ROLE_TANK) &&
                IsValidHostileTarget(pVictim) &&
                AttackStart(pVictim))
                return;

            switch (me->GetMotionMaster()->GetCurrentMovementGeneratorType())
            {
                case IDLE_MOTION_TYPE:
                case FOLLOW_MOTION_TYPE:
                    me->GetMotionMaster()->MoveChase(pVictim);
                    break;
            }
        }
    }

    if (me->IsInCombat())
        UpdateInCombatAI();
}


void PartyBotAI::UpdateOutOfCombatAI()
{
    if (!IsInDuel())
    {
        if (m_resurrectionSpell)
            if (Player* pTarget = SelectResurrectionTarget())
                if (CanTryToCastSpell(pTarget, m_resurrectionSpell))
                    if (DoCastSpell(pTarget, m_resurrectionSpell) == SPELL_CAST_OK)
                        return;

        if (m_role != ROLE_TANK && me->GetVictim() && CrowdControlMarkedTargets())
            return;
    }

    if (CheckForDispelTargets())
        return;

    switch (me->GetClass())
    {
        case CLASS_PALADIN:
            UpdateOutOfCombatAI_Paladin();
            break;
        case CLASS_SHAMAN:
            UpdateOutOfCombatAI_Shaman();
            break;
        case CLASS_HUNTER:
            UpdateOutOfCombatAI_Hunter();
            break;
        case CLASS_MAGE:
            UpdateOutOfCombatAI_Mage();
            break;
        case CLASS_PRIEST:
            UpdateOutOfCombatAI_Priest();
            break;
        case CLASS_WARLOCK:
            UpdateOutOfCombatAI_Warlock();
            break;
        case CLASS_WARRIOR:
            UpdateOutOfCombatAI_Warrior();
            break;
        case CLASS_ROGUE:
            UpdateOutOfCombatAI_Rogue();
            break;
        case CLASS_DRUID:
            UpdateOutOfCombatAI_Druid();
            break;
    }
}

void PartyBotAI::UpdateInCombatAI()
{
    // === OPENER DELAY: Non-tanks wait 2 seconds before engaging ===
    if (m_role != ROLE_TANK && m_role != ROLE_HEALER && !m_combatEngageDelayDone)
    {
        m_combatEngageTimer += PB_UPDATE_INTERVAL;
        if (m_combatEngageTimer < 2000)
            return; // Wait for tank to build threat before DPS engages
        m_combatEngageDelayDone = true;
    }

    // === DPS THREAT THROTTLE: Stop damage abilities when close to tank threat ===
    if (m_role != ROLE_TANK && m_role != ROLE_HEALER && me->GetVictim())
    {
        if (ShouldThrottleDPS(me->GetVictim()))
        {
            // At high threat: stop casting, only auto-attack/wand
            // Healers and tanks are exempt from this check
            return;
        }
    }

    // === BOSS OVERRIDE: check for encounter-specific behavior ===
    if (BossOverride const* pBossOverride = GetCurrentBossOverride())
    {
        if (Unit* pVictim = me->GetVictim())
        {
            // Priority interrupt for boss spells
            if (HandleBossInterrupt(pVictim, pBossOverride))
                return;

            // AoE avoidance (melee run out)
            if (HandleBossAoeAvoidance(pVictim, pBossOverride))
                return;
        }
    }

    if (!IsInDuel())
    {
        if (m_role == ROLE_TANK)
        {
            Unit* pVictim = me->GetVictim();

            // Defend party members.
            if (!pVictim || pVictim->GetVictim() == me)
            {
                if (pVictim = SelectPartyAttackTarget())
                {
                    me->AttackStop(true);
                    AttackStart(pVictim);
                }
            }

            // Taunt target if its attacking someone else.
            if (pVictim && pVictim->GetVictim() != me)
            {
                for (const auto& pSpellEntry : m_spellListTaunt)
                {
                    if (CanTryToCastSpell(pVictim, pSpellEntry))
                    {
                        if (DoCastSpell(pVictim, pSpellEntry) == SPELL_CAST_OK)
                            return;
                    }
                }
            }
        }
        else if (CrowdControlMarkedTargets())
            return;
    }

    if (CheckForDispelTargets())
        return;

    switch (me->GetClass())
    {
        case CLASS_PALADIN:
            UpdateInCombatAI_Paladin();
            break;
        case CLASS_SHAMAN:
            UpdateInCombatAI_Shaman();
            break;
        case CLASS_HUNTER:
            UpdateInCombatAI_Hunter();
            break;
        case CLASS_MAGE:
            UpdateInCombatAI_Mage();
            break;
        case CLASS_PRIEST:
            UpdateInCombatAI_Priest();
            break;
        case CLASS_WARLOCK:
            UpdateInCombatAI_Warlock();
            break;
        case CLASS_WARRIOR:
            UpdateInCombatAI_Warrior();
            break;
        case CLASS_ROGUE:
            UpdateInCombatAI_Rogue();
            break;
        case CLASS_DRUID:
            UpdateInCombatAI_Druid();
            break;
    }

    if (me->GetVictim())
        UseTrinketEffects();
}

bool PartyBotAI::CheckForDispelTargets()
{
    if (me->GetShapeshiftForm() != FORM_NONE)
        return false;

    switch (me->GetClass())
    {
        case CLASS_PALADIN:
        {
            if (m_spells.paladin.pCleanse)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.paladin.pCleanse))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.paladin.pCleanse))
                    {
                        if (DoCastSpell(pFriend, m_spells.paladin.pCleanse) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            break;
        }
        case CLASS_SHAMAN:
        {
            if (m_spells.shaman.pCureDisease)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.shaman.pCureDisease))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.shaman.pCureDisease))
                    {
                        if (DoCastSpell(pFriend, m_spells.shaman.pCureDisease) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }

            if (m_spells.shaman.pCurePoison)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.shaman.pCurePoison))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.shaman.pCurePoison))
                    {
                        if (DoCastSpell(pFriend, m_spells.shaman.pCurePoison) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            break;
        }
        case CLASS_MAGE:
        {
            if (m_spells.mage.pRemoveLesserCurse)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.mage.pRemoveLesserCurse))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.mage.pRemoveLesserCurse))
                    {
                        if (DoCastSpell(pFriend, m_spells.mage.pRemoveLesserCurse) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            break;
        }
        case CLASS_PRIEST:
        {
            if (m_spells.priest.pDispelMagic)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.priest.pDispelMagic))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.priest.pDispelMagic))
                    {
                        if (DoCastSpell(pFriend, m_spells.priest.pDispelMagic) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            if (m_spells.priest.pAbolishDisease)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.priest.pAbolishDisease))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.priest.pAbolishDisease))
                    {
                        if (DoCastSpell(pFriend, m_spells.priest.pAbolishDisease) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            break;
        }
        case CLASS_DRUID:
        {
            SpellEntry const* pDispelSpell = m_spells.druid.pAbolishPoison ?
                m_spells.druid.pAbolishPoison :
                m_spells.druid.pCurePoison;

            if (pDispelSpell)
            {
                if (Unit* pFriend = SelectDispelTarget(pDispelSpell))
                {
                    if (CanTryToCastSpell(pFriend, pDispelSpell))
                    {
                        if (DoCastSpell(pFriend, pDispelSpell) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }

            if (m_spells.druid.pRemoveCurse)
            {
                if (Unit* pFriend = SelectDispelTarget(m_spells.druid.pRemoveCurse))
                {
                    if (CanTryToCastSpell(pFriend, m_spells.druid.pRemoveCurse))
                    {
                        if (DoCastSpell(pFriend, m_spells.druid.pRemoveCurse) == SPELL_CAST_OK)
                            return true;
                    }
                }
            }
            break;
        }
    }

    return false;
}

