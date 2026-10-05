# Ownership deep-dive — Object / Unit / Player / Creature hierarchies

Bead: `as-pnw.5` (child of the `as-pnw` migration-mapping epic).
Method: static analysis of `HEAD` (`origin/main`, post-`1490d74`) only.
Grounding inputs: [subsystems inventory](subsystems.md) (`as-pnw.1`) and
[threading map](threading.md) (`as-pnw.4`). Nothing here was verified with a
running server; dynamic follow-ups are listed in §10.

Headline: entities are **heap-allocated by raw `new` at ~30 call sites, owned
either by their `Map` (grid objects) or by the logout chain (players), with a
**deferred-deletion protocol** (`m_deleted` + `Map::m_objectsToRemove`,
drained on the world tick) papering over a dense graph of **non-owning raw
pointers and intrusive linked lists**. The only sound Rust target is a
**single-owner arena keyed by stable handles** — the current pointer graph
has reference cycles and `delete this` that neither the borrow checker nor
`Rc<RefCell>` can express. §8 names the leaf APIs that cannot survive that
transition.

## 1. Class hierarchy (all in `vmangos/src/game/Objects/`)

```
Object (Object.h:111)
├── Item (Item.h:39) ── Bag (Bag.h:33)
└── WorldObject (Object.h:453)
    ├── DynamicObject (DynamicObject.h:37)
    ├── Corpse (Corpse.h:53)
    └── SpellCaster (SpellCaster.h:315)
        ├── Unit (Unit.h:321)
        │   ├── Player final (Player.h:746)
        │   └── Creature (Creature.h:80)
        │       ├── TemporarySummon (TemporarySummon.h:27)
        │       │   └── TemporarySummonWaypoint (:52)
        │       ├── Pet (Pet.h:142)
        │       └── Totem (Totem.h:33)
        └── GameObject (GameObject.h:41)
```

Notes:

- `Player` is `final`; nothing derives from it. `Creature` is the extension
  point (`Pet`, `Totem`, `TemporarySummon`).
- `SpellCaster` is the shared base of `Unit` and `GameObject` — both can cast.
  `Item` is a bare `Object` (never in-world, never in a grid).
- `CreatureAI* m_AI` (`Creature.h:82`) plus `m_AI_locked` (`:648`) guard
  re-entrant AI replacement during updates.

## 2. Allocation: raw `new`, zero smart pointers

Entities are created with plain `new` and stored as raw pointers. No
`unique_ptr`/`shared_ptr` appears anywhere in the entity lifecycle. Sample
allocation sites (all `*.cpp`):

| Site | What |
|------|------|
| `Maps/Map.cpp:3657` | `new Creature()` for map spawns |
| `Maps/Pool/PoolManager.cpp:475` | `new Creature` for pooled spawns |
| `ObjectMgr.cpp:11118,11161` | `new Creature` when loading spawn data |
| `Objects/Player.cpp:4769` | `new Corpse(...)` on player death |
| `Objects/Player.cpp:15588,20678` | `new Pet` (tame/charm paths) |
| `Objects/Player.cpp:17417`, `Objects/Object.cpp:2338,2383` | `new TemporarySummon` |
| `Objects/GameObject.cpp:107,1219` | `new GameObject` (factory + linked GO) |
| `Spells/SpellEffects.cpp` (7 sites: 959, 2287, 2775, 3038, 3117, 3496, 4600, 4862, 5078, 5325, 5612) | `new GameObject` / `new Pet` / `new Totem` from spell effects |
| `Objects/SpellCaster.cpp:1964-2099` (4 sites) | `new Spell` per cast, for units and game objects |
| `PlayerBots/PlayerBotAI.cpp:110` | `new Player(sess)` for bot characters |

Ownership therefore lives entirely in **convention + the two deletion
funnels** below — there is no type-level owner.

## 3. The two deletion funnels

### 3a. Grid objects are owned by their `Map`

`Map::Remove<T>(obj, remove)` (`Maps/Map.cpp:1252-1294`): unlinks the object
from its grid cell, calls `CleanupsBeforeDelete()` when `remove == true`, and
ends with **`delete obj;`** (`Map.cpp:1292`). This covers `Creature`,
`GameObject`, `DynamicObject`, and `Corpse` (explicit template
instantiations at `Map.cpp:2026-2029`). Transports go through sibling
overloads that also `delete` (`Map.cpp:1327`).

Consequence: a grid object's lifetime is exactly "linked into a loaded grid".
`Map` is the sole owner; everything else holding the pointer is borrowing.

### 3b. Players are owned by the logout chain, never by the remove list

`WorldSession::LogoutPlayer` (`Server/WorldSession.cpp:583`) saves the
character, then calls `_map->Remove(_player, true)` with the comment
`SetPlayer(nullptr); // deleted in Remove/DeleteFromWorld call`.
`Map::Remove(Player*, bool)` (`Maps/Map.cpp:1183-1249`) unlinks the map ref,
grid, visibility, and broadcaster listeners, then calls
`Map::DeleteFromWorld(player)` (`Map.h:343`: "player object will deleted at
call"), which does:

```cpp
sObjectAccessor.RemoveObject(player);   // drop the global index entry
delete player;                          // Map.cpp:297-300
```

`RemoveAllObjectsInRemoveList` has **no `TYPEID_PLAYER` case** — a player in
the remove list hits `default:` and is only logged
(`Map.cpp:1771-1773`). Players also delete owned sub-objects in `~Player`
(`Objects/Player.cpp:287-309`): all `m_items[]`, `PlayerTalkClass`,
instance binds, and bot AI (`RemoveAI()`). `MasterPlayer` (the offline/chat
abstraction, `Chat/MasterPlayer.h`) is likewise session-owned and
`delete`d at logout (`WorldSession.cpp` tail: `delete m_masterPlayer`).

## 4. Deferred deletion: `m_deleted` + `m_objectsToRemove`

Because updates iterate grids while effects (spells, scripts, pool
despawns) can kill objects mid-iteration, deletion is deferred:

1. `WorldObject::AddObjectToRemoveList()` (`Objects/Object.cpp:2248-2255`):
   no-op if `m_deleted` is already set (idempotent re-kill), else
   `GetMap()->AddObjectToRemoveList(this)` and set `m_deleted = true`.
2. `Map::AddObjectToRemoveList` (`Maps/Map.cpp:1720-1727`): asserts the
   object belongs to this map/instance, runs
   `obj->CleanupsBeforeDelete()` **eagerly**, then inserts into
   `m_objectsToRemove` under `m_objectsToRemoveLock`.
3. `Map::RemoveAllObjectsInRemoveList` (`Maps/Map.cpp:1729-1776`): drains the
   set on the **world tick** (called from `Map::Update` paths at
   `Map.cpp:1516,1522,1529` and via `MapManager` at `MapManager.cpp:410`),
   type-switching each entry into the matching `Remove(..., true)`.
4. Escape hatch: `OBJECT_DELAYED_ADD_TO_REMOVE_LIST` defers even the
   list-insertion to `Object::ExecuteDelayedActions`
   (`Objects/Object.cpp:1395-1400`).

Dozens of call sites use this (`Creature.cpp:3958`, `GameObject.cpp:702,839`,
`Pet.cpp:1114`, `TemporarySummon.cpp:258`, `Totem.cpp:147`,
`Corpse.cpp:131`, `DynamicObject.cpp:223`, …). `GameObject.cpp:698` shows the
discipline this demands: *"the object is deleted. Do not use any class
method / attribute!"*

## 5. `CleanupsBeforeDelete`: the manual borrow checker

The virtual chain (`Object.h:658` — *"remove or simplify at least cross
referenced links"*) exists because **cross-references to already-deleted
units are the norm, not the exception**:

- `WorldObject::CleanupsBeforeDelete` (`Object.cpp:1434-1440`):
  `RemoveFromWorld()` + transport-passenger unlink.
- `Unit::CleanupsBeforeDelete` (`Objects/Unit.cpp:8400-8420`): interrupts
  spells, kills events (`KillAllEvents(false)` — non-deletable in-flight
  casts are reaped later by the remove-list drain), `CombatStop()`,
  threat-list teardown (`deleteReferences()` vs offline-marking for
  players), `RemoveAllAuras(AURA_REMOVE_BY_DELETE)`.
- `Creature` (`Creature.cpp:207` + `~Creature` at `:205-213`): chain call,
  then **`delete m_AI`**.
- `GameObject::CleanupsBeforeDelete` (`GameObject.cpp:822-829`): same
  event-kill/reap pattern for in-flight casts.
- `Player::CleanupsBeforeDelete` (`Player.cpp:313-321`): trade/duel
  teardown, then the `Unit` chain.
- `TemporarySummon::CleanupsBeforeDelete` (`TemporarySummon.cpp:280-284`):
  unsummon bookkeeping, then the `Creature` chain.

`Unit.cpp:8405` and `GameObject.cpp:827` carry the invariant in comments:
things that cannot be deleted *now* "will [be] deleted at call in
`Map::RemoveAllObjectsInRemoveList`". The engine cannot prove liveness, so
every teardown is written defensively.

## 6. Non-owning indexes and intrusive lists (borrowers, not owners)

- **Global registries own nothing.** `HashMapHolder<T>`
  (`ObjectAccessor.h:44-82`) maps `ObjectGuid → T*` under a
  `shared_timed_mutex`; `Insert`/`Remove` only index. Only `Player`,
  `Corpse`, `ShipTransport`, and `MasterPlayer` are indexed at all —
  creatures and game objects are reachable **only** through their map/grid.
  `ObjectAccessor::GetUnit/GetCorpseInMap/FindPlayer*` return bare pointers
  with no lifetime guarantee beyond the caller's tick.
- **Grids store raw pointers.** `NGridType`/`GridType` type containers
  (`framework/GameSystem/Grid.h`, `TypeContainer.h`) hold what the `Map`
  owns; iteration during `Map::Update` is exactly what §4's deferral
  protects.
- **Intrusive reference lists.** `RefManager<TO,FROM>`
  (`framework/Utilities/LinkedReference/RefManager.h:30-41`) is a linked
  list of `Reference<TO,FROM>` nodes. Live instances on `Unit`:
  `m_threatManager` (`Unit.h:907`), `m_hostileRefManager` (`:908`),
  `m_followingRefManager` (`:1174`). Reference nodes are heap objects owned
  by the manager lists and torn down by `deleteReferences()`
  (`Threat/HostileRefManager.cpp:30,118-151`); every gameplay link
  (threat, charm, follow, group) is a raw back-pointer through these
  nodes.
- **Corpse duality.** Player corpses live in *both* the grid and
  `ObjectAccessor::i_player2corpse` (`ObjectAccessor.h:147`); bones are
  grid-only (`Map.cpp:3524`). `Map::Remove` notes resurrectable corpses and
  pets are "removed from global lists in own destructor" (`Map.cpp:1291`)
  — destructor-order coupling across two indexes.

## 7. Component ownership inside `Unit`

By-value or owned-pointer members that a Rust port must re-home as
components or resources:

| Member | Declared | Ownership |
|--------|----------|-----------|
| `MotionMaster m_motionMaster` | `Unit.h:1326` | By value; **owns** a `std::stack<MovementGenerator*>` (`MotionMaster.h:82`); `~MotionMaster` `delete`s every non-static generator (`MotionMaster.cpp:150-162`) |
| `SpellAuraHolderMap m_spellAuraHolders` | `Unit.h:632` | Multimap of raw `SpellAuraHolder*`; `Unit` deletes holders on removal (`Unit.cpp:3327-3432`, `:4020`) |
| `ThreatManager m_threatManager` | `Unit.h:907` | By value; non-owning view over others' `HostileReference` nodes |
| `HostileRefManager m_hostileRefManager` | `Unit.h:908` | By value; owns its reference-node list |
| `Item* m_items[PLAYER_SLOTS_COUNT]` | `Player.h:863` | `Player`-owned; `~Player` deletes every slot |
| `Item* m_bagslot[MAX_BAG_SIZE]` | `Bag.h:69` | `Bag`-owned item pointers |
| `CreatureAI* m_AI` | `Creature.h:82` | `Creature`-owned; `delete`d in `~Creature`; `(re)created` by `FactorySelector::selectAI` (`Creature.cpp:1219`) |
| `Player* me` (bot AI) | `AI/PlayerAI.h:32-52` | `PlayerAI` holds a **raw, non-owning** pointer to its player; `SetPlayer` can rebind it |
| In-flight `Spell*` | `SpellCaster.cpp:1964-2099` | `new` per cast; the spell **`delete this`** on completion (`Spells/Spell.cpp:8392`) |

## 8. Required Rust ownership model + doomed leaf APIs

**Required model: single-owner arena with stable handles (generational
indices or GUID-keyed slab).** One owner per entity class — `Map` arenas
for grid objects, a session/character store for players/items — with all
cross-entity links (threat, charm, follow, target, owner, mover) as
**handles resolved through the arena**, never as references or raw
pointers. Intrusive `Reference<>` lists become edge tables; the
`m_deleted` + remove-set protocol becomes an explicit despawn queue drained
by the tick schedule; the ~40 `MaNGOS::Singleton` globals (§9) become
resources passed to systems. `Rc<RefCell>` is explicitly rejected: the
threat/follow/owner cycles (§6) would leak, and the multithreaded tick
(threading map §1: world thread + map thread pools + DB delay threads)
needs `Send`-safe handles, not shared mutable aliases.

Leaf APIs that **cannot survive** that model and must be redesigned, not
transliterated (all paths `vmangos/src/game/` unless noted):

1. `HashMapHolder<T*>` raw-pointer maps + `ObjectAccessor::Find*/Get*`
   returning bare pointers (`ObjectAccessor.h:44-161`) → arena queries
   returning handles/`Option`.
2. `Map::Remove(T*, bool)` + `delete obj` and `Map::DeleteFromWorld`
   (`Maps/Map.cpp:1252-1294,297-300`) → despawn systems on the arena.
3. `WorldObject::AddObjectToRemoveList` / `m_deleted` / `m_objectsToRemove` +
   `RemoveAllObjectsInRemoveList` (`Objects/Object.cpp:2248`,
   `Maps/Map.cpp:1720-1776`) → explicit despawn queue (same tick semantics).
4. The `CleanupsBeforeDelete` virtual chain (`Object.h:658`,
   `Unit.cpp:8400`, `Creature.cpp:207`, `GameObject.cpp:822`,
   `Player.cpp:313`, `TemporarySummon.cpp:280`) → per-component teardown
   systems; destructor-order coupling (e.g. corpse §6) must be made explicit.
5. `RefManager<TO,FROM>` / `Reference<TO,FROM>` intrusive lists
   (`framework/Utilities/LinkedReference/`) → edge tables keyed by handles.
6. `Creature::m_AI` raw + `FactorySelector::selectAI` + `SetAI`
   (`Objects/Creature.h:82`, `Creature.cpp:1219`) → behavior components or boxed strategies owned by the arena entry.
7. `PlayerAI::me` raw `Player*` (`AI/PlayerAI.h:32-52`, incl. `SetPlayer`
   rebinding) → `PlayerHandle`; this is the highest-risk bot-touching
   surface (bot AI ticks every update for every bot).
8. `Spell` `delete this` (`Spells/Spell.cpp:8392`) + `new Spell` per cast
   (`Objects/SpellCaster.cpp:1964-2099`) → cast intents/events; no
   self-deleting objects.
9. `MotionMaster`'s owning `stack<MovementGenerator*>`
   (`Movement/MotionMaster.h:82`, `.cpp:150-162`) → movement-intent
   component driven by the movement system.
10. `ThreatManager` / `HostileRefManager` raw graph (`Objects/Unit.h:907-908`,
    `Threat/`) → threat table system.
11. `Unit::m_spellAuraHolders` raw multimap (`Objects/Unit.h:632`) → aura
    storage keyed by owner handle with explicit cleanup ordering.
12. `Player::m_items[]` / `Bag::m_bagslot[]` raw arrays (`Player.h:863`,
    `Bag.h:69`) → item handles; save/load (`Player::SaveToDB`) operates on
    handles, never pointers.
13. GUID→pointer caches used as shortcuts instead of lookup (e.g.
    `Player::m_mover`; cf. the dangling-pointer guard comment at
    `Objects/Pet.cpp:1100-1112`) → always resolve via arena; cache handles
    only.

Migration-shape note: none of §8 is FFI-safe as-is. The seam design
(`as-pnw.7`) should assume the **first** Rust code owns entity lifetimes
(arena + despawn queue + handle resolution) while gameplay logic is ported
onto it — wrapping the C++ pointer graph in FFI would preserve exactly the
unsoundness this map documents.

## 9. Singleton inventory (process-lifetime globals)

`MaNGOS::Singleton` / `ClassLevelLockable` globals that must become
explicit resources (non-exhaustive; all under `vmangos/src/game/`):

`sWorld` (`World.h:1032`), `sObjectMgr` (`ObjectMgr.h:1594`),
`sMapMgr` (`Maps/MapManager.h:228`), `sObjectAccessor`
(`ObjectAccessor.h:161`), `sTerrainMgr` (`Maps/GridMap.h:251`),
`sPoolMgr` (`Maps/Pool/PoolManager.h:234`),
`sMapPersistentStateMgr` (`Maps/MapPersistentStateMgr.h:421`),
`sGuildMgr` (`Guild/GuildMgr.h:153`), `sLFGMgr` (`LFG/LFGMgr.h:58`),
`sBattleGroundMgr` (`Battlegrounds/BattleGroundMgr.h:317`),
`sScriptMgr` (`ScriptMgr.h:383`), `sSpellMgr` (`Spells/SpellMgr.h:710`),
`sAccountMgr` (`AccountMgr.h:151`), `sSocialMgr` (`SocialMgr.h:167`),
`sWeatherMgr` (`Weather.h:164`), `sGameEventMgr` (`GameEventMgr.h:169`),
`sWaypointMgr` (`Movement/WaypointManager.h:182`), `sAuctionMgr` /
`sAuctionHouseBotMgr` (`AuctionHouse/`), `sGuardMgr` (`GuardMgr.h:67`),
`sTransportMgr` (`Transports/TransportMgr.h:139`), `sTicketMgr`
(`GMTicketMgr.h:280`), `sHonorMaintenancer` (`HonorMgr.h:217`),
`sEventAIMgr` (`AI/CreatureEventAIMgr.h:43`), `sAuraRemovalMgr`
(`AuraRemovalMgr.h:57`), `sAnticheatMgr` (`Anticheat/Anticheat.h:117`),
`sWardenModuleMgr` / `sWardenScanMgr` (`Anticheat/WardenAnticheat/`),
`sCreatureLinkingMgr` (`Group/CreatureLinkingMgr.h:199`),
`sMassMailMgr` (`Mail/MassMailMgr.h:122`), `sZoneScriptMgr`
(`Maps/ZoneScriptMgr.h:104`), `sInstanceStatistics`
(`InstanceStatistics.h:74`), `sAddOnHandler`
(`Handlers/AddonHandler.h:39`), `sAutoBroadCastMgr`
(`AutoBroadCastMgr.h:29`), `sSpellModMgr` (`Spells/SpellModMgr.h:38`),
`sWorldSocketMgr` (`Server/WorldSocketMgr.h:61`) — plus the entire
bot-stack set: `sPlayerBotMgr`, `sGuildBotMgr`, `sBotDirector`,
`sBotFormationMgr`, `sBotChatMgr`, `sBotServiceMgr`, `sBotPatternMgr`
(`PlayerBots/`).

## 10. Dynamic follow-ups (need a running server or instrumentation)

1. Off-tick `HashMapHolder` readers: the `shared_timed_mutex` implies
   concurrent lookup; confirm which threads (SOAP/RA/async-packet per the
   threading map) actually call `FindPlayer*`/`GetUnit` concurrently.
2. `m_AI_locked` / `AIM_Initialize` re-entrancy: exercise script-triggered
   AI swaps under load to see if the lock is load-bearing.
3. Respawn path object identity: confirm a respawned creature is always a
   fresh `new Creature` from static spawn data (never a resurrected
   pointer) across pool, script-command, and BG-AV spawn paths.
4. In-flight spell/event reap timing: measure how often the
   `KillAllEvents(false)` + remove-list-reap path (§5) actually defers past
   one tick.
5. Bot logout ordering: `sBotServiceMgr.ClearRequestsForPlayer` at
   `WorldSession.cpp:597` vs `PlayerBotAI` tick — confirm no tick observes
   a half-logged-out `me`.
6. Corpse-list destructor ordering (§6): confirm bones vs resurrectable
   teardown order under mass player death (e.g. raid wipe) — the highest
   crash-risk coupling found statically.
