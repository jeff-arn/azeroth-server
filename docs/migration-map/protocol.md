# Protocol map: 1.12.1 opcodes, flows, session establishment

Bead: `as-pnw.2`. Static map of the 1.12.1 protocol surface as owned by the
C++ opcode handler tables. No live capture was taken; dynamic verification is
recorded as follow-ups at the end, not executed here.

## Ground-truth files

| File | Owns |
|------|------|
| `vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h` | `enum OpcodesList`: all opcode names + discriminants for 1.12.1 |
| `vmangos/src/game/Server/Protocol/Opcodes.h` | `OpcodeHandler{name, status, packetProcessing, handler}`, `SessionStatus`, `opcodeTable` singleton |
| `vmangos/src/game/Server/Protocol/Opcodes.cpp` | `BuildOpcodeList()`: the 826-row gate table (status + affinity + handler per opcode) |
| `vmangos/src/game/PacketProcessing.h` | `PacketProcessing`: thread-affinity enum for queued packets |
| `vmangos/src/game/Server/WorldSocket.cpp` | World wire format, `SMSG_AUTH_CHALLENGE` / `CMSG_AUTH_SESSION` handshake, `CMSG_PING` fastpath, malformed-packet guard |
| `vmangos/src/game/Server/WorldSession.cpp` | `QueuePacket` (affinity queues), `Update`/`ProcessPackets`, logout sequencing |
| `vmangos/src/game/World.cpp` (`World::UpdateSessions`) | Per-tick session pump, login queue, disconnect handling |
| `vmangos/src/game/Handlers/CharacterHandler.cpp` | Char enum/create/delete, player login, `SMSG_LOGIN_VERIFY_WORLD`, account data, tutorial flags |
| `vmangos/src/game/Handlers/MovementHandler.cpp` | `HandleMovementOpcodes`: all `MSG_MOVE_*` / `CMSG_MOVE_*` |
| `vmangos/src/game/Handlers/*Handler.cpp` | One handler file per subsystem (chat, spell, group, guild, …) |
| `vmangos/src/realmd/AuthSocket.cpp` | realmd command dispatch table (logon / reconnect / realm list / patch) |
| `vmangos/src/realmd/AuthCodes.h` | `eAuthCmd`, `AuthResult`, `AccountFlags` |
| `vmangos/src/realmd/AuthPackets.h` | SRP6 logon structs (`sAuthLogonChallengeBody`, `sAuthLogonProof_C`, `PINData`) |

Older-client opcode headers (`Opcodes_1_5_1.h` … `Opcodes_1_11_2.h`) exist for
reference but 1.12.1 is the selected build (`SUPPORTED_CLIENT_BUILD` picks
`Opcodes_1_12_1.h`). Several `StoreOpcode` rows are additionally gated on
`SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_8_4 / 1_9_4 / 1_5_1` — a Rust port
must model these as build-selected variants, not unconditional entries.

## Census (1.12.1, from code — counted, not estimated)

- `enum OpcodesList`: **827 named opcodes** — CMSG 386, SMSG 360, MSG 81.
  Values cover 0–825 plus 827; **value 826 is unassigned**;
  `NUM_MSG_TYPES = 828`.
- `BuildOpcodeList()`: **826 `StoreOpcode` rows** (plus the `emptyHandler`
  fallback `{"<none>", STATUS_UNHANDLED, …}` for unknown opcodes).
- By gate status: `STATUS_NEVER` **525**, `STATUS_LOGGEDIN` **290**,
  `STATUS_AUTHED` **8**, `STATUS_TRANSFER` **1**
  (`MSG_MOVE_WORLDPORT_ACK`), `STATUS_LOGGEDIN_OR_RECENTLY_LOGGEDOUT` **2**.
- By thread affinity: `PACKET_PROCESS_MAP` **133**, `MOVEMENT` **39**,
  `WORLD` **38** (+ `CHANNEL` 17 / `GROUP` 17 / `GUILD` 22, all aliases of
  WORLD), `DB_QUERY` **9**, `SPELLS` **8**, `SELF_ITEMS` 21 (alias of MAP),
  `MAX_TYPE` **523** (no queue — server-side / unimplemented).
- By handler: `Handle_ServerSide` **359** (server-originated, never accepted
  from client), `Handle_NULL` **163** (ignored), `Handle_EarlyProccess` **2**
  (`CMSG_PING`, `CMSG_AUTH_SESSION` — handled on the socket, never queued),
  `Handle_Deprecated` etc. for the remainder.

## Gate model (`SessionStatus`)

The table is a whitelist: an opcode is processed only if the session is in the
required state; anything else is rejected before the handler runs.

- `STATUS_AUTHED` (no player in world yet) — exactly 8 opcodes:
  `CMSG_CHAR_CREATE`, `CMSG_CHAR_ENUM`, `CMSG_CHAR_DELETE`,
  `CMSG_PLAYER_LOGIN`, `CMSG_GUILD_QUERY`, `CMSG_SET_ACTIONBAR_TOGGLES`,
  `CMSG_CHAR_RENAME`, `CMSG_WARDEN_DATA`.
- `STATUS_LOGGEDIN` (player in world) — the bulk: movement, chat, combat,
  queries, trade, group/guild actions.
- `STATUS_TRANSFER` — only `MSG_MOVE_WORLDPORT_ACK` (inter-map teleport ack).
- `STATUS_NEVER` — never accepted from the client: all SMSGs, obsolete/
  debug/cheat opcodes, and the SRP6 world-table leftovers
  (`CMSG_AUTH_SRP6_BEGIN/PROOF/RECODE` are dead rows — real auth happens in
  realmd + `CMSG_AUTH_SESSION`).

## Thread affinity (`PacketProcessing`)

`WorldSession::QueuePacket` sorts each incoming packet into
`m_recvQueue[packetProcessing]`; `WorldSession::Update` (driven by
`World::UpdateSessions` each tick, `World.cpp:2751`) drains the queues via
`ProcessPackets`. The aliases are load-bearing for a port — they are distinct
names for the same queue:

- `PACKET_PROCESS_WORLD = 0` — not thread-safe; run in `UpdateSessions`.
  `CHANNEL`, `GROUP`, `GUILD` are aliases (world-locked shared state).
- `PACKET_PROCESS_MAP` — current-map safe (read map objects, read groups/
  guilds/loots, send to any player). `SELF_ITEMS` is an alias.
- `PACKET_PROCESS_SPELLS`, `PACKET_PROCESS_MOVEMENT` — same safety as MAP,
  drained more frequently.
- `PACKET_PROCESS_ASYNC` — whenever the session is not updating; never touch
  the map. `DB_QUERY` is an alias (static-data reads).
- `PACKET_PROCESS_MAX_TYPE` — no queue; packet has no client handler.

## Establishment flow 1: auth + realm list (realmd, TCP 3724)

Dispatch is a fixed 8-entry table in `AuthSocket.cpp:117`, gated by
connection state (`STATUS_CHALLENGE → LOGON_PROOF → AUTHED`, plus
`RECON_PROOF` and `PATCH` for the patch path):

1. Client `CMD_AUTH_LOGON_CHALLENGE` (`0x00`) with game name, version triplet,
   build, platform/OS/country, timezone, IP, username
   (`sAuthLogonChallengeBody`, `AuthPackets.h`).
2. Server replies with SRP6 parameters (B, g, N, salt s).
3. Client `CMD_AUTH_LOGON_PROOF` (`0x01`) with A, M1, crc_hash, key count
   (+ `securityFlags`/PIN for post-1.11 clients); server verifies and returns
   `AuthResult` (`WOW_SUCCESS`, `WOW_FAIL_*` in `AuthCodes.h`).
4. Client `CMD_REALM_LIST` (`0x10`); server returns the realm list
   (`RealmList.cpp`); client picks a realm and connects to mangosd.
5. Reconnect uses `CMD_AUTH_RECONNECT_CHALLENGE/PROOF` (`0x02/0x03`); client
   patching uses `CMD_XFER_*` (`0x30–0x34`, `ClientPatchCache.cpp`).

The `account.last_ip` written here is load-bearing for the next stage:
mangosd rejects `CMSG_AUTH_SESSION` when the source IP differs from the IP
that authed at realmd (localhost/own-hostname exempt).

## Establishment flow 2: world session (mangosd, TCP 8085)

Wire format (`WorldSocket.cpp`): server header `{size, cmd}` (size
big-endian-reversed, header RC4-encrypted post-auth); client header decrypted
on receipt; hard guard `size < 4 || size > 0x2800 || cmd >= NUM_MSG_TYPES`
logs and drops (`WorldSocket.cpp:105` — see scanner note below).

1. On connect the server sends `SMSG_AUTH_CHALLENGE` with `m_authSeed`
   (`SendInitialPacketAndStartRecvLoop`), and starts the
   `Network.TimeoutSecsIfNoAuth` timer (default 10 s — unauthed sockets die).
2. Client answers `CMSG_AUTH_SESSION` with client build, server id, account
   name, client seed, and `SHA1(account, 0, clientSeed, authSeed, K)` where K
   is the SRP6 session key. This is one of only two opcodes handled inline on
   the socket (`_HandleCompleteReceivedPacket`); anything else pre-auth is
   dropped with "Client not authed".
3. Server validates, in order: client build (`SMSG_AUTH_RESPONSE` +
   `AUTH_VERSION_MISMATCH`), account/session-key lookup in the `realmd`
   `account` table, source-IP match, ban check, security vs server limit,
   SHA1 digest match. Any failure sends the matching `SMSG_AUTH_RESPONSE`
   code and closes.
4. On success a `WorldSession` is created, the crypt key is set from K, global
   account data + tutorials load, the session joins `sWorld`, and the addon
   packet goes out. Over-capacity sessions park in the login queue
   (`SMSG_AUTH_WAIT_QUE`, drained in `UpdateSessions`).

## Establishment flow 3: char screen → enter world

- `CMSG_CHAR_ENUM` → async characters query → `SMSG_CHAR_ENUM`;
  `CMSG_CHAR_CREATE` / `CMSG_CHAR_DELETE` similarly (all `STATUS_AUTHED`,
  `PACKET_PROCESS_WORLD`).
- `CMSG_PLAYER_LOGIN` (guid) → async `LoginQueryHolder` load. On
  `HandlePlayerLogin`: re-attach if already online, `LoadFromDB`, then
  `SMSG_LOGIN_VERIFY_WORLD` (map + position), account-data times
  (`SMSG_ACCOUNT_DATA_MD5`), friend/ignore lists, MOTD rendered as chat
  messages (**1.12.1 has no `SMSG_MOTD`** — `CharacterHandler.cpp:556`),
  guild MOTD event, then the initial world dump (spells, talents, action
  buttons, update objects).
- Transfers: `SMSG_NEW_WORLD` + `SMSG_TRANSFER_PENDING` / `SMSG_TRANSFER_ABORTED`;
  the client acks with `MSG_MOVE_WORLDPORT_ACK` (the lone `STATUS_TRANSFER`
  opcode). Logout: `CMSG_LOGOUT_REQUEST` → `SMSG_LOGOUT_RESPONSE` →
  `SMSG_LOGOUT_COMPLETE`, cancellable via `CMSG_LOGOUT_CANCEL`.
- `CMSG_PING` never reaches a session: answered inline with `SMSG_PONG`,
  with overspeed-ping kick for players (`_HandlePing`).

## Main packet flows (handler homes)

All CMSG handlers are `WorldSession::Handle*` methods; the files below are
where the logic lives:

| Flow | Representative opcodes | Handler file |
|------|------------------------|--------------|
| Movement (68 `*MOVE*` names; 39 `MOVEMENT`-affinity rows) | `MSG_MOVE_HEARTBEAT`, `MSG_MOVE_START_FORWARD`, `MSG_MOVE_WORLDPORT_ACK`, `CMSG_MOVE_*_ACK` | `Handlers/MovementHandler.cpp` (`HandleMovementOpcodes`) |
| Chat / channels | `CMSG_MESSAGECHAT`, `CMSG_JOIN_CHANNEL`, `CMSG_CHANNEL_*` | `Handlers/ChatHandler.cpp`, `ChannelHandler.cpp` |
| Spells / combat | `CMSG_CAST_SPELL`, `CMSG_ATTACKSWING`, `CMSG_ATTACKSTOP` | `Handlers/SpellHandler.cpp`, `CombatHandler.cpp` |
| Queries (all `DB_QUERY`/async) | `CMSG_NAME_QUERY`, `CMSG_ITEM_QUERY_SINGLE`, `CMSG_CREATURE_QUERY`, `CMSG_WHO` | `Handlers/QueryHandler.cpp`, `MiscHandler.cpp` |
| Group / guild | `CMSG_GROUP_INVITE/ACCEPT`, `CMSG_GUILD_CREATE/ROSTER` | `Handlers/GroupHandler.cpp`, `GuildHandler.cpp` |
| Items / loot / trade | `CMSG_LOOT`, `CMSG_AUTOSTORE_LOOT_ITEM`, `CMSG_BEGIN_TRADE` | `Handlers/ItemHandler.cpp`, `LootHandler.cpp`, `TradeHandler.cpp` |
| Quests / NPC / taxi | `CMSG_QUESTGIVER_ACCEPT_QUEST`, `CMSG_GOSSIP_HELLO`, `CMSG_ACTIVATETAXI` | `Handlers/QuestHandler.cpp`, `NPCHandler.cpp`, `TaxiHandler.cpp` |
| Mail / auction / tickets | `CMSG_SEND_MAIL`, `CMSG_AUCTION_LIST_ITEMS`, `CMSG_GMTICKET_CREATE` | `Handlers/MailHandler.cpp`, `AuctionHouseHandler.cpp`, `GMTicketHandler.cpp` |
| Duel / pet / skill | `CMSG_DUEL_ACCEPTED`, `CMSG_PET_ACTION`, `CMSG_LEARN_TALENT` | `Handlers/DuelHandler.cpp`, `PetHandler.cpp`, `SkillHandler.cpp` |
| Addons | `CMSG_REGISTER_ADDON_PREFIXES` (feeds `sAddOnHandler.BuildAddonPacket`) | `Handlers/AddonHandler.cpp` |
| Warden / anticheat | `CMSG_WARDEN_DATA` (AUTHED-gated), `CMSG_BOT_DETECTED2` (dead row) | `Anticheat/WardenAnticheat/Warden.cpp` |

## Scanner-noise filter

`WorldSocket::DoRecvIncomingData: client sent malformed packet …` volume is
background radiation: internet scanners open TCP 8085 and send non-WoW bytes,
tripping the `size/cmd` guard before any session exists. Likewise
"Client not authed" and repeat-`CMSG_AUTH_SESSION` errors are probe/ retry
artifacts. When mining `Server.log` for protocol work, restrict to lines with
an authenticated account id and ignore pre-auth socket errors.

## `mangos-opcodes` crate gap list

**No Rust crate exists in this workspace** (no `*.rs`, no `Cargo.toml`
anywhere under the rig): there is nothing to validate incrementally, so the
gap is the full surface. A `mangos-opcodes` crate earning its name must
contain, at minimum:

1. Opcode enum: all 827 names with exact discriminants 0–825 + 827,
   `NUM_MSG_TYPES = 828`, and value 826 reserved/unassigned.
2. Direction per opcode (CMSG client→server / SMSG server→client / MSG both).
3. Gate table: status per opcode (`NEVER` 525 / `LOGGEDIN` 290 / `AUTHED` 8 /
   `TRANSFER` 1 / `LOGGEDIN_OR_RECENTLY_LOGGEDOUT` 2), including the 8-entry
   `AUTHED` allowlist above — this is the session-state machine input.
4. Affinity table: `PacketProcessing` per opcode **with the aliases preserved**
   (`SELF_ITEMS`→MAP, `DB_QUERY`→ASYNC, `CHANNEL/GROUP/GUILD`→WORLD), plus the
   drain-frequency distinction for `SPELLS`/`MOVEMENT`.
5. Handler cross-ref: real handler name vs `Handle_ServerSide` (359) vs
   `Handle_NULL` (163) — the latter two define out-of-scope for the port.
6. Build-conditioned rows: opcodes gated on `SUPPORTED_CLIENT_BUILD`
   (`SMSG_CHECK_FOR_BOTS`, `CMSG_BOT_DETECTED2`, `CMSG_PETGODMODE`, …) must be
   feature/build-selected, not unconditional.
7. Non-world-table protocol: `eAuthCmd`/`AuthResult`/`AccountFlags`
   (`AuthCodes.h`), SRP6 struct layouts (`AuthPackets.h`), the
   `SMSG_AUTH_CHALLENGE` → `CMSG_AUTH_SESSION` field order and SHA1
   construction, and the `{size, cmd}` header with endian/RC4 handling.

## Dynamic follow-ups (recorded, not executed)

Per epic instructions these need a live client + capture; none were run here:

- FU-1: Confirm the realmd logon/reconnect byte sequences against a live
  capture (challenge struct, proof struct with PIN/securityFlags, realm-list
  encoding).
- FU-2: Confirm the mangosd enter-world sequence order
  (`SMSG_AUTH_RESPONSE` → addon packet → `SMSG_CHAR_ENUM` → … →
  `SMSG_LOGIN_VERIFY_WORLD` → account-data/times → tutorial flags → motd →
  object dump) and which steps the 1.12.1 client blocks on.
- FU-3: Verify the `CMSG_AUTH_SESSION` SHA1 digest construction byte-for-byte
  (field order, padding, endianness) against a captured login.
- FU-4: Verify transfer sequencing (`SMSG_NEW_WORLD` / `TRANSFER_PENDING` /
  client `MSG_MOVE_WORLDPORT_ACK`) across a real map change.
- FU-5: Sample movement-ack traffic (`CMSG_MOVE_*_ACK`, speed-change flows)
  to confirm server-authoritative movement message order.
