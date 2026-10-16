# 11 — Completeness Critic: gaps, contradictions, unverified claims

Method: read reports 01–10 in full, then re-inspected code bodies myself
(file reads, not snippets) for 9 decisive claims. Verdicts in §1; what would
break a migration guide in §2–§5. Citations `path:line` in
`/Users/jeffarn/Programming/azeroth-server`.

## 0. Top finding

The survey is an accurate map of the **workstation checkout**, but production
runs a binary built 2026-04-14 while the checkout HEAD is 2026-05-26
(report 08 §6, itself verified by stat/ldd). Every other report therefore
describes code production does not run, and no report maps binary→commit.
A migration guide must state which tree it targets before citing any
behavior as "live". Second: quantitative claims (counts of handlers, SQL
sites, new/delete, locks) disagree between reports by method, not typos —
none may be quoted without its counting predicate (§2).

## 1. Spot-checks I performed (9)

| # | Claim (report) | What I read | Verdict |
|---|---|---|---|
| 1 | World tick = 50 ms/20 Hz, ` WorldRunnable` sleeps remainder, no catch-up cap (02 §4.1) | `vmangos/src/mangosd/WorldRunnable.cpp` full body: `WORLD_SLEEP_CONST 50` :39, `sWorld.Update(diff)` :94, remainder sleep :108-109 | **SURVIVED** |
| 2 | `NUM_MSG_TYPES=828`; 825 `StoreOpcode` (02) vs 827-row table/276 handlers (09); `STATUS_NEVER` 525 (02) | `Opcodes_1_12_1.h:830` = 828 ✓; `grep -c "StoreOpcode(" Opcodes.cpp` = **826**; distinct `WorldSession::Handle[A-Za-z_]*` = **276** (4 stubs: NULL/ServerSide/EarlyProccess/Deprecated ⇒ 272 real); `STATUS_NEVER` count 525 ✓ | **PARTIAL** — 02's 825/275/271 and 09's 827 are each off by 1–5; guide number: 826 registrations, 276 distinct, 272 real |
| 3 | Runner: Score×weight, fixed 32-slot array silently truncates, sort desc, fallthrough (05 §4, 10 §3#1) | `vmangos/src/game/PlayerBots/BotBehavior.cpp:16-81` full body (`Candidate[32]` :27, `count<32` guard :46, sort :64, try-until-true :67-78) | **SURVIVED** |
| 4 | `SRP6::Proof()` inverted vs its doc (02 §7) | `SRP6.h:74-80` documents "true if … matches"; `SRP6.cpp:160-161` returns `false` on memcmp==0. Also read full `SRP6.cpp` (the body 02 left unresolved): N :31, g=7 :32, k=3 in `B=(v*3+g^b)%N` :39, WoW interleave `HashSessionKey` :127-154, verifier hash :97-125 | **SURVIVED, and the body is now read** — test vectors are derivable from :44-154; but note `CalculateVerifier` feeds member `A` into the hash (:114) — verify A is empty/zero at account-creation time before trusting vectors |
| 5 | Save: `BeginTransaction(GUIDLow)` + one `REPLACE INTO characters` uber-insert; taxi/explored/equipment text blobs (04 §4.2-4.3) | `Player.cpp:16229-16414` read in full: BeginTransaction :16246, REPLACE :16252, blobs :16319-16382, `_Save*` bundle :16391-16399, Commit :16405; the reused `ostringstream` is safe because `addString(std::ostringstream&)` clears it (`SqlPreparedStatement.h:356-360`) — 04's per-column blob claim holds only via that overload | **SURVIVED** |
| 6 | 46 active DBC loads vs `DBCFilesCount = 45`; no Spell.dbc (06 §1.3) | `DBCStores.cpp:184` = 45; `grep -cE "^\s*LoadDBC\("` = **46**; the 46 filenames at :193-414 contain no `Spell.dbc`/`Map.dbc` | **SURVIVED** |
| 7 | Dashboard: port 8080 not 5000, `mysql.connector` not PyMySQL, `pgrep mangosd` liveness (07 §1,§6) | `dashboard/dashboard.py:10` import, :179 pgrep, :933-936 waitress :8080 | **SURVIVED** |
| 8 | Framing: `4 ≤ size ≤ 0x2800`, `cmd < NUM_MSG_TYPES`, "else drop connection"; one packet in flight (02 §6.1) | `WorldSocket.cpp:82-140` read in full: check at :105 ✓, read re-armed only on `HandlerResult::Okay` :115,135 ✓ — **but** the malformed branch (:105-109) just `return`s: it neither closes the socket nor re-arms the read. The connection wedges open until the no-auth timer fires, it is not "dropped" | **MOSTLY SURVIVED; "drop connection" is wrong** — a Rust port copying the words changes scanner-connection behavior |
| 9 | "No ACE" (01 §3, 02 headline) vs "ACE mutexes in shared/IO" (09 §6) | `grep ACE_` over `vmangos/src/`: only a TODO comment (`Master.cpp:200`), an `ACE_SIZEOF_LONG` guard (`Common.h:94-98`), a doc mention, and **commented-out** `ACE_Guard` lines in `BattleGroundMgr.cpp`. IO lives in `vmangos/src/shared/IO/` (Context/Networking/Timer), not "src/framework + game/Server" as 01 §3 says | **02 right, 09 WRONG; 01's networking location wrong**. Bonus finding: BG guards being commented out means `BattleGroundMgr` runs effectively unlocked — no report noticed |

Score: 6 fully survived, 2 partial, 1 report-vs-report contradiction with a
clear loser. The survey's file:line citations I re-checked were accurate;
its grep-census numbers are where it wobbles.

## 2. Contradictions between reports (guide must pick one predicate)

1. **Opcode census**: 825/275 (02) vs 827/276 (09) vs measured 826/276/272
   (§1#2). Also 02's `STATUS_LOGGEDIN` 289 — my grep counts 290 lines
   (`STATUS_LOGGEDIN,`); ±1 noise everywhere. No count in either report is
   reproducible as stated.
2. **SQL surface**: 1,142 "Database. call lines" (04 §3.3, any method on
   the 4 globals) vs 831 "anchored call sites" (09 §7, Execute/Query-family
   only) vs ~1,300 broad ceiling (09). Same prose phrase, three numbers;
   04's own subtotal (PExecute 361 + PQuery 202) also differs from 09's
   (405 + 266). A work-estimate built on any one is ±40%.
3. **Raw allocation**: "913 new/delete sites" in game excl. bots (03 §3)
   does not reconcile with 09's game-only `new` 659 + `delete` 419 = 1,078.
   One of the regexes counts something else (03 doesn't state its pattern).
4. **World loaders**: 96 `Load*` defs / 84 invoked (04 §7) vs "~120 ordered
   Load* calls, ~98 tables in ObjectMgr" (06 §5). Both plausible under
   different definitions (SpellMgr/ScriptMgr loads included or not) —
   neither says.
5. **LFG exists?** 05 §7: "'LFG' exists only as chat flavor — no queue"
   (about *bots*). Core has a real queue: `vmangos/src/game/LFG/LFGQueue.cpp`
   (473 lines, `LFGQueue::Update` :60) + `LFGHandler` + `LFGMgr`, and 02
   lists its `LfgUpdate` thread. A guide reader citing 05 will delete a
   live subsystem from the plan.
6. **Bot identity check count**: 05 says the `account≥100` test appears
   "~25 places" (§2) and "~10" (§8) in the same report. My crude grep
   finds 32 `<100/>=100` lines in game .cpp excl. PlayerBots (includes
   non-account 100s). None of the three numbers is a verified site list.
7. **Minor drifts**: PlayerBots 24,254 (03/05) vs 24,290 (09 admits the
   36-line .hpp delta); src headers 394 `.h` (01) vs 404 `.h/.hpp` (09);
   Handlers 16,185 (03) vs 16,212 (09). Harmless individually; together
   they show LOC figures are ±0.2% method noise — quote rounded.
8. **Doc-vs-doc already known** (07/10 agree): port 5000, PyMySQL, 3 ghost
   endpoints, behaviour counts 18+19 vs real 19+20 — consistent story, and
   my #6/#7 checks confirm the code side. This is the one place reports
   reinforce each other; keep.

## 3. Coverage of the seven named areas

- **Threading/lock inventory — HALF.** Thread map and queue table (02 §3,§8)
  are the survey's best section (verified `SqlResult` pool of 6 at
  `SqlOperations.cpp:151`). But there is **no lock inventory**: 09 counts
  "147 lock sites / 45 files" without naming one lock, while code has 41
  `shared_timed_mutex` mentions across ≥14 headers (Map store, ObjectMgr,
  ObjectAccessor, GuildMgr, VMapManager2, SocialMgr, Config…) whose
  granularity and ordering are the actual Rust `Send` design input. Also
  missing: which thread may take which lock (deadlock order), the
  commented-out BG locks (§1#9), and reconciliation of 02's default thread
  arithmetic with the live 73-thread process (08 §4) — defaults from
  `.dist` only, never validated against the running config.
- **Opcode/packet compatibility — DISPATCH ONLY.** Framing, status/class
  metadata and queue routing are covered (verified §1#2/#8). There is no
  per-opcode payload catalog for the 272 real handlers (each parses
  `ByteBuffer` positionally and throws — the schema *is* the handler body),
  no SMSG construction-site census, no byte layout for update blocks /
  movement splines beyond prose (03 §5-6), no packet captures as golden
  tests. The guide cannot estimate handler-porting effort from this
  survey; it can only sequence it.
- **Character save format — DEEP IN ONE ROW, SHALLOW IN 22.** 04 nails the
  `characters` row and items (verified §1#5). The other 22 login-query
  tables are only *named* (`Player.h:612-635`): auras, cooldowns, skills,
  reputation, honor, social, homebind, instance bindings, mail, BG data,
  account data/tutorials have no transcribed row shapes or blob formats.
  Absent entirely: character delete/repair flows, `PlayerDump`
  (`vmangos/src/game/PlayerDump.*`), corpse recovery rows, and everything
  written to the `logs` DB. "Save formats are locked" is evidenced for
  maybe 2 of ~30 tables.
- **Bot-to-core coupling — BEST-COVERED, still grep-grade.** 05's 52-hit
  table is a real integration checklist, but hits ≠ sites, the two
  account-check counts conflict (§2#6), `Map.cpp:1781` /
  `BattleGround.cpp:787` contexts are unread (self-declared), the "9
  synthesized session handlers" are never listed by name, and the
  `OnPacketReceived` tap's packet set (which SMSGs bots consume) is not
  enumerated. The Host-API conclusion is sound; its surface area is not
  yet enumerable from the report.
- **Binary asset formats — SOURCE-COMPLETE, BYTE-UNVERIFIED.** Every format
  claim rests on loader+writer code; **zero real .dbc/.map/.vmtile/.mmtile
  bytes were inspected by anyone** (06 §10, confirmed: no assets in this
  checkout). Unread sub-formats: `GroupModel` GMOD mesh layout
  (`WorldModel.cpp:271,330`), Detour payload + vendored version,
  extractor *inputs* (MPQ/ADT/WMO/M2 — needed only if assets are ever
  regenerated), DBC locale string charset, `offmesh.txt` semantics.
  Production `DataDir` fixtures (08 sizes them: 2.9 GiB) are a hard
  prerequisite the guide must schedule, not a footnote.
- **Live deployment facts — GOOD SNAPSHOT, STALE THE DAY IT WAS TAKEN.**
  08 is careful and read-only, but: binary≠checkout (see §0); DB row
  counts/sizes unknown (no creds — 15-day backup sizes are the only
  proxy); `azeroth-realmd` NRestarts=24 unexplained; Caddy/TLS/firewall
  in front of :8080 uninspected; the "quiet world" caveat (2.1% lifetime
  CPU, no sessions in log tail) means **no load baseline exists** — any
  Rust perf-parity claim has nothing to be measured against. Also 08's
  finding that `.conf` files are mode 644/`.env` 664 is a live secret-
  exposure issue the other reports' "secret hygiene" framing misses.
- **Subsystems nobody claimed (all verified present in `vmangos/src/game/`):**
  - **Anticheat/Warden** — `Anticheat/` ~4,900 lines Warden alone
    (`WardenWin.cpp` 1,386) + movement anticheat + own `WardenSessions`
    thread + `logs` DB writes. Mentioned only as a thread row (02) and a
    LOC line (09). Client-check crypto-scan protocol is a real port
    decision (port? disable? Warden is Windows-client-specific) that no
    report frames.
  - **Core LFG** — see §2#5. Queue matching logic unread by anyone.
  - **Battlegrounds internals** — exist (WSG/AB/AV, 6,631 lines of .cpp in
    `Battlegrounds/` + 5,209 script lines) and are *inventoried* in one
    table row by 03 §10, but queue brackets, scoring, and the
    `BattleGroundMap` update path have zero analysis; the disabled locks
    (§1#9) live here.
  - **Chat channels, auction, transports, pools, game events** — all exist
    and are one-row inventoried (03 §10): `Chat/Channel.cpp` (1,002),
    `AuctionHouse/` (AH bot deep-read in 05, the house itself not),
    `Transports/` (995), `Maps/Pool/PoolManager.cpp` (1,231),
    `GameEventMgr.cpp` (1,152). Inventory ≠ migration evidence: channel
    password/moderation state, AH expiry/mail-back flow, transport
    passenger map-transfer, pool spawn-group semantics, game-event
    spawn/despawn hooks are all unexamined.
  - **Barely or never mentioned anywhere**: `ZoneScript`/`OutdoorPvP`
    (40-virtual class — named only in 09's virtual census), `HonorMgr`
    rank system (only its save columns appear), `GMTicketMgr`,
    petitions, duels, taxi (as a system, vs taxi strings), `Weather`,
    `CreatureLinkingMgr`/`CreatureGroups`, `InstanceStatistics`,
    `SniffFile`, `HardcodedEvents`, `AuraRemovalMgr`, `GuardMgr`,
    `MassMailMgr` internals, `CharacterDatabaseCache/Cleaner`
    (04 self-declares), SOAP operation surface and RA command set
    (02: "thread + QueueCliCommand" only), `RealmMerge`, and the entire
    **PostgreSQL backend** (`DatabasePostgre`) — no report decides whether
    it compiles, is dead, or must survive. GM Commands are sized
    (23.6k LOC) but their command-table/permission contract (which 07/10
    say must be preserved for bots) is never enumerated for non-bot
    commands.

## 4. Unverified claims a guide would trip on

1. **"~190 existing Rust crates; realmd SRP logon proven 11/11"** (04 §1,
   from project memory). `vmangos-rs/crates` does not exist in this
   checkout (04 §1.2 and 09 ".rs = 0" both confirm). The single most
   decision-relevant inventory — what Rust already exists — is entirely
   second-hand. Do not cite until the other checkout is surveyed.
2. **macOS build blockers** (01 §7.1-7.6) are predictions; CMake was never
   installed. Honestly flagged, but a guide must not present "C++14 vs
   clang 21" or CMake-4 breakage as findings.
3. **"Keep schema + binary name ⇒ zero Python changes"** (07 §8/§9) is
   overstated: `LevelUp.log` also depends on `%2u` width spacing
   (`Player.cpp:3152`) and the `<BOT>` placeholder position (never
   observed in a live log — 07 self-declares), and `characters.online`
   *freshness timing* is a liveness contract whose C++ writer no one
   pinned. Zero-change is plausible, not proven.
4. **Rust size estimate "250–340k lines"** (09 §9) rests on an asserted
   0.7–1.0× ratio with no calibration against the (uninspectable) existing
   Rust work. Quote as guesswork or drop.
5. **"Recast is built but consumed only by the mmap generator"** (01 §2)
   and "TBB presumed dead" (01 §9): I confirm no `find_package(TBB)`
   anywhere (only `${TBB_INCLUDE_DIRS}` expansions in 3 CMakeLists), so
   "dead" is fair — but whether the generator actually links `Recast`
   was not verified by anyone.
6. **Thread-count defaults as production truth** (02 §3 cites `.dist`):
   live process has 73 threads (08). Whether production configs raise
   `Network.Threads`/`MapUpdate.*`/`*.WorkerThreads` is unknown, so the
   "config surface that must keep working" list may be the wrong subset.
7. **Asset reuse verdicts** ("reusable unchanged", 06 §9) are conditional
   on little-endian + float-bit-exact height/LoS math that no fixture has
   ever tested. Correct as code reading; unproven as engineering fact.
8. **Guild roster edit liveness** — whether the running server re-reads
   dashboard edits to `guild_bot_roster` is untraced by both 05 and 07,
   yet it's the one shared-write path. A guide that promises dashboard
   parity needs this answer.

## 5. What the survey still owes (genuine remaining gaps)

Consolidated from researchers' self-declared unresolved lists (§0 input)
minus what I closed (SRP6.cpp body — read; systemd unit *contents* —
closed by 08 via `systemctl cat` despite 07 listing them unresolved):
macOS configure/compile; per-lock inventory + lock ordering; per-opcode
payload catalog; the other ~28 character-DB save tables' shapes; world-DB
schema + row inventory (dump absent); live DB engine/charset/migration
state; production binary→commit mapping and live thread/config values;
byte-level asset fixtures from production DataDir; GMOD layout + Detour
version; Anticheat/Warden, core LFG, BG/ZoneScript/Honor internals;
SOAP/RA operation surfaces; PostgreSQL backend disposition;
`guild_bot_roster` reload behavior; `characters.online` writer site;
live `LevelUp.log` sample; existing Rust workspace inventory; realmd
restart cause; Caddy/firewall in front of dashboard; backup *restore*
verification; any load/perf baseline. Each is listed short-form in the
submitted `unresolved` array.
