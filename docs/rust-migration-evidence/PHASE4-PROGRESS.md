# Phase 4 Progress — Simulation Cluster

Status: **in progress** (evidence/design slice). This file is the durable
milestone/evidence record for Phase 4 of the C++-to-Rust migration
(`docs/RUST_MIGRATION_ARCHITECTURE.md` §13.2 Phase 4, crate map §12).
Baseline: commit `1bec69f` (Phase 3 complete). All Phase 4 changes stay
uncommitted unless the owner asks.

## Ground rules (carried from Phases 0–3)

- Mixed model routing: wonder-full (inherit) for driver/synthesis/critic
  and semantics-heavy first-of-kind work; muse-spark-1.3 only via
  Workflow per-call `model` for high-volume mechanically checkable work
  after a driver-built exemplar; never verify with a weaker model.
- Writes limited to `vmangos-rs/` and `docs/` (dev-bench fixtures
  outside git at `~/azeroth-dev-fixtures/`). Local dev-bench
  verification only; no production deployment or droplet mutation.
- Character load stays **read-only**; character save is Phase 6. The
  Phase 3 write register (7 statements, see PHASE3-PROGRESS.md) must
  not grow silently — any new write is recorded here before use.
- Owner decisions 2026-10-18 stand: FFI-retain C++ content scripts;
  MySQL-only (PostgreSQL dropped from Rust scope).
- Oracles are independent (C++ source text at production `2bae5f9` +
  raw SQL/DBC), never harvested from Rust output.
- Unsafe confined to `mangos-detour` and the future content-scripts
  FFI shim crate; workspace otherwise `unsafe_code=forbid`.
- In-game smoke tests per slice are the owner checkpoint
  (`.claude/rules/testing.md`); the standing real-client logon gate
  from Phases 1–3 remains open until the owner runs it.

## Slices (order fixed by the inventory critic, PHASE4-INVENTORY.md §2)

0. Codecs + field table + tick contract (MovementInfo codec,
   generated 5875 UpdateFields table, written 50 ms/100 ms diff
   contract). **Milestone A. DONE 2026-10-18.**
1. Entity core + UpdateData (fixture characters only; read-only
   Item creates before any real-DB login). Machinery DONE
   2026-10-18; Player/Creature/GO aggregates land with their
   first consumers (Slices 2–3).
2. Spawn data loaders (pure, parallel with Slice 1; pools joined
   from day one, pooled spawns withheld-and-logged). DONE
   2026-10-19 (see milestone log).
3. One map + visibility + login completion + movement relay
   (owner approved the write list 2026-10-19). DONE
   2026-10-19 (see milestone log).
4. Server movement: speeds/ACKs, spline launch, minimal
   Chase + Home/evade (promoted per critic). DONE
   2026-10-19 (see milestone log).
5. Spells/auras minimum (one nuke + one DoT; CheckCast subset).
   DONE 2026-10-19 (see milestone log). ← Slice 6 current
6. AI: ThreatHost + BasicAI, then EventAI/Conditions subset
   (DB usage triage first). 6a (threat/BasicAI/melee/
   spell lists) DONE 2026-10-19; 6b (EventAI/DB scripts/
   Conditions) DONE 2026-10-19 (see milestone log).
   Slice 6 complete.
7. Content: Deadmines vertical slice. **Owner decision
   2026-10-19: hand-port Deadmines to native Rust** (supersedes
   the 2026-10-18 FFI-retain default for this slice; the FFI
   question for EK/Kalimdor bulk remains open for a later
   phase). No FFI shim crate was created in Phase 4. DONE
   2026-10-19 (see milestone log).
8. Evidence closeout: loaded perf baseline (§14 #49), §14
   annotations only where evidence closes items, full `ci.sh`
   gate, completion audit.

## Owner decisions pending (from PHASE4-INVENTORY.md §4)

- Content strategy confirm/switch (above; Slice 7 only).
- ~~Phase 4 write-list~~ **Decided 2026-10-19 (owner)**:
  presence (`characters.online`), expired-respawn DELETEs at
  boot, and respawn-time INSERT/UPDATE/DELETE during play are
  approved on the dev bench. Position persistence stays out
  (Phase 6). See Write register.
- Pre-1.12 progression builds: 1.12.1-only assumed.
- Spell effects 132/133 (null in C++ dispatch): replicate inert
  assumed; one DB usage query first (Slice 5).
- Parity quirks: HOVER check stays disabled (Object.cpp:119-121)
  until decided; adaptive visibility, GridUnload, BG distance
  533 vs 180 need live prod conf.

## Tick/diff contract (Slice 0, binding for Slices 3–6)

- The world actor ticks at 50 ms (Phase 3). Each tick computes
  `diff` as **real elapsed milliseconds** since the previous tick,
  never an assumed 50.
- C++ `IntervalTimer`/`ShortIntervalTimer` (`Timer.h:73-123`)
  semantics port 1:1 at first use (Slice 3): `Update(diff)`
  accumulates, `Passed()` is `current >= interval`, `Reset()`
  subtracts **one** interval (remainder preserved); interval 0
  always passes. `TimeTracker` counts down, `Passed()` at `<= 0`.
- Map sub-limiters (players/cells/packets; 100 ms in inventory
  evidence, exact config keys pinned in Slice 3) are accumulators
  fed `diff`; when they fire they hand the **accumulated** diff
  onward. Nothing counts ticks.
- All gameplay timers (cast time, aura periodic `timer += period`
  re-arm, EventAI 500 ms poll, DB-script schedule) consume
  accumulated diff exactly as their C++ counterparts do.
- `MapUpdateInterval` is capped at `MIN_MAP_UPDATE_DELAY`
  (50 ms) in C++ (`MapManager.h:115-122`); the Rust 50 ms actor
  tick already matches the cap.
- Map tick internal order is fixed in Slice 3 to
  `Map.cpp:933-1060` (sessions → players → cells →
  SendObjectUpdates → relocation visibility → players), minus
  the continent barrier busy-wait (`:981-993`), which Rust
  scheduling does not reproduce (owner parity decision pending;
  divergence is wire-invisible by inventory analysis).

## Write register

Baseline: Phase 3 register, 7 statements (boot `account.current_realm=0`,
`characters.online=0`, battleground instance reset, realmlist
icon/tz + online + population, corpse DELETE, auth `account.last_ip`,
uptime INSERT/UPDATE). Phase 4 additions **approved by owner
2026-10-19** (dev bench only; implemented in Slice 3, listed
here as they land):

- `characters.online = 1` at login completion / `= 0` at logout
  (the boot-wide `= 0` sweep is already in the baseline).
- `DELETE FROM creature_respawn / gameobject_respawn WHERE
  respawn_time <= UNIX_TIMESTAMP(NOW())` at boot (C++ parity;
  replaces the Slice 2 in-query filter at the binary level —
  the loader stays read-only and filter-capable).
- Respawn-time persistence during play: `creature_respawn` /
  `gameobject_respawn` INSERT-or-UPDATE on death/despawn and
  DELETE on respawn, matching the C++ persistent-state saver.

## Milestone log

- 2026-10-18: Goal created (mixed model routing embedded). Evidence
  inventory workflow launched across 8 surfaces (map/visibility,
  movement, entities/updates, spells/auras, AI/script engines,
  content-script FFI, spawn-data loaders, Rust-side gaps) +
  critic + synthesis.
- 2026-10-18: First inventory run's 8 surface results were lost to
  result-size/cross-run handoff failures (critic's direct anchors
  survived). Re-run with file-based handoff succeeded: 8 digests
  in `phase4-inventory/` (565–765 lines each), critic 14/14
  spot-checks confirmed + count corrections (212 AddSC not 441;
  SpellScript 14 / AuraScript 15 hooks), synthesis
  `PHASE4-INVENTORY.md` (671 lines). Key outcomes: slice order
  corrected (codecs/entities before map; chase promoted; M1a/M1b
  split); `2bae5f9`↔HEAD token-proven formatting-only for
  Map/Object/Spell.cpp + MovementInfo codec; spell data is
  world-DB `spell_template`, not Spell.dbc; no SmartScript in
  tree; content-script ABI is 675 methods (FFI pricing → owner
  decision above). Evidence/design slice closed; Slice 0 started.
- 2026-10-18: Slice 0 (Milestone A) implemented. (1) MovementInfo
  codec in `mangos-movement::movement_info`: seeded read with
  jump latch, ctime-read/stime-write asymmetry, CorrectData
  sanitisation (HOVER rule stays disabled per Object.cpp:119-121),
  VerifyMovementInfo bounds (transport box 250/250/100, summed
  position), SetAsServerSide ordering; server clock passed in
  (documented layering deviation). 13 golden tests hand-derived
  from Object.cpp:61-172 (28/32/44/52/76-byte layouts, cos-before-
  sin, latch sequence). (2) 5875 UpdateFields:
  `scripts/update_fields_gen.py` generates
  `mangos-protocol::update_field_table` from
  `UpdateFields_1_12_1.cpp` rows only (324 rows, census +
  contiguity asserted, regen-stable; PLAYER_END=1282,
  CONTAINER_END=122 trap pinned); handwritten `update_fields.rs`
  adds TypeMask/types/flags, the 5 const flags arrays
  (SetupUpdateFieldFlagsArray semantics), name/offset lookups,
  and the DB-index remap (identity on 5875). `TypeId::Object`
  returns None where C++ logs + null (documented). 7 tests.
  (3) Tick/diff contract recorded above; IntervalTimer types
  land with their first user in Slice 3. Gate: fmt + clippy
  `-D warnings` + tests for both crates; ruff clean on the
  generator. No §14 change yet (inventory §5 annotations land at
  Slice 8 closeout, per plan).
- 2026-10-19: Slice 1 machinery implemented. New crate
  `mangos-entities`: `Values` (words+mirror, typed lanes, u64
  mirror poison, flag/mod helpers, create/update mask builders,
  LoadValues/LoadIntoDataField with atol semantics; setters
  report changed, entity layer marks). `ObjectBase` (identity,
  `_Create`, scale/entry, in-world lifecycle, delayed-mark
  plumbing returning list-operation decisions instead of C++
  virtuals). Block builders in `update.rs`: create/values/
  movement/destroy/direct-public with the full per-viewer
  rewrite chain in C++ arm order (NPC/dynflags/faction/cast-
  target/corpse words arrive as entity-computed `RewriteInputs`;
  attack-time + stat float→u32, GM flags, health-% (f32 math),
  FFA strip, cast-speed floor computed in-builder; GO
  quest-activation recorded as `BuildEffects`). Protocol:
  `UpdateMask` (copy/HasData bug documented, not replicated),
  `UpdateData`/`MovementData` (32 KiB split-before-append,
  out-of-range first packet only, compression threshold/level
  from World.cpp:557-559; failed compression drops the packet
  where C++ sends it corrupt — documented). Tests: 12 values +
  9 update goldens hand-derived from Object.cpp (GO create
  layout, creature LIVING create with ROOT strip + health-%,
  private-field separation, 41-block player mask, marking
  lifecycle, direct-update mirror sync). Two test-author errors
  were caught by the port both times (HasShortFlag u8 quirk,
  attack-time float storage) — code was right, tests fixed.
  No wire captures exist, so "sniff-compare" is hand-derived
  goldens, same standard as the Phase 2 spline builder.
- 2026-10-19: Slice 2 DONE. All in `mangos-world-data`, each
  live-verified against independent SQL oracles (dev bench,
  `--ignored` suites): (a) `creature_template.rs` — 78-column
  loader + CheckCreatureTemplate family, 10,381 templates;
  deviations: auras parsed at load, spell-existence/DBC fix-ups
  run only when the caller supplies id sets (skips reported),
  `script_id` stays 0 with raw `script_name` kept (registry is
  Slice 6/7). (b) `gameobject_template.rs` — 8,984 templates,
  13 transport displays after the C++ 462/562 erasures.
  (c) `spawns.rs` — creature store 66,222 / grid 62,760, GO
  store 56,645 / grid 13,601, hand-computed home cell 538_701
  for guid 1, elevator transport guid 233 (map 209), reload
  does not double-add. C++ asymmetries kept faithfully:
  tail-invalid rows stay in store but never grid-add; creature
  out-of-patch rows get SPAWN_FLAG_DISABLED while GO rows are
  skipped; `instanciated_continent_instance_id = 0` pending
  Slice 3's map manager. (d) `spawn_support.rs` — creature
  addons 3,404 (progressive max-patch; there is **no**
  `creature_template_addon` table at this build — inventory
  digest corrected), equipment templates 5,124 entries/5,117
  surviving rows with the probability-0 flush quirk replicated
  (entry 646 present but empty), CLS for classes {1,2,8} at
  100-length tables with f32 interpolation tail (level-64
  warrior health 3384 = trunc(42 + 63/62 · 3289)); the C++
  `exit(1)` on missing CLS health becomes report errors (a
  loader library must not exit — boot layer decides);
  creature display addons 10,534 (progressive on `build` ≤
  5875) + GO display addons 1,618. (e) Respawn times,
  read-only half: C++ **deletes outdated rows at load**; that
  write is outside the Phase 4 write register, so outdated
  rows are filtered against `now_unix` in the query instead —
  the resulting store is identical (bench: 25 creature rows
  join spawns on matching maps, 0 GO rows). The deletion joins
  the Slice 3 owner write-list question. Continent instance
  fan-out (MAP0_FIRST..LAST; note the GO loader fans out
  without the ContinentsInstantiate config check — C++
  inconsistency) applies when entries meet persistent states
  in Slice 3, not at load. Gate: fmt + clippy `-D warnings` +
  crate tests + all 5 live suites green. Write register
  unchanged (still the Phase 3 seven).
- 2026-10-19: Slice 3 DONE. (a) Timers: `mangos-guid::timer`
  ports IntervalTimer/ShortIntervalTimer/TimeTracker/
  ShortTimeTracker 1:1 (subtract-one reset, remainder kept).
  (b) Player stats inputs: `player_info.rs` loads
  playercreateinfo (40 pairs, display ids from ChrRaces),
  player_classlevelstats, player_levelstats with C++ gap
  copy-fill, player_xp_for_level (gap = prev+100), and an
  exact transcription of `BuildPlayerLevelInfo` growth
  (loop [Max-1, level); my first hand-derived test value
  was wrong, the port right). (c) Aggregates in
  `mangos-entities`: `unit.rs` (WorldObject/Unit bases,
  speeds, create stats, GetStat unsigned-read quirk kept),
  `player.rs` materialization subset of LoadFromDB +
  InitStatsForLevel with naked UpdateMaxHealth/MaxPower —
  live-verified against independent SQL/DBC oracles (bench
  warrior 19: stats 46/34/42/22/26, naked max health 423 =
  183+240, saved health 181 restored, display 49, visible
  equipment incl. creator GUID). Deferred rows (auras,
  spells, quests, skills, actions, reputation, mail) are
  retained verbatim in `Player::deferred`. (d) `creature.rs`
  spawn subset (display rolls, CLS stats with rank rates,
  equipment roll incl. virtual-item prototype bytes, addon
  stand/sheath/emote/mount, respawn bookkeeping) —
  live-verified on spawn guid 1/entry 2843 (hp 2059 =
  round(1716·1.2), armor 2343). (e) `gameobject.rs` create
  subset incl. `UpdateRotationFields` f64 packing (Python
  oracle goldens) and large/infinite/despawn/server-only
  switches. (f) New crate `mangos-map`: cell activation
  from the spawn grid, camera visibility with unit/object
  grey distances + alive/ghost grid rules, creates/
  out-of-range, values replication, movement relay with
  watcher re-evaluation on relocation — live test covers
  activation, mutual visibility, relay bytes, and the
  500-yard fall-out. No cell unload, no adaptive
  visibility, no continent barrier (recorded), creature
  ticks are inert until Slices 4/6. (g) mangosd wiring:
  boot loads DBC subsets + world stores + respawn times
  (expired-row DELETEs run first — the first boot removed
  the bench's 25 expired rows, exactly C++ behavior; the
  Slice 2 respawn test now seeds its rows in a rolled-back
  transaction), login materializes + presence online=1 +
  verify-world at the materialized position + map add +
  init-self, MSG_MOVE_* routes to the map, logout is
  immediate (LOGOUT_COMPLETE, online=0), maps tick with
  real elapsed diffs. Evidence:
  `vmangos-rs/scripts/phase4_scripted_world.py` →
  SCRIPTED-WORLD-PASS (verify-world, compressed updates,
  PRESENCE-ONLINE-1/0, movement accepted, enum after
  logout); Phase 3 script still SCRIPTED-LOGIN-PASS.
  Gate: `./vmangos-rs/ci.sh` with all fuses lit — GATE 0,
  101 suites, zero skips; new live suites were added to
  ci.sh. Write register now: Phase 3 seven + presence +
  boot respawn DELETEs + respawn-row clears (DELETE on
  expired-row consumption); respawn-time REPLACE on death
  lands with creature death in Slice 4/6. The bench
  characters DB was mutated by boot/login exactly as the
  register allows (online flags, expired respawn rows);
  no other tables. Owner real-client logon gate remains
  open for gameplay verification.
- 2026-10-19: Slice 4 DONE. (a) Spline runtime:
  `MoveSpline::interrupt`, `UnitBase` spline + unit-state
  bits, `Map::launch_spline` (Launch port incl.
  first-vertex correction, flag mixing, stop form),
  `update_spline_movement` with chunk continuation,
  `StopMoving`/`DisableSpline`, spline embedded in
  create blocks. Bug found by the Chase test and fixed:
  stop launches need a 2-point path (`MoveTo` sizes to
  2) — a 1-point spline panicked in `init_catmull_rom`.
  (b) MotionMaster enum stack in `mangos-map::motion`
  with Idle/Point/Random/Waypoint/Home/Chase generators
  (Mutate expire-vs-interrupt, Clear, expire+Reset,
  factory default from `default_movement_type`);
  waypoint paths from a new `waypoints.rs` loader
  (84,478 nodes at bench boot; counts match an
  independent SQL oracle per table). (c) Detour
  navmesh PathInfo subset: lazy `.mmtile` loading (3x3
  around query points), FindWalkPoly/findPath/
  findStraightPath, far-from-poly boundary adjust,
  force-destination tail; Home teleports unless
  PATHFIND_NORMAL exactly like C++. Deviations:
  straight-path extraction instead of findSmoothPath,
  no poly-path reuse across calls, query allocated per
  calculate, creature swim/fly shortcut arms absent,
  Chase lacks spreading/back-movement/extrapolation
  and LOS is always true (no vmaps), heights are
  GridMap-only. Tile selection bug found+fixed en
  route: `.map`/`.mmtile` names use the reversed
  `TerrainInfo::GetGrid` pair printed as (y, x) — the
  earlier Slice 3 height check had silently fallen
  back to the z hint; the live test now proves real
  tile heights. (d) Speed/ACK loop: pending movement
  changes + counter on UnitBase, SetSpeedRate
  controller/spline branches, observers-after-ACK,
  SetWalk toggle, ROOT/water-walk/hover/feather-fall
  flag changes; world actor dispatches movement by
  registry handler name (name-prefix routing gone) and
  implements force-speed/flag/root/spline-done ACKs
  (exact pending match or drop; knockback/teleport
  ACKs parse+drop, no senders yet; wrong-ack anticheat
  counter and the 1.14 root-on-landing kick not
  reproduced). UpdateSpeed aura math lands in Slice 5
  (callers pass final rates). Evidence: live_motion
  (waypoints vs SQL, ground vs saved z + real-tile
  proof, wander/waypoint movement + MONSTER_MOVE
  delivery, Point→Home, Chase approach+follow, FORCE
  counter/flat bytes 0/10.5, wrong-counter rejection,
  SPLINE_SET broadcast); scripted world adds
  MONSTER-MOVE-OBSERVED and BOGUS-ACK-DROPPED; full
  gate GATE 0, 103 suites, zero skips. Perf baseline
  (§14 #49, first measurement): bench boot loads all
  world stores in <1s at log granularity (17:37:42
  start and loaded lines same second); populated
  Westfall map (23 creatures in activated cells) ticks
  at 59 µs/tick debug over 400 ticks incl. spline
  integration, generators, and pathfinding on demand.
  Write register unchanged. Owner real-client logon
  gate remains open.
- 2026-10-19: Slice 5 DONE. (a) New crate
  `mangos-spells` (pure data + math): `SpellEntry`
  subset + `SpellEffectData`, DBC aux tables
  (CastTimes/Duration/Range/Radius), and ported
  `CalculateSpellEffectValue`/`GetCastTime` (rank-0
  cast-time subset, no spell chains yet)/
  `CalculateDuration` (duration only, no combos/mods)
  /`CalculatePowerCost`/`CalcArmorReducedDamage`/
  `ditheru`/`IsPositiveEffect`. `spell_template` loads
  in `mangos-world-data` via the C++ max-build query
  with positional `Fields` mapping and the amplitude
  0→5000 load fixup; bench boot logs
  `>> Loaded 22381 spells` (35,752 rows across builds).
  Effects 132/133 inert assumption is now evidence:
  0 bench rows. (b) Cast engine in `mangos-map::spells`
  on the map tick: known-spell check over the deferred
  `character_spell` rows, CheckCast subset (alive,
  power, GCD `StartRecoveryTime`, spell/category
  cooldowns, single-unit/self/party targets, range with
  bounding radii), `SPELL_START/GO/CAST_RESULT/FAILURE/
  FAILED_OTHER` packets, instant completion inside the
  cast call, movement interrupts timed casts, client
  handlers `CMSG_CAST_SPELL/CANCEL_CAST/CANCEL_AURA`
  by registry handler name. (c) Hit/damage exact C++
  formulas: `MagicSpellHitChance` (22% floor, [100,
  9900] hundredths, binary-spell resist multiplier with
  the `SpellMgr::IsBinary` rule), `MeleeSpellMissChance`
  at level-maxed skills, `GetSpellResistChance` incl.
  innate + vulnerability arms, the 31-row
  `RollMagicResistanceMultiplierOutcomeAgainst` table
  verbatim (resistance-70 row sums to 89 in C++, kept;
  DoTs at a tenth chance minus the 4 named spells;
  100% bucket folds to 75%), spell crit from the
  `crit_data` table (+50% magic bonus), armor reduction
  for the NORMAL school only. Effects:
  SCHOOL_DAMAGE (+PERIODICAURALOG on ticks,
  SPELLNONMELEEDAMAGELOG direct), HEAL (no heal log
  yet), APPLY_AURA/PARTY. (d) Auras: holders in
  `mangos-entities::aura`, slot allocation (positive
  0–31, negative 32–47), AURA/FLAGS/LEVELS/APPLICATIONS
  field writes, `SMSG_UPDATE_AURA_DURATION` on apply/
  refresh, duration 0 → permanent (recorded: subset
  spells carry real durations), same-spell+caster
  refresh, periodic ticks with the C++ timer re-arm,
  MOD_ATTACK_POWER flat mods as TWO_SHORT sums,
  cancelable cancel, saved `character_aura` rows
  applied at login (defensive stance + resurrection
  sickness on the bench, 2 holders). (e) Damage applies
  UNIT_FLAG_IN_COMBAT and `DealDamage` death: cast/aura
  cleanup, motion clear, `UnitDied` event, creature
  respawn scheduled from `respawn_delay` with the
  owner-approved `REPLACE INTO creature_respawn (...)
  instance 0 + map` (StoreCreatureRespawn event →
  PresenceWriter) and DELETE on respawn
  (ClearCreatureRespawn); respawn re-materializes from
  the DB row. Recorded deviations: no projectile delay
  (speed > 0 spells fail), no LOS/facing (vmaps not
  wired), no pushback/channels/combos/area targets, no
  spell mods/talents/bonus coefficients (identity), no
  dodge/parry/block, no resource generation yet (rage/
  mana only spent), MOD_STAT/MOD_*_SPEED/MOD_ROOT/
  MOD_STUN stored but inert (UpdateSpeed aura math
  still pending — open item carried to Slice 6/AI),
  corpse stays until respawn (no decay), player death
  is state-only. Oracle: `spell_math_oracle_gen.py`
  (independent Python from DB+DBC, 10 spells × levels
  1/19/60) drives `mangos-spells` test; live_spells
  (SQL-oracle bounds): REND 6546 snapshot amount 7 =
  SQL [7,7], 4 ticks = 12000/3000, tick logs [4,4,4,4]
  (armor-reduced), health delta = log sum; BATTLESHOUT
  5242 AP pos mod 38 (38.5 truncated) + slot field +
  cancel clears; FIREBLAST 2136 damage 21 within
  [27,35] after a 25% partial-resist bucket; death
  fires UnitDied + StoreCreatureRespawn. Scripted
  client adds UNKNOWN-SPELL-DROPPED, CAST-RESULT-NO-POWER
  (77 at 0 rage), CANCEL-CAST-TOLERATED; Phase 3 script
  accepts the new UPDATE_AURA_DURATION (311) pre-PONG.
  Full gate GATE 0, 109 suites, zero skips. Write
  register: respawn persistence line implemented as
  approved (no new statements). Owner in-game smoke
  (Deadmines clear with affected role) still required
  before this slice is gameplay-verified; real-client
  logon gate remains open.
- 2026-10-19: Slice 6a DONE (threat/BasicAI/melee).
  (a) `mangos-map::ai`: per-creature runtime over the
  existing `mangos-threat` manager with a `ThreatHost`
  impl on the map (identity threat modifiers; validity
  = alive + player-vs-creature). Proximity aggro uses
  `GetAttackDistance` (template detection range else
  the 25∓level-diff formula, z clamp 3) gated on the
  creature being in the player's view
  (`MoveInLineOfSight` notifier semantics); react
  states from template data (totem/critter passive,
  NO_AGGRO/civilian defensive, else aggressive).
  `AttackedBy`: Slice 5 `deal_damage` adds 1:1 damage
  threat and engages. BasicAI loop: leash evade
  (template leash range else max(2x detection, 50)
  from combat start), `get_hostile_target` victim
  selection (110/130% rules in the threat crate),
  Chase via Slice 4, spell lists, evade (threat/auras/
  motion cleared, Home, out-of-combat max/3 regen per
  second for health and mana), player combat drop
  after 5 s unreferenced. (b) Melee: swing timers on
  `UnitBase`, `RollMeleeOutcomeAgainst` with integer
  hundredths and C++ order (miss, dodge, parry,
  glancing with the Baeza reduction, block with value,
  crit with the skill adjustment, crushing), skill
  bonuses at level-maxed weapon/defense skills,
  low-level creature scaling, evade outcome vs evading
  targets, armor before outcome multipliers per
  `CalculateMeleeDamage`, `SMSG_ATTACKERSTATEUPDATE`
  5875 layout, `SMSG_ATTACKSTART/STOP`, player
  CMSG_ATTACKSWING/ATTACKSTOP/SET_SELECTION. Player
  dodge/parry/crit percentage fields now materialize
  from the agility tables (parry gated on spell 3127);
  creature chances are the Unit defaults (5% family,
  NO_PARRY/NO_BLOCK extra flags honored). Creature
  MINDAMAGE/AP field words land at spawn
  (`UpdateDamagePhysical` identity arm) and refresh
  with the AP-scale factor when MOD_ATTACK_POWER auras
  apply/expire; player fields are the unarmed 1..2
  bases. (c) `creature_spells` loader (2,595 lists at
  boot) + `DoSpellsListCasts` subset (1.2 s cadence,
  probability, ONLY/NOT_IN_MELEE, AURA_NOT_PRESENT,
  TARGET_CASTING, TRIGGERED/INTERRUPT flags; target
  types self/victim/random-threat, others fall back
  to the victim). Spawn/addon auras (`pending_auras`)
  now apply at spawn. Recorded deviations: no
  CallAssistance, no faction reactions (hostility is
  player-vs-creature), static flags beyond critter/
  totem types not consulted, CF_MAIN_RANGED_SPELL
  kiting and spell-list scripts unmodeled, in-combat
  mana regen identity, melee facing/leeway is
  reach-only, BASE_ATTACK only, player AP damage
  factor stays 1.0 (naked-stat subset has 0 base AP),
  GuardAI/TotemAI/EventAI/scripted names fall back to
  BasicAI cores until 6b/7. Evidence: live_ai vs SQL
  (pillager aggro + ATTACKSTART, damaging swing 19
  inside armored bounds [14,43] from the cls variance
  band, Rend threat 5, leash evade + threat cleared +
  home at full health, player swings land 285->282);
  scripted adds ATTACK-SWING-BOGUS-TOLERATED; boot
  loads 2,595 creature spell templates. Note:
  live_spells and live_ai raise the player dummy
  health pool in-memory because Slice 3 naked stats
  leave the bench warrior at very low max health and
  the now-hostile world kills it mid-scenario
  (observed live: correct evade-on-victim-death).
  Full gate GATE 0, 111 suites, zero skips. Write
  register unchanged. Owner in-game smoke still
  required before gameplay-verified.
- 2026-10-19: Slice 6b DONE (EventAI + DB scripts +
  Conditions). New crate `mangos-scripts-engine`
  (pure data: ScriptInfo/EventAiEvent/Condition/
  BroadcastText/ScriptText + stores). Loaders in
  `mangos-world-data::script_engine` mirror the C++
  structural checks (bench boot: 6243 EventAI events,
  7381 creature AI script steps, 2996 generic steps,
  1843 conditions, 11108 broadcast texts, + 1772
  movement + 18 creature-spells steps). Signed
  `event_param*` columns wrap like C++ `GetUInt32`
  (169 negative rows found by the live test).
  Runtime in `mangos-map::scripts` + `ai.rs`:
  `AiKind::EventAi` over the Basic core, holders with
  the 500 ms cadence, phase masks, EFLAG semantics
  (REPEATABLE/RANDOM_ACTION/NOT_CASTING/CHECK_RESULT),
  hooks for SPAWNED/AGGRO/DEATH/KILL (player victims)/
  HIT_BY_SPELL/SPELL_HIT_TARGET/HIT_BY_AURA/
  MOVEMENT_INFORM/EVADE/REACHED_HOME (+Reset)/
  LEAVE_COMBAT/SCRIPT/RECEIVE_EMOTE (targeted, from
  CMSG_TEXT_EMOTE), OOC_LOS polled at AI rate,
  movement scripts at waypoint arrivals, spell-list
  companion scripts. EventAI actions execute
  immediately in (delay, priority) order — faithful:
  every bench `creature_ai_scripts.delay` is 0 and
  this fork's ProcessAction uses StartDirect; generic
  scripts schedule by delay seconds (map-ms clock,
  finer grain than game-time seconds — recorded).
  ~55 SCRIPT_COMMANDs implemented by bench usage
  (TALK 998, CAST_SPELL 2685, FLEE 1573, SET_PHASE
  family 516, EMOTE, MOVE_TO/MOVEMENT, FIELD_SET/
  MODIFY_FLAGS, DESPAWN/RESPAWN/LOAD spawns, GO
  doors/state/custom anim, factions/morph/mount/
  equipment/stand/sheath, threat rewrites, inst data
  (instanced maps only, in-memory), server variables
  (in-memory only — C++ persistence would be a new
  write, consciously NOT added to the register),
  cooldown adds/removes, combat/melee/react/move
  toggles, ZONE_COMBAT_PULSE, CALL_FOR_HELP (also
  closes the 6a CallAssistance gap for scripts),
  TERMINATE_SCRIPT/CONDITION, SEND_SCRIPT_EVENT...).
  DEAL_DAMAGE replicates the C++ self-target quirk.
  Unsupported (log-shaped no-ops): TEMP_SUMMON (197)
  and the quest/item/taxi/gossip/game-event/
  map-event/creature-group/guardian commands with no
  backing systems yet. Conditions evaluator covers
  the recursive EventAI closure types answerable today
  (AND/OR/NOT, AURA, TEAM, quest state from deferred
  rows, SAVED_VARIABLE, SOURCE_ENTRY, NEARBY_*,
  WOW_PATCH=10, MAP_ID, DISTANCE_*, HEALTH/MANA
  percent, IN_COMBAT, IS_ALIVE, DB_GUID, IS_MOVING,
  IS_PLAYER, LOS=true, LOCAL_TIME on UTC); game-event/
  instance/map-event/group condition types read false
  (recorded). Friendly script targets use the slice
  hostility approximation. Flee = AI state + repeated
  away-points for CreatureFamilyFleeDelay 7000
  (FleeingMovementGenerator approximation; seek-
  assistance falls back to plain flee) + broadcast
  1150 text emote. Chat: SMSG_MESSAGECHAT monster
  layouts, gendered broadcast text, listen ranges
  25/300/25, zone variants map-wide (no zone ids),
  no server-side placeholder substitution (matches
  this fork's MonsterChatBuilder). Emote state
  emotes degrade to command emotes (no Emotes.dbc);
  text-emote animations need EmotesText.dbc.
  SetInvincibilityHpThreshold floors deal_damage.
  Evidence: live_eventai vs independent SQL —
  pillager TIMER_OOC Frost Armor aura, HP<=15% FLEE
  (fled 12.3 yd + flee emote text from SQL), entry
  8503 100% AGGRO talk "I'll cut you!" exactly the
  SQL broadcast text with sender-guid match; the
  pillager 15% aggro SAY was also observed live
  during development (sibling spawn, exact SQL
  text). Scripted socket: TEXT-EMOTE-ECHO +
  EMOTE-TOLERATED, SCRIPTED-WORLD-PASS,
  SCRIPTED-LOGIN-PASS. Full gate GATE 0, 115 suites,
  zero skips. Write register unchanged (despawn
  respawn persistence reuses the approved REPLACE
  statement). Owner in-game smoke still required
  before gameplay-verified.
- 2026-10-19: Slice 7 DONE (Deadmines hand-port +
  temp summons). This fork's Deadmines is mostly
  DB-driven: the hand-port surface was three C++
  units (instance script, gunpowder/cannon/lever GO
  scripts, boss_mr_smite); every other boss already
  runs on Slice 6 EventAI/spell lists. Enabling
  systems built with this slice: temp summons
  (synthetic spawn rows via a materialize_creature
  extraction, local guid band 0xF00000+, TempSummonType
  lifecycle incl. corpse cleanup that never touches
  the DB respawn path, EventAI SUMMONED_UNIT/JUST_DIED/
  JUST_DESPAWN hooks), SCRIPT_COMMAND_TEMP_SUMMON
  (197 bench rows now live), spell effects SUMMON/
  SUMMON_WILD/SUMMON_GUARDIAN (Eject Sneed 5141,
  VanCleef's Allies 5200), triggered casts may now
  start and finish from a dead caster (death-event
  casts), LOCATION_CASTER_DEST/SRC target modes start
  (victim expansion for caster AoE remains the
  recorded Slice 5 gap), CMSG_GAMEOBJ_USE with a
  door/button default toggle, hidden objects
  (VISIBILITY_OFF) in the visibility scan.
  Deadmines specifics: instance state only on map
  36 with a real instance id (door chain on
  Rhahk'Zor/Sneed/Gilnid deaths via kill hooks,
  43199/43201 patrols hidden with 60 s/30 s delayed
  activation at faction 17, cannon → iron door +
  Smite alarm sequence + pirate MovePoint, gunpowder
  overseer two-leg walk, SET_INST_DATA routed through
  instance reactions, GetData64 for the iron door,
  CONDITION_INSTANCE_DATA live); Mr. Smite as a
  script-bound AI state on the Basic core (stomp at
  66/33%, chest run + kneel + equipment swap,
  thrash/slam timers, evade reset). Recorded
  deviations: lazy by-entry instance object lookup,
  DMF chest stays hidden (no area triggers), Smite
  random-attack-point reposition left to chase,
  instance maps are not yet reachable from the world
  server (no portal/binding path; login still falls
  back to homebind — the slice is proven by direct
  instance-map construction). Evidence:
  live_deadmines vs independent SQL — patrol hidden
  (0 visible) → Rhahk kill → door 1 ACTIVE + 3
  patrol visible after 60 s; gunpowder use → overseer
  summoned at the C++ coords and walked 29.3 → 6.2 yd
  to the second leg point; Shredder death → Sneed
  ejected → kill → door 2 ACTIVE; Gilnid → door 3;
  cannon use → iron door ACTIVE + both alarm texts
  (SQL broadcast 1148/1149); Smite 65% → SAY_PHASE_2
  + kneel next to chest + axe display 7427 (SQL
  item 2183), 30% → SAY_PHASE_3 + hammer display
  19766 (SQL item 10756). Scripted socket adds
  GAMEOBJ-USE-BOGUS-TOLERATED. Full gate GATE 0, 117
  suites, zero skips. Write register unchanged.
  Loaded perf re-measured with the full Slice 4-7
  stack: PERF ticks=400 creatures=23 elapsed_ms=44
  us_per_tick=110 (was 23/59 at Slice 4; same shape,
  still three orders under the 50 ms tick budget on
  the bench host). Owner in-game smoke (Deadmines
  clear) still required before gameplay-verified,
  and it needs an instance-entry path the world
  server does not have yet.
- 2026-10-19: Phase 4 CLOSEOUT (completion audit
  against §13.2 Phase 4 + the session goal).
  Delivered and gated (ci.sh GATE 0, 117 suites,
  zero env skips at close): maps/visibility/movement
  (Slices 3-4), entity aggregates over the read-only
  21-query character load (Slices 1-3; domain
  *behaviors* beyond combat — inventory use, quest
  updates, talents — remain deferred-row data, as
  scoped: save is Phase 6, services Phase 5),
  spells/auras subset (Slice 5, differential oracle),
  Threat/BasicAI/melee/creature spell lists (6a),
  EventAI + DB scripts + Conditions (6b), Deadmines
  content hand-port (7). Write register audited by
  grep: exactly the Phase 3 seven statements plus the
  owner-approved 2026-10-19 presence + respawn lines;
  saved variables/instance data/despawned-GO respawn
  times stay in-memory by design. Unsafe scan: none
  outside mangos-detour (workspace forbid intact).
  Perf (§14 #49): micro-baseline recorded in the
  Slice 7 entry; production load test still open.
  §14 annotated where evidence closed items (#7/#50
  partial, #35 tail, #49 measurement, §15.1
  decisions). Remaining open by design, NOT done:
  (1) owner in-game smoke tests (Deadmines clear
  with the affected role) and the standing
  real-client logon gate from Phases 1-3 — no slice
  is gameplay-verified until the owner runs them;
  the Rust world server also has no instance-entry
  path yet, so a real-client Deadmines clear is not
  even reachable today; (2) content beyond
  Deadmines: bench counts 725 creature templates /
  446 distinct script_names (904 world-map spawns,
  2001 instanced) and 214 GO script_names — all but
  the three Deadmines units currently fall back to
  Basic/EventAI cores; the EK/Kalimdor bulk strategy
  (FFI shim vs continued hand-port) is the owner's
  open §15 decision; (3) recorded slice deviations
  (spell/AoE/proc coverage, LOS, straight-path
  movement, condition types reading false, TEMP_SUMMON
  corner semantics) listed per slice above.
  Phase 4 engineering is complete pending those
  owner gates; changes remain uncommitted on
  rust-migration per standing instruction.
