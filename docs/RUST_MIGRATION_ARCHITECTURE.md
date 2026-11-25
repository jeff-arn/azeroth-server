# Architecture for the C++→Rust Migration

**Purpose.** This document is the evidence base for a future Muse goal
that migrates the C++ server (`vmangos/`) to Rust. It describes the codebase
and the live production rig as they are today, the contracts a Rust
implementation must keep, a proposed crate decomposition, and a phased
sequencing with verification per phase (§13). It is not itself a commitment
to migrate, and it does not supersede [../CLAUDE.md](../CLAUDE.md) (rules)
or [ARCHITECTURE.md](ARCHITECTURE.md) (how things work today). Where they
disagree with code, code wins; known documentation drift is catalogued in
the evidence reports (10 §3, 11 §1).

**Snapshot.** Surveyed 2026-10-15/16 against checkout `3589f72`
(2026-05-26) and the live droplet `jeffs-awesome-droplet` (checkout twin
`aaf9b5e`, same commit subject). Production binaries there were built
2026-04-13/14 and do not map to that checkout (Preface, top finding).
Re-verify any fact marked with a date before acting on it.

**How to use this for the migration goal.** Start with §5 (persistence
contract), §11 (ranked risks), and §13 (sequencing). §14 lists every
question this survey could not answer — a goal agent must treat those as
open work, not as license to assume. §15 collects the owner decisions and
Phase 0 evidence-gathering that should precede any Rust in production.

**Method.** Ten parallel scoped surveys (build, runtime, world sim,
database, bots, assets, Python services, live droplet over read-only SSH,
metrics, docs/history), a completeness critic that spot-checked claims
against source bodies, and this synthesis. The full reports are archived
in [rust-migration-evidence/](rust-migration-evidence/) (`01`–`11`, plus
raw droplet captures `raw-*.txt`); citations below such as "05 §8" point
into them, and `file:line` citations point into this repo. Decisive claims
were independently re-corroborated after the survey (e.g. `vmangos/src`
= 461,753 physical lines; 826 `StoreOpcode` registrations against
`NUM_MSG_TYPES=828`; the inverted `SRP6::Proof`; live `mangosd-main`
73 threads / ~1.35 GiB RSS, up 31 d under systemd since 2026-09-14).
Counts that depend on the counting predicate are quoted with their method
or as ranges. Anything labelled *(inferred)* was not directly observed.
Omitted scope is disclosed in the Preface and §14.

## Preface — status, precedence, and what this is not

- **Production phase**: `production` — real characters exist; data is sacred
  (`CLAUDE.md:16-23`). Four locks bind every migration step: append-only DB
  migrations, shape-stable dashboard JSON, additive bot behaviours, locked
  save formats (§5, §8).
- **Precedence**: `CLAUDE.md` wins for rules; `docs/ARCHITECTURE.md` wins for
  how things work today (`CLAUDE.md:50`). Where the two disagree, code wins
  over both (10 §3, 11 §1 #6/#7 confirm documentation drift in counts,
  port, driver, and endpoint tables).
- **Binary≠checkout (top finding, 11 §0)**: production runs a `mangosd`
  binary built 2026-04-14 (08 §6: 19,838,640 B, `ldd` = system libs) while the
  surveyed checkout HEAD is 2026-05-26 (`aaf9b5e` on the droplet, `3589f72`
  twin on the workstation, 08 §6). All behavioural claims below describe the
  checkout, not proven live behaviour. [Update 2026-10-16 (Phase 0): the
  map now exists — both binaries embed `2bae5f921f59892d4c41 /
  2026-04-12 01:47:07 +0000` (commit
  `2bae5f921f59892d4c41002a0869b154f02867d3`); the surveyed checkout is
  10 commits ahead, so production predates the formation system
  (`25ae6c3`). See `rust-migration-evidence/PHASE0-PROGRESS.md`.]
- **Quantitative humility (11 §2)**: handler/SQL/allocation censuses differ
  by counting predicate between reports. This synthesis quotes the critic's
  directly measured values (§2, §3) and gives ranges elsewhere.
- **Omitted by design**: spell internals beyond dispatch, Anticheat/Warden,
  core LFG, BG/ZoneScript/Honor internals, SOAP/RA surfaces, PostgreSQL
  backend, per-opcode payload catalogues, per-lock inventory, live DB
  contents/config values, real asset bytes — all
  named in §14. Nothing below should be read as covering them.
- **From-scratch baseline (owner direction, 2026-10-16)**: a former
  `vmangos-rs/crates` workspace no longer exists and must not be assumed,
  surveyed, or reconciled against. The migration starts from an empty
  Cargo workspace; `vmangos-rs/` in this checkout contains only a stale
  `target/` build directory, which is residue, not source. Historical
  notes about that workspace in the evidence reports (04 §1.2, 11 §4 #1)
  and project memory are superseded by this paragraph; where a prior
  dev-MySQL/`TEST_MYSQL_URL` harness is mentioned, treat it as a design to
  recreate in Phase 0/1, not as existing infrastructure.

## 1. System overview

VMaNGOS-derived WoW 1.12.1 (client build 5875) private server, fully owned
with no upstream relationship (`CLAUDE.md:3`), plus a custom bot stack,
a Flask dashboard, and a Discord bot. Two C++14 daemons share MySQL;
Python services are a read-mostly shell over the same MySQL instance.

```
            WoW 1.12.1 client (build 5875)
              | SRP6 (3724)          | game packets (8085)
              v                      v
          +--------+  sessionkey  +---------+
          | realmd |----(login--->| mangosd |
          | (auth) |     DB)      | (world) |
          +--------+              +----+----+
              |                        |
              +----- MySQL (4 DBs) ----+--- DataDir: 5875/dbc, maps,
              |  realmd|mangos|chars   |    vmaps, mmaps (2.9 GiB, live)
              |  logs                  |
     +--------+--------+             |
     | dashboard       | discord-bot |  pgrep mangosd | LevelUp.log |
     | Flask :8080     | discord.py  |--/tmp/azeroth-maintenance ---+
     +-----------------+-------------+
     systemd: azeroth-{realmd,mangosd,dashboard,discord-bot} (live host)
```

- `realmd` (auth, :3724): SRP6 logon, realm list; links only shared+framework
  (`vmangos/src/realmd/CMakeLists.txt:57-85`) — the smallest migration unit.
- `mangosd` (world, :8085): world simulation, bots, SOAP/RA (off by default);
  one static blob: `game scripts shared framework g3dlite gsoap`
  (`vmangos/src/mangosd/CMakeLists.txt:86-145`).
- No ACE, no Boost: hand-rolled `IO::IoContext` (epoll/kqueue/IOCP)
  (`vmangos/src/shared/IO/Context/IoContext_linux.cpp:52-93`; IO lives in
  `src/shared/IO/`, correcting 01 §3 per 11 §1 #9) + single-threaded 50 ms
  world tick.
- Dashboard (`dashboard/dashboard.py`, 936 lines, waitress :8080 at
  `dashboard/dashboard.py:932-936`) and Discord bot (`discord-bot/bot.py`,
  461 lines) talk to the daemons only via MySQL, `pgrep mangosd`,
  `LevelUp.log`, and `/tmp/azeroth-maintenance` (§8).

### Subsystem map (mangosd)

| Layer | Contents | Anchor |
|---|---|---|
| Entry | `Master::Run`, startup phases, signals | `vmangos/src/mangosd/Master.cpp:129-306` |
| Network | `WorldSocket`, 5 packet classes, opcode table | `WorldSocket.cpp:82-140`; `PacketProcessing.h:25-121` |
| World | `World::Update` phase order, sessions, timers | `World.cpp:1965-2134` |
| Simulation | Map/Grid/Cell, Objects, Spells, Movement, AI | `Map.cpp:933-1060`; §4 |
| Content | `scripts/` (107k LOC) + EventAI/DB scripts | `ScriptLoader.cpp` (441 `AddSC_`) |
| Bots | PlayerBots 24,254 LOC + AH bot 863 | §6 |
| Persistence | 4 DB globals, delay-thread queues | `DatabaseEnv.h:51-54`; §5 |
| Static data | 46 DBCs + maps/vmaps/mmaps + world-DB loaders | §7 |

## 2. Build / target / dependency map and LOC sizing

### 2.1 Build facts

- CMake `3.1...3.20`, C++14 required, no extensions, default Release
  (`vmangos/CMakeLists.txt:19-25`; `cmake/options.cmake:46-48`); 34
  `CMakeLists.txt`; out-of-source enforced. Client build pinned:
  `SUPPORTED_CLIENT_BUILD=CLIENT_BUILD_1_12_1` (`cmake/options.cmake:11`).
- Default build = 11 targets: `mangosd`, `realmd` + `shared, framework,
  game, scripts, g3dlite, Detour, Recast, gsoap, cpptrace-lib`
  (`src/CMakeLists.txt:25-33`; `dep/CMakeLists.txt:18-34`). All static;
  there is no plugin/`.so` seam — the only process seams are
  realmd↔mangosd and the DB (01 §2).
- `game` STATIC links `shared`+`Detour` (`src/game/CMakeLists.txt:595-604`);
  `scripts` links `game` (back-edge, `src/scripts/CMakeLists.txt:339-347`) —
  a Rust crate graph must break this cycle via registry/trait.
- System deps (UNIX, REQUIRED): MySQL, OpenSSL, ZLIB via custom finders
  (`vmangos/CMakeLists.txt:112-130`); CURL only for mailsender. Bundled:
  g3dlite, Detour+Recast (only Detour linked by `game`), gsoap stubs,
  cpptrace, header-only nlohmann/doctest, libmpq (extractors only).
  TBB references are dead (no `find_package(TBB)`; 11 §4 #5). Whether the
  generator links Recast was not verified (11 §4 #5).
- Flags: `--no-warnings -fexceptions -pipe`, `-march=native` default ON
  (host-specific binaries), no `-Wall/-Werror`
  (`cmake/platform/unix/settings.cmake:15-18`; `options.cmake:3`).
  `mangosd` adds `-pthread -rdynamic`, Carbon on APPLE
  (`src/mangosd/CMakeLists.txt:86-145`).
- Configure generates `revision.h`/`Progression.h`/`migrations_list.h` **into
  the source tree** (`vmangos/CMakeLists.txt:173-180`); the migrations glob is
  baked in — the Rust equivalent is a `build.rs` glob (§5).
- Install: prefix forced to `../server` if unset; `bin/`+`etc/`+`lib/`,
  `run-mangosd` restart script, `*.conf.dist` templates
  (`vmangos/CMakeLists.txt:56-86`; `src/mangosd/CMakeLists.txt:147-150`).

### 2.2 LOC sizing (physical lines; method in 09 §10)

Net porting surface: `vmangos/src` = 461,753 physical lines (535 `.cpp`,
404 `.h/.hpp`, ~357k code). Minus vendored-in-src (json.hpp/nonstd, 19,232)
and pre-1.12.1 opcode headers (~5,400) = **≈437k physical / ~335k code**
(09 §9). Vendored `dep/` = 250,245 lines, excluded (crate/FFI/drop); SQL
847,566 lines/2,519 files is data, retained; Python 4,429 lines retained.

| Subtree | Physical LOC | Share | Note |
|---|---:|---:|---|
| `src/game` (incl. PlayerBots 24,254) | 308,545 | 66.8% | Objects 70,964; Spells 33,357; Commands 23,591 |
| `src/scripts` | 107,413 | 23.3% | 223 files; EK 53,989 / Kalimdor 34,626 |
| `src/shared` | 38,057 | 8.2% | First-party ≈19k after vendored headers |
| `src/framework` | 3,181 | 0.7% | Header-mostly |
| `src/realmd` | 2,882 | 0.6% | First Rust binary candidate |
| `src/mangosd` | 1,675 | 0.4% | Entry + SOAP/RA glue |
| `dep/` (vendored, excluded) | 250,245 | — | Only recast 19,398 + libmpq 4,899 plausibly need FFI |
| C++ tests | ~340 | 0.07% | 6 files; links whole game lib (09 §8) |

Concentration: top-5 `.cpp` (Player 22,443; ObjectMgr 12,038; Unit 11,073;
Spell 8,413; SpellAuras 8,261) = 62,228 = 13.5% of `src` (09 §4). A
quoted Rust size of "250–340k lines" (09 §9, 0.7–1.0× ratio) is
uncalibrated guesswork (11 §4 #4) — *(inferred, unproven)*.

## 3. Runtime topology — processes, threads, queues, locks, protocol

### 3.1 Processes and the realmd↔mangosd contract

No IPC socket exists. The contract is **shared login DB only**: realmd writes
`account.sessionkey` synchronously (`AuthSocket.cpp:704-707`,
`MustBeSync`); mangosd reads it in `CMSG_AUTH_SESSION`
(`WorldSocket.cpp:263-282`) and rejects `last_ip` mismatch (`:300-308`);
mangosd publishes liveness via `realmlist` writes (`Master.cpp:207,317`);
realmd polls `RealmList` (`RealmList.cpp:163-172`). Freeze: login-DB schema
+ synchronous sessionkey visibility before the client reaches mangosd.

### 3.2 Startup as a phase barrier

`main` → `Master::Run` spawns `IO[i]` (`Network.Threads`=1 default), loads all
static data synchronously (`sWorld.SetInitialWorldSettings()`), enables async
DB, then blocks on `WorldRunnable` join (`Master.cpp:167-199,306`). Do not
accept logins before static load completes.

### 3.3 Thread map (defaults from `*.conf.dist.in` only — live values unknown)

| Thread(s) | Count | Role |
|---|---|---|
| `mangosd-main` | 1 | Startup, join, ordered teardown |
| `IO[i]` | `Network.Threads`=1 | `IoContext::RunUntilShutdown`; all socket callbacks |
| `WorldRunnable` | 1 | 50 ms/20 Hz tick, `sWorld.Update(diff)` |
| `SystemTimer` | 1 | One-shot timeouts (no-auth kick) |
| `DB:<name>` delay | 1 per DB (4) | FIFO `SqlDelayThread` queue, own connection |
| `SqlResult[0..5]` | 6 per DB (≤24) | Async query callbacks (`SqlOperations.cpp:151`) |
| `AsyncPacket` | 1 | ASYNC-class packets, fenced from world/CLI |
| `CharDB`, `WardenSessions` | 1 each | Cleanup (30 min); anticheat scans |
| `LfgUpdate` | 0/1 | `LFGQueue::Update()` 1 s loop |
| `WorldUpdate[i]` | `AsyncTasks.Threads`=1 | Per-tick async tasks, joined each tick |
| `MapManager[i]` | 2 default | Instanced-map fork-join |
| Continent + per-map pools | config | `MapObj/MapMotion/MapVis/MapCell` (`Map.cpp:149-158`) |
| MovementBroadcaster | 0 (off) | Sharded by player GUID % N |
| `CLI`/`FreezeDetector`/`SOAP` | 0/1 each | Stdin queue; watchdog; gSOAP |

Sources: `Database.cpp:186-228`, `World.cpp:1866-1874`, `Map.cpp:149-158`
(02 §3). Live process has 73 threads (08 §4) — not reconciled with these
defaults (11 §3). `realmd` is effectively single-threaded for protocol:
`MainIoCtx` + DB delay/result threads (`realmd/Main.cpp:333`).

### 3.4 Queues (every cross-thread handoff — 02 §8)

| Queue | Producer → consumer | Sync |
|---|---|---|
| `WorldSession::m_recvQueue[5]` | IO → world/map/async | mutex+deque, **unbounded** |
| `WorldSocket::m_sendQueue` | any → IO drainer | mutex+atomic_flag, cap **1024** |
| `World::addSessQueue` | IO → world | LockedQueue |
| `World::cliCmdQueue` | CLI/RA/SOAP → world | LockedQueue |
| `World::Messager` | any → world | mutex + swap-vector |
| `World::_asyncTasks` | world → WorldUpdate pool | mutex + double-buffer |
| `IoContext::m_contextSwitchQueue` | any → IO (eventfd) | mutex+queue |
| `SqlDelayThread` + serial queues | any → DB delay thread | LockedQueue, FIFO per serialId |
| `SqlResultQueue` | delay → callback pool/world | ThreadPool |
| `ThreadPool` fork-join | world/mapmgr → workers | per-worker queues, `future` |
| Timer deque | any → SystemTimer | mutex+condvar |
| MovementBroadcaster shards | world/maps → workers | `shared_timed_mutex` per shard |

Locks: ~147 lock sites / 45 files by regex (09 §6) but **no named per-lock
inventory or ordering exists** (11 §3); 41 `shared_timed_mutex` mentions span
Map store, ObjectMgr, ObjectAccessor, Config, etc. `BattleGroundMgr` guards
are **commented out** — effectively unlocked (11 §1 #9). Cross-thread
`volatile` flags (`m_stopEvent`, `m_worldLoopCounter`, `m_isRunning`) must
become atomics in Rust (02 §8). The code itself flags the
`WorldSocket` raw `WorldSession*` race (`World.cpp:245`).

### 3.5 Network protocol surface (1.12.1 — frozen by the client binary)

- Framing: `ClientPktHeader {u16 BE size, u32 cmd}`, valid
  `4 ≤ size ≤ 0x2800`, `cmd < 828`; header-only `AuthCrypt`; one packet in
  flight per socket (`WorldSocket.cpp:82-140`). **Correction (11 §1 #8)**:
  the malformed branch (`:105-109`) returns without closing or re-arming —
  the connection wedges open until the no-auth timer, it is not dropped.
- Egress: send-queue cap 1024 ⇒ disconnect; coalescing on the IO thread;
  session cap 0x8000 (`WorldSocket.cpp:516-590`; `WorldSession.cpp:104-112`).
- Opcodes (guide-safe numbers, 11 §1 #2): `NUM_MSG_TYPES=828`
  (`Opcodes_1_12_1.h:830`); **826** `StoreOpcode` registrations; **276**
  distinct handlers, **272** non-stub; `STATUS_NEVER`=525 exact. Status enum
  `Opcodes.h:140-148`; table `Opcodes.h:152-201`, `Opcodes.cpp:33-46`.
  Reports 02 (825/275/271) and 09 (827) are superseded.
- Five processing classes WORLD/MAP/SPELLS/MOVEMENT/ASYNC with documented
  thread-safety contracts (`PacketProcessing.h:25-121`; routing
  `WorldSession.cpp:252-282`). Only PING + AUTH_SESSION run inline on IO
  (`WorldSocket.cpp:159-178`). Aliases: SELF_ITEMS→MAP, DB_QUERY→ASYNC,
  CHANNEL/GROUP/GUILD→WORLD. These contracts — not handler bodies — are the
  Rust concurrency design input.
- `ByteBuffer`: LE cursor, bounds-checked reads throw
  (`ByteBuffer.h:50-490`); exceptions caught at exactly two boundaries
  (`WorldSocket.cpp:181-199`; `WorldSession.cpp:528-553`) — in Rust, both
  become `Result` → log/kick.

### 3.6 Auth (SRP6)

- realmd: 8-command state machine (`AuthSocket.cpp:117`), synchronous
  sessionkey write (`:704-707`); mangosd verifies
  `SHA1(account|0|seeds|K)` (`WorldSocket.cpp:353-374`).
- `SRP6::Proof()` is inverted vs its documentation
  (`SRP6.cpp:156-164` vs `SRP6.h:74-80`): returns `false` on memcmp match;
  `!Proof` means success. Do not port the condition literally.
- The critic read the full `SRP6.cpp` body (11 §1 #4): N `:31`, g=7 `:32`,
  k=3 in `B=(v*3+g^b)%N` `:39`, WoW interleave `HashSessionKey :127-154`,
  verifier hash `:97-125`. Test vectors are derivable, with one caveat:
  `CalculateVerifier` feeds member `A` into the hash (`:114`) — verify `A`
  is empty/zero at account-creation time before trusting vectors. The
  preserved gap "SRP6 internals not fully read" (02) is therefore largely
  closed by 11, but is carried in §14 as instructed.
- Config: `sConfig` map + `shared_timed_mutex`, first-wins duplicates
  (`Config.cpp:42-59,146-150`); World typed snapshots (`World.h:961-964`).
- Signals: SIGINT=restart / SIGTERM=shutdown, SIGSEGV anticrash (deliberately
  unsafe — do not replicate, 02 §9); freeze detector polls
  `m_worldLoopCounter`, terminates, **default off** (`Master.cpp:69-100`).
  Exit codes 0/1/2 (restart) are part of the contract.

## 4. World simulation architecture and god-objects

Core size excluding PlayerBots: 283,427 LOC; `Objects/` 70,964 + `Spells/`
33,357 dominate (03 §1). The core is one circular cluster —
Object/ObjectMgr/Map/Grid/Player/Unit/Spell/Aura/AI/Script — with no
internal FFI seam (03 §11).

- **Hierarchy**: `Object > WorldObject > SpellCaster > Unit > {Creature,
  Player final}` + separate `Item` branch (`Objects/Object.h:111,453`;
  `SpellCaster.h:315`; `Creature.h:80`; `Player.h:746`). Type is a runtime
  tag (`m_objectTypeId`), not the C++ type; cheap unchecked downcasts are
  assumed everywhere. Creature subtypes (pet/totem/temp-summon) are
  flag-discriminated (`Creature.h:146-152`). `Item` has no position/map —
  the one clean boundary (03 §2.1).
- **GUID** is load-bearing: `u64 high|entry|counter` frozen layout
  (`ObjectGuid.h:66-78,129-130`), stored in the object's own update-field
  array (`Object.h:135`); generators `ObjectMgr.h:1437-1444`. GUIDs are the
  cross-system foreign key (DB rows, packets, groups, threat, scripts).
  DB columns store only the 32-bit low counter (§5).
- **Ownership**: raw `new`/`delete` (count contested: 913 sites in game
  excl. bots per 03 §3 vs game-only `new` 659 + `delete` 419 = 1,078 per
  09 §5 — see 11 §2 #3; quote as "≈900–1,100, method-dependent").
  `Map` deletes via deferred remove-list (`Map.cpp:1720-1776`); lookups via
  per-map store (`Map.h:512`) + global `HashMapHolder`
  (`ObjectAccessor.h:44-82`). Intrusive Reference/RefManager pairs plus
  deferred deletion are the dangling-pointer answer. Rust: Map-owned
  slotmaps/arenas keyed by `ObjectGuid`, command-buffer deletion, handles.
- **Update fields**: untyped `u32` union array + mirror, 12 per-build
  layouts (`Object.h:424-431`; `UpdateFields.h:27-45`); replication
  `Map.cpp:2777-2895`. This is the single most migration-hostile pattern:
  faithful first port keeps `Vec<u32>` + index consts (03 §2.3).
- **Map tick**: variable ms diffs, multi-rate gates, checkerboard-parallel
  cells, per-object catch-up clock (`Map.cpp:819-847,933-1060`;
  `Object.h:460-483`; geometry `GridDefines.h:38-59`: 64×64 grids,
  16×16 cells). World phase order + fences `World.cpp:1965-2134`,
  `WorldRunnable.cpp:38-39,66-109` (verified 11 §1 #1). Tick semantics are
  gameplay-visible and must be replicated, not "fixed" to a fixed step.
- **Movement**: `MotionMaster` generator stack (20 types,
  `MotionMaster.h:36-59,82`); splines are server-computed but
  client-executed, so shapes must stay behaviour-compatible;
  `PathFinder`/Detour queries are not thread-safe (`MoveMap.h:44-89`);
  vmap LoS `IVMapManager.h:65-66`.
- **Spells/threat**: `SpellMgr` (`SpellMgr.h:256`); effect/aura/proc
  handler tables (`Spell.h:229`; `Unit.h:188-189`); separable
  `ThreatManager` (`ThreatManager.h:193`, 1,042 LOC) — ports first.
  Internal flows beyond headers/dispatch were not deep-read (03 §12).
- **Scripts — three layers**: core AI (`CreatureAI.h:50`), 223 content files
  / 107,413 LOC (441 `AddSC_` in `ScriptLoader.cpp`), DB EventAI +
  `Maps/ScriptCommands.cpp` 2,526 (`ScriptMgr.h:216-251`). Script *names*
  and DB opcodes are production DB contracts. EventAI/DB-script/Conditions
  interpreters are high-leverage early ports (03 §9).
- **Social/economic** systems are cross-map GUID singletons (30+ `s*Mgr` by
  prose count in 03 §10; the measured figure is 55 singleton macros across
  `src`, 09 §6): `Group` (`Group.h:184-194`), `GuildMgr`
  (`GuildMgr.h:34`), `AuctionHouseMgr` holds `Item*`
  (`AuctionHouseMgr.h:151-159`). **Core LFG exists** (`LFG/LFGQueue.cpp`,
  `LFGQueue::Update :60`) — 05's "no queue" is bot-scope only (11 §2 #5).
  One-row inventories (channels, transports, pools, game events) are not
  migration evidence (11 §3).

God-objects (difficulty ranking, hardest first, 03 §11): (1) `Player`
(25,008 LOC incl. header) + `Unit` (12,608) + update-field model;
(2) `Spell`/`SpellAuras`/`SpellEffects`/procs (24,479); (3) C++ content
scripts (107,413 — volume, not complexity); (4) Map update/visibility/
threading semantics; (5) Movement/PathFinder policy; (6) social/economic
singletons; (7) static loading, GUID type, spline math, packet
serialisation, threat, DB-script interpreters.

## 5. Persistence contract — what must stay byte-compatible

Four DB globals `World/Character/Login/Logs` (`DatabaseEnv.h:51-54`);
mangosd opens all four (`Master.cpp:469`), realmd Login only
(`realmd/Main.cpp:78`); mangosd co-writes `realmlist` (`Master.cpp:207`)
— exactly one owner per startup side-effect in any split deployment.

- **DB layer**: sync pool (1–16) + 1 async connection + `SqlDelayThread`
  serial queues (`Database.h:295-331`; `SqlDelayThread.h:32-39`); deferred
  thread-local transactions (`Database.cpp:388-397`), fire-and-forget error
  model, no nesting. Faithful Rust shape: actor/task per DB with a
  serial-lane key (GUID-low); naive per-query spawn breaks ordering.
- **Query census (method matters, 11 §2 #2)**: 1,142 `Database.` call lines
  (any method, 04 §3.3) vs 831 anchored Execute/Query-family sites (09 §7)
  vs ~1,300 broad ceiling. Printf style dominates (`PExecute` 361 +
  `PQuery` 202 per 04; 405 + 266 per 09) with `escape_string` on only 72
  lines and a 32 KiB `vsnprintf` buffer (`Database.cpp:346-364`);
  prepared statements (174 `SqlStatement` lines) dominate save paths.
- **Rows are positional and stringly-typed**: `Field` is `atol`/`atof`
  (`Field.h:47-58`, NULL→0); character load is 58 columns `fields[0..57]`
  (`Player.cpp:14472-14485`) inside a 23-query holder
  (`Player.h:612-635`). Only the `characters` row + items are
  transcription-deep; the other ~28 save tables (auras/cooldowns/mail/pet/
  BG/…) are named only (11 §3).
- **Save**: `BeginTransaction(GUIDLow)` (`Player.cpp:16246`), 61-bind
  `REPLACE INTO characters` (`:16253`; corrected 2026-10-16 in Phase 0 by
  direct placeholder/column count — earlier surveys said 62), `_Save*` bundle
  (`:16391-16405`). Text blobs: `known_taxi_mask`, `explored_zones` (64
  words), `equipment_cache` (MAKE_PAIR32) at `:16319-16382` — write `''`,
  never NULL. Items: `REPLACE/UPDATE item_instance` state machine
  (`Item.cpp:273`); `charges`/`enchantments` space triples (`:281-295`);
  inventory as separate placement rows (`Player.cpp:16642-16664`);
  `random_property_id` read i16 / written u16 (`Item.cpp:297,421`).
- **Schemas**: `characters.sql` 55 tables (48 MyISAM — no transactional
  rollback to rely on), `logon.sql` 11, `logs.sql` 15; **no world dump in
  repo** (`CLAUDE.md:129`). Migrations: 1,035 `.sql` (1,005 world / 18
  characters / 6 logon / 6 logs), CMake-generated list
  (`migrations_list.h.cmake:1-31`), boot check (`Database.cpp:570`),
  template `make_migration.py:5-25`. The Rust binary must embed the same
  glob and refuse boot on missing entries.
- **Loaders**: 96 `Load*` definitions in `ObjectMgr.cpp`
  (`LoadQuests :5291`), 84 invoked from `World.cpp:1306-1838` with ordering
  constraints (`:1505,1532`); 9 `SQLStorage` tables
  (`SQLStorages.cpp:39-47`). 06's "~120 calls / ~98 tables" counts a wider
  set (11 §2 #4) — pick one definition in the guide.
- **Byte-compat contract** (04 §8, load-bearing verbatim): schema names/
  engines; migration gate; positional SELECT order (append only); blob
  encodings (empty-not-NULL); `atol` coercion + `f32` rounding; low-GUID
  only; `REPLACE` shapes; per-character serial ordering; dual-writer
  login-DB ownership; epoch-int timestamps; charset/escaping parity.
- **No prior Rust work to build on (owner direction, 2026-10-16)**: a
  former `vmangos-rs/crates` workspace (~190 crates, incl.
  `mangos-realmd-mysql`) and its `TEST_MYSQL_URL`-gated live-test harness
  (proven realmd DB logon, 11/11, on 2026-09-27) no longer exist; memory
  of them is historical only. This checkout's `vmangos-rs/` holds only a
  stale `target/` directory. Phase 0/1 therefore includes recreating from
  scratch: the Cargo workspace, the dev-MySQL fixture setup (schemas from
  `vmangos/sql/*.sql` + `*_characters.sql` migrations in timestamp order),
  and env-gated `--ignored` live DB tests in that shape. Do not cite the
  former 11/11 logon result as current verification of anything.

## 6. Bot stack architecture and its coupling surface

`PlayerBots/` = 36 files, 24,254 lines; largest `CombatBotBaseAI.cpp` 3,829,
`BattleBotAI.cpp` 3,026, `PlayerBotMgr.cpp` 2,427; AH bot +863 lines in
`AuctionHouse/AuctionHouseBotMgr.{h,cpp}` (05 §1). Production rule:
behaviours are **added as new layers, not rewrites** (`CLAUDE.md:22`).

- **Bots are real `Player`s with fake sessions**:
  `WorldSession::m_bot` (`WorldSession.h:801`);
  `Player::IsBot() = GetBot()!=nullptr` (`Player.h:2024`). A *second*
  identity coexists: `accountId >= 100` = bot (97 Crowd, 98 Guild, 99 AH,
  100+ Party/Dynamic; `.claude/rules/bots.md:148-153`). Site count for the
  account check is unverified (~25 vs ~10 inside 05 itself; crude grep 32;
  11 §2 #6). Collapse into one typed `Controller` concept in Rust.
- **Runner (cleanest port target, verified)**: `BotBehaviorSet` scores
  `Score(ctx)*roleWeight`, fixed 32-slot stack array (silent truncation),
  sort descending, `Execute` fallthrough (`BotBehavior.cpp:16-81`,
  `Candidate[32] :27`; 11 §1 #3). `BotBehavior` is a 3-method trait-like
  interface (`BotBehavior.h:22-29`). Do not replicate the 32 cap silently.
- **Behaviours**: 39 declared (19 shared + 20 class-unique; docs say 18+19,
  `ARCHITECTURE.md:85` — stale, 10 §3 #2). `BehaviorCasterAttackStop` is
  declared but never registered; Mage has 0 unique. Effective = 38.
- **Tick**: `PartyBotAI` ticks 500 ms (`PartyBotAI.cpp:48`): pre-steps
  (dispel/pet/autoshot), `EvaluateSituation` snapshot with raw `Unit*`
  fields (`BotSituationContext.h:11-73` — GUIDs or scoped borrows in Rust),
  combat/OOC behaviour sets (`BotRoleConfig.cpp:25-145`).
- **BattleBotAI bypasses behaviours entirely**: 18 `final` per-class
  virtuals (`BattleBotAI.h:66-83`) over `{}` defaults
  (`CombatBotBaseAI.h:143-164`) + recorded waypoint data (1,089 lines,
  ports as static arrays). Two-engine decision is unavoidable (§11, §13).
- **Formation**: `BreadcrumbTrail` 40 points / 1 s sampling, role slots
  2/4/8/10 yd, navmesh + 12 yd hostile validation, 2 yd dead-zone
  (`BotFormation.h:43-45,131-154`; verified 10 §3 #3). Never fires in combat
  (`!ctx.currentTarget`). Primitives: `MovePoint` ×26, `MoveChase` ×24,
  `MoveFollow` ×3.
- **Social/services tick on the 60 s `WUPDATE_AUCTIONS`** timer
  (`World.cpp:1992-1995`): `BotChatManager` (2,123 lines; personalities,
  chains, reactive hooks; deferred chat uses absolute `time_t fireTime`,
  never ms countdowns — `.claude/rules/bots.md:34-46`), `BotServiceMgr`
  (957 lines; portal/enchant/transmute/WTB state machine,
  `BotServiceMgr.h:52-90`). Bot "LFG" is chat flavour only (core LFG is
  separate, §4). `BotDirector`: 2-min cycle, GUID pools 33-52/53-72/73-92,
  writes `playerbot.chance` 30/1 (`BotDirector.cpp:37-42,65-70,346`).
  `GuildBotMgr`: roster in `guild_bot_roster`, 5-min level sync, presence
  peak 14 / off-peak 3, `hasLootUpgrades` protects looted gear.
  Encounter learning: 256-slot ring buffer → `bot_learned_patterns`.
- **Talents**: `scripts/generate_talent_builds.py` (643 lines) prints C++
  for 26 builds (`:117,586-643`) → `s_talentBuilds[]`
  (`CombatBotBaseAI.cpp:271`) consumed by `LearnPremadeSpecForClass`
  (`:2754`). Regenerate, never hand-edit. Port as `build.rs` codegen or a
  checked-in data file.
- **Coupling (the crate-boundary decider)**: 52 `IsBot/GetBot`/singleton grep
  hits outside PlayerBots across 15 core files (World, WorldSession,
  Player, `Unit.cpp:767,952`, `Spell.cpp:3719`, `Channel.cpp:676-683`,
  `TradeHandler.cpp:528-533`, `GroupHandler.cpp:157`, `Group.cpp:425`,
  `Chat.cpp` 72 command refs, `SharedDefines.h:104` `CombatBotRoles`).
  Hits ≠ sites (11 §3); `Map.cpp:1781` and `BattleGround.cpp:787` contexts
  are unread; the "9 synthesised session handlers" and the
  `OnPacketReceived` SMSG set are not enumerated by name. The Host-API
  conclusion is sound; its exact surface is not yet enumerable.
- **Singletons (8)**: PlayerBotMgr, BotChatMgr, BotServiceMgr, GuildBotMgr,
  BotDirector, BotFormationMgr, BotPatternMgr, AuctionHouseBotMgr → owned
  fields on one `BotSystem` struct. DB tables: `playerbot`,
  `guild_bot_roster`, `guild_bot_spec_weights`, `bot_learned_patterns`,
  `auctionhousebot`; ~35 config keys, ~18 absent from `.dist` (05 §10).
  Save suppression (`PlayerBot.AllowSaving`, default false) is
  load-bearing for production data — model it in the type of the session.

Critical gotchas to preserve (10 §4, each a shipped-and-reverted bug):
`Score()` must check preconditions; absolute deadlines on the 60 s tick;
formation combat guard; PW:S never on a warrior tank >15% HP; one
curse/warlock/target (Recklessness/Fear banned); `.partyrole` must set
`m_resetSpellData`; `BotSpellKit`-only spell access for all classes.

## 7. Static data / binary formats and reuse verdicts

One knob: `DataDir` (`World.cpp:904`); live value `"."` with the unit
WorkingDirectory as `run/bin/` (08 §6). Boot loads ~120 ordered `Load*`
calls (definition-dependent, §5). **No real asset bytes are in this
checkout** (gitignored, `.gitignore:8-14`) — every verdict below rests on
paired loader+writer source (06 §10) and is therefore code-reading-correct
but byte-unproven (11 §4 #7). Production DataDir fixtures are a hard
prerequisite, not a footnote.

| Format | Evidence | Verdict (LE targets) |
|---|---|---|
| DBC (`.dbc`) | WDBC 20 B LE header, field-count check only (`DBCFileLoader.cpp:47-90,187`); 46 active loads vs constant 45 (`DBCStores.cpp:182-184,193-414`; verified 11 §1 #6); no `Spell.dbc` — spells from `spell_template` (`SpellMgr.cpp:3535,3550`); 59 fmt strings; `pack(1)` structs (`DBCStructure.h:39`) | **Reuse unchanged.** Small parser; transcribe 46 fmts + locale first-non-empty merge. `scripts/dbc.py` + tests as oracle. |
| Terrain (`.map`) | `MAPS` + `z1.4`, 40 B header, AREA/MHGT/MLIQ, 3 height encodings (`GridMap.cpp:34-38,85,164-206`; `GridMapDefines.h:5-17`) | **Reuse unchanged.** Port 3 encodings + interpolation bit-exactly (f32). |
| VMap (`.vmtree/.vmtile`) | `VMAP_7.0` (raw `VMAPs05`), field-wise `ModelSpawn` + BIH `u32` tree (`VMapDefinitions.h:26-27`; `ModelInstance.cpp:178-222`; `BIH.cpp:255-284`) | **Reuse unchanged.** Port BIH wire tree + spawn format; add bounds checks C++ lacks. |
| `temp_gameobject_models` | No magic; EOF-terminated records (`GameObjectModel.cpp:41-68`) | **Reuse unchanged.** Trivial. |
| MMap params (`.mmap`) | Raw `dtNavMeshParams` struct dump, no magic (`MoveMap.cpp:85-87`) | **Fragile.** Parse field-wise; layout = vendored Detour headers. |
| MMap tiles (`.mmtile`) | `MMAP` v6, 20 B header + opaque Detour payload; `dtVersion` unchecked by core (`MoveMap.cpp:159-195`; `MoveMapSharedDefines.h:25-37`) | **Only via compatible Detour (FFI)** or reimplementation; else regenerate. |
| Config (`.conf`) | Hand-rolled quirks (first-wins, `#`/`[` abort, 1024-byte cap; `Config.cpp:69-153`); 648 mangosd + 33 realmd options in `.example` | **Reuse unchanged** if quirks replicated. |
| World DB (MySQL) | ~100 query shapes + patch-selection + load order (§5) | **Reuse same DB.** Port = row-mapper transcription. |

Only DBC loading does endian conversion (`ByteConverter.h:57-77`); maps/
vmaps/mmaps are raw LE `fread`. Extractors (`contrib/`, `CMakeLists.txt:
20-23`) are offline one-way producers — they do **not** need porting for a
Rust server to run. Unread: `GroupModel` GMOD per-field layout
(`WorldModel.cpp:271,330` — located, not line-audited); vendored Detour
version / `DT_NAVMESH_VERSION`; extractor inputs (MPQ/ADT/WMO/M2).

## 8. Python services and pinned cross-language contracts

Neither Python service migrates; both pin contracts a Rust pair must not
break (07). Dashboard: 23 routes (7 HTML, 16 JSON), waitress :8080
(`dashboard/dashboard.py:932-936`). Discord bot: 2 commands + 4 loops,
zero DB writes. Both import `mysql.connector` (`dashboard/dashboard.py:10`;
`bot.py:8`) — CLAUDE.md's "PyMySQL" and "port 5000" are drift (07 §1,
verified 11 §1 #7).

- **Shape-stable set**: the `CLAUDE.md:21` rule over
  `docs/ARCHITECTURE.md:331-352`. `/api/characters`, `/api/items`,
  `/api/guilds` are listed but **do not exist**; `/api/activity :261`,
  `/api/auctions :315`, `/api/bots :369`, `POST /api/guild/gear-tier :730`,
  `DELETE /api/addons :906` exist but are unlisted — treat as de-facto
  stable (tests/templates consume them). `dashboard/tests/test_api_shapes.py:
  95-344` locks keys/types harder than the prose rule. Read handlers
  swallow exceptions into empty 200s (`dashboard.py:257-258,311-312`) —
  validation needs live-DB payload diffs, not just shape tests.
- **Sole shared write**: dashboard `UPDATE`s `characters.guild_bot_roster`
  (`spec, role, gear_tier, raid_assigned`) at `dashboard.py:688,713,719,
  746,751`; C++ `GuildBotMgr` SELECTs + UPDATEs the same columns
  (`GuildBotMgr.cpp:58,480,501,691`); ENUMs in migrations
  `20260325140000:15` + `20260326100000:11`, re-validated in Python
  (`dashboard.py:666,680,739`). Three-way agreement must hold byte-for-byte.
  Whether C++ re-reads the roster after startup is **untraced** (07 §10,
  11 §4 #8) — a guide promising dashboard-edit liveness needs this answer.
- **Schema pinning**: cross-schema joins pin `realmd`/`mangos`/`characters`
  on one MySQL instance (`dashboard.py:234,325,629`; `bot.py:73`); exact
  capitalisation pinned (`item_template.Quality`, `ItemLevel`); raid-lock =
  any `creature_disabled` row for maps {409,249,469,309,509,531,533}
  (`dashboard.py:116-124,351-362`). No SQLite/Postgres swap without
  rewriting both services.
- **Auth pins SRP6 storage**: `dashboard.py:404-415` + `auth.py:20-64`
  recompute the verifier from hex `v`/`s`; test vectors in
  `dashboard/tests/test_auth.py`. Session tiers pin `guild_member.rank ≤1`
  for writes (`dashboard.py:456-468`) and `gmlevel ≥2` for admin (`:780-791`).
- **Non-DB coupling (exactly three side channels, no SOAP/RA/sockets)**:
  (1) `pgrep mangosd` process name (`dashboard.py:179`; `bot.py:52`) — the
  Rust binary must be discoverable as `mangosd`; (2) `LevelUp.log` line
  shape (`Player.cpp:3152` writer vs `dashboard.py:269` / `bot.py:226`
  parsers, `<BOT>` skip; `%2u` width spacing matters, 11 §4 #3);
  (3) `/tmp/azeroth-maintenance` flag existence (`dashboard.py:29`;
  `bot.py:26`). The C++ writer site for `characters.online` was not pinned
  (07 §10) — freshness timing is the liveness contract, proven only from
  the consumer side.
- **Human/bot sentinel** `account < 100` vs `>= 100` hard-coded
  (`dashboard.py:194-197`; `bot.py:74`; `ARCHITECTURE.md:353`) — preserve,
  including the 97/98/99 edge cases; do not "fix" silently.
- **Verdict with caveat**: keep MySQL schema + binary name `mangosd` and
  Python changes are *plausibly* zero (07 §8–§9) — but "zero" is unproven
  until LevelUp spacing/`<BOT>` position and `characters.online` freshness
  are observed against a live log/DB (11 §4 #3). Budget: hit all 16 JSON
  routes + `!status` against the same DB snapshot and diff payloads.
- No `.service` files are in the repo; unit contents were closed from the
  live host by 08 (`systemctl cat`, §9) despite 07 listing them unresolved
  (11 §5).

## 9. Live production deployment facts (droplet, observed 2026-10-16)

Read-only inspection via `wow@jeffs-awesome-droplet` (account per
`ARCHITECTURE.md:44`; FQDN fails host-key, short name works, 08 §1).

- **Host**: Ubuntu 22.04.5, kernel 5.15, 4 vCPU (DO-Regular), 7.8 GiB RAM
  (3.3 GiB available, swap 1.3/2.0 GiB used — historical, si/so=0), 78 GiB
  disk (41 GiB free), up 32 days. **No rustc/cargo**; gcc 11.4, CMake 3.22.
- **Not dedicated**: EQEmu `spire-launcher` + 25 `zone` processes (~1.5 GiB),
  a second `mysqld` (LXD), Docker, Caddy share the box (08 §4).
- **Units**: `azeroth-{realmd,mangosd,dashboard,discord-bot}`, all
  `Type=simple`, `User=wow`, `Restart=always`, `ExecStart` under
  `/home/wow/wow-private-server` (`systemctl cat`; matches
  `ARCHITECTURE.md:46-51`). **Legacy `mangosd.service`/`realmd.service`
  point at deleted `/home/wow/vmangos` and crash-loop** (NRestarts
  90,690 / 272,071) — disable before any cutover claims :3724/:8085.
- **Footprint** (`ps`): `mangosd` RSS 1,417,508 KiB, **73 threads**, 2.1%
  lifetime CPU, 31 d elapsed; `realmd` 10,420 KiB, 10 threads; Python
  ~40 MiB each; `mysqld` 1,024,604 KiB. `PlayerLimit = 100` (safe key).
- **Ports** (`ss`): 3724, 8085, 8080 on `0.0.0.0`; MySQL `127.0.0.1:3306`
  only. `Server.log` shows internet scanners feeding malformed packets to
  8085 continuously — parser robustness/fuzzing is a production requirement.
- **Deployed vs surveyed**: checkout `aaf9b5e` (2026-05-26,
  `fix/database-h-forward-decl`) but running binaries built 2026-04-13/14,
  unstripped, `ldd` = `libmysqlclient.so.21` / OpenSSL 3 / glibc 2.35.
  Deploys are manual and infrequent; build tree (719 MiB) kept in place.
- **Assets** (`DataDir="."` in `run/bin/`): mmaps 2.0 GiB, vmaps 574 MiB,
  maps 279 MiB, `5875/` DBC 28 MiB. Live `*.conf` mode 644, `.env` 664 —
  credential files world/group-readable (§11; fix regardless of migration).
- **Backups**: cron `0 4 * * *` `scripts/backup-db.sh`; 60 `.sql.gz` =
  15 days × 4 DBs to 2026-10-15; `mangos` 32 MiB gz, `characters` 13 MiB gz.
  Restore has **never been tested** (11 §5). World was near-idle at
  inspection ("quiet world", 2.1% lifetime CPU) — **no load baseline
  exists** against which to claim perf parity.
- **Rust headroom**: on-box `cargo build --release` risks OOM at 3.3 GiB
  available; build off-box against a glibc 2.35 floor (22.04 container) or
  musl with a pure-Rust MySQL driver. Ship finished binaries; stamp git
  revision into `--version`; keep the old binary for rollback.

## 10. Local build verification result (this Mac, 2026-10-15)

**Blocked at step zero** (01 §7): `cmake -S vmangos -B /tmp/…/build
-DCMAKE_BUILD_TYPE=Release` → `cmake: command not found`, exit 127. Host:
Darwin arm64, Apple clang 21, make 3.81; Homebrew MySQL 26.7 + OpenSSL 3
present; no cmake/ninja. The checkout was left clean (the three generated
headers remain absent). The six predicted next blockers are *(inferred,
unverified — do not present as findings, 11 §4 #2)*: CMake 4 minimums;
x86_64/Carbon/`-rdynamic` assumptions; Find-module paths (Homebrew
`openssl@1.1` hard-coding); OpenSSL 3 / C++14 vs clang 21; cpptrace
backend. Production build (Linux, CMake 3.22, gcc 11.4 per 08 §2) is the
only proven build path. A Cargo workspace removes the entire class of
find-module/Carbon/`-march=native` breakage for local development.

## 11. Migration-relevant risks, ranked

Ranking synthesises 03 §11 (difficulty), 09 §5–§6 (density), 02 §10
(concurrency), 11 §3–§4 (coverage holes). Ranks are judgement on top of
measured inputs — the inputs are cited; the ordering is *(inferred)*.

### R1 — God-object cluster + untyped update fields (highest)

`Player` + `Unit` + `Spell`/`Aura`/`Proc` form a three-way circular cluster
over an untyped `u32[]` model (§4). 2332 raw-`new` sites in `src` (09 §5;
game-only figures contested, §4), ≥319 raw-pointer members (lower bound),
~490 hard casts (110 `reinterpret_cast`, 155 `const_cast` — 94% in game,
226 `dynamic_cast`). Every `Unit*` field is a latent borrow-checker fight.
Mitigation: arena/slotmap ownership, GUID handles, command-buffer deletes.

### R2 — 55 singletons / 9,539 uses

55 `sX` macros, 9,539 token uses; `sLog` 4,054 (maps to `tracing`),
`sObjectMgr` 1,458/110 files, `sWorld` 1,063/135 files, `sSpellMgr` 467
(09 §6). No safe mutable-singleton idiom exists in Rust; thread an explicit
context or accept a transitional `LazyLock<RwLock<…>>` crutch with a
removal plan. These use counts are the refactor's true size.

### R3 — Single-threaded assumptions that are actually multi-threaded

World tick is single-threaded, but MAP/SPELLS/MOVEMENT handlers read
cross-map state, `SendPacket` reaches any session, maps fork-join, the
AsyncPacket fence is the only thing making session iteration sound, and
`BattleGroundMgr` is effectively unlocked (§3.4). The 73-thread live
process proves production is not "single-threaded". Audit list:
groups/guilds/loot shared-reads from map threads (02 §10). Per-lock
inventory and ordering are absent — build them before designing `Send`
bounds (11 §3).

### R4 — No test safety net

C++ tests ~340 lines (0.07%); `vmangos_tests` links the whole game lib
(`vmangos/tests/CMakeLists.txt:1-6`); existing tests cover only pure
islands. "Tested" = in-game smoke tests
(`.claude/rules/testing.md`). Golden fixtures (packets, saves, spell calcs,
`[BotAI]` traces) must be built *before* translating Player/Spell/Unit
(R5–R7 in §13), or parity is unprovable.

### R5 — Content scripts volume + name contracts

107,413 LOC across 223 files, names referenced from world-DB rows (§4).
Hand-porting file-by-file is not schedulable. Options (§13): FFI-retain
behind the 31-slot `Script` registry, mechanical transpilation, or defer.
Wrong choice here dominates total effort more than the core does.

### R6 — Hostile network edge + parser parity

8085 faces the internet with daily malformed traffic (§9); 272 real
handlers parse `ByteBuffer` positionally with no payload catalogue (11 §3).
Each handler body *is* its schema; `read<T>` bit-cast acceptance must
become fallible newtype conversion. Fuzz the Rust codec against the C++
wedge-vs-drop behaviour (§3.5).

### R7 — Data correctness on sacred data

MyISAM majority (no rollback), printf-SQL injection census forced at
~563–671 sites, 22 of 23 login queries untranscribed, world DB absent,
live charset/migration state unverified (§5). A single save-ordering or
blob bug corrupts real characters. Differential testing (C++ save → Rust
load → Rust save → row byte-compare) is mandatory before any production
character is touched; rehearse on a restored nightly dump (13 MiB gz, §9).

### R8 — Detour payload lock-in (the only hard FFI)

`.mmtile` payloads are meaningful only to a compatible Detour build (§7).
Reusing 2.0 GiB of production mmaps unchanged implies FFI to the vendored
Recast/Detour (or a large reimplementation). Decide first; everything else
in §7 is pure Rust.

### R9 — Operational debt a migration must not inherit

Binary≠checkout with no revision stamping; two crash-looping legacy units
(~363k restarts); world-readable credential files; untested restores; no
load baseline (§9). Each is cheap to fix at cutover, expensive after.

### Unsafe/FFI candidates (bounded list)

| Site | Treatment | Why |
|---|---|---|
| Detour query + `.mmtile` payload | **FFI wrap** vendored C++ | Opaque serialised graph (§7); Recast needed only offline |
| DBC/maps/vmaps parsers | Safe Rust, unaligned LE reads | `reinterpret_cast` pockets → `from_le_bytes` (§7) |
| BIH traversal / spline math | Safe Rust port | Pure f32; golden-test vs C++ |
| g3dlite (62k in dep) | **Replace** (glam-class crate) | Math subset only |
| gSOAP (SOAP admin) | **Drop or tiny HTTP** | Surface unenumerated; default off |
| cpptrace | **Replace** (`backtrace`) | — |
| libmpq (4,899) | FFI or keep C++ extractors | Offline tools only; stay C++ longest |
| `dtNavMeshParams` (`.mmap`) | Field-wise parse, no transmute | Struct-dump fragility |
| SRP6/BigNumber | Vetted crate + C++ test vectors | WoW LE interleave pins variant (§3.6) |
| Header crypto (`AuthCrypt`) | Safe, per-connection state | Order-sensitive; lives in codec |

Vendored `dep/` (250,245 lines): gsoap/windows/doctest/cpptrace (~105k)
vanish; vendored-in-src json/nonstd (19,232) → serde/std. MySQL/OpenSSL:
prefer pure-Rust MySQL + rustls to avoid recreating find-module pain
(01 §8); note musl-static then has no `libmysqlclient.so.21` dependency.

## 12. Proposed Rust crate decomposition (mapped to current components)

Seams follow the existing CMake targets (`framework → shared → game →
scripts`, 01 §8) and the runtime actor topology (02 §10). Per the
from-scratch baseline (Preface, §5) no Rust crates exist to reconcile
with: this map is the greenfield layout for a new Cargo workspace
(scaffolded in Phase 0, §13.2). Grow it phase by phase — create a crate
only when its phase starts, not all twenty-one up front.

| Proposed crate | Maps to today | Rationale / boundary rule |
|---|---|---|
| `mangos-protocol` | `Opcodes`, `ByteBuffer`, `WorldPacket`, `AuthCrypt` | Pure wire types; `const [Handler; 828]` with status/class metadata ported first, mechanically. |
| `mangos-guid` (+ shared types) | `ObjectGuid`, `SharedDefines` | `#[repr(transparent)] struct ObjectGuid(u64)`; frozen bit layout. |
| `mangos-db` (per-DB actors) | `src/shared/Database/*` | Serial-lane (GUID-low) ordering, prepared binds, migration gate (`build.rs` glob). One actor per DB. |
| `mangos-dbc` | `DBCFileLoader`, `DBCStores`, fmt/structs | Owned `Vec<Record>`; 46 fmts; locale merge. Oracle: `scripts/dbc.py`. |
| `mangos-assets` | `GridMap`, vmap loaders/BIH, `temp_gameobject_models` | LE-explicit parsers; f32-exact heights/LoS. Shared with any future tool. |
| `mangos-detour` (FFI) | vendored Detour + `PathFinder` policy | Thin unsafe wrapper; policy (filters, fallbacks) in safe code. |
| `mangos-config` + `mangos-log` | `Config.*`, `sLog` | Byte-compatible parser quirks; `Arc<Config>` snapshot swap; `tracing` for 4,054 `sLog` uses. Keep log *formats* (LevelUp). |
| `mangos-build-info` | `revision.h` generation (`CMakeLists.txt:143-174`) | build.rs runs the same git commands (`rev-parse --short=20`, `show -s --format=%ci`, same fallbacks); binaries print `_FULLVERSION`-shaped stamps. Created in Phase 0. |
| `mangos-auth` (realmd bin) | `src/realmd` + `SRP6` | **First binary**: 2,882 LOC, shared+framework only. Must keep sync sessionkey write. |
| `mangos-world-data` | `ObjectMgr` loaders, `SpellMgr`, `SQLStorage` | Read-only `Arc` static stores; load order + patch-selection preserved. |
| `mangos-map` | `Map`, Grid/Cell, visibility, update replication | Per-map actor owning arenas; checkerboard → rayon; variable-diff semantics kept. |
| `mangos-entities` (modules, not files) | `Objects/` — Player split by domain | Decompose Player (inventory/quests/talents/social); never 1 file → 1 file. |
| `mangos-spells` | `Spells/` + procs | Ports last, with entities; handler tables → `match` on generated enums. |
| `mangos-movement` | `MotionMaster`, splines, waypoints | Enum generator stack; spline packet builder early with golden tests. |
| `mangos-threat` | `Threat/` (1,042 LOC) | Self-contained; ports first inside sim. |
| `mangos-scripts-engine` | EventAI, DB scripts, Conditions (~4k LOC engines) | Interpreters over DB data — thousands of behaviours for free. |
| `mangos-scripts-content` (or FFI shim) | `src/scripts/` 107k LOC | Registry of name→factory; FFI-retain C++ or transpile (§13). |
| `mangos-services` | Group/Guild/AH/Mail/Social/Chat/LFG/BG | Async services over same schemas; GUID handles, no cross-map borrows. |
| `mangos-bots` | `PlayerBots/` + AH bot | One crate, modules 1:1 with the 8 singletons under `BotSystem`; `BotBehavior` trait; requires Host API (below). |
| `mangosd` (bin, same process name) | `src/mangosd`, `World` | Tokio runtime + world actor (50 ms interval) + per-map executors + gated async worker. |
| Python services | unchanged | Keep. Contracts in §8 are the spec. |
| Extractors | stay C++ | Offline; port only for a Rust-only toolchain goal. |

Bot Host API (the price of a separable `mangos-bots`, 05 §8): (1) headless
session accepting the synthesised opcodes + packet tap; (2) Player/Map
read+command surface (cast, move primitives, threat, auras, inventory/trade,
teleport); (3) event taps — chat, whispers, trade completion, group
membership, damage/death/cast, level-up, logout (the 52 hits); (4) scheduler
slots (500 ms per bot; 60 s social; 2-min director; 5-min level-sync;
30-min AH buyer). Extractable islands today without the Host API:
waypoint data, ring buffer, formation math, talent tables.

## 13. Suggested migration sequencing

### 13.1 Strategy choice: strangler at process + DB seams, big-bang inside the cluster

There is no in-process ABI seam (everything static, §2.1), so the only
honest strangler seams are: **realmd first** (process + login DB), then
**pure crates with golden tests**, then **mangosd as one coordinated
cluster replacement** behind frozen wire/DB/asset/log contracts. A
file-at-a-time FFI strangler *inside* mangosd is possible only around the
cluster's edges (packets in, packets out, DB, files, Detour — 03 §11) or by
keeping C++ content scripts behind the script registry while the core moves.
Both options are sequenced below; pick one before Phase 3.

### 13.2 Phases (each independently shippable/rollback-able)

**Phase 0 — Evidence, hygiene & workspace scaffolding (no Rust in production).**
Scaffold the new Cargo workspace from scratch (workspace manifest,
rustfmt/clippy gates, CI build+test, dev-MySQL fixture scripts and the
env-gated `--ignored` live-test convention from §5 — all recreated, none
inherited; clear the stale `vmangos-rs/target/` residue rather than
building over it);
map production binary→commit and stamp revisions going forward; disable
legacy crash-loop units; fix credential-file modes; test a backup restore;
copy production DataDir fixtures + a nightly DB dump to the dev bench;
collect golden corpora (client packet captures incl. scanner junk, SRP
vectors from §3.6, DBC inputs via `scripts/dbc.py`, LevelUp samples,
dashboard payloads for all 16 JSON routes + `!status`).
*Verify*: restore boots the C++ server; corpora replay deterministically.

**Phase 1 — realmd in Rust (first binary).**
`mangos-auth` + `mangos-protocol` (auth half) + `mangos-db` (login) +
`mangos-config/log`, against the same login DB; C++ mangosd unchanged.
Shadow-test: dual-run on a cloned DB, compare realm lists/sessionkeys.
*Verify*: the Phase 0 live-test harness, newly built (`TEST_MYSQL_URL`-
gated, `--ignored` DB-backed SRP logon + realm-list tests passing from
zero — the former workspace's 11/11 result is history, not evidence);
dashboard
`test_auth.py` vectors against Rust-written rows; real client logon →
realm list → C++ mangosd entry, end to end.

**Phase 2 — Pure crates against production inputs (no production swap).**
`mangos-guid`, `mangos-dbc`, `mangos-assets` (maps/vmaps), spline packet
builder, `mangos-threat`, formation math, ring buffer, talent codegen —
each with byte/behaviour parity tests vs C++ on production fixtures
(heights, LoS, DBC records, `SMSG_MONSTER_MOVE` bytes). Detour FFI spike:
same `.mmtile` → same paths.
*Verify*: fixture suites are the deliverable; failures block Phase 3.

**Phase 3 — mangosd skeleton that boots and stays empty.**
Tokio IO + world actor + session/queue topology (§3), config parity,
static loading (`mangos-world-data` incl. migration gate), opcode table
(826/276/272 with handlers stubbed by class), character *load* (all 23
queries transcribed first — close 11 §3), login to an empty world.
*Verify*: boots against cloned prod DB + assets; client logs in, sees
characters; fuzzed codec survives scanner corpus; freeze/heartbeat
decisions made explicitly (replace SIGSEGV anticrash with supervisor +
periodic saves, 02 §9).

**Phase 4 — Simulation cluster (the long pole).**
Maps/visibility/movement, entities (Player decomposed), spells/auras,
AI + script engines (EventAI/DB/Conditions first), content scripts per the
Phase-0 choice (FFI-registry vs transpile — region by region: world
scripts and one dungeon vertical slice before EK/Kalimdor bulk).
*Verify per slice*: in-game smoke tests are the project norm
(`.claude/rules/testing.md` — e.g. a Deadmines clear with the affected
role, evidence in the PR); differential spell fixtures (same inputs →
same damage/auras); spline/golden packet diffs.

**Phase 5 — Services, then bots.**
Social/economic services (conventional CRUD + indexes), GM command surface
(port the table/permissions contract; 23.6k LOC can trail), then
`mangos-bots` on the Host API: behaviour runner + 38 behaviours, formation,
chat/services (absolute-deadline scheduler), director/guild/presence,
AH bot, BattleBot engine decision (keep dual-engine or migrate to
behaviours — BG objectives have no behaviour equivalent today, 05 §5).
*Verify*: `.partybot debug` `[BotAI]` trace comparison vs C++ as oracle;
Deadmines 5-bot clear per role; guild roster dashboard round-trip
(incl. the untraced reload-liveness question, answered by now).

**Phase 6 — Character save cutover (last, gated).**
Rust saves behind differential testing on a cloned prod DB (C++ save →
Rust load → Rust save → row byte-compare incl. all blobs, §5), then
canary: non-production realm or test accounts on the live host (own host
or maintenance window — the droplet has no spare capacity, §9).
*Cutover*: off-box-built binary (glibc 2.35 floor or musl), same
`ExecStart` path + process name `mangosd`, old binary kept for rollback,
`/tmp/azeroth-maintenance` semantics preserved; keep both binaries able to
read the same assets/DB during transition.
*Verify*: full Python suites + live payload diffs; LevelUp replay; real
client session incl. level-up, trade with a service bot, dungeon clear.

### 13.3 What moves first / last (summary)

| First (bounded, testable) | Last (coupled/sacred) | Never/strangest |
|---|---|---|
| realmd; GUID; DBC; maps/vmaps; threat; spline builder; script interpreters; static loaders | Character save; Spell/Aura cluster; Player/Unit; BG internals; BattleBot engine; GM commands | Extractors (stay C++); Python services (stay); SIGSEGV anticrash (drop, deliberate); Postgre backend (decide: compile/dead/drop — see §14) |

### 13.4 Verification strategy (standing, per phase)

- **Unit/parity**: `cargo test` replaces doctest; port the 4 existing test
  files + Python `test_dbc.py` corpus; add golden tests before each god-file.
- **Contract**: dashboard/discord pytest suites (best-tested code in the
  repo, ~35% tests) run unchanged on every cutover; shape *and* live-data
  diffs (§8).
- **In-game smoke tests** remain the acceptance evidence for bot/spell/
  movement changes (CLAUDE.md definition of done): screenshots or
  `Server.log` excerpts in the PR description.
- **Gates re-homed, not dropped** (10 §5): conventional commits + semver
  release derivation, gitleaks hard-fail, rustfmt/clippy analogues,
  append-only migration tooling.
- **Perf**: no baseline exists (§9) — create one in Phase 0 (fixed bot
  population, tick-time + RSS capture) before claiming parity; the bar is
  ~1.35 GiB RSS, `PlayerLimit=100`, same 2.9 GiB asset set.

## 14. Unresolved questions (preserved verbatim; do not resolve by invention)

Every preserved gap from the workflow input is carried here verbatim.
Two were partially closed by later reports and are flagged inline
([closed by 11 §1 #4], [contents closed by 08]) but retained as instructed.

1. macOS configure/compile unverified — blocked at missing CMake; 6 predicted next blockers listed in report §7 (CMake4 minimums, x86_64/Carbon/-rdynamic, Find module paths, OpenSSL3/C++14 vs clang21, cpptrace backend) [Phase 0 2026-10-16: native macOS still unverified, but the off-droplet Linux-container build works and is the oracle route; see PHASE0-PROGRESS.md] [Phase 2 2026-10-18: small data point only — the vendored Detour C++ sources compile natively under Apple clang 21 as part of vmangos-rs/ci.sh (mangos-detour build.rs); mangosd itself still unverified on macOS]
2. Linux production build time/flags and systemd unit details not inspected (live system out of scope) [unit contents later closed by 08; build time/flags still open] [Phase 0: container build of both targets at 2bae5f9 measured ~60 s (arm64, gcc 11.4/cmake 3.22 defaults); droplet's own historical build flags still uninspected]
3. USE_STD_MALLOC/TBB usage in production: no in-tree setter found, presumed dead
4. SRP6.cpp interleave/hash internals not fully read (header+call sites+Proof only) - needed for Rust test vectors [full body read by 11 §1 #4; A-at-creation caveat remains] [caveat CLOSED in Phase 0: fresh SRP6 at all 4 call sites, A feeds zero bytes; vectors derived and oracle-checked] [Phase 1 2026-10-17: FULLY CLOSED — SRP6 implemented in mangos-protocol and proven against the 2bae5f9 oracle by packet/DB differential + a shadow logon into oracle mangosd; the inverted SRP6::Proof is cancelled by its sole negating call site (AuthSocket.cpp:639) so observed wire semantics are standard — see PHASE1-PROGRESS.md]
5. Windows IOCP/ServiceWin32 impls skimmed via headers only
6. Live production thread-count config values not read (secret hygiene); only *.conf.dist.in defaults cited
7. Spell.cpp/SpellAuras.cpp internal flows beyond headers/dispatch not deep-read [PARTIALLY CLOSED in Phase 4 Slice 5 (2026-10-19): the cast/aura cores needed for a functional subset (CheckCast arms, GCD/cooldowns, school damage/heal, APPLY_AURA holders + periodic ticks, spell hit/resist/crit math incl. the binary-spell rule) were deep-read line-by-line from production 2bae5f9, ported, and verified against an independent Python/SQL oracle; Slice 7 added summon effects and dead-caster triggered-cast semantics. Still unread/unported: spell mods, proc chains, most effect/aura handlers, channeling/pushback rules, threat-from-healing nuances — see PHASE4-PROGRESS.md Slice 5 deviations]
8. mangosd.conf cadence-gate defaults not read (credential-bearing, out of bounds)
9. Battlegrounds/Anticheat/Server interiors and src/shared DBC loading left to sibling scopes [Phase 3 2026-10-17: the Server interior (WorldSocket/WorldSession/OpcodeHandler admission chain, AddSession queue, Master boot sequence) was read line-by-line and ported; Battlegrounds/Anticheat interiors remain unread]
10. vmangos-rs/crates absent in this checkout — existing Rust MySQL crates (mangos-realmd-mysql per memory) not inspectable here [closed 2026-10-16 by owner direction: the former workspace no longer exists; migration starts from scratch — see Preface]
11. World DB content dump gitignored/absent — world table/row inventory and loader validation data unavailable [Phase 0: 2026-10-15 nightly world dump restored on the dev bench (174 tables) — inventory/validation now possible locally; still absent from the repo]
12. Live production DB not queried (read-only) — engine/charset drift and applied-migration state unverified [Phase 0 via restored snapshot: migrations complete (1005/18/6/6); characters engines 48 MyISAM/13 InnoDB; mixed collations observed (utf8mb3/utf8mb4/latin1) — charset parity remains a Phase 1/3 watch item] [Phase 1 2026-10-17: login-DB path exercised end to end on the restored snapshot by BOTH servers with byte-identical results (account/sessionkey/realm rows); no charset divergence observed on the realmd surface (ASCII usernames) — remains a watch item for world/character text in Phase 2/3] [Phase 3 2026-10-17: world/character text exercised on the snapshot through the binary protocol — character names, mail subjects, and account-data blobs byte-exact vs independent oracle (298 load rows); no divergence observed, still a watch item for non-ASCII player input at save time (Phase 6)]
13. Postgre backend parity, CharacterDatabaseCache/Cleaner, per-loader SELECT catalog sampled not exhaustive; Python dashboard/discord DB writers out of scope [Python writers later scoped by 07: guild_bot_roster only]
14. Largest .cpp bodies read selectively: per-behavior Score/Execute internals, BotDirector cycle math, AH pricing formulas, GuildBot gear scoring characterized from headers/docs/greps, not line-by-line
15. docs/BOT_ARCHITECTURE_PROPOSAL.md and gitignored research docs not read [proposal read by 10; gitignored docs still absent]
16. Live config values, DB contents, online bot counts not inspected (no-secrets constraint); defaults cited are code/.dist only
17. Map.cpp:1781 and BattleGround.cpp:787 IsBot branches located but surrounding logic not fully read
18. No real .dbc/.map/.vmtile/.mmtile bytes inspectable in this checkout (assets absent, gitignored); format claims rest on paired loader+writer source — production DataDir fixtures needed for byte verification [Phase 0: full production DataDir (2.9 GiB) copied to the dev bench; all 158 DBC headers size-validated; byte-level parser parity tests are Phase 2 work] [Phase 2 2026-10-18: CLOSED — byte/record-level parity proven against independent oracles on the production fixtures: all 158 DBCs vs scripts/dbc.py (headers, section hashes, produced-value digests for all 46 stores); GridMap heights/area/liquid bit-identical on all 2,429 tiles; vmap parse digests on all 3,490 .vmo + 1,250 .vmtile + 43 .vmtree plus BIH LoS/height probes f32-exact vs brute-force/exhaustive oracles; .mmtile payloads load through the Detour FFI with bit-identical path results (see #21) — see PHASE2-PROGRESS.md]
19. GroupModel GMOD per-field layout (WorldModel.cpp:271,330) located but not line-audited
20. World (mangos) DB schema file not in repo; table shapes evidenced from C++ queries only [Phase 0: schema now inspectable from the restored snapshot on the dev bench; still not in the repo]
21. Vendored Detour version/DT_NAVMESH_VERSION and internal tile payload layout not read (treated opaque, as MoveMap.cpp does) [Phase 2 2026-10-18: CLOSED — read: DT_NAVMESH_VERSION 7, DT_POLYREF64 with custom 12/21/31 salt/tile/poly split plus a small DetourNavMeshQuery.cpp tweak (recastnavigation.diff); tile payload stays opaque exactly as in MoveMap.cpp, loaded via dtAlloc+DT_TILE_FREE_DATA. The spike is GREEN: mangos-detour compiles those vendored sources behind a C shim (build.rs + cc, -ffp-contract=off) and production .mmtile path queries match an independent C++ harness bit-for-bit — no Phase 3 fallback needed; poly-refs depend on tile load order, so future parity should key on geometry]
22. systemd unit file contents (ExecStart/User/Restart) exist only on prod host, not in repo [closed by 08 via systemctl cat]
23. Whether GuildBotMgr re-reads guild_bot_roster after startup (dashboard-edit liveness) not traced [CLOSED in Phase 0 (evidence 23): startup-only SELECT — dashboard edits need a mangosd restart]
24. Exact sLog.Player LevelUp.log prefix inferred from Player.cpp:3152, no live log sample read [Phase 0: full production LevelUp.log (51,612 lines) read on the dev bench; dashboard parser verified against it]
25. C++ writer site for characters.online not pinned to a line (consumer side only) [CLOSED in Phase 0 (evidence 23): five writer sites pinned; 15-min autosave freshness contract]
26. dashboard/tests/test_integration.py and template JS field usage not fully inspected
27. MySQL per-DB/table sizes: ERROR 1045 passwordless, no ~/.my.cnf/.mylogin.cnf; creds only in .env/*.conf which rules forbade reading [Phase 0 snapshot-derived sizes: mangos ~170 MiB, characters ~66 MiB, realmd/logs ~0.2 MiB each]
28. /var/lib/mysql on-disk size: Permission denied for wow, sudo out of scope
29. Owners of other-user loopback listeners (3000-3010,7778,8081/2,9500), Caddy site config, docker inventory (docker.sock permission denied)
30. Cause of azeroth-realmd NRestarts=24 and exact commit mapping of Apr-14 running binary (BuildID only, no journal forensics) [mapping CLOSED in Phase 0: commit 2bae5f921f59892d4c41002a0869b154f02867d3; NRestarts cause still open]
31. Guild name conflict (README.md:11 'Vanguard of Azeroth' vs ARCHITECTURE.md:277 'Sweatlords') unverifiable from repo — name lives in DB, not code [Phase 0 via snapshot: 'Sweatlords' exists, 'Vanguard of Azeroth' does not — README is stale]
32. 3 gitignored docs/ research notes (ARCHITECTURE.md:149) absent from tree, contents uninspectable
33. macOS configure/compile unverified (no CMake; blockers predicted only) [duplicate of #1, short form]
34. per-lock inventory + lock ordering absent (41 shared_timed_mutex mentions unnamed) [Phase 0: named inventory + two sampled ordering pairs in evidence 21; exhaustive whole-program ordering still open]
35. per-opcode payload catalog absent (272 handlers parse positionally) [Phase 0: generated skeleton in evidence 22 (826 rows, counts reconciled); 47 '?'-rows + ~10 helpers need Phase 3 triage] [Phase 3 2026-10-17: registry generated and census-asserted in-crate (826/276/525/828); all 47 '?'-rows hand-triaged in evidence 22 section 7 — 36 are the single MovementInfo helper (exact field list recorded, parser itself is Phase 4), 11 are member-access reads now typed. Remaining: spell-target helper expansion belongs to Phase 5 spell work] [Phase 4 2026-10-19: the MovementInfo parser landed in Slice 0 (golden-vector verified vs C++ text); the spell-target reading subset landed with the Slice 5 cast engine (single-unit/self/dest modes; full SpellCastTargets parity remains future work)]
36. ~28 char-DB save tables untranscribed (auras/cooldowns/mail/pet/BG/etc) [Phase 0: full save/load transcription in evidence 20 (61-bind REPLACE, 10 in-txn sub-saves, dead holder slot found)]
37. world DB schema absent; live engine/charset/migration state unverified [duplicate of #11/#12]
38. production binary-commit mapping + live thread/config values unknown [duplicate of #6/#30; mapping closed 2026-10-16 in Phase 0 — both binaries are commit `2bae5f921f59892d4c41002a0869b154f02867d3` per their embedded revision strings; live config values remain unread]
39. no real asset bytes verified (production DataDir fixtures needed) [duplicate of #18] [closed with #18 in Phase 2]
40. GMOD layout + vendored Detour version unread [duplicate of #19/#21] [Detour half closed with #21 in Phase 2; GMOD layout (#19) still unread]
41. Anticheat/Warden, core LFG, BG, ZoneScript, Honor internals unread [duplicate of #9, extended]
42. SOAP op surface + RA command set unenumerated
43. PostgreSQL backend disposition undecided; CharacterDatabaseCache/Cleaner unread [duplicate of #13] [CLOSED by owner decision 2026-10-18 (Phase 3 kickoff): PostgreSQL is dropped from Rust scope; MySQL-only. CharacterDatabaseCache/Cleaner still unread and still unported]
44. guild_bot_roster re-read-after-startup (dashboard liveness) untraced [duplicate of #23] [closed with #23]
45. characters.online C++ writer site unpinned; no live LevelUp.log sample [duplicate of #24/#25] [closed with #24/#25]
46. existing Rust workspace (~190 crates claim) uninspectable in this checkout [duplicate of #10; closed with it]
47. realmd NRestarts=24 cause unknown; backup restore never tested [extends #30] [restore half CLOSED in Phase 0: nightly dumps restore cleanly and the production-commit oracle boots on them to 'World initialized.']
48. Caddy/TLS/firewall fronting :8080; MySQL per-table sizes unknown [duplicate of #27/#29]
49. no load/perf baseline (live world idle at inspection) [Phase 0: idle baseline recorded in PHASE0-PROGRESS.md; loaded baseline still open for Phase 3] [Phase 3 2026-10-17: still open — the Rust skeleton is an empty world by design, so no loaded baseline is meaningful yet; idle bench boot of the Rust binary (50 ms world tick) is healthy but is not a perf measurement] [Phase 4 2026-10-19: bench-host micro-baseline only (NOT a production load test): live_motion PERF harness, 400 ticks over a populated single-player map slice (23 active creatures): 59 us/tick at Slice 4 (movement only), 110 us/tick with the full Slice 4-7 stack (spells/AI/EventAI/content) — i.e. ~0.2% of the 50 ms tick budget on this workload. A real loaded baseline (many players, continent-wide activation) remains open]
50. Spell.cpp/SpellAuras.cpp internals beyond dispatch unread [duplicate of #7] [partially closed with #7 in Phase 4]
51. Map.cpp:1781/BattleGround.cpp:787 bot-branch contexts unread [duplicate of #17]
52. bot per-behavior Score/Execute, AH pricing, GuildBot gear math not line-read [duplicate of #14]
53. Windows IOCP/ServiceWin32 headers-only; USE_STD_MALLOC prod use presumed dead [duplicate of #3/#5]
54. guild name + 3 gitignored research docs unverifiable from repo [duplicate of #31/#32]

## 15. Handoff for the future migration goal

**Decisions the owner must make (not inferable from the codebase).**

1. **Content scripts strategy** (the effort-dominating choice, R5/§13.1):
   FFI-retain the 107k-line C++ scripts behind the registry, mechanically
   transpile them, or hand-port region by region. Required before Phase 3.
   [Owner decisions: 2026-10-18 FFI-retain chosen as the default direction;
   2026-10-19 (Phase 4 Slice 7) the owner chose to hand-port Deadmines
   natively instead of building the shim first — done and gated. The
   strategy for the EK/Kalimdor bulk (FFI shim vs continued hand-port)
   remains OPEN and should be decided with the Deadmines cost data in
   PHASE4-PROGRESS.md: this fork is heavily DB-driven, so the measured
   hand-port surface per dungeon may stay small]
2. **Detour disposition** (R8): wrap the vendored Detour via FFI to reuse
   the 2.0 GiB production `.mmtile` set, reimplement tile loading/query,
   or regenerate navmeshes with a Rust-side pipeline.
3. **PostgreSQL backend**: compile-tested, dead, or dropped — parity was
   not established (04, §14 #13/#43).
4. **Workspace home and edition**: the migration starts from scratch
   (Preface). Decide the new Cargo workspace's location (fresh
   `vmangos-rs/` in this repo, clearing the stale `target/`, vs a sibling
   checkout), Rust edition/toolchain pin, and MSRV policy before Phase 0
   scaffolding. §12 is the layout to grow into, crate by crate.
5. **BattleBot engine**: keep the dual-engine design or migrate BG bots
   onto the behaviour runner (BG objectives have no behaviour equivalent
   today, 05 §5).
   **Decided (Phase 5, Slice F): keep the dual engine.** `BattleBotAI`
   (3,026 lines) + `CombatBotBaseAI` (3,829) + waypoint data (1,089) is
   an objective-driven state machine — flag-carrier logic, BG state,
   recorded waypoint graphs, packet-driven queue/port flow, premade
   character spawn directly into a BG instance — that bypasses
   `BotBehaviorSet` in the fork itself. The Rust behaviour runner is
   party-centric (master-relative formation, party roles, scored
   fall-through) and has no objective or waypoint-graph concept;
   forcing BG objectives into scored behaviours would distort both
   engines and break fork-semantic parity, which is the migration
   rule. The Rust `mangosd` also has no battleground subsystem yet
   (no queues, BG maps, or objectives), so there is nothing for a
   BattleBot port to drive. Port plan when BG internals land in a
   later phase: a separate engine module in `mangos-bots` mirroring
   `BattleBotAI`/`CombatBotBaseAI` structure (per-class combat/OOC
   functions, waypoint tables as generated static arrays), sharing
   only Host API primitives with the party engine (session-less login
   train, spell kits, movement). Until then Phase 5 ships the
   `.battlebot` command-table/permissions contract (Slice C) with
   handlers unported, exactly like the rest of the GM handler backlog.
6. **MySQL driver / TLS**: pure-Rust (enables musl-static, no
   `libmysqlclient` dependency) vs FFI to the system client (§11 FFI table).

**Phase 0 checklist (evidence to collect before any Rust ships).**

- [ ] Scaffold the from-scratch Cargo workspace + CI + dev-MySQL/`TEST_MYSQL_URL`
      live-test harness (§5, §13.2 Phase 0); §14 #10/#46 are closed by the
      from-scratch direction, not by a survey.
- [ ] Map the running 2026-04-14 production binary to a commit; stamp git
      revision into binary `--version` from now on (§9, R9).
- [ ] Test a restore of a nightly DB backup end-to-end (§9).
- [ ] Copy production `DataDir` fixtures (DBC/maps/vmaps/mmaps samples) to
      the dev bench for byte-level parser tests (§7).
- [ ] Capture golden corpora: client+scanner packet traces, SRP vectors
      (§3.6 caveat checked), `LevelUp.log` samples, all 16 dashboard JSON
      payloads + `!status` against one DB snapshot (§8, §13.2 Phase 0).
- [ ] Record a perf baseline: fixed bot population, tick time, RSS (§9 —
      none exists today).
- [ ] Transcribe the remaining character save/load surface: all 23 load
      queries and the ~28 untranscribed save tables (§5, §14 #36).
- [ ] Build the per-lock inventory and lock ordering (§3.4, §14 #34) and
      the per-opcode payload catalogue by handler (§14 #35).
- [ ] Answer `guild_bot_roster` reload liveness (§8, §14 #23) and pin the
      `characters.online` C++ writer (§14 #25).
- [ ] Get a working C++ build off the droplet (Linux container per §10)
      so parity oracles can be produced on demand.

**Goal-setting note.** When the migration goal is set in Muse, point it
at this file and name the phase (§13.2) in scope. Phases are independently
shippable and rollback-able by design; a single goal spanning Phases 0–6
would discard that property. The production locks in `CLAUDE.md`
(append-only migrations, shape-stable dashboard JSON, additive bot
behaviours, locked save formats) apply to every phase unchanged.

*End of document. Sources: reports 01–11 in
[rust-migration-evidence/](rust-migration-evidence/); all
`file:line` citations therein were spot-verified by the critic (11 §1:
6/9 fully survived, 2 partial, 1 contradiction resolved). Where reports
disagreed on counts, this document quotes the critic's measured values and
states the counting predicate; where evidence was absent, the gap is named
rather than filled.*






