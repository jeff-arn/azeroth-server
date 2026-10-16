# 02 — Process & Network Runtime (mangosd / realmd)

Scope: entry points, main loops, threading, sockets, packet framing, opcode
dispatch, session queueing, SRP auth, realmd↔mangosd contract, config,
signals/shutdown, freeze detection. Read-only inspection; all citations are
`path:line` relative to workspace root `/Users/jeffarn/Programming/azeroth-server`.
No secrets were read (only `*.conf.dist.in` templates).

**Headline for Rust:** there is no ACE reactor anymore. Both daemons run a
hand-rolled ASIO-like `IO::IoContext` (Linux epoll / BSD-macOS kqueue /
Windows IOCP) with callback-style async sockets, plus a single-threaded
50 ms world tick. The Rust boundary is therefore: Tokio-style IO tasks
↔ bounded/locked queues ↔ one logical world thread (with carefully fenced
map/async side threads) ↔ per-DB delay threads. Almost every cross-thread
handoff in C++ is an explicit queue listed in §7; almost everything else
assumes single-threaded ownership and uses raw pointers + `volatile` flags
that Rust must replace with atomics/channels/`Arc`.

---

## 1. Two processes, no direct IPC socket

| | realmd (auth) | mangosd (world) |
|---|---|---|
| Entry | `vmangos/src/realmd/Main.cpp:81` `main()` | `vmangos/src/mangosd/Main.cpp:65` `main()` → `Master::Run()` |
| Default port | 3724 (`RealmServerPort`, `vmangos/src/realmd/realmd.conf.dist.in:177`) | 8085 (`WorldServerPort`, `vmangos/src/mangosd/mangosd.conf.dist.in:90`) |
| DBs opened | Login only (`vmangos/src/realmd/Main.cpp:78`, `StartDB()` :397) | World+Character+Login+Logs globals (`vmangos/src/mangosd/Main.cpp:54-57`) |
| Game state | none; realm list cache only | all (`World` singleton) |

Realmd→mangosd contract is **shared MySQL only** — no socket, pipe, or RPC
between them was found in either entry point or listener setup:

- realmd writes `account.sessionkey/last_ip/last_login` synchronously on
  SRP success (`vmangos/src/realmd/AuthSocket.cpp:704-707`, explicitly
  `DbExecMode::MustBeSync` to avoid a race with the client reaching mangosd).
- mangosd reads that row in `CMSG_AUTH_SESSION`
  (`vmangos/src/game/Server/WorldSocket.cpp:263-282`, selects
  `sessionkey, last_ip, v, s, …`), rejects if `last_ip` ≠ world-socket peer
  IP (:300-308) — i.e. "must have visited realmd first" is enforced via DB.
- mangosd publishes liveness by writing `realmlist` (`realmflags & ~OFFLINE`,
  `population=0`, `realmbuilds`) at startup
  (`vmangos/src/mangosd/Master.cpp:202-208`) and `| OFFLINE` at shutdown
  (:317); realmd polls `realmlist` into memory
  (`vmangos/src/realmd/RealmList.cpp:163-172`, refreshed every
  `RealmsStateUpdateDelay` default 20 s, `vmangos/src/realmd/Main.cpp:195`).
- mangosd also writes `uptime`, `realmcharacters`, `account.current_realm`
  into the login DB (`vmangos/src/game/World.cpp:2024,2867`;
  `vmangos/src/mangosd/Master.cpp:486-495`).

*Rust implication:* the two binaries can be ported independently; the
contract to freeze is the login-DB schema (`account`, `realmlist`,
`realmcharacters`, `uptime`, `account_banned/ip_banned`, `allowed_clients`)
plus timing (sessionkey must be visible before the client connects to
mangosd — keep that write synchronous/awaited, not fire-and-forget).

## 2. Startup sequences

### 2.1 mangosd

1. Parse args, `sConfig.LoadFromFile` before anything else
   (`vmangos/src/mangosd/Main.cpp:70-85`), open logs, optional POSIX
   daemonise / Windows service (:89-113), then `sMaster.Run()` (:144).
   Exit codes: 0 normal, 1 error, 2 restart-for-restarter (:147-149).
2. `Master::Run()` (`vmangos/src/mangosd/Master.cpp:129`): PID file,
   `_StartDB()` — all four DBs, each `*.Connections` sync + `*.WorkerThreads`
   async (`StartDB()` :402-457; password masked in log :414-447) — verify
   `RealmID` exists in `realmlist` (:153-162).
3. Create **one** `IO::IoContext`, spawn `Network.Threads` (default 1)
   IO runner threads named `IO[i]` (:167-180).
4. `sWorld.SetInitialWorldSettings()` (:183) — loads *all* static/game data
   synchronously on the main thread; also spawns several world-owned
   threads near its end (§3). Only after this are async DB transactions
   allowed (:188-193, "forbid any async transactions during server startup").
5. Hook signals (:196), spawn `WorldRunnable` thread (:199), mark realm
   online in DB (:207), optional CLI thread (:219), optional RA listener
   on same IoContext (:222-224, `Ra.Port` default 3443 :102-122), optional
   SOAP thread (:275-280, default 7878), optional FreezeDetector (:282-287).
6. Start world listener via `sWorldSocketMgr.StartWorldNetworking` (:300),
   then main thread **blocks on `world_thread.join()`** (:306). The main
   thread owns ordered teardown after the join (§9).

### 2.2 realmd

Single `main()` (`vmangos/src/realmd/Main.cpp:81`): config (:96), PID file,
login DB (:176), `sRealmList.Initialize` (:195), patch cache + system timer
singletons (:209-210), expire bans (:215-218), create IoContext (:223),
bind listener (:232), `AutoAcceptSocketsUntilClose` creating one
`AuthSocket` per connection (:242-278, incl. PROXY-v2 trusted-proxy path),
hook signals (:281), `AllowAsyncTransactions` (:327), spawn **one** IO
thread `MainIoCtx` (:333), then main thread idles in a 1 s sleep loop that
only pings MySQL every `MaxPingTime` (:339-356). All real work is on the
single IO thread + DB delay threads.

*Rust implication:* startup is a strict phase barrier (sync load → enable
async DB → start accepting). Preserve it: don't accept/log in players
before static data is loaded; `AllowAsyncTransactions` maps to "don't
spawn background DB writers during load".

## 3. Complete thread map (runtime)

Thread creation is centralised through `IO::Multithreading::CreateThread`
(named `std::thread` wrapper, `vmangos/src/shared/IO/Multithreading/CreateThread.cpp:11-23`)
and `ThreadPool` (`vmangos/src/shared/ThreadPool.cpp:103`).

### 3.1 mangosd

| Thread(s) | Count (default) | Spawned at | Owns / does |
|---|---|---|---|
| `mangosd-main` | 1 | process main, renamed `Master.cpp:270` | startup, then `join(WorldRunnable)`, ordered shutdown |
| `IO[i]` | `Network.Threads`=1 (`Master.cpp:170,179`) | `Master.cpp:179` | run shared `IoContext::RunUntilShutdown()`; all socket callbacks, accept, RA |
| `WorldRunnable` | 1 | `Master.cpp:199` | the 20 Hz world tick, `World::Update` (§4) |
| `SystemTimer` | 1 (Unix) | singleton `AsyncSystemTimer` (`Master.cpp:269`; thread in `.../Timer/impl/unix/AsyncSystemTimer.cpp:11`) | one-shot timeouts (no-auth kick etc.), ~16 ms res (`AsyncSystemTimer.h:36-39`) |
| `CLI` | 0/1 (`Console.Enable`) | `Master.cpp:219` | stdin via `select()` (`CliRunnable.cpp:78,124`) → queues commands only |
| `FreezeDetector` | 0/1 (`MaxCoreStuckTime`, default 0=off) | `Master.cpp:286` | watchdog over world loop counter (§9) |
| `SOAP` | 0/1 (`SOAP.Enabled`) | `MaNGOSsoap.cpp:75` via `Master.cpp:279` | gSOAP HTTP; queues CLI commands (`MaNGOSsoap.cpp:117`) |
| `DB:<dbname>` delay threads | 1 per DB × 4 DBs (`*.WorkerThreads`) | `Database.cpp:186-188,228` | each owns one MySQL conn, drains its `SqlDelayThread` queue |
| `SqlResult[0..5]` | 6 per `SqlResultQueue` (one per DB ⇒ up to 24) | `SqlOperations.cpp:151` `new ThreadPool("SqlResult", 6)` | execute async query callbacks off delay threads |
| `AsyncPacket` | 1 | `World.cpp:1874` | `PACKET_PROCESS_ASYNC` packets (chat/db-query) when world session update not running (§6) |
| `CharDB` | 1 | `World.cpp:1873` | every 30 min: delete old chars/mails (`World.cpp:1232-1250`) |
| `LfgUpdate` | 0/1 (patch/config gated) | `World.cpp:1866` | `LFGQueue::Update()` 1 s loop (`LFGQueue.cpp:63,218`) |
| `WardenSessions` | 1 | `World.cpp:1869` → `Anticheat.cpp:98` | anticheat/Warden session scans |
| `WorldUpdate[i]` | `AsyncTasks.Threads`=1 | lazily `World.cpp:2033-2034` | per-tick async tasks run *concurrently with* map update, joined via `future` each tick (:2038,2060-2061) |
| `MapManager[i]` | `MapUpdate.Instanced.UpdateThreads`=2 | `MapManager.cpp:42,45` | instanced-map updates (fork-join per tick) |
| `NewMapForPlayer` | 1 | `MapManager.cpp:42,339` | instance creation |
| `MapContinent[i]` | #continents, created on demand | `MapManager.cpp:346-349` | continent map updates (fork-join) |
| per-continent-map pools `MapObj/MapMotion/MapVis/MapCell` | config; defaults Obj 4−1, Vis 4−1, Motion 0, MTCells 0 | `Map.cpp:149-158` (continents only) | object/motion/visibility/cell sub-updates |
| MovementBroadcaster workers | `Network.PacketBroadcast.Threads`=0 (off) | `World.cpp:1871`; `MovementBroadcaster.cpp:37-38` raw `std::thread` | sharded by player GUID % N (`.cpp:46`) |
| Mailer worker | 0/1 (only `ENABLE_MAILSENDER` builds) | `MailerService.cpp:30` | outbound mail queue (mostly a realmd feature) |

Defaults from `vmangos/src/mangosd/mangosd.conf.dist.in:79-88,306-327,2820-2824`.
Bot subsystems (`sPlayerBotMgr` etc.) are updated **on the world thread**
(`World.cpp:2115`), not on separate threads, apart from chat/director hooks
on the auction timer (`World.cpp:1991-1995`).

### 3.2 realmd

| Thread | Count | Spawned at | Owns |
|---|---|---|---|
| `realmd-main` | 1 | main, renamed `realmd/Main.cpp:211` | 1 s idle/MySQL-ping loop (:339) |
| `MainIoCtx` | 1 | `realmd/Main.cpp:333` | entire auth protocol (all `AuthSocket` callbacks) |
| `SystemTimer` | 1 (Unix) | singleton (`realmd/Main.cpp:210`) | `MaxSessionDuration` kicks (`AuthSocket.cpp:68-77`) |
| `DB:…` + `SqlResult[0..5]` | 1 + 6 | as mangosd, login DB only | async login-DB work |

Note realmd is effectively single-threaded for protocol logic: every
`AuthSocket` state machine runs on the one IO thread, so `m_status`, `srp`,
etc. need no locks (`vmangos/src/realmd/AuthSocket.h:86-148`).

*Rust implication:* reproduce this as: N IO/runtime workers (can be Tokio
multi-thread, but per-connection state must stay sequential), 1 world
actor/task, 1 async-packet task gated by a mutex equivalent, per-map
fork-join via rayon/scoped tasks, and a DB layer with ordered delay queues
(ordering matters: delay threads are FIFO per DB, §8). Thread *count*
defaults are config surface that must keep working (`Network.Threads`,
`MapUpdate.*`, `*.WorkerThreads`).

## 4. Main loops

### 4.1 World tick (the single-threaded heart)

`WorldRunnable::operator()` (`vmangos/src/mangosd/WorldRunnable.cpp:47-130`):
fixed-cadence loop targeting `WORLD_SLEEP_CONST 50` ms = 20 Hz (:38-39).
Each iteration increments `volatile World::m_worldLoopCounter` (:68),
computes `diff` ms since last tick, calls `sWorld.Update(diff)` (:94), then
sleeps only the remainder `50 − updateTime` (:108-109); an overrun tick runs
the next tick immediately (no catch-up cap, `diff` just grows).

`World::Update(diff)` order (`vmangos/src/game/World.cpp:1965-2134`) is
load-bearing and must be preserved in Rust:

1. timers/game time, mass mail, auctions+bots on auction timer (:1971-1998)
2. **async packets fenced off** (`m_canProcessAsyncPackets=false` + take
   `m_asyncPacketsMutex` :2000-2014): run `Messager` callbacks, then
   `UpdateSessions(diff)` (world-class packets, login queue, session
   create/delete — §6)
3. swap async-task list under lock, dispatch to `WorldUpdate` pool
   (:2036-2040), `sMapMgr.Update` + BG/guard/zone updates (:2042-2045)
   — maps internally fork-join (§3) and process movement/spells/map
   packets (`Map.cpp:883-884,939-951`) — then `job.wait()` (:2060-2061)
4. `UpdateResultQueue()` — async DB callbacks for Character/World/Login,
   time-budgeted (`World.cpp:2076,2850-2856`)
5. corpses, game events, remove-list, instance resets, bot mgr, autobroadcast,
   account mgr (:2082-2119)
6. **async packets fenced off again** around `ProcessCliCommands()` (:2121-2129)

Shutdown tail runs on the same thread: `World::Shutdown()` (kick+save all,
join CharDB/LFG/AsyncPacket/Warden, `World.cpp:191-207`), delete BGs,
unload maps (`WorldRunnable.cpp:119-129`).

### 4.2 IO loop

`IoContext::RunUntilShutdown()` may run on multiple threads concurrently
(`IoContext.h:42-45`). Linux impl (`IoContext_linux.cpp:52-93`):
`epoll_wait(…, 250 events, 500 ms)` loop while `volatile m_isRunning`;
events dispatch either to the context-switch queue (drained under
`m_contextSwitchQueueLock`, :74-85) or directly via
`((SystemIoEventReceiver*)event.data.ptr)->OnIoEvent(…)` (:89) — the
socket object's own address is registered in epoll/kqueue
(`AsyncSocket.h:39-45`), so sockets are address-pinned after
`InitializeAndFixateMemoryLocation()`. BSD uses `kevent`
(`IoContext_bsd.cpp:36`), Windows IOCP (`IO/README.md:5-9`).

Socket model (`IO/README.md:27-42`, `AsyncSocket.h:52-70`): **one transfer
per direction** (one outstanding Read, one Write, one context-switch),
state machine in a single `std::atomic<int>` bitfield
(`AsyncSocket.h:92-115`) instead of mutexes; buffers are *not* copied —
caller must keep socket+buffer alive until callback (done via
`shared_from_this` captures everywhere). On POSIX a ready socket may invoke
the callback inline on the caller's thread without an IO-thread hop
(`IO/README.md:34-41,107-108`) — a subtle re-entrancy property Rust/Tokio
will *not* reproduce automatically; audit callers that assumed either
behaviour.

`select()` survives only outside the game network path: CLI stdin
(`CliRunnable.cpp:78,124`) and outbound connect (`SocketConnector.cpp:84`).
ACE is gone except one TODO comment (`Master.cpp:200`).

### 4.3 realmd loop

No tick: main thread sleeps 1 s × N and pings MySQL
(`realmd/Main.cpp:330-356`); everything else is IO-thread callbacks.
Realm-list refresh is lazy, driven by client requests via
`RealmList::UpdateIfNeed()` (`RealmList.cpp:148-161`), not a timer thread.

## 5. Listeners & connection lifecycle

- World: `WorldSocketMgr::StartWorldNetworking` binds one
  `AsyncSocketAcceptor` (`BindIP`, world port) on the shared IoContext
  (`WorldSocketMgr.cpp:33-48`). Accept callback (IO thread) wraps the fd in
  `AsyncSocket`, fixates it, sets send-buffer/`TCP_NODELAY` from config
  (:67-89), optionally reads a PROXY-v2 header if peer ∈
  `Network.TrustedProxyServers` (:92-107), then `WorldSocket::Start()`.
  `GetLeastUsedIoContext()` is a stub returning the single context
  (:116-122) — no per-thread accept sharding today.
- Auth: identical pattern in `realmd/Main.cpp:232-278` (own
  `TrustedProxyServers` key), creating `AuthSocket`.
- RA (telnet admin, default off): second acceptor on the *same* mangosd
  IoContext (`Master.cpp:102-122`); `RASocket` only enqueues CLI commands
  (`RASocket.cpp:232`). SOAP (default off) is a separate blocking gSOAP
  thread doing the same (`MaNGOSsoap.cpp:52-75,117`).
- Both game sockets arm an unauthenticated/idle kill timer on the
  `SystemTimer` thread: world `Network.TimeoutSecsIfNoAuth` default 10 s
  (`WorldSocket.cpp:592-607`), auth `MaxSessionDuration` default 300 s
  (`AuthSocket.cpp:66-80`). Timers capture raw `this` and rely on
  destructor cancellation — in Rust this becomes a `tokio::time::timeout`
  / `AbortHandle` tied to connection-task lifetime.
- Ownership: sockets are `enable_shared_from_this`; every async callback
  captures `self = shared_from_this()` (e.g. `WorldSocket.cpp:87`,
  `AuthSocket.cpp:107`). Dropping the last ref (no pending IO) closes the
  fd. `WorldSession` holds `shared_ptr<WorldSocket>` while `WorldSocket`
  holds a **raw** `WorldSession* m_Session` (`WorldSocket.h:71`),
  nullified by `FinalizeSession()` (:119) from the session destructor —
  the code itself flags the remaining race (`World.cpp:245`
  "Still there is race condition in WorldSession* being used in the
  Sockets"). Rust must break this cycle explicitly (e.g. session owns a
  channel/handle to the socket task; socket sends packets to a session
  inbox, never holds `&Session`).

## 6. Packet framing, ByteBuffer, dispatch

### 6.1 Wire framing (world, client build 1.12.1)

Ingress (`WorldSocket::DoRecvIncomingData`, `WorldSocket.cpp:82-140`), run
entirely on IO thread(s), strict read-exact state machine:

1. Read 6-byte `ClientPktHeader {u16 size (big-endian on wire), u32 cmd}`
   packed struct (`WorldSocket.h:50-54`).
2. `m_Crypt.DecryptRecv` over the header (:100) — header-only stream cipher
   (`AuthCrypt`), no-op until keyed at auth (`AuthCrypt.cpp:35-54`).
3. Endian-fix, validate `4 ≤ size ≤ 0x2800` and `cmd < NUM_MSG_TYPES(828)`
   else drop connection (:102-109).
4. Read `size−4` payload bytes directly into a freshly allocated
   `WorldPacket` (:111-138), then recurse for the next header only if the
   handler result is `Okay` — i.e. **one packet in flight per socket,
   strictly ordered, no pipelining buffer**.

Egress: `WorldSession::SendPacket` (callable from world/map/async threads)
→ `WorldSocket::SendPacket(WorldPacket)` by value
(`WorldSocket.cpp:516-538`): push onto `std::queue<WorldPacket>` under
`m_sendQueueLock`, hard cap 1024 queued packets ⇒ disconnect (:523-529);
an `atomic_flag` ensures one drainer, which hops to the IO thread via
`EnterIoContext`, coalesces *all* queued packets into one reused
`ByteBuffer`, prepends 4-byte `ServerPktHeader {u16 size BE, u16 cmd}`
(`WorldSocket.cpp:51-59`), encrypts each header, single `Write`, repeat
(:540-590). Comment at :521 is the design rule: "We don't want to allocate
or encrypt anything inside the world thread". Session-level cap: packets
>0x8000 bytes are dropped before queueing (`WorldSession.cpp:104-112`).

Auth framing differs: 1-byte command first (`AuthSocket.cpp:103-117`),
then per-command fixed/variable packed structs (`AuthPackets.h:15-140`,
`#pragma pack(1)` with a static_assert tripwire :150), read with the same
read-exact callback chaining; no header encryption (SRP instead, §7).

### 6.2 ByteBuffer / WorldPacket

`ByteBuffer` (`vmangos/src/shared/ByteBuffer.h:50-490`): `vector<uint8>` +
independent read/write cursors; little-endian `<<`/`>>` for all ints,
floats, C-strings (NUL-terminated), packed GUIDs/XYZ (:377-467); every
read bounds-checks and **throws `ByteBufferException`** (:30-42,327-328).
Default reserve 0x1000 (:53). `WorldPacket : ByteBuffer` adds `u16 opcode`
+ receive timestamp (`WorldPacket.h:30-66`).

*Rust implication:* this maps 1:1 to a `BytesMut` cursor type, but the
error model changes: handlers today rely on exceptions being caught at
exactly two boundaries (`WorldSocket.cpp:181-199` for inline packets,
`WorldSession.cpp:528-553` for queued ones, both optionally kicking on
`KickPlayerOnBadPacket`). In Rust every handler parse must return
`Result` and the same two boundaries must map `Err` → log/kick. Unchecked
`read<T>` via raw pointer cast (`ByteBuffer.h:335`) also means handlers
implicitly accept any bit pattern — newtyped enums need fallible conversion.

### 6.3 Opcode table (counted)

Build selection: CMake cache `SUPPORTED_CLIENT_BUILD=CLIENT_BUILD_1_12_1`
(`vmangos/cmake/options.cmake:11`) → generated `Progression.h` → includes
`Opcodes_1_12_1.h` (`Opcodes.h:38-39`). Other builds (1.5.1–1.11.2) keep
parallel enum headers (`Opcodes.h:40-54`) — same handler table, different
numeric values; a Rust port targeting only 1.12.1 can drop 7 headers.

Counts (scripted over source, this inspection):

- Enum space: `NUM_MSG_TYPES = 828` (`Opcodes_1_12_1.h`, tail), split
  CMSG 386 / SMSG 360 / MSG 81 (+null).
- `StoreOpcode` registrations in `Opcodes.cpp`: **825** (some gated by
  `#if SUPPORTED_CLIENT_BUILD > …`).
- By status: `STATUS_NEVER` 525 (server-only/deprecated — includes all
  SMSG), `STATUS_LOGGEDIN` 289, `STATUS_AUTHED` 8,
  `STATUS_LOGGEDIN_OR_RECENTLY_LOGGEDOUT` 2, `STATUS_TRANSFER` 1
  (status enum `Opcodes.h:140-148`).
- Distinct handler methods referenced: **275**, of which 271 are real
  handlers (rest: `Handle_NULL/Handle_ServerSide/Handle_EarlyProccess/
  Handle_Deprecated` stubs). Many opcodes share one handler (all movement
  variants → `HandleMovementOpcodes`).
- By processing class: WORLD 37 + aliases GROUP 17/GUILD 22/CHANNEL 17,
  MAP 133 + SELF_ITEMS 21, MOVEMENT 39, SPELLS 8, ASYNC-alias DB_QUERY 9,
  MAX_TYPE (never queued) 522.

Table mechanics: singleton `Opcodes` with `std::map<u16, OpcodeHandler>`
built once in ctor (`Opcodes.h:162-201`, `Opcodes.cpp:36-46`);
`OpcodeHandler {name, status, packetProcessing, WorldSession::*handler}`
(`Opcodes.h:152-158`). Lookup is by member-function pointer dispatch:
`(this->*opHandle.handler)(*packet)` (`WorldSession.cpp:1055`), wrapped by
delayed-teleport guard (:1047-1071). Unknown opcode ⇒ `emptyHandler`
(STATUS_UNHANDLED/MAX_TYPE, `Opcodes.cpp:33`) ⇒ packet silently dropped in
`QueuePacket` (`WorldSession.cpp:270-278`).

*Rust implication:* replace with a `const [Handler; 828]` array (or
`match` on a repr-u16 enum) carrying {name, status, class}; the
status/class metadata — not the handler bodies — is the concurrency
contract, so port the table first and mechanically. Keep opcode *names*
for log compatibility.

### 6.4 Session queueing & the five processing classes

`WorldSocket` handles exactly two opcodes inline on the IO thread —
`CMSG_PING` (→`SMSG_PONG`, overspeed-ping kick,
`WorldSocket.cpp:442-503`) and `CMSG_AUTH_SESSION` (§7) — everything else
requires `m_Session != nullptr` and is handed off:
`m_Session->QueuePacket(std::move(packet))` (:170-178).

`QueuePacket` (`WorldSession.cpp:252-282`) classifies by opcode-table
`packetProcessing` (with a `CMSG_MESSAGECHAT` sub-classification by chat
type, :220-264) into one of **5 per-session `LockedQueue<unique_ptr<WorldPacket>, mutex>`**
(`WorldSession.h:784`; `LockedQueue` = mutex+deque, unbounded,
`LockedQueue.h:29-90`). Classes (`PacketProcessing.h:25-76`), with their
documented thread-safety contracts — the single most important document
for the Rust concurrency design:

| Class | Drained by | Contract (summary) |
|---|---|---|
| `WORLD` (0) | world thread `UpdateSessions` | not thread-safe; may write anything global |
| `MAP` (1) | owning map's update thread (`Map.cpp:947-950`) | may write current map only; read groups/guilds; `SendPacket` to anyone |
| `SPELLS` (2) | map thread, higher frequency (`Map.cpp:860-866,883-884`) | same as MAP |
| `MOVEMENT` (3) | map thread, higher frequency (`Map.cpp:883`) | same as MAP |
| `ASYNC` (4) | `AsyncPacket` thread (`World.cpp:1936-1962`) | never concurrent with WORLD/CLI; may run during map updates; "Be careful touching the player. Never touch the map." |
| `MAX_TYPE` (5) | — | dropped |

Aliases: SELF_ITEMS→MAP, DB_QUERY→ASYNC, CHANNEL/GROUP/GUILD→WORLD
(`PacketProcessing.h:89-121`).

Drain: `ProcessPackets(filter)` (`WorldSession.cpp:456-555`) pops while
`CanProcessPackets()` (socket open, or bot) and per-packet flood limits
allow (`AllowPacket`), checks `SessionStatus` against player state
(logged-in / transfer / authed / in-queue), then `ExecuteOpcode`.
`MapSessionFilter` additionally requires player in-world
(`WorldSession.cpp:48-65`); because `LockedQueue::next(checker)` peeks
under the same lock (`LockedQueue.h:76-90`), an unprocessable head packet
blocks that queue (head-of-line) rather than being skipped.

Session registry: `WorldSocket` (IO thread) creates `WorldSession` with
`new` and calls `sWorld.AddSession` → `addSessQueue` (LockedQueue)
(`WorldSocket.cpp:417,433`; `World.cpp:237`). Only the world thread moves
it into `m_sessions` map, resolves duplicate-account kicks, login queue
admission (`LoginPerTick`), and `delete`s sessions whose `Update()`
returns false (`World.cpp:2785-2827`). Disconnected-but-in-world sessions
park in `m_disconnectedSessions` for up to 120 s
(`WorldSession.cpp:348-367,564-580`).

*Rust implication:* model each class as a separate channel into its
owning executor (world actor / per-map actor / async worker). The
contracts in `PacketProcessing.h` become `Send` bounds + which state each
executor owns; anything classified MAP today freely reads cross-map state
(groups/guilds) — that shared-read surface is the hard part (needs
snapshots or sharded ownership), not the queues themselves. Note inbound
queues are *unbounded* (only flood-count throttling per update); Rust
should add bounds + drop/kick policy deliberately, it changes behaviour.

## 7. SRP auth flow (client → realmd → client → mangosd)

State machine per `AuthSocket`, single IO thread, 8 commands gated by
`m_status` (`AuthSocket.cpp:117` table; states `AuthSocket.h:86-94`):

1. `CMD_AUTH_LOGON_CHALLENGE` (STATUS_CHALLENGE → set INVALID while
   processing, `AuthSocket.cpp:198-206`): parse packed challenge
   (build/platform/os/locale/username, `AuthPackets.h:15-35`), look up
   account, load verifier `v` + salt `s` into `SRP6`
   (`srp.SetVerifier/SetSalt`, :367), `CalculateHostPublicEphemeral` (:402),
   reply B, g, N, s (:408-412); status → LOGON_PROOF (:446).
2. `CMD_AUTH_LOGON_PROOF`: read client A(32)+M1(20)+crc(20) [+PIN block if
   securityFlags] (`AuthPackets.h:54-79`; `AuthSocket.cpp:469-507`), then
   `_HandleLogonProof__PostRecv` (:577): build check → patch-download
   diversion if invalid (:582-589, STATUS_PATCH + XFER protocol :511-576),
   `CalculateSessionKey(A)`/`HashSessionKey`/`CalculateProof` (:592-599),
   optional PIN/TOTP/geolock verification (:601-636), `srp.Proof(M1)`
   (:639). **Quirk to preserve-or-fix consciously:** success branch is
   `if (!srp.Proof(...) && pinResult)` (:639) — `Proof` is inverted vs its
   own doc: it returns `false` on memcmp match
   (`Crypto/Authentication/SRP6.cpp:156-164` vs `SRP6.h:74-80`), so
   `!Proof` means "proofs match". Do not port this condition literally.
3. On success: synchronous `UPDATE account SET sessionkey=K,…` (:704-707,
   see §1), `srp.Finalize()` → M2 proof packet (build-dependent layout,
   :156-195), status → AUTHED (:714-719). Wrong password: failed-login
   counter/ban logic from :721.
4. `CMD_REALM_LIST` (AUTHED only): serialises cached `RealmList`
   (`_HandleRealmList`, :943), picking local vs external address by subnet
   (`RealmList.cpp:285-293`). Client now disconnects from realmd.
5. Reconnect variant: `CMD_AUTH_RECONNECT_CHALLENGE` reloads stored
   `sessionkey` (:853-864), `CMD_AUTH_RECONNECT_PROOF` checks SHA1 over
   login+R1+challenge+K (:883-930) → AUTHED without password.
6. World leg: mangosd sends `SMSG_AUTH_CHALLENGE` with random `m_authSeed`
   on connect (`WorldSocket.cpp:66,505-514`). Client replies
   `CMSG_AUTH_SESSION` {build, serverId, account, clientSeed, digest}
   (:238-243). `WorldSocket::_HandleAuthSession` (:223-440, IO thread,
   **does blocking sync DB queries on the IO thread**) verifies:
   acceptable build (:248), account row incl. bans/GM level (:263-282),
   `last_ip` match (:300), `SHA1(account ‖ 0 ‖ clientSeed ‖ serverSeed ‖
   K) == digest` (:353-374, K = `account.sessionkey` hex from realmd step
   3), then creates `WorldSession`, keys header crypto
   `m_Crypt.SetKey(K); Init()` (:417-420), loads account/tutorial data
   (more sync DB), `sWorld.AddSession` (:433), sends addon packet.

Header crypto after auth (`AuthCrypt.cpp:29-74`): byte-wise feedback
cipher over first 6 (recv) / 4 (send) header bytes only, state `(i,j)`
per direction, keyed by raw K bytes. Stateful and order-sensitive —
in Rust it lives inside the per-connection codec (`Framed`), never shared.

*Rust implication:* SRP6 + BigNumber (OpenSSL BN today, see
`Crypto/Authentication/SRP6.h:27-115`) needs a vetted Rust equivalent
(`srp` crate uses different group conventions — verify N/g/k and the
WoW-specific little-endian interleave in `HashSessionKey` against test
vectors from the C++ code before trusting it). The verifier/salt (`v`,`s`)
in the production login DB pin the exact variant. Also note step 6 does
*synchronous* DB on an IO thread today; Tokio port should `await` it (or
spawn_blocking) — that changes auth latency under load, for the better,
but keep the one-packet-at-a-time ordering per socket.

## 8. Config, DB threading at the boundary, other queues

**Config (`sConfig`):** custom INI-ish parser, `unordered_map<string,string>`
behind `shared_timed_mutex` (shared-lock reads, exclusive reload)
(`Config.h:27-57`, `Config.cpp:36-59,157-200`). Typed getters with
code-site defaults everywhere (`GetIntDefault("Network.Threads",1)` style);
`#`/quote handling, duplicate key = first wins (`Config.cpp:146-150`).
Two layers: raw `sConfig` strings, and World's typed snapshot arrays
(`m_configUint32Values[]` etc., `World.h:961-964`) filled by
`LoadConfigSettings()` with min/max clamping (`World.cpp:2891+`).
`Reload()` exists but runtime reload is partial (many values read once at
startup; `configNoReload` guards, `World.h:935-938`). No env-var layer in
the daemons proper. *Rust:* keep the
file format + key names + defaults byte-compatible (production confs are
gitignored, formats in `*.conf.dist.in`); `Arc<Config>` snapshot swap
replaces the rwlock map; preserve first-wins duplicates and clamp rules.

**DB threading (boundary only; deep dive is another report):** per
`Database`: pool of sync query connections (locked per connection) + 1
async connection + `nWorkers` `SqlDelayThread`s, each FIFO
`LockedQueue<SqlOperation*>` with its own connection
(`Database.cpp:141-191,218-231`; `SqlDelayThread.h:29-61`), plus a
6-thread `SqlResultQueue` callback pool (`SqlOperations.cpp:151`).
World thread drains results via `ProcessResultQueue(maxTime)` each tick
(`World.cpp:2850-2856`) — callbacks therefore execute on SqlResult threads
*or* are marshalled back, depending on API used; audit per call site when
porting to `sqlx`/pool + oneshot channels. `MustBeSync` escape hatch
(`Database.cpp:402`) exists and is used for the sessionkey write (§1).

**Every cross-thread queue in scope:**

| Queue | Producer → consumer | Sync |
|---|---|---|
| `WorldSession::m_recvQueue[5]` (`WorldSession.h:784`) | IO → world/map/async | mutex+deque, unbounded |
| `WorldSocket::m_sendQueue` (`WorldSocket.h:101-103`) | any → IO drainer | mutex + atomic_flag, cap 1024 |
| `World::addSessQueue` (`World.h:1008`) | IO → world | LockedQueue |
| `World::cliCmdQueue` (`World.h:1001`) | CLI/RA/SOAP threads → world (`World.cpp:2830-2846`) | LockedQueue |
| `World::Messager` (`Messager.h:34-52`) | any → world (executed :2004) | mutex + swap-vector |
| `World::_asyncTasks` (`World.h:870-872`) | world → WorldUpdate pool | mutex + double-buffer swap |
| `IoContext::m_contextSwitchQueue` (`IoContext.h:77-78`) | any → IO threads (eventfd wakeup, `IoContext_linux.cpp:99-105`) | mutex+queue |
| `SqlDelayThread::m_sqlQueue` + serial queue | any → DB delay thread | LockedQueue |
| `SqlResultQueue` | delay threads → callback pool / world drain | ThreadPool queues |
| `ThreadPool` workloads (`ThreadPool.h:210-211` atomics) | world/mapmgr → workers, fork-join via `future` | per-worker queues |
| Timer deque (`AsyncSystemTimer.h:65-68`) | any → SystemTimer thread | mutex+condvar |
| MovementBroadcaster shards | world/maps → N workers | `shared_timed_mutex` per shard (`MovementBroadcaster.cpp:29`) |

**Flags instead of synchronisation (must become atomics in Rust):**
`volatile bool World::m_stopEvent` (`World.h:940`), `volatile uint32
m_worldLoopCounter` (:683), `volatile bool IoContext::m_isRunning`
(`IoContext.h:67`), `volatile bool stopEvent` in realmd
(`realmd/Main.cpp:77`), plain `bool m_canProcessAsyncPackets` written by
world thread + read by AsyncPacket thread (only the accompanying mutex
makes this sound-ish, `World.cpp:1941-1961,2001-2013`), `ThreadPool`
`volatile bool busy`. Raw cross-thread pointers: `WorldSocket::m_Session`
(§5), `World::m_sessions` values read by AsyncPacket/Map threads while
world thread owns insert/delete (`World.cpp:1949,2792-2812`) — the AsyncPacket
fence covers session *update* but iteration vs deletion relies on that
same fence; preserve the fence semantics exactly or switch to
`Arc<Session>` + snapshot iteration.

## 9. Signals, shutdown, freeze/watchdog, anticrash

- mangosd signals `Master::_OnSignal` (`Master.cpp:518-570`, hooked
  :572-581): SIGINT → `StopNow(RESTART_EXIT_CODE)`, SIGTERM/SIGBREAK →
  `StopNow(SHUTDOWN_EXIT_CODE)`; handler just sets the volatile stop flag
  (+re-arms itself) — all work happens via loop exit. SIGSEGV →
  "anticrash": log stacktrace, optional fork+abort coredump (:500-508),
  optional announce + `SaveAllPlayers` + 25 s sleep, then rethrow/crash
  for real (:531-566); re-arm timer processed in WorldRunnable
  (`WorldRunnable.cpp:76-92`). This is deliberately unsafe signal-handler
  work (DB + sleeps in handler); Rust should *not* replicate it — use
  panic=abort + external supervisor + periodic saves, and call the
  behaviour change out in the guide.
- realmd signals: SIGINT/SIGTERM/SIGBREAK → `stopEvent=true`
  (`realmd/Main.cpp:378-394`); no SIGSEGV handler.
- In-game shutdown: `World::ShutdownServ` countdown (announces, then
  `StopNow`), `ShutdownCancel` (`World.cpp:2664-2722`) — triggered via
  CLI/GM commands through `cliCmdQueue`.
- mangosd teardown order (main thread after world join,
  `Master.cpp:306-396`): unhook signals → join freeze/SOAP → mark realm
  offline (sync DB) → stop world networking → stop RA → stop timers →
  `ioCtx->Shutdown()` + join IO threads → clear online accounts → flush
  mass mail → `StopServer()` ×4 DBs (joins delay threads) → unblock+join
  CLI (closes stdin on POSIX :392) → return `World::GetExitCode()`.
  World-thread tail (save/kick, map unload) happens before all this
  (§4.1). realmd teardown: close listener → stop timers → shutdown
  IoContext + join → halt DB delay thread → unhook signals
  (`realmd/Main.cpp:358-373`).
- Freeze detector `freezeDetector()` (`Master.cpp:69-100`): 1 s poll of
  `m_worldLoopCounter`; if unchanged for `MaxCoreStuckTime` (default 0 =
  disabled; only in `NDEBUG` builds :90-96) → `std::terminate()`. No
  stack capture, no graceful save. *Rust:* equivalent = watchdog task
  watching world-actor heartbeat timestamp; decide terminate-vs-dump
  policy explicitly (current default is OFF in the dist config).

## 10. Rust async/concurrency boundary — summary

1. **Keep the actor topology, change the mechanism.** IO tasks (Tokio) →
   per-session classified channels → world actor (single task, 50 ms
   `interval`) + per-map executors (fork-join scoped tasks) + one gated
   async-packet worker → per-DB ordered writer queues. Every arrow exists
   in C++ today (§8 table); none need inventing.
2. **Single-threaded assumptions to encode as ownership:** `World`,
   `m_sessions`, session create/delete, CLI execution, MAP-class writes
   per map, `AuthSocket`/`WorldSocket` per-connection state (incl.
   `AuthCrypt` counters). Shared-read exceptions (groups/guilds/loot read
   from map threads, `SendPacket` to any session) are the audit list.
3. **Replace, don't transliterate:** `volatile` flags → `AtomicBool/
   AtomicU32` + `CancellationToken`; `LockedQueue` → `mpsc` (decide
   boundedness — today unbounded inbound, 1024-cap outbound);
   `shared_from_this` callback chains → connection tasks owning socket
   halves; raw `WorldSession*` in socket → channel handle;
   `ByteBufferException` → `Result<T, ParseError>` at the same 2 catch
   boundaries; address-pinned epoll objects → nothing (Tokio owns fds).
4. **Preserve externally visible behaviour:** 20 Hz tick + intra-tick
   phase order (§4.1), one-packet-in-flight ordering per socket, send
   coalescing on IO side, sessionkey sync-write before realm list,
   login-queue admission on world thread, config keys/defaults, exit
   codes (0/1/2-restart), `realmlist` online/offline writes.
5. **Deliberate divergences to flag in the guide:** SIGSEGV anticrash
   (drop), inline-on-caller-thread IO callbacks (gone under Tokio),
   sync DB on IO thread during auth (becomes async), freeze detector
   default-off terminate (re-decide), unbounded inbound queues (bound them).

## Unresolved / not inspected

- `SRP6` internals (`SRP6.cpp` interleave/hash details) and BigNumber —
  only the header + call sites were read; needed for test vectors.
- Windows IOCP + Winsock impls (`*_windows.cpp`) and `ServiceWin32` —
  skimmed via headers only; irrelevant if Rust targets Linux only.
- `ThreadPool` work-stealing details and `Map.cpp` internal sub-pool
  scheduling beyond spawn sites/config keys.
- Live production config values (thread counts actually deployed) — only
  `*.conf.dist.in` defaults were read, per secret-hygiene rules.
- SOAP/gSOAP surface beyond "thread + QueueCliCommand".


