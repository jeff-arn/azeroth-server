# Phase 6 — character save cutover (progress)

## Slice plan

0. Instance entry (enabling debt from Phase 4 Slice 7;
   unblocks the deferred Deadmines gates).
1. Character save (differential byte-compare on a
   cloned bench DB).
2. Canary preparation only — no production cutover
   without a separate owner gate.

## Slice 0 inventory (2026-10-19)

Fork path (`WorldSession::HandleAreaTriggerOpcode`,
`MiscHandler.cpp:675`): DBC trigger lookup →
`IsPointInAreaTriggerZone` (5 yd slack) → script /
quest / tavern-rest / BG-entrance branches →
`areatrigger_teleport` row → level (patch ≥ 1.4,
`Instance.IgnoreLevel` config) + condition gates
with `SendAreaTriggerMessage` → `Player::TeleportTo`
(`Player.cpp:1846`): same-map = MSG_MOVE_TELEPORT to
controller + ACK; cross-map = SMSG_TRANSFER_PENDING →
RemoveFromMap → SMSG_NEW_WORLD → AddToMap on the
instance resolved through MapManager + binding.

Data on the bench: `areatrigger_teleport` 121 rows
(Deadmines entrance id 78 → map 36 (-15,-385,62),
required_level 10; exits 119/121 → map 0),
`areatrigger_template` 502 rows (DBC-side trigger
zones), `character_instance (guid, instance,
permanent)` binding table exists in characters DB.

Rust seams: `Map::new` already takes `instance_id`
(Phase 4 content requires a real one: door chain
only fires when instance_id != 0). Gaps:
actor `maps: HashMap<u32, Map>` keyed by map_id only
(instance 0 hard-coded in `enter_map`);
`MaterializeInputs.saved_map_is_instance` hard-coded
`false` in `world_actor::materialize_player` (saved
instance positions are currently *accepted* onto an
instance-0 map — fork instead relocates to the
instance entrance or homebind by bind state);
no areatrigger loaders, no CMSG_AREATRIGGER handler,
no cross-map teleport packet flow, no binding store,
no instance-id allocation.

Slice 0 sub-slices:
- 0a data: `areatrigger_template` +
  `areatrigger_teleport` loaders (world-data), Map
  DBC instanceable/parent lookup, `character_instance`
  load/write + instance reset bookkeeping (mangos-db).
- 0b actor: `(map_id, instance_id)` map keying everywhere
  `player_map` points; instance-id allocation; group
  bind resolution shared with login; real
  `saved_map_is_instance` computation (bound instance
  login, else fork entrance relocation).
- 0c teleport: CMSG_AREATRIGGER (teleport + tavern
  branches first; quest/script branches follow fork
  guards), same-map MSG_MOVE_TELEPORT + ACK,
  cross-map TRANSFER_PENDING/NEW_WORLD with object
  rebuild for mover and watchers.
- 0d verify: scripted portal entry into Deadmines,
  five-bot clear per role, OrbStack visible entry.

## Slice 0a — data loaders (done, 2026-10-19)

- `mangos-world-data/src/map_template.rs`: full
  `map_template` loader (patch-max), fork parent
  fixups (missing/continent parent on dungeons → 0)
  and reset-delay rate scaling (`Rate.InstanceResetTime`,
  new config key, default 1.0). Ghost-entrance erase
  validation not ported (needs valid-coord seam;
  recorded for 0b).
- `mangos-world-data/src/area_trigger.rs`:
  `areatrigger_template` (max build ≤ 5875) +
  `areatrigger_teleport` (patch-max), fork fixups
  (scriptless condition/cooldown zeroed; teleports
  with unknown trigger/map or zero destination
  dropped), `IsPointInAreaTriggerZone` ported
  exactly (radius + rotated box), entrance/go-back
  lookups. `script_name` kept as the DB string (fork
  resolves to a core script id; handler slice owns
  that mapping).
- `mangos-db/src/instance.rs`: boot load of
  `instance` + `character_instance` +
  `group_instance` with `InitMaxInstanceId` floor
  (100), fork-exact bind insert/update and instance
  insert statements. Not yet wired into the actor
  (0b owns allocation/binding state).
- Wired into `WorldStores` (`maps`, `area_triggers`)
  + boot counts. Bench: ">> Loaded 44 map templates,
  449 area triggers, 115 area trigger teleport
  definitions" — exact match vs independent SQL
  patch/build-max counts. `./ci.sh` exit 0
  (89 suites, new unit tests in both modules);
  `TWO-CLIENT-SMOKE-PASS` after restart.

## Slice 0b — instanced map keying + binding (done, 2026-10-19)

- `MapKey { map_id, instance_id }` replaces raw map-id
  keying across the world actor (`maps`,
  `player_map`, bot host, handlers). Continents and
  non-instanceable maps run as instance 0.
- `InstanceRuntime` (allocation from the boot
  `InitMaxInstanceId` floor, char/group binds) +
  `InstanceStore` writes. `resolve_map_key` at map
  entry implements the fork bound-state order
  (personal bind, else group bind) and the
  `InstanceMap::Add` entry binds: fresh allocation
  inserts the `instance` row (reset now+2h non-raid,
  0 raid), grouped entry creates the group bind and
  drops other members' non-permanent personal binds,
  permanent group bind grants the personal permanent
  bind, solo entry creates the non-permanent
  personal bind. Per-character `bound_instances`
  from character load seed binds the boot snapshot
  lacks.
- Login: `saved_map_known`/`saved_map_is_instance`
  now computed from `map_template`; a saved
  in-dungeon position survives only when a bound
  instance matches the saved instance id (fork
  `_LoadFromDB` rule). Deviation: unbound logins go
  to homebind where the fork relocates to the
  go-back trigger destination — go-back lands with
  the 0c teleport flow. `SMSG_INSTANCE_SAVE_CREATED`
  on permanent-bind grant also lands in 0c.
- Verified: `./ci.sh` exit 0 (89 suites);
  `/tmp/instance_login_probe.py` —
  `INSTANCE-LOGIN-PROBE-PASS` (unbound Deadmines
  login relocates to homebind; bound login lands
  inside map 36 at the entrance);
  `SCRIPTED-WORLD-PASS`, `BOT-PROBE-PASS`;
  two-client smoke intermittent on this bench
  (passes on fresh boots; same flake family as
  Phase 5 closeout).

## Slice 0c — area-trigger portal + cross-map teleport (done, 2026-10-19)

- `CMSG_AREATRIGGER` handler + far-teleport flow in
  fork order (`SMSG_TRANSFER_PENDING` /
  `SMSG_NEW_WORLD` / saved-instances from
  `SendNewWorld` only; `SMSG_INSTANCE_SAVE_CREATED`
  at worldport-ACK finish). `Instance.IgnoreLevel`
  honored; raid-dungeon level gate at
  `Progression >= WOW_PATCH_104` (generated
  `Progression.h` numbering, 104 = 2).
- Tavern triggers toggle `PLAYER_FLAGS_RESTING`
  (0x20). Go-back login relocation: unbound
  in-dungeon logins relocate via the destination's
  go-back trigger (group bind first; Naxxramas
  hardcoded exit) instead of homebind.
- Recorded deviations (in code): DB-script /
  `OnAreaTrigger` script execution unported,
  quest-explore credit skipped (no quest
  progression), ghost corpse-in-dungeon re-entry
  refused (no corpse tracking), BG/cheat/taxi
  branches absent, localized teleport messages
  absent, rest accrual deferred to the save slice.
- Verified: `./ci.sh` exit 0;
  `/tmp/dm_entry_probe.py` — `DM-ENTRY-PROBE-PASS`
  (portal entry, group bind, same-instance co-entry
  relay, exit trigger);
  `INSTANCE-LOGIN-PROBE-PASS` with go-back
  relocation; `SCRIPTED-WORLD-PASS`,
  `BOT-PROBE-PASS`.

## Slice 0d — party-bot portal follow (done, 2026-10-19)

- Fork party-bot catch-up teleport (`.goname`-style):
  a session-less bot whose leader is >100yd away or
  on another map teleports to the leader when out of
  combat and the leader is alive; reuses
  `teleport_player` (bots finish worldport inline)
  and clears the formation trail per the fork.
- Verified: `/tmp/bot_portal_probe.py` —
  `BOT-PORTAL-FOLLOW-PASS` (bot observed at the
  Deadmines destination via monster-move relays).

## Verify Slice 0 — Deadmines five-bot clear (in progress, 2026-10-19)

Prerequisite parity debt found and fixed while
running the deferred Phases 4–5 clear gate:

- Instance-scoped respawn rows: creature/gameobject
  respawn store/lookup/clear now carry the instance
  id (was hardcoded 0 — one Deadmines kill's ~12h
  delay poisoned every later instance) plus
  outdated-row cleanup at spawn load and
  orphan-instance respawn cleanup in
  `load_instances` (reused instance ids). Full
  `PackInstances` lifecycle remains unported.
- `Heights::ground_z` no longer blindly returns the
  gridmap height above dungeons (302 over
  Deadmines): a grid height >10yd above the hint
  falls back to the hint. The vmap height/LOS
  manager remains deferred debt.
- Player regeneration ported
  (`Player::RegenerateAll`/`RegenerateHealth`/
  `Regenerate` + `UpdateManaRegen`, spirit tables,
  fraction carry, five-second rule via
  `SetLastManaUse` at `TakePower`); previously only
  creature regen existed, so casters drained to
  `SPELL_FAILED_NO_POWER` permanently. Rates are
  the fork 1.0 defaults (`Rate.*` config parsing
  not yet ported). Deviations recorded in
  `mangos-map/src/regen.rs`.
- Equipped-item stat application at materialize
  (fork `_ApplyItemMods`/`_ApplyItemBonuses` end
  state at zero aura modifiers): stat `BASE_VALUE`
  sums + pos/neg display halves, armor/resistances
  (holy written 0), max health/mana with item
  health/mana, melee/ranged attack power, weapon
  damage fields + attack times, shield block value;
  player arm of the AP-aura damage refresh now runs
  `CalculateMinMaxDamage`. Enchantments, random
  properties, set bonuses, equip spells, and
  aura-fed modifier groups remain deferred (noted
  in code). Also fixed `init_combat_percentages`
  reading agility as float bits (`GetStat` is an
  unsigned-word cast, `Unit.h:390`).
- `.bot` info `Total:` is the load-time roster
  snapshot (`m_stats.totalBots`, set once in fork
  `Load` step 7); manual adds no longer move it.
- Result: `DM-SEGMENT1-CLEAR` — entrance corridor
  through Rhahk'Zor with the five-bot party
  (`/tmp/dm_clear_probe.py`, `DM-CLEAR-PROBE-PASS`).
  Segment 2 (Sneed/Gilnid/Smite/VanCleef) extension
  in flight.
- Regression battery after the three fixes:
  `BOT-PROBE-PASS`, `BOT-PARTY-PROBE-PASS`,
  `DM-ENTRY-PROBE-PASS`,
  `INSTANCE-LOGIN-PROBE-PASS`,
  `BOT-PORTAL-FOLLOW-PASS`, `BOTAI-TRACE-PASS`,
  `TRADE-PROBE-PASS`, `WTB-ORACLE-PROBE-PASS`,
  `MAIL-PROBE-PASS`, `AH-PROBE-PASS`,
  `GUILD-PROBE-PASS`, `CMD-PROBE-PASS`,
  `PETITION-PROBE-PASS`, `DASHBOARD-RT-PASS`,
  chat/item-query OK, AHBot populate+buyer OK.
  `bot_chat` ambient trade step is probabilistic
  (120–300s boot-armed fork timer; whisper/service
  routes log-verified); `party_probe` is the manual
  rig-pairing half, not standalone.
- Segment 2 diagnosis (2026-10-19): lumber room +
  Shredder (79223/642) killed repeatedly, Sneed (643)
  never spawns. Fork chain: `creature_ai_events`
  64201 (type 6 DEATH, chance 100) ->
  `creature_ai_scripts` 64201 cmd 15 CAST_SPELL
  5141 `Eject Sneed` (datalong2=7 triggered,
  data_flags=4 target-self) -> spell effect 41
  misc 643. Instrumented bench run proved
  `script_try_cast` is never called for 5141:
  failure is upstream. Verified correct by reading:
  bench DB rows, both loaders, `kill_unit` hook
  ordering, `eventai_active`, holder gates, chance
  roll, `run_script_direct` table, target
  resolution, dead-caster gate, summon lifecycle.
  Bisect prints (`EAI-DEATH`/`RUN-SCRIPT`,
  temporary) confirm the chain works for other
  creatures; awaiting a clean Shredder kill under
  the prints to localize.

## Slice 1 inventory: fork `Player::SaveToDB` surface

Fork source `vmangos/src/game/Objects/Player.cpp`
(line refs at inventory time):

- `SaveToDB(online, force)` :16229 — gates:
  `IsSavingDisabled()` (bots never save),
  far-teleport deferral (`DELAYED_SAVE_PLAYER`).
  `BeginTransaction(GUIDLow)` — per-character
  serial lane; `m_nextSave` = `Interval.Save`.
- 61-bind `REPLACE INTO characters` :16253 —
  position via teleport-dest when teleporting
  (instance written 0), `finiteAlways` floats,
  `NormalizeOrientation`, text blobs
  `known_taxi_mask` / `current_taxi_path` /
  `explored_zones` (64 words) / `equipment_cache`
  (`MAKE_PAIR32` perm/temp enchant per visible
  slot + 1 bag slot), honor block, `drunk` =
  `PLAYER_BYTES_3 & 0xFFFE`, `logout_time` =
  `time(nullptr)` at every save, `create_time`.
- Bundle order :16396-16405: `_SaveBGData`
  (`character_battleground_data`, gated
  `m_needSave`), `_SaveInventory` :16561
  (item uState machine: NEW/CHANGED/REMOVED over
  `character_inventory` + `item_instance` via
  `Item::SaveToDB`, anti-dupe DELETE),
  `_SaveQuestStatus` :16677 (quest uState),
  `_SaveSpells` :16812 (spell uState),
  `_SaveSpellCooldowns` :4042 (delete-all +
  reinsert live), `_SaveAuras` :16456 (delete-all
  + reinsert savable), `_SaveSkills` :16745
  (skill uState + `character_forgotten_skills`),
  `m_reputationMgr.SaveToDB()`
  (`character_reputation` dirty set),
  `m_honorMgr.Save()` (`character_honor_cp`),
  `GetSession()->SaveTutorialsData()`
  (`account_tutorial` in login DB — dual-writer
  note: session-scoped, not character row).
- Post-commit: `_SaveStats` :16841
  (`character_stats`, logout-or-config gated),
  `Pet::SavePetToDB(PET_SAVE_AS_CURRENT)`.
- Adjacent writers in scope for parity checks:
  `SaveInventoryAndGoldToDB` / `SaveGoldToDB`
  :16425/:16445 (money UPDATE), `SavePositionInDB`
  :16892 (printf UPDATE, zeroes transport +
  `current_taxi_path=''`), `SaveRecallPosition`
  :6107.

Rust today: `mangos-db::character_load` reads the
full inverse (characters row 58 cols + `instance`
append, auras, spells, queststatus, honor_cp,
reputation, inventory+instance join, item_loot,
actions, homebind/skills/etc. per holder). No
character save exists; Phase 3-5 write register
covers presence/respawn/auction/mail/petition/
instance rows only. Known aggregate gaps to close
for byte-parity (from earlier slices): runtime
equip/unequip stat reapplication, rest accrual,
played-time accumulation, explored-zones updates,
taxi mask/path mutation, drunk byte, watched
faction, honor manager live state, BG data,
pet save. Oracle: cloned bench DB, C++ save ->
Rust load -> Rust save -> per-table row
byte-compare (§5: blobs `''` never NULL, `atol`
coercion, f32 rounding, low-GUID only, REPLACE
shapes, GUID-low serial ordering).

## Slice 1 — character save + differential oracle (done, 2026-10-19)

Implementation: `crates/mangos-db/src/character_save.rs`
(61-bind characters REPLACE, auras/cooldowns/spells/
skills/quests/reputation/honor/BG/stats, inventory/
item/loot statement sets, homebind inserts,
`save_to_db` bundle in fork order BG→inventory→
quests→spells→cooldowns→auras→skills→reputation→
honor, stats post-bundle gated, actions at logout
only, `SaveInventoryAndGoldToDB` lane). Snapshot
assembly in `crates/mangosd/src/character_save.rs`
(dynamic character flags, taxi/explored/equipment
blobs, transport offsets, `SavedBaseline` diffs
replacing C++ `uState`, re-baseline after success).
Scheduling in the world actor: synthesized-homebind
insert at login, played-time accrual by whole
in-world seconds, periodic save on
`PlayerSave.Interval` (default 900000, fork
urand-staggered first fire), logout bundle+actions
(including forced pending-teleport form), save-all
on actor shutdown. Bots save only under
`PlayerBot.AllowSaving` (default false). Opcodes:
`CMSG_SET_ACTION_BUTTON`, action-bar toggles,
`CMSG_PLAYED_TIME`.

Differential oracle (`characters_clone` =
mysqldump of bench characters; canonical dumps via
`vmangos-rs/scripts/character_state_dump.py`;
scripted clients through the real bench binary
pointed at the clone):

- Idle pass (guid1 Ruikanis, the richest bench row:
  96 quest rows, 54 reputations, 45 inventory rows,
  41 spells, 30 actions, 2 auras): after
  login/logout, **every table byte-identical** except
  `played_time_*` (+7s session), `logout_time`
  (now), `health` (out-of-combat regen), and one
  aura `duration` (-7.5s) — all fork-exact live
  drift. All blobs (`known_taxi_mask`,
  `current_taxi_path`, `explored_zones`,
  `equipment_cache`), float positions, honor
  floats, quest timers, and item rows round-trip
  byte-exact.
- Mutation pass: movement writes exact
  position/orientation floats; action-button
  set/remove/add produces exactly the three
  expected `character_action` deltas; Bloodrage
  writes the 60s cooldown row with epoch expiry.
  (Money mail errored on mailbox range at the
  Westfall position — `characters.money` itself
  round-trips inside the row REPLACE.)
- Fresh-aura pass (priest PW:Fortitude self-cast):
  new `character_aura` row saved with correct
  base points (20), max/duration (1800000/~1793k),
  mask. Battle Shout's earlier absence was a rage
  cast failure (`SPELL_FAILED_NO_POWER` 77 on the
  wire), not a save-filter drop.
- Periodic pass (`PlayerSave.Interval = 20000`):
  moved position and accrued played time were in
  the clone DB **before logout** —
  `PERIODIC-SAVE-PROVEN`.
- Bench stack pass (real bench `characters`):
  Groktar move → logout → relog lands at the moved
  position — `BENCH-PERSISTENCE-PASS`.

Adjudicated gaps (recorded, not hidden): no XP/
level-up runtime exists, so the level-up
persistence gate is unrunnable until a progression
slice lands; dependent-spell set is empty by
construction (no learn path mutates
`deferred.spells`; documented at the `diff_spells`
call site — the first learn path must keep
dependent rows out or pass them); pet save absent
(pet system unported); item cooldown rows and
`m_permanentCooldowns` unmodeled; aura
`AuraInterruptFlags` leave-world gate unparsed,
cast-item guid/charges written 0; BG
`m_needSave` never set (pre-existing
`character_battleground_data` rows are never
cleaned at BG-map login); tutorial saves stay in
the login-DB session lane as in the fork;
spell-crit/healing stats write 0; runtime
equip/unequip item queue absent (inventory is
service-DB-backed, so the save queue is all
`Unchanged` today).

## Slice 2 — canary preparation (done as prep, 2026-10-19)

`PHASE6-CANARY-PREP.md`: off-box build recipe
(container, glibc floor, Detour `g++` need, repo
root mount for vendored headers), same-`ExecStart`
install with in-place C++ binary preservation,
validation checklist, binary-first rollback. The
recipe was executed locally: Linux aarch64 release
`mangosd`/`realmd` built clean, max symbol
`GLIBC_2.34` ≤ 2.35 floor. The droplet-arch
(amd64) build also completed clean after a host
disk-full episode was cleared (17 GiB reclaimed
from build caches): x86-64 ELF, max symbol
`GLIBC_2.34`. **Nothing was installed anywhere; no
production cutover without a separate owner gate.**

## Verify Slice 0 residuals (2026-10-19, later runs)

Three more live five-bot attempts on the bench:
run 1 stalled on the Evoker (probe waited for one
exact spawn; `entry_fallback` — already used for
neighboring trash — was missing on that call and
fixed probe-side); run 2 (after the fix) cleared
entrance trash fast (Miner/Evoker/both Overseers)
then wiped at Rhahk'Zor (priest → tank → master →
warlock → hunter, `UnitDied` 17:11–17:13). Combined
with prior `DM-SEGMENT1-CLEAR` runs and the
harness-proven Sneed/EventAI chain
(`LIVE-DEADMINES-PASS`), instance entry/binding/
portal-follow are verified; a live `DM-FULL-CLEAR`
is still unproven and is a bot-survivability/probe
navigation problem (patrol/add variance), not an
instance-system or save defect. Also still open
from the phase verification list: OrbStack
level-up persistence (no XP runtime to level with)
and live payload diffs / LevelUp replay (no C++
fork binary on this box to diff against; the
clone-DB differential oracle above is the
substitute evidence for save bytes).

## Slice 1b — true C++ differential + parity fixes (done, 2026-10-19)

The Phase 0 C++ oracle binary (production fork,
`2bae5f9`) was rediscovered off-box and run in
Docker against InnoDB-converted clones of the same
base dump as the Rust clones, upgrading the Slice 1
oracle from Rust-vs-Rust to true C++-save vs
Rust-save byte comparison. The first cycles found
four real gaps, all fixed fork-exact:

- Quest rows: timed quests now convert absolute
  `timer` epochs to remaining-ms at load and project
  back at save; deliver-count quests are recounted
  at load (`AdjustQuestReqItemCount`) and the
  touched set rewrites every save, matching fork
  `QUEST_CHANGED` churn.
- Rest: offline accrual at load plus online accrual
  (`ComputeRest`/`SetRestBonus` rates at fork
  defaults; `Rate.Rest.*` config not yet wired, same
  posture as other `Rate.*`), resting-flag
  maintenance on zone change (capital area flags,
  PvP-realm enforcement) and tavern areatriggers.
  Tavern exit-box clearing is not ported.
- Energize: `PERIODIC_ENERGIZE` joined the periodic
  aura list, instant `ENERGIZE` applies, and
  `SPELL_EFFECT_TRIGGER_SPELL` casts through the
  triggered pipeline (Bloodrage 2687 → 29131).

Final paired run (fresh clones, identical scripted
mutation cycle — login, legal move, action buttons,
Bloodrage, money mail, logout): 294 dump lines per
side, **zero unexplained diffs**. Position is
byte-exact (`-10639.6, 1184.13, 34.7032, o=2`).
Residual diffs are all adjudicated non-save
classes: 58 quest timers and the spell-cooldown
expiry differ by exactly the cycles' wall-clock
offset (uniform −107s / +106s vs `logout_time`),
one aura `duration` by 68ms of remaining-time at
save, and `health`/`power2` as live-stat-at-save
variance (below).

Two adjudications worth recording:

- **Health** (C++ 145 vs Rust 297 in the final
  pair): guid1 carries Resurrection Sickness
  (15007). The Rust aura runtime does not yet apply
  `MOD_STAT`/stat-percent aura effects, so Rust
  stats/regen run high while the aura *row* saves
  identically. This is a Phase 4 aura-application
  gap, not a save-serialization defect; `power2`
  (200 vs 180) is the same class of tick-timing
  variance (an earlier paired check matched at
  180=180, so it is variance, not a constant
  offset). Save correctness for these fields is
  write-what-live-state-holds, proven by every
  other field.
- **Movement**: the fork silently drops all
  movement opcodes until the client sends
  `CMSG_SET_ACTIVE_MOVER` (`GetConfirmedMover()`
  null, `MovementHandler.cpp:294`), and its
  anticheat rejects flagless displacement
  (`CheckTeleport`) and forward heartbeats without
  `MSG_MOVE_START_FORWARD` (`CheckMoveStart`). The
  scripted client's earlier position divergence was
  this input path; with the handshake and a legal
  sequence both servers save the identical final
  position. Rust has no movement anticheat and no
  active-mover gate (Phase 3 leniency; real clients
  always send the handshake and are unaffected).

Full `vmangos-rs ./ci.sh` after Slice 1b:
`CI_EXIT=0`, 127 suites. Remaining phase residuals
unchanged: live `DM-FULL-CLEAR` (bot
survivability), OrbStack level-up persistence (no
XP runtime), live payload diffs / LevelUp replay
(now feasible with the oracle binary, not yet run).
No production cutover; canary stays gated on a
separate owner decision.

## Phase 6 closeout (2026-10-19)

Owner waived the three residual verification gates
(live `DM-FULL-CLEAR`, OrbStack level-up
persistence, live payload diffs / LevelUp replay)
as follow-up slices for a later goal and accepted
the Slice 0/1/1b/2 evidence above as Phase 6
complete. Work is left uncommitted on
`rust-migration` pending an owner commit request.
Production cutover remains behind its own separate
owner gate (see
[PHASE6-CANARY-PREP.md](PHASE6-CANARY-PREP.md)).
