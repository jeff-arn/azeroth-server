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
#include <algorithm>
#include <random>
#include "BotChatManager.h"
#include "BotCombatEventBuffer.h"
#include "BotFormation.h"
#include "BotRoleConfig.h"
#include "BotServiceMgr.h"
#include "Chat.h"
#include "CreatureAI.h"
#include "GuildBotMgr.h"
#include "MotionMaster.h"
#include "ObjectMgr.h"
#include "Opcodes.h"
#include "Player.h"
#include "PlayerBotMgr.h"
#include "Spell.h"
#include "SpellAuras.h"
#include "TradeData.h"
#include "World.h"
#include "WorldPacket.h"

enum PartyBotSpells
{
    PB_SPELL_FOOD = 1131,
    PB_SPELL_DRINK = 1137,
    PB_SPELL_AUTO_SHOT = 75,
    PB_SPELL_SHOOT_WAND = 5019,
    PB_SPELL_HONORLESS_TARGET = 2479,
};

#define PB_UPDATE_INTERVAL 500
#define PB_MIN_FOLLOW_DIST 3.0f
#define PB_MAX_FOLLOW_DIST 6.0f
#define PB_MIN_FOLLOW_ANGLE 0.0f
#define PB_MAX_FOLLOW_ANGLE 6.0f

// Boss Override Registry has been migrated to BotCombatEventBuffer.cpp
// (InitEncounterOverrides) as permanent learned patterns.


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

// Explicit player role assignments set via .partyrole command.
// Maps human player GUID -> role. Checked before heuristics in FindPartyTank().
static std::unordered_map<ObjectGuid, CombatBotRoles> s_playerRoleAssignments;

void PartyBotAI::SetPlayerRole(ObjectGuid guid, CombatBotRoles role)
{
    if (role == ROLE_INVALID)
        s_playerRoleAssignments.erase(guid);
    else
        s_playerRoleAssignments[guid] = role;
}

CombatBotRoles PartyBotAI::GetPlayerRole(ObjectGuid guid)
{
    auto it = s_playerRoleAssignments.find(guid);
    return it != s_playerRoleAssignments.end() ? it->second : ROLE_INVALID;
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

            // Explicit .partyrole assignment takes priority over all heuristics
            auto it = s_playerRoleAssignments.find(pMember->GetObjectGuid());
            if (it != s_playerRoleAssignments.end())
            {
                if (it->second == ROLE_TANK)
                    return pMember;
                else
                    continue; // explicitly assigned a non-tank role — skip heuristics
            }

            // Bot role assignment
            if (pMember->AI())
            {
                if (CombatBotBaseAI* pBotAI = dynamic_cast<CombatBotBaseAI*>(pMember->AI()))
                {
                    if (pBotAI->GetRole() == ROLE_TANK)
                        return pMember;
                }
            }

            // Heuristic fallback for unassigned human players
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
    me->GetMotionMaster()->MovePoint(0, pSafeTarget->GetPositionX(), pSafeTarget->GetPositionY(), pSafeTarget->GetPositionZ(), MOVE_PATHFINDING);
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

    bool const isEating = me->HasAura(PB_SPELL_FOOD);
    bool const isDrinking = me->HasAura(PB_SPELL_DRINK);

    // Hysteresis: start eating/drinking at low thresholds, but once started
    // keep going until nearly full to avoid sit-stand-sit loops.
    bool const needToEat = me->GetHealthPercent() < (isEating ? 90.0f : 60.0f);
    bool const needToDrink = (me->GetPowerType() == POWER_MANA) && (me->GetPowerPercent(POWER_MANA) < (isDrinking ? 90.0f : 40.0f));

    if (!needToEat && !needToDrink)
        return false;

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

bool PartyBotAI::IsInDungeonOrRaid() const
{
    Map* map = me->GetMap();
    return map && map->IsDungeon();
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
                if (((pEnemy->GetLevel() + 5) > me->GetLevel()) && ((pEnemy->GetHealth() * 4) > me->GetHealth()) && pEnemy->GetVictim() && pEnemy->GetVictim() != me && pEnemy->IsValidAttackTarget(me) && pEnemy->CanHaveThreatList())
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

    if (pSpellEntry->HasAuraInterruptFlag(AURA_INTERRUPT_DAMAGE_CANCELS) && AreOthersOnSameTarget(pTarget->GetObjectGuid()))
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

    // Don't initiate attack if stealthed — let the opener behavior handle
    // the first strike. Just set up the chase from behind.
    if (me->HasAuraType(SPELL_AURA_MOD_STEALTH))
    {
        me->SetTargetGuid(pVictim->GetObjectGuid());
        me->GetMotionMaster()->MoveChase(pVictim, 1.0f, 3.0f); // behind
        return true;
    }

    if (me->Attack(pVictim, true))
    {
        // Tank announces pulling (with cooldown)
        if (m_role == ROLE_TANK && m_pullChatCooldown == 0 && !me->IsInCombat())
        {
            if (sBotChatMgr.CanChat(me->GetGUIDLow(), CHAT_COOLDOWN_PARTY))
            {
                static const char* pullMsgs[] = {"pulling", "inc", "going in"};
                sBotChatMgr.SendPartyChat(me, pullMsgs[urand(0, 2)]);
                sBotChatMgr.RecordChat(me->GetGUIDLow(), CHAT_COOLDOWN_PARTY);
                m_pullChatCooldown = 15000; // 15s internal cooldown
            }
        }

        if (GetRole() == ROLE_RANGE_DPS && me->GetPowerPercent(POWER_MANA) > 10.0f && me->GetCombatDistance(pVictim) > 8.0f)
            me->SetCasterChaseDistance(25.0f);
        else if (me->HasDistanceCasterMovement())
            me->SetCasterChaseDistance(0.0f);

        if (GetRole() == ROLE_RANGE_DPS || GetRole() == ROLE_HEALER)
            me->GetMotionMaster()->MoveChase(pVictim, 25.0f);
        else
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

        // Kill fleeing mobs before they pull additional packs.
        if (me->IsInCombat())
        {
            if (Group* pGroup = me->GetGroup())
            {
                for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
                {
                    if (Player* pMember = itr->getSource())
                    {
                        for (const auto pAttacker : pMember->GetAttackers())
                        {
                            if (!IsValidHostileTarget(pAttacker) || !me->IsWithinDist(pAttacker, 50.0f))
                                continue;
                            auto mt = pAttacker->GetMotionMaster()->GetCurrentMovementGeneratorType();
                            if (mt == FLEEING_MOTION_TYPE || mt == TIMED_FLEEING_MOTION_TYPE || mt == ASSISTANCE_MOTION_TYPE)
                                return pAttacker;
                        }
                    }
                }
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
                if (IsValidHostileTarget(pAttacker) && me->IsWithinDist(pAttacker, 50.0f))
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

            if ((pMember->GetHealthPercent() < 90.0f) && !pMember->GetAttackers().empty() && !pMember->IsImmuneToMechanic(MECHANIC_SHIELD))
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
            if (!pTarget->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL) && IsValidHostileTarget(pTarget) && !AreOthersOnSameTarget(pTarget->GetObjectGuid()))
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

Unit* PartyBotAI::SelectAutoCCTarget() const
{
    SpellEntry const* pCC = GetCrowdControlSpell();
    if (!pCC)
        return nullptr;

    Player* pTank = FindPartyTank();
    Unit* pTankTarget = pTank ? pTank->GetVictim() : nullptr;
    // Fallback: don't CC whatever the leader is attacking.
    Player* pLeader = GetPartyLeader();
    Unit* pLeaderTarget = pLeader ? pLeader->GetVictim() : nullptr;
    Group* pGroup = me->GetGroup();
    if (!pGroup)
        return nullptr;

    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        if (Player* pMember = itr->getSource())
        {
            if (!pMember->IsAlive())
                continue;

            for (const auto pAttacker : pMember->GetAttackers())
            {
                if (pAttacker == pTankTarget || pAttacker == pLeaderTarget)
                    continue;
                if (pAttacker->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL))
                    continue;
                if (!IsValidHostileTarget(pAttacker))
                    continue;
                if (!me->IsWithinDist(pAttacker, 30.0f))
                    continue;

                // Skip if already raid-marked (another bot is handling it).
                bool isMarked = false;
                for (uint8 i = 0; i < TARGET_ICON_COUNT; ++i)
                {
                    if (pGroup->GetTargetWithIcon((RaidTargetIcon)i) == pAttacker->GetObjectGuid())
                    {
                        isMarked = true;
                        break;
                    }
                }
                if (isMarked)
                    continue;

                // Check if our CC spell actually works on this target.
                if (!CanTryToCastSpell(pAttacker, pCC))
                    continue;

                return pAttacker;
            }
        }
    }
    return nullptr;
}

bool PartyBotAI::AutoMarkForCC(Unit* pTarget)
{
    Group* pGroup = me->GetGroup();
    if (!pGroup)
        return false;

    // CC mark priority: Moon > Square > Diamond > Triangle.
    // Skull/Cross reserved for kill targets, Star for sap.
    static const RaidTargetIcon ccIcons[] = {
        RAID_TARGET_ICON_MOON,
        RAID_TARGET_ICON_SQUARE,
        RAID_TARGET_ICON_DIAMOND,
        RAID_TARGET_ICON_TRIANGLE,
    };

    for (auto icon : ccIcons)
    {
        if (!pGroup->GetTargetWithIcon(icon))
        {
            pGroup->SetTargetIcon(icon, pTarget->GetObjectGuid());
            m_marksToCC.push_back(icon);
            m_autoMarksToCC.push_back(icon);
            return true;
        }
    }
    return false; // all CC marks in use
}

void PartyBotAI::ClearAutoMarks()
{
    if (m_autoMarksToCC.empty())
        return;

    Group* pGroup = me->GetGroup();
    for (auto mark : m_autoMarksToCC)
    {
        // Remove from manual CC list.
        m_marksToCC.erase(std::remove(m_marksToCC.begin(), m_marksToCC.end(), mark), m_marksToCC.end());
        // Clear the raid icon.
        if (pGroup)
            pGroup->SetTargetIcon(mark, ObjectGuid());
    }
    m_autoMarksToCC.clear();
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
    // printf("Bot received %s\n", LookupOpcodeName(packet->GetOpcode()));
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
                        std::unique_ptr<WorldPacket> beginData = std::make_unique<WorldPacket>(CMSG_BEGIN_TRADE, 0);
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
                // The other player accepted. Schedule accept (timer will re-apply enchant + accept).
                if (me->GetTradeData() && !me->GetTradeData()->IsAccepted())
                {
                    m_pendingTradeAcceptTimer = 500;
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
                // Don't set enchant spell — it causes accept loops.
                // Enchant will be applied directly after trade completes.
                me->GetTradeData()->SetLastModificationTime(0);
                me->GetTradeData()->SetScamPreventionDelay(0);

                WorldPacket acceptData(CMSG_ACCEPT_TRADE, 4);
                acceptData << uint32(1);
                me->GetSession()->HandleAcceptTradeOpcode(acceptData);
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

    // Social chat timer countdowns
    if (m_oomChatCooldown > PB_UPDATE_INTERVAL)
        m_oomChatCooldown -= PB_UPDATE_INTERVAL;
    else
        m_oomChatCooldown = 0;

    if (m_pullChatCooldown > PB_UPDATE_INTERVAL)
        m_pullChatCooldown -= PB_UPDATE_INTERVAL;
    else
        m_pullChatCooldown = 0;

    // Delayed greeting after joining group
    if (m_greetDelayTimer > 0)
    {
        if (m_greetDelayTimer <= PB_UPDATE_INTERVAL)
        {
            m_greetDelayTimer = 0;
            if (me->GetGroup())
                sBotChatMgr.OnGroupMemberJoined(me, me->GetGroup());
        }
        else
            m_greetDelayTimer -= PB_UPDATE_INTERVAL;
    }

    if (!me->IsInWorld() || me->IsBeingTeleported())
        return;

    if (!m_initialized)
    {
        AddToPlayerGroup();

        // Guild bots converted from presence: existing character, needs spec + gear
        if (!m_race && !m_class && sGuildBotMgr.IsEnabled() && sGuildBotMgr.IsGuildBot(me->GetGUIDLow()))
        {
            LearnPremadeSpecForClass();

            if (m_role == ROLE_INVALID)
                AutoAssignRole();

            if (!sGuildBotMgr.HasLootUpgrades(me->GetGUIDLow()))
            {
                if (me->GetLevel() < 19)
                    AutoEquipGear(PLAYER_BOT_AUTO_EQUIP_RANDOM_GEAR);
                else
                {
                    GuildBotInfo const* gbi = sGuildBotMgr.GetBotInfo(me->GetGUIDLow());
                    if (gbi)
                        AutoEquipGear(sGuildBotMgr.GetAutoEquipOptionForTier(gbi->gearTier));
                    else
                        AutoEquipGear(PLAYER_BOT_AUTO_EQUIP_RANDOM_GEAR);
                }
            }
        }
        else if (m_race && m_class) // temporary character
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
                        {
                            // Low-level guild bots (<19) have no premade templates, use random gear
                            if (me->GetLevel() < 19)
                                AutoEquipGear(PLAYER_BOT_AUTO_EQUIP_RANDOM_GEAR);
                            else
                                AutoEquipGear(sGuildBotMgr.GetAutoEquipOptionForTier(gbi->gearTier));
                        }
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

        // Initialize behavior system
        m_kit = std::make_unique<BotSpellKit>(me->GetClass(), this);
        m_combatBehaviors = BuildBehaviorSet(me->GetClass(), m_role, true, this, this);
        m_oocBehaviors = BuildBehaviorSet(me->GetClass(), m_role, false, this, this);
        sBotPatternMgr.SetActive(true);

        AddAllSpellReagents();
        me->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_SPAWNING);
        // Warlock pet summoned from out-of-combat AI (has tank-aware pet selection).
        if (me->GetClass() != CLASS_WARLOCK)
            SummonPetIfNeeded();
        me->SetHealthPercent(100.0f);
        me->SetPowerPercent(me->GetPowerType(), 100.0f);

        uint32 newzone, newarea;
        me->GetZoneAndAreaId(newzone, newarea);
        me->UpdateZone(newzone, newarea);

        m_initialized = true;

        // Initial formation slot assignment.
        if (Group* pGroup = me->GetGroup())
        {
            if (FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid))
                fs->AssignSlots(pGroup);
        }

        // Schedule a greeting after a short random delay (2-5 seconds)
        if (!m_greetedGroup)
        {
            m_greetedGroup = true;
            m_greetDelayTimer = urand(2000, 5000);
        }

        return;
    }

    if (m_resetSpellData)
    {
        ResetSpellData();
        PopulateSpellData();
        m_kit = std::make_unique<BotSpellKit>(me->GetClass(), this);
        m_combatBehaviors = BuildBehaviorSet(me->GetClass(), m_role, true, this, this);
        m_oocBehaviors = BuildBehaviorSet(me->GetClass(), m_role, false, this, this);
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

    // Formation: record leader's trail breadcrumb.
    sBotFormationMgr.UpdateTrail(m_leaderGuid, pLeader, WorldTimer::getMSTime());

    // Formation: reassign slots if group composition changed.
    if (Group* pGroup = me->GetGroup())
    {
        if (FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid))
        {
            if (fs->lastGroupSize != pGroup->GetMembersCount())
                fs->AssignSlots(pGroup);
        }
    }

    if (pLeader->InBattleGround() && !me->InBattleGround())
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

    if (me->HasUnitState(UNIT_STATE_FEIGN_DEATH) && me->HasAuraType(SPELL_AURA_FEIGN_DEATH) && !me->IsInCombat() && (!me->GetPet() || !me->GetPet()->IsInCombat()) && !me->SelectRandomUnfriendlyTarget(nullptr, 20.0f, false, true))
        me->RemoveSpellsCausingAura(SPELL_AURA_FEIGN_DEATH);

    if (me->HasUnitState(UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL))
    {
        BreakCrowdControlEffects();
        return;
    }

    if (me->IsDead())
    {
        // Wipe detection: if all group members are dead, react once
        if (!m_wipeReacted && me->GetGroup())
        {
            bool allDead = true;
            for (GroupReference* itr = me->GetGroup()->GetFirstMember(); itr; itr = itr->next())
            {
                Player* member = itr->getSource();
                if (member && member->IsInWorld() && member->IsAlive())
                {
                    allDead = false;
                    break;
                }
            }
            if (allDead)
            {
                m_wipeReacted = true;
                sBotChatMgr.OnPartyWipe(me->GetGroup());

                // If inside a dungeon/raid, activate wipe respawn logic
                if (IsInDungeonOrRaid())
                {
                    m_dungeonWipeDetected = true;
                    // Stagger delay: 2-3 seconds per bot, based on formation slot
                    int slotIndex = (m_formationSlotIndex >= 0) ? m_formationSlotIndex : urand(0, 4);
                    m_dungeonRespawnDelay = (slotIndex + 1) * urand(2000, 3000);
                }
            }
        }

        if (me->InBattleGround())
        {
            if (me->GetDeathState() == CORPSE)
            {
                me->BuildPlayerRepop();
                me->RepopAtGraveyard();
            }
        }
        else if (m_dungeonWipeDetected)
        {
            // Dungeon wipe path: count down stagger delay, then teleport+revive
            // when leader is alive. Teleport BEFORE revive to avoid mob aggro
            // at the corpse location.
            if (m_dungeonRespawnDelay > PB_UPDATE_INTERVAL)
            {
                m_dungeonRespawnDelay -= PB_UPDATE_INTERVAL;
            }
            else
            {
                m_dungeonRespawnDelay = 0;
                Player* pLeader = GetPartyLeader();
                if (pLeader && pLeader->IsAlive() && pLeader->IsInWorld())
                {
                    // Teleport to leader while still dead (no aggro)
                    if (!me->IsStopped())
                        me->StopMoving();
                    me->GetMotionMaster()->Clear(false, true);
                    me->GetMotionMaster()->MoveIdle();

                    if (FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid))
                        fs->trail.Clear();

                    char name[128] = {};
                    strcpy(name, pLeader->GetName());
                    ChatHandler(me).HandleGonameCommand(name);

                    // Now revive at the leader's location
                    me->ResurrectPlayer(0.5f);
                    me->SpawnCorpseBones();
                    me->CastSpell(me, PB_SPELL_HONORLESS_TARGET, true);

                    // Reset wipe state
                    m_dungeonWipeDetected = false;
                    m_wipeReacted = false;
                }
            }
        }
        else
        {
            // Normal path (outside instances, or partial wipe in dungeon)
            if (ShouldAutoRevive())
            {
                me->ResurrectPlayer(0.5f);
                me->SpawnCorpseBones();
                me->CastSpell(me, PB_SPELL_HONORLESS_TARGET, true);

                // In dungeon: flag for teleport to leader if far away
                if (IsInDungeonOrRaid())
                {
                    Player* pLeader = GetPartyLeader();
                    if (pLeader && pLeader->IsAlive() && !me->IsWithinDistInMap(pLeader, 100.0f))
                        m_needPostReviveTeleport = true;
                }
            }
        }

        return;
    }

    // Post-revive teleport for partial dungeon wipes: bot revived normally
    // but leader is far away at the instance entrance.
    if (m_needPostReviveTeleport)
    {
        m_needPostReviveTeleport = false;
        Player* pLeader = GetPartyLeader();
        if (pLeader && pLeader->IsAlive() && !me->IsWithinDistInMap(pLeader, 100.0f))
        {
            if (!me->IsStopped())
                me->StopMoving();
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();

            if (FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid))
                fs->trail.Clear();

            char name[128] = {};
            strcpy(name, pLeader->GetName());
            ChatHandler(me).HandleGonameCommand(name);
        }
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
        if (pCurrentSpell->getState() == SPELL_STATE_PREPARING && pCurrentSpell->m_spellInfo->IsHealSpell())
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

        // Teleport to leader if too far away (but not if leader is a ghost).
        if (!me->IsWithinDistInMap(pLeader, 100.0f) && !IsInDuel() && pLeader->IsAlive())
        {
            if (!me->IsStopped())
                me->StopMoving();
            me->GetMotionMaster()->Clear(false, true);
            me->GetMotionMaster()->MoveIdle();

            // Clear breadcrumb trail on teleport.
            if (FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid))
                fs->trail.Clear();

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
        // Always re-evaluate target for focus fire (tank assist, marks).
        Unit* pBestTarget = SelectAttackTarget(pLeader);

        if (!pVictim || !IsValidHostileTarget(pVictim))
        {
            if (pVictim)
                me->AttackStop();

            if (pBestTarget)
            {
                AttackStart(pBestTarget);
                return;
            }
        }
        else if (pBestTarget && pBestTarget != pVictim)
        {
            // Switch to higher-priority target (tank's target, marked target).
            me->AttackStop();
            AttackStart(pBestTarget);
            return;
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
                if (me->IsInDisallowedMountForm() && me->GetDisplayId() != me->GetNativeDisplayId() && me->HasAuraType(SPELL_AURA_MOD_SHAPESHIFT))
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

    // Don't override movement if stealthed and approaching a target
    if (me->HasAuraType(SPELL_AURA_MOD_STEALTH) && me->GetTargetGuid())
        return;

    if (!me->IsMoving())
    {
        if (!pVictim && pLeader->IsAlive())
        {
            bool groupInCombat = pLeader->IsInCombat();
            bool isCaster = (m_role == ROLE_HEALER || m_role == ROLE_RANGE_DPS);
            float desiredDist = (groupInCombat && isCaster) ? frand(12.0f, 18.0f) : frand(PB_MIN_FOLLOW_DIST, PB_MAX_FOLLOW_DIST);

            // Re-issue follow if not following, or if combat state changed and
            // we're a caster too close to the leader (was following at melee range)
            bool needFollow = me->GetMotionMaster()->GetCurrentMovementGeneratorType() != FOLLOW_MOTION_TYPE && me->GetMotionMaster()->GetCurrentMovementGeneratorType() != POINT_MOTION_TYPE;
            if (!needFollow && isCaster && groupInCombat && me->GetDistance(pLeader) < 10.0f)
                needFollow = true;

            if (needFollow)
            {
                // Formation-aware follow: use breadcrumb trail + slot positioning.
                FormationState* fs = sBotFormationMgr.GetFormation(m_leaderGuid);
                if (fs && m_formationSlotIndex >= 0 && !fs->trail.IsEmpty())
                {
                    float fx, fy, fz;
                    if (fs->CalculateSlotPosition(m_formationSlotIndex, me->GetMap(), me, fx, fy, fz))
                    {
                        float distToSlot = me->GetDistance(fx, fy, fz);
                        if (distToSlot > BotFormationDefaults::MOVE_DEADZONE)
                        {
                            me->GetMotionMaster()->MovePoint(0, fx, fy, fz, MOVE_PATHFINDING);
                        }
                    }
                    else
                    {
                        // Fallback: old follow behavior.
                        me->GetMotionMaster()->MoveFollow(pLeader, desiredDist, frand(PB_MIN_FOLLOW_ANGLE, PB_MAX_FOLLOW_ANGLE));
                    }
                }
                else
                {
                    // No formation data yet — use old follow.
                    me->GetMotionMaster()->MoveFollow(pLeader, desiredDist, frand(PB_MIN_FOLLOW_ANGLE, PB_MAX_FOLLOW_ANGLE));
                }
            }
        }
        else
        {
            if (!me->HasUnitState(UNIT_STATE_MELEE_ATTACKING) && (GetRole() == ROLE_MELEE_DPS || m_role == ROLE_TANK) && IsValidHostileTarget(pVictim) && AttackStart(pVictim))
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
    // Clear auto-assigned CC marks between pulls.
    ClearAutoMarks();

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

    // Dispel: free pre-step, runs before behavior scoring
    if (CheckForDispelTargets())
        return;

    if (m_kit)
    {
        SituationContext ctx;
        EvaluateSituation(this, ctx);

        // If we have a victim (from AttackStart) but aren't technically in combat
        // yet, use combat behaviors — mirrors the old class-file pattern where
        // UpdateOutOfCombatAI_X() called UpdateInCombatAI_X() when victim present.
        if (me->GetVictim() && m_combatBehaviors)
        {
            // Pet attack: free action before behavior scoring
            if (Pet* pPet = me->GetPet())
            {
                if (!pPet->GetVictim())
                {
                    if (pPet->GetEntry() == 416)
                        pPet->RemoveAurasDueToSpell(4511);
                    pPet->GetCharmInfo()->SetIsCommandAttack(true);
                    pPet->AI()->AttackStart(me->GetVictim());
                }
            }
            m_combatBehaviors->RunBestBehavior(ctx, *m_kit);
        }
        else if (m_oocBehaviors)
        {
            m_oocBehaviors->RunBestBehavior(ctx, *m_kit);
        }
    }
}

void PartyBotAI::UpdateInCombatAI()
{
    m_wipeReacted = false; // reset wipe flag when in combat
    m_dungeonWipeDetected = false;
    m_needPostReviveTeleport = false;

    // === BOSS KILL DETECTION ===
    if (m_lastCombatTarget)
    {
        Unit* lastTarget = sObjectAccessor.GetUnit(*me, m_lastCombatTarget);
        if (lastTarget && lastTarget->IsDead() && lastTarget->IsCreature())
        {
            Creature* pCreature = static_cast<Creature*>(lastTarget);
            bool isBoss = sBotPatternMgr.HasPatternsForCreature(pCreature->GetEntry()) || pCreature->GetCreatureInfo()->rank >= 3;
            if (isBoss && me->GetGroup())
            {
                sBotChatMgr.OnBossKilled(me->GetGroup(), pCreature->GetEntry());
                m_lastCombatTarget.Clear();
            }
        }
    }

    // Track current target for boss kill detection
    if (Unit* pVictim = me->GetVictim())
        m_lastCombatTarget = pVictim->GetObjectGuid();

    // === OOM CALLOUT ===
    if (m_oomChatCooldown == 0 && me->GetPowerType() == POWER_MANA)
    {
        float manaPct = me->GetPowerPercent(POWER_MANA);
        if (manaPct < 10.0f && (m_role == ROLE_HEALER || me->GetClass() == CLASS_MAGE || me->GetClass() == CLASS_WARLOCK || me->GetClass() == CLASS_PRIEST))
        {
            if (sBotChatMgr.CanChat(me->GetGUIDLow(), CHAT_COOLDOWN_PARTY))
            {
                static const char* oomMsgs[] = {"oom", "oom need mana", "low mana", "need to drink"};
                sBotChatMgr.SendPartyChat(me, oomMsgs[urand(0, 3)]);
                sBotChatMgr.RecordChat(me->GetGUIDLow(), CHAT_COOLDOWN_PARTY);
                m_oomChatCooldown = 30000;
            }
        }
    }

    // === OPENER DELAY: Non-tanks wait 2 seconds before engaging ===
    if (m_role != ROLE_TANK && m_role != ROLE_HEALER && !m_combatEngageDelayDone)
    {
        m_combatEngageTimer += PB_UPDATE_INTERVAL;
        if (m_combatEngageTimer < 2000)
            return;
        m_combatEngageDelayDone = true;
    }

    // === DISPEL: free pre-step ===
    if (CheckForDispelTargets())
        return;

    // === PET ATTACK: free action, runs before behavior scoring ===
    if (Unit* pVictim = me->GetVictim())
    {
        if (Pet* pPet = me->GetPet())
        {
            if (!pPet->GetVictim())
            {
                // Strip Phase Shift from Imp and disable its auto-cast so
                // the pet AI doesn't immediately re-apply it.
                if (pPet->GetEntry() == 416) // Imp
                {
                    pPet->RemoveAurasDueToSpell(4511);
                    pPet->GetCharmInfo()->ToggleCreatureAutocast(4511, false);
                }
                pPet->GetCharmInfo()->SetIsCommandAttack(true);
                pPet->AI()->AttackStart(pVictim);
            }
        }
    }

    // === HUNTER AUTO SHOT: free pre-step, keep autorepeat running ===
    if (me->GetClass() == CLASS_HUNTER)
    {
        if (Unit* pVictim = me->GetVictim())
        {
            if (me->HasSpell(PB_SPELL_AUTO_SHOT) && !me->IsMoving() && !me->IsNonMeleeSpellCasted() && !me->GetCurrentSpell(CURRENT_AUTOREPEAT_SPELL))
            {
                switch (me->CastSpell(pVictim, PB_SPELL_AUTO_SHOT, false))
                {
                case SPELL_FAILED_NEED_AMMO:
                case SPELL_FAILED_NO_AMMO:
                    AddHunterAmmo();
                    break;
                }
            }
        }
    }

    // === BEHAVIOR SYSTEM ===
    if (m_combatBehaviors && m_kit)
    {
        SituationContext ctx;
        EvaluateSituation(this, ctx);

        m_combatBehaviors->RunBestBehavior(ctx, *m_kit);
    }

    if (me->GetVictim())
        UseTrinketEffects();
}

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
        // Stealth when leader is in combat (about to get a target)
        if (pLeader->IsInCombat())
            return true;

        if (pLeader->IsDead() || pLeader->IsFeigningDeathSuccessfully() || pLeader->HasAuraType(SPELL_AURA_MOD_STEALTH) || pLeader->HasAuraType(SPELL_AURA_MOD_INVISIBILITY))
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
            if (shouldStealth && CanTryToCastSpell(me, pStealthSpell) && DoCastSpell(me, pStealthSpell) == SPELL_CAST_OK)
                return true;
        }
    }
    return false;
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
            SpellEntry const* pDispelSpell = m_spells.druid.pAbolishPoison ? m_spells.druid.pAbolishPoison : m_spells.druid.pCurePoison;

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
