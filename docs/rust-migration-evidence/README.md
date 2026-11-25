# Rust Migration — Evidence Reports

Scoped surveys behind [../RUST_MIGRATION_ARCHITECTURE.md](../RUST_MIGRATION_ARCHITECTURE.md),
produced 2026-10-15/16 against checkout `3589f72` (2026-05-26) and the live
droplet `jeffs-awesome-droplet` (read-only SSH). Each report cites `file:line`
in this repo. Where reports disagree on a count, the critic (`11`) measured
directly and the main document quotes the critic's value with its counting
predicate.

| File | Scope |
|---|---|
| `01-build-system.md` | CMake targets/deps, local build probe |
| `02-runtime-core.md` | Processes, threads, sockets, opcodes, SRP |
| `03-world-sim.md` | Objects, maps, spells, movement, scripts |
| `04-database.md` | DB layer, schemas, migrations, character save/load |
| `05-bots.md` | PlayerBots stack and core coupling |
| `06-data-assets.md` | DBC/maps/vmaps/mmaps formats, reuse verdicts |
| `07-python-services.md` | Dashboard + Discord contracts |
| `08-live-droplet.md` | Production host, systemd, footprint, backups |
| `09-metrics.md` | LOC, singletons, allocation/cast/lock censuses |
| `10-docs-history.md` | Documented architecture vs code, project rules |
| `11-critic.md` | Completeness check, corrected counts, gaps |
| `raw-*.txt` | Raw read-only captures from the droplet |
| `PHASE0-PROGRESS.md` | Phase 0 execution record: decisions, checklist, evidence paths |
| `20-save-load.md` | Phase 0: character save/load transcription (61-bind REPLACE) |
| `21-locks.md` | Phase 0: per-lock inventory + sampled ordering pairs |
| `22-opcode-catalog.md` (+ `.csv`, generator) | Phase 0: 826-opcode payload catalogue skeleton |
| `23-traces.md` | Phase 0: guild_bot_roster liveness + characters.online writers |
| `24-synthesis.md` | Phase 0: verifier spot-checks over 20–23 (6/6 survived) |

These are point-in-time evidence, not living documentation. Re-run the
relevant scope before relying on a changed area.

Update 2026-10-16: passages in `04`, `09`, `10`, and `11` about a former
`vmangos-rs/crates` workspace (~190 crates) and its `TEST_MYSQL_URL`
harness are superseded — that workspace no longer exists and the
migration starts from scratch. See the Preface and §5 of the main
document.
