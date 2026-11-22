# Subsystem inventory: live vs dead vs vestigial

Scope: `vmangos/src` tree only. No DB, no configs, no runtime logs.
Bead: `as-pnw.1`. Epic: `as-pnw` (Map VMaNGOS codebase before re-planning Rust port).

Survey date: 2026-10-04. Tree measured at 955 files under `vmangos/src`
(bead text says 929; drift is +26 files, likely local bot/custom work since the estimate).
Breakdown: `game` 493, `scripts` 258 (257 `.cpp`/`.h` + `CMakeLists.txt`),
`shared` 143, `framework` 30, `mangosd` 17, `realmd` 13, plus top-level
`src/CMakeLists.txt` (493+258+143+30+17+13+1 = 955).

## Verdict legend

- **live**: compiled into the default build AND called on a real server path
  (world tick, login flow, GM command, script hook, or socket I/O).
- **vestigial-but-called/compiled**: present in the build but only exercised on
  a non-default path (other OS, other CMake flag, offline tool logic linked
  into the server lib). Called or compiled, but dead in the default Linux
  `USE_SCRIPTS=ON` server.
- **dead**: compiled (or at least listed) but with zero callers or includers
  found by static search. All dead verdicts below are static-only.

## Virtual-dispatch caveat (read before trusting any dead verdict)

Static `grep` for a symbol misses every virtual-dispatch call site. This tree
is heavy on them: `CreatureAI` / `ScriptedAI` overrides, `Aura` handlers,
`Grid` notifiers/visitors (`GridNotifiers.h`, `TypeContainerVisitor.h`),
`FactoryHolder`/`ObjectRegistry` creation, `ScriptCommands` dispatched via
`Map::ScriptCommand_*` function-pointer tables (`Map.h:847-944`), and
`Messager<T>` lambdas (`World::GetMessager`, `LFGQueue::GetMessager`).
A "no callers" result therefore means "no direct callers found", not
"unreachable at runtime". Each dead/vestigial claim lists a dynamic
verification follow-up. Nothing here was verified with a running server.

## Top-level map

| Path | Files | Verdict | Evidence (src-tree only) |
|---|---|---|---|
| `src/CMakeLists.txt` | 1 | live | `add_subdirectory(shared framework realmd game mangosd)` + `if(USE_SCRIPTS) add_subdirectory(scripts)` |
| `src/framework/` | 30 | live | Template infra included across game (`Policies/Singleton.h`, `GameSystem/Grid.h`, `Utilities/EventMap.h` — see `ObjectMgr.h`, `Map.h`, `GridStates.cpp`) |
| `src/game/` | 493 | live (with vestigial pockets) | `game` static lib, `pchdef.h` precompiled header; details per subsystem below |
| `src/mangosd/` | 17 | live | `Main.cpp` + `Master.cpp` world-server entry; `CliRunnable`, `WorldRunnable`; `remote/` RA + SOAP both wired |
| `src/realmd/` | 13 | live | `Main.cpp` + `AuthSocket.cpp` auth flow; `RealmList`, `ClientPatchCache` both referenced |
| `src/scripts/` | 258 | live (with one conditional fallback) | `ScriptLoader.cpp:AddSC_*` registered; `mangosd/CMakeLists.txt:86` + `src/CMakeLists.txt:31` gate on `USE_SCRIPTS` (default `ON` per `vmangos/cmake/options.cmake:4`) |
| `src/shared/` | 143 | live (with vestigial Postgre path) | `shared` static lib; `IO`, `Config`, `Database`, `Auth`, `Crypto` all referenced; Postgre files compiled but inactive by default |

## `framework/` (30 files — all live)

Header-heavy template/policy library. No `.cpp` except
`Policies/ObjectLifeTime.cpp`. All five areas are included by game code:

- `Dynamic/` (`FactoryHolder.h`, `ObjectRegistry.h`) — live. Creature/AI
  registration (`CreatureAIRegistry.cpp`, `CreatureAISelector.cpp`).
- `GameSystem/` (`Grid.h`, `NGrid.h`, `GridLoader.h`, `TypeContainer*.h`) —
  live. Grid/map backbone (`Map.h`, `MapManager.h`, `GridMap.h`).
- `Platform/` (`CompilerDefs.h`, `Define.h`) — live. Included via `Common.h`
  everywhere.
- `Policies/` (`Singleton.h`, `ThreadingModel.h`, …) — live. Every
  `sXxxMgr` singleton (`sObjectMgr`, `sWorld`, `sLFGMgr`, `sGuardMgr`, …).
- `Utilities/` (`EventMap`, `EventProcessor`, `LinkedReference`, `TypeList`,
  `ByteConverter`) — live. `EventMap` used by AI/scripts; `LinkedReference`
  used by `Group`/`Follower`/`HostileRef` managers.

## `game/` subsystems

### AI/ (41 files) — live

`CreatureAI`, `BasicAI`, `CritterAI`, `GuardAI`, `GuardEventAI`,
`CreatureEventAI(+Mgr)`, `CreatureAIRegistry`, `CreatureAISelector`.
`BasicAI::SummonGuard` calls `sGuardMgr.SummonGuard` (`BasicAI.cpp:95`);
`CreatureEventAI.cpp:28` includes `GuardMgr.h`. Registry/selector is the
virtual-dispatch hub — static caller counts understate reachability.

### Anticheat/ (19 files) — live

`Anticheat.cpp/.h` + `MovementAnticheat/` + `WardenAnticheat/`
(`Warden.cpp`, `WardenWin/Mac`, `WardenModule/Scan*Mgr`).
Live: `WorldSession.cpp:1074` `sAnticheatMgr->CreateWardenFor(...)`,
`:1082` `CreateAnticheatFor(...)`; `World.cpp:1804/1869` load + start
Warden thread, `:206` stop. `SniffFile` packet dumps wire into
`WorldSession.h:317` and `MovementAnticheat.cpp:146`.

### AuctionHouse/ (4 files) — live

`AuctionHouseMgr` + `AuctionHouseBotMgr`. Handler path:
`Handlers/AuctionHouseHandler.cpp`; bot manager is the AH-bot population
control. No dead verdict.

### Battlegrounds/ (11 files) — live

`BattleGround{,Mgr,AB,AV,WS}`. Handler: `Handlers/BattleGroundHandler.cpp`.
Bot wiring: `PlayerBotMgr::AddBattleBot` (`PlayerBotMgr.cpp:595-641`)
instantiates `BattleBotAI`; auto-join tick at `:298-353` gated by
`BattleBot.AutoJoin` config (`World.cpp:975`).

### Chat/ (14 files) — live

`Chat`, `Channel(+Mgr)`, `AbstractPlayer`, `MasterPlayer(+Chat)`,
`AsyncCommandHandlers`, `MonsterChatBuilder`. `MasterPlayer`/`AbstractPlayer`
are referenced from `AccountMgr`, `Map.cpp`, `Channel.cpp`, `Mail.cpp`,
`SocialMgr.cpp`, `WorldSession.cpp`, `PlayerBots/PlayerBotAI.cpp` — the
player-chat abstraction is load-bearing, not dead.

### Commands/ (11 files) — live

`*Commands.cpp` (Account, Character, Creature, Debug, Lookup, Misc, Object,
Server, Teleport, Ticket, Unit). Registered via `Chat.cpp` command tables
(e.g. `sendMassCommandTable` at `Chat.cpp:585`); `CharacterCommands.cpp:4885/
4955` drives `PlayerDumpReader/Writer`; `ServerCommands.cpp:1875` reloads
`AuraRemovalMgr`, `:1812` reloads weather.

### Database/ (11 files) — live

`DBCStores`, `DBCStructure/Enums/fmt`, `SQLStorages`,
`CharacterDatabaseCache/Cleaner`. `CharacterDatabaseCache` is hot:
`Objects/Pet.cpp:115-603`, `Objects/Player.cpp`, `Spells/Spell.cpp:6916`,
`PlayerBots/CombatBotBaseAI.cpp:2694`, `ObjectMgr.cpp`.

### Group/ (7 files) — live

`Group(+RefManager/Reference)`, `CreatureLinkingMgr`. Group LFG hooks call
`sLFGMgr` (`Group.cpp:413/459/500`) and `sWorld.GetLFGQueue().GetMessager()`
(`Group.cpp:454/496/591`); `Group.cpp:425/531` call `sGuildBotMgr`.

### Guild/ (4 files) — live

`Guild`, `GuildMgr`. Handler: `Handlers/GuildHandler.cpp`.

### Handlers/ (27 files) — live

Per-opcode `WorldSession` handlers (Addon, Auction, BG, Channel, Character,
Chat, Combat, Duel, GM ticket, Group, Guild, Item, Loot, Mail, Misc, Movement,
NPC, Pet, …). This is the opcode-to-logic dispatch layer; `Opcodes.h` is in
`pchdef.h:11` and included across game.

### LFG/ (6 files) — live

`LFGMgr`, `LFGHandler`, `LFGQueue`, `LFGDefines`. Meeting-stone flow is live:
`LFGHandler.cpp:77` + `Maps/ScriptCommands.cpp:1209` call
`sLFGMgr.AddToQueue`; `Group.cpp:648-711` calls role/priority helpers.
`LFGQueue::Update()` (the blocking match loop, `LFGQueue.cpp:60`) runs on a
dedicated `LfgUpdate` thread gated by patch/config (`World.cpp:1864-1866`).
Do not mistake the Messager indirection for dead code.

### Mail/ (4 files) — live

`Mail`, `MassMailMgr`. `MassMailMgr::AddMassMailTask` posts async character-DB
queries (`MassMailMgr.cpp:73`); `Master.cpp:342` ticks `sMassMailMgr`;
`Chat.cpp:585` exposes `sendMass*` GM commands; `World.cpp:674` configures
`MassMailer.SendPerTick`.

### Maps/ (38 files incl. Pool/, ZoneScript area) — live

`Map(+Manager)`, `GridMap`, `GridNotifiers(+Impl)`, `GridSearchers`,
`GridStates`, `InstanceData`, `MapPersistentStateMgr`, `ObjectGridLoader`,
`ScriptCommands`, `Pool/PoolManager`, zone scripts. `ObjectGridLoader` is
constructed per grid (`Map.cpp:369`); `ScriptCommands.cpp:2226-2260` serves
`Join/LeaveCreatureGroup`; pool groups drive spawn selection
(`PoolManager.cpp:722-1079`).

### Movement/ (45 files) — live

`MotionMaster`, all `*MovementGenerator`s (Idle/Home/Confused/Fear/Fleeing/
Cyclic/Point/Waypoint/…), `spline/*`, `FollowerRefManager`.
`MotionMaster.cpp:26` includes `CyclicMovementGenerator.h`, `:584-602`
implements `MoveCyclicWaypoint`; fear/flee/confuse generators are referenced
from `Unit.cpp`, `MotionMaster`, `MovementHandler`.

### Objects/ (67 files) — live

`Object`, `Player`, `Creature`, `GameObject`, `Item`, `Bag`, `Corpse`,
`DynamicObject`, `Pet`, `Corpse`, `MirrorTimer`, `MovementInfo`,
`ObjectGuid`, `ObjectAccessor`, `ObjectPosSelector`, `Unit*`, ….
`ObjectPosSelector` is used by `Object.cpp:2579`; `Camera` (`Camera.cpp/.h`)
backs `Player::GetCamera()` (`Map.cpp:1380/1628/1642`,
`GridNotifiers.cpp:34-167`, `GridDefines.h:62-67` grid camera type).

### OutdoorPvP/ (5 files) — live but narrow

`OutdoorPvPEP/SI` + `Register.cpp`. `RegisterZoneScripts()` gates both zones
on `SUPPORTED_CLIENT_BUILD > 1.11.2`, patch `>= 112`, and
`CONFIG_BOOL_OUTDOORPVP_{EP,SI}_ENABLE`. Only EPL (EP) and Silithus (SI)
exist — no EPL/WG-style coverage, but that is scope, not death.

### PacketBroadcast/ (5 files) — live

`MovementBroadcaster`, `PlayerBroadcaster`, `ChatCommands`. Referenced from
`Maps/Map.cpp`, `GridNotifiers.cpp`, `Objects/Player.cpp` + `Object.cpp`,
`World.h/.cpp`, `Handlers/CharacterHandler.cpp`, `PlayerBots/PlayerBotAI.cpp`.
`ChatCommands.cpp` is the live `pbcast` GM debug surface
(`HandlePBCastStatsCommand`, `HandlePBCastSetThreadsCommand`).

### PlayerBots/ (36 files) — live (custom stack)

`PlayerBotAI/Mgr`, `PartyBotAI`, `CombatBotBaseAI`, `BattleBotAI(+Waypoints)`,
`BotBehavior(+ClassUnique/Shared)`, `BotDirector`, `BotFormation`,
`BotRoleConfig`, `BotServiceMgr`, `BotSituationContext`, `BotSpellKit`,
`BotChatManager`, `BotCombatEventBuffer`, `GuildBotMgr`. Wired via
`World.cpp`, `Group.cpp`, `PlayerBotMgr.cpp` (see Battlegrounds/Guild notes).
Per `CLAUDE.md`, new behaviors extend `BotBehavior`; nothing here is dead.

### Server/ (7 files + Protocol/) — live

`WorldSession(+Mgr)`, `WorldSocket(+Mgr)`, `WorldPacket`,
`Protocol/Opcodes*.h` (1.5.1 through 1.12.1, all included by `Opcodes.h:39-53`).
Older opcode headers are version tables, not dead code — they ship
unconditionally. Socket I/O runs on the custom `shared/IO` stack
(`WorldSocketMgr.cpp:95` PROXYv2 handshake).

### Spells/ (18 files) — live

`Spell(+Mgr/Entry/Defines)`, `SpellAuras`, `SpellEffects`, `AuraTracker`,
`SpellModMgr/Modifier`, `SpellClassMask`. `SpellAuras.cpp:4141` serves the
Love-Is-In-The-Air gossip path; `Spell.cpp:6916` consults the pet cache.

### Threat/ (4 files) — live

`ThreatManager`, `HostileRefManager`. Core combat aggro path.

### Transports/ (4 files) — live

`Transport(+Mgr)`. Ship/zeppelin path.

### vmap/ (23 files — 22 live, 1 dead header, 1 vestigial tool)

Runtime LOS/collision: `BIH(+Wrap)`, `DynamicTree`, `GameObjectModel`,
`MapTree`, `ModelInstance`, `RegularGrid`, `VMapManager2`, `VMapFactory`,
`VMapDefinitions`, `WorldModel.cpp/.h` (compiled at
`game/CMakeLists.txt:234`, used by `GameObjectModel.h:46` and
`ModelInstance.h:70-84`). Two exceptions:

- `vmap/VMapTools.h` — **dead header**. Listed in `game/CMakeLists.txt:468`
  but zero includers anywhere in `vmangos/src` (only its own include guard
  matches). Content is a G3D BSP-tree iterator helper. Dynamic check: build
  with the header removed from the CMake list; confirm no compile break.
- `vmap/TileAssembler.cpp/.h` — **vestigial-but-compiled**. The `.cpp` IS in
  the build (`game/CMakeLists.txt:231`) and the header in `:464`, but zero
  references to `TileAssembler`, `AssembleTile`, or
  `calculateTransformedBound` exist outside the pair itself — the single
  out-of-file hit is a comment in `vmap/VMapDefinitions.h:30`
  ("defined in TileAssembler.cpp currently..."). This is offline vmap-baking
  logic linked into the server lib, never called at runtime.
  Dynamic check: confirm no server startup/log path touches it; consider
  moving it to a tools target.

### Loose `game/*.cpp/.h` singletons — all live

`AccountMgr`, `AuraRemovalMgr` (tick via `Map.cpp:432`, reload via
`ServerCommands.cpp:1875`), `AutoBroadCastMgr` (singleton + `Update`),
`Camera` (see Objects/), `Conditions`, `CreatureGroups`
(`CreatureGroup::AddMember/OnMemberAttackStart/OnMemberDied/OnLeaveCombat/
OnRespawn` in `CreatureGroups.cpp:25-144`, driven via `Map.cpp:3681`,
`ScriptCommands.cpp:2226-2260`, `GridNotifiers.h:945-949`),
`Formulas.h`, `GMTicketMgr` (`sTicketMgr` in `Player.cpp:127`,
`TicketCommands.cpp`), `GameEventMgr` + `HardcodedEvents`
(`GameEventMgr.cpp:261` loads hardcoded list; `HardcodedEvents.cpp:1824`
implements it), `Geometry.h`, `GossipDef`, `GuardMgr`
(`sGuardMgr.SummonGuard` from `Creature.cpp:3523`, `BasicAI.cpp:95`,
tick at `World.cpp:2044`), `HonorMgr`, `InstanceStatistics`
(`sInstanceStatistics` in `Unit.cpp:1239`, `World.cpp:1317`,
`instance_naxxramas.cpp:1040-1046`), `ItemEnchantmentMgr`, `Language.h`,
`LootMgr`, `LoveIsInTheAir.h` (`SpellAuras.cpp:4141`),
`ObjectAccessor`, `ObjectGridLoader` (see Maps/), `ObjectGuid`,
`ObjectMgr`, `ObjectPosSelector` (see Objects/), `PlayerDump`
(`CharacterCommands.cpp:4885/4955`), `QuestDef`, `ReputationMgr`
(`Player.cpp:6485/6508`, `ObjectMgr.cpp:5476`), `ScriptMgr`,
`SharedDefines.h`, `SniffFile` (see Anticheat/), `SocialMgr`
(`Map.cpp:3389`, `MasterPlayer.cpp:435`, `WorldSession.cpp:765-766`,
`CharacterHandler.cpp:657`, `MiscHandler.cpp:534-598`),
`StatSystem.cpp`, `UnitAuraProcHandler`, `UnitEvents.h`, `Weather`
(`sWeatherMgr` in `World.cpp:1529`, `ServerCommands.cpp:113/1812`),
`World(.cpp/.h)`, `WorldStates.h`.

## `mangosd/` (17 files — live)

`Main.cpp` (entry), `Master.cpp` (boot, includes `MassMailMgr.h:38`,
`remote/RemoteAccess/RASocket.h:48`, `remote/soap/MaNGOSsoap.h:49`; ticks
mass mail at `:342`; RA socket start at `:117`), `CliRunnable`,
`WorldRunnable`, `mangosd.conf.dist.in`, Windows `.ico/.rc` (platform
conditional — vestigial on Linux, live on Windows; not dead).
`remote/RemoteAccess/RASocket.*` (GM remote console) and
`remote/soap/MaNGOSsoap.*` (SOAP endpoint) are both included and started —
no dead remote stack. `monitor-mangosd` / `run-mangosd` are ops helpers.

## `realmd/` (13 files — live)

`Main.cpp` (entry, stands up `MailerService` at `:156-157`, PROXYv2 accept at
`:259`), `AuthSocket.cpp/.h` (auth flow; `ClientPatchCache.h:28` include,
patch-hash check at `AuthSocket.cpp:551`, SendGrid mail at `:681-689`),
`AuthCodes/Packets.h`, `RealmList`, `ClientPatchCache`
(`sRealmdPatchCache` preloaded at `Main.cpp:209`), `realmd.conf.dist.in`,
Windows `.ico/.rc` (same platform note as mangosd).

## `scripts/` (258 files — live, one conditional fallback)

- `ScriptLoader.cpp/.h` — live. `AddSC_*` registry (e.g.
  `AddSC_npc_j_eevee()` at `ScriptLoader.cpp:478`, declared at `:235`).
  Both loader variants are stripped from the scripts-lib source list
  (`scripts/CMakeLists.txt:279-281`, "Used in game ScriptMgr, not need here")
  because the game lib consumes the entry point — that is layering, not death.
- `ScriptLoader_noscripts.cpp` (`void AddScripts() {}`) —
  **vestigial-but-called under `USE_SCRIPTS=OFF`**. Default build is
  `USE_SCRIPTS=ON` (`vmangos/cmake/options.cmake:4`), so this empty fallback
  ships unreferenced in the default configuration and activates only in
  no-scripts builds. Dynamic check: configure `OFF` once and confirm the
  server boots scriptless.
- `battlegrounds/` (2), `spells/` (11 class/item/special scripts),
  `eastern_kingdoms/` (146), `kalimdor/` (70), `world/` (20 incl.
  `npc_j_eevee`, `dragons_of_nightmare`, `scourge_invasion`,
  `elemental_invasions`, `world_event_wareffort`, seasonal
  `love_is_in_the_air.h`/`midsummer.h`) — all live content scripts.
  `npc_j_eevee` is genuinely used (`dreadsteed_ritual.cpp:132-134`
  summons entry 14500 and `dynamic_cast`s to `npc_j_eevee_dreadsteedAI`).
- `custom/` (`custom.cpp/.h`, `custom_creatures.cpp`) — live.
  `AddSC_zero_scripts()` calls `AddSC_custom_creatures()`.
- `PrecompiledHeaders/scriptPCH.*` — live build infra.

## `shared/` (143 files — live, one vestigial DB path)

- `Auth/` (`AuthCrypt`) — live (auth session crypto; `BigNumber`-adjacent).
- `Config/` — live (`sConfig` across `mangosd/Main/Master/CliRunnable`,
  `realmd`, `game/World`).
- `Crypto/` (`BigNumber`, `Authentication/SRP6`, `Encoding`, `Encryption`,
  `Hash/MD5/SHA1/HMAC`) — live. `BigNumber` referenced from hash/SRP6 and
  session key paths (`WorldSession` Warden creation takes `BigNumber* K`).
- `Database/` — live MySQL path; **Postgre trio vestigial-but-compiled**:
  `DatabasePostgre.h/.cpp`, `QueryResultPostgre.h/.cpp`,
  `PGSQLDelayThread.h` are all in `shared/CMakeLists.txt:62-99`, yet the only
  non-self references are `#ifdef DO_POSTGRESQL` / `#ifndef DO_POSTGRESQL`
  guards (`DatabaseEnv.h:29`, `DatabasePostgre.cpp:22`,
  `QueryResultPostgre.cpp:22`, plus the MySQL-side `#ifndef` mirrors). No
  `#define DO_POSTGRESQL`, compiler `-D` flag, or CMake option defines it
  anywhere in `vmangos/src` or `vmangos/cmake`, so the default build compiles
  the Postgre sources but never selects `DatabasePostgre` as `DatabaseType`
  (defaults to `DatabaseMysql`, `DatabaseEnv.h:38-44`).
  Dynamic check: confirm no CI job passes `-DDO_POSTGRESQL`; candidate for a
  `USE_POSTGRESQL` option or deletion.
- `IO/` (custom ASIO-style lib: `IoContext`, `Networking/AsyncSocket*`,
  `Filesystem`, `Multithreading`, `Timer`, `ReadableBuffer`, `Utils`) — live.
  This is the socket backbone: `WorldSocket*`, `WorldSocketMgr.cpp:95`,
  `realmd/AuthSocket`, `realmd/Main.cpp:259`, `mangosd/Master.cpp:117`,
  plus `IO::Multithreading::CreateThreadPtr("LfgUpdate", …)` at
  `World.cpp:1866`. The `README.md` comparison to Boost ASIO is current.
- `Memory/`, `Multithreading/Messager.h`, `nonstd/expected|optional.hpp`,
  `json.hpp`, `ByteBuffer`, `Common`, `Log`, `Timer.h`, `Util`, `Duration`,
  `EnumFlag`, `Errors`, `LockedQueue`, `ThreadPool`, `ThreadSpecificPtr`,
  `TimePeriod`, `ProgressBar`, `RealmZone` — all live infra.
- `MailerService`, `SendgridMail` — live in `realmd` (see realmd/ notes).
  Not used by `mangosd`/game — that is layering, not death.
- `ProxyProtocol/ProxyV2Reader` — live (`WorldSocketMgr.cpp:95`,
  `realmd/AuthSocket.cpp:44`, `realmd/Main.cpp:39/259`).
- `PosixDaemon` / `ServiceWin32` — platform-conditional pair, each vestigial
  on the other OS, live overall. Same for `WheatyExceptionReport`
  (Windows crash reporting) and the `.ico/.rc` resources.
- `SystemConfig.h` — live build-config header.

## Dead-code rollup (static-only; dynamic checks required)

| # | Location | Verdict | Static evidence | Virtual-dispatch risk | Dynamic follow-up |
|---|---|---|---|---|---|
| 1 | `game/vmap/VMapTools.h` | dead header | Listed in `game/CMakeLists.txt:468`; zero `#include "VMapTools.h"` in tree | Low (template header; risk is a forgotten tool, not a vtable) | Remove from CMake list, rebuild; confirm clean |
| 2 | `game/vmap/TileAssembler.cpp/.h` | vestigial-but-compiled | In build (`game/CMakeLists.txt:231/464`); zero callers of `TileAssembler`/`AssembleTile` outside the pair | Low (concrete class, no vtable dispatch into it) | Confirm no runtime/log path; move to a tools target or gate behind option |
| 3 | `shared/Database/DatabasePostgre.*`, `QueryResultPostgre.*`, `PGSQLDelayThread.h` | vestigial-but-compiled | Compiled (`shared/CMakeLists.txt:62-99`); only non-self refs are `#ifdef`/`#ifndef DO_POSTGRESQL` guards; no `#define`, `-D` flag, or CMake option sets it | None (concrete DB driver) | Confirm no CI passes `-DDO_POSTGRESQL`; decide option-vs-delete |
| 4 | `scripts/ScriptLoader_noscripts.cpp` | vestigial in default build | Empty `AddScripts(){}`; active only when `USE_SCRIPTS=OFF` (default `ON`) | Medium (script registry is macro/function-pointer driven) | Configure one `OFF` build; confirm scriptless boot |

No other dead subsystems found. In particular, the usual suspects are all
live: Warden (`WorldSession.cpp:1074`, `World.cpp:1804/1869`),
`LFGQueue::Update` (dedicated thread, `World.cpp:1866`), `BattleBotAI`
(`PlayerBotMgr.cpp:595-641`), `ClientPatchCache`
(`AuthSocket.cpp:551`, `Main.cpp:209`), `MassMailMgr`
(`Master.cpp:342`), `SniffFile` (`WorldSession.h:317`), `Camera`
(grid camera type + `Player::GetCamera` callers), `InstanceStatistics`
(Naxxramas wipe/kill counters), `GuardMgr` (guard-summon path).

## Dynamic verification follow-ups (recorded, not executed)

Static-first per epic `as-pnw` instructions; these belong to later map beads:

1. Boot `mangosd` + `realmd` from this tree; exercise login → character → world
   (feeds protocol/persistence maps `as-pnw.2`, `as-pnw.3`).
2. Enable Warden + movement anticheat; confirm thread starts and sessions
   attach (anticheat is statically live but behaviorally unverified here).
3. Join a meeting-stone queue; confirm `LfgUpdate` thread processes
   add/remove (Messager indirection hides this from static call graphs).
4. Trigger guard summon, AH bot tick, mass-mail tick, weather reload, GM
   ticket flow — each is one GM command away and would close the
   virtual-dispatch gap for the singletons listed above.
5. Build with `VMapTools.h` removed from CMake, and one `USE_SCRIPTS=OFF`
   configure, to discharge dead-code rows 1 and 4.

## Method note

`find vmangos/src -type f | wc -l` for counts; per-directory `ls` + `find`
for L2 enumeration; `game/CMakeLists.txt`, `shared/CMakeLists.txt`,
`scripts/CMakeLists.txt`, `src/CMakeLists.txt`, `vmangos/cmake/options.cmake`
for build membership; `grep -rn <symbol> vmangos/src --include='*.cpp'
--include='*.h'` for caller evidence, with same-file self-hits filtered.
Quoted `file:line` evidence above is the static record.
