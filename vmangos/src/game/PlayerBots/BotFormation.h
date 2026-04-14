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

#ifndef MANGOS_BOT_FORMATION_H
#define MANGOS_BOT_FORMATION_H

#include "Common.h"
#include "ObjectGuid.h"

class Map;
class Player;
class Group;

// ============================================================
// Breadcrumb Trail — ring buffer of leader path waypoints
// ============================================================

struct BreadcrumbPoint
{
    float x = 0.0f;
    float y = 0.0f;
    float z = 0.0f;
    float orientation = 0.0f;
    uint32 timestamp = 0;
};

class BreadcrumbTrail
{
public:
    static constexpr uint32 MAX_CRUMBS = 40;
    static constexpr float MIN_RECORD_DIST_SQ = 4.0f;   // 2yd minimum movement
    static constexpr uint32 RECORD_INTERVAL_MS = 1000;   // match bot tick rate

    void RecordPoint(float x, float y, float z, float o, uint32 now);
    void Clear();

    // Walk trail backward by arc-length and interpolate to the exact distance.
    bool GetPointAtDistance(float targetDist, BreadcrumbPoint& out) const;

    // From startIndex, walk further back by additionalDist yards.
    bool GetPointFurtherBack(uint32 startIndex, float additionalDist, BreadcrumbPoint& out) const;

    // Find the trail index closest to a world position.
    uint32 FindNearestIndex(float x, float y, float z) const;

    uint32 Size() const { return m_count; }
    bool IsEmpty() const { return m_count == 0; }

    // Get a point by raw ring-buffer offset (0 = newest).
    bool GetPoint(uint32 offsetFromHead, BreadcrumbPoint& out) const;

private:
    // Convert a "steps back from head" offset to a ring-buffer array index.
    uint32 RingIndex(uint32 stepsBack) const;

    BreadcrumbPoint m_points[MAX_CRUMBS];
    uint32 m_head = 0;
    uint32 m_count = 0;
    uint32 m_lastRecordTime = 0;
};

// ============================================================
// Formation Slot — one bot's assigned position in the column
// ============================================================

struct FormationSlot
{
    uint8 slotIndex = 0;
    uint8 role = 0;              // CombatBotRoles value
    float trailDistBehind = 0.0f;
    float lateralOffset = 0.0f;  // +right / -left relative to trail direction
    ObjectGuid assignedBot;
};

// ============================================================
// Formation State — per-group trail + slot assignments
// ============================================================

struct FormationState
{
    BreadcrumbTrail trail;
    std::vector<FormationSlot> slots;
    ObjectGuid leaderGuid;
    uint8 lastGroupSize = 0;

    // Rebuild slot assignments from current group composition.
    void AssignSlots(Group* pGroup);

    // Compute world position for a given slot.
    // Returns true if a valid position was found.
    bool CalculateSlotPosition(uint32 slotIndex, Map* map, Player* bot,
                               float& outX, float& outY, float& outZ) const;

    // Compute a safe flee point along the trail (backward toward cleared area).
    // Returns true if a safe point was found.
    bool CalculateFleePoint(Player* bot, Map* map,
                            float& outX, float& outY, float& outZ) const;
};

// ============================================================
// Formation Manager — singleton, keyed by leader GUID
// ============================================================

class BotFormationMgr
{
public:
    static BotFormationMgr& Instance();

    FormationState* GetFormation(ObjectGuid leaderGuid);
    void RemoveFormation(ObjectGuid leaderGuid);
    void UpdateTrail(ObjectGuid leaderGuid, Player* pLeader, uint32 now);

private:
    BotFormationMgr() = default;
    std::unordered_map<uint64, FormationState> m_formations;
};

#define sBotFormationMgr BotFormationMgr::Instance()

// Slot distance constants by role
namespace BotFormationDefaults
{
    // Trail distance behind leader (yards)
    constexpr float TANK_TRAIL_DIST     = 2.0f;
    constexpr float MELEE_TRAIL_DIST    = 4.0f;
    constexpr float RANGED_TRAIL_DIST   = 8.0f;
    constexpr float HEALER_TRAIL_DIST   = 10.0f;

    // Lateral offset from trail center (yards)
    constexpr float MELEE_LATERAL       = 1.0f;
    constexpr float RANGED_LATERAL      = 1.0f;

    // Flee point search parameters
    constexpr float FLEE_MIN_DIST       = 15.0f;
    constexpr float FLEE_MAX_DIST       = 30.0f;
    constexpr float FLEE_HOSTILE_SCAN   = 12.0f;

    // Position dead-zone to prevent jitter
    constexpr float MOVE_DEADZONE       = 2.0f;

    // Aggro safety scan radius for lateral slot validation
    constexpr float SLOT_HOSTILE_SCAN   = 12.0f;
}

#endif
