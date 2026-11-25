# Phase 0 — Progress

Goal driver: Muse session `granite-dubhe` (goal created 2026-10-16).
Spec: [../RUST_MIGRATION_ARCHITECTURE.md](../RUST_MIGRATION_ARCHITECTURE.md)
§13.2 Phase 0 + §15 checklist. Branch: `rust-migration`. All changes stay
uncommitted for owner review unless the owner asks for a commit.

## Objective (corrected wording)

The session goal objective contains one garbled fragment in scope item
(1) ("in universal-apple-darwin..."). Read item (1) as:

> (1) scaffold the from-scratch Cargo workspace (workspace manifest,
> pinned toolchain/edition recorded in the progress doc, rustfmt/clippy
> gates, CI build+test config) under `vmangos-rs/`; writes are limited
> to `vmangos-rs/` and `docs/` except dev-bench fixture copies kept
> outside git.

All other objective text stands as written in the goal.

## Owner decisions (2026-10-16)

- **Model routing (mixed, applies to this goal and all later phases)**:
  the goal-driver session, synthesis, the completeness critic/verifier,
  and first-of-kind or semantics-heavy work (ownership design, spell/save
  behaviour, the persistence contract) run on `muse-wonder-full-1.0`
  (omit `model` so children inherit the parent route). High-volume,
  mechanically checkable execution — search/inventory sweeps,
  transcribing tables after a wonder-full exemplar exists (opcodes, DBC
  formats, loader row-mappers), test-running and log triage — is routed
  down to `muse-spark-1.3` via the Workflow per-call `model` field.
  Native `subagent_spawn` has no model parameter and always inherits;
  use Workflow children when a down-route is wanted. Background/reminder
  agents already run on spark via the catalog and need no instruction.
  Never down-route the verifier of a piece of work below the model that
  produced it.
- Driver: this session (not a separate agent).
- Workspace: `vmangos-rs/` in this repo, from scratch; clear the stale
  `vmangos-rs/target/` residue first.
- Droplet: hygiene actions may be executed, but each production-affecting
  action needs the owner's explicit confirmation immediately before it
  runs.

## Environment inventory (2026-10-16, observed)

- Host toolchain: Homebrew `rustc 1.98.1` / `cargo 1.98.1`; **no rustup**
  — a `rust-toolchain.toml` pin will not take effect until rustup is
  installed. Toolchain-management choice (install rustup vs Homebrew
  package vs droplet/CI-only pin) is an open Phase 0 decision; do not
  silently install rustup.
- `vmangos-rs/` contains only `target/` (1.7 GiB, `debug/` build output
  from the former workspace). No `Cargo.toml`, no `crates/`. Clearing it
  is checklist step 1; it is regenerable build output, not source.
- No CMake on this Mac (docs §10) — C++ parity oracles need the Linux
  container route noted in the main document.

## Checklist (from main document §15)

- [x] Scaffold Cargo workspace + CI + dev-MySQL/`TEST_MYSQL_URL` harness
      — 2026-10-16. Workspace in `vmangos-rs/` (edition 2024,
      rust-version 1.85, `unsafe_code` forbidden, clippy pedantic with
      cast allowances for wire code); gate of record is
      `vmangos-rs/ci.sh` (fmt, clippy `-D warnings`, build, test, live
      tests when `TEST_MYSQL_URL` set) — no Rust job in root CI yet
      (root `.github/workflows/` is outside this goal's write boundary;
      add later with owner approval). Pattern crate `mangos-guid`
      (12 golden tests from `ObjectGuid.h` formulas, all passing) and
      seed crate `mangos-db` (pure-Rust `mysql` 26 driver per main
      document §11 recommendation; owner-overridable before Phase 1
      builds on it). Fixtures: local MySQL 26.7 (brew) rebuilt by
      `vmangos-rs/scripts/setup-dev-mysql.sh` → realmd 15 tables /
      6 migrations, characters 60 / 18, logs 11 / 6 (base dumps need
      session `sql_mode=''` on modern servers); pre-rebuild dump kept at
      `/tmp/dev-mysql-pre-phase0-20261016.sql.gz`. Live test
      `live_connect_and_count_migrations` passes.
      Credentials in `vmangos-rs/.env.test` (mode 600, gitignored).
- [x] Map production binary→commit; stamp revisions going forward
      — 2026-10-16. Both production binaries embed
      `2bae5f921f59892d4c41 / 2026-04-12 01:47:07 +0000 / Linux_x64
      (little-endian)` (read via `strings`, read-only), i.e. commit
      `2bae5f921f59892d4c41002a0869b154f02867d3` (v0.9.0 docs commit).
      `mangosd` built 2026-04-14 02:03 UTC, `realmd` 2026-04-13 02:34 —
      minutes before the Apr-14 02:06 commit batch, so production does
      NOT include the formation system (`25ae6c3`) or its follow-ups.
      The surveyed checkout `3589f72` is 10 commits ahead (968 files,
      ~119k lines churn; list in git log `2bae5f9..3589f72`). C++
      stamping already exists (CMake `rev-parse --short=20` +
      `show -s --format=%ci` → `src/shared/revision.h`, CMakeLists.txt:
      143-174) and surfaces in the startup banner and the `uptime`
      table's `revision` column. Rust equivalent created:
      `crates/mangos-build-info` (build.rs mirrors those exact git
      commands and fallbacks; 2 tests). Closes the binary→commit half
      of main-document §14 #30/#38; the `NRestarts` cause stays open.
- [x] Droplet hygiene (per-action confirmation): disable legacy units,
      credential-file modes
      — executed 2026-10-16 with the owner's per-action confirmation
      in-session. `mangosd.service`/`realmd.service` (the legacy pair
      pointing at the deleted `/home/wow/vmangos`, at 90,876 and
      272,630 restarts and still failing that minute) are now
      `disabled`/`inactive`. `.env`, `mangosd.conf`, `realmd.conf`
      are mode 600 (were 664/644/644). All four `azeroth-*` services
      verified active immediately after; `mangosd-main` kept its PID.
      **Restore-boots evidence (upgrades the restore item above):**
      the production-commit oracle `mangosd` (container build,
      `2bae5f9`, arm64) booted against the locally restored snapshot
      + the full copied DataDir (2.9 GiB now on the bench) and
      reached `World initialized.` in 2 s, inserted its `uptime`
      row stamped `2bae5f921f59892d4c41`, and ran until deliberately
      stopped after ~3.5 min (log: `/tmp/oracle-run/logs/Server.log`
      on the dev bench; config `/tmp/oracle-run/etc/mangosd.conf`
      with bot systems disabled for a clean boot). Shutdown-tail
      MySQL 1785 GTID errors are a local MySQL 26.7 GTID-mode
      artifact against MyISAM tables, not a restore defect.
- [x] Test a backup restore into a scratch DB end-to-end
      — 2026-10-16, done locally (better than on the droplet: zero
      production impact, no confirmation needed). All four 2026-10-15
      nightly dumps restored cleanly into local MySQL: realmd 15
      tables, characters 61, mangos 174, logs 11; 23 accounts, 287
      characters, 10,912 creature_template, 20,034 item_template.
      Live migration state proven complete at snapshot time:
      world 1005/1005, characters 18/18, logon 6/6, logs 6/6
      (max world id `20260331120000`); characters engines
      48 MyISAM / 13 InnoDB (matches main document §5).
      **Schema-drift finding**: prod `characters.bot_assignments`
      (10 rows; read by `DynamicBotAI` via `BotDirector`, shape:
      guid, behavior enum, target_map/zone/x/y/z/level, radius,
      faction) has no `CREATE TABLE` in any repo schema/migration —
      it was created out of band. The fixture script therefore does
      not create it. Follow-up (owner-approved migration to formalize
      the table) is recommended; creating it is outside this goal's
      write boundary and touches the append-only data lock.
      **Bot/pool note**: the local fixture DBs were replaced by the
      snapshot; rebuild clean fixtures any time with
      `vmangos-rs/scripts/setup-dev-mysql.sh`.
      **Not done**: booting the C++ server against the restored
      snapshot (no local C++ build, §10) — restore-load is proven,
      boot is not; see checklist item "Working C++ build".
- [x] Copy production DataDir fixtures + nightly DB dump to dev bench
      — 2026-10-16. `~/azeroth-dev-fixtures/` (79 MiB, checksums in
      `MANIFEST.sha256`, 172 files): all four 2026-10-15 nightly dumps,
      the full `5875/dbc/` set (27 MiB), sampled
      `maps/` (incl. 200,536-byte max-size tile), `vmaps/` (000.vmtree
      + tile), `mmaps/` (000.mmap + smallest/largest tiles), and log
      excerpts (LevelUp tail 2000, Server.log malformed-traffic sample
      300 lines — log-derived, not wire captures). Magics verified
      locally: `WDBC`, `MAPSz1.4`, `VMAP_7.0`, `.mmtile` magic is the
      LE u32 `MMAP` (bytes `PAMM`, tile version field 6); `.mmap` is a
      raw struct dump with no magic (main document §7).
- [x] Collect golden corpora (packets, SRP vectors, DBC inputs,
      LevelUp samples, 16 dashboard payloads + `!status`)
      — Dashboard capture 2026-10-16:
      `~/azeroth-dev-fixtures/corpora/dashboard/` (payloads +
      `manifest.json` + capture script) taken against the restored
      snapshot. All 9 GET routes captured (status, players, activity,
      auctions, raids, bots, guild roster/detail, addons); 6 write
      routes probed sessionless (uniform 401 `{"error": ...}`).
      Findings: (a) **prod bug** — `/api/guild/bot/<guid>` 500s
      against the current world DB (`it.ItemLevel` queried,
      `item_template.item_level` actual; MySQL 1054). Shape-stable
      rules mean fixing it is an owner decision, recorded here not
      fixed. (b) `/api/addons` is genuinely empty in production
      (`addons.json` is `[]` on the droplet too) so the detail route
      has nothing to capture. (c) `/api/activity` needs the full
      LevelUp.log: only 20 of 51,612 lines are non-`<BOT>`.
      **Privacy**: `logs/LevelUp.full.log` on the dev bench contains
      real player IPs — local bench only, never commit or share.
      Discord `!status`: query replicated in
      `corpora/discord_status.json` (2 human characters exist,
      0 online at dump time; embed rendering is presentation-only).
      Packet corpus remains log-derived — no wire captures exist
      anywhere; Phase 3 fuzzing should generate them synthetically.
      — analysis 2026-10-16: `~/azeroth-dev-fixtures/corpora/`.
      **SRP**: `srp_vectors.json` (+ derivation script) — §3.6 caveat
      CLOSED by inspection: all four `CalculateVerifier` call sites
      (`AccountMgr.cpp:56,125,160,269`) use a fresh `SRP6`; `A` is the
      zero BigNumber whose `AsByteArray()` is empty, so it feeds zero
      bytes. Two of three dashboard oracle vectors match the
      C++-faithful derivation; the third (salt=1) exposed a dashboard
      divergence (it pads the salt to 32 bytes, C++ feeds minimal
      length) that is unreachable for `BN_rand(top=0)` salts
      (`BigNumber.cpp:55`; all 6 non-NULL production salts are 64 hex
      chars). Rust must port the C++ feed; pin both in Phase 1 tests.
      **DBC**: `dbc_headers.json` — all 158 copied DBCs (4 zero-byte)
      pass header size arithmetic (20 + records×size + string block =
      file size); `Map.dbc` decodes ("Azeroth");
      `scripts/tests/test_dbc.py` + `dashboard/tests/test_auth.py`
      25 passed in the dev-bench venv.
      **Still pending**: dashboard 16-route payload capture + bot
      `!status` equivalent against the restored snapshot; packet
      corpus remains log-derived (no wire captures exist).
- [x] Record a perf baseline — IDLE baseline only, 2026-10-16 01:41 UTC
      (read-only; a fixed-population load test would need production
      changes, see objective fallback): host load 0.47, `mangosd-main`
      73 threads, RSS 1,412,860 KiB, 2.1% lifetime CPU, up 31d17h;
      `realmd-main` 10 threads, 10,420 KiB; `PlayerLimit = 100`;
      3,338 MiB memory available. World population at capture: idle
      (0 users per `uptime`; consistent with main document §9). Any
      parity claim must beat or match this footprint, and a loaded
      baseline remains to be captured in Phase 3+ on the dev bench.
- [x] Transcribe remaining character save/load surface (23 load queries,
      ~28 save tables) — also §14 #36
      — 2026-10-16, mixed-routing workflow (spark child + wonder-full
      verifier), archived as `20-save-load.md` here. Load: 22 holder
      slots (21 live + dead `BATTLEGROUND_DATA`), LOADFROM 58 columns
      re-verified. Save: one GUIDLow transaction covering the
      `characters` REPLACE (**61** binds — corrects the main
      document's 62, fixed there too) and 10 in-transaction sub-saves;
      stats/pet/corpse outside it; `SaveToDB` triggers no
      mail/auction/social writes.
- [x] Per-lock inventory/ordering (§14 #34); per-opcode payload
      catalogue (§14 #35)
      — 2026-10-16, same workflow: `21-locks.md` (named inventory by
      subsystem; two sampled co-held ordering pairs, one
      verifier-confirmed: `m_visibleGUIDs_lock` →
      `HashMapHolder<Player>::i_lock`; BattleGroundMgr confirmed
      unguarded) and `22-opcode-catalog.md` + `.csv` (826 rows, counts
      reconciled exactly, 4 end-to-end manual checks). Residuals for
      Phase 3: 47 `?`-typed rows need human triage, ~10 `call(...)`
      helpers unexpanded, lock-nesting survey sampled not exhaustive.
- [x] `guild_bot_roster` reload liveness (§14 #23); `characters.online`
      writer (§14 #25)
      — 2026-10-16, `23-traces.md`, verifier-confirmed: (A) roster is
      SELECTed once at startup (`GuildBotMgr.cpp:58`, via
      `World.cpp:1819`); no periodic/command re-read exists, so
      **dashboard roster edits are invisible until mangosd restart**
      and can be clobbered by later C++ updates from stale memory.
      (B) `online` written at create=0, login=1, every autosave
      (15-min default), logout=0, bulk 0 at boot/shutdown; stale-1
      possible after crash until next boot.
- [x] Working C++ build off the droplet (Linux container)
      — 2026-10-16. Recipe: Ubuntu 22.04 image (gcc 11.4 / cmake
      3.22.1, matching the droplet), deps from CI plus
      `libelf-dev libdw-dev binutils-dev ca-certificates` (cpptrace
      `FetchContent`s libdwarf at configure — needs network). Build
      from a full `git clone` of the repo (a linked worktree stamps
      revision `unknown` because its `.git` file points outside the
      mount). Results:
      (a) **Production commit `2bae5f9`: `mangosd` + `realmd` both
      build clean in ~60 s** (arm64 container on this Mac; oracle
      binaries, not droplet-deployable x86_64), correctly stamped.
      (b) **HEAD `3589f72`: `realmd` builds; `mangosd` does not.**
      First failure: `src/mangosd/WorldRunnable.cpp` —
      `BattleGroundDefines.h:157 'Team' does not name a type`,
      `MovementInfo.h 'Position' does not name a type`. Root cause:
      the tree-wide clang-format include alphabetization in `dbbd226`
      broke TUs that relied on transitive include order (the `game`
      library is masked by its PCH; the `mangosd` target has no PCH);
      `3589f72` fixed one instance (`Database.h`) and its message
      scopes verification to the shared+game libraries only. There
      may be further broken TUs behind this one. Fixing C++ includes
      is outside this goal's write boundary; recorded as a
      pre-existing breakage for the owner (it also means the
      surveyed checkout cannot itself serve as a bootable oracle
      until fixed — use `2bae5f9`).
      Build trees kept at `/tmp/azeroth-clone` (HEAD) and
      `/tmp/azeroth-prod` (`2bae5f9`); image `vmangos-build:22.04`
      (Dockerfile draft at `/tmp/vmangos-build/Dockerfile` — move it
      into the repo with owner approval when CI gains a build job).

## Log

- 2026-10-16: Goal created (blocked once to repair a garbled objective
  fragment, then reactivated; the runtime does not allow a replacement
  goal while one is unfinished, hence the correction above).
  Environment inventory recorded. No Phase 0 execution yet.
- 2026-10-16: Mixed model routing added as binding owner decision
  (see Owner decisions).
- 2026-10-16: Scope 1+2 executed. Stale `vmangos-rs/target/` (1.7 GiB)
  cleared. Workspace + `mangos-guid` + `mangos-db` created; full
  `ci.sh` gate green including the live MySQL test. Local fixture DBs
  rebuilt from repo schemas+migrations (counts in checklist).
- 2026-10-16: Scopes 3–6 executed (binary→commit `2bae5f9`,
  `mangos-build-info`, snapshot restore + oracle boot to
  `World initialized.`, corpora incl. the dashboard salt-padding
  divergence and `ItemLevel` prod bug, idle baseline).
- 2026-10-16: Scope 4 droplet actions executed with per-action
  confirmation; scope 7 audit done; main document §14 annotated for
  20 closed/advanced items; clean fixtures rebuilt as the final local
  DB state; final gates: `ci.sh` green (15 Rust tests incl. live),
  Python suites 78 passed / 4 skipped.

## Completion audit (2026-10-16)

Requirement-by-requirement against the goal objective:

| Scope | Evidence | Verdict |
|---|---|---|
| 1 Workspace | `vmangos-rs/` (3 crates), `ci.sh` green end-to-end | done (root-CI Rust job = recorded follow-up; exit criteria accept CI-equivalent local runs) |
| 2 Dev MySQL + harness | `scripts/setup-dev-mysql.sh`; live test passes | done |
| 3 Binary map + stamping | `strings` → `2bae5f9…67d3`; `mangos-build-info` crates | done |
| 4 Droplet hygiene | units disabled/inactive; modes 600; services verified after | done (restore executed locally instead of on the droplet — strictly safer, same dumps) |
| 5 Corpora | `~/azeroth-dev-fixtures/corpora/` (SRP, DBC headers, dashboard payloads, discord status) + `logs/` | done (packet corpus is log-derived; no wire captures exist) |
| 6 Perf baseline | idle snapshot above | done in the objective's idle fallback form |
| 7 Durable state | this file + §14 annotations (20 items) in the main document | done |

Carried forward openly (not Phase 0 blockers): HEAD `mangosd`
include-order breakage; `bot_assignments` schema-drift migration;
dashboard `ItemLevel` fix; root-CI Rust job; loaded perf baseline;
remaining §14 items without a Phase 0 annotation. Everything is
uncommitted on `rust-migration` for owner review.
