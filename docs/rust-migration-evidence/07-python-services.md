# 07 — Python Services (Dashboard + Discord Bot): Contracts Pinning a C++→Rust Migration

Scope: `dashboard/` (Flask) and `discord-bot/` (discord.py). Neither is being migrated;
both pin contracts a Rust `mangosd`/`realmd` must not break. Read-only inspection.
All citations are `file:line` against file bodies read in this session.

## 1. Inventory

| Service | Entry point | Size | Server / runtime | DB driver (actual code) |
|---|---|---|---|---|
| Dashboard | `dashboard/dashboard.py` | 936 lines, 23 `@app.route` | waitress `serve(app, host="0.0.0.0", port=8080)` `dashboard/dashboard.py:932-936` | `mysql.connector` `dashboard/dashboard.py:10` |
| Dashboard auth | `dashboard/auth.py` | 64 lines, pure crypto, no DB/Flask `dashboard/auth.py:1-11` | — | — |
| Discord bot | `discord-bot/bot.py` | 461 lines, `discord.Client` (not cogs) `discord-bot/bot.py:33-35` | `client.run(TOKEN)` `discord-bot/bot.py:461` | `mysql.connector` `discord-bot/bot.py:8` |
| Discord helper | `discord-bot/post_patch_notes.py` | 55 lines, no DB | one-shot Discord post `discord-bot/post_patch_notes.py:19-40` | — |

Doc drift (do not trust docs over code):

- `CLAUDE.md` / `.claude/rules/database.md:101` / `.claude/rules/security.md:62` say PyMySQL;
  both services actually import `mysql.connector` (`dashboard/dashboard.py:10`,
  `discord-bot/bot.py:8`, pinned as `mysql-connector-python` in `discord-bot/requirements.txt:2`).
- `CLAUDE.md:80` and `README.md:110` say dashboard port 5000; code binds 8080
  (`dashboard/dashboard.py:936`).
- No `dashboard/requirements.txt` is tracked (`git ls-files dashboard` lists none), despite
  `.claude/rules/python.md:25-34` assuming one per service. Dashboard deps
  (flask, waitress, mysql-connector-python, werkzeug) are unpinned in-repo.
- `README.md:32,74,92` still describes a Python `bot-director/` service; no such directory
  exists — BotDirector is C++ (`docs/ARCHITECTURE.md:244-246`). Stale doc only.

Tests: `dashboard/tests/` (conftest, test_api_shapes, test_auth, test_helpers,
test_integration) and `discord-bot/tests/` (conftest, test_commands, test_helpers,
test_levelup_parser). Both conftests inject dummy `DB_PASSWORD` (and `DISCORD_BOT_TOKEN`)
because both modules read `os.environ["DB_PASSWORD"]` at import and would raise `KeyError`
(`dashboard/tests/conftest.py:1-9`, `discord-bot/tests/conftest.py:1-8`).

## 2. Dashboard routes — complete list (23)

HTML (Jinja `render_template`):

| Route | Handler | Auth |
|---|---|---|
| `GET /` | `dashboard/dashboard.py:170-172` | none |
| `GET|POST /login` | `dashboard/dashboard.py:471-499` | — |
| `GET /logout` | `dashboard/dashboard.py:502-505` | session clear |
| `GET /guild` | `dashboard/dashboard.py:531-539` | `login_required` |
| `GET /addons` | `dashboard/dashboard.py:794-797` | `login_required` |
| `GET /addons/<addon_id>` | `dashboard/dashboard.py:800-809` | `login_required` |
| `GET /addons/download/<filename>` | `dashboard/dashboard.py:925-929` (`send_from_directory`) | `login_required` |

JSON:

| Route | Handler | Auth | Success shape (keys) |
|---|---|---|---|
| `GET /api/status` | `:175-222` | none | `{online, maintenance, uptime_seconds, uptime_display, players_online, bots_online}` (+`error` only in fallback) |
| `GET /api/players` | `:225-258` | none | list of `{name, level, race, class, class_color, zone, faction, money, played_time, account}` |
| `GET /api/activity` | `:261-312` | none | list of `{timestamp, name, class, class_color, level}` (≤10) |
| `GET /api/auctions` | `:315-343` | none | `{total, by_quality:[{name, color, count}]}` |
| `GET /api/raids` | `:346-366` | none | list of `{name, phase, unlocked}` |
| `GET /api/bots` | `:369-389` | none | `{total_online, by_zone:[{zone, count}]}` |
| `GET /api/guild/roster` | `:542-586` | **none** (no decorator) | list of `{guid, name, race, class, class_id, class_color, level, spec, role, gear_score, gear_tier, raid_assigned, active, guild_name, guild_rank, faction}` |
| `GET /api/guild/bot/<int:guid>` | `:589-654` | **none** | bot fields + `gear:[{slot, item_id, name, quality, quality_color, item_level}]` |
| `POST /api/guild/bot/<int:guid>/spec` | `:657-699` | `guild_write_required` | `{"ok": true}`; errors `{"error": …}` 400/401/403/404/500 |
| `POST /api/guild/raid-plan` | `:702-727` | `guild_write_required` | `{"ok": true}` |
| `POST /api/guild/gear-tier` | `:730-760` | `guild_write_required` | `{"ok": true, "updated": N}` |
| `GET /api/addons` | `:812-830` | none | list of `{id, name, description, author, latest_version, latest_date, release_count}` |
| `GET /api/addons/<addon_id>` | `:833-839` | none | full addon object incl. `releases[]` |
| `POST /api/addons` | `:842-864` | `admin_required` (gmlevel ≥ 2) | addon object, 201 |
| `POST /api/addons/<addon_id>/releases` | `:867-903` | `admin_required` | `{version, changelog, filename, date}`, 201; multipart zip upload |
| `DELETE /api/addons/<addon_id>` | `:906-922` | `admin_required` | `{"ok": true}` |

Templates consume exactly these `/api/*` paths (grep of `dashboard/templates/*.html`
returns status/players/activity/auctions/raids/bots/guild-roster/guild-bot/gear-tier/
raid-plan/addons), so a shape break is user-visible in the bundled UI, not only external.

Discord bot "routes": message commands `!status` (`discord-bot/bot.py:398-435`) and
`!patchnotes` (`:437-453`), plus 4 background loops started once in `on_ready`
(`:387-390`): `maintenance_check_loop` 15 s, `server_status_loop` 60 s
(`STATUS_CHECK_INTERVAL :29`), `player_activity_loop` 30 s (`:30`),
`levelup_log_watcher` 5 s (`:300`). Hard-coded channel IDs `:13-15`.

## 3. Which JSON shapes are "shape-stable"?

Rule: "Endpoints in `dashboard/dashboard.py` listed in docs/ARCHITECTURE.md cannot
change response shapes without a versioned route. Adding fields is fine; renaming or
removing them is not" (`CLAUDE.md:21`; repeated `.claude/rules/python.md:72`).

`docs/ARCHITECTURE.md:331-352` lists: `GET /`, `GET /guild`, `/api/status`,
`/api/players`, **`/api/characters`, `/api/items`, `/api/guilds`** (bold = NOT in code —
doc drift), `/api/raids`, `/api/guild/roster`, `/api/guild/bot/<guid>`,
`POST …/bot/<guid>/spec`, `POST /api/guild/raid-plan`, `GET /addons`,
`GET /addons/<id>`, `GET /api/addons`, `POST /api/addons`,
`POST /api/addons/<id>/releases`.

Precisely:

- **Declared shape-stable (listed ∩ implemented):** `/api/status`, `/api/players`,
  `/api/raids`, `/api/guild/roster`, `/api/guild/bot/<guid>` (+ its POST spec),
  `POST /api/guild/raid-plan`, `GET|POST /api/addons`,
  `POST /api/addons/<id>/releases`. Additive-only; rename/remove ⇒ `/api/v2/...`.
- **Declared but nonexistent:** `/api/characters`, `/api/items`, `/api/guilds` have no
  handler. A Rust migration owes them nothing; the doc needs fixing.
- **Implemented but unlisted in ARCHITECTURE** (not formally locked by the CLAUDE rule,
  but locked de facto by tests/templates — treat as stable): `/api/activity`,
  `/api/auctions`, `/api/bots`, `POST /api/guild/gear-tier`, `GET /api/addons/<id>`,
  `DELETE /api/addons/<id>`, `/login`, `/logout`, `/addons/download/<filename>`.

Second, harder lock: `dashboard/tests/test_api_shapes.py:1-13` asserts exact key sets +
types for status (`:95-111`), players (`:139-157`), raids (`:183-184`), auctions
(`:199-203`), bots (`:215-218`), activity (`:250-251`), addons (`:378-381`), and the
validation/status-code contract of the three guild POSTs (`:264-344`). Any Rust work that
changes DB semantics feeding these fields fails these tests if key/type drift results.
Note the quirk: several read handlers swallow all exceptions and return 200 with
`[]`/`{total:0,…}` (`dashboard.py:257-258, 311-312, 365-366, 388-389`), so DB breakage
from a migration would present as "empty dashboard", not errors — shape tests pass while
data silently vanishes. Migration validation needs a live-DB smoke test, not just shapes.

## 4. MySQL usage

### Connection pattern (both services)

- Config read once at import from env: `DB_HOST` (default 127.0.0.1), `DB_USER`
  (default `mangos`), `DB_PASSWORD` (**required**, `os.environ["DB_PASSWORD"]` raises
  if unset), `DB_PORT` (3306) — `dashboard/dashboard.py:21-26`,
  `discord-bot/bot.py:17-22`, key names in `.env.example:7-11`. Values never inspected.
- Fresh connection per operation, no pool: `get_db(database)` copies config, sets
  database, `mysql.connector.connect(**config)` (`dashboard.py:129-132`;
  discord equivalent `bot.py:58-62`). Open-query-close in every handler.
- Raw SQL strings only — no ORM. All user-reachable values use `%s` parameters
  (e.g. `dashboard.py:143, 405-406, 594-603, 746-748`). Only string-interpolated SQL is
  built from allow-listed fragments: fixed column names in the spec UPDATE
  (`:670-689`, values still parameterized, presence checked against literals at
  `:666-667, :680-681`) and `%s`-placeholder expansion for `IN (...)` (`:717-721`).
- Databases used: `characters` (most), `mangos` (world), `realmd` (auth). `logs` DB is
  never touched by either Python service. Cross-schema joins are used and therefore pin
  all three schema names on one MySQL instance: `characters ⋈ realmd.account`
  (`dashboard.py:234`, `bot.py:73`), `characters.auction ⋈ mangos.item_template`
  (`dashboard.py:325`), `character_inventory ⋈ mangos.item_template` (`dashboard.py:629`).
  A Rust server must keep the 3-database layout (or provide same-named schemas/views).

### Reads — exact tables/columns

Dashboard:

| DB.table | Columns read | Where |
|---|---|---|
| `mangos.area_template` | `name` WHERE `entry=%s` (zone-id fallback, cached in-process) | `dashboard.py:141-149` |
| `characters.characters` | COUNT WHERE `online=1 AND account<100` / `>=100`; `guid,name,level,race,class,zone,money,played_time_total,account`; `zone` GROUP BY | `:194-197, :231-236, :374-382` |
| `realmd.account` | `id, username, gmlevel, v, s` WHERE `username=%s` (upper-cased); joined for `username` | `:234, :404-408` |
| `characters.auction` | COUNT(*); `item_id` (join key) | `:320, :322-328` |
| `mangos.item_template` | `entry, Quality, name, ItemLevel` (capitalized column names!) | `:325, :627-629` |
| `mangos.creature_disabled` | `map` GROUP BY (raid lockout = map present) | `:351` |
| `characters.guild_member` | `guild_id, rank, guid` | `:429-435, :554` |
| `characters.guild` | `guild_id, name` | `:555` |
| `characters.guild_bot_roster` | `guid, spec, role, gear_score, gear_tier, raid_assigned, active` | `:548-557, :596-600` |
| `characters.character_inventory` | `slot, item_id, guid, bag` WHERE `bag=0 AND slot<19` (equipped only) | `:627-633` |

Discord bot (read-only, single query, polled every 30 s and on `!status`):
`characters.characters` `guid,name,level,race,class,online,account` ⋈ `realmd.account.id`
for `username`, WHERE `account<100` — note it fetches all human characters and filters
`online` in Python (`bot.py:65-82, :319-324, :411-412`).

### Writes — the only DB writes in either service

Dashboard writes **only** `characters.guild_bot_roster`, all behind
`guild_write_required`:

1. `UPDATE guild_bot_roster SET spec?, role?, gear_tier? WHERE guid=%s` +
   `conn.commit()` (`dashboard.py:668-697`).
2. Raid plan: `UPDATE … SET raid_assigned=0 WHERE active=1` then
   `SET raid_assigned=1 WHERE guid IN (...)`, single commit (`:710-722`).
3. `UPDATE … SET gear_tier=%s WHERE guid=%s | WHERE active=1` (`:741-758`).

There are zero `INSERT`/`DELETE` statements against MySQL anywhere (grep of both entry
points: only SELECT + the 4 UPDATE statements above; the `DELETE /api/addons` route is
file-only, §6). The Discord bot performs **zero writes** of any kind.
So: no Python code writes `characters`, inventory, money, or account player data —
`guild_bot_roster` bot metadata is the sole shared-write surface.

That write is load-bearing on the C++ side: `GuildBotMgr::LoadRoster()` SELECTs the same
columns (`vmangos/src/game/PlayerBots/GuildBotMgr.cpp:58-61`) and C++ also writes
`raid_assigned/active/spec/role/gear_score/gear_tier`
(`GuildBotMgr.cpp:480, 490, 501, 667, 691`). Schema source of truth:
`vmangos/sql/migrations/20260325140000_characters.sql:12-20`
(`role enum('tank','healer','dps')`, `spec varchar(32)`, `raid_assigned/active tinyint(1)`,
`gear_score float`, PK `guid`), plus `gear_tier ENUM('fresh_60','dungeon_geared','pre_raid_bis')`
added in `20260326100000_characters.sql:11` and `personality` in
`20260331120000_characters.sql:11-12` (personality read by C++ only, never by Python).
The dashboard re-validates the same enums in Python
(`dashboard.py:666, :680, :739`) — three-way agreement (SQL ENUM, Python, C++ string
comparisons e.g. `GuildBotMgr.cpp:87-89`) that Rust must preserve byte-for-byte.

Note the missing hot-reload path: `LoadRoster()` reads at C++ startup; whether mangosd
notices dashboard edits without restart/re-summon was not traced (see Unresolved).

## 5. Auth / session handling

- Password check is SRP6 verifier comparison against `realmd.account.v/s`:
  `authenticate_user` upper-cases username, fetches `id,username,gmlevel,v,s`
  (`dashboard.py:399-420`), then `auth._verify_srp6_password` recomputes
  `v = g^SHA1(s_LE ‖ digest) mod N` with WoW 1.12 N/g and compares integers
  (`dashboard/auth.py:16-17, 20-64`). This **pins the SRP6 storage format**:
  `s`/`v` as hex strings, SHA1("USER:PASS") identity hash, endianness shuffle matching
  C++ `SRP6::CalculateVerifier`/`AccountMgr::CalculateShaPassHash` (`auth.py:7-10`).
  A Rust realmd that changes how `v`/`s` are computed or stored silently breaks every
  dashboard login (auth.py:9 says exactly this; test vectors in
  `dashboard/tests/test_auth.py`, referenced `auth.py:10`).
- On failure: `None` → generic "Invalid username or password." (`dashboard.py:496-497`);
  all exceptions also collapse to `None` (`:419-420`) — indistinguishable from DB-down.
- Session is Flask's signed cookie: `app.secret_key = FLASK_SECRET_KEY or os.urandom(32)`
  (`:16`) — ephemeral per process if env unset (restart logs everyone out; documented
  `.env.example` FLASK section and `.claude/rules/python.md:65`). Session keys set at
  login: `account_id, username, gmlevel, guild_id, guild_rank, char_name`
  (`dashboard.py:479-492`); guild context from first `characters ⋈ guild_member` row
  (`:423-441`). `logout` = `session.clear()` (`:504`). No expiry/rotation config.
- Authorization tiers (all enforced from session-cached values, stale until re-login):
  `login_required` — any `account_id` (`:444-453`, redirects to `/login?next=…`);
  `guild_write_required` — `guild_rank ≤ 1` else 403, unauthenticated 401 JSON
  (`:456-468`); `admin_required` — `gmlevel ≥ 2` (SEC_MODERATOR) (`:780-791`).
  This pins `guild_member.rank` semantics (0=GM, 1=officer) and `account.gmlevel`
  thresholds as a cross-language contract.
- Gaps worth noting for migration parity reports (not Rust's to fix): public read APIs
  (`/api/guild/roster`, `/api/guild/bot/<guid>`, `/api/players`, `/api/addons`) have no
  auth; the `next` redirect at `:494-495` is unsanitized. Discord side: identity is a
  single `DISCORD_BOT_TOKEN` env var, empty token exits at `bot.py:458-460` — but note
  `os.environ.get("DISCORD_BOT_TOKEN", "")` at `:11` while `post_patch_notes.py:15`
  hard-requires it; `!patchnotes` is available to any Discord user (no role check,
  `bot.py:437-453`) whereas `!status` is read-only.

## 6. Direct interaction with mangosd/realmd beyond MySQL

There is **no** socket, SOAP, RA (remote access), telnet, HTTP, or packet interaction
with either C++ binary: grep for `socket|SOAP|telnet|requests.|urllib|8085|3724` over
`dashboard/*.py discord-bot/*.py` returns nothing. The coupling is exactly three
host-local side channels:

1. **Process name `mangosd`** — both services shell out to `pgrep mangosd`
   (`dashboard.py:179`, `bot.py:52`); dashboard also `ps -o etimes= -p <pid>` for uptime
   (`dashboard.py:185`). *A Rust binary must literally be invocable/discoverable as a
   process named `mangosd`*, or status endpoints report offline forever. `realmd` is
   never probed (only its DB).
2. **`LevelUp.log` file format** — env `LEVELUP_LOG`, default
   `/home/wow/wow-private-server/vmangos/run/bin/LevelUp.log` (`dashboard.py:28`,
   `bot.py:24`). Dashboard reads whole file, last 10 matches, regex
   `(\d{4}-…).*Character (\w+):\d+ \[(c\d+) (r\d+)\] reaches level\s+(\d+)`
   (`dashboard.py:264-269`); bot tails it every 5 s with the same pattern minus timestamp
   (`bot.py:226, :229-300`); both skip lines containing `<BOT>` (`dashboard.py:295`,
   `bot.py:258`). Writer (C++): `sLog.Player(..., LOG_LEVELUP, "Character %s:%u [c%u r%u] reaches level %2u, ...")`
   at `vmangos/src/game/Objects/Player.cpp:3152`, file opened as `LogFile.LevelUp` →
   `LevelUp.log` in `vmangos/src/shared/Log.cpp:110`; `<BOT>` is the session remote-IP
   placeholder set in `vmangos/src/game/Server/WorldSession.cpp:73`. *Rust must emit the
   identical line shape, timestamp prefix, `[c<class> r<race>]` codes, `<BOT>` marker,
   and default path/name* — or update both Python parsers + their tests
   (`dashboard/tests/test_api_shapes.py:234-251`,
   `discord-bot/tests/test_levelup_parser.py`).
3. **Maintenance flag file `/tmp/azeroth-maintenance`** — existence only
   (`dashboard.py:29, :177`; `bot.py:26, :44-46`, polls 15 s). Created/removed by
   `scripts/maintenance.sh:13` which also rewrites `Motd =` in `mangosd.conf` (effective
   next start). Policy: while flag exists, bot suppresses offline/online and
   login/logout announcements (`bot.py:196, :315-317`) and dashboard reports
   `maintenance: true`. A Rust server need not read the flag itself, but migration
   downtime semantics depend on keeping the file contract.

Dashboard-local file state (not C++-facing): addon registry
`dashboard/static/addons/addons.json` (currently `[]`) read/written by
`_load_addons/_save_addons` (`dashboard.py:768-777`); release zips saved as
`<addon_id>_v<version>.zip` via `secure_filename` (`:884-891`) and deleted on addon
DELETE (`:914-918`). No locking — noted only because it is the dashboard's only
non-DB write besides sessions.

## 7. systemd units

**No `.service` (or `.timer`) unit files exist anywhere in the repo** (`find . -name
"*.service"` outside `.git` = empty). Units are referenced by name only:
`azeroth-dashboard.service`, `azeroth-discord-bot.service`, `azeroth-realmd.service`,
`azeroth-mangosd.service` with entry points `dashboard/dashboard.py`,
`discord-bot/bot.py`, `vmangos/run/bin/realmd` (3724), `vmangos/run/bin/mangosd` (8085)
(`docs/ARCHITECTURE.md:44-51`), and secrets via
`EnvironmentFile=/home/wow/wow-private-server/.env` for the Python services
(`docs/ARCHITECTURE.md:44`, `.claude/rules/python.md:38`). ExecStart/working-dir/restart
policy/User= are therefore **unverifiable from the repo** — the unit contents live only
on the production host. Migration implication: swapping the Rust binary at
`vmangos/run/bin/mangosd` path (and keeping argv0/pgrep name `mangosd`) is the
zero-touch option for all four units; renaming requires host-side unit edits nobody can
review from this repo.

## 8. The precise cross-language contract (must-not-break list)

A Rust `realmd` + `mangosd` can replace the C++ pair with zero Python changes iff it
preserves **all** of the following. "W" = Python writes it, C++/Rust reads it (highest
risk); "R" = Python only reads.

### 8.1 Tables and columns

| DB.table.column(s) | Dir | Semantics pinned |
|---|---|---|
| `characters.guild_bot_roster.guid, spec, role, raid_assigned, active, gear_tier` | **W** | dashboard UPDATEs (`dashboard.py:688,713,719,746,751`); C++ LoadRoster + updates (`GuildBotMgr.cpp:58,480,490,501,691`); enums §8.2 |
| `characters.guild_bot_roster.gear_score` | R (C++ writes :667) | float, displayed rounded 1dp (`dashboard.py:575,620`) |
| `characters.characters.guid, account, name, race, class, level, zone, money, played_time_total, online` | R | `online=1` must mean "currently in world" and be kept current by the server; bots too (guild/crowd/dynamic bots set online like players) |
| `realmd.account.id, username, gmlevel, v, s` | R | username stored/compared upper-case; SRP6 format §5; `gmlevel≥2` = admin |
| `characters.guild_member.guid, guild_id, rank` | R | rank 0/1 gate all dashboard writes |
| `characters.guild.guild_id, name` | R | display only |
| `characters.character_inventory.guid, bag, slot, item_id` | R | `bag=0 AND slot<19` = equipped; slot numbering 0-18 = Head…Tabard (`dashboard.py:508-528`) |
| `characters.auction.item_id` (+ row existence) | R | count + quality breakdown |
| `mangos.item_template.entry, name, Quality, ItemLevel` | R | exact column capitalization (`Quality`, `ItemLevel`) |
| `mangos.area_template.entry, name` | R | zone-id → name fallback (also hard-coded subset `dashboard.py:58-114`) |
| `mangos.creature_disabled.map` | R | presence of any row for map ∈ {409,249,469,309,509,531,533} ⇒ raid "locked" (`dashboard.py:116-124, 351-362`) |

Join keys pinned transitively: `characters.account = realmd.account.id`,
`guild_bot_roster.guid = characters.guid`, `guild_member.guid = characters.guid`,
`character_inventory.item_id = auction.item_id = mangos.item_template.entry`.

### 8.2 Value domains / sentinels

- **Human vs bot discriminator: `account < 100` human, `>= 100` bot** — hard-coded in
  dashboard (`:194-197, :235, :374`), discord bot (`bot.py:74`), docs
  (`docs/ARCHITECTURE.md:353`). Note special accounts below 100 (97 crowd, 98 guild,
  99 AH per ARCHITECTURE) are therefore counted/pluralized inconsistently with the
  `>=100` rule — preserve behavior, don't "fix" silently during migration.
- `role ∈ {tank, healer, dps}`, `gear_tier ∈ {fresh_60, dungeon_geared, pre_raid_bis}`
  (SQL ENUM + Python allow-lists + C++ string compares, §4); `spec` free-form ≤32 chars
  but de-facto the seeded names (`20260325140000_characters.sql:88-107`).
- Race IDs 1-8, class IDs {1,2,3,4,5,7,8,9,11} (no 6/10), Alliance races {1,3,4,7},
  quality 0-5 — duplicated lookup tables in both Python services
  (`dashboard.py:31-56`, `bot.py:85-143`).
- `money` in copper (÷10000 g, ÷100 s — `dashboard.py:154-158`);
  `played_time_total` in seconds (`:161-167, :252`).

### 8.3 Non-DB contracts

- Process name `mangosd` for `pgrep` (§6.1); `LevelUp.log` line format + `<BOT>` +
  default path (§6.2); `/tmp/azeroth-maintenance` flag (§6.3);
  schema names `realmd/mangos/characters` on one MySQL instance with cross-DB joins
  under a single `mangos`-user credential (§4); env keys `DB_HOST/DB_PORT/DB_USER/
  DB_PASSWORD/LEVELUP_LOG/FLASK_SECRET_KEY/DISCORD_BOT_TOKEN` (`.env.example`).
- MySQL wire compatibility itself: Python uses `mysql-connector-python`, so Rust may
  use any MySQL client internally, but the database must remain real MySQL (or a
  protocol-compatible server) — no SQLite/Postgres swap without rewriting both services.

### 8.4 What Rust may freely change (w.r.t. Python only)

Internal world simulation, opcode handling, bot AI, `guild_bot_roster.personality`
values, `guild_bot_spec_weights`, log files other than `LevelUp.log`, any table/column
not in §8.1, and adding columns/tables anywhere (all Python SELECTs name explicit
columns; no `SELECT *`). Adding JSON fields to dashboard responses is allowed by the
shape rule; renaming/removing is not (`CLAUDE.md:21`).

## 9. Rust-migration implications (this scope only)

1. The Python layer is a **thin, read-mostly shell over MySQL + 3 file/process signals**.
   It is the easiest possible migration neighbour: keep MySQL schema/semantics and it
   never notices the swap. The risk rank is: (1) `guild_bot_roster` shared-write enums,
   (2) SRP6 `v/s` format (breaks logins silently — reuse `dashboard/tests/test_auth.py`
   vectors against Rust-written rows), (3) `characters.online` freshness & account-100
   boundary, (4) `LevelUp.log` byte format, (5) binary/process name `mangosd`.
2. Failure modes are silent (exception-swallowing handlers return empty 200s; auth
   failure == DB failure). Budget a parallel-run comparison: same DB snapshot, hit all
   16 JSON routes + `!status`, diff payloads, and replay a level-up — plus the existing
   suites (`dashboard/tests`, `discord-bot/tests`, all DB-mocked, so they prove shape
   not data).
3. Don't inherit the doc drift into the migration guide: ARCHITECTURE lists 3
   nonexistent endpoints, CLAUDE/README give the wrong port (5000 vs real 8080) and
   wrong driver (PyMySQL vs mysql-connector), and there is no dashboard
   `requirements.txt` to reproduce its environment from the repo alone.
4. If Rust renames the world binary or splits DBs, the blast radius is exactly:
   `pgrep` in 2 files, 3 cross-schema joins, 1 log path/format, 1 flag file — small,
   enumerable, and listed in §6/§8. If Rust keeps names, blast radius is zero Python
   changes.

## 10. Unresolved / not inspected

- systemd unit file *contents* (ExecStart, User, Restart) — exist only on prod host, §7.
- Whether C++ (`GuildBotMgr::LoadRoster`) re-reads `guild_bot_roster` after startup
  (i.e. dashboard edits may need re-summon/restart) — reload triggers not traced.
- Exact `sLog.Player` prefix (timestamp/IP rendering incl. where `<BOT>` lands in the
  line) — inferred from `Player.cpp:3152` + `WorldSession.cpp:73`, not from a live
  `LevelUp.log` sample (file not present/read in this workspace).
- Where `characters.online` is set/cleared in C++ save path (not pinned to a line here;
  Python only proves the consumer side) — the DB-contract child should cite the writer.
- `dashboard/tests/test_integration.py` contents and template HTML/JS field usage beyond
  API-path grep.
- Real `.env`/conf values and live DB contents intentionally not read (read-only/secret
  hygiene).


