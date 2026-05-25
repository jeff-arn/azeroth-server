# Python rules

The Python services in this repo are the Flask dashboard (`dashboard/`) and the Discord bot (`discord-bot/`). Plus a few ops scripts (`scripts/`) and the bot talent-build generator. Style is ruff (config in `ruff.toml`).

## Target

- Python 3.10+ (`target-version = "py310"` in `ruff.toml`). Pattern matching, parenthesized context managers, and PEP 604 union syntax (`int | None`) are all fair game.
- Line length: 120 (formatter handles wrapping). Lint ignores `E501` because formatter manages line length.
- Lints enabled: `E` (pycodestyle errors), `F` (pyflakes), `W` (warnings), `I` (isort).

## Formatting and linting

- `ruff format` is authoritative. Run before committing — the pre-commit hook checks staged Python files.
- `ruff check --fix` for autofixable lint issues.
- Quote style: double quotes. Indent: spaces (4).
- `ruff.toml` excludes `vmangos/` and `venv/`. Don't add Python files under `vmangos/` — that's the C++ tree.

## Virtual environments

Each service has its own venv:

- `dashboard/venv/` (gitignored)
- `discord-bot/venv/` (gitignored)

Create with:

```bash
python3 -m venv dashboard/venv
dashboard/venv/bin/pip install -r dashboard/requirements.txt
```

**Never commit a `venv/` directory.** The `.gitignore` blocks it; if you somehow add one, the pre-commit gitleaks scan will probably catch credentials it contains.

Dependencies are pinned in `requirements.txt` per service. Bump versions deliberately; don't auto-update.

## Environment variables

Configuration loads from environment variables (populated by `EnvironmentFile=/home/wow/wow-private-server/.env` in the systemd unit). Read them **at startup**, not per-request. The pattern:

```python
# Good — read once at import or in a config object
DB_CONFIG = {
    "host": os.environ.get("DB_HOST", "127.0.0.1"),
    "user": os.environ.get("DB_USER", "mangos"),
    "password": os.environ["DB_PASSWORD"],  # required, fail loud if missing
    "port": int(os.environ.get("DB_PORT", "3306")),
}

# Bad — re-reads on every request, masks missing config
def get_db():
    return pymysql.connect(
        host=os.environ.get("DB_HOST"),
        password=os.environ.get("DB_PASSWORD"),  # silently None if unset
        ...
    )
```

Required env vars (no default — fail loud if missing):
- `DB_PASSWORD`
- `DISCORD_BOT_TOKEN` (discord-bot only)

Optional with sensible defaults:
- `DB_HOST` (`127.0.0.1`), `DB_PORT` (`3306`), `DB_USER` (`mangos`)
- `LEVELUP_LOG` (path to the world server's level-up log)
- `FLASK_SECRET_KEY` (dashboard auto-generates per-process if missing — acceptable for ephemeral session signing)

`.env.example` is the source of truth for which keys exist. If you add a new env var, add it to `.env.example` in the same commit.

## Flask (dashboard)

- The dashboard is currently a single-file app (`dashboard/dashboard.py`). If it grows past ~1000 lines, split into Flask blueprints — don't grow the monolith past readability.
- **Response shapes are locked** for API endpoints listed in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md#dashboard-api). Production phase rule: additions are fine; renames and removals require a versioned route (`/api/v2/...`).
- Always return JSON via `jsonify(...)` for API routes — don't return a `dict` directly (works in newer Flask but inconsistent).
- HTTP status codes: 200 for success, 400 for client error, 401 for unauthenticated, 403 for unauthorized, 404 for not found, 500 for unexpected. Don't return 200 with `{"error": "..."}` — let the status code reflect reality.
- Templates live in `dashboard/templates/`. Static assets in `dashboard/static/`. Jinja2 autoescape is on by default — don't disable it.

## discord.py (Discord bot)

- The bot uses discord.py 2.x. Cogs > single-file event handlers as the bot grows. The current `discord-bot/bot.py` may be small enough to live as-is.
- The Discord event loop is async. **Do not call blocking I/O directly** from inside an event handler — it stalls the whole bot. Use `aiohttp` for HTTP, `aiomysql` if DB load becomes a concern, or wrap blocking calls with `asyncio.to_thread(...)`.
- `requests` (sync) is acceptable in startup paths or short ops scripts. **Not** in the bot's event loop.
- Bot replies should rate-limit themselves. Don't send a message in response to every event without a cooldown — Discord will rate-limit you anyway and the user experience suffers.
- Token: `DISCORD_BOT_TOKEN` env var. If it's empty at startup, fail loud with a clear error message; don't try to run without auth.

## Ops scripts (`scripts/`)

- Bash for ops (`backup-db.sh`, `release.sh`). Python for data work (`generate_talent_builds.py`).
- Bash scripts should `set -euo pipefail` at the top.
- Python scripts in `scripts/` use the project's Python 3.10+ baseline. Standalone — they don't need to import from the dashboard or discord-bot.
- A script that needs DB creds reads from `.env` via `python-dotenv` or sources it manually in bash.

## Logging

- Use `logging` over `print`. The discord-bot uses systemd journal — `print` works but bypasses the logging hierarchy.
- `logging.basicConfig(level=logging.INFO, format=...)` at app startup.
- Log levels: `DEBUG` for verbose diagnostics, `INFO` for normal operation, `WARNING` for unusual but not broken, `ERROR` for "this didn't work". Don't log `INFO` for every request — that's what nginx-style access logs are for.
- Never log a DB password, Discord token, or full request body that may contain secrets.

## Testing

There's no unit test framework wired up yet for Python services. See `.claude/rules/testing.md` for what counts as "tested" in this project today. If you do add tests, use `pytest`, put them under `dashboard/tests/` or `discord-bot/tests/`, and add a `pytest` step to CI.

## Anti-patterns

- Reading env vars on every request
- Synchronous `requests.get(...)` inside a discord.py event handler
- A single Flask route function that's >150 lines
- Returning HTTP 200 with `{"error": "..."}`
- Committing `venv/` or `__pycache__/`
- `pip install` without pinning — every new dependency goes into `requirements.txt`
- Silently catching `Exception` — at minimum log it, ideally narrow the catch
- `eval`/`exec` on user-supplied data — there is no good reason for it in this service
