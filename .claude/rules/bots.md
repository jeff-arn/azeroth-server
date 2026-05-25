# Bot AI rules

The bot stack is the unique thing about this project. It has cost real time to get right, and most of the gotchas below come from specific bugs that shipped and then had to be reverted. Read this before touching `PartyBotAI`, `BotBehavior`, `BotFormation`, `BotChatManager`, `BotServiceMgr`, or the AH/Director systems. The full architecture lives in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md); this file is the rules digest.

## Behavior layer architecture (v0.9.0)

Bot AI uses three layers: **situation context → scored behaviors → execute with fallthrough**. The per-class if/else chains that preceded v0.9.0 are gone — do not bring them back.

The runner:

1. `EvaluateSituation()` fills a `SituationContext` snapshot once per tick (HP, mana, threat, target, party state, formation slot, flee target).
2. Each registered `BotBehavior::Score(ctx)` returns 0–100 urgency, multiplied by the role weight from `BotRoleConfig`.
3. `BotBehaviorSet::RunBestBehavior()` sorts by score and tries each in descending order until one's `Execute()` returns `true`.
4. `Execute()` returns `false` when the action can't run (spell on cooldown, out of range, etc.) and the runner falls through to the next scorer.

### Adding behaviors

- New behaviors go in `BotBehaviors_Shared.{h,cpp}` (shared across classes) or `BotBehaviors_ClassUnique.{h,cpp}` (class-specific). Register them in `BotRoleConfig` with a sensible role weight.
- `Score()` must check preconditions, not just `Execute()`. A behavior that returns score >0 but always fails in `Execute()` will dominate the sort order and block lower-scoring behaviors from running. **This is the single most common bug.**
- `Execute()` returns `true` when it actually issued the action, `false` when it couldn't and wants the runner to try the next behavior.
- Don't replace existing behaviors. Add new ones. If an existing behavior is wrong, fix its `Score()` or `Execute()` — don't delete it without checking what it covers for live bots.

### Pre-steps (free actions before scoring)

Some actions are "free" — they happen before behavior scoring, every tick:

- Pet attack command (warlock imp Phase Shift strip, hunter pet)
- Dispel check (`CheckForDispelTargets()`)
- Hunter Auto Shot autorepeat maintenance
- Caster ranged positioning via `AttackStart()` (25yd `MoveChase` for `ROLE_RANGE_DPS` / `ROLE_HEALER`)

If you find yourself wanting to score a "free action" as a behavior, it's probably a pre-step instead.

## The `time_t` rule (critical)

Anything deferred in the bot social/chat system must use `time_t fireTime` (absolute Unix time), **not** a millisecond countdown. The chat update tick runs every 60s on the `WUPDATE_AUCTIONS` timer. A countdown of e.g. `delay -= elapsed_ms` will collapse every queued line of a chain into one burst on the next tick.

**Always**: `msg.fireTime = time(nullptr) + delay_seconds;` then check `if (now >= msg.fireTime) fire();`.

**Never**: `msg.delayMs -= elapsed; if (msg.delayMs <= 0) fire();` — broken at the 60s tick rate.

This rule applies to:
- General chat threading (topic pools, conversation chains, debates, keyword hooks)
- Trade-chat-driven service responses (whisper-after-delay)
- Any new deferred message in `BotChatManager`

## `BotSpellKit` abstraction

`BotSpellKit` (`vmangos/src/game/PlayerBots/BotSpellKit.{h,cpp}`) is the **only** way new behaviors should reference class-specific spells. It maps generic capabilities to the class's `m_spells` union:

- `GetInterrupt()` — Kick, Pummel, Counterspell, Earth Shock, etc.
- `GetFiller()` — Wand shot, auto-shot, basic damage cycle
- `GetTaunt()` — class-appropriate taunt
- `GetDispel()` — dispel-magic-style ability
- (More — see the header)

New behaviors must go through `BotSpellKit`. Direct references to e.g. `m_spells.warrior.Pummel` from a shared behavior re-introduce per-class coupling and break the layer.

When you add a new capability (e.g. `GetSelfShield()`), wire it in `BotSpellKit` for every class that has one — even if only one class implements it today. The whole point is that behaviors don't know which class they're running on.

## Movement and formation

The formation system in `BotFormation.{h,cpp}` replaced the old random-angle `MoveFollow`. It uses a breadcrumb trail (ring buffer, 40 points, 1s sampling, >2yd movement threshold) and arc-length interpolation to place bots in role-ordered slots behind the leader.

### Movement primitives

- `MovePoint(...)` — one-shot pathfinding move to a coordinate. Use for **slot-based formation**, kite destinations, and rogue stealth-opener behind-target positioning.
- `MoveChase(...)` — continuous chase with a follow distance. Use for **combat positioning** (25yd for ranged/healer, 1yd for melee/tank).
- `MoveFollow(...)` — legacy continuous follow. Used as a fallback when the breadcrumb trail is empty (first few ticks after spawn or teleport). Not used for normal formation movement.

`MoveChase`'s angle parameter is a pathfinding hint, not a positional constraint. If you need a precise position relative to a target (e.g. behind it), use `MovePoint` to a calculated coordinate.

### Formation must NOT fire during combat

`BehaviorPosition` and `BehaviorKite` only consume formation data when `!ctx.currentTarget`. During combat, the existing `MoveChase` distances apply. **This is a known regression source** — the first formation pass had bots pulled toward their trail slot during combat and stacked on mobs.

### Lateral offset safety

When placing a bot to the side of the trail center, run two checks:

1. **Navmesh validation**: `Map::GetWalkHitPosition()` from trail center to the candidate lateral position. If the walk-hit moves the destination >3yd from where you asked, collapse to trail center (corridor too narrow).
2. **Aggro safety scan**: `AnyHostileUnitInObjectRangeCheck` within 12yd of the candidate. If hostiles found, collapse to trail center. Navmesh checks walkability, not whether a position is dangerously close to live mobs.

### Other gotchas

- `POINT_MOTION_TYPE` must be excluded from the main loop's `needFollow` check alongside `FOLLOW_MOTION_TYPE`. Otherwise the follow logic immediately overwrites formation movement.
- `BreadcrumbTrail::Clear()` on teleport (>100yd jump). Otherwise bots try to walk back through the gap.
- `POINT_MOTION_TYPE` arrival generates a `MovementInform` — do not rely on `MoveChase` arrival semantics for `MovePoint` destinations.

## Role and tank detection

`FindPartyTank()` is the canonical source of truth. Priority order:

1. Explicit `.partyrole` assignment via the `s_playerRoleAssignments` static map
2. Bot AI `ROLE_TANK` for bots
3. Heuristics: Bear Form or shield equipped

**Critical**: a human player with an explicit non-tank role skips the heuristics. Otherwise a paladin DPS with a shield was being misidentified as the tank. Whenever you add a new tank-detection path, preserve this gate.

`.partyrole tank|healer|dps|clear` is available to all players (`SEC_PLAYER`). When the role changes, you must set `m_resetSpellData = true` to trigger the behavior set rebuild — otherwise the bot keeps the old weights.

## Class- and role-specific rules

### Healers
- Follow the leader at 12–18yd during combat (not 3–6yd). Configured in role-weighted behaviors.
- `BehaviorPosition` and `BehaviorWandFiller` fall back to the **tank's target** when the healer has no victim of their own.

### Casters
- Call `AttackStop()` every tick to suppress melee auto-attack. Without this they walk into melee range to autoattack between spells.
- Never call `AttackStop()` before `EvaluateSituation()` — it clears `m_attacking` (the victim reference) and the context will see a stale state.

### Rogues (stealth opener)
- `AttackStart()` skips `me->Attack()` while stealthed (preserves stealth — otherwise the autoattack flag breaks stealth on the next tick).
- `BehaviorRogueStealthOpener` calculates a point behind the target via `MovePoint` and checks `IsBehindTarget()` before opening.
- Main loop skips follow logic when stealthed with a target set.

### Priests (PW:S rules)
- **Never** cast Power Word: Shield on warrior tanks above 15% HP. The mob loses threat from the shield's threat modifier and the warrior loses rage generation.
- The 15% emergency exception applies in three places: pre-pull, in-combat critical threshold (<30% HP), and `SelectShieldTarget()`. All three checks must agree.

### Warlocks
- `TeachWarlockPetSpells()` in `CombatBotBaseAI.cpp` teaches grimoire abilities (Blood Pact, Fire Shield, Torment, Suffering, Sacrifice, Seduction, Spell Lock) with `ACT_ENABLED` so the pet auto-casts.
- Pet summoning runs in the OOC behavior set so `FindPartyTank()` has group context. Voidwalker is excluded from the summon pool when a tank is present.
- Only one curse per warlock per target. `BehaviorWarlockCurseManage` checks `HasAura()` for every curse spell pointer in `Score()` — returns 0 if any curse is already applied. Priority: Tongues > Elements > Shadow > Agony.
- Curse of Recklessness is banned (it increases the mob's attack power). Fear is also banned in dungeons (too risky).

### Hunters
- Imp Phase Shift (warlock imp, in a hunter party) — `Pet attack` pre-step strips Phase Shift aura via `ToggleCreatureAutocast(4511, false)` before sending the pet to attack.
- Hunter pet attack: pre-step, before scoring.

## Talent and spell population

`LearnPremadeSpecForClass()` in `CombatBotBaseAI.cpp` is the source of truth for bot specs at every level 1–60. It replaced the old DB-template approach. The flow on spawn:

1. Pick a `BotTalentBuild` matching the bot's role (or random if no role set; the role is derived from the build)
2. `ResetTalents()` + walk the build's ordered talent point array, calling `LearnTalent()` per entry until out of points
3. `LearnClassSpellsForLevel()` iterates `SkillLineAbility` and learns all class trainer spells at or below the bot's level (filtered by spell family, skips talents)
4. `PopulateSpellData()` scans learned spells by name and populates the spell pointers in `m_spells`

The 26 PvE talent builds (9 classes × 2–3 specs) live as static `BotTalentBuild` arrays generated by `scripts/generate_talent_builds.py` from DBC files. Regenerate them if you change the source data; don't hand-edit the C++ arrays.

`LearnRandomTalents()` is kept as a legacy fallback. Don't make it the primary path again.

The old `player_premade_spell_template` DB tables are still used for **gear** only — not talents or spells.

## Player vs bot identification

`account ID < 100` = human, `>= 100` = bot. Used throughout filtering logic. Specific bot accounts:

- 97 — CrowdBot
- 98 — GuildBot
- 99 — AHBot simulated sellers
- 100+ — PartyBot, DynamicBot

When you add a new bot category, allocate a fresh account ID and document it in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md). Don't reuse account IDs.

## Debugging

`.partybot debug` (`SEC_MODERATOR`) toggles per-tick behavior logging for the selected bot. Output goes to `Server.log` as `[BotAI]` lines with all non-zero scores and the winning behavior + result. Use this before assuming a behavior is broken — often the answer is "another behavior scored higher and ran first."

## Anti-patterns (these have shipped and been reverted)

- Re-introducing per-class polymorphism inside scored behaviors. Use `BotSpellKit`.
- Calling `MoveFollow` for formation positioning. Use `MovePoint` to a calculated slot.
- Countdown ms timers for deferred chat. Use `time_t fireTime`.
- Setting the legacy `m_isBuffing` flag from inside a behavior. The main `UpdateAI` loop reads it — behaviors must not touch it.
- Casting PW:S on the warrior tank above 15% HP.
- Hand-editing `BotTalentBuild` arrays instead of regenerating from `scripts/generate_talent_builds.py`.
