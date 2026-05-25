# Database rules

The server uses MySQL 5.7+ with four databases. Real characters exist on the live server — phase is `production` and the discipline below reflects that.

## Databases

| DB | Source of truth | Migrations |
|---|---|---|
| `realmd` | account credentials, realmlist, banlist | `vmangos/sql/logon.sql` (initial), `vmangos/sql/migrations/` (additions) |
| `mangos` | world content (creatures, gameobjects, quests, AH listings, scripts) | initial dump pulled separately (gitignored), `vmangos/sql/migrations/` for additions |
| `characters` | player characters, inventory, guilds, mail, social | `vmangos/sql/characters.sql` (initial), `vmangos/sql/migrations/` |
| `logs` | event logging (GM commands, levelups, etc.) | `vmangos/sql/logs.sql` |

Production access:

- Host: `127.0.0.1:3306`
- User: `mangos`
- Password: read from `.env` (gitignored)

The mangosd / realmd binaries get their credentials from `mangosd.conf` / `realmd.conf` (also gitignored). The `.example` templates are tracked.

## Migration discipline (production phase)

**Migrations are append-only.** A migration that has shipped to the live server must never be edited or deleted. If you need to undo something a migration did, write a new migration that does the undo.

### Workflow

1. Create a new migration:
   ```bash
   python3 vmangos/sql/make_migration.py "add personality column to guild_bot_roster"
   # Creates: vmangos/sql/migrations/<timestamp>_add_personality_column_to_guild_bot_roster.sql
   ```
   (Or `./vmangos/sql/touch_migration.sh` for a blank template if the script feels heavy.)

2. Edit the file. Write idempotent SQL — runnable multiple times without error.

3. Apply locally to verify:
   ```bash
   mysql -u mangos -p characters < vmangos/sql/migrations/<timestamp>_*.sql
   ```

4. Commit the migration alongside the code that needs it. Migration + code go in the same commit (or the same PR) so deployments don't get out of order.

### Idempotency

Every migration must be idempotent. The patterns:

```sql
-- Adding a column: guarded
ALTER TABLE guild_bot_roster
  ADD COLUMN IF NOT EXISTS personality VARCHAR(32) NOT NULL DEFAULT 'chill';

-- Creating an index: drop-then-create or check first
CREATE INDEX IF NOT EXISTS idx_personality ON guild_bot_roster(personality);

-- Inserting reference data: ON DUPLICATE KEY UPDATE or INSERT IGNORE
INSERT INTO guild_bot_spec_weights (class, spec, stat, weight)
VALUES (1, 'prot', 'stamina', 2.5)
ON DUPLICATE KEY UPDATE weight = VALUES(weight);
```

A migration that errors on re-run is a migration that costs hours during an incident. Write them so that running them twice is a no-op.

### What you cannot do in production

- `DROP TABLE` on any table holding player data (`characters.character_inventory`, `characters.item_instance`, `characters.character_*`, `realmd.account`, etc.)
- `DROP COLUMN` on player-data tables
- Narrowing a column type (`VARCHAR(64) → VARCHAR(32)`, `BIGINT → INT`) — data truncation
- Adding a `NOT NULL` column **without a default**. Adding `NOT NULL DEFAULT '...'` is fine.
- Renaming a column (consumers may be in flight). Add the new name, dual-write, deprecate later.
- Changing a primary key.

When you genuinely need to do one of these, talk through the plan first — there's no shortcut.

### `custom/` vs `migrations/`

- `vmangos/sql/migrations/` — timestamped, ordered, applied once. The forward-only stream.
- `vmangos/sql/custom/` — reusable SQL snippets, helpers, or content additions that get pulled into world DB reloads. Not part of the migration stream.

If you're adding ongoing schema or content evolution → `migrations/`. If you're adding a snippet that gets sourced into world DB rebuilds (like a custom item or NPC) → `custom/`.

## Schema conventions

- Table names: lowercase, words separated by underscores, plural where it makes sense (`guild_bot_roster`, `character_inventory`, `item_instance`).
- Primary keys: existing tables follow MaNGOS conventions — sometimes `guid`, sometimes `id`, sometimes a composite. Match the surrounding table family.
- Booleans: `TINYINT(1)` or `TINYINT UNSIGNED`. MySQL `BOOLEAN` is an alias.
- Timestamps: `INT UNSIGNED` for Unix-epoch seconds (legacy MaNGOS pattern) or `DATETIME` for new tables. Don't mix in one table.
- Strings: `VARCHAR(...)` with a length that reflects the actual constraint. `TEXT` only when the content is genuinely unbounded.
- Charset: `utf8mb4` for any new table. Existing tables may use older `utf8` (which is 3-byte) — don't migrate them as a side effect.

## Foreign keys

MaNGOS-era schemas mostly don't use foreign key constraints. The application maintains referential integrity. Don't add FK constraints to legacy tables without checking that the application doesn't already write rows in an order that would violate them.

New tables with self-contained relationships (e.g. `guild_bot_roster` ↔ `guild_bot_spec_weights`) can use FKs if it helps. Use `ON DELETE CASCADE` carefully — players have lost data that way.

## Connection pooling and async

- Game server queries go through `CharacterDatabase` / `WorldDatabase` / `LoginDatabase` (in `vmangos/src/`). Use prepared statements where available.
- `AsyncQuery` callbacks run on a background thread — do not access game objects directly from there. The pattern is: async query → callback enqueues a world-thread task → world tick processes it.
- Python services (dashboard, discord-bot) use PyMySQL with a fresh connection per request (Flask) or a cached connection per cog (discord.py). Don't share a single connection across threads.

## Credentials

- `.env` holds DB credentials for Python services. Gitignored. Use `.env.example` (tracked) as the template.
- `mangosd.conf` / `realmd.conf` hold DB credentials for the C++ services. Gitignored. Use `*.conf.example` (tracked) as templates.
- Never embed a DB password in any tracked file. Never log a DB password.
- Don't paste a real password into a commit message. The pre-commit hook runs `gitleaks` to catch this; don't trust it as the only line of defense — read your diff.

## Backups

- `scripts/backup-db.sh` runs nightly at 04:00 UTC via cron, with 14-day retention.
- Output goes to `backups/` (gitignored).
- A migration that destroys data without an explicit "we already have a backup of this" comment is wrong.

## DB-related anti-patterns

- Editing an already-shipped migration to fix a bug. Add a new migration instead.
- `DROP TABLE characters.<anything>`. Just don't.
- `TRUNCATE` on a player-data table in a migration. (Truncating world content tables is sometimes legitimate — they're rebuilt from the world DB dump.)
- `ALTER TABLE ... CHANGE COLUMN <old> <new> ...` (renames). Add the new column, dual-write, drop later.
- Schema changes from inside Python services. All schema changes go through `vmangos/sql/migrations/`.
- Hardcoded DB credentials anywhere in the repo.
