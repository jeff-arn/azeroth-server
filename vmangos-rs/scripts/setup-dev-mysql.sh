#!/usr/bin/env bash
# Rebuild the LOCAL dev MySQL fixture databases (realmd, characters, logs)
# from the repo's base schemas + migrations, and (re)set the dev `mangos`
# user. This is the Rust live-test fixture from
# docs/RUST_MIGRATION_ARCHITECTURE.md section 5 / Phase 0.
#
# - Local dev machine only. Never point MYSQL_ADMIN at production.
# - Drops and recreates the three fixture DBs. Whatever was in them is
#   gone; they are fixtures, not data. (The world DB cannot be built from
#   this repo alone: no base world dump is tracked, CLAUDE.md "What
#   lives in this repo", so `mangos`/world is skipped.)
# - Credentials: reads vmangos-rs/.env.test if present, else generates a
#   dev-only password and writes that file (mode 600, gitignored).
set -euo pipefail

rs_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(cd "$rs_dir/.." && pwd)"
env_file="$rs_dir/.env.test"
admin=(mysql -u root)
if [ -n "${MYSQL_ADMIN:-}" ]; then
  # shellcheck disable=SC2206
  admin=($MYSQL_ADMIN)
fi

if [ -f "$env_file" ]; then
  # shellcheck disable=SC1090
  . "$env_file"
fi
if [ -z "${DEV_MYSQL_PASSWORD:-}" ]; then
  DEV_MYSQL_PASSWORD="$(openssl rand -hex 16)"
  umask 077
  {
    echo "DEV_MYSQL_PASSWORD=$DEV_MYSQL_PASSWORD"
    echo "TEST_MYSQL_URL=mysql://mangos:$DEV_MYSQL_PASSWORD@localhost:3306/realmd"
    echo "TEST_MYSQL_URL_CHARACTERS=mysql://mangos:$DEV_MYSQL_PASSWORD@localhost:3306/characters"
  } >"$env_file"
  echo "wrote $env_file (new dev password generated)"
fi

"${admin[@]}" <<SQL
CREATE USER IF NOT EXISTS 'mangos'@'localhost' IDENTIFIED BY '$DEV_MYSQL_PASSWORD';
CREATE USER IF NOT EXISTS 'mangos'@'127.0.0.1' IDENTIFIED BY '$DEV_MYSQL_PASSWORD';
ALTER USER 'mangos'@'localhost' IDENTIFIED BY '$DEV_MYSQL_PASSWORD';
ALTER USER 'mangos'@'127.0.0.1' IDENTIFIED BY '$DEV_MYSQL_PASSWORD';
GRANT ALL PRIVILEGES ON realmd.* TO 'mangos'@'localhost';
GRANT ALL PRIVILEGES ON realmd.* TO 'mangos'@'127.0.0.1';
GRANT ALL PRIVILEGES ON characters.* TO 'mangos'@'localhost';
GRANT ALL PRIVILEGES ON characters.* TO 'mangos'@'127.0.0.1';
GRANT ALL PRIVILEGES ON logs.* TO 'mangos'@'localhost';
GRANT ALL PRIVILEGES ON logs.* TO 'mangos'@'127.0.0.1';
SQL

load_db() {
  local db="$1" base="$2" suffix="$3"
  echo "== $db <= vmangos/sql/$base + *_$suffix.sql migrations"
  # The base dumps are MySQL 5.5-era (zero-date defaults et al); modern
  # servers in strict mode reject them. Relax sql_mode for the loading
  # session only -- the resulting schema is what the C++ server expects.
  local lax="SET SESSION sql_mode=''"
  "${admin[@]}" -e "DROP DATABASE IF EXISTS \`$db\`; CREATE DATABASE \`$db\`;"
  "${admin[@]}" --init-command="$lax" "$db" <"$repo_root/vmangos/sql/$base"
  local f
  for f in $(ls "$repo_root/vmangos/sql/migrations/"*"_$suffix.sql" | sort); do
    "${admin[@]}" --init-command="$lax" "$db" <"$f"
  done
  "${admin[@]}" -N -e "SELECT CONCAT('   tables=', COUNT(*)) FROM information_schema.tables WHERE table_schema='$db'; SELECT CONCAT('   migrations=', COUNT(*)) FROM \`$db\`.migrations;"
}

load_db realmd logon.sql logon
load_db characters characters.sql characters
load_db logs logs.sql logs

echo "done. TEST_MYSQL_URL for realmd is in $env_file"
echo "note: world (mangos) DB skipped -- no base dump in repo"
