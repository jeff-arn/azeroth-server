# 04 — Persistence Layer: Database Evidence Base for C++→Rust Migration

Workspace: `/Users/jeffarn/Programming/azeroth-server` (VMaNGOS WoW 1.12.1, C++14, production).
Method: read-only inspection of file bodies; counts via `grep` over `vmangos/src` (method noted per count).
No secrets are reproduced; config values cited only from committed `.example`/`.dist` templates.

## 1. Prior decisions recorded in project memory (scope `personal_project`)

Read via `read_memory` on 2026-10-15 session; these are prior-agent notes, not repo source of truth.

### 1.1 `rust-migration/dev-mysql.md`
- Local dev MySQL via Homebrew (`brew install mysql`, launchd, port 3306, server v26.x);
  schemas from `vmangos/sql/*.sql` load clean despite the newer server version.
- Dev databases: `realmd` (from `logon.sql`) and `characters` (from `characters.sql` **plus all
  `*_characters.sql` migrations applied in timestamp order**, required for e.g. `character_flags`).
- `TEST_MYSQL_URL='mysql://mangos:<pw>@localhost:3306/realmd'` (or `/characters`) gates live tests;
  `REALMD_DATABASE_URL` of the same shape selects the MySQL store in the Rust realmd binary.
  Dev password is deliberately **not** recorded in memory.
- `realmd` live test (`mangos-realmd-mysql -- --ignored`) self-seeds and passed; `characters` live
  tests need one synthetic row (guid=1/account=1/name=RUSTTEST) with non-null float/int columns
  zeroed and `known_taxi_mask`/`explored_zones`/`current_taxi_path` set to empty (loader rejects NULLs).
- Proven live 2026-09-27: full DB-backed SRP logon + realm-list-from-`realmlist` via
  `/tmp/verify_realmd_dblogon.py` 11/11.
- Naming gotcha: `target/debug/mangos-realmd` is a stale pre-rename artifact; live binary is
  `mangos-realmd-server` (package `mangos-realmd-server`). `mangos-mangosd-server` still emits
  binary `mangos-mangosd` via `[[bin]]`.

### 1.2 `migration-discipline.md`
- Before porting any C++ seam, survey existing crates first (`ls vmangos-rs/crates` +
  `grep -rl "<C++ symbol>" vmangos-rs/crates/*/src/lib.rs`); workspace already holds ~190 crates.
- Duplicating a seam in a new crate is worse than not porting; if the seam exists and is tested,
  leave it; if wrong, fix in place. Named overlaps: melee hit-table, spell-crit vs `Unit.cpp`/`SpellCaster.cpp`.
- **Note for this report:** at inspection time `vmangos-rs/crates` did **not** exist in this checkout
  (`ls vmangos-rs/crates` → 0 entries / path absent). The Rust workspace referenced by memory is
  either on another branch/machine or not yet committed here. Rust-side DB crates therefore could
  not be inventoried from this tree — see §10 Unresolved.

### 1.3 `adapter-selection.md`
- Detect missing adapters with `ls crates/ | sed 's/-adapter$//' | sort | uniq -c | awk '$1==1'`
  (count 1 = no adapter dir). Never use `ls | grep <name> | grep -v adapter` (caused corpse +
  talent rewrites). Before writing a new adapter dir, check
  `git log --oneline HEAD -- vmangos-rs/crates/<name>-adapter` for prior work.

Implication: prior Rust work already targets the **same MySQL schemas over the wire**
(`mysql://…/realmd`, `…/characters`) rather than a new store, and live tests are gated on
`TEST_MYSQL_URL` with `--ignored`. A Rust persistence port must reuse, not duplicate, those crates.

## 2. The four databases and process ownership

Globals are declared in [DatabaseEnv.h:51-54](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/DatabaseEnv.h:51):
`WorldDatabase`, `CharacterDatabase`, `LoginDatabase`, `LogsDatabase`, all of type
`DatabaseType` = `DatabaseMysql` (or `DatabasePostgre` under `DO_POSTGRESQL`, DatabaseEnv.h:29-43).

| Logical handle | Default DB name | Owner process(es) | Content |
|---|---|---|---|
| `WorldDatabase` | `mangos` (world) | `mangosd` only | Static world content: templates, spawns, loot, quests, locales |
| `CharacterDatabase` | `characters` | `mangosd` only | Player-owned mutable state: characters, items, guilds, mail, AH |
| `LoginDatabase` | `realmd` (logon) | **both** `realmd` and `mangosd` | Accounts (SRP `v`/`s`), bans, `realmlist`, `realmcharacters` |
| `LogsDatabase` | `logs` | `mangosd` only | Anticheat/warden fingerprints, GM/log tables |

Evidence:
- `mangosd` defines all four globals ([Main.cpp:54-57](/Users/jeffarn/Programming/azeroth-server/vmangos/src/mangosd/Main.cpp:54))
  and opens all four in `Master::_StartDB()` ([Master.cpp:469](/Users/jeffarn/Programming/azeroth-server/vmangos/src/mangosd/Master.cpp:469)),
  each gated on `CheckRequiredMigrations(...)`; on shutdown all four get `StopServer()` (Master.cpp:346-349)
  and `AllowAsyncTransactions()` after startup (Master.cpp:190-193).
- `realmd` defines only `LoginDatabase` ([realmd/Main.cpp:78](/Users/jeffarn/Programming/azeroth-server/vmangos/src/realmd/Main.cpp:78)),
  reads `LoginDatabaseInfo` from config (Main.cpp:399), checks `MIGRATIONS_LOGON` (Main.cpp:446).
- Shared-writer hazard: `mangosd` also writes `realmd` — e.g. `Master.cpp:207` updates `realmlist`
  realmflags/population/builds, `Master.cpp:490` clears `account.current_realm`. Any Rust split
  (realmd-only vs mangosd-only) must preserve this cross-process write contract or coordinate it.
- Connection strings are semicolon tuples `host;port;user;password;database` parsed in
  [Database.cpp:89-120](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.cpp:89)
  (`"."` host means localhost+socket). Template example: `realmd.conf.dist.in:173`
  `LoginDatabaseInfo = "127.0.0.1;3306;mangos;mangos;realmd"` (placeholder credentials only).

CLAUDE.md names three databases (`realmd`, `mangos`, `characters`) "plus `logs`" — four total,
matching the code. The world DB's *content dumps are not in this repo*: CLAUDE.md:129 and
`docs/ARCHITECTURE.md:39` place them in gitignored `vmangos-database/`. Only its *migrations*
(§6) live here. A Rust world loader cannot be validated from this checkout alone.

## 3. C++ database layer architecture

Files under `vmangos/src/shared/Database/` (31 files): `Database.{h,cpp}`,
`DatabaseMysql.{h,cpp}`, `DatabasePostgre.{h,cpp}`, `Field`, `QueryResult{,Mysql,Postgre}`,
`SqlOperations`, `SqlDelayThread`, `SqlPreparedStatement`, `SQLStorage{,Impl.h}`,
`DatabaseCallback.h`, `DatabaseImpl.h`, `DBCFileLoader`, `DBCStore.h`.
Game-side: `vmangos/src/game/Database/` holds `SQLStorages`, `DBCStores/DBCStructure/DBCfmt`,
`CharacterDatabaseCache`, `CharacterDatabaseCleaner`.

### 3.1 Abstraction shape
- `SqlConnection` abstract base ([Database.h:47-124](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.h:47)):
  `Query/QueryNamed/Execute`, `escape_string`, `Begin/Commit/RollbackTransaction`,
  `ExecuteStmt`, per-connection prepared-statement holder, `recursive_mutex` via RAII `Lock`.
- `Database` ([Database.h:132-350](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.h:132)):
  pool of sync query connections (round-robin, `m_nQueryCounter`, pool 1–16 per
  `Database.cpp:36-37`), **one** async connection (`m_pAsyncConn`, Database.h:326),
  a `LockedQueue<SqlOperation*>` delay queue, N `SqlDelayThread` workers with per-worker
  *serial* queues, thread-local current transaction (`ThreadSpecificPtr<SqlTransaction>`), and a
  prepared-statement registry (`unordered_map<string,int>`).
- `MySQLConnection` wraps raw `MYSQL*` ([DatabaseMysql.h:78-106](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/DatabaseMysql.h:78));
  `MySqlPreparedStatement` wraps `MYSQL_STMT*`/`MYSQL_BIND*` (DatabaseMysql.h:47-76).
  PostgreSQL backend exists behind `DO_POSTGRESQL` but MySQL is the production path
  (CLAUDE.md: MySQL 5.7+). Header warns MySQL ≥8.0 is "incompatible" (DatabaseMysql.h:36-44),
  yet dev-memory §1.1 records schemas loading clean on server v26.x — driver-level, not schema-level, risk.
- Results are **positional and stringly-typed**: `Field::GetUInt32()` is `atol(mValue)`,
  `GetFloat()` is `atof` ([Field.h:47-58](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Field.h:47)).
  NULL → 0/empty. There is no column-name binding on the hot path (`QueryNamed` exists but is rare).
  Rust implication: column *order* in every SELECT is load-bearing; a typed Rust row mapper must
  reproduce C `atol/atof` coercion semantics (leading-numeric parse, NULL→0) or be stricter only
  where data is proven clean.

### 3.2 Sync vs async execution
- Sync reads: `Query/PQuery` lock a pooled connection and execute inline (Database.h:144-157).
  `PQuery` is `vsnprintf` into a 32 KiB stack buffer (`MAX_QUERY_LEN`, Database.h:42) then `Query`
  ([Database.cpp:346-364](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.cpp:346)); truncation aborts the query.
- Async writes: `Execute/PExecute` default to `DbExecMode::CanBeAsync` → enqueue `SqlPlainRequest`
  on the delay queue; `MustBeSync` or pre-`AllowAsyncTransactions()` → `DirectExecute` on the
  async connection ([Database.cpp:388-410](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.cpp:388)).
  Return value in async mode "just gives you a hint" whether enqueue succeeded (Database.h:234).
- Async reads: templated `AsyncQuery/AsyncPQuery/DelayQueryHolder` with member/static callbacks
  (Database.h:172-232), results delivered via `ProcessResultQueue` on the owning thread.
- `SqlDelayThread` ([SqlDelayThread.h:32-62](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/SqlDelayThread.h:32)):
  one thread body draining `m_sqlQueue` plus a `m_serialDelayQueue`; serial operations
  (keyed by e.g. character GUID-low, see §4) preserve per-entity ordering across workers.
- Transactions are **deferred-queue transactions**, not immediate `START TRANSACTION` on the
  caller thread: `BeginTransaction(serialId)` creates a thread-local `SqlTransaction`;
  subsequent `Execute` calls append to it (Database.cpp:393-397); `CommitTransaction()` enqueues
  the bundle (direct variant `CommitTransactionDirect()` exists, Database.h:249). Nested
  transactions assert-fail (Database.cpp:477). Rust must preserve: bundle atomicity on the delay
  thread, per-serialId ordering, and the fire-and-forget error model (callers rarely observe failure).

### 3.3 String-format SQL vs prepared statements — prevalence

Counts (`grep` over `vmangos/src`, `*.cpp` unless noted):

| Call shape | Count | Notes |
|---|---:|---|
| `<Db>.PExecute` (printf) | 361 | CharDB 250, World 68, Login 37, Logs 6 |
| `<Db>.PQuery` (printf) | 202 | World 87, CharDB 79, Login 36 |
| `<Db>.Query` (raw string) | 192 | World 128, CharDB 50, Login 11, Logs 3 |
| `<Db>.Execute` | 29 | CharDB 15, Login 7, + Direct variants below |
| `DirectExecute/DirectPExecute` | ~35 | sync escape hatch, mostly startup/shutdown |
| `AsyncPQuery/AsyncQuery` | ~19 | CharDB 14, Login 5 |
| `DelayQueryHolder` | 4 | character login path |
| **Any `<Db>.` call site (`*.cpp`)** | **1,142 lines** | `grep -rE "(World\|Character\|Login\|Logs)Database\."` |
| `PQuery\|PExecute` anywhere (`*.cpp`) | 689 lines | includes `stmt.PExecute`, log helpers |
| `escape_string` calls (`*.cpp`) | 72 lines | manual escaping before `%s` interpolation |
| `SqlStatement` mentions (`*.cpp`) | 174 lines | prepared-statement API surface |
| `CreateStatement(...)` in `game/` | ~30 sites sampled | concentrated in save paths (Item, Pet, Mail, Player) |

Two coexisting regimes:
1. **Printf SQL dominates reads and ad-hoc writes** (~563 PQuery+PExecute call sites). Safety rests
   entirely on callers remembering `escape_string` (only 72 call lines) and correct `%u/%s`
   specifiers. Classic example, realmd account lookup with pre-escaped login
   ([AuthSocket.cpp:297](/Users/jeffarn/Programming/azeroth-server/vmangos/src/realmd/AuthSocket.cpp:297)).
2. **Prepared statements dominate high-frequency save paths**: `Database::CreateStatement(id, fmt)`
   with `?` placeholders, typed `addUInt32/addString/addFloat…`, cached in `static SqlStatementID`
   locals (e.g. [Item.cpp:273](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Item.cpp:273),
   [Player.cpp:16252](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp:16252)).
   Under MySQL these become real `MYSQL_STMT` prepares per connection
   ([Database.cpp:53-87](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.cpp:53));
   the base class offers a "plain" fallback that re-formats text (`SqlPlainPreparedStatement`,
   Database.cpp:40).

Rust implication: the migration surface is not "one query builder" — it is ~1,100 call sites in
two idioms. Prepared-statement sites port 1:1 to `sqlx`/`mysql_async` binds. Printf sites need
case-by-case conversion; each `%s` site must be audited for whether escaping was actually applied
(a latent-injection census the Rust port gets forced to do — a security benefit, but schedule it).

## 4. Character save/load path (the sacred path)

Production phase (CLAUDE.md): real characters exist; save formats are locked.

### 4.1 Load: `Player::LoadFromDB`
[Player.cpp:14470](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp:14470) (file is 22,443 lines).
Login issues a `SqlQueryHolder` bundle of **23 named queries** enumerated in
[Player.h:612-635](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.h:612)
(`PLAYER_LOGIN_QUERY_LOADFROM`, `_LOADGROUP`, `_LOADBOUNDINSTANCES`, `_LOADAURAS`, `_LOADSPELLS`,
`_LOADQUESTSTATUS`, `_LOADHONORCP`, `_LOADREPUTATION`, `_LOADINVENTORY`, `_LOADITEMLOOT`,
`_LOADACTIONS`, `_LOADSOCIALLIST`, `_LOADHOMEBIND`, `_LOADSPELLCOOLDOWNS`, `_LOADGUILD`,
`_LOADBGDATA`, `_LOADACCOUNTDATA`, `_LOADSKILLS`, `_LOADMAILS`, `_LOADMAILEDITEMS`,
`_BATTLEGROUND_DATA`, `_FORGOTTEN_SKILLS`, … `MAX_PLAYER_LOGIN_QUERY`), executed async via
`DelayQueryHolder` and unpacked with `holder->TakeResult(...)` (Player.cpp:14485, 14878).

The main `characters` row is consumed **positionally, fields[0..57]** — the authoritative column
list is the comment block at Player.cpp:14472-14483 (58 columns: `guid, account, name, race,
class, gender, level, xp, money, skin, face, hair_style, hair_color, facial_hair,
bank_bag_slots, character_flags, position_x…, known_taxi_mask, …, explored_zones, ammo_id,
action_bars, world_phase_mask, create_time, instance`). Any Rust loader must select columns in
exactly this order (or map by name with an equivalence test). Load also *writes* on validation
failure (sets `CHARACTER_FLAG_RENAME`, Player.cpp:14520) — a side effect a pure-Rust reader must keep.

### 4.2 Save: `Player::SaveToDB`
[Player.cpp:16229-16405](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp:16229):
- Early-outs: bots/saving-disabled (16236), mid-teleport → delayed op (16240-16244).
- Opens `CharacterDatabase.BeginTransaction(GetGUIDLow())` (16246) — the GUID-low is the
  *serialId*, guaranteeing this character's save bundles execute in order on the delay threads.
- One 62-bind prepared `REPLACE INTO characters (...) VALUES (?,…)` "uberInsert" (16252-16389).
  `REPLACE` semantics (delete+insert on PK conflict) must be preserved; note column set differs
  slightly from the load SELECT (save writes `played_time_*`, `logout_time=now`, omits `zone` on
  load order, etc.) — byte-compat means matching each direction separately.
- Then, still inside the transaction bundle: `_SaveBGData, _SaveInventory, _SaveQuestStatus,
  _SaveSpells, _SaveSpellCooldowns, _SaveAuras, _SaveSkills`, reputation + honor saves,
  tutorials (16391-16403). `CommitTransaction()` at 16405.
- Outside the bundle: `_SaveStats()` (logout-gated) and pet save (16409-16414).
- Character *creation* uses a parallel one-shot insert at Player.cpp:16074-16197
  (`BeginTransaction(guidlow)` … `CommitTransaction()`), with a slightly different column list.

### 4.3 Space-separated text blobs inside columns (byte-compatible formats)
Several `characters` columns are **space-delimited integer strings**, not normalized tables:
- `known_taxi_mask` (`longtext`): `ss << m_taxi` — TaxiMaskSize numbers (Player.cpp:16319-16321).
- `current_taxi_path` (`text`): `SaveTaxiDestinationsToString()` (16323-16324).
- `explored_zones` (`longtext`): 64 uint32 words (`PLAYER_EXPLORED_ZONES_SIZE 64`, Player.h:70),
  written `value << " "` (16364-16366), read via `_LoadIntoDataField` (14544).
- `equipment_cache` (`longtext`): per equipment slot `itemEntry packed(ench1,ench2)` pairs
  (`MAKE_PAIR32`), plus one bag slot (16368-16382).
Dev-memory §1.1 confirms the loader **rejects NULL** for these columns — Rust must write `''`
(empty string), never NULL, and must reproduce trailing-space tokenization tolerance.

## 5. Item instance & inventory serialization

Two tables, joined application-side:
- `item_instance` — the item itself. `Item::SaveToDB` ([Item.cpp:253-370](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Item.cpp:253))
  is a state machine on `uState` (`ITEM_NEW` → prepared `REPLACE INTO item_instance (item_id,
  owner_guid, creator_guid, gift_creator_guid, count, duration, charges, flags, enchantments,
  random_property_id, durability, text, generated_loot, guid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
  `ITEM_CHANGED` → `UPDATE … WHERE guid=?`, `ITEM_REMOVED` → `DeleteAllFromDB()` + `delete this`,
  Item.cpp:273, 305-312). Loot contents spill to `item_loot` rows (gold encoded as `item_id=0`,
  Item.cpp:336).
- `character_inventory` — placement only: `(guid, bag, slot, item_guid, item_id)`.
  `Player::_SaveInventory` ([Player.cpp:16561-16675](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp:16561))
  walks the dirty-item queue, runs anti-duplication checks (deleting orphaned slot rows and
  flagging anticheat, 16609-16617), then INSERT/UPDATE/DELETEs the placement row per item state
  (16642/16653/16664) and calls `item->SaveToDB()` (16672).

Text-blob formats inside `item_instance`:
- `charges`: `MAX_ITEM_PROTO_SPELLS` ints, space-separated with trailing space (Item.cpp:281-284);
  parsed with `Tokenizer(..., ' ', MAX_ITEM_PROTO_SPELLS)` + `atoi` (Item.cpp:406-409).
- `enchantments`: per enchantment slot a triple `id duration charges`, space-separated
  (Item.cpp:288-295); parsed into data fields (Item.cpp:419-420).
- `random_property_id` is **signed** on read (`GetInt16`, Item.cpp:421) but written via
  `addUInt16` (Item.cpp:297) — preserve the bit pattern, don't "fix" the sign.
- `Item::LoadFromDB` ([Item.cpp:372-466](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Item.cpp:372))
  consumes fields positionally `[creator_guid, gift_creator_guid, count, duration, charges,
  flags, enchantments, random_property_id, durability, text, item_guid, item_id]` and performs
  *repair writes* at load (duration/durability/binding fixes, Item.cpp:452-463).
- GUIDs: DB stores only the 32-bit low counter; high-GUID type (`HIGHGUID_ITEM/PLAYER`) is
  re-synthesized in code (Item.cpp:378, 393-394). Rust types must not persist full 64-bit GUIDs
  into these columns.

## 6. Schema sources: base dumps vs migrations

### 6.1 Base dumps (`vmangos/sql/*.sql`) — structure only, no world content
| File | Lines | Tables | Engines | Target DB |
|---|---:|---:|---|---|
| `characters.sql` | 912 | 55 | 48 MyISAM / 7 InnoDB | `characters` |
| `logon.sql` | 211 | 11 | 6 MyISAM / 5 InnoDB | `realmd` |
| `logs.sql` | 241 | 15 | (mixed) | `logs` |
| *(world base dump)* | — | — | — | **absent** — external `vmangos-database/` (CLAUDE.md:129) |

Headers show a HeidiSQL dump from MySQL 5.5.53 (`characters.sql:1-11`); `CREATE TABLE IF NOT
EXISTS`, `FOREIGN_KEY_CHECKS=0`, `SQL_MODE='NO_AUTO_VALUE_ON_ZERO'`. **MyISAM majority matters**:
most `characters` tables have no transactional atomicity at the storage-engine level, so the
§3.2 "transaction" bundle is only as atomic as InnoDB tables allow; crash-mid-save behavior is
engine-dependent per table. A Rust port must not assume transactional rollback on `characters`.
Example: `characters` table itself is `ENGINE=MyISAM` (characters.sql, `characters` DDL);
`account` in realmd is InnoDB (logon.sql, `account` DDL).

### 6.2 Migrations (`vmangos/sql/migrations/`)
- **1,038 entries** in the directory (`ls | wc -l`): 1,005 `*_world.sql`, 18 `*_characters.sql`,
  6 `*_logon.sql`, 6 `*_logs.sql`, plus `README`, `merge.sh/.bat`. (Suffix counts via
  `ls | sed 's/.*_//' | sort | uniq -c`.) A further 1,436 files sit in `sql/old_migrations/`
  (pre-2021 history, same naming).
- Naming: `YYYYmmddHHMMSS_<database>.sql` in UTC, database ∈ {world, characters, logon, logs}
  (`migrations/README`). Range in `migrations/`: 2021-05-15 (`20210515110157_world.sql`) through
  future-dated 2026 bot-era files (`20260406120000_characters.sql`).
- Idempotency pattern (current): stored-procedure guard checking the `migrations` table before
  applying, then self-registration — see template in
  [make_migration.py:7-25](/Users/jeffarn/Programming/azeroth-server/vmangos/sql/make_migration.py:7)
  and live example `20260406120000_characters.sql` (creates `bot_learned_patterns` with
  `CREATE TABLE IF NOT EXISTS`). The older README describes the simpler first-line
  `INSERT INTO migrations VALUES (...)` form; both generations exist in the tree.
- `make_migration.py` (31 lines) only ever emits a `_world.sql` file (line 5 hard-codes the
  suffix) — characters/logon/logs migrations are renamed by hand. `touch_migration.sh` is the
  empty-template alternative (CLAUDE.md).
- Enforcement is **compile-time + startup-time**: CMake glob-generates `migrations_list.h`
  with one C array per database ([migrations_list.h.cmake:1-31](/Users/jeffarn/Programming/azeroth-server/vmangos/cmake/generators/migrations_list.h.cmake:1));
  at boot `Database::CheckRequiredMigrations` ([Database.cpp:570](/Users/jeffarn/Programming/azeroth-server/vmangos/src/shared/Database/Database.cpp:570))
  reads `SELECT * FROM migrations`, refuses to start on missing entries, warns on extras.
  Adding a migration therefore requires a rebuild before the server will accept (or demand) it.
- CLAUDE.md production rule: migrations are append-only; shipped migrations are never edited;
  player-data schema additions go through `vmangos/sql/migrations/` with `IF NOT EXISTS` guards.

Rust implication: the Rust binary must embed an equivalent required-migrations list (generated
from the same directory glob, ideally by `build.rs`) and perform the same startup check against
each DB's `migrations` table, or a Rust `mangosd` will boot against an under-migrated production
DB silently. The glob order and suffix parsing of the CMake file (split on last `_`) define the set.

### 6.3 Other SQL in tree
`sql/custom/` (31 files + `repack/`) holds optional content toggles (events, PvP sets, resource
rates) — operators' choice, not enforced by `CheckRequiredMigrations`. `sql/translations/`
holds locale content. Neither is part of the boot contract.

## 7. World-content loading at startup (ObjectMgr)

`ObjectMgr.cpp` is 12,038 lines. Count method: `grep -c "^void ObjectMgr::Load\|^bool ObjectMgr::Load"` → **96 Load* definitions**;
`World.cpp` invokes `sObjectMgr.Load*` at **84 call sites** during `World::SetInitialWorldSettings`
sequence ([World.cpp:1306-1838](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/World.cpp:1306));
`ObjectMgr.cpp` alone contains 101 `WorldDatabase.Query/PQuery` lines.

Representative loaders (definition lines in ObjectMgr.cpp): `LoadCreatureTemplates` :1173,
`LoadCreatures` :2275, `LoadGameobjects` :2481, `LoadItemPrototypes` :3770, `LoadPlayerInfo` :4475,
`LoadGroups` :5131, `LoadQuests` :5291, `LoadGameObjectTemplates` :7760, `LoadCorpses` :8237
(characters DB!), `LoadConditions` :11565 — plus ~86 more covering locales (9 `*Locales`
loaders), loot-adjacent data, gossip, trainers, vendors, taxi, reputation, spawns' addons,
faction-change tables, and saved variables (`LoadSavedVariable` :629, worldstate persistence).

Ordering constraints are encoded as comments in World.cpp and are part of the contract, e.g.
"must be after LoadCreatureTemplates() and LoadCreatures()" (World.cpp:1505), quests after
DBCs+templates (1532), locales after their base templates (1657-1665). Two loading mechanisms:
1. Hand-written `WorldDatabase.Query("SELECT …")` + positional `fields[N]` parsing (majority).
2. Declarative `SQLStorage` tables driven by format strings — 9 storages declared in
   [SQLStorages.cpp:39-47](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Database/SQLStorages.cpp:39)
   (`creature_addon`, `page_text`, `map_template`, `conditions`, `area_template`,
   `mail_text_template`, `pet_spell_data`, display-info addons), sharing machinery with DBC loading
   (`srcflt`/format-char layout must match struct layout in `DBCStructure.h`).

Rust implication: startup loading is ~96 table-specific parsers with cross-loader ordering and
in-memory index building; it is the single largest read-only port surface. It is also the safest
to port first (no writes), but validation requires the external world DB dump absent from this
repo (§6.1) — against an empty world DB most loaders log "0 rows" and continue, masking bugs.

## 8. Exact DB contract a Rust implementation must keep byte-compatible

A Rust server sharing production databases with (or replacing) the C++ server must preserve:

1. **Wire & names**: MySQL protocol; database names `realmd`/`mangos`(world)/`characters`/`logs`
   as configured; table/column names, types, defaults, engines exactly as base dumps + 1,035
   in-tree migrations define them. No renames, no type narrowing (CLAUDE.md production rules).
2. **Startup gate**: required-migrations set = glob of `vmangos/sql/migrations/*.sql` split by
   suffix; refuse boot when an ID is missing from that DB's `migrations` table (Database.cpp:570).
3. **Positional row contracts**: every SELECT's column order consumed via `fields[N]`, notably
   the 58-column character load (Player.cpp:14472-14483), the 12-field item load
   (Item.cpp:374-375), and ~96 ObjectMgr loader SELECTs. Column additions must append, never insert.
4. **Text-blob encodings**: space-separated decimal integers with trailing space in
   `known_taxi_mask`, `current_taxi_path`, `explored_zones` (64 words), `equipment_cache`
   (packed MAKE_PAIR32 enchant pairs), `item_instance.charges`, `item_instance.enchantments`
   (id/duration/charges triples). Empty string, never NULL, for absent data.
5. **Numeric coercion**: C `atol/atof/atoi` semantics on read (NULL/garbage → 0, prefix parse);
   signedness quirks preserved (`random_property_id` read i16 / written u16; `watched_faction`
   −1 sentinel; `gender & 0x01`). Floats stored as MySQL `float` (32-bit) — Rust must round via
   `f32`, not persist `f64` precision.
6. **GUID encoding**: only 32-bit low counters in DB columns (`guid`, `owner_guid`, …);
   high-GUID type bits are code-side. `transport_guid` is the exception column type (`bigint`)
   but still written with the low counter (Player.cpp:16310-16313).
7. **Write shapes**: `REPLACE INTO characters` full-row upsert (§4.2); `REPLACE/UPDATE`
   item_instance by state machine (§5); inventory as separate placement rows; loot gold as
   `item_id=0`; delete cascades done application-side (no FK reliance — dumps disable FK checks).
8. **Transaction/ordering model**: per-character serial ordering (serialId = GUID-low) across
   async workers; save bundle = character row + inventory/quests/spells/cooldowns/auras/skills/
   reputation/honor as one queued unit; fire-and-forget error visibility; no nested transactions;
   MyISAM tables give no rollback — do not design Rust saves that require it.
9. **Cross-process ownership**: `realmd` DB written by both processes (`realmlist` flags/
   population/builds, `account.current_realm/sessionkey/last_ip/failed_logins`, bans);
   `mangosd` startup side effects (`characters.online=0`, battleground instance reset,
   Master.cpp:492-495) and `realmd` startup ban expiry (realmd/Main.cpp:215-218) must each have
   exactly one owner in any split deployment.
10. **Timestamps**: unix-epoch integers in `bigint` columns (`logout_time`, `create_time`, …),
    `UNIX_TIMESTAMP()`/`NOW()` SQL-side in auth paths — Rust must not mix local-time formatting.
11. **Escaping**: string columns (`name`, mail text, guild MOTD, …) historically written through
    `mysql_real_escape_string`-equivalent + printf; Rust bound parameters produce the same stored
    bytes only if charset (utf8/utf8mb4 per dumps) and no-backslash-escape assumptions match.

Anything beyond this list (pool sizes, thread counts, query batching, ORM choice) is free
implementation space for Rust.

## 9. Rust-migration implications (scope-limited summary)

- **Effort split**: ~1,142 DB call sites; hot save paths (~174 prepared-statement lines) port
  mechanically to bound queries; ~563 printf sites need individual audit (escaping, specifiers,
  32 KiB truncation assumptions at Database.cpp:352).
- **Driver choice constraints** (from evidence, not preference): must speak MySQL prepared
  statements, tolerate MyISAM semantics (§6.1), expose per-connection state equivalent to
  thread-local transactions, and connect with the existing `host;port;user;pass;db` config or a
  translated URL (dev-memory already uses `mysql://` URLs on the Rust side, §1.1).
- **Async model translation**: C++ delay-thread + serial queues ⇒ Rust actor/task per DB with a
  serial-lane key (GUID-low) is the faithful translation; naive `tokio::spawn` per query would
  break §8.8 ordering and can interleave a later save ahead of an earlier one.
- **Safest sequencing**: (a) read-only world loaders (§7) against a cloned world DB; (b) realmd
  login reads/writes (already proven live per §1.1); (c) character load; (d) character save last,
  with differential testing (C++ save → Rust load → Rust save → byte-compare rows) on the §4.3/§5
  blob formats before any production character is touched.
- **Do-not-duplicate**: per §1.2/§1.3, inventory `vmangos-rs/crates` (in the checkout that has
  it) for existing `*-mysql` crates before writing new DB code; memory names `mangos-realmd-mysql`
  as an existing, live-tested crate.

## 10. Unresolved / not inspected

- `vmangos-rs/` Rust workspace is absent from this checkout — existing Rust DB crates
  (`mangos-realmd-mysql` et al. per memory) could not be inspected or counted here.
- World DB base dump/content (`vmangos-database/`, gitignored) not present — world table count,
  row volumes, and loader validation data unavailable; world schema known only via its 1,005 migrations.
- Live production DB not queried (read-only rule + no credentials handled): actual engine
  overrides, row counts, charset/collation drift vs dumps, and applied-migration state unverified.
- `DatabasePostgre` parity, `CharacterDatabaseCache/Cleaner` internals, `SQLStorage` format-string
  layouts, and per-loader SELECT inventories were sampled, not exhaustively catalogued.
- Dashboard/Discord Python DB access (PyMySQL per CLAUDE.md) is a third writer family — out of
  this scope but part of the full byte-compat contract for shared tables (e.g. account/characters reads).

## Appendix: count reproducibility

```
ls vmangos/sql/migrations | wc -l                                   # 1038
ls vmangos/sql/migrations | sed 's/.*_//' | sort | uniq -c          # 1005 world / 18 characters / 6 logon / 6 logs
grep -rE "(World|Character|Login|Logs)Database\." vmangos/src --include=*.cpp | wc -l   # 1142
grep -c "CREATE TABLE" vmangos/sql/{characters,logon,logs}.sql      # 55 / 11 / 15
grep -c "^void ObjectMgr::Load\|^bool ObjectMgr::Load" vmangos/src/game/ObjectMgr.cpp  # 96
grep -c "sObjectMgr.Load" vmangos/src/game/World.cpp                # 84
```

