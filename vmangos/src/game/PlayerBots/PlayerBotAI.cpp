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
#include "BotChatManager.h"
#include "BotServiceMgr.h"
#include "GuildBotMgr.h"
#include "BotDirector.h"
#include "Opcodes.h"
#include "TradeData.h"
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


// ==================== CrowdBotAI ====================

// Spawn location for crowd bots — either a precise spot or an inn center point
struct CrowdBotSpawn
{
    float x, y, z;
    float radius;
    uint32 mapId;
};

// City hotspot spawns: precise positions for dense clusters
// Each entry is one bot's exact position (with small random offset from radius)

// City crowd bot spawn points
static const CrowdBotSpawn CITY_SPAWNS[] =
{
    // IF Bank front — main Alliance gathering area (GUIDs 100-124, 25 bots)
    // Within 10 yards of center (-4919.8, -958.6, 501.5)
    { -4919.8f, -958.6f, 501.5f, 2.0f, 0 },
    { -4915.0f, -954.0f, 501.5f, 2.0f, 0 },
    { -4924.5f, -963.0f, 501.5f, 2.0f, 0 },
    { -4913.5f, -960.0f, 501.5f, 2.0f, 0 },
    { -4926.0f, -957.0f, 501.5f, 2.0f, 0 },
    { -4916.0f, -951.0f, 501.5f, 2.0f, 0 },
    { -4923.5f, -966.0f, 501.5f, 2.0f, 0 },
    { -4912.0f, -957.0f, 501.5f, 2.0f, 0 },
    { -4927.5f, -960.0f, 501.5f, 2.0f, 0 },
    { -4919.8f, -968.0f, 501.5f, 2.0f, 0 },
    { -4919.8f, -949.0f, 501.5f, 2.0f, 0 },
    { -4925.0f, -953.0f, 501.5f, 2.0f, 0 },
    { -4914.5f, -964.0f, 501.5f, 2.0f, 0 },
    { -4927.0f, -963.0f, 501.5f, 2.0f, 0 },
    { -4912.5f, -954.0f, 501.5f, 2.0f, 0 },
    { -4923.0f, -951.5f, 501.5f, 2.0f, 0 },
    { -4916.5f, -965.5f, 501.5f, 2.0f, 0 },
    { -4926.5f, -956.0f, 501.5f, 2.0f, 0 },
    { -4913.0f, -961.5f, 501.5f, 2.0f, 0 },
    { -4922.0f, -950.0f, 501.5f, 2.0f, 0 },
    { -4917.5f, -967.0f, 501.5f, 2.0f, 0 },
    { -4928.0f, -955.0f, 501.5f, 2.0f, 0 },
    { -4911.5f, -962.0f, 501.5f, 2.0f, 0 },
    { -4924.0f, -958.6f, 501.5f, 2.0f, 0 },
    { -4915.5f, -958.6f, 501.5f, 2.0f, 0 },
    // SW AH area (GUIDs 125-134)
    { -8811.0f, 667.0f, 97.0f, 3.0f, 0 },
    { -8815.0f, 671.0f, 97.0f, 2.0f, 0 },
    { -8808.0f, 663.0f, 97.0f, 2.0f, 0 },
    { -8818.0f, 675.0f, 97.0f, 3.0f, 0 },
    { -8805.0f, 660.0f, 97.0f, 2.0f, 0 },
    { -8813.0f, 669.0f, 97.0f, 2.0f, 0 },
    { -8820.0f, 664.0f, 97.0f, 2.0f, 0 },
    { -8807.0f, 673.0f, 97.0f, 3.0f, 0 },
    { -8816.0f, 660.0f, 97.0f, 2.0f, 0 },
    { -8810.0f, 676.0f, 97.0f, 2.0f, 0 },
    // SW Bank area (GUIDs 135-139)
    { -8880.0f, 644.0f, 96.0f, 3.0f, 0 },
    { -8876.0f, 648.0f, 96.0f, 2.0f, 0 },
    { -8884.0f, 640.0f, 96.0f, 2.0f, 0 },
    { -8872.0f, 646.0f, 96.0f, 3.0f, 0 },
    { -8878.0f, 650.0f, 96.0f, 2.0f, 0 },
    // Goldshire Inn (GUIDs 140-144)
    { -9456.0f, 45.0f, 57.0f, 3.0f, 0 },
    { -9460.0f, 42.0f, 57.0f, 2.0f, 0 },
    { -9452.0f, 48.0f, 57.0f, 2.0f, 0 },
    { -9458.0f, 38.0f, 57.0f, 3.0f, 0 },
    { -9464.0f, 46.0f, 57.0f, 2.0f, 0 },
    // IF Bank front — additional 25 bots (GUIDs 145-169)
    { -4920.5f, -952.0f, 501.5f, 2.0f, 0 },
    { -4919.0f, -965.0f, 501.5f, 2.0f, 0 },
    { -4925.5f, -961.5f, 501.5f, 2.0f, 0 },
    { -4914.0f, -955.5f, 501.5f, 2.0f, 0 },
    { -4921.5f, -967.5f, 501.5f, 2.0f, 0 },
    { -4918.0f, -949.5f, 501.5f, 2.0f, 0 },
    { -4926.0f, -964.5f, 501.5f, 2.0f, 0 },
    { -4913.5f, -952.5f, 501.5f, 2.0f, 0 },
    { -4928.5f, -958.0f, 501.5f, 2.0f, 0 },
    { -4911.0f, -959.0f, 501.5f, 2.0f, 0 },
    { -4922.5f, -953.5f, 501.5f, 2.0f, 0 },
    { -4917.0f, -963.5f, 501.5f, 2.0f, 0 },
    { -4924.5f, -955.0f, 501.5f, 2.0f, 0 },
    { -4915.0f, -962.0f, 501.5f, 2.0f, 0 },
    { -4921.0f, -949.0f, 501.5f, 2.0f, 0 },
    { -4918.5f, -968.0f, 501.5f, 2.0f, 0 },
    { -4927.0f, -954.5f, 501.5f, 2.0f, 0 },
    { -4912.5f, -962.5f, 501.5f, 2.0f, 0 },
    { -4923.0f, -966.0f, 501.5f, 2.0f, 0 },
    { -4916.5f, -951.0f, 501.5f, 2.0f, 0 },
    { -4925.0f, -959.5f, 501.5f, 2.0f, 0 },
    { -4914.5f, -957.5f, 501.5f, 2.0f, 0 },
    { -4920.0f, -950.5f, 501.5f, 2.0f, 0 },
    { -4919.5f, -966.5f, 501.5f, 2.0f, 0 },
    { -4929.0f, -957.0f, 501.5f, 2.0f, 0 },
    // IF Bank interior (GUIDs 170-174, 5 bots)
    // Within 2 yards of (-4891.7, -990.8, 503.9)
    { -4891.7f, -990.8f, 503.9f, 1.0f, 0 },
    { -4890.5f, -989.8f, 503.9f, 1.0f, 0 },
    { -4892.8f, -991.8f, 503.9f, 1.0f, 0 },
    { -4890.0f, -991.5f, 503.9f, 1.0f, 0 },
    { -4893.2f, -989.5f, 503.9f, 1.0f, 0 },
    // IF AH area (GUIDs 175-189, 15 bots)
    // Within 3 yards of AH vendor 1 (-4959.9, -906.4, 503.8)
    { -4959.9f, -906.4f, 503.8f, 1.0f, 0 },
    { -4958.0f, -904.5f, 503.8f, 1.0f, 0 },
    { -4961.8f, -908.3f, 503.8f, 1.0f, 0 },
    { -4957.5f, -907.5f, 503.8f, 1.0f, 0 },
    { -4962.2f, -905.0f, 503.8f, 1.0f, 0 },
    { -4958.5f, -909.0f, 503.8f, 1.0f, 0 },
    { -4961.0f, -904.0f, 503.8f, 1.0f, 0 },
    { -4957.0f, -906.0f, 503.8f, 1.0f, 0 },
    { -4962.5f, -907.0f, 503.8f, 1.0f, 0 },
    { -4959.0f, -909.2f, 503.8f, 1.0f, 0 },
    { -4960.5f, -903.8f, 503.8f, 1.0f, 0 },
    { -4957.8f, -908.5f, 503.8f, 1.0f, 0 },
    { -4962.0f, -906.0f, 503.8f, 1.0f, 0 },
    { -4958.2f, -904.0f, 503.8f, 1.0f, 0 },
    { -4961.5f, -909.0f, 503.8f, 1.0f, 0 },
};
static const size_t NUM_CITY_SPAWNS = sizeof(CITY_SPAWNS) / sizeof(CITY_SPAWNS[0]);
static const uint32 CITY_GUID_MIN = 100; // GUIDs 100-189

// Inn locations: 3 bots per inn, spread around center point
struct CrowdBotInnLocation
{
    float x, y, z;
    float radius;
    uint32 mapId;
    const char* name;
};

static const CrowdBotInnLocation INN_LOCATIONS[] =
{
    // Eastern Kingdoms (map 0)
    { -5582.0f,  -525.0f, 398.0f, 4.0f, 0, "Kharanos" },
    { -5405.0f, -2930.0f, 340.0f, 4.0f, 0, "Thelsamar" },
    { -10628.0f, 1036.0f,  34.0f, 4.0f, 0, "Sentinel Hill" },
    { -9278.0f, -2216.0f,  66.0f, 4.0f, 0, "Lakeshire" },
    { -10559.0f,-1189.0f,  28.0f, 4.0f, 0, "Darkshire" },
    { -3745.0f,  -751.0f,  10.0f, 4.0f, 0, "Menethil Harbor" },
    {  -808.0f,  -564.0f,  18.0f, 4.0f, 0, "Southshore" },
    { -1247.0f, -2516.0f,  22.0f, 4.0f, 0, "Refuge Pointe" },
    {   320.0f, -2066.0f, 122.0f, 4.0f, 0, "Aerie Peak" },
    {  2271.0f, -5312.0f,  82.0f, 4.0f, 0, "Light's Hope Chapel" },
    { -7510.0f, -1038.0f, 228.0f, 4.0f, 0, "Morgan's Vigil" },
    { -10920.0f,-3439.0f,  22.0f, 4.0f, 0, "Nethergarde Keep" },
    { -14414.0f,  419.0f,  22.0f, 4.0f, 0, "Booty Bay" },
    { -6554.0f, -1101.0f, 310.0f, 4.0f, 0, "Thorium Point" },
    // Kalimdor (map 1)
    {  9837.0f,   966.0f,1306.0f, 4.0f, 1, "Dolanaar" },
    {  6456.0f,   683.0f,   9.0f, 4.0f, 1, "Auberdine" },
    {  2715.0f,  -379.0f, 107.0f, 4.0f, 1, "Astranaar" },
    {  -472.0f,  1546.0f,  88.0f, 4.0f, 1, "Nijel's Point" },
    { -4371.0f,  3338.0f,  12.0f, 4.0f, 1, "Feathermoon" },
    { -3680.0f, -4389.0f,  10.0f, 4.0f, 1, "Theramore" },
    { -7167.0f, -3785.0f,   8.0f, 4.0f, 1, "Gadgetzan" },
    {  6724.0f, -4606.0f, 720.0f, 4.0f, 1, "Everlook" },
    { -6815.0f,   836.0f,  42.0f, 4.0f, 1, "Cenarion Hold" },
    {  -996.0f, -3827.0f,   6.0f, 4.0f, 1, "Ratchet" },
};
static const size_t NUM_INN_LOCATIONS = sizeof(INN_LOCATIONS) / sizeof(INN_LOCATIONS[0]);
static const uint32 INN_GUID_MIN = 300;  // GUIDs 300-371 (3 per inn)
static const uint32 BOTS_PER_INN = 3;

// Idle emotes that look natural for AFK city players
static const uint32 CROWD_BOT_EMOTES[] =
{
    1,   // EMOTE_ONESHOT_TALK
    25,  // EMOTE_ONESHOT_POINT
    396, // EMOTE_ONESHOT_TALK_NOSHEATHE
    92,  // EMOTE_ONESHOT_EAT_NOSHEATHE
    3,   // EMOTE_ONESHOT_WAVE
};
static const size_t NUM_CROWD_BOT_EMOTES = sizeof(CROWD_BOT_EMOTES) / sizeof(CROWD_BOT_EMOTES[0]);

void CrowdBotAI::BeforeAddToMap(Player* player)
{
    uint32 guid = botEntry ? botEntry->playerGUID : 0;
    if (!guid) return;

    float cx, cy, cz, radius;
    uint32 mapId;

    // City crowd bots (GUIDs 100-144): precise spawn points
    if (guid >= CITY_GUID_MIN && guid < CITY_GUID_MIN + NUM_CITY_SPAWNS)
    {
        uint32 index = guid - CITY_GUID_MIN;
        cx = CITY_SPAWNS[index].x;
        cy = CITY_SPAWNS[index].y;
        cz = CITY_SPAWNS[index].z;
        radius = CITY_SPAWNS[index].radius;
        mapId = CITY_SPAWNS[index].mapId;
    }
    // Inn crowd bots (GUIDs 300+): 3 per inn, centered on inn location
    else if (guid >= INN_GUID_MIN)
    {
        uint32 innIndex = (guid - INN_GUID_MIN) / BOTS_PER_INN;
        if (innIndex >= NUM_INN_LOCATIONS)
            innIndex = 0;
        cx = INN_LOCATIONS[innIndex].x;
        cy = INN_LOCATIONS[innIndex].y;
        cz = INN_LOCATIONS[innIndex].z;
        radius = INN_LOCATIONS[innIndex].radius;
        mapId = INN_LOCATIONS[innIndex].mapId;
    }
    else
    {
        // Guild bots or other non-crowd bots: use their DB position as-is
        m_homeX = player->GetPositionX();
        m_homeY = player->GetPositionY();
        m_homeZ = player->GetPositionZ();
        return;
    }

    float x = cx + frand(-radius, radius);
    float y = cy + frand(-radius, radius);
    float z = cz;

    player->Relocate(x, y, z);
    player->SetLocationMapId(mapId);

    m_homeX = x;
    m_homeY = y;
    m_homeZ = z;
}

void CrowdBotAI::OnPlayerLogin()
{
    if (!me) return;

    // Guild bots keep their DB level (synced to player level ±3 by SyncBotLevels).
    // Regular crowd bots randomize: 70% level 60, 30% level 40-59.
    if (!sGuildBotMgr.IsGuildBot(me->GetGUIDLow()))
    {
        uint32 level = urand(0, 99) < 70 ? 60 : urand(40, 59);
        if (me->GetLevel() != level)
        {
            me->GiveLevel(level);
            me->InitTalentForLevel();
            me->SetUInt32Value(PLAYER_XP, 0);
        }
    }

    // Equip premade gear — use preraid-bis (dungeon blues), not phase6-bis
    if (!m_gearEquipped)
    {
        m_gearEquipped = true;
        for (int i = EQUIPMENT_SLOT_START; i < EQUIPMENT_SLOT_END; ++i)
            me->AutoUnequipItemFromSlot(i);

        uint32 bestEntry = 0;
        uint32 bestLevel = 0;
        for (auto const& itr : sObjectMgr.GetPlayerPremadeGearTemplates())
        {
            if (itr.second.requiredClass == me->GetClass() &&
                itr.second.level <= me->GetLevel() &&
                itr.second.level > bestLevel)
            {
                // For level 60 crowd bots, prefer preraid-bis over phase6-bis
                if (me->GetLevel() == 60 && itr.second.level == 60)
                {
                    // Skip phase6-bis entries (only use preraid-bis)
                    if (itr.second.name.find("phase6") != std::string::npos)
                        continue;
                }
                bestLevel = itr.second.level;
                bestEntry = itr.second.entry;
            }
        }
        if (bestEntry)
            sObjectMgr.ApplyPremadeGearTemplateToPlayer(bestEntry, me);
        else
            me->AddStartingItems();
    }

    // ~25% of level 60 crowd bots sit on their race's epic mount
    if (me->GetLevel() == 60 && urand(1, 100) <= 25)
    {
        // Epic mount spell IDs by Alliance race
        static const uint32 HUMAN_MOUNTS[]   = { 23227, 23229, 23228 };  // Swift Palomino/Brown/White Steed
        static const uint32 DWARF_MOUNTS[]   = { 23238, 23239, 23240 };  // Swift Brown/Gray/White Ram
        static const uint32 NELF_MOUNTS[]    = { 23221, 23219, 23338 };  // Swift Frost/Mist/Stormsaber
        static const uint32 GNOME_MOUNTS[]   = { 23225, 23223, 23222 };  // Swift Green/White/Yellow Mechanostrider

        uint32 mountSpell = 0;
        switch (me->GetRace())
        {
            case 1: mountSpell = HUMAN_MOUNTS[urand(0, 2)];  break;  // Human
            case 3: mountSpell = DWARF_MOUNTS[urand(0, 2)];  break;  // Dwarf
            case 4: mountSpell = NELF_MOUNTS[urand(0, 2)];   break;  // Night Elf
            case 7: mountSpell = GNOME_MOUNTS[urand(0, 2)];  break;  // Gnome
        }
        if (mountSpell)
            me->CastSpell(me, mountSpell, true);
    }

    // Randomize initial emote timer so bots don't all emote at the same time
    m_emoteTimer = urand(5000, 30000);
    m_chatTimer = urand(60000, 300000);

    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "[CrowdBot] %s (lvl %u) spawned at (%.1f, %.1f, %.1f)",
        me->GetName(), me->GetLevel(), m_homeX, m_homeY, m_homeZ);
}

void CrowdBotAI::UpdateAI(uint32 const diff)
{
    PlayerBotAI::UpdateAI(diff);

    // Handle pending trade accept timer (for service trades)
    if (m_pendingTradeAcceptTimer)
    {
        if (m_pendingTradeAcceptTimer <= diff)
        {
            m_pendingTradeAcceptTimer = 0;
            if (me->GetTradeData() && !me->GetTradeData()->IsAccepted())
            {
                // Don't set enchant spell here — it causes accept loops.
                // The enchant will be applied directly after trade completes.

                // Clear scam prevention delay and call accept handler directly
                me->GetTradeData()->SetLastModificationTime(0);
                me->GetTradeData()->SetScamPreventionDelay(0);

                WorldPacket acceptData(CMSG_ACCEPT_TRADE, 4);
                acceptData << uint32(1);
                me->GetSession()->HandleAcceptTradeOpcode(acceptData);

                sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
                    "[CrowdBot] %s called HandleAcceptTradeOpcode directly", me->GetName());
            }
        }
        else
            m_pendingTradeAcceptTimer -= diff;
    }

    m_updateTimer += diff;
    if (m_updateTimer < 5000)
        return;
    uint32 elapsed = m_updateTimer;
    m_updateTimer = 0;

    if (!me || !me->IsAlive())
    {
        // Instant resurrect if somehow killed
        if (me && me->GetDeathState() == CORPSE)
        {
            me->ResurrectPlayer(1.0f);
            me->SpawnCorpseBones();
        }
        return;
    }

    // Idle emotes on a timer
    m_emoteTimer = (m_emoteTimer > elapsed) ? m_emoteTimer - elapsed : 0;
    if (m_emoteTimer == 0)
    {
        uint32 emote = CROWD_BOT_EMOTES[urand(0, NUM_CROWD_BOT_EMOTES - 1)];
        me->HandleEmoteCommand(emote);
        m_emoteTimer = urand(15000, 60000); // next emote in 15-60 seconds
    }

    // 5% chance per tick to micro-wander (2-3 yards from home)
    if (urand(0, 99) < 5 &&
        me->GetMotionMaster()->GetCurrentMovementGeneratorType() == IDLE_MOTION_TYPE)
    {
        float x = m_homeX + frand(-3.0f, 3.0f);
        float y = m_homeY + frand(-3.0f, 3.0f);
        float z = m_homeZ;
        me->UpdateGroundPositionZ(x, y, z);
        me->GetMotionMaster()->MovePoint(0, x, y, z, MOVE_PATHFINDING | MOVE_EXCLUDE_STEEP_SLOPES);
    }
}

void CrowdBotAI::OnPacketReceived(WorldPacket const* packet)
{
    if (!me)
        return;

    switch (packet->GetOpcode())
    {
        case SMSG_TRADE_STATUS:
        {
            WorldPacket pktCopy(*packet);
            uint32 status;
            pktCopy >> status;

            if (status == TRADE_STATUS_BEGIN_TRADE)
            {
                // A player initiated trade with this crowd bot.
                if (me->GetTradeData())
                {
                    Player* trader = me->GetTradeData()->GetTrader();
                    bool accepted = trader && sBotServiceMgr.ShouldAcceptTrade(me, trader);
                    sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL,
                        "[CrowdBot] %s trade request from %s — ShouldAcceptTrade=%s",
                        me->GetName(), trader ? trader->GetName() : "null", accepted ? "YES" : "NO");
                    if (accepted)
                    {
                        std::unique_ptr<WorldPacket> beginData =
                            std::make_unique<WorldPacket>(CMSG_BEGIN_TRADE, 0);
                        me->GetSession()->QueuePacket(std::move(beginData));
                    }
                    else
                    {
                        me->TradeCancel(true, TRADE_STATUS_BUSY);
                    }
                }
            }
            else if (status == TRADE_STATUS_OPEN_WINDOW)
            {
                // Trade window open — set up bot's side (enchant spell, etc.)
                if (me->GetTradeData())
                {
                    Player* trader = me->GetTradeData()->GetTrader();
                    if (trader)
                        sBotServiceMgr.OnTradeWindowOpen(me, trader);
                }
            }
            else if (status == TRADE_STATUS_TRADE_ACCEPT)
            {
                // Player accepted — set enchant spell and accept via timer.
                if (me->GetTradeData() && !me->GetTradeData()->IsAccepted())
                {
                    m_pendingTradeAcceptTimer = 500;
                }
            }
            return;
        }
    }
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
    if (ainame == "CrowdBotAI")
        return new CrowdBotAI();
    return new PlayerBotAI();
}
