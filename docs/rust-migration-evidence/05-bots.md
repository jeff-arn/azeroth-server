# 05 — Custom Player-Bot Stack: Architectural Evidence for C++→Rust Migration

Scope: `vmangos/src/game/PlayerBots/` plus every hook into core game code, the
AH bot in `vmangos/src/game/AuctionHouse/`, GM command surface, config, DB
tables, and `scripts/generate_talent_builds.py`. Read-only inspection;
all citations are `file:line` in the workspace at time of writing.
Governing docs: `docs/ARCHITECTURE.md` (bot sections from line 72),
`.claude/rules/bots.md` (168 lines, rules digest). Where docs and code
disagree, code wins and the discrepancy is flagged.

## 1. Inventory — 36 files, 24,254 lines in PlayerBots/ (+863 AH bot)

`wc -l vmangos/src/game/PlayerBots/*` = **24,254 lines, 36 files**
(18 header/source pairs). Sorted by size (source files):

| File | Lines | Role |
|---|---|---|
| `CombatBotBaseAI.cpp/.h` | 3829 / 596 | Shared combat base: spell-union population, healing engine, talent/gear bootstrap |
| `BattleBotAI.cpp/.h` | 3026 / 117 | Battleground bots (WSG/AB/AV), legacy per-class virtuals |
| `PlayerBotMgr.cpp/.h` | 2427 / 138 | Lifecycle singleton: roster load, login/logout, random/battle bots, GM handlers |
| `BotChatManager.cpp/.h` | 2123 / 293 | Social chat: trade/guild/party/general, personalities, conversation threading |
| `PartyBotAI.cpp/.h` | 1790 / 132 | Dungeon party bots: main tick, pre-steps, behavior dispatch, formation glue |
| `PlayerBotAI.cpp/.h` | 1300 / 157 | Base bot AI + lightweight AIs (Crowd, Dynamic, PopulateArea, AggressiveHorde, …) |
| `BattleBotWaypoints.cpp/.h` | 1089 / 64 | Recorded BG movement packets + waypoint path tables (mostly data) |
| `GuildBotMgr.cpp/.h` | 1014 / 157 | Persistent guild roster, gear scoring, presence system, raid assembly |
| `BotBehaviors_Shared.cpp/.h` | 992 / 260 | 19 shared scored behaviors |
| `BotServiceMgr.cpp/.h` | 957 / 206 | Stateful player services: portal/enchant/transmute/mooncloth/WTB-buy |
| `BotBehaviors_ClassUnique.cpp/.h` | 741 / 281 | 20 class-specific scored behaviors |
| `BotFormation.cpp/.h` | 437 / 156 | Breadcrumb trail + slot formation + flee points |
| `BotCombatEventBuffer.cpp/.h` | 408 / 130 | 256-event ring buffer + learned encounter patterns (`BotPatternMgr`) |
| `BotDirector.cpp/.h` | 353 / 74 | Population placement: assigns dynamic bots to zones with humans |
| `BotSpellKit.cpp/.h` | 342 / 59 | Capability adapter over the per-class spell-pointer union |
| `BotSituationContext.cpp/.h` | 221 / 79 | Per-tick `SituationContext` snapshot (`EvaluateSituation`) |
| `BotRoleConfig.cpp/.h` | 145 / 16 | Factory: class+role → weighted `BotBehaviorSet` (combat & OOC) |
| `BotBehavior.cpp/.h` | 81 / 64 | `BotBehavior` trait-like base + `BotBehaviorSet` scored runner |

Outside the directory but part of the stack:
`vmangos/src/game/AuctionHouse/AuctionHouseBotMgr.{h:119,cpp:744}` (863 lines),
`vmangos/src/game/AI/PlayerAI.{h,cpp}` (upstream bot-AI base class),
GM command tables/impls in `vmangos/src/game/Chat/Chat.cpp` and
`vmangos/src/game/PlayerBots/PlayerBotMgr.cpp` (72 `Handle*Bot*Command`
references across Chat/PlayerBotMgr).

**Rust implication:** ~25k lines is one large crate by itself, but it is
*not* separable from the world crate: bots are real `Player` objects with
real `WorldSession`s (see §8). The only cleanly extractable sub-crates are
pure-data/algorithm islands: `BattleBotWaypoints` (data tables),
`BotCombatEventBuffer` ring buffer, `BotFormation` math, talent-build tables.

## 2. Class hierarchy (the "bot" is a Player driven by an AI object)

```
PlayerAI (vmangos/src/game/AI/PlayerAI.h)
└─ PlayerBotAI            PlayerBotAI.h:29   — session/login/teleport-ack plumbing
   ├─ PlayerCreatorAI     PlayerBotAI.h:47   — spawn-new-character helper
   ├─ PlayerBotFleeingAI  PlayerBotAI.h:64   — test/event bot
   ├─ MageOrgrimmarAttackerAI PlayerBotAI.h:71 — scripted event bot
   ├─ PopulateAreaBotAI   PlayerBotAI.h:79   — city/area scenery wanderers
   │  └─ AggressiveHordeBotAI PlayerBotAI.h:94 — contested-zone PvP gankers
   ├─ CrowdBotAI          PlayerBotAI.h:111  — ultra-light city scenery (emotes/idle chat)
   ├─ DynamicBotAI        PlayerBotAI.h:130  — placement read from BotDirector assignments
   └─ CombatBotBaseAI     CombatBotBaseAI.h:74 — spell union, healing, talents, gear
      ├─ PartyBotAI       PartyBotAI.h:31    — scored-behavior dungeon bots (v0.9.0)
      └─ BattleBotAI      BattleBotAI.h:36   — BG bots, legacy per-class virtuals (18 `final` overrides, BattleBotAI.h:66-83)
```

Key structural facts:

- `PlayerBotAI` owns the fake-client lifecycle: `OnSessionLoaded` logs the
  character in (`PlayerBotAI.cpp:34-38`), `SpawnNewPlayer` creates a real
  `Player` + `MasterPlayer`, inserts it in cache/map/accessor
  (`PlayerBotAI.cpp:82-156`), and base `UpdateAI` synthesizes client acks
  (`HandleMoveTeleportAckOpcode`, `HandleMoveWorldportAckOpcode`,
  `PlayerBotAI.cpp:40-55`). Bots therefore exercise the *same* session,
  movement, visibility and save code as humans.
- The session→bot link is `WorldSession::m_bot`
  (`vmangos/src/game/Server/WorldSession.h:801`, accessors :276-277), a
  `shared_ptr<PlayerBotEntry>`; `PlayerBotEntry` (`PlayerBotMgr.h:31-46`)
  owns the AI via `std::unique_ptr<PlayerBotAI> ai` and carries
  guid/account/chance/state/chatBot/customBot/requestRemoval.
- `Player::IsBot()` is exactly `m_session->GetBot() != nullptr`
  (`vmangos/src/game/Objects/Player.h:2024`). A *second*, independent
  bot-identity convention coexists: **account ID < 100 = human, ≥ 100 = bot**
  (`.claude/rules/bots.md:148-153`: 97 CrowdBot, 98 GuildBot, 99 AHBot
  sellers, 100+ Party/Dynamic). The account check appears in ~25 places
  including core files (`Channel.cpp:674,681`, `Player.cpp:3254`,
  `WorldSession.cpp:596`, `ChatHandler.cpp:530`, `TradeHandler.cpp:523`).

**Rust implication:** two overlapping identity mechanisms (session-attached
entry vs account-id range) must collapse into one typed concept
(e.g. `enum Controller { Human(Session), Bot(BotKind) }`) or the port will
reproduce the bug class where one check is used where the other was meant.
The fake-client design means a Rust world server needs an internal
"headless session" implementing the same opcode-handler surface bots call
directly (§8).

## 3. Lifecycle — PlayerBotMgr (roster, login/logout, scaling)

`PlayerBotMgr` is a MaNGOS singleton (`sPlayerBotMgr`, `PlayerBotMgr.h:137`)
holding `std::map<uint64, shared_ptr<PlayerBotEntry>> m_bots` (:117).

- **Load:** `Load()` (`PlayerBotMgr.cpp:68`) reads the characters-DB
  `playerbot` table (:93) — guid, account, AI-name string — and instantiates
  AIs via the string factory `CreatePlayerBotAI(ainame)`
  (`PlayerBotAI.cpp:1267-1300`: `MageOrgrimmarAttackerAI`,
  `*PopulationAI` ×3, `PlayerBotFleeingAI`, `*HordeBotAI` ×4 zones
  (duplicated entries :1287-1294 — dead duplicate branches), `DynamicBotAI`,
  `CrowdBotAI`, default plain `PlayerBotAI`). Party/Battle bots are *not*
  in this table; they are created on demand by GM commands / BG queue logic
  (`AddBattleBot`, `PlayerBotMgr.cpp:595`).
- **Update loop:** `World::Update` calls `sPlayerBotMgr.Update(diff)` every
  world tick (`vmangos/src/game/World.cpp:2115`); internally throttled by
  `PlayerBot.UpdateMs` (default 10000 in code `PlayerBotMgr.cpp:58`, 1000 in
  `mangosd.conf.dist.in:3019`). `AddOrRemoveBot()` (:391) walks toward the
  target population; `AddRandomBot` picks weighted by each entry's `chance`
  column — which `BotDirector` rewrites in the DB (chance=30 assigned /
  1 unassigned, `BotDirector.cpp:346`). Time-of-day scaling:
  `GetScaledBotTarget()` with `RandomBot.PeakHour/OffPeakPercent/TimezoneOffset`
  (`PlayerBotMgr.cpp:60-62`, fields `PlayerBotMgr.h:130-134`).
- **Login/logout:** `OnBotLogin/OnBotLogout` (`PlayerBotMgr.cpp:167-178`)
  flip `PB_STATE_*` (`PlayerBotMgr.h:24-29`). Saving is globally gated by
  `PlayerBot.AllowSaving` (default false): `WorldSession` destructor/logout
  passes `!m_bot || sPlayerBotMgr.IsSavingAllowed()` (`WorldSession.cpp:81,393`),
  `CharacterHandler.cpp:493` skips character-save bookkeeping for bots, and
  `Player.cpp:20922` (`IsSavingDisabled() || IsBot()`) short-circuits a save
  path. Guild bots are the exception that *does* persist gear through normal
  `character_inventory`/`item_instance` (docs/ARCHITECTURE.md:280).
- **Temp/chat bots:** `m_tempBots` account→expiry map
  (`PlayerBotMgr.cpp:512-522`), `ForceAccountConnection` (:651) lets a bot
  session bypass the one-session-per-account rule (called from
  `WorldSession.cpp:432`); `CanProcessPackets()` treats socket-less bot
  sessions as live (`WorldSession.cpp:454`).

**Rust implication:** lifecycle is DB-table-driven (`playerbot.chance` as a
control channel written by BotDirector, read by PlayerBotMgr). A port should
replace the DB round-trip with an in-process channel, but must keep the
table for roster persistence. The save-suppression flag is load-bearing for
production character data — it must be modeled as a type-level property of
the session, not a runtime bool forgotten at one call site.

## 4. The tick — PartyBotAI and the scored-behavior runner (v0.9.0)

Party bots tick at **500 ms** (`PB_UPDATE_INTERVAL`, `PartyBotAI.cpp:48`),
gated in `PartyBotAI::UpdateAI` (:880-908). One tick is:

1. Pending service-trade accept: synthesizes `CMSG_ACCEPT_TRADE` and calls
   `me->GetSession()->HandleAcceptTradeOpcode` directly (:883-902).
2. Social timers (OOM/pull chat cooldowns, delayed group greeting :910-932).
3. First-tick init block (:937-1065): join leader's group, clone-or-learn
   spec (`LearnPremadeSpecForClass` / `CloneFromPlayer`), `AutoAssignRole`,
   gear (`AutoEquipGear` honoring guild gear-tier and the
   never-wipe-loot-upgrades rule :949-1004), `PopulateSpellData`, build
   `m_kit` + combat/OOC behavior sets (:1030-1033), `sBotPatternMgr.SetActive(true)`
   (:1034), formation slot assignment (:1050-1055).
4. `m_resetSpellData` rebuild path (:1067-1075) — role changes via
   `.partybot setrole` / `.partyrole` set this flag to rebuild behavior sets.
5. Leader/formation bookkeeping: dead leader → `botEntry->requestRemoval`
   (:1077-1082); `sBotFormationMgr.UpdateTrail` every tick (:1088).
6. Combat or OOC dispatch: `UpdateInCombatAI` (:1518) /
   `UpdateOutOfCombatAI` (:1467). Both run **pre-steps** first —
   `CheckForDispelTargets()` (:1485/:1570), pet attack command (:1506/:1588),
   hunter Auto Shot maintenance — then `EvaluateSituation(this, ctx)`
   (:1491/:1615) fills the snapshot once, then
   `m_combatBehaviors->RunBestBehavior(ctx, *m_kit)` (:1509/:1617) or the
   OOC set (:1513).

**The runner** (`BotBehavior.cpp:16-81`) is deliberately tiny and is the
cleanest port target in the stack: for each registered behavior with
`roleWeight > 0`, compute `Score(ctx) * roleWeight`, keep positives in a
fixed 32-slot stack array (:27 — silent truncation beyond 32 candidates),
sort descending (:64), `Execute()` in order until one returns `true`
(fallthrough, :67-78). `BotBehavior` is a 3-method interface —
`Score`/`Execute`/`GetName` (`BotBehavior.h:15-30`) — i.e. a Rust trait.
Debug: `SetDebugTarget` logs every non-zero score + winner as `[BotAI]`
lines (:41-44, :71-74), toggled by `.partybot debug`.

**SituationContext** (`BotSituationContext.h:11-73`) is a plain snapshot
struct: self HP/mana/rage/energy/form/combo points, threat ratio + throttle
flag, target ptr/health/distance/casting, party aggregate (tank HP, most
injured ally, heal-needed flags, loose adds), encounter (`LearnedPattern*`,
boss flag), cached role, and formation/flee target coordinates. Raw `Unit*`
pointers inside (`currentTarget`, `tankUnit`, `mostInjuredAlly`) are the
main borrow-checker friction point for a port: in Rust these become
`ObjectGuid`s resolved at Execute time, or the whole tick must run inside a
scoped world borrow.

**Roles:** `CombatBotRoles` lives in core `SharedDefines.h:104-111`
(`ROLE_INVALID/MELEE_DPS/RANGE_DPS/TANK/HEALER`) — a bot concept leaked into
the global defines header. Human role overrides are a static map behind
`PartyBotAI::SetPlayerRole/GetPlayerRole` (`PartyBotAI.h:48-50`, impl
`PartyBotAI.cpp:146`), set by `.partyrole` (SEC_PLAYER).
`FindPartyTank()` priority: explicit assignment → bot `ROLE_TANK` →
Bear-Form/shield heuristic, with the explicit-non-tank gate
(`.claude/rules/bots.md:90-100`).

## 5. Every BotBehavior subclass (19 shared + 20 class-unique = 39)

Docs say 18+19 (`docs/ARCHITECTURE.md:85-86`); headers actually declare
**19 shared** (`grep -c` on `BotBehaviors_Shared.h`) and **20 class-unique**
(`BotBehaviors_ClassUnique.h`). One declared behavior,
`BehaviorCasterAttackStop` (`BotBehaviors_Shared.h:247-258`, impl
`BotBehaviors_Shared.cpp:976-992`), is **never registered** in
`BotRoleConfig.cpp` — dead code; caster `AttackStop()` is done elsewhere
(rules `.claude/rules/bots.md:108-110`). Effective registered set: 38.

Shared (`BotBehaviors_Shared.h`, role = typical weighting from
`BotRoleConfig.cpp:32-55`):

1. `BehaviorSelfPreserve` :13 — emergency cooldowns at critical HP (all roles, healer 1.2×)
2. `BehaviorInterrupt` :26 — kick/pummel/CS target casts (all)
3. `BehaviorTaunt` :39 — peel mobs off non-tanks (tank 1.5×, others 0 — PartyBotAI-only)
4. `BehaviorHealParty` :52 — main healing via CombatBotBaseAI heal engine (healer 1.5×)
5. `BehaviorEmergencyHeal` :65 — hybrid off-heal when healer unavailable (DPS 0.5×)
6. `BehaviorTankThreat` :78 — threat rotation (tank 1.5× only)
7. `BehaviorThreatDump` :91 — Feint/Fade/FD near aggro pull (non-tank)
8. `BehaviorCrowdControl` :104 — CC marked/secondary targets (ranged-weighted)
9. `BehaviorKite` :117 — casters escape melee (healer/ranged only)
10. `BehaviorExecuteTarget` :130 — sub-20% finishers
11. `BehaviorApplyDots` :144 — DoT upkeep
12. `BehaviorAoeDamage` :157 — AoE at 3+ enemies
13. `BehaviorDirectDamage` :170 — filler nuke/attack (the default)
14. `BehaviorSelfBuff` :183 — missing self-buffs (combat + OOC sets)
15. `BehaviorPosition` :196 — range keeping / formation slot (OOC 0.5×)
16. `BehaviorDispel` :209 — party debuff removal (PartyBotAI-only; also a pre-step)
17. `BehaviorPartyBuff` :222 — MotW/Fort/AI (OOC set only)
18. `BehaviorWandFiller` :235 — idle healers wand tank's target (healer only)
19. `BehaviorCasterAttackStop` :248 — **unregistered/dead** (see above)

Class-unique (`BotBehaviors_ClassUnique.h`, wired per-class in
`BotRoleConfig.cpp:58-97` combat / :111-141 OOC, all at weight 1.0):

- Warrior (2): `WarriorStanceDance` :13, `WarriorRageDump` :25
- Rogue (4): `RogueStealthOpener` :41 (MovePoint behind target + `IsBehindTarget`),
  `RogueComboFinisher` :53, `RogueStealth` :65 (OOC), `RoguePoisonWeapons` :77 (OOC)
- Hunter (3): `HunterPetManage` :93, `HunterAspectSwap` :105, `HunterDeadZone` :117
- Paladin (2): `PaladinBlessing` :133 (OOC), `PaladinSealJudge` :145
- Shaman (1): `ShamanTotemManage` :161
- Priest (2): `PriestShadowformToggle` :177, `PriestShieldLogic` :189
  (PW:S never on warrior tank >15% HP — `.claude/rules/bots.md:117-119`)
- Warlock (3): `WarlockPetSummon` :205 (OOC set; Voidwalker excluded when
  tank present), `WarlockCurseManage` :217 (one curse/target, Recklessness
  banned), `WarlockLifeTap` :229
- Druid (3): `DruidFormShift` :245, `DruidRebirth` :257, `DruidInnervate` :269
- Mage (0): no class-unique behaviors; fully served by shared set + SpellKit.

**SpellKit** (`BotSpellKit.h:13-57`) is the anti-coupling adapter: behaviors
ask for capabilities (`GetInterrupt`, `GetFiller`, `GetTaunt`,
`GetThreatDump`, `GetExecuteAbility`, `GetAoeSpell`, `GetDotSpells`,
`GetCrowdControl`, `GetRoot`, `GetSelfBuff/PartyBuff`,
`GetEmergencyAbility`, `GetEscapeAbility`) instead of touching the
`m_spells` union — a 50-slot raw array overlaid with 9 per-class pointer
structs (`CombatBotBaseAI.h:288-596`, ~200 named `SpellEntry const*`
fields). Population is by spell-name scan in `PopulateSpellData()`.
In Rust this union becomes an `enum ClassSpells { Warrior{..}, … }` or a
`HashMap<Capability, SpellId>`; the name-scan population step and the
union's implicit nullability are the two things a port should *not*
reproduce literally.

**BattleBotAI does not use behaviors at all**: it `final`-overrides the 18
per-class `Update{In,OutOf}CombatAI_<Class>` virtuals that
`CombatBotBaseAI` defaulted to `{}` (`CombatBotBaseAI.h:143-164`,
`BattleBotAI.h:66-83`) — the pre-v0.9.0 architecture preserved for BGs.
A Rust port must either keep two AI engines or migrate BattleBot onto the
behavior runner (behavior sets are class+role parameterized already, but BG
objective logic — flag carrying `BattleBotAI.h:62`, waypoint following
:100-114 — has no behavior-layer equivalent today).

## 6. Formation & movement

`BotFormationMgr` is a singleton keyed by leader GUID
(`BotFormation.h:115-129`, `unordered_map<uint64, FormationState>`).
Per group: a `BreadcrumbTrail` ring buffer — 40 points, 1 s sampling,
>2 yd movement threshold (`BotFormation.h:43-45`) — plus role-ordered
`FormationSlot`s (tank 2 yd / melee 4 yd / ranged 8 yd / healer 10 yd behind
on the trail, ±1 yd lateral; `BotFormation.h:131-154`). Slot position =
arc-length walk back along the trail + lateral offset, validated by
(a) navmesh `Map::GetWalkHitPosition` (collapse to trail center if hit
moves >3 yd) and (b) 12 yd hostile scan (`.claude/rules/bots.md:77-82`).
`CalculateFleePoint` searches 15-30 yd *back* along the cleared trail
(`BotFormation.h:106-108`) for `BehaviorKite`.

Movement primitives actually used in PlayerBots (grep counts over *.cpp):
`MovePoint` ×26 (formation slots, kite, rogue opener — one-shot),
`MoveChase` ×24 (combat range: 25 yd ranged/healer, 1 yd melee/tank),
`MoveFollow` ×3 (legacy fallback when trail empty). Hard rules that a port
must preserve: formation never fires in combat (`!ctx.currentTarget` guard),
2 yd move dead-zone (`BotFormation.h:150`), trail cleared on >100 yd
teleport jump, `POINT_MOTION_TYPE` excluded from the follow check
(`.claude/rules/bots.md:61-88`). BattleBots use a completely separate
system: pre-recorded movement-packet paths + waypoint vectors per BG
(`BattleBotWaypoints.h:23-62`, 1089 lines of data in the .cpp) — pure data,
ports as static arrays verbatim.

## 7. Social & services layers (what actually exists)

Two singletons, both ticked on the **60 s `WUPDATE_AUCTIONS` timer**, not
the world tick (`vmangos/src/game/World.cpp:1992-1993`) — which is why all
deferred chat uses absolute `time_t fireTime`, never ms countdowns
(`BotChatManager.h:127-134`, `.claude/rules/bots.md:34-45`). (One violation
survives by design at a different cadence: `PendingWhisper::delayMs`,
`BotChatManager.h:236-244`, decremented by `UpdatePendingWhispers(diff)` —
it receives the 60 s interval as `diff`, so whisper delays quantize to the
minute tick.)

**BotChatManager** (`sBotChatMgr`, 2123 lines): personalities
(generic/enthusiastic/chill/sarcastic/quiet, `BotChatManager.h:28-35`, from
`guild_bot_roster.personality`; quiet ≈30% rate), per-bot per-channel
cooldowns (:50-64), trade-chat advertising, guild chat, party reactions
(join greeting, level-up, boss kill, wipe, OOM, pulling — API :194-200),
whisper replies (:185), and zone-population General chat
(`0.09 msgs/min/bot`, :272) with 8 topic pools (:70-81), scripted
conversation chains, debates, and keyword reactive hooks
(`OnGeneralChatMessage` :209, fired from `Channel::Say`).
**"LFG" exists only as chat flavor** — an LFG topic pool and keyword hook
(`BotChatManager.cpp:150,1048-1051`); there is no dungeon-finder/queue
logic. Bots join groups only via `.partybot` commands or guild-presence
conversion (§8).

**BotServiceMgr** (`sBotServiceMgr`, 957 lines): a 4-state request machine
per (bot,player) pair (`BotServiceMgr.h:52-90`: asked-destination →
waiting-for-trade → in-progress → complete, 300 s timeout) delivering
mage portals (triggered cast, 6 city spells :19-27), enchants (applied
directly via `Item::SetEnchantment` post-trade to dodge trade-window
accept loops — `docs/ARCHITECTURE.md:221`), alchemy transmutes, tailor
mooncloth, and WTB buying (15-item registry at AH fair value −10%,
per-bot daily budget, `BotServiceMgr.h:144-198`). Profession identity is
inferred, not stored: `IsMageBot/IsEnchanterBot/IsAlchemistBot/IsTailorBot`
(:129-132) all gate on `GetAccountId() >= 100` then class/skill checks
(`BotServiceMgr.cpp:687-712`). Entry points: whispers
(`HandleServiceWhisper`) and proactive trade-chat detection
(`BotChatManager::OnTradeChatMessage` → `CreateProactiveRequest` +
minimap `SMSG_GOSSIP_POI` ping, `BotChatManager.h:203,215`).

**AuctionHouseBotMgr** (`sAuctionHouseBotMgr`, outside PlayerBots/):
seller simulation — 6 item categories × 4 seller archetypes
(`AuctionHouseBotMgr.h:14-51`), weighted pools from world-DB
`auctionhousebot` table, 72 h wipe-and-repopulate cycle, plus a buyer
system scanning player auctions every 30 min with price-vs-fair-value
purchase probability and 50 g/cycle budget (:108-116).
`GetFairBuyoutForItem` (:76) is shared with BotServiceMgr's WTB pricing —
a cross-singleton dependency a port should make explicit.

**BotDirector** (353 lines): every 2 min (`BotDirector.UpdateInterval`,
`BotDirector.cpp:69-70`) scans online *humans* (`GetAccountId() >= 100`
skipped, :152-153), classifies zones capital/friendly/contested
(`BotDirector.h:11-30`), builds `BotAssignment`s from fixed GUID pools
(Horde 33-52, Alliance city 53-72, Alliance world 73-92 —
`docs/ARCHITECTURE.md:246-266`), writes `playerbot.chance` 30/1, and
`DynamicBotAI` pulls its assignment back via `sBotDirector.GetAssignment`
(`BotDirector.h:52-53`). Contested zones get aggressive bots (10% lvl-60
"ganker"/"hero").

**GuildBotMgr** (1014 lines): persistent guild "Sweatlords" — roster in
characters-DB `guild_bot_roster` (guid/name/class/spec/role/personality/
gear_score/gear_tier/has_loot_upgrades; queried `GuildBotMgr.cpp:59`,
mutated :480-501,:667-691), stat weights in `guild_bot_spec_weights`
(:292), 5-min level sync to GM ±3 (:261-269, interval `GuildBotMgr.h:144`),
time-of-day *presence* system (peak 14 / off-peak 3 online by default,
`GuildBotMgr.cpp:38-43`) that logs bots in as CrowdBot scenery and converts
them to PartyBotAI on invite (`ConvertPresenceToParty`, `GuildBotMgr.h:111`).
Gear persists in normal character tables; `hasLootUpgrades` permanently
protects looted gear from auto-equip wipes (`GuildBotMgr.h:41`).

**Encounter learning** (`BotPatternMgr` in `BotCombatEventBuffer.{h,cpp}`):
256-slot ring buffer fed by hooks in `Unit.cpp` (damage/death) and
`Spell.cpp` (cast start); learns (creatureEntry, spellId) → response
(interrupt/move-away/spread/save-CD) with decaying confidence, persisted
to characters-DB `bot_learned_patterns` (load/save
`BotCombatEventBuffer.cpp:260-310`, `World.cpp:1884,2667`). Only active
when a party bot exists (`SetActive`, `BotCombatEventBuffer.h:113-115`).

## 8. Coupling surface into core code (the crate-boundary decider)

Outside `PlayerBots/` and `AuctionHouse/`, bot-aware core sites total
**52 grep hits** for
`IsBot|GetBot|sPlayerBotMgr|sBotChatMgr|sBotServiceMgr|sGuildBotMgr|sBotDirector|sBotPatternMgr|sAuctionHouseBotMgr`,
plus ~10 account-`<100` comparisons and the Chat.cpp command tables.
Grouped by subsystem (this is the port's integration checklist):

| Core area | Sites | Nature |
|---|---|---|
| `World.cpp` | Load :1810-1831,1884; Update :1991-1995,2115; DeleteAll :193; config :970-976; SaveToDB :2667 | Singleton lifecycle: 6 bot singletons + AH bot loaded/ticked from world loop; 4 on 60 s AUCTIONS timer, PlayerBotMgr every tick |
| `WorldSession.{h,cpp}` | h:276-277,801; cpp:81,116-117,182-183,352,393,432,454,596-597 | `m_bot` ownership; packet tap `ai->OnPacketReceived` on send; save gating; forced account connection; socket-less liveness; service cleanup on logout |
| `Player.{h,cpp}` | h:2024; cpp:2208,3254-3255,14502,14737,20893-20922 | `IsBot()`; AI attach/reattach from session entry; account-mismatch save guard; level-up chat hook; `OnPlayerInWorld` |
| `Unit.cpp` / `Spell.cpp` | Unit:767,952,7082; Spell:3719 | Pattern-learning recorders in damage/death/cast hot paths; "controlled by own client" excludes bots |
| `Channel.cpp` | :674-683 | Trade/General chat taps → service requests + reactive hooks (human-only via account<100) |
| `ChatHandler.cpp` | :529-531 | Whisper-to-bot → `BotChatMgr.HandleWhisper` (account≥100 test) |
| `TradeHandler.cpp` | :523-533 | Trade completion with a bot → `BotServiceMgr.OnTradeCompleted(bot, human, gold)` (either orientation) |
| `GroupHandler.cpp` | :157,176-177 | Invite of a presence guild bot → convert Crowd→Party AI and join |
| `Group.cpp` | :425,531 | Member removal → `GuildBotMgr.OnBotRemovedFromGroup` (convert back to presence) |
| `CharacterHandler.cpp` | :493 | Skip save when bot && !AllowSaving |
| `MiscHandler.cpp` | :91,118 | /who list hides bots unless `PlayerBot.ShowInWhoList` |
| `BattleGround.cpp` | :787 | `IsBot()` branch in BG player handling |
| `Map.cpp` | :1781 | `!IsBot()` branch in a per-player map update/visibility path |
| `Chat.cpp` (+`Chat.h`, impls largely in `PlayerBotMgr.cpp`) | command tables :69-111,716-718; 72 handler refs | GM surface: `.bot`, `.partybot` (20 subcommands), `.partyrole`, `.battlebot`, `.guild raid*`, `.ahbot` |
| `SharedDefines.h` | :104-111 | `CombatBotRoles` enum in global header |

In the other direction, PlayerBots code includes core headers pervasively
(top counts across the 36 files): `Player.h` ×18, `Group.h` ×9, `World.h`
×7, `Spell.h` ×7, `ObjectMgr.h` ×7, `Opcodes.h` ×6, `MotionMaster.h` ×4,
`DatabaseEnv.h` ×4, `Chat.h` ×4. Bots also *synthesize client input* by
calling session opcode handlers directly (9 distinct handlers:
accept-trade, teleport/worldport ack, battlemaster join, battlefield port,
area trigger, gameobject use) and read server→client packets via the
`OnPacketReceived` tap (BG invites, trade windows).

**Crate-boundary conclusion:** there is no "bot crate" separable from a
"world crate" without first defining a narrow *Host API*: (1) headless
session that accepts the 9 synthesized opcodes and exposes the packet tap;
(2) `Player`/`Group`/`Map` read+command surface used by behaviors
(cast, move primitives, threat, auras, inventory/trade, teleport);
(3) event taps (chat channels, whispers, trade completion, group
membership, damage/death/cast, level-up, logout) — the 52 sites above are
exactly that event list; (4) scheduler slots (per-tick 500 ms per bot,
60 s social tick, 2 min director, 5 min level-sync, 30 min AH buyer).
Inside that boundary, natural Rust modules map 1:1 to today's singletons:
`bot_mgr` (lifecycle), `behavior` (trait + runner + 38 impls),
`spell_kit`, `formation`, `chat`, `services`, `director`, `guild`,
`patterns`, `ah_bot`. The singletons themselves (8: `sPlayerBotMgr`,
`sBotChatMgr`, `sBotServiceMgr`, `sGuildBotMgr`, `sBotDirector`,
`sBotFormationMgr`, `sBotPatternMgr`, `sAuctionHouseBotMgr`) become owned
fields on a `BotSystem` struct — they already only interact through narrow
calls (fair-price lookup, presence conversion, assignment fetch).

## 9. Talent build data pipeline

`scripts/generate_talent_builds.py` (643 lines) parses extracted client
DBCs (`TalentTab.dbc`, `Talent.dbc`, `Spell.dbc` via a local `dbc` module;
hardcoded prod path :11) and resolves **26 researched PvE builds**
(9 classes × 2-3 specs; `BUILDS` dict :117ff, counted 26 keys:
Warrior 3, Paladin 3, Hunter 3, Rogue 2, Priest 2, Shaman 3, Mage 3,
Warlock 3, Druid 4) from (talent-name, ranks) lists — ordered as spent
from level 10 — into DBC talent IDs, **printing C++ to stdout** (:586-643).
Output is pasted into `CombatBotBaseAI.cpp` as
`static BotTalentBuild const s_talentBuilds[]` (:271, count :308,
struct :72). Consumer: `LearnPremadeSpecForClass()` (:2754-2810) picks a
build matching role (or random, then derives role), `ResetTalents()` +
`LearnTalent()` per entry, then `LearnClassSpellsForLevel()` (:2811)
learns all trainer spells via `SkillLineAbility`, then
`PopulateSpellData()`. `LearnRandomTalents()` remains as legacy fallback
(:2861 comment). Rule: regenerate, never hand-edit the arrays
(`.claude/rules/bots.md:131-144`).

**Rust implication:** this is build-time codegen — port as a `build.rs` or
checked-in generated data file (`bot_talent_builds.rs`), keeping the Python
script as the source of truth until DBC parsing exists in Rust. The data
itself (26 ordered (talentId, ranks) lists) is tiny and stable.

## 10. Config knobs (names only; live values not inspected)

In `mangosd.conf.dist.in:2918-3028`: `AHBot.Enable/ah.fid/itemcount`,
`RandomBot.Enable/MinBots/MaxBots/Refresh`, `PlayerBot.AllowSaving/Debug/
UpdateMs/ShowInWhoList`, `PartyBot.MaxBots/SkipChecks/AutoEquip/
RandomGearLevelDifference`, `BattleBot.AutoEquip/AutoJoin` (the
`PartyBot/BattleBot/PlayerBot` subset is registered as typed world config
in `World.cpp:971-976`). Read directly via `sConfig` and **absent from the
.dist file** (undocumented knobs a migration must not lose):
`RandomBot.PeakHour/OffPeakPercent/TimezoneOffset` (`PlayerBotMgr.cpp:60-62`),
`BotChat.Enable`, `BotChat.TradeChat.IntervalMin/Max`,
`BotChat.GuildChat.IntervalMin/Max`, `BotChat.Cooldown.*` ×6
(`BotChatManager.cpp:199-216`), `BotDirector.Enable/UpdateInterval`
(`BotDirector.cpp:69-70`), `BotService.Enable`, `BotService.Buy.BudgetPerDay`
(`BotServiceMgr.cpp:190-191`), `GuildBot.Enable/AccountId`,
`GuildBot.Presence.*` ×5 (`GuildBotMgr.cpp:28-43`), plus AHBot refresh/buyer
settings read in `AuctionHouseBotMgr.cpp`. Total ≈35 bot config keys.

DB tables (characters DB unless noted): `playerbot` (roster+chance),
`guild_bot_roster`, `guild_bot_spec_weights`, `bot_learned_patterns`;
world DB: `auctionhousebot`. Schema/migrations under `vmangos/sql/`
(11 recent migration files reference bot tables, e.g.
`vmangos/sql/migrations/20260325*`–`20260406*`, plus `characters.sql`).
Production rule: append-only migrations (CLAUDE.md) — a Rust port inherits
these tables as-is.

## 11. Testing / verification status

No automated C++ tests for the bot stack were found (only discord-bot
Python tests exist repo-wide). One function is explicitly shaped for testability —
`IsChatCooldownExpired(now, lastChat, cooldown)` with injected clock
(`BotChatManager.h:18-22`) — but no test calls it. Verification is in-game
smoke tests + `.partybot debug` `[BotAI]` scoring logs
(`.claude/rules/testing.md`, `.claude/rules/bots.md:157-159`). For a Rust
port this means: the behavior runner, formation math, cooldown logic and
pattern buffer are unit-testable in isolation and should gain tests during
port; end-to-end correctness has no golden harness and would need
Server.log `[BotAI]` trace comparison as the oracle.

## 12. Unresolved / not inspected

- Bodies of the largest .cpp files were read selectively (headers fully;
  `PartyBotAI::UpdateAI` and manager entry points in body). Per-behavior
  Score/Execute internals, `BotDirector::RunDirectorCycle` math, AH pricing
  formulas and `GuildBotMgr` gear scoring were characterized from headers,
  docs and targeted greps, not line-by-line.
- `docs/BOT_ARCHITECTURE_PROPOSAL.md` and the gitignored research docs
  (`CLASS_GROUP_BEHAVIOR_GUIDE.md` etc., `docs/ARCHITECTURE.md:149`) were
  not read.
- Live config values, DB contents, and actual online bot counts were not
  inspected (read-only/no-secrets constraint); all defaults cited are
  code/.dist defaults.
- `Map.cpp:1781` and `BattleGround.cpp:787` bot branches were located but
  their surrounding logic was not fully read.


