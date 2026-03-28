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

#include "PlayerBotMgr.h"
#include "PlayerBotAI.h"
#include "BotDirector.h"
#include "Player.h"
#include "Log.h"
#include "SocialMgr.h"
#include "MotionMaster.h"
#include "ObjectMgr.h"
#include "MapManager.h"
#include "MoveSpline.h"
#include "Opcodes.h"
#include "WorldPacket.h"

bool PlayerBotAI::OnSessionLoaded(PlayerBotEntry* entry, WorldSession* sess)
{
    sess->LoginPlayer(entry->playerGUID);
    return true;
}

void PlayerBotAI::UpdateAI(uint32 const diff)
{
    if (me->IsBeingTeleportedNear())
    {
        WorldPacket data(MSG_MOVE_TELEPORT_ACK, 10);
        data << me->GetObjectGuid();
#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_9_4
        data << uint32(0) << uint32(0);
#else
        data << uint32(0);
#endif
        me->GetSession()->HandleMoveTeleportAckOpcode(data);
    }
    if (me->IsBeingTeleportedFar())
        me->GetSession()->HandleMoveWorldportAckOpcode();
}

void PlayerBotAI::Remove()
{
    if (me)
    {
        if (me->AI() == this)
            me->SetAI(nullptr);
        me = nullptr;
    }
}

void PlayerBotFleeingAI::OnPlayerLogin()
{
    me->GetMotionMaster()->MoveFleeing(me);
    me->SetCheatGod(true);
}

// MageOrgrimmarAttackerAI event
enum
{
    SPELL_FROST_NOVA = 122,
    SPELL_FIREBOLT = 133,
    AURA_REGEN_MANA = 430,
};


bool PlayerBotAI::SpawnNewPlayer(WorldSession* sess, uint8 class_, uint32 race_, uint32 mapId, uint32 instanceId, float x, float y, float z, float o, Player* pClone)
{
    ASSERT(botEntry);
    std::string name = sObjectMgr.GenerateFreePlayerName();
    normalizePlayerName(name);

    uint8 gender;
    uint8 skin;
    uint8 face;
    uint8 hairStyle;
    uint8 hairColor;
    uint8 facialHair;

    if (pClone)
    {
        gender = pClone->GetByteValue(UNIT_FIELD_BYTES_0, UNIT_BYTES_0_OFFSET_GENDER);
        skin = pClone->GetByteValue(PLAYER_BYTES, PLAYER_BYTES_OFFSET_SKIN_ID);
        face = pClone->GetByteValue(PLAYER_BYTES, PLAYER_BYTES_OFFSET_FACE_ID);
        hairStyle = pClone->GetByteValue(PLAYER_BYTES, PLAYER_BYTES_OFFSET_HAIR_STYLE_ID);
        hairColor = pClone->GetByteValue(PLAYER_BYTES, PLAYER_BYTES_OFFSET_HAIR_COLOR_ID);
        facialHair = pClone->GetByteValue(PLAYER_BYTES_2, PLAYER_BYTES_2_OFFSET_FACIAL_STYLE);
    }
    else
    {
        gender = urand(0, 1);
        Player::SelectRandomAppearance(race_, gender, hairStyle, hairColor, face, facialHair, skin);
    }

    Player* newChar = new Player(sess);
    uint32 guid = botEntry->playerGUID;
    if (!newChar->Create(guid, name, race_, class_, gender, skin, face, hairStyle, hairColor, facialHair))
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "PlayerBotAI::SpawnNewPlayer: Unable to create a player!");
        delete newChar;
        return false;
    }
    newChar->SetLocationMapId(mapId);
    newChar->SetLocationInstanceId(instanceId);
    newChar->SetAutoInstanceSwitch(false);
    newChar->GetMotionMaster()->Initialize();
    // Set instance
    if (instanceId && mapId > 1) // Not a continent
    {
        DungeonPersistentState* state = (DungeonPersistentState*)sMapPersistentStateMgr
                .AddPersistentState(sMapStorage.LookupEntry<MapEntry>(mapId), instanceId, time(nullptr) + 3600, false, true);
        newChar->BindToInstance(state, true, true);
    }
    // Generate position
    Map* map = sMapMgr.FindMap(mapId, instanceId);
    if (!map)
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "PlayerBotAI::SpawnNewPlayer: Map (%u, %u) not found!", mapId, instanceId);
        delete newChar;
        return false;
    }
    newChar->Relocate(x, y, z, o);
    sObjectMgr.InsertPlayerInCache(newChar);
    newChar->SetMap(map);
    newChar->SaveRecallPosition();
    newChar->CreatePacketBroadcaster();
    MasterPlayer* mPlayer = new MasterPlayer(sess);
    mPlayer->LoadPlayer(newChar);
    mPlayer->SetSocial(sSocialMgr.LoadFromDB(nullptr, newChar->GetObjectGuid()));
    if (!newChar->GetMap()->Add(newChar))
    {
        sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "PlayerBotAI::SpawnNewPlayer: Unable to add player to map!");
        delete newChar;
        return false;
    }
    sess->SetPlayer(newChar);
    sess->SetMasterPlayer(mPlayer);
    sObjectAccessor.AddObject(newChar);
    newChar->SetCanModifyStats(true);
    newChar->UpdateAllStats();
    return true;
}
bool MageOrgrimmarAttackerAI::OnSessionLoaded(PlayerBotEntry* entry, WorldSession* sess)
{
    return SpawnNewPlayer(sess, CLASS_MAGE, RACE_GNOME, 1, 0, 1017.0f, -4450, 12, 0.65f);
}

void MageOrgrimmarAttackerAI::UpdateAI(uint32 const diff)
{
    PlayerBotAI::UpdateAI(diff);
    if (me->GetLevel() != 60)
        me->GiveLevel(60);
    // DEATH
    if (!me->IsAlive())
    {
        sPlayerBotMgr.DeleteBot(me->GetGUIDLow());
        /*
        if (me->GetDeathState() < CORPSE)
            return;
        if (me->GetDeathState() == CORPSE && me->GetDeathTimer() && me->GetDeathTimer() < (6 * MINUTE * IN_MILLISECONDS - 30000))
        {
            me->SetHealth(1);
            me->RepopAtGraveyard();
        }
        else if (me->GetDeathState() == CORPSE && !me->GetDeathTimer())
        {
            me->ResurrectPlayer(0.5f);
            me->SpawnCorpseBones();
        }
        */
        return;
    }
    // COMBAT AI
    if (me->IsNonMeleeSpellCasted(false) || (me->HasAura(AURA_REGEN_MANA) && me->GetPower(POWER_MANA) != me->GetMaxPower(POWER_MANA)))
        return;
    float range = me->IsInCombat() ? 30.0f : frand(15, 30);
    Unit* target = me->SelectNearestTarget(range);
    if (target && !me->IsWithinLOSInMap(target))
        target = nullptr;
    // OOM ?
    if (me->GetPower(POWER_MANA) < 40 && target && me->IsInCombat())
    {
        if (me->Attack(target, true))
            me->GetMotionMaster()->MoveChase(target);
        return;
    }
    // Stop chase if has mana
    if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == CHASE_MOTION_TYPE)
        me->GetMotionMaster()->MovementExpired();
    bool nearTarget = target && target->CanReachWithMeleeAutoAttack(me);
    if (me->IsSpellReady(SPELL_FROST_NOVA) && me->GetPower(POWER_MANA) > 50)
        if (nearTarget)
            me->CastSpell(me, SPELL_FROST_NOVA, false);
    if (nearTarget && target->HasUnitState(UNIT_STATE_CAN_NOT_MOVE))
    {
        // already runing
        if (!me->movespline->Finalized())
            return;
        // Try to kit
        float x, y, z;
        me->GetPosition(x, y, z);
        float d = me->GetDistance(target);
        d += me->GetObjectBoundingRadius();
        d += target->GetObjectBoundingRadius();
        x += (x - target->GetPositionX()) * 5.0f / d;
        y += (y - target->GetPositionY()) * 5.0f / d;
        me->UpdateGroundPositionZ(x, y, z);
        me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
        return;
    }

    if (target && me->GetPower(POWER_MANA) > 50)
    {
        uint32 spellId = SPELL_FIREBOLT;
        me->SetFacingToObject(target);
        if (!me->movespline->Finalized())
            me->StopMoving();

        /*float z = me->GetPositionZ();
        me->UpdateGroundPositionZ(me->GetPositionX(), me->GetPositionY(), z);
        me->Relocate(me->GetPositionX(), me->GetPositionY(), z);
        me->m_movementInfo.moveFlags = 0;
        me->SendHeartBeat();*/

        me->CastSpell(target, spellId, false);
        return;
    }
    // OUT OF COMBAT REGEN
    if (!me->IsInCombat() && me->GetPower(POWER_MANA) < 150)
    {
        if (!me->movespline->Finalized())
            me->StopMoving();
        me->CastSpell(target, AURA_REGEN_MANA, false);
        return;
    }
    // MOVEMENT AI
    float x, y, z = 0; // Where to go
    float r = 10;
    if (me->movespline->Finalized())
    {
        if (me->GetPositionX() < 1000.0f)
        {
            x = 1176;
            y = -4404;
        }
        else if (me->GetPositionX() + 10 < 1176.0f)
        {
            x = 1176;
            y = -4404;
        }
        else if (me->GetPositionX() + 10 < 1357.0f)
        {
            switch (urand(0, 2))
            {
                case 0:
                    x = 1357;
                    y = -4376;
                    break;
                case 1:
                    x = 1354;
                    y = -4412;
                    break;
                case 2:
                    x = 1346;
                    y = -4339;
                    break;
            }
        }
        else if (me->GetPositionX() + 10 < 1421.0f)
        {
            // Porte orgri
            x = 1427;
            y = -4362;
            z = 25.0f;
            r = 4;
        }
        else
        {
            switch (urand(0, 2))
            {
                case 0:
                    x = 1516;
                    y = -4410;
                    z = 17.0f;
                    r = 4;
                    break;
                case 1:
                    x = 1538;
                    y = -4347;
                    z = 18;
                    r = 3;
                    break;
                case 2:
                    x = 1617;
                    y = -4426;
                    z = 12;
                    r = 4;
                    break;
            }
        }
        if (!z)
        {
            z = me->GetPositionZ();
            me->UpdateGroundPositionZ(x, y, z);
        }
        r = 20;
        if (!me->GetMap()->GetWalkRandomPosition(nullptr, x, y, z, r))
            return;
    }
    else
    {
        return;
        if (urand(0, 20) == 0) // random move
        {
            me->GetPosition(x, y, z);
            r = frand(0, 2);
            float angle = me->GetOrientation() + frand(-M_PI_F / 2, M_PI_F / 2);
            x += r * cos(angle);
            y += r * sin(angle);
            if (!me->GetMap()->GetWalkHitPosition(nullptr, me->GetPositionX(), me->GetPositionY(), me->GetPositionZ(), x, y, z))
                return;
        }
        else
            return;
    }
    me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
}

void PopulateAreaBotAI::BeforeAddToMap(Player* player)
{
    if (player->GetInstanceId() || player->GetTeam() != _team)
        return;
    if (player->GetMapId() != _map || !player->IsWithinDist3d(_x, _y, _z, _radius * 2))
    {
        float x = _x;
        float y = _y;
        float z = _z;
        Map* map = sMapMgr.CreateMap(_map, player);
        while (!map->GetWalkRandomPosition(nullptr, x, y, z, _radius));
        player->Relocate(x, y, z);
        player->SetLocationMapId(_map);
    }
}

void PopulateAreaBotAI::OnPlayerLogin()
{
    if (urand(0, 1))
        me->GetMotionMaster()->MoveConfused();
}


// ==================== DynamicBotAI ====================

void DynamicBotAI::LoadAssignment()
{
    if (m_loaded) return;
    m_loaded = true;

    uint32 guid = botEntry ? botEntry->playerGUID : 0;
    if (!guid) return;

    BotAssignment const* assignment = sBotDirector.GetAssignment(guid);
    if (!assignment) return;

    _map = assignment->targetMap;
    _x = assignment->targetX;
    _y = assignment->targetY;
    _z = assignment->targetZ;
    _radius = assignment->radius;
    uint32 targetLevel = assignment->targetLevel;
    _team = assignment->faction;
    _aggressive = assignment->aggressive;

    if (me)
    {
        if (targetLevel > 0 && me->GetLevel() != targetLevel)
        {
            me->GiveLevel(targetLevel);
            me->InitTalentForLevel();
            me->SetUInt32Value(PLAYER_XP, 0);
        }
        // Equip premade gear template based on class and level
        if (!m_gearEquipped)
        {
            m_gearEquipped = true;

            // Unequip any existing gear first
            for (int i = EQUIPMENT_SLOT_START; i < EQUIPMENT_SLOT_END; ++i)
                me->AutoUnequipItemFromSlot(i);

            // Find best matching premade gear template for this class/level
            // Templates exist at levels 19, 29, 39, 49, 60
            uint32 bestEntry = 0;
            uint32 bestLevel = 0;
            for (auto const& itr : sObjectMgr.GetPlayerPremadeGearTemplates())
            {
                if (itr.second.requiredClass == me->GetClass() &&
                    itr.second.level <= me->GetLevel() &&
                    itr.second.level > bestLevel)
                {
                    bestLevel = itr.second.level;
                    bestEntry = itr.second.entry;
                }
            }
            if (bestEntry)
            {
                sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[DynamicBot] %s (lvl %u, class %u) equipping premade template %u (template lvl %u)",
                    me->GetName(), me->GetLevel(), me->GetClass(), bestEntry, bestLevel);
                sObjectMgr.ApplyPremadeGearTemplateToPlayer(bestEntry, me);
            }
            else
            {
                sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[DynamicBot] %s (lvl %u, class %u) no premade template found, using starter items",
                    me->GetName(), me->GetLevel(), me->GetClass());
                me->AddStartingItems();  // Fallback for very low level bots
            }
        }
    }
}

void DynamicBotAI::OnBotEntryLoad(PlayerBotEntry* entry)
{
    // Load assignment data early so we know the team
}

void DynamicBotAI::BeforeAddToMap(Player* player)
{
    uint32 guid = botEntry ? botEntry->playerGUID : 0;
    if (!guid) return;

    BotAssignment const* assignment = sBotDirector.GetAssignment(guid);
    if (!assignment) return;

    uint32 targetMap = assignment->targetMap;
    float tx = assignment->targetX;
    float ty = assignment->targetY;
    float tz = assignment->targetZ;
    float tRadius = assignment->radius;

    // Relocate to target area with some randomness
    if (player->GetMapId() != targetMap || !player->IsWithinDist3d(tx, ty, tz, tRadius * 2))
    {
        float x = tx;
        float y = ty;
        float z = tz;
        Map* map = sMapMgr.CreateMap(targetMap, player);
        if (map)
        {
            x += frand(-tRadius * 0.5f, tRadius * 0.5f);
            y += frand(-tRadius * 0.5f, tRadius * 0.5f);
            map->GetWalkRandomPosition(nullptr, x, y, z, tRadius);
        }
        player->Relocate(x, y, z);
        player->SetLocationMapId(targetMap);
    }
}

void DynamicBotAI::OnPlayerLogin()
{
    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[DynamicBot] OnPlayerLogin called for guid %u",
        botEntry ? botEntry->playerGUID : 0);
    LoadAssignment();

    // Set level from assignment
    uint32 guid = botEntry ? botEntry->playerGUID : 0;
    if (guid)
    {
        BotAssignment const* assignment = sBotDirector.GetAssignment(guid);
        if (assignment && assignment->targetLevel > 0 && me->GetLevel() != assignment->targetLevel)
            me->GiveLevel(assignment->targetLevel);
    }
}

Unit* DynamicBotAI::FindEnemyPlayer(float range)
{
    Map::PlayerList const& players = me->GetMap()->GetPlayers();
    Unit* closest = nullptr;
    float closestDist = range;

    for (auto const& ref : players)
    {
        Player* player = ref.getSource();
        if (!player || !player->IsAlive() || player == me)
            continue;
        if (player->GetTeam() == me->GetTeam())
            continue;
        if (player->IsGameMaster())
            continue;
        // Don't attack other bots
        if (player->IsBot())
            continue;
        float dist = me->GetDistance(player);
        if (dist < closestDist && me->IsWithinLOSInMap(player))
        {
            closest = player;
            closestDist = dist;
        }
    }
    return closest;
}

void DynamicBotAI::DoMeleeCombat(Unit* target)
{
    if (!me->IsInCombat())
    {
        me->Attack(target, true);
        me->GetMotionMaster()->MoveChase(target);
        return;
    }
    if (!me->CanReachWithMeleeAutoAttack(target))
        me->GetMotionMaster()->MoveChase(target);
}

void DynamicBotAI::DoCasterCombat(Unit* target)
{
    uint32 spellId = 0;
    uint8 cls = me->GetClass();

    if (cls == CLASS_MAGE) spellId = 133;
    else if (cls == CLASS_WARLOCK) spellId = 686;
    else if (cls == CLASS_PRIEST) spellId = 585;
    else if (cls == CLASS_SHAMAN) spellId = 403;
    else if (cls == CLASS_HUNTER)
    {
        if (!me->IsInCombat()) me->Attack(target, false);
        if (me->GetDistance(target) < 8.0f)
        {
            float x = me->GetPositionX();
            float y = me->GetPositionY();
            float z = me->GetPositionZ();
            float d = me->GetDistance(target);
            if (d > 0)
            {
                x += (x - target->GetPositionX()) * 10.0f / d;
                y += (y - target->GetPositionY()) * 10.0f / d;
                me->UpdateGroundPositionZ(x, y, z);
                me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
            }
        }
        return;
    }

    if (spellId && !me->IsNonMeleeSpellCasted(false))
    {
        if (!me->IsInCombat()) me->Attack(target, false);
        me->SetFacingToObject(target);
        if (!me->movespline->Finalized()) me->StopMoving();
        me->CastSpell(target, spellId, false);
    }
}

void DynamicBotAI::UpdateAI(uint32 const diff)
{
    PlayerBotAI::UpdateAI(diff);

    // One-time init log
    if (!m_initLogged && me)
    {
        m_initLogged = true;
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[DynamicBot] UpdateAI first call for %s (lvl %u, class %u, loaded=%u, geared=%u)",
            me->GetName(), me->GetLevel(), me->GetClass(), m_loaded ? 1 : 0, m_gearEquipped ? 1 : 0);
    }

    m_updateTimer += diff;
    if (m_updateTimer < 1000)
        return;
    m_updateTimer = 0;

    if (!me || !me->IsAlive())
    {
        if (me && me->GetDeathState() == CORPSE)
        {
            m_regenTimer += 1000;
            if (m_regenTimer > 30000)
            {
                me->ResurrectPlayer(1.0f);
                me->SpawnCorpseBones();
                m_regenTimer = 0;
            }
        }
        return;
    }

    // Reload assignment periodically (every ~2 min) to pick up director changes
    m_reloadTimer += 1000;
    if (m_reloadTimer > 120000)
    {
        m_loaded = false;
        m_gearEquipped = false;
        LoadAssignment();
        m_reloadTimer = 0;
    m_initLogged = false;
    }

    // Out of combat regen
    if (!me->IsInCombat())
    {
        if (me->GetHealth() < me->GetMaxHealth())
            if (me->GetHealth() < me->GetMaxHealth()) me->SetHealth(me->GetHealth() + me->GetMaxHealth() / 20);
        if (me->GetPower(POWER_MANA) < me->GetMaxPower(POWER_MANA))
            if (me->GetPower(POWER_MANA) < me->GetMaxPower(POWER_MANA)) me->SetPower(POWER_MANA, me->GetPower(POWER_MANA) + me->GetMaxPower(POWER_MANA) / 20);
    }

    // If aggressive, look for enemies
    if (_aggressive)
    {
        if (me->IsInCombat() && me->GetVictim() && me->GetVictim()->IsAlive())
        {
            uint8 cls = me->GetClass();
            bool isMelee = (cls == CLASS_WARRIOR || cls == CLASS_ROGUE);
            if (isMelee) DoMeleeCombat(me->GetVictim());
            else DoCasterCombat(me->GetVictim());
            return;
        }

        Unit* target = FindEnemyPlayer(30.0f);
        if (target)
        {
            uint8 cls = me->GetClass();
            bool isMelee = (cls == CLASS_WARRIOR || cls == CLASS_ROGUE);
            if (isMelee) DoMeleeCombat(target);
            else DoCasterCombat(target);
            return;
        }
    }

    // No target - wander (only if we have a valid assignment with real coordinates)
    if (_x != 0.0f && _y != 0.0f && _radius > 0.0f &&
        me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE)
    {
        float x = _x + frand(-_radius, _radius);
        float y = _y + frand(-_radius, _radius);
        float z = me->GetPositionZ();
        me->UpdateGroundPositionZ(x, y, z);
        me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
    }
}


// ==================== AggressiveHordeBotAI ====================

void AggressiveHordeBotAI::OnPlayerLogin()
{
    // Don't do confused movement like PopulateAreaBotAI — we want purposeful roaming
    // Just wander the area normally
}

Unit* AggressiveHordeBotAI::FindEnemyPlayer(float range)
{
    // Find nearest enemy faction player (not bots, not same faction)
    Map::PlayerList const& players = me->GetMap()->GetPlayers();
    Unit* closest = nullptr;
    float closestDist = range;

    for (auto const& ref : players)
    {
        Player* player = ref.getSource();
        if (!player || !player->IsAlive() || player == me)
            continue;
        // Only attack opposite faction real players
        if (player->GetTeam() == me->GetTeam())
            continue;
        // Skip GMs that are invisible
        if (player->IsGameMaster())
            continue;
        float dist = me->GetDistance(player);
        if (dist < closestDist && me->IsWithinLOSInMap(player))
        {
            closest = player;
            closestDist = dist;
        }
    }
    return closest;
}

void AggressiveHordeBotAI::DoMeleeCombat(Unit* target)
{
    if (!me->IsInCombat())
    {
        me->Attack(target, true);
        me->GetMotionMaster()->MoveChase(target);
        return;
    }

    if (!me->CanReachWithMeleeAutoAttack(target))
    {
        me->GetMotionMaster()->MoveChase(target);
    }
}

void AggressiveHordeBotAI::DoCasterCombat(Unit* target)
{
    // Basic caster spells by class
    uint32 spellId = 0;
    uint8 cls = me->GetClass();

    if (cls == CLASS_MAGE)
        spellId = 133;  // Fireball
    else if (cls == CLASS_WARLOCK)
        spellId = 686;  // Shadow Bolt
    else if (cls == CLASS_PRIEST)
        spellId = 585;  // Smite
    else if (cls == CLASS_SHAMAN)
        spellId = 403;  // Lightning Bolt
    else if (cls == CLASS_HUNTER)
    {
        // Hunters use auto shot at range
        if (!me->IsInCombat())
            me->Attack(target, false);
        if (me->GetDistance(target) < 8.0f)
        {
            // Too close, back up
            float x = me->GetPositionX();
            float y = me->GetPositionY();
            float z = me->GetPositionZ();
            float d = me->GetDistance(target);
            if (d > 0)
            {
                x += (x - target->GetPositionX()) * 10.0f / d;
                y += (y - target->GetPositionY()) * 10.0f / d;
                me->UpdateGroundPositionZ(x, y, z);
                me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
            }
        }
        return;
    }

    if (spellId && !me->IsNonMeleeSpellCasted(false))
    {
        if (!me->IsInCombat())
            me->Attack(target, false);
        me->SetFacingToObject(target);
        if (!me->movespline->Finalized())
            me->StopMoving();
        me->CastSpell(target, spellId, false);
    }
}

void AggressiveHordeBotAI::UpdateAI(uint32 const diff)
{
    PlayerBotAI::UpdateAI(diff);

    m_updateTimer += diff;
    if (m_updateTimer < 1000)
        return;
    m_updateTimer = 0;

    if (!me || !me->IsAlive())
    {
        // Auto-resurrect after 30 seconds
        if (me && me->GetDeathState() == CORPSE)
        {
            m_regenTimer += 1000;
            if (m_regenTimer > 30000)
            {
                me->ResurrectPlayer(1.0f);
                me->SpawnCorpseBones();
                m_regenTimer = 0;
            }
        }
        return;
    }

    // Out of combat regen
    if (!me->IsInCombat())
    {
        if (me->GetHealth() < me->GetMaxHealth())
            if (me->GetHealth() < me->GetMaxHealth()) me->SetHealth(me->GetHealth() + me->GetMaxHealth() / 20);
        if (me->GetPower(POWER_MANA) < me->GetMaxPower(POWER_MANA))
            if (me->GetPower(POWER_MANA) < me->GetMaxPower(POWER_MANA)) me->SetPower(POWER_MANA, me->GetPower(POWER_MANA) + me->GetMaxPower(POWER_MANA) / 20);
    }

    // Already in combat — keep fighting current target
    if (me->IsInCombat() && me->GetVictim() && me->GetVictim()->IsAlive())
    {
        uint8 cls = me->GetClass();
        bool isMelee = (cls == CLASS_WARRIOR || cls == CLASS_ROGUE);

        if (isMelee)
            DoMeleeCombat(me->GetVictim());
        else
            DoCasterCombat(me->GetVictim());
        return;
    }

    // Look for enemy players within aggro range
    float aggroRange = 30.0f;
    Unit* target = FindEnemyPlayer(aggroRange);

    if (target)
    {
        uint8 cls = me->GetClass();
        bool isMelee = (cls == CLASS_WARRIOR || cls == CLASS_ROGUE);

        if (isMelee)
            DoMeleeCombat(target);
        else
            DoCasterCombat(target);
        return;
    }

    // No target — wander around the zone
    if (me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE)
    {
        float x = _x + frand(-_radius, _radius);
        float y = _y + frand(-_radius, _radius);
        float z = me->GetPositionZ();
        me->UpdateGroundPositionZ(x, y, z);
        me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
    }
}

PlayerBotAI* CreatePlayerBotAI(std::string ainame)
{
    if (ainame == "MageOrgrimmarAttackerAI")
        return new MageOrgrimmarAttackerAI();
    if (ainame == "IronforgePopulationAI")
        return new PopulateAreaBotAI(0, -4928.5f, -946.6f, 501.6f, ALLIANCE, 100.0f);
    if (ainame == "StormwindPopulationAI")
        return new PopulateAreaBotAI(0, -8829.5f, 625.6f, 93.9f, ALLIANCE, 50.0f);
    if (ainame == "OrgrimmarPopulationAI")
        return new PopulateAreaBotAI(1, 1568, -4405.87f, 8.13f, HORDE, 150.0f);
    if (ainame == "PlayerBotFleeingAI")
        return new PlayerBotFleeingAI();
    if (ainame == "STVHordeBotAI")
        return new AggressiveHordeBotAI(0, -12412.8f, 171.2f, 3.4f, HORDE, 200.0f);
    if (ainame == "HillsbradHordeBotAI")
        return new AggressiveHordeBotAI(0, -40.7f, -957.0f, 56.3f, HORDE, 150.0f);
    if (ainame == "AshenvaleHordeBotAI")
        return new AggressiveHordeBotAI(0, 949.9f, -1435.7f, 62.7f, HORDE, 200.0f);
    if (ainame == "TanarisHordeBotAI")
        return new AggressiveHordeBotAI(1, -7193.4f, -3793.5f, 9.8f, HORDE, 150.0f);
    if (ainame == "STVHordeBotAI")
        return new AggressiveHordeBotAI(0, -12412.8f, 171.2f, 3.4f, HORDE, 200.0f);
    if (ainame == "HillsbradHordeBotAI")
        return new AggressiveHordeBotAI(0, -40.7f, -957.0f, 56.3f, HORDE, 150.0f);
    if (ainame == "AshenvaleHordeBotAI")
        return new AggressiveHordeBotAI(0, 949.9f, -1435.7f, 62.7f, HORDE, 200.0f);
    if (ainame == "TanarisHordeBotAI")
        return new AggressiveHordeBotAI(1, -7193.4f, -3793.5f, 9.8f, HORDE, 150.0f);
    if (ainame == "DynamicBotAI")
        return new DynamicBotAI();
    return new PlayerBotAI();
}
