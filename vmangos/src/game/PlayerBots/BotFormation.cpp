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

#include "BotFormation.h"
#include <algorithm>
#include <cmath>
#include "Cell.h"
#include "CellImpl.h"
#include "CombatBotBaseAI.h"
#include "GridNotifiers.h"
#include "GridNotifiersImpl.h"
#include "Group.h"
#include "Map.h"
#include "PartyBotAI.h"
#include "Player.h"

// ============================================================
// Utility
// ============================================================

static float Distance3D(BreadcrumbPoint const& a, BreadcrumbPoint const& b)
{
    float dx = a.x - b.x;
    float dy = a.y - b.y;
    float dz = a.z - b.z;
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}

static float Distance3D(float x1, float y1, float z1, float x2, float y2, float z2)
{
    float dx = x1 - x2;
    float dy = y1 - y2;
    float dz = z1 - z2;
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}

static void Lerp(BreadcrumbPoint const& a, BreadcrumbPoint const& b, float t, BreadcrumbPoint& out)
{
    out.x = a.x + (b.x - a.x) * t;
    out.y = a.y + (b.y - a.y) * t;
    out.z = a.z + (b.z - a.z) * t;
    out.orientation = a.orientation;
}

// Check if any hostile creature is alive within range of a world point.
static bool HasHostilesNearPoint(Player* bot, Map* map, float x, float y, float z, float range)
{
    // We need a WorldObject at the scan center. Since Cell::VisitGridObjects can
    // take x,y coordinates directly, we use that overload with the bot as the
    // hostility reference but scan from the candidate point.
    std::list<Unit*> hostiles;
    MaNGOS::AnyHostileUnitInObjectRangeCheck check(bot, bot, range);
    MaNGOS::UnitListSearcher<MaNGOS::AnyHostileUnitInObjectRangeCheck> searcher(hostiles, check);
    Cell::VisitGridObjects(x, y, map, searcher, range);
    return !hostiles.empty();
}

// ============================================================
// BreadcrumbTrail
// ============================================================

uint32 BreadcrumbTrail::RingIndex(uint32 stepsBack) const { return (m_head + MAX_CRUMBS - 1 - stepsBack) % MAX_CRUMBS; }

void BreadcrumbTrail::RecordPoint(float x, float y, float z, float o, uint32 now)
{
    if (now - m_lastRecordTime < RECORD_INTERVAL_MS)
        return;

    if (m_count > 0)
    {
        BreadcrumbPoint const& last = m_points[RingIndex(0)];
        float dx = x - last.x;
        float dy = y - last.y;
        float dz = z - last.z;
        if (dx * dx + dy * dy + dz * dz < MIN_RECORD_DIST_SQ)
            return;
    }

    m_points[m_head] = {x, y, z, o, now};
    m_head = (m_head + 1) % MAX_CRUMBS;
    if (m_count < MAX_CRUMBS)
        m_count++;
    m_lastRecordTime = now;
}

void BreadcrumbTrail::Clear()
{
    m_head = 0;
    m_count = 0;
    m_lastRecordTime = 0;
}

bool BreadcrumbTrail::GetPoint(uint32 offsetFromHead, BreadcrumbPoint& out) const
{
    if (offsetFromHead >= m_count)
        return false;
    out = m_points[RingIndex(offsetFromHead)];
    return true;
}

bool BreadcrumbTrail::GetPointAtDistance(float targetDist, BreadcrumbPoint& out) const
{
    if (m_count == 0)
        return false;

    if (m_count == 1 || targetDist <= 0.0f)
    {
        out = m_points[RingIndex(0)];
        return true;
    }

    float accumDist = 0.0f;
    for (uint32 i = 0; i < m_count - 1; ++i)
    {
        BreadcrumbPoint const& curr = m_points[RingIndex(i)];
        BreadcrumbPoint const& next = m_points[RingIndex(i + 1)];
        float segLen = Distance3D(curr, next);

        if (accumDist + segLen >= targetDist)
        {
            float t = (segLen > 0.001f) ? (targetDist - accumDist) / segLen : 0.0f;
            Lerp(curr, next, t, out);
            return true;
        }
        accumDist += segLen;
    }

    // Trail is shorter than targetDist — return oldest point.
    out = m_points[RingIndex(m_count - 1)];
    return true;
}

bool BreadcrumbTrail::GetPointFurtherBack(uint32 startIndex, float additionalDist, BreadcrumbPoint& out) const
{
    if (startIndex >= m_count)
        return false;

    if (additionalDist <= 0.0f)
    {
        out = m_points[RingIndex(startIndex)];
        return true;
    }

    float accumDist = 0.0f;
    for (uint32 i = startIndex; i < m_count - 1; ++i)
    {
        BreadcrumbPoint const& curr = m_points[RingIndex(i)];
        BreadcrumbPoint const& next = m_points[RingIndex(i + 1)];
        float segLen = Distance3D(curr, next);

        if (accumDist + segLen >= additionalDist)
        {
            float t = (segLen > 0.001f) ? (additionalDist - accumDist) / segLen : 0.0f;
            Lerp(curr, next, t, out);
            return true;
        }
        accumDist += segLen;
    }

    // Not enough trail — return oldest point.
    out = m_points[RingIndex(m_count - 1)];
    return true;
}

uint32 BreadcrumbTrail::FindNearestIndex(float x, float y, float z) const
{
    uint32 bestIdx = 0;
    float bestDistSq = std::numeric_limits<float>::max();

    for (uint32 i = 0; i < m_count; ++i)
    {
        BreadcrumbPoint const& pt = m_points[RingIndex(i)];
        float dx = x - pt.x;
        float dy = y - pt.y;
        float dz = z - pt.z;
        float distSq = dx * dx + dy * dy + dz * dz;
        if (distSq < bestDistSq)
        {
            bestDistSq = distSq;
            bestIdx = i;
        }
    }
    return bestIdx;
}

// ============================================================
// FormationState
// ============================================================

void FormationState::AssignSlots(Group* pGroup)
{
    slots.clear();
    if (!pGroup)
        return;

    lastGroupSize = pGroup->GetMembersCount();

    // Collect bots with roles, assign slot distances.
    uint8 meleeCount = 0;
    uint8 rangedCount = 0;

    for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        Player* pMember = itr->getSource();
        if (!pMember)
            continue;

        // Only assign slots to bots, not human players.
        PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(pMember->AI());
        if (!partyAI)
            continue;

        CombatBotRoles role = static_cast<CombatBotRoles>(partyAI->m_role);

        FormationSlot slot;
        slot.assignedBot = pMember->GetObjectGuid();
        slot.role = role;

        switch (role)
        {
        case ROLE_TANK:
            slot.trailDistBehind = BotFormationDefaults::TANK_TRAIL_DIST;
            slot.lateralOffset = 0.0f;
            break;
        case ROLE_MELEE_DPS:
            slot.trailDistBehind = BotFormationDefaults::MELEE_TRAIL_DIST;
            slot.lateralOffset = (meleeCount % 2 == 0) ? BotFormationDefaults::MELEE_LATERAL : -BotFormationDefaults::MELEE_LATERAL;
            meleeCount++;
            break;
        case ROLE_RANGE_DPS:
            slot.trailDistBehind = BotFormationDefaults::RANGED_TRAIL_DIST;
            slot.lateralOffset = (rangedCount % 2 == 0) ? BotFormationDefaults::RANGED_LATERAL : -BotFormationDefaults::RANGED_LATERAL;
            rangedCount++;
            break;
        case ROLE_HEALER:
            slot.trailDistBehind = BotFormationDefaults::HEALER_TRAIL_DIST;
            slot.lateralOffset = 0.0f;
            break;
        default:
            // ROLE_INVALID — treat as melee DPS
            slot.trailDistBehind = BotFormationDefaults::MELEE_TRAIL_DIST;
            slot.lateralOffset = (meleeCount % 2 == 0) ? BotFormationDefaults::MELEE_LATERAL : -BotFormationDefaults::MELEE_LATERAL;
            meleeCount++;
            break;
        }

        slots.push_back(slot);
    }

    // Assign indices and update each bot's slot index.
    for (uint8 i = 0; i < slots.size(); ++i)
    {
        slots[i].slotIndex = i;

        // Find the bot and set its slot index.
        for (GroupReference* itr = pGroup->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            Player* pMember = itr->getSource();
            if (pMember && pMember->GetObjectGuid() == slots[i].assignedBot)
            {
                if (PartyBotAI* partyAI = dynamic_cast<PartyBotAI*>(pMember->AI()))
                    partyAI->m_formationSlotIndex = i;
                break;
            }
        }
    }
}

bool FormationState::CalculateSlotPosition(uint32 slotIndex, Map* map, Player* bot, float& outX, float& outY, float& outZ) const
{
    if (slotIndex >= slots.size() || trail.IsEmpty())
        return false;

    FormationSlot const& slot = slots[slotIndex];

    // 1. Get the base trail point at this slot's distance behind the leader.
    BreadcrumbPoint basePoint;
    if (!trail.GetPointAtDistance(slot.trailDistBehind, basePoint))
        return false;

    // Start with the trail center as the output.
    outX = basePoint.x;
    outY = basePoint.y;
    outZ = basePoint.z;

    // 2. If no lateral offset, we're done (tank, healer center positions).
    if (std::fabs(slot.lateralOffset) < 0.01f)
        return true;

    // 3. Compute trail direction at this point for the perpendicular vector.
    //    We need two trail points to get a direction.
    BreadcrumbPoint prevPoint;
    float dirX, dirY;

    if (trail.GetPointAtDistance(slot.trailDistBehind + 1.0f, prevPoint))
    {
        dirX = basePoint.x - prevPoint.x;
        dirY = basePoint.y - prevPoint.y;
    }
    else
    {
        // Fallback: use leader orientation.
        dirX = std::cos(basePoint.orientation);
        dirY = std::sin(basePoint.orientation);
    }

    // Normalize direction in XY plane.
    float len = std::sqrt(dirX * dirX + dirY * dirY);
    if (len < 0.001f)
        return true; // Degenerate — no direction, stay at center.

    dirX /= len;
    dirY /= len;

    // 4. Perpendicular vector (rotate 90 degrees clockwise in XY).
    float perpX = dirY;
    float perpY = -dirX;

    // 5. Candidate lateral position.
    float candX = basePoint.x + perpX * slot.lateralOffset;
    float candY = basePoint.y + perpY * slot.lateralOffset;
    float candZ = basePoint.z;

    // 6. Navmesh validate: can we walk from trail center to the candidate?
    float testX = candX, testY = candY, testZ = candZ;
    if (!map->GetWalkHitPosition(nullptr, basePoint.x, basePoint.y, basePoint.z, testX, testY, testZ))
    {
        // Navmesh says no — stay at trail center.
        return true;
    }

    // Check if the navmesh moved us too far from where we wanted.
    float drift = Distance3D(testX, testY, testZ, candX, candY, candZ);
    if (drift > 3.0f)
        return true; // Corridor too narrow — collapse to trail center.

    // 7. Aggro safety check: are there hostiles near the candidate position?
    if (bot && HasHostilesNearPoint(bot, map, testX, testY, testZ, BotFormationDefaults::SLOT_HOSTILE_SCAN))
    {
        // Mobs nearby — collapse to trail center to avoid pulls.
        return true;
    }

    // Position is valid — use the lateral offset.
    outX = testX;
    outY = testY;
    outZ = testZ;
    return true;
}

bool FormationState::CalculateFleePoint(Player* bot, Map* map, float& outX, float& outY, float& outZ) const
{
    if (!bot || !map || trail.Size() < 2)
        return false;

    // 1. Find bot's nearest trail point.
    uint32 nearestIdx = trail.FindNearestIndex(bot->GetPositionX(), bot->GetPositionY(), bot->GetPositionZ());

    // 2. Walk backward along the trail in steps, looking for a safe point.
    float searchDist = BotFormationDefaults::FLEE_MIN_DIST;
    float const step = 5.0f;

    while (searchDist <= BotFormationDefaults::FLEE_MAX_DIST)
    {
        BreadcrumbPoint candidate;
        if (!trail.GetPointFurtherBack(nearestIdx, searchDist, candidate))
            return false;

        // 3. Check for hostiles near candidate.
        if (!HasHostilesNearPoint(bot, map, candidate.x, candidate.y, candidate.z, BotFormationDefaults::FLEE_HOSTILE_SCAN))
        {
            // 4. Navmesh validate.
            float testX = candidate.x, testY = candidate.y, testZ = candidate.z;
            if (map->GetWalkHitPosition(nullptr, bot->GetPositionX(), bot->GetPositionY(), bot->GetPositionZ(), testX, testY, testZ))
            {
                outX = testX;
                outY = testY;
                outZ = testZ;
                return true;
            }
        }

        searchDist += step;
    }

    return false; // No safe point found — caller uses RunAwayFromTarget fallback.
}

// ============================================================
// BotFormationMgr
// ============================================================

BotFormationMgr& BotFormationMgr::Instance()
{
    static BotFormationMgr instance;
    return instance;
}

FormationState* BotFormationMgr::GetFormation(ObjectGuid leaderGuid)
{
    uint64 key = leaderGuid.GetRawValue();
    auto it = m_formations.find(key);
    if (it != m_formations.end())
        return &it->second;

    // Create a new formation state.
    FormationState& fs = m_formations[key];
    fs.leaderGuid = leaderGuid;
    return &fs;
}

void BotFormationMgr::RemoveFormation(ObjectGuid leaderGuid) { m_formations.erase(leaderGuid.GetRawValue()); }

void BotFormationMgr::UpdateTrail(ObjectGuid leaderGuid, Player* pLeader, uint32 now)
{
    if (!pLeader)
        return;

    FormationState* fs = GetFormation(leaderGuid);
    if (!fs)
        return;

    fs->trail.RecordPoint(pLeader->GetPositionX(), pLeader->GetPositionY(), pLeader->GetPositionZ(), pLeader->GetOrientation(), now);
}
