# Per-lock inventory: vmangos/src/game + vmangos/src/shared (branch rust-migration)

Scope: named synchronization primitives declared as class/struct members or
globals in `vmangos/src/game` and `vmangos/src/shared`: `std::mutex`,
`shared_timed_mutex`, `recursive_mutex`, `LockedQueue` internals, `std::atomic`
members guarding cross-thread state. Read-only inspection; every declaration
below was observed in the file body at the cited line. Lock-site samples note
what state each lock guards. Supplements main-doc section 3.4 / §14 #34.

Method: repo-wide `mutex` sweep over both trees, `std::atomic` sweep, then
targeted reads of declaration headers and 2–3 lock sites per primitive.

## 1. Singleton class-level locks (MaNGOS::ClassLevelLockable)

One process-wide static mutex per singleton; guards singleton instance
creation/access (and, by convention, any unsynchronised static state reached
through the singleton):

- `MapManager` — `std::recursive_mutex` (`Maps/MapManager.h:79`, Guard alias
  `:85`; instantiated `Maps/MapManager.cpp:38-40`). Recursive: map create /
  update paths re-enter (e.g. `CreateNewInstancesForPlayers` during update).
- `TerrainManager` — `std::mutex` (`Maps/GridMap.h:208`, Guard `:244`;
  instantiated `Maps/GridMap.cpp:1193-1195`).
- `MapPersistentStateManager` — `std::mutex`
  (`Maps/MapPersistentStateMgr.h:340`; `Maps/MapPersistentStateMgr.cpp:39-41`).
- `ObjectAccessor` — `std::mutex` (`ObjectAccessor.h:84`;
  `ObjectAccessor.cpp:36-38`). Guards the singleton only; per-object maps use
  `HashMapHolder` locks (§2) and `i_playerGuard`/`i_corpseGuard` (§3).
- `WorldSocketMgr` — `std::mutex` (`Server/WorldSocketMgr.h:43`).
- `Config` — `std::shared_timed_mutex` (`shared/Config/Config.h:30`;
  instantiated `shared/Config/Config.cpp:21`) PLUS member `m_configLock`
  (`Config.h:54`) guarding `m_configMap`. Write site `Config.cpp:48`, read
  site `Config.cpp:159`.
- `AsyncSystemTimer` — `std::mutex`
  (`shared/IO/Timer/AsyncSystemTimer.h:22`).
- `Log` — `std::mutex` (`shared/Log.h:134`).

## 2. Global object registries (ObjectAccessor / HashMapHolder)

- `HashMapHolder<T>::i_lock` — static `std::shared_timed_mutex` per
  instantiation (`ObjectAccessor.h:49` typedef, `:80` decl; defined
  `ObjectAccessor.cpp:325`; instantiations `:329-332` for `Player`, `Corpse`,
  `ShipTransport`, `MasterPlayer`). Guards each `m_objectMap`. `Insert`/`Remove`
  take `WriteGuard`, `Find` takes `ReadGuard` (`ObjectAccessor.h:53-70`).
  `SaveAllPlayers` holds a `ReadGuard` across the whole save loop
  (`ObjectAccessor.cpp:160`).
- `ObjectAccessor::i_playerGuard`, `i_corpseGuard` — `std::mutex`
  (`ObjectAccessor.h:149-153`). Guard `i_player2corpse` and the
  `playerNameToPlayerPointer` / `playerNameToMasterPlayerPointer` maps.
  Sites: `GetCorpseForPlayerGUID` (`ObjectAccessor.cpp:180`), `RemoveCorpse`
  (`:195`), `AddCorpse` (`:214`), `AddCorpsesToGrid` (`:227`).
  NOTE: `AddObject(Player*)` (`:300-304`) inserts into `HashMapHolder<Player>`
  (taking `i_lock`) and mutates `playerNameToPlayerPointer` with NO
  `i_playerGuard`, while `FindPlayerByNameNotInWorld` (`:93-104`) reads that
  map likewise unguarded — name-map access appears lock-free (single-threaded
  assumption, not verified).

## 3. Map subsystem (per-Map instance unless noted)

- `Map::m_objectsStore_lock` — `mutable shared_timed_mutex` (`Maps/Map.h:691`),
  guards `m_objectsStore`. `InsertObject`/`EraseObject` exclusive
  (`Map.h:515,521`); `GetObject<T>` shared (`Map.h:527`).
- `Map::m_dynamicTreeLock` — `mutable shared_timed_mutex` (`Maps/Map.h:678`),
  guards `m_dynamicTree`. Read sites `Maps/Map.cpp:3327,3349-3367`, write
  sites `:3335,3342`.
- `Map::m_objectsToClientUpdateLock` (`Map.h:643`), `m_unitsRelocatedLock`
  (`:648`), `m_unitsMvtUpdateLock` (`:651`) — `mutable std::mutex` trio
  guarding the同名 cross-thread work sets fed to `ThreadPool`s
  (`m_objectThreads`, `m_motionThreads`, `m_visibilityThreads`, `m_cellThreads`,
  `Map.h:665-668`). Sites `Map.cpp:2736-2770`.
- `Map::m_objectsToRemoveLock` (`Map.h:717`; sites `Map.cpp:1725,1734`),
  `m_scriptSchedule_lock` (`Map.h:721`, `MapMutexType = std::mutex` per
  `Map.h:178`; sites `Map.cpp:2464,2546`), `m_guidGenerators_lock`
  (`Map.h:729`; site `Map.cpp:2972` guards all five per-map guid generators),
  `m_corpseRemovalLock` (`Map.h:654`), `m_bonesLock` (`Map.h:658`).
- `MapManager` members: `m_continentMutex` (`Maps/MapManager.h:194`, +CV `:195`)
  guards `i_maps`/continent creation (`MapManager.cpp:796,806,812`);
  `m_scheduledInstanceSwitches_lock[2]` (`MapManager.h:205`; site
  `MapManager.cpp:774`); `m_scheduledFarTeleportsLock` (`MapManager.h:212`;
  sites `MapManager.cpp:711,734,758`).
- `MapPersistentStateManager::m_cellObjectGuidsMutex` — `shared_timed_mutex`
  (`Maps/MapPersistentStateMgr.h:149`, accessor `:125`); write sites
  `MapPersistentStateMgr.cpp:155-182`.
- `ObjectMgr::m_MapObjectGuids_lock` — `shared_timed_mutex`
  (`ObjectMgr.h:1551`) guards `m_MapObjectGuids`; sites
  `ObjectMgr.cpp:2466-2648,8773-8781`. `GetCellLoadingObjectsMutex()`
  (`ObjectMgr.h:1193`) exposes the cell-loading-objects mutex (per-cell TODO).
- `GridMap::m_mutex` — `LOCK_TYPE = std::mutex` (`Maps/GridMap.h:202-204`);
  site `GridMap.cpp:1126` (`LOCK_GUARD`).
- `MMapManager::loadedMMaps_lock` — `shared_timed_mutex`
  (`Maps/MoveMap.h:100`); read sites `MoveMap.cpp:61,126`, write `:108`.
  `MMapData::navMeshQueries_lock` (`MoveMap.h:63`; sites
  `MoveMap.cpp:333,340`), `tilesLoading_lock` (`MoveMap.h:65`; site
  `MoveMap.cpp:133`), `lockForModels` (`MoveMap.h:104`; site `MoveMap.cpp:444`).
- `VMapManager2::m_modelsLock` — `shared_timed_mutex` (`game/vmap/VMapManager2.h:74`)
  guards `iLoadedModelFiles`; double-checked read→unlock→write upgrade in
  `acquireModelInstance` (`game/vmap/VMapManager2.cpp:298-304`).

## 4. Sessions, sockets, networking

- `WorldSession::m_recvQueue` — `LockedQueue<unique_ptr<WorldPacket>,
  std::mutex>[PACKET_PROCESS_MAX_TYPE]` (`Server/WorldSession.h:784`): one
  producer/consumer queue per packet-processing type, fed by the socket thread,
  drained by the world thread.
- `WorldSession::m_movementPacketCompressorMutex` (`Server/WorldSession.h:830`;
  site `Server/WorldSession.cpp:193`) guards `m_movementPacketCompressor`.
- `WorldSocket::m_sendQueueLock` (`Server/WorldSocket.h:101`) guards
  `m_sendQueue`; manual `lock()`/`unlock()` in `SendPacket`
  (`Server/WorldSocket.cpp:522-531`, drops+disconnects over 1024 queued) and
  the IO-thread drain loop (`:564-572`).
- `World`: `m_asyncTaskQueueMutex` (`World.h:870`; sites
  `World.cpp:2036,3066`) guards `_asyncTasks`; `m_asyncPacketsMutex`
  (`World.h:984`; sites `World.cpp:1941,2002,2123`); `cliCmdQueue`
  (`World.h:1001`) and `addSessQueue` (`World.h:1008`), both
  `LockedQueue<…, std::mutex>`, move CLI commands / new sessions across
  threads into the world update.
- `AsyncSocket::m_atomicState` — `atomic<int>` bitmask
  (`shared/IO/Networking/AsyncSocket.h:115`); comment at `:91` says the atomic
  exists precisely to avoid mutexing the fast path.
- `IoContext::m_contextSwitchQueueLock` (`shared/IO/Context/IoContext.h:77`)
  guards `m_contextSwitchQueue` (Linux epoll path; site
  `IoContext_linux.cpp:78`).
- `AsyncSystemTimer`: `m_pendingTimers_mutex` (Windows, `AsyncSystemTimer.h:52`)
  / `m_orderedPendingTimer_mutex` (Unix, `:65`); sites
  `impl/unix/AsyncSystemTimer.cpp:28,60,87`.

## 5. Broadcast / visibility / movement

- `PlayerBroadcaster::m_listeners_lock`, `m_queue_lock` (`game/PacketBroadcast/PlayerBroadcaster.h:31-32`)
  guard `m_listeners` / `m_queue`.
- `MovementBroadcaster::m_thread_locks` — `vector<shared_timed_mutex>`
  (`game/PacketBroadcast/MovementBroadcaster.h:25`); exclusive on
  re-partition (`MovementBroadcaster.cpp:47,57`), shared on broadcast
  (`:91,111`).
- `Player::m_visibleGUIDs_lock` — `mutable shared_timed_mutex`
  (`Objects/Player.h:1787`) guards `m_visibleGUIDs`; write sites
  `Objects/Player.cpp:18587-18724`, read sites `:19402,21793`,
  `Camera.cpp:148`, `Maps/GridNotifiers.cpp:81`.
- `Player::m_visibleGobjsQuestAct_lock` (`Objects/Player.h:1789`; site
  `Objects/Object.cpp:627`), `Player::m_boundInstancesMutex`
  (`Objects/Player.h:2467`; sites `Objects/Player.cpp:15765-17035`).
- `GameObject::m_UniqueUsers_lock` — `shared_timed_mutex`
  (`Objects/GameObject.h:294`); sites `Objects/GameObject.cpp:721-818`.
- `Unit::asyncMovesplineLock` (`Objects/Unit.h:1448`); ALL acquisitions are
  `try_to_lock` (`Objects/Unit.cpp:3088`, `Movement/MotionMaster.cpp:828`,
  `Movement/RandomMovementGenerator.cpp:108`,
  `Movement/TargetedMovementGenerator.cpp:77`) — a deliberate former-deadlock
  fix documented in the comment block `MotionMaster.cpp:820-827`.
- `ObjectGuidGenerator::lock` (`ObjectGuid.h:323-325`).

## 6. Social / guild / account / misc game

- `SocialMgr::_socialMapLock` (`SocialMgr.h:164`); read `:274`, write
  `:292,334` in `SocialMgr.cpp`.
- `GuildMgr`: `m_guildMutex` (`Guild/GuildMgr.h:80`), `m_guid2GuildMutex`
  (`:82`), `m_petitionsMutex` (`:85`); inline fast paths in the header
  (`:51-65`), bulk ops in `Guild/GuildMgr.cpp:52-308`.
- `AccountMgr`: `m_ipBannedMutex` (`AccountMgr.h:142`), `m_accountPersistentDataMutex`
  (`:148`); sites `AccountMgr.cpp:219,351,405,521,528`.
- `Transport::m_passengerMutex` (`Transports/Transport.h:67`; sites
  `Transports/Transport.cpp:206,232,447`).
- Anticheat: `Anticheat::m_wardenSessionsMutex` (`Anticheat/Anticheat.h:104`;
  sites `Anticheat/Anticheat.cpp:112,172,178`),
  `MovementAnticheat::m_packetLogMutex` (`Anticheat/MovementAnticheat/MovementAnticheat.h:159`;
  sites `.../MovementAnticheat.cpp:147,393`), `Warden::m_packetQueueMutex`
  (`Anticheat/WardenAnticheat/Warden.hpp:186`; site `Warden.cpp:631`).
  Cross-class acquisition: `Handlers/MiscHandler.cpp:1286` takes
  `m_warden->m_packetQueueMutex` from the packet handler.
- `InstanceStatistics`: `m_wipesMutex`, `m_creatureKillsMutex`,
  `m_customCountersMutex` (`InstanceStatistics.h:68-70`; sites
  `InstanceStatistics.cpp:148,179,210`).
- `ScriptMgr::m_scheduledScripts` — `atomic<int>` (`ScriptMgr.h:380`).

## 7. Shared infra: DB, threading, LockedQueue, atomics

- `SqlConnection::m_mutex` — `recursive_mutex` (`shared/Database/Database.h:119-120`);
  `SqlConnection::Lock` RAII guard takes it on construction (`:86-93`), so every
  `Database::Query/DirectExecute` serialises on the target connection.
- `Database::m_nQueryCounter` — `atomic<int>` (`Database.h:318`) for
  round-robin connection selection; `m_nQueryConnPoolSize` (`:317`) is a plain
  int (init-time only, presumably).
- `SqlQueue = LockedQueue<SqlOperation*, std::mutex>` (`Database.h:44`);
  `SqlResultQueue : LockedQueue<IQueryCallback*, std::mutex>`
  (`shared/Database/SqlOperations.h:107,114`).
- `SqlDelayThread`: `m_sqlQueue` + `m_serialDelayQueue` (`shared/Database/SqlDelayThread.h:34-39`);
  `m_running` is `volatile bool` (`:41`), NOT atomic.
- `LockedQueue` internals (`shared/LockedQueue.h:27-120`): single
  `LockType _lock` (`:33`) guards `_queue` (`:36`); `_canceled` is a PLAIN
  `bool` (`:39`, `//*volatile*/` commented out) always accessed under `_lock`
  (`:103-114`). No spinlock/custom CAS anywhere — plain mutex queue.
- `ThreadPool::m_mutex` — `shared_timed_mutex` (`shared/ThreadPool.h:205`)
  guards workload/status; exclusive on submit (`ThreadPool.cpp:33,49`), shared
  + `condition_variable_any` wait in `worker::waitForWork` (`:82-84`).
  `m_active`/`m_index` are `atomic<int>` (`ThreadPool.h:210-211`).
- `Messager<T>::m_messageMutex` (`shared/Multithreading/Messager.h:50`);
  `Execute` swaps under lock then runs callbacks UNLOCKED (`:35-45`).
- `MailerService::_queue_lock` (`shared/MailerService.h:69`) guards
  `_mail_queue`; `_stop` is `atomic_bool` (`:62`); sites
  `MailerService.cpp:61,77`.
- Other cross-thread atomics: `MapManager::i_continentUpdateFinished`
  (`Maps/MapManager.h:196`), `MovementBroadcaster::m_stop`
  (`MovementBroadcaster.h:20`), `WorldSocket::m_sendQueueIsRunning`
  (`Server/WorldSocket.h:103`), `GridMap.h:137` (`AtomicLong` alias);
  `Map.cpp:2818,2926` are function-LOCAL atomics (parallel map-update
  counters), not members. No `pthread_mutex`, ACE, ZThread, SRWLOCK, or
  hand-rolled spinlocks found in either tree.

## 8. Explicit checks requested

- **BattleGroundMgr — CONFIRMED unguarded.** `m_lock` is commented out
  (`Battlegrounds/BattleGroundMgr.h:101-102`), `schedulerLock` is commented
  out (`:303`); the only "mutex" mentions in `BattleGroundMgr.cpp`
  (`:213,245,984`) are code comments ("this code needs mutex", "create
  mutex"). `m_queuedPlayers`, `m_queuedGroups`, `m_selectionPools`,
  `m_queueUpdateScheduler` have NO active lock. Matches the main-doc gap note.
- **ObjectAccessor/HashMapHolder** — see §2. Per-type `shared_timed_mutex`
  statics + two instance mutexes; name maps look unguarded (see NOTE §2).
- **Map store** — see §3 (`m_objectsStore_lock` + 8 further per-Map locks).
- **World session queues** — see §4 (`m_recvQueue`, `addSessQueue`,
  `cliCmdQueue`, `m_sendQueueLock`, `m_async*Mutex`es).
- **SqlDelayThread** — see §7 (two `LockedQueue`s; `volatile bool m_running`).

## 9. Nested / sequential acquisitions (sampled paths)

Observed ORDERING PAIRS (outer → inner, both held simultaneously):

1. `Player::m_visibleGUIDs_lock` (unique) → `HashMapHolder<Player>::i_lock`
   (shared): `Maps/GridNotifiers.cpp:81` holds the visible lock across the
   loop at `:82-93`, which calls `Map::GetPlayer` → `Map.cpp:2619` →
   `ObjectAccessor::FindPlayer` (`ObjectAccessor.cpp:84-91`) →
   `HashMapHolder<Player>::Find` taking `ReadGuard` (`ObjectAccessor.h:65-70`).
   Notably `Camera.cpp:148` takes the SAME visible-lock read guard, so any path
   holding `i_lock` (e.g. `SaveAllPlayers`, `ObjectAccessor.cpp:160`) that
   reached a visible-lock would invert this order — no such inversion was found
   in sampled paths, but the pair is the one to watch in Rust (per-player lock
   → global registry lock).
2. `PlayerBroadcaster::m_queue_lock` → `m_listeners_lock`, consistent order in
   both joint acquisitions: `ProcessQueue` (`PlayerBroadcaster.cpp:53`,
   `q_g` released `:55` while `v_g` still held) and `FreeAtLogout` (`:108`).
   No reverse order found.

SEQUENTIAL (explicitly released before the next is taken — NOT nesting):

3. `MMapManager::loadedMMaps_lock` → `MMapData::tilesLoading_lock`:
   `MoveMap.cpp:126-133` — `rlock.unlock()` at `:128` precedes `wlock`.
4. `VMapManager2::m_modelsLock` read → write upgrade: `slock.unlock()` at
   `VMapManager2.cpp:303` precedes `ulock` at `:304` (double-checked reload).
5. `ObjectGridLoader`: `sObjectMgr` cell-mutex then persistent-state
   cell-mutex in SEPARATE scopes, never co-held
   (`ObjectGridLoader.cpp:234-243,255-265`).
6. `Unit::asyncMovesplineLock` is `try_to_lock` everywhere (§5) — by design
   contributes no ordering edge (see `MotionMaster.cpp:820-827` deadlock note).

No other co-held pairs were found in the sampled call paths (World async-task
and DB `SqlConnection::Lock` sites take a single lock each).

## 10. Gaps / unverified

- `World::m_QueuedSessions` (`World.h:1004`, type `Queue`) — the `Queue`
  typedef target was not resolved (second search returned no hit); presumed a
  `LockedQueue` alias but UNCONFIRMED.
- Name-map thread-safety in `ObjectAccessor.cpp:300-309` (unguarded read/write
  of `playerNameToPlayerPointer`) relies on an assumed single-threaded caller;
  callers were not enumerated.
- `i_playerGuard` (`ObjectAccessor.h:152`) had no lock site surface in the
  sampled grep output — usage (if any) unconfirmed.
- Nesting survey is sampled, not exhaustive: full pairwise verification would
  need a whole-program lock-order pass (good Rust-migration follow-up).

