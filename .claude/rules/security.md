# Security rules

This is a private server with real player accounts, real characters, and a public-internet-facing realmd. Security is not optional. The rules below are what the project actually does today, not aspirational.

## Secrets

### What lives where

- `.env` (gitignored) — DB password, Discord bot token, Flask secret key (optional), `LEVELUP_LOG` path
- `vmangos/run/etc/mangosd.conf` (gitignored) — game server's DB credentials, AHBot config, BotChat config, console password
- `vmangos/run/etc/realmd.conf` (gitignored) — auth server's DB credentials, realm registration

Tracked counterparts:
- `.env.example` — keys with placeholders (no values)
- `vmangos/run/etc/mangosd.conf.example` — full config with safe defaults, no credentials
- `vmangos/run/etc/realmd.conf.example` — same

When you add a new env var or config key, **update the `.example` file in the same commit** so anyone setting up a fresh server doesn't have to dig through code to find what's needed.

### Never in git

- DB passwords, Discord tokens, API keys, OAuth secrets, console passwords, JWT signing keys
- SSH private keys, TLS private keys
- `.env`, `mangosd.conf`, `realmd.conf` themselves
- Player IPs, email addresses, account password hashes (these live in the DB, never in the repo)

### Secret scanning

`.githooks/pre-commit` runs `gitleaks` on staged changes. The hook **hard-fails** if gitleaks is not installed — secret scanning is not optional. Install instructions are in `CLAUDE.md`.

`.gitleaks.toml` allowlists known-safe paths: build artifacts, vendored deps, the `.example` templates. If you find yourself wanting to add a path to the allowlist because gitleaks flagged something, **read the flag first** — it's right more often than it's wrong.

If gitleaks catches a real secret in a commit you haven't pushed yet:

1. Don't push.
2. Rewrite history to remove the secret (`git rebase -i`, edit the commit, force-push to your branch — never to main).
3. Rotate the secret. The history-rewrite removes it from git but not from anywhere it might have leaked (clones, hooks that ran, CI logs).

If gitleaks catches a secret that's already on the remote:

1. Rotate the secret immediately.
2. Then deal with the git history. Removing it from `main` requires force-push coordination; talk through it before doing it.

## Auth server (realmd)

- WoW 1.12.1 uses **SRP6** (Secure Remote Password) for account login. The client sends `v` and `s` values; the server verifies without learning the password.
- Implementation lives in `vmangos/src/realmd/`. **Don't roll new auth.** SRP6 is correct and matches the client.
- `realmd.account.v` and `realmd.account.s` cannot be computed externally. To set a password, use the mangosd console (`.account set password <user> <newpass>`) with `Console.Enable = 1` in `mangosd.conf`.
- The WoW 1.12.1 client enforces a **16-character password maximum**. The server's `.account set password` command rejects longer. Don't try to be helpful here — the client will reject the longer password at login anyway.
- Banlist: `realmd.account_banned` / `realmd.ip_banned` are how bans work. Use them; don't invent a new ban system.

## World server (mangosd)

- The world server is **not** a public-facing port in well-configured deployments — only the realm server (`3724`) is exposed. The world server (`8085`) is reached after auth via a redirect.
- Console access: `mangosd.conf` has `Console.Enable` (default `0`). Enable it only when you're going to use it; turn it back off. The console runs as `mangos` user — if it's exposed, it's root-on-the-mangos-process.
- GM commands: locked behind `SEC_MODERATOR` / `SEC_GAMEMASTER` / `SEC_ADMINISTRATOR` via `mangosd_command_table`. When adding a new admin command, pick the **lowest** security level that makes sense. PartyBot was downgraded from `SEC_ADMINISTRATOR` to `SEC_MODERATOR` deliberately — match that thinking.

## Dashboard (Flask)

- The dashboard exposes the server's data over HTTP. If it's reachable from the public internet, treat it as such.
- **Don't expose write endpoints without auth.** `POST /api/guild/raid-plan` and `POST /api/addons` should require admin auth. The current state of this is documented (or should be) in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md). When adding a new write endpoint, gate it.
- SQL queries through PyMySQL use parameterized queries (`%s` placeholders + tuple args). **Never** string-concatenate user input into SQL.
- Jinja2 autoescape is on for `.html` templates. Don't disable it. `|safe` only on values you constructed yourself, never on user-supplied content.
- Flask session signing uses `app.secret_key`. The current code falls back to `os.urandom(32)` if `FLASK_SECRET_KEY` is unset — that's acceptable for ephemeral sessions but means sessions don't survive a restart. Set `FLASK_SECRET_KEY` in `.env` if persistence matters.
- File upload (`POST /api/addons/<id>/releases`) — validate the upload size, restrict file extensions, store outside the web root if you don't trust the content. The current implementation stores under `dashboard/static/addons/` (gitignored).

## Discord bot

- The bot token (`DISCORD_BOT_TOKEN`) is full credentials. Don't paste it in chat, in commits, in logs, in error messages.
- Bot replies don't echo private info from accounts (passwords, emails, IPs). Default to "user-facing" info only.
- A leaked bot token requires: regenerate token in the Discord developer portal, update `.env`, restart the bot. Old token is invalidated automatically on regen.

## Dependencies

- **Python**: each service has a `requirements.txt` with pinned versions. Don't add a dependency without pinning. Run `pip-audit` or `safety check` periodically:
  ```bash
  pip-audit -r dashboard/requirements.txt
  pip-audit -r discord-bot/requirements.txt
  ```
- **C++**: dependencies live in `vmangos/dep/` (vendored — MySQL connector, OpenSSL, zlib, curl, etc.). Don't auto-update them. If a CVE forces an upgrade, do it as its own commit with a clear `chore(vmangos): upgrade <lib> from X to Y for CVE-...` message.
- **System packages** (MySQL server, system OpenSSL): tracked by the OS package manager; out of scope for this repo but worth keeping current on the server itself.

## Logs

- Server logs (`Server.log`) contain GM commands, IPs, chat. They're sensitive. Don't paste full log excerpts into public PRs — redact IPs and account names, or paste only the lines relevant to the change.
- `LevelUp.log` is read by the discord-bot to post levelup notifications. It's a public-facing data source (the announcement is public). Account IDs in this log map to characters via the `characters.characters` table — that mapping is what makes IDs sensitive.
- Logs are rotated by `logrotate` (configured at the OS level on the live server). Don't commit log files; `.gitignore` covers `*.log` already.

## Backups

- `scripts/backup-db.sh` produces gzipped SQL dumps in `backups/` (gitignored). They contain everything in the DB — including `realmd.account` (password verifiers, emails).
- Don't move backup files into the repo. Don't paste a backup file into Discord, a gist, or any third-party tool.
- Backup retention is 14 days. Older backups are deleted by the script. If you need long-term retention, configure off-host backup separately.

## When you find a vulnerability

If you find a vulnerability — in the dashboard, the auth flow, a game server packet handler, or a dependency:

1. **Don't post it in a public issue.** Open a private channel with the maintainer (jeff-arn directly, or Discord DM).
2. **Don't commit the fix with a message that explains the exploit.** Land the fix first, write up the CVE-style description after deployment.
3. **Rotate any credentials that might have been exposed by the flaw.**

## Anti-patterns

- `git add .env` or `git add mangosd.conf` "just to share it with myself on the other machine"
- String-concatenating user input into SQL or shell commands
- Disabling Jinja autoescape because a value "is safe, trust me"
- Catching `Exception` and silently continuing
- Logging the full request body / form data without filtering known-sensitive fields
- Running mangosd or realmd as root. Use the `mangos` (or equivalent) unprivileged user.
- A new admin endpoint without auth, "we'll add auth later"
- Pinned but stale dependencies that haven't been audited in 6+ months
