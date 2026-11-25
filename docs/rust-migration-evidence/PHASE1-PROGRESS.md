# Phase 1 — Progress (Rust realmd)

Goal driver: Muse session `granite-dubhe` (goal created 2026-10-16,
after Phase 0 completed and was pushed in `eda3541`/`dda8a07`).
Spec: [../RUST_MIGRATION_ARCHITECTURE.md](../RUST_MIGRATION_ARCHITECTURE.md)
§13.2 Phase 1 + §12. Branch: `rust-migration`. Verification is
local-only (owner decision); changes stay uncommitted unless asked.

## Owner decisions carried over

- Mixed model routing exactly as in `PHASE0-PROGRESS.md` (wonder-full
  driver/synthesis/critic + first-of-kind work; spark-1.3 Workflow
  children for mechanical transcription; never verify below producer).
- Pure-Rust `mysql` driver (seeded in Phase 0 `mangos-db`),
  owner-overridable before later phases build on it.
- Real 1.12.1 client logon is an owner checkpoint, never faked.

## Characterization (scope 1, 2026-10-16)

**HEAD vs production**: `src/realmd` + `src/shared/Crypto` differ from
`2bae5f9` by formatting only (include alphabetization, brace style;
verified with whitespace-insensitive diffs). One spec serves both; the
oracle binary is the `2bae5f9` container build.

**Trap 1 — inverted `SRP6::Proof`: the double inversion cancels.**
`Proof` returns `false` when `memcmp` matches
(`SRP6.cpp:156-164`), and its sole call site negates it into the
success branch: `if (!srp.Proof(lp->M1, 20) && pinResult)`
(`AuthSocket.cpp:639`). Observed wire behaviour is therefore standard
(accept iff M1 matches). The Rust port implements sane
`proof_matches` semantics and pins accept-on-match / reject-on-mismatch
against the oracle; it does not replicate the inverted internal API.
Reconnect proof (`AuthSocket.cpp:883`) uses direct `memcmp` and never
calls `Proof`.

**Trap 2 — salt feed**: C++ feeds the minimal-length little-endian
salt into the verifier hash (Phase 0 corpus); `BN_rand(top=0)`
guarantees 32-byte salts so the dashboard's padded variant agrees on
all producible salts. Port the C++ feed (already decided).

** SRP6 math to port (from `SRP6.cpp`, all feed orders matter)**:
- `CalculateHostPublicEphemeral` (:35-42): `b` = 19-byte random,
  `B = (3v + g^b) mod N`.
- `CalculateSessionKey` (:69-88): reject `A==0` or `A mod N == 0`;
  `u = SHA1(A_LE || B_LE)` as LE integer (BigNumber `AsByteArray`
  defaults: minimal, reversed); `S = (A · v^u)^b mod N`.
- `HashSessionKey` (:127-154): interleave-SHA1 over the 32-byte
  `AsByteArray(32)` of S → 40-byte K. Note `AsByteArray(32)` is
  big-endian-padded then REVERSED (default `reverse=true`), i.e. S is
  consumed little-endian zero-extended to 32 bytes.
- `CalculateProof` (:44-67): `M = SHA1(t3 || SHA1(username) || s || A
  || B || K)` with every BigNumber fed via `AsByteArray()` (minimal
  LE) and `t3 = SHA1(N) XOR SHA1(g)` re-interpreted via `SetBinary`
  (little-endian) before feeding — byte order of t3's feed is
  therefore reversed-digest minimal-LE, a detail test vectors must
  cover.
- `Finalize` (M2, :168-175): `SHA1(A || M || K)`, same feed rules.
- `CalculateVerifier` (:97-125) and the empty-A feed: Phase 0 corpus.

**Wire surface** (`AuthCodes.h:29-40`, dispatch `AuthSocket.cpp:117`):
8 client commands — LOGON_CHALLENGE 0x00, LOGON_PROOF 0x01,
RECONNECT_CHALLENGE 0x02, RECONNECT_PROOF 0x03, REALM_LIST 0x10,
XFER_ACCEPT 0x32, XFER_RESUME 0x33, XFER_CANCEL 0x34 (0x30/0x31 are
server→client patch initiate/data). State machine:
CHALLENGE→LOGON_PROOF/RECON_PROOF→AUTHED, PATCH branch from proof.
Packet layouts are packed structs in `AuthPackets.h` (challenge
header+body with variable username, proof 32+20+20+1 [+1 security
flags for build ≥5428, +36-byte PINData when flag 0x01], reconnect
proof 16+20+20+1, XFER structs). Proof responses are build-variant
(`AUTH_LOGON_PROOF_S` family).

**DB surface**: login DB only (`account` incl. `v`/`s`/`sessionkey`/
`totp_secret`/`geolock_pin`, `realmlist`, `realmcharacters`,
`account_banned`/`ip_banned`, failed-login tracking). Config keys
actually read by realmd are inventoried in the work log (24 keys incl.
`LoginDatabaseInfo`, `RealmServerPort`, `StrictVersionCheck`,
`WrongPass.*`, `GeoLocking`, mailer keys behind `ENABLE_MAILSENDER`
— compile-time in C++, runtime-dead unless built with it; the Rust
port treats mail as out of scope and records that).

## Checklist

- [x] Characterize contract; pin both SRP traps (above)
- [x] `mangos-protocol` (auth half) + packet golden tests
      — buffer reader/writer, auth packet parse/build (challenge,
      proof incl. pre-1.11 + PIN, reconnect), full SRP6 server math.
      13 tests: oracle/derived verifier goldens (incl. C++
      leading-zero-salt vector), client↔server handshake agreement,
      packet byte layouts (5875 proof response = 26 bytes), auth
      result/PROPASS constants verified against `AuthCodes.h`.
      Workspace gate green (28 Rust tests).
- [x] `mangos-db` login support (actor, binds, migration gate)
      — `LoginDb` per-DB actor thread over one sync `mysql` conn;
      query set transcribed 1:1 from the C++ call sites (see
      `login.rs` header for the map + deliberate geolock exclusions).
- [x] `mangos-config`/`mangos-log` minimal slices
      — config parser is a byte-exact port of the C++ state machine
      incl. space-only whitespace, decorative quotes (`quoteFound`
      resets per char so `#` always ends a value), first-key-wins,
      1023-byte `fgets` chunking; 9 quirk tests. Log keeps the C++
      `YYYY-MM-DD HH:MM:SS ` + `ERROR: ` file format, overwrite on
      open, `LogFile.TimeStamp` suffixing.
- [x] `mangos-auth` binary (`realmd`): SRP logon + reconnect, realm
      list (both wire variants), patch XFER, bans + wrong-password
      autoban, sessionkey persistence, PIN grid + TOTP/fixed-PIN
      verification, `--version`/banner via `mangos-build-info`,
      C++-compatible CLI (`-c/-v/-h/-s`), migration gate at boot.
- [x] Live-DB logon tests; `test_auth.py` vs Rust-written rows
      — `mangos-db/tests/live_login.rs` (ignored+`TEST_MYSQL_URL`):
      account create with Rust SRP v/s, read-back, session stamp,
      autoban rows, migration gate. Dashboard `_verify_srp6_password`
      verified a Rust-created row (DASHCHECK) from the DB;
      `pytest dashboard/tests/test_auth.py` 13 passed.
- [x] Oracle-differential packet/DB tests (C++ realmd @2bae5f9)
      — independent stdlib client `vmangos-rs/scripts/logon_client.py`
      (self-validates against golden verifiers before any I/O) ran
      identical scenarios on both servers: unknown account, wrong
      password, full logon (client-verified M2), realm list,
      reconnect, autoban. Raw realm-list + challenge-error bytes
      byte-identical (0 differing bytes). DB rows match, incl.
      `sessionkey` == client-derived K on both servers.
- [x] Shadow run into oracle mangosd — **PASS 2026-10-17**
      Rust `realmd` (Linux ARM64 container build) + oracle `mangosd`
      @2bae5f9 (container) against restored snapshot DBs
      (`~/azeroth-dev-fixtures/backups/*_20261015_040001.sql.gz`) and
      the copied DataDir. `vmangos-rs/scripts/world_client.py`:
      SRP logon via Rust realmd → `CMSG_AUTH_SESSION` → `AUTH_OK` →
      char enum → create `Rustshadow` (guid 94) → enum →
      `CMSG_PLAYER_LOGIN` → `SMSG_LOGIN_VERIFY_WORLD` (map 0,
      Northshire start). Stable across reruns; `characters` +
      `realmcharacters` side effects verified in DB.
- [ ] Owner checkpoint: real 1.12.1 client logon — **OWNER-PENDING**
      (prepared: run the Linux Rust realmd container + oracle
      mangosd against the bench snapshot DBs, point the client's
      `realmlist.wtf` at the bench IP:37241, update the bench
      `realmlist.address` to the bench IP first since the snapshot
      row holds the droplet address. Record owner's result here.)

## Log

- 2026-10-16: Goal created. Characterization above from direct code
  reads (driver session). HEAD≡prod for realmd (formatting only).
- 2026-10-16→17: All five Phase 1 crates landed; `ci.sh` green
  (39 Rust tests incl. `TEST_MYSQL_URL` live tests).
- Bugs caught by the ladder, fixed at root cause:
  (1) protocol challenge builder missed the C++ fixed `0x00` byte
  before the result (`AuthSocket.cpp:281`) — builder + its golden
  test both corrected (the test had encoded the same wrong shape);
  (2) `LoginDb::create_account` mixed named `params!` with `?`
  placeholders (driver error, first live test to touch it);
  (3) logon-proof read length 75 vs the struct's 74 bytes
  (scripted-client differential caught the hang).
- Characterized deviations from the C++ oracle (deliberate,
  owner-visible; wire contract unchanged):
  (a) Wrong-password autoban threshold: Rust bans at exactly
  `WrongPass.MaxCount`; the C++ async `PExecute` increment lags its
  sync count read, banning one attempt late under rapid failures
  (observed: MaxCount=3 banned on attempt 4).
  (b) Patch XFER after a mid-run patch-file change: the C++ hash
  recalculation reads through a `::dup()`-shared file offset and
  then streams zero bytes (`FileHandle.cpp` `DuplicateFileHandle`);
  with the hash cached both stream identically (md5-verified,
  byte-identical XFER_INIT). Rust always streams from offset 0.
  (c) Scope exclusions per charter (documented in `mangos-auth`
  crate docs): PROXY protocol v2, geolock/geoip enforcement + its
  PIN writes and mail (`ENABLE_MAILSENDER` is compile-time-off in
  production), `-s run` daemonization (foreground instead), CPU
  affinity/priority, and DB-error handling (C++ trusts its DB
  layer; Rust closes the session on driver errors instead of
  fabricating replies). `GetIntDefault` on malformed values: C++
  crashes via uncaught `std::stoi`; Rust returns the default.
- Bench state note: local MySQL now holds the restored snapshot DBs
  (plus bench-only SHADOWTEST account + `Rustshadow` character).
  Rebuild clean fixtures anytime with
  `vmangos-rs/scripts/setup-dev-mysql.sh`. Evidence scripts kept at
  `vmangos-rs/scripts/{logon_client,world_client}.py`; raw captures
  and server logs under `/tmp/realmd-test`, `/tmp/oracle-realmd`,
  `/tmp/rust-shadow`, `/tmp/oracle-run/logs` (this machine only).
