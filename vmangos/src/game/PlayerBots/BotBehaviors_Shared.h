#ifndef MANGOS_BOT_BEHAVIORS_SHARED_H
#define MANGOS_BOT_BEHAVIORS_SHARED_H

#include "BotBehavior.h"

class CombatBotBaseAI;
class PartyBotAI;

// All shared behaviors store a pointer to the bot AI for access to
// me, m_spells, DoCastSpell(), CanTryToCastSpell(), and utilities.

// 1. Self-Preserve: emergency cooldowns when HP is critically low
class BehaviorSelfPreserve : public BotBehavior
{
public:
    BehaviorSelfPreserve(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "SelfPreserve"; }
private:
    CombatBotBaseAI* m_ai;
};

// 2. Interrupt: interrupt target's casting
class BehaviorInterrupt : public BotBehavior
{
public:
    BehaviorInterrupt(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Interrupt"; }
private:
    CombatBotBaseAI* m_ai;
};

// 3. Taunt: grab mobs attacking non-tank party members
class BehaviorTaunt : public BotBehavior
{
public:
    BehaviorTaunt(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Taunt"; }
private:
    PartyBotAI* m_ai;
};

// 4. HealParty: heal injured party members (healer role)
class BehaviorHealParty : public BotBehavior
{
public:
    BehaviorHealParty(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "HealParty"; }
private:
    CombatBotBaseAI* m_ai;
};

// 5. EmergencyHeal: hybrid classes heal when healer is unavailable
class BehaviorEmergencyHeal : public BotBehavior
{
public:
    BehaviorEmergencyHeal(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "EmergencyHeal"; }
private:
    CombatBotBaseAI* m_ai;
};

// 6. TankThreat: build and maintain threat on target (tank role)
class BehaviorTankThreat : public BotBehavior
{
public:
    BehaviorTankThreat(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "TankThreat"; }
private:
    CombatBotBaseAI* m_ai;
};

// 7. ThreatDump: reduce threat when close to pulling aggro
class BehaviorThreatDump : public BotBehavior
{
public:
    BehaviorThreatDump(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "ThreatDump"; }
private:
    CombatBotBaseAI* m_ai;
};

// 8. CrowdControl: CC secondary targets
class BehaviorCrowdControl : public BotBehavior
{
public:
    BehaviorCrowdControl(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "CrowdControl"; }
private:
    PartyBotAI* m_ai;
};

// 9. Kite: escape melee range when being attacked as a caster
class BehaviorKite : public BotBehavior
{
public:
    BehaviorKite(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Kite"; }
private:
    PartyBotAI* m_ai;
};

// 10. Execute: use execute-range abilities on low HP targets
class BehaviorExecuteTarget : public BotBehavior
{
public:
    BehaviorExecuteTarget(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Execute"; }
private:
    bool kit_has_execute(SituationContext const& ctx) const;
    CombatBotBaseAI* m_ai;
};

// 11. ApplyDots: apply damage-over-time spells to target
class BehaviorApplyDots : public BotBehavior
{
public:
    BehaviorApplyDots(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "ApplyDots"; }
private:
    CombatBotBaseAI* m_ai;
};

// 12. AoeDamage: use AoE when 3+ enemies present
class BehaviorAoeDamage : public BotBehavior
{
public:
    BehaviorAoeDamage(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "AoeDamage"; }
private:
    CombatBotBaseAI* m_ai;
};

// 13. DirectDamage: filler nuke/attack (default when nothing else scores higher)
class BehaviorDirectDamage : public BotBehavior
{
public:
    BehaviorDirectDamage(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "DirectDamage"; }
private:
    CombatBotBaseAI* m_ai;
};

// 14. SelfBuff: apply missing self-buffs
class BehaviorSelfBuff : public BotBehavior
{
public:
    BehaviorSelfBuff(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "SelfBuff"; }
private:
    CombatBotBaseAI* m_ai;
};

// 15. Position: move to optimal range (ranged stay at 25yd, melee chase)
class BehaviorPosition : public BotBehavior
{
public:
    BehaviorPosition(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Position"; }
private:
    CombatBotBaseAI* m_ai;
};

// 16. Dispel: remove debuffs from party members
class BehaviorDispel : public BotBehavior
{
public:
    BehaviorDispel(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "Dispel"; }
private:
    PartyBotAI* m_ai;
};

// 17. PartyBuff: apply party buffs to group members (OOC priority)
class BehaviorPartyBuff : public BotBehavior
{
public:
    BehaviorPartyBuff(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "PartyBuff"; }
private:
    CombatBotBaseAI* m_ai;
};

// 18. WandFiller: healers wand the tank's target when idle in combat
class BehaviorWandFiller : public BotBehavior
{
public:
    BehaviorWandFiller(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WandFiller"; }
private:
    PartyBotAI* m_ai;
};

// 19. AttackStop: casters stop melee auto-attack (runs every tick, very high score)
class BehaviorCasterAttackStop : public BotBehavior
{
public:
    BehaviorCasterAttackStop(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "CasterAttackStop"; }
private:
    CombatBotBaseAI* m_ai;
};

#endif
