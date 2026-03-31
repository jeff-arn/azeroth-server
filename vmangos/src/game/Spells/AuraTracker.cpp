/*
 * AuraTracker — sends aura lifecycle events to group members via
 * CHAT_MSG_ADDON so that client-side addons can accurately track
 * debuffs/buffs with spell IDs, GUIDs, durations, and stack counts.
 *
 * Message format (prefix "AURA", tab-separated from payload):
 *   EVENT:spellId:srcGuid:dstGuid:school:auraType:stacks:duration:remaining[:extra]
 *
 * Events: A (applied), R (removed), RF (refresh), T (tick), DS (dose+), DR (dose-)
 */

#include "AuraTracker.h"
#include "Config/Config.h"
#include "Opcodes.h"
#include "Player.h"
#include "WorldPacket.h"

namespace
{

bool IsEnabled()
{
    return sConfig.GetBoolDefault("AuraTracker.Enable", false);
}

bool ShouldTrack(SpellAuraHolder const* holder)
{
    if (!holder || !holder->GetTarget())
        return false;

    // Skip passive/invisible auras — not useful to addons.
    if (holder->IsPassive())
        return false;

    return true;
}

// Format a GUID as a hex string (e.g. "0xF130004D2B008852").
void FormatGuid(ObjectGuid guid, char* buf, size_t bufSize)
{
    snprintf(buf, bufSize, "0x%llX", (unsigned long long)guid.GetRawValue());
}

// Build an SMSG_MESSAGECHAT packet with CHAT_MSG_PARTY + LANG_ADDON.
// The 1.12 client fires CHAT_MSG_ADDON for the addon, keyed on the
// prefix before the tab character in the message body.
void BuildAddonPacket(WorldPacket& data, char const* message)
{
    // CHAT_MSG_PARTY requires two copies of senderGuid (see BuildChatPacket).
    // Using LANG_ADDON causes the client to fire CHAT_MSG_ADDON to addons.
    data.Initialize(SMSG_MESSAGECHAT);
    data << uint8(CHAT_MSG_PARTY);
    data << uint32(LANG_ADDON);
    data << ObjectGuid();                       // senderGuid (1st copy)
    data << ObjectGuid();                       // senderGuid (2nd copy)
    data << uint32(strlen(message) + 1);
    data << message;
    data << uint8(0);                           // CHAT_TAG_NONE
}

// Send the addon packet to every group member that is in range of the
// aura's target.  If the target is a player, broadcast to their group.
// If a creature, send to all nearby players (mirroring how
// SendPeriodicAuraLog already works via SendObjectMessageToSet).
void SendToNearbyGroup(Unit const* target, WorldPacket& data)
{
    if (!target || !target->IsInWorld())
        return;

    // Use the same delivery path as combat log packets: every player
    // in visual range of the target.  This naturally covers the entire
    // group/raid since they must be nearby to see the aura.
    target->SendObjectMessageToSet(&data, true);
}

char const* RemoveReasonString(AuraRemoveMode mode)
{
    switch (mode)
    {
        case AURA_REMOVE_BY_EXPIRE:       return "expire";
        case AURA_REMOVE_BY_DISPEL:       return "dispel";
        case AURA_REMOVE_BY_CANCEL:       return "cancel";
        case AURA_REMOVE_BY_DEATH:        return "death";
        case AURA_REMOVE_BY_STACK:        return "stack";
        case AURA_REMOVE_BY_SHIELD_BREAK: return "break";
        case AURA_REMOVE_BY_CHANNEL:      return "channel";
        case AURA_REMOVE_BY_RANGE:        return "range";
        case AURA_REMOVE_BY_GROUP:        return "group";
        default:                          return "default";
    }
}

} // anonymous namespace

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

void AuraTracker::SendAuraApplied(SpellAuraHolder const* holder)
{
    if (!IsEnabled() || !ShouldTrack(holder))
        return;

    SpellEntry const* spell = holder->GetSpellProto();
    Unit const* target = holder->GetTarget();

    char srcBuf[24], dstBuf[24];
    FormatGuid(holder->GetCasterGuid(), srcBuf, sizeof(srcBuf));
    FormatGuid(target->GetObjectGuid(), dstBuf, sizeof(dstBuf));

    char payload[256];
    snprintf(payload, sizeof(payload),
        "AURA\tA:%u:%s:%s:%u:%c:%u:%d:%d",
        spell->Id,
        srcBuf,
        dstBuf,
        spell->School,
        holder->IsPositive() ? 'B' : 'D',
        holder->GetStackAmount(),
        holder->GetAuraMaxDuration(),
        holder->GetAuraDuration());

    WorldPacket data;
    BuildAddonPacket(data, payload);
    SendToNearbyGroup(target, data);
}

void AuraTracker::SendAuraRemoved(SpellAuraHolder const* holder, AuraRemoveMode mode)
{
    if (!IsEnabled() || !ShouldTrack(holder))
        return;

    SpellEntry const* spell = holder->GetSpellProto();
    Unit const* target = holder->GetTarget();

    char srcBuf[24], dstBuf[24];
    FormatGuid(holder->GetCasterGuid(), srcBuf, sizeof(srcBuf));
    FormatGuid(target->GetObjectGuid(), dstBuf, sizeof(dstBuf));

    char payload[256];
    snprintf(payload, sizeof(payload),
        "AURA\tR:%u:%s:%s:%u:%c:%u:0:0:%s",
        spell->Id,
        srcBuf,
        dstBuf,
        spell->School,
        holder->IsPositive() ? 'B' : 'D',
        holder->GetStackAmount(),
        RemoveReasonString(mode));

    WorldPacket data;
    BuildAddonPacket(data, payload);
    SendToNearbyGroup(target, data);
}

void AuraTracker::SendAuraRefresh(SpellAuraHolder const* holder)
{
    if (!IsEnabled() || !ShouldTrack(holder))
        return;

    SpellEntry const* spell = holder->GetSpellProto();
    Unit const* target = holder->GetTarget();

    char srcBuf[24], dstBuf[24];
    FormatGuid(holder->GetCasterGuid(), srcBuf, sizeof(srcBuf));
    FormatGuid(target->GetObjectGuid(), dstBuf, sizeof(dstBuf));

    char payload[256];
    snprintf(payload, sizeof(payload),
        "AURA\tRF:%u:%s:%s:%u:%c:%u:%d:%d",
        spell->Id,
        srcBuf,
        dstBuf,
        spell->School,
        holder->IsPositive() ? 'B' : 'D',
        holder->GetStackAmount(),
        holder->GetAuraMaxDuration(),
        holder->GetAuraDuration());

    WorldPacket data;
    BuildAddonPacket(data, payload);
    SendToNearbyGroup(target, data);
}

void AuraTracker::SendAuraDose(SpellAuraHolder const* holder, bool increased)
{
    if (!IsEnabled() || !ShouldTrack(holder))
        return;

    SpellEntry const* spell = holder->GetSpellProto();
    Unit const* target = holder->GetTarget();

    char srcBuf[24], dstBuf[24];
    FormatGuid(holder->GetCasterGuid(), srcBuf, sizeof(srcBuf));
    FormatGuid(target->GetObjectGuid(), dstBuf, sizeof(dstBuf));

    char payload[256];
    snprintf(payload, sizeof(payload),
        "AURA\t%s:%u:%s:%s:%u:%c:%u:%d:%d",
        increased ? "DS" : "DR",
        spell->Id,
        srcBuf,
        dstBuf,
        spell->School,
        holder->IsPositive() ? 'B' : 'D',
        holder->GetStackAmount(),
        holder->GetAuraMaxDuration(),
        holder->GetAuraDuration());

    WorldPacket data;
    BuildAddonPacket(data, payload);
    SendToNearbyGroup(target, data);
}

void AuraTracker::SendPeriodicTick(Aura const* aura, uint32 amount)
{
    if (!IsEnabled())
        return;

    SpellAuraHolder const* holder = aura->GetHolder();
    if (!ShouldTrack(holder))
        return;

    SpellEntry const* spell = aura->GetSpellProto();
    Unit const* target = aura->GetTarget();

    char srcBuf[24], dstBuf[24];
    FormatGuid(holder->GetCasterGuid(), srcBuf, sizeof(srcBuf));
    FormatGuid(target->GetObjectGuid(), dstBuf, sizeof(dstBuf));

    char payload[256];
    snprintf(payload, sizeof(payload),
        "AURA\tT:%u:%s:%s:%u:%c:%u:%u:%d",
        spell->Id,
        srcBuf,
        dstBuf,
        spell->School,
        holder->IsPositive() ? 'B' : 'D',
        holder->GetStackAmount(),
        amount,
        holder->GetAuraDuration());

    WorldPacket data;
    BuildAddonPacket(data, payload);
    SendToNearbyGroup(target, data);
}
