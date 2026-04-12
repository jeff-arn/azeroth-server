#ifndef MANGOS_BOT_SPELL_KIT_H
#define MANGOS_BOT_SPELL_KIT_H

#include "Common.h"
#include <vector>

struct SpellEntry;
class CombatBotBaseAI;

// Abstract capability interface over the m_spells union.
// Maps generic tactical actions to class-specific spells.
// Does not own or modify spell data — pure read-only adapter.
class BotSpellKit
{
public:
    BotSpellKit(uint8 classId, CombatBotBaseAI* ai);

    // === Interrupt ===
    SpellEntry const* GetInterrupt() const;
    SpellEntry const* GetInterruptAlt() const; // backup interrupt (Shield Bash, etc.)

    // === Healing ===
    // These delegate to the existing shared healing system in CombatBotBaseAI
    CombatBotBaseAI* GetAI() const { return m_ai; }

    // === Damage ===
    SpellEntry const* GetFiller() const;            // Primary nuke/attack
    SpellEntry const* GetExecuteAbility() const;    // Sub-20% finisher
    SpellEntry const* GetAoeSpell() const;          // AoE damage
    void GetDotSpells(std::vector<SpellEntry const*>& out) const; // All available DoTs

    // === CC / Utility ===
    SpellEntry const* GetCrowdControl() const;
    SpellEntry const* GetRoot() const;
    SpellEntry const* GetThreatDump() const;        // Feint, Fade, Cower, FD

    // === Buffs ===
    SpellEntry const* GetSelfBuff() const;          // Demon Armor, Ice Armor, etc.
    SpellEntry const* GetPartyBuff() const;         // MotW, Fort, AI, etc.

    // === Tank ===
    SpellEntry const* GetTaunt() const;
    SpellEntry const* GetAoeTaunt() const;

    // === Emergency ===
    SpellEntry const* GetEmergencyAbility() const;  // Ice Block, Last Stand, etc.

    // === Movement / Escape ===
    SpellEntry const* GetEscapeAbility() const;     // Blink, Vanish, Dash, etc.

    // === Class identity ===
    uint8 GetClass() const { return m_classId; }

private:
    uint8 m_classId;
    CombatBotBaseAI* m_ai;
};

#endif
