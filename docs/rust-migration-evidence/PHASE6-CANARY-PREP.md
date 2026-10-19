# Phase 6 Slice 2 — Canary Preparation (prep only, gated)

Status: **prepared, not executed.** No production cutover, no
droplet writes, and no live-host canary happen under Phase 6.
Running any step in §4–§6 on the droplet requires a separate
explicit owner gate.

## 1. What the canary would be

Per `docs/RUST_MIGRATION_ARCHITECTURE.md` §13.2 Phase 6: the
Rust `mangosd` (and `realmd`) built off-box, installed at the
same path the systemd unit already executes
(`vmangos/run/bin/mangosd`, process name `mangosd`), reading
the same config, assets (DataDir dbc/maps/vmaps/mmaps), and
databases as the C++ binary. The C++ binary is kept on disk
for immediate rollback. `/tmp/azeroth-maintenance` semantics
are unchanged (the Rust server never touches that flag; the
owner's maintenance tooling keeps working as-is).

Canary shape (owner picks at gate time): a non-production
realm or test accounts on the live host during a maintenance
window — the droplet has no spare capacity for a side-by-side
realm (§9), so the realistic shape is a maintenance-window
swap with test accounts first, then player traffic.

## 2. Build recipe (off-box, glibc 2.35 floor)

The droplet target is Ubuntu 22.04-class glibc (2.35). Build
in a Linux container, never on the droplet:

```bash
docker run --rm -v "$PWD:/repo" -w /repo/vmangos-rs \
  rust:1-bookworm bash -c '
    apt-get update && apt-get install -y g++ libssl-dev pkg-config &&
    cargo build --release -p mangosd -p mangos-auth'
# artifacts: vmangos-rs/target/release/{mangosd,realmd}
```

Notes:
- `mangos-detour` compiles vendored C++ through `build.rs`
  (`cc`), so the container needs `g++`; `-ffp-contract=off`
  is already pinned in the build script.
- A musl static build is the fallback if glibc symbol drift
  ever bites; not needed while the build box floor (bookworm
  glibc 2.36 build host vs 2.35 target — verify with
  `objdump -T mangosd | grep GLIBC_` max ≤ 2.35, or build in
  an `ubuntu:22.04` container to be exact).
- Verify the artifact before shipping: `file`, `ldd` inside
  an `ubuntu:22.04` container, and a bench boot of the exact
  release binary against the cloned bench DB (same
  differential oracle as Slice 1).

## 3. Compatibility surface (already proven on the bench)

- Config: same `mangosd.conf` keys; Phase 6 adds reads for
  `PlayerSave.Interval` (default 900000 ms),
  `PlayerSave.Stats.MinLevel` (0),
  `PlayerSave.Stats.SaveOnlyOnLogout` (true),
  `PlayerBot.AllowSaving` (false) — all defaulting to fork
  behavior, so an unmodified production conf is valid.
- DB: no schema changes. The save surface is the fork's own
  statements (see PHASE6-PROGRESS Slice 1 inventory);
  differential byte-compare on a cloned characters DB
  passed (idle, mutation, fresh-aura, periodic passes).
- Assets: both binaries read the same extracted client data;
  the Rust server writes nothing under DataDir.

## 4. Install procedure (owner-gated; not run)

1. Announce maintenance; set `/tmp/azeroth-maintenance`.
2. `scripts/backup-db.sh` — full backup; this is the DB
   rollback point. Verify the dump loads (row counts).
3. Stop `azeroth-mangosd` (and `azeroth-realmd` if swapping
   both). Players are saved by the C++ server on shutdown.
4. Preserve the old binaries in place:
   `cp -a vmangos/run/bin/mangosd vmangos/run/bin/mangosd.cpp-2bae5f9`
   (same for `realmd`).
5. Copy the release binaries to `vmangos/run/bin/{mangosd,realmd}`
   (same filenames → unit `ExecStart` untouched).
6. Start `azeroth-realmd`, then `azeroth-mangosd`; watch
   `Server.log` through world init and first bot cycle.

## 5. Canary validation checklist (owner-gated)

- Test-account real-client login (stock 5875), move, logout,
  relog: position/health/aura persisted.
- Trade with a service bot; mail money; AH list/buy.
- Five-bot Deadmines clear.
- Dashboard loads; guild roster round-trip; LevelUp.log
  still parses (discord bot unaffected).
- `characters.online` freshness within the 15-minute
  autosave contract (evidence 23).
- Soak: one full bot-population cycle with no new
  `Server.log` error classes vs the C++ baseline.

Known blocker to clear at the gate, not hide: **there is no
XP/level-up runtime yet** (`PLAYER_XP` loads and saves, but
nothing grants XP), so "level-up persistence across relog"
cannot be exercised until a progression slice lands or the
owner waives it for the canary. Pet save, item-cooldown
rows, and the aura interrupt-flag gate are likewise
documented gaps (Slice 1 audit), acceptable only by owner
decision.

## 6. Rollback (owner-gated)

1. Stop the Rust `azeroth-mangosd`/`azeroth-realmd`.
2. Restore `mangosd.cpp-2bae5f9` → `mangosd` (same for
   `realmd`); start the units. Binary rollback is sufficient
   for any pure-server defect: the Rust save writes only
   fork-shaped rows, which the C++ loader reads.
3. Only if save corruption is proven: restore the §4 step 2
   backup (loses canary-window playtime — announce first).
4. Clear `/tmp/azeroth-maintenance` when the C++ stack is
   verified healthy.

Rollback trigger examples: login failures, save-diff alarms,
bot population collapse, or any player-data anomaly. When
in doubt, roll back the binary first and diagnose off-line;
the DB restore is the last resort, not the first.
