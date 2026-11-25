# Tier 3 progress: content depth + battlegrounds

Goal: Tier 3 (vmap manager, content-script
corpus, battlegrounds) on `rust-migration`,
fork `vmangos/src/` as oracle. Tier 2 baseline
commit 680d0eba.

## Slice 1 inventory: runtime vmap manager

### Fork shape (`vmangos/src/game/vmap/`)

- `VMapManager2` (global via `VMapFactory`):
  per-map `StaticMapTree*` + global WorldModel
  file cache (`acquireModelInstance(basepath,
  filename)` → `WorldModel::readFile`).
  Queries: `isInLineOfSight`, `getObjectHitPos`
  (hit point + `pModifyDist` pushback),
  `getHeight`, `getAreaInfo`, `isUnderModel`,
  `GetLiquidLevel`. Coordinate conversion
  `convertPositionToInternalRep(x,y,z) =
  (mid-x, mid-y, z)`, `mid = 0.5*64*533.33333333`
  — self-inverse.
- `StaticMapTree::InitMap` reads `.vmtree`:
  magic, `iIsTiled` byte, NODE chunk = the
  map-level BIH (**prebuilt in file** — no
  runtime BIH build), GOBJ = global spawns
  (non-tiled maps only). `iTreeValues` sized
  `iTree.primCount()`; spawns land at
  `referencedVal` slots, refcounted across
  tiles (`iLoadedSpawns`).
- `LoadMapTile(tileX, tileY)`: reads
  `%03u_%02u_%02u.vmtile` spawns, acquires each
  model, `model->setModelFlags(spawn.flags)`,
  fills slots. Non-tiled maps: fake tile loads
  for lifetime accounting. Tiles load on grid
  activation (`Map::LoadMapAndVMap`).
- `ModelInstance(spawn, model)`:
  `iInvRot = Matrix3::fromEulerAnglesZYX(
  pi*iRot.y/180, pi*iRot.x/180, pi*iRot.z/180)
  .inverse()`, `iInvScale = 1/iScale`; spawn
  fields inherited incl. `iBound` (file
  bound when `MOD_HAS_BOUND`, else default
  AABox). `intersectRay`: AABox
  `intersectionTime` early-out → ray to model
  space (`invRot*(origin-pos)*invScale`,
  `invRot*dir`), `WorldModel::IntersectRay`
  with `dist*invScale`, hit distance rescaled
  by `iScale`. G3D `fromEulerAnglesZYX(yaw,
  pitch, roll) = Rz(yaw)*Ry(pitch)*Rx(roll)`.
- Queries: `isInLineOfSight` = normalized ray,
  `stopAtFirstHit=true`, inverted hit;
  `getObjectHitPos` = nearest hit point, then
  `pModifyDist` shift (negative clamps at
  source when hit closer than `|modify|`);
  `getHeight` = vertical ray (down for
  `maxSearchDist>=0`, else up), height =
  `z ∓ hitDist`, +inf on miss.
- `Map::GetLosHitPosition` = static
  `getObjectHitPos` then dynamic-tree
  (`GetDynamicObjectHitPos`, gameobject
  models) against the possibly trimmed dest.
- `TerrainInfo::GetHeightStatic` combine:
  probe from `z2 = z+2`; widen search to reach
  mapHeight; miss → 10000 fallback → upward
  search when terrain >30 above → near-terrain
  retry; selection: vmap wins when
  `z < mapHeight || vmapHeight > mapHeight`,
  else map height. `Map::GetHeight` then maxes
  with dynamic-tree height.
- Config: `vmap.enableLOS` /
  `vmap.enableHeight` (fork defaults false;
  bench conf sets both 1),
  `vmap.enableIndoorCheck` (true),
  `DetectPosCollision` (true).

### Rust state

- Parser complete and parity-tested
  (`mangos-assets/src/vmap.rs`): `Bih`
  (read + `intersect_ray` with entry callback
  receiving `objects[off]` = referencedVal),
  `WorldModel::intersect_ray` (MOD_M2 gate),
  `GroupModel`, `MapTreeFile` (tiled + BIH +
  global spawns), `parse_tile_file`,
  `ModelSpawn`, `Ray`, `AABox`, `Vec3`.
  Missing primitives: `Mat3` (Euler ZYX +
  inverse), ray-vs-AABox intersection time.
- No runtime manager: `Heights`
  (`motion.rs`) is GridMap-only with the
  `hint+10` dungeon heuristic explicitly
  standing in for vmap. `DataDir::vmaps_dir()`
  exists; bench data present
  (`datadir/vmaps/*.vmtree/.vmtile` + model
  files). No LoS anywhere in spell target
  checks today.
- Consumers waiting: Blink
  `first_collision_dest` (two
  `GetLosHitPosition` trims, Tier 2 Slice 6.1),
  `GetNearPoint` selector/LoS fallbacks
  (6.5), all `ground_z` callers.

### Slice 1 scope decision

Build, fork-shaped but lazy (NavHost
precedent — tiles load around query points
instead of grid activation):

1. `mangos-assets`: `Mat3` +
   `Ray::intersection_time_aabox` (G3D slab
   semantics, inf on miss).
2. `mangos-map/src/vmaps.rs`: `ModelInstance`,
   `StaticMapTree` (init from `.vmtree`,
   tile load/unload bookkeeping, queries
   transcribed), `VmapHost` (per-map tree +
   model cache, lazy 3x3 tile ensure around
   query endpoints, enable flags).
3. `Heights` gains a `VmapHost`;
   `ground_z` becomes the literal
   `GetHeightStatic` transcription (falls
   back to current GridMap behavior when
   vmaps dir unset / height calc disabled).
   `Map::set_vmaps_dir` + config threading
   (`vmap.enableLOS`, `vmap.enableHeight`).
4. Consumers: `Map::get_los_hit_position`
   (static half), Blink trims in
   `first_collision_dest`,
   `Map::is_in_line_of_sight` helper.
   Spell-target LoS wiring is a separate
   behavior surface — recorded, not in this
   slice. `GetNearPoint` `ObjectPosSelector`
   occupancy walk likewise recorded (its LoS
   predicate becomes available).
5. Out of slice (recorded): dynamic
   gameobject tree (`DynamicTree`/
   `GameObjectModel`, feeds the second half of
   `GetLosHitPosition`/`GetHeight`),
   `getAreaInfo`/`isUnderModel`/`GetLiquid
   Level` vmap halves (indoor check, liquid),
   tile unload (host keeps tiles; fork
   unloads with grids — memory nuance only).

Verification plan: `live_vmap` in-process
test against bench datadir (data-driven:
largest-bound spawn segment blocked / hit
pos between endpoints / height over model
above terrain / open-field clear), existing
live suites as regression (height combine is
the risky change — `live_deadmines`,
`live_motion`, `live_spells`), `./ci.sh`,
bench restart + smoke, Blink-into-wall wire
leg.

## Slice 1 — results (done)

Implemented (wonder, first-of-kind):

- `mangos-assets/src/vmap.rs`: `Mat3`
  (G3D `fromEulerAnglesZYX`, cofactor inverse
  with the fork `|det| <= 1e-6` zero rule),
  `Ray::intersection_time_aabox` (G3D
  candidate-plane slab test), `Vec3::length`,
  scalar mul.
- `mangos-map/src/vmaps.rs`: `ModelInstance`
  (SetUnscaledModel/GetIH/GetInvRot/intersect
  per fork; MOD_M2 per instance — see scope
  note), `StaticMapTree` (InitMap `numSpawns`
  slot vector, first-writer tile fill,
  LoadMapTile range clip, `IsInLineOfSight`,
  `GetObjectHitPos`, `GetHeight`, fork epsilon
  `dir*(1e-5)` tightening), `VmapHost` (lazy
  per-map tree, 3×3 tile loads around query
  endpoints inside the static-biased tile
  rect, per-host model cache, world-coordinate
  query API with the manager's enable gates and
  NaN-safe coordinate guard).
- `Heights::ground_z`: literal
  `TerrainInfo::GetHeightStatic` combine when
  vmap height is enabled (`z+2` probe, widened
  search, 10000 fallback, upward arm,
  near-terrain arm, `z < mapHeight ||
  vmapHeight > mapHeight` select); sentinel
  result falls back to the caller's hint so
  existing motion callers keep working. When
  vmaps are unset/disabled the old GridMap
  heuristic runs unchanged.
- `Map::is_in_line_of_sight` /
  `Map::get_los_hit_position` (static halves).
- Blink (`first_collision_dest`): both fork
  trims wired (−1.0 horizontal at collision
  height with 2D dist recompute, −0.5 vertical
  onto dest); path target now at `destZ +
  halfHeight` like the fork.
- Config/deps: `ServerConfig.vmap_enable_los`/
  `vmap_enable_height` from `vmap.enableLOS`/
  `vmap.enableHeight` (fork defaults false;
  bench conf = 1), `WorldDeps.vmaps_dir`,
  `InitialWorldSettings.vmaps_dir =
  DataDir/vmaps`, map creation calls
  `Map::set_vmaps_dir`.

Verification:

- Host-level bench-data test
  (`bench_vmap_host_queries`, ignored):
  `LIVE-VMAP-PASS filled=9326` — 101-tile
  spawn init, 9-tile fill around Goldshire
  (9326 slots), horizontal/vertical height
  rays, blocked-chord hit + trimmed hit pos,
  open-air LoS, MOD_M2 ignore arm, ocean
  no-hit sentinel. (First cut failed on the
  zone-scale Stormwind WMO chord being open
  air — test now picks building-sized bounds;
  host code was correct.)
- `live_motion` green with
  `set_vmaps_dir(..., true, true)` — the
  GetHeightStatic combine is exercised against
  the C++-saved fixture z and all motion legs.
- `./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s1.log`).
- Bench rebuilt + restarted (LISTEN 18099,
  vmaps enabled by bench conf);
  `TWO-CLIENT-SMOKE-PASS`.
- Blink-into-wall end-to-end wire leg not run:
  the trim primitive is proven at host level
  and the integration type-checks through
  `leap_effect`; a dedicated wall-blink wire
  probe needs a mage fixture positioned at a
  known wall — recorded, do with the first
  script slice that teleports near geometry.

Recorded remainder (Slice 1):

- Dynamic gameobject tree (`DynTreeImpl`):
  `GetLosHitPosition`/`GetHeight` second
  halves, door collision state.
- `ObjectPosSelector` full port (`GetNearPoint`
  occupancy walk). Rust today only has
  `GetNearPoint2D`/`GetNearPointAroundPosition`
  subsets; the LoS predicate they need
  (`Map::is_in_line_of_sight`) now exists.
  Port when a Slice 2/3 consumer reaches it.
- Spell-target LoS wiring (`IsWithinLOS` in
  unit-target checks / `SPELL_FAILED_NO_LOS`):
  predicate available, call sites untouched.
- `CanReachPosition`, `getAreaInfo`,
  `isUnderModel`, indoor flag, liquid levels.
- Tile unload (lazy tiles accumulate; memory
  nuance only).
- `GetHeightStatic` no-grid arm defaults
  `mapHeight` to the sentinel so vmap-only
  dungeons resolve; fork would skip the query
  entirely without a grid (behavioral nuance
  confined to maps lacking `.map` files).

## Slice 2 — inventory (content-script corpus)

Fork taxonomy (`vmangos/src/game/ScriptMgr.h`
`struct Script`, 25 hook slots; binding by
name→`ScriptId` at load from template
`script_name` columns: `creature_template`,
`gameobject_template`, `areatrigger_template`,
`spell_template`, `map_template`). Corpus
classification of the 221 content `.cpp`
(excluding `ScriptLoader*`/precompiled) by
registered hooks — 749 `Script` objects total:

- `GetAI` creature AI: 177 files (base classes:
  `ScriptedAI` 149, `npc_escortAI` 26,
  `FollowerAI` 7, `Scripted_NoMovementAI` 3,
  `ScriptedPetAI`/critter/totem/misc few)
- `GOGetAI` 33, `pGOHello` 21, `pGOOpen` 3
- `GetSpellScript` 36, `GetAuraScript` 17
- `pQuestAcceptNPC` 33, `pGossipHello` 23,
  `pGossipSelect` 17, `pQuestRewardedNPC` 12,
  `pQuestRewardedGO` 4, `pGOQuestAccept` 2
- `GetInstanceData` 24, `pAreaTrigger` 19,
  `pProcessEventId` 8, `pEffectDummy*` 10
- 6 files register nothing themselves: the 4
  nightmare dragons ride `boss_dragon_of_
  nightmare.cpp`'s shared registration;
  `alterac_mountains.cpp` is an empty
  `AddSC`; `custom/custom.cpp` only chains
  `AddSC_custom_creatures`.

DB binding census (bench): 446 distinct
creature `script_name`s, 83 gameobject names,
`map_template.script_name` per instance map
(patch-keyed), `spell_template.script_name`.

Rust coverage today:

- Data-driven layer DONE: `mangos-scripts-
  engine` stores + `mangos-map/src/scripts.rs`
  executor (DB script timelines, EventAI,
  conditions, texts — Phases 4–6/Depth).
- Hand ports: ONLY Deadmines
  (`mangos-map/src/content.rs`):
  `instance_deadmines` (hardcoded map 36 check
  in `map.rs:578`, no `map_template` loading),
  `boss_mr_smite` (ad-hoc `script_name ==`
  dispatch in `ai.rs`), 3 GO hello scripts
  (match in `gameobject_use`). All other boss
  behavior in Deadmines is EventAI.
- `customs.rs` covers fork `EffectDummy`/
  `EffectScriptEffect` spell customs (Tier 2),
  NOT the `scripts/spells/*` `SpellScript`/
  `AuraScript` corpus (bound via
  `spell_template.script_name`, not loaded).
- No generic binding layer: name dispatch is
  ad-hoc at three call sites; no gossip/quest
  script callbacks; no areatrigger script
  dispatch (`script_name` loaded, unused);
  no escort/follower state machine (waypoints
  exist; `script_escort_data`/`script_waypoint`
  load state unverified); no GO `UpdateAI`
  ticking beyond the gunpowder spline special
  case; no `pProcessEventId` dispatch
  (`scripted_event_id` table exists).

Execution order (kind-first; wonder builds
each framework + first exemplar, spark
Workflow lanes transcribe per-file after):

1. **Instance generalization** (wonder):
   load `map_template.script_name`, replace
   hardcoded `deadmines` with name-dispatched
   instance state enum, port one small second
   instance end-to-end (Shadowfang Keep) with
   a `live_*` test. Then spark lanes per
   instance batch (instance file + its GO/
   creature files travel together).
2. **GO hello/AI** (wonder): generalize
   `gameobject_use` dispatch + GO update tick
   state; exemplar from the SFK batch.
3. **CreatureAI framework** (wonder):
   `ScriptedAI` lifecycle in `ai.rs`
   (reset/aggro/update/just-died/killed-unit/
   movement-inform/damage-taken hooks +
   per-script state enum), plain-boss
   exemplar; escort/follower framework as its
   own sub-exemplar (`npc_escortAI` is 26
   files and DB-driven waypoints).
4. **Spell/Aura scripts** (wonder census
   first): load `spell_template.script_name`,
   dispatch beside `customs.rs`; class-spell
   files partially overlap Tier 2 aura/effect
   handlers — census per bound name decides
   transcription vs already-covered.
5. **Gossip/quest callbacks** (wonder):
   engine hooks in the mangosd gossip/quest
   handlers, exemplar NPC, then lanes.
6. **Areatrigger + event-id + dummy
   delegation** (wonder): wire the three
   remaining dispatch points; small counts,
   mostly mechanical after.

Verification per batch: `./ci.sh`, a
`live_<instance>` in-process test in the
`live_deadmines` pattern per dungeon batch,
wire probe where player-visible, bench
restart + two-client smoke at batch close.

## Slice 2 step 1 — instance host + SFK (done)

Implemented (wonder):

- Instance host generalization:
  `Map.instance: Option<InstanceState>` enum
  (`content.rs`), created in `Map::new` from
  `map_template.script_name` when
  `instance_id != 0` (fork `CreateInstanceData`
  gate subset; continent-named scripts remain
  uncreated — recorded). Dispatchers
  `instance_on_creature_create/on_object_create/
  on_creature_death/on_creature_enter_combat
  (new hook, fired from `ai.rs::enter_combat`
  fresh-aggro)/set_data/get_data/get_data64/
  update/save/load`; Deadmines bodies renamed
  `deadmines_*` unchanged. `live_deadmines`
  now loads real `map_template` (was default).
- `instance_shadowfang_keep` full port:
  encounter[6] + GUID registry, hidden wolf
  guards (faction 35→17 reveal 6 s after
  Baron/Cmd death, `spawntimesecsmin`
  7201/7202 markers, timer stalls while boss
  object gone — fork quirk), Vincent dead
  stand-state + Arugal hidden after intro,
  door open-on-load arms, Fenrus howl 6017,
  voidwalker counter door, Save/Load stream
  (IN_PROGRESS→NOT_STARTED). Persistence of
  the save string to DB is NOT wired (no Rust
  instance-save plumbing exists yet —
  recorded framework gap, same as Deadmines).
- `spell_haunting_spirits` AuraScript + fork
  periodic-dummy plumbing: `SPELL_AURA_DUMMY`
  added to `AuraEffectState::is_periodic`,
  `process_periodic_ticks` DUMMY arm →
  `periodic_dummy_tick` (7057: 5% cast 7067),
  creation marks (7057, eff 0) periodic with
  5 s period per `SetPeriodicTimer` semantics.
  The fork hardcoded `PeriodicDummyTick`
  switch remainder (7054, 8067, 21051, 21094,
  23487, 21827, 21863, 24596, 29705/29726/
  29727, paladin family 20425/20961/20962/
  20967/20968) lands with step 4 census.

Verification: `live_shadowfang`
`LIVE-SHADOWFANG-PASS` (Vincent bytes1=0x7,
guard reveal+faction, Fenrus howl packet,
doors, save/load, haunt summon entry 4958;
DB death script set TYPE_FENRUS through the
generic path unprompted), `live_deadmines`/
`live_spells`/`live_motion`/`live_ai` green,
`./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s2a.log`),
bench restarted, `TWO-CLIENT-SMOKE-PASS`.

## Slice 2 step 1b — instance batch 1 (done)

Spark Workflow lanes transcribed WC/RFK/RFD
instance files to scratch; wonder reviewed
line-by-line vs fork and integrated with
fixes (lane errors caught in review: RFK u8
ward counter → u32 wrapping; RFK invented
`rfk_save` — fork has no `Save()` override,
dropped; RFD invented `rng.urand` →
`spell_rng().irand(-5,5)`; RFD missed existing
`set_walk` helper; WC closest-druid selection
made min-dist). Framework additions used:
`creature_move_waypoint` for the Agathelos
kick (`SetDefaultMovementType` unmodeled),
`set_walk` for gong waves.

Recorded remainders from this batch:
- WC: `SetDefaultGossipMenuId(202)` on the
  disciple (gossip framework, step 5);
  `at_dmf_chest_wc` areatrigger script
  (step 6); Mutanus dead-corpse despawn arm
  needs loot emptiness (Map holds no loot
  state; alive-only despawn today).
- RFD: `SetLootState(GO_JUST_DEACTIVATED)` on
  cup fires (no GO loot state in Rust);
  `go_gong` GOHello caller in
  `razorfen_downs.cpp` (GO step 2 batch).
- WC/RFD/RFK companion creature/escort/GO
  files (`wailing_caverns.cpp`,
  `razorfen_kraul.cpp`, `razorfen_downs.cpp`)
  port with steps 2–3 (escort AI + GOHello).

Verification: `live_instance_batch1` —
`LIVE-WAILING-CAVERNS-PASS`,
`LIVE-RAZORFEN-KRAUL-PASS`,
`LIVE-RAZORFEN-DOWNS-PASS` (wave 1 = 8 fiends
= leader + 7, gong lock/unlock flags, ward
countdown over 2 bench keepers, disciple
SPECIAL + save string + Load reset);
regressions green (`live_deadmines`,
`live_shadowfang`, `live_spells`, `live_ai`);
`./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s2b.log`);
bench restarted, `TWO-CLIENT-SMOKE-PASS`.

## Slice 2 step 1c — instance batch 2 (done)

Spark lanes transcribed Onyxia's Lair,
Maraudon, Scholomance, Gnomeregan instance
files; wonder reviewed vs fork and integrated
with fixes:
- Save streams byte-checked per file
  (Maraudon/Scholomance loop streams keep the
  fork trailing space; Gnomeregan separator
  form kept).
- Door/button toggles that must CLOSE
  (Kirtonos gate on IN_PROGRESS, Gnomeregan
  bomb-face deactivate) write `GO_STATE_READY`
  directly — Rust `use_door` is open-only,
  fork `DoUseDoorOrButton` toggles.
- Scholomance `GetData(TYPE_GANDLING)` side
  effect (SPECIAL + summon) reproduced via
  read-only derivation in the getter plus the
  fork's own `SummonGandlingIfPossible` call
  at the end of `SetData`/`Load`.
- Maraudon `OnCreatureRespawn` needs no
  separate dispatcher: respawns re-enter via
  `instance_on_creature_create`, whose larva
  arm is identical.

Recorded remainders from this batch:
- Onyxia: whelp-spawner GO `CastSpell(17646)`
  on object create (no gameobject-cast path
  in Rust); `IsEncounterInProgress` (no raid
  lockout consumer in Rust yet).
- Maraudon: `SendGameObjectCustomAnim` spew
  animation dropped (timer/flag kept); larva
  `Respawn()` rides `despawn_creature_now` +
  `respawns_due` re-materialization instead
  of same-object revival; corrupted-vine
  removal is immediate vs fork end-of-tick
  remove list.
- Scholomance: Kirtonos brazier
  `ResetDoorOrButton` on FAIL (no door-reset
  helper / GO loot state); `go_brazier_herald`
  and `go_viewing_room_door` companion GO
  scripts port with step 2.
- Gnomeregan: `DoRespawnGameObject(HOUR)` for
  red rockets + sorted charges (no GO respawn
  helper; tracking kept); explosive-charge
  `Use(blastmaster)` creature-as-user GO use;
  final-chamber door `ResetDoorOrButton` /
  loot-state gates (LOCKED flag writes kept);
  `GetBombFaces()` raw pointer has no Rust
  equivalent — bomb-face state lives in
  `GnomereganState` for the Thermaplugg lane.
- Companion boss/escort files for all four
  dungeons port with steps 2–3.

Verification: `live_instance_batch2` —
`LIVE-ONYXIA-PASS`, `LIVE-MARAUDON-PASS`,
`LIVE-SCHOLOMANCE-PASS` (Gandling actually
summoned at the study after six DONEs, save
string captured pre-flip), `LIVE-GNOMEREGAN-PASS`
(charge field 3 correctly does not refresh
save); regressions green (batch1, Deadmines,
Shadowfang, spells, AI, motion); `./ci.sh`
`CI_EXIT=0` (`/tmp/ci_t3s2c.log`); bench
restarted on the new binary (LISTEN 18099),
`TWO-CLIENT-SMOKE-PASS`.

### Batch 3 (2026-10-28): Zul'Farrak, Blackfathom Deeps

Spark lanes (2) via Workflow, wonder-reviewed
line-by-line against fork cpp+h and integrated
(`content.rs` `zf_*`, `bfd_*`;
`InstanceState::ZulFarrak/BlackfathomDeeps`).

Review fixes beyond lint: ZF save refresh fires
on ANY field set DONE (fork cpp 187-195 streams
EndDoorEncounter; lane had narrowed it to the
end-door field — including the EVENT_PYRAMID =
WAVE_1 == 3 coincidence); BFD wave-mob home
anchor is Kelris `home_position` with facing 0
(`GetRespawnCoord` + `SetHomePosition(x,y,z,0)`;
lane used current position); BFD 5th-shrine
IN_PROGRESS index guarded (fork ASSERT/UB) as a
no-op instead of a panic.

Recorded remainder:
- ZF `MoveNPCIfAlive` `SetCombatStartPosition` +
  `SetHomePosition` (cpp 290-291): no Map
  helpers; NPCs move but keep old home/combat-
  start anchors.
- BFD `SetInCombatWithZone` (cpp 295): no
  helper; wave mobs walk home instead of
  zone-aggroing. `go_fire_of_akumaiAI::OnUse`
  (cpp 355-375) is a GO script — Slice 2 step 2.
- Companion boss files port with steps 2-3.

Verification: `live_instance_batch3` —
`LIVE-ZULFARRAK-PASS` (pyramid wave 1 = +23
creatures, any-DONE save refresh "0" -> "3",
Load keeps only DONE), `LIVE-BLACKFATHOM-PASS`
(shrine wave 0 = +4 snapjaws after 3s timer,
save "3 1 0", Load resets IN_PROGRESS);
regressions green (batch1, batch2, Deadmines,
Shadowfang, spells, AI, motion); `./ci.sh`
`CI_EXIT=0` (`/tmp/ci_t3s2d.log`); bench
restarted on the new binary (LISTEN 18099),
`TWO-CLIENT-SMOKE-PASS`.

### Batch 4 (2026-10-28): Scarlet Monastery, Zul'Gurub, Uldaman, Sunken Temple

Spark lanes (4) via Workflow, wonder-reviewed
line-by-line against fork cpp+h and integrated
(`content.rs` `sm_*`, `zg_*`, `uld_*`, `st_*`).
New shared machinery this batch (wonder-built):
`Creature::SetInCombatWithZone` reduced helper
(`ai.rs::set_in_combat_with_zone`, also retro-wired
into BFD wave spawns); `ZoneScript::OnCreatureSpellHit`
dispatch (`instance_on_creature_spell_hit`, hooked in
`spells.rs::apply_effects` beside the EventAI hooks);
`instance_set_data64`, `instance_is_encounter_in_progress`,
and `instance_check_condition` dispatchers;
`SpellAuraHolder::SetStackAmount` reduced helper
(`spells.rs::set_aura_stack_amount`);
`collect_targets` now resolves `LOCATION_CASTER_SRC`
(A=22) through implicit target B, including script
AoE (`SPELL_TARGETS_ALL` at source, caster excluded).

Review fixes beyond lint: SM fork `Save()` streams
the live encounter pair (no cached string; lane had
one) and Ashbringer door writes are direct
`GO_STATE_ACTIVE`; SM `OnCreatureSpellHit` (spell
28441 on Mograine) transcribed by hand (the lane
contract had no slot for it). ZG `Create()` runs
lazily on first update with the real game-day roll
((unix/86400) mod arithmetic), `SPECIAL` death/rez
times store `now_unix` truncated to u32 exactly as
the fork, Hakkar stacks follow fork arms (cast when
absent, `SetStackAmount` when adjusting). Uldaman
`RespawnMinion`/FAIL implemented as despawn-now +
refreeze-on-create (immune bits only, or full
`SetFrozenState` for FAIL) with deduped registries;
stone-keeper victim search reduced to nearest living
player (fork `SelectNearestTarget`). ST zone-combat
arms wired through the new helper.

Recorded remainder:
- Global: modifier magnitudes do not scale by aura
  stack in the Rust aura system (`SetStackAmount`
  writes the count + slot byte only); aura refresh
  never increments stack. Pre-existing engine gap,
  not ZG-specific.
- `spell_script_target` bounds filtering for script
  AoE not modeled; `CSTATE_COMBAT_WITH_ZONE` leash
  state and pet `EnterCombatWithTarget` in
  `SetInCombatWithZone` not modeled.
- SM: Whitemane `SetDeathState(JUST_DIED)`+`Respawn()`
  reset arm; chapel door loot-state/IN_USE writes;
  `SetFacingToObject`; `HandleEmote` one-shots;
  `SetFactionTemporary`/`SetDisplayId`/`SetObjectScale`
  on the Highlord apparition;
  `AreaTrigger_at_cathedral_entrance` (areatrigger
  kind, step 6).
- ZG: `sGameEventMgr` random-boss reroll on
  `SetData(RANDOM_BOSS, 0)`; Mar'li trash zone/area
  gate (1977/3379); `ForcedDespawn(3000)` delay;
  `go_table_madness` gossip + Gahz'ranka summon
  event (non-instance scripts); first-tick-late
  `Create()` visibility of encounter[9].
- Uldaman: force-respawn lands a tick later than
  fork synchronous `Respawn()`; `SetFactionTemporary`
  restore semantics (sticky faction 470 kept);
  `SummonGameObject` Ancient Treasure (no temp GO
  summon helper).
- Sunken Temple: `DoRespawnGameObject` idol/lights
  (guid tracking kept); trap `Use(Atalarion)`
  creature-as-user GO use; `RemoveAllObjectsIn
  RemoveList` (Rust removal is immediate).
- Companion boss/escort/GO files port with steps 2-3.

Verification: `live_instance_batch4` 4/4 —
`LIVE-SCARLET-MONASTERY-PASS` (live Save "0 0" ->
"4 3", Load resets, 28441 on Mograine summons the
Highlord), `LIVE-ZULGURUB-PASS` (game-day roll
15082 at bench time, Hakkar stacks 5 -> 4 across a
priest DONE + pulse, no re-roll after Load),
`LIVE-ULDAMAN-PASS` (set_data64 waker round-trip,
Ironaya unfreezes to faction 415 after the 27s
keystone timer), `LIVE-SUNKEN-TEMPLE-PASS` (wrong
statue starts but does not advance, correct order
completes the circle and reveals Atal'arion);
regressions green (batches 1-3, Deadmines,
Shadowfang, spells, AI, motion); `./ci.sh`
`CI_EXIT=0` (`/tmp/ci_t3s2e.log`); bench restarted
(LISTEN 18099), `TWO-CLIENT-SMOKE-PASS`.

### Batch 5 (2026-10-28): Ruins of Ahn'Qiraj, Molten Core, Blackrock Spire, Temple of Ahn'Qiraj

Spark lanes (4) via Workflow, wonder-reviewed
against fork cpp+h and integrated (`aq20_*`,
`mc_*`, `brs_*`, `aq40_*`). New machinery:
`InstanceData::OnCreatureEvade` dispatch
(`instance_on_creature_evade`, hooked at the top
of `ai.rs::enter_evade_mode`). AQ20's evade body
was hand-transcribed at integration (the lane
contract predated the dispatcher).

Review fixes beyond lint: MC fork `Save()` also
recomputes live (lane cached it); MC DONE-gated
trash removals use never-respawn despawn (lane's
0-second respawn would hot-loop through the
create hook); push dedupes for respawn re-entry
(AQ20 elites, BRS room mobs, AQ40 guards/grasps);
AQ40 whisper borrow fix.

Recorded remainder:
- DialogueHelper (fork timed dialogue sequencer):
  BRS stadium intro/waves and AQ40 twins intro/dead
  dialogues unwired; `brs_just_did_dialogue_step`,
  `brs_speaker_for_entry`, `brs_sort_room_event_mobs`
  are integrated as pub methods awaiting the
  dialogue/areatrigger dispatchers (Slice 2 steps
  3/6 exemplars). Same for AQ40 areatrigger entry
  points (`aq40_handle_temple_area_trigger`,
  `aq40_handle_stomach_trigger`).
- Entry-swap (`UpdateEntry`/`SetEntry`/
  `AIM_Initialize`): MC annihilator/firelord swap,
  AQ40 trash randomization, BRS Bannok swap (the
  5% roll + flag are kept).
- `OnCreatureRespawn` as a distinct event: Rust
  re-enters through the create hook (MC/AQ40
  DONE-gated removals ride it, Maraudon precedent);
  respawn-only extras (MC Majordomo/Ragnaros DONE
  marking on respawn) not distinguished.
- AQ20: Ossirian crystal system (`SpawnNewCrystals`
  + pylon `OnObjectCreate`) needs free-position
  `SummonGameObject`; Rajaxx rep handout (no
  reputation helpers); Andorov group spawn
  (`LoadCreatureSpawnWithGroup`, spawn_flags=2 row)
  + `SetRespawnTime` family; gossip menu ids.
- MC: rune `go_rune_MC` hello/UpdateRune (step 2);
  `DoRespawnGameObject` Firelord cache.
- BRS: areatrigger/GO-AI scripts in the same file
  (steps 2/6); `SetFacingTo`, detection-distance /
  no-call-assistance setters, creature groups.
- AQ40: Ouro spawner `Respawn()`; stomach
  air-proximity hack (launch/fall state); C'Thun
  whisper charm (fork ToDo).

Verification: `live_instance_batch5` 4/4 —
`LIVE-MOLTEN-CORE-PASS` (live Save 17 slots, Garr
GetData64, rune IN_PROGRESS survives Load),
`LIVE-RUINS-OF-AHNQIRAJ-PASS` (gladiator counter,
cached save refresh discipline, Andorov DONE
load-reset), `LIVE-BLACKROCK-SPIRE-PASS` (UBRS
brazier/door sequence opens the door on timer),
`LIVE-TEMPLE-OF-AHNQIRAJ-PASS` (bug-trio SPECIAL
counting, trailing-space save, bounds-guarded
GetData/CheckCondition); regressions green
(batches 1-4, Deadmines, Shadowfang, spells, AI,
motion); `./ci.sh` `CI_EXIT=0`
(`/tmp/ci_t3s2f.log`); bench restarted (LISTEN
18099), `TWO-CLIENT-SMOKE-PASS`. Five instance
scripts remain (Stratholme, BRD, BWL, Naxxramas,
Dire Maul).

### Batch 6 (2026-10-28): Stratholme, Blackrock Depths, Dire Maul, Blackwing Lair, Naxxramas — instance corpus complete

Spark lanes (5) via Workflow, wonder-reviewed
against fork cpp+h and integrated (`strat_*`,
`brd_*`, `dm_*`, `bwl_*`, `naxx_*`). All 24
`instance_*` scripts in `vmangos/src/scripts` now
have a Rust port behind the generalized instance
host.

New shared dispatchers (driver-built exemplars for
the hook kinds):
- `ZoneScript::OnPlayerEnter/OnPlayerLeave`
  (`instance_on_player_enter/leave`, hooked at the
  end of `Map::add_player` / head of
  `Map::remove_player`). First consumer: Dire
  Maul (inner-door-key sweep via a new map-side
  `DestroyItemCount` over the saved inventory +
  `SetItemCount` events; King-of-the-Gordok strip
  on leave).
- `ZoneScript::OnPlayerDeath`
  (`instance_on_player_death`, hooked in
  `spells.rs::kill_unit` beside the creature-death
  dispatch; kill_unit is Rust's single death
  funnel). First consumer: Naxxramas corpse
  scarabs (hand-written body).
- `InstanceData::CustomSpellCasted`
  (`instance_custom_spell_casted`): the fork's
  only call site is the hardcoded spell-27517
  dummy arm in `SpellEffects.cpp:567`, so the Rust
  hook fires per hit target only for 27517 in
  `apply_effects`. Consumer: BRD Theldren
  invocation (invocated guard + challenger guid).

Driver fixes at integration: BWL breath/color
rolled lazily on first creature create and marked
rolled on `Load` (fork rolls `urand(0,19)` in
`Initialize`, cpp 190-191; the state ctor has no
RNG — ZG lazy-create precedent); BWL evade
immunities ride `apply_aura_by_id`
(`AddAura(ADD_AURA_PERMANENT)` equivalent) instead
of the lane's empty arms; DM `GetChoRushEquipment`
kept pub for the unported `boss_chorush`; BRD
`DoSummonCreatureAndAttack` kept `#[allow(dead_code)]`
(no fork callers either).

Recorded remainder:
- BWL scalar `GetData64` slots (`DATA_EGG`,
  `DATA_HOW_EGG`, breath/color, scepter run time)
  cannot ride `Option<ObjectGuid>`; step-3 boss
  ports will read state fields directly.
  `CONDITION_SCEPTER_WIN` needs the asking player's
  guid, which `instance_check_condition` does not
  carry (reads false). Entry-swap whelps,
  `ForcedDespawn(1000)` delay, egg-GO summon,
  technician threat-helper (`OnPlayerDeath` body
  is bookkeeping on that helper only).
- BRD: `ReplacePrincessIfPossible` (team/quest-
  reward entry-swap), kill-credit + arena-spoils
  `DoRespawnGameObject`, `SetRespawnDelay` lyceum
  sweep, wander/`MoveFollow` spirits,
  `GetRandomPoint` (irand jitter approximates).
- Dire Maul: broken-trap re-arm on enter
  (`SummonGameObject` fixed trap + Delete; only
  the key sweep is ported), tribute/shard
  `DoRespawnGameObject`, `SetFactionTemporary` +
  delayed Cho'Rush yell (no temp-faction or
  delayed-execution helper).
- Stratholme: `CheckCondition` kinds needing
  player/quest state (Dead-Man's-Plea credit,
  kindness summoners), GM/visibility/targetable
  gates in the fate/gate-trap updaters,
  `GetRandomPoint`, Ysida `CastSpell(5)` literal
  kept as-is.
- Naxxramas: areatrigger greetings/DK-wing intro
  (step 6), DialogueHelper users, Gothik sub-boss
  trigger list, FAIL wipe statistics/rep handout/
  horsemen chest respawn, dead-boss `Respawn()`
  loop, Heigan trap registry (boss-side),
  `SelectAttackingTarget(RANDOM)` for corpse
  scarabs (current victim used instead), scarab
  `SendSpellGo` counter fixed at 0.

Verification: `live_instance_batch6` 5/5 —
`LIVE-STRATHOLME-PASS` (trailing-space 11-slot
save, Baron GetData64, Load resets),
`LIVE-BLACKROCK-DEPTHS-PASS` (Thunderbrew triple
count with passed-DONE-only save refresh, 20-slot
separator save, live 27517 cast starts Theldren
and stores the challenger),
`LIVE-DIRE-MAUL-PASS` (Cho'Rush equipment roll
cached 1..=3, sparse 10-token Load slot map, King
aura stripped on leave, inner door key destroyed
on re-enter through real add/remove_player),
`LIVE-BLACKWING-LAIR-PASS` (scepter run excluded
from IsEncounterInProgress, champion+FAIL drives
CheckCondition, Razorgore GetData64, 14-token
save with rolled breath/color ≤19),
`LIVE-NAXXRAMAS-PASS` (horsemen SPECIAL×4,
trailing-space save, Kel'Thuzad SPECIAL dropped
while the player is away from the chamber center
and accepted in range, Thaddius SPECIAL→FAIL on
Load, player death summons exactly 5 corpse
scarabs); regressions green (batches 1-5,
Deadmines, Shadowfang, spells, AI, motion);
`./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s2g.log`);
bench rebuilt + restarted (LISTEN 18099),
`TWO-CLIENT-SMOKE-PASS`.

## Slice 2 step 2 — GO hello/AI framework (done)

Driver-built framework generalizing the three
ad-hoc Deadmines arms in `gameobject_use`:

- `GoAiState` enum + `for_script` (mirrors
  `InstanceState`), stored per-GO in
  `Map::go_ai`, created in the GO materialization
  funnel (`go_ai_create`, beside
  `instance_on_object_create`), removed in
  `despawn_gameobject_now`, ticked from
  `Map::tick` (`go_ai_update`, fork
  `GameObject::Update` → `UpdateAI`).
- `gameobject_use` now follows fork
  `GameObject::Use` order: `GameObjectAI::OnUse`
  (`go_ai_on_use`) → `pGOHello` (`go_hello`) →
  default door/button toggle subset. Handled =
  true return skips the next stage, exactly as
  the fork.
- `SummonedMovementInform` generalized
  (`go_ai_summoned_movement_inform`); the ai.rs
  point-arrival path no longer special-cases the
  gunpowder script name.

Exemplars ported (wonder, first-of-kind each):
- `pGOHello`: Deadmines lever/cannon/gunpowder
  re-ported literally (fork return values now
  honored: cannon falls through, lever blocks
  unless the foundry door is open, gunpowder
  requires the instance). The Phase-4 subset's
  invented "lever opens iron door" behavior is
  gone; fork opens that door through the button
  `Use` tail (DB script start + linked trap),
  still unmodeled (recorded below).
- `pGOHello` new: `go_rune_MC` (+ `UpdateRune`)
  from the MC instance cpp — closes the batch-5
  remainder; Majordomo summon incl. the
  crash-recovery field-write branch.
- `GOGetAI` `OnUse`-only: `go_fire_of_akumai`
  (BFD instance cpp).
- `GOGetAI` `UpdateAI` + state:
  `go_containment_coffer` (20s despawn) and
  `go_lunar_festival_firecracker` (ctor-armed
  urand(30,60)s for the two static entries,
  `OnUse` re-arms urand(0,2)s).

Recorded remainder:
- DB-GO `Despawn()` respawn cycling is
  unmodeled system-wide (`despawn_gameobject_now`
  removes permanently; fork reschedules per DB
  respawn timers). Firecracker world spawns
  therefore won't cycle until that system lands;
  the same gap underlies the instance
  `DeleteLater` precedents.
- Coffer/firecracker despawn tick has no live
  test: every binding spawn is game-event-gated
  (bench loader default-spawns only
  game_event=0) and spell GO-summon
  (`Create Containment Coffer` 9082) is
  unmodeled. Creation + `OnUse` dispatch are
  proven live via Fire of Aku'mai; the countdown
  arm is code-reviewed against the fork bodies.
- `UseDoorOrButton` full semantics (auto-close
  cooldown restore, `GO_FLAG_IN_USE`, linked
  trap trigger, `ScriptsStart(sGameObjectScripts)`)
  remain subset; trap-trigger `OnUse` gate
  (fork GameObject.cpp trap path) not wired —
  Rust trap firing does not route through
  `gameobject_use`.
- `GameObjectAI` surfaces not yet dispatched:
  `SetData`, `OnActivateBySpell`, `JustSummoned`,
  `SummonedCreatureJustDied`, `OnRemoveFromWorld`
  (add when a ported script reaches them).
  `pGOOpen` (3 files, spell-effect call site)
  rides step 4/6.

Verification: `live_go_scripts` 2/2 —
`LIVE-GO-FIRE-OF-AKUMAI-PASS` (OnUse refuses
before Kelris DONE; after: ACTIVE +
NO_INTERACT + shrine IN_PROGRESS through the
real `gameobject_use`), `LIVE-GO-RUNE-MC-PASS`
(douse refused with boss alive, accepted after
DONE, all-runes gate summons Majordomo —
found live at the fork coordinates);
regressions green (Deadmines — the use-path
rewrite — batches 3/5, spells, AI); `./ci.sh`
`CI_EXIT=0` (`/tmp/ci_t3s2h.log`); bench rebuilt
+ restarted (LISTEN 18099),
`TWO-CLIENT-SMOKE-PASS`.

#### GO lane corpus (step 2 spark lanes, driver-reviewed)

10 lane files (`go_world_scripts`, `go_blackrock_depths`,
`go_stratholme`, `go_scholomance_zf`, `go_dreadsteed`,
`go_ek_misc_a/b`, `go_kalimdor_a/b`, `go_world_events`)
spliced into `content.rs` (22873 lines); 31 new
`GoAiState` variants; take-out/restore dispatch in
`go_ai_update`/`go_ai_on_use`; `go_ai_just_summoned`
hooked in `summon_creature` (GO summoner),
`go_ai_summoned_creature_just_died` in `kill_unit`,
Darrowshire `OnRemoveFromWorld` in
`despawn_gameobject_now`; ctor hooks at
materialization (pedestal timers, corrupted-plant
pairing, ghost-magnet scan, scourge docks,
Darrowshire reset). Driver fixes on top of lanes:
dreadsteed ring re-reads GO position per summoned
imp (fork cpp 421) + home facing = ring angle;
phase-4 wave arm narrowed to fork cases 0/1;
Stratholme postman orientation 0.0; shallow-grave
`use_count` gate; quest gates 7046/5151/2882;
lards hello state unified; rift index order
FIRE=0/AIR=1/EARTH=2/WATER=3.

Verification: workspace clippy `-D warnings`
EXIT=0; `live_go_scripts` 5/5 — fire-of-akumai,
rune-MC (pre-lane) + `LIVE-GO-HEROD-LEVER-PASS`
(door READY→ACTIVE→READY through `gameobject_use`),
`LIVE-GO-STRATHOLME-POSTBOX-PASS` (data 9:
1 IN_PROGRESS + 3 Undead Postmen live → 4 SPECIAL
→ 3 DONE, via the `go_open` dispatcher),
`LIVE-GO-VIEWING-ROOM-DOOR-PASS` (data 14 → DONE).
Regressions green: deadmines, batches 1-6, spells,
AI, motion (shadowfang flaked once in the batch
loop, green 3x standalone after); `./ci.sh`
CI_EXIT=0 (`/tmp/ci_t3s2i.log`); bench rebuilt +
restarted (LISTEN 18099), TWO-CLIENT-SMOKE-PASS.

GO lane remainders (recorded, not dropped):
- World: repair-bot skill 202/>=300 gate;
  silithyste session log + `SetLootState`;
  bells/darkmoon game-event gated.
- BRD: `SummonGameObject` nameplates; motion
  clear/facing/temp-faction in creature bodies
  sharing the file (step 3).
- Stratholme: scarlet cannon temp GO + Use;
  `SetLootState`; `GetRandomPoint` reduced to
  `irand(0,2)` square jitter (postman summons).
- Scholomance/ZF/RFD: table_theka quest 2936 +
  gossip 1653 (step 5); `SetCombatStartPosition`.
- Dreadsteed: `ProcessEventId` start/second-part
  unwired (step 6); GO-caster casts; flame ring
  x18 `SummonGameObject`; respawn/refresh/custom
  anim; `SetFacingToObject`; guid-low
  99774-99782 lookup folded to entry match.
- EK misc: Urok/Darrowshire/Helcular/Dusty
  cross-script dispatchers partially wired
  (quest-reward callers step 5, banner DB event
  step 6); `SummonGameObject`; respawn-time/
  refresh; quest/team/GM defender scan
  (Darrowshire Reset); herod lever loot-state
  approximated via `go_state`; transpolyporter
  has no hello body in the fork.
- Kalimdor: Foulweald/Ghost-Magnet/Celebras
  creature-side notifications (step 3);
  `SummonGameObject`; custom anim; respawn
  delays; corrupted-plant `pQuestRewardedGO`
  (step 5); Ossirian `SpawnNewCrystals` — and
  the whole Ossirian slice has no spawns on the
  bench DB (no 12261/crystal rows), so it is
  code-reviewed only.
- World events: all event-gated (68-71, 6);
  saved vars 30008-30015; stage-up; GO-caster
  28344; visibility modifier — inert until the
  game-event system exists (`game_event_active`
  returns false).
- `go_open` still has no Rust call site (fork
  caller is the spell open-lock effect,
  SpellEffects.cpp:2120, unmodeled); postbox /
  brazier-herald / shallow-grave bodies are
  wired behind it and postbox is live-proven
  through the dispatcher.

#### Step 3: CreatureAI framework + Herod exemplar (driver-built)

Fork census: 321 `GetAI` registrations (dominant
kind). Rust seam generalized from the Deadmines
exemplar: `CreatureAi.smite: Option<SmiteState>`
became `script: Option<CreatureScriptAi>` (one
variant per script, `for_script` by
`creature_template.script_name`, mirroring
`GoAiState`). Dispatchers on `Map`:
`creature_script_reset` (ctor + evade),
`creature_script_aggro` (fresh `enter_combat`,
beside `eventai_enter_combat`),
`creature_script_update` (replaces the generic
combat loop like `update_smite` did),
`creature_script_enter_evade` (pre-base at the
old smite-reset site), `creature_script_just_died`
+ `creature_script_killed_unit` (`kill_unit`,
before `ai.remove`), `creature_script_just_summoned`
(creature summoner in `summon_creature`),
`creature_script_movement_inform` (point motion,
absorbs the smite spline special-case). Shared
`script_combat_drive` = smite's hostile-target /
evade / proximity / leash / attack-start / chase
head; melee swings tick globally so fork
`DoMeleeAttackIfReady()` transcribes to nothing.
`smite_health_percent` generalized to
`creature_health_percent`.

Exemplar: `boss_herod` + `mob_scarlet_trainee`
(full fork file; lever already ported in step 2).
Fork values verbatim (timers, 6194/6534/6195/6196/
7798/2842 texts, 8260/15496/8989/8269, myrmidon
spawn 1926.03/-370.61/18 ±frand 3, room center
1965.09/-431.61 r32, trainee point chains).
Remainders inside the exemplar: `SetNoXP` (no
no-XP flag in Rust creature state);
`IsNonMeleeSpellCasted` reduced to the active-cast
store; `IsInRange` bounding-radius fold unmodeled
(center distance vs `MELEE_RANGE`+10 = 11.0);
`UNIT_STATE_ROOT` does not gate Rust motion yet —
whirlwind cast also `stop_moving`s and the update
skips the chase drive while rooted (fork effect);
trainee ctor `urand(1000,6000)` rolled at first
reset dispatch (no-RNG-in-`new` rule).

Verification: `live_creature_scripts` —
`LIVE-BOSS-HEROD-PASS` (proximity aggro through
the script drive → `Aggro` spawns myrmidons found
live at the fork point → 40% set-health → Frenzy
8269 aura → spell-kill → trainee 6575 found live
+ door 101854 ACTIVE). Test-harness note: melee
swings whiff vs +levels and relocate/evade cycles
regen the boss — kills use the batch-test
direct-damage pattern with per-tick tracking.
Regressions green (deadmines — the smite refactor —
shadowfang, GO 5/5, batches 1/6, spells, AI,
motion); clippy workspace `-D warnings` 0;
`./ci.sh` CI_EXIT=0 (`/tmp/ci_t3s3a.log`); bench
rebuilt + restarted (LISTEN 18099),
TWO-CLIENT-SMOKE-PASS.

## Slice 2 step 3b: EK CreatureAI lanes + dispatcher batches

11 spark lanes (Workflow, wonder exemplar = step 3a)
transcribed the Eastern Kingdoms creature corpus to
`/tmp/tier3-lanes/cai_*.rs` (34,717 lines); driver
spliced 185 scripts + the rotting/diseased maggot pair
into `content.rs` with a fork `GetAI` manifest
cross-check (`/tmp/cai_manifest.json`). Uncovered by
design (recorded remainders, not transcribed):
escort-framework scripts (grark_lorkrub, windsor
escorts, emi_shortfuse, kernobee, galen, daphne,
ek_zones escort set) — the fork `EscortAI` waypoint
runtime has no Rust host yet. Lane NEEDS-MAP-HELPER
density is highest in cai_bwl/cai_naxx/zg (GO-caster
casts, `SummonGameObject`, SetFacingTo, UpdateEntry/
morph, respawn-delay/visibility overrides, threat-wipe
variants); each site carries an inline
`NEEDS-MAP-HELPER` note in the spliced body. EK lane
bodies are compile+manifest verified; per-lane fork
value spot-review is rolling (herod/anubrekhan/
core-rager done; timer `< diff` vs `<= diff` semantics
checked where probed). Batch4 expectation moved with
the port, fork-verified: Hakkar script `Aggro` sets
`TYPE_HAKKAR IN_PROGRESS` (`boss_hakkar.cpp:123`),
save `3 0 0 0 0 0 0 0 1 15082 0 0`.

Dispatcher batch 1: `SummonedCreatureJustDied`
(kill_unit summoner fan-out), `SummonedMovementInform`
(point arrival), `SummonedCreatureDespawn`
(`despawn_creature_now` summoner branch),
`JustRespawned` (`update_respawns` after spawn),
EnterCombat-as-Aggro arms (fork `ScriptedAI::
EnterCombat` -> `Aggro`). Batch 2 (130 arms):
`DamageTaken` + `AttackedBy` in `deal_damage_school`
(after immunity backstop, before invincibility
threshold; `&mut damage`, zero cancels), `SpellHit` /
`SpellHitTarget` beside the EventAI hooks in
`apply_effects` (fork `Spell::DoAllEffectOnTarget`
gates: victim/caster creature, no self-exclusion),
`JustReachedHome` in `process_home_arrivals`,
`MoveInLineOfSight` + `AttackStart` overrides in
`script_combat_drive`. `KilledUnit` gating fixed to
fork: fires for any victim type when the killer is a
creature (was creature-victims only).

Drive semantics recorded: `MoveInLineOfSight`
candidates are players within the visibility distance,
polled per tick while out of combat (fork fires on
sight-entry edge via `AIRelocationNotifier`, for any
unit type); bodies gate on their own ranges. The
per-tick poll + players-only surface is the recorded
subset. `AttackStart` override returning handled
skips the drive chase sync for that victim change.

Root fix reached by the batch: generic `attack_start`
only set the melee victim, so script bodies
transcribing fork `CreatureAI::AttackStart` (which
enters combat: threat + `Aggro`) pulled bosses that
then froze out of combat (Anub'Rekhan at spawn,
meleeing without ever setting IN_PROGRESS). Creature
attackers out of combat now route through
`enter_combat` (pets excluded; PetAI owns their
entry). The batch6 scarab regression under this batch
was a test-harness artifact: scarabs summon correctly
on `OnPlayerDeath` (probed: 5 alive at corpse), but
the revived fixture relocated to its exact death
position so no visibility scan was due; the test now
relocates with an offset.

Live proof (`live_creature_scripts`):
`LIVE-ANUBREKHAN-LOS-PULL-PASS` (player 40yd out —
past aggro range, inside the script's 55yd pull —
boss engages, `Aggro` sets TYPE_ANUB_REKHAN
IN_PROGRESS), `LIVE-CORE-RAGER-DAMAGE-TAKEN-PASS`
(sub-50% crossing hit zeroed + heal, fork
`mob_core_rager` 266-272), `LIVE-BOSS-HEROD-PASS`
unchanged. Regressions: deadmines, shadowfang, GO
5/5, instance batches 1-6, spells, AI, motion all
green; clippy workspace `-D warnings` 0; `./ci.sh`
CI_EXIT=0 (`/tmp/ci_t3s3b.log`); bench rebuilt +
restarted (LISTEN 18099), TWO-CLIENT-SMOKE-PASS.

Next: Kalimdor + world CreatureAI lanes (6:
cai_aq_ruins, cai_aq_temple, cai_dire_maul_feralas,
cai_kal_instances, cai_kal_zones, cai_world), then
Slice 2 steps 4-6 (spell/aura script census,
gossip/quest-accept, areatrigger/event/dummy), then
Slice 3 battlegrounds.

## Slice 2 step 3c: Kalimdor + world CreatureAI lanes

6 spark lanes (cai_aq_ruins, cai_aq_temple,
cai_dire_maul_feralas, cai_kal_instances,
cai_kal_zones, cai_world; 34,877 lines) spliced with
the fork-census manifest (195 scripts incl. driver
recovered mob_anubisath_sentinel + hunter-demon set
npc_solenor/npc_artorius/npc_Merok/npc_Shai; DB
cross-check: only custom_npc_summon_debugAI,
generic_random_move, npc_AlteracDardosh,
npc_sandstalker lack template bindings — the two
never-registered-in-fork are variant-only, sandstalker
templates carry empty script_name in this DB). 706+
dispatcher arms across batch-1/2 dispatchers.
content.rs now ~104.6k lines.

Fork gated binds implemented in `for_script_gated`
(bind site `ensure_ai`): `npc_j_eevee` by map
(429 Dire Maul / 289 Scholomance, else no AI),
`boss_dragon_of_nightmare` by entry
(14889/14888/14890/14887; saved-variable UpdateEntry
permutation unmodeled). Entry-gated target-dummy aura
pair bound at reset via `object_entry` (lane NOTE-4
closed for that script).

Splice defects found and fixed during integration
(driver): enter_evade arms initially inserted into
the just_died dispatcher (enter_evade's match is
followed by the reset call, so its tail shape
differs) — relocated, all other dispatchers audited
clean; shared-base wrappers (bug trio, twins) were
already per-variant in the lanes; Alzzin
movement_inform/summoned_creature_just_died and
Priestess aggro hand-wired (mixed lane prefixes);
105 borrow sites hoisted out of state-accessor scopes
(rng stream order shifts where a roll became
unconditional — noted class, same as EK); dummy
entry consts restored with fork values
(2674/12426 — lane had dropped them, patterns were
bindings). Framework fix reached by Onyxia:
`script_combat_drive` chase sync now honors
`ai.combat_movement=false` (fork SetCombatMovement)
— script point motion (phase transitions) was being
overridden by chase every tick.

Unmodeled remainder (lane `NEEDS-MAP-HELPER` +
`NO-DISPATCHER-YET` markers inline): escort set
(celebras_spirit, npc_belnistrasz, willix, snufflenose
FollowerAI, disciple_of_naralex, shay/kindal, AV event
troops/world-boss bases); corpse_removed, healed_by,
aura_removed, on_script_event, receive_emote,
begin_event family (gossip/quest entry points — step
5 kinds); DoResetThreat (9 AQ sites + Onyxia),
SetFly/SetLevitate/SetFacingTo, SummonGameObject,
NearTeleportTo for units, InterruptNonMeleeSpells,
visibility/faction temporary overrides. BG creature
scripts (spirit guide, AV bosses) are transcribed but
unreachable until Slice 3 maps exist.

Verification: `live_creature_scripts` 5/5 —
`LIVE-OSSIRIAN-PASS` (aggro -> Strength 25176 aura +
tornado 15428 found live at fork spawn point),
`LIVE-ONYXIA-PASS` (engage -> 60% -> phase-2
transition -> Hover 17131), Herod/Anub/CoreRager
unchanged. Regressions: deadmines, shadowfang, GO
5/5, instance batches 1-6, spells, AI, motion green;
clippy workspace `-D warnings` 0; `./ci.sh` CI_EXIT=0
(`/tmp/ci_t3s3c.log`); bench rebuilt + restarted
(LISTEN 18099), TWO-CLIENT-SMOKE-PASS.

Next: Slice 2 steps 4-6 (spell/aura script census
vs the Phase-3 spell-script host, gossip/quest-accept
kind, areatrigger/event/dummy kinds), then Slice 3
battlegrounds.

## Slice 2 steps 4-6 inventory (fork vs Rust by kind)

Fork `Script` registration fields (ScriptMgr.h) and
counts in `vmangos/src/scripts` (census
`/tmp/fork_kind_census.json`), Rust host status:

- GetAI (creature): 448 — DONE step 3 (EK+Kal+world).
- GOGetAI/pGOHello/pGOOpen: DONE step 2 host
  (`go_hello`, GoAi dispatch); pGOQuestAccept (2),
  pGOGossipHello (1), pQuestRewardedGO (5) ride the
  quest/gossip kinds below.
- GetSpellScript: 114 registrations; GetAuraScript:
  31; DB `spell_template.script_name` binds 145
  names over 635 spell rows (fork/DB sets match
  exactly; census `/tmp/spell_script_census.json`).
  Rust: NO spell-script host; `SpellEntry` does not
  even carry `script_name` (loader drops the column).
  Fork hook usage: OnEffectExecute 67, OnCheckCast
  15, OnSetTargetMap 14, OnSummon 7, OnCheckTarget 6,
  tail <=3; AuraScript: OnAfterApply 14,
  OnBeforeApply 9, OnPeriodicTrigger 7,
  OnPeriodicDummy 4, OnPeriodicTickEnd 3, OnProc 2,
  tail 1. Only 7/145 names have any spell-id mention
  in Rust spell core — fork scripts exist where
  generic DBC behavior is insufficient, so default
  census verdict is NOT covered.
- pGossipHello 31 / pGossipSelect 24 (+SelectWithCode
  in the Script struct): Rust has gossip packet
  handlers + DB gossip, NO script-name dispatch.
- pQuestAcceptNPC 49 / pQuestRewardedNPC 16 /
  pQuestComplete / pNPCDialogStatus (in struct):
  Rust quest handlers exist, NO script-name dispatch.
  Many quest-accept scripts are escort starters —
  they also need the escort runtime (step-3
  remainder) to do anything observable.
- pAreaTrigger 24: Rust has areatrigger teleport
  handling, NO script-name dispatch.
- pProcessEventId 10: fork caller is the DBScripts /
  game-event machinery; Rust game-event system is
  still the `game_event_active=false` stub — likely
  recorded remainder unless a ported script reaches
  it through dbscripts.
- pEffectDummyCreature 8 / GameObj 1 / AuraDummy 1:
  Rust dummy-effect dispatch exists generically in
  spells.rs but not by script name.

Order (per inventory plan): step 4 spell/aura host +
corpus, step 5 gossip+quest host + corpus, step 6
areatrigger/dummy (+event disposition). Each kind:
driver builds host + first exemplar, spark Workflow
lanes after.

### Step 4a: spell/aura script HOST + driver exemplars (done)

- `SpellEntry.script_name` now loaded
  (`mangos-world-data` field 150 / DB col 151).
- Host `crates/mangos-map/src/spell_scripts.rs`,
  dispatch by `script_name` (host-enum deferred, same
  as early CreatureScriptAi). Wired hooks:
  - `SpellScript::OnCheckCast` at end of `cast_spell`
    validation (fork `Spell::CheckCast` tail):
    `Some(reason)` fails the cast.
  - `SpellScript::OnEffectExecute` per effect in
    `apply_effects` (= `DoAllEffectOnTarget`); `false`
    skips the effect; fork `spell->damage` mutation
    rides a `script_damage` channel into the
    school/weapon damage arms.
  - `AuraScript::OnBeforeApply` / `OnAfterApply`
    holder-level in `add_aura_holder` /
    `remove_aura_at`, fork `ApplyModifier` order
    (before/after handlers, per present effect).
  - `AuraScript::OnPeriodicDummy` in
    `periodic_dummy_tick` before the hardcoded
    family switch; the prior inline
    `spell_haunting_spirits` arm moved into the
    dispatcher unchanged.
- Exemplar 1 (spell): `spell_paladin_holy_shock`
  (20473/20929/20930): OnCheckCast facing gate
  (`FAILED_UNIT_NOT_INFRONT=124`, fork-verified,
  <1.4yd always-facing + PI arc) and friendly/hostile
  heal/hurt sub-casts 25914/25913/25903 /
  25912/25911/25902. Live: `LIVE-HOLY-SHOCK-SPELL-
  SCRIPT-PASS` (124 when facing away, hurt damages
  Herod, self-cast heals; fixture level raised to 60
  — player spell damage vs +level boss otherwise
  always resists, which also explains why earlier
  Moonfire "spell-kills" needed hp preset to 1).
- Exemplar 2 (aura): `spell_curse_of_bleakheart`
  (6946, felwood.cpp): OnBeforeApply
  `SetPeriodicTimer(5000)` — fork semantics set
  period AND timer (`SpellAuras.h:498`), DBC
  amplitude is 0; OnPeriodicDummy 5% self-cast
  6945. Live: `LIVE-BLEAKHEART-AURA-SCRIPT-PASS`
  (holder period/timer 5000 at apply; 6945 fired).
- Regression: live_creature_scripts 7/7,
  live_spells ok, workspace clippy 0,
  `./ci.sh` CI_EXIT=0 (`/tmp/ci_t3s4.log`), bench
  rebuilt+restarted, `TWO-CLIENT-SMOKE-PASS`.
  Note: smoke fixtures guid 3/4 had drifted beyond
  the 25yd say range across runs (script moves +4yd
  per run, positions persist); guid 4 repositioned
  to (1649.4,-4365.4) on the bench. Pre-existing
  smoke hermeticity gap, not a code regression.
- Remaining hooks wired when first reached by a
  lane script (OnSetTargetMap 14, OnSummon 7,
  OnCheckTarget 6, aura OnPeriodicTrigger 7 /
  OnPeriodicTickEnd 3 / OnProc 2 / remove-mode arg
  for OnBeforeApply(apply=false) arms, etc.).
  Next: spark Workflow lanes by fork file
  (`spells/spell_*.cpp` 11 + boss-local spell
  scripts), wonder splices/reviews/verifies.

### Step 4b: spell/aura script lanes spliced (done)

- 10 spark lanes (`/tmp/tier3-lanes/sp_*.rs`, 145
  fork registrations over 40 files) reviewed and
  spliced lane-by-lane into `spell_scripts.rs`
  (~5.1k lines). Live dispatcher arms now cover
  the class corpora (paladin/warrior/shaman/rogue/
  warlock/hunter/mage/priest/druid), item and
  special scripts, and the boss-local spell
  scripts (Nefarian/Thaddius/Grobbulus/Naxx/AQ).
- Hosts grown by lane reach: `OnAfterHit` seam
  (end of per-victim effect pass; bubble, first
  aid), `OnPeriodicTrigger` seam (Mana Tide,
  Curse of Idiocy, Frost Trap, Thaddius charge
  auras, Darkmoon turrets — the latter completed
  with the now-`pub(crate)` power seam),
  `OnSetTargetMap` + fork max-target trim in
  `collect_targets` (mode/radius/max/closest;
  closest-sort or random erase keeping the
  explicit target), `OnCheckTarget` victim
  filter, `triggered_by_aura_spell` /
  `triggered_by_spell` threaded into the
  effect dispatcher (Touch of Weakness, Mutagen
  Explosion x1.5//1.5), HEAL arm honors the
  `script_damage` channel (Swiftmend), holder
  caster threaded into `OnAfterApply` (Wyvern
  Sting), `remove_some_cooldowns` / `unit_power`
  / `unit_set_power` / `unit_block_value` made
  `pub(crate)`.
- Lane-behavior probe: `LIVE-CONFLAGRATE-
  CHECKCAST-PASS` (17962 fails 103 without the
  caster's Immolate). Regression: live_creature_
  scripts 8/8, live_spells + deadmines ok,
  workspace clippy 0, `./ci.sh` CI_EXIT=0
  (`/tmp/ci_t3s4c.log`), bench rebuilt/restarted,
  `TWO-CLIENT-SMOKE-PASS`.
- Fork-faithful non-wirings: Blood Fury and
  Curse of Agony dummy bodies compile out at
  1.12.1 (helpers in tree, unarmed). ~30 helpers
  wait on dispatchers not yet wired
  (`NO-DISPATCHER-YET` markers): OnProc
  (Ignite/Combustion), OnSummon (Inferno, Eye of
  Naxxramas, AV shredder), OnCast (Execute
  dummy), OnInit/OnAuraInit/OnHolderInit,
  OnSuccessfulStart/Finish/Dispel, OnHit,
  OnPeriodicTickEnd, OnAuraValueCalculate +
  OnPeriodicCalculateAmount (GDR). Per-arm
  UNMODELED notes stay in code (combo points,
  reactive targets, remove-mode, Meteor target
  count, cast-item delay threading, GO/lock
  state, dispel immunity, extra attacks).
- Channel caveat (recorded): fork scripts mutate
  the rolled `spell->damage`; the host channel is
  an additive delta, so multiplicative/absolute
  arms recompute the base via `effect_value`
  (dice re-roll; exact for flat effects).
  Periodic-trigger caster resolution in Rust
  uses the wearer-casts attribute, not the fork's
  wearer-default + original-caster guid.

Next: step 5 gossip/quest script host.

### Step 5a: gossip/quest host + res-fixer exemplar (driver, wonder)

Fork dispatch (`ScriptMgr.h`): gossip/creature scripts expose
`pGossipHello`/`pGossipSelect` (+Coded) per creature template
`ScriptName`; quest hooks `pQuestAcceptNPC`/`pQuestRewardedNPC`
fire from the quest accept/reward completions. Rust host
(`crates/mangos-map/src/gossip.rs`): `gossip_script_on_hello`
(committee: builds `GossipMenuData{npc_text, items}`),
`gossip_script_on_select` (`Close` vs re-`ShowMenu`),
`quest_script_on_accept`/`quest_script_on_rewarded` (empty
dispatchers, lanes fill). Session side in `world_actor`:
`CMSG_GOSSIP_HELLO` (379) validates session player, resolves
the creature on the session map, calls the host by
`CreatureState.script_name`, stores `SessionEntry.gossip_menu`
for `CMSG_GOSSIP_SELECT_OPTION` (380) sender/action lookup;
unhandled hello closes with `SMSG_GOSSIP_COMPLETE` (default
npc_text/quest-menu gossip preparation: UNMODELED remainder —
quests use `CMSG_QUESTGIVER_HELLO`; DB-driven default gossip
menus not built). `SMSG_GOSSIP_MESSAGE` wire follows
`Player::PrepareGossipMenu`/send ordering with 0 quest items
(fork builds quest items in: UNMODELED remainder until a lane
file needs them); coded field always 0 (no coded-gossip
registrations reached yet); item text: positive ids resolve
via broadcast texts (male/female by player gender), negative
`mangos_string` ids UNMODELED (empty). Quest seams:
`quest_script_on_accept` after successful `quest_add`;
`quest_script_on_rewarded` at `quest_reward` end.
Exemplar `npc_res_fixes` (fork registration name; file:
`custom/resurrector.cpp`, entries 13856/13857 `npcflag 1`):
hello = menu text 6440 + item broadcast 8995 (sender 1,
action 1001); select = remove race res-sickness aura
(`ChrRaces.dbc` produced field 9; fork fallback 15007),
`DurabilityRepairAll` (no-cost, divine-provider arm),
close. New plumbing: `RaceDbc.res_sickness_spell_id` +
`PlayerInfos::res_sickness_spell_id`, `Map::durability_repair_all`.
Probe (`bench_res_fixer_gossip`): bench spawn guid 2530718
(entry 13857, map 1 (1660,-4370,31.25)) inserted into the
dev-bench world DB; menu shape asserted, seeded 15007 holder
removed by select → `LIVE-RES-FIXER-GOSSIP-PASS`. Spawn-count
oracle in `live_spawns.rs` updated 66222→66223 /
62760→62761 (both phases count the bench spawn). CI green;
bench restarted; `TWO-CLIENT-SMOKE-PASS` (two earlier smoke
runs timed out at movement relay within ~4 min of a fresh
boot while presence worked — warm-up lag, passed on rerun;
not gossip-related, recorded as observed bench behavior).
Wire-level gossip probe: not yet (in-process only) — REMAINDER
for lane closeout.

### Step 5b: gossip/quest corpus lanes + npc_gossip store

Six spark lanes by fork file (battlegrounds files deferred to
Slice 3), 28 hello / 22 select / 49 accept / 13 rewarded arms,
114 impl fns spliced into `gossip.rs` (lane files
`/tmp/tier3-lanes/gq_*.rs`). Driver spot-review vs fork:
Nagmara (4201/5040/2727/2729 exact), Simone/Krug
(`GetGossipTextId` sites), Jaina autograph cast (23122 self,
menu 7012), dying-archaeologist `DEFAULT_GOSSIP_MESSAGE` —
all faithful. One systematic lane gap found and fixed:
`Player::GetGossipTextId` had been stubbed `text_id: 0`.
Added the fork store `npc_gossip(npc_guid, textid)` to
`ScriptStores.npc_gossips` (`load_npc_gossips`) and host
helper `Map::gossip_text_id` (spawn-guid lookup, nonzero,
else `DEFAULT_GOSSIP_MESSAGE` 16777215); patched Simone,
Krug (hello+select), and the four custom-NPC hellos
(enchant/profession/premade gear/premade spec — all fork
`GetGossipTextId`). Shared instance constants/helpers widened
to `pub(crate)` in `content.rs` (BRD/BWL/DM/GNO/ZF data ids +
getters, `player_quest_incomplete`). Clippy: impl-level allows
for fork-literal lints (`unreadable_literal`,
`excessive_precision`, `too_many_lines`, `unused_self`,
`unnecessary_wraps`, `match_same_arms`).
Verification: `./ci.sh` green; live regressions 9/9
(`LIVE-RES-FIXER-GOSSIP-PASS` included); bench restart;
`TWO-CLIENT-SMOKE-PASS`; new wire probe
`scripts/gossip_wire_probe.py` → `LIVE-GOSSIP-WIRE-PASS`
(CMSG_GOSSIP_HELLO → menu text 6440 + broadcast-8995 item
"Click here to remove res effects and repair all gear" →
select → SMSG_GOSSIP_COMPLETE; fork writes the raw u64 guid
in SMSG_GOSSIP_MESSAGE, confirmed in `GossipDef.cpp:130`).
REMAINDERS recorded: escort/follower framework (most quest
accepts degrade to say/faction only — `npc_escortAI::Start`
unbuilt), `PrepareQuestMenu`/quest items in gossip menus,
default (non-script) gossip via `gossip_menu`/`gossip_text`
DB tables, coded gossip (`pGossipSelectWithCode`,
`pGOUseWithKeyCode` style), `mangos_string` negative item
texts, gossip hello/select host gates (fork
`GetNPCIfCanInteractWith` distance/alive/flag checks not
mirrored — same-map session lookup is the only constraint),
custom premade gear/spec template stores, profession
HasSkill/SetSkill prims, AQ war-effort collector AI state.
Per-lane UNMODELED comments mark each residual call site.

### Step 6a: areatrigger + dummy + process-event hosts (driver, wonder)

Kind inventory vs fork `Script` (`ScriptMgr.h:223-246`):
pAreaTrigger 24 regs/19 files, pProcessEventId 10/8,
pEffectDummyCreature 8/8, pEffectDummyGameObj 1,
pEffectAuraDummy 1, pGOGossipHello 1, pQuestRewardedGO 5,
pGOQuestAccept 2, pGOOpen 3 (host existed, still no
call site — SPELL_EFFECT_OPEN_LOCK unmodeled),
pGOGossipSelect/pGossipSelectWithCode/pQuestComplete: 0
registrations in the fork corpus (nothing to port).

Areatrigger: the session runtime already existed (DBC
store, zone test, condition/cooldown gates, quest
credit, tavern, teleport); only script execution was
missing. New `crates/mangos-map/src/area_triggers.rs`
(`Map::area_trigger_script` by `script_name`) wired into
the gate with fork order script -> quest -> tavern
(the old Rust order evaluated tavern first; corrected).
DB arm: `areatrigger_scripts` added to `ScriptStores`
+ `Map::scripts_start_area_trigger` (was logged
"unexecuted"). Exemplar `at_murloc_camp` (darkshore,
trigger 1966) -> `LIVE-MURKDEEP-AREATRIGGER-PASS`.
Corpus: 23 lane arms spliced (instances 14, world 9),
spot-checked ring-of-law vs fork (data ids/coords exact;
grimstone PlayerEnteredArena UNMODELED-marked).

Dummy kinds: new `dummy_scripts.rs` hosts
`effect_dummy_creature` (called at the `EffectDummy`
tail — `eff_index` plumbed through — and at the
`Aura::TriggerSpell` no-trigger/no-AuraScript tail),
`effect_aura_dummy` fanned out per DUMMY effect index
after `aura_script_on_after_apply` apply/remove, and
`effect_dummy_gameobject` (no call site: GO spell
targets unmodeled wholesale — REMAINDER). Exemplars:
`peon_wake_up` -> `LIVE-PEON-WAKEUP-DUMMY-PASS`
(root-caused a probe failure to the test harness's
empty quest store; `load_stores` now loads real quest
templates), viscidus freeze aura dummy wired to the
existing AI reset glue.

Process events + GO use: stores `event_scripts`,
`gameobject_scripts`, `scripted_event_id` loaded;
`process_events.rs` dispatcher + `scripts_start_event`
/ `scripts_start_gameobject`. `gameobject_use` gains
fork-subset arms: QUESTGIVER (2: script hello +
`MapEvent::QuestGiverHello` -> actor sends quest
list), GOOBER (10: pagetext, quest gate, process
event / event_scripts / gameobject_scripts, IN_USE +
ACTIVE), CAMERA (13: cinematic packet + event), and
the door/button `sGameObjectScripts` start. UNMODELED
tails recorded in-code: GO gossip menus, GO-cast quest
credit (`RewardPlayerAndGroupAtCast`), linked traps,
custom anim, auto-close/loot-state, goober spellId
GO-cast. Exemplar `event_test_of_endurance` (747) ->
`LIVE-TEST-OF-ENDURANCE-EVENT-PASS` (Harpy Foodstuffs
goober with quest-1150 gate summons Grenka once).

GO quest flow: session quest handlers generalized
from creature-only to `quest_object` (creature or GO;
`quest_relations` gameobject starts/ends) across
hello/status/query/accept/request-reward/choose-reward,
with `pGOQuestAccept`/`pQuestRewardedGO` seams
branching to new gossip.rs GO dispatchers
(+`pGOGossipHello`). Wire probe
`scripts/go_quest_wire_probe.py` -> `LIVE-GO-QUEST-WIRE-PASS`
(Control Console quest list; 0 quests visible for the
fixture's level window — packet/guid/title asserted).
CI + smoke green; bench restarted on the new build.
Lane bodies for dummy/process/GO-quest follow in 6b.

### Step 6b: dummy / process-event / GO-quest lane bodies (spark lanes, wonder-spliced)

Spliced from `/tmp/tier3-lanes/{dm_dummy,pe_events,go_quest}.rs`:
`dm_dummy` 7 creature dummy arms + 1 GO arm + 8 fns in
`dummy_scripts.rs`; `pe_events` 9 arms/fns in
`process_events.rs`; `go_quest` 1 GO hello (`go_scarab_gong`),
2 GO accept, 5 GO rewarded + 8 fns in `gossip.rs`. Splice
caveat: the dm lane emitted its dispatch arms commented; the
first splice inserted fns only (dead-code warning exposed
it) — arms hand-inserted in `Some("name")` form afterward.
Spot reviews vs fork: `dm_capture_grark` exact (14250,
eff 0, >25% health returns true, EMOTE_SUBMIT 4918,
faction 85, evade; positive ids are the Rust
`do_script_text` convention), `process_event_summon_gahzranka`
exact (source/player/instance gates, triggered 12816 cast
before the creature check, `IN_PROGRESS`, `Respawn`
approximated via despawn-now re-materialization),
`go_scarab_gong` hello returns true with
`PrepareQuestMenu` UNMODELED (the type-2 use arm pushes
`QuestGiverHello` regardless, so the wire quest list
matches the fork). Verification: `./ci.sh` `CI_EXIT=0`
(log `/tmp/ci_t3s6d.log`), live creature-scripts suite
12/12, bench `mangosd` rebuilt + restarted directly,
`TWO-CLIENT-SMOKE-PASS` (two preceding relay timeouts were
fixture drift: the smoke's own movement leg saved guid 3
at 1669.4,-4373.4; resetting 1657.4/1649.4 fixtures passed).

**Slice 2 closeout.** Every registered script kind in the
223-file corpus now has a Rust host + ported arms:
CreatureAI/instance/GO-AI (content.rs), spell/aura scripts,
gossip/quest (creature + GO), areatrigger, creature/GO/aura
dummy, process events, GO quest flow. Recorded remainders
stay as `UNMODELED:` / `NO-DISPATCHER-YET` / `NO-CALLSITE-YET`
in-code comments (spell hook family ~34, escort framework,
GO casting wholesale, gossip defaults, `event_the_conterattack`
has no fork body). BG scripts remain Slice 3 scope.

## Slice 3: battleground engine + maps

### Slice 3 inventory (fork vs Rust)

Fork surface (`vmangos/src/game/Battlegrounds/`, 8 767 lines
+ handler 677): `BattleGroundMgr` (queue: 4 queue types x
6 brackets, group queue infos, invite/free-slot flow,
`CreateInitialBattleGrounds` from `battleground_template`
with `patch <= WowPatch` gating), `BattleGround` base
(players/scores/status machine, start/end, marks + mail
overflow, honor/reputation rewards, world states, DB event
spawns, closest-graveyard, spirit-guide resurrection),
per-map `BattleGroundWS` (710), `BattleGroundAB` (574),
`BattleGroundAV` (1 710). Types: AV=1 (map 30), WS=2
(map 489), AB=3 (map 529); `BattleGroundDefines.h` carries
sounds/texts/marks/intervals/score types. Session surface:
`Handlers/BattleGroundHandler.cpp` (11 handlers:
BattlemasterHello, BattlefieldJoin, BattlemasterJoin,
PlayerPositions, PVPLogData, BattlefieldList, Port,
LeaveBattlefield, Status, AreaSpiritHealerQuery/Queue).
No `BattlemasterList.dbc` in the fork's DBC stores; start
locations resolve through `WorldSafeLocs.dbc` ids in
`battleground_template`. Scripts: `battleground.cpp`
(`npc_spirit_guide` AI+gossip, `npc_etendard` AI) and
`battleground_alterac.cpp` (5 025 lines of AV content).

Rust today: opcodes exist in `mangos-protocol`; **zero**
engine (no queue state on player, no BG map state, no
handlers, no template load; `map_template::is_battleground`
only). Placement mirrors instances: per-match rules on
`Map` in `mangos-map::battlegrounds`, queue/invite/session
side in `mangosd` (world_actor + new module), data in
`mangos-world-data` (template table + `WorldSafeLocs`
loader over the existing DBC format). Steps: (1) data
layer; (2) queue manager + handlers + port-in teleport
(wonder core); (3) WSG exemplar (flags, scores, world
states); (4) AB; (5) AV engine + `battleground_alterac`
content; (6) BG spell leftovers 23383/23384, 30882,
mark-mail overflow; (7) `battleground.cpp` script pair.

## Slice 3 steps 1-2: data layer + engine core (verified)

Step 1 data layer (`mangos-world-data::battlegrounds`): `SafeLocStore`
(`WorldSafeLocs.dbc` fmt `nifffxxxxxxxxx` + `world_safe_locs_facing`,
missing facing = 0), `load_battleground_templates` (patch-selected rows),
`BattleMasters` (`battlemaster_entry`), `BattleEventIndexes`
(`creature_battleground` / `gameobject_battleground`, `BG_EVENT_NONE = 255`
skipped), `GraveyardLinks` (`game_graveyard_zone`), `BattlegroundEntrances`
(`areatrigger_bg_entrance`, fork validations). Bench boot:
`battlegrounds: 3 templates, 122 safe locs, 24 battlemasters, 2393 event
indexes, 189 graveyard links, 6 entrances`.

Step 2 engine core:
- `BgConfig` (`BgConfig::bg_config`, fork World.cpp:763-783; QueuesCount
  0 -> patch10?3:1).
- Player `BgPlayerState` (queue slots, entry point, bg id/team, bracket
  helpers, `save_row()`), save/load wiring in character save + materialize.
- Map-side `BgState` (`mangos-map::battlegrounds`): base lifecycle
  (premature finish, door/start timers, `AddPlayer`, scores/pvp-log,
  `EndBattleGround`/`EndNow`, kill hooks, DB event registry + spawn modes,
  closest graveyard, honor gain 188.3 + rank table), AT/flag-click seams
  (FLAGSTAND 24 / FLAGDROP 26, stealth/invisibility strip).
- Actor `BgManager` (`world_actor::battleground`): queue lanes, invites
  (80s/40s timers), fill/premade/normal/free-slots selection, manager tick,
  BG raids (transient db_id 0, original-group preserve), handlers
  (battlemaster hello/join, battlefield join/port/status/list/leave,
  PVP log, positions), login relocation, level-up queue ejection.
- Wire facts: port = mapId u32 + action u8; leave = mapId u32 + combat
  gate; positions = `MSG_BATTLEGROUND_PLAYER_POSITIONS` 745.

Verification: `./ci.sh CI_EXIT=0` (`/tmp/ci_t3s3step2.log`), live
creature-scripts 12/12, bench rebuilt/restarted, `TWO-CLIENT-SMOKE-PASS`
(first run failed on fixture drift: guid3 position moved between reset
and run; re-reset passed).

Recorded remainders (engine core): WSG/AB/AV kind bodies are dispatch
stubs (steps 3-5); `bg_add_object` returns false (no dynamic
`CreateGameObject(entry)` runtime); GO loot-state DoorClose precondition
unmodeled; spirit-guide resurrect queue returns time 0 / no-op (step 7);
mark mail overflow + spells 23383/23384/30882 (step 6); mounted /
total-immune arms of `CanUseBattleGroundObject` unmodeled in the flag
click gate; `HonorMgr` rank/standing/daily-weekly recompute (Slice 4).

## Slice 3 step 3: Warsong Gulch exemplar (verified)

`mangos-map::bg_ws` (fork `BattleGroundWS.cpp` wholesale): flag
keepers/states/timers, `Update` flag timers (23 s respawn, 10 s drop),
`Reset` tail in `ws_setup` (ghost-gate active at 0, flags/spiritguides
`BG_EVENT_NONE` until doors open), pickup/return/drop/capture flows
(sounds, BCT texts 9801-9809/10022/10023, world states 1545/1546,
1581/1582, 1601, 2338/2339), bracket honor tables, capture reputation
(890/889 x35), exit areatriggers 3669/3671, fixed graveyards
(769-772 by status/team, wired into `bg_closest_graveyard`),
`FillInitialWorldStates` wired into `update_player_zone` for BG
maps type 2, `EndBattleGround` WS reward stage before the base
ending, real `ws_flag_carriers_pub`.

Verification: `./ci.sh CI_EXIT=0` (`/tmp/ci_t3s3step3.log`), in-process
probe `LIVE-WSG-PASS` (`live_wsg.rs`: setup/start timeline, flagroom ->
main graveyard switch, score counters 7/8/3, drop -> ON_GROUND ->
timer respawn, bracket-5 win honor +198), live creature-scripts 12/12,
bench rebuilt/restarted, `TWO-CLIENT-SMOKE-PASS` (3rd attempt; first
two were the recorded post-boot fixture-timing flake).

Recorded remainders (WS): ground-flag clickable gameobjects need
`SPELL_EFFECT_SUMMON_OBJECT` (dropped-flag GO never spawns, so ground
return/pickup arms are unreachable until step 6 adds the effect; flag
returns happen through the drop timer); `CastSpell(23334/23336)` on
drop is a no-op for the same reason; `IsBgWeekend` hardcoded false
(no game-event manager); buff-trap GO trigger area triggers
3686/3687/3706-3709 return true with no trap runtime behind them.

## Slice 3 step 4: Arathi Basin (verified)

`mangos-map::bg_ab` (fork `BattleGroundAB.cpp`): five-node
states/timers, `_CreateBanner`/`_SendNodeUpdate` world states
(1767+/1782+/1772+/1792+/1787+ with plusArray, icons
1842-1846, occupied counts 1778/1779, resources 1776/1777,
max/warning 1780/1955), resource ticks (12/9/6/3/1 s,
10/10/10/10/30), rep/honor tic thresholds (200/330 post-1.10),
node kill credits 15001-15005, near-victory at >1800, win at 2000,
graveyards (entrance 890/889 pre-start, closest occupied node
GY by squared XY, start 898/899 fallback), quest-reward spells
24061/24064 at 4/5 bases, exit ATs 3948/3949. LANG 650-660
node messages route through the new `MapEvent::BgLangMessage2`
(actor resolves nested mangos_string args). `BgState.buff_change`
added (AB ctor true); buff GO setup degrades through the
unmodeled `bg_add_object` like WS.

Verification: `./ci.sh CI_EXIT=0` (`/tmp/ci_t3s3step4.log`),
`LIVE-AB-PASS` (entrance->start GY, contested->occupied timer,
+10 resource tick, stables graveyard, bases counters 9/10,
win at 2000 with bracket-4 honor +198), live creature-scripts
12/12, bench rebuilt/restarted, `TWO-CLIENT-SMOKE-PASS`
(retry after the recorded fixture flake).

Recorded remainders (AB): buff trap gameobjects ("HandleTriggerBuff"
+ dynamic creation, same step-6 runtime gap as WSG ground flags);
`IsBgWeekend` false; GM arm of the pre-start graveyard gate skipped.

## Slice 3 step 5 (engine): Alterac Valley (verified)

`mangos-map::bg_av` (fork `BattleGroundAV.cpp`): 15 grave/tower
nodes (assault -> 5 min `DestroyPoint`, snowfall first-cap,
stoneheart-bunker event 25 quirk), reinforcement score (600,
clamped at 0, no win at zero per the fork's removed condition),
mine ownership/reclaim/tick timers, captain buff cycle
(120 s + urand(0,4)*60 s), Snivvle 70 s yell, starting-event-3
supply/tamed ladder spawns (dead in practice, ported as written),
`HandleQuestComplete` (armor scraps +20 with crate ladders,
air/blood/mine/hide/tame counters with thresholds, flat-rep id
list) wired from `quest_reward` (fork Player.cpp:13008 position),
`CheckSpellCast` shredder charms (21544/21565, 0x2F fail),
`EndBattleGround` surviving towers/graves/mines/captain rewards,
`GetClosestGraveYard` (start-seeded nearest owned grave,
pre-start 611/610).

Verification: `./ci.sh CI_EXIT=0` (`/tmp/ci_t3s3step5.log`),
`LIVE-AV-PASS` (setup owners, mine tick 600->601, death -1,
snowfall assault->controlled, GY 169, counters 11/13/15,
end honor), live creature-scripts 12/12, bench rebuilt/restarted,
`TWO-CLIENT-SMOKE-PASS` (3rd attempt; fixture flake).

Recorded remainders (AV engine): banner GO deletion
(`AddObjectToRemoveList` — no GO remove-list runtime; event spawn
modes still flip); landmine-expert GO sweep; GM arm in
`GetClosestGraveYard`; monster say/yell broadcast map-wide instead
of listen range; `IsBgWeekend` false (no 1584/396 flat honor);
GM `HandleCommand` debug hooks unported (no Rust BG command
dispatcher); `battleground_alterac.cpp` content scripts port
next (challenge invocation consumers of the engine state).

## Slice 3 step 5b inventory: battleground_alterac.cpp

Fork `vmangos/src/scripts/battlegrounds/battleground_alterac.cpp`
(5,025 lines) registers 23 script names in `AddSC_bg_alterac`:
CreatureAI (in Rust via `content.rs`): npc_Vanndar, npc_DrekThar,
npc_Balinda, npc_Galvangar, npc_WarMaster, npc_AlteracBowman,
npc_korrak_the_bloodrager, AV_dismount, AV_mines, AV_commander,
av_warrider. Escort/wave-derived AI (no `npc_escortAI`
framework): AV_NpcEventAI base, AV_NpcEventTroopsAI,
AV_npc_troops_chief_EventAI, av_world_boss_ally/horde,
Lokholar_assistants/superior_elementals, av_guard_summoned.
GO AI: go_av_landmine, go_av_beacon. Gossip/quest hooks:
npc_AV_blood_collector (hello/select/on-rewarded), npc_AV_troops_chief
(on-rewarded), npc_av_trigger_for_quest. Spell script:
spell_alterac_shredder (OnSummon only).

Hosts reached: `CreatureScriptAi` + `creature_script_update`
(content.rs), `gossip_script_on_gossip_hello/select`,
`quest_script_on_rewarded` (gossip.rs), spell-script host
(spell_scripts.rs), GO AI (content.rs gameobject_use).

Plan: (a) blood-collector quest hook + gossip port (wonder,
first-of-kind challenge consumer); `setChallengeInvocationCounter`
means ADD (engine helper to add); escort `Start` arms record as
UNMODELED (no escort runtime), vendor-list arm unmodeled, beacon
item grants via actor `quest_grant_item`. (b) npc_Balinda
exemplar (wonder) -> spark Workflow lanes for the remaining
Creature/GO/gossip scripts (Dardosh skipped: registration
commented out in fork). Escort-family scripts (5 named +
world-boss pair) stay UNMODELED behind the Slice 2-recorded
escort-framework remainder; their engine state is already live.

## Slice 3 step 5b — `battleground_alterac.cpp` content (2026-10-29)

Census outcome (fork 5,025 lines, 23 script names): the whole Alterac
CreatureAI set was already ported in Slice 2 (tree-verified this step):
`npc_Vanndar`, `npc_DrekThar`, `npc_Balinda`, `npc_Galvangar`,
`npc_WarMaster`, `npc_AlteracBowman`, `npc_korrak_the_bloodrager`,
`AV_dismount`, `AV_mines`, `AV_commander`, `av_warrider`, `AV_landmine`
(npc), `npc_av_trigger_for_quest`, plus `npc_spirit_guide`
(`battleground.cpp`). Driver review this step fixed Balinda's three
recorded NEEDS gaps (Horde-team `IsHostileTo` gate in
`MoveInLineOfSight`, map-30 leash gate, real threat-list count within
6.0 for the Arcane Explosion integer-division roll).

Driver ports this step (first-of-kind BG-engine consumers):
- `npc_AV_blood_collector` (`gossip.rs`): `QuestComplete` hook feeding
  the challenge counters by `ReqItemId[0]` (blood/hide/flesh/medal/
  supplies, tamed 7001/7027), world-boss ready → counter reset +
  `setPlayerGoStatus`; full fork `GossipHello` (quartermaster ground
  progress chain, smith armor thresholds + reinforcement menus
  6073/6217/6218, falconer distance/flag arms, quest-relation loop,
  Teravaine override, objective-reached assault items incl. global
  air, Renferal/Thurloga boss-progress items) and `GossipSelect`
  (challenge resets + GO status + say/shout, supply menus 6778-6784/
  6731-6733, boss texts 6098-6100/6175-6177, armor upgrade
  500/1000/1500). Recorded UNMODELED here: beacon/assault-order item
  grants (no map-side item grant), `SEND_VENDORLIST` (no vendor-list
  gossip outcome), `PrepareQuestMenu`, escort `Start` arms,
  VIPORE self-cast 5759 + faction-template flips.
- `npc_AV_troops_chief` `QuestComplete`: shout emote, ground-assault
  reset + GO status on 6901/6846 (escort Start UNMODELED).
- `go_av_landmine` + `go_av_beacon` on the `GoAiState` host:
  landmine `OnUse` hostile despawn + default trap proceeds (fork
  `UpdateAI` respawn-delay arm runs while despawned — DB-respawn
  cycling unmodeled); beacon ctor entry side/`GAMEOBJECT_LEVEL`/
  faction 83/84, 60 s invocation → war rider 13178/13161 summon +
  delete, `OnUse` delete. Dynamic-GO creation
  (`SPELL_EFFECT_SUMMON_OBJECT`) remains the Slice-3 recorded
  remainder, so DB-spawned instances are what bind today.
  `SetOwnerGuid` has no Rust GO owner field yet.
- `spell_av_create_shredder` (21544/21565): generic
  `spell_script_on_summon` dispatch added at the summon-effect site;
  stamps `UNIT_CREATED_BY_SPELL = EffectTriggerSpell[1]` +
  `UNIT_FIELD_CREATEDBY = caster`. General summon creator-field
  writes (non-shredder spells) remain unwritten — broader wire
  surface, recorded.

Verification: `./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s3step5b2.log`);
`live_av` extended — blood turn-in counter +5 via production
`quest_script_on_rewarded`, smith hello menu 6073 → select menu
6784 via production gossip dispatch, landmine GO 179325 + Balinda
11949 materialization; `LIVE-AV-PASS`. Bench rebuilt/restarted
(listening 18099), `TWO-CLIENT-SMOKE-PASS`. Unverified: AV CreatureAI
combat pulls (Alliance-only fixture; enemy-team pull probe remains).
Escort family (`AV_NpcEventAI`, troops/world-boss escorts) stays
UNMODELED behind the Slice-2 `npc_escortAI` framework remainder.

## Slice 3 steps 6–7 — BG leftovers + spirit guide (2026-10-29)

Reachability census (fork `vmangos/src/` as oracle):
- Spells 23383/23384 (flag clicks): fork `EffectScriptEffect` =
  `EventPlayerClickedOnFlag` + stealth/invisibility aura strip +
  GO delete. The Rust GO-use flag path (`gameobject_use`, types
  24/26) already strips both aura types before dispatch; base-flag
  removal rides `bg_spawn_event` despawns. Verified covered, no
  code change (strip order differs from fork — before vs after the
  event; no observable difference for carrier/return arms).
- Battleground-mark mail overflow: fork-dead. `RewardItem` and
  `SendRewardMarkByMail` have zero callers fork-wide; marks flow
  through win/lose spells 24950–24955 (`CREATE_ITEM` 20558/20559/
  20560) → `DoCreateItem`, whose full-bag remainder drops. Rust
  already mirrors that chain end to end (`bg_reward_mark` → spell
  cast → `MapEvent::CreateItem` → `store_created_item` plan/store
  with silent overflow drop). `bg_reward_item` doc corrected.
- Spell 30882 (EPL tower-capture test): gates every target on
  `IsOutdoorPvPActive`; the OutdoorPvP system has no Rust runtime
  (Tier-2 recorded), so the DND chain stays inert here exactly as
  it would with OutdoorPvP inactive. No code; remainder stands.
- Spirit guide (`battleground.cpp`): CreatureAI 30 s heal-wave
  timer was ported in Slice 2; `DamageTaken` zeroing rides the
  damage-taken dispatch. Added this step: `GossipHello` arm
  self-casting `SPELL_WAITING_TO_RESURRECT` 2584 triggered
  (fork sends no menu; host returns the default-text empty menu,
  klinfran precedent). `CorpseRemoved` repop and the 22012
  area-resurrect effect ride the recorded player ghost/corpse
  flow gap; `InterruptNonMeleeSpells` before the wave cast has
  no helper (channel restart approximates).

Verification: `./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s3step6.log`);
bench rebuilt/restarted (listening 18099), `TWO-CLIENT-SMOKE-PASS`.

## Tier 3 closeout audit (2026-10-29)

Requirement-by-requirement, against the goal text:
1. Runtime vmap manager + Height/LOS consumers — Slice 1 sections
   above (tile cache, `GetLosHitPosition`, vmap half of
   `UpdateAllowedPositionZ`, Blink/`GetNearPoint` rewires).
2. Content-script corpus (223 files by region/kind) — Slice 2
   sections above; instance/GO/CreatureAI/spell/aura/gossip/
   quest/areatrigger/dummy/process-event/GO-quest hosts all
   landed with per-batch live markers.
3. Battleground engine + WSG/AB/AV rules + content scripts +
   Tier-2 BG leftovers — Slice 3 steps 1–7 above.
4. Tier-2 remainders touched only when reached; the rest stay
   recorded (escort framework, dynamic-GO creation, OutdoorPvP,
   ghost/corpse flow, HonorMgr rank/standing, game events,
   ~34 `NO-DISPATCHER-YET` spell hooks, EK/Kal rolling spot
   review).

Fresh closeout verification (this session, uncommitted tree):
`./ci.sh` `CI_EXIT=0` (`/tmp/ci_t3s3step6.log`); live suite
`LIVE-WSG-PASS`, `LIVE-AB-PASS`, `LIVE-AV-PASS`,
`live_creature_scripts` 12/12; bench `mangosd` rebuilt and
listening on 18099 with `TWO-CLIENT-SMOKE-PASS`; smoke fixtures
guid 3/4 restored to their bench home positions, guid 1
untouched. Working tree: 68 changed paths, all under
`vmangos-rs/` and `docs/`, HEAD still `680d0eba` (uncommitted
per owner instruction; nothing pushed to production paths).
