# vmangos-rs

Rust port of the vmangos game server, built phase by phase per
[`docs/RUST_MIGRATION_ARCHITECTURE.md`](../docs/RUST_MIGRATION_ARCHITECTURE.md)
(crate map §12, sequencing §13). Started from scratch on 2026-10-16;
there is no prior Rust code in this tree.

## Layout

Crates are created only when their phase starts — do not scaffold the
full §12 map up front.

| Crate | Phase | Maps to |
|---|---|---|
| `crates/mangos-guid` | 0 (pattern crate) | `vmangos/src/game/ObjectGuid.{h,cpp}`, packed GUID in `ByteBuffer.h` |

## Toolchain

- Edition 2024, `rust-version` floor 1.85 (workspace `Cargo.toml`).
- `rust-toolchain.toml` pins 1.98.1 for rustup/CI. The current dev Mac
  uses Homebrew rustc 1.98.1 without rustup, so the pin is honoured by
  CI and rustup users only; keep Homebrew in step manually.
- `unsafe_code` is forbidden workspace-wide; the Detour FFI crate
  (Phase 2 spike) will be the named exception when it exists.

## Gates

```bash
./ci.sh   # fmt --check, clippy -D warnings, build, test
```

## Live DB tests

Database-backed tests are `#[ignore]`d and read `TEST_MYSQL_URL`
(e.g. `mysql://mangos:<dev-pw>@localhost:3306/realmd`). `ci.sh` runs
them only when that variable is set, mirroring the convention recorded
in the migration document (§5). Fixture setup lives with the `mangos-db`
crate when it is created in Phase 1.
