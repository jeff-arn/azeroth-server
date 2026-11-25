# Phase 5 Progress — Services, Then Bots

Goal: Phase 5 per `docs/RUST_MIGRATION_ARCHITECTURE.md` §13.2, on branch
`rust-migration` after Phase 4 closeout commit
`d45a70cf4a4f67fad95ee6487b43658a216db5f3`.

Mixed-model routing (mandatory): `muse-wonder-full-1.0` is the driver and
builds every first-of-kind slice itself; `muse-spark-1.3` only via Workflow
for mechanical transcription after a wonder-built exemplar exists; verification
never uses a weaker model than the producer. Writes stay in `vmangos-rs/` and
`docs/` plus local dev-bench fixtures. Character save remains Phase 6.

## Inventory at Phase 5 start (verified 2026-10-18)

Already in Rust:

- `mangos-bots` islands that need no Host API: `formation`, `breadcrumb`,
  `combat_events` + `patterns`, `talents` + `talent_builds` (27 tests).
  No behaviour runner, Host API, chat/services, director, guild, or AH yet.
- `mangosd` login surface reads social state: `character_social` rows load
  into `SocialRow` and the login train sends friend/ignore lists. No service
  mutation handlers exist; service opcodes currently appear only in
  `session_core` flood-limit tables and `CMSG_MESSAGECHAT` rerouting.
- Static queries (name/creature/gameobject/item) are done; item prototypes,
  factions, areas, and weather load at boot.

C++ service surface to port (handler file sizes, `vmangos/src/game/Handlers/`):
MiscHandler 1288 (friends/ignore/who are here), MailHandler 935,
GroupHandler 901, ChatHandler 867, AuctionHouseHandler 841, GuildHandler 790,
TradeHandler 786, ChannelHandler 312.

## Slice order

- **Slice A (exemplar, wonder-built): friends/ignore mutation.**
  `CMSG_ADD_FRIEND`, `CMSG_DEL_FRIEND`, `CMSG_ADD_IGNORE`,
  `CMSG_DEL_IGNORE` from MiscHandler + `character_social` writes on the bench
  characters DB, `SMSG_FRIEND_STATUS` responses, name-to-guid resolution, and
  loopback + scripted-client verification. This reuses the existing login
  friend/ignore read path as the oracle (log out, log in, lists reflect the
  mutation) and establishes the service-handler pattern (parse, DB service
  call, session state update, response) for later spark transcription.
- **Slice B: remaining services** in dependency order: who list, chat
  channels + message chat routing, group/party, guild, mail, trade, auction
  house. Each follows the Slice A pattern with its own C++ contract pass.
- **Slice C: GM command table/permissions contract** (the 23.6k LOC command
  bodies trail per the architecture plan).
- **Slice D: `mangos-bots` Host API + behaviour runner** in `mangosd`,
  scoring/fall-through semantics matching `BotBehaviorSet`.
- **Slice E: behaviours, formation wiring, chat/services scheduler,
  director/guild/presence.**
- **Slice F: AH bot and the BattleBot engine decision** (dual-engine vs
  behaviours; BG objectives have no behaviour equivalent today).

## Slice A — friends/ignore mutation (complete, 2026-10-18)

Implemented `CMSG_ADD_FRIEND` / `CMSG_DEL_FRIEND` / `CMSG_ADD_IGNORE` /
`CMSG_DEL_IGNORE` from `MiscHandler.cpp:488-599` + `SocialMgr.cpp`:

- `crates/mangosd/src/social.rs` — pure half: `FriendsResult` codes,
  `normalize_player_name`, list mutation with the 50/25 limits, the
  `SocialWrite` statement choice, `SMSG_FRIEND_STATUS` builder.
- `crates/mangos-db/src/social.rs` — name lookup plus the four
  `character_social` statements, chosen by in-memory state exactly as
  `PlayerSocial` does (the PK is `(guid, friend, flags)`, so no upsert).
- Social state lives on the Player (`DeferredLoad.social`, taken from the
  character load at materialize); handlers in the world actor follow the
  query-handler routing pattern. `AllowTwoSide.AddFriend` and
  `SEC_MODERATOR` wired into `ServerConfig`/`security`.
- C++ quirks preserved: unknown or unnormalizable names get no reply;
  delete always replies; list-full replaces the added result.
- Deviations: no AFK/DND or GM-visibility gates (no Rust session state
  for them yet); online = player present in a map. DB writes are
  synchronous `DbActor` calls whose errors only log (C++ `PExecute`
  queue semantics).

Evidence: unit tests in `social.rs`; loopback `assert_social_flow`
(already/removed/added-offline/ignore/enemy/self/ignore-removed);
`live_social` DB round-trip on bot fixtures; `./ci.sh` exit 0; bench
restarted and a scripted probe passed all nine cases as Ruikanis
(including the JEFF `SEC_MODERATOR` two-side bypass) with bench
`character_social` restored; OrbStack stock client: Groktar added and
removed Morka through the Friends UI ("Morka added to friends." /
"Morka removed from friends list."), DB row observed and deleted,
artifacts `/private/tmp/wowrig/artifacts/slicea-*.png`. The
`two_client_smoke.py` relay timeout flaked twice in a row then passed
— the same intermittent failure seen on the pre-slice binary at
Phase 4 closeout, not a Slice A regression; worth a later look at
relay-vs-visibility timing.

## Slice B.1 — who list (complete, 2026-10-18)

Implemented `CMSG_WHO` / `SMSG_WHO` from
`MiscHandler.cpp:60-367` (`WhoListClientQueryTask`):

- `crates/mangosd/src/who.rs` — request parser (silent drop for
  over-limit zone/string counts), the pure filter in C++ check order
  (player-security team/GM gates, level range with the
  `MAX_LEVEL`→255 rule, race/class masks as `1 << id`,
  battleground-instance scoping, zone list, name/guild substrings,
  zone-name search strings, 49-entry cap) and the response builder
  with the C++ online-count rule.
- Area display names now load from `area_template.name` into
  `AreaStore` for the search-string arm.
- Configs wired: `AllowTwoSide.WhoList` (false),
  `GM.InWhoList.Level` (`SEC_ADMINISTRATOR`).
- Deviations: synchronous execution (no one-in-flight flag), guild
  names empty until the guild service, no GM-invisibility or
  bot-hiding gates (no such Rust state yet).

Evidence: unit tests in `who.rs`; loopback `assert_who_replies`
(exact entry bytes + empty filter result); `./ci.sh` exit 0; live
probe as Ruikanis (unrestricted + name filter + `westfall`
zone-name search + non-match); OrbStack stock client Who tab +
Refresh lists "Groktar — Durotar — 35 Warrior / 1 Person Found"
(artifacts `sliceb-who-*.png`). The tab does not auto-query on
switch in this build; Refresh sends the request.

## Slice B.2a — chat core (complete, 2026-10-18)

Implemented `HandleMessagechatOpcode`
(`ChatHandler.cpp:149-724`) for SAY, YELL, EMOTE, WHISPER,
AFK, and DND; party/guild/raid/channel/battleground types
drop exactly where C++ drops them for a player with no
group, guild, or channels (services land in B.2b+):

- `crates/mangosd/src/chat.rs` — `CMSG_MESSAGECHAT` parser,
  `BuildChatPacket` shapes (SAY/PARTY/YELL repeat the
  sender guid), language-spell table, `GetYellRange`.
- `Map::broadcast_packet_in_range` for say/yell/emote with
  C++ listen ranges and the emote team gate; whisper with
  inform echo, not-found/wrong-faction notices, staff
  accept-whispers gate, and AFK/DND session state
  (chat tag + stored replies + auto-reply on whisper).
- Configs wired: `ListenRange.Say/Yell/TextEmote`,
  `YellRange.*`, `AllowTwoSide.Interaction.Chat`.
- Language knowledge is spell-based (stored spells +
  creation defaults), matching `KnowsLanguage`.
- Side quest that unblocked verification:
  `Player::_LoadSkills` field population
  (`PLAYER_SKILL_INFO_1_1` triplets with language 300/300,
  mono 1/1, level-max fixups) plus a new `SkillDbc`
  (SkillLine/SkillRaceClassInfo/SkillTiers). The stock
  client derives known languages from skill fields, not
  the spellbook; bench fixture characters also had zero
  `character_skills` rows, so language rows (Orcish 109,
  Troll 315) were seeded for guids 2/3/4.
- Deviations: antispam/mute/level-restriction/telemetry
  family unported; unlearned language drops silently (no
  `SMSG_NOTIFICATION`); whisper delivery follows fork
  default-config behavior (`WhisperRestriction` off, client
  filters ignores); no GM-mode language/tag, AFK in-combat
  guard, `LogChat`, or link security checks; `_LoadSkills`
  skips C++ corrective DB writes and
  `UpdateSkillTrainedSpells`.

Evidence: unit tests in `chat.rs` and `skill_info.rs`;
entities `skill_fields` test (fixups, forbidden/unknown/
zero skips); loopback `assert_chat_flow` (say echo,
unlearned-language drop, whisper self + inform + AFK
auto-reply, not-found); `./ci.sh` exit 0 (125 suites);
`two_client_smoke.py` now asserts cross-client say and
whisper (`CHAT-SAY-A-TO-B`, `CHAT-WHISPER-A-TO-B`) —
passed, after the known intermittent relay timeout flaked
twice (same pre-slice signature); login-train and
scripted-world audits pass. OrbStack stock client:
whisper pair and AFK lines rendered, Morka's yell heard,
and self say/yell rendered once alive
(artifacts `chatb2-13/22/27*.png`). Diagnosis note: early
rig say failures were the C++ dead-player drop working as
designed — a level 55 Den Grunt at the Valley of Trials
spawn kills the 1000-HP fixture characters in ~10s
(`UnitDied` in bench `Server.log`); that a friendly guard
attacks players at spawn is a Phase 4 faction/AI finding,
not chat. Ruff is not installed on this host right now
(pre-commit gate unaffected; changed Python self-tests
pass via `--self-test`).

## Slice B.2b — chat channels (complete, 2026-10-18)

Full channel service: `Channel` + `ChannelMgr` as a pure
state machine (`crates/mangosd/src/channels.rs`) with
actor-resolved player views and emit-time recipient
snapshots (C++ join/kick broadcast ordering). All 16
channel opcodes (join/leave/list/password/owner/set-
owner/moderator/unmoderator/mute/unmute/invite/kick/
ban/unban/announcements/moderate) plus `CHAT_MSG_CHANNEL`
say; built-in detection from a new `ChatChannelDbc`
(`ChatChannels.dbc` pattern match per
`GetChannelEntryFor`); per-faction managers with the
fork's staff security channels (`Warden` et al.);
`CleanupChannels` on session remove. Configs wired:
`AllowTwoSide.Interaction.Channel`,
`GM.JoinOppositeFactionChannels` (incl. the two-side-
chat force-off), `Channel.SilentlyGMJoin`,
`ChannelInvite.MinLevel`, `WorldChan.MinLevel`.
Deviations: honor rank constant 0 (WorldDefense mutes
like rank 0), account-flag vanish-mute and bot chat
hooks unported, list visibility security-gate only,
channel-owner name resolves from online players only,
account max level approximated by player level.

Evidence: unit tests (`channels.rs` lifecycle/password/
cleanup, `chat_channels.rs` pattern match); loopback
`assert_channel_flow` (exact notify/list/channel-chat
bytes for built-in + custom incl. owner mode-change);
`./ci.sh` exit 0 (125 suites), clippy `-D warnings`
clean; two-client smoke `CHANNEL-JOIN-BOTH`,
`CHANNEL-SAY-A-TO-B`, `CHANNEL-LEAVE-BOTH`,
`TWO-CLIENT-SMOKE-PASS`. OrbStack stock client: auto-
joined [1. General - Durotar] and [3. LocalDefense -
Durotar] on world entry, `[1. General] [Groktar]:
general channel works`, joined [4. Rigchan] and spoke
in it (artifacts `chan-02/03*.png`).

## Verification gates (standing)

- `vmangos-rs/ci.sh` (fmt, clippy `--all-targets -D warnings`, build, tests,
  live ignored suites under `.env.test`) and Ruff on changed Python.
- Scripted clients against the bench (`login_train_audit.py --strict`,
  `two_client_smoke.py`, `phase4_scripted_world.py`) stay green per slice.
- User-visible service behavior: OrbStack `wow-rig-arm` stock-5875 client.
- Phase exit: `.partybot debug` `[BotAI]` trace comparison vs C++,
  Deadmines five-bot clear per role, guild roster dashboard round-trip.

## Slice B.3 — group/party/raid (complete, 2026-10-18)

Full group service: `Group` as a pure state machine
(`crates/mangosd/src/group.rs`) over an internal-key map
in the world actor (uncreated invite shells included),
with `groups`/`group_member` persistence byte-for-
statement from C++ (`crates/mangos-db/src/group.rs`:
boot `LoadGroups` incl. stale-row and `<2`/`>40`member
cleanup, create/member/leader/raid/subgroup/assistant
writes). All `GroupHandler.cpp` opcodes: invite /
accept / decline / uninvite(+guid) / set-leader /
disband-leave / loot method / loot roll (validated
no-op until group loot) / raid convert / subgroup
change+swap / assistant / target icons / ready check /
minimap ping / random roll / party-member-stats
request (full snapshot; pet fields zero, no pet
aggregate yet). Party/raid/leader/warning chat wired
into `on_message_chat` with the subgroup-scoped party
broadcast and two-side-group language rule. Login
reattaches the persisted group (`_LoadGroup` +
`UpdatePlayerOnlineStatus`), logout drops invites and
posts the offline `SendUpdate`; `PLAYER_FLAGS`
group-leader bit is maintained on every transition.
Deviations: no battleground/original-group duality,
no LFG/meeting-stone, loot rolls, instance binds
(`group_instance`, homebind/reset-on-join — Phase 6),
offline-leader promotion timer, `BroadcastGroupUpdate`
value forcing, continuous `SMSG_PARTY_MEMBER_STATS`
streaming (request-driven full snapshots only), or
guild-bot invite hooks (Slice E); `IsGameMaster()`
reads as account security. `AllowTwoSide.Interaction.Group`
wired.

Evidence: 7 state-machine unit tests (exact
`SMSG_GROUP_LIST` bytes, write ordering, disband,
leader promotion, subgroups, target icons); loopback
`assert_group_flow` (bad-name result, self-invite
teardown, solo roll, offline stats form); `./ci.sh`
exit 0; `LOGIN-TRAIN-PASS`, `SCRIPTED-WORLD-PASS`.
Two-client bench smoke: `PARTY-INVITE-ACCEPT`,
`PARTY-CHAT-A-TO-B`, `PARTY-LEAVE-DISBAND` (incl.
`group_member` row cleanup), `TWO-CLIENT-SMOKE-PASS`.
OrbStack stock client (artifacts `party-02/03*.png`):
`Cannot find 'Zulrath'.` for the offline invite, then
with the scripted Zulrath probe online: "You have
invited Zulrath to join your group", "Zulrath joins
the party", "Looting changed to group loot / Loot
threshold set to Uncommon", leader crown, party frame,
`[Party]` chat both ways; the persisted bench group
(1 `groups` row, 2 members) was verified in MySQL and
cleaned, bench restarted (PID 6200), smoke re-green.
Ruff still unavailable on this host (script
`--self-test` passes).

## Slice B.4 — guild (complete, 2026-10-18)

Full guild service: `Guild` as a pure state machine
(`crates/mangosd/src/guild.rs`) with `guild`,
`guild_rank`, `guild_member`, `guild_eventlog`
persistence statement-for-statement
(`crates/mangos-db/src/guild.rs` boot `LoadGuilds`
incl. orphan/broken-row cleanups and event-log prune;
rank rebuild/rewrite and `CheckGuildStructure` in the
state machine). All `GuildHandler.cpp` opcodes:
query, invite/accept/decline, info, roster, promote/
demote, leave/remove/disband/leader, MOTD, public/
officer notes, rank set/add/delete, info text, emblem
save (10g charge; tabard-designer proximity check
skipped — no NPC-flag surface in the actor yet).
Guild/officer chat with rank speak/listen rights and
ignore filtering; login MOTD + signed-on, logout
stats refresh + signed-off; who list now fills guild
names (closes the B.1 deviation). Key fork fact:
`CMSG_GUILD_CREATE` is `STATUS_NEVER` — guilds are
created only by charter petitions, so the create
handler exists but stays unreachable exactly as in
C++; petitions are the follow-up slice B.4b. Other
deviations as listed in the guild module docs
(guid-ordered roster/succession vs C++ hash order,
zone-0 member repair, UTC creation date,
security-as-GM for cross-faction checks).

Evidence: 7 state-machine unit tests; loopback
`assert_guild_flow` over a boot-seeded guild (exact
query/roster bytes, MOTD event, guild-chat echo,
sole-leader disband) plus login-burst guild events and
the who guild-name expectation; `./ci.sh` exit 0;
`LOGIN-TRAIN-PASS` (now reporting `SMSG_GUILD_EVENT`
conditional), `SCRIPTED-WORLD-PASS`,
`TWO-CLIENT-SMOKE-PASS` (spawn-guard deaths made two
attempts flake first — the known Phase 4 faction
finding, `UnitDied 3/4` in bench log). Bench fixture
guild 9001 "Bench Wardens" (Zulrath GM, Groktar
officer, Morka initiate) seeded in MySQL; scripted
probe `GUILD-ROSTER-OK`, `GUILD-CHAT-A-TO-B`,
`GUILD-PROMOTE-OK`, `GUILD-DEMOTE-OK`,
`GUILD-EVENTLOG 3,4`, `GUILD-PROBE-PASS` with DB rank
round-trips verified. OrbStack stock client
(artifacts `guild-02/04*.png`): `[Guild] [Groktar]:
guild chat works`, roster pane "Bench Wardens — 3
Guild Members (1 Online)" with levels/classes/zone
and the MOTD. Bench on PID 8981; fixture guild kept
for later slices (dashboard round-trip at phase gate).

## Slice B.4b — guild charter petitions (complete)

`PetitionsHandler.cpp` + `GuildMgr` petition map in
`mangosd::petition` (pure state machine + `PetitionWrite`)
over `mangos-db::petition` (`petition`, `petition_sign`,
charter `item_instance`/`character_inventory`,
`reserved_name`). Boot `LoadPetitions` with orphan-sign
cleanup and owner repair. All nine opcodes: showlist,
buy (registrar/tabard-designer NPC gate, name taken /
reserved / `IsValidCharterName` with the strict-mask
language classes, 10s cost, buy/equip error packets),
show signatures, query, rename, sign (own/already-signed
results + owner notice, team and invited gates), decline,
offer, turn-in (guild created via the B.4 service with
all signers at the lowest rank). `RemovePetitionsAndSigns`
purge wired into every guild-join path with the fork's
exact statements (owned petition object stays registered
in memory; its DB row goes). New config: `MinPetitionSigns`,
`MinCharterName`, `StrictCharterNames`.

Item-system deviation (recorded): the runtime Item-object
system is Phase 6, so the charter exists only as DB rows
— the same on-disk shape C++ leaves because buy saves
inventory immediately — and `SendNewItem`/client bag
rendering joins with that phase; charter destruction by
manual item deletion likewise waits for inventory. NPC
gate keeps exists/flag/alive/distance; reaction/combat
gates join with general NPC interaction. Character-delete
purge unhooked (delete runs pre-world).

Evidence: service + DB unit tests (boot indexing, exact-
complete threshold, cleanup exception, charter-name
classes incl. Cyrillic, enchantment text); loopback
`assert_petition_flow` (exact query/show/sign-results/
rename/turn-in bytes over a seeded charter); `./ci.sh`
exit 0 (125 suites); `LOGIN-TRAIN-PASS`,
`SCRIPTED-WORLD-PASS`, `TWO-CLIENT-SMOKE-PASS`. Bench
end-to-end probe against Urtrun Clanbringer (temporary
`MinPetitionSigns = 2`): `PETITION-SHOWLIST-OK`,
`PETITION-BUY-OK pid=1 charter=769108 slot=23`,
`PETITION-SIGN-OK` x2, `PETITION-SIGN-DUP-OK`,
`PETITION-QUERY-OK`, `PETITION-TURNIN-OK guild=9002`,
`PETITION-ROSTER-OK`, `PETITION-PROBE-PASS`, with DB
round-trips for petition/sign/item/guild rows; probe
guild deleted and fixtures/conf restored afterwards.
Stock-client rig skipped deliberately: the charter item
cannot render in bags until the Phase 6 item system, so
the only client-visible surface (the created guild) is
the B.4 surface already rig-verified. Bench on PID 13403;
fixture guild 9001 untouched.

### B.4b erratum — ObjectGuid wire form

Plain `ObjectGuid` streams as a full u64 in the fork
(`ByteBuffer <<`/`>> ObjectGuid`); only `PackedGuid`
is packed. B.4b as first landed used packed guids on
petition packets (self-consistent in scripted tests,
wrong for the real client). All petition guid reads
and writes now use full u64s; loopback expectations
and the bench probe were updated and re-verified
(`PETITION-PROBE-PASS` again, full CI green). Mail
(B.5) was written with u64 guids from the start.

## Slice B.5 — mail (complete)

`MailHandler.cpp` over the per-player deferred mail
rows: send (mailbox gate, trial/GM/antispam gates,
postage+money charge, uint8-wrapping recipient cap,
team gate, one attachment with the row-level
`CanBeTraded` subset, COD cap, account spam ledger),
list (exact `SMSG_MAIL_LIST_RESULT` item blocks),
item-text query, take money/item (COD payment mail to
the sender), mark read (3-day expiry clamp), delete,
return-to-sender (checked=RETURNED, cross-account
item delay), permanent letter copy (item 8383),
next-mail-time, delivery tracking with the per-tick
`SMSG_RECEIVED_MAIL` notice, and the boot
`ReturnOrDeleteOldMails` sweep (fork-faithful:
item_text rows are never swept). New config:
`AllowTwoSide.Interaction.Mail`, `MailDeliveryDelay`,
`GM.AllowTrades`, `MailSpam.*`,
`Mails.COD.ForceTag.MaxLevel`.

Deviations (recorded): attachments move as DB rows
until the Phase 6 item system (first-free-backpack
slot store; stack merging/bag placement, bound-by-
enchant and loot-guid trade checks unported);
mail-template item generation unported (template
mails show empty); expired-mail sweep runs at boot
only (no daily re-run); mailbox gate keeps
exists/type/distance; item suffix factor written 0
(as elsewhere); money/item changes persist per
statement while coinage itself still saves in
Phase 6.

Evidence: service unit tests (tracker/delivery,
times/visibility, spam ledger, packet bytes);
loopback `assert_mail_flow` (next-mail-time 0.0 with
a seeded unread mail, item-text round-trip) —
mailbox-gated flows are bench-covered because the
fake world has no mailbox gameobject; `./ci.sh`
exit 0 (125 suites); `LOGIN-TRAIN-PASS`,
`SCRIPTED-WORLD-PASS`, `TWO-CLIENT-SMOKE-PASS`.
Bench probe at the Bank of Orgrimmar mailbox
(temporary `MailDeliveryDelay = 0`): `MAIL-NEXT-NONE-OK`,
`MAIL-DELAYED-NOTIFY-OK` (seeded in-transit mail lands
via the tick), `MAIL-SEND-TEXT-OK`, `MAIL-LIST-OK`,
`MAIL-SEND-MONEY-OK`, `MAIL-TAKE-MONEY-OK`,
`MAIL-SEND-ITEM-OK`, `MAIL-TAKE-ITEM-COD-OK` (COD
payment mail checked=8 money=250), `MAIL-MARK-READ-OK`,
`MAIL-RETURN-OK`, `MAIL-PERMANENT-OK`, `MAIL-DELETE-OK`,
`MAIL-PROBE-PASS`; boot sweep `MAIL-SWEEP-PASS`
(delete textless, flip returnable, destroy
COD-payment). Rig: Groktar logged in at the Bank of
Orgrimmar with a seeded unread mail
(`mail-03/04*.png`); the mailbox mesh could not be
reliably clicked in the flat llvmpipe render in
reasonable attempts, so stock-client list rendering
remains unverified — the packet is byte-verified
bench-side. Fixtures/conf restored; bench on PID
17236.

## Slice B.6 — trade (complete)

`TradeHandler.cpp` / `TradeData` over DB-backed
item rows: initiate (alive/busy/team/distance/trial
gates; stun/taxi/logging-out not in the actor yet),
begin/busy/ignore/cancel, set gold/item, clear item,
unaccept, and accept with the fork's exact fan-out —
`SetAccepted(false)` BACK_TO_TRADE notices on every
modification, the 200 ms anti-scam delay compared in
whole `time_t` seconds, accepted-stays-set on the
distance failure, gold-cap and `GM.AllowTrades`
cancels to the accepter only, `CanBeTraded`
revalidation, backpack capacity failure
notifications (LANG 802/803), pairwise `MoveItems`
(first free backpack slot; wrapped gift-creator
rewrite), and the money swap. New config:
`AllowTwoSide.Interaction.Trade`. Client bag 255
(backpack row set) and carried-bag positions are
translated to stored rows in
`mangos_db::trade::item_at_pos`. Logout/disconnect
cancels via `leave_map`.

Deviations (recorded): trade spells on the
non-traded slot unported (item-target casts belong
to the Phase 6 item system; the slot is
display-only and the spell field stays 0); capacity
is backpack free slots + outgoing count (no stack
merging or bag placement); `CanBeTraded` keeps the
row subset (soulbound flag, non-empty/equipped bag,
generated loot); the bot-service `OnTradeCompleted`
hook lands with Slice E; distance is the plain 3D
distance vs `TRADE_DISTANCE` (no object-size
subtraction); suffix factor 0 as elsewhere.

Evidence: unit tests (status/extended packet bytes,
`CanBeTraded` rules); loopback `assert_trade_flow`
(initiate NO_TARGET and self-BUSY — the fake backend
has one account, so the exchange itself is
bench-covered); `./ci.sh` exit 0; `LOGIN-TRAIN-PASS`,
`SCRIPTED-WORLD-PASS`, `TWO-CLIENT-SMOKE-PASS`.
Bench probe `/tmp/trade_probe.py` with exact status
sequencing: `TRADE-INITIATE-OK`, `TRADE-OPEN-OK`,
`TRADE-GOLD-OK`, `TRADE-ITEM-OK`, `TRADE-ACCEPT-OK`,
`TRADE-COMPLETE-OK`, `TRADE-DB-OK` (linen owner/inv
moved 5 → 6), `TRADE-MONEY-OK` (follow-up trade
offers the exact post-swap 850), `TRADE-CANCEL-OK`,
`TRADE-PROBE-PASS`. Fixtures cleaned; bench on PID
20549.

## Slice B.7 — auction house (complete)

`AuctionHouseHandler.cpp` + `AuctionHouseMgr`:
hello, sell (price caps, GM/trial gates, concurrent
account limit, 2/8/24h durations, deposit formula,
`CanBeTraded` + conjured/duration/bank checks,
already-auctioned rejection), place bid (own-auction
and own-account blocks, start-bid/outbid increment
rules, difference-only self-raise, outbid refund
mail with pre-update notification values), buyout
(successful + won mails, item re-own), cancel (cut
charge, bidder refund, item returned), and the
three list queries (buyout-ordered search with the
fork's count/total quirks, owner, bidder +
outbidded ids). Houses follow `LoadAuctionHouses`:
cross-faction single, unlinked pre-1.9 per DBC
entry, else linked per team; auctioneer house from
creature faction template via the fork's switch
(FactionTemplate.dbc mask fallback unported —
unknown templates take the goblin default).
`AuctionHouse.dbc` is loaded into the world stores.
Expiry + 5-minute IP-lock aging run on the actor
tick; boot does the fork's cleanup (missing-item
auctions deleted, unknown-house auctions mailed
back from house 7). Auction mails ride the B.5
mail path (type 2, stationery 62, COPIED, instant).
New config: `AllowTwoSide.Interaction.Auction`,
`Progression.UnlinkedAuctionHouses`,
`Rate.Auction.Time/Deposit/Cut`,
`Auction.Deposit.Min`,
`Auction.AccountConcurrentLimit`.

Deviations (recorded): list queries answer
synchronously (no async task/one-flight flag); the
`usable` and recipe-known filters are ignored and
name search matches the base prototype name only;
GM self-guid auctioneer access unported (Slice C);
auction items are DB rows (stack splitting on sell
does not exist — whole stacks list as carried);
suffix factor 0; the AH bot (`AuctionHouseBotMgr`)
is Slice F.

Evidence: unit tests (deposit/cut/outbid, house-map
modes, query filters, info block bytes, DBC
lookup); loopback `assert_auction_flow` (sell/bid
against a ghost auctioneer return DATABASE /
ITEM_NOT_FOUND — the fake world has no spawns);
`./ci.sh` exit 0; `LOGIN-TRAIN-PASS`,
`SCRIPTED-WORLD-PASS`, `TWO-CLIENT-SMOKE-PASS`.
Bench probe `/tmp/auction_probe.py` at Auctioneer
Wabang (house 6, patch-current spawn 12699):
`AH-HELLO-OK`, `AH-SELL-OK` (deposit 1 on linen),
`AH-LIST-OK`, `AH-OWNER-LIST-OK`, `AH-BID-OK`,
`AH-RAISE-OK`, `AH-OUTBID-OK` (refund mail 160c,
checked=4), `AH-BUYOUT-OK` (profit mail
900+deposit-45 cut, won item mail + owner rewrite),
`AH-CANCEL-OK` (refund + item-back mails),
`AH-EXPIRE-PASS` (past-expiry row settled by
boot-load + tick: item mailed back, row deleted),
`AH-PROBE-PASS`. Fixtures cleaned; bench on PID
23498.

Finding: the first probe draft ran an unscoped
`DELETE FROM auction` against the dev bench,
wiping its 198 pre-existing (bot-seeded) auction
rows before later drafts scoped deletes to the
probe sellers. Bench-only loss; the item_instance
rows remain orphaned (matching how the fork leaves
them) and the AH seed can be regenerated whenever
the AH bot lands in Slice F.

## Slice C — GM command table/permissions contract (complete)

The fork's `ChatCommand` surface as data + semantics,
handlers trailing per §13.2. The full static table
(83 tables, 842 entries incl. `partybot`, `bot`,
`ahbot`, `battlebot`) was extracted from
`Chat.cpp::getCommandTable` by a script into
`mangosd/src/commands_table.rs` (independent
cross-checks: parsed entries == raw entry starts in
source, security histogram matches `SEC_*` counts).
`mangosd/src/commands.rs` carries the contract:
`FindCommand` (case-insensitive abbreviation, ""
subcommand tail restore, exact mode for DB rows),
`FillFullCommandsName`, world-DB `command` overrides
(security/help/flags with the fork's log lines), RBAC
(`rbac_permissions`/`rbac_account_permissions`/
`rbac_command_permissions` masks; ban denies, grant
allows below level), `isAvailable`,
`ShowHelpForCommand`/`ShowHelpForSubCommands`
rendering (including the fork's empty-name
"Command  have subcommands:" quirk). Dot/bang
commands are consumed in `CMSG_MESSAGECHAT` for the
same chat types as the fork, gated by
`PlayerCommands` for SEC_PLAYER; unknown →
`LANG_NO_CMD`, unknown subcommand → `LANG_NO_SUBCMD`
+ help, unavailable → `LANG_COMMAND_UNAVAILABLE`,
GM command use is logged. Registered handlers:
`help` and `commands` only — every other known
command takes the handler-false path (help text if
present, else `LANG_CMD_SYNTAX`, plus the
subcommand list) until ported. The generated table
file is exempted from rustfmt drift by regeneration,
not hand edits.

Deviations (recorded): handlers trail (all but
help/commands); DB-flags only-on-self gate and
critical-log extras need selection state; console
input does not exist (`allow_console` is data only);
command execution is inline on the actor where the
fork defers to the world thread (same thread here);
the extractor lives at `/tmp/extract_commands.py`
(out of tree, like other probes).

Evidence: unit tests (table shape incl. partybot
debug, abbreviation/find, RBAC ban/grant, override
application); loopback `assert_command_flow`
(`.commands` list shows player commands and hides
partybot, `.gm on` consumed silently at player
level, `..dots` says normally); `./ci.sh` exit 0;
`LOGIN-TRAIN-PASS`, `SCRIPTED-WORLD-PASS`,
`TWO-CLIENT-SMOKE-PASS`. Bench probe
`/tmp/commands_probe.py` (TESTUSER temporarily
granted `account_access` 6, removed after):
`CMD-HELP-OK` ("Commands available to you:" +
partybot listed), `CMD-SUBLIST-OK`,
`CMD-SYNTAX-OK` (`.partybot debug`),
`CMD-UNKNOWN-OK`, `CMD-OVERRIDE-OK`
(`.debug setvaluebyindex` shows its Syntax help at
GM 6 and is unavailable to a player; both bench
`command` overrides loaded at boot: ">> Loaded 842
chat commands"), `CMD-UNAVAILABLE-OK`,
`CMD-HELPTEXT-OK` (".help partybot" → "Manage party
bots"), `CMD-PROBE-PASS`. Bench on PID 25558.

## Slice D — BotSystem minimum + Host API (2026-10-19)

`mangos-bots` gained the behaviour layer: `role.rs`
(`BotRole`/`BotNiche`), `context.rs`
(`SituationContext` snapshot, C++ defaults),
`host.rs` (`BotHost` trait: unit views, party
members, `can_try_cast`/`cast_spell`, chase/follow/
point/stop movement, target, chat), `spell_kit.rs`
(capability struct; resolution in Slice E),
`behavior.rs` (weighted fallthrough runner,
`[BotAI]` debug formats preserved; deliberate
deviations: no silent 32-behaviour cap, stable tie
order), `follow.rs` (`FollowLeader` exemplar).

`mangosd` wires it in: bots are session-less
players logged in through the real login train
(synthetic session, sinking `SessionControl`
channel, fake account id à la `GenBotAccountId`
with the ownership gate evaluated against the
character row), motion-mastered via
`Map::motion_initialize_player`, ticked at 500 ms
on absolute deadlines with a self+party
`EvaluateSituation` subset. `ActorBotHost`
implements `BotHost` over the world actor
(no `&mut Map` crosses the crate boundary).
Commands: `.bot add/delete/info` (exact fork
replies), `.partybot debug` (all-bot toggle,
"Check Server.log"; selection targeting deferred
with selection state).

Verification: `./ci.sh` exit 0 (incl. 29
`mangos-bots` tests). Bench probe
`/tmp/bot_probe.py` (Orgrimmar anchor; Valley
spawn unusable due to the known friendly-Den-Grunt
issue): `BOT-ADD-OK`, `BOT-FOLLOW-OK moves=6`
(bot spawned 40y out, chase splines observed by
the master, `[BotAI] -> EXECUTED: FollowLeader`
in Server.log), `BOT-DEBUG-OK`, `BOT-INFO-OK`,
`BOT-DELETE-OK`, `BOT-PROBE-PASS`. Regressions:
commands probe PASS (its `.partybot debug`
expectation updated from the Slice-C handler-false
path), `SCRIPTED-WORLD-PASS`, two-client smoke
PASS on rerun (first two attempts hit the known
Den Grunt spawn-death flake at `CHAT-SAY-A-TO-B`).
Bench on PID 27953, fixtures restored.

Known Slice D limits (Slice E/F): empty spell
kit, `can_try_cast` is known+alive only, bot chat
logs instead of fanning out, situation target/
threat/environment fields unset, `.bot add` does
not group-invite (party attach lands with the
party-bot behaviours), no `PlayerBotMgr`
config/respawn, `.partybot add <class>` character
creation unported.

## Slice E.1+E.2 — Spell resolution + shared behaviours (2026-10-19)

E.1: `PopulateSpellData` ported. Name-match
chains extracted by `/tmp/extract_class_spells.py`
into generated `class_spells_table.rs` (275 rows
across 9 classes, 0 unresolved; `A!B` needs the
exclude-needle form for Healing Wave).
`resolve_class_spells` mirrors the classifier:
id-order scan, highest-rank wins (rank 0 in the
Rust store → id order, like unranked C++ spells),
heal sets with the fork's equal-points set
collapse, taunt list, resurrection, and the
paladin/shaman/mage/rogue random post-fixups.
`SpellKit::resolve` implements all 15
`BotSpellKit` getters and carries the classified
spellbook (`Arc<ClassSpells>`). Slice D's
invented context/runner were replaced with the
real ones: `SituationContext` field-for-field,
per-entry role weights, `Score` with host access
(the fork's scores query live world via the AI
pointer), and the exact `[BotAI]` trace formats.

E.2: all 19 shared behaviours ported
(`behaviors_shared.rs`) with the healing system
(`healing.rs`: `SelectHealTarget`,
most-efficient-spell walk with the undershoot
break, pre-heal incl. the fork's
return-target-as-true quirk, buff target
selection). `BotRoleConfig` set assembly with
fork weights (combat/OOC). Host grew the verbs
the behaviours need (attackers, auras, threat,
party tank, motion kind, dispel scan, CC marks
via group raid icons); map grew matching query
helpers. Evaluator now fills self/target/threat/
tank/loose-adds/formation sections; dispatcher
pre-steps (caster attack-stop, dispel scan) and
the 2s opener delay run in `update_bots`.

Verification: `./ci.sh` exit 0. Bench
`BOT-PROBE-PASS` with the real sets: Position
drives follow (45 spline moves), seeded priest
fixture casts Power Word: Fortitude + Inner Fire
via PartyBuff/SelfBuff (`[BotCast] ... Started`),
trace lines match fork formats. Regressions:
`SCRIPTED-WORLD-PASS`, two-client PASS.

Known E.2 limits: LoS folded into the 30y gate;
combat distance is plain 3D (no reach
subtraction); `SelfBuff`/`PartyBuff` scores gate
on class (union lives in the kit, not visible to
`Score` — a wasted fallthrough when already
buffed); pre-heal cast-time gate not yet checked;
pets/duels/shield heuristic unfilled;
`SelfBuff` re-checks aura in execute. Combat
behaviours need target acquisition (E.6 party
system) before they fire live.

## Slice E.3+E.4 — Class behaviours + formation (2026-10-19)

All 19 class-unique behaviours ported
(`behaviors_class.rs`: warrior stance/rage,
rogue stealth opener/stealth/finisher/poisons,
hunter pet/aspect/dead-zone, paladin
blessing/seal-judge, shaman totems, priest
shadowform/shield, warlock pet/curse/lifetap,
druid form/rebirth/innervate) and wired into
`BuildBehaviorSet` combat/OOC with fork weights.
Pet, weapon-buff, and totem executes go through
host verbs (`pet`, `cast_weapon_buff`,
`totem_present` over a 60s cast-timestamp
bookkeeping) that stay inert until those
subsystems exist host-side; combo points remain
0 (no combo system in the Rust spell engine).

Formation: the island `FormationManager`
(breadcrumb trail, slot assignment, slot/flee
math — already a faithful `BotFormation` port)
is integrated: trail recording per leader on bot
ticks, slot assignment on group-size change
(group members, or leader + sibling bots when
ungrouped), evaluator fills slot position +
flee point (flee only when meleed), with the
fork's leader-follow fallback when no slot
position exists. Walk-hit is a passthrough until
the map exposes navmesh walk-hit.

Verification: `./ci.sh` exit 0 after each step;
`BOT-PROBE-PASS` throughout (follow now rides
`Position` → slot/fallback, 44-45 spline moves;
buff casts `Started`).

## Slice E.5 — bot chat + services (2026-10-19)

`BotChatManager` lands as pure `ChatCore`
(`mangos-bots/src/chat.rs`) over a generated
`chat_pools.rs` (35 pools, 221 lines, 20 whisper
patterns — `/tmp/extract_chat_pools.py`, never
hand-edited): per-channel cooldowns with the
fork's bot-count scaling, five personalities,
trade/guild ambient timers on absolute deadlines
(§9 scheduler rule), delayed whispers (5-20s),
party greet/gratz/boss/wipe reactions. Wipe
mirrors the fork exactly: no `CanChat` gate, no
cooldown recorded, dead bots react.

`BotServiceMgr` lands as pure `ServiceCore`
(`mangos-bots/src/services.rs`): portal/enchant/
transmute/mooncloth request state machines
(300s timeout; tips portal 10000, transmute/
mooncloth 20000, enchant charged at the fork's
common-tier floor 0 despite its header constant),
portal destination + enchant tables with the
deterministic rare-enchant knowledge hash, WTB
buy side with per-day budget (AH fair-value
oracle injected by the host; returns unknown
until the Slice F AH bot lands).

Actor wiring: `BotChat.*` / `BotService.*`
config with fork boot lines, whisper routing
service → buy → generic delayed reply, bot trade
auto-begin/accept for service-backed requests,
completion delivery (portal = triggered
skip-known self-cast; enchant/transmute/
mooncloth resolve to whisper + log — item
application waits on item saves, matching the
fork's transmute/mooncloth log-only delivery),
trade-channel posts via `Channel::Say`
skipCheck on "Trade - City", party greet when a
bot lands grouped. Emote/Channel are not
`BotChatKind` variants; emotes ride the map text
-emote broadcast. Trade-chat proactive service
requests (`CreateProactiveRequest`) exist in the
core; channel monitoring hooks land with E.6.

Verification: `./ci.sh` exit 0 (89 suites);
`BOT-CHAT-PROBE-PASS` (generic whisper 'hi
there', enchant service confirm, ambient 'WTS
leather and hides, check AH' in Trade - City
with 3-4s intervals); regressions
`BOT-PROBE-PASS`, `TRADE-PROBE-PASS`,
`MAIL-PROBE-PASS`, `AH-PROBE-PASS`,
`CMD-PROBE-PASS`, `SCRIPTED-WORLD-PASS`,
`TWO-CLIENT-SMOKE-PASS` (two flake retries —
known Den Grunt spawn deaths). Mail/AH probes
needed fixture reseeds (consumed seed mail
990001, linen 769109, Orgrimmar parking,
`MailDeliveryDelay=0`); all temp conf lines
removed, account_access restored, fixtures back
at Valley `online=0`. Enchant item application
and mailbox stock-client rendering remain
unverified (noted deviations).

## Slice E.6 — party system, director, guild bots, presence (2026-10-19)

Target acquisition (`SelectAttackTarget` +
`AttackStart`): non-healer party bots re-evaluate
each tick — focus marks, tank's victim, leader's
victim, own attackers, party attackers within 50y —
with role chase distances and the tank pull announce
(15s internal cooldown, party-chat cooldown shared).
The fleeing-attacker step is skipped: the Rust motion
layer has no fleeing generator type to detect.
Validity folds to alive + creature/opposite-team
player (visibility/CC/immune gates not yet present).
Group glue: bots auto-accept group invites (real
invite/accept train, greet + follow); `.partybot
load` spawns near the leader, follows, group-joins;
`.partybot setrole/attackstart/attackstop/ccmark/
focusmark/clearmarks` and `.partyrole` landed with
fork texts. Roles now come from
`CombatBotBaseAI::AutoAssignRole` signature spells
(previously every bot ran MeleeDps sets). Party
wipe/boss-kill chat hooks fire from bot-tick state
(rank>=3 last-target kill; pattern-manager half and
the level-up gratz have no Rust event source pre
Phase 6 and remain unwired).

Population (`PlayerBotMgr`) + director
(`BotDirector`) as pure cores in `mangos-bots`
(`population.rs`, `director.rs`, generated zone
tables): `playerbot` boot load, peak-cosine scaled
target, weighted random add/remove (fork comparison
order mirrored), director cycles on absolute
deadlines writing `playerbot.chance` (30 assigned /
1 standby), same-map assignment relocation at world
entry, radius wander for leaderless assigned bots
(DynamicBotAI roaming reduced; its PvP ganking and
cross-map relocation wait on player map transfer).
`.bot info` now reports real stats; `.bot
start/stop/ranadd/add_all` landed; personality
assignment corrected to fork semantics (guild roster
else Generic — no randomization).

Guild bots (`GuildBotMgr` core in `guild_bots.rs`):
roster/spec-weight boot load + guild detection,
populate-into-guild at rank 2 with role public
notes, presence churn on the peak curve (first check
~30s), leader±3 level sync every 5 min, presence →
party conversion on invite (unified engine: roster
role lock without the fork's despawn/respawn cycle),
despawn on group removal, `.guildraid
fill/assemble/disband/check/roster/botinfo`. Gear
scoring ported pure; equip-driven upgrade evaluation
has no bot equip event source yet. `.partybot
add/clone` (character generation) and the remaining
partybot utility handlers trail with the GM handler
backlog per §13.2.

Verification: `./ci.sh` exit 0 (89 suites, committed
loopback/robustness deps updated);
`BOT-PARTY-PROBE-PASS` (info Min:3 Max:10 Total:222,
raid composition 2/5/6/7, partybot load grouping
asserted in group_member, director chance=30 writes,
guild presence reached 2 online); regressions
`BOT-PROBE-PASS`, `BOT-CHAT-PROBE-PASS`,
`SCRIPTED-WORLD-PASS`, `TWO-CLIENT-SMOKE-PASS`.
Bench restored: probe conf lines removed, playerbot
chances snapshot-restored, fixtures offline.

## Slice F — AH bot + BattleBot decision

AH bot (`AuctionHouseBotMgr`): pure core in
`mangos-bots/src/ah_bot.rs` (six categories with
quota bands, weighted picks, seller-type 40/25/20/15
distribution with per-type stack rolls and price
jitter floored at 0.5×, bid 70-90% of buyout, fair
buyout per unit from `buyout/min_stack`, buyer
purchase-probability bands 95/80/50/20/5/1/0,
refresh/80%-restock/buyer gates). Host application in
the world actor: boot load of `auctionhousebot` rows
+ `auctionhousebot_state` stamp + seller identities
(account window guids 900001-900100, created from the
fork name pool when short), house resolution via
`house_id_for_faction_template(ahfid)` → DBC row
(fork `GetAuctionHouseEntry(uint32)`), 48h listings
through the real auction insert path, wipe of
bot-account/owner-0 auctions on refresh, buyer
buyouts reusing the successful-sale mail path with
the item instance deleted. `.ahbot update` (forced
update) and `.ahbot reload` (DB reload + forced
update) landed; `AHBot.*` config plumbed with fork
defaults (disabled by default). The E.5 WTB whisper
oracle now reads `AhBotCore::fair_buyout` instead of
the placeholder 0. Two fork-parity notes: float rolls
ride integer picks at 1e-6 resolution (distributions
match, sequences do not), and bot items get
`random_property_id = 0` (the fork's
`GenerateItemRandomPropertyId` has no Rust
equivalent yet).

Verification: `./ci.sh` exit 0 (89 suites, six new
`ah_bot` unit tests); bench probe with temp
`AHBot.Enable=1` — boot loaded 173 items / 12
sellers, refresh populated 200 listings in house 7,
buyer bought the planted cheap player linen auction
(sale mail 85c after cut, item instance deleted,
refresh stamp written); `WTB-ORACLE-PROBE-PASS`
(bot quoted 1g44s = 80000/50 × 0.9 for Black Lotus).
Regressions all green after fixture restoration:
`AH-PROBE-PASS`, `MAIL-PROBE-PASS`, `TRADE-PROBE-PASS`,
`CMD-PROBE-PASS`, `BOT-PROBE-PASS`,
`BOT-CHAT-PROBE-PASS`, `BOT-PARTY-PROBE-PASS`.
Bench restored: temp conf lines removed, fixtures
offline at Valley, money/item fixtures reseeded.

BattleBot engine decision (explicit, per §13.2):
**keep the dual engine** — recorded in
`docs/RUST_MIGRATION_ARCHITECTURE.md` §14 #5. The
fork's `BattleBotAI` bypasses behaviours entirely
(objective state machine + waypoint graphs + premade
BG spawn); the Rust server has no battleground
subsystem to drive yet. When BG internals land,
BattleBot ports as a separate `mangos-bots` engine
sharing Host API primitives only. Phase 5 ships the
`.battlebot` command contract with handlers
unported, like the rest of the GM handler backlog.

## Final verification gates (2026-10-19)

- `./ci.sh` exit 0, 89 suites (fmt, clippy
  `--all-targets -D warnings`, tests). Diff scope:
  `vmangos-rs/` + `docs/` only.
- Scripted tier: `SCRIPTED-WORLD-PASS`,
  `LOGIN-TRAIN-PASS` (strict), `TWO-CLIENT-SMOKE-PASS`
  (two consecutive; two post-restart warm-up relay
  timeouts observed first — visibility establishment
  timing, recorded as flake).
- Bench probes, all passing after fixture
  restoration: `BOT-PROBE-PASS`, `BOT-CHAT-PROBE-PASS`,
  `BOT-PARTY-PROBE-PASS`, `AH-PROBE-PASS`,
  `MAIL-PROBE-PASS`, `TRADE-PROBE-PASS`,
  `CMD-PROBE-PASS`, `WTB-ORACLE-PROBE-PASS`.
- `[BotAI]` trace vs C++ (`/tmp/botai_trace_probe.py`):
  `.partybot load` + `.partybot debug` + bot-only
  engagement via `.partybot attackstart`; 719 scored
  lines + 719 winner lines captured from Server.log,
  **zero** lines failing the three fork format
  regexes from `BotBehavior.cpp:43/58/73`
  (`raw=%.1f weight=%.1f → %.1f`, winner
  `(%.1f) OK|FAILED`, NO BEHAVIOR line). Combat-set
  winners for the priest bot: `DirectDamage (9.0) OK`,
  `Position (17.5) OK`. One recorded trace delta vs
  the fork: already-buffed `SelfBuff`/`PartyBuff`
  emit repeated FAILED fallthrough lines because
  Rust gates the aura check in `Execute` (the spell
  union lives in the kit, which `Score` does not
  receive); the fork returns 0 from `Score` and
  stays silent. Behaviour outcome is identical;
  trace volume differs. `BOTAI-TRACE-PASS`.
- OrbStack stock-5875 client (`wow-rig-arm`):
  login → world → `.bot add Shadowtusk` typed in
  chat; screenshot `p5-04-botadd.png` (pixel-checked)
  shows the party frame with Shadowtusk, the
  "Level 36 Troll Priest (Player)" tooltip, channel
  auto-joins, and `[PlayerBotMgr] Bot added :
  'Shadowtusk', GUID=5`. The stock FrameXML
  `OptionsFrame.lua:282` dialog is the rig's known
  first-entry quirk.
- Guild roster dashboard round-trip
  (`/tmp/dashboard_rt_probe.py`, real Flask app via
  test client against the bench DB): GET
  `/api/guild/roster` returned the 20-bot
  `guild_bot_roster` the Rust `GuildBotMgr` loads;
  officer POST `/api/guild/bot/200/spec`
  role tank→healer read back through the roster API
  and the DB row, then restored exactly.
  `DASHBOARD-RT-PASS`. Caveat: GET
  `/api/guild/bot/<guid>` fails on the bench with
  `Unknown column 'it.ItemLevel'` — the bench world
  DB uses snake_case (`item_level`/`quality`) while
  the production dashboard query uses the droplet's
  `ItemLevel`/`Quality` naming. Bench schema drift,
  not a Phase 5 code defect (dashboard stays Python
  per §13.2); flagging for the owner since the same
  query would fail if the droplet DB is ever
  re-dumped from the newer schema.
- **Deadmines five-bot clear per role: NOT RUN —
  blocked.** The Rust world server has no
  instance-entry path (no portal teleport / instance
  binding; login falls back to homebind), the exact
  blocker recorded at Phase 4 Slice 7 closeout.
  Instance entry is world/instance feature work
  outside Phase 5's services+bots scope, so this
  gate cannot be executed without new feature work
  the phase does not authorize. Substitute evidence
  on record: Phase 4 `live_deadmines` (door chain,
  patrols, cannon, Smite phases vs independent SQL)
  for content mechanics, plus Phase 5 party/role/
  combat behaviour verified in the open world
  (probes + trace above). Owner decision needed:
  schedule instance entry (then this gate becomes
  runnable) or accept the substitute evidence for
  Phase 5 close.

Phase 5 deliverables (services, GM contract, bots
incl. AH bot + BattleBot decision) are implemented
and verified as above. Work remains uncommitted on
`rust-migration` per owner instruction.
