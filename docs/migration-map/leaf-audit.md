# Leaf audit: 294 measured crates vs subsystem catalog

Bead: `as-pnw.6`. Epic: `as-pnw` (Map VMaNGOS codebase before re-planning Rust port).
Depends on: `as-pnw.1` (`docs/migration-map/subsystems.md` — live/dead/vestigial inventory).
Method: static analysis only. No builds, no test runs, no server boot.

## 1. Count reconciliation (read first)

The bead text says **646 existing crates**. The measured workspace says **294**.

- Source measured: `origin/rust-migration` tip `57889b5` (`chore: ignore vmangos-rs build artifacts`, 2026-09-27), path `vmangos-rs/crates/`.
- Measurement: `git ls-tree -d --name-only <tip> vmangos-rs/crates/ | wc -l` = **294**. Every one of the 294 directories contains `src/lib.rs` (`git ls-tree -r --name-only <tip> vmangos-rs/crates/ | grep lib.rs | wc -l` = 294).
- History: workspace starts at 17 crates (`f5ea8ae`, 2026-09-27, `feat(vmangos): add vmangos-rs workspace with shared foundation crates`) and grows to 294 by `63c8b3b` (`fix(vmangos): satisfy current clippy lints in older leaf crates`). No branch or commit found with 646 crate directories.
- The `main` branch (`1490d74`, this worktree's base) contains **zero** crates — there is no `crates/` or `vmangos-rs/` directory checked out here. All crate evidence below comes from reading the `origin/rust-migration` tip via `git show` / `git archive`, not from the local checkout.

**Audit scope decision:** this document audits the **measured 294 crates**, clustered below. If a 646-crate workspace exists elsewhere (unpushed, or counted by a different rule such as test/example targets), it was not visible from this worktree and the per-cluster verdicts still apply — clusters are defined by subsystem, not by headcount. Dynamic follow-up 1 records the re-count.

## 2. Inventory summary

- Total Rust: **93,327 lines** across the 294 `src/lib.rs` files (`awk` sum over `wc -l`), mean **~317 lines/crate**.
- Size spread is extreme. Smallest: `mangos-homebind` (39), `mangos-rez-sickness` (41), `mangos-farsight`/`mangos-null-ai` (42). Largest: `mangos-battle-waypoints` (6,933), `mangos-check-cast` (3,976), `mangos-bot-behavior` (2,446), `mangos-proc` (2,183), `mangos-anticheat` (1,828). The long tail is single-gate leaves (~40–150 lines: boolean verdicts over 2–4 scalars plus tests); the head is four god-leaves that each deserve to be their own cluster.
- Inter-crate dependencies: **19 of 294** crates declare a `path = "../mangos-*"` dependency (parsed from `Cargo.toml`, excluding the `path = "src/lib.rs"` lib-path line). The other **275 are standalone**. The dependency graph is nearly flat: `mangos-xp` (4 dependents: `xp-gain`, `group-xp`, `reputation`, `honor`), `mangos-util` (4: `gmticket`, `health-leech`, `mana-leech`, `proc`), `mangos-bignum` (2: `hash`, `srp6`), `mangos-pet-kind` (2: `pet-abandon`, `pet-xp`), plus 9 single edges (`guid-alloc→guid`, `guild-roster→guild-rank`, `honor-weekly→honor-ranks`, `null-ai→creature-ai`, `rep-standing→reputation`, `social→bytebuffer`, `spell-rank→spell-target`). No cluster forms a coherent crate stack; each leaf re-declares its own scalars, constants, and error codes.
- C++ source concentration (from `//!` doc headers naming `vmangos/src/...`): `game/Objects` (111 mentions), `game/Handlers` (39), `game/Spells` (36), `game/AI` (16), `game/Movement` (13). Everything else is ≤8 mentions. Coverage is **game-logic leaves only** — see §4 for the infra gaps.
- Transliteration markers: **123 of 294** `lib.rs` files match at least one of `wrapping_*`, `as u32`, `coin_flip`, `inverted`, `fail-all`, `banker`, `truncat`, `mirror of`, `faithful`. Representative instances: `wrapping_sub` timestamp math in `mangos-anticheat`, `wrapping_*` byte cipher in `mangos-authcrypt`, `coin_flip` parameter for `urand(0,1)` halves in `mangos-reputation`, kept inverted SRP6 `Proof` in `mangos-srp6`, fail-all base32 decode in `mangos-base32`, `as u32` float truncation mirroring C++ `uint32(...)` casts in `mangos-group-xp`, `f64` pow/sqrt accumulation in anticheat overspeed, `SPELL_CAST_OK = 0xFF` never-sent sentinel in `mangos-check-cast`, first-max-wins ties and the strict 35% damage-credit gate. These are documented parity choices, and several of them are exactly what the bead means by "C++ quirks enshrined" — flagged per cluster below.

## 3. Verdict legend

Verdicts are **per crate CLUSTER**, per the bead:

- **keep**: port is sound at roughly this granularity; future work extends it without restructuring. Applies to foundation codecs, data tables, and the anticheat math core (parity is the point there).
- **merge**: logic is correct but the splitting is the bug — N crates cover one subsystem seam and must fold into one crate (one owner, one error type, one test surface) before the migration proceeds. This is the dominant verdict (275 standalone leaves).
- **redo**: the cluster ports the wrong thing, or ports it at the wrong layer — gate-soup over folded scalars that discards the ownership/threading context the Rust seam design (`as-pnw.7`) must decide. Keep the tests as vectors, rewrite the shape.

Three flags modify a verdict:

- **dead**: crate covers code `as-pnw.1` ruled dead/vestigial. **No cluster earns this flag** — see §4.
- **dup**: two or more crates model overlapping state or adjacent lines of the same function. Named explicitly so `as-pnw.7` can pick one owner.
- **translit**: crate preserves a C++-specific behavior (wrapping, truncation, sentinel, inverted proof, fail-all decode, coin-flip rounding) that a re-plan must either re-specify as protocol/product behavior or deliberately drop. Translit is not automatically bad — for wire crypto and anticheat timing it is required parity — but it must be a conscious decision, not an accident of transcription.

## 4. What the crates do NOT cover (gaps, not verdicts)

Against the `as-pnw.1` subsystem inventory, entire layers have zero (or one incidental) leaf:

| Subsystem (`subsystems.md` §) | Crate coverage | Note |
|---|---|---|
| `framework/` (30 files: Grid, NGrid, TypeContainer, Singleton, EventMap) | 2 mentions (`framework/Utilities` in `events`, `event-processor`) | Grid/NGrid/TypeContainer/Singleton have no leaves. Correct — these are ownership/threading decisions for `as-pnw.7`, not leaf math. Do not backfill with transliterated headers. |
| `shared/IO` (socket backbone), `shared/Database` (MySQL + delay threads) | 0 leaf mentions each | Only incidental DB-lookup doc references (e.g. `auction`, `guild-add` name stored procedures as caller-side). Async DB, delay threads, `IoContext` are threading-map (`as-pnw.4`) concerns, unported. |
| `game/Maps` (Map, MapManager, GridMap, ObjectGridLoader, ScriptCommands, pools) | 2 mentions (`pool`, `geometry`) | No Map/MapManager/GridStates/InstanceData leaves. Correct at leaf stage. |
| `game/vmap` (BIH, DynamicTree, GameObjectModel, MapTree) | 0 | No vmap leaves. Good — keeps the audit clear of the one dead header (`VMapTools.h`) and the vestigial `TileAssembler` pair. |
| `realmd` / `mangosd` (entries, AuthSocket, WorldRunnable, Master) | 1 incidental each (`config` parses both `.conf` formats) | No auth-flow, world-tick, or remote-console leaves. Correct. |
| `scripts/` content (257 files) | 2 mentions (`scripts/world` in the two world-event leaves) | Zone/boss scripts unported except via event leaves. Expected. |

**Dead-code check (explicit):** zero leaves reference `VMapTools.h`, `TileAssembler`/`AssembleTile`, `DO_POSTGRESQL`/`DatabasePostgre`/`QueryResultPostgre`/`PGSQLDelayThread`, or `ScriptLoader_noscripts`. The four `as-pnw.1` dead/vestigial rows have **no Rust coverage** — the right outcome. No `dead` verdicts are issued in §5, and `as-pnw.7` must keep it that way (do not create crates for dead headers, the Postgre trio, or the no-scripts fallback).

## 5. Per-cluster verdicts

### C1 — Foundation codecs and math core — **keep**

Crates (~21): `mangos-guid`, `mangos-guid-alloc`, `mangos-bytebuffer`, `mangos-flags`, `mangos-queue`, `mangos-events`, `mangos-event-processor`, `mangos-geometry`, `mangos-util`, `mangos-utf8`, `mangos-time`, `mangos-config`, `mangos-base32`, `mangos-bignum`, `mangos-hash`, `mangos-authcrypt`, `mangos-srp6`, `mangos-geometry`, `mangos-static-flags`, `mangos-spline` (math half), `mangos-money` (scalar half).
C++ sources: `shared/` (ByteBuffer, EnumFlag, LockedQueue, Util, Duration/TimePeriod/Timer, Config, Crypto/BigNumber/SRP6/AuthCrypt, banker note in `Formulas.h`), `framework/Utilities/EventMap`, `game/Geometry.h` + `NormalizeMapCoord`, `game/ObjectGuid.h/.cpp`.
Rationale: this is the original 17-crate workspace (`f5ea8ae`) plus mechanical additions. Each crate owns one well-specified codec or math unit with round-trip tests (LE codec + packGUID, RFC 4648 vectors, RFC 2202 HMAC vectors, full SRP6 handshake test). The deliberately-not-ported list in `vmangos-rs/README.md` (TypeList, LinkedList, CreationPolicy, ThreadingModel, Singleton) is correct — those are call-site idioms, not leaves.
Translit flags (accepted, keep as parity): `wrapping_*` cipher arithmetic in `authcrypt`, Euclidean `%` + LE `SetBinary` + 0-on-error `SetHexStr` in `bignum`, inverted `Proof` in `srp6`, fail-all decode + budget gate in `base32`. These mirror wire/protocol behavior; changing them breaks interop. Record them as specified behavior in `as-pnw.7`, do not "fix" them.
No dup. No dead.

### C2 — XP / reputation / honor progression math — **merge** (dup, translit)

Crates (11): `mangos-xp`, `mangos-xp-gain`, `mangos-group-xp`, `mangos-explore-xp`, `mangos-damage-credit`, `mangos-reputation`, `mangos-rep-standing`, `mangos-faction-standing`, `mangos-honor`, `mangos-honor-ranks`, `mangos-honor-weekly`.
C++ sources: `game/Formulas.h` (`XP::*`, `xp_in_group_rate`), `game/Group/Group.cpp:1236-1274,2278-2286` (group slice), `game/Objects/Player.cpp` (`CalculateReputationGain`, `RewardReputation`, `GetHonorGain`), `game/ReputationMgr.h/.cpp` (rank tables, `SetOneFactionReputation`, `SetAtWar`), `game/Database/DBCStructure.h` (`FactionTemplateEntry`), `game/HonorMgr`.
Rationale: one progression-rewards seam split across 11 crates. The layering is real (`xp` ← `xp-gain`/`group-xp`/`reputation`/`honor`; `reputation` ← `rep-standing`; `honor-ranks` ← `honor-weekly`) but the splits are single-function: `rate_for` (one line, `group-xp`), `rep_points_to_rank` (one iterator, `reputation`), ladder durations. Several crates re-derive `gray_level` instead of sharing it. Merge into one `progression-rewards` crate with modules (`xp`, `group`, `reputation`, `honor`); keep `faction-standing` (pure DBC mask logic) as its submodule, not a crate.
Dup (explicit): `reputation` vs `rep-standing` vs `faction-standing` (three crates for gain math, store update, and template masks — one `ReputationMgr` seam); `xp` vs `xp-gain` vs `group-xp` vs `explore-xp` (four crates for one `Formulas.h` + two `Group.cpp` ranges); `honor` vs `honor-ranks` vs `honor-weekly` (gain vs tables vs weekly decay).
Translit (re-specify in `as-pnw.7`): `coin_flip` halves in `reputation::round_float` (C++ `urand(0,1)` — nondeterministic product behavior enshrined as a parameter; decide deterministic rule), `as u32` truncation of float shares (C++ `uint32(...)` casts), first-max-wins ties in `group-xp::gather` (strict `<`), strict 35% `damage-credit` gate with corpse-raid bypass and wrapping products, `488.0` racial-leader constant and post-1.9 end-of-rank cap in honor. Keep the tests as vectors; the merged crate must document each as specified behavior.

### C3 — Melee / combat core — **merge** (dup, translit)

Crates (~16): `mangos-melee-chance`, `mangos-melee-outcome`, `mangos-melee-taken`, `mangos-combat-swing`, `mangos-combat-timer`, `mangos-attack-swing`, `mangos-attack-timer`, `mangos-absorb-resist`, `mangos-armor`, `mangos-block`, `mangos-block-roll`, `mangos-crit-bonus`, `mangos-weapon-damage`, `mangos-threat`, `mangos-aggro`, `mangos-taunt`.
C++ sources: `game/Objects/Unit.cpp` (`AttackerStateUpdate`, `DealDamage`, `CalculateMeleeDamage`), `game/Objects/Unit.h` (attack timers), `game/Threat/ThreatManager`, `game/StatSystem.cpp`, `game/Formulas.h` (weapon/armor pieces).
Rationale: one combat-resolution pipeline (swing timer → chance → outcome → taken → threat) shattered into per-gate crates. `attack-swing` vs `combat-swing` vs `melee-outcome` cover adjacent lines of the same `AttackerStateUpdate` flow; `block` vs `block-roll` is one roll split in two. Merge into a single `melee-combat` crate; `threat`/`aggro`/`taunt` become its `threat` module (2 `Threat` mentions in the corpus — the manager itself is unported, only the math leaves exist).
Translit: wrapping products in damage-credit-adjacent paths, `f64` vs `f32` accumulation choices, first-max/tie rules copied from group code. Re-verify each constant against `Unit.cpp` once, in the merged crate, rather than once per leaf.

### C4 — Spell pipeline (cast → check → effects → aura) — **merge**, with C4a **redo**

Crates (18 `spell-*` + ~14 satellites): `spell-batch/cost/reagents/range/target/rank/duration/crit/hit/resist/positive/bonus/power/prepare/cast/update/cancel/finish`, `cast-spell`, `cast-time`, `check-cast`, `gcd`, `pushback`, `interrupt`, `reflect`, `dispel`, `immunity`, `breakable-aura`, `holder-update`, `aura-charges/removal/stack/tick`, `proc`, `proc-trigger`, `dot-ramp`, `spell-target`.
C++ sources: `game/Spells/Spell.cpp` (cast, check, update, cancel, finish), `game/Spells/SpellMgr.cpp`, `game/Spells/SpellAuras.cpp`, `game/Spells/SpellEffects.cpp`, `game/UnitAuraProcHandler.cpp`.
Rationale: the spell system is one state machine; the crates model its phase-transition predicates one predicate per crate. The predicates are individually well-tested but the composition — the thing that must be right — lives nowhere. Merge into one `spell-pipeline` crate with `cast`, `check`, `effects`, `aura`, `proc` modules. Keep the per-gate tests; they become module tests.
Dup: `spell-cast` vs `cast-spell` (names alone prove the split is unprincipled — adjacent lines of `Spell::cast`/`prepare` in two crates); `aura-charges/stack/tick/removal` (four crates for one aura lifecycle); `proc` vs `proc-trigger` (chance selection vs trigger gate, same `UnitAuraProcHandler.cpp` flow); `spell-cancel` vs `spell-finish` vs `spell-update` (three teardown-phase crates).
Translit: `SPELL_CAST_OK = 0xFF` never-sent sentinel, `SPELL_FAILED_*` code table, `SPELLFAMILY_*` Loatheb constants, PPM normalization (`speed_ms * PPM / 600`), customChance-overrides-base then ppm-overrides-custom precedence in `proc::select_proc_chance`. All must be re-specified as protocol behavior in the merged crate.

#### C4a — `mangos-check-cast` (3,976 lines) — **redo** (translit)

This single crate is larger than most clusters. It ports `Spell::CheckCast` (`Spell.cpp:5472-5535`) as a flat gate-list over folded scalars. The gates are correct and heavily tested, but the shape — one function taking dozens of pre-folded booleans — pushes every ownership question (aura scans, DB lookups, shapeshift tables, terrain probes) to an unspecified "caller-side" that does not exist yet. Keep every test vector; redo the crate as a staged validator (`check` module of the merged spell crate) whose stage boundaries match the seam design, not the C++ line order. The `SharedDefines.h:1738` pettame constant and Loatheb family table move with it as data.

### C5 — Loot pipeline — **merge** (dup)

Crates (11): `loot-access/chance/error/fill/money/open/perm/release/roll/take/valid`.
C++ sources: `game/LootMgr.cpp`, `game/Handlers/LootHandler.cpp`, `game/Objects/Player.cpp` loot lines.
Rationale: eleven crates for one loot flow (access → valid → roll → fill → money → take/release). `loot-take` (980 lines) is the only substantial one; the rest are 1–3-gate leaves (`loot-error` is an error-code table, `loot-perm` a permission predicate). Merge into one `loot` crate; error codes become its error enum. No translit beyond standard `as u32` truncation; no dead.

### C6 — Quest / gossip / trainer — **merge**

Crates (~8): `quest-accept/dialog/req/reward/status`, `gossip`, `trainer`, `gossip-adjacent `petition``.
C++ sources: `game/QuestDef`, `game/Objects/Player.cpp` quest lines, `game/Handlers/QuestHandler` (via MiscHandler mentions), `game/Conditions`.
Rationale: one quest-dialog flow (`req` → `dialog` → `accept` → `reward` → `status`) split per handler arm. Merge into one `quest` crate with `gossip`/`trainer` as dialog-source modules. `condition` stays shared (used beyond quests).

### C7 — Item / equipment / inventory — **merge** (dup)

Crates (~14): `item-enchant/fit/move/proficiency/state/wrap`, `destroy-item`, `open-item`, `read-item`, `use-item`, `equip-cooldown`, `set-ammo`, `ammo`, `durability`, `repair`, `inventory-pos`, `bank`, `slot-buy`.
C++ sources: `game/Objects/Item.cpp`, `game/Objects/Bag`, `game/Handlers/ItemHandler.cpp`, `game/ItemEnchantmentMgr`.
Rationale: one inventory seam (pos → fit → move → state → enchant/durability) split per opcode arm. `item-fit` vs `inventory-pos` vs `item-move` are three views of one slot-validation predicate. Merge into one `item-inventory` crate. `bank`/`slot-buy` (purchased-slot math) fold in as a submodule, not separate crates.

### C8 — Pet / charm subsystem — **merge** (dup)

Crates (15): `pet-abandon/action/actionbar/ai/autocast/command/event-ai/kind/loyalty/respec/slot/spells/tp/xp`, `scripted-pet-ai`.
C++ sources: `game/Objects/Pet.cpp`, `game/Handlers/PetHandler.cpp`, `game/AI/` pet-adjacent selectors.
Rationale: one pet seam (kind → slot → spells → action/actionbar → command/autocast → AI → xp/loyalty/tp) split per method group. `pet-ai` (1,118 lines) and `pet-action` (895) are the substance; `pet-kind` is the shared enum both depend on (one of the 19 real inter-crate edges — correctly factored, keep that shape inside the merge). `pet-event-ai` vs `event-ai` vs `scripted-pet-ai` triple-models the AI-dispatch hierarchy statically; the virtual-dispatch caveat from `as-pnw.1` applies — merge into one `pet` crate with `ai` module rather than three crates re-describing vtables. Merge `pet-xp`/`pet-tp`/`pet-loyalty` (all progression-shaped) as submodules; note the overlap with C2 and resolve ownership in `as-pnw.7` (proposal: progression math lives in C2's crate, pet calls it — do not duplicate `gray_level` a third time).

### C9 — Movement generators — **merge**, with spline/transport carved out for redo

Crates (~16): `chase-move`, `confused-move`, `cyclic-move`, `fear-move`, `fleeing-move`, `home-move`, `idle-move`, `point-move`, `random-move`, `motion-master`, `follower`, `leash`, `reach`, `fall`, `move-ack`, `move-speed`, plus `escort` (C11-adjacent, listed here for movement behavior).
C++ sources: `game/Movement/` (`MotionMaster`, all `*MovementGenerator`s, `FollowerRefManager`), `game/Handlers/MovementHandler.cpp`, `game/Objects/Player.cpp` fall/farsight lines.
Rationale: one movement-generator family split per generator class — the most defensible 1:1 split in the workspace, and still wrong as crates. Each generator is 100–300 lines of pure target-selection math over caller-provided positions; the composition (MotionMaster priority,Ack ordering, heartbeat timing) is unported. Merge into one `movement` crate with one module per generator; `motion-master` becomes its scheduler module, not a crate. `move-ack` (935 lines) and `fall`/`heartbeat` carry the timing-critical transliteration (wrapping clock compares, `MIN_FALLING_TIME_MS` gate, skipped-heartbeat threshold) — keep those tests verbatim in the merge.
Translit: `wrapping_sub`/`wrapping_add` clock arithmetic throughout (C++ `uint32`/`int32` timestamp wrap — required parity for anticheat-adjacent timing, keep and specify); `move-ack` arrival-callback folding.
Carve-out: `spline`, `transport`, `waypoint`, `waypoint-mgr` are NOT in this merge — see C10/C13.

### C10 — Spline / transport / maps adjacency — **redo** (scope)

Crates (4): `mangos-spline`, `mangos-transport`, `mangos-geometry` (map-coord half), `mangos-pos-selector`.
C++ sources: `game/Movement/spline/*`, `game/Transports/Transport*.cpp`, `game/Maps/GridDefines.h` (`NormalizeMapCoord`), `game/Objects/ObjectPosSelector`.
Rationale: these four crates sit at the boundary between pure math (spline interpolation, coordinate normalization — keepable) and unported ownership (Transport is a `GameObject`-adjacent world entity ticked by Maps; `ObjectPosSelector` is used by `Object.cpp:2579` in a grid context). As leaves they compile, but a merged `movement` home would enshrine the wrong seam — transport position authority and spline evaluation belong to the Maps/world-tick design (`as-pnw.4`, `as-pnw.7`), not to a leaf-math crate. Redo: keep spline-interp and coord-normalize as pure functions (tests survive), but re-home transport/pos-selector logic when the map-ownership seam is designed. Do not extend these crates until then.

### C11 — Creature / NPC AI — **merge**

Crates (~12): `creature-ai`, `basic-ai`, `critter-ai`, `null-ai`, `ai-select`, `scripted-ai`, `event-ai`, `guard-ai`, `guard-mgr`, `totem-ai`, `escort`, `creature-group`.
C++ sources: `game/AI/` (41 files: CreatureAI/BasicAI/CritterAI/GuardAI/EventAI, Registry, Selector), `game/CreatureGroups`, `game/GuardMgr`.
Rationale: one AI-dispatch hierarchy (`Registry`/`Selector` → `CreatureAI` overrides → `EventAI`/`GuardAI`/`TotemAI` specializations) flattened into per-class crates. `null-ai → creature-ai` is a real edge (the idle-AI default) and the correct shape — inside one crate it becomes a constructor, not a dependency. `event-ai` + `pet-event-ai` (C8) + `totem-ai` triple-describe the same event-dispatch table; merge all three as modules of one `creature-ai` crate. The `as-pnw.1` virtual-dispatch caveat is load-bearing here: static gate lists understate reachability through `CreatureAIRegistry`/`Selector`, so the merged crate must be reviewed against the registry, not just the generators. `guard-mgr`/`creature-group` (summon/group membership math) fold in as modules.

### C12 — Group / guild / social — **merge** (dup)

Crates (~16): `group`, `group-loot`, `group-xp` (math; see C2), `guild-add/rank/roster`, `guild-bot`, `social`, `chat-channel/dispatch/flood`, `mail`, `petition`, `who-list`, `query`, `target-icons`, `inspect`, `meeting-stone`, `lfg-roles`.
C++ sources: `game/Group/Group.cpp`, `game/Guild/Guild*.cpp`, `game/Chat/` (Channel, MasterPlayer/AbstractPlayer), `game/Handlers/` (Group/Guild/Chat/Query), `game/Mail/Mail.cpp`, `game/LFG/`, `game/SocialMgr`.
Rationale: one social seam (group ↔ guild ↔ chat ↔ mail) split per handler file. `group` (929 lines) is the substance (membership, loot method, XP gather — though the XP math belongs to C2); `guild-roster` (869) re-derives `guild-rank` (a real edge — keep that layering inside the merge). `chat-channel` vs `chat-dispatch` vs `chat-flood` is one channel pipeline in three crates. Merge into one `social` crate (`group`, `guild`, `chat`, `mail` modules); `group-xp` math moves to C2's crate with a call, not a copy. `social → bytebuffer` is a real edge (packet assembly) — keep as a dependency of the merged crate.

### C13 — Battlegrounds + battle-bots — **merge**, with C13a data **keep**

Crates (~12): `bg-av/bracket/flag/honor/node/premature/queue/type`, `battle-bot`, `battle-path`, `capture-point`, plus `bgs-adjacent honor` (C2).
C++ sources: `game/Battlegrounds/` (BG, Mgr, AB/AV/WS), `game/Handlers/BattleGroundHandler.cpp`, `game/PlayerBots/BattleBotAI` + waypoints.
Rationale: one BG seam (type → bracket → queue → premature → node/flag/capture → honor) split per struct. `bg-type`/`bg-bracket`/`bg-queue` are small pure predicates that belong together; `bg-av`/`bg-flag`/`bg-node`/`capture-point` are per-map scoring rules that belong together as data + scoring functions. `battle-bot` (903) + `battle-path` are the bot-driver half — see C15 for the bot-architecture verdict, but the BG half merges here regardless.
Dup: `bg-honor` duplicates C2 honor math at the BG boundary — resolve ownership in `as-pnw.7` (proposal: C2 owns honor math, this crate calls it).

#### C13a — `mangos-battle-waypoints` (6,933 lines) — **keep as data, split code from table** (translit)

The largest crate in the workspace is a mechanically transcribed waypoint graph (`BattleBotWaypoints`: per-BG registries, `no_reverse` sets, `Arrival` callbacks). Verdict is keep — but as DATA, not logic. The transcription shape (named `WSG_HORDEFLAGROOM_TO_HORDEGRAVEYARD`-style constants with float crumbs) faithfully preserves tunings that must not be re-derived. Required follow-up: split the table (generated `paths.rs`, checked against the C++ source) from the traversal logic (reverse-travel, arrival dispatch — currently caller-side stubs), and record the transcription provenance (source commit + generator, if any) so `as-pnw.7` can treat it as a frozen asset. Arrival callbacks, motion, and randomness stay caller-side by design — that boundary is correct.

### C14 — OutdoorPvP / world events / seasonal — **merge**

Crates (~8): `outdoor-ep`, `silithus`, `elemental-invasion`, `scourge-invasion`, `war-effort`, `holiday-events`, `game-event`, `dragons-adjacent escort` (no — escort stays C11).
C++ sources: `game/OutdoorPvP/` (EP/SI + Register), `game/GameEventMgr` + `HardcodedEvents`, `scripts/world/` (wareffort, invasions, seasonal `love_is_in_the_air`/`midsummer`), `game/Weather` (adjacent tick).
Rationale: one world-event seam (event schedule → zone capture → invasion spawns → seasonal quests) split per content drop. `outdoor-ep` vs `silithus` is literally the two `RegisterZoneScripts` arms from `as-pnw.1` §OutdoorPvP; they share the capture-point helper (`capture-point` currently sits in C13 — move it here or share it, do not copy it). Merge into one `world-events` crate. No translit beyond table constants; no dead (EP/SI are live-but-narrow per `as-pnw.1`, and the crates correctly cover only what exists).

### C15 — Player bots (custom stack) — **redo** (translit, scope)

Crates (~10 + consumers): `bot-behavior` (2,446), `bot-director`, `bot-patterns`, `bot-population`, `bot-service`, `bot-chat`, `bot-spellkit` (882), `formation`, `player-ai`, plus consumers `battle-bot` (C13), `guild-bot` (C12).
C++ sources: `game/PlayerBots/` (36 files: PlayerBotAI/Mgr, PartyBotAI, CombatBotBaseAI, BattleBotAI, BotBehavior(+ClassUnique/Shared), BotDirector, BotFormation, BotRoleConfig, BotServiceMgr, …), wired via `World.cpp`, `Group.cpp`, `PlayerBotMgr.cpp`.
Rationale: this is the custom bot stack — the one subsystem where `CLAUDE.md` names an architectural rule (new behaviors extend `BotBehavior`; the runner scores and falls through). The crates port individual behavior gates as free functions over folded scalars, which discards exactly the structure the re-plan must preserve: the scoring/fallthrough runner, role config, formation, spell-kit selection, and service lifecycle. `bot-behavior` + `bot-patterns` + `bot-director` triple-model the runner without owning it. Redo as a designed `bots` crate (or crate family, if `as-pnw.7` so decides) whose shape follows the runner — behavior trait, scoring, fallthrough composition — with the current gate tests ported as behavior unit tests. Do not merge these leaves into gameplay crates and do not extend them in place; per project rules, bot behaviors are added as new layers, and that rule survives the port.
Translit risk is highest here: bot tuning constants (auto-join ticks, population targets, spell-kit priorities) look like magic numbers in leaf form but are live product behavior (`BattleBot.AutoJoin`, `World.cpp:975`, `PlayerBotMgr.cpp:298-353/595-641`). Every constant needs a provenance comment before the redo.
