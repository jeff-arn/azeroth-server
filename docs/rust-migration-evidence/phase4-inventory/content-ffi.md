# Phase 4 inventory — Content scripts + FFI-retain

- Surface: C++ content scripts (`vmangos/src/scripts/`) and the
  owner decision 2026-10-18 to FFI-retain them from a Rust host
  (`PHASE4-PROGRESS.md` "Owner decisions"; unsafe confined to the
  future content-scripts FFI shim crate).
- Oracle: C++ at checkout HEAD `1bec69f` (branch `rust-migration`).
  Production binary is `2bae5f9`; divergences flagged only where
  inspected (§f).
- Method: every body cited was opened with `read_file`; search used
  only to locate candidates. Counts below are executed counts
  (find/wc/grep/python over the tree), not estimates.

## (a) Verified structure / flow

### Registration chain (static, no dynamic loading, no FFI exists)

1. Build: `vmangos/cmake/options.cmake:4` `USE_SCRIPTS=ON` by
   default. `vmangos/src/CMakeLists.txt:31-32` adds
   `add_subdirectory(scripts)`; `vmangos/src/scripts/CMakeLists.txt:339-347`
   builds `add_library(scripts STATIC …)` + `target_link_libraries(scripts game)`.
2. `vmangos/src/mangosd/CMakeLists.txt:86-95` links `mangosd` against
   `game scripts shared framework g3dlite gsoap` when `USE_SCRIPTS`.
3. `ScriptLoader.cpp` itself is compiled into **game**, not scripts:
   `scripts/CMakeLists.txt:279-281` removes it ("Used in game
   ScriptMgr, not need here") and `game/CMakeLists.txt:491-500`
   appends `ScriptLoader.cpp` (or `ScriptLoader_noscripts.cpp`,
   whose whole body is `void AddScripts() {}`) to `game_SRCS`.
4. Runtime: `ScriptMgr::LoadScriptNames()`
   (`vmangos/src/game/ScriptMgr.cpp:1725-1752`) collects distinct
   DB `script_name` strings, sorts them, index 0 = `""`.
   `ScriptMgr::Initialize()` (`ScriptMgr.cpp:2057-2080`) resizes
   `m_scripts` to `GetScriptIdsCount()`, calls `AddScripts()` at
   `ScriptMgr.cpp:2070`, then logs an error for every DB script
   name with no registered `Script` (`:2073-2077`).
5. `AddScripts()` (`vmangos/src/scripts/ScriptLoader.cpp:250-491`)
   calls the per-file `AddSC_*` functions. Each `AddSC_*` does
   `new Script`, sets `Name` + function-pointer slots, calls
   `RegisterSelf()` (`ScriptMgr.cpp:2739-2752`): looks the name up
   by binary search (`GetScriptId`, `:1755-1768`); if the DB never
   assigned that name the Script is `delete`d and an error logged
   ("Script will not be used"), else `m_scripts[id]=this`.
6. Dispatch is by integer script id resolved at load from DB rows:
   `ScriptMgr::GetCreatureAI` (`:1779-1787`), `GetGameObjectAI`
   (`:1789-1797`), `CreateInstanceData` (`:1799-1807`),
   `GetSpellScript`/`GetAuraScript` (`:1809-1827`), plus ~17
   `On*`/`GetDialogStatus` forwarders (`ScriptMgr.h:337-353`)
   that index `m_scripts[owner->GetScriptId()]` and null-check
   the slot before calling through the raw function pointer.

### The `Script` record and its slots

`vmangos/src/game/ScriptMgr.h:216-251`: one `std::string Name`,
20 plain function pointers (`pGossipHello` … `pGOOpen`,
`:223-242`) and 5 factory pointers (`GOGetAI`, `GetAI`,
`GetInstanceData`, `GetSpellScript`, `GetAuraScript`, `:244-248`).
`SpellScript` (`:138-170`, 17 virtuals) and `AuraScript`
(`:172-214`, 14 virtuals) are separate polymorphic bases whose
hooks receive live `Spell*`/`Aura*`/`SpellAuraHolder*`.

### AI / instance bases scripts actually subclass

- `ScriptedAI : BasicAI : CreatureAI`
  (`AI/ScriptedAI.h:18`, `AI/BasicAI.h:22`, `AI/CreatureAI.h:50`).
  `CreatureAI` exposes ~35 virtual callbacks (`CreatureAI.h:59-162`:
  `MoveInLineOfSight`, `EnterCombat`, `DamageTaken`, `JustDied`,
  `MovementInform`, `UpdateAI(uint32 diff)`, …), public
  `Creature* const m_creature` (`:208`), and helpers
  `DoCastSpellIfCan`/`DoMeleeAttackIfReady` (`:165,184`).
  `BasicAI` adds proximity-aggro + guard summoning and
  `Permissible()=PERMIT_BASE_NORMAL` (`BasicAI.cpp:24`).
  `ScriptedAI` adds pure-virtual `Reset()` (`ScriptedAI.h:48`),
  `Aggro`, and ~25 `Do*` helpers (`:61-113`, bodies in
  `ScriptedAI.cpp`, 334 lines) plus public `Creature* me` (`:114`).
- `ScriptedInstance : InstanceData : ZoneScript`
  (`AI/ScriptedInstance.h:30`, `Maps/InstanceData.h:36`).
  `InstanceData` gives `Map* instance`, `Initialize/Load/Save`,
  `GetData/SetData/GetData64/SetData64` (`InstanceData.h:42-80`);
  `ZoneScript` gives the create/death/enter callbacks Deadmines
  overrides (`Maps/ZoneScript.h:213-224`); `ScriptedInstance`
  adds door/button + storage helpers (`ScriptedInstance.h:37-61`)
  and `DialogueHelper` (`:101-142`).
- `GameObjectAI` (`AI/GameObjectAI.h`, whole file): 8 virtuals,
  `GameObject* me`.
- Every script TU includes `scriptPCH.h` (228 of 258 files),
  which pulls 24 core headers (`Creature.h`, `Unit.h`, `Spell.h`,
  `ObjectMgr.h`, `World.h`, … —
  `scripts/PrecompiledHeaders/scriptPCH.h:8-31`). There is no
  narrow script-facing header: scripts compile against the full
  game object model.

## (b) Sizes / counts (executed, HEAD `1bec69f`)

Tree `vmangos/src/scripts/`: **258 files = 223 `.cpp` + 34 `.h` +
CMakeLists; 107,413 LOC total (103,742 cpp + 3,671 h)**.

| Region | .cpp | .h | LOC |
|---|---|---|---|
| eastern_kingdoms | 132 | 14 | 53,989 |
| kalimdor | 59 | 11 | 34,626 |
| world | 14 | 6 | 8,521 |
| battlegrounds | 2 | 0 | 5,209 |
| spells | 11 | 0 | 3,239 |
| custom | 2 | 1 | 1,286 |
| top-level (ScriptLoader 491 + noscripts 3) | 2 | 1 | 504 |
| PrecompiledHeaders | 1 | 1 | 39 |

Largest TUs: `battlegrounds/battleground_alterac.cpp` 5,025;
`kalimdor/silithus/silithus.cpp` 2,886; `blackrock_depths.cpp`
2,414; `dire_maul/instance_dire_maul.cpp` 2,303;
`world/npcs_special.cpp` 2,260; `naxxramas/instance_naxxramas.cpp`
2,097. Instance scripts: **24 `instance_*.cpp`**; boss scripts:
**113 `boss_*.cpp`**.

### Re-verification of the prior "441 AddSC_*" claim — REFUTED

Prior claim (`PHASE4-INVENTORY.md` §1.9 item 5, Slice 6, §risk 1):
"`AddScripts()` ScriptLoader.cpp:250 calling 441 AddSC_* fns,
invoked ScriptMgr.cpp:2070". The anchors are right
(`AddScripts()` does begin at `ScriptLoader.cpp:250`; the call
is at `ScriptMgr.cpp:2070`) but **441 is a grep artifact**:
`grep -c AddSC_ ScriptLoader.cpp` = 441 = 212 active forward
declarations (`:8-248`) + 212 active calls (`:253-490`) + 9
commented declarations + 8 commented calls. Verified by
parsing: **212 active, distinct, duplicate-free `AddSC_*` calls**
and the declaration set equals the call set exactly.
Definitions in the tree: 216 `void AddSC_*()`. The 4
not directly called from `AddScripts()`: `AddSC_custom_creatures`
(reachable — called by `AddSC_zero_scripts`,
`custom/custom.cpp:20`), `AddSC_alterac_mountains` (empty body,
commented out at `ScriptLoader.cpp:158,402`),
`AddSC_boss_order_of_silver_hand` and `AddSC_npc_sandstalker`
(defined, never called — their registrations are dead at HEAD).

### True registration volume

- **749 `new Script` allocations = 749 distinct `->Name="…"`
  assignments (zero duplicate names) = 749 `RegisterSelf`
  calls** (7 of them `RegisterSelf(false)`).
- Slot population across the 749: `GetAI` 448, `GetSpellScript`
  114, `GOGetAI` 50, `pQuestAcceptNPC` 49, `pGOHello` 33,
  `pGossipHello` 31, `GetAuraScript` 31, `GetInstanceData` 24,
  `pGossipSelect` 24, `pAreaTrigger` 24, `pQuestRewardedNPC` 16,
  `pProcessEventId` 10, `pEffectDummyCreature` 8, rest ≤5.
  One Script may fill several slots (e.g. gunpowder, §a/Deadmines).
- Subclass counts (regex over `struct/class X : public Y`):
  `ScriptedAI` 309, `SpellScript` 49, `GameObjectAI` 40,
  `npc_escortAI` 38, `AuraScript` 30, `ScriptedInstance` 13
  direct (24 `GetInstanceData` slots; remainder reach the base
  via intermediates or non-`instance_*` files), `FollowerAI` 10,
  `Scripted_NoMovementAI` 3, `ScriptedPetAI` 2.
- Whole-tree call surface: **675 distinct `->Method(` names over
  14,966 call sites**; 174 `dynamic_cast` uses; 975 `urand(`;
  961 `DoScriptText(`; 126 `GetCreatureListWithEntryInGrid`;
  singleton touches `sLog` 152, `sObjectMgr` 86, `sWorld` 46.
  `GetMotionMaster()->` fans out to 16 distinct methods
  (`MovePoint` 246, `Clear` 40, `MoveIdle` 34, `MoveFollow` 26,
  `MoveChase` 23, …). 73 distinct `#include "..."` targets.

## (c) DB / config inputs

- **script_name binding (the join key):** `LoadScriptNames`
  (`ScriptMgr.cpp:1732`) unions `script_name` from 6 tables:
  `areatrigger_template`, `creature_template`,
  `gameobject_template`, `scripted_event_id`, `spell_template`,
  `map_template`. Owners cache the resolved id at template load
  (`ObjectMgr.cpp:1302` creature, `:7824` gameobject,
  `:6757` areatrigger). A Rust host must reproduce this
  name→id assignment (sorted order decides ids, `:1750`) or
  replace it wholesale — ids are not persisted, names are.
- **ScriptMgr-owned DB content** (`LoadDatabase`,
  `ScriptMgr.cpp:2082-2088` + loaders): `script_texts`
  (`:2095`), `custom_texts` (`:2156`) — negative-entry ranges in
  `ScriptMgr.h:73-79`; `script_waypoint` (`:2220-2228`);
  `script_escort_data` (`:2285`); `scripted_event_id`
  (`:1683`). Plus the 11 DB-script tables loaded via
  `LoadScripts(map, table)` (`:1388-1597`): areatrigger,
  gameobject, quest_end, quest_start, spell, generic, event,
  creature_spells, gossip, creature_movement, creature_ai
  scripts — those belong to the DB-script *interpreter* surface
  (Phase 4 slice 5), not to C++ content scripts, but share
  `ScriptMgr` and the same text/target helpers.
- **Config:** no `mangosd.conf` script tuning found; the only
  switch is build-time `USE_SCRIPTS` (`cmake/options.cmake:4`).
  With it OFF the server still runs: `AddScripts()` is empty,
  every DB script name logs "No script found" and all
  `GetAI/GetInstanceData/…` dispatch returns `nullptr`
  (creatures fall back to core AI selection). That is the
  existing degraded mode a phased port can mirror.
- Deadmines specifically reads **no DB rows directly**: all ids
  are compile-time constants in `deadmines.h:8-36` (NPC/GO/quest
  entries, text ids 1148/1149 resolved through `script_texts`).
  Its patrol logic keys off DB *spawn data* smuggled through
  `creature.respawn_delay` sentinel values 43199/43201/43202
  (`instance_deadmines.cpp:83-94,194,227,264`) — invisible in
  code review unless the spawn rows are also ported.

## (d) Wire-visible semantics a Rust port must replicate

Scripts have no wire format of their own; everything below is
observable by a 1.12 client through normal object updates, chat,
and movement packets, and is driven from the script bodies read.

- **Text/sound/emote:** `DoScriptText(textId, source, target)`
  (961 call sites) resolves `script_texts`/`custom_texts` rows to
  monster say/yell/emote + sound. Deadmines: Smite phase yells
  use broadcast-text ids 1344/1345 (`boss_mr_smite.cpp:29-30,245,273`),
  instance alarm lines 1148/1149 (`instance_deadmines.cpp:261,271`).
  Wrong text table = visibly wrong (or missing) RP lines.
- **GO state changes:** door open/close via `DoUseDoorOrButton` /
  `UseDoorOrButton(0,true)` on boss death and cannon fire
  (`instance_deadmines.cpp:102-117,151-157`); cannon/lever/gunpowder
  interaction gating returns `bool` that decides whether default
  GO use proceeds (`deadmines.cpp:27-71` — note `go_door_lever_dm`
  returns `!(door open)`, gunpowder returns `true` to consume).
- **Visibility/faction flips:** patrols spawn hidden + friendly
  (faction 35, `VISIBILITY_OFF`) and flip to visible + hostile
  (faction 17) on timers after boss deaths
  (`instance_deadmines.cpp:83-94,184-248`); DMF chest is hidden
  until an areatrigger + quest-complete check reveals it
  (`:141-142,289-299`). These are update-field/visibility events.
- **Movement:** `MovePoint` with `MOVE_PATHFINDING|MOVE_RUN_MODE`,
  `MoveChase`, spline arrival via `MovementInform` +
  `inSpline` flag (`boss_mr_smite.cpp:102-160,265` instance side).
  Smite's run-to-chest, kneel (`UNIT_STAND_STATE_KNEEL`), sheath
  changes, and `SetVirtualItem` weapon swaps at 66%/33% HP
  (`boss_mr_smite.cpp:164-201,241-284`) are all client-visible.
- **Combat mechanics:** spell casts gated on `DoCastSpellIfCan==
  CAST_OK` (stomp 6432 gates the whole equip sequence — if the
  cast fails, phase does not advance, `:243-255`); threat
  semantics (`AttackStop(true)` clears victim but keeps threat
  list; re-acquire via `SelectAttackingTarget(TOPAGGRO)`,
  `:188,252`); aura removal (`RemoveAurasDueToSpell(6433)`);
  `DoMeleeAttackIfReady` cadence; `SelectHostileTarget()` must
  be polled each `UpdateAI` or the boss never re-engages
  (`:204-235` — note timers tick in a separate branch while
  victimless during equip).
- **Timers are diff-driven, not wall-clock:** every timeout is
  `if (timer <= uiDiff) … else timer -= uiDiff`
  (`instance_deadmines.cpp:186-282`, `boss_mr_smite.cpp:209-295`).
  A Rust host must feed the same map-update `diff` (ms) and call
  instance `Update` / AI `UpdateAI` exactly once per map tick,
  or patrol/alarm/equip timing drifts observably.
- **Instance persistence contract:** `SetData/GetData` encounter
  states use `NOT_STARTED/IN_PROGRESS/FAIL/DONE/SPECIAL = 0..4`
  (`ScriptedInstance.h:15-22`); `Save()/Load()` strings via
  `GenSaveData/LoadSaveData` (`:50-51`) persist to the
  instance-save DB. Deadmines itself keeps state only in memory
  (`m_auiEncounter[1]`, GUID fields, timers —
  `instance_deadmines.cpp:31-68`) and re-derives GUIDs from
  `OnCreatureCreate/OnObjectCreate` (`:71-143`), including
  position-range disambiguation for same-entry doors
  (`:136-139`) — spawn order/position fidelity matters.

## (e) Smallest viable first slice + FFI-retain assessment

### Deadmines sampled ABI (the floor for ONE instance)

Files: `instance_deadmines.cpp` 314 + `deadmines.cpp` 110 +
`boss_mr_smite.cpp` 321 + `deadmines.h` 38 = **783 LOC**, giving
**6 registered Scripts**: `instance_deadmines` +
`at_dmf_chest_dm` (`instance_deadmines.cpp:301-314`),
`go_door_lever_dm` / `go_defias_cannon` / `go_defias_gunpowder`
(`deadmines.cpp:91-110`), `boss_mr_smite`
(`boss_mr_smite.cpp:313-321`). Distinct API used, executed count:
**42 distinct `ptr->Method(` names** (top: `GetEntry` 10,
`GetMotionMaster` 10, `GetGUID` 8, `GetRespawnDelay` 7,
`SetVisibility`/`SetFactionTemplateId`/`SetVirtualItem` 6,
`GetGameObject`/`GetVictim`/`GetHealthPercent` 5) + 6
free/member helpers (`GetCreatureListWithEntryInGrid` 5,
`DoCastSpellIfCan` 5, `DoScriptText` 4, `DoUseDoorOrButton` 3,
`DoMeleeAttackIfReady`, `urand`) + `MotionMaster::{MovePoint,
MoveChase,Clear}` chained off `GetMotionMaster()` + raw C-style
cast `(ScriptedInstance*)pGo->GetInstanceData()`
(`deadmines.cpp:29,41,55`) + direct public-field access
`pInstance->instance` (`:34`) and `m_creature->GetCreatureInfo()->equipment_id`
(`boss_mr_smite.cpp:72`). Callbacks the host must *deliver*:
`UpdateAI, Reset, AttackedBy, AttackStart, MovementInform`
(CreatureAI/ScriptedAI), `Initialize, OnCreatureCreate,
OnCreatureDeath, OnObjectCreate, SetData/GetData/GetData64,
Update` (InstanceData/ZoneScript), `SummonedMovementInform`
(GameObjectAI), `pGOHello` ×3, `pAreaTrigger` ×1.

So even the smallest dungeon needs, from a Rust host: opaque
Creature/Unit/Player/GameObject/Map handles with ~45 methods,
threat + victim queries, spell-cast attempt with result code,
motion-master commands + arrival callbacks, grid search by
entry, GO state/use, summon-creature, quest-status query,
script-text broadcast, equipment/virtual-item/stand/sheath
mutation, and 17 inbound virtual callbacks — before any other
boss in the same instance (VanCleef et al. are EventAI/DB or
core AI; only Mr. Smite is C++-scripted here).

### Honest FFI-retain assessment (owner decision 2026-10-18)

FFI-retain means keeping the 103,742-LOC C++ script library
linked and callable. The object-model blockers, grounded above:

1. **Scripts subclass core C++ classes and are constructed by
   the core** (`new boss_mr_smiteAI(pCreature)` returned as
   `CreatureAI*`, `boss_mr_smite.cpp:311`). A Rust host must
   either (i) keep C++ `Creature/Unit/Map` objects as the real
   simulation objects (Rust becomes a shell around C++ — the
   "port" never owns state), or (ii) build C++ shim objects
   that *mirror* Rust entities and forward every one of the 675
   distinct methods back into Rust. There is no third option:
   the scripts' `this`-adjacent state (`m_creature`, `me`,
   `instance`) is a raw pointer they dereference freely,
   including chained `GetMotionMaster()->…`, template access
   (`GetCreatureInfo()->equipment_id`), `dynamic_cast` (174×,
   needs real RTTI/vtables), and C-style downcasts
   (`deadmines.cpp:29`).
2. **The surface is the whole game API, not a callback set** —
   confirming the prior critic note (§1.9 item 5) with counts:
   675 methods / 14,966 sites / 73 headers via a 24-header PCH.
   Any shim is therefore a second, hand-maintained
   implementation of the Unit/Creature/Spell/Map API in C++
   forwarding to Rust. Its cost scales with the API, not with
   the 212 registration functions.
3. **Lifetime/threading:** C++ scripts assume synchronous,
   same-tick mutation and pointer stability during `UpdateAI`;
   a Rust ECS/arena host must pin or handle-map every object a
   script can reach (`FindNearestGameObject`, grid lists of raw
   `Creature*` in `std::list`) for the duration of each call,
   and materialize C++ containers at the boundary.
4. **Build coupling:** scripts compile only against the full
   game target (PCH list, `scripts/CMakeLists.txt:363-391`);
   retaining them keeps the C++ game library (or a stub twin
   of every header it exposes) in the build indefinitely.

Conclusion: FFI-retain is viable as a *bridge* only if the shim
is treated as a first-class crate+CXX layer whose budget is
"re-implement the script-facing C++ API", starting from the
45-method Deadmines floor and growing per region. It is not a
narrow `extern "C"` registration hook; `AddScripts()` alone
buys nothing without the object model behind it.

### Alternative graduations (grounded in observed code)

- **Transpile-by-region, spells first:** `spells/` is smallest
  (11 cpp, 3,239 LOC) and its hooks are the most regular
  (114 `GetSpellScript` + 31 `GetAuraScript` over two fixed
  bases, `ScriptMgr.h:138-214`) — but it needs live Spell/Aura
  objects, so it still waits on the spells slice.
- **Instance-by-instance Rust rewrite, Deadmines first:** 783
  LOC / 6 Scripts / ~45 host methods is hand-portable and
  matches the planned slice-6 vertical slice; the same host
  trait surface then grows boss-by-boss (113 `boss_*.cpp`,
  309 `ScriptedAI` subclasses give the bulk estimate).
- **Keep DB-side systems native:** EventAI/DB-scripts (slice 5)
  already cover non-C++ creatures; running the Rust host with
  the equivalent of `USE_SCRIPTS=OFF` (all dispatch → None,
  core AI fallback) is a shippable degraded mode while regions
  graduate, exactly as C++ behaves today without scripts.
- Suggested order: Deadmines Rust rewrite → other 5-man
  instances by ascending LOC (`instance_*.cpp` + their boss
  files) → world/quest gossip scripts → spells/auras → raids
  (Naxxramas/AQ/BWL clusters dominate EK/Kalimdor LOC) →
  battlegrounds last (single 5,025-LOC AV file).

### Entry dependencies for the first slice

Map/instance lifecycle + spawn data (incl. `respawn_delay`
sentinels), Creature/Unit core (threat, victim, health%,
equipment), MotionMaster point/chase + arrival inform,
spell-cast attempt + aura removal, GO use/state, grid search,
script-text broadcast, areatrigger + quest-status — i.e.
Phase 4 slices 2-5 surfaces for the Deadmines map only, as the
prior inventory already sequenced (Slice 6 after 2-5).

## (f) Unresolved / divergences

- **Prod `2bae5f9` divergence (inspected):** `git diff 2bae5f9
  HEAD -- vmangos/src/scripts` touches 260 files, but the
  sampled diffs are formatting-only: `ScriptLoader.cpp` has
  *zero* `diff -w` changes; Deadmines files differ only by
  include order, brace collapsing, trailing blank lines
  (inspected hunks). `ScriptMgr.cpp/.h` show -w numstat
  80/120 and 17/24 — NOT line-inspected here, so semantic
  parity of ScriptMgr prod-vs-HEAD is **unresolved**; the full
  230-file -w script diff (2,802+/7,907-) was not audited.
- **DB-side truth not inspected:** which of the 749 registered
  names the production DB actually assigns (thus how many
  survive `RegisterSelf`), and the `script_texts` row content
  for ids 1148/1149/1344/1345 — requires DB access, out of
  scope for this source-only pass.
- **Dead registrations:** `boss_order_of_silver_hand` and
  `npc_sandstalker` Scripts can never register (their `AddSC_*`
  are uncalled); if the DB assigns those names, HEAD logs
  "No script found" — confirm against DB before porting.
- **`ScriptedInstance` direct-subclass count (13) vs 24
  `GetInstanceData` slots:** the other 11 reach the factory
  via intermediate bases/files not individually opened here.
- **Escort/follower/pet AI bases** (`npc_escortAI` 38,
  `FollowerAI` 10, `ScriptedPetAI` 2 subclasses) were counted
  but their headers/bodies were not deep-read in this pass;
  their waypoint DB coupling (`script_waypoint`,
  `script_escort_data`) will add shim surface beyond Deadmines.
- **No Rust-side script host exists yet to measure against:**
  `vmangos-rs/crates/` contains no script/FFI crate at HEAD
  (grep for script host types found only test/auth mentions);
  shim sizing above is derived purely from C++ call sites.
