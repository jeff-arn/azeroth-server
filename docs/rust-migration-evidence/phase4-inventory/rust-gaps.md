# Phase 4 Inventory — Rust-Side State + Production Divergence Sample

Surface: what the Rust server (`vmangos-rs/`, branch `rust-migration`, HEAD
`1bec69f`) actually implements today, what is stubbed, the first opcodes a
populated-world slice needs, and whether the C++ oracle at HEAD has drifted
semantically from the production binary (`2bae5f9`) in three Phase 4 core
files. All cited bodies were opened with `read_file`; search was used only to
locate candidates. Divergence method: `git diff -w` hunk inspection plus an
independent token-stream comparison (comments and `#include` lines stripped).

Headlines:

1. The Rust world is an authenticated empty room: after a 5-packet login
   train, `player_in_world` is never set, so every `LoggedIn` opcode is
   silently dropped by design.
2. Exactly 4 opcodes have real handlers (2 at the socket, 2 in the world
   actor); the other 272 C++ handler names exist only as inert strings.
3. The leaf crates Phase 4 needs (movement splines, threat tables, GUIDs)
   are complete and byte-tested but have no world to plug into: there is no
   `Player`, `Unit`, `Map`, or object-update type anywhere in Rust.
4. Divergence sample verdict: `Map.cpp`, `Object.cpp`, and `Spell.cpp` are
   **formatting-only** different between `2bae5f9` and HEAD — body token
   streams are order-identical. Zero semantic deltas touch Phase 4 anchors
   in these files. (Scope caveats in section (f).)

## (a) Verified structure / flow

### Process and boot (`mangosd`)

- Binary entry mirrors C++ `Master`: args/banner, exit codes 0/1/2 with
  SIGINT→2 (restart), SIGTERM→0, no SIGSEGV anticrash inheritance
  (`crates/mangosd/src/main.rs:1-11`, 898 lines total).
- Boot order is C++ order for the implemented subset: RealmID required →
  World/Character/Login/Logs connect + migration gate →
  `clearOnlineAccounts` writes (`main.rs:451-506`) → DataDir + six
  start-area map/vmap existence checks → `mangos_string` load (empty aborts)
  → `realmlist` publish → corpse cleanup → listener (`main.rs:508-549`).
- The other ~90 `ObjectMgr`/DBC/script loaders are explicitly logged as
  deferred, never silently skipped (`main.rs:508-513`;
  `crates/mangos-world-data/src/lib.rs:4-10`).

### Connection path (`socket.rs`, 553 lines)

- Per-connection Tokio task owns the TCP stream and both `AuthCrypt`
  directions; sends `SMSG_AUTH_CHALLENGE` on accept
  (`crates/mangosd/src/socket.rs:76-91`).
- Routing mirrors `WorldSocket::_HandleCompleteReceivedPacket`: only
  `CMSG_PING` (476) and `CMSG_AUTH_SESSION` (493) are handled at the socket;
  everything else requires a session and is forwarded to the world actor,
  subject to queue admission (`socket.rs:250-296`). Pre-auth non-ping/auth
  packets close the connection (`socket.rs:268-277`).
- Framing invariants live in `mangos-protocol`: 6-byte client / 4-byte
  server headers, max client size `0x2800`, opcodes `< 828`
  (`crates/mangos-protocol/src/world.rs:19-33`). The decoded header is
  cached across reads because header decode advances crypt state and must
  never run twice (`socket.rs:169-220`).
- Ping path: overspeed tracking with kick, latency forwarded to the world
  as `SetLatency`, `SMSG_PONG` replied at the socket (`socket.rs:315-362`).
- One deferred item is recorded in-code: `BuildAddonPacket` over the auth
  addon tail was initially unported (`socket.rs:9-13`); PHASE3-PROGRESS
  records it as landed later (`PHASE3-PROGRESS.md:220-222`) — the header
  comment is stale relative to the progress log (see unresolved).

### World actor (`world_actor.rs`, 850 lines)

- Single owner of all session state on a 50 ms Tokio tick
  (`WORLD_TICK`, `world_actor.rs:46-47`); socket tasks reach it only via a
  bounded channel of capacity 1024 (`world_actor.rs:49-50`, message enum at
  `:173-206`); outbound flow is a per-session bounded control channel of
  1025, matching the C++ ">1024 at push closes" rule
  (`world_actor.rs:117-120`; try-send-or-disconnect at `:239-244`).
- Freeze-detector replacement: actor stamps each tick; binary watchdog logs
  past 10 s and exits non-zero past 30 s (`world_actor.rs:52-115`).
- Admission reproduces `World::AddSession_`: long-form `AUTH_OK` /
  `WAIT_QUEUE`+position, login queue with grace-period skip, population
  publish, short-form packets on promotion (`world_actor.rs:439-540`).
  Duplicate login replaces the old session without save-on-logout
  (recorded Phase 3 simplification, `world_actor.rs:11-13`, `:311-317`).
- The five C++ per-class receive queues collapse into one FIFO per session,
  recorded as unobservable "while every handler is a stub"
  (`world_actor.rs:8-11`). This becomes observable the moment Map/Movement
  handlers are real — a Phase 4 entry dependency.
- Dispatch: `update_sessions` runs the full `ProcessPackets` filter per
  queued packet, but only two opcodes branch to real work —
  `CMSG_CHAR_ENUM` (55) and `CMSG_PLAYER_LOGIN` (61); every other
  `Execute` lands in `execute_stub`, a debug log line and drop
  (`world_actor.rs:720-819`, stub at `:822-835`).

### Admission core (`session_core.rs`, 382 lines) — real, not stubbed

- `QueuePacket` class resolution incl. `CMSG_MESSAGECHAT` rerouting by chat
  type and the `PACKET_PROCESS_MAX_TYPE` silent IO-side drop
  (`session_core.rs:27-53`).
- `AllowPacket` flood accounting: 10 exempt query opcodes never count;
  very-slow (11) also count as slow (5 more); refuse when
  `very_slow > 2 || slow > 8 || total > 300` per update, and refusal stops
  that session's queue for the tick with the packet consumed
  (`session_core.rs:85-164`, `:188-191`).
- `ProcessPackets` status filter is complete for all six statuses
  (`session_core.rs:192-235`). The Phase 4-critical fact: `SessionCore`
  docs state `has_player` was kept permanently false in Phase 3
  (`session_core.rs:7-9`); today login sets `has_player=true,
  in_world=false` (`world_actor.rs:680`), so `LoggedIn` packets return
  `DropSilent` (`session_core.rs:200-204`) and `Transfer` packets would
  `Execute` (`:215-222`) — into the stub.

### Player login (`player_login.rs`, 308 lines) — stops before the map

- Gates reproduced: opcode-level (`WorldAvailable`, already-loading /
  already-logged-in, non-player GUID → `SMSG_CHARACTER_LOGIN_FAILED` byte 1)
  at `world_actor.rs:588-631`; load-level (exists / account match /
  transfer-locked → kick) at `player_login.rs:45-63`.
- On success the train is exactly five packet groups in C++ order:
  `SMSG_LOGIN_VERIFY_WORLD` (map + 3×f32 position + orientation from the DB
  row, `player_login.rs:79-89`), account-data MD5, `SMSG_FRIEND_LIST`
  (everyone offline, `:94-108`), `SMSG_IGNORE_LIST`, MOTD system lines
  (`world_actor.rs:682-699`). Then: "player entered the (empty) world;
  Phase 3 stops before map add" (`world_actor.rs:700-703`).
- Deliberate stops recorded at `player_login.rs:4-10` and
  `world_actor.rs:633-637`: everything from
  `SendInitialPacketsBeforeAddToMap` onward is missing — no initial
  spells/action-buttons/auras packets, no map add, no own-object create,
  no `characters.online=1` / `account.current_realm` presence writes, no
  transport relocation (`player_login.rs:73-77`), no bad-name rename-flag
  write (`player_login.rs:47-49`), tutorials packet deferred with the
  initial-packets train (PHASE3-PROGRESS:215-218).
- The full read-only character snapshot (21 issued queries, 22-slot holder)
  is stored on the session but marked `#[allow(dead_code)]` — "consumed as
  handlers grow past stubs" (`world_actor.rs:232-234`, stored at `:691`).
  There is no `Player` object built from it.

### Leaf crates Phase 4 will consume

- `mangos-guid` (549 lines, single file): complete `ObjectGuid` value
  semantics, `high:16 << 48 | entry:24 << 24 | counter:24` frozen layout,
  packed-GUID codec (`crates/mangos-guid/src/lib.rs:1-15`). One recorded
  deviation: the generator returns `None` on overflow instead of logging
  and stopping the server; shutdown policy belongs to the (missing) world
  layer (`lib.rs:11-15`).
- `mangos-movement` (1,673 lines: `spline.rs` 504, `move_spline.rs` 633,
  `packet_builder.rs` 510, `lib.rs` 26): Catmull-Rom spline math and the
  `SMSG_MONSTER_MOVE` / object-create packet builder, byte-exact against
  hand-derived golden tests, including the linear packed-offset `[-ZERO]`
  nudge and duration back-patch (`crates/mangos-movement/src/lib.rs:1-13`;
  `packet_builder.rs:1-10`). Explicitly absent: `MoveSplineInit`,
  `MotionMaster`, waypoint generators, transport handling — "belongs to
  the Phase 3 host and is intentionally absent" (`lib.rs:11-13`). In Phase
  4 terms: the *serialization* half of creature movement exists; the
  *initiation/lifecycle* half does not. `DEFAULT_MAX_POINTS_PER_PACKET =
  80` mirrors `Movement.MaxPointsPerPacket` (`packet_builder.rs:17-18`).
- `mangos-threat` (986 lines: `lib.rs` 192, `manager.rs` 565, `hated_by.rs`
  229): the full threat-table state machine is pure and ported —
  dirty-flag rules, online/offline containers, taunt temp-threat, the
  110%/130% victim-selection rule, threat-assist division
  (`crates/mangos-threat/src/lib.rs:4-15`). Every reach into `Unit` is a
  method on the `ThreatHost` trait (12 methods: spell-mod/crit/total
  modifiers, add/direct gates, GM/online/pet-owner checks, threat-area,
  valid-target, low-priority, melee-reach, faction;
  `lib.rs:110-167`), to be implemented by the world layer that does not
  yet exist. `calc_threat` control flow is in-crate (`lib.rs:169-192`).
- `mangos-world-data` (624 lines): `DataDir` resolution with C++
  normalization and `maps/`, `vmaps/`, `mmaps/`, `5875/` (DBC) subpaths
  (`lib.rs:20-72`); six start-area asset existence checks
  (`assets_check.rs`, 251 lines); `mangos_string` table (`strings.rs`,
  175 lines); enum-only stores — item displays, pet families,
  player-create pairs at patch-max (`enum_stores.rs`, 126 lines). Recorded
  bench counts (PHASE3-PROGRESS:262-267, not re-verified here): 976
  strings, 17,707 item defs, 10,381 creature templates, 40 create infos.
  No spell/creature/gameobject stores for simulation exist.
- `mangos-protocol` (8,138 lines, of which the generated
  `opcode_table.rs` is 6,639): wire core + the full registry. Crucially,
  `OpcodeMeta.handler_name` is a string with no dispatch target —
  "dispatch targets are resolved by `handler_name` until the session layer
  maps them to typed handlers" (`crates/mangos-protocol/src/opcodes.rs:8-19`).
  There is no handler table to fill in; Phase 4 must create the mapping
  mechanism itself.

### What does not exist at all (verified by search over all crates)

No `Player`, `Unit`, `Creature`, `GameObject`, `WorldObject`, `Map`,
grid/cell, visibility, `UpdateData`, or `SMSG_UPDATE_OBJECT` construction
exists in Rust. Search hits for those names are only: the two doc comments
naming `SendInitialPacketsBeforeAddToMap`, one `packet_builder.rs:204`
comment about create-values blobs, the opcode constant itself, and asset
file parsers (`mangos-assets` gridmap/vmap readers). The simulation object
model is a greenfield build inside an otherwise faithful shell.

## (b) Sizes / counts

| Item | Count | Anchor |
|---|---|---|
| `mangosd` crate | 13 files, 4,335 lines (`world_actor` 850, `main` 898, `socket` 553, `session_core` 382, `player_login` 308) | `wc -l crates/mangosd/src/*.rs` |
| Opcode slots / registered / gap | 828 / 826 / `0x33A` unregistered | `opcodes.rs:60-62` (test-asserted) |
| Distinct C++ handler names | 276 | `opcodes.rs:70-77` (test-asserted) |
| Real Rust handlers | 4: `CMSG_PING`, `CMSG_AUTH_SESSION` (socket); `CMSG_CHAR_ENUM`, `CMSG_PLAYER_LOGIN` (actor) | `socket.rs:256-267`; `world_actor.rs:752-758` |
| Status census | Never 525, LoggedIn 290, Authed 8, LoggedInOrRecentlyLoggedOut 2, Unhandled 2, Transfer 1 | parsed from `opcode_table.rs` (828 rows) |
| Processing census | MaxType 524, Map 154, World 94, Movement 39, Async 9, Spells 8 | parsed from `opcode_table.rs` |
| Movement-family opcodes | 22 rows → one handler `HandleMovementOpcodes` | `opcode_table.rs` (e.g. `:2111-2115`) |
| Cast opcodes | 3: `CMSG_CAST_SPELL`, `CMSG_CANCEL_CAST` (Spells class), `CMSG_PET_CAST_SPELL` (Map class) | `opcode_table.rs` parse |
| Flood limits / tick / queues | 2 very-slow, 8 slow, 300 total per update; 50 ms tick; 1024 world channel; 1025 send queue | `session_core.rs:163`; `world_actor.rs:47-50,120` |
| Character load | 21 issued queries / 22 slots; `character_load.rs` 736 lines | `PHASE3-PROGRESS.md:55,79-82` |
| Tests in touched crates | 46 `#[test]` in `mangosd`+`mangos-movement`+`mangos-threat` src (plus threat integration tests) | `grep -c` |
| Divergence sample (`-w`) | Map 257 changed lines/40 hunks; Object 260/45; Spell 570/105; total 377 + / 710 − | `git diff -w --stat 2bae5f9 HEAD` |
| Divergence sample (raw) | 1,784 + / 2,117 − across the 3 files | `git diff --stat 2bae5f9 HEAD` |
| Body tokens (comments/includes stripped) | Map 23,313; Object 22,167; Spell 44,412 — order-identical old vs new, all three | token-stream comparison, this run |
| Include sets | Map 37=37; Spell 29=29; Object 35→34 (duplicate `Geometry.h` deduplicated) | this run |
| Commits `2bae5f9..HEAD` | 14; only 1 touches the 3 sampled files (`289bcf53`, the clang-format squash) | `git log --oneline` |
| Game tree drift (context) | 434 files, 10,576 + / 15,914 − under `vmangos/src/game/` (`-w`) | `git diff -w --stat` tail |

## (c) DB / config inputs

- Four MySQL databases (MySQL-only Rust scope, owner decision
  PHASE3-PROGRESS:30-32): World, Character, Login, Logs, each
  connect-then-migration-gate at boot; missing migration refuses boot,
  extras warn (`main.rs:453-506`; gate census 1005/18/6/6 recorded at
  PHASE3-PROGRESS:163-166).
- Character reads: the 23-column enum SELECT and the 21-query
  `LoginQueryHolder` transcription (row, group, bound instances, auras,
  spells, quest status, honor, reputation, inventory, actions, social,
  homebind, cooldowns, guild, account data, skills, mail, …) — the full
  field set is visible in the test fixture at `player_login.rs:270-292`.
  Position/orientation/map for verify-world come from the `characters`
  row (`player_login.rs:79-89`).
- Writes today are exactly the 7-statement unavoidable-write register
  (PHASE3-PROGRESS:90-102, 237-241): boot presence resets, corpse DELETE,
  realmlist publish/population, `last_ip`, uptime insert/update.
  `characters.online` stays 0 through login — Phase 4 must add presence
  writes and, eventually, position persistence (character save proper is
  Phase 6, PHASE3-PROGRESS:36-39).
- Config: `mangosd.conf` consumed unmodified for the implemented subset.
  `ServerConfig` (`crates/mangosd/src/lib.rs:55+`) carries RealmID, port
  8085, BindIP, `MaxOverspeedPings` (default 2, 1 clamps to 2),
  `Network.KickOnBadPacket`, `TimeoutSecsIfNoAuth` (10),
  `CharacterScreenMaxIdleTime`, `PlayerLimit` (100; non-positive encodes a
  security floor), `Antiflood.Sanction` (kick bit `0x08` honored, rest
  logged), `LoginQueue.GracePeriodSecs`, `RestrictUnverifiedAccounts`,
  `WorldAvailable`, MOTD + `WowPatch` (patch name table at
  `player_login.rs:147-162`). Phase 4 will need the large unimplemented
  remainder (visibility distances, rates, movement/anticheat knobs) —
  none are read today.
- Filesystem inputs: `DataDir` → `maps/`, `vmaps/`, `mmaps/`, `5875/`
  DBCs (`mangos-world-data/src/lib.rs:48-72`). Only existence + magic of
  six start-area grids is checked; terrain height queries for movement
  landing/validation are not wired into `mangosd` (asset parsers exist in
  `mangos-assets` but have no session-path consumer).

## (d) Wire-visible semantics a Rust port must replicate

Already replicated (must not regress while adding simulation):

- AuthCrypt rolling XOR both directions, header shapes, size limits,
  malformed-header close + log text (`world.rs:19-33`; `socket.rs:192-203`).
- The full admission chain in C++ order: IO-side `MaxType` silent drop →
  per-session flood refusal (consumes the packet, stops the tick's queue,
  exact `AntiFlood:` log line, kick only if sanction bit `0x08`) → status
  filter with distinct silent / debug-unexpected / error-not-allowed /
  debug-unhandled outcomes and exact reason strings
  (`session_core.rs:55-75,188-235`; `world_actor.rs:751-807`).
- `CMSG_MESSAGECHAT` queue-class rerouting by chat type (say/yell→Map,
  whisper/party/raid/BG→Async, else World) at IO time, short-body
  fallback to the registered class (`session_core.rs:31-53`).
- Login-queue packet shapes: long `AUTH_OK`/`WAIT_QUEUE` at admission,
  short `AUTH_OK` + short position updates on promotion, idle-clock reset
  on promotion, trial billing flags `0x04|0x40`
  (`world_actor.rs:160-163,418-540`).
- Login train byte shapes verified by unit tests: verify-world is
  `u32 map + 4×f32` (`player_login.rs:243-253`); system chat is type
  `0x0A`, `LANG_UNIVERSAL` 0, zero sender, length+1, cstring, tag 0
  (`player_login.rs:127-143,255-265`); login-failed is the single byte 1
  (`player_login.rs:18-21,65-71`).
- Send-queue overflow disconnects rather than blocking, at 1025 queued
  (`world_actor.rs:117-120,239-244`); socket→world backpressure closes
  the connection on a full 1024 channel (`socket.rs:283-292`).
- Movement packet serialization quirks already locked by golden tests:
  packed-offset `[-ZERO]` nudge, duration back-patch, fake `Enter_Cycle`
  and `Runmode` flag bits, facing-byte fallthrough to Normal on zero or
  multiple mask bits (`packet_builder.rs:34-50`).
- Bad-packet policy is config-driven in both layers: log
  `ByteBufferException`, disconnect only when `KickOnBadPacket`
  (`socket.rs:298-313`; `world_actor.rs:706-716`).

Not yet on the wire at all (Phase 4 must create, byte-faithful to C++):
`SMSG_UPDATE_OBJECT` create/update/destroy blocks, `SMSG_MONSTER_MOVE`
emission (builder exists, no emitter), movement broadcasts of other
players' `MSG_MOVE_*`, spell cast/go/failed packets, stand-state and
selection echoes, zone/weather updates, and the entire
`SendInitialPacketsBeforeAddToMap` train (initial spells, action buttons,
auras, tutorials, bind point, etc.).

## (e) Smallest viable first slice + entry dependencies

**Slice: "stand in a populated world and be seen moving."** One player
completes login into a real (single) map, sees static spawned creatures/
gameobjects and one other player, can stand/sit and run around, and is
seen doing so. No combat resolution yet.

Entry dependencies, in order (each blocks the next):

1. **Object model minimum** — an Object/Unit/Player representation with
   GUID, position/orientation, and the `UpdateFields` value block needed
   to serialize an own-player create. Nothing exists today; the stored
   `CharacterLoad` snapshot (`world_actor.rs:691`) is the construction
   input. `mangos-guid` is ready.
2. **Login completion** — `SendInitialPacketsBeforeAddToMap` train, map
   add, own `SMSG_UPDATE_OBJECT` create, presence writes
   (`characters.online=1`), then `set_player_state(true, true)`. Until
   the last step flips, every opcode below is `DropSilent` by
   construction (`session_core.rs:200-204`).
3. **Map + visibility minimum** — one non-instanced map, a spatial index
   (grid), spawn data load (creature/gameobject stores — new
   `mangos-world-data` loaders), and create/destroy broadcasting on
   visibility-range entry/exit. Terrain height from `mangos-assets`
   gridmaps for landing/ground snapping.
4. **Per-class queues** — undo the recorded FIFO collapse
   (`world_actor.rs:8-11`): once Movement/Map/Spells handlers mutate
   shared object state, the C++ per-class pass order becomes observable.
   Also create the `handler_name` → typed-handler mapping mechanism
   (`opcodes.rs:8-11`); today dispatch is a 2-opcode `if` chain plus stub.
5. **Typed handlers for the first opcode set** (below). `HandleMovement`
   needs a Rust `MovementInfo` parser/validator (C++:
   `vmangos/src/game/Handlers/MovementHandler.cpp`) plus relocation +
   broadcast; the spline *packet* side (`mangos-movement`) is already
   done for server-driven creature movement.

First opcodes needing real handlers (stand + move, populated world).
Status/processing/handler are from the generated registry; ids are 1.12.1:

| # | Opcode (id) | Processing / C++ handler | Why it is in the first set |
|---|---|---|---|
| 1 | `MSG_MOVE_*` family (181–219, 238 `HEARTBEAT`, …) | Movement / `HandleMovementOpcodes` (22 opcodes, 1 handler) | All client locomotion; parse `MovementInfo`, relocate, broadcast to observers |
| 2 | `CMSG_STANDSTATECHANGE` (257) | Map / `HandleStandStateChangeOpcode` | Stand/sit/loot posture; the "stand" half of the slice |
| 3 | `CMSG_SET_ACTIVE_MOVER` (618) | Movement / `HandleSetActiveMoverOpcode` | Client asserts which GUID it drives (self vs possessed/pet) |
| 4 | `MSG_MOVE_WORLDPORT_ACK` (220) | World, status **Transfer** (only Transfer row) / `HandleMoveWorldportAckOpcode` | Completes teleports/map transfers; executes today into the stub |
| 5 | `CMSG_NEXT_CINEMATIC_CAMERA` (251) + `CMSG_COMPLETE_CINEMATIC` (252) | Map | Fresh characters are stuck in the intro cinematic without these |
| 6 | `CMSG_ZONEUPDATE` (500) | Map / `HandleZoneUpdateOpcode` | Client zone-change reports driving zone state/weather/exploration |
| 7 | `CMSG_NAME_QUERY` (80), `CMSG_CREATURE_QUERY`, `CMSG_GAMEOBJECT_QUERY`, `CMSG_ITEM_QUERY_SINGLE` | flood-exempt set (`session_core.rs:87-98`) | Client fires these for every newly visible object; unanswered = "Unknown" entities |
| 8 | `CMSG_SET_SELECTION` (317) | Spells / `HandleSetSelectionOpcode` | Target selection; prerequisite state for casts and attacks |
| 9 | `CMSG_CAST_SPELL` (302) + `CMSG_CANCEL_CAST` (303) | Spells / `HandleCastSpellOpcode`, `HandleCancelCastOpcode` | First visible action; needs spell store + cast pipeline (may be slice 1.5) |
| 10 | `CMSG_ATTACKSWING` (321) + `CMSG_ATTACKSTOP` (322) | Spells | Auto-attack start/stop against the selected target |
| 11 | `CMSG_USE_ITEM` (171) | Map / `HandleUseItemOpcode` | Hearthstone/food; second entry into the spell pipeline |
| 12 | `CMSG_AREATRIGGER` (180) | Map / `HandleAreaTriggerOpcode` | Inn/rest and portal triggers crossed while running around |
| 13 | `CMSG_MESSAGECHAT` say/yell path | Map class via rerouting (`session_core.rs:47-51`) | Cheapest two-player "populated world" proof |
| 14 | `CMSG_LOGOUT_REQUEST` (75) + `CMSG_LOGOUT_CANCEL` (78) | World / Map | Clean exit + `recently_logged_out` semantics; very-slow flood class |

Deliberately excluded from slice 1: pet casts (no pets), taxi/transport
movement, battleground opcodes, mail/auction/guild (all `World`/`Async`
classes that work against DB state, not map state).

## (f) Production divergence sample + unresolved

### Verdict for the three sampled files: formatting-only

`git diff -w 2bae5f9 HEAD` still shows 377 + / 710 − because clang-format
(ColumnLimit 999) joins wrapped lines, which `-w` does not erase.
Inspected hunks confirm the pattern in each file: include-block sorting
(`Map.cpp` header hunk), constructor/init-list collapsing onto one line
(`Map::Map`), `void\nMap::EnsureGridCreated` signature joining, macro-call
argument reflowing (`Object.cpp` `MovementInfo::CorrectData`
`REMOVE_VIOLATING_FLAGS`), one-liner bodies (`Object::SetObjectScale`),
ternary and condition joining (`Spell.cpp` `SpellCastTargets::Update`,
`Spell::FillTargetMap`), and `):` → `) :` init-list spacing (`Spell`
constructors).

Independent token-stream proof (this run): with comments and `#include`
lines stripped, the token sequences of `2bae5f9` and HEAD are
**order-identical** for all three files (Map 23,313 tokens; Object 22,167;
Spell 44,412; multisets equal). Include sets are unchanged except
`Object.cpp` drops one duplicate `#include "Geometry.h"` (35→34 lines,
no set change). The only commit in `2bae5f9..HEAD` (14 commits) touching
these files is `289bcf53`, whose bundled sub-commits are the clang-format
18 pass + 18.1.3 alignment, self-described as "No semantic code changes"
— corroborated, not merely trusted.

**Consequence for Phase 4:** for Map, Object, and Spell *logic*, the HEAD
C++ checkout is a valid oracle for the production binary `2bae5f9`. No
semantic delta touches Phase 4 anchors (map grid lifecycle,
`MovementInfo` correction, spell targeting/effect dispatch) in these
files. Line numbers, however, have shifted massively (Spell raw diff:
2,594 changed lines), so any production-side log/backtrace line reference
must be re-anchored to HEAD before use as port evidence.

### Unresolved

- **Sample vs tree:** only 3 of 434 changed files under `vmangos/src/game/`
  (10,576 + / 15,914 −, `-w`) were token-verified. Extrapolating
  "formatting-only" to the other 431 files is uninspected; the same cheap
  token check should be run tree-wide before any file is used as a
  production oracle.
- **Semantic commits outside the sample (log-level evidence only):**
  `2bae5f9..HEAD` also contains `25ae6c39` (breadcrumb-trail formation),
  `6f082201` (combat AI tuning), `603620f9` (addon telemetry via
  ChatHandler), and `437f3e06` (docs) per `git log --oneline`. Bodies were
  not inspected in this surface; AI/movement-adjacent ports must treat
  those areas as *known-divergent* from production until sampled.
- **Token-proof limits:** the comparison strips comments and includes and
  tokenizes naively; it cannot see preprocessor condition changes inside
  stripped lines, string-literal whitespace is preserved as single tokens
  (good), but macro definitions spanning lines with `\`-continuations
  would tokenize identically even if line structure changed semantics.
  No such case was spotted in inspected hunks; not exhaustively excluded.
- **Stale header comment:** `socket.rs:9-13` still calls `BuildAddonPacket`
  unported while PHASE3-PROGRESS:220-222 records it landed. Which is
  authoritative was not verified against the addon code path in this run.
- **Real-client logon remains owner-pending** (PHASE3-PROGRESS:33-35,
  291-293): all Rust login evidence is from the scripted client; a real
  1.12.1 client may exercise wire paths (addon tail, queue behaviors) the
  scripted client does not.
- **Bench counts not re-verified:** world-data census figures (976
  strings, 17,707 items, 10,381 creature templates) are quoted from
  PHASE3-PROGRESS:262-267; no DB was queried in this run.
- **`ThreatHost` implementor census:** the trait's world-layer role is
  documented at `mangos-threat/src/lib.rs:4-7,110-112`; whether any
  non-test implementor exists outside the crate was not separately
  grepped — absence is inferred from the missing Unit/Map types.
- **C++ handler bodies for the first-opcode table** (MovementHandler,
  SpellHandler, MiscHandler under `vmangos/src/game/Handlers/`) were
  located but not read in this surface; per-opcode C++ semantics belong to
  the movement/spell inventory surfaces. The table's status/processing/
  handler claims rest on the generated registry, which is itself
  test-asserted against the C++ census (`opcodes.rs:59-78`).
