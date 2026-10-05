# Seam design: where crate boundaries should sit

Bead: `as-pnw.7` (child of the `as-pnw` migration-mapping epic).
Inputs: [protocol](protocol.md) (`as-pnw.2`), [persistence](persistence.md)
(`as-pnw.3`), [ownership](ownership.md) (`as-pnw.5`), plus
[subsystems](subsystems.md) (`as-pnw.1`), [threading](threading.md)
(`as-pnw.4`), [leaf audit](leaf-audit.md) (`as-pnw.6`).
Method: static analysis only. Nothing here was verified against a running
server; dynamic follow-ups from the input maps are inherited, not repeated.

Headline: the 294 measured leaves (`origin/rust-migration`, see leaf audit
§1) are **per-gate math over folded scalars with no composition owner**.
The re-plan converges them into **17 crates on three seams** — protocol
wire, persistence, and sim ownership — plus foundation codecs and one
frozen data asset. Merges are the plan; the only creates are the three
seam crates (which do not exist yet) and the redo targets named below.
No per-struct crates survive.

## 1. The three seams (boundary rules)

### Seam A — protocol wire (`protocol-wire`, create)

Owns everything the client can observe on the wire, and nothing else.

Contents (from protocol map §§census–gap-list):

1. Opcode enum: all 827 names with exact discriminants 0–825 + 827,
   `NUM_MSG_TYPES = 828`, value 826 reserved.
   Source: `vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h`.
2. Direction per opcode (CMSG / SMSG / MSG).
3. Gate table: session status per opcode (`NEVER` 525 / `LOGGEDIN` 290 /
   `AUTHED` 8 / `TRANSFER` 1 / `LOGGEDIN_OR_RECENTLY_LOGGEDOUT` 2),
   including the 8-entry `AUTHED` allowlist.
   Source: `Opcodes.cpp` `BuildOpcodeList()`, `Opcodes.h` `SessionStatus`.
4. Affinity table: `PacketProcessing` per opcode **with aliases preserved**
   (`SELF_ITEMS`→MAP, `DB_QUERY`→ASYNC, `CHANNEL`/`GROUP`/`GUILD`→WORLD)
   plus the SPELLS/MOVEMENT drain-frequency distinction.
   Source: `PacketProcessing.h`, `WorldSession::QueuePacket`.
5. Handler cross-ref: real handler vs `Handle_ServerSide` (359) vs
   `Handle_NULL` (163). The latter two are out-of-scope markers for the
   port, not behavior to reimplement.
6. Build-conditioned rows (`SUPPORTED_CLIENT_BUILD` gates) as
   build-selected variants, never unconditional entries.
7. Non-world-table protocol: `eAuthCmd` / `AuthResult` / `AccountFlags`
   (`realmd/AuthCodes.h`), SRP6 struct layouts (`AuthPackets.h`), the
   `SMSG_AUTH_CHALLENGE` → `CMSG_AUTH_SESSION` field order and SHA1
   construction, `{size, cmd}` header with endian/RC4 handling
   (`WorldSocket.cpp`), and the realmd 8-entry dispatch table
   (`AuthSocket.cpp:117`) with its connection-state gates.

**Boundary rule A1:** no gameplay crate names an opcode discriminant, a
gate status, or a crypto layout. Packet flows (`Handlers/*Handler.cpp`
homes, protocol map §flows) live in gameplay crates and *reference*
opcodes; the enum, gates, and crypto live here.

**Boundary rule A2:** transliterated wire behavior stays bit-exact and is
specified here, not "fixed": `wrapping_*` cipher arithmetic
(`authcrypt`), Euclidean `%` + LE `SetBinary` + 0-on-error `SetHexStr`
(`bignum`), inverted SRP6 `Proof`, fail-all base32 decode. These are
interop, not quirks.

### Seam B — persistence (`persistence`, create)

Owns durability semantics: what is written, in what transaction, with
what retention — and the parity bars that prove it.

Contents (from persistence map §§1–7):

1. DB topology as sealed roles: `mangos` read-mostly content (GM writes
   only, behind the privilege gate), `characters` mutable state (55
   tables), `logs` append-only best-effort, `realmd` auth decisions.
2. Save transaction boundaries: fat `REPLACE INTO characters` + sidecar
   saves in one transaction (`Player::SaveToDB`, `Player.cpp:16229`);
   `_SaveStats` outside it; anti-dupe inventory+gold fast path
   (`SaveInventoryAndGoldToDB`); lone-item-save orphan risk documented,
   not widened.
3. Mail state machine: send → expire → return-or-delete with exact delays
   (3 d COD / 30 d default / 1 h auction-pending), the return loop
   (player-to-player item mail renews with fresh 30-day expiry;
   system/returned/COD cascades), reaper throttle and skip-online rules.
4. Auction lifecycle: write-through rows, expiry→mail (never back to
   inventory), scheduler-watermark-only bot persistence.
5. Guild/group/petition write-through, pet/corpses/instance/respawn paths,
   startup hygiene (`CharacterDatabaseCleaner`).
6. Per-subsystem parity bars (persistence map §7) as acceptance tests:
   field-for-field player core, guid-stability + zero-new-orphans items,
   retention-semantics mail, lifecycle + money-conservation auction,
   membership-after-restart social, reset-time instance semantics,
   reclaimable corpses, read-parity world content, auth-decision parity,
   best-effort logs, reconstruction parity for memory-only state,
   no-residue bots.
7. Crash-loss budget: up to one `PlayerSave.Interval` (15 min default) +
   unflushed lone writes. The port preserves it or renegotiates in
   writing — it is not allowed to drift by accident.

**Boundary rule B1:** gameplay crates compute *decisions* (loot winner,
auction cut, mail to send); this crate owns *durability* (row shapes,
transaction scope, retention timers, guid sequencing). No gameplay crate
issues its own `REPLACE` semantics or invents a second mail-expiry rule.

**Boundary rule B2:** `mangos` (world content) is sealed at runtime. The
only write paths are GM tooling (`Creature.cpp`, `GameObject.cpp`,
`WaypointManager.cpp`, `GameEventMgr.cpp` in persistence map §4) and
they stay behind the same privilege gate — never in the hot loop, never
in a gameplay crate.

### Seam C — sim ownership (`sim-core`, create first)

Owns entity lifetimes and all cross-entity links. This is the highest-risk
seam and the one the port must build first (ownership map §8).

Model (from ownership map §§3–8, threading map §§2–3):

1. Single-owner arena keyed by stable handles (generational indices or
   GUID-keyed slab): `Map` arenas for grid objects, a
   session/character store for players/items. All links (threat, charm,
   follow, target, owner, mover) are handles resolved through the arena.
2. Explicit despawn queue drained by the tick schedule, replacing
   `m_deleted` + `m_objectsToRemove` with identical tick semantics
   (kill is deferred past iteration; `RemoveAllObjectsInRemoveList`
   ordering is the spec).
3. Edge tables replacing `RefManager`/`Reference<>` intrusive lists
   (threat, hostile, follow, group).
4. Per-component teardown systems replacing the `CleanupsBeforeDelete`
   virtual chain, with destructor-order couplings (corpse duality,
   resurrectable-corpse/pet global-list removal) made explicit.
5. Movement-intent components replacing `MotionMaster`'s owning
   `stack<MovementGenerator*>`; cast intents/events replacing `new Spell`
   + `delete this`; threat-table system replacing
   `ThreatManager`/`HostileRefManager`; aura storage keyed by owner
   handle; item handles for `Player::m_items[]` / `Bag::m_bagslot[]`.
6. The ~40 `MaNGOS::Singleton` globals (ownership map §9) become explicit
   resources passed to systems — including the full bot-stack set.
7. Threading contract (threading map): one world tick at 50 ms owns all
   game state; bounded parallelism fans out and re-joins inside the tick
   (map / object / visibility pools, with timeout-abort + leftover
   semantics); background threads own IO, DB delay queues (serial-id
   routing), async packets, and timers — never game state. Handles are
   `Send`-safe; `Rc<RefCell>` is rejected (threat/follow/owner cycles
   would leak, and the tick is multithreaded).

**Boundary rule C1:** no gameplay crate holds a raw entity pointer, a
GUID→pointer shortcut cache (`Player::m_mover` pattern), or a
rebindable `Player*` (`PlayerAI::me`). Resolution goes through the
arena every time; handles may be cached, pointers may not.

**Boundary rule C2:** no FFI wraps the C++ pointer graph. The first Rust
code owns lifetimes (arena + despawn queue + handle resolution) while
gameplay logic ports onto it. Wrapping first preserves exactly the
unsoundness the ownership map documents.

## 2. Target crate layout (16 crates)

| # | Target crate | Source cluster (leaf audit) | Verdict carried in |
|---|--------------|-----------------------------|--------------------|
| 1 | `core-codecs` | C1 foundation codecs + math core (~21) | keep |
| 2 | `protocol-wire` | — (does not exist; protocol gap list §7 items) | **create** |
| 3 | `persistence` | — (does not exist; persistence §§2–7) | **create** |
| 4 | `sim-core` | — (does not exist; ownership §8 model) | **create first** |
| 5 | `progression-rewards` | C2 xp/rep/honor (11) | merge |
| 6 | `melee-combat` | C3 combat core (~16) | merge |
| 7 | `spell-pipeline` | C4 + C4a `check-cast` (18 + ~14) | merge, C4a redo as `check` module |
| 8 | `loot` | C5 loot pipeline (11) | merge |
| 9 | `quest-dialog` | C6 quest/gossip/trainer (~8) | merge |
| 10 | `item-inventory` | C7 item/equipment (~14) | merge |
| 11 | `pet` | C8 pet/charm (15) | merge |
| 12 | `movement` | C9 generators (~16) | merge |
| 13 | `creature-ai` | C11 creature/NPC AI (~12) | merge |
| 14 | `social` | C12 group/guild/social (~16) | merge |
| 15 | `battlegrounds` | C13 BG + battle-bots (~12) | merge |
| 16 | `world-events` | C14 OutdoorPvP/events (~8) | merge |
| 17 | `bots` | C15 player bots (~10 + consumers) | **redo** as runner-shaped crate |
| — | `battle-waypoints` | C13a (6,933 lines) | keep **as data**: split generated `paths.rs` (frozen asset, transcription provenance recorded) from traversal logic |

Carve-outs that must NOT merge into the adjacent crate (leaf audit
C10): `spline` interpolation and coordinate normalization survive as
pure functions (tests kept); `transport` position authority and
`ObjectPosSelector` grid-context logic re-home to the maps/tick design
when it exists — not to `movement`. Infra with zero leaf coverage
(`framework/` grid/singleton, `shared/IO` sockets, `shared/Database`
delay threads, `game/Maps`, `vmap`, `realmd`/`mangosd` entries,
`scripts/` content) is designed at the seam level when its turn comes;
do not backfill with transliterated headers.

Explicit non-crates (subsystems map): `vmap/VMapTools.h` (dead),
`TileAssembler` pair (vestigial tool), the Postgre trio
(`DatabasePostgre`/`QueryResultPostgre`/`PGSQLDelayThread`), and
`ScriptLoader_noscripts` fallback. No crates are ever created for
these; the Postgre path is a `USE_POSTGRESQL` option-or-deletion
decision, not a port surface.

## 3. Leaf APIs that must change to compose

"Change" means the current per-crate function shape cannot be kept as a
public API of the merged crate — it folds into a module, takes handles
instead of scalars, or moves to a different owner. Per-gate tests
survive as module vectors; signatures do not.

**Ownership seam (blocks everything, ownership map §8).** All 13 doomed
leaf shapes must be redesigned, not transliterated:
`HashMapHolder<T*>` + `ObjectAccessor::Find*/Get*` bare-pointer queries
→ arena handle queries; `Map::Remove(T*, bool)` + `delete obj` /
`DeleteFromWorld` → despawn systems; `AddObjectToRemoveList` /
`m_deleted` / `m_objectsToRemove` + drain → explicit despawn queue;
`CleanupsBeforeDelete` chain → teardown systems;
`RefManager`/`Reference<>` → edge tables; `Creature::m_AI` +
`FactorySelector::selectAI` + `SetAI` → behavior components;
`PlayerAI::me` rebindable raw pointer → `PlayerHandle` (highest-risk
bot surface); `Spell` `delete this` + per-cast `new` → cast intents;
`MotionMaster` owning generator stack → movement-intent component;
threat/hostile managers → threat table; `m_spellAuraHolders` raw
multimap → handle-keyed aura storage; `m_items[]`/`m_bagslot[]` raw
arrays → item handles; GUID→pointer shortcut caches → arena resolution.

**Progression (C2 → `progression-rewards`).** `xp` vs `xp-gain` vs
`group-xp` vs `explore-xp` (one `Formulas.h` + two `Group.cpp` ranges in
four crates — `gray_level` derived in several places becomes one
function); `reputation` vs `rep-standing` vs `faction-standing` (gain
math / store update / template masks become modules, one owner);
`honor` vs `honor-ranks` vs `honor-weekly` (gain vs tables vs decay).
Translit re-specs owned by this crate: `coin_flip` halves
(nondeterministic `urand(0,1)` needs a deterministic rule),
`as u32` float-share truncation, first-max-wins ties, the strict 35%
damage-credit gate + corpse-raid bypass, the `488.0` racial-leader
constant and post-1.9 end-of-rank cap.

**Combat (C3 → `melee-combat`).** `attack-swing` vs `combat-swing` vs
`melee-outcome` (adjacent lines of one `AttackerStateUpdate` flow);
`block` vs `block-roll` (one roll split in two); `threat`/`aggro`/`taunt`
become the `threat` module (the manager itself is unported — only math
leaves exist). Cross-ownership: `damage-credit` gate policy is shared
with C2 — one definition, one caller direction.

**Spells (C4 → `spell-pipeline`).** `spell-cast` vs `cast-spell`
(adjacent `Spell::cast`/`prepare` lines in two crates);
`aura-charges`/`stack`/`tick`/`removal` (one aura lifecycle in four
crates); `proc` vs `proc-trigger` (chance vs gate of one
`UnitAuraProcHandler.cpp` flow); `spell-cancel` vs `spell-finish` vs
`spell-update` (three teardown phases). `check-cast` is restaged as the
`check` module whose stage boundaries match §1 seams, not C++ line
order. Translit re-specs: `SPELL_CAST_OK = 0xFF` sentinel,
`SPELL_FAILED_*` table, Loatheb `SPELLFAMILY_*` table, PPM normalization,
customChance-overrides-base-then-ppm precedence, pettame constant.

**Loot (C5 → `loot`).** All eleven `loot-*` arms fold; `loot-take` is the
substance, `loot-error` becomes the crate error enum, `loot-perm` a
predicate. Composition (access → valid → roll → fill → money →
take/release) is owned here for the first time.

**Items (C7 → `item-inventory`).** `item-fit` vs `inventory-pos` vs
`item-move` (three views of one slot-validation predicate become one);
`bank`/`slot-buy` purchased-slot math becomes a submodule.

**Pets (C8 → `pet`, calls C2).** `pet-ai` + `pet-action` are the
substance; `pet-kind` stays the shared enum (the one correctly-factored
edge — keep that layering inside the merge);
`pet-event-ai`/`event-ai`/`scripted-ai` triple-description of AI dispatch
becomes one `ai` module (virtual-dispatch caveat applies — review
against the registry); `pet-xp`/`pet-tp`/`pet-loyalty` become
submodules that **call** `progression-rewards` instead of re-deriving
`gray_level` a third time.

**Movement (C9 → `movement`).** Each generator becomes a module;
`motion-master` becomes the scheduler module (priority + ack ordering —
currently unported, must be written, not folded from a stub);
`move-ack` arrival-callback folding and `fall`/heartbeat timing gates
(`MIN_FALLING_TIME_MS`, skipped-heartbeat threshold, `wrapping_*`
clocks) keep their tests verbatim.

**AI (C11 → `creature-ai`).** `null-ai` becomes a constructor, not a
dependency; `event-ai` + C8 `pet-event-ai` + `totem-ai` merge as modules
(one event-dispatch table, three descriptions today);
`guard-mgr`/`creature-group` become membership modules.

**Social (C12 → `social`, calls C2).** `chat-channel` vs `chat-dispatch`
vs `chat-flood` (one channel pipeline in three crates);
`guild-roster` re-derivation of `guild-rank` keeps its layering inside
the merge; `group` membership/loot stays, but `group-xp` math moves to
`progression-rewards` (call, not copy); `bg-honor` (C13) likewise calls
C2. The `social → bytebuffer` path edge is real — the merged crate keeps
it as a dependency.

**BGs (C13 → `battlegrounds`).** `bg-type`/`bracket`/`queue` small
predicates join `bg-av`/`flag`/`node` + `capture-point` map scoring as
data + functions; `battle-bot` + `battle-path` driver half merges here
but stays split from the waypoint *table* (C13a), whose arrival
callbacks and traversal logic are caller-side by design.

**World events (C14 → `world-events`).** `outdoor-ep` vs `silithus` (two
`RegisterZoneScripts` arms) merge; `capture-point` helper lives here or
is shared — not copied.

**Bots (C15 → `bots`, redo).** `bot-behavior` + `bot-patterns` +
`bot-director` triple-model the scoring/fallthrough runner without
owning it. The redo shapes the crate around the runner (behavior trait,
scoring, fallthrough composition, role config, formation, spell-kit
selection, service lifecycle) and ports current gate tests as behavior
unit tests. Bot tuning constants (`BattleBot.AutoJoin`, population
targets, spell-kit priorities) each get a provenance comment first.
Project rule survives the port: new behaviors are new layers, and this
crate is where that rule is enforced.

## 4. Crate-consolidation rules for the re-plan (`as-pnw.8` input)

1. **Freeze then merge.** No new leaf crates after this design. Locks:
   no crates for dead/vestigial rows (§2 non-crates), no backfill of
   unported infra as transliterated headers, no extension of redo
   targets in place (C4a, C10 carve-outs, C15). Work moves clusters
   toward §2, never sideways.
2. **One owner per state; dups name a winner.** Every `dup` in §3 resolves
   to exactly one home: honor math → `progression-rewards`; group-xp math
   → `progression-rewards`; pet xp/tp/loyalty → callers of it; BG honor →
   caller of it; capture-point → one home shared, never copied;
   `gray_level` → exactly one definition. The re-plan review checks each
   dup line off; a merge that preserves two owners is not done.
3. **Composition lives in the merged crate.** The runner (bots), the
   pipeline (spells), the scheduler (motion-master), the lifecycle
   (loot), the table+traversal split (waypoints) are written where they
   are missing — merging leaves without adding the missing composition
   owner reproduces the current failure at larger granularity.
4. **Dependency direction is fixed.** Gameplay → `sim-core` handles,
   → `persistence` durability, → `protocol-wire` opcodes;
   `social` may depend on `bytebuffer`/`core-codecs`; small math (C2)
   is called, not copied. Foundation (`core-codecs`) depends on nothing.
   `sim-core` depends on nothing gameplay-shaped. The 19 existing path
   edges grow into this DAG; cycles are rejected at review.
5. **Handles at every seam.** Merged-crate public APIs take and return
   arena handles, table keys, and event intents — never raw pointers,
   never pre-folded boolean bags that push ownership questions to a
   nonexistent caller (the C4a failure shape). New public signatures are
   reviewed against rules C1/B1/A1.
6. **Tests are vectors, preserved.** Per-gate tests move with their logic
   into module tests (timing-critical: `move-ack`, fall/heartbeat,
   `wrapping_*` clocks; wire-critical: codec round-trips, SRP6 handshake,
   HMAC vectors). Merges that drop vectors to "simplify" are rejected.
7. **Translit needs a spec line.** Every `translit` item in §3 ships with
   one recorded decision: protocol/product behavior (kept bit-exact, with
   source) or deliberately dropped (with rationale). Accidental
   transcription is the failure; the list in §3 is the checklist.
8. **Tuning provenance before redo.** Bot constants, Loatheb tables,
   waypoint transcription (source commit + generator, if any), honor
   constants, and mail/auction timing knobs each carry a provenance
   comment before their redo/merge lands. Untuned numbers do not move.
9. **Data/code split for generated assets.** `battle-waypoints` sets the
   pattern: generated tables check in separately from traversal logic,
   frozen and diffable against the C++ source.
10. **Parity bars gate the seams, not the leaves.** `persistence` §7 bars
    and the threading tick contract (§1-C) are the merge acceptance
    criteria. A merged crate that passes its unit vectors but breaks a
    seam bar (orphan budget, retention semantics, tick ordering,
    crash-loss budget) does not land.

## 5. What the re-plan (`as-pnw.8`) still has to decide

- Order: `sim-core` first (nothing gameplay-shaped composes without
  handles), then `protocol-wire` + `persistence` (independent of each
  other), then merges in dependency order (C2 before its callers C8/C12/
  C13; spell-pipeline before aura-adjacent pet work).
- Whether `bots` is one crate or a family (runner + behaviors +
  services). This design requires the runner shape either way; the
  split is packaging.
- Whether maps/tick (`game/Maps`, threading §§2–3) becomes its own seam
  crate or lives inside `sim-core`. The threading contract in §1-C is
  required regardless of where it is housed.
- Dynamic follow-ups inherited from inputs (live capture for auth/enter-
  world/movement ordering; droplet read-only mail/item queries and conf
  values; off-tick `HashMapHolder` readers; AI-swap re-entrancy; respawn
  identity; spell-reap timing; bot logout ordering; corpse teardown
  order; map-cadence clamp confirmation) — each needs a running server
  and none blocks the merges above except where a merge's parity bar
  names it.
