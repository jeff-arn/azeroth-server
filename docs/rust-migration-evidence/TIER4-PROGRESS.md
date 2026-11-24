# Tier 4 progress — canary readiness and owner-gated canary

Goal: bring the Rust servers from the Tier 3 baseline
(`9d4f5335`) to a defensible production canary, then stop
for a separate owner decision on cutover. The repository
fork in `vmangos/src/` remains the semantic oracle.
No droplet or production write is in scope until the
later explicit owner gate in Slice 5.

## Slice 1 inventory — current-state readiness audit

### Baseline and method (2026-10-29)

- Baseline: `rust-migration` at `9d4f5335` (Tier 3),
  clean working tree at the start of this audit.
- Static marker census over `vmangos-rs/crates/**/*.rs`:
  `UNMODELED` 386, `NO-DISPATCHER-YET` 307,
  `NO-CALLSITE-YET` 1, `NEEDS-HOST` 72, and
  `NEEDS-MAP-HELPER` 1280. These are overlapping comment
  occurrences, not unique defects; the classifications
  below group them by reachable player-visible system.
- Opcode reachability census: `/tmp/tier4_opcode_audit.py`
  parsed all 828 `OpcodeMeta` rows and found 300
  registered inbound candidates. 109 have neither an
  opcode-constant nor fork-handler-name reference in
  `mangosd/src` outside the flood-classification lists.
  The session dispatcher sends otherwise-unrouted
  `Dispatch::Execute` packets to `execute_stub`
  (`world_actor.rs:18583`, body at `:19207`), which only
  logs and drops the packet. Representative families were
  manually spot-checked below; this count is therefore a
  real unhandled-opcode candidate set, subject only to
  fork handlers that are themselves intentional no-ops.
- Local host is macOS arm64 with Rust 1.98.1; OrbStack's
  Docker CLI is present. The historic droplet target is
  x86-64 Ubuntu 22.04 / glibc 2.35 class. No droplet was
  inspected or modified for this slice.

### Executive verdict

The current tree is **not canary-ready**. Tier 3 closed
world simulation/content/BG depth, but the refresh audit
found core real-client session surfaces that still reach
the Phase 3 stub. Those must be closed, or explicitly
and narrowly waived by the owner, before a live-host
canary can be defensible.

### Confirmed canary blockers

#### B0-1. Character lifecycle and broad opcode coverage

The following representative handlers have no Rust dispatch
and fall to `execute_stub`: `CMSG_CHAR_CREATE`,
`CMSG_CHAR_DELETE`, and `CMSG_CHAR_RENAME`. Character
enumeration and login work, but a real client cannot create
or delete a character through the Rust world server. This
is a production-surface blocker even though existing
characters can enter the world.

The 109-candidate opcode set also includes core families
for inventory movement/equip/open/read/split/destroy,
vendor/bank/stable/trainer/binder services, loot money
and release, taxi activation, talent learning/reset, pet
stabling, quest sharing, duel responses, and resurrection.
Each family needs a fork-handler inventory and wire probe;
they cannot be treated as one generic "misc opcodes" item.

#### B0-2. Player death, corpse, ghost, and resurrection flow

Player death currently becomes state-only
(`DeathState::Dead`, `spells.rs:4919-4924`; module note at
`spells.rs:25-27`). There is no player corpse object,
`CMSG_REPOP_REQUEST` handler, corpse query/reclaim path,
spirit-healer activation, self-resurrect response, or full
ghost visibility/movement lifecycle. The BG helper explicitly
records corpse creation and ghost side effects as unmodeled
(`battlegrounds.rs:1111-1117`), and the area spirit-healer
handlers return time 0 or no-op
(`world_actor/battleground.rs:2781-2818`). Death is a
universal production path, so this blocks both world and
battleground canaries.

#### B0-3. Inventory mutation and NPC economic services

The save/load, item-use, loot-item, trade, mail, and auction
paths exist, but ordinary client inventory rearrangement and
NPC services are largely in the stub set: swap/auto-equip,
split/destroy/open/read/wrap, auto-store to bag/bank,
vendor list/buy/sell/buyback/repair, banker and bank slots,
stable operations, binder activation, trainer list/purchase,
and tabard activation. Existing characters can be loaded with
their saved gear, but normal play cannot reliably change gear,
buy supplies, learn from trainers, bank items, or stable pets.

#### B0-4. Progression and travel handlers

`CMSG_LEARN_TALENT`, talent-wipe confirmation, skill/pet
unlearn, taxi node status/query/activation/express, summon
responses, meeting-stone operations, raid-info/reset-instance
requests, and related travel handlers are stubbed. Taxi strings
persist, but there is no taxi system state or flight runtime.
Talents and flight paths are core progression/travel surfaces,
not post-canary polish.

#### B0-5. Dynamic game objects and game-object casting

Dynamic GO creation has no map runtime: `bg_add_object`
returns `false` (`battlegrounds.rs:2279-2293`), the spell
engine has no `EFFECT_SUMMON_OBJECT` arm, and 48
`SummonGameObject` references remain in content/gossip code.
GO-originated casts are likewise unmodeled wholesale. This
blocks dropped BG flags/traps/beacons and a long tail of
quest and encounter objects, even where their script bodies
have already been transcribed.

#### B0-6. Escort and follower frameworks

There is still no `npc_escortAI` or `FollowerAI` runtime.
The current tree has 210 escort/follower references across
six Rust files, including 69 `UNMODELED` lines in that
family. Tier 3 transcribed many dependent script surfaces,
but their `Start`/`StartFollow` calls degrade to no-ops.
This blocks a material set of escort quests and the AV
world-boss/troop marches. It is a first-of-kind framework
slice for the wonder driver; dependent per-script wiring can
then be audited mechanically.

#### B0-7. Script quest-menu and text completion

Script gossip often cannot present the fork's quest entries:
`PrepareQuestMenu` remains unmodeled at 15 audited sites,
because script-built `GossipMenuData` has no quest-item
model. Default quest flow exists separately, but scripted
NPCs that rely on the prepared quest overlay can strand a
quest turn-in or offer. `CMSG_NPC_TEXT_QUERY` and
`CMSG_PAGE_TEXT_QUERY` are also in the stub set. These are
core quest-UX surfaces and must be inventoried by reachable
NPC/quest rather than waived globally.

### Blockers unless narrowly waived for the canary window

- **HonorMgr:** CP rows are appended and saved, but rank and
  standing recomputation plus daily/weekly totals are
  unmodeled (`battlegrounds.rs:1537-1545`). A canary shorter
  than an honor calculation boundary could be waived only
  if the owner accepts frozen ranks/standings; production
  cutover cannot.
- **Game events:** `game_event_active` always returns false
  (`content.rs:33137-33143`), event-linked spawn rows are
  suppressed, BG weekends read inactive, and seasonal script
  gates stay inert. This is waivable only for a canary window
  with no expected event content; it is a production blocker.
- **Spell-script seam families:** 72 `NEEDS-HOST` notes remain
  in `spell_scripts.rs`, dominated by `OnSetTargetMap` (15),
  `OnPeriodicTrigger` (8), `OnCheckTarget` (6), and
  `OnSummon` (6), plus lifecycle/proc seams. Generic spell
  effects often still execute, so classification must be by
  reachable spell/class/content importance, not comment count.
- **Content helper burn-down:** the 1,280
  `NEEDS-MAP-HELPER` occurrences are concentrated in
  `content.rs` and include movement/pathing, scans, faction
  and visibility/display writes, object respawn/summon,
  threat/combat helpers, sounds/emotes, and encounter-local
  state. They do not all block a canary, but acceptance runs
  must convert failures on selected real-play paths into
  root-cause fixes rather than treating the count as noise.
- **OutdoorPvP:** no runtime exists. EPL/Silithus objectives
  and spell 30882 remain inert. This is a plausible short
  canary waiver, but not full-parity cutover behavior.
- **Configuration parity:** only a subset of fork config is
  consumed today. Rates and operator-facing settings need a
  key-by-key audit against the production configuration
  shape (without reading or printing secrets) so the canary
  does not silently run with fork-default behavior where
  production is tuned.

### Canary-acceptable or post-canary candidates

Subject to explicit owner waiver, the initial canary need not
wait for GM tickets/survey workflows, Warden data handling,
meeting-stone/LFG polish, duels, inspect polish, tutorials,
cinematic camera completion, Far Sight/camera dynobjects,
or niche boss cosmetic helper behavior, provided acceptance
testing proves they fail closed and do not corrupt state.
They remain production-parity work, not completed features.

### Surfaces already in place

The audit does not reopen the Tier 1-3 results: auth and
character enumeration/login, save/load, movement, combat and
spells, XP/level-up, quests, groups/guilds, trade/mail/AH,
the bot population/director/guild-bot/party-bot runtimes,
instances and the content corpus, vmaps, and WSG/AB/AV all
have local CI/live evidence in the prior tier files. Tier 4
must still re-run those gates against the exact release
artifact; prior green results are inputs, not substitutes.

### Slice 2 dependency order

1. Character create/delete and the session-opcode families,
   starting with a wonder-built handler exemplar and a
   family-by-family fork inventory.
2. Player corpse/ghost/repop/spirit-healer lifecycle,
   including BG resurrection timing and queue responses.
3. Inventory mutation, NPC services, trainer/talent, bank/
   stable, and taxi travel.
4. Dynamic game-object creation, expiry/ownership, and
   game-object-originated casts.
5. HonorMgr recomputation and the game-event manager.
6. `npc_escortAI`/`FollowerAI` framework, then dependent
   script wiring and live escort probes.
7. Spell seam families and content helper root causes in
   reachability order, driven by acceptance scenarios.

### Opcode-family classification

The 109 no-direct-reference candidates were exhaustively
partitioned by `/tmp/tier4_opcode_classification.py`
(asserted set equality with the audit output):

- B0 character lifecycle (3): character create, delete,
  and rename.
- B0 death and resurrection (6): corpse reclaim/query,
  repop, resurrection response, self-resurrect, and spirit
  healer activation.
- B0 inventory and item mutation (13): autobank,
  auto-equip, auto-store bag/bank, destroy, open/read,
  ammo, split, swap, and wrap.
- B0 loot completion (4): auto-store loot, master give,
  loot money, and loot release.
- B0 NPC services and economy (13): banker, binder,
  buyback, bank slot, buy item variants, stable slot,
  vendor list, repair, sell, trainer list/buy, and tabard.
- B0 pets and stable (7): pet unlearn/info and all stable
  list/store/revive/swap/retrieve operations.
- B0 progression (3): talent learn/wipe and skill unlearn.
- B0 travel and summons (5): taxi activation/status/query
  variants and summon response.
- B0 quest and text queries (8): item/NPC/page text queries,
  quest push/confirm/result, questgiver cancel, and
  quest autolaunch.
- B0 session, combat, and reputation controls (8):
  auto-repeat/channel/growth cancellation, player/logout
  cancellation, and watched/at-war/inactive faction writes.
- B1 social and client state (11): ignored chat, friend
  list, inspect/honor inspect, time query, sheath, stand
  state, helm/cloak/PvP toggles, and zone update.
- B1 group, raid, and duel (8): duel accept/cancel,
  meeting stones, LFG, raid info, and reset instances.
- Waiver-candidate support/administration (11): GM survey
  and tickets, whois, and GM teleport/raw-position packets.
- Waiver-candidate presentation/telemetry (9): cinematics,
  far sight, mount animation, movement-time skip, tutorials,
  and Warden data.

B0 means "close or obtain a specific owner waiver before a
live-host canary"; B1 means the canary could be time-boxed
around it only with an explicit waiver. The classification
does not prove that every non-candidate opcode is complete:
grouped handlers can still contain per-opcode fallback arms,
which Slice 2 acceptance probes must sample.

No Slice 2 code had been written at the end of this audit
step. The next action is character creation/deletion as the
first-of-kind handler slice. No production or droplet access
occurred.

## Slice 2 step 1 closed: character create/delete

`CMSG_CHAR_CREATE`/`CMSG_CHAR_DELETE` are now real handlers
(were `execute_stub`), fork-transcribed from
`CharacterHandler.cpp:181-384`, `Player::SaveNewPlayer`
(`Player.cpp:16051`), `Player::DeleteFromDB`
(`Player.cpp:4361`), `MasterPlayer::Create/SaveActions`,
`ObjectMgr::CheckPlayerName`, and `ItemPrototype::
GetAllowedEquipSlots`:

- Create: creating-disabled team mask, race/class DBC and
  not-playable gates, `normalizePlayerName` +
  `CheckPlayerName` (length + `isValidString` families),
  reserved-name table, name-in-use, session-count realm
  limit (fork starts the count at 10 until enum), PvP-realm
  two-side gate, then the `SaveNewPlayer` REPLACE
  (create-info position, base health/power formulas, taxi
  word 0, equipment-cache pass, ammo id, validated starter
  actions) and `realmcharacters` sync. Config keys added:
  `CharactersCreatingDisabled`, `CharactersPerRealm`,
  `AllowTwoSide.Accounts`, `StartPlayerLevel`,
  `GM.StartLevel`, `StartPlayerMoney`, `MinPlayerName`,
  `StrictPlayerNames`.
- First login: `AddStartingItems` subset at materialize
  (played-total 0 + empty inventory): best-slot equip,
  bag slots, backpack stacks. Side fix: created items now
  write prototype max durability (was 0 for all reward
  items).
- Delete: loading-character guard, guild-leader refusal,
  owner/account gates, online logout, guild/group/petition
  live removal, method 0 cascade with COD mail return to
  sender and pet sub-table cleanup, method 1 soft unlink,
  realm recount; `CharDelete.Method/MinLevel` honored and
  the `DeleteOldCharacters` keep-days sweep runs at world
  start (fork `LIMIT 0,2`).
- Recorded unmodeled inside this step: `ValidateAppearance`
  `CharSections` gate (tamper-only; waiver candidate),
  `ValidateName` profane/reserved DBC regexes (no stores),
  COD-return live `AddMail` delivery to an online sender
  (visible after relog), item-prototype validity drops in
  COD return, `CanEquipNewItem` proficiency recheck +
  second equip pass in the starter grant, corpse-to-bones
  on delete (corpse system is the next B0), and the fork's
  out-of-bounds bag-slot writes in the equipment-cache
  pass (capped, not replicated).
- `CMSG_CHAR_RENAME` remains a stub (still B0, not part of
  this step).

Evidence: loopback `character_create_and_delete_round_trip`
(create → enum → first login + grant → logout → delete →
enum) green; live MySQL `create_and_delete_round_trip` in
`live_characters.rs` (method 0, method 1, keep-days sweep
visibility) green on the bench clone; full `ci.sh` exit 0
(`/tmp/ci_t4_charlife6.log`); bench mangosd rebuilt and
restarted (listening on 18099), `TWO-CLIENT-SMOKE-PASS`.
No production or droplet access occurred. Next: player
death/corpse/ghost flow.


## Slice 2 step 2 CLOSED: player death / corpse / ghost / spirit resurrection

Fork oracle: `Player::KillPlayer`, `BuildPlayerRepop`,
`RepopAtGraveyard`, `ResurrectPlayer`, `Corpse::Create/
SaveToDB/LoadFromDB`, `ConvertCorpseForPlayer`,
`HandleRepopRequest/ReclaimCorpse/SpiritHealerActivate`.

Landed:
- `mangos-entities::corpse` (types, appearance snapshot,
  bones conversion, `CorpseSaveRow`) + `Player.death_timer_ms`.
- Map corpse storage/cells/visibility + `mangos-map::death`:
  `KillPlayer` (creature-kill 10% durability + death packet,
  PvP-death flavor, CORPSE state, 6-min forced repop timer,
  reclaim-delay ladder 30/60/120 with the fork quirk that a
  disabled delay config still returns the 30s base), repop
  (ghost 8326 / wisp 20584+20585, water-walking, corpse
  creation with appearance/equipment fields, reclaim-delay
  notice, closest-graveyard teleport event), reclaim
  (radius + delay gates, 50% resurrect, bones), resurrect,
  spirit-healer gate (NPC flag 0x20, interact range),
  resurrect + sickness 15007 + durability loss, corpse
  expiry (3-day reclaimable, 60-min bones).
- `PLAYER_FLAGS_GHOST` derived from `SPELL_AURA_GHOST`
  holders (`HandleAuraGhost` recompute, set/clear on
  apply/remove — also drives the ghost save/relog flag).
- Persistence: `mangos-db::corpse` (REPLACE save, delete by
  owner, boot load with appearance JOIN, guid seed); actor
  drains `CorpseSave`/`CorpseDelete`, boot-loads corpses and
  seeds each map at first creation (expired-at-boot corpses
  convert to bones + row delete).
- Actor arms for CMSG_REPOP_REQUEST (346),
  CMSG_RECLAIM_CORPSE (466), CMSG_SPIRIT_HEALER_ACTIVATE
  (540) incl. the cross-map corpse-graveyard compare and
  teleport; character delete now converts the corpse to
  bones. BG deaths keep the Tier 3 spirit-guide wave
  (`kill_unit` hooks `kill_player` only outside BGs).
- Config: `Death.SicknessLevel`, `Death.CorpseReclaimDelay
  .PvP/.PvE`, `Death.Bones.World/.BG` wired into MapConfig.

Recorded unmodeled: one-tick JUST_DIED intermediate,
`HasPendingMovementChange` repop deferral, spirit of
redemption / soulstone self-res, BG insignia looting,
`MSG_CORPSE_QUERY`, in-instance entrance-corpse reclaim
shortcut, spirit-healer interact range uses a temporary
50 yd bound (fork 11.1 combined radii), ghost owner-only
visibility nuances.

Evidence: `live_death_loop` green (`LIVE-DEATH-PASS`:
kill → CORPSE, repop → ghost flag + 8326 + CorpseSave +
graveyard teleport (safe loc 4, Westfall), reclaim blocked
inside 30s delay then 50% resurrect + CorpseDelete, second
death → spirit resurrect + 15007 + bones); MySQL
`corpse_save_load_delete_round_trip` green (6/6
live_characters incl. create/delete after adding a
create-serialization lock — parallel `MAX(guid)+1` tests
raced); full `ci.sh` exit 0 (`/tmp/ci_t4_death5.log`);
bench rebuilt/restarted (18099, "Loaded 0 persisted
corpses"), `TWO-CLIENT-SMOKE-PASS`; guid 1 verified
intact (Ruikanis, 19, map 0), bench `corpse` table empty
after probes. No production or droplet access occurred.
Next: character rename, then inventory/NPC services.

## Slice 2 step 3 CLOSED: character rename

Fork oracle: `HandleCharRenameOpcode` +
`HandleChangePlayerNameOpcodeCallBack`
(`CharacterHandler.cpp:799,842`).

Landed: `CMSG_CHAR_RENAME` (711) real handler —
normalize (`CHAR_NAME_NO_NAME` 69), `CheckPlayerName`
codes, reserved gate for `SEC_PLAYER` (75), then one
`rename_character` DB op carrying the fork gate
(account ownership + `CHARACTER_FLAG_RENAME` 0x4000 +
name free; every miss replies `CHAR_CREATE_ERROR` 47)
and update (clear 0x4000, set
`RENAME_NEEDS_GM_REVIEW` 0x8000). Success replies
`SMSG_CHAR_RENAME` `RESPONSE_SUCCESS` + guid + name
and broadcasts `SMSG_INVALIDATE_PLAYER`. The fork's
async callback collapses to one actor call; its
name-cache update has no Rust counterpart (name
queries read the DB). `ValidateName` profane/reserved
DBC regexes remain recorded-unmodeled (same as create).

Evidence: loopback `character_rename_round_trip`
(success + invalidate + enum, second rename 47,
digit name 73) green, suite 6/6; live MySQL
`rename_gate_and_update_round_trip` (no-flag /
wrong-account / taken-name negatives, success flag
flip) green, suite 7/7; `ci.sh` exit 0
(`/tmp/ci_t4_rename.log`); bench rebuilt/restarted.
Next: inventory mutation + NPC services.

## Slice 2 step 4 plan: inventory mutation + NPC services

Audit (2026-10-29 tree): the whole family is stubbed —
`SWAP_ITEM/SWAP_INV_ITEM/AUTOEQUIP/SPLIT/DESTROY`,
vendor `LIST_INVENTORY/BUY/SELL/REPAIR`, bank, trainer,
taxi, talents, stable have no handlers. There is no
live Item entity: inventory is DB rows on the player
(`saved.inventory`), and `build_save_bundle` writes
every row `Unchanged`, so even death-loop durability
loss does not persist today; grants bypass via
`create_reward_item` DB events.

Substrate (fork `uState` diff pattern already used for
spells/skills/quests): `deferred.inventory` becomes
the live row set, `saved.inventory` the baseline;
save diffs by item guid (New/Changed/Removed —
`mangos-db` statements already exist), rebaseline
after save. Then in order: (4a) substrate with zero
behavior change; (4b) slot ops (swap/move/auto-equip/
split/destroy + visible-item refresh + equip error
packets); (4c) vendor list/buy/sell/repair; (4d) bank;
(4e) trainer; (4f) taxi; (4g) talents; (4h) stable.
Each sub-step gets loopback/live evidence before the
next.

4a/4b1 evidence: substrate live (`deferred.inventory`
+ save diff + rebaseline; `inventory_diff_states`
unit test; grants/destroys keep both sets in step as
they are DB-immediate); item creates at login
(`mangos-entities::item::item_object` +
`Map::build_init_items`, loopback login asserts both
seed item GUIDs in the update stream); `ci.sh` exit
0 (`/tmp/ci_t4_inv4a.log`, `/tmp/ci_t4_inv4b1c.log`);
bench rebuilt/restarted. Note: durability loss and
all future row mutations now persist through the
save diff (previously written `Unchanged`).

4b2 evidence (slot ops CLOSED): new
`mangos-map/src/inventory.rs` (~2000 lines) with fork
positional model (`InvPos`, bag/slot validation,
bank reachability), `CanUseItem`/`CanEquipItem`/
`CanUnequipItem`/`CanStoreItem`/`CanBankItem`/
`FindEquipSlot` transcriptions and the
`SwapItem`/`SplitItem`/`DestroyItem`/
`AutoEquipItem`/`AutoEquipItemSlot` flows; snapshot-
diff client sync (create / values-with-mask-clear /
destroy) and `SMSG_INVENTORY_CHANGE_FAILURE` codes
verified against `ItemDefines.h`. Fork-exactness
catches during verification: `CanEquipItem` gate
order (stunned/combat → `FindEquipSlot` → `CanUseItem`
→ unique) with `EQUIP_ERR_ITEM_CANT_BE_EQUIPPED` (20)
for no slot (`Player.cpp:9666-9678`); split clones
take fresh GUIDs from the boot-seeded counter
(`max_item_guid` + `observe_item_guid`, also observed
at login build as belt-and-braces). Actor dispatch:
`CMSG_AUTOEQUIP_ITEM` 266, `CMSG_SWAP_ITEM` 268,
`CMSG_SWAP_INV_ITEM` 269, `CMSG_SPLIT_ITEM` 270,
`CMSG_AUTOEQUIP_ITEM_SLOT` 271, `CMSG_DESTROYITEM`
273. Loopback `inventory_slot_ops_round_trip` green
(move → split clone create → clone destroy-object →
wrong-slot reject 20 → unequip → auto-equip → reject
again proving source occupied); full loopback 7/7,
robustness 3/3, `ci.sh` exit 0
(`/tmp/ci_t4_inv4b2g.log`), bench rebuilt/restarted
(pid 56782), `TWO-CLIENT-SMOKE-PASS`. Test-harness
lessons recorded: PONG is socket-answered ahead of
the actor tick (flush barriers must ride the actor
FIFO; now `CMSG_NAME_QUERY`), and the fork overspeed
ping tracker kicks pings inside a 27s interval.
UNMODELED (recorded, not dropped): `CMSG_DROP_ITEM`
272 (ground-item creation), `CanTakeMoreSimilarItems`
max-count gate position in `CanEquipItem`,
`m_weaponChangeTimer`, trade-window split arm,
gift-wrap/container-loot persistence fields,
auto-unequip mail fallback. Next: 4c vendor.

4c evidence (vendor CLOSED): new
`mangos-world-data::vendor_items` (template-first
`LoadVendors`/`LoadVendorTemplates` with fork
validity gates, discovered-item side effects applied
via `ItemPrototypes::mark_discovered`, unused-
template post-check; the fork's accidental
template-duplicate slot lookup is reproduced and
documented), `mangos-world-data::durability`
(cost/quality DBC tables + fork cost formula), and
`faction_template` rows on `FactionStore`
(template → reputation faction, friend/hostile
helpers). New `mangos-map/src/vendor.rs` (~1500
lines): `GetNPCIfCanInteractWith` gate (with
feign-death + interacting-interrupt tail, reaction
hostile check, unfriendly block, 5yd boundary
distance), `SendListInventory` (class/race/rep/
condition filters, movement pause, 128 cap, sold-out
rows still listed), `BuyItemFromVendor` (+ in-slot
bag resolution, limited-stock counters with
random/dynamic restock cadence, honor/reputation/
condition gates, inventory + equipment placement),
sell (charge/durability value scaling, partial-split
clone, cast-item interrupt), buyback (ring cursor
including the fork empty-slot overwrite quirk,
login-relative expiry timestamps), repair
(single + all in fork slot order). Actor dispatch:
414 list, 416 sell, 418/419 buy, 656 buyback, 680
repair. Loopback `vendor_round_trip` green (list
bytes incl. stock/price, buy remaining-stock 4,
over-stock reject 1, sell split silent, buyback,
repair durability 11→20 in the values stream);
loopback 8/8, robustness 3/3, `ci.sh` exit 0
(`ci_t4_vendor5` run), bench rebuilt/restarted
(pid 63218; boot log `Loaded 207 vendor template
items, 11425 vendor items`), `TWO-CLIENT-SMOKE-PASS`.
Bench note: `/tmp` cleanup had deleted
`bench-lan.conf` (and the mysql probe helper) over
the long idle gap; both were recreated (config from
the template keys + `.env.test`, DataDir expanded).
UNMODELED (recorded in `vendor.rs` header):
random-property generation on bought items (global
grant gap), quest item add/remove update packets on
buy/sell, forced reputation rank overrides,
money-trade audit logging, dynamic-restock global
session count (map-local players used; threshold
unreachable per-map in practice). Next: 4d bank.

4d evidence (bank CLOSED): `CheckBanker` /
`banker_activate` (`SMSG_SHOW_BANK` 440, records
`current_banker_guid`) and `buy_bank_slot`
(`BankBagSlotPrices.dbc` loaded into stores;
failure-only `SMSG_BUY_BANK_SLOT_RESULT` 442,
success writes `PLAYER_BYTES_2` byte 2, persisted
by the existing save path). The vendor interact
gate was split into a pure `npc_can_interact` plus
preamble wrapper; inventory `can_use_bank` now runs
the full fork gate against the recorded banker
(5yd boundary distance, replacing the 4b-era 11.1yd
approximation). Actor dispatch 439/441. Loopback
`vendor_round_trip` extended: show-bank packet,
silent slot purchase with bytes-2 = 1 in the values
stream, backpack→bank move accepted at the banker;
loopback 8/8, `ci.sh` exit 0, bench
rebuilt/restarted, `TWO-CLIENT-SMOKE-PASS`.
UNMODELED: `.bank` GM command-table security check
(GM-on proxy in map). Next: 4e trainer.

4e evidence (trainer CLOSED): spell-data
prereqs landed first — `SkillLineAbility.dbc`
loader (new DBC format), `SpellStore.skill_line_abilities` +
`spell_chains` (`derive_spell_chains` ability pass
incl. the 2366/20154 fork quirks, `spell_chain`
DB overlay via `spell_chains.rs`; talent-seeded
chains deferred to the 4g talent slice).
`mangos-world-data::trainer_spells` loads
templates→creatures→greetings with the fork gates
(LEARN_SPELL wrapper check, trainer-flag check,
template-duplicate rejection, `reqLevel` default =
wrapper spellLevel, profession trainer_type=2);
`IsSpellValid` + talent-wrapper skip recorded
UNMODELED in the module header. Map side
(`trainer.rs`): `IsTrainerOf` (class/pet/mount/
tradeskill branches incl. wrong-class gossip menus
and the fork's mount switch reading `trainer_class`
for race ids), `IsSpellFitByClassAndRace`,
`GetTrainerSpellState` (incl. wrapper-misc-value
profession quirk and primary-profession
GREEN_DISABLED), list encoding with chain req/prev
ordering, buy via the normal cast path (visual-222
self-cast rule, charge only on cast start).
Cast-engine fix discovered by the slice: target
mode 0/18 now runs the fork's caster-dest
per-effect fill for the modeled groups
(LEARN_SPELL family → explicit unit target;
3200 of 4676 bench trainer wrappers are mode-0).
Loopback `trainer_round_trip` (list GREEN/RED/GRAY,
mage gossip 328, level-gated TRAIN_FAIL 2, learn +
coinage 100→50 + GRAY re-list) green; loopback
9/9, `ci.sh` exit 0, bench boot `Loaded 6051
trainer spells` (3 loader diagnostics), bench
rebuilt/restarted (pid 66826),
`TWO-CLIENT-SMOKE-PASS`. Test-helper fix:
`values_field` no longer aborts the whole search on
the first unparseable non-update packet body.
UNMODELED: buy-path `IsWithinLOSInMap`, greeting
locales. Next: 4f taxi.

4f evidence (taxi CLOSED): `TaxiNodes.dbc` format
added; `mangos-world-data::taxi::TaxiStore`
(graph, vertices, `sTaxiNodesMask` with the
spell-path exclusion + no-outgoing-paths quirk,
`taxi_path_transitions` with fork row gates).
Map side (`taxi.rs` + `Generator::Flight` in
`motion.rs`): status query, learn-node + menu,
`ActivateTaxiPathTo` (all reply codes, known-node
and taxi-cheater gates, mounted/shapeshift/casting
gates, fork `(2*D)^3` distance quirk, express
stitching with transitions and per-leg boundary
charging, reputation-discount fares), flight
spline at 32 yd/s with mount display + client-
control flags, landing finalize, multi-leg
continuation, cross-map continuation
(`TeleportPlayer` + `TaxiResume` worldport-ack
relaunch; the 7 cross-map DBC paths are transport
routes, unreachable as player taxi), relog
`ContinueTaxiFlight` (fork proximity start-node
search) + `LoadTaxiDestinationsFromString`
integrity relocate in player materialize,
`SPELL_EFFECT_SEND_TAXI` effect arm. Human players
now get the fork-always-present idle MotionMaster
at `add_player` (previously bot-only). Loopback
`taxi_round_trip` (learn both nodes, menu mask,
reply OK + spline + fare 100→90 + mount up/down)
green; loopback 10/10, robustness 3/3, `ci.sh`
exit 0; bench boot `Loaded 85 taxi nodes, 287
taxi paths, 9582 path vertices`, bench
rebuilt/restarted (pid 69852),
`TWO-CLIENT-SMOKE-PASS`. UNMODELED in `taxi.rs`
header: trade cancel at flight start, anticheat
report, pet reagent return, honorless cast /
hostile-ref offline toggle. Next: 4g talents.

4g evidence (talents CLOSED): `TalentEntry` /
`TalentSpellPos` on `SpellStore` (Talent.dbc +
TalentTab.dbc at boot), talent-seeded spell chains
in `derive_spell_chains` (fork order: talents →
abilities → DB overlay). Map side (`talents.rs`):
`LearnTalent` (class gate, rank/points/depend/
row-spent gates), map-side `learn_spell_full` /
`remove_spell_full` (talent rank replacement with
`SMSG_REMOVED_SPELL`, used-point counting,
primary-profession point adjust — also closing
the 4e trainer profession-point gap — passive-like
cast on learn), `UpdateFreeTalentPoints` /
`InitTalentForLevel` (login recount; `give_level`
switched from delta hack to fork semantics),
`ResetTalents` with the config cost curve
(`Rate.Respec*` wired through ServerConfig →
MapConfig; monthly decay at 1.12.1), wipe-confirm
handler + `CHARACTER_FLAG_RESET_TALENTS_ON_LOGIN`.
Pet `GetTPForSpell` now reads the SkillLineAbility
store. Loopback `talent_round_trip` (learn rank 1
points 3→2, row-locked refuse, rank-up replace
used=2, wipe money-gate) green; loopback 11/11,
`ci.sh` exit 0, bench rebuilt/restarted (pid
71555), `TWO-CLIENT-SMOKE-PASS`. UNMODELED in
`talents.rs` header: non-talent spellbook
supersede, dependent-spell autolearn, pet reagent
return. Next: 4h stable.

**4h stable (pet stable master) — CLOSED.**
`mangos-map/src/stable.rs`: fork `NPCHandler.cpp`
stable arms — `MSG_LIST_STABLED_PETS` list encoding
(live pet or slot-0 row first, stabled rows after,
count patched back at body offset 8),
`HandleStablePet` (alive gate, stablemaster +
hunter gate per `Player.cpp:2526`, free-slot scan,
fork quirk `free_slot <= stable_slots`),
`HandleUnstablePet` (slot-range, tameable template,
current-pet refusal), `HandleBuyStableSlot`
(`StableSlotPrices.dbc` in `WorldStores.`
`stable_slot_prices`, `STABLE_ERR_MONEY`),
`HandleStableRevivePet` (fork empty handler — true
no-op), `HandleStableSwapPet`. New
`pets::load_pet_by_number` / `PetLoadKind::`
`StableOut` over the refactored `load_pet_with_row`
(slot promotion applies). Actor dispatch: six
opcodes added to the vendor packet queue. Loopback
`stable_round_trip` (hunter fixture: current 501 +
stabled 502; list, buy slot 2 @ 50c, swap, stable,
unstable, silent revive) green; loopback 12/12,
`ci.sh` exit 0, bench rebuilt/restarted (pid
73599), `TWO-CLIENT-SMOKE-PASS`. UNMODELED in
`stable.rs` header: GM/self-guid `CheckStableMaster`
(`stable` chat command), cache-iteration order.
Next B0: dynamic GO creation + GO casting +
`SPELL_EFFECT_SUMMON_OBJECT`.

**B0-5 dynamic GO / GO casting / SUMMON_OBJECT —
CLOSED (runtime).** New `mangos-map/src/`
`gameobjects.rs`: `GameObject::spawn_dynamic` +
fork-exact `is_spawned` (`GameObject.h:118`),
`Map::summon_gameobject` (`Object.cpp:2423`),
`unit_add/remove/remove_all_gameobjects`
(cooldown-on-event hold, owner cleanup on
RemoveFromWorld), dynamic `GameObject::Update`
subset (arming, timer expiry, charges,
JUST_DEACTIVATED delete/hide), trap trigger arm
(hunter selector over engine hostility
approximation, owner-cast; environmental
player search, GO-cast), `go_cast_spell`
(SpellCaster GO caster: fork `GetLevel`,
no-power, cast-time via pending triggered,
channeled/missile-delay recorded), `go_use_direct`
+ `go_trigger_linked` (trap/door/button/chest/
spell-focus/spellcaster arms), goober spellId
tail cast wired in the opcode path.
`SPELL_EFFECT_SUMMON_OBJECT_WILD` (76) + `SLOT1-4`
(104-107) with object slots, `SMSG_GAMEOBJECT_`
`SPAWN_ANIM`, linked-trap summon, WSG dropped-flag
registration + deletion. `bg_add_object`
implemented (blueprint model), `SpawnBGObject`
respawn-time semantics, `HandleTriggerBuff`
(AB buff objects now materialize). Content
exemplars wired: scarlet cannon, Urok runes.
Runtime-created GOs now set `scan_due` (stationary
viewers previously never saw them). Loopback
`summon_object_round_trip` (env trap GO-cast,
slot summon + hunter trap fire, slot replacement,
wild summon create) green; loopback 13/13,
`ci.sh` exit 0, bench rebuilt/restarted (pid
78164), `TWO-CLIENT-SMOKE-PASS`. Remaining
content burn-down (already recorded in-tree):
16 `NEEDS-MAP-HELPER: SummonGameObject` sites in
`content.rs` + 2 in `gossip.rs` now have their
helper (`Map::summon_gameobject`/`go_use_direct`)
and are mechanical per-site follow-ons.
UNMODELED in `gameobjects.rs` header: static GO
loot-state machine beyond traps, fishing bobber,
spell-focus `focusObject`, just-summoned GO-AI
notify. Next B0: escort framework
(`npc_escortAI`/`FollowerAI`).

B0-6 escort/follower framework CLOSED
(runtime): `crates/mangos-map/src/escort.rs`
ports `npc_escortAI` (`ScriptedEscortAI.cpp`:
Start/Stop/SetEscortPaused/SetRun, waypoint tick
with ctor 2500ms first delay, 1s player/group
100yd range check, end-of-path return-home vs
questgiver-flag + disappear + optional instant
respawn, MovementInform incl. POINT_LAST_POINT
combat return and >1000yd quirk, EnterCombat
combat-start record, evade-to-anchor instead of
home) and `FollowerAI` (`ScriptedFollowerAI.cpp`:
StartFollow/Paused/Complete, 300ms leader/range/
quest-status abort tick, leader group fallback,
JustDied group quest fail). Quest engine:
`fail_quest` (repeatable-remove vs FAILED-in-
place, timed packet split), `group_event_fail_
happens`, `group_event_happens` +
`area_explored_or_event_happens` in `xp.rs`.
Hooks: map tick, movement informs, fresh
`enter_combat`, `enter_evade_mode`, `kill_unit`,
despawn cleanup; script dispatchers
`creature_script_escort_waypoint_reached/_start`,
`just_started_escort`, `escort_update`,
`escort_failed` in `content.rs`. Exemplar wired
end-to-end: `npc_daphne_stilwell` (westfall.cpp:
quest-accept start, waypoint summons/sheath/
virtual item/SetRun/prologue/GroupEventHappens
at 17, shoot update, reset texts; ranged
AttackStart override recorded). Volcor +
celebras spirit quest-accept stubs upgraded to
real `escort_start` (script remainders recorded
in-tree). Loopback `escort_round_trip` green
(accept → MONSTER_MOVE launches → 100yd leash
→ SMSG_QUESTUPDATE_FAILED 1651); loopback 14/14,
`ci.sh` exit 0, bench rebuilt/restarted (pid
82090, 1542 script waypoint nodes from bench
world DB), `TWO-CLIENT-SMOKE-PASS`. Fixture fix
with bench-wide meaning: loopback fake character
world_phase_mask was 0 (WORLD_ALONE) — fork
default is WORLD_DEFAULT_CHAR 0x01; the loopback
player previously saw no creatures at all.
Remaining burn-down (mechanical, helpers exist):
~10 gossip-start escort stubs in `gossip.rs`
(`npc_escortAI::Start` UNMODELED arms), 13
`NEEDS-MAP-HELPER: MoveFollow` sites in
`content.rs`, per-script escort AI ports
(WaypointReached/update arms). UNMODELED in
`escort.rs` header: AssistPlayerInCombat LOS
assist, default UpdateEscortAI passive-aggro
pull, exact size-adjusted distances,
DisappearAndDie corpse-delay arm, fork
Point-generator resume (WaypointStart can
refire after combat). Next B0: HonorMgr
rank/standing/daily-weekly.

B0-7 HonorMgr CLOSED (runtime): per-player
honor manager in `mangos-entities/src/honor.rs`
(rank bands/visual/bar exactly fork
`CalculateRank`, CP rows, DK immediate RP
subtract, `Update()` field recompute at login
and save), kill flows in
`crates/mangos-map/src/honor.rs`
(`RewardHonorOnDeath` damage-history group/
solo split with `GetHonorGain` full patch
gates, civilian DKs, racial-leader 488,
`SMSG_PVP_CREDIT` incl. victimless BONUS —
correcting the earlier BG comment), damage
history recorded in `deal_damage_school`,
hooks in `kill_unit`/kill-XP recipients. The
existing procs-only `IsHonorOrXPTarget`
subset (player victims always true) was
replaced by the fork-full version (grey-level
gate for players too). Weekly maintenancer in
`mangos-db/src/honor.rs`: saved-variables init,
marker-gated weekly recompute at boot
(standings distributions 1.12/legacy BRK sets,
RP earning/decay/level caps, inactive decay,
city-protector flags from last week's
standings, flush + CP cleanup + HCR report to
`HonorDir`), runtime `CheckMaintenanceDay`
marker + log in the world actor. Config:
`MaintenanceDay`, `MinHonorKills`, `RpDecay`,
`PvP.PoolSizePerFaction`,
`PvP.DishonorableKills`, `PvP.CityProtector`,
`AutoHonorRestart`, `PvP.AccurateTimeline`,
`PvP.AccurateRewards`, `TimeZoneOffset`,
`HonorDir` added to `ServerConfig` (+ map
bundle). Fork `%.1f` rounding on CP/RP flush
writes. Loopback `honor_login_fields_round_
trip` green (all honor client fields exact
from seeded CP rows); db math unit tests
green; loopback 15/15 via `ci.sh` exit 0;
bench rebuilt/restarted (pid 85497): boot
maintenance really ran (bench marker was 1;
0/0/0 scores, HCR written, days advanced —
bench fixture then fast-forwarded to the
current week since its stored days lagged
real time by design of the fork catch-up),
`TWO-CLIENT-SMOKE-PASS`. UNMODELED recorded:
`LOG_HONOR` per-add log lines, AutoHonorRestart
shutdown scheduling (marker+log only),
quest-type honor spell effect, GM `.honor`
commands, honor refresh on the pending-
teleport save path. Next B0: game-event
manager.

B0-8 game-event manager CLOSED (data-driven
core): `crates/mangos-world-data/src/
game_events.rs` ports the `GameEventMgr`
loader and schedule verbatim (`CheckOneGame
Event` with leap-day offset, `NextCheck`,
`Update` min-delay fold; patch_min/max event
windows; creature-data max-patch-<= subquery;
quest patch_min filter). `crates/mangos-map/
src/game_events.rs` gates `spawn_creature`/
`spawn_gameobject` on the signed event id
against the actor-pushed active set, applies
transitions to loaded cells (despawn/material
ize the signed guid lists), and applies
`game_event_creature_data` at spawn and on
toggle (`UpdateEntry` subset via the existing
`creature_update_entry`, display, equipment,
`ApplyGameEventSpells` with aura-removal +
triggered self-cast). `Map::game_event_active`
now reads real state. World actor runs the
schedule (`Initialize` from boot-read `game_
event_status` + truncate in `mangos-db/src/
game_events.rs`, INSERT/DELETE on transitions,
fork log lines, `LANG_EVENTMESSAGE` announce
behind `Event.Announce`), seeds new maps, and
gates event quests at accept (`UpdateEvent
Quests` equivalent). UNMODELED, recorded: the
7 hardcoded `WorldEvent` impls in
`HardcodedEvents.cpp` (WarEffort, Scourge —
disabled in the bench DB, ElementalInvasion,
DragonsOfNightmare, DarkmoonFaire,
FireworksShow, ToastingGoblets) — fork skips
hardcoded events in the schedule loop, so
their spawns/quests stay inactive exactly as
before this slice; `SendEventMails` (the
`game_event_mail` table has 0 rows in the
world-content DB); pool event spawn/unspawn
(pool manager itself is a pre-existing gap);
quest-giver menu hiding for inactive event
quests (accept is refused; the menu may still
list them); full `UpdateEntry` stat re-init.
Verification: scheduling unit tests
(`check_windows`) green; loopback
`game_event_spawns_round_trip` green (positive
event spawn visible at login, negative never
materializes); `./ci.sh` exit 0 (107 suites);
bench rebuilt/restarted (pid 89391): `>> Loaded
94 game events.`, 6 active at boot (27 Night,
46/47/48 fishing cycles, 83 AQ Gate, 152
Southshore Assassins), `game_event_status`
holds exactly those 6 rows, `TWO-CLIENT-
SMOKE-PASS`. Remaining B0: spell NEEDS-HOST
seams, config-key parity.

B0-9 spell NEEDS-HOST seams, batch 1: census of
the 72 markers in `spell_scripts.rs` found 35
stale (seams landed in earlier slices:
OnSetTargetMap 15, OnPeriodicTrigger 7,
OnCheckTarget 6, OnAfterHit 2,
remove_some_cooldowns visibility 3, misc 2 —
comments only, runtime already correct) and
~31 genuinely unwired. New host seams wired
with fork callsites: `OnHit` (Spell.cpp:1489
family; hit path in apply_effects gated on
can_trigger, miss path in finish_cast) —
PW:Shield now applies Weakened Soul 6788;
`OnProc` override protocol in procs.rs —
Mage Ignite (rank %s, stack accumulate +
ModStackAmount + Refresh semantics on holder
fields, fresh custom-bp apply) and Combustion
counter; `OnCast` (Spell.cpp:3827) — Elune's
Candle entry-keyed sub-spell; `OnSuccessful
Finish` (Spell.cpp:4519, immediate path) —
Holy Nova heal sub-spell, Cannibalize aura,
Wolfshead energy; `OnSuccessfulDispel`
(SpellEffects.cpp:2521) — Devour Magic heal;
`OnPeriodicTickEnd` — Nefarian mage class-call
polymorph forcing; `OnSuccessfulStart`
(Spell.cpp:3585) — spell_instant_cast cast-time
zeroing; `OnSummon` dispatcher gained Warlock
Inferno (1122/24670 enslave+root+burn). All
script_name bindings verified present in bench
spell_template. `./ci.sh` exit 0 (107 suites);
bench restarted (pid 91187), TWO-CLIENT-SMOKE-
PASS. Remaining seam work: OnSummon stub bodies
(target dummy faction/flags, chained essence
whisper, Umi yeti follow, vanquished tentacle
charm-stay, eye-of-naxx attacker-for-helper),
OnInit per-cast state (gnomeregan fallout,
darkmoon tonk pet unsummon), aura remove-mode
threading (grobbulus 28206 dispel branch,
combustion buff cancel, blood fury — the last
build-gated out at 1.12.1), holder script-state
storage (GDR trio, five-finger trio),
shape-lost strip + DBC Stances column
(sweeping strikes), channeled OnSuccessful
Finish site. Curse-of-agony dummy correctly
dead (fork `#if <= 1_10_2`).

B0-9 batch 2: OnSummon stub bodies completed
and registered (target dummy faction +
PLAYER_CONTROLLED flags assign, chained essence
whisper 4438-4445, Umi's yeti area follow/text/
waypoint resume, vanquished tentacle stay pin,
eye-of-naxxramas attacker-for-helper
AttackStart); `OnInit` per-cast state
(`ActiveCast.script_effect_pick`, rolled at
cast_spell top, threaded to OnEffectExecute) —
gnomeregan collecting fallout fires exactly one
pre-rolled effect; aura remove-mode threading
(`AuraRemoveMode::{Default,Cancel,Dispel,
Expire}` through remove_aura_at_mode into
OnBeforeApply) — Grobbulus injection now casts
Mutagen Explosion 28206 with/without aura
context by dispel, combustion buff cancel drops
the counter aura. `./ci.sh` exit 0; bench
restarted (pid 92466), TWO-CLIENT-SMOKE-PASS.
Still recorded: holder script-state storage
(GDR, five-finger), temporary-pet pair
(darkmoon tonk), shape-lost strip + DBC Stances
(sweeping strikes), channeled finish site.
Next B0: config-key parity audit.

Config parity progress + LOOT FINDING: fork-read
inventory extracted from World.cpp (566 keys).
Shared-key defaults verified against fork CODE
defaults (dist template is stale in ~10 spots;
Rust matches code). Real default divergence
fixed: instance/BG visibility was hardcoded 100
(fork 170/533) — Visibility.Distance.* +
Grey/Relocation/ForceActiveObjects now read and
applied per map kind; Rate.Creature.* (15) +
Corpse.Decay.* (5) + Rate.Creature.Aggro read
into CreatureRates/MapConfig (entities decay
default corrected to fork 300/600/1200/3600/
900). Classification of the remaining unread
set: Anticheat/Warden (125, no runtime — waiver
candidate), logging/threading/perf (~90, N/A in
actor model), GM/RA/SOAP admin surfaces,
DynamicRespawn system, OutdoorPvP enables
(system waiver), ~150 gameplay scalars whose
Rust constants already equal fork defaults
(parity holds unless prod conf overrides —
Slice 5 runbook MUST diff the droplet
mangosd.conf keys against the Rust-read set).
`./ci.sh` exit 0 (summon_object flake isolated-
green twice + loopback 16/16); bench pid 93857,
TWO-CLIENT-SMOKE-PASS.

TOP FINDING (B0-10, outranks remaining config):
creature/GO loot generation does not exist.
No loot-template loading, no per-corpse loot,
no CMSG_LOOT/MONEY/RELEASE handlers — kills
grant XP only ("Rust Map holds no loot",
content.rs). The Slice 1 audit under-scoped
this to 4 ancillary opcodes. Next slice: loot
core (template loaders, generation, corpse
loot state, loot window handlers).

B0-10 loot core (creature corpse scope) CLOSED:
all nine `*_loot_template` loaders are in
`mangos-world-data::loot` with the fork patch
window + forbidden-items query, template/group/
reference generation, money generation, and
`map_loot_disabled` (0 rows on bench). Map side
(`mangos-map::loot`) now has fork-shaped taps
(first player-side damage, pet-tap replacement,
clear on evade/respawn/despawn), kill-time
`GenerateLootForBody` after XP, allowed-looter
reward-distance fills for quest/FFA/conditional
items, per-viewer dynamic flags (tapped/lootable),
`SendLoot`/`LootView` serialization, and handlers
for `CMSG_LOOT`, `CMSG_LOOT_RELEASE`,
`CMSG_LOOT_MONEY`, `CMSG_AUTOSTORE_LOOT_ITEM`,
and `CMSG_LOOT_MASTER_GIVE`. Group loot and
need-before-greed rolls are implemented
map-locally with the fork voter gates
(allowed looter + `AllowedForPlayer` + loot XP
distance, plus `CanUseItem` for NBG), 60s
timeout, single-looter auto-win, need/greed/pass
resolution, and the fork's roll-grant quirks
(no item-push packet; need/greed store-fail and
offline-winner arms differ exactly as in
`CountTheRoll`). Verification: new loopback
`loot_round_trip` green (kill fixture critter,
loot window shows fixed 137 copper + 2x Linen
Cloth, autostore grants + removed notify, money
take moves coinage 100000 -> 100137, release
responds); `./ci.sh` exit 0 (107 suites); bench
rebuilt/restarted (pid 98880): `>> Loaded 3602
creature loot templates.`, TWO-CLIENT-SMOKE-PASS
after resetting guid 3/4 to the shared Durotar
spawn point. Recorded fork-parity security
finding for the owner gate: fork
`HandleAutostoreLootItemOpcode` does not
re-check `SendLoot` permission, so a NONE-
permission window holder who guesses a plain
unblocked slot can take it when
`AllowedForPlayer` passes; fork `CountRollVote`
also counts repeat votes per call. Both are
replicated verbatim for parity and need an
owner decision before canary if a safe-direction
deviation is desired. Still UNMODELED (recorded
in `loot.rs` header): GO chest/fishing/skinning/
pickpocket/item-open/corpse-insignia sources, AI
`FillLoot`, `CAN_WIELD_LOOT` spawn pre-gen,
actor round-robin rotation/`SendLooter`/
PARTYKILLLOG, disenchant-on-roll, damage-origin
loot gating (shared with XP), corpse-decay
acceleration, `lootForCreator`, BG-group arms,
random property ids. Slice 2 remains open for
the remaining B0 items; next: config-parity
classification close-out.

Config-parity classification close-out (Slice 2):
rebuilt the fork-code inventory from
`World.cpp`/shared/mangosd literal reads: 548
world-config keys. Exact quoted-key presence in
Rust `src/` after this batch is 184, with 364
literal-absent keys; literal absence is not
proof of unread because several families are
constructed dynamically (notably the movement
`Anticheat.*` per-check rows in
`mangosd/src/anticheat.rs`). Wired and consumed
in this batch: `MaxGroupXPDistance` (XP/loot/
honor/BG reward bands), signed
`Quests.LowLevelHideDiff` /
`Quests.HighLevelHideDiff` (negative disables,
fork-exact), `ShowHealthValues`,
`SendLootRollUponReconnect` (fixes the B0-10
pending-roll resend to be opt-in like the fork),
`AllFlightPaths` (login sets the taxi-cheat
extra flag), `AddonChannel`, `Rate.Health`,
`Rate.Mana`, `Rate.Rage.Loss`, `Rate.Energy`,
and all three `Rate.Rest.*` multipliers
(online tick + offline materialization).
`ServerConfig` unit tests cover fork-code
defaults and non-default reads for the new keys
(`defaults_match_cpp_code`,
`tier4_config_closeout_values_are_read`).
Remaining literal-absent keys classify as:
(1) no Rust runtime/admin/architecture surface
— non-movement anticheat detectors + Warden,
RA/SOAP, console/GM-ticket surfaces,
LogsDB/PerformanceLog/Smartlog, and the
MapUpdate/threading/grid/network tuning family;
(2) absent features that stay B0-or-waiver —
OutdoorPvP enables, DynamicRespawn (defaults
are all zero/off), LFG matchmaking, fishing /
skill-chance / mining knobs, and instance
reset-hour/per-hour/unload scheduling;
(3) behavior exists behind fork-default
constants, so a droplet non-default would
diverge — remaining `Rate.*` (fall, focus,
loyalty, mining, rage income, reputation,
talent, personal XP), creature family/attack/
stealth/threat scalars, chat flood and
min-level restrictions, durability loss,
environmental/mirror timers, corpse/bones
timers, movement extrapolation, visibility
in-flight/group-mode, `Spell.EffectDelay`, and
the Progression/PvP accurate toggles; and
(4) dead in the fork (e.g.
`CharactersPerAccount` has no consumer).
The local bench conf carries only 11 keys and
none of the literal-absent set, so it cannot
prove production parity: Slice 5 must diff the
droplet `mangosd.conf` (key names and normalized
non-secret values only) against the Rust-read
set, and any non-default in class (3) is a
blocker or an explicit owner waiver.

Slice 3 exact release artifacts + bench
acceptance: built `mangosd` and `realmd` in an
`ubuntu:22.04` linux/amd64 container
(glibc 2.35-0ubuntu3.15) with rustc/cargo
1.98.1 into
`vmangos-rs/target-ubuntu2204-amd64/release/`.
Verification inside the same exact container:
both are x86-64 ELF pie executables, `ldd`
resolves only system `libstdc++/libgcc_s/libm/
libc`, and the maximum required glibc symbol is
`GLIBC_2.34` (≤ the droplet 2.35 floor).
SHA-256: mangosd
`208d14e73cb17dd0c336bf6a3124e1ac90521e02aba787ea22fca8d8cc8e2cad`,
realmd
`72587f8402b1a86d4af8d542ce9baf866f19c8edcca362e6dc13fad303e64c50`.
The exact amd64 `mangosd` then booted in the
container against the cloned bench DB/DataDir on
side port 18101 (`World server listening on
0.0.0.0:18101`, 94 game events, 3602 creature
loot templates) and shut down cleanly on the
test timeout's SIGTERM. The host debug bench was
separately rebuilt/restarted after the config
batch and `TWO-CLIENT-SMOKE-PASS` re-ran green.
Slice 3 is closed locally; nothing was copied
to or installed on the droplet.

Slice 4 validation (local, partial by design):
differential — `./ci.sh` green at 107 suites
after the config batch (loopback protocol/save
fixtures are the standing differential harness;
`loot_round_trip` included). Soak — the
deterministic loopback suite passed 17/17 twice
back-to-back; the live bench two-client smoke
passes when its fixture characters are alive,
but a 5× bench soak is NOT counted: the bench
bot population owns the same BOT_ characters
and they are repeatedly saved dead
(`health=0`) between iterations, which makes
the say step fail for fixture reasons, not a
chat regression (say passes immediately after a
down-state revive + restart). Performance —
release binaries are 14 MB (`mangosd`) / 4.5 MB
(`realmd`); the debug bench RSS at boot is
~47 MB; the exact amd64 release boot reached
listening in the same log second as world load
completion. No formal tick-time/FPS harness was
run — that row is unverified beyond
footprint/boot. Real client — NOT re-run for
Tier 4 changes in this sandbox (earlier phases
proved real-client login/movement; loot/config
behavior still needs an owner real-client pass
before canary). Rollback — the Tier 4 diff
contains no SQL migrations or schema changes,
so binary-first rollback remains the Phase 6
posture; no local C++ binary exists under
`vmangos/run/bin/` for a swap rehearsal here,
so the runbook must rehearse the swap with the
droplet's preserved C++ binary at window start.

Slice 5 runbook + owner gate: `docs/
rust-migration-evidence/TIER4-CANARY-RUNBOOK.md`
records the exact artifacts/hashes, five
pre-window gates (droplet config diff without
secrets, real-client loot pass, dedicated
non-bot smoke characters, verified DB backup,
C++ binary preservation/rehearsal), seven
numbered owner waiver decisions (fork-parity
loot holes, absent loot sources,
reputation-on-kill, OutdoorPvP, content-helper
remainder, anticheat/Warden/admin surfaces,
DynamicRespawn-if-enabled), the window
procedure, abort criteria, and binary-first
rollback. **STOP here.** Slice 6 runs only
after later explicit owner approval; no
droplet read or write was performed for
Slices 3–5.

Slice 6 canary execution (2026-11-10, owner
approved all seven runbook waivers plus three
gate-1 additions below, chose canary-side smoke
over a bench release pass, and gave an explicit
go): Gate 1 diffed the droplet `mangosd.conf`
(644 keys) and `realmd.conf` (33 keys) against
the `.dist` templates and the Rust literal set —
`realmd` identical; `mangosd` 34 differing
values, of which the 20 not literally read by
Rust are all class-(1) threading/pool/network/
compression tuning plus `Console.Enable=0`
(aligned with the reduced console) and
`GM.AllowPublicChannels=1`. New class-(2)
findings enabled on the droplet and waived by
the owner for the canary: `CrowdBot.*`,
`BattleBot.AutoJoin`, `AuraTracker.Enable`.
`DynamicRespawn.*` are all zero/off (waiver 7
stays a waiver); `OutdoorPvP.*.Enable=1` makes
waiver 4 concrete; `Spell.EffectDelay=400`
equals the fork code default; `Rate.XP.*=1.4`
is read by Rust. Gate 4 backup
`backups/db/*_20261110_022425.sql.gz` (all four
databases) gzip-verified. Gate 5 preserved the
C++ binaries as `mangosd.cpp-bak`
(`483cf925…`) and `realmd.cpp-bak`
(`5078ec5a…`); staged Rust artifacts matched
the runbook hashes on the droplet. Window:
maintenance mode on, C++ units stopped (the
C++ `mangosd` ignored SIGTERM until systemd's
timeout SIGKILL; both real accounts were
offline, bot saves are on a 60 s interval),
Rust `realmd` started on 3724, and the first
Rust `mangosd` start exposed a real fork-parity
defect — it read legacy `*DatabaseInfo` keys
while fork `Master::StartDB` and the production
conf use `*Database.Info` (fork `realmd` keeps
the legacy key, so Rust `realmd` was correct).
A generated `run/bin/mangosd.conf` with legacy
alias lines got the canary up (listening
02:36:42 UTC, 4144 creature loot templates,
population climbing, no panics/save errors);
the source was then fixed in
`crates/mangosd/src/main.rs` to the fork keys,
unit/loopback tests pass (106 lib, 17/17
loopback; `summon_object_round_trip` flaked
once under load and passed isolated and on full
rerun), the exact Ubuntu 22.04 artifact was
rebuilt (`mangosd` SHA-256 now
`5b922f0270a4da96e2b94f31a2337d7da28a43d818ff1635b4c5022cf6bacb4f`,
max `GLIBC_2.34`; `realmd` unchanged), and a
second swap put the fixed binary on the plain
symlinked production conf (listening 02:50:00
UTC, same 9 non-fatal loader-diagnostic classes
as the first boot, population rebuilding again,
which also proves first-boot Rust saves reload
under the fixed binary). Rollback remains
binary-first via the `.cpp-bak` binaries and
the pre-window backup. Outstanding at record
time: owner real-client smoke on Ruikanis
(gate 2 canary-side, incl. loot + persistence),
then maintenance mode off; the droplet Discord
bot unit was already crash-looping before the
window (untouched, per non-goals); one gate-1
fetch printed the DB password into the agent
session transcript before redaction — rotate it
after the window. Tier 4 changes remain
uncommitted.

Canary movement stall (2026-11-10, owner
smoke): the owner could log in and walk
client-side, but his server position never
moved — mobs flickered at the 100 yd
visibility boundary around the login spot,
targeting worked, auto-attacks never swung.
Root cause: `CMSG_SET_ACTIVE_MOVER` parsing in
`world_actor.rs` read a *packed* GUID, while
the fork reads a raw u64
(`ObjectGuid.cpp:176-182`, plain `operator>>`).
A real client's raw GUID 1
(`01 00 00 00 00 00 00 00`) packed-parses to
empty, so `client_mover_guid` stayed `None`
and every movement packet was silently
dropped at the confirmed-mover gate. Scripted
clients sent the packed form, which is why
bench smokes passed while the real client
froze. Fixed by parsing the raw u64
(`ObjectGuid::from_wire_raw`); the two-client
smoke script now also sends the fork shape.
Verified locally with a real-client-shaped
movement probe (start-forward/heartbeat/stop
position persisted) and loopback 17/17; the
bench two-client say step still fails on the
known saved-dead bot-fixture interference,
unrelated to this fix. Canary redeployed with
`mangosd` SHA-256
`1829f2ae4398cdd95305f428e2d8fd93a9df138192da26bc410a656321fd5f8e`
(still carrying the guid-1 VISDBG trace
instrumentation until owner movement is
verified, then it comes out and the artifact
is rebuilt/re-hashed). Owner movement
verification is the outstanding gate-2 item.

Movement fix verified (2026-11-10): the owner
logged in, walked, and fought a boar; the
server-side saved position moved ~132 yd from
the login spot with zero movement-drop trace
lines. All VISDBG instrumentation was then
stripped, format/clippy/loopback re-verified,
and the clean artifact redeployed: `mangosd`
SHA-256
`9a02bedab4048e1476b2777c070e695633c1eae2c26b61e9b90bbd0a73a70c0b`
(zero VISDBG strings in the binary), listening
2026-11-10 16:25:55 UTC. Remaining smoke items
are loot gold/item plus logout/relogin
persistence, after which maintenance mode goes
off and the observation window starts.

Smoke round 2 fixes (2026-11-10): the owner
reported (1) spirit healer visible while alive
and (2) killed boar despawned instantly,
unlootable. (1) The visibility scan never
applied the fork's `IsInvisibleForAlive`
symmetry (`Unit.cpp:6526`, ghost aura 9036 /
spirit-service NPC flags); `Map::is_visible`
now gates alive viewers against
invisible-for-alive targets using the existing
`creature_invisible_for_alive` helper. (2)
Creature death never promoted `JUST_DIED` to
`CORPSE` and never armed `corpse_decay_timer`,
so the corpse failed the alive-viewer grid
rule on the next scan and was out-of-ranged
instantly; `kill_unit` now arms decay
(despawn-instantly 1 ms, else rank corpse
delay) and promotes to `CORPSE` fork-style, a
new `update_corpse_decay` tick performs
`RemoveCorpse` on expiry, and full loot applies
`AllLootRemovedFromCorpse` scaling by
`Rate.Corpse.Decay.Looted`. Regression:
`live_map` bench test extended with
corpse-persistence-across-rescan and
spirit-service invisibility assertions
(passes); loopback 17/17, fmt/clippy clean.
Canary redeployed: `mangosd` SHA-256
`e36254745a0605fe2a64811e1c8e8180ed362c57f56d0804726545ba54c3228e`,
listening 2026-11-10 16:51:43 UTC. Owner
re-verification of loot + spirit healer is the
remaining smoke item.

Coldridge gnome probe (2026-11-10): driving the
stock 1.12.1 client in `wow-rig-arm` ourselves, a
fresh gnome mage (`Probegnome`, bench guid
900014) entered Coldridge Valley already dead
(empty health bar, release dialog) and saved
`hp=0` on disconnect. Headless reproduction:
materialize was correct (51/51 from basehp 32 +
stamina 19), but a melee swing killed the
character seconds after login — attacker was the
Coldridge Mountaineer (entry 853, spawn guid
398, level 55, ~7 yd from the spawn point) for
156 damage. Root cause: `can_initiate_attack`
proximity aggro had no faction-reaction check;
the fork gates it on `m_creature->IsHostileTo`
(`BasicAI.cpp:64`, reaction via
`WorldObject::GetReactionTo`, `Object.cpp:3436`).
Fix: `can_initiate_attack` now requires the
existing `creature_hostile_to_player` reaction
bit (contested-guard, reputation-rank, and
template arms, `vendor.rs`); missing templates
stay neutral per fork `GetFactionReactionTo`.
GuardAI's extra arms (attacking a friendly,
hostile-to-players) are recorded UNMODELED in
`ai.rs`. All 20 mangos-map live tests now load
the real `FactionStore` (`load_factions(conn)`)
instead of an empty default, and `live_ai` grew
a friendly-guard phase (stand by the Mountaineer
10 s: no combat, no damage). Verified: loopback
17/17, live_ai/live_map/live_death/live_xp/
live_motion pass, fmt/clippy clean, scripted
gnome login/logout now persists hp=51. This bug
also plausibly explains the canary bot-population
decay (bots materialize and are executed by town
guards). Canary redeploy of this fix is pending
the VM quest pass below.

Coldridge gnome probe, defect 2 (2026-11-10):
in the VM client alive, right-clicking Sten
Stoutarm opened no dialog. Scripted reproduction:
`CMSG_GOSSIP_HELLO` answered `SMSG_GOSSIP_COMPLETE`
— only the script arm of `HandleGossipHelloOpcode`
was modeled; plain gossip/questgiver NPCs (no
script `OnGossipHello`) never produced a menu, so
no quest could be accepted from them in a real
client (quest interaction only worked through
`CMSG_QUESTGIVER_HELLO`, which gossip-flag NPCs
never trigger). Implemented the fork default arm:
new `mangos-world-data::gossip_menus` load
(`gossip_menu`, `gossip_menu_option`), Map
`gossip_default_hello`/`gossip_prepare_menu`/
`gossip_default_select` (`PrepareGossipMenu`,
`GetGossipTextId`, `OnGossipSelect` creature arm)
with the existing `GetNPCIfCanInteractWith`,
condition, vendor/trainer/taxi/talent/bank/stable
predicates and destinations, and actor-side
`SendPreparedGossip` assembly reusing
`PrepareQuestMenu`. Recorded remainders in code:
POI sends, `sGossipScripts` starts, pet-unlearn
confirm packet, movement pause, spirit-guide
query, BG level-bracket approximation, the
pre-existing missing `CMSG_BINDER_ACTIVATE`
handler behind the innkeeper confirm. Regression:
new committed `scripts/default_gossip_wire_probe.py`
(menu text 4937, quests 179+233 at Sten) passes;
script `gossip_wire_probe.py` still passes;
loopback 17/17; clippy/fmt clean for touched
crates (ruff unavailable on this host for the new
script). Note the fork interact distance is 5 yd
minus bounding radii — early VM right-clicks were
also simply out of range. Canary redeploy pending
the rest of the VM quest pass.

Coldridge gnome probe, defect 2 root cause found
(2026-11-10, later session): with the default arm
implemented, the scripted menu parsed byte-perfect
and tcpdump proved it reached the VM, yet the stock
client never fired `GOSSIP_SHOW`. The missing piece
was not the menu: at build 5875 the client resolves
the menu's npc text through its WDB cache and, on a
miss, withholds `GOSSIP_SHOW` while sending
`CMSG_NPC_TEXT_QUERY` (383) and waiting for
`SMSG_NPC_TEXT_UPDATE` (384). The Rust server had
no handler for 383 (opcode registered, never
routed), so every real-client gossip with an
uncached text silently stalled — scripted clients
never perform the cache step, which is why only
the VM saw it. Implemented fork-exactly
(`QueryHandler.cpp:346 HandleNpcTextQueryOpcode`):
new `npc_text` store (`ObjectMgr::LoadNPCText`,
id 0 skipped) in `ScriptStores` next to
`broadcast_texts`, `build_npc_text_update` in
`mangosd::queries` (8 options; probability, male/
female with the fork empty-gender fallback,
language, emote delay/id triples; missing text or
broadcast text -> fork `Greetings $N` default
block; session-locale lookup UNMODELED, enUS store
as elsewhere), routed with the other static cache
queries in the world actor. Regressions: two unit
tests on the builder; `default_gossip_wire_probe.py`
extended to play the cache step (asserts update for
4937 carries broadcast text 7589 at probability
1.0) passes; loopback 17/17; fmt/clippy clean.
VM proof: fresh client process (the previous one
had also wedged its right-button handling after
~1.5 h — wire captures showed clicks produced no
hello at all; relaunch cured it), Probegnome
right-clicked Sten Stoutarm, GossipFrame opened
with the real text, and quests 179 (Dwarven
Outfitters) and 233 (Coldridge Valley Mail
Delivery) were accepted through the details frames
(screenshots gnome18-gossip/-questdetails/
-accepted3). All temporary `GOSSIP_DBG_MODE`
bisection code from the earlier session removed.
Canary redeploy of guard + gossip + npc-text fixes
still pending the wolf/turn-in leg.

Coldridge gnome probe, defect 3 (2026-11-10,
later session): the accepted gnome could not cast
any starter spell in either client. Scripted
reproduction against the bench returned
`SMSG_CAST_RESULT` spell 168 (Frost Armor) reason
145 (`SPELL_FAILED_UNKNOWN`). The client-created
character had zero `character_spell` rows, while
`build_initial_spells` merged race/class defaults
only into `SMSG_INITIAL_SPELLS`; the map cast
gate `player_knows_spell` consulted only persisted
rows in `deferred.spells`, so the client spellbook
looked complete but every creation spell was
uncastable. Fork semantics are the opposite split:
`LearnDefaultSpells` adds defaults as dependent
spells at create and at every load
(`Player.cpp:521`, `:14872`), and `_SaveSpells`
never persists dependent spells. Fix: the runtime
known-spell check now also consults
`PlayerInfos::default_spells(race, class)` without
writing defaults into `deferred.spells` or the DB.
Regression: committed
`scripts/default_spell_cast_wire_probe.py` logs in
the creation-only gnome, casts Frost Armor,
requires `SMSG_SPELL_GO` with no cast result, and
asserts spell 168 is still absent from
`character_spell` after logout; it passes.

Coldridge gnome probe, defect 4 (2026-11-10,
later session): Fireball and every other
projectile spell (`SpellEntry::speed > 0`) were
rejected at prepare with `SPELL_FAILED_UNKNOWN`.
Implemented the fork delayed-hit shape in the
spell slice: `SMSG_SPELL_GO` and hit/miss rolls
stay at launch, each hit schedules a
`DelayedSpellHit` for
`floor(max(5, distance3d) / speed * 1000)` ms,
effects land in `update_casts`, and the last hit
runs the `finish()` tail. Recorded deviations in
`spells.rs`: dealer values are computed at landing
instead of snapshotted at launch, and the player
spell-batching delay for instant spells is not
modeled. Regression: committed
`scripts/projectile_spell_wire_probe.py` places
the gnome ~13 yd from Ragged Young Wolf spawn 344
and requires SPELL_START/SPELL_GO at launch plus
`SMSG_SPELLNONMELEEDAMAGELOG` and AI reaction
after flight; it passes.

Coldridge gnome probe, defect 5 (2026-11-10,
later session): after accepting quest 233 in the
VM, its mail source item (2187) was missing from
inventory after relog. `quest_grant_item` created
the DB rows immediately but appended only to
`player.saved.inventory` (the save baseline),
never to the live `deferred.inventory`, and sent
no `SendNewItem` push. The item was invisible
in-world, and the next save diff treated the
baseline-only row as deleted and removed its DB
rows. Fix: build the row once, push it to both
live and baseline inventories, and send
`SMSG_ITEM_PUSH_RESULT`, matching the
`store_created_item` lockstep pattern. This covers
quest source items and turn-in reward items, the
two `quest_grant_item` callers. The starter-quest
probe below asserts the mail row is present after
accept + logout/save.

Scripted navigation driver + VM model note
(2026-11-11): screenshot piloting of the VM client
proved too lossy for combat/loot (invisible models,
frame-sampled synthetic clicks, one long-lived
client process that wedged right-button input).
Per owner direction, Option 1 is now implemented:
`scripts/scripted_driver.py` issues high-level
commands over the wire protocol — straight-line
`walk_to`/`walk_route`/`approach_guid` movement
heartbeats at run speed, a best-effort
`SMSG_UPDATE_OBJECT` world tracker (creates,
movement/spline blocks, value fields including
health), GCD-aware `cast_until_dead`, and loot
parse/autostore — with an offline `--self-test`.
`starter_quest_wire_probe.py` uses it to walk
between wolf spawns instead of teleporting per
wolf. The driver deliberately treats a loot
response as the authoritative corpse check because
tracked health can lag. One driver bug found live:
server sets GCD at cast execution, so immediate
recasts return reason 60
(`SPELL_FAILED_NOT_READY`); `cast_until_dead` now
waits out that specific result instead of treating
it as a dead/invalid target, after which a
one-wolf diagnostic killed and looted wolf 344
(items 7073x2, 750x1). Separately, the VM's
invisible-model problem is renderer-path, not
asset, related: the client data is stock and
`gx.log` is empty, terrain/UI/particles rendered
under `gxApi "opengl"` while skinned M2 models
never appeared (including character select);
switching the VM copy's `WTF/Config.wtf` to
`gxApi "direct3d"` made the character-select gnome
and scene render immediately. Keep Direct3D for
future VM visual checks; use the scripted driver
for logic loops.

Coldridge gnome probe, defect 6 + starter loop
complete (2026-11-11): with the driver killing and
looting wolves, Tough Wolf Meat reached the
inventory but `character_queststatus.item_count1`
stayed 0 and Sten offered no reward. The fork's
live increment (`Player::ItemAddedQuestCheck`,
called from `StoreNewItem`) had no Rust equivalent:
deliver-quest counts were only recounted at load,
at accept, and inside `can_complete_quest`.
Implemented `Player::item_added_quest_check`
(capped per-objective increment in log-slot order,
source-item toast suppression, completion
transition) with the notification tail on Map
(`SMSG_QUESTUPDATE_ADD_ITEM` in batches of 63 +
`SMSG_QUESTUPDATE_COMPLETE`), wired into the
shared new-item paths: `Map::store_new_item`
(loot/vendor), actor `quest_grant_item`, and
actor `store_created_item`. Regression: new
entities test
`item_added_quest_check_increments_and_completes_deliver_quest`.
The full scripted starter loop now passes end to
end (`LIVE-STARTER-QUESTS-PASS`): accept 179+233
at Sten through default gossip + npc-text, mail
item persisted, 233 turned in at Talin (+190 XP),
driver walks wolf routes casting Frost Armor/
Fireball and loots 8 meat with the persisted
counter reaching 8 live, 179 turned in at Sten
(row `1:1:8`). Driver lessons recorded in code:
reason 60/97/54 are pacing/facing retries, reason
77 waits out mana regen, and a loot response (not
tracked health) is the authoritative corpse check.
Known driver limitation: a few wolves died out of
loot range of their spawn fallback point and were
skipped (tracked position is absent for objects
created in the login burst, which `login_client`
drains before the driver attaches); enough
reachable wolves remain for the objective. Gates
after all fixes: loopback 17/17, live_ai/live_map/
live_death/live_xp/live_motion/live_spells pass,
fmt/clippy clean for touched crates,
default-spell and projectile wire probes pass.
Canary redeploy (guard, gossip, npc-text, default
spells, projectile, quest-grant, item-count fixes)
is the next step and needs owner approval; the
owner should also rotate the droplet DB password
exposed in an earlier session transcript.

Saved-ghost login defect (2026-11-12, owner
report on the droplet): logging into Ruikanis
showed neutral-mob aggro, an empty spell bar
while apparently alive, spells back after dying,
and no release/reclaim at the corpse — read as
alive/dead modes switched. Droplet ground truth
(read-only): the live artifact is still the
2026-11-10 `e3625474` build (the fixes above were
never redeployed, incl. the proximity-aggro
faction gate, which is the neutral-aggro arm);
Ruikanis is saved as a ghost (`character_flags`
0x2000, hp 1, sole saved aura 8326, resurrectable
corpse row present) from the 2026-11-11 smoke.
Two real-client death-loop gaps in the local tree
kept that ghost unplayable and unreclaimable:
(1) `MSG_CORPSE_QUERY` (534) had no handler
(recorded unmodeled in `death.rs`), so the stock
client never learned the corpse position and
never offered reclaim at the body — implemented
fork-exactly (`HandleCorpseQueryOpcode`,
`QueryHandler.cpp:298`: found/not-found shapes,
dungeon ghost-entrance display arm; entrance
height only when that map is already loaded).
(2) `refresh_ghost_flag` set only
`PLAYER_FLAGS_GHOST`; the fork's
`HandleAuraGhost` also carries
`UNIT_VIS_FLAGS_GHOST` in the
`UNIT_FIELD_BYTES_1` vis byte (offset 3) on any
unit — added. Plus fork `KillPlayer` field
writes (`UNIT_DYNAMIC_FLAGS` clear +
release-timer byte on non-instanceable maps).
Regression: new loopback
`ghost_login_corpse_query_and_reclaim_round_trip`
(saved-ghost fixture: flag + vis byte at login,
corpse-query answer for the persisted corpse,
reclaim resurrects and clears both bits; failed
pre-fix on the vis byte), `queries` shape unit
test, and `live_death` `is_ghost` now requires
both bits. Gates: loopback 18/18, mangosd lib
109, `LIVE-DEATH-PASS`, clippy `-D warnings` and
fmt clean. These join the pending redeploy set;
Ruikanis should reclaim or spirit-resurrect
normally once it lands. No droplet write was
performed.

Post-canary re-verification (2026-11-12, fresh
goal session, Step 1 of the redeploy run): full
`./ci.sh` with the `.env.test` environment is
green end to end (144 suite results, exit 0)
after three drift fixes, each proven against an
independent oracle before any expectation moved:
(1) DBC parity goldens regenerated with the
Python oracle for the two Tier 4 stores
(`SkillLineAbility`, `TaxiNodes`; 46 -> 48, diff
exactly the two `StoreGolden` rows). (2)
`live_spawns` grid totals re-derived by SQL for
the B0-8 semantics (event spawns grid-register,
materialization gated in-map): creatures 65_622
and gameobjects 24_044, both exact matches.
(3) `live_creature_spawn` corpse delay 60 -> 300
(the old pin repeated a wrong conf.example
claim; fork `Corpse.Decay.NORMAL` default is
300 and the binary reads the droplet keys).
Extra live suites beyond `ci.sh` all green:
the twelve Tier 3 map suites (instance batches
1-6, AB/AV/WSG, creature/GO scripts,
Shadowfang). `live_instance_batch6` Dire Maul
exposed two live-inventory bugs in the Tier 4
step 4a saved/deferred split, both fixed: the
fixture grants in lockstep now, and
`destroy_player_item_count` mutated only the
save baseline (the Gordok key survived in the
live inventory and the next save diff would have
resurrected it) — it now moves live + baseline
together like the actor grant paths; the UBRS
seal check and durability-repair enumeration
also read the live rows. A fresh side bench
(current tree, port 18100, bots disabled; the
long-lived 18099 bench untouched) passed every
wire probe: `TWO-CLIENT-SMOKE-PASS` (fixture
characters re-co-located on map 1 first),
`LIVE-DEFAULT-GOSSIP-WIRE-PASS`,
`LIVE-DEFAULT-SPELL-CAST-PASS`,
`LIVE-PROJECTILE-SPELL-PASS`,
`LIVE-STARTER-QUESTS-PASS`. New maintained
probe `scripts/death_wire_probe.py` adds the
stock-client-shaped death pass and found one
more fork divergence: `MSG_MOVE_TELEPORT_ACK`
always took the anticheat wrong-ack arm
("no sender") although near teleports are sent;
it now consumes the pending teleport change
(`FindPendingMovementTeleportChange`, new unit
test) with no anticheat strike on mismatch, per
`HandleMoveTeleportAckOpcode`. The same probe
caught `reclaim_corpse` honoring the packet
guid; the fork ignores it and resolves
`Player::GetCorpse` — fixed, loopback now sends
a bogus guid to lock that in. Clean rerun from
a fresh side boot (0 persisted corpses):
`LIVE-DEATH-WIRE-PASS` (die -> release ->
corpse query -> reclaim at 50%, spirit-healer
resurrect, saved-ghost logout/relogin ->
reclaim), zero `ByteBufferException` lines.
Step 1 closed; next is the amd64 artifact
rebuild and the owner approval stop.

Step 2 blocker (2026-11-12, same session): the
exact-container build cannot start — OrbStack
was shut down at the prior session's end and
its VM wedges on headless start from this
session (`orb start` twice, helper restart:
vmgr reports the guest kernel stacks sampled as
hung and the docker forward at 0.250.250.2:2375
times out; `docker` never answers; Data volume
at 95% may be involved). The owner was asked to
open the OrbStack app (no reply at record time).
Exact recipe recovered from the prior session
log for the rerun: `docker run --rm --platform
linux/amd64 -v "$PWD:/repo" -w /repo/vmangos-rs
-e CARGO_TARGET_DIR=/repo/vmangos-rs/target-
ubuntu2204-amd64 ubuntu:22.04` with rustup
1.98.1, `cargo build --release -p mangosd
-p mangos-auth`, then file/ldd/GLIBC-max +
sha256, and the side-port 18101 bench boot.
No droplet state changed.

Step 2 unblocked via headless Colima (2026-11-21):
the owner has no GUI access to the build Mac.
The missing `/usr/local/bin` was created and the
OrbStack CLI/socket links were pre-created over
SSH (`orb`, `orbctl`, `docker`, `docker-compose`,
`docker-credential-osxkeychain`, `kubectl`, plus
`/var/run/docker.sock`), after which OrbStack's
log reaches `CLI setup complete` — but its guest
`scon`/`vcontrol` services still never answer
(`0.250.250.2:2375`/`:8000` and
`disk/report_stats` timeouts; 2.2.2 also reports
an update available that only opens its GUI
updater). With the owner's go-ahead for a headless
alternative, standalone Colima 0.10.3 + Lima 2.3.0
(checksum-verified release downloads, isolated
`/tmp` home) came up on Virtualization.Framework
with Rosetta (`linux/amd64` supported, Docker
29.5.2). The exact Slice 3 `ubuntu:22.04`
`linux/amd64` recipe then built `mangosd`
`a4b87d0fce619cc40907fd2324c3b7345f19dd569
d86e1ea8cf9893d1069719e` and `realmd`
`810723a589b1d1191c41a0053462fed64ab32e4b010
c399d2a84164099680f85` (host `shasum` agrees;
both ELF x86-64 PIE, `ldd` system libraries
only, max `GLIBC_2.34`). The pending amd64
`mangosd` booted in-container against the cloned
bench DB/DataDir on side port 18101 to
`World server listening` (94 game events, 4144
creature loot templates, 222 bot identities,
0 persisted corpses) and took the test timeout's
SIGTERM. Runbook hashes/status updated to the
pending set; the droplet still runs `e3625474…`.
Step 2 closed; next is the Step 3 owner approval
stop. No droplet write, restart, commit, or push
was performed.

Step 4 redeploy window (2026-11-21, owner go):
fresh backup `*_20261121_154428.sql.gz` ran and
passed `gzip -t` for all four databases before
any stop. Artifacts were staged on the droplet
and re-hashed in place (`a4b87d0f…` / `810723a5…`,
native `ldd` clean, `--version` runs). Units
stopped in runbook order (`mangosd`, then
`realmd`); the live Rust pair was preserved as
`mangosd.e362-bak` / `realmd.7258-bak` (hashes
match the pre-window live set; the C++ `.cpp-bak`
pair untouched), the pending pair installed by
atomic rename and re-hashed at the exec paths.
`realmd` then `mangosd` started: both units
active, 3724/8085 listening, `World server
listening on 0.0.0.0:8085` at 15:47:35 UTC after
the full world load (66222 creature spawns on
the droplet DB, 94 game events, 4144 loot
templates, 2 persisted corpses, no save errors
at first check). One boot note for the smoke:
honor maintenance is marked due and will run at
the next restart (`AutoHonorRestart` is on;
scheduling is not modeled). Owner real-client
smoke on Ruikanis and maintenance-off are the
remaining window items; rollback is the
preserved `e362`/`7258` pair (binary-first, DB
restore only on observed save corruption).

Smoke defect + hotfix (2026-11-21): the first
owner smoke found Ruikanis could not talk to the
spirit healer, and his Nov 11 corpse was gone.
Three fork divergences composed the strand:
(1) `npc_can_interact` carried an invented early
arm rejecting every dead-player interaction whose
requested flag mask lacked the spirit-service
bits — `HandleGossipHelloOpcode` passes mask 0,
so the healer menu never opened (the fork gate
is the later static-flag/`IsVisibleForDead`
check, which stays). (2) The Spirit Heal dummy
effect (17251) had no arm, so even a menu would
never produce `SMSG_SPIRIT_HEALER_CONFIRM`;
`m_originalCasterGUID` is now plumbed
(`next_original_caster` → `ActiveCast` →
effects/delayed hits) and the dummy confirms to
the affective player like `SpellEffects.cpp:805`.
(3) `Player::LoadCorpse` was unimplemented and
seeded bones kept the dead row's `ghost_time`,
so an expired corpse vanished on its first decay
tick and a corpseless ghost stayed stranded;
seeded bones now start a fresh hour, and login
without a resurrectable corpse resurrects at 50%
(loaded-corpse arm also refreshes the release
timer byte and converts a live login's leftover
corpse). New loopback regressions
(`ghost_spirit_healer_gossip_round_trip`,
`ghost_login_without_resurrectable_corpse_resurrects`)
failed pre-fix (gossip timeout; health stayed 1)
and pass after; full `./ci.sh` green, targeted
clippy `-D warnings` clean. Rebuilt exact-recipe:
`mangosd` `6a450854…`, `realmd` unchanged
`810723a5…` (the untracked dashboard
`20261121175904_logs.sql` was held out of the
build glob and restored, so no production schema
change rode along; it is applied only to the
bench clone). Boot check on 18101 passed, fresh
droplet backup `*_20261121_183524.sql.gz`
verified, the first-redeploy pair preserved as
`mangosd.a4b8-bak` / `realmd.8107-bak`, hotfix
installed and listening on 3724/8085 with no
save errors at first check. Owner retest and
maintenance-off remain.

Rage-generation smoke defect + staged fix
(2026-11-21): Ruikanis gained no rage in combat
because Rust's shared damage path never called
the fork's `Player::RewardRage`. The conversion
formula, `Rate.Rage.Income`, Berserker Rage's
1.3x taken-damage term, direct-damage dealt arm,
survived-damage-taken arm, and the melee
dodge/parry 75% clean-damage arm are now wired;
the maintained `live_ai` regression failed
pre-fix at 0 rage after landed damage and now
observes rage from damage taken and during
player melee, alongside a formula unit test and
the three saved-ghost loopback tests. Targeted
fmt, package tests, and Clippy passed. Rebuilt
with the exact container holdout above:
pending `mangosd` `6386c1a8…`, `realmd`
unchanged `810723a5…`; host hashes match, both
max `GLIBC_2.34`, and the pending world server
booted to listening on side port 18101. This
pair is staged locally only and stops at a
fresh owner approval gate; the droplet still
runs `6a450854…`, with no droplet install,
restart, schema change, commit, or push yet.

Rage hotfix installed (2026-11-21, owner go):
fresh backup `*_20261121_194226.sql.gz` passed
`gzip -t` for all four databases; the staged pair
re-hashed on the droplet (`6386c1a8…` /
`810723a5…`, native `ldd` clean). The live
saved-ghost pair was preserved as
`mangosd.6a45-bak` (the identical `realmd` keeps
its `realmd.8107-bak`), the rage pair installed
by atomic rename and re-hashed at the exec paths,
and both units returned active with
`World server listening on 0.0.0.0:8085` at
19:47:10 UTC. No schema change was applied.
Owner rage retest and maintenance-off remain.

Spell-campaign pilot hardening + stance gate
(2026-11-21): the level-60 scripted-client pilot
was rebuilt so verdicts bind to parsed
SPELL_GO/CAST_RESULT bodies, target GUIDs,
event-window damage logs, complete spell-chain
rank metadata, and a two-counter dummy reset.
The first honest rerun failed only Growl 6795:
it landed in caster form because `SpellEntry`
never loaded `Stances`/`StancesNot` and
`Map::cast_spell` had no shapeshift gate.
`GetErrorAtShapeshiftedCast` is now ported
(fields 12/13 loaded, talent-learn exception,
stance-form flag from SpellShapeshiftForm.dbc);
direct Bear -> Cat correctly fails
`SPELL_FAILED_NOT_SHAPESHIFT` (61), so the
pilot/live scenario uses the stock-shaped
cancel-Bear-then-Cat sequence. `live_spells`
failed pre-fix at Moonfire-in-Bear (`Started`
instead of `Failed(61)`) and now passes with
Bear Growl started, Cat/caster bear-gated casts
`Failed(86)`, plus targeted fmt/clippy/package
tests. The rebuilt bench then returned
`CAST_RESULT(86)` for Growl over the wire and
`SPELL-CAMPAIGN-PILOT-PASS` (8 PASS, the
expected Growl FAIL, Rebirth still honest
BLOCKED `wire-target-unsupported`). G1 is now replayed in the generator
(`--g1`, exact train-mode fit semantics; all
nine union counts reproduced with 0
exclusions), and G2 is enumerated by
`spell_matrix_gen.py --talent-builds`: legal
selected builds exist for
Paladin/Hunter/Shaman/Warlock/Druid, while
Warrior/Rogue/Priest/Mage checked-in builds
fail fork prerequisite/tier legality, so full
six-class fan-out remains blocked and no
droplet write or restart was performed for
this local spell work.

Stance-gate Step 1/Step 2 re-verification
(2026-11-21): full `./ci.sh` with the
`.env.test` environment and the dashboard logs
migration held out/restored passed after one
test-fixture fix (`live_ai` now enters Battle
Stance before Rend; threat 6 restored).
`live_death` passed separately. Scripted wire
probes against the rebuilt native bench all
passed: `TWO-CLIENT-SMOKE-PASS`,
`LIVE-DEFAULT-GOSSIP-WIRE-PASS` (the probe now
snapshots/resets/restores Probegnome starter
quest rows), `LIVE-DEFAULT-SPELL-CAST-PASS`,
`LIVE-PROJECTILE-SPELL-PASS`,
`LIVE-STARTER-QUESTS-PASS`, and
`LIVE-DEATH-WIRE-PASS` (die 46s, corpse in
60s, reclaim, ghost relog, spirit resurrect,
alive relog). Fresh Ubuntu 22.04 linux/amd64
artifacts were then built in an isolated 6 GiB
Colima profile after the default 2 GiB profile
OOM-killed release `rustc`: pending
`mangosd a8c380801c4bf1409f6d91f20111ea7bf5
a3ac51b4f0a757c0049afe0c4b86d4` and
`realmd eb92ba54e2fdef560130ef6658e9a15290
7705213a0206128852f18b0bc637d5` (join the
wrapped hashes), both x86-64 ELF PIE, system
`ldd` only, max `GLIBC_2.34`. The amd64
`mangosd` booted on side port 18101 to
listening (94 events, 4144 loot templates,
222 bots, 0 corpses). Runbook hashes/status
updated. This stops at the Step 3 owner gate:
no backup, staging, droplet write, restart,
commit, or push for the stance-gate pair.

Stance-gate redeploy window (2026-11-22, owner
`proceed`): read-only preflight matched the
live rage pair (`6386c1a8…` / `810723a5…`),
3724/8085 listening, Ruikanis alive/offline at
health 483 with `character_flags=0`, saved
Battle Stance aura 2457, and no corpse row.
Fresh backup `*_20261122_022122.sql.gz`
passed `gzip -t` for all four databases; the
live pair was preserved as
`mangosd.6386-bak` / existing
`realmd.8107-bak`. The stance-gate pair was
staged and re-hashed on the droplet
(`a8c38080…` / `eb92ba54…`, native `file`/
`ldd`/`--version` clean), units were stopped
in runbook order, binaries swapped by atomic
rename only (no migrations or config changes),
and `realmd` then `mangosd` returned active:
`World server listening on 0.0.0.0:8085` at
02:22:33 UTC after the full droplet load
(66222 creature spawns, 94 game events, 4144
loot templates, 222 bots, 2 persisted corpses),
3724/8085 reachable from this workstation, and
no severe new save/session errors at first
check. Owner Ruikanis real-client smoke
(movement, neutral non-aggro, kill/loot,
gossip/quest, death reclaim + spirit healer,
logout/relogin, rage spend/gain) and any
maintenance closeout remain; no commit or
push was made.

Owner smoke pass (2026-11-22): owner reported
`smoke looks good` on Ruikanis. Post-smoke
read-only verification matched live hashes
`a8c38080…` / `eb92ba54…`, both game units
active on 3724/8085, realm `realmflags=0` and
`flag=0` (maintenance already off; no flag
write needed), Ruikanis online with health
483, `character_flags=0`, saved Battle Stance
aura row, and no corpse row; the new
`Server.log` had no severe save/session
errors. Tier 4 canary closeout now awaits the
separate owner commit/cutover decision; no
commit or push was made.

Paladin spell campaign full run (2026-11-22, bench
127.0.0.1:18099): 84 rows — PASS 45,
PASSIVE-VERIFIED 11, BLOCKED 28, FAIL 0
(`SPELL-CAMPAIGN-RUN-CLASS-PALADIN-PASS`).
Failure clusters from the first full run, all
fixed test-first in shared-root-cause order:
(1) unsettlable 178-spell seed (talent cost 55
> 51; fork `InitTalentForLevel` wiped 18 spells
on login → CAST_RESULT 145 on 20066/20218):
generator now excludes the 4 merged-chain
talent ranks no legal book holds
(20473/20911/20925/26573), seed 174 cost 51,
cast set byte-identical; (2) full-ally heals
(9 rows): ally re-damaged per heal leg;
(3) dummy pool exhaustion (12 rows): reuse of
observably-clean dummies; (4) Blessing of
Freedom 1044 NOT_READY (20 s RecoveryTime):
cooldown wait + retry; (5) passive talents
(11 rows): learned-only PASSIVE-VERIFIED;
(6) Lay on Hands 10310: engine had no Effect 67
arm → implemented fork-exact
`EffectHealMaxHealth` (heal_max_health_value +
live `bench_spells_and_auras` section, failed
first); (7) Consecration 20924: periodic ticks
ride 590 not 592 → harness parses both;
(8) Holy Shock 25913/25914 OUT_OF_RANGE: heal
leg approaches the ally; (9) five 1-HP heals
(19968/19980-19982/19993): generator marks
sub-observable heals marker-only. Honest
BLOCKED remainder: 9 script-evidence-unknown,
8 effect-marker-only, 7 fixture-reagent (engine
has no reagent gate; no harness grant path),
3 no-party, 1 fixture-gear. Reagent gate is the
next engine seam. No commit or push was made.

Druid spell campaign full run (2026-11-22/23, bench
127.0.0.1:18099, fan-out matrix 63 last-rank rows
over merged S7 chains: seed G1 union 187 + balance
talent 15, cost exactly 51, excluded [5570, 16857]):
first full run 14 PASS / 15 PASSIVE-VERIFIED /
21 BLOCKED / 13 FAIL, final reruns 27 PASS /
15 PASSIVE-VERIFIED / 21 BLOCKED / 0 FAIL
(`run-class Druid: 63 rows`, zero failure
clusters; the class gate still lists the 13
engine-totem + 1 wire-target-unsupported BLOCKED
rows, which are honest no-scenario BLOCKED, not
FAILs). Failure clusters from the first full run,
all fixed test-first in shared-root-cause order:
(1) mana starvation (NO_POWER 77 on Dire Bear
9634, Hurricane 17402, MotW 9885 and Abolish
Poison 2893 ally legs): the 999999 seed clamps to
max ~2464 on the first fork-exact regen tick and
percent-cost shapeshifts plus Tranquility drain
the pool, so the harness now paces every cast leg
(POWER_RETRIES 12 shared by the self/unit/hostile/
ground/blessing/heal retries; blessing legs ride
the power retry too), never an engine change;
(2) Tranquility channel blocked the next cast
(97 on 9885): ENGINE FIX fork-exactly —
`HandleCancelAuraOpcode`'s channeled arm
(`SpellHandler.cpp`) interrupts the channel, so
`cancel_aura` (`spells/aura.rs`) now interrupts
the matching current cast; live `bench_spells_and_
auras` CANCEL-CHANNEL section (Tranquility cast,
cancel, channel-field 0, recast Started) observed
FAILING first (channel field stuck 9863) and
passing after; live wire probe confirmed the
recast GO on the rebuilt bench;
(3) Teleport: Moonglade 18960 moved the caster to
map 1 mid-run and wedged the session (transfer
packets observed, worldport never ACKed), failing
that row plus the next 8 with timeouts: generator
now derives a `teleport` kind for teleport-only
ground rows (Paladin/pilot artifacts byte-stable)
and the harness runs a teleport leg (bound GO +
TRANSFER_PENDING/NEW_WORLD verify, then reseed +
fresh login back on the bench map);
(4) dead-dummy TARGETS_DEAD 101 (Insect Swarm
24977 picked a Hurricane-killed counter on stale
WorldState): hostile leg re-picks a live counter
within the attempt cap (shared);
(5) damaging-DoT aura race (24977 killed the
100-hp dummy before the aura check): the shared
verdict now accepts death with attributed
damage/periodic logs for rows with no direct-
damage effect (Paladin rows keep the strict
aura-only gate);
(6) Innervate 29166 360 s cooldown (NOT_READY 60
on the second leg): blessing legs run ally-first
and the shared second-leg retry waits out the
full DBC cooldown plus margin (390 s, expiry
proven like Freedom's 20 s);
(7) Hurricane's holder-less 8 s channel blocked
the teleport row (97): ground casts register for
the next row's cancel and the cancel path sends
even with no visible holder, firing the new
channel interrupt (proves the engine arm live).
Honest BLOCKED remainder: 13 engine-totem
(stance/form scenarios unbuilt), 1 wire-target-
unsupported (Rebirth 20748: the three-layer
RESURRECT gap stands — effect routes but has no
resurrect body, the cast alive-gate rejects dead
targets, and the wire/parser drops corpse GUIDs;
left BLOCKED per the attempt-only-if-complete
rule), 1 fixture-reagent (Gift of the Wild 21850;
engine has no reagent gate, no harness grant
path), 6 effect-marker-only (2 dispels, 4 form
threat markers). Reagent rows and Rebirth behave
exactly as the Paladin synthesis predicted.
No regression: pilot SPELL-CAMPAIGN-PILOT-PASS
before and after (also after the engine
rebuild), Paladin full rerun on the final tree
84 rows 45/11/28/0 with gate PASS, generator and
campaign self-tests PASS, Paladin/pilot matrix
artifacts byte-identical, cargo fmt clean, clippy
-D warnings clean on mangos-map, 96 unit tests
pass, live bench_spells_and_auras passes with
CHARACTER_LOAD_GUID=1. One engine file changed
(spells/aura.rs channeled cancel arm + live
regression), else scripts + evidence only; bench
mangosd rebuilt from the current tree and the
proven bench-lan.conf pair restarted once (a
prior outage had taken the old bench down; the
side-bench pair was never touched). The 302
ByteBufferException seen twice in Server.log
comes from malformed CMSG_CAST_SPELL traffic
during cancel/cast races and is non-fatal
(runs complete). No commit or push was made.

Central completion of the Druid slice (the
workflow runner went silent for 62 minutes after
writing the entry above and was stopped; the
parent finished the fix loop directly):
(1) the 13 engine-totem rows now cast through a
new harness `form_gated` leg (enter the required
shapeshift form per the row's Stances bits, cast
through the natural self/hostile leg, exit the
form; a stuck form downgrades PASS to FAIL);
actual totem rows (totem fields set) stay
engine-totem BLOCKED for Shaman later;
(2) generator aura-side fix: stance-gated rows
derived aura_on_caster unconditionally, so
Growl/Bash/Rip-class debuffs would have been
checked on the wrong unit — side now follows the
row's targets (Paladin artifact unchanged);
(3) short auras (Growl's 3 s taunt) are latched
by sliced polling in the hostile leg instead of
one long poll that outlived them; bear rage rows
get a Growl-pull + Enrage setup that waits on the
observed rage field; areaselect-at-caster rows
stand at melee standoff (Demoralizing Roar at
13 yd landed on nobody);
(4) engine divergence found by the feral rows
and fixed test-first: finishers (Rip 9896,
Ferocious Bite 31018, Mangle 22570) landed at
0 combo points while the fork rejects them in
CheckCast (`Spell.cpp:7108`). Rust now returns
`SPELL_FAILED_NO_COMBO_POINTS` (70; warriors read
BAD_TARGETS per the fork) when a player finisher's
explicit unit target is not the combo target;
regression `finisher_without_combo_target_
rejected` observed failing first (Started) and
passing after. The generator emits
`needs_combo_points` per row plus the class's
lowest-rank combo builder (Druid Claw 1082);
the harness builds one point on a sturdy Ice
Claw Bear dummy (a rank-1 Claw kills the 100 hp
wolves) and walks back to the arena afterwards;
(5) bench-only speedup: campaign dummy entries
(705/707/724/1196) now respawn in 2-3 s in the
bench world DB (spawn times load at boot; stale
creature_respawn rows cleared).
Final Druid run on the rebuilt bench: 63 rows,
PASS=40, PASSIVE-VERIFIED=15, BLOCKED=8
(6 effect-marker-only, 1 wire-target-unsupported
Rebirth, 1 fixture-reagent), FAIL=0,
SPELL-CAMPAIGN-RUN-CLASS-DRUID-PASS; pilot
SPELL-CAMPAIGN-PILOT-PASS on the same binary;
independent artifact audit 1:1 with zero vacuous
PASS rows. Owner rule change recorded the same
day: production compatibility no longer gates
implementation work, and commit/push/deploy are
pre-authorized (credential hygiene stands).

Post-parity deploy (2026-11-23, pre-authorized):
the full parity tree (582822c2 engine/systems,
20609416 campaign, 243b8f18 talent legalization)
was built in the ubuntu:22.04 amd64 recipe
(mangosd cf583736…, realmd 1582565b…; container
side-port boot to listening first), the droplet
was backed up (`*_20261123_070357.sql.gz`, gzip
verified), the dashboard logs migration
`20261121175904` was applied (logs gate 6 -> 7;
world/characters/logon already at census), and
the pair was swapped binary-only with the
stance-gate pair preserved as `mangosd.a8c3-bak`
/ `realmd.eb92-bak`. Both units active, 3724/8085
listening, fresh boot log clean (2 persisted
corpses). Side finding: `azeroth-discord-bot`
is in a long-standing crash loop
(`LoginFailure: Improper token`), unrelated to
the swap; needs an owner-issued token.

Central completion of the Warlock slice (the
workflow runner stalled again after the first
full run and was stopped; parent finished
centrally): first run 113 rows 45 PASS /
14 PASSIVE-VERIFIED / 34 BLOCKED / 20 FAIL in
6 clusters. Root causes: a persisted imp from
the pet rows assisted on every hostile cast and
killed 100 hp dummies mid-observation (fixed:
seed_character clears character_pet + satellites,
pets are parked REACT_PASSIVE on summon, dummy
pool expanded to 48 counters); landed casts were
erased by cooldown-blocked recasts (Curse of
Doom; landed GO now retained); channeled rows
poisoned the next cast (engine had no
CMSG_CANCEL_CHANNELLING handler — implemented
fork-exact with regression
interrupt_channeling_ends_live_channel_only);
Sacrifice needed per-effect target routing
(TARGET_UNIT_CASTER effects now land on the
caster inside mixed spells; regression
unit_caster_effect_lands_on_caster_not_victim)
plus a TARGET_UNIT_CASTER_MASTER collect arm and
a pet-cast harness leg (voidwalker summon +
grimoire teach 7885/19447 seeded as setup spells
+ CMSG_PET_ACTION cast; shield on master and pet
suicide both observed). Tick-band gate learned
the lethal-pair rule (one in-band tick carries
the band proof when two ticks would kill the
dummy; periodicity proven by rank 1). Final run:
113 rows (112 matrix + summon sub-row),
PASS=62, PASSIVE-VERIFIED=14, BLOCKED=37
(17 effect-marker-only, 12 fixture-reagent,
4 fixture-gear, 2 no-party, 2 script), FAIL=0,
SPELL-CAMPAIGN-RUN-CLASS-WARLOCK-PASS.
Pilot PASS; Druid/Paladin regression smokes PASS
(17329's shared-cooldown NOT_READY in subset
smoke confirmed a subset artifact by solo PASS);
fmt/clippy/workspace tests/live_spells green.
Rank coverage note: cast set now first+last+
signature-change ranks (Paladin/Druid matrices
regenerated to the same rule for future reruns;
their recorded results stand for the rows cast).
Warlock deploy (same day): mangosd 6d28d530
built in the ubuntu:22.04 amd64 recipe,
side-port boot-checked, droplet backed up
(*_20261123_201521), swapped binary-only with
the previous pair preserved as mangosd.cf58-bak
(realmd 1582565b unchanged, not restarted);
8085 listening on the fresh boot.

Hunter spell campaign full run (2026-11-23/24, bench
127.0.0.1:18099, fan-out matrix 80 last-rank rows
over merged S7 chains: seed G1 union 123 + mm
talent 16, cost exactly 51, excluded [19306, 19386];
seed_all 142 with the Call Pet fixture setup):
first full run 52 PASS / 13 PASSIVE-VERIFIED /
14 BLOCKED / 1 FAIL, final rerun 53 PASS /
13 PASSIVE-VERIFIED / 14 BLOCKED / 0 FAIL
(`run-class Hunter: 80 rows`, zero failure
clusters; honest BLOCKED remainder 13
effect-marker-only + 1 target-not-dead
(Revive Pet), zero FAILs). Failure clusters
from the first full run, all fixed test-first in
shared-root-cause order:
(1) pet scenario (3 rows BLOCKED no-pet): the
engine auto-loads the slot-0 saved pet at world
entry (fork `Player::LoadPet`), so the seeded
Timber Wolf 69 (tameable: type 1, family 1,
flags 16; pet_type 1, slot 0, created_by_spell 0)
was already out and the Call Pet 883 cast was a
silent no-op GO with nothing new to track. The
harness now reuses the already-out pet
(`find_active_pet`: fixture entry, campaign
health > 1000 so wild wolves and 100-hp dummies
never qualify, within 20 yd), parked REACT_PASSIVE
like a summon; fixture summon still casts when
nothing qualifies (regression asserts cover the
reuse and the wild/dummy/far/position-less
rejections; live rerun 136/13544/1002 PASS);
(2) shared category cooldown (24133 Wyvern Sting
r3 read NOT_READY 60 after 24132 armed category
1111/120 s; the matrix carries only recoveryTime):
the hostile leg now waits out the full fork
cooldown resolved live from the bench oracle
(`live_cooldown_wait_s`: max(recoveryTime,
categoryRecoveryTime) + 5 s margin, 600 s cap
matching `not_ready_retry`) once per row within
the 5-attempt cap (a landed GO never waits: the
Curse-of-Doom retained-GO arm keeps priority);
24133 PASS after 2 attempts with cooldown wait
+125 s. No engine change: the fork rejects
same-category ranks mid-cooldown by design.
Honest BLOCKED remainder: 7 bare trap placements
(Freezing/Immolation/Frost/Explosive ranks: the
trap object is wire-invisible and the trigger
damage attributes to the trap-effect spell,
whose own hostile/ground rows verify it),
5 threat/dispel markers (Disengage x2,
Distracting Shot x2, Tranquilizing Shot),
1 dispel marker (Improved Mend Pet 24406),
1 target-not-dead (Revive Pet 982: the RESURRECT
3-layer gap stands). Pet/ammo scenario outcomes:
Call Pet fixture summon + REACT_PASSIVE parking
PASS (Mend Pet r1/r7 and Eyes of the Beast all
PASS on the reused pet); ranged fixture (Ashwood
Bow 5596 in ranged slot 17 + Rough Arrows +
mainhand sword) gates all 23 weapon rows; no
min-range gate in Rust (fork dead-zone 8-35 yd
for range-114 rows) — Hunter hostile standoff
rides 13 yd; Auto Shot 75 PASS with 592 damage
(no autorepeat modeling needed for the verdict).
No regression: pilot SPELL-CAMPAIGN-PILOT-PASS
before and after; Paladin (10310+20924), Druid
(6795+9896), Warlock (688+7812+980) smokes PASS
on the final tree; generator + campaign
self-tests PASS; P/D/W artifacts byte-identical
(matrices untouched — cooldown data stays a live
harness read); cargo fmt clean, ruff clean,
live_spells (incl. bench_spells_and_auras) PASS
with CHARACTER_LOAD_GUID=1; independent artifact
audit 1:1 with zero vacuous PASS rows. One
harness file changed (spell_campaign.py:
find_active_pet + ensure_pet reuse, cooldown_wait_s
+ live_cooldown_wait_s + hostile-leg NOT_READY arm,
self-test asserts), plus the Hunter matrix artifact
and evidence; bench mangosd already ran the
current tree (debug binary newer than every .rs;
listener 18099 verified, no restart), the
side-bench pair never touched. No commit or push
was made.

Shaman spell campaign full run (2026-11-23/24, bench
127.0.0.1:18099, fan-out matrix 93 cast rows over
merged S7 chains: seed G1 union 159 + enhance talent
15 known, cost exactly 51, excluded [16190];
seed_all 174): first full run 23 PASS /
15 PASSIVE-VERIFIED / 49 BLOCKED / 6 FAIL, final
rerun 27 PASS / 15 PASSIVE-VERIFIED / 51 BLOCKED /
0 FAIL (`SPELL-CAMPAIGN-RUN-CLASS-SHAMAN-PASS`).
Failure clusters from the first full run, all fixed
test-first in shared-root-cause order:
(1) target-45 unit heals (Healing Wave 331/25357,
Chain Heal 1064/10623): the engine `check_target`
had no mode-45 arm, so every cast died
`FAILED_UNKNOWN` (145); implemented the fork-exact
arm (`SpellEntry.h` IsExplicitPositiveTarget treats
45 as an explicit friendly unit target, hostile →
BAD_TARGETS) plus the execution-time unit-target
fill (fork `Spell.cpp:2949` pushes `pUnitTarget`;
chain jumps stay explicitly unverified). Live Rust
regression `bench_chain_heal_target_mode` failed
145 first, now asserts Started on self-cast and
Failed(9) on a dest target.
(2) The same rows then healed the caster, not the
ally (the fill fallback `vec![caster]`): GO with no
ally rise; the fill arm in (1) covers it.
(3) Healing Wave r1 (331, oracle band ≈35-45) sits
inside the +34 regen-tick quantum (the pilot's
pinned-5187-rather-than-5185 precedent for exactly
this): the engine now emits the fork's
SMSG_SPELLHEALLOG from the direct-heal effect
(`DealHeal` → `SendHealSpellLog`: victim, caster,
spell, amount, crit 0) whenever the caster or the
victim is a player; the heal leg binds the log
against a live-oracle fork band (331 band [35,48];
logs 41 and 45 observed) with a no-health-drop
sanity, and flushes the caster socket post-cast
(the first bind missed because completion packets
arrived while only the ally was polled).
(4) Lightning Breath 24844/25012 are wind-serpent
autocasts (powerType focus): the engine honestly
answers NO_POWER (77), so a shared `pre_dispatch`
rule routes focus-cost rows to no-pet BLOCKED (no
shipped matrix carries a focus-cost row; the
self-test totality pins exactly these two).
The class gate now accepts the documented
engine-totem blocker (Sentry stays BLOCKED with
live failing-cast evidence). Honest BLOCKED
remainder: 45 effect-marker-only (38 totems +
Windfury x2, Parry, Purge x2, Cure x2), 2 no-pet
(Lightning Breath), 2 wire-target-unsupported
(Ancestral Spirit x2, RESURRECT seam), 1
fixture-reagent (Reincarnation 21169, no 17030 in
bags), 1 engine-totem (Sentry 6495). Totem engine
outcome: NO slice implemented — all 39 totem rows
stay BLOCKED; a live Sentry probe (dest-cast 6495
→ CAST_RESULT(145), opcodes [169,221,304,502] with
no GO, no new objects, no aura 6495) is the
failing-cast evidence. Slot bookkeeping, passive
casts, active pulse AI, and lifetime remain the
described future slice. Flakes noted, none
reproduced: the first pilot attempt failed Warrior
6546 (stance aura) and Druid 8921 (dummy reset),
then passed clean; Chain Heal r3 read CASTER_DEAD
once, then passed twice; one full-run attempt
aborted on the drift gate (health=1350 after a
dummy-battered run; manual reseed verified
999999, rerun green). Bench provenance: rebuilt
mangosd debug from the current tree twice
(mode-45 check+fill; heal log) and restarted ONLY
the proven bench-lan pair on 18099 (listener
verified each time); the side-bench pair and
realmd were never touched. Pilot
SPELL-CAMPAIGN-PILOT-PASS before and after;
P/D/W/H regression smokes (10310+20924,
6795+9896, 688+7812+980, 136+3044) PASS on the
final binary; generator + campaign self-tests
PASS; cargo fmt clean; clippy -D warnings clean
(mangos-map); cargo test -p mangos-map lib 99
pass; full live_spells (4 tests including the new
chain-heal target test) PASS with
CHARACTER_LOAD_GUID=1; ruff check + format clean.
Shipped results files byte-identical (restored
after every smoke; only the Shaman artifacts and
one summary section added). No commit or push was
made.
Central verification of the Shaman slice (the
workflow verifier went silent and was stopped at
the owner's restart point; parent verified
centrally): results 1:1 vs the 93-row matrix,
zero vacuous PASS rows, BLOCKED all in the
closed enum (45 effect-marker-only covers the
totem-placement shapes, 2 wire-target, 1 each
engine-totem/fixture-reagent/no-pet pair),
fmt/clippy/self-tests/ruff clean, mangos-map
99 unit tests pass, live bench_spells_and_auras
PASS with CHARACTER_LOAD_GUID=1. Totem rows
riding classify-only stay tracked in
PARITY-LONG-TAIL section 2 (totem engine
completion converts them).
Shaman deploy (same day): mangosd dd746fb3 built
in the ubuntu:22.04 amd64 recipe, side-port
boot-checked, droplet backed up, swapped
binary-only (previous pair preserved as
mangosd.6d28-bak; realmd unchanged), 8085
listening on the fresh boot.

Warrior spell campaign full run (2026-11-24, bench
127.0.0.1:18099, fan-out matrix 85 cast rows over
merged S7 chains: seed G1 union 121 + fury talent
17 known, cost exactly 51, overlap [], excluded []):
first full run 30 PASS / 19 PASSIVE-VERIFIED /
16 BLOCKED / 20 FAIL, final rerun 54 PASS /
19 PASSIVE-VERIFIED / 12 BLOCKED / 0 FAIL
(`run-class Warrior: 85 rows`, zero failure
clusters; honest BLOCKED remainder 12
effect-marker-only, zero FAILs). Failure clusters
from the runs, all fixed test-first in
shared-root-cause order:
(1) rage-blind setup (16 rows NO_POWER): the harness
read UNIT_FIELD_POWER1 (23, the mana slot, absent
for a rage user) instead of field 24, so every
setup saw 0 rage; the field fix plus a
cooldown-aware Bloodrage loop (live 65 s waits on
NOT_READY, up to 8 cycles) recovered 10 rows, and
the rest needed combat maintenance — rage drains
2/tick out of combat (fork-exact) and a 60 s
cooldown wait with nothing referencing the warrior
wipes the pool faster than Bloodrage refills it
(probe: pool 0 after 3 Bloodrages +268 s waits,
tap never engaged because Rend is stance-gated
and stanceless rows read 86). Fix: an Execute
(20647, stanceless, 1 rage, ~1 damage) tap on the
row dummy (spare counter borrowed for dummy-less
self/ground rows) the moment Bloodrage answers
NOT_READY, so retaliation holds combat through
the wait; verified live on 250- and 300-cost rows
before the rerun.
(2) mixed unit+area targeting (Intimidating Shout
5246 ground→hostile reroute still read 9 then
GO-no-aura): the merged victim list dropped the
explicit unit target in `collect_targets` (fill-time
script check), so the 20511 stun trigger never
fired. Fork-exact engine fix with a live-before
regression pair (`target_exemption_tests`: trigger
lands on the exempt main target, area fear
skips it): the explicit unit target now survives
the fill-time check (fork `PushTarget` path) while
area effects still exclude it per effect (fork
per-effect `CheckTarget`); the hostile leg checks
the triggered holder (live_oracle 20511) when the
derivation is vacuous. No other fan-out matrix
has a ground-bucket row with a hostile unit
target, so no shipped routing changes.
(3) shared 30-min category (Shield Wall 871 arms
category 132/1800 s, so Recklessness 1719 reads
NOT_READY): the self leg now waits the live
oracle category total in capped chunks and
retries (one ~30 min wait, then PASS).
(4) self-heal rows (Bloodthirst restores 23880/89/
90/91, cost 0, heal 10-20): a relog self-heal leg
(SQL-damage to 300 offline like the ally legs,
relog, cast, heal-log bind against the oracle
band) — all four PASS with logs 10/13/17 in band.
(5) wounded-warrior deaths (7386 read CASTER_DEAD
at 48 hp after a 101 s setup): ambient dummy
retaliation grinds health down across the run, so
rows below 1200 hp relog SQL-topped-up before
casting (rage persists across the relog; stances
are entered per row afterwards).
(6) aura-lag flakes (Shield Block 2565 GO with no
aura twice; Recklessness once; Piercing Howl 6 s
daze vs the 6 s ground window): self-leg and
stance entries poll for the aura (8 s) and the
ground leg latches short auras in-window (plus
the post-window check), and form entry retries
once from a verified-bare caster form.
Stance/rage/gear scenario outcomes: Battle (2457)
first with Defensive (71) and Berserker (2458)
seeded, warrior cancel set, stance-gated rows
ride form_gated with setup verification (one
2457 entry flaked GO-no-aura, passed on the
verified-bare retry; one Berserker entry flaked
once the same way); rage via Bloodrage cycles +
tap-engaged beater (3-cycle 300-cost rows verified
live); fixture gear (1161 mainhand + 1166 shield
offhand) seeds every run with the honest
fixture-gear BLOCKED path intact (no Warrior
row routes the item leg); Charge/Intercept stay
BLOCKED effect-marker-only (no movement leg);
Thunder Clap/Demoralizing ride hostile/ground
areaselect correctly (Demoralizing 11556 PASS);
Overpower needs no combo gate (none-marker, no
combo path); Execute 20647 unstanced PASS
(1 damage, rage drained fork-exactly, no <20%
gate in the engine so no pre-damage scenario);
Taunt/Mocking Blow left as derived (aura/damage
present, castable; marker-only disposition stays
open). Mocking Blow 20560 needed one dummy
re-pick after repeated OUT_OF_RANGE (new hostile
re-pick arm, like the dead-dummy arm).
Bench provenance: rebuilt mangosd debug from the
current tree (target-exemption engine fix) and
restarted ONLY the proven bench-lan pair on 18099
(listener verified; the side-bench pair and realmd
never touched). Pilot SPELL-CAMPAIGN-PILOT-PASS
before and after (known flakes: Druid 8921
dummy-reset twice and the 5187 heal-band edge
once, then clean); P/D/W/H/S regression smokes
(10310+20924, 6795+9896, 688+7812+980, 136+3044,
8042+331) PASS on the final binary and tree;
generator + campaign self-tests PASS; ruff clean;
cargo fmt clean; clippy -D warnings clean
(mangos-map); cargo test -p mangos-map lib 101
pass (incl. the 2 new exemption tests, failing
before the fix); full live_spells (4 tests incl.
bench_spells_and_auras) PASS with
CHARACTER_LOAD_GUID=1. Shipped results files
byte-identical (restored after every smoke; only
the Warrior artifacts and this section added).
No commit or push was made.

Warrior correction — Execute HEALTHLESS_20 gate
(2026-11-24, same bench/tree): the verifier
falsified the section above's Execute claim.
Row 20647 had PASSed at dummy health 100->98
and the record called the missing <20% gate
fork behavior; the fork `Spell::CheckCast`
TargetAuraState gate (`Spell.cpp:5868-5875`)
rejects Execute unless the target carries
HEALTHLESS_20 (health strictly below 20%, the
bit `SetHealth` maintains). The Rust engine
loaded neither aura-state field: fixed by
loading `casterAuraState`/`targetAuraState`
(spell_template select positions 17/18) into
`SpellEntry`, adding
`aura_state::HEALTHLESS_20_PERCENT = 2`, and
gating the cast in `cast.rs` (computed live
from unit health — no aura-state field is
maintained; only HEALTHLESS_20 is modeled):
unit target at/above 20% reads
`FAILED_BAD_TARGETS` (10), a non-unit cast
reads `FAILED_BAD_IMPLICIT_TARGETS`.
Regression `execute_gate_rejects_above_20_
percent_health` (`combo_tests`): observed
`Started != Failed(10)` before the fix, green
after, including the open-gate cast on a
directly wounded sub-20% victim. The matrix
now flags `requires_low_health_target` (only
the three Execute rows 5308/20647/20662); the
live 20647 scenario enters Battle Stance and
pre-damages with repeated Rend (6546) to an
alive sub-20% read before casting — final
row pre shows 4 Rend casts to hp 10/100, then
the bound Execute damage log. A full-health
smoke reads CAST_RESULT(10). Rows 5308 and
20662 stay effect-marker-only BLOCKED.
Follow-on harness break, fixed test-first:
the first corrected full rerun read 50 PASS /
19 PASSIVE-VERIFIED / 12 BLOCKED / 4 FAIL, one
NO_POWER cluster (20569, 23923, 23925, 25286)
— the rage setup's combat tap had been
Execute itself (1 rage, ~1 damage at full
health), illegal after the gate, and its
Rend/Hamstring replacement never landed on
stanceless rows (86), so every Bloodrage grant
decayed through the 65 s cooldown wait
(`bloodrage short: rage 0 after 4 cast(s)
+286s`). A damage-free Demoralizing Shout tap
was tried and falsified live (aura landed,
dummy never attacked, rage 680->0 in 60 s).
The tap is now stance-aware damage: Rend in
Battle/Defensive, Hamstring in Berserker,
Battle entry first when stanceless (the
affected rows carry stance mask 0), melee
approach first, engagement proven by the tap
aura on the dummy; a 60 s live probe then held
rage flat with the dummy attacking and alive.
Final rerun: 85 rows, 54 PASS / 19
PASSIVE-VERIFIED / 12 BLOCKED
(effect-marker-only) / 0 FAIL, class-gate
PASS; the six-row failure subset (incl.
Execute and a 300-cost Mortal Strike) passed
first with `tap-engaged` on the previously
failing rows. Gates on the final tree: pilot
PASS before the first rerun and after the
final (temp-root artifacts; one Druid 8921
dummy-reset flake, clean on rerun); both
Python self-tests and ruff clean; full
`./ci.sh` with the `.env.test` environment
exit 0 (fmt, workspace clippy -D warnings,
workspace build/tests, and the env-gated live
suites incl. live_spells 4/4, no skip lines);
independent artifact audit PASS (row set ==
matrix, manifest == results, every PASS
carries evidence + SPELL_GO, Execute predamage
alive below 20%). Pilot-clobbered shipped
artifacts were restored to HEAD before the
final run, so the summary diff is only the
appended Warrior section.
