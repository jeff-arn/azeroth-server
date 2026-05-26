/*
 * AuraTracker — sends aura lifecycle events to group members via
 * CHAT_MSG_ADDON so that client-side addons can accurately track
 * debuffs/buffs with spell IDs, GUIDs, durations, and stack counts.
 */

#ifndef MANGOS_AURA_TRACKER_H
#define MANGOS_AURA_TRACKER_H

#include "SpellAuras.h"
#include "UnitDefines.h"

namespace AuraTracker
{
    void SendAuraApplied(SpellAuraHolder const* holder);
    void SendAuraRemoved(SpellAuraHolder const* holder, AuraRemoveMode mode);
    void SendAuraRefresh(SpellAuraHolder const* holder);
    void SendAuraDose(SpellAuraHolder const* holder, bool increased);
    void SendPeriodicTick(Aura const* aura, uint32 amount);
} // namespace AuraTracker

#endif // MANGOS_AURA_TRACKER_H
