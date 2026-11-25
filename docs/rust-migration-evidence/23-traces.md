# Phase 0 traces: guild_bot_roster reload liveness (#23) + characters.online writer (#25)

READ-ONLY investigation. All claims verified against file bodies on branch rust-migration.
C++ paths below are relative to repo root (`vmangos/...`); line numbers are 1-based.

## (A) guild_bot_roster reload liveness — ANSWER: load once at startup, never re-read

### A1. Every C++ read of `guild_bot_roster` (exhaustive: repo-wide `*.cpp`/`*.h` grep hits only this file)

- `vmangos/src/game/PlayerBots/GuildBotMgr.cpp:58-61` — the ONLY SELECT:
  `SELECT r.guid, c.name, c.race, c.class, r.spec, r.role, r.raid_assigned,
  r.active, r.gear_score, r.gear_tier, r.personality FROM guild_bot_roster r
  JOIN characters c ON r.guid = c.guid ORDER BY r.guid`, inside
  `GuildBotMgr::LoadRoster()` (`GuildBotMgr.cpp:52`), which `m_roster.clear()`s
  then rebuilds the in-memory `m_roster` vector + `m_guidToIndex` map.
- All other C++ touches are write-through UPDATEs, never reads:
  - `:480` `SetRaidAssigned` — UPDATE raid_assigned
  - `:490` `SetBotActive` — UPDATE active
  - `:501` `SetBotSpec` — UPDATE spec, role
  - `:667` `RecalculateGearScore` — UPDATE gear_score
  - `:691` `SetGearTier` — UPDATE gear_tier
- Each setter first mutates the in-memory copy (`m_roster[it->second].X = ...`,
  e.g. `:479`, `:489`, `:499-500`) and then persists. So C++-initiated edits
  stay coherent in-RAM; externally-initiated (dashboard) edits do not.

### A2. Classification of the single read

- **Startup-only.** Call chain: `World::Load ... sGuildBotMgr.Load()`
  (`vmangos/src/game/World.cpp:1818-1819`) → `GuildBotMgr::Load()`
  (`GuildBotMgr.cpp:26`) → `LoadRoster()` (`GuildBotMgr.cpp:46`).
  `LoadRoster` is `private` (`GuildBotMgr.h:120`) and has exactly one caller
  (`GuildBotMgr.cpp:46`; verified by repo-wide `LoadRoster` grep).
- **Periodic: NO.** `GuildBotMgr::Update()` (`GuildBotMgr.cpp:201-223`) runs on
  the `WUPDATE_AUCTIONS` tick (`World.cpp:1994`) but only does level-sync
  (`SyncBotLevels`, 5-min `LEVEL_SYNC_INTERVAL`, `GuildBotMgr.h:144`) and
  presence login/logout (`UpdatePresence`). Neither re-SELECTs the roster table.
- **On-command (.reload or GM command): NO.**
  - `.bot reload` → `HandleBotReloadCommand` calls `sPlayerBotMgr.Load()`
    (`PlayerBotMgr.cpp:681-686`), NOT `sGuildBotMgr.Load()`. `PlayerBotMgr`
    never references `guild_bot_roster` (zero grep hits outside GuildBotMgr).
  - Guild GM commands (`Chat.cpp:76` guildbot table: fill/assemble/disband/
    check/roster/botinfo) read the in-memory `GetRoster()` (`PlayerBotMgr.cpp:
    2289,2375,2407`) or mutate via setters; none re-load from DB. There is no
    `guildbot reload` subcommand.
- **On-event: NO.** Group removal (`Group.cpp:425,531`), invites
  (`GroupHandler.cpp:157,177`), loot (`CombatBotBaseAI.cpp:3779-3810`), chat
  (`BotChatManager.cpp:481-493`) all consume the cached `m_roster`
  (`IsGuildBot`/`GetBotInfo`/`GetRoster`); no DB re-read on any event path.

### A3. Definitive liveness answer

If the dashboard UPDATEs a `guild_bot_roster` row while mangosd is running,
**the running server never observes it** — not on any timer, not via any
`.reload`/GM command, not on any event. The change takes effect only at the
next mangosd (re)start, when `World.cpp:1819 → GuildBotMgr::Load() →
LoadRoster()` re-reads the table. Worse, a C++-initiated edit to the same
row's columns (raid_assigned/active/spec/role/gear_score/gear_tier) afterwards
would overwrite the dashboard's value from the stale in-memory copy
(`:479-480` pattern), and a dashboard edit to `spec`/`role` risks being
clobbered the same way. New rows (new guids) are likewise invisible until
restart (`m_guidToIndex` is only built in `LoadRoster`, `:103`).
Caveat: dashboard UPDATEs to `characters`-side columns (e.g. name) for a
roster guid are equally invisible to `m_roster` (name is snapshotted at
`:76`), though live game systems read `characters` from their own paths.

## (B) characters.online writer sites — all C++ writes pinned

`characters.online` (`tinyint`, default 0, `vmangos/sql/characters.sql:63`,
indexed `idx_online`, `:102`) has exactly four C++ write sites:

1. **Create: 0.** `Player::Create`-path insert (`Player.cpp:16078-16124`)
   hardcodes `uberInsert.addUInt32(0); // online` (`Player.cpp:16124`).
2. **Login: 1.** `WorldSession::HandlePlayerLogin` issues
   `UPDATE characters SET online = 1 WHERE guid = ?`
   (`Handlers/CharacterHandler.cpp:639-640`), right after map add
   (`:625-632`), alongside `account.online = 1` (`:642-643`).
3. **Steady-state: 1, every save.** `Player::SaveToDB(bool online = true, ...)`
   (`Player.h:1285`; impl `Player.cpp:16229`) writes the `online` column via
   `REPLACE INTO characters ...` (`:16252-16256`) with
   `uberInsert.addUInt32(online)` (`:16328`), except `if (!IsInWorld())
   online = false;` (`:16326-16327`). Called with the default (`online=true`)
   from the periodic autosave in `Player::Update` (`Player.cpp:1250-1256`,
   `SaveToDB()`), cadence `PlayerSave.Interval` default 15 min
   (`World.cpp:565`; first save jittered, `Player.cpp:150-154`).
4. **Logout: 0.** `WorldSession::LogoutPlayer` calls
   `_player->SaveToDB(false, removedFromMap)` (`WorldSession.cpp:712-713`),
   which stores 0 (player still in world at that point, so the `:16326`
   guard is not what fires — the explicit `false` argument is). The code
   comment at `:671-672` confirms the design: the `account.online = 0` reset
   (`:676-677`) happens here while the character row relies on this save.
   (If the player is not in world, `Save=false`, `:663-667`, no save occurs;
   row keeps its last value — normally 1 from autosave — until cleanup.)
5. **Startup/shutdown cleanup: 0 for all.** `Master::clearOnlineAccounts()`
   runs both before world boot (`Master.cpp:478-479`) and on shutdown
   (`:336-338`): `UPDATE characters SET online = 0 WHERE online<>0`
   (`Master.cpp:492`), plus the `account` table reset (`:490`).
   Notably there is NO equivalent cleanup on crash between these points —
   a kill -9 leaves stale 1s until next boot (shutdown path also saves
   online players first, `Master.cpp:551`).

No other `UPDATE characters ... online` exists in C++ (repo-wide
`UPDATE \`characters\`` grep: only `:492` touches `online`; the rest touch
name/level/flags/money/position/honor/instance).

### Freshness contract consumers actually get

- `online = 1` means "logged in and was in world at last login-or-save";
  refreshed at login (`CharacterHandler.cpp:639`) and at worst every
  `PlayerSave.Interval` (15 min default) while playing (`Player.cpp:1250`).
- `online = 0` is written synchronously at clean logout (`WorldSession.cpp:
  713`), at shutdown, and at next boot (`Master.cpp:492`). Grace window for
  dirty disconnect/crash: row can read 1 for a logged-out character until
  the next clean shutdown/boot (or their next login's overwrite).
- Consumers poll, nothing pushes: dashboard `/api/status` counts
  `WHERE online = 1 AND account </>= 100` (`dashboard/dashboard.py:194-197`),
  `/api/players` filters `c.online = 1` (`dashboard.py:235`), `/api/bots`
  counts/filters the same (`dashboard.py:374-382`); discord `get_online_players`
  SELECTs `c.online` for `account < 100` and filters in Python
  (`discord-bot/bot.py:65-82`, `:319-324`, `:411-412`).
  So both UIs inherit exactly the staleness above (up-to-15-min-fresh 1s;
  possibly-stale 1s after unclean disconnect; exact 0s after clean logout).
  Bots share the column: guild/crowd/dynamic bot characters are real
  `characters` rows on bot accounts (>= 100) and get online=1 via the same
  login/save paths when spawned in world.
- Rust implication: any Rust replacement of login/logout/save/cleanup must
  reproduce all five sites byte-for-byte in effect (values AND triggers),
  or dashboard/discord semantics drift. `idx_online` (`characters.sql:102`)
  exists, so the polling queries are index-backed.

## Unresolved / not verified (read-only limits)

- Live-DB confirmation (actual stale-1 rows after a crash) not inspected —
  no live database access per task rules.
- Whether the dashboard roster editor warns operators that edits need a
  mangosd restart — dashboard template text not inspected (out of scope).
- `HasLootUpgrades` persistence (in-memory only vs column) not traced;
  irrelevant to the two assigned questions.
