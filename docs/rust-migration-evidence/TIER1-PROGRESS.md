# Tier 1 parity runtimes — progress

Goal: build the missing runtime subsystems that block
recorded depth-gap handlers (quest turn-in, pets,
item use, dynamic objects, cross-map transfer,
knockback, mount/transform), fork-exact against
production C++ `2bae5f9`.

## Slice 1 inventory — quest turn-in (2026-10-20)

Fork flow (`Handlers/QuestHandler.cpp`, 784 lines;
`QuestDef.cpp`; `Player::RewardQuest` in
`Objects/Player.cpp`):

- `CMSG_QUESTGIVER_STATUS_QUERY` → questgiver
  status from player quest state + creature quest
  relations.
- `CMSG_QUESTGIVER_HELLO` → `SMSG_QUESTGIVER_QUEST_LIST`
  (available + completable quests at the NPC).
- `CMSG_QUESTGIVER_QUERY_QUEST` → details packet;
  `CMSG_QUESTGIVER_ACCEPT_QUEST` → `CanTakeQuest` +
  `AddQuest` (quest log slot, src item/spell,
  timed-quest arming).
- Objective progression feeds `QuestStatusData`
  counters (`m_creatureOrGOcount`, `m_itemcount`,
  explored flag) from kill/item/explore hooks.
- `CMSG_QUESTGIVER_COMPLETE_QUEST` →
  `SendQuestGiverRequestItems` (can-complete flag).
- `CMSG_QUESTGIVER_REQUEST_REWARD` →
  `CanCompleteQuest` + `CompleteQuest` (status →
  COMPLETE) → `SendQuestGiverOfferReward`.
- `CMSG_QUESTGIVER_CHOOSE_REWARD` →
  `CanRewardQuest` → `RewardQuest`: destroy req
  items, grant choice + fixed reward items,
  reputation, clear log slot, XP =
  `Quest::XPValue(level) * Rate.XP.Quest`
  (level-band scaled `ceilf`, QuestDef.cpp:176),
  max-level XP→money conversion, `RewOrReqMoney`
  (positive scaled by `Rate.Drop.Money`), reward
  mail, status rewarded/COMPLETE (NONE if
  repeatable), `SMSG_QUESTGIVER_QUEST_COMPLETE`
  (`SendQuestReward`), reward spell cast, script
  hooks.

Rust state before this slice:

- `quest_template` **Lite** store only (ReqItem
  ids/counts + LimitTime), built for Phase 6
  load/save parity. Its loader comment claims no
  `patch` column — stale: the bench schema has
  `patch` (4725 rows across patches 0–10) and the
  Lite loader loads all rows with unspecified
  winner order. The full store must select
  max `patch <= WowPatch` like the other template
  loaders.
- `character_queststatus` load/save complete
  (deferred rows + save projection with timer
  semantics), so runtime mutations of
  `deferred.quest_status` persist for free.
- No `CMSG_QUESTGIVER_*` handlers; opcode ids
  exist in the protocol table.
- `Rate.XP.Quest` already plumbed into map XP
  rates (world_actor config → `XpRates.quest`).
- Money mutation exists (`modify_money`); item
  grant has a DB-row precedent (petition charter,
  `mangos_db::petition::create_charter_item`) but
  its client push notes it "joins with the runtime
  item system" — the live grant path must be
  pinned during 1b.
- Area-trigger quest explore credit is explicitly
  skipped today ("quest progression unported",
  world_actor).

Slice split:

- **1a** — full `quest_template` store (rewards,
  objectives, texts, emotes, flags) with patch
  selection + derived helpers (`xp_value`,
  repeatable, reward counts); questgiver status /
  hello / query / accept handlers; quest log slot
  fields; objective credit for kills, items, and
  exploration through existing hooks.
- **1b** — turn-in: complete/request-reward/
  choose-reward handlers, `CanCompleteQuest` /
  `CanRewardQuest` subsets, `RewardQuest`
  (items via the pinned grant path, reputation,
  XP, money, mail deferred if mail surface is
  unreachable, reward spell via map cast),
  proven by live probe + C++ save differential
  on `character_queststatus`.

Inventory addendum (same day):

- Full `QuestTemplates` store landed in
  `mangos-world-data::quest_template` with patch
  selection (max `patch <= WowPatch`); live-proven:
  `QUEST-TEMPLATES count=4433` at patch 10
  (`live_quest_templates`). The Lite store stays
  for load/save parity.
- Quest log update fields (`PLAYER_QUEST_LOG_*`)
  are never written by Rust today; Slice 1 must
  add a derived log-slot sync from
  `deferred.quest_status`.
- There is **no live inventory-grant path**
  anywhere in the Rust runtime: petition, mail,
  and trade all insert `item_instance` +
  `character_inventory` rows directly, and no
  vendor-buy/loot-store live path exists. Quest
  item rewards in 1b therefore follow the DB-row
  grant pattern (visible after relog) as a
  recorded deviation; Slice 3 (item-use runtime)
  builds the live grant and the reward path
  switches to it.
- World-actor handlers reach the live `Player`
  through `self.maps` + `map.player_mut`, so
  questgiver handlers belong in the world actor,
  with kill credit in the map death path next to
  kill XP (`Map::reward_quest_xp` to be added
  beside `grant_kill_xp`; quest XP takes no
  rested bonus, fork `GiveXP(xp, nullptr)`).

## Slice 1 evidence — quest turn-in (2026-10-20)

Live wire probe (`/tmp/quest_probe.py`, bench
mangosd on `bench-lan.conf`, guid 1): `QUEST-PROBE-PASS`.

- Leg A (quest 432, kill 6× entry 1115, full cycle
  on the wire): `QUESTGIVER_STATUS dialog=2`
  (grey `CHAT` — level 19 vs QL9), quest list
  contains 432, details, accept; incomplete
  `COMPLETE_QUEST` returns `REQUEST_ITEMS`;
  six `SMSG_QUESTUPDATE_ADD_KILL` credits (1/6 …
  6/6, fork layout quest/entry/count/req/victim)
  with `QUESTUPDATE_COMPLETE` at the 6th;
  completable `COMPLETE_QUEST` delegates straight
  to `OFFER_REWARD` (fork `SendQuestGiverRequestItems`
  delegation, no req items); `CHOOSE_REWARD` →
  `QUESTGIVER_QUEST_COMPLETE` with
  `xp=59` (= `ceil(590 × 0.1)`, `XPValue` at diff
  +10) and `money=200`; DB after logout:
  q432 `status=1 rewarded=1`, money 4410→4610.
- Leg B (quest 14 seeded COMPLETE, turn-in at
  level): `REQUEST_REWARD` → `OFFER_REWARD` →
  choose slot 0 → complete packet `xp=1600`
  (full `XPValue` at QL17/19), DB xp delta
  exactly 1600, `reward_choice=1566` (fork stores
  the chosen **item id**), new `item_instance`
  row for 1566 granted into an equipped bag
  (backpack was 16/16 full).
- Fixture restored byte-exact (backup-table
  restore after session logout), verified by
  `live_character_load` + `live_char_enum` oracle
  tests; `TWO-CLIENT-SMOKE-PASS`; full CI green
  (`CI_EXIT=0`).

Save differential on `character_queststatus`
(vs fork `Player::_SaveQuestStatus`): DELETE /
INSERT / UPDATE statement texts are byte-identical
including the fork's spacing quirks (``, ` `` after
`timer`, double space before `WHERE`); bind order
identical. Value semantics: `reward_choice` now
the chosen item id (fork), `timer` = remaining
ms/1000 + now on both sides (fork `GetGameTime`,
Rust save projection), status NEW→INSERT /
CHANGED→UPDATE / DELETED→DELETE classification
matches the fork `uState` discipline.

Fork-exactness fixes landed with the probe:

- Dispatch: quest opcodes were bucketed to the
  Phase-3 stub; added to the guild/social packet
  bucket reaching `on_guild_packet`.
- `CanSeeStartQuest` rewritten to fork semantics
  (Satisfy* set + `GetLevel() + 7 >= MinLevel`);
  previous transcription had the hide window
  inverted. `PrepareQuestMenu` and
  `GetDialogStatus` rewritten to the fork bodies
  (incl. `defstatus = NONE`, leading-emote count,
  auto-complete arms, low-level `CHAT` demotion).
- `SMSG_QUESTGIVER_STATUS` writes u32 (was u8);
  details / offer-reward / request-items /
  quest-complete builders rewritten to the fork
  layouts (leading-`count` reward slots,
  hidden-rewards triple-zero, `RewSpell` field,
  request-items flag tail `0x02/0x03|0x00/0x04/0x08`,
  complete packet `0x03` + reward item slots).
- Turn-in handlers rewritten to fork flow:
  `COMPLETE_QUEST` resolves no NPC and computes
  completable via `CanRewardQuest` /
  `CanCompleteRepeatableQuest`; request-reward
  completes then requires COMPLETE; choose
  gates on `CanRewardQuest`, resends the offer
  on failure, chains `GetNextQuest` details via
  `NextQuestInChain` + starter relation. The
  fork's unconditional
  `QUEST_FAILED_NOT_ENOUGH_MONEY` (22) packet
  from the money arm is reproduced.
- `CanTakeQuest` required-money arm removed
  (fork checks the purse only at reward);
  `skipStatusCheck` arm added for the
  auto-complete `CanRewardQuest` path.
- `GetItemCount` / `DestroyItemCount` now use
  the fork slot walk (equipment/backpack/keyring
  + inventory-bag contents counted; backpack →
  keyring → bags destroyed; bank excluded from
  both — previously bank rows counted toward
  quest item totals).
- Reward grant slot search widened from
  backpack-only to backpack → equipped bags
  (prototype capacities), actor-side selection
  with DB insert at the chosen `(bag, slot)`.

Recorded deviations / unmodeled remainder:

- Item rewards are DB-row grants (visible after
  relog; no `SendNewItem` live push) until
  Slice 3. Stack-merging onto partial stacks
  unmodeled; when every bag is full the reward
  item drops (fork instead refuses via
  `CanRewardQuest` + `SendQuestFailed`).
- `AUTO_REWARDED` pays at the request-reward
  completion transition (fork pays inside any
  `CompleteQuest`, silently); map-layer
  completions of such quests pay at their next
  request-reward.
- Dead-player quest-enders (`IsInvisibleForAlive`
  creatures, fork q3912 note) unmodeled: dead
  players are refused at request/choose.
- GO/item questgivers, gossip menus (HELLO goes
  straight to the quest list), quest sharing,
  timed-quest failure ticking, reward mail,
  script hooks, GO kill/objective credit
  (negative `ReqCreatureOrGOId`), raid-group
  kill-credit exclusion, reward-spell caster
  selection by spell-effect inspection (field
  based instead), `HasItemCount` in-trade
  exclusion.
- `Quests.LowLevelHideDiff` /
  `Quests.HighLevelHideDiff` config keys unread
  (fork defaults 4/7 compiled in); max player
  level hardcoded 60.

Probe dead-ends (for the record): the scripted
client must send **raw** 8-byte GUIDs for
`CMSG_SET_SELECTION` / `CMSG_ATTACKSWING` (packed
GUIDs parse as garbage targets and the player
never swings); kills persist `creature_respawn`
rows across bench restarts, so spawns swung at in
one run stay dead in the next; fixture health=181
means the probe character dies to quest mobs
unless healed pre-login; and a probe-session
disconnect-save overwrites any DB restore done
while the socket is still open (logout first).

## Slice 2 inventory — pet runtime (2026-10-20)

Fork surface (`Objects/Pet.{h,cpp}` 2378 lines,
`Handlers/PetHandler.cpp` 593):

- Types: `SUMMON_PET=0`, `HUNTER_PET=1`,
  `GUARDIAN_PET=2`, `MINI_PET=3`; controlled =
  summon/hunter. Save modes: DELETED=-1,
  AS_CURRENT=0, stable 1..MAX_PET_STABLES,
  NOT_IN_SLOT=100, REAGENTS=101.
- GUID quirk: this fork's
  `CreatureInfo::GetHighGuid()` is always
  `HIGHGUID_UNIT` pre-3.x, so a pet's wire GUID is
  `(UNIT, entry=petNumber, mapLocalLow)` — the pet
  number sits in the GUID entry slot while
  `OBJECT_FIELD_ENTRY` carries the template entry.
  `GenerateLocalLowGuid(HIGHGUID_PET)` only picks
  the lowguid counter space.
- `SPELL_EFFECT_SUMMON_PET` (56) →
  `Unit::EffectSummonPet(spellId, misc=entry,
  petLevel)`: unsummon old pet of different entry,
  `LoadPetFromDB(owner, entry)` first (player);
  entry 0 = hunter Call Pet (any current/
  non-stabled row, silent fail). New pet: global
  `GeneratePetNumber()` (counter seeded past DB
  max `character_pet.id`), spawn at
  `PET_FOLLOW_DIST=2.0` / `PET_FOLLOW_ANGLE=π/2`,
  SUMMON_PET, owner faction, PETEXPERIENCE=0,
  PETNEXTLEVELEXP=1000, CREATED_BY_SPELL,
  `InitStatsForLevel(level, owner)`,
  COMMAND_FOLLOW, REACT_DEFENSIVE (player owner),
  generated demon name (half-name tables, family
  fallback), `InitPetCreateSpells`
  (`petcreateinfo_spell` + passives + pet auras +
  training-point debit), full health/mana,
  immediate `SavePetToDB(PET_SAVE_AS_CURRENT)` +
  `PetSpellInitialize`.
- `LoadPetFromDB` (login `Player::LoadPet` with
  current=true; spell with entry/number): row =
  23 columns (id … pet_type); temporary summons
  (duration>0) never load as current; summon pets
  re-level to owner, saved health/mana restored
  (full when called non-current), slot promoted
  to 0 with the two UPDATEs, action bar +
  teach-spell strings parsed, auras/spells/
  cooldowns from pet_aura/pet_spell/
  pet_spell_cooldown, dead-on-load when saved
  health is 0 (except fresh summon-pet call).
- `SavePetToDB(mode)`: controlled + player owner
  only; dead current → NOT_IN_SLOT; body is
  DELETE(id) [ + slot-dedupe UPDATE ] [ + hunter
  other-pet DELETE ] + 23-column INSERT (note
  double space after `` `entry`, `` in the column
  list), health clamped to ≥1, action-bar and
  teach-spell data as space-joined strings with
  trailing spaces. Callers: summon, learn pet
  spell, tame, rename-adjacent paths,
  `Unsummon(mode)` (logout `RemovePet`,
  dismiss/abandon, stable), periodic player save
  (`Player.cpp:16656` saves AS_CURRENT inside
  SaveToDB when pet present — check at wiring).
- `GivePetXP`: HUNTER_PET only, alive, level <
  min(60, owner level); curve
  `GetXPForPetLevel = GetXPForLevel/4`;
  `GivePetLevel` re-inits stats and adds
  (loyalty−1) TP. Fed from
  `RewardSinglePlayerAtKill` (only when owner XP
  > 0, i.e. mob not grey to owner) with
  `XP::Gain(pet, victim)`, and from the group
  reward path analogously.
- `SPELL_EFFECT_LEARN_PET_SPELL` (57): pet alive,
  `CanLearnPetSpell` (family/TTP gates), TP debit
  (`GetTPForSpell` via SkillLineAbility
  reqtrainpoints), learn, save, re-init spells.
- `CMSG_PET_ACTION`: petGuid+data+targetGuid;
  ACT_COMMAND 0x07 → `HandlePetCommand`
  (stay/follow/attack/abandon), ACT_REACTION 0x06
  (passive also interrupts+AttackStop), spell
  flags 0x81/0x01/0xC1 → known+ready checks,
  `SMSG_PET_CAST_FAILED` on failure, cast,
  hunter `CheckLearning`. Plus handlers:
  PET_ABANDON (hunter → DELETED, else
  NOT_IN_SLOT), PET_RENAME, PET_NAME_QUERY,
  PET_SET_ACTION, PET_CAST_SPELL (item-cast
  variant), PET_STOP_ATTACK, PET_CANCEL_AURA,
  PET_SPELL_AUTOCAST, PET_UNLEARN.
- `SMSG_PET_SPELLS` (`PetSpellInitialize`):
  guid, u32 0, u8 react, u8 command, u8 0,
  u8 enabled-flag, 10 action-bar u32s
  (`MAKE_UNIT_ACTION_BUTTON`), u8 spell count,
  spell buttons (permanent pets only), cooldown
  block. `RemovePetActionBar`: guid 0 only.
  Default bar (`InitPetActionBar`): attack/
  follow/stay commands, 4 disabled spell slots,
  aggressive/defensive/passive reactions.

Rust today:

- No pet state anywhere: no owner/charm fields
  set on creatures, `character_pet` read only by
  the char-enum join, no pet save. Effect 56/57
  fall into the unhandled arm; temp summons
  (28/41/42) exist as plain creatures tracked in
  `Map.summons` with lifecycle timers.
- Update fields all exist in the protocol table
  (CHARM 6, SUMMON 8, CHARMEDBY 10, SUMMONEDBY
  12, CREATEDBY 14, PETNUMBER 139,
  PET_NAME_TIMESTAMP 140, PETEXPERIENCE 141,
  PETNEXTLEVELEXP 142, CREATED_BY_SPELL 146).
- `Creature::create` hardcodes `HighGuid::UNIT`
  with `first_creature_id` as GUID entry — pet
  spawn fits by passing petNumber there plus a
  level-reselect (`select_level`) at the pet
  level; pet-specific stat overrides need a new
  `pet_levelstats` world store (bench: 483 rows /
  8 entries) and `petcreateinfo_spell` (7 rows).
- AI is player-centric (proximity aggro targets
  players only; `pet_owner_to_register` threat
  hook is a stub). Chase generator exists (no
  Follow generator); pet follow = Chase(owner,
  2.0, π/2) as first slice, divergence recorded
  if spline semantics differ.
- Architecture fit: pet rows load with the
  character (DeferredLoad sibling), live pet
  state map-side next to `summons`, pet-number
  allocation via an actor-seeded shared counter
  (fork `m_NextPetNumber`), saves leave the map
  as `MapEvent` snapshots consumed by the world
  actor's DB lane (summon/learn/unsummon/logout
  trigger points). Pet handlers join the world
  actor packet buckets like the quest handlers.
- Bench fixture guid 1 is a level-19 warrior
  with zero pet rows and no pet spells learned;
  the live probe seeds Summon Imp (688, entry
  416) temporarily and restores. Hunter-pet XP
  proof needs a seeded HUNTER_PET row + Call
  Pet (883) or direct row-driven load.

Slice split:

- **2a** — world stores (`pet_levelstats`,
  `petcreateinfo_spell`, pet name halves), DB
  pet row load/save/delete with fork statement
  texts, `Creature` pet spawn + map `PetState`,
  SUMMON_PET new-pet path (warlock demons),
  follow + defensive/attack combat, dismiss via
  PET_ABANDON, PET_ACTION command/react arms,
  SMSG_PET_SPELLS, save-on-summon/logout.
- **2b** — LoadPetFromDB paths (login current
  pet + Call Pet), hunter pet XP + level-ups,
  LEARN_PET_SPELL, rename/name-query/set-action/
  stop-attack/cast-spell handlers, pet spell
  casting from the bar, save differential proof.

Out of slice (recorded): taming
(`TAME_CREATURE` channel + `CreateBaseAtCreature`),
stable master swap, mini pets, guardian-pet
typing of temp summons, pet auras/cooldowns
persistence beyond row core, happiness/loyalty
ticking, feeding, pet revive/mend spell arms
beyond generic heal targeting, possession/charm
reuse of the pet bar, group pet update flags.

## Slice 2 evidence — pet runtime (2026-10-21)

Live wire probe (`/tmp/pet_probe.py`, bench
mangosd on `bench-lan.conf`, guid 1 class/level
swapped per leg and restored):
`PET-PROBE-PASS`, cleanup `pet_rows=0
spell688=0`.

- Warlock leg (Summon Imp 688, entry 416):
  summon creates the row (`id=1 entry=416 slot=0
  created_by_spell=688 level=19 react=1` with a
  rolled `pet_name`); `SMSG_PET_SPELLS` carries
  a premake GUID (`0xf130…`, pet number in the
  GUID entry slot), react/command 1/1, the
  default bar, and Firebolt 3110 as
  `0x81000c26`; abandon writes slot 100
  (`PET_SAVE_NOT_IN_SLOT`); resummon reloads the
  same pet number instead of creating a new pet;
  logout writes slot 0; the next login loads the
  current pet with no cast.
- Hunter leg (seeded wolf row, entry 69):
  login load; rename persists (`Fang`,
  renamed=1); teach spell 2817 lands 2816 on
  the bar and in `pet_spell`; owner kill with
  the pet on passive persists pet `xp=66
  level=12 slot=0` after logout. The 66 matches
  the fork formula independently:
  `creature_xp_gain(12, 9)` = (12×5+45) ×
  ((8+9−12)/8) = 65.6 → 66.
- Fixture verified after the probe
  (`live_character_load` + `live_char_enum`
  oracle tests pass; see drift note below);
  `TWO-CLIENT-SMOKE-PASS`; full CI green
  (`CI_EXIT=0`); bench restarted on the rebuilt
  binary and re-smoked.

Save differential on `character_pet` /
`pet_spell` (vs fork `Pet::_SaveToDb` in
`vmangos/src/game/Objects/Pet.cpp`): statement
texts and bind order transcribed from the fork,
including the double space after `` `entry`, ``,
current-health clamped to ≥1, and trailing-space
action-bar/teach-spell string encodings; saves
leave the map as `MapEvent::SavePet/DeletePet/
PromotePet/RenamePet` snapshots consumed by the
world actor DB lane at the fork trigger points
(summon, learn, unsummon, logout, teleport).

Fork-exactness points landed with the probe:

- Pet GUIDs are premake `HIGHGUID_UNIT` with the
  pet number in the entry slot and the template
  entry in `OBJECT_FIELD_ENTRY` (the fork's
  pre-3.x quirk); pet numbers come from a shared
  counter seeded `MAX(id)+1`.
- Pet rows load with the character (deferred
  load); `SUMMON_PET` and Call Pet replicate the
  fork's lazy row selection over the loaded rows
  because the map cannot read the character DB
  at cast time.
- Solo hunter pet XP is `creature_xp_gain(pet
  level, mob)` gated on owner XP > 0; group pet
  XP mirrors the owner's share when the pet is
  at or below the group's not-gray max level.
  Pet kill credit (XP and quest) attributes to
  the owner; pet corpses never respawn.
- `TARGET_LOCATION_UNIT_MINION_POSITION` (32)
  joins the caster-targeted target arm (Summon
  Imp failed `FAILED_UNKNOWN` without it).

Recorded deviations / unmodeled remainder:

- The inventory's out-of-slice list stands
  (taming, stables, mini pets, guardian typing,
  pet aura/cooldown persistence, happiness/
  loyalty ticking and feeding, possession).
  Additionally: `LearnPetPassives` /
  `CastPetAuras` unmodeled (no pet-family spell
  store); training-point costs read 0 (no
  SkillLineAbility store, balance-only effect);
  family-name fallback is the template name;
  rename validation is a subset of the fork's;
  far teleports reload the pet from its row
  instead of transferring the live creature;
  defensive discovery sees melee attackers only;
  pet talk unmodeled.
- Follow is Chase(owner, 2.0, π/2) with no
  Follow generator; during the probe the pet's
  chase emitted `MovementInform type=6 point=1`
  continuously in `Server.log` (65k+ lines) —
  recorded as a spline-semantics divergence
  candidate (arrival re-inform loop), behavior
  on the wire was correct.
- Unproven live: pet level-up, Call Pet (883)
  cast path, pet bar spell cast, invalid-name
  rename packet, dead-pet load, group pet XP.
  Handler/unit coverage exists for these arms;
  the wire probe covered the paths above only.

Probe dead-ends (for the record): the kill leg
failed for several runs because the Slice 1
troggs are level 8–9 — grey to a level-19 owner
(`gray_level(19)=13`), so owner XP is 0, no
`SMSG_LOG_XPGAIN` is sent, and solo pet XP is
correctly skipped by the fork gate; the probe
now seeds owner 13 / pet 12 for that leg. The
same grey trap makes "no XP packet" a useless
kill signal at high level differences. Fixture
drift after wire probes is wider than the probe
backups: ticked aura durations, a saved trogg
debuff (3148), debug-script position seeding,
and the enum-only `zone` column (40) all needed
`/tmp/restore_oracle_fixture.py` + a `zone=40`
update before the oracle tests passed. The
two-client bot fixtures also drift 4 yd per
smoke run (relay moves persist on logout) and
needed a position reset once apart beyond the
25 yd say range.

## Slice 3 inventory — item-use + inventory grant (2026-10-21)

Fork oracle note: `/tmp/azeroth-prod/vmangos/`
is an incomplete copy (no `Handlers/`, empty
`Spells/`); the repo fork at `vmangos/src/game/`
is the usable semantic oracle, as in Slice 2.

Fork paths (all read in full):

- `WorldSession::HandleUseItemOpcode`
  (`Handlers/SpellHandler.cpp:36`): reads
  bag/slot/spellSlot, resolves `GetItemByPos`,
  gates proto presence, `spellSlot < 5` +
  `SpellId != 0` + trigger `ON_USE`,
  equip-required classes must be equipped,
  `CanUseItem`, in-trade refusal, non-combat
  spell refusal in combat, bind-on-use binding,
  `SpellCastTargets::ReadForCaster`,
  `IsTargetValidForItemUse`, shapeshift gate;
  failures send `SendEquipError`
  (`SMSG_INVENTORY_CHANGE_FAILURE`) or a cast
  result. Then `Player::CastItemUseSpell`
  (`Player.cpp:7451`) casts **every** `ON_USE`
  spell on the prototype, first non-triggered,
  the rest triggered, each with
  `SetCastItem(item)`.
- Cast-item semantics in `Spell`
  (`Spells/Spell.cpp`): `CalculatePowerCost`
  returns 0 with a cast item and `TakePower`
  early-returns (item casts are free);
  `SMSG_SPELL_START`/`GO` write the item GUID
  as the first packed GUID (`WriteGuidHelper`
  at 4611/4645); `SendSpellCooldown` (3798)
  calls `AddCooldown(spell, castItem proto,
  cooldown-on-event)`; `TakeCastItem` (5166,
  invoked in `cast()` after `SendSpellGo`)
  walks all prototype spells with charges:
  negative `SpellCharges` marks the item
  expendable, each use moves charges one step
  toward 0 (written back only when
  `Stackable < 2`), and an expendable item at
  0 charges is destroyed (`DestroyItemCount`
  1). Triggered casts skip `TakeCastItem`
  (except trade-item targets).
- `Player::AddCooldown` (`Player.cpp:21871`):
  item spell rows override category
  (`SpellCategory` when nonzero), spell
  cooldown (`SpellCooldown >= 0`), and
  category cooldown
  (`SpellCategoryCooldown >= 0`); the created
  cooldown records the item id.
  `SPELLMOD_COOLDOWN` applies to the spell
  recovery if nonzero, else the category
  recovery. `SpellCaster::IsSpellReady` uses
  the same item category override at the gate.
  Permanent (cooldown-on-event) holds convert
  at aura fade with an `SMSG_COOLDOWN_EVENT`.
  `_LoadSpellCooldowns`/`_SaveSpellCooldowns`
  persist `(spell, item, spell_expire,
  category, category_expire)` rows.
- `Spell::DoCreateItem`
  (`Spells/SpellEffects.cpp:1820`, effect 24
  via `EffectCreateItem`): player victims
  only; count = effect damage clamped to
  `[1, proto.Stackable]`; `CanStoreNewItem`
  with full/unique overflow reduced silently
  by `noSpace`, other errors sent as equip
  errors; `StoreNewItem` (stack-merging),
  signature items get `ITEM_FIELD_CREATOR`,
  `SendNewItem` (`SMSG_ITEM_PUSH_RESULT`:
  player guid, received, created, showInChat,
  bag slot u8, slot or -1 when stacked, entry,
  suffix factor, random property, count —
  1.10.2+ fields), then `UpdateCraftSkill`.
  Battleground-mark mail overflow is Tier 3.

Rust today:

- `CMSG_USE_ITEM` (171) exists in the opcode
  table (`PacketProcessing::Map`,
  `HandleUseItemOpcode`) but has no dispatch
  in `world_actor.rs` — packets are dropped.
  Item prototypes load in full, including the
  five `_ItemSpell` rows (trigger, charges,
  cooldown, category, category cooldown).
- Effect 24 has no constant in
  `mangos-spells` defines and falls into the
  silent `_ => {}` arm of `apply_effects`
  (`mangos-map/src/spells.rs:1692`). The
  SUMMON_PET arm beside it is the exemplar for
  a new effect arm.
- Cooldowns are otherwise complete: map-side
  `CooldownEntry { expiry, item_id, permanent }`
  keyed spell/category, `has_cooldown`/
  `add_cooldowns`, login seeding from
  `character_spell_cooldown` rows
  (`seed_cooldowns`, session
  `initial_spell_cooldowns`), and logout
  snapshot (`player_cooldown_rows`, which
  already carries `item_id` — its comment
  anticipates this slice). Gaps: no item
  overrides at add/gate, `item_id` always 0
  for new entries, and the snapshot derives
  category from the spell, so an item category
  override would save the wrong category.
  `SMSG_SPELL_COOLDOWN`/`COOLDOWN_EVENT`/
  `ITEM_PUSH_RESULT` opcodes exist; none are
  emitted anywhere yet.
- Inventory ownership: the map-side `Player`
  keeps the `saved.inventory` row baseline;
  the world actor writes DB rows and mutates
  the baseline in lockstep (quest grant,
  `world_actor.rs:8220` free-slot search
  backpack → equipped bags, no stack merge).
  `mangos-db::quest_reward` already provides
  `create_reward_item` / `set_item_count`
  (guid allocation via `petition::next_item_guid`)
  and says in its module doc that Slice 3
  builds the live grant path on it. The map
  itself has no DB access, so map-originated
  grants/consumptions cross as `MapEvent`s
  (Slice 2 `SavePet` precedent).
- Bench fixture guid 1 already carries a
  Lesser Healing Potion (858: spell 440,
  charges −1, cooldown/category 120000/4) at
  bag 0 slot 32 — the probe needs no seeded
  consumable, only a taught conjure spell.

Slice split:

- **3a** — `CREATE_ITEM` effect: defines
  constant, effect arm (player victim, count
  clamp, proto check + equip error), new
  `MapEvent::CreateItem`, actor-side
  store-new-item (stack merge, `max_count`,
  free-slot search, silent overflow
  reduction), `SendNewItem` packet, baseline
  sync; creator guid for signature items.
- **3b** — item-use casting: `CMSG_USE_ITEM`
  bucket + handler with fork gates (subset
  recorded), `CastOptions.cast_item` through
  the cast pipeline (skip-known, zero power,
  item category cooldown gate, item GUID in
  SPELL_START/GO), finish-time item cooldown
  overrides + `CooldownEntry.category`, and
  `TakeCastItem` charge consumption via
  actor events (charges text write, expendable
  destroy, bind-on-use flags).

Out of slice (recorded): `CMSG_OPEN_ITEM` /
`READ_ITEM` (loot windows, page text), item
equip/swap handlers beyond what exists,
proc-on-equip item spells (`ON_EQUIP`/
`ON_PROC` triggers beyond current proc arms),
`UpdateCraftSkill` profession skill-ups,
random property generation (writes 0 as the
quest path does), shapeshift gate (no forms
runtime until Slice 7 display work), trade
state, BG mark mail, `CanUseItem` arms beyond
level/skill/reputation basics, and the
pre-existing SPELL_START second-GUID
divergence (Rust writes the target where the
fork writes the caster unit).

## Slice 3 evidence — item-use + inventory grant (2026-10-21)

Live wire probe (`/tmp/item_probe.py`, bench
mangosd on `bench-lan.conf`, guid 1 class/level
swapped per leg and restored):
`ITEM-PROBE-PASS`, cleanup `potion_count=1
conjured_rows=0 cooldown_rows=0`.

- Conjure leg (Conjure Water 5504 at level 5):
  first cast sends `SMSG_ITEM_PUSH_RESULT`
  (received=1 created=1, entry 5350, count 4,
  real slot) and writes the `item_instance` +
  `character_inventory` rows; second cast
  merges the same stack (count 8, slot field
  `0xFFFFFFFF`). Count 4 matches the fork value
  formula over the bench `spell_template` row:
  base 1 + (min(5,13)−4)×2 real-points/level
  + 1 base dice. At level 19 the same cast
  produced a full 20 stack (formula value 20
  clamped to `Stackable`), also fork-correct.
- Potion leg (Lesser Healing Potion 858 at bag
  255 slot 32, stack inflated to 2):
  `CMSG_USE_ITEM` casts 440 with the item GUID
  (`0x4000…`) as the first packed GUID of
  `SMSG_SPELL_GO`; the stack drops to 1
  (expendable charge consumption persisted);
  an immediate second use fails
  `CAST_RESULT 440/60` (`FAILED_NOT_READY`);
  after logout `character_spell_cooldown`
  holds `440, item 858, category 4`; after
  relog the seeded cooldown still blocks.
- Fixture verified after the probe
  (`live_character_load` + `live_char_enum`
  oracle tests pass after
  `restore_oracle_fixture.py` + `zone=40`);
  `TWO-CLIENT-SMOKE-PASS`; full CI green
  (`CI_EXIT=0`, including 4 new store-plan unit
  tests); bench restarted on the rebuilt binary
  and re-smoked.

Fork-exactness points landed:

- `CREATE_ITEM` (24): player victims only,
  count clamped to `[1, Stackable]`, unknown
  prototype fails `EQUIP_ERR_ITEM_NOT_FOUND`
  (23); the grant crosses to the world actor as
  `MapEvent::CreateItem` and lands through a
  `CanStoreNewItem` subset (unique `max_count`
  cap, stack merges, then free slots — backpack
  before equipped bags; overflow reduced
  silently per `noSpace`); `SendNewItem` writes
  the real slot only when the first destination
  holds the whole grant, else −1; signature
  items (`HasSignature`) get `creator_guid`.
- Item casts: free power (`CalculatePowerCost`
  0 / `TakePower` skip), known-spell check
  skipped, GCD kept, item GUID as first packed
  GUID of SPELL_START/GO, and `TakeCastItem`
  after SPELL_GO (expendable walk with the
  fork's per-spell `withoutCharges` overwrite,
  charges written back only when `Stackable < 2`,
  triggered casts skip consumption).
- Item cooldowns: `_ItemSpell` overrides at
  gate and add (`SpellCategory` /
  `SpellCooldown` / `SpellCategoryCooldown`),
  fork's either-or `SPELLMOD_COOLDOWN` arm,
  effective category + item id recorded on the
  spell-key entry so `_SaveSpellCooldowns`
  round-trips item rows, and permanent-hold
  conversion re-picks item durations.
- `HandleUseItemOpcode` gates ported over the
  row baseline (proto/spell-slot/trigger,
  equip-required, `CanUseItem` subset with
  level error carrying the required level,
  non-combat refusal at result 60) with
  `SendEquipError` packet shapes; bind-on-use
  family sets `ITEM_DYNFLAG_BOUND` in baseline
  and DB before the cast.

Bug found and fixed by the probe (pre-existing,
not Slice 3 code): map-side cooldown entries
survived logout, so a later login with fewer DB
rows (or rows deleted) still saw the previous
session's cooldowns. The fork's cooldown map
lives on the Player and is reloaded from DB at
login; `seed_cooldowns` now drops the player's
existing map entries before seeding.

Recorded deviations / unmodeled remainder:

- The inventory's out-of-slice list stands
  (open/read item, on-equip/on-proc item
  triggers beyond existing proc arms,
  `UpdateCraftSkill`, random property written
  0, shapeshift gate, trade state, BG mark
  mail, SPELL_START second-GUID divergence).
  Additionally: `CanUseItem` honor-rank and
  proficiency-skill arms, soulbound-to-other,
  skill gate on raw skill value (no bonus),
  `IsTargetValidForItemUse` treated as no
  requirements (no `item_required_target`
  store; 37 bench rows over-permitted),
  item-use aura interrupts (no `AuraInterruptFlags`
  parse), item/GO/location cast targets in
  `CMSG_USE_ITEM`, `BagFamily` specialization
  and keyring priority in the store plan,
  unique-equipped checks, and quest rewards
  still use the Slice 1 grant (no stack merge,
  no live push packet) rather than the new
  store path.
- Unproven live: equip-error packets, bind-on-use
  flag write, signature creator stamp,
  multi-spell on-use items, channeled item
  spells, non-combat refusal, and reputation /
  skill / level gates. Unit coverage covers the
  store plan; the wire probe covered the paths
  above only.

Probe dead-ends (for the record): spells taught
after login are not in the loaded spellbook
(conjure legs failed until taught pre-login);
conjure counts scale with caster level, so a
level-19 expectation of "2 waters" is wrong
(20 at the cap); the cooldown query column is
`item_id`, not `item` (a silent empty result
cost one diagnosis cycle); and the stale-map-
cooldown bug above surfaced only when a debug
script deleted DB rows between probe runs.

## Slice 4 inventory — dynamic objects (2026-10-21)

Fork paths (all read in full):

- `Spell::EffectPersistentAA`
  (`SpellEffects.cpp:1924`): finds the caster's
  existing dynobj for the spell
  (`GetDynObject` by spell id) and refreshes +
  relocates it (alive reset, duration reset,
  position update values), else
  `CreateDynObject` → `DynamicObject::Create`
  (local low guid `HIGHGUID_DYNAMICOBJECT`,
  values: entry = spell id, caster, bytes =
  `AREA_SPELL`, spell id, radius =
  `ComputeEffectRadius`, position, facing),
  then `SetDuration(m_duration)`,
  `AddToWorld`, `AddDynObject` on the caster.
  Spawn position: unit target if set, else the
  cast destination.
- `DynamicObject` (`Objects/DynamicObject.cpp`):
  `Update` deletes when the caster is gone,
  when a channeled AoE's channel object no
  longer matches, or when the alive countdown
  expires; `Delay` raises the alive countdown
  back to full duration when delayed; ticking
  is `MaNGOS::DynamicObjectUpdater`
  (`Maps/GridNotifiersImpl.h:119`): every map
  tick it visits units in search cells, gates
  on `IsWithinDistInMap(radius)`,
  `NeedsRefresh` (2 s per-target refresh
  throttle over the affected map),
  immune-to-AoE / evade / GM flags, valid
  attack-or-assist, player-caster LoS, the
  unflagged-PvP restriction, and spell
  immunities; on apply it pulls combat/threat
  for harmful spells, adds/refreshes the
  `PersistentAreaAura` on the (spell, caster)
  holder, resets its duration to the dynobj's
  remaining alive time, and stamps the
  affected map. `Delete` sends the despawn
  anim and removes caster linkage.
- `PersistentAreaAura::Update`
  (`SpellAuras.cpp:855`): each aura tick, the
  effect removes itself (`AURA_REMOVE_BY_RANGE`,
  and `RemoveAffected` so re-entry reapplies)
  when the dynobj is gone or the target is
  outside its radius — unless the spell has
  `ATTR_EX3_NO_AVOIDANCE` (explosive traps).
- Ground targeting wire format
  (`SpellCastTargets::Read` in `Spell.cpp`):
  u16 mask, then unit/GO/corpse packed GUIDs,
  item packed GUID, source xyz (`0x20`),
  dest xyz (`0x40`), string; mask 0 fills the
  destination from the caster. Bench anchors:
  Flamestrike 2120 (effect 2 target 16
  AoE-at-dest + effect 27 periodic damage
  target 28 AoE-at-dynobj, amplitude 2000),
  Rain of Fire 5740 (channeled effect 27).

Rust today:

- `SpellCastTargets` equivalent is two-variant
  (`CastTarget::SelfCast | Unit`); actor parsers
  handle masks 0 and 0x0002 only, so ground
  spells fail `check_target`
  (`FAILED_UNKNOWN`) — Flamestrike cannot cast.
  `check_target` already passes `LOCATION_*`
  modes as caster-targeted; modes 16/17/28/48
  are unhandled. Effect 27 has a defines
  constant but falls into the silent default
  arm of `apply_effects`.
- All object plumbing exists but is unused for
  this kind: `TypeId::DynamicObject`,
  `HIGHGUID_DYNAMICOBJECT`, field flags and
  offsets (end 16), `ObjectBase`/`WorldObjectBase`
  shared with GameObjects, per-player visible
  sets with create/out-of-range in
  `update_visibility_of` (dynobj needs new arms
  in `is_visible`/`build_create_for` plus a cell
  set), and `summon_counter` for local low
  guids. `SMSG_GAMEOBJECT_DESPAWN_ANIM` (533)
  exists in the opcode table.
- Aura runtime is directly reusable:
  `apply_spell_aura` holder construction
  (periodic state, duration via
  `calculate_duration_ms` + `SPELLMOD_DURATION`),
  `add_aura_holder` refresh semantics, and
  `update_auras` ticking. Missing: a holder →
  dynobj link for range-removal, and the save
  filter must skip dynobj holders (fork does
  not persist persistent auras across logout).
- Radius/duration data present (`radii` table,
  `duration_index`); cast-time destination
  range gating exists for unit targets only.

Slice split (one implementation pass, no
lettered sub-slices needed beyond order):

- Location targets: `CastTarget::Dest` through
  the pipeline (actor targets parser shared by
  cast/item-use, range gate to destination,
  `check_target` location arms, SPELL_START/GO
  targets block writes dest flag + xyz).
- Dynobj runtime: `dynobjects.rs` store +
  entity values + cell/visibility arms,
  spawn/refresh from the effect arm, per-tick
  visit with the fork gate subset, holder
  apply/refresh with dynobj link, range-removal
  in `update_auras`, lifetime/channel/caster-gone
  deletion with despawn anim.

Out of slice (recorded): `Farsight` dynobjs
(portals/focus are other `DynObjectType`s made
elsewhere), unit-targeted moving dynobjs,
trap arming specifics beyond the generic path,
`ComputeEffectRadius` spellmod nuances beyond
the shared helper, the 2 s refresh boundary's
off-by-one-tick detail, PvP-flag and
immune-to-AoE gates where the underlying
player/creature state is unmodeled, and
3D-vs-2D distance choice (recorded at
implementation).

### Slice 4 result (2026-10-21, uncommitted)

Implemented as inventoried. Location targets:
`CastTarget::Dest(Position)` end-to-end — shared
actor targets parser (`SpellCastTargets::Read`
order: unit, GO, corpse, item, source, dest;
string targets rejected), destination range
gate and ground-enumeration victim search in
`check_target`/`collect_targets` (modes 15, 16,
30, 31 enumerate by radius and hostility at the
destination or caster; 28/29 enumerate nothing
at cast time), `targets_write` emits the `0x0040`
dest block, SPELL_START/GO second GUID is the
caster for destination casts.

Dynobj runtime (`mangos-map/src/dynobjects.rs`,
entity in `mangos-entities/src/dynobject.rs`):
`Map.dynobjects` + `CellObjects.dynobjects`,
create values fork-exact (entry = spell id,
`DYNAMICOBJECT_BYTES` = 1 AREA_SPELL, 1543
flare diameter quirk), spawn once per cast from
`finish_cast` (`EffectPersistentAA` runs on the
location target list, not per victim), immediate
affected pass at spawn then a 2 s throttled
updater pass, holder apply/refresh with the
dynobj link stamped and duration reset to the
object's remaining lifetime, lifetime countdown
with caster-death deletion, `SMSG_GAMEOBJECT_DESPAWN_ANIM`
(533, guid + 1) to viewers, visibility via the
object grey band + scan-due fanout on spawn.
`AuraHolder.dynobj` link added (not persisted);
`update_auras` runs a `PersistentAreaAura::Update`
pre-pass removing whole holders whose dynobject
is gone or whose target left the radius, and
reloaded persistent holders (no link) remove
themselves on the first tick, matching the
fork's null-dynobject behavior. Whole-holder
vs per-effect removal is a recorded divergence.
Distance checks are 2D (`dist2d`), matching the
fork's grid search convention used elsewhere.

Load-bearing fork data found during probing:
Flamestrike 2120 has two effects — SCHOOL_DAMAGE
at target 16 and PERSISTENT_AREA_AURA at target
28 whose **periodic damage aura rides on the
persistent effect itself** (`effectApplyAuraName2`
= PERIODIC_DAMAGE, amplitude 2000). The holder
builder in `apply_spell_aura` now includes
PERSISTENT_AREA_AURA effects (fork `CreateAura`
parity); without this the dynobj applied empty
holders and nothing ticked.

Not modeled / unproven-live (recorded):
single-target dynobj replacement in
`AddDynObject` (every cast spawns a fresh
object), channeled dynobj lifetime ending on
channel break (only caster death / expiry end
it), spellmod radius on the dynobj spawn
(`ComputeEffectRadius` spellmod half not
applied to `effect_radius`), LoS, PvP and
immune-to-AoE updater gates, out-of-range
holder removal latency vs the fork's per-tick
check (Rust checks every `update_auras`),
creature-caster dynobjs (path exists, unproven
live), and real-client visual confirmation of
the ground effect.

Evidence (bench `mangosd` rebuilt + restarted,
`PORT_OK` on 18099):

- `/tmp/dynobj_probe.py` → `DYNOBJ-PROBE-PASS`:
  mage-swapped guid 1 casts Flamestrike 2120
  (mask `0x0040` + xyz at own feet beside trogg
  spawn 4378); `SPELL_GO 2120` seen, 4 direct
  `SMSG_SPELLNONMELEEDAMAGELOG` hits, 12
  `SMSG_PERIODICAURALOG` ticks over the 8 s
  lifetime, `DESPAWN_ANIM guid=0xf100000848f00001`
  (high `0xF100`, entry 2120) at expiry. Fixture
  backed up and restored.
- Oracle differentials pass after fixture
  restore: `live_char_enum`
  (`char_enum_matches_python_oracle ... ok`) and
  `live_character_load`
  (`character_load_matches_oracle ... ok`).
- `two_client_smoke.py` → `TWO-CLIENT-SMOKE-PASS`
  (bot pair positions reset into say range
  first: known fixture drift, now 28 yd apart
  before reset).
- `./ci.sh` → `CI_EXIT=0` (`/tmp/ci_slice4.log`);
  new `dynobject::tests::create_writes_fork_fields`
  unit test covers the fork create fields.

Slice 4 closes Tier 1 item (4). Next: Slice 5
(cross-map transfer + `spell_target_position`).

## Slice 5 inventory (2026-10-21)

Fork semantics (`Spell::EffectTeleportUnits`
SpellEffects.cpp:1536, `EffectLeapForward`
:5187, `LoadSpellTargetPositions` SpellMgr.cpp:43,
`Unit::NearTeleportTo` Unit.cpp:9863,
`Player::TeleportTo` Player.cpp:1846):

- TELEPORT_UNITS (5) switches on
  `EffectImplicitTargetB`: 9 HOME_BIND → players
  only, `TeleportToHomebind`; 17 DATABASE (and 7
  script-AoE alias) → `spell_target_position`
  row, same-map `NearTeleportTo`, cross-map
  players-only `TeleportTo` (creatures silently
  skipped cross-map); 18 CASTER_DEST → the
  *caster* near-teleports to the unit target's
  position with the caster's orientation;
  default → client dest location required, near
  teleport there with the target's orientation.
  Taxi-flying targets are skipped.
- The store loads build-ranged rows and drops
  rows whose map is unknown, whose coords are
  all zero, whose spell does not exist, or whose
  spell lacks target 17 in A or B of any effect.
- Non-teleport spells with target-A 17 get their
  cast destination filled from the same store at
  target-map build (Spell.cpp:3048); TELEPORT_UNITS
  spells are explicitly excluded there (handled
  in the effect). This fill matters for
  dynobj/summon placement spells keyed to 17.
- Blink (1953) is LEAP (29), target A=1,
  target B=47/55 (FRONT / FRONT_LEAP build
  variants), radius index 9. Destination is
  filled at target-map build by
  `GetFirstCollisionPosition(pos, radius, angle)`
  over the directional family 41–44, 47–50, 55
  (angle offsets from facing: FRONT_LEFT and
  MINION_POSITION +0.25π, BACK_LEFT +0.75π,
  BACK +π, BACK_RIGHT +1.25π, FRONT_RIGHT
  +1.75π, LEFT +π/2, RIGHT −π/2, FRONT and
  FRONT_LEAP 0; radius index 0 forces radius 0),
  then `EffectLeapForward` near-teleports the
  target to the filled dest.
- Creature near teleport sends
  `SendTeleportToObservers` **twice** (before and
  after `CreatureRelocation`) with motion-master
  Interrupt/Reset around it; player near teleport
  goes through `Player::TeleportTo` same-map arm
  (controller ACK train).

Rust state: the expensive machinery already
exists — `teleport_player_in_map` (controller
`MSG_MOVE_TELEPORT_ACK` + observer
`MSG_MOVE_TELEPORT`), actor `teleport_player`
with `pending_teleports`/`finish_teleport`/
worldport-ACK completion and pet unsummon,
`relocate_unit`, `Map::ground_z`, player
`homebind.location`, `implicit_target_b` on
spell effects, build-filter loader conventions
(`? BETWEEN build_min AND build_max`). Missing:
`spell_target_position` store + boot load,
TELEPORT_UNITS and LEAP effect arms, creature
near-teleport observer packets, a map→actor
transfer event, Blink's collision dest fill
(vmaps are not loaded — height-stepping
approximation to record), and the target-17
dest fill for non-teleport spells (Slice 4's
`cast_dest` falls back to caster position;
filling from the store for 17-mode effects is
in scope here since the store now exists).

Out of slice (recorded): taxi-flight gate
(no taxi system), TELE_TO_* option side
effects beyond position (combat/pet handling
exists only where noted), `TeleportTo`
entry-check failures (instance/battleground
gates beyond `resolve_dest_key`), far-teleport
of creatures (fork also skips), vmap-exact
Blink collision, `TELEPORT_UNITS_FACE_CASTER`
(1537-style sibling effect if unreachable from
loaded templates — re-check at implementation).

### Slice 5 result (2026-10-21, uncommitted)

Implemented as inventoried:

- `mangos-world-data/src/spell_target_position.rs`:
  build-ranged loader with the fork's rejection
  rules (unknown map, all-zero coords, spell
  missing or lacking target 17 in A/B), wired
  into `WorldStores.spell_target_positions` and
  the boot load (validated against the loaded
  spell entries and map store).
- TELEPORT_UNITS (5) effect arm switched on
  implicit target B exactly as the fork:
  homebind (players only, from
  `Player.homebind`), database rows (17 and the
  7 script alias), caster-dest (18: the *caster*
  lands on the victim with the caster's
  orientation), and the client-dest default.
  Same-map destinations near-teleport in place;
  cross-map players cross to the actor as
  `MapEvent::TeleportPlayer` into the existing
  `teleport_player` far-transfer flow;
  cross-map creatures are skipped (fork parity).
- Creature near teleport
  (`teleport_creature_in_map`): `MSG_MOVE_TELEPORT`
  to observers before **and** after relocation,
  fork-exact; motion Interrupt/Reset collapses
  to a stop (recorded).
- LEAP (29) with the directional destination
  fill: angle offsets for the 41–44/47–50/55
  family from implicit target B (A fallback),
  radius-index-0 → radius 0, client dest wins
  when sent. `first_collision_dest` walks the
  navmesh path (`find_path_points(from, end,
  false, true)`) and takes the path end — the
  fork's `PathFinder` analogue; its vmap `LoS`
  trim is not modeled (recorded).
- Target-17 dest fill for non-teleport spells
  in `cast_dest` (dynobj/summon placement now
  honors database destinations on the same map).

Not modeled / unproven-live (recorded):
`TELEPORT_UNITS_FACE_CASTER` was not found in
loaded templates during implementation (no
arm); taxi gate; `TELE_TO_*` combat/transport
option side effects; creature-victim spell
teleports beyond the near arm (path exists,
unproven live); hearthstone cooldown-on-use
item spell path beyond the generic cast;
real-client visual confirmation of Blink.

Evidence (bench rebuilt + restarted, `PORT_OK`):

- `/tmp/teleport_probe.py` →
  `TELEPORT-PROBE-PASS`, three legs on guid 1
  (mage swap, fixture backed up/restored,
  homebind temporarily on map 1):
  Blink `dest=(-5704.58,-1600.29,383.20)`,
  exactly +20.00 along facing via
  `MSG_MOVE_TELEPORT_ACK`; Teleport: Stormwind
  `dest=(-9003.46,870.03,29.62)` — the
  `spell_target_position` row byte-for-byte (the
  cast needed the caster walked clear of a
  hostile Dun Morogh Mountaineer whose hits
  produced `SMSG_SPELL_DELAYED` pushback, also
  live evidence for the delay path);
  Hearthstone far transfer:
  `SMSG_TRANSFER_PENDING` + `SMSG_NEW_WORLD`
  map 1 at the homebind, world-entry update
  traffic after `MSG_MOVE_WORLDPORT_ACK`, and
  logout persistence `map=1` at the homebind
  coordinates in `characters.characters`.
- Oracle differentials pass after fixture
  restore (`live_char_enum`, `live_character_load`
  both ok); `two_client_smoke.py` →
  `TWO-CLIENT-SMOKE-PASS` (bot pair re-co-located
  first: fixture drift); `./ci.sh` →
  `CI_EXIT=0` (`/tmp/ci_slice5.log`), including
  the `WorldStores` literal updates across the
  live test harnesses.

Slice 5 closes Tier 1 item (5). Next: Slice 6
(knockback movement generator).

## Slice 6 inventory (2026-10-21)

Fork semantics (`Spell::EffectKnockBack`
SpellEffects.cpp:5368, `Unit::KnockBackFrom` /
`Unit::KnockBack` Unit.cpp:9991,
`MovementPacketSender::SendKnockBackToController/
ToObservers` MovementPacketSender.cpp:225,
`WorldSession::HandleMoveKnockBackAck`
MovementHandler.cpp:762):

- **Creature correction to the goal text**: this
  fork has **no creature knockback arc**.
  `Unit::KnockBack` opens with "Effect properly
  implemented only for players" and returns
  unless `IsMovedByPlayer()`; there is no
  `KnockbackMovementGenerator` anywhere in the
  tree. Fork-exact slice 6 is therefore the
  player path only, and creature victims are a
  deliberate no-op (they keep the stun/root and
  spline-finalized gates' outer shape, which is
  unobservable). This overrides the goal's
  "spline arc for creatures" phrasing; recorded
  here rather than silently inventing one.
- Effect: remove Dream Fog Sleep (24778), then
  `KnockBackFrom(caster, misc/10, damage/10)`
  where `damage` is the calculated effect value.
  `EffectPlayerPull` (124) is the negative-speed
  sibling through the same helper.
- `KnockBackFrom`: stunned/rooted → no-op;
  angle = away from source (self-source:
  orientation + π); players get `SetLaunched(true)`
  before `KnockBack`, which additionally
  requires a finalized spline, interrupts
  non-melee spells, sends `SMSG_MOVE_KNOCK_BACK`
  (packed guid, movement counter, vcos, vsin,
  speedXY, **−speedZ**), pushes a `KNOCK_BACK`
  pending movement change carrying the *wire*
  values, sets jump initial speed, and notifies
  the anticheat (`OnKnockBack`: m_knockBack=true,
  jump counters reset).
- Ack (`CMSG_MOVE_KNOCK_BACK_ACK`): guid +
  counter + `MovementInfo`; the pending change
  must match counter, type, and all four jump
  fields within 0.01 (`FindPendingMovementKnockbackChange`)
  or the packet drops with a wrong-ack anticheat
  hit (only when counter is 0 or in the future).
  Spline-done pending or being-teleported →
  silent return. Then flag/position anticheat
  tests, `SetFallInformation(0)`, relocation,
  and `MSG_MOVE_KNOCK_BACK` to observers echoing
  the ack's jump fields.
- Anticheat launched state: `Player::m_launched`
  makes `CheckTeleport` pass wholesale; cleared
  on fall-end opcodes (`MSG_MOVE_FALL_LAND`,
  `MSG_MOVE_START_SWIM`) in the generic movement
  handler. `m_knockBack` (anticheat-side) skips
  jump-speed-change / wall-climb / speed-hack
  checks and clears on fall-end or ROOT.

Rust state: `MOVEMENT_CHANGE_KNOCK_BACK` (12)
exists but `PendingMovementChange` has no
knockback payload and no matcher; the ack
handler's knockback arm is a stub that always
wrong-acks ("No senders yet"); anticheat already
carries `knock_back` reads/clears and
`jump_initial_speed`, but nothing sets them and
there is no `launched` state; `MovementInfo.jump`
fields parse/write already. Missing: pending
knockback info + matcher, map-side
`knock_back_from` + controller sender + a
map→actor anticheat notification, the real ack
arm + observer broadcast, anticheat `launched`
(+ teleport-check exemption + fall-end clear),
and the KNOCK_BACK (98) / PLAYER_PULL (124)
effect arms.

Probe reachability: spell 25060 (effect 98,
target 22 src-location, misc 25) taught to the
probe player self-casts into the full player
path (victim == caster); spell 514 (effect 98,
target 6) covers the creature no-op.

### Slice 6 result (2026-10-25, uncommitted)

Implemented as inventoried, fork-exact on the
creature question: no creature arc (this fork
launches player-moved units only; the goal
text's creature spline does not exist upstream
of us and was not invented).

- `PendingMovementChange` carries
  `KnockbackInfo` (wire values, speedZ negated);
  `find_pending_knockback_change` mirrors the
  fork's 0.01 four-field tolerance with
  erase-on-hit (unit-tested).
- Map `knock_back_from`: stun/root and
  spline-finalized gates, non-melee cast
  interrupt for **all** unit kinds (fork order),
  then the player branch — counter-allocated
  pending change, `SMSG_MOVE_KNOCK_BACK` to the
  controller, `MapEvent::KnockBack` so the actor
  marks the anticheat launched and seeds jump
  speed. KNOCK_BACK (98) and PLAYER_PULL (124)
  effect arms share it (pull = distance-clamped
  negative horizontal, misc vertical;
  Dream Fog Sleep dispel on the knockback arm).
- Ack arm rewritten to `HandleMoveKnockBackAck`:
  pending match (wrong-ack anticheat hit on
  mismatch, sibling-arm precedent), pending
  spline-done / being-teleported silent drops,
  relocation from the ack info, then
  `MSG_MOVE_KNOCK_BACK` to observers. The fork's
  flag/position anticheat tests *on the ack*
  and `SetFallInformation(0)` are not run
  (recorded; sibling ack arms do neither).
- Anticheat: `launched` session state with
  `on_knock_back` (knock_back + launched + jump
  counter reset + jump initial speed),
  `CheckTeleport` wholesale exemption while
  launched or being teleported (via
  `MoveFacts.launched`), cleared on fall-end
  opcodes (FALL_LAND/START_SWIM).

Not modeled / unproven-live (recorded):
observer packet observed only from the mover's
socket in probes (two-client knockback
visibility unproven); wrong-ack counter
nuance (fork only counts when the pending list
is empty and counter is 0-or-future; Rust
counts every mismatch like its sibling arms);
possessed-unit knockback (no possession);
real-client arc visual.

Evidence (bench rebuilt + restarted after
rebuilding the `/tmp` bench conf and mysql
helper lost to a tmp cleanup; `PORT_OK`):

- `/tmp/knockback_probe.py` →
  `KNOCKBACK-PROBE-PASS`: spell 514 at a trogg —
  `go514=1 knockback_packets=0` (creature
  no-op); spell 25060 self-cast —
  `KNOCK guid=0x1 counter=0 vcos=-1.000
  vsin=-0.000 xy=2.50 z=-23.10` (facing +x,
  backward launch; z from dice over base 218),
  ack with matching jump fields accepted, and
  post-land Blink `dest=(-5713.58,...)` proves
  the server relocated from the ack (landed
  −5733.58 + 20).
- `knockback_pending_matches_within_tolerance_and_erases`
  unit test ok; oracle differentials ok
  (`live_char_enum`, `live_character_load` —
  the latter needed the recreated
  `/tmp/restore_oracle_fixture.py` extended to
  rewrite guid 1's characters row, quest
  timers, and skills from the oracle after
  probe drift); `two_client_smoke.py` →
  `TWO-CLIENT-SMOKE-PASS`; `./ci.sh` →
  `CI_EXIT=0` (`/tmp/ci_slice6.log`).

Slice 6 closes Tier 1 item (6). Next: Slice 7
(mount/transform display handling).

## Slice 7 inventory — mount/transform display

Fork: HandleAuraMounted (SpellAuras.cpp:2151) Real-only: apply →
misc=creature entry → Creature::ChooseDisplayId → random-gender
addon 50% other-gender swap (ObjectMgr.cpp:1776) → Mount(display,
spellId); remove → Unmount(true). Unit::Mount (Unit.cpp:5957):
UNIT_FIELD_MOUNTDISPLAYID write, RemoveAurasWithInterruptFlags(
MOUNT_CANCELS=0x20000)/channel-interrupt, creature speed update;
Unmount mirrors with DISMOUNT_CANCELS=0x40. Player::Mount
(Player.cpp:18023) adds MOUNTRESULT packets, pet unsummon/disable,
ResetExtraAttacks. UpdateSpeed MOVE_RUN mounted arm (Unit.cpp:7117):
main = max-positive MOD_INCREASE_MOUNTED_SPEED(32), stack =
multiplier MOD_MOUNTED_SPEED_ALWAYS(130), non-stack =
MOD_MOUNTED_SPEED_NOT_STACK(172).
HandleAuraTransform (SpellAuras.cpp:2533): precedence — new
transform applies iff none current, new negative, or current
positive; misc==0 = Orb of Deception race/gender table; misc!=0 =
template ChooseDisplayId (unknown → UNIT_DISPLAY_ID_BOX=4);
SetDisplayId + SetTransformScale(amount%) + m_transformSpell;
removal of current restores native, re-equips creatures, re-applies
remaining transforms preferring negative.

Rust surfaces: UnitBase::set_display_id (UpdateModelData baked),
WorldStores.model_for_display/scale_for_display, private
choose_display (entities creature.rs:1128) needs pub wrapper,
DisplayInfoAddons.creature (gender/other-gender), creature
load_equipment + map creature_load_equipment, refresh_cc_flags
recompute idiom to mirror, refresh_speed lacks mounted arm.
Constants missing: MOUNTED=78, MOD_INCREASE_MOUNTED_SPEED=32,
MOD_MOUNTED_SPEED_ALWAYS=130, MOD_MOUNTED_SPEED_NOT_STACK=172,
MOUNT_CANCELS/DISMOUNT_CANCELS. FOUND BUG: Rust
MOD_SPEED_NOT_STACK=130 is fork's MOD_MOUNTED_SPEED_ALWAYS; fork
MOD_SPEED_NOT_STACK=171 — constant fix included in this slice.
No shapeshift display runtime (transform fallback to shapeshift
display moot); pet SetEnabled unmodeled; ResetExtraAttacks
unmodeled (no extra-attack store).

## Slice 7 result — mount/transform display

Implemented as a holder-derived recompute
(`refresh_display`, mirroring `refresh_cc_flags`),
called from `add_aura_holder` and `remove_aura_at`
before `refresh_speed`: MOUNTED transitions choose
the template display (`choose_display_id_for_template`
+ random-gender swap), write
`UNIT_FIELD_MOUNTDISPLAYID`, strip
`MOUNT_CANCELS`/`DISMOUNT_CANCELS` interrupt auras,
and send player `SMSG_MOUNTRESULT(10)` /
`DISMOUNTRESULT(3)` (display 0 → strip +
`NOTMOUNTABLE(3)`). TRANSFORM winner simulates fork
precedence over holders in apply order (proven equal
to fork's last-negative-else-last removal re-apply);
apply does template display choice (unknown →
BOX 4), Orb of Deception 16739 race/gender table
with fork scale ratios, `SetTransformScale` against
captured base scale over the native model scale,
creature equipment swap to the transform template
and back on restore. `refresh_speed` gained the
UpdateSpeed mounted arm (MOD_INCREASE_MOUNTED_SPEED
32 / MOD_MOUNTED_SPEED_ALWAYS 130 /
MOD_MOUNTED_SPEED_NOT_STACK 172) and the mislabeled
MOD_SPEED_NOT_STACK constant was corrected 130→171.

Live evidence (`/tmp/mount_probe.py`, bench 18099):
MOUNTRESULT=10, MOUNTDISPLAY 0→2346 (Felsteed entry
304), forced run speed 11.200, cancel →
DISMOUNTRESULT=3, display 0, speed 7.000; Polymorph
118 on trogg spawn 4378: DISPLAY 726→856 (sheep
template 16372). `MOUNT-PROBE-PASS`, oracle
`live_character_load` + `live_char_enum` pass,
`TWO-CLIENT-SMOKE-PASS`, `./ci.sh CI_EXIT=0`
(`/tmp/ci_slice7.log`), bench restarted on final
binary (`PORT_OK`).

Unmodeled/recorded: pet unsummon/disable at mount
(+`PET_UNSUMMON_AT_MOUNT` config), ResetExtraAttacks
(no extra-attack store), channel mount-interrupt,
shapeshift-display fallback after transform (no
shapeshift display runtime), multi-mount-aura
dismount-on-first-fade fork quirk (recompute keeps
mounted while any MOUNTED holder remains),
Silithus 29519 flag drop, taxi/GM non-aura Mount()
entry points, damage-cancel of mount auras via
generic interrupt flags (pre-existing gap, not
mount-specific).
