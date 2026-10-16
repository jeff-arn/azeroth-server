# 03 — World Simulation Core (`vmangos/src/game`, excluding `PlayerBots`)

Evidence base for a future C++→Rust migration guide. Scope is the world
simulation core only; `PlayerBots/` (24,254 LOC) is a sibling scope and is
excluded from every count below except where coupling *into* the core is
noted. All line citations were taken from inspected file bodies in this
checkout. This document states architecture and evidence; migration
sequencing opinions are limited to per-subsystem implications.

## 1. Size inventory

| Area | Files | LOC (.h+.cpp) | Share |
|---|---|---|---|
| `src/game` total (excl. `PlayerBots`) | 485 total incl. bots (236 .cpp / 249 .h) | **283,427** | 100% |
| `Objects/` | — | 70,964 | 25.0% |
| `Spells/` | — | 33,357 | 11.8% |
| `Commands/` (GM/debug) | — | 23,591 | 8.3% |
| `Maps/` (+`Pool/`) | — | 20,975 | 7.4% |
| `Handlers/` (opcode handlers) | 27 files | 16,185 | 5.7% |
| `Server/` (WorldSocket/session/opcodes) | — | 10,495 | 3.7% |
| `Movement/` (+`spline/`) | — | 8,767 | 3.1% |
| `Battlegrounds/` | — | 8,667 | 3.1% |
| `Chat/` | — | 7,981 | 2.8% |
| `AI/` (core AI, excl. content scripts) | 33 files | 7,592 | 2.7% |
| `vmap/` (collision/LoS) | — | 5,159 | 1.8% |
| `Anticheat/` | — | 5,239 | 1.8% |
| `Group/` | — | 4,017 | 1.4% |
| `Guild/` | — | 1,971 | 0.7% |
| `AuctionHouse/` | — | 1,929 | 0.7% |
| `Threat/` | — | 1,042 | 0.4% |
| top-level `src/game/*.cpp/.h` (World, ObjectMgr, ScriptMgr, LootMgr, …) | — | remainder (~55k) | ~19% |
| `src/scripts/` (content scripts, separate tree) | 223 .cpp | **107,413** | — |

Method: `find vmangos/src/game -path '*PlayerBots*' -prune … | xargs wc -l`.
`src/scripts` is not inside `src/game` but is inseparable from the sim core
(see §9); it adds 38% on top of the core LOC.

### 1.1 God-classes (largest translation units, excl. `PlayerBots`)

| Rank | File | LOC | Class / role |
|---|---|---|---|
| 1 | `Objects/Player.cpp` | 22,443 | `Player` — inventory, quests, spells, taxi, trade, save/load, bot hooks |
| 2 | `ObjectMgr.cpp` | 12,038 | `ObjectMgr` — all DB template/spawn loading + GUID allocation |
| 3 | `Objects/Unit.cpp` | 11,073 | `Unit` — combat, damage, auras application, movement glue |
| 4 | `Spells/Spell.cpp` | 8,413 | `Spell` — cast pipeline, targeting, checks |
| 5 | `Spells/SpellAuras.cpp` | 8,261 | `Aura`/`SpellAuraHolder` — ~200 aura handler functions |
| 6 | `Spells/SpellEffects.cpp` | 5,805 | `Spell::Effect*` — per-effect handlers, spawns objects directly |
| 7 | `Commands/CharacterCommands.cpp` | 5,806 | GM commands (not sim, but linked in) |
| 8 | `Objects/Creature.cpp` | 4,118 | `Creature` — spawn, AI hookup, loot, vendor/trainer |
| 9 | `Spells/SpellMgr.cpp` | 3,888 | `SpellMgr` — DBC+DB spell data loading/correction |
| 10 | `Maps/Map.cpp` | 3,728 | `Map` — update loop, visibility, grid lifecycle |
| 11 | `Objects/Object.cpp` | 3,674 | `Object`/`WorldObject` — update fields, visibility, LoS helpers |
| 12 | `World.cpp` | 3,201 | `World` — global tick, config, session mgmt |
| 13 | `ScriptMgr.cpp` | 2,930 | DB-script loading + dispatch |
| 14 | `Objects/GameObject.cpp` | 2,596 | `GameObject` — GO types, transports, fishing, chests |
| 15 | `Objects/Player.h` | 2,565 | `Player` declaration alone (~1,283 member-ish lines) |
| 16 | `Maps/ScriptCommands.cpp` | 2,526 | DB script command interpreter |
| 17 | `Group/Group.cpp` | 2,485 | `Group` — party/raid, loot rules, BG queue glue |
| 18 | `Objects/SpellCaster.cpp` | 2,387 | shared Unit/GameObject casting base |
| 19 | `Objects/Pet.cpp` | 2,351 | `Pet : Creature` |
| 20 | `UnitAuraProcHandler.cpp` | 2,000 | proc dispatch table implementation |

Header sizes tell the same story: `Player.h` 2,565 lines, `ObjectMgr.h`
1,596, `Unit.h` 1,535, `GridNotifiers.h` 1,585, `SharedDefines.h` 1,841.
Any Rust port is dominated by five files (`Player.cpp`, `ObjectMgr.cpp`,
`Unit.cpp`, `Spell.cpp`, `SpellAuras.cpp` = 62,228 LOC, 22% of the core).

**Rust implication.** There is no "small core" to port first inside
`Objects/`+`Spells/`: the five god-files call each other directly
(`Player.cpp` has 64 `#include`s, `Unit.cpp` 42, `Object.cpp` 34 —
measured). A Rust rewrite must either port this cluster as one crate with
internal modules mirroring the C++ split, or define an FFI seam *around*
the whole cluster (see §11).

## 2. Object hierarchy, GUIDs, and the update-field model

### 2.1 Hierarchy (single inheritance, virtual dispatch)

```
Object                                   Objects/Object.h:111
├── Item : Object                        Objects/Item.h:39
│   └── Bag : Item                       Objects/Bag.h:33
└── WorldObject : Object                 Objects/Object.h:453
    ├── Corpse : WorldObject             Objects/Corpse.h:53
    ├── DynamicObject : WorldObject      Objects/DynamicObject.h:37
    └── SpellCaster : WorldObject        Objects/SpellCaster.h:315
        ├── GameObject : SpellCaster     Objects/GameObject.h:41
        └── Unit : SpellCaster           (via SpellCaster; Unit.h)
            ├── Creature : Unit          Objects/Creature.h:80
            │   ├── Pet : Creature        (subtype-flagged, see below)
            │   ├── Totem : Creature
            │   └── TemporarySummon : Creature
            └── Player final : Unit      Objects/Player.h:746
```

Notes from inspected headers:

- `Object` is the universal base: virtual destructor, deleted copy
  (`Object.h:447-448`), `AddToWorld()/RemoveFromWorld()` lifecycle
  (`Object.h:118-133`), and the *type system is a runtime tag*, not the
  C++ type: `m_objectType`/`m_objectTypeId` (`Object.h:420-421`) with
  `IsPlayer()/ToPlayer()/IsCreature()/ToCreature()/ToUnit()/ToGameObject()`
  downcast helpers (`Object.h:378-404`). Rust equivalent is an enum or
  trait-object + `downcast`, but call sites assume cheap unchecked
  casts everywhere.
- `SpellCaster : WorldObject` (`SpellCaster.h:315`) exists solely so
  `Unit` and `GameObject` share cooldown/proc/casting code. In Rust this
  is a trait + shared struct, not a base class — but ~2,387 LOC of
  `SpellCaster.cpp` accesses `this` as both.
- `Creature` subtypes (pet/totem/temp-summon) are **flag-discrimated, not
  type-discriminated**: `m_subtype` with `IsPet()/IsTotem()` and raw
  `reinterpret_cast<Totem*>` (`Creature.h:146-152`). `Player` is `final`
  (`Player.h:746`) — bots do not subclass it; they drive real `Player`
  objects (out of scope, but it means the core cannot assume
  "Player ⇒ human session" — cf. `Map::HaveRealPlayers()` checking
  `IsBot()`, `Map.cpp:1778-1784`).
- `Item` does **not** derive from `WorldObject` (`Item.h:39`): items have
  no position/map; they live inside `Player`/containers/corpses/mail/AH
  and replicate through the owner's update queue. This split is the one
  clean boundary in the hierarchy and maps well to a Rust enum split
  (`WorldEntity` vs `ItemEntity`).

### 2.2 GUID scheme

`ObjectGuid` wraps a raw `uint64` (`ObjectGuid.h:124-239`):
`high(16) | entry(24) | counter(24)` for entry-bearing types
(creature/pet/gameobject/transport), `high(16) | counter(32)` otherwise
(player/item/corpse/dynamic) — constructors at `ObjectGuid.h:129-130`,
`HasEntry()` at `ObjectGuid.h:216-235`, `HighGuid` constants
(`HIGHGUID_UNIT=0xF130`, `HIGHGUID_PLAYER=0x0000`, …) at
`ObjectGuid.h:66-78`. The GUID is *also stored inside the object's own
update-field array* (`OBJECT_FIELD_GUID`, `Object.h:135-137`), so identity
and wire representation are the same memory. `PackedGuid` is the
variable-length wire encoding (`ObjectGuid.h:253-279`).

Allocation: per-type monotonic counters with a freed-GUID reuse queue,
`ObjectGuidGenerator<High>` (`ObjectGuid.h:281-300`), owned by
`ObjectMgr` members `m_StaticCreatureGuids`, `m_CharGuids`, `m_ItemGuids`,
`m_CorpseGuids` (`ObjectMgr.h:1437-1444`), seeded from DB maxima at boot.
Dynamic (runtime-spawned) creature/GO GUIDs come from per-map/per-object
counters distinct from static DB GUIDs (`GenerateStaticCreatureLowGuid`,
`ObjectMgr.h:952-958`).

**Rust implication.** `ObjectGuid` ports trivially as a `#[repr(transparent)]
struct ObjectGuid(u64)` newtype — it is already effectively that. The hard
part is that GUIDs are used as *the* cross-system foreign key everywhere
(DB rows, packets, group member lists `Group.h:187-194`, threat refs,
script targets), so the bit layout is a frozen production contract
(character DBs store raw GUIDs; changing layout = data migration).

### 2.3 Update fields: the real data model

Every `Object` owns a heap array viewed simultaneously as
`int32*/uint32*/float*` via union (`Object.h:424-429`) plus a mirror array
for change detection (`Object.h:431`), indexed by generated enums
(`OBJECT_FIELD_*`, `UNIT_FIELD_*`, `PLAYER_FIELD_*`). All replicated state
(health, flags, display id, faction, …) lives *only* in this array;
typed getters/setters bit-cast sub-fields (`Object.h:178-232,`). Field
layouts are **per-client-build**: 12 header variants
`UpdateFields_1_2_4.h … UpdateFields_1_12_1.h` selected by
`SUPPORTED_CLIENT_BUILD` (`UpdateFields.h:27-45`).

**Rust implication.** This is the single most migration-hostile pattern in
the codebase: untyped indexed storage with union punning, no field-level
types, and ~all gameplay code reading/writing raw indices. A faithful Rust
port has two options, both expensive: (a) keep a `Vec<u32>` + index consts
(unsafe-free but retains C++-style bug surface), or (b) generate typed
structs per build and rewrite every `GetUInt32Value(UNIT_FIELD_*)` call
site (thousands). Option (a) is the realistic first port; the generated
`UpdateFields_*.cpp` flag tables port as data.

## 3. Ownership: raw `new`/`delete`, deferred removal, intrusive refs

Measured in `src/game` (excl. bots): **913** raw `new `/`delete ` sites vs
**472** `unique_ptr`/`shared_ptr` mentions (mostly in newer/threading
code). World objects are invariably heap-allocated with bare `new` at
dozens of call sites and owned by convention, not by type:

- Creatures: `new Creature` in `Map.cpp:3657` (summon), `Creature.cpp:3979`,
  `PoolManager.cpp:475`, `ObjectMgr.cpp:11118`, plus spell-effect spawns.
- GameObjects: factory `GameObject.cpp:107` (`return new GameObject`),
  `SpellEffects.cpp:959,3496,4600,5078,5612`.
- Pets/DynamicObjects: `SpellEffects.cpp:2287,2775,3038,3117,5325`,
  `Player.cpp:15588,18780,20678`. Corpses: `Player.cpp:4769`,
  `Map.cpp:3524`.

Who deletes: the **Map**, via a deferred remove list — never the spawner.
`Map::AddObjectToRemoveList` calls `CleanupsBeforeDelete()` then queues
under lock (`Map.cpp:1720-1727`); `Map::RemoveAllObjectsInRemoveList`
switches on `TypeId` and `Remove(ptr, true)` (which deletes) at a safe
point (`Map.cpp:1729-1776`), driven from `World::Update` via
`sMapMgr.RemoveAllObjectsInRemoveList()` (`World.cpp:2100`). Players are
special: `Map::DeleteFromWorld(Player*)` "player object will deleted at
call" (`Map.h:343`, `Map.cpp:297`), owned by the session/login flow.

Lookup/registry is split three ways, none owning:

1. **Per-map store**: `Map::m_objectsStore` (`InsertObject/EraseObject`
   templates under `shared_timed_mutex`, `Map.h:512-519`) +
   `GetCreature/GetGameObject/GetUnit/GetWorldObject(ObjectGuid)`
   (`Map.h:499-510`). Authoritative for everything on a map.
2. **Grids**: objects are linked into `GridRefManager<T>` intrusive lists
   per cell type-list (`GridDefines.h:63-78`) — spatial index, not owner.
3. **Global holders (non-owning)**: `HashMapHolder<T>` static
   `unordered_map<ObjectGuid,T*>` with its own RW lock
   (`ObjectAccessor.h:44-82`) for `Player`, `Corpse`, `ShipTransport`,
   `MasterPlayer` — the things findable across maps. `ObjectAccessor`
   singleton (`ObjectAccessor.h:84,161`) is the cross-map facade
   (`FindPlayer`, `GetUnit(obj,guid)` at `ObjectAccessor.h:98-107`).

Cross-references between live objects use MaNGOS intrusive
`Reference`/`RefManager` pairs that auto-unlink on destruction:
`MapReference`/`MapRefManager` (players on a map), `GroupReference`,
`FollowerReference`, `HostileReference` (threat, §7). This is the codebase's
answer to dangling pointers: raw `T*` + intrusive unlink + `IsInWorld()`
/`IsDeleted()` flag checks (`Object.h:371,436`) + deferred deletion.

`ObjectMgr` (singleton `sObjectMgr`, `ObjectMgr.h:1594`; class at
`ObjectMgr.h:586`) owns **no live objects** — it owns all *static* data:
creature/GO/item/quest templates, spawn tables, locales, vendors,
trainers, loaded once from MySQL (`LoadCreatureTemplates`,
`LoadGameObjectTemplates`, `LoadQuests`, … `ObjectMgr.h:631-928`) plus the
GUID generators. It is referenced from 140 .cpp files with 1,317
`sObjectMgr.` call sites (measured, excl. bots).

**Rust implication.** Ownership must be *designed*, not transliterated:
the natural Rust shape is `Map` owning entities in slotmaps/arenas keyed
by `ObjectGuid`, with deferred-delete queues preserved (gameplay code
deletes objects mid-iteration pervasively — the remove-list pattern must
survive as a command buffer). Intrusive RefManager pairs become
generational handles or `Weak` refs; every `Unit*` field (victims,
targets, charmers, owners…) is a latent borrow-checker fight. Static
template data (`ObjectMgr`) is the easy part: immutable `HashMap`s behind
`OnceLock`/`Arc`, loaded at boot — a good first Rust crate candidate.

## 4. Map / InstanceMap / Grid / Cell system

### 4.1 Geometry constants (frozen by client + extracted data)

From `GridDefines.h:38-59`: 64×64 grids per map, each 533.333 yd;
16×16 cells per grid (33.33 yd cells); 1024×1024 cells per map;
`MAP_SIZE = 64 × 533.333`. `ComputeGridPair/ComputeCellPair` convert
world coords (`GridDefines.h:165-167`). These numbers are baked into
`.map`/`.mmap`/vmap tile files and the 1.12 client — non-negotiable in
any port.

A `Cell` is a 64-bit packed struct (grid_x/y, cell_x/y, nocreate flag,
`Cell.h:49-88`); iteration is visitor-based over compile-time type lists:
`AllWorldObjectTypes = (Player, Creature, Corpse, Camera)`,
`AllGridObjectTypes = (GameObject, Creature, DynamicObject, Corpse)`
(`GridDefines.h:63-65`), `Grid<Player,…>` / `NGrid<16,…>`
(`GridDefines.h:74-75`). `Cell::VisitGridObjects/VisitWorldObjects/
VisitAllObjects(center, visitor, radius)` (`Cell.h:97-109`) is *the*
spatial query API used by spells, AI, visibility, and chat alike;
`GridNotifiers.h` (1,585 lines) + `GridSearchers` hold ~100 visitor
functors (nearest creature, any hostile unit, message delivery, …).

### 4.2 Map classes and lifecycle

`Map : GridRefManager<NGridType>` (`Map.h:309`) — note the map *is* the
grid container. Subclasses `WorldMap`, `DungeonMap`, `BattleGroundMap`
(`Map.h:965-1008`) differ mainly in enter rules, persistence, and reset.
`MapManager` singleton (`sMapMgr`, `MapManager.h:79,228`) owns all maps
(`i_maps`, deleted in dtor `MapManager.cpp:49-54`), creates instances on
demand (`CreateMap` `MapManager.cpp:107`, `CreateInstance`
`MapManager.h:183`), and runs the grid state machine
(`GRID_STATE_ACTIVE/IDLE/REMOVAL`, `MapManager.cpp:76-99`).

Grids load lazily around players (`Map::LoadGrid`, `ObjectGridLoader`
spawns DB creatures/GOs for the grid) and unload when idle
(`m_gridExpiry`, `Map.h:396-398`). Terrain height/liquids come from
`GridMap`/`TerrainInfo`/`TerrainManager` (`GridMap.h:51,140,208`,
`sTerrainMgr`), ref-counted and shared across all instances of the same
map id — the first piece of **cross-map shared state** (read-only after
load, hence `Arc`-friendly).

Per-map mutable state held directly on `Map`: player list
(`m_mapRefManager`, `Map.h:452-453`), active non-player list,
objects-to-remove set, objects-to-client-update set, relocated-units set,
transports set, scripted-event map (`m_mScriptedEvents`, `Map.h:455-471`),
weather system, `InstanceData* m_data`, dynamic vmap tree
(`m_dynamicTree`), and four thread pools (cells, motion, objects,
visibility). `DungeonMap` adds instance save binding via
`MapPersistentState` (`Map.h:423-424`).

### 4.3 The map update loop and diff timing

Tick chain (all `diff` values are **uint32 milliseconds**, variable —
there is no fixed timestep):

1. `World::Update(diff)` (`World.cpp:1965`) — timers, sessions, then
   `sMapMgr.Update(diff)`; also auctions/mail/groups on slower timers.
2. `MapManager::Update` fans maps out: continents largely on dedicated
   threads, instances on a `ThreadPool` sized by
   `CONFIG_UINT32_MAPUPDATE_INSTANCED_UPDATE_THREADS`
   (`MapManager.cpp:42-46`); interval target
   `CONFIG_UINT32_INTERVAL_MAPUPDATE`.
3. `Map::DoUpdate(maxDiff)` clamps diff (`Map.cpp:921-931`) →
   `Map::Update(t_diff)` (`Map.cpp:933-1060`):
   - `m_dynamicTree.update` (moving GO collision),
   - session packet processing split by class (movement/spells at higher
     frequency: `UpdateSessionsMovementAndSpellsIfNeeded`
     `Map.cpp:875-886`, gated by
     `CONFIG_UINT32_MAPUPDATE_UPDATE_PACKETS_DIFF`),
   - `UpdatePlayers()` (`Map.cpp:888-919`) — gated by
     `…_UPDATE_PLAYERS_DIFF`, skips inactive non-combat players and
     accumulates their skipped time (`AddSkippedUpdateTime`),
   - `UpdateCells()` (`Map.cpp:819-847`) — gated by
     `…_UPDATE_CELLS_DIFF`; marks cells around players + active objects
     and visits them with `ObjectUpdater`, **multithreaded on continents**
     in two checkerboard steps with a safe-distance stride so adjacent
     cells are never updated concurrently
     (`UpdateActiveCellsAsynch/Callback` `Map.cpp:743-792`), synchronous
     elsewhere (`Map.cpp:794-816`); async `MotionMaster` updates are
     farmed to `m_motionThreads` in the same pass (`Map.cpp:836-845`),
   - `SendObjectUpdates()` (§5), `UpdateVisibilityForRelocations()`,
   - a second player pass, corpse cleanup, then a **continent barrier**:
     continents spin updating players until all continents finish or
     `INTERVAL_MAPUPDATE` elapses (`Map.cpp:981-993`),
   - grid state updates, `ScriptsProcess()`, instance script
     `m_data->Update`, weather (`Map.cpp:1008-1022`),
   - adaptive load shedding: visibility/grid-activation distances are
     raised/lowered at runtime based on measured tick time
     (`Map.cpp:1031-1058`).

Per-object effective diff is not the tick diff: each `WorldObject`
carries a `WorldUpdateCounter` and `UpdateHelper` passes
*time-since-this-object-last-updated* (`Object.h:74-100,460-483`), so
creatures in long-inactive cells receive large catch-up diffs. Spells,
regen, and AI all assume this semantic.

**Coupling/shared state.** Maps are the unit of parallelism and are
*mostly* isolated, but not hermetic: `ObjectAccessor` player lookups,
`Group`/`Guild`/chat, transports moving players between maps
(`Map::UpdateSync` `Map.cpp:677-691`), and `sObjectMgr`/`sSpellMgr`
globals all cross map boundaries. Threading inside one map (cell/object/
visibility pools) relies on careful spatial partitioning and
`shared_timed_mutex` stores rather than ownership separation.

**Rust implication.** The per-map-actor shape is actually Rust-friendly
(one `Map` = one task owning its entities, cross-map ops as messages),
and the checkerboard parallel cell update is a ready-made `rayon`
pattern. But the *variable-diff + per-object catch-up clock* semantics
and the multi-rate gates (packets/players/cells at different cadences,
continent barrier) are behavioral contracts that affect spell timing and
must be replicated exactly, not "cleaned up" into a fixed tick.

## 5. Visibility and update-data replication

Replication is pull-based per observing player, in three pieces:

1. **Change marking.** Any `SetUInt32Value`-family write whose value
   differs from the mirror sets bits and enqueues the object in its map's
   `m_objectsToClientUpdate` via `AddToClientUpdateList()`
   (`Object.h:156-162`); `Item`s enqueue on their owner `Player` instead.
2. **Build & send.** `Map::SendObjectUpdates()` (`Map.cpp:2777-2895`)
   drains that set across `m_objectThreads`; each object
   `BuildUpdateData(UpdateDataMapType&)` (player→`UpdateData` map,
   `Object.h:70,158`) decides per-observer which fields that observer may
   see (`_SetUpdateBits` + per-field flags: public/owner/party/…,
   `Object.h:415-417`). `UpdateData` accumulates create/values/movement/
   out-of-range blocks (`UpdateData.h:32-93`) and `Send()` compresses
   (`PacketCompressor`, zlib) into `SMSG_COMPRESSED_UPDATE_OBJECT` when
   large. A timeout (`CONFIG_UINT32_MAP_OBJECTSUPDATE_TIMEOUT`) can
   defer leftovers to the next tick (`Map.cpp:2845-2848`).
3. **Visibility (who exists for whom).** Each `Player` owns a `Camera`
   attached to a viewpoint `WorldObject` (usually itself; farsight/
   possession re-target it) (`Camera.h:35-61`). Relocation queues the
   unit in `m_unitsRelocated`; `Map::UpdateVisibilityForRelocations()`
   (`Map.cpp:2899…`) + `Player::UpdateVisibilityOf` create/destroy
   objects per camera based on distance (`m_visibilityDistance`,
   adaptively tuned §4.3), stealth/invisibility, GM flags, and transport
   state. `Map::UpdateObjectVisibility/UpdateActiveObjectVisibility`
   (`Map.h:428-432`, `Map.cpp:1594-1642`) handle non-player active
   objects. Movement packets flow separately through packet broadcasters
   (`PacketBroadcast/`, `MovementData` batching `UpdateData.h:96-110`).

**Coupling.** Visibility is `Player`-centric to the core:
`BuildCreateUpdateBlockForPlayer`, `DestroyForPlayer`,
`BuildValuesUpdateBlockForPlayer` are on base `Object`
(`Object.h:152-176`) and take `Player* target` for flag filtering;
bots are real `Player`s so they pay/receive identical replication.

**Rust implication.** The wire format (update blocks, field-flag tables,
packed GUIDs, zlib threshold) is dictated by the 1.12.1 client and ports
as pure serialization — very portable. The *observer-set computation*
(cameras, adaptive distance, stealth rules) is entangled with `Player`
state and grid visits; in Rust it becomes a per-map system producing
per-connection outbox buffers, which is a natural fit but a rewrite, not
a transliteration (no virtual `BuildUpdateData` on entities).

## 6. Movement, splines, waypoints

- `MotionMaster` is a **stack** of heap `MovementGenerator*` per `Unit`
  (`class MotionMaster : std::stack<MovementGenerator*>`,
  `MotionMaster.h:82-179`); `Mutate()` pushes, expire/clean pops with
  deferred cleanup during update (`MMCF_UPDATE` dance,
  `MotionMaster.h:61-66,110-123`). 20 generator types
  (`MotionMaster.h:36-59`): Idle, Random, Waypoint, Cyclic (DB-settable,
  ≤ `MAX_DB_MOTION_TYPE=4`), Confused, Chase/Follow (`Targeted…`), Home,
  Flight (taxi), Point, Fleeing/Feared, Distract, Assistance, Charge,
  Patrol, Distancing, Effect. Generators own a `PathInfo` and drive a
  `Movement::MoveSpline`.
- Spline layer (`Movement/spline/`, ~2k LOC): `MoveSpline`
  (`MoveSpline.h:40`), `MoveSplineInit` builder (`MoveSplineInit.h:32`),
  Catmull-Rom evaluation (`spline.h/.impl.h`), and
  `packet_builder` serializing `SMSG_MONSTER_MOVE` variants. The spline
  is computed server-side but *executed by clients* for creatures, so
  its math must stay bit-compatible in behavior (clients extrapolate).
- Player movement is the inverse: client-authoritative position reports
  (`MovementInfo::Read`, `Object.cpp:61-103`) validated by anticheat,
  with server-initiated corrections via pending-change records
  (`PlayerMovementPendingChange`, `Unit.h:61-79`).
- Waypoints: `WaypointManager` singleton (`sWaypointMgr`,
  `WaypointManager.h:182`) loads `creature_movement*` DB tables into
  shared immutable paths referenced by guid/entry;
  `WaypointMovementGenerator` walks them and fires DB movement scripts
  at points.

**Coupling/shared state.** `MotionMaster` holds raw `Unit* m_owner`
(`MotionMaster.h:176`) and generators hold raw target refs
(`FollowerReference` for follow/chase targets so target death unlinks).
Async path: units flagged `NeedsAsyncUpdate` are moved on
`m_motionThreads` (`Map.cpp:836-845`) — generators must therefore be
tolerant of running off the map's main thread, a known fragility seam.

**Rust implication.** A generator stack is an enum stack in Rust —
cleaner than C++, but generators call back into AI
(`MovementInform`), spells (charge), and scripts (waypoint scripts)
mid-update, i.e. re-entrant `&mut World` access. Expect a command-buffer
or split-phase design (compute spline → apply). Spline math + packet
builder port nearly 1:1 as pure functions — good early crate with golden
packet tests.

## 7. Pathfinding (mmaps/Detour) and collision (vmaps)

Three independent spatial data layers, all from **offline-extracted
binary files** (gitignored; generated from client data):

| Layer | Files | Loader/API | Used for |
|---|---|---|---|
| Terrain `.map` | height grid + liquid | `GridMap`/`TerrainInfo` (`GridMap.h:51,140`) | ground Z, water, area/zone |
| mmaps `.mmap`/`.mmtile` | Detour navmesh tiles | `MoveMap` (`MoveMap.h:44-90`), loads `mmaps/%03i.mmap` + tiles (`MoveMap.cpp:73-168`), version-checked headers | creature pathfinding |
| vmaps `.vmtree`/… | BIH of WMO/M2 models | `VMapManager2 : IVMapManager` (`VMapManager2.h:64`), `isInLineOfSight`, `getHeight` (`IVMapManager.h:65-66`) | LoS, spell/projectile collision, height under models |

`PathFinder` (alias `PathInfo`, `PathFinder.h:59-166`) wraps Detour:
per-query `dtNavMeshQuery` (explicitly *not* thread-safe — one per
thread via `NavMeshQuerySet` map keyed by `thread::id`,
`MoveMap.h:44,61-89`), poly corridor → smoothed point path
(`findSmoothPath`, `PathFinder.h:161-163`), max 256 polys/points
(`PathFinder.h:39-40`), with fallback modes when tiles are missing or
the unit flies/swims (`PathType` flags `PathFinder.h:45-57`;
`BuildShortcut`, `BuildPathWithoutMMaps`, `PathFinder.h:153-155`).
Dynamic obstacles (doors, elevators, transports) are handled outside
Detour: `Map::m_dynamicTree` (`DynamicTree` in `vmap/`) updated per tick
(`Map.cpp:937`), plus transport-local navmeshes.

The Detour/Recast copy is vendored with a local patch
(`vmangos/dep/recastnavigation/{Detour,Recast,recastnavigation.diff}`).

**Rust implication.** Do **not** rewrite Detour: bind the vendored C++
via FFI (or a `detour` crate) and port only `PathInfo`'s policy layer
(filters, slope exclusion, fallbacks, LoS path cutting
`CutPathWithDynamicLoS`). mmap/vmap *file formats* are frozen inputs —
loaders must parse identical bytes. vmap BIH raycasts are hot,
pure-geometry, and data-parallel: a credible standalone Rust crate, but
only worth it after an FFI baseline proves behavioral parity
(heights/LoS differences are gameplay-visible).

## 8. Spells, auras, combat, threat

### 8.1 Spell pipeline (33,357 LOC in `Spells/` + proc handler)

- `SpellMgr` singleton (`sSpellMgr`, `SpellMgr.h:256,710`) loads/corrects
  all spell data: DBC `SpellEntry` records plus ~15 DB override tables
  (chains, proc events, threat, targets, groups/stack rules, elixirs,
  pet auras… `SpellMgr.h:631-666`). Read-only after boot → `Arc` data.
- A cast is a heap `Spell` object (`Spell.h`) created by `SpellCaster`,
  running prepare → cast-time → channel → hit/miss rolls per target →
  `HandleEffects` dispatching to ~180 `Effect*` methods by effect-type
  enum (`Spell.h:229…`, implemented in `SpellEffects.cpp` 5,805 LOC).
  Effects **directly mutate the world**: spawn creatures/GOs/dynamic
  objects with bare `new` (§3), teleport, create items, start scripts.
- Auras: `SpellAuraHolder` + per-effect `Aura` (`SpellAuras.cpp`
  8,261 LOC) with a static handler table indexed by aura type
  (`Aura::` apply/remove/periodic), diminishing returns
  (`DiminishingReturn`, `Unit.h:89-114`), and stack/group rules from
  `SpellMgr`. Procs dispatch through a function-pointer table
  `pAuraProcHandler AuraProcHandler[TOTAL_AURAS]` of **member-function
  pointers on `Unit`** (`Unit.h:188-189`), implemented in
  `UnitAuraProcHandler.cpp` (2,000 LOC).
- Scriptability hooks are woven through: `SpellScript`/`AuraScript`
  virtual override points (`ScriptMgr.h:180-214`) fetched per spell from
  `sScriptMgr` (§9), plus dummy-effect escape hatches
  (`pEffectDummyCreature/GO`, `pEffectAuraDummy`, `ScriptMgr.h:239-241`).

### 8.2 Combat & threat

Melee/ranged resolution, damage calculation (`CalcDamageInfo`,
`Unit.h:132-151`), absorb/resist splitting, rage, weapon skill, and
stat recomputation (`StatSystem.cpp`) live on `Unit` (11,073 LOC) —
combat is not a separable module, it is methods on the entity.
Threat is the exception — a genuinely separable component:
`ThreatManager` per creature (`ThreatManager.h:193-240`) owning sorted
`ThreatContainer`s of intrusive `HostileReference`s bidirectionally
linked to each attacker's `HostileRefManager`
(`HostileRefManager.h:35`), with taunt/offline-list handling;
`getHostileTarget()` victim selection runs the 110%/130% melee/ranged
rules. Total `Threat/` = 1,042 LOC.

**Coupling.** Spells are the most coupled subsystem in the server:
`Spell` holds caster/target `Unit*`/`GameObject*`/`Item*`, reads
`ObjectMgr`/`SpellMgr` globals, writes update fields, moves units
(knockbacks/charges via `MotionMaster`), and triggers AI combat entry.
`Unit` ↔ `Spell` ↔ `Aura` is a three-way circular dependency.

**Rust implication.** Threat ports first (self-contained, testable,
deterministic). The spell/auras cluster ports last and together; the
handler-table patterns (effect/aura/proc dispatch by integer enum from
DBC data) map to Rust `match` over generated enums — the *data* (which
spell has which effects) stays in DBC/DB, so behavioral parity is
checkable spell-by-spell. Member-function-pointer proc table becomes a
plain function table over `&mut Unit`-equivalent state.

## 9. Creature AI and the script system (three layers)

Content behavior comes in three stacked systems; all three must survive
any migration because world-DB data references them by name/id.

### 9.1 C++ core AI (`AI/`, 7,592 LOC)

`CreatureAI` abstract base with ~40 virtual event hooks
(`UpdateAI(diff)`, `EnterCombat`, `JustDied`, `MoveInLineOfSight`,
`MovementInform`, `SpellHit`, … `CreatureAI.h:50-222`), holding
`Creature* const m_creature` (`CreatureAI.h:208`). Each `Creature` owns
`CreatureAI* m_AI` (`Creature.h:82`). Selection is a scored factory
registry: `CreatureAIFactory<T>`/`SelectableAI::Permit()` with
permission tiers (`CreatureAI.h:224-249`), registered in
`CreatureAIRegistry.cpp` (Null, Basic, Critter, Guard, Pet, Totem,
EventAI variants). `BasicAI` implements generic combat (victim chase,
spell-list casting via `UpdateSpellsList` `CreatureAI.h:197-199`);
`ScriptedAI`/`ScriptedEscortAI`/`ScriptedFollowerAI` are base classes
for content scripts; `PlayerAI` handles charmed/possessed players —
note this is core, unrelated to `PlayerBots`.

### 9.2 C++ content scripts (`src/scripts/`, 107,413 LOC, 223 .cpp)

| Subtree | .cpp | LOC |
|---|---|---|
| `eastern_kingdoms/` | 132 | 53,989 |
| `kalimdor/` | 59 | 34,626 |
| `world/` | 14 | 8,521 |
| `battlegrounds/` | 2 | 5,209 |
| `spells/` | 11 | 3,239 |
| `custom/` | 2 | 1,286 |

Registration is manual C linkage: each file exports `AddSC_*()` which
constructs `Script` objects and calls `RegisterSelf()`; 441 `AddSC_`
declarations/calls are listed in `ScriptLoader.cpp`. A `Script` is a
POD of 26 function pointers + 5 factories (`GetAI`, `GOGetAI`,
`GetInstanceData`, `GetSpellScript`, `GetAuraScript`,
`ScriptMgr.h:216-251`), bound to a **name string** that world-DB rows
(`creature_template.ScriptName`, `gameobject_template`, `instance_template`,
`areatrigger_scripts`, gossip/quest tables) reference. `ScriptMgr`
(scripts are compiled **into** the `mangosd` binary — no dynamic
loading — with a `ScriptLoader_noscripts.cpp` stub variant for
script-less builds) dispatches `OnGossipHello`, `OnQuestAccept`,
`OnEffectDummy`, `GetCreatureAI`, … (`ScriptMgr.h:331-353`).
`InstanceData` subclasses per dungeon hold encounter state machines,
owned by the `Map` (`m_data`, updated `Map.cpp:1019-1020`).

### 9.3 DB scripting (data-driven, no recompilation)

- **EventAI**: `creature_ai_scripts` rows (event/action/params) run by
  `CreatureEventAI : BasicAI` (`CreatureEventAI.h:296`), loaded by
  `CreatureEventAIMgr` (`sEventAIMgr`). The default brain for most
  world creatures.
- **DB scripts**: ~10 `*_scripts` tables (generic, gossip, quest
  start/end, spell, movement, event, areatrigger…) loaded by
  `ScriptMgr::Load*Scripts()` (`ScriptMgr.h:259-269`) into
  `ScriptMapMap`s, executed as delayed command queues per map:
  `Map::ScriptsStart/ScriptCommandStart/ScriptsProcess`
  (`Map.h:473-482`) interpreting ~100 opcodes in
  `Maps/ScriptCommands.cpp` (2,526 LOC) — talk, emote, cast, summon,
  teleport, set flags, start other scripts.
- **Conditions**: `conditions` table predicates (`Conditions.cpp`
  1,367 LOC) gating gossip/quests/loot/spells/areatriggers.

**Coupling/shared state.** Scripts receive raw `Player*`/`Creature*`/
`GameObject*`/`Map*` and freely call any core API — there is no sandbox
or narrow interface; script state lives in AI instance members and
`InstanceData` maps. Script *names* and DB script *ids/opcodes* are
production DB contracts.

**Rust implication.** This is the largest single migration risk after
`Player`/`Unit`: 107k LOC of content scripts cannot be hand-ported
file-by-file on any sane schedule. Options in order of realism:
(1) keep C++ scripts behind a stable FFI "script API" surface (the 31
`Script` slots + `CreatureAI` hooks define it) while the core moves;
(2) transpile mechanically (scripts are stylized: state machines on
timers + `DoCast`/summon calls); (3) move EventAI/DB-script
interpreters early — they are *interpreters over DB data*, so porting
the ~4k-LOC engines preserves thousands of DB-authored behaviors for
free. The DB-script + EventAI + Conditions engines are therefore high
leverage early ports; C++ boss scripts are the long tail.

## 10. Quests, loot, groups, guilds, auction, chat (inventory)

| System | Size | Owning/global state | Cross-map? | Coupling notes |
|---|---|---|---|---|
| Quests | `QuestDef` 638 LOC + logic inside `Player.cpp`, `ObjectMgr` quest tables, `QuestHandler` | Templates in `sObjectMgr`; per-player state on `Player` (quest log in update fields) | Player-bound; objectives count events on any map | Creature/GO quest relations in `ObjectMgr.h:832-898`; scripts hook accept/reward (§9.2) |
| Loot | `LootMgr` 1,995 LOC | Loot templates in `sObjectMgr`-loaded stores; live `Loot` on corpse/GO | No (map-local rolls) | Reference-loot recursion, group loot rules call into `Group`; `CreatureAI::FillLoot` script hook (`CreatureAI.h:173`) |
| Groups (party/raid) | `Group/` 4,017 LOC (`Group.cpp` 2,485) | `ObjectMgr::GroupMap` global; `Group` holds member **GUIDs**, resolves via `ObjectAccessor` (`Group.h:184-194`) | **Yes — fully cross-map** (members on different maps/instances) | Loot method, BG/raid subgroups, instance binding; updated from `World::Update` (`World.cpp` WUPDATE_GROUPS) |
| Guilds | `Guild/` 1,971 LOC | `GuildMgr` singleton `m_GuildMap: unordered_map<uint32,Guild*>` (`GuildMgr.h:34,81`) | **Yes — global**, DB-persisted | Ranks/motd/bank-less vanilla model; `GuildBotMgr` (bot scope) layers on top without core changes |
| Auction House | `AuctionHouse/` 1,929 LOC | `AuctionHouseMgr` singleton owning `Item*` map (`AuctionHouseMgr.h:151-159`); three house objects (A/H/neutral) | **Yes — global**, timer-driven from `World::Update` WUPDATE_AUCTIONS | Owns real `Item` objects while listed — an ownership exception (§3); expiry mails items back |
| Mail | `Mail/` 1,081 LOC + `MassMailMgr` | DB-backed; items move Player↔mail↔AH | Yes (global service) | Item ownership transfer point |
| Chat/channels | `Chat/` 7,981 LOC (`Chat.cpp` 3,479 incl. command table) | `ChannelMgr` per faction; global channels span maps | **Yes** (world/guild/raid channels) | `AbstractPlayer`/`MasterPlayer` abstraction lets chat work for offline/master-node players (`ObjectAccessor.h:109-117`) |
| Social/friends/ignore | `SocialMgr` + `PlayerSocial` | Global singleton, DB-backed | Yes | Master-server only access guards (`Player.h:763-772`) |
| Battlegrounds | 8,667 LOC | `BattleGroundMgr` singleton queues + per-BG `BattleGroundMap` | Yes (queues global, play instanced) | Deep `Group`/`Player` coupling; own script hooks |
| Transports | 1,262 LOC | `GenericTransport` GOs spanning maps; `TransportMgr` | **Yes — move players between maps** (`Map::UpdateSync`) | Passengers stored per transport; the main legitimate cross-map entity mover |
| Game events / world states | `GameEventMgr`, `WorldStates.h`, `OutdoorPvP` 1,880 | Global timers from `World::Update` WUPDATE_EVENTS | Yes | Spawn/despawn hooks into `ObjectMgr` pools |
| Pools (spawn pools) | `Maps/Pool/` | `PoolManager` global templates, per-map spawned subsets | Templates global, instances per-map | `new Creature` site (`PoolManager.cpp:475`) |
| GM commands | `Commands/` 23,591 LOC | Stateless handlers over `ChatHandler` | Via target player/session | Not simulation, but calls everything; port last / FFI |

Pattern: **everything social/economic is a global singleton service
operating on GUIDs** (30+ `s*Mgr` singletons measured across
`src/game/**/*.h`), while everything spatial is per-map. The singleton
services call `ObjectAccessor::FindPlayer(guid)` to materialize a
`Player*` on whatever map it currently lives — the standard cross-map
access idiom a Rust design must replace with messages or a global
player directory returning handles, since a borrowed `&mut Player`
cannot be held across map tasks.

**Rust implication.** Guild/Auction/Mail/Social/Group-registry are
CRUD-over-MySQL + in-memory indexes with little per-tick work: they are
the most conventionally portable systems here (async services over the
same schemas — note production rule: schemas are append-only, CHAR DB
formats locked, per `CLAUDE.md`). Their difficulty is not algorithms
but *item/player ownership hand-offs* (AH holds `Item*`, mail holds
`Item*`, trade swaps `Item*`) — in Rust these must become explicit
ownership transfers of an `Item` value or GUID-keyed arena entries.

## 11. Cross-cutting summary for the migration guide

**What the seam topology looks like.** The core is a single circular
cluster — `Object/ObjectMgr/Map/Grid/Player/Unit/Spell/Aura/AI/Script`
— glued by raw pointers, runtime type tags, an untyped u32 field array,
and 30+ global singletons (1,317 `sObjectMgr.` sites alone). There is
no internal API boundary to FFI across inside this cluster; credible
FFI seams are at its *edges*:

- **In**: extracted data files (.map/.mmap/vmap/DBC — frozen bytes),
  MySQL schemas (append-only in production), network packets (frozen
  1.12.1 wire format), DB script/EventAI/condition tables (data).
- **Out**: update/movement packets per session.
- **Side**: Detour (vendored C++, keep via FFI), zlib compression.

**Difficulty ranking (evidence-based, hardest first):**

1. `Player` (25,008 LOC .cpp+.h) + `Unit` (12,608) + update-field model
   — god-objects; every subsystem calls them; untyped field storage.
2. `Spell`/`SpellAuras`/`SpellEffects`/procs (24,479 LOC) — circular
   with `Unit`, spawns objects, script hooks throughout.
3. C++ content scripts (107,413 LOC) — volume, not complexity; needs
   FFI-retention or transpilation strategy, names are DB contracts.
4. `Map` update/visibility/threading (variable diffs, catch-up clocks,
   checkerboard parallelism, adaptive distances) — semantics are
   gameplay-visible and must be replicated, not redesigned casually.
5. Movement/PathFinder policy (spline client-execution contract,
   generator re-entrancy).
6. Social/economic singletons (portable shape; ownership hand-offs).
7. Static data loading (`ObjectMgr` templates, `SpellMgr`, waypoints),
   GUID type, spline math, packet serialization, threat, DB-script/
   EventAI/condition interpreters — bounded, testable, good first ports.

**Invariants any Rust design must preserve** (from evidence above):
GUID bit layout (§2.2); grid/cell geometry (§4.1); variable-diff +
per-object catch-up timing (§4.3); deferred deletion (§3); update-field
wire semantics per client build (§2.3, §5); spline shapes clients
execute (§6); mmap/vmap/terrain file formats (§7); script names, DB
script opcodes, EventAI semantics (§9); character/inventory/guild DB
formats (production append-only rule).

## 12. Verification & gaps

Verified by direct reads: hierarchy headers, `ObjectGuid.h`, grid/cell
constants, `Map::Update/RemoveAllObjectsInRemoveList/SendObjectUpdates`
bodies, `MotionMaster.h`, `PathFinder.h`, `CreatureAI.h`, `ScriptMgr.h`,
`ThreatManager.h`, `UpdateData.h`, `ObjectAccessor.h`, `World::Update`
body; counts via `wc`/`grep` as cited. Not inspected in depth (would
need follow-up passes): `Spell.cpp`/`SpellAuras.cpp` internal flows
beyond headers/dispatch shape, `Battlegrounds` internals, `Anticheat`
movement validation, `Server/` session/socket threading (sibling
scope), exact config defaults for the map-update cadence gates
(`mangosd.conf` values not read — credential-bearing file, out of
bounds for this read-only pass), and DBC loading details in
`src/shared`/`src/framework` (sibling scope).

