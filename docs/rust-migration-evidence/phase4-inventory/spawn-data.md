# Phase 4 inventory — spawn / world data loaders

- Workspace `/Users/jeffarn/Programming/azeroth-server`, branch
  `rust-migration`, HEAD `1bec69f` (inspected oracle).
- Port oracle is C++ at HEAD under `vmangos/src/`. Production
  binary is `2bae5f9`; `git diff --stat 2bae5f9..HEAD` shows
  large churn in this surface (`ObjectMgr.cpp` ~3,953 lines,
  `CreatureDefines.h` ~550, `PoolManager.*` ~446), so HEAD
  semantics below must not be assumed identical to production
  without a field-level diff. No production-vs-HEAD field
  divergence was verified in this pass; the churn size itself
  is the flagged risk.
- Dev MySQL was reachable read-only via `vmangos-rs/.env.test`
  (`TEST_MYSQL_URL_WORLD` → `mangos`, `TEST_MYSQL_URL_CHARACTERS`
  → `characters` on `localhost:3306`). Row counts below were
  executed read-only; credentials are not recorded here.
- Method: every loader body cited was opened with offsets.
  Search was used only to locate candidates.

## (a) Verified structure / flow

### Boot order (spawn-relevant slice)

`World::SetInitialWorldSettings` in
[World.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/World.cpp:1419)
loads in this order (all `WorldDatabase` unless noted):

1. `LoadGameObjectTemplates()` at World.cpp:1420, then
   transport models need the GO display ids.
2. `LoadCreatureDisplayInfoAddon()` at World.cpp:1465
   (model info — must precede creature templates).
3. `LoadEquipmentTemplates()` at World.cpp:1468.
4. `LoadCreatureSpells()` at World.cpp:1471 and
   `LoadCreatureClassLevelStats()` at World.cpp:1474.
5. `LoadCreatureTemplates()` at World.cpp:1477.
6. `LoadCreatures()` at World.cpp:1501 (spawn rows).
7. `LoadCreatureAddons()` at World.cpp:1505 — comment states
   "must be after LoadCreatureTemplates() and LoadCreatures()".
8. `sCreatureGroupsManager->Load()` at World.cpp:1510.
9. `LoadGameobjects()` at World.cpp:1513, then
   `LoadGameobjectsRequirements()` :1516 and
   `LoadGameObjectDisplayInfoAddon()` :1519.
10. `sCreatureLinkingMgr.LoadFromDB()` at World.cpp:1523
    ("must be after Creatures"), then
    `sPoolMgr.LoadFromDB()` at World.cpp:1526.
11. `LoadQuests()` at World.cpp:1532 — comment states
    "must be loaded after DBCs, creature_template,
    item_template, gameobject tables"; relations at :1536.
12. `LoadCreatureRespawnTimes()` at World.cpp:1557 and
    `LoadGameobjectRespawnTimes()` at :1560, both from
    `CharacterDatabase` (see respawn section).

Template loaders are single-entry reloadable too:
`LoadCreatureTemplate(entry)` and
`LoadGameObjectTemplate(entry)` reuse the same SELECT with
an `entry = %u` filter
([ObjectMgr.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ObjectMgr.cpp:1195),
[ObjectMgr.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ObjectMgr.cpp:7783)).
Spawn loaders take `bool reload = false` and skip grid
re-insertion for already-present GUIDs
([ObjectMgr.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ObjectMgr.cpp:2275),
[ObjectMgr.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/ObjectMgr.cpp:2481)).

### Template loaders

`LoadCreatureTemplates()` at ObjectMgr.cpp:1173 selects
78 columns (indices 0–77) from `creature_template` with
progressive patch selection
`patch = max(patch <= WowPatch)` per entry, then
`LoadCreatureInfo(fields)` at ObjectMgr.cpp:1217 fills
`CreatureInfo` in `m_creatureInfoMap[entry]`
(`unordered_map<uint32, unique_ptr<CreatureInfo>>`).
`CreatureInfo` is defined at
[CreatureDefines.h](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/CreatureDefines.h:234):
level/faction/npc_flags, 4× display_id/scale/probability,
mount display, speeds, detection/call-for-help/leash,
type/pet_family/rank/unit_class, 5 multipliers +
damage_variance/school, attack times, 6 resistances,
trainer quad, 3 loot ids + gold range, 4 spell ids +
spell lists + spawn spell + auras string, ai_name,
movement/inhabit, civilian/racial_leader, equipment_id,
trainer_id/vendor_id, immunity masks, static_flags1/2,
flags_extra, script_id. The `auras` column is stored as a
duplicated C string cast to `uint32 const*` at load
(ObjectMgr.cpp:1286) and converted later — a Rust port must
parse the space-separated spell list, not copy the cast.

`LoadGameObjectTemplates()` at ObjectMgr.cpp:7760 selects
35 columns (0–34: `entry,type,displayId,name,icon,faction,
flags,size,data0..data23,mingold,maxgold,script_name`)
with the same max-patch rule, fills `GameObjectInfo`
([GameObjectDefines.h](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/GameObjectDefines.h:211))
via `LoadGameObjectInfo` at ObjectMgr.cpp:7805, then runs
`CheckGameObjectTemplate` over every entry (ObjectMgr.cpp:7775)
which clamps `size`, forces invalid `type` to GENERIC (5)
and validates per-type lock/trap ids. The 24 `dataN`
fields are a C union reinterpreted per `type`
(GameObjectDefines.h:221) — the port needs a typed enum
per GO type, not a raw `[i32; 24]` passthrough, or every
consumer re-derives the union layout.

`LoadQuests()` at ObjectMgr.cpp:5291 selects 131 columns
(0–130) from `quest_template` with max-patch selection,
constructs `Quest(fields)` per row into
`m_QuestTemplatesMap`, then post-processes integrity
(method/special-flags bounds, item/creature/GO refs,
zone/sort validity). `Quest` is
[QuestDef.h](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/QuestDef.h:211)
(407 lines) + QuestDef.cpp (231 lines). This is the widest
single loader in the surface and is only needed at spawn
time indirectly (questgiver flags come from templates +
relations), but it gates any populated-area slice that
shows `!`/`?` gossip state.

### Spawn-row loaders (the placement tables)

`LoadCreatures(reload)` at ObjectMgr.cpp:2275 queries
24 columns from `creature` LEFT JOIN `game_event_creature`,
`pool_creature`, `pool_creature_template`. Stored fields
(ObjectMgr.cpp:2387–2408) into `CreatureData`
([CreatureDefines.h](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/CreatureDefines.h:370)):
`creature_id[5]` (`id,id2..id5`), `position (map,x,y,z,o)`,
`spawntimesecsmin/max`, `wander_distance`,
`health_percent/mana_percent`, `movement_type`,
`spawn_flags`, `visibility_mod`, plus derived
`instanciatedContinentInstanceId`. The three join columns
(`event`, both `pool_entry`) are **not stored** — they only
decide grid insertion: a spawn is added to
`m_MapObjectGuids` via `AddCreatureToGrid` (ObjectMgr.cpp:2461)
only when `!alreadyPresent && existsInPatch && event==0 &&
GuidPoolId==0 && EntryPoolId==0` (ObjectMgr.cpp:2452).
Out-of-patch spawns are kept in `m_CreatureDataMap` with
`SPAWN_FLAG_DISABLED` OR-ed in (ObjectMgr.cpp:2419) rather
than dropped — distinct from GO handling below.

`LoadGameobjects(reload)` at ObjectMgr.cpp:2481 queries
22 columns from `gameobject` with the same three joins
(`game_event_gameobject`, `pool_gameobject`,
`pool_gameobject_template`). Stored into `GameObjectData`
([GameObjectDefines.h](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/GameObjectDefines.h:910)):
`id, position, rotation0..3, spawntimesecsmin/max (i32),
animprogress, go_state, spawn_flags, visibility_mod` +
continent instance id. Grid insertion mirrors creatures
(ObjectMgr.cpp:2624) except `GAMEOBJECT_TYPE_TRANSPORT`
rows go to `sTransportMgr.AddElevatorTransportForMap`
instead. Unlike creatures, out-of-patch GO rows are
`continue`-skipped entirely (ObjectMgr.cpp:2524) and never
enter `m_GameObjectDataMap`.

Both loaders validate eagerly and log `LOG_DBERROR` +
`LOG_DBERRFIX` fix-up SQL: missing template entry → skip
row; invalid patch range → clamp to 0..10; bad map →
skip; `spawntimesecsmax < min` → raise max; creature-only
health/mana percent clamps and movement/wander fix-ups
(ObjectMgr.cpp:2351–2450); GO-only rotation range
[-1,1], `state < MAX_GO_STATE`, coordinate validity
(ObjectMgr.cpp:2590–2619). A Rust port that skips these
clamps will diverge on dirty data even if happy-path
spawns match.

Grid index: `CellObjectGuids { creatures, gameobjects,
corpses }` per `cell_id = y*TOTAL_NUMBER_OF_CELLS_PER_MAP+x`
per map (`MapObjectGuids`, ObjectMgr.h:151–158), mutated
under `m_MapObjectGuids_lock`. This index — not the raw
tables — is what cell activation iterates to call
`LoadFromDB` (see wire semantics).

### Addon / equipment / model-info loaders

`LoadCreatureAddons()` at ObjectMgr.cpp:1685 delegates to
the `SQLStorage` overload at ObjectMgr.cpp:1622 over
`sCreatureDataAddonStorage` (`creature_addon` table, key
`guid`, format `"iiiibbis"` —
[SQLStorages.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Database/SQLStorages.cpp:27)).
`CreatureDataAddon` (CreatureDefines.h:415):
`guid, display_id, mount_display_id, equipment_id,
stand_state, sheath_state, emote_state, auras`.
Post-load validation zeroes unknown display/mount ids,
forces `equipment_id = -1` when the equipment template is
missing (ObjectMgr.cpp:1656), clamps stand/sheath/emote,
converts the aura string, and warns for addons whose GUID
has no spawn row (ObjectMgr.cpp:1690). Loaded with
`LoadProgressive(WowPatch)` — patch-versioned like
templates.

`LoadEquipmentTemplates()` at ObjectMgr.cpp:1697 reads
`creature_equip_template (entry, probability, item1..3)`
filtered by `WowPatch BETWEEN patch_min AND patch_max`
into `m_CreatureEquipmentMap: entry → EquipmentTemplate
{ totalProbability, Vec<EquipmentEntry> }`
(CreatureDefines.h:334). Post-load it zeroes any item
whose prototype is missing or not hand-equippable
(ObjectMgr.cpp:1744). Selection at spawn is a weighted
roll `urand(0, totalProbability-1)` in
`ChooseEquipmentEntry` (CreatureDefines.h:345).

`LoadCreatureDisplayInfoAddon()` at ObjectMgr.cpp:1796
loads `sCreatureDisplayInfoAddonStorage`
(`creature_display_info_addon`, key `display_id`, format
`"iffffbi"`, SQLStorages.cpp:28) via
`LoadProgressive(SUPPORTED_CLIENT_BUILD, "build")` —
note: versioned by **client build**, not WowPatch.
`CreatureDisplayInfoAddon` (CreatureDefines.h:427):
`display_id, bounding_radius, combat_reach, speed_walk,
speed_run, gender, display_id_other_gender`. It validates
DBC existence, gender bounds, speed floors, and playable-race
model pairs (ObjectMgr.cpp:1801–1898).
`GetCreatureDisplayInfoRandomGender` (ObjectMgr.cpp:1776)
gives a 50% swap to the other-gender display when present.

`LoadGameObjectDisplayInfoAddon()` at ObjectMgr.cpp:8090
loads `sGameObjectDisplayInfoAddonStorage`
(`gameobject_display_info_addon`, format `"iffffff"`):
`GameObjectDisplayInfoAddon { display_id, min/max xyz }`
(GameObjectDefines.h:930) — the GO bounding box used for
interaction distance. Plain `Load()`, no patch filter.

`LoadCreatureSpells()` at ObjectMgr.cpp:1904 reads
`creature_spells` (8 spell slots × 11 columns + entry =
89 columns) plus `creature_spells_scripts` ids into
`m_CreatureSpellsMap`. `LoadCreatureClassLevelStats()` at
ObjectMgr.cpp:2043 reads `creature_classlevelstats`
(15 columns) per distinct `unit_class` into
`m_CreatureCLSMap: class → Vec<stats by level>`.
Both are spawn-time dependencies: without CLS a spawned
creature has no health/damage base; without spells map
the template `spell_list_id` dangles.

`LoadGameobjectsRequirements()` (query at
ObjectMgr.cpp:8017: `gameobject_requirement (guid,
reqType, reqGuid)`) links GO spawns to required
creature/GO GUIDs. Small, but GO visibility in quest
areas depends on it.

### Pools and spawn groups

Pools exist and are first-class. `PoolManager::LoadFromDB()`
at
[PoolManager.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/Pool/PoolManager.cpp:663)
(1,231 lines total) loads, all patch-filtered
(`WowPatch BETWEEN patch_min AND patch_max`):

- `pool_template (entry, max_limit, flags, description,
  instance)` → `m_poolTemplate[pool_id]`
  (`PoolTemplateData`, PoolManager.h:41); every pool gets
  `POOL_FLAG_AUTO_SPAWN` forced on (PoolManager.cpp:711).
- `pool_creature` UNION `pool_creature_template`
  (joining `creature` on entry) →
  `m_poolCreatureGroups[pool_id]` of `PoolObject
  { guid, spawnFlags, chance, exclude }` (PoolManager.h:60).
- `pool_gameobject` / `pool_gameobject_template` likewise.
- `pool_pool (pool_id, mother_pool, chance, flags)` for
  nested pools (query at PoolManager.cpp:896).

Pool membership is why the spawn loaders withhold grid
insertion: pooled GUIDs are spawned by `PoolGroup<T>::
SpawnObject` up to `MaxLimit` (with chance weighting and
an optional population-scaling flag) and tracked in
per-map-state `SpawnedPoolData`. The join-based
withholding in `LoadCreatures`/`LoadGameobjects` uses the
raw pool tables, while `PoolManager` re-reads the same
tables with its own patch logic — the two must agree or
a spawn is either double-spawned or never spawned.

There is no table literally named `spawn_group`. The two
group mechanisms are: (1) **creature groups** —
`CreatureGroupsManager::Load()` at
[CreatureGroups.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/CreatureGroups.cpp:411)
reads `creature_groups (leader_guid, member_guid, dist,
angle, flags)` and `creature_groups_entry_limit
(leader_guid, creature_id, min_count, max_count)`;
formation offsets + per-entry count limits, keyed by
leader DB GUID via `ConvertDBGuid` (requires the spawn to
exist in `m_CreatureDataMap`). (2) **multi-id spawns** —
`creature_id[5]` with uniform-random `ChooseCreatureId`
(CreatureDefines.h:389), overridden by group limits when
the GUID belongs to a creature group. `Map::
LoadCreatureSpawnWithGroup` at Map.cpp:3675 spawns a
leader and its group members together.

`GameEvent` membership (`game_event_creature` /
`game_event_gameobject`) also withholds grid insertion;
event activation spawns/despawns them later. Counts in
section (b) show this is not a corner case.

### Respawn storage (which DB)

Respawn times live in the **character DB**, not the world
DB. `MapPersistentStateManager::LoadCreatureRespawnTimes()`
at
[MapPersistentStateMgr.cpp](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Maps/MapPersistentStateMgr.cpp:986)
first deletes expired rows, then reads
`creature_respawn (guid, respawn_time, map, instance)`
LEFT JOIN `instance` from `CharacterDatabase`, validates
each GUID against `sObjectMgr.GetCreatureData` + map
match, and fans continent rows out across instanced
continent ids when `CONFIG_BOOL_CONTINENTS_INSTANCIATE`
is set. `LoadGameobjectRespawnTimes()` at
MapPersistentStateMgr.cpp:1063 mirrors it over
`gameobject_respawn`. Runtime writes go through
`MapPersistentState::SaveCreatureRespawnTime /
SaveGORespawnTime`. The spawn tables' `spawntimesecsmin/
max` (world DB, static) only seed the delay; the
absolute deadline is character-DB state that survives
restarts.

### Phase 3 Rust cross-check

[enum_stores.rs](/Users/jeffarn/Programming/azeroth-server/vmangos-rs/crates/mangos-world-data/src/enum_stores.rs:1)
(126 lines) is a deliberate narrowing: `load_enum_stores`
at enum_stores.rs:90 loads only `item_template (entry,
display_id, inventory_type)`, `creature_template (entry,
pet_family)`, and `playercreateinfo (race, class)` — the
three lookups character enum needs. Patch selection
already matches C++ (`max(patch <= ?)`, enum_stores.rs:92).
[mangos-world-data lib.rs](/Users/jeffarn/Programming/azeroth-server/vmangos-rs/crates/mangos-world-data/src/lib.rs:12)
exposes only `assets_check`, `enum_stores`, `strings`;
its doc comment states the remaining ~90 loaders are
deferred and logged. **Nothing spawn-related exists in
Rust yet**: no `CreatureInfo`, no spawn rows, no grid
index, no pools, no respawn store. `WowPatch` plumbing
does exist (`mangosd` lib.rs:147 clamps config to 10;
main.rs:606 passes it to `load_enum_stores`), so new
loaders can reuse the same patch parameter and
`DbActor::call` convention proven by the enum tests.

## (b) Sizes / counts

Code size (HEAD, `wc -l`):

| File | Lines |
|---|---:|
| `ObjectMgr.cpp` (all loaders) | 12,038 |
| `ObjectMgr.h` | 1,596 |
| `CreatureDefines.h` | 674 |
| `GameObjectDefines.h` | 953 |
| `QuestDef.h` + `QuestDef.cpp` | 638 |
| `PoolManager.cpp` + `.h` | 1,500 |
| `CreatureGroups.cpp` + `.h` | 665 |
| Rust `enum_stores.rs` (Phase 3) | 126 |

Loader widths: `creature_template` 78 cols,
`gameobject_template` 35, `quest_template` 131,
`creature` spawn query 24, `gameobject` spawn query 22,
`creature_spells` 89, `creature_classlevelstats` 15,
`creature_equip_template` 5.

Dev-bench row counts (read-only, executed this pass;
"effective" = max-`patch <= 10` deduplicated):

| Table (world DB `mangos`) | Raw rows | Effective entries |
|---|---:|---:|
| `creature_template` | 10,912 | 10,381 |
| `creature` (spawns) | 66,222 | — (patch_min/max range) |
| `creature_addon` | 3,413 | progressive by patch |
| `creature_equip_template` | 5,182 | patch-range filtered |
| `creature_display_info_addon` | 10,537 | by client build |
| `creature_spells` | 2,596 | — |
| `creature_classlevelstats` | 189 | — |
| `gameobject_template` | 9,007 | 8,984 |
| `gameobject` (spawns) | 56,653 | patch-range filtered |
| `quest_template` | 4,725 | 4,433 |
| `pool_template` | 7,272 | 7,255 in patch 10 |
| `pool_creature` / `pool_creature_template` | 140 / 56 | — |
| `pool_gameobject` / `pool_gameobject_template` | 29,478 / 100 | — |
| `pool_pool` | 5,591 | — |
| `creature_groups` | 5,799 | — |
| `game_event_creature` / `game_event_gameobject` | 3,064 / 12,137 | — |
| `item_template` (Phase 3 reference) | — | 17,707 |

Character DB (`characters`): `creature_respawn` 25 rows,
`gameobject_respawn` 0 at probe time (expired rows are
deleted at load, so steady-state is small).

Distribution facts that shape the slice: map 0 holds
24,650 creature spawns and 26,195 GO spawns (map 1:
29,074 / 24,274); 8,812 spawn rows use `id2..id5`
multi-entry selection; non-zero `spawn_flags` on 392
creature + 472 GO rows; 29,478 pooled GO GUIDs means
**over half of all GO spawns are pool-managed** (mostly
herb/ore nodes) — a slice that ignores pools shows a
visibly wrong world for gameobjects even if creatures
look right. The Phase 3 live test already pins
`creature_count() == 10,381` and `item_count() == 17,707`
(`live_enum_stores.rs`), corroborating the template
counts above by an independent route.

## (c) DB / config inputs

- **World DB** (`WorldDatabase`): `creature_template`,
  `creature`, `creature_addon`, `creature_equip_template`,
  `creature_display_info_addon`, `creature_spells`,
  `creature_spells_scripts`, `creature_classlevelstats`,
  `gameobject_template`, `gameobject`,
  `gameobject_display_info_addon`, `gameobject_requirement`,
  `quest_template` (+ quest relation tables),
  `pool_template`, `pool_creature`,
  `pool_creature_template`, `pool_gameobject`,
  `pool_gameobject_template`, `pool_pool`,
  `creature_groups`, `creature_groups_entry_limit`,
  `game_event_creature`, `game_event_gameobject`,
  `map_template` (via `sMapStorage`, SQLStorages.cpp:43 —
  spawn map validation reads it), `item_template`
  (equipment validation).
- **Character DB** (`CharacterDatabase`):
  `creature_respawn`, `gameobject_respawn`, `instance`
  (reset times joined at respawn load).
- **DBC stores** consulted during validation:
  `CreatureDisplayInfo`, `GameObjectDisplayInfo`,
  `ChrRaces`, `Emotes`, `Map` (via `sMapStorage`).
  A spawn with an unknown display id is dropped or
  zeroed, so DBC loading must precede these loaders.
- **Config**: `WowPatch` (`World.cpp:461`, default patch
  1.2, range 0–10) drives every `max(patch<=)` and
  `BETWEEN patch_min AND patch_max` filter;
  `SUPPORTED_CLIENT_BUILD` (5875) versions
  `creature_display_info_addon`;
  `CONFIG_BOOL_CONTINENTS_INSTANCIATE` changes respawn
  fan-out and pool `instance` handling
  (PoolManager.cpp:710). Rust side: `.env.test` provides
  `TEST_MYSQL_URL_WORLD` / `TEST_MYSQL_URL_CHARACTERS`;
  `mangosd` already parses `WowPatch` clamped to 10.
- No files on disk are read by these loaders (maps/vmaps
  are a separate surface); spawn Z is taken from the DB
  as-is except the dead-flyer ground snap in
  `LoadFromDB` (Creature.cpp:1855), which needs terrain
  height at spawn time, not load time.

## (d) Wire-visible semantics a Rust port must replicate

1. **Patch selection is observable.** Template fields
   (stats, spells, display, loot) differ per patch row;
   spawn existence flips with `patch_min/max`. The port
   must implement both forms exactly: `max(patch <= P)`
   for `*_template` + `creature_addon` (progressive),
   and `P BETWEEN patch_min AND patch_max` for spawn
   rows, equipment, and pools. The creature/GO asymmetry
   matters: out-of-patch creatures stay in the store
   flagged `DISABLED` (queryable, `.npc` commands see
   them); out-of-patch GOs vanish from the store.
2. **GUID identity.** DB GUID + entry form the full
   `ObjectGuid(HIGHGUID_UNIT|GAMEOBJECT, entry, lowguid)`;
   `creature_id[0]` is always the GUID entry even when a
   random alternate id is chosen for stats/AI
   (Creature.cpp:1814, 1836). Clients see the `id` entry;
   combat uses the chosen one. Multi-id spawns re-roll
   per respawn and force AI re-init
   (Creature.cpp:1911).
3. **Spawn flags** (`ObjectDefines.h:127`): `ACTIVE`
   (always-loaded object), `DISABLED` (never spawns;
   also synthesized for out-of-patch), `RANDOM_RESPAWN`,
   `DYNAMIC_RESPAWN`, `FORCE_DYNAMIC_ELITE`,
   `EVADE_OUT_HOME_AREA`, `NOT_VISIBLE` (spawns with
   `VISIBILITY_OFF`, Creature.cpp:1907), `DEAD` (spawns
   as corpse with 0 health/mana, Creature.cpp:1842).
   `health_percent == 0` also implies dead.
4. **Respawn timing.** Delay = `urand(spawntimesecsmin,
   spawntimesecsmax)` per spawn (`GetRandomRespawnTime`,
   CreatureDefines.h:388); GO negative `spawntimesecsmin`
   means "not spawned by default, delay = -min"
   (GameObject.cpp:972); non-despawnable GOs get
   `GO_FLAG_NODESPAWN` and no respawn (GameObject.cpp:949).
   Absolute deadlines persist in character-DB respawn
   tables and suppress spawning across restarts
   (Creature.cpp:1850, GameObject.cpp:963); expired
   deadlines are cleared with a write-back of 0.
5. **Display/equipment randomness is server-side.**
   Display id is picked from the 4 template slots by
   probability, overridable per-GUID by
   `creature_addon.display_id`, mount likewise; the
   other-gender 50% swap (ObjectMgr.cpp:1783) changes
   the model clients render. Equipment is the weighted
   roll in (a); priority at spawn is event data >
   addon `equipment_id` > template `equipment_id`
   (Creature.cpp:1946–1956), with `-1` meaning
   explicitly none. `stand_state/sheath_state/
   emote_state` and addon/template auras are all in the
   create block.
6. **Pool/event withholding.** Pooled or event-linked
   spawns must not appear via the normal grid path;
   pools spawn exactly `MaxLimit` weighted picks (plus
   population scaling when flagged) and roll a
   replacement on despawn. Getting the join logic wrong
   by one table doubles or erases ~29k GO nodes.
7. **Movement seed.** `movement_type` + `wander_distance`
   from the spawn row (not the template, unless the C++
   fallback applies) become the default motion generator
   (Creature.cpp:1900); loader fix-ups (random-with-0-
   wander → idle, idle-with-wander → 0) are part of the
   observable behavior on dirty rows.
8. **Validation side effects are spec.** Skipped rows
   (bad entry/map/rotation/state/display) never reach
   clients; clamped values (patch range, respawn range,
   health/mana %) do. A port that loads raw rows without
   the ObjectMgr.cpp:2317–2619 checks will show extra
   or corrupt objects the C++ server never sends.
9. **Visibility modifiers.** Non-zero `visibility_mod`
   overrides sight range per spawn (Creature.cpp:1847,
   GameObject.cpp:983) — affects when the create packet
   is sent at all.

## (e) Smallest viable first slice + entry dependencies

Goal: one populated start area visible and correctly
respawning — e.g. a single map-0 cell cluster around a
race start (Northshire) — without events, linking, or
transports.

Load order for the slice (all patch-aware, reusing the
Rust `WowPatch` + `DbActor` conventions):

1. `creature_display_info_addon` store (bounding radius /
   combat reach / speeds / gender swap) — needed by any
   creature create block. Entry dep: CreatureDisplayInfo
   DBC for validation (may defer validation, keep data).
2. `creature_equip_template` → equipment map.
   Entry dep: item prototypes for the hand-equippable
   check; Phase 3 `ItemDisplay` is **not** enough
   (needs `InventoryType` — actually present — plus
   existence; reuse enum store and extend if needed).
3. `creature_classlevelstats` + full `creature_template`
   (extend enum_stores' pet-family query into the 78-col
   `CreatureInfo`; keep `EnumStores` as a view or replace
   it — decide once, do not load the table twice).
4. `gameobject_template` (35 cols, typed `data` enum)
   + `gameobject_display_info_addon`.
5. Spawn rows **filtered to one map** (`WHERE map = 0`
   is 24,650 rows — filter further by cell range at the
   SQL or immediately post-load; the C++ loads all maps,
   the slice may load one map and must say so in logs,
   never silently). Include the three pool/event joins
   from day one, but the slice may treat pooled/event
   spawns as "withheld, not yet spawned" — correct for
   creatures in a start area (only 196 pool rows), visibly
   incomplete for GO nodes (state it, do not fake it).
6. `creature_addon` for the loaded GUIDs + grid index
   (`MapObjectGuids` equivalent: `map → cell → guid set`).
7. Respawn store: read `creature_respawn` /
   `gameobject_respawn` from the character DB, expire +
   write back through the existing Rust character-DB
   actor. Even the slice needs this or killed quest mobs
   reappear on every restart.
8. Defer with explicit logs (Phase 3 pattern):
   `PoolManager` spawning, `creature_groups` formations
   (slice spawns members at their own DB coords — wrong
   formation, right presence), creature linking, game
   events, transports/elevators, `creature_spells`
   lists, quests (no `!` markers until `quest_template`
   + relations land).

Entry dependencies (must exist before/at this slice):
DBC display stores, `map_template` for map validation,
terrain height for the dead-flyer snap (deferrable:
slices spawns are ground mobs), cell activation + map
update loop from the simulation surface to consume the
grid index, and `ObjectGuid`/update-field plumbing to
emit create blocks. The loader slice itself is pure
data: it can land and be live-tested (counts + spot
rows, mirroring `live_enum_stores.rs`) before any map
loop exists.

## (f) Unresolved

- Production `2bae5f9` vs HEAD `1bec69f` field-level
  divergence in this surface was not diffed body-by-body;
  the diffstat (~2.4k insertions) is large enough that
  at least formatting/refactoring dominates, but any
  semantic change (patch rules, flag handling) is
  unconfirmed. Needs a targeted diff of the loader
  bodies before parity claims against production.
- `SQLStorage::LoadProgressive` exact tie-breaking when
  multiple rows share the max patch/build (creature_addon,
  display addon) was not opened; the template loaders'
  SQL form was verified, the storage-framework form was
  taken from call sites only.
- `Creature::Create` consumption of template fields
  (display-probability pick, stats derivation from CLS
  × multipliers, aura application) was read only at the
  `LoadFromDB` boundary; the full field-to-wire mapping
  belongs to the creature-object surface.
- Pool spawn algorithm details (chance normalization,
  `POOL_FLAG_MAXLIMIT_SCALING_LINEAR` population curve,
  mother-pool recursion) were inventoried at load level;
  `PoolGroup::SpawnObject` bodies were not fully read.
- `sCreatureLinkingMgr.LoadFromDB` and
  `sGameEventMgr.LoadFromDB` table sets were not opened;
  only their withholding effect on grid insertion and
  their boot-order position are verified here.
- Whether dev-bench counts equal production counts is
  unconfirmed; counts above describe the bench world DB
  (`mangos`) only, at probe time.
- `gameobject_requirement` runtime effect (which packet
  or visibility rule consumes it) was not traced beyond
  the loader query.
