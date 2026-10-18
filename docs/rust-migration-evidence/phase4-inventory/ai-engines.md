# Phase 4 Inventory — Creature AI + Script Engines

Oracle: C++ at checkout HEAD `1bec69f` (`vmangos/src/`). Production binary
`2bae5f9` not inspected for divergence in this surface; no divergence claim
is made. All anchors are `file:line` in the checkout.

Naming correction up front: there is **no** `ReactorAI`/`AggressorAI` class
in this tree and **no** SmartScript equivalent (see §a.5). The
passive/reactive/aggressive distinction is `ReactStates`
(`REACT_PASSIVE=0, REACT_DEFENSIVE=1, REACT_AGGRESSIVE=2`,
[UnitDefines.h:730](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/UnitDefines.h:730))
stored per-creature (`Creature::m_reactState`, default `REACT_DEFENSIVE`),
and the generic combat brain is `BasicAI`. DB scripts are **not** in
`dbscripts_on_*` tables in this fork; they live in eleven `*_scripts`
tables loaded by one generic loader (§a.3).

## (a) Verified structure / flow

### a.1 CreatureAI interface and selection

`CreatureAI` is a ~45-virtual interface owned by `Creature`
(`Creature* const m_creature`,
[CreatureAI.h:50](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.h:50),
[CreatureAI.h:208](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.h:208)).
Lifecycle/reaction hooks: `MoveInLineOfSight`, `EnterCombat`,
`EnterEvadeMode`, `OnCombatStop`, `JustReachedHome`, `JustDied`,
`JustRespawned`, `KilledUnit`, `AttackedBy`, `SpellHit`,
`SpellHitTarget`, `MovementInform`, `ReceiveEmote`,
`OnScriptEventHappened`, `SummonedCreatureJustDied/Despawn`,
`GroupMemberJustDied`, `OnMoveInStealth`, `UpdateAI(diff)`,
`UpdateAI_corpse(diff)`, `AttackStart`
([CreatureAI.h:66](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.h:66)–156).
Shared helpers live on the base: `DoMeleeAttackIfReady`,
`DoCastSpellIfCan`, `DoCast/DoCastAOE`, creature-spells list
(`SetSpellsList`/`UpdateSpellsList`/`DoSpellsListCasts` over
`m_CreatureSpells`), `SetMeleeAttack`/`SetCombatMovement` toggles
([CreatureAI.h:165](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.h:165)–221;
impl [CreatureAI.cpp:35](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.cpp:35)–364).

Selection is `FactorySelector::selectAI`
([CreatureAISelector.cpp:37](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAISelector.cpp:37)),
called from `Creature::AIM_Initialize`
([Creature.cpp:1219](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Creature.cpp:1219)).
Priority order, exactly:

1. Player-possessed non-pet → `NullCreatureAI` (Selector:40-41).
2. C++ script AI via `sScriptMgr.GetCreatureAI(creature)` for
   non-controlled, non-charmed creatures (Selector:44-46) — dispatches
   through `m_scripts[creature->GetScriptId()]->GetAI`
   ([ScriptMgr.cpp:1778](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:1778)).
3. Controlled player pet / charmed → `PetAI`; totem → `TotemAI`
   (Selector:57-60).
4. `AIName=="EventAI"` on pet → `PetEventAI`; on guard →
   `GuardEventAI` (Selector:63-68).
5. Registry lookup by `creature_template.AIName` string (Selector:71-72).
6. Guard → `GuardAI`; critter type → `CritterAI` (Selector:74-78).
7. `Permissible()` contest over all registered factories, highest wins
   (Selector:81-98): `PERMIT_BASE_NO=-1, IDLE=1, NORMAL=100,
   SPECIAL=200`
   ([CreatureAI.h:239](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAI.h:239)).
   `BasicAI::Permissible` unconditionally returns `PERMIT_BASE_NORMAL`
   ([BasicAI.cpp:24](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/BasicAI.cpp:24)),
   `CreatureEventAI::Permissible` returns SPECIAL iff `AIName=="EventAI"`
   ([CreatureEventAI.cpp:50](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:50)),
   `NullCreatureAI` returns IDLE
   ([NullCreatureAI.h:39](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/NullCreatureAI.h:39)).
8. Fallback `new NullCreatureAI` (Selector:101-104).

Registered factories (9): `NullAI, BasicAI, CritterAI, GuardAI, PetAI,
TotemAI, EventAI, PetEventAI, GuardEventAI`
([CreatureAIRegistry.cpp:42](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureAIRegistry.cpp:42)–50).
Note the registry key is `"NullAI"` while the class/fallback name is
`NullCreatureAI` — a string quirk a port must preserve if it keys by name.

Class hierarchy that matters: `CreatureEventAI : BasicAI : CreatureAI`
([CreatureEventAI.h:296](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.h:296),
[BasicAI.h:22](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/BasicAI.h:22));
`ScriptedAI : BasicAI` is the base for hand-written C++ boss scripts
([ScriptedAI.h:18](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/ScriptedAI.h:18)),
plus `ScriptedEscortAI/FollowerAI/PetAI`, `GuardEventAI/PetEventAI`
(EventAI variants), `PlayerAI` (bots/possession), `GameObjectAI`
(separate interface, 40 lines).

### a.2 BasicAI (the "Reactor/Aggressor" equivalent) and NullAI

`BasicAI::MoveInLineOfSight` is the proximity-aggro path: gate on
`IsProximityAggroAllowedFor` (PvP-only static flag, existing-victim,
dungeon, no-leash-evade rules,
[BasicAI.cpp:26](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/BasicAI.cpp:26)–42),
`CanInitiateAttack`, Z-range, attack/detection distance, LOS,
targetability, hostility, accessible-place; then
`EnterCombatWithTarget` or civilian `SummonGuard`
([BasicAI.cpp:44](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/BasicAI.cpp:44)–68).
`BasicAI::UpdateAI` = `SelectHostileTarget()` → bail without victim →
`UpdateSpellsList` → `DoMeleeAttackIfReady`
([BasicAI.cpp:84](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/BasicAI.cpp:84)–93).
`NullCreatureAI` no-ops `MoveInLineOfSight/AttackStart/AttackedBy`; only
`UpdateAI` has a body (38-line .cpp).

### a.3 DB script engine (ScriptMgr + ScriptCommands)

One generic loader `ScriptMgr::LoadScripts(ScriptMapMap&, tablename)`
selects 20 columns (`id, delay, command, datalong..4, target_param1/2,
target_type, data_flags, dataint..4, x,y,z,o, condition_id`) ordered by
`id, delay, priority`
([ScriptMgr.cpp:74](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:74)–84),
validates per-command (the switch starting
[ScriptMgr.cpp:157](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:157)
runs ~1200 lines), and stores `ScriptMap = multimap<delay, ScriptInfo>`
keyed by script id in 11 global maps
([ScriptMgr.h:54](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.h:54)–67):
`sAreaTriggerScripts, sCreatureAIScripts, sCreatureMovementScripts,
sCreatureSpellScripts, sEventScripts, sGameObjectScripts,
sGenericScripts, sGossipScripts, sQuestEndScripts, sQuestStartScripts,
sSpellScripts`, loaded from tables `areatrigger_scripts,
creature_ai_scripts, creature_movement_scripts,
creature_spells_scripts, event_scripts, gameobject_scripts,
generic_scripts, gossip_scripts, quest_end_scripts,
quest_start_scripts, spell_scripts`
([ScriptMgr.cpp:1388](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:1388),
1420, 1433, 1458, 1483, 1535, 1559, 1576, 1583, 1590, 1597).

Command vocabulary: `eScriptCommand` has **93 commands** (0–92,
`SCRIPT_COMMAND_MAX=93`, plus `SCRIPT_COMMAND_DISABLED=9999`)
([ScriptCommands.h:35](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/ScriptCommands.h:35)–343).
Each `ScriptInfo` is a tagged union over `datalong[4]/dataint[4]/x/y/z/o`
whose field meanings are documented inline per command (e.g. TALK=0,
EMOTE=1, MOVE_TO=3, CAST_SPELL=15, TEMP_SUMMON_CREATURE=10,
MODIFY_THREAT=29, SET_PHASE=44, START_SCRIPT=39).
Target resolution: `ScriptTarget` enum, **30 types** (0–29 +
`TARGET_T_END`): provided, hostile top/second/last/random/random-not-top/
nearest/farthest aggro, owner, nearest creature/GO by entry, by db guid,
from instance data, friendly variants, map-event targets, nearest player
variants, random by entry
([ScriptCommands.h:1128](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/ScriptCommands.h:1128)–1206).

Execution has two modes in `Map`: immediate
`ScriptCommandStartDirect` (resolve final targets → condition check with
`CONDITION_FROM_DBSCRIPTS` → dispatch via `m_ScriptCommands[command]`
member-function table; failure returns the `SF_GENERAL_ABORT_ON_FAILURE`
bit,
[Map.cpp:2469](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/Map.cpp:2469)–2475)
and delayed `ScriptCommandStart` → `m_scriptSchedule` multimap keyed by
`gameTime+delay`, drained by `ScriptsProcess` each map tick
([Map.cpp:2455](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/Map.cpp:2455)–2467,
[Map.cpp:2544](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/Map.cpp:2544)+).
Command bodies: `ScriptCommands.cpp`, 2526 lines. Invalid rows are not
dropped but neutered to `SCRIPT_COMMAND_DISABLED` at load
([ScriptMgr.cpp:66](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:66)–72).

`creature_ai_scripts` is special: `delay != 0` is rejected (EventAI
actions run synchronously)
([ScriptMgr.cpp:1602](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.cpp:1602)–1617).

### a.4 EventAI

`EventAI_Type`: **37 event types** (0–36 + `EVENT_T_END=37`): timers
in/out of combat, HP/mana % (self/target), aggro, kill, death, evade,
hit-by-spell/aura, range, OOC LOS, spawned, target casting, friendly
HP/CC/missing-buff, summoned unit/died/despawn, quest accept/complete
(**rejected at load as not implemented**,
[CreatureEventAIMgr.cpp:218](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAIMgr.cpp:218)–221),
reached home, receive emote, aura present/missing (self/target),
movement inform, leave combat, script event, group member died, victim
rooted, stealth alert, spell-hit-target
([CreatureEventAI.h:38](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.h:38)–79).
Flags: `EFLAG_REPEATABLE, RANDOM_ACTION, NOT_CASTING, CHECK_RESULT,
DEBUG_ONLY`
([CreatureEventAI.h:81](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.h:81)–88).
Each event carries up to `MAX_ACTIONS=3` pointers into
`sCreatureAIScripts`, a `condition_id`, an inverse phase mask (phases
0–31, `MAX_PHASE=32`), chance %, and a 4×u32 param union
([CreatureEventAI.h:34](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.h:34)–36,
98–278). There is **no separate EventAI action enum**: actions *are* DB
script commands (§a.3). `ProcessAction` iterates the action's ScriptMap
calling `Map::ScriptCommandStartDirect(source=creature,
target=invoker-or-victim)`
([CreatureEventAI.cpp:454](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:454)–469).

Runtime flow: constructor copies the creature's event vec from
`sEventAIMgr` map (by entry), drops DEBUG_ONLY in release, enables
MoveInLOS if any OOC_LOS event, fires SPAWNED events immediately, then
`Reset()`
([CreatureEventAI.cpp:59](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:59)–97).
`ProcessEvent` gates: enabled & timer 0 → inverse-phase mask →
NOT_CASTING → `IsConditionSatisfied(..., CONDITION_FROM_EVENTAI)` →
per-type predicate + repeat-timer rearm (`urand(repeatMin,repeatMax)`) →
disable if not REPEATABLE → `event_chance <= rand()%100` rejects →
run all 3 actions or one random (RANDOM_ACTION) → CHECK_RESULT re-arms
with 0 timer on script failure
([CreatureEventAI.cpp:99](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:99)–452).
Note ordering quirk: the repeat timer is rearmed and non-repeatable
events are disabled **before** the chance roll, so a chance-miss still
consumes the event/cooldown. Polled event types are evaluated only from
`UpdateEventsOn_UpdateAI`, throttled to `EVENT_UPDATE_TIME=500ms`
accumulated diff; timers do not decrement while phase-masked
([CreatureEventAI.cpp:761](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:761)–828).
Callback-driven types dispatch from the matching `CreatureAI` overrides
(EnterCombat→AGGRO, JustDied→DEATH, SpellHit→HIT_BY_SPELL/HIT_BY_AURA,
MovementInform, ReceiveEmote, OnScriptEventHappened→SCRIPT, etc.,
CreatureEventAI.cpp:471–894). `KilledUnit` only forwards player victims
([CreatureEventAI.cpp:573](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:573)–576)
— combined with `kill.playerOnly`, non-player kill events are dead code.
`UpdateAI` = `SelectHostileTarget` → events → spells list → melee
([CreatureEventAI.cpp:739](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAI.cpp:739)–759).

Load: `CreatureEventAIMgr::LoadCreatureEventAI_Events` selects 14
columns from `creature_ai_events`, validates type `< EVENT_T_END`,
creature template exists, chance clamped to 100, condition exists
(else zeroed), per-type param sanity, then binds `action{1,2,3}_script`
into `sCreatureAIScripts` (missing action logs and leaves null)
([CreatureEventAIMgr.cpp:38](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAIMgr.cpp:38)–362).
Load-order dependency: `creature_ai_scripts` must load before events.

### a.5 SmartScript check (negative, verified)

`grep -ri "SmartScript|SmartAI|smart_scripts" vmangos/src vmangos/sql`
returns **zero hits**. The only `smart` hits in src are unrelated
(smart pointers/log wording). The data-driven layer is exactly:
EventAI events → DB script commands → Conditions. There is no
source-type/action-type/target-type triple-table system to port.

### a.6 Conditions

`ConditionType`: **63 values** — 3 combinators (`NOT=-3` deprecated,
`OR=-2`, `AND=-1`), `NONE=0`, and 59 leaf types (1–59: aura, item,
equipped, area, reputation min/max, team, skill/skill-below, quest
rewarded/taken/available/none, saved variable, game event, holiday,
race/class, level, source entry, spell known, instance script/data,
nearby creature/GO/player, WoW patch, escort, gender, is-player,
has-flag, last waypoint, map id, map-event data/active/targets, LOS,
distance to target/position, moving, has-pet, HP/mana %, in-combat,
reaction, in-group, alive, object spawned/loot/GO-state/fit-condition,
PvP rank, db guid, local time, creature-group member/dead, area
explored, cant-path-to-victim, AD commission aura)
([Conditions.h:29](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Conditions.h:29)–263).
Flags: `REVERSE_RESULT, SWAP_TARGETS` (Conditions.h:265-269). Evaluation:
`ConditionEntry::Meets` → param-requirement check → `Evaluate` switch
(146 `case CONDITION_` labels across validation+evaluation in
Conditions.cpp; `Meets` at
[Conditions.cpp:106](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Conditions.cpp:106),
`Evaluate` at :128), with AND/OR/NOT recursing into other condition ids.
Free entry point `IsConditionSatisfied(id, target, map, source,
ConditionSource)`
([Conditions.cpp:1361](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Conditions.cpp:1361)).

Call sites (12 `ConditionSource` values, Conditions.h:271-285):
EventAI (CreatureEventAI.cpp:111), DB scripts (Map.cpp:2471, 2562;
ScriptCommands.cpp:1093, 1862), map events (Map.cpp:1097-1120), loot
(LootMgr.cpp:373), gossip menu/option (Player.cpp:11915, 12257), quest
availability (Player.cpp:13218), vendor (Player.cpp:18257,
ItemHandler.cpp:843), areatrigger teleport (Map.cpp:2590,
MiscHandler.cpp:819), spell_area, debug command. Conditions are thus a
shared service, not an AI-private helper.

### a.7 C++ script library (ScriptDev2-style)

`Script` struct = name + 21 function pointers (gossip, quest, GO,
areatrigger, dummy effects, `GetAI`, `GOGetAI`, `GetInstanceData`,
`GetSpellScript`, `GetAuraScript`)
([ScriptMgr.h:216](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ScriptMgr.h:216)–251);
`SpellScript`/`AuraScript` add ~14/~17 virtual hooks each
(ScriptMgr.h:138-214). Scripts self-register via `AddScripts()`
(`ScriptLoader.cpp`, 491 lines) into `m_scripts[scriptId]`, where
scriptId resolves from DB `script_name` strings. Volume:
`vmangos/src/scripts/` = **223 .cpp files, 103,742 lines** — by far the
largest sub-surface, and it is hand-written per-boss/quest logic, not an
interpreter.

### a.8 AI update cadence

`Creature::Update` (ALIVE case) calls `AI()->UpdateAI(diff)` **every map
tick** the creature is updated, where `diff` is the world tick diff —
unless leashing or target-unreachable >24s, which substitutes
`EnterEvadeMode`
([Creature.cpp:942](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Creature.cpp:942)–960).
Dead creatures get `UpdateAI_corpse(diff)` instead (Creature.cpp:840-845).
There is no AI-specific tick divider in `Creature::Update`; throttling
is internal: EventAI polls at 500ms (`EVENT_UPDATE_TIME`), and map
update itself may skip inactive grids (comment at Creature.cpp:952:
"AI not react good at real update delays"). `MoveInLineOfSight` is
event-driven from visibility/movement, not polled in UpdateAI.

## (b) Sizes / counts

| Item | Size |
|---|---|
| `vmangos/src/game/AI/` (42 files) | 7,364 lines total |
| CreatureAI.h/.cpp + Impl | 250 + 364 + 437 |
| BasicAI | 43 + 95 |
| CreatureEventAI + Mgr | (345+894) + (44+362) |
| PetAI / ScriptedEscortAI | 797 / 527 (largest AIs) |
| ScriptMgr.h/.cpp | 390 + 2,930 |
| ScriptCommands.h/.cpp | 1,209 + 2,526 |
| Conditions.h/.cpp | 351 + 1,367 |
| C++ script library `src/scripts/` | 223 files, 103,742 lines |
| EventAI event types | 37 (2 load-rejected: quest ×2) |
| DB script commands | 93 (+ DISABLED sentinel) |
| Script target types | 30 |
| Condition types | 63 (59 leaf + NONE + AND/OR/NOT) |
| Registered AI factories | 9 |
| DB script tables / global maps | 11 |
| Rust `mangos-threat` (existing) | 986 lines (lib+manager+hated_by) |

## (c) DB / config inputs

- `creature_template.AIName` (factory key / `"EventAI"`) and
  `script_name`→scriptId (C++ script binding) drive selection §a.1.
- `creature_ai_events` (14 cols: id, creature_id, condition_id,
  event_type, event_inverse_phase_mask, event_chance, event_flags,
  event_param1..4, action1..3_script) — EventAI table
  ([CreatureEventAIMgr.cpp:44](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/AI/CreatureEventAIMgr.cpp:44)–47).
- Eleven `*_scripts` tables sharing the 20-column `ScriptInfo` shape
  (§a.3); `creature_ai_scripts` is the EventAI action pool.
- `conditions` via `sConditionStorage` (`condition_entry` key, fmt
  `iiiiiii` = entry, type, value1..4, flags)
  ([SQLStorages.cpp:44](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Database/SQLStorages.cpp:44)).
- Supporting: `script_texts`/`script_texts_custom` (TALK),
  `script_waypoints` (escorts), escort data, `creature_spells`
  (BasicAI/EventAI spell lists via `SetSpellsList`), broadcast texts.
- No AI-specific world config keys were found in the inspected files;
  cadence constants are compile-time (`EVENT_UPDATE_TIME=500`).
  (World config for e.g. threat/combat timers lives outside this
  surface and was not inventoried here.)

## (d) Wire-visible semantics a Rust port must replicate

1. **Selection precedence** (§a.1 order 1–8) — determines observable
   behaviour per spawn: scripted C++ AI beats EventAI beats BasicAI;
   Pet/Guard EventAI remapping; possession forces NullAI.
2. **Aggro entry**: LOS + attack-distance + Z-range + hostility +
   accessible-place gating before `EnterCombatWithTarget`; civilians
   summon guards instead of attacking (BasicAI.cpp:44-68). Wrong gating
   = visible pull-range/through-wall-aggro differences.
3. **Victim selection each tick** via threat (`SelectHostileTarget` in
   both BasicAI and EventAI UpdateAI) — Rust `mangos-threat` already
   ports the 110%/130% rule; AI must call it at the same point
   (before melee/spell decisions, every tick).
4. **EventAI timing**: 500ms poll granularity for timer/percent/range/
   aura events; initial timers armed on EnterCombat (in-combat) and
   Reset (OOC); repeat = uniform `urand(min,max)`; timers frozen while
   phase-masked; chance roll **after** rearm/disable (miss consumes
   cooldown); non-repeatable fires once per combat (re-enabled on
   EnterCombat default branch, CreatureEventAI.cpp:643-646); phase
   resets to 0 on death (CreatureEventAI.cpp:570).
5. **Action execution is synchronous and ordered** for EventAI
   (delay must be 0), target = invoker else current victim; RANDOM_ACTION
   picks exactly one non-null action uniformly.
6. **DB script scheduling**: delayed scripts fire in `(id, delay,
   priority)` load order from a time-ordered multimap; a command
   returning "abort" terminates all pending actions of that script id
   for the same source/target pair (`TerminateScript`, Map.cpp:2529-2541);
   `SF_GENERAL_ABORT_ON_FAILURE` converts load/lookup/condition failure
   into abort. Target buddy resolution + swap/self flags (§a.3) change
   who says/casts/moves — directly wire-visible.
7. **Condition semantics**: AND/OR recursion, REVERSE_RESULT and
   SWAP_TARGETS flags, requirement failure = false (not error), and
   per-source behaviour differences keyed by `ConditionSource`.
8. **Text/emote/sound commands** (TALK with up to 4 random broadcast
   texts, EMOTE, PLAY_SOUND flags) produce chat packets — text id
   ranges (`script_texts` at -1,000,000…) must map identically.
9. **Evade/leash interplay**: UpdateAI is replaced by EnterEvadeMode on
   leash or 24s unreachable (Creature.cpp:949-952); EventAI EVADE /
   LEAVE_COMBAT / REACHED_HOME events hang off those transitions.
10. **Spell-list casting** (`creature_spells`): per-entry initial delay
    `urand(delayInitialMin,Max)` at assignment (CreatureAI.h:47),
    shared `m_uiCastingDelay` gating in `UpdateSpellsList`.

## (e) Smallest viable first slice + entry dependencies

**Slice: BasicAI + NullAI + selection, then EventAI interpreter over a
DB-script command subset.** Conditions and the C++ script library are
explicitly out of slice 1.

1. `CreatureAi` trait mirroring the hook set actually used by BasicAI/
   EventAI (UpdateAI, MoveInLineOfSight, EnterCombat, EnterEvadeMode,
   JustDied, JustRespawned, SpellHit, MovementInform, AttackedBy…) +
   `select_ai` implementing §a.1 steps 1,3–8 (step 2 returns None until
   a script registry exists). Dependencies: creature template
   (`AIName`, type, guard/pet/totem flags) from `mangos-world-data`.
2. BasicAI: proximity aggro + `SelectHostileTarget` + melee. Entry
   deps: **threat** (`mangos-threat` exists — needs its `ThreatHost`
   impl from the world layer: attackability, chase geometry),
   **movement** (`mangos-movement` exists; chase generator + evade-to-home),
   hostility/faction and LOS/accessibility queries from simulation.
3. EventAI interpreter: load `creature_ai_events` +
   `creature_ai_scripts` (via `mangos-db`), implement ProcessEvent
   gating/500ms polling/phase/chance semantics (§d.4) and an initial
   command subset covering the common boss toolkit: TALK, EMOTE,
   CAST_SPELL, TEMP_SUMMON_CREATURE, SET_PHASE(+RANDOM/RANGE),
   MODIFY_THREAT/ADD_THREAT, ATTACK_START, ENTER_EVADE_MODE, FLEE,
   SET_FACTION, SET_MELEE_ATTACK, SET_COMBAT_MOVEMENT, MOVE_TO,
   MOVEMENT, DESPAWN_CREATURE, UPDATE_ENTRY, MORPH, PLAY_SOUND,
   SEND_SCRIPT_EVENT, START_SCRIPT (generic). Remaining commands log
   as unimplemented (C++ analogue: DISABLED at load) rather than abort.
   Entry deps: **spell casting** (`TryToCast`/cast flags — the hard
   dependency; CAST_SPELL is near-useless without the spell system),
   summon/spawn service, broadcast-text table, script target resolution
   over the threat list (types 1–7 need `mangos-threat` ordering).
4. Conditions: port `Evaluate` for the subset referenced by loaded
   EventAI/DB-script rows first (data-driven triage), keeping the
   `IsConditionSatisfied` signature so loot/gossip/vendor can adopt it
   later. Full 59-leaf port is a follow-up.
5. C++ script library (103k lines): do **not** port wholesale. Provide
   a Rust `Script` registry keyed by `script_name` with `GetAI`
   equivalents written per-creature on demand (ScriptedAI-style base
   struct), prioritised by instance/boss content actually enabled.

Threat/movement/spell dependency summary: threat ✅ ported
(`mangos-threat`, host trait unimplemented by world layer), movement
crate exists (`mangos-movement`, chase/evade generators not verified
in this inventory), spell system ❌ not present as a crate — it gates
CAST_SPELL, aura conditions, and HIT_BY_SPELL/AURA events.

## (f) Unresolved

- Production binary `2bae5f9` vs HEAD `1bec69f` divergence for this
  surface: not diffed (no claim either way).
- DB row counts for `creature_ai_events`, `creature_ai_scripts`,
  `conditions` in the live world DB: not queried; needed to triage
  the condition/command subsets in §e by actual usage.
- `CreatureAIImpl.h` (437 lines, factory `Create` bodies) and
  `ScriptedAI.cpp` helper semantics: located, not fully read.
- `GuardEventAI`/`PetEventAI` deltas vs `CreatureEventAI` (79/245
  lines): not read; assumed thin overrides.
- Whether `mangos-movement` already provides chase + home/evade
  generators equivalent to C++ `CHASE_MOTION_TYPE` reachability
  (`IsReachable` feeds the 24s evade rule): not verified here.
- Map-event system (`START_MAP_EVENT` etc., `ZoneScriptMgr`) overlaps
  this surface via Conditions/script commands but belongs to the map
  inventory; only its script-command face is covered above.
- `PlayerAI` (359 lines, bot/possession control) excluded by scope.
