# Azeroth Server

VMaNGOS-based World of Warcraft 1.12.1 private server with a custom bot stack, a Flask web dashboard, and a Discord bot. The vmangos source is fully owned — no upstream relationship — so changes land directly, not as PRs against an upstream project.

## Stack

- Game server: C++14, CMake build, MySQL 5.7+ (three databases: `realmd`, `mangos`, `characters`, plus `logs`)
- World server (`mangosd`, port 8085) and realm server (`realmd`, port 3724), both compiled from `vmangos/src/`
- Dashboard: Python 3.10+, Flask, PyMySQL, served via systemd
- Discord bot: Python 3.10+, discord.py, also systemd-managed
- Style: clang-format for C++ (config at `vmangos/.clang-format`), ruff for Python (`ruff.toml`)
- Scripts: bash for ops (`scripts/backup-db.sh`, `scripts/release.sh`), Python for data gen (`scripts/generate_talent_builds.py`)

## Project phase

**Current phase**: `production` <!-- Real characters exist on the server. Treat their data as sacred. -->

The phase governs how strict the rules are about backwards compatibility. In `production`:

- DB migrations are **append-only**. Never edit a migration that has shipped. Never `DROP TABLE` or narrow a column type on a player-data table without a written deprecation plan.
- The dashboard JSON API is **shape-stable**. Endpoints in `dashboard/dashboard.py` listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#dashboard-api) cannot change response shapes without a versioned route. Adding fields is fine; renaming or removing them is not.
- Bot behaviors are added as new layers, not rewrites. The `BotBehaviorSet` runner in `vmangos/src/game/PlayerBots/BotBehavior.cpp` scores and falls through — adding a new `BotBehavior` subclass is the additive path. Replacing or deleting an existing scored behavior requires checking what it's currently doing in `Score()` for live bot populations.
- Save-data formats (character inventory, item instance, guild bot roster) are locked. Schema additions go through `vmangos/sql/migrations/` with `IF NOT EXISTS` guards.

If you are unsure which phase you are in, read this file. Do not assume.

## Hard requirements (regardless of phase)

- **Secret hygiene**. Never commit `.env`, `mangosd.conf`, or `realmd.conf`. The `.example` templates are committed; the live files are gitignored. The pre-commit hook scans staged changes via `gitleaks` and hard-fails if gitleaks is not installed — secret scanning is not optional. See `.claude/rules/security.md`.
- **Conventional commits**. The `commit-msg` hook enforces `<type>(<scope>)!: <subject>`. Project scopes: `vmangos`, `dashboard`, `discord-bot`, `scripts`, `db`, `bots`, `docs`. See `.claude/rules/commits.md`.
- **Append-only player data**. See `.claude/rules/database.md` for the migration discipline.
- **Sources for WoW mechanics**. Patch notes, wowhead-classic, vanilla wiki, Wayback Machine, BlueTracker. Never private-server forums. See `vmangos/CONTRIBUTING.md` and `.claude/rules/wow-domain.md`.

## Rules files

Read these for their respective domains:

- `.claude/rules/cpp.md` — C++14 style, VMaNGOS conventions, logging, ownership
- `.claude/rules/bots.md` — Bot AI behavior layer architecture and the gotchas that bite
- `.claude/rules/wow-domain.md` — Vanilla 1.12.1 mechanics, GUIDs, opcodes, talent system, sources of truth
- `.claude/rules/database.md` — MySQL schema layout, migration discipline, credential handling
- `.claude/rules/python.md` — Python services (Flask dashboard, Discord bot), ruff, venv conventions
- `.claude/rules/clean-code.md` — Function design, naming, comments, anti-over-engineering
- `.claude/rules/commits.md` — Conventional Commits with this project's scopes
- `.claude/rules/testing.md` — What "tested" means here (mostly in-game gameplay smoke tests)
- `.claude/rules/security.md` — Secrets, dependency hygiene, auth handling

## Architecture reference

Deep dive into the bot stack, formation system, social/services architecture, dashboard endpoints, and design lessons: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). When this file and the architecture reference disagree, this file wins for *rules*; the architecture reference wins for *how things work today*.

## Git hooks

Two hooks live in `.githooks/`, wired up with one command per clone:

```bash
git config core.hooksPath .githooks
```

- **`pre-commit`** — scans staged changes for secrets via `gitleaks` (hard-fails if missing), runs `clang-format --dry-run -Werror` on staged C++ files, runs `ruff check` and `ruff format --check` on staged Python files. C++ build is not run here (too slow for a commit hook) — CI handles that.
- **`commit-msg`** — validates the subject line against Conventional Commits 1.0 with this project's scopes. Pure bash, no commitlint dependency.

If `gitleaks` is not on PATH, install it:
- Linux: `brew install gitleaks` (Linuxbrew) or download a release binary from https://github.com/gitleaks/gitleaks/releases
- Windows: `winget install Gitleaks.Gitleaks`

## Common commands

```bash
# --- Build the game server ---
cd vmangos/build
cmake .. -DCMAKE_BUILD_TYPE=Release
make -j$(nproc) && make install

# --- Run the servers (after configs are filled in) ---
vmangos/run/bin/realmd     # auth, port 3724
vmangos/run/bin/mangosd    # world, port 8085

# --- Python services (after `python3 -m venv` + pip install per service) ---
dashboard/venv/bin/python dashboard/dashboard.py    # Flask, port 5000
discord-bot/venv/bin/python discord-bot/bot.py

# --- systemd in production ---
sudo systemctl status azeroth-realmd azeroth-mangosd azeroth-dashboard azeroth-discord-bot
sudo systemctl restart azeroth-dashboard

# --- Database ---
mysql -u mangos -p characters                       # password from .env
scripts/backup-db.sh                                # manual backup; cron runs nightly

# --- Migrations ---
python3 vmangos/sql/make_migration.py "add column ..."   # creates timestamped file
./vmangos/sql/touch_migration.sh                          # empty template
mysql -u mangos -p < vmangos/sql/migrations/<file>.sql    # apply

# --- Release ---
scripts/release.sh              # auto-detect bump from commits since last tag
scripts/release.sh --dry-run    # preview

# --- Logs ---
tail -f vmangos/run/bin/Server.log
journalctl -u azeroth-discord-bot -f
```

## Definition of done

A change is done when:

- Code compiles clean — `make` with no new warnings against the current baseline
- `clang-format --dry-run -Werror` passes on changed C++ files
- `ruff check` and `ruff format --check` pass on changed Python files
- No migration that has already shipped has been edited
- Commit subject matches Conventional Commits with a project scope
- For bot behavior, spell, or movement changes: an in-game smoke test was run (e.g. a Deadmines clear with the affected role) and the result is noted in the PR description — screenshots or `Server.log` excerpts count as evidence
- For dashboard API changes: the existing JSON shape was preserved (additive only in production phase)
- Secrets review: nothing in the diff looks like a real credential, token, or password

## What lives in this repo vs elsewhere

In this repo:
- C++ game server (`vmangos/`)
- Flask dashboard (`dashboard/`)
- Discord bot (`discord-bot/`)
- Ops scripts (`scripts/`)
- DB schemas and migrations (`vmangos/sql/`)
- Architecture reference and research notes (`docs/`)

Not in this repo:
- World DB content dumps (too large; pulled separately into `vmangos-database/`, gitignored)
- Extracted client data (maps, mmaps, vmaps, dbc — all gitignored under `vmangos/run/bin/`)
- Player-facing addons / UI mods (their own repos; the dashboard hosts release zips, listed at `/addons`)
- Compiled binaries (gitignored under `vmangos/run/bin/`)
