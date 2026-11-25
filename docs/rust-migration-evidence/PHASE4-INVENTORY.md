# Phase 4 Inventory — Evidence Synthesis

Oracle: C++ at HEAD `1bec69f` (branch `rust-migration`), `vmangos/src/`.
Production binary `2bae5f9`: token-proven formatting-only only for
`Map.cpp`/`Object.cpp`/`Spell.cpp` + MovementInfo codec (rust-gaps §f,
movement §f); HEAD is **not** a proven production oracle elsewhere
(critic §2.5). Inputs (all read in full for this synthesis):
`phase4-inventory/{map-visibility,movement,entities-updates,
spells-auras,ai-engines,content-ffi,spawn-data,rust-gaps,critic}.md`
(565+691+765+387+432+402+594+420+201 lines). Per-surface files are
authoritative for detail; this file is a digest + plan and invents no
evidence. Critic corrections (§2.1–2.3, §3) override digest counts:
SpellScript 14 hooks / AuraScript 15 hooks (+dtors; ScriptMgr.h:138-214);
`Script` record 20 plain + 5 factory = 25 fn-ptrs (ScriptMgr.h:216-251);
`CreatureAI` 37 virtuals incl. dtor (36 hooks); `mangos-movement`
1,673 src + 467 tests = 2,140 lines; movement opcodes 41/36 (handler
basis) vs 39 (registry Movement-class basis) — do not sum.

## 1. Per-surface digest

### 1.1 Map / grid / visibility — `map-visibility.md`

- Hierarchy: `Map`→64×64 `NGrid`→16×16 cells; `MAX_NUMBER_OF_GRIDS=64`,
  `SIZE_OF_GRIDS=533.33333f`, cell 33.3333yd, 1024 cells/axis,
  `MAP_SIZE=34133.33f` (GridDefines.h:38-59); double `+0.5` + x/y swap
  (:153-167); `Cell` uint64 union (Cell.h:49-88). World store
  Player/pet-Creature/resurrectable-Corpse/Camera vs Grid store
  GO/non-pet Creature/DynObj/bones (GridDefines.h:63-65, Map.cpp:216-295);
  per-map `m_objectsStore` Creature/Pet/GO/DynObj (Map.h:690-692).
- Map classes `WorldMap/DungeonMap/BattleGroundMap` (Map.h:309-1032);
  `IsContinent()==id 0|1`; sharding ids 1-6/11-17 off by default
  (MapManager.cpp:636-698, Map.h:181). Grid load: `EnsureGridCreated`
  Map.cpp:303-323, `EnsureGridLoaded` :348-379 (flag set before load),
  `ObjectGridLoader.cpp:225-314` (static + persistent-state GUID merge).
  Unload: `Active→IDLE(one tick)→Removal→UnloadGrid` Map.cpp:1502-1548,
  GridStates.cpp:29-67; BGs never unload grids (Map.cpp:994-1006).
- Tick `Map::Update` Map.cpp:933-1060, order: dynamicTree+sessions
  (:937-951) → `UpdatePlayers` (:957) → `UpdateCells` (:960) →
  `SendObjectUpdates` (:964) → `UpdateVisibilityForRelocations`
  (:967) → `UpdatePlayers` again (:971) → corpses → `!Instanceable()`
  barrier spin re-calling `UpdatePlayers` (:981-993, a third+ site;
  latency filler, not lockstep) → grid states/scripts/weather →
  adaptive distances (:1032-1058). `UpdateSync` (transports only)
  :677-691 runs synchronously before per-map phase; sub-limiters
  UpdatePlayers/Cells/Packets 100ms (World.cpp:949-952);
  `MapUpdateInterval` capped to 50ms (MapManager.h:115-122).
- `UpdateCells`: sync mark-bitset (`marked_cells` 128KiB/Map,
  Map.h:715) over activation squares; cells outside get no update
  (frozen). Async continent striping Map.cpp:743-846, default
  MTCells.Threads=0.
- Visibility: 3 triggers → `Player::UpdateVisibilityOf`
  Player.cpp:18577-18739; distance 2D camera-relative with grey
  hysteresis unit +1yd / flight-object +10yd (Object.cpp:2195-2215);
  base 100/170/533-code(180-example)/100yd; per-viewer
  `m_visibleGUIDs`; transports map-wide blast, never in set
  (Map.cpp:1678-1708). `SendObjectUpdates` Map.cpp:2777-2895: 100ms
  budget, tail deferred, inserts dropped while sending (:2732-2753).
- Bot branch: `HaveRealPlayers` Map.cpp:1778-1784 has exactly one
  caller (`BattleBotAI.cpp:746`); bot-only maps tick normally.
- Sizes: Map.cpp/.h 3728/1049; MapManager 814/230; ObjectGridLoader
  387/117; Maps/ total 20,773 lines; 4 continent thread pools/Map.
- Smallest slice (§e): single-threaded one-continent `WorldMap`,
  no unload (`GridUnload=false` semantics), fixed 100yd, camera=body.
- Unresolved (§f, all preserved): `IsVisibleForInState` bodies unread
  (Unit.h:618, GameObject.cpp:1090, DynamicObject.cpp:269);
  `GetRelocationLowerLimitSq` value; UpdateData wire format not
  opened *in this digest* (covered by entities §A.6 — critic §2.6);
  PlayerBroadcaster semantics; 0-thread pool READY semantics;
  `MapUpdate.Tick.*` defaults; live prod conf (BG 533 vs 180);
  spawn schema (covered by spawn-data); full prod semantic diff;
  GridSearchers/ViewPoint detail.

### 1.2 Movement — `movement.md`

- Build pinned 1.12.1 (5875). `MovementInfo` MovementInfo.h:157-247;
  Read/CorrectData/Write Object.cpp:61-172. Base 28B, max 76B.
  Read `ctime` (:65) vs Write `stime` (:141) asymmetry; order
  transport(+24)→pitch(+4)→fallTime→jump z,cos,sin,xy (+16,
  cos-first trap)→splineElevation(+4); stateful jump latch on
  copy-seeded parse (MovementHandler.cpp:311); `CorrectData`
  strips exclusive flag pairs (:105-136); HOVER check commented
  out (:119-121). Catalog `22-opcode-catalog.md` §7 verified exact.
- Handlers MovementHandler.cpp (1201 lines): 22 generic via
  `HandleMovementOpcodes` :285-403 + 6 speed-ACK + 3 flag-ACK +
  2 root-ACK + teleport/knockback/spline-done/worldport etc. =
  41 StoreOpcode entries, 36 carry MovementInfo. Pending-change
  ACK protocol: counter-tagged, applied on ACK (speed before
  relocation, :491-497), then observers. Relay same-opcode,
  `CMSG_MOVE_FALL_RESET` swallowed (:371-372), SplineDone
  rewritten HEARTBEAT/STOP (:866). Anticheat delegated.
- `MovementPacketSender` 3-step forced changes (487 lines);
  spline engine: `MoveSplineFlag` 32-bit, gravity
  19.29110527038574 (util.cpp:24-32), thread-local spline ids,
  `SMSG_MONSTER_MOVE` 221/_TRANSPORT 686, Runmode always set on
  wire, linear dest-relative 11-bit packed + z-nudge, 80 pts/pkt.
- `MotionMaster` stack (945 lines): bottom default never expired;
  Mutate expires CHASE/HOME/DISTRACT/DISTANCING else interrupts;
  19 generator types, 4 DB-legal. Waypoints: 3 `creature_movement*`
  tables, orientation 100.0=no-facing, flight speed 32.0.
- Speeds `baseMoveSpeed[6]` Unit.cpp:66-73: 2.5/7.0/4.5/4.722222/
  2.5/π; NPC run rate 1.14286; `UpdateSpeed` :7087-7227.
- Sizes: Movement/ 10,834 lines; Rust `mangos-movement` spline-only
  (1,673 src + 467 tests), zero MovementInfo/MotionMaster/waypoint.
  DB write-back exists: WaypointManager repairs coords with UPDATEs
  + DDL renumber (WaypointManager.cpp:133-150,378-420).
- Prod: codec token-identical 2bae5f9↔HEAD; remainder unaudited.
- Unresolved (§f): MovementAnticheat internals (header only);
  generator algorithm bodies (Targeted/Random/Home/Confused/
  Fleeing/Fear/Cyclic/Patrol tail); `StopMoving` tail, pending
  timeout sweep, `HandleFall` formula; default-generator plumbing;
  broadcaster config source; Rust spline parity not re-audited;
  `Spline<T>` curve math black-boxed. Slices M1/M2/M3/M4 — M1 must
  split M1a codec / M1b handler (critic §3.3); chase is M4 there
  but AI needs it earlier (critic §3.1 — resolved in §2 below).

### 1.3 Entities + update fields — `entities-updates.md`

- Tree: Object→Item→Bag; Object→WorldObject→{DynObj, Corpse,
  SpellCaster→{GameObject, Unit(abstract)→{Creature→Pet/Totem/
  TempSummon, Player final}}} (Object.h:111,453; Unit.h:321).
  Identifiers: TypeID 0-7, TypeMask, HighGuid bits 63..48
  (ObjectGuid.h:33-78); TRANSPORT high→TypeId OBJECT quirk (:176-201),
  already in Rust `mangos-guid` (549 lines). Creature subtype
  double-discriminated (class + `m_subtype`).
- Values: one `u32[]` + same-size mirror per object (Object.h:424-433,
  Object.cpp:212-221); dirty = mirror≠values; **single global mirror**
  consumed once per tick for all viewers; `SetUInt64` poisons both
  mirror halves (:1112-1136). Setters mark delayed client update.
- Field tables: compile-time single-build; 11 variants; 5875 =
  324 rows (316 fields + 8 END). 1.12.1 ENDs: OBJECT 6, ITEM 48,
  CONTAINER 122, UNIT 188, PLAYER 1282 (0x502), GO 26, DYN 16,
  CORPSE 38. **Header-comment trap verified:** comments understate
  by 6 — codegen from `.cpp` rows only (critic check #6).
  Flag distribution PUBLIC 176/PRIVATE 78/GROUP_ONLY 20/etc.
- Wire: SMSG_UPDATE_OBJECT 169/DESTROY 170/COMPRESSED 502;
  CREATE = nonzero∩visible, VALUES = changed∩visible, empty
  suppressed; movement section LIVING (MovementInfo + 6 speeds +
  spline) vs HAS_POSITION floats; 32KiB split-before-append;
  zlib if >128B (level 1). Per-viewer rewrites (Object.cpp:576-940)
  in order: NPC-flag strip, attack-time float→u32, stat float→u32,
  GM flags, dynflags, cross-faction, health-as-percent (default,
  `ShowHealthValues=false`), casting-target override, cast-speed;
  GO dyn-flags computed per-target, force-sent every block.
  Children-first: self Player create emits owned Items before the
  player block (Player.cpp:4199-4236).
- Construction: login consumes 22 queries (Player.h:610-635);
  `characters` 58 cols; no whole-values blob — only explored-zones
  (64w) + taxi strings persist raw, rest recomputed. Rust can keep
  the same char-DB schema.
- Sizes: Objects/ 71,517 lines; Player.cpp 22,443 (~35%); Player
  ~286 members through bases; 28 banner domains (inventory 143
  methods~, quests 89, stats 72, …) — one aggregate + domain fns,
  not independent owners. Player values+mirror 10,256B.
- Unresolved (§f): full prod audit (field tables verified equal;
  Object.cpp token proof in rust-gaps supersedes in part — critic
  §4); `CreateCorpse`/`Pet::Create`/Item DB mapping unread;
  ObjectMgr loaders + MovementInfo + visibility owned by siblings;
  config keys beyond 4 wire keys unenumerated; pre-1.12 progression
  support is an open product decision; no `sizeof` measurements.
  Slice is **fixture-characters only** (critic §3.5): real login
  needs read-only Item creates.

### 1.4 Spells / auras — `spells-auras.md` (§14 #7/#50)

- Flow: HandleCastSpell (SpellHandler.cpp:286) → `prepare`
  Spell.cpp:3458 → SpellEvent `update` :4101 → `cast` :3699 →
  immediate/delayed → `DoAllEffectOnTarget` :1284 →
  `DoSpellHitOnUnit` :1699 → `HandleEffects` :5407 → table :5429.
  GCD starts at prepare (:3593), cooldown at cast (:3798); cancel
  in PREPARING refunds GCD (:3632). Damage applied after procs.
- Effects: `TOTAL_SPELL_EFFECTS=134` but table initialises 0-131
  only; 132/133 are **null fn-ptrs** (SpellEffects.cpp:55,187-188;
  critic #7). 96 methods, aliasing (WeaponDmg×4 etc.);
  `EffectSchoolDMG` only accumulates — damage is central
  (Spell.cpp:1519-1579); `EffectDummy` huge per-id switch.
- Auras: `TOTAL_AURAS=193`, 122 handlers; holder ≤3 auras;
  48 visible slots (32 pos + 16 neg) writing UNIT_FIELD_AURA*
  (SpellAuras.cpp:7339-7377); periodic re-arm `timer+=period`
  (:541); PvP heartbeat 12s avg/15s max. Snapshot semantics: caster
  bonus frozen at apply, victim taken-mods per tick (:5675-5766).
- Data is world-DB `spell_template` (SpellMgr.cpp:3530), **not**
  Spell.dbc; 14+ SpellMgr support tables (proc/group/chain/…);
  20 `Load*` fns. Oracles pinned: `CalculateSpellEffectValue`
  SpellCaster.cpp:876, `CalculateSpellDamage` :951, DoT tick
  SpellAuras.cpp:5675, cost/time/duration — golden-fixture ready.
- Sizes: Spells/ 33,357 lines (Spell 8413, SpellAuras 8261,
  SpellEffects 5805, SpellMgr 3888). No Rust spell crate exists.
- Unresolved (§f): `CheckCast` (~1100 lines), FillTargetMap head,
  delayed-launch, absorb/resist, bonus bodies, holder stacking
  decision, aura packets unread — largest single unknown (critic
  §4); script-binding table names at SpellMgr.cpp:2676 unverified;
  char-DB spell persistence; whether any DB row uses effects
  132/133; prod diff uninspected; `AuraTracker` unread.

### 1.5 AI engines — `ai-engines.md`

- No `ReactorAI`/`AggressorAI`, **no SmartScript** (grep 0 hits,
  critic #10). `ReactStates` 0/1/2 (UnitDefines.h:730); generic
  brain `BasicAI`; `CreatureAI` 36 hooks (CreatureAI.h:50,208).
  Selection `selectAI` CreatureAISelector.cpp:37 order:
  possessed→Null, C++ script AI, Pet/Totem, Pet/Guard EventAI,
  AIName registry (9 factories), Guard/Critter, Permissible
  contest (Basic 100 / Event SPECIAL 200 / Null 1), else Null.
  Registry key `"NullAI"` quirk preserved.
- `BasicAI::MoveInLineOfSight` aggro gates (BasicAI.cpp:26-68);
  UpdateAI = SelectHostileTarget→spells→melee (:84-93).
- DB scripts: one loader, 11 `*_scripts` tables, 20-col
  `ScriptInfo` (ScriptMgr.cpp:74-84,1388-1617); **93 commands**
  0-92 (+DISABLED 9999, ScriptCommands.h:35-343); 30 target types;
  bodies ScriptCommands.cpp 2,526 lines; immediate vs delayed
  (`m_scriptSchedule`, Map.cpp:2455-2544). EventAI actions *are*
  these commands (no separate action enum).
- EventAI: **37 types** (2 quest types load-rejected), ≤3 actions,
  32 phases, 500ms poll, chance roll **after** rearm/disable
  (miss consumes cooldown), timers frozen while phase-masked
  (CreatureEventAI.cpp:99-828). Conditions: 63 types (59 leaf +
  NONE/AND/OR/NOT), 12 sources, shared service (Conditions.h:29-285,
  Conditions.cpp:106-1361).
- C++ library: `Script` 25 fn-ptrs; `src/scripts/` 223 cpp /
  103,742 lines. Cadence: `UpdateAI(diff)` every tick creature
  updates; leash/24s-unreachable substitutes evade
  (Creature.cpp:942-960); dead → `UpdateAI_corpse`.
- Rust: `mangos-threat` 986 lines ports tables + 110/130% rule,
  but its 12-method `ThreatHost` has **no non-test implementor**
  (critic §3.8, verified by grep) — unscoped bridge work.
- Unresolved (§f): prod divergence not diffed; live-DB row counts
  for `creature_ai_events`/`creature_ai_scripts`/`conditions`
  unqueried (blocks subset sizing); `CreatureAIImpl.h` /
  ScriptedAI helpers, Guard/Pet EventAI deltas unread; whether
  Rust movement has chase/home generators unverified (it does not
  per movement/rust-gaps); map-event overlap; `PlayerAI` excluded.

### 1.6 Content scripts + FFI-retain — `content-ffi.md`

- Static registration, no FFI: `scripts` STATIC lib →
  `AddScripts()` ScriptLoader.cpp:250 → `ScriptMgr.cpp:2070`.
  **"441 AddSC" REFUTED:** 212 active calls (=212 decls); 216
  definitions, 2 dead (`boss_order_of_silver_hand`,
  `npc_sandstalker`), 1 indirect, 1 empty/commented (critic #11).
- True volume: **749 `new Script` / distinct names / RegisterSelf**;
  slots GetAI 448, GetSpellScript 114, GOGetAI 50, GetAuraScript
  31, GetInstanceData 24, …; subclasses ScriptedAI 309,
  SpellScript 49, GameObjectAI 40, escort 38, AuraScript 30,
  ScriptedInstance 13 direct (24 slots). Whole-tree call surface:
  **675 distinct methods / 14,966 sites**, 174 `dynamic_cast`,
  961 `DoScriptText`, 73 includes via 24-header PCH. Tree:
  258 files, 107,413 LOC (EK 53,989 / Kalimdor 34,626 / …);
  24 `instance_*`, 113 `boss_*`.
- Deadmines floor: 783 LOC, 6 Scripts, 42 distinct methods + 6
  helpers + 17 inbound callbacks; `respawn_delay` sentinels
  43199/43201/43202 smuggle patrol logic from spawn rows
  (instance_deadmines.cpp:83-94) — spawn slices must keep verbatim.
- FFI-retain verdict: viable only as a first-class shim
  re-implementing the script-facing C++ API (scripts subclass core
  classes, hold raw pointers, need RTTI); `AddScripts()` alone
  buys nothing. Alternatives: Rust rewrite instance-by-instance
  (Deadmines first), `USE_SCRIPTS=OFF` degraded mode meanwhile;
  "spells-scripts first" means *smallest*, not earliest (critic §3.9).
- Unresolved (§f): ScriptMgr -w diff (80/120, 17/24) not
  line-inspected (scripts tree sampled formatting-only, 230-file
  -w diff unaudited); which of 749 names prod DB assigns; text-row
  contents 1148/1149/1344/1345; dead-name DB check; 11 indirect
  instance factories; escort/follower/pet bases unread; no Rust
  script host exists to measure.

### 1.7 Spawn / world data — `spawn-data.md`

- Boot order World.cpp:1419-1560 (GO templates → display addon →
  equipment → creature spells/CLS → creature templates → spawns
  → addons → groups → GOs → linking → pools → quests → respawn).
  Loaders (ObjectMgr.cpp 12,038 lines): `creature_template`
  78 cols `max(patch<=WowPatch)` (:1173), `gameobject_template`
  35 cols (:7760, `data0..23` is a per-type union — needs typed
  enum), `quest_template` 131 cols (:5291), `creature` 24-col
  spawn query (:2275), `gameobject` 22-col (:2481), addons /
  equipment / display stores, `creature_spells` 89 cols, CLS 15.
- Dev-bench counts (executed, read-only): creature_template
  10,912 raw / 10,381 effective; creature spawns 66,222;
  gameobject_template 9,007/8,984; GO spawns 56,653;
  quest_template 4,725/4,433; pool_template 7,272; pool GO
  29,478 (**>½ of GO spawns pooled**); creature_groups 5,799;
  game_event 3,064/12,137. Map 0: 24,650 creature + 26,195 GO.
- Semantics: pooled/event GUIDs withheld from grid index
  (:2452,:2624); out-of-patch creatures stay DISABLED, GOs vanish;
  validation clamps are spec (:2317-2619); multi-id `creature_id[5]`
  (8,812 rows) with `id` always the wire entry; respawn deadlines
  live in **character DB** (`creature_respawn`/`gameobject_respawn`,
  MapPersistentStateMgr.cpp:986,1063), delays from world rows.
- Rust: only enum narrowing exists (`enum_stores.rs` 126 lines,
  patch rule already correct); nothing spawn-related yet.
- Unresolved (§f): prod field-level divergence (ObjectMgr ~3,953
  churn lines — needs token-check treatment); `LoadProgressive`
  tie-breaking; `Creature::Create` field→wire tail; PoolGroup
  spawn bodies; linking/game-event table sets; dev vs prod counts;
  `gameobject_requirement` runtime effect untraced.

### 1.8 Rust state + divergence sample — `rust-gaps.md`

- Rust world is an authenticated empty room: 50ms Tokio actor
  (world_actor.rs:46-47), login sets `has_player=true,
  in_world=false` (:680), so all 290 LoggedIn opcodes DropSilent;
  exactly **4 real handlers** (PING/AUTH_SESSION socket,
  CHAR_ENUM/PLAYER_LOGIN actor); 272 other handler names inert
  strings (opcodes.rs:8-19 — Phase 4 must build the
  handler-name→typed-handler mechanism). Census: 828 slots /
  826 registered / 276 handlers; classes Map 154, World 94,
  Movement 39, Async 9, Spells 8. Five C++ queues collapsed to
  one FIFO (world_actor.rs:8-11) — becomes observable once
  handlers are real.
- Login stops before map: 5-packet train only (player_login.rs);
  `SendInitialPacketsBeforeAddToMap` onward missing; 21-query
  snapshot stored `#[allow(dead_code)]`, no `Player` built.
  **No `Player/Unit/Creature/GameObject/Map`/grid/visibility/
  `UpdateData` exists anywhere in Rust** (verified by search).
- Ready leaves: `mangos-guid` 549 (overflow→None deviation noted),
  `mangos-movement` spline+packet-builder byte-golden (no
  Init/MotionMaster), `mangos-threat` pure tables (host trait
  unimplemented), `mangos-world-data` 624 (enum stores only),
  `mangos-protocol` 8,138. `mangosd` 4,335 lines / 13 files.
- Writes today = 7-statement register; `characters.online` stays
  0. Config subset only; no visibility/rate/anticheat knobs read.
- Divergence proof: comment/include-stripped token streams
  order-identical for Map (23,313) / Object (22,167) / Spell
  (44,412) 2bae5f9↔HEAD; only `289bcf53` clang-format touches
  them. 3/434 files only — do not extrapolate (§f).
- Unresolved (§f): tree-wide token check pending (431 files);
  semantic commits 25ae6c39/6f082201/603620f9 uninspected
  (formation/combat-AI = known-divergent neighbourhood); token
  proof limits (preprocessor/macro continuations); stale
  `socket.rs:9-13` BuildAddonPacket comment vs PHASE3-PROGRESS;
  real-client logon owner-pending; bench counts quoted not
  re-queried; ThreatHost census inferred (critic confirmed it).

### 1.9 Completeness critic — `critic.md`

- 14/14 load-bearing spot-checks CONFIRMED (§1 table); no main
  anchor refuted. Corrections in §2 (counts above) and 10
  cross-surface order errors in §3, all folded into §2 below:
  chase must be promoted (§3.1); single owner for intra-object
  `Unit::Update` order (§3.2); M1a/M1b split (§3.3); tick/diff
  model before map slice (§3.4); fixture-only login (§3.5);
  pooled-GO fixtures (§3.6); retire visibility stubs together
  (§3.7); ThreatHost as named work (§3.8); script graduation
  order (§3.9); opcode-count bases (§3.10). Verdict: inventory
  usable for planning after those fixes.

## 2. Final ordered slice plan

Cross-cutting rules (apply to every slice): (i) one owner for
intra-object update order — entities `Unit::update` runs spline →
cast-interrupt check → cast/aura timers → AI, in that documented
slot inside map `UpdateCells` (critic §3.2); (ii) tick model:
50ms actor tick with explicit accumulated `diff` and C++ 100ms
sub-limiters ported as accumulators, settled in Slice 0 — every
timer (cast, aura, EventAI 500ms, scripts) consumes accumulated
diff, never tick counts (critic §3.4); (iii) visibility stubs
(`IsVisibleForInState=true`, GROUP_ONLY-never) retire together
before any multi-client parity claim (critic §3.7); (iv) HEAD
treated as oracle only where §5 allows until tree-wide token
check runs.

### Slice 0 — Codecs + field table + tick contract (no world)

- Entry: none (pure, testable today).
- Scope: MovementInfo codec M1a (movement §e) incl. seeded parse,
  CorrectData, Verify bounds, flags/masks; generated 5875
  update-field table from `UpdateFields_1_12_1.cpp` rows (entities
  §e.2) + per-type flags arrays; written tick/diff contract.
- Exit/oracle: golden byte vectors 28/32/44/52/76B + jump-latch
  sequence + CorrectData pairs vs Object.cpp:61-172; golden
  asserts PLAYER_END=1282, CONTAINER_END=122 (trap check).

### Slice 1 — Entity core + UpdateData (fixture characters)

- Entry: Slice 0; `mangos-guid` (exists).
- Scope: `Values` (words + mirror, typed lanes, poison rules),
  Object/WorldObject/Unit-min aggregates, `UpdateData` builder
  (169/170/502, 32KiB split, zlib>128), movement-section writer
  (LIVING no-spline + HAS_POSITION), per-viewer rewrite subset
  (health-%, NPC flags, dynflags, GO dyn-flags), visibility stub
  behind the real call shape. Player/Creature/GO only; Item/Bag
  deferred — **fixture logins only**; read-only Item creates are
  an explicit sub-step before any real-DB login (critic §3.5).
- Exit/oracle: CREATE blocks byte-compared to C++ sniffs for the
  same fixture spawn; two-client health-% + private-field tests.

### Slice 2 — Spawn data (pure loaders, parallel with Slice 1)

- Entry: DBC display stores, `WowPatch` plumbing (exists),
  `DbActor::call` pattern; no map loop needed.
- Scope (spawn §e order): display addon → equipment → CLS +
  full 78-col `CreatureInfo` (replace/extend enum query — load
  once) → GO template (typed `data` enum) + GO display addon →
  one-map spawn rows **with pool/event joins from day one**
  (pooled = withheld-and-logged) + validation clamps →
  `creature_addon` + grid index → character-DB respawn store.
  Preserve `respawn_delay` verbatim (sentinels, §1.6). Defer
  with logs: PoolManager spawning, groups formations, linking,
  events, transports, creature_spells, quests.
- Exit/oracle: live count + spot-row tests mirroring
  `live_enum_stores.rs` (10,381 / 8,984 effective templates);
  map-0 cell fixture contains exactly the non-pooled GUIDs.

### Slice 3 — One map + visibility + login completion + M1b relay

- Entry: Slices 0-2; session send path; per-class queues +
  handler mechanism (rust-gaps §e.4); terrain heights (assets).
- Scope: single-threaded one-continent WorldMap (map §e): coord
  math, NGrid/cell stores, load-on-demand from Slice 2 index,
  tick in Map.cpp:933-1060 order (sessions→players→cells→
  SendObjectUpdates→relocation visibility→players; no barrier,
  no unload, fixed 100yd), 3-path visibility reduced to
  object+relocation, camera=body; login completion
  (initial-packets train, own create, `characters.online=1`,
  `set_player_state(true,true)`); MovementInfo relay M1b
  (22-opcode handler, anticheat as stub trait with reject-time
  plumbed) + first opcodes (rust-gaps table rows 1-8, 12-14).
- Exit/oracle: scripted two-client stand/move/be-seen in a
  Northshire cell with **non-pooled** GO fixtures (critic §3.6);
  create/destroy pairing + grey-hysteresis sniff vs C++.

### Slice 4 — Server movement: speeds/ACKs + spline launch + chase

- Entry: Slice 3; Detour/pathfinding surface for pathfound moves.
- Scope: M2 (base speeds, rates, `MovementPacketSender` 3-step,
  12 ACK handlers; aura mods behind interface until Slice 5);
  M3 (`MoveSplineInit::Launch` atop existing Rust spline,
  MotionMaster stack with Idle/Point/Waypoint, waypoint loader
  minus DB write-back — log instead, see §6); **minimal Chase +
  Home/evade promoted into this slice** (fixes critic §3.1;
  full Targeted behaviours + fear/flight family stay later).
- Exit/oracle: NPC walks a DB waypoint loop and chases/returns
  home on sniffs; MONSTER_MOVE bytes vs existing golden builder;
  speed-ACK round-trip with counter mismatch drops.

### Slice 5 — Spells/auras minimum (one nuke + one DoT)

- Entry: Slices 1+3 (fields, map tick slot, powers/health);
  `spell_template` 2-row loader + CastTimes/Duration/Radius/
  Range DBC; threat hooks (`mangos-threat` + host started here).
- Scope (spells §e): prepare→timer→cast with CheckCast subset,
  explicit-target FillTargetMap only, 2 effect handlers
  (SCHOOL_DAMAGE accumulate, APPLY_AURA), oracles 1+2 flat,
  1 aura (PERIODIC_DAMAGE) with slot writes + re-arm rule,
  START/GO/FAILED + cooldown wire. No procs/scripts/mods/crit/
  AoE/absorb — each a named follow-up, `CheckCast` first.
- Exit/oracle: golden fixtures for the 4 pinned damage/cost
  functions; client casts, DoT ticks at C++ cadence, aura
  appears/expires in update fields.

### Slice 6 — AI: ThreatHost + BasicAI, then EventAI subset

- Entry: Slice 4 chase, Slice 5 casts; one DB query pass for
  `creature_ai_events`/`creature_ai_scripts`/`conditions` usage
  triage (ai §f) **before** estimating this slice.
- Scope: world-layer `ThreatHost` (12 methods, named deliverable,
  critic §3.8); `select_ai` steps 1,3-8 + BasicAI/NullAI;
  EventAI interpreter (500ms poll, phase/chance order) over the
  ai §e command subset (~20 commands, rest load-disabled);
  Conditions `Evaluate` for rows actually referenced; delayed
  DB-script scheduler. Pet/Guard variants after core.
- Exit/oracle: proximity pull ranges + victim switches match
  C++ on a fixture pack; one EventAI boss timeline (timer casts
  + phase change) replays within one poll (500ms) tolerance.

### Slice 7 — Content: Deadmines vertical slice

- Entry: Slices 3-6 instance-map equivalents (instance maps,
  GO use/state, areatrigger, quest-status query, script texts).
- Scope: owner-chosen path (§4): (a) hand-port Deadmines
  (783 LOC / 6 Scripts / ~45 host methods) to a Rust script
  registry keyed by `script_name`, growing boss-by-boss — the
  inventoried graduation (instances → gossip/world → spells
  scripts → raids → battlegrounds last); or (b) FFI shim crate
  budgeted as "re-implement the script-facing C++ API" starting
  at the same 45-method floor. Until then run scripts-off
  degraded mode (dispatch→None, core AI fallback) explicitly.
- Exit/oracle: full DM clear scripted-client run: doors, cannon,
  Smite 66/33% equips, patrol flips on boss deaths, DMF chest.

Deferred beyond Slice 7 (named, not dropped): grid unload,
transports, continent sharding/barrier, async pools, adaptive
distances, Dungeon/BG map classes, pools spawning, groups/
linking/events, procs, full CheckCast/targeting, anticheat
internals, pets/totems/summons, Corpse/DynObj (slice 5 needs
DynObj only for AoE — pull in when first AoE lands).

## 3. Smallest first implementable milestone

**Milestone A (= Slice 0): MovementInfo codec + 5875 field table.**
Needs no world, DB, or client; unblocks Slices 1+3. Exact first
files to create in `vmangos-rs/` (only edits besides these:
`lib.rs` module lines):

1. `crates/mangos-movement/src/movement_info.rs` — `MovementInfo`,
   `MovementFlags`+masks, `read` (seeded), `write`,
   `correct_data`, `verify` (coord/transport-box bounds),
   `set_as_server_side`.
2. `crates/mangos-movement/tests/movement_info_golden.rs` —
   vectors for base/each-conditional/all (28/52/32/44/32/76B),
   cos-before-sin, stime-slot, latch sequence, CorrectData.
3. `crates/mangos-protocol/src/update_fields.rs` — generated
   324-row 5875 const table (offset/size/type/flags) + END consts
   + per-type flags arrays; generation notes in-module (source
   = `UpdateFields_1_12_1.cpp`, never header comments).
4. `crates/mangos-protocol/src/update_data.rs` — deferred to
   Slice 1; do **not** create in Milestone A.

Oracle: hand-derived bytes from Object.cpp:61-172 (existing
crate golden-test pattern) + `PLAYER_END==1282 &&
CONTAINER_END==122` assert. HOVER branch stays disabled as in
C++ (:119-121) until §4 owner decision — parity first.

## 4. Driver vs owner decisions

Owner decisions (product/parity calls; slices assume the bracketed
default only where stated):

- Content strategy: hand-port by region vs FFI-retain shim vs
  indefinite scripts-off (content-ffi §e). Prior direction on
  record: FFI-retain decided 2026-10-18 — but §1.6 prices it as
  a full API re-implementation; owner to confirm with that price
  or switch to hand-port (Slice 7 (a) vs (b)). **Blocks Slice 7
  only.**
- Pre-1.12 progression builds: 1.12.1-only [assumed] vs all 11
  field/opcode tables (entities §f). Blocks any multi-build claim.
- Spell effects 132/133: replicate null/inert [assumed] vs route
  to `EffectApplyAreaAura`; needs one DB query for row usage
  first (spells §f).
- Parity-vs-fix quirks: HOVER check disabled (Object.cpp:119-121);
  waypoint Reset velocity quirk; adaptive visibility distances
  (port vs fixed = explicit divergence, map §d.4); `GridUnload`
  example-conf 0 vs code-default true; BG distance 533 vs 180 —
  the last two need the live prod conf (map §f).
- Character save scope stays Phase 6; Phase 4 presence writes
  (`characters.online`) + respawn-time writes are in-scope,
  position persistence is not (rust-gaps §c) — owner to confirm
  the write list before Slice 3 (§6).
- Real-client (vs scripted) logon validation remains
  owner-pending (rust-gaps §f); needed before parity sign-off.

Driver decisions (sequencing/engineering, reversible):

- Slice order in §2 (entities+codecs before map; chase promoted;
  M1a/M1b split; spawn loaders parallel as pure data).
- Tick contract mechanics (50ms actor + accumulated diff +
  100ms sub-limiters as accumulators), provided timer semantics
  in §2 rules hold.
- Fixture choices: Northshire start cell, non-pooled GO fixtures,
  fixture characters until read-only Item creates land.
- Waypoint DB write-back: log-only in Rust (no boot-time world-DB
  UPDATE/DDL) unless owner asks for self-healing parity (§6).
- Run tree-wide token check (431 files) + ObjectMgr/ScriptMgr
  samples early — cheap, unblocks oracle claims (§5).
- DB triage queries (EventAI/conditions row usage; effects
  132/133; 749 script names vs prod DB) before Slice 6/7 sizing.

## 5. Architecture §14 — closed vs open (by number)

Source: `docs/RUST_MIGRATION_ARCHITECTURE.md` §14 (items 1-54).
Only items a digest in this inventory directly supports are
listed; everything else is unchanged by Phase 4 (not re-scored).

| §14 # | Status after this inventory | Supporting digest |
|---|---|---|
| 7 (=50) Spell/SpellAuras internals | **Partially closed:** pipeline, dispatch (134/132), aura lifecycle, 4 damage oracles deep-read; still open: CheckCast, targeting head, absorb/resist, stacking | spells-auras §a/§f |
| 17 (=51) Map.cpp:1781 bot branch | **Closed (map half):** `HaveRealPlayers` single caller, no tick gating; BattleGround.cpp:787 half not in any digest — still open | map-visibility §a (bot branch) |
| 35 opcode payload catalog (movement part) | **Closed for MovementInfo:** field list + ACK wrappers byte-pinned, catalog §7 verified; spell-target helper remains Phase 5 | movement §A2-§A3 |
| 11/20/13 (world data/loaders sampled) | **Partially closed:** spawn/template/quest/pool loader SELECTs, boot order, dev-bench counts executed; per-loader exhaustiveness beyond this surface still open | spawn-data §a-§c |

Explicitly **still open** and Phase-4-relevant: #6 live prod
thread/config values (map §f); #9/#41 Anticheat + BG interiors
(movement §f, map BG deferrals); #14/#52 selective large-body
reading (generator + script-library bodies); #34 lock ordering
(no digest covers it); #36 save tables (out of scope, Phase 6);
#49 loaded perf baseline (no populated Rust world yet).
All other §14 numbers: no digest evidence — status unchanged.

## 6. Read-only / no-save risks

1. **Login completion adds writes by design:** `characters.online=1`
   (+ `account.current_realm`) — must join the 7-statement write
   register consciously, not incidentally (rust-gaps §c).
2. **Respawn store writes character DB:** expired-row DELETE at
   load + deadline write-backs (spawn §d.4); a read-only boot
   must skip expiry and accept stale suppression, stated in logs.
3. **Waypoint loader self-heals the world DB in C++** (UPDATEs +
   temp-table DDL, movement §c). Rust must default log-only.
4. **Grid unload persists:** `SaveRespawnTime` on unload
   (map §a) — Slice 3 ships no-unload, so risk is deferred, not
   removed; persistent-state cell GUIDs are the second spawn
   source and must be read even when writes are off.
5. **Script/content mutation is unbounded:** 749 scripts mutate
   entities directly; scripts-off mode is the only read-safe
   default until Slice 7's host exists (§1.6).
6. **Aura/spell state dirties fields that later persist** (auras,
   cooldowns are char-DB state in C++); Slice 5 must keep them
   in-memory only and prove no save path is reachable.
7. **Sanitisation gaps become persisted garbage:** decode-only
   MovementInfo (no CorrectData/Verify) accepts positions the
   oracle discards — Slice 0 includes both for this reason.
8. **Checkout≠production outside token-proven files:** a no-save
   proof on HEAD does not transfer to `2bae5f9` for ObjectMgr/
   ScriptMgr/AI areas until §5 checks run (critic §2.5).

## 7. Complete unresolved register

Every §f item from the nine inputs, grouped by surface (detail in
the per-surface files; nothing here is new evidence):

**Map/visibility:** IsVisibleForInState bodies; relocation
threshold values; PlayerBroadcaster/IsMapSlow; 0-thread pool
READY; `MapUpdate.Tick.*` defaults; live prod conf values
(interval cap vs sub-limiters, BG 533 vs 180, GridUnload);
full prod semantic diff; GridSearchers/ViewPoint detail.
(UpdateData format + spawn schema: resolved by sibling digests.)

**Movement:** MovementAnticheat internals; Targeted/Random/Home/
Confused/Fleeing/Fear/Cyclic/Patrol-tail algorithms; StopMoving
tail; pending-change storage/timeout; HandleFall formula;
SendMovementPacket body; default-generator selection plumbing;
broadcaster config registration; Rust spline parity re-audit;
Spline<T> curve coefficients; generator prod equivalence.

**Entities:** full prod audit beyond field tables + Object.cpp
token proof; CreateCorpse / InitEntry-SelectLevel tails /
Item DB mapping / Pet::Create (incl. pet GUID high); ObjectMgr
SQL detail (spawn-data covers loaders, not every column check);
config keys beyond 4 wire keys; progression-build product
decision; byte-exact `sizeof` per entity.

**Spells/auras:** CheckCast, FillTargetMap head,
HandleDelayedSpellLaunch, CalculateDamageAbsorbAndResist,
SpellDamageBonusDone/Taken bodies, holder stacking decision,
aura apply/remove packets; script-binding table names/counts;
char-DB spell/cooldown persistence + initial spells; DB usage
of effects 132/133; prod diff; AuraTracker wire relevance.

**AI engines:** prod divergence; live row counts for
creature_ai_events / creature_ai_scripts / conditions;
CreatureAIImpl.h + ScriptedAI helpers; Guard/Pet EventAI deltas;
Rust chase/home equivalence (absent — Slice 4 builds it);
map-event/ZoneScript overlap; PlayerAI (bots) out of scope.

**Content/FFI:** ScriptMgr prod -w diff line audit + 230-file
scripts -w audit; prod-DB assignment of the 749 names; text
rows 1148/1149/1344/1345; dead registrations vs DB; 11 indirect
ScriptedInstance factories; escort/follower/pet AI bases;
no Rust host to measure shim against.

**Spawn data:** prod field-level divergence (token check
pending); LoadProgressive tie-breaking; Creature::Create
field→wire tail; PoolGroup::SpawnObject bodies (chance/scaling/
mother-pool); linking + game-event table sets; dev vs prod
counts; gameobject_requirement runtime consumer.

**Rust gaps / divergence:** tree-wide token check (431 files);
commits 25ae6c39/6f082201/603620f9 bodies; token-proof blind
spots; socket.rs BuildAddonPacket stale comment; real-client
logon; bench-count re-query (spawn-data re-queried its own);
first-opcode C++ handler bodies (owned by sibling digests).

**Critic (cross-cutting, open until §2 lands):** corrected
counts must propagate to any estimate built from raw digests
(§2.1-2.3); Unit::Update ownership + tick/diff model are design
obligations, not yet code; "HEAD = production oracle" restricted
to Map/Object/Spell + movement codec until the checks above run.

**Owner decisions open:** content strategy; progression builds;
effects 132/133; HOVER/adaptive-distance/GridUnload/BG-distance
parity calls; Phase 4 write-list confirmation (§4, §6).
