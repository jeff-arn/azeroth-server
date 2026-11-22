# Tier 4 Canary Runbook — Owner Gate

Status: **EXECUTED 2026-11-10; REDEPLOYED +
HOTFIXED TWICE 2026-11-21; stance-gate rebuild
installed 2026-11-22 (owner Ruikanis smoke
reported good; commit decision pending).**
The owner
approved the waivers (plus gate-1 additions
CrowdBot, BattleBot.AutoJoin, AuraTracker) and
the window ran; see the Slice 6 record at the
end of `TIER4-PROGRESS.md`. The droplet
currently runs the installed stance-gate pair
in the table below (`a8c38080…` /
`eb92ba54…`); rollback pairs are preserved on
the droplet as `mangosd.6386-bak` /
`realmd.8107-bak` (stance-gate pre-swap rage
pair), `mangosd.6a45-bak` (rage-hotfix pre-swap),
`mangosd.a4b8-bak` / `realmd.8107-bak` (first
redeploy), `mangosd.e362-bak` /
`realmd.7258-bak` (Nov 10 canary), and the
untouched C++ `.cpp-bak` pair. The stance-gate
pair was installed 2026-11-22 after a fresh
explicit owner go, a verified backup
(`*_20261122_022122.sql.gz`), staged re-hash,
and binary-only swap; both units returned
active/listening with no severe new log lines
at first check. The owner then reported the
Ruikanis smoke looks good; post-smoke read-only
checks still matched the live hashes, both game
units active/listening, realm `realmflags=0`
and `flag=0` (maintenance already off), and no
severe save/session errors in the new log. No
commit or push is authorized by this record.

## Artifacts (Slice 3 + 2026-11-21 rebuild, verified locally)

Built in `ubuntu:22.04` linux/amd64
(glibc 2.35-0ubuntu3.15), rustc/cargo 1.98.1,
`cargo build --release -p mangosd -p mangos-auth`,
target dir `vmangos-rs/target-ubuntu2204-amd64/`.
The 2026-11-21 rebuild used the same container
recipe with Colima/Lima (Virtualization.Framework
+ Rosetta) as the Docker engine because
OrbStack's guest did not come up headlessly.
The pending stance-gate rebuild used an
isolated 6 GiB Colima profile (`buildvm`, since
stopped) with `CARGO_BUILD_JOBS=1` and
`RUSTFLAGS=-C codegen-units=1` after the
default 2 GiB profile OOM-killed `rustc` in
`mangos-map`; the dashboard logs migration was
held out of the build glob and restored, so
this pair still expects the pre-dashboard
logs migration set and requires no schema
change.

| Binary | Live SHA-256 (stance gate) | Previous SHA-256 (rage hotfix) |
| --- | --- | --- |
| `mangosd` | `a8c380801c4bf1409f6d91f20111ea7bf5a3ac51b4f0a757c0049afe0c4b86d4` | `6386c1a8921a0a44d1163884884012343ccd9ca98594c37aaec25f78c25d96b0` |
| `realmd` | `eb92ba54e2fdef560130ef6658e9a152907705213a0206128852f18b0bc637d5` | `810723a589b1d1191c41a0053462fed64ab32e4b010c399d2a84164099680f85` |

Both live binaries are x86-64 ELF PIE, max
glibc symbol `GLIBC_2.34` (≤ droplet 2.35);
host and container `sha256sum`/`file`/`ldd`
agree (system `libstdc++/libgcc_s/libm/libc`
only). The amd64 `mangosd` booted in
the isolated container against the cloned
bench DB/DataDir to `World server listening
on 0.0.0.0:18101` (94 game events, 4144
creature loot templates, 222 bot identities,
0 persisted corpses) and shut down on the
test timeout's SIGTERM; its only migration
note was the bench logs DB's extra
`20261121175904` warning. All 2026-11-21
builds held the dashboard migration
`20261121175904_logs.sql` out of the build
glob (restored immediately): its tables are
not required by the world server, so no
production schema change rode along. Rebuild
and re-hash at window start if any source
changed after this document; a hash mismatch
aborts the window.

## Pre-window gates (all required)

1. **Droplet config diff.** On the droplet,
   list `mangosd.conf` key names and normalized
   values for non-secret keys only (never print
   database credentials or connection strings).
   Diff against the Rust-read set in
   `TIER4-PROGRESS.md` (config close-out). Any
   non-default in the "behavior exists behind
   fork-default constants" class (remaining
   `Rate.*`, creature AI scalars, chat flood /
   min-levels, durability, environmental/mirror
   timers, corpse/bones timers, visibility
   in-flight/group-mode, `Spell.EffectDelay`)
   is a blocker or an explicit waiver below.
2. **Real-client pass** on the bench release
   binary: login, movement, kill a mob, loot
   window (gold + item), take money/item,
   group roll with a second client,
   logout/relogin persistence spot-check.
   Tier 4 loot/config changes have loopback
   coverage only so far.
3. **Dedicated smoke characters.** Do not reuse
   bot-population characters for canary smoke;
   the bench bot manager owns and periodically
   saves them (observed saved-dead fixture
   interference during Slice 4 soak).
4. **Backup.** Run `scripts/backup-db.sh` and
   verify the dump exists before stopping any
   service. Tier 4 ships no migrations; do not
   run any.
5. **C++ rollback rehearsal.** Confirm the
   droplet's current C++ `mangosd`/`realmd`
   binaries are preserved in place (copy to
   `mangosd.cpp-bak` / `realmd.cpp-bak` outside
   the unit exec path) and the exact restart
   commands are written down before the swap.

## Owner waiver decisions (decide each)

1. **Fork-parity loot holes (replicated
   verbatim):** autostore does not re-check
   `SendLoot` permission (a NONE-permission
   window holder who guesses a plain unblocked
   slot can take it); repeat roll votes are
   counted per call. Keep for parity, or
   approve a safe-direction deviation first.
2. **Loot sources absent:** gameobject chests,
   fishing, skinning, pickpocket, item-open,
   corpse insignia. Creature corpse loot only.
3. **Reputation-on-kill absent** (quest/BG
   reputation paths exist; kill credit does not
   grant faction reputation).
4. **OutdoorPvP absent** (Silithus/EPL objectives
   do not run). Battlegrounds exist.
5. **Content helper remainder:** ~1280
   `NEEDS-MAP-HELPER` script sites, holder
   script-state storage, script gossip
   `PrepareQuestMenu` sites, and the recorded
   spell-seam remainders in `TIER4-PROGRESS.md`.
6. **Anticheat/Warden:** movement anticheat is
   implemented; non-movement detectors and
   Warden are not. RA/SOAP/console and GM-ticket
   surfaces are reduced vs the fork.
7. **DynamicRespawn** off unless the droplet
   config enables it (then it becomes a blocker,
   not a waiver — fork defaults are off).

## Window procedure (after approval only)

1. Announce maintenance; stop
   `azeroth-mangosd`, then `azeroth-realmd`.
2. Verify backup from gate 4 is present.
3. Install artifacts to the existing binary
   paths (same `mangosd.conf` / `realmd.conf`,
   same DataDir, same systemd units). Keep the
   C++ backups from gate 5 untouched.
4. Start `azeroth-realmd`, then
   `azeroth-mangosd`; watch `Server.log` for
   world load completion, game events, and loot
   template load lines; confirm listening ports
   3724/8085.
5. Owner smoke (gate 2 list) on a real client
   with the dedicated characters from gate 3,
   including one group kill with a loot roll
   and one logout/relogin persistence check.
6. Hold a fixed observation window with the
   rollback commands ready; only then open the
   realm normally.

## Abort / rollback

Abort immediately on: artifact hash mismatch,
world load failure, save errors in `Server.log`,
loot window/money/item defects in smoke, or any
character-data anomaly. Rollback is binary-first
(no schema change to undo): stop the Rust units,
restore `mangosd.cpp-bak` / `realmd.cpp-bak` to
the exec paths, start the C++ units, verify
login. Restore the database from the gate-4
backup only if save corruption is observed;
character data written by the Rust server in
fork-shaped rows is otherwise left in place.

## Explicit non-goals for the canary

No migrations, no dashboard/discord changes, no
config changes beyond reading the existing
droplet conf, no new characters on production
accounts for testing, and no deletion of the
C++ binaries or build tree.
