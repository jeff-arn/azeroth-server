# Phase 3 — Progress (mangosd skeleton, empty world)

Goal driver: Muse session `granite-dubhe` (goal created 2026-10-18,
after Phase 2 completed and the Phase 1+2 baseline was committed and
pushed).
Spec: [../RUST_MIGRATION_ARCHITECTURE.md](../RUST_MIGRATION_ARCHITECTURE.md)
§13.2 Phase 3, with §3, §5, §11, and §12 as the governing design
records. Branch: `rust-migration`. Baseline commit:
`3084cb17391460378317a6dd4da15b289efca53a`.
Verification is local-only against cloned/snapshot databases and
production fixtures on the dev bench. Nothing deploys to production;
changes stay uncommitted unless the owner asks.

## Owner decisions

- **Model routing (mixed, carried over from Phase 0):** the
  goal-driver session, synthesis, completeness critic/verifier, and
  first-of-kind or semantics-heavy work run on
  `muse-wonder-full-1.0` (omit `model` so children inherit).
  High-volume mechanically checkable work after a driver-built
  exemplar (opcode tables, loader row-mappers, inventories,
  test-running and log triage) routes to `muse-spark-1.3` through
  Workflow per-call `model`. Native `subagent_spawn` always inherits.
  Never verify with a weaker model than the producer.
- **Content scripts (2026-10-18): FFI-retain the C++ scripts** behind
  the script-registry boundary initially. Phase 3 must preserve that
  boundary in the skeleton architecture; it does not transpile or
  hand-port the 107k-line script corpus as part of the empty-world
  milestone.
- **Database backend (2026-10-18): drop PostgreSQL from Rust scope.**
  MySQL is the only supported Rust backend for Phase 3; do not carry
  a second unproven database abstraction through the world loaders.
- Real 1.12.1 client logon from Phase 1 remains **OWNER-PENDING**.
  Phase 3 will prove the local scripted path first and treat the real
  client as an owner checkpoint before any production-facing claim.
- Character saving remains out of scope: Phase 3 character work is
  read-only load/list/login-to-empty-world. Any unavoidable session
  or bookkeeping write discovered in the C++ path must be named here
  before it is reproduced; the character save cutover stays Phase 6.

## Checklist

- [x] Inventory the C++ boot/session/opcode surface and current Rust
      seams; choose the smallest bootable Phase 3 slice without
      weakening the §13.2 exit criteria.
- [x] Scaffold the Rust `mangosd` binary and any §12 crates required
      by the first slice; keep world simulation out of scope.
- [x] Config parity for the boot path plus the world/character/login
      migration gate.
- [x] Static world-data loading needed to boot against cloned
      production DB/assets and stay empty. (Boot gates + strings;
      item templates/DBC subsets land with the character slice.)
- [x] Session/queue topology and the 826-entry opcode table
      (server/client split reconciled; handlers stubbed by class).
- [x] Transcribe the 21 issued character-load queries read-only (22-slot holder; goal text's "23" reconciled below); prove the
      character-list data path against cloned snapshots.
- [ ] Scripted local login reaches character list and enters the
      empty world; scanner/fuzz corpus handling is robust.
- [ ] Supervisor/heartbeat and periodic-save replacement decisions
      recorded explicitly (no SIGSEGV anticrash inheritance).
- [ ] Full `vmangos-rs/ci.sh` at end state, §14 evidence updates,
      and a requirement-by-requirement completion audit.


## Inventory synthesis (Milestone 1 closed)

Driver-verified anchors (this session, against the C++ sources):

- Session path: `WorldSocket` accept/challenge/auth/ping framing,
  `AuthCrypt` rolling XOR (6 recv / 4 send bytes,
  modulo-before-increment, u8 index), client header (BE size
  including the 4-byte cmd, LE cmd; valid `4 <= size <= 0x2800`,
  `cmd < 828`), server header (BE size = payload+2, LE cmd),
  `WorldPacket` semantics.
- Opcode surface: 826 `StoreOpcode` rows, `NUM_MSG_TYPES = 828`
  (deliberate gap `0x33A`), 276 distinct handler names,
  525 `STATUS_NEVER` rows, `PacketProcessing` 0..5 with aliases
  (`SELF_ITEMS=MAP`, `DB_QUERY=ASYNC`, `CHANNEL/GROUP/GUILD=WORLD`).
- Character load: `LoginQueryHolder` has 22 slots plus the MAX
  sentinel; only 21 queries are issued (the battleground slot is
  never populated). The goal text says "23 queries"; reconciled as
  21 issued / 22 slots — no 23rd query will be invented. Character
  enumeration is a separate 23-column SELECT
  (`CharacterHandler.cpp:164-178`).
- Auth digest: `SHA1(account || 0u32 || clientSeed || serverSeed ||
  K-minimal-LE)`; independent hashlib vector (account `TESTUSER`,
  client `0x11223344`, server `0x55667788`, K bytes `[0x02,0x01]`)
  = `6f29f2ebc2b4d23be1dc82a6c97e6fa077864a50`.

## Unavoidable-write register

Writes the C++ server performs at boot and auth/login that a
behaviour-compatible skeleton must either reproduce deliberately
(dev-bench only) or explicitly defer. Boot: `account.current_realm=0`,
`characters.online=0`, battleground instance reset, realmlist
icon/timezone + offline-flag clear/set, corpse DELETE, `uptime`
INSERT + periodic UPDATE. (Correction: mangosd performs no
expired-`ip_banned` DELETE — that lives in GM commands and the
Phase 1 realmd; AccountMgr only loads the list.) Auth/login:
`account.last_ip`, `characters.online=1`,
`account.current_realm/online=1`. No other writes are permitted in
Phase 3; character save remains Phase 6.

## Log

- 2026-10-18: Goal created from the committed/pushed Phase 1+2
  baseline. Owner selected FFI-retain for content scripts and
  MySQL-only Rust database scope before architectural commitments.
  Initial source inventory is running under the mixed-model routing.

- Inventory milestone closed: session topology, opcode surface,
  config/loaders, and character-load surface mapped; synthesis and
  unavoidable-write register recorded above.
- `mangos-protocol::world` exemplar (AuthCrypt, framing,
  WorldPacket, auth-session/ping) landed with hand-derived test
  vectors. Deviations from UB/unsafe C++ behaviour, noted in code:
  empty-key AuthCrypt no-ops; oversized server payload errors
  instead of truncating; malformed headers return decoded values
  for logging.
- Opcode registry generated by `vmangos-rs/scripts/opcode_table_gen.py`
  into `vmangos-rs/crates/mangos-protocol/src/opcode_table.rs`;
  census asserted in-generator and in-crate:
  `rows=826 handlers=276 never=525 slots=828`;
  `OPCODE-REGEN-STABLE`. Protocol crate: 27 tests green,
  clippy `-D warnings` clean.

- `mangosd` crate landed: Tokio listener + per-connection tasks
  (challenge, socket-layer auth/ping with the C++ decision order,
  overspeed-ping kick, malformed-header close, no-auth deadline),
  a single world actor at the 50 ms tick with per-session FIFO
  queues, the full `ProcessPackets` status filter + `AllowPacket`
  flood accounting as a pure core, and class stubs behind
  `Dispatch::Execute`. Evidence: 21 unit tests + 4 loopback
  integration tests green (challenge->auth->ping/pong with a
  hand-rolled client crypt, stubbed `CMSG_CHAR_ENUM`, wrong-digest
  rejection without a `last_ip` write, pre-auth close rules,
  malformed-header close); binary boots on loopback and exits
  2 on SIGINT / 0 on SIGTERM (`World.h:77-79`, `Master.cpp:522-529`).
- Supervisor/heartbeat decisions (explicit, per exit criteria):
  exit codes 0/1/2 preserved; no SIGSEGV anticrash handler is
  inherited - a Rust panic in a task is a process-fatal bug to be
  fixed, not masked. The C++ freeze detector is replaced by a
  watchdog over the world-actor tick counter (stuck >10 s logs an
  error, >30 s exits non-zero); the counter/watchdog lands with
  the boot/health verification slice. Periodic-save replacement:
  not applicable in Phase 3 by design - there is no character
  save path at all until Phase 6, so no periodic saver exists to
  replace; the only writes are the unavoidable-write register
  above.
- Known scaffold gaps, deliberate: login-DB backend is a loud
  `NotWiredBackend` (next slice, in `mangos-db`);
  `BuildAddonPacket` over the auth addon tail is not ported
  (scripted client sends none; a real client will need it before
  the owner checkpoint); `GetServerAddresses` hostname expansion
  is loopback-only for now; the five per-class receive queues
  collapse into one per-session FIFO while all handlers are stubs.

- DB slice landed: `mangos-db` gained `ConnectionInfo` (promoted
  from realmd; realmd now shares it), a generic per-DB `DbActor`
  (LoginDb shape + ping/reconnect), `WorldAuthDb` with the
  transcribed 14-field world-auth query (prepared binds;
  `DATEDIFF` window kept), `last_ip` write, and `ip_banned`
  check; and the real migration gate - `build.rs` globs
  `vmangos/sql/migrations/*.sql` like
  `migrations_list.h.cmake` (census test: 1005/18/6/6),
  `check_required_migrations` mirrors `Database.cpp:570-625`
  (missing refuses boot, extras warn). `mangosd` main now runs
  the `Master::_StartDB` sequence (RealmID!=0, World/Character/
  Login/Logs connect+gate with masked log lines,
  `clearOnlineAccounts`) before binding the listener.
  Evidence: binary boots against the dev bench (all four gates
  pass, SIGTERM exit 0); live ignored suites green
  (`live_world_auth` 3, `live_login` 2, `live_mysql` 1).
  Note: no 128-char truncation or `required_` skip exists in
  this codebase's gate - that recollection was checked against
  `Database.cpp` and dropped.

- Static-load slice landed: new `mangos-world-data` crate with
  `DataDir` resolution, the six start-area `ExistMapAndVMap`
  checks (grid math validated against fixture filenames, both
  magics verified), and `mangos_string` loading with the C++
  ignore rules. Binary boot now runs, in C++ order: DB gates ->
  `clearOnlineAccounts` -> DataDir + asset gate -> strings
  (976 on the bench) -> `realmlist` icon/timezone publish ->
  corpse cleanup -> listener, with the deferred-loaders state
  logged explicitly. Fixture/live suites added to `ci.sh`
  (`MANGOS_DATADIR`, `TEST_MYSQL_URL_WORLD`) and green.
  Recorded deviation: string locale columns stay in raw
  `LocaleConstant` slots instead of the C++ dynamic DBC-locale
  indices (identical while only enUS DBCs are installed).

- Char-enum path landed end to end: `mangos-db::characters`
  (23-column enum query transcribed, positional `Fields`
  reader mirroring C++ `Field` coercion), enum stores in
  `mangos-world-data` (item displays + pet families +
  playercreateinfo pairs, patch-max selection at `WowPatch`),
  `BuildEnumData` port proven BYTE-EXACT against an independent
  Python oracle on the bench snapshot (168 bytes, account 5).
  World admission now matches `AddSession_`: long `AUTH_OK` /
  login queue with short forms on promotion, population
  publish, queue grace; global account data + tutorials load
  at auth. Loopback covers the full flow.

- Character slice landed: char enum end-to-end (previous entry)
  plus the full 21-query read-only `load_character` proven
  FIELD-EXACT against an independent Python oracle over the
  bench snapshot (298 rows for guid 1, all sections). Player
  login wired: opcode gates (`WorldAvailable`, loading/duplicate,
  player guid), LoadFromDB gates (exists/account/locked ->
  kick), then the empty-world train in C++ order:
  LOGIN_VERIFY_WORLD, ACCOUNT_DATA_MD5, FRIEND_LIST,
  IGNORE_LIST, MOTD sys messages. Scripted bench run
  (`vmangos-rs/scripts/phase3_scripted_login.py`) passes:
  byte-exact enum, verify-world at Ruikanis' saved position,
  foreign-guid kick. Deliberate Phase 3 stops within this
  slice: no presence writes (online=1), no name-check rename
  write, no transport relocation, tutorials packet stays with
  the deferred initial-packets train.

- Player login landed (previous entry) plus `BuildAddonPacket`
  (zlib addon tail, Blizzard hide/key rules, C++ precedence
  quirk preserved as behaviour). Watchdog implemented: world
  actor stamps every tick; the binary's watchdog thread logs
  past 10 s and exits non-zero past 30 s. Robustness corpus
  green: malformed headers/pings close or are ignored per the
  bad-packet policy, 60 seeded random streams and a sampled
  all-opcodes sweep leave the server able to complete a valid
  login. Full scripted bench run passes with the addon tail.

- Realm lifecycle writes implemented and bench-verified:
  online publish (offline flag clear, population 0,
  `realmbuilds` "5875 6005 6141 "), `uptime` INSERT + periodic
  UPDATE (`UpdateUptimeInterval`, default 10 min; 0 disables -
  recorded deviation from the C++ every-tick-at-0 timer),
  offline publish at shutdown. Bench `realmlist` after
  shutdown: realmflags=2, builds written; latest `uptime` row
  carries the Rust binary's revision. Write audit: the full
  Rust write set is exactly 7 statements (boot resets x3,
  corpse DELETE, realmlist icon/tz + online + population,
  last_ip, uptime insert/update) - all on the register;
  `characters.online` stayed 0 through a full scripted login.

## Completion Audit (2026-10-18)

Phase 3 scope vs result:

- Tokio IO + world actor + session/queue topology (section 3):
  single world actor on a 50 ms tokio tick; bounded
  socket->writer channels (overflow disconnects, never blocks
  the socket); admission/login-queue gating with CMSG byte
  semantics; per-session flood defense (6000 pkt/30 s) with
  C++-identical rejection logs; packet dispatch via generated
  828-slot table (826 opcodes, 276 implemented, 272 mandated
  core, 525 STATUS_NEVER) with class stubs for auth/session/
  threadunsafe/world/map membership and per-processing-class
  immediate effects transcribed.
- Config parity: mangosd.conf consumed unmodified;
  ServerConfig mirrors SetInitialConfigs for the Phase 3
  subset including the C++-faithful security clamp and
  UpdateUptimeInterval; manual-spawn-only startup, exit codes
  0/1/2 preserved (SIGINT->2 verified, SIGTERM->0).
- Static loading (mangos-world-data incl. migration gate):
  start_area set verified at boot over the unmodified
  production DataDir; mangos_strings + enum stores loaded at
  WowPatch-10 (976 strings, 17,707 item defs, 10,381 creature
  templates, 40 create infos); migration gate fails boot on
  any missing applied migration across all 4 DBs.
- Character load (all 21 issued queries; "23" reconciled in
  milestone 2): field-exact against an independent SQL oracle
  (298-line digest, guid 1); char enum byte-exact vs a second
  independent oracle (168 bytes, account 5).
- Login to an empty world: full scripted train - auth
  challenge/session, char enum, player login initial packets,
  addon info (byte-exact modulo adler), foreign-guid kick,
  logout with countdown cancel; realm published online with
  realmbuilds + uptime row, offline at shutdown.
- Freeze/heartbeat decisions explicit: watchdog thread
  (>10 s log, >30 s non-zero exit) replaces the SIGSEGV-swallowing
  anticrash; supervisor restart is the recovery path;
  periodic saves are N/A in Phase 3 (no save path exists until
  Phase 6) - recorded decision, not an omission.
- Codec robustness: loopback corpus (garbage pre-auth,
  unknown opcodes, oversize size field, 826-opcode sweep)
  plus protocol codec unit tests; fuzz corpus seeded.

Gates (this run): `vmangos-rs/ci.sh` exit 0, 202 tests, 0
failed, 0 skipped with all 5 live/oracle env fuses lit;
fmt + clippy `-D warnings` clean; opcode-table and both
oracles regen-stable. File scope: vmangos-rs/ + docs/ only.

Deferred/owner items: real-client logon (owner), loaded perf
baseline (section 14 #49, Phase 4+), C++ FreezeDetector unit
decision on the droplet (no change made).
