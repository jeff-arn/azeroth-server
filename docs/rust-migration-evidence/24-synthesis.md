# Phase 0 synthesis — verifier pass over 20/21/22/23

Verifier re-opened the cited bodies on branch `rust-migration` and spot-checked
6 decisive claims (2 save/load, 2 locks, 1 opcode row, 1 trace). Score: **6/6
survived**, with 1 minor numeric correction found en route (see Corrections).
Everything below not marked as verifier-checked is carried from the child
reports as-is.

## Spot-check outcomes

| # | Claim (source) | Verifier evidence | Verdict |
|---|---|---|---|
| S1 | `PLAYER_LOGIN_QUERY_BATTLEGROUND_DATA` (enum 20) is a dead slot: no query, no consumer (20-save-load) | Enum at `Player.h:610-636`, slot `:632`; repo-wide search for the symbol returns only `Player.h:632`; `LoginQueryHolder::Initialize()` (`CharacterHandler.cpp:73-100`) issues `SetPQuery` for the other 21 slots and none for it | **Survived** |
| S2 | `Player::SaveToDB` root at `Player.cpp:16229`; GUIDLow txn `16246`→`16405`; `characters` REPLACE has 61 binds; stats/pet save after commit | `BeginTransaction(GetGUIDLow())` `Player.cpp:16246`, `CommitTransaction()` `:16405`; direct count of `?` in VALUES list `:16262-16270` = 61; in-txn sub-saves `:16391-16403`; `_SaveStats()` `:16409-16410` and pet save `:16413-16414` both after commit | **Survived** |
| L1 | BattleGroundMgr is unguarded: both locks commented out, `.cpp` mentions are comments only (21-locks) | `// std::recursive_mutex m_lock;` `BattleGroundMgr.h:101-102`; `// std::mutex schedulerLock;` `:303`; `grep mutex BattleGroundMgr.cpp` hits only `:213,245,984`, all comments | **Survived** |
| L2 | `HashMapHolder<T>::i_lock` is a per-type static `shared_timed_mutex`, `Find` takes `ReadGuard`; nesting pair `m_visibleGUIDs_lock` → `i_lock` exists (21-locks) | `ObjectAccessor.h:49-51` typedefs, `Find` `ReadGuard` `:65-70`, `static LockType i_lock` `:80`; chain: `GridNotifiers.cpp:81` unique-locks `m_visibleGUIDs_lock`, loop calls `GetMap()->GetPlayer` `:86` → `Map::GetPlayer` (`Map.cpp:2619-2621`) → `ObjectAccessor::FindPlayer` → `Find` | **Survived** |
| O1 | CSV row `CMSG_WORLD_TELEPORT` (0x008): 6 reads `uint32 time \| uint32 mapId \| float positionX/Y/Z \| float orientation`, handler `MiscHandler.cpp:1127` (22-opcode-catalog; NOT one of the child's 3 manual checks) | Body `MiscHandler.cpp:1127-1145`: declarations `:1133-1138`, reads `recv_data >> time/mapId/positionX/positionY/positionZ/orientation` `:1140-1145` — order, count, types all match the CSV row. Bonus: CSV has 826 data rows; 47 lines contain `?`, matching the child's 47 `?`-row count | **Survived** |
| T1 | `guild_bot_roster` is read once at startup only; dashboard edits invisible until restart (23-traces A) | Sole SELECT in `GuildBotMgr::LoadRoster()` `GuildBotMgr.cpp:58-61`; repo-wide `LoadRoster` grep: definition `:52`, decl `GuildBotMgr.h:120`, single call `GuildBotMgr.cpp:46` inside `Load()` (`:26`); startup call `sGuildBotMgr.Load()` at `World.cpp:1819`. No other caller exists to re-read | **Survived** |

## Corrections to child claims

- **20-save-load, LOADQUESTSTATUS column count: 15 → 14.** The SELECT at
  `CharacterHandler.cpp:85` lists 14 columns (quest, status, rewarded,
  explored, timer, mob_count1-4, item_count1-4, reward_choice) and the
  consumer `_LoadQuestStatus` reads at most `fields[13]`. The "15" in the
  LOAD table and the "(15)" layout label are off by one; 15 is the SAVE-side
  bind count including `guid` (child's SAVE row, unaffected). No other child
  claim checked here needed correction. (Verifier also recounted LOADFROM:
  58 SELECT columns at `CharacterHandler.cpp:73-80` — child's 58 confirmed.)

## What is now established, per scope

### Save/load (20-save-load.md)
- Login load = 22-slot `PlayerLoginQueryIndex` holder (`Player.h:610-636`),
  query texts `CharacterHandler.cpp:65-103`; 21 live slots + 1 dead
  (`BATTLEGROUND_DATA`, verifier-confirmed S1). LOADFROM = 58 positional
  `characters` columns (verifier-recounted).
- Save root `Player::SaveToDB` `Player.cpp:16229`; one GUIDLow transaction
  `16246`→`16405` (verifier-confirmed S2) covering `characters` REPLACE
  (61 binds), bgdata, inventory/items, quest, spell, cooldown, aura,
  skills (+forgotten), reputation, honor CP, tutorial. Outside it:
  `character_stats` (post-commit, logout-gated), pet (own transaction),
  corpse (death-only).
- A `SaveToDB` triggers zero mail/auction/social writes; actions+mails save
  separately via `MasterPlayer::SaveToDB` at logout only; social/mail-send/
  auction writes are immediate on their own paths. (Child-cited, not
  re-checked beyond S2's post-commit region.)

### Locks (21-locks.md)
- BattleGroundMgr queues have no active lock (verifier-confirmed L1) — the
  one known unguarded cross-thread surface.
- Global registries: per-type static `shared_timed_mutex` in
  `HashMapHolder<T>` (verifier-confirmed L2); per-Map store has 9+ locks;
  session path uses `LockedQueue` (plain mutex, no spinlock) + socket send
  lock; `SqlDelayThread` = 2 `LockedQueue`s + `volatile bool m_running`.
- Two co-held ordering pairs sampled: `m_visibleGUIDs_lock` →
  `HashMapHolder<Player>::i_lock` (verifier-confirmed L2) and
  `PlayerBroadcaster m_queue_lock` → `m_listeners_lock` (child-cited);
  MoveMap/VMap "upgrades" are unlock-before-relock, not nesting.

### Opcode catalog (22-opcode-catalog.md + .csv)
- 826 registrations / 276 distinct handlers / 272 non-stub / 525
  `STATUS_NEVER`, zero delta vs census, zero enum mismatches; CSV row count
  and 47 `?`-row count verifier-corroborated, one independent row
  (CMSG_WORLD_TELEPORT) verifier-confirmed end-to-end (O1). Child's own 3
  manual checks (CharCreate, Messagechat, CastSpell) stand as reported.
- Skeleton is not yet codegen-ready: helper-hidden reads are opaque
  `call(...)` steps and 47 rows need human triage (see Unresolved).

### Traces (23-traces.md)
- (A) `guild_bot_roster`: single startup SELECT, no periodic/command/event
  re-read (verifier-confirmed T1) — dashboard roster edits take effect only
  at next mangosd restart and can be clobbered by later C++ setter writes
  from the stale in-memory copy.
- (B) `characters.online`: five write sites (create 0, login 1, every
  autosave 1 at 15-min default, logout 0, boot/shutdown bulk 0); consumers
  (dashboard, discord bot) poll and inherit that staleness contract.
  (Child-cited; verifier checked trace (A) only.)

## Consolidated unresolved (verbatim from children — none resolved here)

- BATTLEGROUND_DATA enum slot 20: no SetPQuery/TakeResult anywhere — dead slot, presumed leftover
- ITEM_CHANGED rows may skip Item::SaveToDB via test!=item branch Player.cpp:16619-16625 (control-flow dependent)
- World::m_QueuedSessions Queue typedef target unresolved (World.h:1004)
- ObjectAccessor name-map unguarded access assumes single-threaded callers, not enumerated
- i_playerGuard usage unconfirmed; nesting survey is sampled, not exhaustive whole-program
- 47 CSV rows contain '?'-typed read steps needing human triage before codegen
- Top ~10 call(...) helpers (movement, spell targets) not expanded to field lists
- Single-statement brace-less control edge cases only spot-checked on 3 handlers
- No live-DB check for stale online=1 rows (read-only task, no DB access).
- Dashboard roster-editor restart warning text not inspected (out of scope).

Note: the first item is now verifier-corroborated as an observation (S1);
it remains on this list because *why* the slot exists (leftover vs reserved)
is still unestablished — kept verbatim rather than resolved by invention.
