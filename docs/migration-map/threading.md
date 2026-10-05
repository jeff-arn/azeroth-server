# Threading and tick map — VMaNGOS `mangosd`

Bead: `as-pnw.4` (child of the `as-pnw` migration-mapping epic).
Method: static analysis of `HEAD` (`origin/main`, `3589f72`) only. No runtime
measurement was performed; timing and contention questions are recorded as
dynamic follow-ups in §8.

Headline: `mangosd` runs **one world-tick thread** at a 50 ms cadence that owns
all game state, with **bounded parallelism fanning out and re-joining inside
each tick** (per-map updates, per-cell updates, object/visibility fan-out), plus
a small set of **always-on background threads** (network IO, DB delay workers,
async packets, CharDB watchdog, system timers). Several parallel paths are
**compiled in but disabled by stock config** (MTCells, motion threads, packet
broadcast, freeze detector).

## 1. Process thread topology

Threads created during startup, in order (`vmangos/src/mangosd/Master.cpp`,
`Master::Run`):

| # | Thread(s) | Created at | Role | Config (dist default) |
|---|-----------|------------|------|----------------------|
| 1 | `mangosd-main` | `Master.cpp:270` (rename of initial thread) | Joins the world thread (`Master.cpp:306`), owns shutdown sequencing | — |
| 2 | `IO[0..N-1]` | `Master.cpp:177-180` | Boost.Asio-style `IoContext` runners: socket accept + all async socket IO | `Network.Threads = 1` (`mangosd.conf.dist.in:2820`) |
| 3 | `WorldRunnable` | `Master.cpp:199` | The single world tick (§2) | fixed 50 ms (`WORLD_SLEEP_CONST`, `WorldRunnable.cpp:39`) |
| 4 | `CLI` | `Master.cpp:219` | Console command loop (`CliRunnable`) | `Console.Enable` (default true) |
| 5 | `SystemTimer` | `AsyncSystemTimer.cpp:11` (first `sAsyncSystemTimer` use, `Master.cpp:269`) | Wall-clock one-shot async timers (`IO::Timer`) | — |
| 6 | `FreezeDetector` | `Master.cpp:284-287` | Watches `World::m_worldLoopCounter`, `std::terminate()` on stuck world loop (NDEBUG only) | `MaxCoreStuckTime = 0` = **disabled** (`mangosd.conf.dist.in:286`) |
| 7 | SOAP thread | `Master.cpp:279` | External SOAP control interface | `SOAP.Enabled = false` = **disabled** |
| — | RA acceptor | `Master.cpp:224`, on the shared `ioCtx` | Remote-access listener; no dedicated thread | `Ra.Enable = false` = **disabled** |

Threads created during world init (`World::SetInitialWorldSettings` tail,
`vmangos/src/game/World.cpp:1866-1874`):

| Thread | Role |
|--------|------|
| `LfgUpdate` (`World.cpp:1866`) | `m_lfgQueue.Update()` loop (only when `GetWowPatch() >= WOW_PATCH_103` or LFG not accurate) |
| `WardenSessions` (`Anticheat.cpp:98`, started `World.cpp:1869`) | `UpdateWardenSessions()`: 1 s loop over active Warden sessions (§6; inert when Warden disabled) |
| `CharDB` (`World.cpp:1873`) | `CharactersDatabaseWorkerThread()`: 1 s loop; logs if `CharacterDatabase.HasAsyncQuery()` for >30 min (`World.cpp:1232-1242`) |
| `AsyncPacket` (`World.cpp:1874`) | `ProcessAsyncPackets()`: 20 ms loop; processes `PACKET_PROCESS_ASYNC` packets only while `m_canProcessAsyncPackets` is set (`World.cpp:1936-1962`) |

Database threads (`vmangos/src/shared/Database/Database.cpp:141-191,218-248`):

- One `SqlDelayThread` ("DB:\<dbname>") per async worker per database.
  `Master::_StartDB` reads `<Name>Database.Connections` (sync pool, default 1)
  and `<Name>Database.WorkerThreads` (async workers, default 1) for each of
  World / Character / Login / Logs (`Master.cpp:402-457`,
  `mangosd.conf.dist.in:78-88`). Stock total: **4 DB delay threads**.
- Each delay thread runs `SqlDelayThread::run()`: 10 ms poll loop
  (`SqlDelayThread.cpp:40-71`) draining the shared delay queue plus its serial
  queue, with a reachability `Ping()` every `Database.AliveCheckInternal`
  (dist default 600, `mangosd.conf.dist.in:89`).
- Serial routing: operations with a nonzero serial id are pinned to
  `worker = serialId % numWorkers` (`Database.cpp:545-558`) — ordering per
  serial id, no load balancing.

Thread pools created lazily or per-map (all `ThreadPool`,
`vmangos/src/shared/ThreadPool.h`):

| Pool | Name | Size (dist default) | Created at |
|------|------|---------------------|------------|
| World async tasks | `WorldUpdate` | `AsyncTasks.Threads = 1` | First `World::Update` (`World.cpp:2031-2035`), `MySQL` worker type |
| Instance updates | `MapManager` | `MapUpdate.Instanced.UpdateThreads = 2` | `MapManager` ctor (`MapManager.cpp:42-46`), `MySQL` workers |
| Instance creation | `NewMapForPlayer` | 1 | `MapManager` ctor |
| Continent updates | `MapContinent` | resized to continent count | Each `MapManager::Update` (`MapManager.cpp:344-348`) |
| Per-continent-map | `MapObj` | `MapUpdate.ObjectsUpdate.MaxThreads - 1` (4-1) | Continent `Map` ctor only (`Map.cpp:147-161`) |
| Per-continent-map | `MapVis` | `MapUpdate.VisibilityUpdate.MaxThreads - 1` (4-1) | Continent `Map` ctor only |
| Per-continent-map | `MapCell` | `MapUpdate.Continents.MTCells.Threads - 1` (**0-1 = none**) | Continent `Map` ctor only |
| Per-continent-map | `MapMotion` | `Continents.MotionUpdate.Threads` (**0 = none**) | Continent `Map` ctor only |
| Packet broadcast | `MovementBroadcaster` | `Network.PacketBroadcast.Threads` (**0 = disabled**) | World init (`World.cpp:1871`), 50 ms frequency |

Notes:

- Instance/dungeon maps get **no** per-map pools; only continents do
  (`Map.cpp:147` gate). With stock config the `MapCell`/`MapMotion` pools are
  size 0 and the broadcast pool is disabled — those code paths exist but never
  run concurrently on a stock server.
- `WorldSocketMgr::GetLeastUsedIoContext()` returns the single shared `ioCtx`
  (`WorldSocketMgr.cpp:116-122`); there is no per-thread socket affinity.

## 2. World tick (`WorldRunnable` → `World::Update`)

`WorldRunnable::operator()` (`WorldRunnable.cpp:47-117`):

1. `WorldDatabase.ThreadStart()` (no-op, `Database.cpp:250`) + `InitResultQueue()`.
2. Loop until `World::IsStopped()`: `m_worldLoopCounter++`, `diff = now - prev`,
   slow-tick perf log, anticrash rearm bookkeeping, `sWorld.Update(diff)`,
   then sleep the remainder of the 50 ms slot (`WORLD_SLEEP_CONST`). Overruns
   are **not** caught up — the next `diff` simply grows.

`World::Update(diff)` ordering (`World.cpp:1965-2134`):

1. Snapshot clocks (`m_currentMSTime/MTime/MDiff`); advance the six
   `WUPDATE_*` `IntervalTimer`s (`World.h:83-92`: auctions, uptime, corpses,
   events, save-var, groups).
2. `_UpdateGameTime`, `sMassMailMgr.Update`, auction/bot managers
   (gated on `WUPDATE_AUCTIONS`).
3. Take `m_asyncPacketsMutex`, set `m_canProcessAsyncPackets = false`
   (pauses the `AsyncPacket` thread), run the world `Messager` queue, then
   `UpdateSessions(diff)` (§4). Re-enable async packets.
4. Uptime row write — **synchronous** `LoginDatabase.PExecute`
   (gated on `WUPDATE_UPTIME`).
5. Lazily create the `WorldUpdate` pool; swap `_asyncTasks` to `_asyncTasksBusy`
   and dispatch (`processWorkload` → `std::future`).
6. `sMapMgr.Update(diff)` (§3), then `sBattleGroundMgr`, `sGuardMgr`,
   `sZoneScriptMgr` updates.
7. Offline-leader group updates (gated on `WUPDATE_GROUPS`).
8. `job.wait()` — the world thread **blocks until the async tasks finish**.
9. `SaveVariables` (gated), `UpdateResultQueue()` — drains Character, World,
   and Login result queues on the world thread, bounded by
   `AsyncQueriesTickTimeout` (dist default 0; bound semantics live in
   `SqlResultQueue::Update`, not verified here). Note: the Logs queue is
   **not** drained here.
10. Corpses, game events, `RemoveAllObjectsInRemoveList`, instance-reset times,
    honor maintenance, `PlayerBotMgr`, `AutoBroadCastMgr`, `AccountMgr`
    (ban-list reload).
11. Take `m_asyncPacketsMutex` again, `ProcessCliCommands()`.
12. `TerrainMgr` cleanup if `CleanupTerrain` (dist default 1).

Ordering consequences: session updates see this tick's timers but last tick's
maps; map updates see this tick's sessions; async DB callbacks and CLI run
after maps. Anything queued via `World::AddAsyncTask` runs concurrently with
step 6 and is joined at step 8.

## 3. Map updates (`MapManager::Update` → `Map::DoUpdate` → `Map::Update`)

`MapManager::Update` (`MapManager.cpp:289-408`) is itself gated on `i_timer`
(`MapUpdateInterval`; see the clamp note below):

1. `ExecuteDelayedPlayerTeleports()` (near teleports scheduled on the main
   thread, e.g. area triggers).
2. Per map: skip via `ShouldUpdateMap(now, Empty.UpdateTime)`; else
   `UpdateSync(mapsDiff)` (transports) + `MarkNotUpdated()`. Instance maps
   become work items for the shared `MapManager` pool (run inline if the pool
   is not `READY`); continent maps become work items for the `MapContinent`
   pool, each guarded by `IsUpdateFinished()` / continent-finished flags.
3. Instance creation is offloaded to the `NewMapForPlayer` thread
   (`CreateNewInstancesForPlayers` spins on `asyncMapUpdating` with 20 ms
   sleeps, `MapManager.cpp:251-287`).
4. Barrier: the world thread pumps the instance pool until all continents
   report finished or `INTERVAL_MAPUPDATE` elapses, then joins continents,
   `SwitchPlayersInstances()`, and runs far teleports.
5. Crash sweep (`IsCrashed` → `CrashUnload` + erase) and unload sweep
   (`CanUnload` → `UnloadAll` + erase). `i_timer` reset.

`Map::DoUpdate(maxDiff)` (`Map.cpp:921-931`) clamps the per-map `diff` to the
accumulated manager `diff` and calls `Map::Update(diff)`; it also refreshes
`m_lastPlayerLeftTime` whenever the map `HavePlayers()`.

`Map::Update(t_diff)` phases (`Map.cpp:933-1060`):

1. `m_dynamicTree.update`, movement/spell packet processing if
   `UpdatePacketsDiff` (100 ms) elapsed (`UpdateSessionsMovementAndSpellsIfNeeded`).
2. Per-player `WorldSession::Update` via `MapSessionFilter`.
3. `UpdatePlayers()` if `UpdatePlayersDiff` (100 ms) elapsed.
4. `UpdateCells()` if `UpdateCellsDiff` (100 ms) elapsed: continents with a
   `READY` `MapCell` pool use `UpdateActiveCellsAsynch` (mark + two
   strided parallel passes over `m_cellThreads`, `Map.cpp:771-792`), otherwise
   synchronous; queued unit motion goes to the `MapMotion` pool when `READY`
   (`Map.cpp:830-846`).
5. `SendObjectUpdates()` — parallel over the `MapObj` pool with a
   `MapUpdate.ObjectsUpdate.Timeout` (100 ms) abort; leftovers stay queued for
   the next tick (`Map.cpp:2777-2895`).
6. `UpdateVisibilityForRelocations()` — parallel over the `MapVis` pool with a
   `MapUpdate.VisibilityUpdate.Timeout` (100 ms) abort; same leftover
   semantics (`Map.cpp:2899-2966`). Non-continents run single-threaded.
7. Corpses/bones removal; continents then busy-wait for the global continent
   barrier while running extra player updates.
8. Grid state machine (`UpdateGridState`), scripted events, scripts, instance
   data, weather. Slow-map perf log + adaptive visibility/grid-activation
   tuning (all tick thresholds default 0 = disabled).

Effective cadence note: `MapManager::SetMapUpdateInterval` clamps any value
above `MIN_MAP_UPDATE_DELAY` (50, `GridDefines.h:46`) **down** to 50
(`MapManager.h:115-123`), and `SetInitialWorldSettings` enforces a minimum of
50 on the configured value (`World.cpp:573`). With the dist default
`MapUpdateInterval = 100`, the effective interval is therefore **50 ms** —
the config knob has no effect above 50. Flagged for dynamic confirmation.

## 4. Session / packet paths

Three packet-processing paths exist:

- **World path**: `World::UpdateSessions` (`World.cpp:2751-2827`) — login-queue
  admission (`LoginPerTick`), `addSessQueue` drain, per-session
  `WorldSession::Update`, disconnected-session updates. Runs with async
  packets paused (see §2 step 3).
- **Map path**: inside `Map::Update` (see §3) — per-player session update plus
  movement/spell/cooldown processing at the ~100 ms `UpdatePacketsDiff`
  cadence (`Map::ProcessSessionPackets`, `Map.cpp:850-873`).
- **Async path**: the `AsyncPacket` background thread handles
  `PACKET_PROCESS_ASYNC` packets whenever the world thread is *not* in steps
  3 or 11 (`m_canProcessAsyncPackets` + `m_asyncPacketsMutex`,
  `World.cpp:1936-1962,2001-2013,2122-2128`).

Exact opcode → process-type mapping lives in `PacketProcessing.h` and
`WorldSession::Update/ProcessPackets` and was not exhaustively traced here
(see follow-ups).

## 5. Timers

Two timer systems:

- **`IntervalTimer`** (tick-driven, `World.h:951`, `MapManager::i_timer`,
  per-map gating timestamps): advanced with the tick `diff` on the world
  thread — deterministic relative to game time, stalls when the tick stalls.
- **`AsyncSystemTimer`** (wall-clock singleton + `SystemTimer` thread):
  one-shot callbacks independent of the tick (`IO/Timer/AsyncSystemTimer.h`).

Clock: `WorldTimer::getMSTime()`; per-tick snapshots in
`World::m_currentMSTime/MTime/MDiff` and per-map `m_currentTime`.
`WORLD_SLEEP_CONST` (50 ms) sets the world cadence; `MIN_MAP_UPDATE_DELAY`
(50 ms) floors/clamps the map cadence — the two loops run 1:1 in practice.

## 6. Droplet ground truth vs template defaults

Per the bead, droplet `mangosd.conf` ground truth: **vmap LOS on, vmap height
on, vmap indoor-check on, mmap pathfinding on, Warden off**. The repo template
(`vmangos/src/mangosd/mangosd.conf.dist.in`) ships exactly these values, so
the droplet matches stock defaults on all five knobs:

| Knob | Droplet | Template default |
|------|---------|------------------|
| `vmap.enableLOS` | 1 | 1 (`dist.in:278`) |
| `vmap.enableHeight` | 1 | 1 (`dist.in:279`) |
| `vmap.enableIndoorCheck` | 1 | 1 (`dist.in:280`) |
| `mmap.enabled` | 1 | 1 (`dist.in:281`) |
| `Warden.WinEnabled` / `Warden.OSXEnabled` | 0 / 0 (off) | 0 / 0 (`dist.in:1550-1551`) |

Why it matters for threading: vmap/mmap **on** means height/LOS/indoor queries
and pathfinding run inside the tick (map-update and motion paths above) —
they are CPU load on the world/map threads, not offloaded. Warden **off**
means the `WardenSessions` thread still spawns (`World.cpp:1869`) but
`CreateWardenForInternal` returns null for both client builds when both
`...Enabled` flags are 0, so its 1 s loop iterates an empty session list
(`Anticheat.cpp:70-85,107-124`).

Other stock tuning the map relies on (all `mangosd.conf.dist.in`):

| Knob | Default | Effect |
|------|---------|--------|
| `Network.Threads` | 1 | Single IO thread for all sockets |
| `MapUpdateInterval` | 100 (effective 50, see §3) | Map-manager gate |
| `MapUpdate.Instanced.UpdateThreads` | 2 | Shared instance-update pool |
| `MapUpdate.ObjectsUpdate.MaxThreads` / `.Timeout` | 4 / 100 ms | Per-continent object fan-out |
| `MapUpdate.VisibilityUpdate.MaxThreads` / `.Timeout` | 4 / 100 ms | Per-continent visibility fan-out |
| `MapUpdate.UpdatePacketsDiff` / `.UpdatePlayersDiff` / `.UpdateCellsDiff` | 100 ms each | Sub-tick gating inside `Map::Update` |
| `MapUpdate.Continents.MTCells.Threads` / `.SafeDistance` | 0 / 1066 | Cell parallelism **disabled** |
| `Continents.MotionUpdate.Threads` | 0 | Async motion **disabled** |
| `MapUpdate.Empty.UpdateTime` | 0 (= disabled) | All maps update every tick, even empty |
| `AsyncTasks.Threads` | 1 | `WorldUpdate` pool size |
| `AsyncQueriesTickTimeout` | 0 | Result-queue drain bound (§2 step 9) |
| `Network.PacketBroadcast.Threads` / `.Frequency` | 0 / 50 ms | Broadcast pool **disabled** |
| `MaxCoreStuckTime` | 0 | Freeze watchdog **disabled** |
| `GridCleanUpDelay` / `CleanupTerrain` | 300000 ms / 1 | Grid expiry + terrain cleanup on |
| `Terrain.Preload.Continents` / `.Instances` | 0 / 0 | No preload; note the dist warning that `MapUpdate.Threads > 1` without preload risks tile-load vs pathfinding races (`dist.in:187`) |
| Each `<Db>.Connections` / `<Db>.WorkerThreads` | 1 / 1 | 1 sync conn + 1 delay thread per DB |
| `Database.AliveCheckInternal` | 600 | DB ping cadence for delay threads |

## 7. Rust-port reading

- The world tick is already an **ownership boundary**: everything between the
  step-5 dispatch and the step-8 join is the only window where game state is
  touched off-thread, and it re-joins before async DB callbacks run. A port
  can treat one tick as `&mut World` with scoped parallel map updates.
- The DB layer is already **message-passing** (delay queues in, result queues
  drained on the tick) — the natural async seam; keep callback-on-tick
  semantics.
- The parallel map/cell/object sections share grids and object sets with only
  coarse locking (`MapManagerLock`, per-set mutexes) plus timeout-abort
  leftovers — the highest-risk area to re-own, see follow-ups.

## 8. Dynamic follow-ups (require runtime measurement)

1. Tick distributions under load: world 50 ms vs map effective-50 ms drift;
   overrun growth (no catch-up cap besides the `DoUpdate` clamp); confirm the
   `SetMapUpdateInterval` always-50 clamp is intended.
2. `MapManager` pool saturation: how often instance updaters fall back to
   inline (`status() != READY`) and how long the continent barrier waits
   (`INTERVAL_MAPUPDATE`).
3. Contention: `MapManagerLock` hold times during `Update`; the two
   `m_asyncPacketsMutex` exclusion windows per tick; `m_objectsToClientUpdate`
   / `m_unitsRelocated` behavior when the 100 ms fan-out timeouts trip
   (leftovers, duplicate builds, missed updates).
4. DB backpressure: delay-queue depths per database; `UpdateResultQueue`
   worst-case tick stall with `AsyncQueriesTickTimeout = 0`; Logs-queue
   drain path (not in `World::Update`).
5. `AsyncPacket` thread throughput vs the paused windows; which opcodes ride
   `PACKET_PROCESS_ASYNC` (finish the `PacketProcessing.h` trace).
6. Confirm dead-by-default paths stay dead on the droplet (MTCells, motion,
   broadcast, freeze detector, SOAP, RA) via `Server.log` thread-startup
   lines and perf-log counters.
7. Memory ordering: `m_worldLoopCounter` / `m_masterLoopCounter` are plain
   `volatile` cross-thread flags — TSan/dynamic review for the freeze
   detector and any future watchdog.
8. vmap/mmap CPU share inside the tick (height/LOS/indoor + pathfinding with
   no terrain preload) — the main tick-cost driver to profile first.

