# Bot AI Architecture Proposal: Behavior Layer System

## Problem Statement

The current PartyBot AI is a monolithic priority-list system spread across 12 files and ~9,300 lines of C++. Each of the 9 class files is a self-contained `if/else` chain that handles everything: positioning, healing, threat management, CC, interrupts, and damage rotation — all interleaved together.

This creates three recurring problems:

1. **Behavioral gaps from copy-paste divergence.** Each class re-implements shared concerns independently. A healer druid has no kiting logic because the kiting code lives in the Balance branch. A Shadow Priest won't emergency-heal a dying tank because the heal check is in the Holy branch. Every class has its own version of "stay at 25 yards" and "interrupt the boss" — or doesn't, if the author forgot.

2. **Boss mechanics require manual cataloging.** The Boss Override Registry hardcodes spell IDs and response types for 20+ encounters. Every new dungeon boss needs a developer to research spell IDs, decide on response types, and add entries. Bots are helpless against uncataloged bosses.

3. **Role transitions are all-or-nothing.** A bot is exactly one of ROLE_TANK / ROLE_HEALER / ROLE_MELEE_DPS / ROLE_RANGE_DPS. A feral druid can't offtank while occasionally throwing a heal. A Shadow Priest can't toss a Flash Heal when the healer is OOM. The role enum gates entire code branches.

## Proposed Architecture

Split the AI into three distinct layers that communicate through a shared evaluation context:

```
┌─────────────────────────────────��───────────────────────┐
│                    SITUATION LAYER                       │
│  Reads world state → produces a SituationContext struct  │
│  (threat ratios, incoming damage, party health,         │
│   mob cast bars, positional danger, mana pressure)      │
└────────────────────────┬────────────────────────────────┘
                         │ SituationContext
                         ▼
┌─────────────────────────────────────────────────────────┐
│                    BEHAVIOR LAYER                        │
│  Composable behavior modules with dynamic priorities     │
│  BehaviorKite, BehaviorInterrupt, BehaviorHealParty,    │
│  BehaviorApplyDots, BehaviorTankThreat, etc.            │
│  Each scores itself against SituationContext → best wins │
└────────────────────────┬────────────────────────────────┘
                         │ Selected BotAction
                         ▼
┌─────────────────────────────────────────────────────────┐
│                   EXECUTION LAYER                        │
│  Class-specific spell resolution                         │
│  "Interrupt" → Kick (Rogue), Counterspell (Mage), etc. │
│  "HealDirect" → Flash Heal (Priest), Healing Touch, etc│
│  "Kite" → Blink+Frost Nova (Mage), Entangling Roots... │
└─────────────────────────────────────────────────────────┘
```

### Layer 1: Situation Layer

A single function that runs once per AI tick and produces a read-only snapshot of everything the bot can observe. This replaces the dozens of ad-hoc checks scattered across class files.

```cpp
struct SituationContext
{
    // Self
    float healthPct;
    float manaPct;
    bool  isBeingMeleed;          // any attacker in melee range
    uint8 meleeAttackerCount;     // attackers within 10yd
    bool  isMoving;
    bool  isCasting;

    // Threat
    float threatRatio;            // my threat / tank's threat on current target
    bool  shouldThrottle;         // threatRatio >= 0.90

    // Target
    Unit* currentTarget;
    float targetHealthPct;
    float distanceToTarget;
    bool  targetIsCasting;        // interruptible cast in progress
    uint32 targetCastSpellId;
    uint32 targetCastTimeLeft;    // ms remaining

    // Party
    float tankHealthPct;          // 0 if no tank or tank dead
    bool  tankIsAlive;
    float lowestAllyHealthPct;
    Unit* mostInjuredAlly;
    uint8 alivePartyMembers;
    uint8 deadPartyMembers;
    bool  anyAllyNeedsHeal;       // anyone below 70%
    bool  criticalHealNeeded;     // anyone below 30%

    // Encounter
    CombatEventBuffer const* recentEvents;  // ring buffer ref
    LearnedPattern const* activePattern;    // if target is casting a known-dangerous spell
    bool  isBossEncounter;

    // Environment
    bool  inDungeon;
    bool  inRaid;
};
```

**Why a struct, not scattered checks:** Every behavior module needs overlapping subsets of this data. Computing it once and passing it through eliminates redundant work (today `FindPartyTank()` is called independently by 6+ different code paths per tick) and ensures all behaviors see the same snapshot.

### Layer 2: Behavior Layer

Replace the monolithic `if/else` chains with composable **behavior modules**. Each module represents one tactical concern (interrupt, kite, heal, apply DoTs, etc.) and knows how to score its own urgency.

```cpp
// Abstract base for all behavior modules
struct BotBehavior
{
    virtual ~BotBehavior() = default;

    // How important is this behavior right now? 0 = irrelevant, 100 = life-or-death.
    virtual float Score(SituationContext const& ctx) const = 0;

    // Attempt to execute. Returns true if an action was taken (spell cast, movement started).
    virtual bool Execute(SituationContext const& ctx, BotSpellKit const& kit) = 0;

    // Can this bot even use this behavior? (checked once at init, not per tick)
    virtual bool IsApplicable(BotSpellKit const& kit) const = 0;

    // Behavior identity for debugging / logging
    virtual char const* GetName() const = 0;
};
```

The AI tick becomes:

```cpp
void PartyBotAI::UpdateInCombatAI()
{
    SituationContext ctx = EvaluateSituation();

    // Score all behaviors, pick the highest
    BotBehavior* best = nullptr;
    float bestScore = 0.0f;

    for (auto* behavior : m_behaviors)
    {
        float score = behavior->Score(ctx);
        if (score > bestScore)
        {
            bestScore = score;
            best = behavior;
        }
    }

    if (best)
        best->Execute(ctx, m_spellKit);
}
```

#### Example Behavior Modules

**BehaviorInterrupt** — universal across all classes:
```cpp
float Score(SituationContext const& ctx) const override
{
    if (!ctx.targetIsCasting)
        return 0.0f;

    // Known dangerous spell from ring buffer patterns?
    if (ctx.activePattern && ctx.activePattern->confidence >= 2)
        return 95.0f;   // Near-maximum priority

    // Boss override interrupt list? (backward compat)
    if (m_bossOverride && IsInInterruptList(ctx.targetCastSpellId))
        return 95.0f;

    // Generic interruptible heal or big nuke?
    if (IsHealSpell(ctx.targetCastSpellId))
        return 80.0f;

    return 0.0f;
}

bool Execute(SituationContext const& ctx, BotSpellKit const& kit) override
{
    // kit.GetInterrupt() resolves to class-specific spell:
    // Kick, Counterspell, Earth Shock, Pummel, etc.
    if (SpellEntry const* interrupt = kit.GetInterrupt())
        return TryCast(ctx.currentTarget, interrupt);
    return false;
}
```

**BehaviorKite** — any caster being melee'd:
```cpp
float Score(SituationContext const& ctx) const override
{
    if (!ctx.isBeingMeleed)
        return 0.0f;
    if (IsRoleMelee())
        return 0.0f;

    // Scale urgency with health: lower health = more urgent to escape
    // At 100% HP: score 30 (low priority, keep casting)
    // At 30% HP: score 75 (high priority, need to move)
    return 30.0f + (70.0f - ctx.healthPct) * 0.65f;
}

bool Execute(SituationContext const& ctx, BotSpellKit const& kit) override
{
    // Try root first
    if (SpellEntry const* root = kit.GetRoot())
    {
        if (TryCast(ctx.currentTarget, root))
            return true;
    }

    // Then just run toward tank
    RunTowardSafePosition();
    return true;
}
```

**BehaviorEmergencyHeal** — any class with a heal, regardless of role:
```cpp
float Score(SituationContext const& ctx) const override
{
    if (!ctx.criticalHealNeeded)
        return 0.0f;

    // Healers always respond to critical heal needs
    if (IsRoleHealer())
        return 90.0f;

    // Non-healers only emergency heal when the actual healer can't
    // (dead, OOM, or CC'd)
    if (ctx.tankHealthPct < 15.0f && !IsHealerAvailable())
        return 85.0f;

    return 0.0f;
}
```

**BehaviorApplyDots** — DoT classes only:
```cpp
float Score(SituationContext const& ctx) const override
{
    if (ctx.shouldThrottle)
        return 0.0f;

    // Higher score on high-HP targets (DoTs have time to tick)
    if (ctx.targetHealthPct > 60.0f && HasUnappliedDots(ctx))
        return 55.0f;

    // Lower score on low-HP targets (direct damage better)
    if (ctx.targetHealthPct < 30.0f)
        return 15.0f;

    return HasUnappliedDots(ctx) ? 45.0f : 0.0f;
}
```

#### Behavior List Per Role

Each role gets a different set of behaviors with different base weights:

| Behavior | Tank | Healer | Melee DPS | Range DPS |
|----------|------|--------|-----------|-----------|
| BehaviorInterrupt | Yes | - | Yes | Yes |
| BehaviorTankThreat | Yes | - | - | - |
| BehaviorTaunt | Yes | - | - | - |
| BehaviorHealParty | - | Yes | - | - |
| BehaviorEmergencyHeal | - | Yes | Yes* | Yes* |
| BehaviorKite | - | Yes | - | Yes |
| BehaviorApplyDots | - | - | Yes | Yes |
| BehaviorDirectDamage | - | - | Yes | Yes |
| BehaviorAoeDamage | - | - | Yes | Yes |
| BehaviorCrowdControl | - | - | Yes | Yes |
| BehaviorThreatDump | - | - | Yes | Yes |
| BehaviorPosition | Yes | Yes | Yes | Yes |
| BehaviorSelfPreserve | - | Yes | Yes | Yes |

*Only if the bot has healing spells (hybrid classes).

### Layer 3: Execution Layer (BotSpellKit)

The execution layer maps abstract actions to class-specific spells. This replaces the per-class spell pointer structs with a capability-oriented interface:

```cpp
struct BotSpellKit
{
    // Populated from existing PopulateSpellData, reorganized by capability
    SpellEntry const* GetInterrupt() const;      // Kick, Counterspell, Pummel...
    SpellEntry const* GetRoot() const;            // Frost Nova, Entangling Roots...
    SpellEntry const* GetSilence() const;         // Silence, Spell Lock...
    SpellEntry const* GetTaunt() const;           // Taunt, Growl, Hand of Reckoning...

    // Healing: returns best spell for the deficit
    SpellEntry const* GetDirectHeal(int32 missingHealth) const;
    SpellEntry const* GetPeriodicHeal() const;

    // Damage
    SpellEntry const* GetFiller() const;          // Shadow Bolt, Frostbolt, Wrath...
    SpellEntry const* GetInstantDot() const;      // Corruption, SW:Pain, Moonfire...
    SpellEntry const* GetCastDot() const;         // Immolate, etc.
    SpellEntry const* GetAoe() const;             // Rain of Fire, Blizzard...
    SpellEntry const* GetExecute() const;         // Shadowburn, Execute...

    // Utility
    SpellEntry const* GetSelfBuff() const;        // Demon Armor, Ice Armor...
    SpellEntry const* GetCrowdControl() const;    // Polymorph, Banish, Hibernate...
    SpellEntry const* GetThreatDump() const;      // Feint, Cower, Fade...

    // Movement
    SpellEntry const* GetEscapeAbility() const;   // Blink, Vanish, Dash...
};
```

This is a thin adapter over the existing `m_spells` union. No spell data changes — just a uniform way to ask "give me your interrupt" without knowing the class.

### Layer 4 (Optional): Combat Event Ring Buffer

Observation-based encounter learning, as discussed earlier:

```cpp
struct CombatEvent
{
    uint32 timestamp;
    ObjectGuid source;
    ObjectGuid target;
    uint32 spellId;
    uint8  type;          // CAST_START, DAMAGE, AURA_APPLY, DEATH
    int32  value;         // damage or heal amount
};

class CombatEventBuffer
{
    CombatEvent m_events[256];
    uint16 m_head = 0;

public:
    void Push(CombatEvent const& e);
    void ScanForPatterns(std::vector<LearnedPattern>& out) const;
};

struct LearnedPattern
{
    uint32 triggerSpellId;
    uint32 responseDelay;
    uint8  suggestedResponse;   // INTERRUPT, MOVE_AWAY, SPREAD
    uint8  confidence;          // incremented each time pattern repeats
    float  damageRatio;         // damage / victim max HP
};
```

Patterns are detected by scanning backwards from a significant event (big damage or death) to find a precursor cast. The ring buffer lives at the party level (shared across all bots in the group) since one bot's observation benefits everyone.

This feeds into `SituationContext::activePattern` which `BehaviorInterrupt` and `BehaviorSelfPreserve` already check.

---

## Pros and Cons

### Pros

**Eliminates behavioral gaps by construction.** `BehaviorKite` exists once and every caster gets it. No more "the Druid healer forgot to run away." If you add a new escape mechanic to BehaviorKite, all 5 caster classes benefit immediately.

**Hybrid class behavior emerges naturally.** A Feral Druid with `BehaviorEmergencyHeal` will pop out of form and heal when the healer dies — not because someone wrote that specific case, but because the emergency heal behavior scores higher than the melee damage behavior when a party member is at 15% HP and no healer is available.

**Boss encounters generalize.** The ring buffer means bots learn from wipes. First pull: "that spell did 60% of the tank's HP, preceded by spell X." Second pull: "I should interrupt spell X." No developer intervention needed for new bosses.

**Scoring is tunable.** If healers are healing too aggressively and not doing enough damage, lower `BehaviorHealParty`'s base score. If DPS isn't interrupting enough, raise `BehaviorInterrupt`'s base score. These are single numeric constants, not structural code changes.

**Dramatically reduces code duplication.** The 9 class files (3,706 lines total) mostly implement the same ~12 tactical decisions with different spells. The behavior layer expresses each decision once. The class files shrink to just `BotSpellKit` mappings.

**Backward compatible migration path.** The `BotSpellKit` wraps the existing `m_spells` union — no spell data migration needed. Boss overrides can feed directly into the ring buffer's `LearnedPattern` table as "pre-learned" patterns with max confidence. Migration can be done one class at a time.

### Cons

**Scoring is a new design surface.** The current system has no ambiguity about priority — it's a linear `if/else` chain. With scoring, two behaviors might be close (heal at 48.0 vs. interrupt at 47.5) and the "wrong" one wins. Tuning scores requires playtesting and iteration. This is the single biggest risk.

**Harder to trace "why did the bot do that."** With the current system, you can read the class file top to bottom and know exactly what happened. With behaviors, you need to know the score of every active behavior at that tick. Requires logging infrastructure to debug effectively.

**Performance overhead.** Scoring ~15 behaviors per tick per bot adds CPU cost vs. the current early-return chains. The current system short-circuits the moment it finds work; the behavior system must evaluate all candidates every tick to find the maximum. For 20-40 party bots this is negligible. For 150+ crowd/dynamic bots, it could matter — but those bots use `CrowdBotAI` / `DynamicBotAI`, not `PartyBotAI`, so they're unaffected.

**Ring buffer pattern detection has false positives.** A mob that casts a harmless spell right before an unrelated damage event would get learned as "dangerous." Confidence thresholds mitigate this, but some wrong patterns will be learned. Needs a decay mechanism (reduce confidence over time) and probably a cap on total patterns per creature entry.

**Doesn't address positioning intelligence.** Behaviors like "spread for AoE" need a target position to spread from. "Kite" needs pathfinding awareness (don't run into another pack). The current movement system (`MoveChase`, `RunAwayFromTarget`) is limited. This proposal improves *when* to move but doesn't improve *where* to move.

**New code to write and maintain.** The behavior system is a new abstraction layer. Until all 9 classes are migrated, you'd have two systems running in parallel — the old `if/else` chains for unmigrated classes and the new behavior system for migrated ones. The migration period would be the messiest phase.

---

## Migration Strategy

### Phase 1: Situation Layer (low risk, immediate value)

Extract the ad-hoc checks into `EvaluateSituation()` returning `SituationContext`. The existing class files can read from this struct instead of calling `FindPartyTank()`, `ShouldThrottleDPS()`, `GetAttackersInRangeCount()` independently. This is a pure refactor — behavior doesn't change.

**Files changed:** `CombatBotBaseAI.h` (add struct), `PartyBotAI.cpp` (add `EvaluateSituation()`), all 9 class files (replace ad-hoc checks with `ctx.` reads).

**Estimated scope:** ~400 lines new, ~200 lines removed across class files.

### Phase 2: BotSpellKit adapter (low risk)

Wrap the existing `m_spells` union with the capability-oriented `BotSpellKit` interface. Purely additive — old code still works, new code can use either interface.

**Files changed:** `CombatBotBaseAI.h` (add `BotSpellKit`), `CombatBotBaseAI.cpp` (populate kit from existing spell data).

**Estimated scope:** ~200 lines new, 0 lines removed.

### Phase 3: Extract shared behaviors (medium risk)

Pull the universal behaviors out of class files into standalone modules: `BehaviorInterrupt`, `BehaviorKite`, `BehaviorEmergencyHeal`, `BehaviorPosition`. These run *alongside* the existing class-specific code, injected into the shared `UpdateInCombatAI()` dispatcher where boss overrides currently run. This is where the healer-kiting gap and cross-role emergency healing get fixed.

**Files changed:** New `BotBehaviors.cpp/.h`, modified `PartyBotAI.cpp` (dispatcher), minor removals from class files where behavior now handles the concern.

**Estimated scope:** ~600 lines new, ~300 lines removed from class files.

### Phase 4: Combat Event Ring Buffer (medium risk, can be independent)

Add `CombatEventBuffer` hooked to combat log events. Add `LearnedPattern` detection and persistence. Seed with existing Boss Override Registry data as pre-learned patterns. Can run in parallel with Phase 3.

**Files changed:** New `CombatEventBuffer.cpp/.h`, modified `PartyBotAI.cpp` (hook events, feed to SituationContext).

**Estimated scope:** ~500 lines new.

### Phase 5: Full behavior migration (high effort, per-class)

Migrate class files one at a time from `if/else` chains to behavior sets. Each class file shrinks from ~300-700 lines to ~50-100 lines of `BotSpellKit` mappings plus any class-unique behaviors (e.g., Warlock pet management, Hunter pet control).

**Can be done incrementally** — one class per release. Warrior and Mage are good first candidates (cleanest rotation logic, fewest edge cases).

---

## Proof of Concept: Warrior Migration

To make this concrete, here's how the Warrior — the simplest DPS class — would look under the new system.

### Current Warrior (PartyBotAI_Warrior.cpp — 353 lines)

The existing file handles in one `if/else` chain:
- Berserker Rage (fear break)
- Overpower (dodge proc)
- Execute (low HP target)
- Shield Block / Shield Wall / Last Stand (tank survivability)
- Challenging Shout (AoE taunt)
- Bloodthirst / Mortal Strike (primary attack)
- Whirlwind (AoE)
- Cleave (AoE rage dump)
- Sunder Armor (threat builder)
- Hamstring (for PvP / kiting mobs)
- Battle Shout / Demoralizing Shout (buffs/debuffs)

### Under the Behavior System

**Behaviors attached to a Protection Warrior:**
```
BehaviorTankThreat         — Sunder Armor, Revenge, Heroic Strike
BehaviorTaunt              — Taunt, Challenging Shout
BehaviorInterrupt          — Pummel, Shield Bash
BehaviorSelfPreserve       — Shield Block, Shield Wall, Last Stand
BehaviorApplyDebuff        — Demoralizing Shout, Thunder Clap
BehaviorAoeDamage          — Whirlwind, Cleave (when rage > threshold)
BehaviorFearBreak          — Berserker Rage (warrior-specific)
BehaviorSelfBuff           — Battle Shout, Bloodrage
```

**Behaviors attached to a Fury Warrior:**
```
BehaviorInterrupt          — Pummel (same module, no code change)
BehaviorThreatDump         — (none available for warrior — scores 0)
BehaviorDirectDamage       — Bloodthirst, Heroic Strike
BehaviorExecute            — Execute (< 20% HP)
BehaviorApplyDebuff        — Demoralizing Shout (if no tank applying it)
BehaviorAoeDamage          — Whirlwind, Cleave
BehaviorReactiveProc       — Overpower (on dodge), Revenge (on block)
BehaviorFearBreak          — Berserker Rage
BehaviorSelfBuff           — Battle Shout
```

**Warrior-specific BotSpellKit mappings:**
```cpp
// This is all that remains in the "class file"
void WarriorSpellKit::Populate(CombatBotSpells const& spells)
{
    m_interrupt    = spells.warrior.pPummel;
    m_interruptAlt = spells.warrior.pShieldBash;
    m_taunt        = spells.warrior.pTaunt;
    m_aeTaunt      = spells.warrior.pChallengingShout;
    m_filler       = spells.warrior.pBloodthirst
                   ? spells.warrior.pBloodthirst
                   : spells.warrior.pMortalStrike;
    m_execute      = spells.warrior.pExecute;
    m_aoe          = spells.warrior.pWhirlwind;
    m_debuff       = spells.warrior.pDemoralizingShout;
    m_selfBuff     = spells.warrior.pBattleShout;
    // ... etc
}
```

**What happens during combat (Fury Warrior, single target):**
```
Tick 1: EvaluateSituation() → target at 80% HP, not casting, threat at 0.3
  BehaviorSelfBuff:      Battle Shout missing → score 40
  BehaviorApplyDebuff:   Demo Shout missing → score 35
  BehaviorDirectDamage:  target alive, not throttled → score 50
  BehaviorAoeDamage:     only 1 target → score 0
  → Winner: BehaviorDirectDamage (50) → casts Bloodthirst

Tick 2: target casting a heal, threat at 0.4
  BehaviorInterrupt:     target casting heal → score 80
  BehaviorDirectDamage:  → score 50
  → Winner: BehaviorInterrupt (80) → casts Pummel

Tick 3: target at 18% HP, threat at 0.85
  BehaviorExecute:       target < 20% → score 70
  BehaviorDirectDamage:  → score 50
  BehaviorThreatDump:    no ability → score 0 (warrior just stops if throttled)
  → Winner: BehaviorExecute (70) → casts Execute

Tick 4: threat at 0.92 (above throttle threshold)
  BehaviorDirectDamage:  throttled → score 0
  BehaviorExecute:       throttled → score 0
  BehaviorSelfBuff:      Battle Shout up → score 0
  BehaviorApplyDebuff:   Demo Shout up → score 0
  → Winner: nothing scores > 0 → auto-attack only
```

### What This Buys (Warrior Example)

- **Pummel interrupt** now handled by a shared module. If we improve interrupt timing (e.g., waiting until 70% cast to avoid jukes), all classes benefit.
- **Fear break** is a shared `BehaviorFearBreak` that also handles Tremor Totem (Shaman), WotF (Undead), Berserker Rage (Warrior). Write once.
- **Demo Shout awareness**: `BehaviorApplyDebuff` can check if *anyone* in the party already applied Demo Shout / Thunderclap and skip it. Today each warrior independently applies it.
- **Fury → emergency offtank**: If the main tank dies and the Fury warrior has a shield in bags, `BehaviorSelfPreserve` scores high enough to equip shield + Defensive Stance. This is impossible in the current architecture without explicitly coding it into the Warrior file.

---

## Proof of Concept: Ring Buffer Pattern Learning

### Scenario: First Ragfire Chasm Run (No Boss Overrides)

**Pull 1: Taragaman the Hungerer**

The party engages. Taragaman casts Fire Nova (spell 11969). The ring buffer records:

```
T=0ms     CAST_START  source=Taragaman  spell=11969
T=2000ms  DAMAGE      source=Taragaman  spell=11969  target=Warrior  value=340
T=2000ms  DAMAGE      source=Taragaman  spell=11969  target=Rogue    value=310
T=2000ms  DAMAGE      source=Taragaman  spell=11969  target=Druid    value=280
```

After the damage events, `ScanForPatterns()` runs and detects:
- Spell 11969 hit 3 targets simultaneously within 100ms
- Average damage = 310 (roughly 30% of a level 16's HP)
- Precursor was CAST_START on spell 11969, 2000ms prior

**Learned pattern:**
```
triggerSpellId = 11969
responseDelay  = 2000ms
response       = MOVE_AWAY  (multi-target simultaneous damage = AoE)
confidence     = 1          (first observation)
damageRatio    = 0.30
```

The pull continues and Taragaman casts Fire Nova again:
```
T=15000ms  CAST_START  source=Taragaman  spell=11969
T=17000ms  DAMAGE × 3  (same pattern)
```

**Confidence bumped to 2.** Pattern is now actionable.

**Pull 2: Taragaman (wipe recovery)**

Taragaman starts casting Fire Nova:
```
T=0ms  CAST_START  source=Taragaman  spell=11969
```

`SituationContext` now contains:
```cpp
ctx.activePattern = { spell=11969, response=MOVE_AWAY, confidence=2 }
```

`BehaviorSelfPreserve::Score()` sees `activePattern->response == MOVE_AWAY` and returns 75.0. The melee DPS and tank run out to 13 yards (avoidAoeRange default + buffer). The healer was already at 25yd and is unaffected.

**Result:** The bots learned to avoid Fire Nova without any entry in the Boss Override Registry.

### Scenario: Learned Pattern Persistence

After the run, patterns are written to a DB table:

```sql
CREATE TABLE bot_learned_patterns (
    creature_entry   INT UNSIGNED NOT NULL,
    trigger_spell_id INT UNSIGNED NOT NULL,
    response_type    TINYINT UNSIGNED NOT NULL,  -- 0=INTERRUPT, 1=MOVE_AWAY, 2=SPREAD
    response_delay   INT UNSIGNED NOT NULL,      -- ms
    confidence       TINYINT UNSIGNED NOT NULL,
    damage_ratio     FLOAT NOT NULL,
    PRIMARY KEY (creature_entry, trigger_spell_id)
);
```

On server restart, patterns are loaded back. New bots joining a Ragefire group already know to avoid Fire Nova on their first pull.

### False Positive Mitigation

**Problem:** A mob casts a harmless buff on itself, then a completely unrelated damage event happens. The scanner incorrectly links them.

**Mitigations:**
1. **Minimum damage threshold.** Only scan for patterns when damage exceeds 15% of victim's max HP. Small ticks from DoTs or incidental damage are ignored.
2. **Source matching.** The precursor cast and the damage must come from the same source GUID. A buff on mob A doesn't get linked to damage from mob B.
3. **Confidence decay.** Patterns lose 1 confidence per 10 encounters where the trigger spell is cast but no significant damage follows. False patterns naturally decay to 0 and get pruned.
4. **Manual override.** The existing Boss Override Registry entries are loaded as patterns with `confidence = 255` (permanent). They can never be decayed or overwritten.

---

## What We Keep

This isn't a rewrite — it's a refactor that preserves the core strengths:

- **The per-class spell pointer system** (`m_spells` union, `PopulateSpellData`) stays exactly as-is. `BotSpellKit` wraps it.
- **The shared healing system** (`SelectHealTarget`, `HealInjuredTargetDirect`, etc.) stays. `BehaviorHealParty` calls the same functions.
- **The threat calculation** (`GetThreatRatio`, `ShouldThrottleDPS`) stays. `SituationContext` computes it once and behaviors read from there.
- **Tank detection** (`FindPartyTank`) stays. Computed once in `EvaluateSituation()`.
- **Boss Override Registry** stays as a seed for the ring buffer. Entries become "pre-learned patterns" with permanent confidence.
- **All existing spell logic** (curse overwrite prevention, PW:S warrior rules, warlock pet selection) stays in the execution layer / class-specific behavior hooks.
- **CrowdBotAI and DynamicBotAI** are unaffected — they don't use the PartyBot combat system.

## What Dies

- The 9 per-class `if/else` priority chains (3,706 lines) — replaced by behavior scoring + spell kit mappings
- Duplicated positioning code across class files
- Duplicated interrupt logic across class files
- Hard role gating that prevents hybrid behavior

## Estimated Total Effort

| Phase | New Lines | Removed Lines | Risk |
|-------|-----------|---------------|------|
| 1. SituationContext | ~400 | ~200 | Low |
| 2. BotSpellKit | ~200 | 0 | Low |
| 3. Shared behaviors | ~600 | ~300 | Medium |
| 4. Ring buffer | ~500 | 0 | Medium |
| 5. Class migration (×9) | ~900 | ~3,500 | High (per-class) |
| **Total** | **~2,600** | **~4,000** | |

Net reduction of ~1,400 lines with significantly more capable bot behavior.
