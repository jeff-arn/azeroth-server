# Phase 4 inventory — Map/Grid/Cell + visibility + update replication

Oracle: C++ at HEAD `1bec69f` (branch `rust-migration`), `vmangos/src/`.
Production binary `2bae5f9`: sampled diffs only (see §f). All anchors below
were read as implementation bodies, not search hits, unless marked.

## (a) Verified structure / flow

### Spatial hierarchy

- Three levels: `Map` → 64×64 `NGridType` slots → 16×16 `GridType` cells each.
  Constants `vmangos/src/game/Maps/GridDefines.h:38-59`:
  `MAX_NUMBER_OF_GRIDS=64`, `SIZE_OF_GRIDS=533.33333f`,
  `MAX_NUMBER_OF_CELLS=16`, cell size `533.33333/16 = 33.3333f`,
  `TOTAL_NUMBER_OF_CELLS_PER_MAP=1024` per axis, `MAP_SIZE=34133.33f`.
- Coordinate mapping is double-precision with `+0.5` rounding and an
  **x/y swap** (`RET_TYPE(y_val, x_val)`), `GridDefines.h:153-167`.
  `CoordPair` clamps to `[0, LIMIT-1]` on construction (`GridDefines.h:83-94`);
  `NormalizeMapCoord` clamps to `±(MAP_HALFSIZE-0.5)` (`:169-175`).
- `Cell` packs grid/cell coords + `nocreate` into a `uint64` union
  (`vmangos/src/game/Maps/Cell.h:49-88`); `Cell(CellPair)` splits by
  `/16` and `%16` (`vmangos/src/game/Maps/CellImpl.h:30-37`).
  `nocreate` is the load-on-visit switch: `Map::Visit` only calls
  `EnsureGridLoaded` when `!cell.NoCreate()`, and silently no-ops if the
  grid pointer is null or data not loaded (`Map.h:1034-1048`).
- Two disjoint object stores per cell (`GridDefines.h:63-65`):
  *World* objects `Player, Creature(pets only), Corpse(resurrectable), Camera`;
  *Grid* objects `GameObject, Creature(non-pet), DynamicObject, Corpse(bones)`.
  `Map::AddToGrid` specializations route pets/bones accordingly
  (`Map.cpp:216-261`); removals mirror them (`Map.cpp:263-295`).
  Per-map GUID store `m_objectsStore` holds only
  `Creature, Pet, GameObject, DynamicObject` behind a `shared_timed_mutex`
  (`Map.h:690-692`, accessors `:512-529`). Players/corpses/ships live in
  global `HashMapHolder<>` maps instead (see ObjectAccessor below).
- `NGrid` (`vmangos/src/framework/GameSystem/NGrid.h:72-179`) owns
  `GridType i_cells[16][16]`, a `grid_state_t`, `i_GridObjectDataLoaded`,
  and `GridInfo` (expiry `TimeTracker` + unload locks: explicit bool and
  active-spawn refcount, `:34-61`).
- `Map` owns `NGridType* m_grids[64][64]` + parallel
  `bool m_bLoadedGrids[64][64]` for terrain tiles and a
  `bitset<1024*1024> marked_cells` (`Map.h:709-715`). Terrain tile
  indices are mirrored: `gx = 63 - grid_x` (`Map.cpp:315-321`,
  unload `:1535-1544`).

### Map classes and creation

- `Map` (base, `Map.h:309-963`) → `WorldMap` (`:965-975`),
  `DungeonMap` (`:977-1006`), `BattleGroundMap` (`:1008-1032`).
  Classification comes from DBC `MapEntry` (`Map.h:80-101`):
  `IsContinent()` is literally `id==0 || id==1`; `Instanceable()` covers
  instance/raid/BG map types.
- `MapManager::CreateMap` (`MapManager.cpp:107-149`): instanceable →
  `CreateInstance` (player only); else `WorldMap` keyed by
  `MapID(mapId, instanceId)` where instanceId is forced to 0 unless it is
  a *reserved continent-shard* id (`< RESERVED_INSTANCES_LAST=100`,
  `Map.h:181`) and `id<=1`. New world maps run `CreateInstanceData(true)`,
  `SpawnActiveObjects()`, `sZoneScriptMgr.MapLoaded`.
- Continent sharding (off by default, `Continents.Instanciate`):
  `GetContinentInstanceId` (`MapManager.cpp:636-698`) maps position to
  shard ids 1–6 (Eastern Kingdoms) / 11–17 (Kalimdor) via hardcoded
  polyline boundary tests (`IsNorthTo`, `:621-634`; shard enum
  `MapManager.h:34-55`). Cross-shard moves are deferred:
  `ScheduleInstanceSwitch` → `SwitchPlayersInstances` after the tick
  (`MapManager.cpp:770-791`, invoked `:367`).
- `Map` ctor (`Map.cpp:122-165`): zeroes grid arrays, calls
  `Map::InitVisibilityDistance()` (**non-virtual dispatch** — subclasses
  re-call their own override after base construction, `Map.cpp:2042`,
  `:2359`), loads terrain ref, persistent state, weather; **only for
  continents** creates four thread pools (objects, motion, visibility,
  cells; `:147-161`); then spawns ship transports + elevator transports
  (`:163-164`, `:549-583`).
- `SpawnActiveObjects` (`Map.cpp:199-206` + loader `:169-196`) force-loads
  (`no_unload=true`) every grid containing a DB spawn with
  `SPAWN_FLAG_ACTIVE`, filtered by continent-shard id.

### Grid load path

- `EnsureGridCreated` (`Map.cpp:303-323`): allocates `NGridType` with
  `m_gridExpiry` and `CONFIG_BOOL_GRID_UNLOAD`, links it into the map's
  `GridRefManager`, sets state `GRID_STATE_IDLE`, loads terrain/VMap tile.
- `EnsureGridLoaded` (`Map.cpp:348-379`): sets `GridObjectDataLoaded=true`
  **before** loading (re-entrancy guard, comment `:362-366`), runs
  `ObjectGridLoader::LoadN()` over all 256 cells, then
  `ObjectAccessor::AddCorpsesToGrid`. Returns true only on first load.
- `EnsureGridLoadedAtEnter` (`Map.cpp:325-346`): on first load also does
  `ResetGridExpiry(0.1f)` + state=`GRID_STATE_ACTIVE`; adds the entering
  player to the grid if given.
- `ObjectGridLoader` (`vmangos/src/game/ObjectGridLoader.cpp:225-314`):
  per cell, `cell_id = y*1024 + x`; GUID sets come from **two** sources
  merged per type — static `ObjectMgr` cell data and per-map
  `MapPersistentState` cell data (separate shared locks). `LoadHelper`
  (`:153-195`) instantiates, `LoadFromDB(guid, map)`, `AddToWorld`,
  `AddToActive` if active, `ViewPoint.Event_AddedToWorld`; failures are
  deleted and skipped. Continent-shard filtering in `IsEnabledOnMap`
  (`:132-151`). Resurrectable corpses load via `ObjectWorldLoader`
  (`:267-280`) from `ObjectAccessor` player→corpse map.
- Player entry additionally force-loads the activation radius:
  `Map::Add(Player)` → `LoadMapCellsAround(GetGridActivationDistance())`
  (`Map.cpp:411`), which is `Cell::VisitAllObjects(..., dont_load=false)`
  with a null notifier (`Object.cpp:3388-3393` — visit side effect is
  grid creation/loading). `ForceLoadGridsAroundPosition`
  (`Map.cpp:381-391`) does the same for far-teleport instance creation.

### Grid state machine / unload

- States `INVALID/ACTIVE/IDLE/REMOVAL` (`NGrid.h:63-70`), dispatched by
  `MapManager::UpdateGridState` (`MapManager.cpp:92-99`) from the tail of
  `Map::Update` for every loaded grid — **skipped entirely for
  battlegrounds** (`Map.cpp:994-1006`, comment: manually added BG objects
  would not reload from DB).
- `ActiveState` (`GridStates.cpp:29-44`): checks only when the tracker
  passes (tracker reset to `0.1 * gridExpiry` while busy); if
  `ActiveObjectsInGrid()==0 && !ActiveObjectsNearGrid(x,y)` → stop grid
  (creatures evade, threat cleared, dynobjects removed —
  `ObjectGridStoper`, `ObjectGridLoader.cpp:361-385`) → `IDLE`.
- `IdleState` (`GridStates.cpp:46-51`): immediately resets expiry to full
  `m_gridExpiry` and moves to `REMOVAL` (so IDLE lasts one tick).
- `RemovalState` (`GridStates.cpp:53-67`): if not unload-locked and
  tracker passed → `Map::UnloadGrid`; on failure resets expiry and retries.
- `UnloadGrid` (`Map.cpp:1502-1548`): refuses if `!pForce &&
  ActiveObjectsNearGrid`; drains remove-list ×3 interleaved with
  `MoveToRespawnN` (creatures whose respawn cell is in another grid are
  relocated home first, `ObjectGridLoader.cpp:49-82,316-326`) and
  `UnloadN`; unloader calls `CleanupsBeforeDelete`, `SaveRespawnTime`
  (unless immediate-save config), `RemoveFromWorld`, `delete`
  (`ObjectGridLoader.cpp:334-359`). Then deletes the `NGrid` and unloads
  the terrain tile. Reload is just the normal load path above — DB state
  (respawn times, persistent-state cell GUIDs) is the only memory.
- `ActiveObjectsNearGrid` (`Map.cpp:1836-1870`): expands the grid's cell
  rect by `ceil(visibilityDistance / 33.3333) + 1` cells (at 100yd → 4
  cells ≈133yd) and tests every player + active non-player position.
  Note it uses the **current adaptive** `m_visibilityDistance`.

### Tick: MapManager → Map::Update (critic anchors re-verified)

- `MapManager::Update` (`MapManager.cpp:289-408`) is gated by
  `i_timer` (`MapUpdateInterval`). Per cycle: run delayed far-teleports;
  for each map passing `ShouldUpdateMap` call **`UpdateSync(mapsDiff)`
  synchronously on the manager thread**, then queue `DoUpdate` —
  instanceable maps on the `MapManager` thread pool, non-instanceable
  (continent shards + tram etc.) one task each on a continent pool sized
  to the task count. Instance-creation for far teleports runs on its own
  1-thread pool polling every 20ms (`:251-287`). After pools join:
  `SwitchPlayersInstances`, far-teleports again, crashed-map unload,
  `CanUnload` sweep deleting empty instance maps.
- `ShouldUpdateMap` (`Map.cpp:3458-3481`): skip when no players
  (**bots count as players** — it uses `HavePlayers()`, not
  `HaveRealPlayers()`) and `now - m_lastPlayerLeftTime >
  MapUpdate.Empty.UpdateTime` (default 0 = never skip), unless corpses
  are pending removal.
- `UpdateSync` (`Map.cpp:677-691`) updates **transports only**, before and
  outside the per-map parallel phase, because transports can trigger
  map↔map teleports. Transports are not grid objects: they live in
  `m_transports` (`Map.h:695-697`), update even in inactive grids, and
  add/remove bypasses cells entirely (`Map.cpp:507-535`, `:1296-1329`).
- `DoUpdate` (`Map.cpp:921-931`) clamps `diff` to `maxDiff` (= elapsed
  timer value), refreshes `m_lastPlayerLeftTime` if populated.
- `Map::Update` body is exactly `Map.cpp:933-1060` — **critic anchor
  confirmed**. Order:
  1. `m_dynamicTree.update`, session packet pump (`WorldSession::Update`
     with `MapSessionFilter`) — `:937-951`.
  2. `UpdateSessionsMovementAndSpellsIfNeeded()` + **`UpdatePlayers()`
     at `:957`** — first of the double call, confirmed.
  3. `UpdateCells(t_diff)` (`:960`) — creature/GO/dynobject updates.
  4. `SendObjectUpdates()` (`:964`) — field-change replication.
  5. `UpdateVisibilityForRelocations()` (`:967`) — create/destroy churn.
  6. `UpdateSessionsMovementAndSpellsIfNeeded()` + **`UpdatePlayers()`
     again at `:971`** — second call confirmed. Both calls are
     rate-limited internally (next item), so the second is usually a
     no-op unless ≥`UpdatePlayersDiff` elapsed during steps 3–5.
  7. Corpse/bones cleanup (`:974-975`).
  8. **Continent barrier `:981-993` — confirmed, with a correction:**
     the condition is `!Instanceable()`, not `IsContinent()`, so it also
     covers non-instanceable non-continent maps (e.g. Deeprun Tram).
     Each such map calls `MarkContinentUpdateFinished()` (atomic counter
     vs `i_maxContinentThread`, `MapManager.cpp:793-802`) then spins —
     repeatedly `UpdateSessionsMovementAndSpellsIfNeeded()` +
     `UpdatePlayers()` (a **third+** `UpdatePlayers` site, `:989`) —
     until all continent maps finish or `MapUpdateInterval` elapses
     from tick start. This is a latency-filling barrier, not a lockstep
     simulation barrier: fast maps keep servicing players while waiting.
  9. Grid-state sweep (above), scripted events (1s timer), `ScriptsProcess`,
     `InstanceData::Update`, weather (`:994-1022`).
  10. Continent-only adaptive distances (`:1032-1058`, see §d).
  `DungeonMap::Update` is a pure forwarder (`Map.cpp:2233`);
  `BattleGroundMap::Update` adds `BattleGround::Update` after (`:2364-2372`).
- Sub-rate limiters (all ms, `World.cpp:949-952` defaults):
  `UpdatePlayers` gated by `MapUpdate.UpdatePlayersDiff=100`
  (`Map.cpp:888-919`); `UpdateCells` by `UpdateCellsDiff=100`
  (`:819-847`); movement/spell packets by `UpdatePacketsDiff=100`
  (`:875-886`). `UpdatePlayers` also skips inactive continent players
  (not in combat, no recent spell packet, no scheduled event) for
  `Continents.InactivePlayers.SkipUpdates` cycles (default 0 = off),
  accumulating skipped time into the next real update (`:898-917`);
  non-continents never skip.

### UpdateCells split (assigned range Map.cpp:743-875)

- Sync path `UpdateActiveCellsSynch` (`:794-816`): clears `marked_cells`,
  then for each player and each active non-player calls
  `UpdateCellsAroundObject` (`:693-723`), which visits every cell in the
  `GetGridActivationDistance()` square (`Cell::CalculateCellArea`,
  `CellImpl.h:39-48`) exactly once via the mark bitset, applying
  `ObjectUpdater(diff, now)` to both grid and world containers.
  `ObjectUpdater` = `WorldObject::UpdateHelper::UpdateRealTime` per
  object (`GridNotifiers.cpp:179-187`). Cells outside all activation
  squares receive **no updates at all** (frozen AI/timers).
- Async path (continents only, when cell pool READY; `:831-834`):
  `UpdateActiveCellsAsynch` (`:771-792`) first marks cells around all
  players/actives (`MarkCellsAroundObject`, `:725-740`), then runs
  `UpdateActiveCellsCallback` (`:743-769`) in 2 steps × N threads,
  striping rows by `safeDistCells = MTCells.SafeDistance/33.3333 + 1`
  (default 1066yd → 32 cells) so concurrent rows stay ≥ safe-distance
  apart. Default `MTCells.Threads=0` → pool created with
  `max(0-1,0)=0` extra threads (`Map.cpp:157`); behaviour then depends
  on ThreadPool READY semantics (not inspected — §f).
- After cells, queued async motion updates (`m_unitsMvtUpdate`) run on
  the motion pool and the set is cleared (`:836-846`).
- Active-object registry: `m_activeNonPlayers` (`Map.h:686-688`),
  `AddToActive`/`RemoveFromActive` (`Map.cpp:1872-~1930`); adding a
  DB-spawned creature also `incUnloadActiveLock()` on its **respawn**
  grid to prevent clone-on-reload. `Map::Add<T>` only grid-loads for
  active objects; inactives just ensure the `NGrid` exists
  (`Map.cpp:481-484`) — an inactive object can sit in an unloaded-data grid.

### Visibility add/remove (critic anchor Map.cpp:1594-1688)

Anchor range is real but coarse. Actual layout: `UpdateObjectVisibility`
`:1594-1607`, `UpdateActiveObjectVisibility` ×3 `:1609-1646`,
`SendInitSelf` `:1648-1676`, `SendInitTransports` `:1678-1692`.

Three distinct trigger paths converge on `Player::UpdateVisibilityOf`:

1. **Object-centric (enter/leave world):** `Map::Add/Remove` (player
   `:426`, `:1234`; generic `:499`, `:1281`) call
   `UpdateObjectVisibility(obj, cell, pair)`, which visits *Cameras* in
   cells within `GetVisibilityDistance()` (`VisibleChangesNotifier`,
   `GridNotifiers.cpp:34-38`) so nearby players re-evaluate the object;
   if the object is a player it also sweeps all active objects
   map-wide (`UpdateActiveObjectVisibility`, comment notes ~360 actives
   world-wide uninstanced, `Map.cpp:1602-1606`).
2. **Camera-centric (full re-scan):** `Camera::UpdateVisibilityForOwner`
   (`Camera.cpp:141-153`) copies `m_visibleGUIDs`, visits all objects
   within visibility distance of the camera **source** (usually the
   player body; can be a possessed/farsight unit — `Camera::SetView`
   `:51-91` registers the source as an active object), then
   `VisibleNotifier::Notify` (`GridNotifiers.cpp:40-113`): transport
   passengers and active objects are rescued from the leftover set,
   leftovers become out-of-range GUIDs, are erased from
   `m_visibleGUIDs`, unsubscribed from those players' broadcasters, and
   reciprocal out-of-range is pushed to affected players.
3. **Relocation-centric (movement):** `Unit::OnRelocated`
   (`Unit.cpp:10208-10225`) enqueues a delayed action once squared
   distance from `m_lastNotifiedPosition` exceeds
   `World::GetRelocationLowerLimitSq()` (value not inspected — §f);
   `Map::UpdateVisibilityForRelocations` (`Map.cpp:2899-2966`) drains
   `m_unitsRelocated` (continent: visibility pool, adaptive thread
   count, `MapUpdate.VisibilityUpdate.Timeout=100ms` budget — leftovers
   stay queued for next tick) calling
   `Unit::ProcessRelocationVisibilityUpdates` (`Unit.cpp:10227-10235`)
   = camera re-scan for viewers of this unit + `UpdateObjectVisibility`.

Decision per (viewer, target) — `Player::UpdateVisibilityOf`
(`Player.cpp:18577-18739`, three variants: immediate-send, and
`UpdateData`-batching generic + Player-specialized):
visible ⇐ `target->FindMap()` ∧
`IsWithinVisibilityDistanceOf(viewer, viewPoint, inVisibleList)` ∧
`IsVisibleForInState(viewer, viewPoint, inVisibleList)`.
On gain: build create block, insert GUID into `m_visibleGUIDs`, and for
player targets `m_broadcaster->AddListener(viewer)`. On loss: out-of-range
block / `DestroyForPlayer`, erase GUID, `RemoveListener`.
Distance test (`Object.cpp:2195-2215`, `IsWithinVisibilityDistanceOf`):
2D distance (`IsWithinDistInMap(..., is3D=false)`) from the **viewPoint**
(camera source), threshold
`max(mapVisibility + (inVisibleList ? greyUnit : 0), targetVisibilityModifier)`;
taxi-flying viewers use flight distance + grey-object instead; same-transport
pairs skip the distance test entirely.
`IsVisibleForInState` bodies (stealth/GM/phase etc.) were located
(`Unit.h:618`, `GameObject.cpp:1090`, `DynamicObject.cpp:269`) but not
read — §f.

Cell/grid relocation itself: `PlayerRelocation` (`Map.cpp:1343-1389`)
moves the player between cells/grids (loading the destination grid via
`EnsureGridLoadedAtEnter`), fires `ViewPoint.Event_GridChanged`, and
force-activates the entered grid (`ResetGridExpiry(0.1f)` + ACTIVE).
`DoPlayerGridRelocation` (`:1392-1424`) is the extrapolation variant
(no `Relocate`/`OnRelocated`). `CreatureRelocation` (`:1426-1448`) →
`CreatureCellRelocation` (`:1450-1473`): non-active creatures may not
enter unloaded grids — they fall back to `CreatureRespawnRelocation`
(`:1475-1500`, teleport home + motion re-init) or stay frozen.

### Replication: SendObjectUpdates

- Dirty set `m_objectsToClientUpdate` (`Map.h:641-644`);
  `AddUpdateObject` (`Map.cpp:2732-2738`) **drops inserts while
  `m_processingSendObjUpdates`** (same pattern for relocations,
  `:2747-2753`) — updates dirtied during the send phase wait a tick.
- `SendObjectUpdates` (`Map.cpp:2777-2895`): snapshots iterators,
  work-steals via `std::atomic_int` across the continent object pool
  (instances/world maps without a pool run single-threaded — pools are
  continent-only, ctor `:149-154`), each worker accumulates a per-player
  `UpdateDataMapType` via `Object::BuildUpdateData` then sends.
  Global budget `MapUpdate.ObjectsUpdate.Timeout=100ms`: on expiry the
  unprocessed tail **remains in the set** for the next tick (`:2845-2848`).
- Broadcast helpers: `MessageBroadcast`/`MessageDistBroadcast`
  (`Map.cpp:585-669`) visit Cameras only, radius = visibility distance
  or caller dist, no-create, and silently drop if the sender's grid
  isn't loaded. Player movement packets additionally go through
  `PlayerBroadcaster` listener sets maintained by visibility gain/loss
  (above) — body not inspected (§f).

### ObjectAccessor (global registry)

`vmangos/src/game/ObjectAccessor.cpp` (332 lines): singleton over
`HashMapHolder<Player|Corpse|ShipTransport|MasterPlayer>` static maps
(`:322-332`) + name→pointer maps (`:297-319`). Map-relevant roles:
player lookup by GUID/name regardless of map (`GetUnit` fast-path for
players, `:52-64`; otherwise delegates to the object's own map store),
and the resurrectable-corpse registry `i_player2corpse` feeding grid
loads (`AddCorpsesToGrid :225-245`, instance-filtered) and cross-map
corpse conversion via `Map::AddCorpseToRemove` (`:247-280`).
`Map::GetPlayer/GetUnit/GetWorldObject` are map-local counterparts
(`Map.h:499-510`).

### Bot branch at Map.cpp:1781

`Map::HaveRealPlayers()` (`Map.cpp:1778-1784`, decl `Map.h:439`):
iterates `m_mapRefManager`, returns true on first `!IsBot()`.
Repo-wide search found exactly **one caller**:
`PlayerBots/BattleBotAI.cpp:746`. Nothing in the map tick, grid unload,
or `ShouldUpdateMap` uses it — bot-only maps tick, activate cells, and
pin grids exactly like human-populated maps. A Rust port needs the
predicate only for bot AI parity, not for simulation gating.

## (b) Sizes / counts

| Item | Value | Anchor |
|---|---|---|
| Grids per map axis / total slots | 64 / 4096 ptrs | `GridDefines.h:38`, `Map.h:709` |
| Cells per grid | 16×16 = 256 | `NGrid.h:177` |
| Cells per map axis / total | 1024 / 1,048,576 | `GridDefines.h:54` |
| Cell / grid edge | 33.3333yd / 533.3333yd | `GridDefines.h:40,49` |
| `marked_cells` bitset | 1,048,576 bits = 128 KiB per Map | `Map.h:715` |
| Map.cpp / Map.h | 3728 / 1049 lines | wc |
| MapManager.cpp/.h | 814 / 230 | wc |
| ObjectGridLoader.cpp/.h | 387 / 117 | wc |
| ObjectAccessor.cpp/.h | 332 / 163 | wc |
| GridNotifiers .cpp/.h/Impl | 227 / 1585 / 627 | wc |
| Cell.h + CellImpl.h + GridDefines.h | 116 + 239 + 185 | wc |
| GridStates.cpp/.h, Camera.cpp/.h | 67/62, 161/138 | wc |
| Whole `Maps/` dir (incl. GridMap, MoveMap, scripts) | 20,773 lines total with loader/accessor/camera | wc |
| Script commands embedded in Map | 93 fn ptrs (`SCRIPT_COMMAND_MAX`) | `Map.h:864-958` |
| Continent thread pools per continent Map | 4 (obj/motion/vis/cell) | `Map.h:665-668` |
| Reserved continent-shard instance ids | 1..100 (`RESERVED_INSTANCES_LAST`) | `Map.h:181` |
| Active objects scan cost note | ~360 world-wide if uninstanced | `Map.cpp:1604` (comment) |
| SendObjectUpdates cost note | ~2ms/object @500 nearby players | `Map.cpp:2779-2780` (comment) |

Derived: at 100yd visibility the activation square is
`CalculateCellArea(±100yd)` ≈ 7×7 cells ≈ 49 cells per player before
mark-dedup; unload-proximity halo is 4 cells (see §a).

## (c) DB / config inputs

Static / DBC (read at startup, consumers above):

- `Map.dbc` → `MapEntry` (`Map.h:80-101`): map type, maxPlayers,
  resetDelay, ghost entrance, scriptId. Drives `WorldMap` vs
  `DungeonMap` vs `BattleGroundMap` and `IsContinent()==(id 0|1)`.
- World-DB spawn tables via `ObjectMgr` cell index:
  `GetCellObjectGuids(mapId, cell_id)` (`ObjectMgr.h:1182`, decl only)
  returns creature/gameobject/corpse GUID sets per cell; each object's
  full row is then read by `LoadFromDB(guid, map)` inside
  `ObjectGridLoader` (`ObjectGridLoader.cpp:174`). Table names/columns
  are inside `ObjectMgr`/entity loaders — not inspected here (§f).
- Per-instance persistent state (`MapPersistentState`): second,
  mutable cell-GUID source merged on every grid load
  (`ObjectGridLoader.cpp:240-243,261-264`) + respawn times saved on
  unload (`:347-348`) / deleted on instance reset
  (`Map.cpp:2326-2327`).
- Characters DB: `SELECT MAX(id) FROM instance` seeds instance-id
  allocation (`MapManager.cpp:446-457`), floor 100.
- Terrain data files (`.map`/vmap/mmap trees): `TerrainMgr.LoadTerrain`
  in `Map` ctor, per-tile `Load(gx,gy)` on grid creation
  (`Map.cpp:112-120`); optional full preload
  (`MapManager.cpp:57-74`).

Config (`World.cpp` defaults; shipped example
`vmangos/run/etc/mangosd.conf.example` values noted where they differ):

| Key | Code default | Example conf | Effect (anchor) |
|---|---|---|---|
| `MapUpdateInterval` | 100ms, min 50 (`World.cpp:573`) | 100 | Manager tick gate; **but** `SetMapUpdateInterval` caps any value >50 back to 50 (`MapManager.h:115-122`) — effective interval is 50ms regardless. Verify against prod conf (§f). |
| `GridCleanUpDelay` | 300000ms, min 60000 (`World.cpp:569`) | 300000 | `m_gridExpiry` for every new Map (`MapManager.cpp:42`) |
| `GridUnload` | true (`World.cpp:563`) | **0** | false ⇒ every grid born with explicit unload lock (`Map.cpp:307`, `NGrid.h:39`) — grids never unload under example conf |
| `Visibility.Distance.Continents/Instances/BG/InFlight` | 100/170/**533**/100 (`ObjectDefines.h:30-32`, `World.cpp:827-869`) | 100/170/**180**/100 | Base visibility + grid-activation distance per map class; clamped to `[45*Rate.Creature.Aggro, 533.33-grey]` |
| `Visibility.Distance.Grey.Unit/Object` | 1 / 10 (`World.cpp:813,819`) | 1 / 10 | Hysteresis added only when already visible (`Object.cpp:2201,2209`) |
| `MapUpdate.UpdatePacketsDiff/UpdatePlayersDiff/UpdateCellsDiff` | 100/100/100 (`World.cpp:949-951`) | — | Sub-rate limiters (§a) |
| `MapUpdate.ObjectsUpdate.MaxThreads/Timeout` | 4 / 100ms (`World.cpp:942-943`) | — | `SendObjectUpdates` pool (continents) + budget |
| `MapUpdate.VisibilityUpdate.MaxThreads/Timeout` | 4 / 100ms (`World.cpp:944-945`) | — | Relocation visibility pool + budget |
| `MapUpdate.Continents.MTCells.Threads/SafeDistance` | 0 / 1066 (`World.cpp:947-948`) | — | Async cell striping (§a) |
| `Continents.MotionUpdate.Threads` | 0 (`World.cpp:960`) | — | Motion pool size (0-thread pool still constructed, `Map.cpp:155`) |
| `Continents.InactivePlayers.SkipUpdates` | 0 (`World.cpp:952`) | — | Inactive-player skip cycles |
| `Continents.Instanciate` | false (`World.cpp:959`) | 0 | Enables continent shards 1–6/11–17 |
| `MapUpdate.Empty.UpdateTime` | 0 (`World.cpp:941`) | — | Empty-map tick suspension (`ShouldUpdateMap`) |
| `Instance.UnloadDelay` | 1800000ms (`World.cpp:643`) | — | `DungeonMap` unload timer (`Map.cpp:2046,2241`) |
| `Terrain.Preload.Continents/Instances` | true/true (`World.cpp:961-962`) | — | Startup terrain preload |
| `MapUpdate.Tick.Lower/Increase.*`, `*.Min.*` distances | read at `Map.cpp:1034-1057` (defaults not inspected, §f) | `Visibility.Distance.Continents.Min=60` | Adaptive distance controller |

Visibility-distance initialization chain: ctor sets
`DEFAULT_VISIBILITY_DISTANCE=100` (`Map.cpp:122`), then
`Map::InitVisibilityDistance` overwrites with continent values for
**every** map (`:208-213`), then `DungeonMap`/`BattleGroundMap` ctors
overwrite again with instance/BG values (`:2051-2056`, `:2377-2382`).
`WorldMap` has no override, so non-continent world maps (tram) use
continent distances. `MapManager::InitializeVisibilityDistanceInfo`
(`MapManager.cpp:101-105`) re-runs the (virtual) initializer on all
live maps after config load.

## (d) Wire-visible semantics a Rust port must replicate

1. **Create/destroy pairing is per-viewer and stateful.** Each player
   has `m_visibleGUIDs`; gain ⇒ exactly one create block (batched into
   `UpdateData`, compressed when large — `UpdateData` internals not
   inspected, §f), loss ⇒ out-of-range/destroy. Self is never in the
   set (Player specialization early-return, `Player.cpp:18662-18667`);
   MO-transports and GO entry 181223 (Naxx necropolis) are never
   inserted (`Player.cpp:18620-18637`), so they also never get
   out-of-range — cleanup for transports is the separate
   `SendInitTransports`/`SendRemoveTransports` full-map blast on
   add/remove (`Map.cpp:1678-1708`), ignoring distance entirely.
2. **Hysteresis (grey distances).** Objects are *acquired* at base
   distance but *retained* until base+1yd (units) — flight viewers
   retain objects to flight+10yd. Replicating only the base distance
   causes client-visible pop-in/out flicker at the boundary.
3. **Distance is 2D and camera-relative.** Z is ignored; the origin is
   the camera source (farsight/possession moves the visibility centre
   away from the player body, and that source becomes an active object
   that keeps its cells ticking). Target-side
   `VisibilityModifier` (e.g. `MAX_VISIBILITY_DISTANCE=533.33` on some
   creatures/GOs, located at `Creature.cpp:612`, `GameObject.cpp:249`)
   overrides the map distance via `max(...)`.
4. **Adaptive distances are wire-visible.** Continents shrink/grow
   `m_visibilityDistance` and `m_gridActivationDistance` by ±1yd per
   tick based on tick duration and broadcaster slowness
   (`Map.cpp:1032-1058`), floored by Min configs, capped at the
   configured continent max. Under load, players literally see less
   far and fewer cells simulate. A port that fixes distances changes
   gameplay visibility under load (acceptable only as an explicit
   divergence).
5. **Tick ordering within a cycle:** session packets → players →
   cells (creatures) → field updates (`SendObjectUpdates`) →
   visibility creates/destroys → players again. Consequence: an object
   that becomes visible this tick sends its create *after* this tick's
   field updates; field changes dirtied during the send phase are
   dropped until next tick (`m_processingSendObjUpdates` guard).
   Clients therefore observe ≥1-tick replication latency for churn
   during heavy phases, and both send phases can truncate at their
   100ms budgets, deferring the tail (unordered-set order — no
   fairness guarantee).
6. **Frozen out-of-range world.** Cells beyond every player's
   activation square do not update: creature AI, respawn timers driven
   by updates, and GO timers pause; grid unload additionally evades
   combat and deletes dynobjects. Grid state itself is invisible to
   clients except through this freezing and through respawn behaviour
   after reload (objects re-materialize from DB state).
7. **Transport exceptionalism:** transports update first every cycle
   (`UpdateSync`), exist outside grids, are created for every entering
   player map-wide, and same-transport visibility ignores distance.
   Passengers are rescued from out-of-range in `VisibleNotifier::Notify`.
8. **Broadcast radius = visibility radius.** `MessageBroadcast` family
   delivers to Cameras in cells within the map's *current* (adaptive)
   visibility distance, searched as a cell square/octagon
   (`CellImpl.h:58-115`, radius clamped to 533.33 at `:72-73`) — actual
   Euclidean filtering, where present, happens inside notifiers
   (`MessageDistDeliverer` checks `IsWithinDist`, plain
   `MessageDeliverer` does not — `GridNotifiers.cpp:115-151`). Some
   packets therefore reach players in square corners beyond the
   nominal radius.
9. **Login/enter sequence:** `SendInitTransports` then `SendInitSelf`
   (self create + own transport + visible co-passengers) then the
   visibility sweep (`Map.cpp:417-426`, order comment `:413-416`);
   far-teleport entry first clears `m_visibleGUIDs` (`:422-423`).
   Reordering breaks door/quest-giver visibility per the inline comment.
10. **Instance vs continent differences that reach the wire:**
    instances/BGs never barrier-wait, never skip inactive players,
    run single-threaded replication, BGs never unload grids;
    instances use 170yd (BG 180yd per example conf) flat visibility
    with no adaptation. Dungeon entry/exit also drives bind/reset
    packets (`SMSG_INSTANCE_SAVE_CREATED`, transfer aborts —
    `Map.cpp:2075,2100,2214-2217`) — owned by instance logic, listed
    here only as map-boundary effects.

## (e) Smallest viable first slice

**Slice: single-threaded `WorldMap` for one continent map id (0 or 1),
no sharding, no grid unload, fixed 100yd visibility.**

Contents, in dependency order:

1. Coord math: `ComputeGridPair/ComputeCellPair` with the exact
   double-precision `+0.5` rounding, x/y swap, and clamping
   (`GridDefines.h:153-183`) — every other subsystem keys off this.
2. Storage: `Map { grids: [[Option<NGrid>; 64]; 64] }`,
   `NGrid { cells: [[CellStore; 16]; 16], state, data_loaded }` with
   the world/grid type split (pets+players+cameras vs the rest) and a
   per-map GUID→object store for creature/GO/dynobject/pet.
3. Load-on-demand: `EnsureGridCreated/Loaded` equivalents fed by an
   `ObjectMgr`-shaped cell-GUID index (can start as an in-memory
   fixture), `data_loaded` set before population, corpse injection
   stubbed to empty.
4. Tick loop at fixed interval implementing `Map::Update` order:
   sessions → players → `UpdateActiveCellsSynch` (mark-bitset dedup,
   activation square) → `SendObjectUpdates` (single-threaded, no time
   budget) → `UpdateVisibilityForRelocations` (drain-all) → players.
   Skip the continent barrier (single map ⇒ trivially satisfied).
5. Visibility core: per-player `visible_guids` set +
   `UpdateVisibilityOf` three-condition test with grey-distance
   hysteresis, the three trigger paths reduced to two (object-centric
   on add/remove; relocation queue on movement threshold), camera =
   player body only. `IsVisibleForInState` starts as `true` with a
   TODO tied to §f.
6. Relocation: player/creature cell moves with grid-load-on-enter,
   entered-grid force-activation, creature respawn-fallback.

Entry dependencies (must exist first, owned by other surfaces):
`WorldObject/Unit/Player` identity + position + `Update` stubs;
`UpdateData` create/update/out-of-range block builders and session
send; config values in §c (hardcode defaults initially);
`ObjectGuid` allocation. Deferred until later slices: transports
(own registry + `UpdateSync`), grid unload state machine (ship with
`GridUnload=false` semantics — matches shipped example conf),
continent sharding + barrier, async pools, adaptive distances,
`DungeonMap`/`BattleGroundMap`, active-object map-wide sweeps,
terrain/VMap loading (height/LOS surface), script commands.

## (f) Unresolved

- `IsVisibleForInState` bodies for Unit/GameObject/DynamicObject
  (stealth, invisibility, GM, phase, spectator rules) — located at
  `Unit.h:618`, `GameObject.cpp:1090`, `DynamicObject.cpp:269`, not read.
- `World::GetRelocationLowerLimitSq()` / `GetRelocationAINotifyDelay()`
  values and config keys — relocation enqueue threshold unquantified.
- `UpdateData` wire format (block types, compression threshold,
  `SMSG_UPDATE_OBJECT` vs compressed variant) — only call sites read;
  `vmangos/src/game/Objects/UpdateData.{cpp,h}` not opened.
- `PlayerBroadcaster`/`MovementBroadcaster` queueing semantics
  (listener delivery, `IsMapSlow` criterion feeding §d.4) — not opened.
- ThreadPool READY semantics for 0-thread pools (whether async cell
  path runs at default config) and `MapUpdate.Tick.*` config defaults.
- Effective prod values: production `mangosd.conf` not in repo sample;
  `MapUpdateInterval` cap-to-50 behaviour (`MapManager.h:118`) vs the
  100ms sub-limiters, and BG distance 533 (code) vs 180 (example conf),
  need the live config to settle.
- Spawn-table schema behind `ObjectMgr::GetCellObjectGuids` and
  `LoadFromDB` (shared with the object/DB surfaces).
- Prod divergence `2bae5f9`→HEAD: `git diff -w --stat` shows
  Map.cpp 257, MapManager.cpp 282, GridDefines.h 39,
  ObjectGridLoader.cpp 71, ObjectAccessor.cpp 18 changed lines; the
  hunks actually sampled (GridDefines constants, Map.cpp includes) are
  formatting/reordering only with identical constant values. A full
  semantic diff of the tick/visibility functions was **not** performed,
  so behaviour parity with the production binary is unconfirmed.
- `GridSearchers`, `GridNotifiersImpl.h` visitor bodies and
  `ViewPoint` event plumbing beyond `Camera.cpp` were not read;
  searcher-based queries (nearest-unit etc.) belong to AI/spell surfaces.
