#!/usr/bin/env bash
# Local CI-equivalent gate for the Rust workspace. The root GitHub
# workflow (.github/workflows/ci.yml) has no Rust job yet; until one is
# added with owner approval, this script is the gate of record
# (docs/RUST_MIGRATION_ARCHITECTURE.md Phase 0 exit criteria).
set -euo pipefail
cd "$(dirname "$0")"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test --workspace

# Live DB tests are #[ignore]d and env-gated; they run only when a dev
# MySQL URL is provided (see README.md and docs section 5).
if [ -n "${TEST_MYSQL_URL:-}" ]; then
  cargo test -p mangos-db -- --ignored
else
  echo "TEST_MYSQL_URL unset -- skipping live DB tests"
fi

# Production-fixture parity suites (Phase 2) are #[ignore]d and gated
# on fixture locations; see each crate's tests for the env vars.
if [ -n "${MANGOS_DBC_DIR:-}" ]; then
  cargo test -p mangos-dbc -- --ignored
else
  echo "MANGOS_DBC_DIR unset -- skipping production DBC parity tests"
fi
if [ -n "${MANGOS_MAPS_DIR:-}" ]; then
  cargo test -p mangos-assets --test production_gridmap -- --ignored
else
  echo "MANGOS_MAPS_DIR unset -- skipping production gridmap parity tests"
fi
if [ -n "${MANGOS_VMAPS_DIR:-}" ]; then
  cargo test -p mangos-assets --test production_vmap -- --ignored
else
  echo "MANGOS_VMAPS_DIR unset -- skipping production vmap parity tests"
fi
if [ -n "${MANGOS_MMAPS_DIR:-}" ]; then
  cargo test -p mangos-detour --test production_mmap -- --ignored
else
  echo "MANGOS_MMAPS_DIR unset -- skipping production mmap parity tests"
fi
