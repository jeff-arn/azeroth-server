#ifndef MANGOS_BOT_BEHAVIORS_CLASS_UNIQUE_H
#define MANGOS_BOT_BEHAVIORS_CLASS_UNIQUE_H

#include "BotBehavior.h"

class CombatBotBaseAI;
class PartyBotAI;

// ============================================================
// Warrior (2)
// ============================================================

class BehaviorWarriorStanceDance : public BotBehavior
{
public:
    BehaviorWarriorStanceDance(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WarriorStanceDance"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorWarriorRageDump : public BotBehavior
{
public:
    BehaviorWarriorRageDump(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WarriorRageDump"; }
private:
    CombatBotBaseAI* m_ai;
};

// ============================================================
// Rogue (3)
// ============================================================

class BehaviorRogueStealthOpener : public BotBehavior
{
public:
    BehaviorRogueStealthOpener(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "RogueStealthOpener"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorRogueComboFinisher : public BotBehavior
{
public:
    BehaviorRogueComboFinisher(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "RogueComboFinisher"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorRogueStealth : public BotBehavior
{
public:
    BehaviorRogueStealth(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "RogueStealth"; }
private:
    PartyBotAI* m_ai;
};

class BehaviorRoguePoisonWeapons : public BotBehavior
{
public:
    BehaviorRoguePoisonWeapons(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "RoguePoisonWeapons"; }
private:
    CombatBotBaseAI* m_ai;
};

// ============================================================
// Hunter (3)
// ============================================================

class BehaviorHunterPetManage : public BotBehavior
{
public:
    BehaviorHunterPetManage(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "HunterPetManage"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorHunterAspectSwap : public BotBehavior
{
public:
    BehaviorHunterAspectSwap(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "HunterAspectSwap"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorHunterDeadZone : public BotBehavior
{
public:
    BehaviorHunterDeadZone(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "HunterDeadZone"; }
private:
    PartyBotAI* m_ai;
};

// ============================================================
// Paladin (2)
// ============================================================

class BehaviorPaladinBlessing : public BotBehavior
{
public:
    BehaviorPaladinBlessing(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "PaladinBlessing"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorPaladinSealJudge : public BotBehavior
{
public:
    BehaviorPaladinSealJudge(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "PaladinSealJudge"; }
private:
    CombatBotBaseAI* m_ai;
};

// ============================================================
// Shaman (1)
// ============================================================

class BehaviorShamanTotemManage : public BotBehavior
{
public:
    BehaviorShamanTotemManage(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "ShamanTotemManage"; }
private:
    CombatBotBaseAI* m_ai;
};

// ============================================================
// Priest (2)
// ============================================================

class BehaviorPriestShadowformToggle : public BotBehavior
{
public:
    BehaviorPriestShadowformToggle(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "PriestShadowformToggle"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorPriestShieldLogic : public BotBehavior
{
public:
    BehaviorPriestShieldLogic(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "PriestShieldLogic"; }
private:
    PartyBotAI* m_ai;
};

// ============================================================
// Warlock (3)
// ============================================================

class BehaviorWarlockPetSummon : public BotBehavior
{
public:
    BehaviorWarlockPetSummon(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WarlockPetSummon"; }
private:
    PartyBotAI* m_ai;
};

class BehaviorWarlockCurseManage : public BotBehavior
{
public:
    BehaviorWarlockCurseManage(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WarlockCurseManage"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorWarlockLifeTap : public BotBehavior
{
public:
    BehaviorWarlockLifeTap(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "WarlockLifeTap"; }
private:
    CombatBotBaseAI* m_ai;
};

// ============================================================
// Druid (3)
// ============================================================

class BehaviorDruidFormShift : public BotBehavior
{
public:
    BehaviorDruidFormShift(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "DruidFormShift"; }
private:
    CombatBotBaseAI* m_ai;
};

class BehaviorDruidRebirth : public BotBehavior
{
public:
    BehaviorDruidRebirth(PartyBotAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "DruidRebirth"; }
private:
    PartyBotAI* m_ai;
};

class BehaviorDruidInnervate : public BotBehavior
{
public:
    BehaviorDruidInnervate(CombatBotBaseAI* ai) : m_ai(ai) {}
    float Score(SituationContext const& ctx) const override;
    bool Execute(SituationContext const& ctx, BotSpellKit& kit) override;
    char const* GetName() const override { return "DruidInnervate"; }
private:
    CombatBotBaseAI* m_ai;
};

#endif
