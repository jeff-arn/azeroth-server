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

#ifndef MANGOS_PARTYBOTAI_H
#define MANGOS_PARTYBOTAI_H

#include <memory>
#include <unordered_map>
#include <vector>
#include "BotBehavior.h"
#include "BotSituationContext.h"
#include "BotSpellKit.h"
#include "CombatBotBaseAI.h"
#include "Group.h"
#include "ObjectAccessor.h"


class PartyBotAI : public CombatBotBaseAI
{
public:
    PartyBotAI(Player* pLeader, Player* pClone, CombatBotRoles role, uint8 race, uint8 class_, uint8 level, uint32 mapId, uint32 instanceId, float x, float y, float z, float o) : CombatBotBaseAI(), m_race(race), m_class(class_), m_level(level), m_mapId(mapId), m_instanceId(instanceId), m_x(x), m_y(y), m_z(z), m_o(o)
    {
        m_role = role;
        m_leaderGuid = pLeader->GetObjectGuid();
        m_cloneGuid = pClone ? pClone->GetObjectGuid() : ObjectGuid();
        m_updateTimer.Reset(2000);
    }
    PartyBotAI(Player* pLeader, uint32 mapId, uint32 instanceId, float x, float y, float z, float o) : CombatBotBaseAI(), m_mapId(mapId), m_instanceId(instanceId), m_x(x), m_y(y), m_z(z), m_o(o)
    {
        m_role = ROLE_INVALID;
        m_leaderGuid = pLeader->GetObjectGuid();
        m_updateTimer.Reset(2000);
    }

    // Explicit role assignments for human players, set via .partyrole command.
    static void SetPlayerRole(ObjectGuid guid, CombatBotRoles role);
    static CombatBotRoles GetPlayerRole(ObjectGuid guid);

    bool OnSessionLoaded(PlayerBotEntry* entry, WorldSession* sess) final;
    void OnPlayerLogin() final;
    void UpdateAI(uint32 const diff) final;
    void OnPacketReceived(WorldPacket const* packet) final;

    void CloneFromPlayer(Player const* pPlayer);
    void AddToPlayerGroup();

    bool CanTryToCastSpell(Unit const* pTarget, SpellEntry const* pSpellEntry) const final;
    Player* GetPartyLeader() const;
    bool AttackStart(Unit* pVictim);
    Unit* SelectAttackTarget(Player* pLeader) const;
    Unit* SelectPartyAttackTarget() const;
    Player* SelectResurrectionTarget() const;
    Player* SelectShieldTarget() const;
    Unit* GetMarkedTarget(RaidTargetIcon mark) const;
    bool CanUseCrowdControl(SpellEntry const* pSpellEntry, Unit* pTarget) const;
    bool DrinkAndEat();
    bool ShouldAutoRevive() const;
    bool IsInDungeonOrRaid() const;
    bool RunAwayFromTarget(Unit* pEnemy);
    Player* FindPartyTank() const;
    float GetThreatRatio(Unit* pEnemy) const;
    bool ShouldThrottleDPS(Unit* pEnemy) const;
    bool CrowdControlMarkedTargets();
    Unit* SelectAutoCCTarget() const;
    bool AutoMarkForCC(Unit* pTarget);
    void ClearAutoMarks();
    bool EnterCombatDruidForm();
    bool ShouldEnterStealth() const;
    bool EnterStealthIfNeeded(SpellEntry const* pStealthSpell);

    bool CheckForDispelTargets();
    void UpdateInCombatAI() final;
    void UpdateOutOfCombatAI() final;

    // Behavior system members (initialized after PopulateSpellData)
    std::unique_ptr<BotBehaviorSet> m_combatBehaviors;
    std::unique_ptr<BotBehaviorSet> m_oocBehaviors;
    std::unique_ptr<BotSpellKit> m_kit;

    std::vector<RaidTargetIcon> m_marksToCC;
    std::vector<RaidTargetIcon> m_marksToFocus;
    std::vector<RaidTargetIcon> m_autoMarksToCC; // auto-assigned CC marks, cleared between pulls
    ShortTimeTracker m_updateTimer;
    ObjectGuid m_leaderGuid;
    ObjectGuid m_cloneGuid;
    uint8 m_race = 0;
    uint8 m_class = 0;
    uint8 m_level = 0;
    uint32 m_mapId = 0;
    uint32 m_instanceId = 0;
    float m_x = 0.0f;
    float m_y = 0.0f;
    float m_z = 0.0f;
    float m_o = 0.0f;
    bool m_resetSpellData = false;
    uint32 m_combatEngageTimer = 0;
    bool m_combatEngageDelayDone = false;

    // Trade service support (portal tips, etc.)
    uint32 m_pendingTradeAcceptTimer = 0;

    // Social chat cooldowns
    bool m_greetedGroup = false;
    uint32 m_greetDelayTimer = 0; // countdown ms before greeting
    uint32 m_oomChatCooldown = 0; // ms until OOM chat allowed again
    uint32 m_pullChatCooldown = 0; // ms until pull announcement allowed
    ObjectGuid m_lastCombatTarget; // track target death for boss kill detection
    bool m_wipeReacted = false; // prevent spamming wipe reaction

    // Dungeon wipe respawn system
    bool m_dungeonWipeDetected = false; // full wipe detected inside dungeon/raid
    uint32 m_dungeonRespawnDelay = 0; // ms countdown before revive+teleport after wipe
    bool m_needPostReviveTeleport = false; // partial wipe: teleport to leader after reviving

    // Formation system
    int8 m_formationSlotIndex = -1; // assigned by FormationState::AssignSlots
};

#endif
