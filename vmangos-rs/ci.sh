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
  cargo test --workspace -- --ignored
else
  echo "TEST_MYSQL_URL unset -- skipping live DB tests"
fi
