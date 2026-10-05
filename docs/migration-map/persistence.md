# Persistence map: tables, save/load paths, parity definitions

Bead: `as-pnw.3` — static-first map for the Rust migration epic (`as-pnw`).
Method: source queries over `vmangos/src/game`, schemas in `vmangos/sql/*.sql`,
defaults in `vmangos/src/*/​*.conf.dist.in`. No live-DB access from this session;
every claim needing the droplet is marked **[VERIFY]** in §8.
Live `mangosd.conf` values are unknown here — config values below are
`.dist.in` defaults unless stated otherwise.

## 1. Database topology (MySQL vs memory)

`mangosd` opens four MySQL databases plus the world instant in RAM.
`realmd` opens one. Connection strings live in
`vmangos/src/mangosd/mangosd.conf.dist.in:77-88` and
`vmangos/src/realmd/realmd.conf.dist.in:173`:

| Handle (code)        | Physical DB (default) | Opened by | Role |
|----------------------|-----------------------|-----------|------|
| `WorldDatabase`      | `mangos`              | mangosd   | Static world content + a few GM/mgr writes. Bulk-loaded to RAM at startup, read-mostly at runtime (§4). |
| `CharacterDatabase`  | `characters`          | mangosd   | All mutable player/guild/auction/mail/instance state. The save path (§3). 55 tables in `vmangos/sql/characters.sql`. |
| `LogsDatabase`       | `logs`                | mangosd   | Append-only observability (chat, trade, warden, instance stats…). 15 tables in `vmangos/sql/logs.sql`. Writers: `WardenMac/Win.cpp`, `Player.cpp`, `Creature.cpp`, `BattleGround.cpp`, `World.cpp`, `InstanceStatistics.cpp`. |
| `LoginDatabase`      | `realmd`              | mangosd + realmd | Auth/account data shared between processes. 11 tables in `vmangos/sql/logon.sql`. mangosd only writes `realmcharacters` (char counts per account/realm). |
| — (realmd only)      | `realmd`              | realmd    | `account`, `account_access`, `account_banned`, `ip_banned`, `realmlist`, geoip tables. (`uptime` lives here too but is written by mangosd — `World.cpp:1756,2024`.) |

Schema evolution: `vmangos/sql/migrations/` holds 1038 files —
1005 `*_world.sql`, 18 `*_characters.sql`, 6 `*_logs.sql`, 6 `*_logon.sql`
(plus merge scripts). The world-DB base schema is **not** in this repo
(content dumps live in gitignored `vmangos-database/`); migrations are the
only world-schema source here. All databases carry a `migrations` table.

Related: the bot roster (`dashboard` guild-bot roster, `playerbot` table below)
is character data, not a separate store.

## 2. Characters-DB table inventory (grouped by subsystem)

55 `CREATE TABLE` statements in `vmangos/sql/characters.sql`:

- **Player core**: `characters` (one fat `REPLACE` row per save — position,
  money, xp, taxi mask, honor block, explored zones, equipment cache…),
  `character_stats`, `character_homebind`, `character_tutorial`,
  `character_gifts`, `character_forgotten_skills`, `character_duplicate_account`,
  `character_honor_cp`, `characters_guid_delete`, `characters_item_delete`,
  `character_deleted_items`.
- **Progression**: `character_action`, `character_aura`, `character_skills`,
  `character_spell`, `character_spell_cooldown`, `character_queststatus`,
  `character_reputation`, `character_battleground_data`.
- **Items**: `character_inventory` (slot map: bag/slot → item guid),
  `item_instance` (the item objects themselves, §5), `item_loot`,
  `item_text` (letter bodies, referenced by `mail.item_text_id`).
- **Mail**: `mail`, `mail_items` (§5).
- **Trade/economy**: `auction` (one row per live listing).
- **Social**: `character_social` (friend/ignore lists), `guild`, `guild_member`,
  `guild_rank`, `guild_eventlog`, `petition`, `petition_sign`, `groups`,
  `group_member`, `group_instance`.
- **Pets**: `character_pet`, `pet_aura`, `pet_spell`, `pet_spell_cooldown`.
- **Instances/respawn**: `instance` (dungeon persistent state + script data),
  `instance_reset` (per-player raid reset times), `character_instance`
  (binds), `group_instance`, `creature_respawn`, `gameobject_respawn`,
  `game_event_status`, `worldstates`, `world` (server-startup beacon row),
  `saved_variables`.
- **GM/support**: `gm_tickets`, `gm_surveys`, `gm_subsurveys`, `corpse`.
- **Bots**: `playerbot` (`char_guid → chance/comment/ai`; config only —
  never written by the save path).
- **Housekeeping**: `migrations`.

Memory-only (no backing table anywhere in the four schemas — reconstructed
every boot/login, safe to drop on crash): sessions, map grids/cells,
transports, live battleground/arena instances, LFG queues, auction in-memory
objects (rebuilt from `auction`), online-player cache, `MassMailMgr` queue.
Nothing the client can observe across a restart may live *only* here —
that invariant is what §7 turns into parity bars.

## 3. Save/load paths

### 3.1 Player: `Player::SaveToDB` / `Player::LoadFromDB`

- Save: `vmangos/src/game/Objects/Player.cpp:16229`. One
  `CharacterDatabase` transaction per save: fat `REPLACE INTO characters`
  + `_SaveBGData`, `_SaveInventory`, `_SaveQuestStatus`, `_SaveSpells`,
  `_SaveSpellCooldowns`, `_SaveAuras`, `_SaveSkills`,
  `m_reputationMgr.SaveToDB()`, `m_honorMgr.Save()`, tutorial save via
  session. `_SaveStats()` runs **outside** the transaction and only on
  logout unless `PlayerSave.Stats.SaveOnlyOnLogout = 0` (default 1,
  `mangosd.conf.dist.in:275`).
- Anti-dupe fast path: `SaveInventoryAndGoldToDB` (`Player.cpp:16425`) —
  inventory + money only, serialized in a transaction on the player GUID
  so a relog mid-query cannot dupe.
- Load: `Player::LoadFromDB` (`Player.cpp:14470`), fed by the login
  `SqlQueryHolder`: `PLAYER_LOGIN_QUERY_*` in
  `vmangos/src/game/Handlers/CharacterHandler.cpp:76-98`
  (char row, inventory, mail, spells, cooldowns, actions, auras, quests…).
- Triggers: periodic autosave every `PlayerSave.Interval` (default
  `900000` ms = 15 min, conf:273, code default `World.cpp:565`;
  per-player countdown `m_nextSave`, `Player.h:1280`, reset on every save);
  logout (`WorldSession.cpp:713`); login (`:362`); GM commands; delayed
  save if mid-far-teleport (`DELAYED_SAVE_PLAYER`, `Player.h:640`).
- Bots: `IsSavingDisabled()` (`Player.h:1296`) short-circuits **all** of
  the above — temporary bots (`Player.cpp:401`), faction-change ghosts,
  and the paths in `SaveInventoryAndGoldToDB`, pet save (`Pet.cpp:423`),
  corpse handling (`Player.cpp:4821`), group save (`Group.cpp:1547`).
  Bots are pure memory: no save, no load, no DB residue.

### 3.2 Items: `character_inventory` + `item_instance`

- `Item::SaveToDB` (`Objects/Item.cpp:273`): `REPLACE` when new, `UPDATE`
  on dirty fields (duration/flags/durability have a light path at `:456`).
  Crash-consistency rule: the `characters` row and its items commit in the
  same transaction (player save), but a lone item save (loot, trade, mail
  attach) commits alone — a crash between the two can orphan an
  `item_instance` row whose owner never references it (§5).
- Ownership is a single `owner_guid` column, rewritten on every transfer
  including mail attach (`Mail.cpp:269`).

### 3.3 Mail: send → expire → return-or-delete

- Send: `MailDraft::SendMailTo` (`Mail/Mail.cpp:288`) inserts one `mail`
  row + `mail_items` rows + re-points `item_instance.owner_guid` to the
  receiver. Expiry defaults (`Mail.cpp:304-317`): COD → 3 days, auction
  sale pending → 1 hour, everything else → 30 days.
- Producers (`SendMailTo` call sites outside `Mail/`): auction won/sold/
  expired/outbid (`AuctionHouseMgr.cpp:185,230,264,453`,
  `AuctionHouseHandler.cpp:179`), quest reward mail (`Player.cpp:13077`),
  battleground marks (`BattleGround.cpp:862`, 15-min deliver / 1-day
  expiry), item restore (`ObjectMgr.cpp:11476`, 30-day), player mail
  (`MailHandler.cpp:420,670`), GM commands (`MiscCommands.cpp:1017,1098,
  1153`), game-event mass mail (`GameEventMgr.cpp:1048-1051`) and the
  `.send massmail` GM commands (`MiscCommands.cpp:915,1048,1128`) via
  `MassMailMgr`, which fans out to **every non-deleted character**
  (`MassMailMgr.cpp`, throttled at `MassMailer.SendPerTick = 10`,
  conf:1159).
- Reaper: `ObjectMgr::ReturnOrDeleteOldMails` (`ObjectMgr.cpp:6661`),
  run at startup (`World.cpp:1712`) and every 30 min on a dedicated
  thread alongside `Player::DeleteOldCharacters` (`World.cpp:1244-1246`).
  Rules per expired row: no items and no letter body → `DELETE`; has
  items and player-sent and not already returned → **returned to sender
  with a fresh 30-day expiry** (a renewal loop, §5); has items but
  system-sent / already-returned / COD-payment → `DELETE` **plus**
  cascade deletes of `mail_items`, `item_instance`, `item_text`
  (`ObjectMgr.cpp:6540-6580`). Online receivers are skipped until logout;
  the live-server path pages at `Mail.ReturnedMail.PerTick = 5`
  (conf:1160).
- Read path: login holder query `PLAYER_LOGIN_QUERY_LOADMAILS`
  (`CharacterHandler.cpp:98`); unread/undelivered tracked in memory
  (`AddNewMailDeliverTime`, `Mail.cpp:338`).

### 3.4 Auction: `auction` table + in-memory `AuctionHouseObject`

- Load all at startup (`AuctionHouseMgr.cpp:392`); every list/bid/buyout
  writes through immediately (`INSERT` at `:852`, deletes on sale/
  expiry nearby). Expiry sweep in `AuctionHouseMgr::Update`
  (`World.cpp:1997` calls it on the world tick), expired lots go out as
  mail (`SendAuctionExpiredMail`, `:235`), **not** back to inventory —
  so every auction event is also a mail event.
- `AuctionHouseBotMgr` persists only its scheduler watermark
  (`auctionhousebot_state.last_refresh_time`, `:285`); bot listings
  themselves are ordinary `auction` rows. Bot disabled by default
  (`AHBot.Enable = 0`, conf:2937).

### 3.5 Guilds, groups, petitions, tickets

- Guild: `LoadGuildFromDB` / `LoadRanksFromDB` / `LoadMembersFromDB`
  (`Guild/Guild.cpp:291,346,423`), saved through `GuildMgr` on member/
  rank/MOTD changes (`guild`, `guild_member`, `guild_rank`,
  `guild_eventlog`). No periodic guild save — every mutation writes
  through.
- Groups: `group_member` rows inserted/deleted on join/leave
  (`Group/Group.cpp:1547ff`); BG groups and saving-disabled players are
  never persisted. `groups`/`group_instance` hold the leader/dungeon
  shell.
- Petitions/signatures and GM tickets/surveys write through on change
  (`petition*`, `gm_tickets`, `gm_surveys`, `gm_subsurveys`;
  tickets loaded at startup, `World.cpp:1705-1708`).

### 3.6 Pets, corpses, instances, respawns

- Pets: `Pet::SavePetToDB` (`Objects/Pet.cpp:408`) → `character_pet` +
  `pet_aura` / `pet_spell` / `pet_spell_cooldown`; called from player
  save and on stable/dismiss. Skipped for saving-disabled owners.
- Corpses: `Corpse::SaveToDB` (`Objects/Corpse.cpp:110`) on death;
  all corpses reloaded at startup (`LoadCorpses`, `World.cpp:1619`) so
  reclaim works across restarts. Decay tuning is config, not schema
  (`Corpse.Decay.*`, conf:1346-1351).
- Instances: `DungeonPersistentState::SaveToDB`
  (`Maps/MapPersistentStateMgr.cpp:238`) `INSERT`s `instance(id, map,
  reset_time, data)` (`:253`); unbind/reset `DELETE`s (`:689,951`);
  startup purges non-dungeon rows (`:797`). Binds live in
  `character_instance` (+ `group_instance`), reset schedules in
  `instance_reset`.
- Respawn: `creature_respawn` / `gameobject_respawn` written on death/use
  (immediately if `SaveRespawnTimeImmediately = 1`, conf:145-146, else
  at grid unload) — i.e. the world breathes through the characters DB.
- Startup skill/spell hygiene: `CharacterDatabaseCleaner` drops
  `character_skills`/`character_spell` rows whose IDs no longer exist in
  DBC/mgr stores.

## 4. World DB (`mangos`): read-mostly content

~84 `sObjectMgr.Load*` calls in `World.cpp:1306-1660` (strings, factions,
taxi, mail templates, item prototypes, creature/gameobject templates and
spawns, quests, trainers, vendors, graveyards, gossip, corpses, player
cache…). Runtime writes to `WorldDatabase` are the exception, not the
rule — exhaustive `PExecute` (non-SELECT) sweep:

| Writer | Statement | Meaning |
|--------|-----------|---------|
| `CreatureGroups.cpp:301,307` | `DELETE`/`INSERT creature_groups` | GM-formed creature groups |
| `GameEventMgr.cpp:131` | `UPDATE game_event SET disabled` | GM enables/disables events |
| `AuctionHouseBotMgr.cpp:285` | `UPDATE auctionhousebot_state` | Scheduler watermark |
| `Creature.cpp:1618-1624,2054-2059`, `GameObject.cpp:913-914,1011-1013` | `PExecuteLog` DELETEs/REPLACE on `creature`, `gameobject`, `*_battleground`, `game_event_*` | **GM `.add/.delete` commands only** |
| `WaypointManager.cpp:149,258,357` | `UPDATE creature_movement*` | GM waypoint edits |

Parity consequence (§7): the Rust port must treat `mangos` as sealed
content at runtime — the only write paths are GM tooling, which belongs
behind the same privilege gate, not in the hot loop.

## 5. Hot tables and the mail anomaly

Given (bead inputs): `mail` 676k rows / `mail_items` 750k rows for
287 characters (~2.4k mails/char); `item_instance` 237 MB / 806k rows.
No live-DB access here, so the mechanism below is code-verified and the
attribution is a hypothesis with confirmatory queries in §8.

**Retention mechanics (verified in code).** Three facts combine:

1. **30-day default retention.** Every non-COD mail lives 30 days
   (`Mail.cpp:314`). Steady-state row count ≈ daily send rate × 30 —
   any automated sender is multiplied by a month.
2. **The return loop renews instead of deleting.** An expired mail *with
   items* between two players is not deleted: it is `UPDATE`d back to
   the sender with a **fresh 30-day `expire_time`**
   (`ObjectMgr.cpp:6569`). Only system/returned/COD mail with items is
   cascade-deleted. Unclaimed player-to-player item mail therefore
   *pings-pongs for as long as both accounts exist*, and each leg
   re-points `mail_items.receiver_guid` and
   `item_instance.owner_guid` without ever freeing the rows.
3. **Receivers that never read.** The reaper skips online players until
   logout and throttles live-server processing to 5 mails/tick
   (`Mail.ReturnedMail.PerTick`, conf:1160). Mail to characters that
   never log in (bot-roster alts, abandoned characters — `playerbot`
   marks config, not activity) sits until expiry, then enters the
   return loop or waits behind the throttle.

**Send-side multipliers (verified call sites, volume unverified).**
Each auction lifecycle event emits mail (outbid refund, won item, sale
proceeds, expiry return — `AuctionHouseMgr.cpp:185,230,264,453`); BG
marks mail on every match end (`BattleGround.cpp:862`); quest reward
mail fires per completion (`Player.cpp:13077`); mass-mail fans out to
*every* non-deleted character per invocation
(`MassMailMgr.cpp`, producers: `GameEventMgr.cpp:1048-1051`,
`MiscCommands.cpp:915,1048,1128`). With a few hundred mostly-bot
characters receiving automated mail nobody reads, 2.4k rows/char over
months of uptime is the expected shape, not an aberration.

**Why `item_instance` tracks `mail_items` (verified linkage,
counts need §8).** A mailed item is *not* copied: the same
`item_instance` row is re-owned to the receiver (`Mail.cpp:269`) and
only deleted when its mail row is (`ObjectMgr.cpp:6552-6557`). So
every unclaimed mail-item parks one `item_instance` row indefinitely:
750k `mail_items` ≈ 806k `item_instance` is consistent with
mail-attached items dominating the table and player inventories being
a small remainder. Orphan risk runs the other way too — the lone-item
save path (§3.2) can strand rows no slot table references.

**[VERIFY]** Confirm on the droplet (read-only): `mail` grouped by
`receiver_guid` (top receivers — bots?), by `message_type`/`sender`
(auction vs mass-mail vs player); `item_instance` anti-join against
`mail_items` + `character_inventory` + `auction` (orphan count);
`SELECT DATEDIFF(NOW(), FROM_UNIXTIME(deliver_time))` histogram
(age structure — a 30-day sawtooth implicates retention, a long tail
implicates the return loop).

## 6. What lives only in memory (recap for seam design)

Authoritative-in-RAM, rebuilt from DB each boot/login: map grids and
all transient spawns/visibility, transports, sessions, live BG/LFG
state, the `MassMailMgr` queue, online-player and auction object
caches. Authoritative-in-DB, cached in RAM: everything in §2–§4.
Crash-loss budget today: up to one `PlayerSave.Interval` (15 min
default) of player progression + any unflushed lone-item/mail/auction
writes. The Rust port must preserve or explicitly renegotiate that
budget — see `as-pnw.7` (seams).

## 7. Per-subsystem parity definitions (for the Rust port)

"Parity" is per-subsystem, not one global bar. Each row names the
source of truth and the test that would prove the port equivalent.

| Subsystem | Source of truth | Parity = … |
|-----------|----------------|------------|
| Player core + stats/skills/spells/quests/reputation/auras/cooldowns | `characters` + `character_*` sidecars | **Field-for-field**: same login → identical client-visible state (position, stats, spells, cooldowns). `REPLACE` semantics preserved — no partial-row writes. |
| Inventory / items | `character_inventory` slot map + `item_instance` rows; guid sequence | **Guid-stability**: item GUIDs never reused across restart; owner mapping exact after every transfer. Orphan budget: zero new orphans vs C++ baseline under the same workload. |
| Mail | `mail` + `mail_items` + retention rules | **Retention-semantics**: identical send→expire→return/delete transitions and delays (3d COD / 30d default / 1h auction-pending); steady-state row counts within noise under the same send workload. |
| Auction | `auction` rows + expiry→mail transitions | **Lifecycle + money conservation**: same expiry outcomes, same cut/deposit math, no item or copper created/destroyed across the sweep. |
| Guilds / groups / petitions | `guild*`, `groups`/`group_member`, `petition*` | **Membership-after-restart**: roster, ranks, leadership identical across a restart; BG groups still ephemeral. |
| Pets | `character_pet` + `pet_*` | Same as player core for active/stable pets. |
| Instances / binds / respawns | `instance`, `character_instance`, `instance_reset`, `*_respawn` | **Reset-time semantics**: same reset scheduling and bind persistence; respawn timers survive restart identically. |
| Corpses | `corpse` | **Reclaimability**: corpse present and lootable after restart until decay. |
| World content (`mangos`) | Dump + 1005 world migrations | **Read parity**: same rows → same spawns/quests/vendors; migrations stay append-only; GM write paths gated identically. |
| Accounts / bans (`realmd`) | `account`, `account_banned`, `ip_banned`, `realmcharacters` | **Auth-decision parity**: same allow/deny for every credential/ban state. |
| Logs (`logs`) | `logs_*`, instance stats | **Best-effort**: lossy is acceptable; schema additive-only. |
| Memory-only (§6) | — (rebuilt) | **Reconstruction parity**: same observable state after boot/login; no durability bar. |
| Bots | `playerbot` config + *no* saves | **No-residue**: bot sessions still write nothing; roster config format unchanged. |

Cross-cutting: transaction boundaries (§3.1–§3.2) are part of parity —
the port must not widen the crash-loss budget (15-min autosave +
write-through events) without a written decision, and save-data formats
(character inventory, item instance, guild bot roster) stay locked per
production-phase rules (`CLAUDE.md`).

## 8. Dynamic verification follow-ups (not executed here)

Per epic method, recorded — not run:

1. Droplet read-only queries from §5 (`mail` receiver/type histograms,
   `item_instance` anti-join, age histogram).
2. Production `mangosd.conf` values for `PlayerSave.Interval`,
   `AHBot.Enable`/`itemcount`, `MassMailer.SendPerTick`,
   `Mail.ReturnedMail.PerTick` (defaults cited here may differ live).
3. Hot-path confirmation from `Perf.log`/slow-query log (no
   `perf_schema` access with the `mangos` user): expect
   `characters` REPLACE, `item_instance` REPLACE/UPDATE,
   `mail`+`mail_items` INSERT/SELECT-by-receiver at the top.
4. `world` DB table census from a live `SHOW TABLES` (schema not in repo).
5. Confirm top `mail.receiver_guid`s against `playerbot` roster
   (bot-mail hypothesis).
