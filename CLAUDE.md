# Azeroth Private Server

VMaNGOS-based WoW 1.12.1 private server with custom bot AI, a web dashboard, and Discord integration.

## Owner
- GitHub: jeff-arn
- Remote: `https://github.com/jeff-arn/azeroth-server.git`

## Directory Structure

```
~/wow-private-server/              ← git repo root
├── .env                           ← secrets (gitignored)
├── .env.example                   ← template for .env
├── .gitignore
├── CLAUDE.md
├── dashboard/                     ← Flask web UI
│   ├── dashboard.py
│   ├── templates/dashboard.html
│   └── venv/                      (gitignored)
├── discord-bot/                   ← Discord status/activity bot
│   ├── bot.py
│   ├── post_patch_notes.py
│   ├── requirements.txt
│   └── venv/                      (gitignored)
├── scripts/
│   ├── backup-db.sh               ← daily DB backup (cron)
│   └── release.sh                 ← version bump + tag + Discord notify
├── vmangos/                       ← C++ game server (owned, no upstream)
│   ├── src/                       ← source code
│   ├── sql/                       ← schema + migrations
│   ├── build/                     (gitignored)
│   ├── run/bin/                   (binaries + map data, gitignored)
│   └── run/etc/
│       ├── mangosd.conf           (gitignored)
│       ├── mangosd.conf.example   ← tracked template
│       ├── realmd.conf            (gitignored)
│       └── realmd.conf.example    ← tracked template
└── vmangos-database/              ← initial DB import (dumps gitignored)
```

## Services

All Python services use `EnvironmentFile=/home/wow/wow-private-server/.env` for secrets.

| Service | systemd unit | Entry point |
|---------|-------------|-------------|
| Web Dashboard | `azeroth-dashboard.service` | `dashboard/dashboard.py` |
| Discord Bot | `azeroth-discord-bot.service` | `discord-bot/bot.py` |
| Realm Server | `azeroth-realmd.service` | `vmangos/run/bin/realmd` (port 3724) |
| World Server | `azeroth-mangosd.service` | `vmangos/run/bin/mangosd` (port 8085) |

## Secrets

All secrets live in `.env` (gitignored). Python services read from environment variables. The `.env.example` file shows the required keys:
- `DB_HOST`, `DB_PORT`, `DB_USER`, `DB_PASSWORD` — MySQL credentials
- `DISCORD_BOT_TOKEN` — Discord bot authentication

VMaNGOS configs (`mangosd.conf`, `realmd.conf`) also contain DB credentials and are gitignored. Use the `.conf.example` files as templates.

## Database

- **Host**: 127.0.0.1:3306
- **User**: mangos (see .env for password)
- **Databases**: `realmd`, `mangos` (world data), `characters`, `logs`
- **Migrations**: `vmangos/sql/migrations/` (timestamped, procedural, idempotent)
- **Backups**: `scripts/backup-db.sh` runs daily at 4am UTC via cron, 14-day retention

## Custom VMaNGOS Changes

The vmangos source is fully owned — no upstream relationship. Key customizations:
- PartyBot commands downgraded from SEC_ADMINISTRATOR to SEC_MODERATOR
- Behavior Layer AI: scored behavior system replacing per-class if/else chains (v0.9.0)
- Combat Event Ring Buffer: encounter pattern learning with DB persistence
- DPS threat throttling and combat opener logic

### PartyBot AI Architecture (v0.9.0 — Behavior Layer System)

Bot AI uses a three-layer scored behavior system (replaced per-class if/else chains in v0.9.0). Architecture doc: `docs/BOT_ARCHITECTURE_PROPOSAL.md`.

**Source files:**
- `PartyBotAI.cpp` — main UpdateAI loop, pre-steps (pet attack, dispel, Auto Shot), behavior dispatch
- `BotSituationContext.h/.cpp` — `SituationContext` struct computed once per tick (self HP/mana, threat, target, party state, encounter patterns)
- `BotBehavior.h/.cpp` — `BotBehavior` base class + `BotBehaviorSet` runner (scores all, tries in descending order with fallthrough)
- `BotBehaviors_Shared.h/.cpp` — 18 shared behaviors (interrupt, kite, heal, tank, CC, position, wand filler, etc.)
- `BotBehaviors_ClassUnique.h/.cpp` — 19 class-specific behaviors (stealth opener, combo finishers, totem management, form shifting, etc.)
- `BotSpellKit.h/.cpp` — abstract capability interface over `m_spells` union (`GetInterrupt()`, `GetFiller()`, `GetTaunt()`, etc.)
- `BotRoleConfig.h/.cpp` — factory wiring behaviors to class + role with weight multipliers
- `BotCombatEventBuffer.h/.cpp` — ring buffer for encounter learning, pattern detection, DB persistence
- `CombatBotBaseAI.h/.cpp` — spell pointer structs, spell population, shared healing system (unchanged)

**How it works:**
1. `EvaluateSituation()` fills a `SituationContext` snapshot (HP, mana, threat, target, party state)
2. Each behavior's `Score(ctx)` returns 0-100 urgency, multiplied by role weight
3. `RunBestBehavior()` sorts by score, tries each in descending order until one's `Execute()` returns true
4. If Execute fails (spell on CD, out of range, etc.), the runner falls through to the next-highest scorer

**Pre-steps (free actions before behavior scoring):**
- Pet attack command (warlock imp Phase Shift strip, hunter pet)
- Dispel check (`CheckForDispelTargets()`)
- Hunter Auto Shot autorepeat maintenance
- Caster ranged positioning via `AttackStart()` (25yd `MoveChase` for ROLE_RANGE_DPS/ROLE_HEALER)

**Role weights** (Tank / Healer / Melee DPS / Ranged DPS):
- Healing: 0.3 / 1.5 / 0.0 / 0.0
- Tank threat/taunt: 1.5 / 0.0 / 0.0 / 0.0
- Kite: 0.0 / 1.0 / 0.0 / 1.2
- DPS: 0.5 / 0.2 / 1.0 / 1.0

**Healer positioning**: Healers follow leader at 12-18yd during combat (instead of 3-6yd). `BehaviorPosition` and `BehaviorWandFiller` fall back to tank's target when healer has no victim.

**Rogue stealth opener**: `AttackStart()` skips `me->Attack()` when stealthed (preserves stealth). `BehaviorRogueStealthOpener` calculates a point behind the target via `MovePoint` (not `MoveChase` — MoveChase can't reposition to a specific angle once in range). Explicit `IsBehindTarget()` check before casting. Main loop skips follow logic when stealthed with target set.

**Imp Phase Shift**: `TeachWarlockPetSpells()` teaches Phase Shift with `ACT_ENABLED` (auto-casts when idle). Pet attack pre-step strips Phase Shift aura + disables auto-cast via `ToggleCreatureAutocast(4511, false)` before sending to attack.

**Debug**: `.partybot debug` (SEC_MODERATOR) toggles per-tick behavior logging for a selected bot. Logs all non-zero scores and the winning behavior + result to Server.log as `[BotAI]` lines.

**BattleBotAI compatibility**: 18 per-class virtual methods changed from `= 0` to `{}` empty defaults. BattleBotAI's `final` overrides still work unchanged.

**Key design lessons (v0.9.0):**
- Behaviors MUST check preconditions in `Score()`, not just `Execute()`. A behavior that scores >0 but always fails blocks lower behaviors.
- Free actions (pet commands, dispels, Auto Shot) should be pre-steps in the dispatcher, not scored behaviors.
- Never set legacy state flags (`m_isBuffing`) from behaviors — the main UpdateAI loop checks these.
- `AttackStop()` clears `m_attacking` (the victim reference). Never call it before `EvaluateSituation()`.
- `MoveChase` angle parameter is a pathfinding hint, not a positional constraint. Use `MovePoint` to a calculated coordinate for precise positioning (e.g., rogue behind-target).
- The main loop's `MoveFollow` overrides behavior-issued movement. Role-aware follow distance and stealth guards are needed.
- `.partybot setrole` must set `m_resetSpellData = true` to trigger behavior set rebuild.

**Tank detection** (`FindPartyTank()`): Checks in priority order — (1) explicit `.partyrole` assignment via `s_playerRoleAssignments` static map, (2) bot AI `ROLE_TANK`, (3) heuristics: Bear Form or shield equipped. Human players with an explicit non-tank role skip heuristics entirely (prevents paladin DPS with shield being misidentified). Command: `.partyrole tank|healer|dps|clear` — available to all players (`SEC_PLAYER`).

**Priest shield (PW:S) rules**: Never cast on warrior tanks except below 15% HP (emergency). Applies in three places: pre-pull, in-combat critical threshold (<30%), and `SelectShieldTarget()`.

**Spec-aware talent & spell system**: `LearnPremadeSpecForClass()` in `CombatBotBaseAI.cpp` replaces the old DB-template approach. 26 researched PvE talent builds (all 9 classes × 2-3 specs each) defined as static `BotTalentBuild` arrays. On spawn:
1. Pick a build matching the bot's role (or random if no role set, then set role from build)
2. `ResetTalents()` + walk the build's ordered talent point array, calling `LearnTalent()` per entry until out of points
3. `LearnClassSpellsForLevel()` — iterates `SkillLineAbility`, learns all class trainer spells at or below the bot's level (filtered by spell family, skips talents)
4. `PopulateSpellData()` scans learned spells by name and populates spell pointers (unchanged)
- Works at every level 1-60 — no template gaps. Old `player_premade_spell_template` DB tables are still used for **gear** only.
- Talent build data generated by `scripts/generate_talent_builds.py` from DBC files.
- `LearnRandomTalents()` kept as legacy fallback but no longer the primary path.

**Warlock pet spells**: `TeachWarlockPetSpells()` in `CombatBotBaseAI.cpp` teaches all grimoire-learned abilities (Blood Pact, Fire Shield, Torment, Suffering, Sacrifice, Seduction, Spell Lock, etc.) to the pet via `Pet::AddSpell()` with `ACT_ENABLED` for auto-cast. Keyed by creature entry (416=Imp, 1860=VW, 1863=Succubus, 417=Felhunter) and owner level. Called once per pet instance from `BehaviorWarlockPetSummon`, tracked by `m_lastTaughtPetGuid`. Warlock pet summoning is handled by the OOC behavior set so it can check `FindPartyTank()` — Voidwalker is excluded from the summon pool when a tank is present.

**Warlock curse overwrite prevention**: Only one curse per warlock can be active on a target. `BehaviorWarlockCurseManage` checks `HasAura()` for all known curse spell pointers in `Score()` — returns 0 if any curse is already applied. Priority: Tongues (casters) > Elements > Shadow > Agony. Curse of Recklessness removed (increases mob attack power). Fear also removed (too risky in dungeons).

**Account password limit**: WoW 1.12.1 client enforces a 16-character password maximum. The server's `.account set password` command rejects passwords longer than 16 characters. SRP6 auth (v/s fields in `realmd.account`) cannot be computed externally — must use the mangosd console with `Console.Enable = 1`.

Research docs in `docs/` (gitignored): `CLASS_GROUP_BEHAVIOR_GUIDE.md`, `DUNGEON_RAID_GUIDE.md`, `bot_class_behavior_instructions.md`

### Bot Social & Services Architecture

Bot social behavior and tradeskill services are managed by two singletons:
- `BotChatManager` — trade/guild/general chat, personality system, spam control, party chat, trade chat monitoring
- `BotServiceMgr` — stateful service interactions (portal, enchant, transmute, mooncloth, WTB buying)

**Personality system**: Guild bots have archetypes (enthusiastic, chill, sarcastic, quiet) stored in `guild_bot_roster.personality`. Affects chat style, message pool selection, and frequency (quiet bots speak 30% as often). World/city bots use generic personality.

**Spam control**: Per-bot cooldowns by chat type (`BotChatCooldown`). Frequency scales up when bot count exceeds threshold. Cooldown types: trade, guild, party, say, general.

**Party/raid chat**: Bots greet on group join (2-5s delay), say "gratz" on level-up, react to boss kills + emote /cheer, react to wipes + /cry, call out OOM and pulling.

**General channel chat**: Zone-population-driven. Rate = `botCount × 0.09` msgs/min per zone (~4-5/min in IF at peak). `UpdateGeneralChat()` runs on the `WUPDATE_AUCTIONS` timer (60s interval) — **not** every world tick. Channel name resolved dynamically ("General - Ironforge").

**Conversational threading** (#24): General chat uses a layered system instead of isolated random messages:
- **8 topic pools** (LFG, gold, gear, bored, newbie, server, PVP, professions) — consecutive messages on the same topic carry forward up to 4 lines / 2 minutes before rotating
- **15 conversation chains**: scripted 2-4 bot exchanges (lfg BRD, gold farming, ony attune, etc.) — 12% chance per message slot
- **7 debate templates**: classic WoW arguments (fury vs arms, horde vs alliance racials, best gold farm, etc.) with variant lines — 8% chance per message slot
- **19 reactive keyword hooks**: `Channel::Say` hook fires `OnGeneralChatMessage()` for human General chat messages; bots respond 15–45s later to keywords like "brd", "ony", "pvp", "gold", class names, etc.
- **Scheduling**: All threaded messages use `PendingGeneralMessage::fireTime` (absolute `time_t`), **not** a ms countdown. This is critical because `Update()` fires every 60s — a countdown would collapse all chain lines into one burst. Always use `time_t` for any deferred message scheduling in this system.
- **Per-zone state**: `ZoneConversationState` tracks active topic/chain/debate independently per city. Chains abort cleanly if a participant logs out mid-conversation.

**Trade chat monitoring**: Hooks `Channel::Say()` to detect player service requests ("WTB port", "LF enchanter"). Bot whispers player proactively with location + minimap marker (`SMSG_GOSSIP_POI`). Creates `BotServiceRequest` with destination/enchant parsed from trade message. Only picks bots in the player's same zone. One response per player at a time (`HasActiveRequest` check).

**Service flow**: Player posts in trade → bot whispers with minimap marker → player walks to bot → opens trade → puts gold + item → accepts → bot accepts (direct `HandleAcceptTradeOpcode` call, no packet queue) → service delivered.

**Enchant delivery**: Applied directly to player's equipped weapon via `Item::SetEnchantment()` after trade completes. Does NOT use the trade window spell system (which causes accept loops due to `SetSpell` clearing both sides' accept state). `ForceSpell()` added to `TradeData` for cases where spell display is needed without side effects.

**Portal delivery**: Cast as triggered spell (`CastSpell` with `triggered=true`) to bypass spell-known checks. Portal party restriction removed in `GameObject.cpp` so any player can use bot portals.

**WTB buying** (#13): Bots follow through on WTB trade messages. 15-item registry with AH fair market pricing (10% discount). Gold budget 200g/day per bot.

**Service request lifecycle**: 5-minute timeout, cleared on player logout (`WorldSession::LogoutPlayer`), duplicate prevention per player.

**Config** (`mangosd.conf`): `BotChat.Enable`, `BotChat.TradeChat.IntervalMin/Max`, `BotChat.Cooldown.*`, `BotService.Enable`, `BotService.Buy.BudgetPerDay`

### Auction House Bot

Realistic AH pricing model in `AuctionHouseBotMgr`:
- **Farmability-weighted selection**: items appear proportional to how farmable they are in-game
- **Category quotas**: trade goods 35-40%, crafting 15-20%, consumables 20-25%, recipes 3-5%, equipment 5-10%, leveling 10-15%
- **Simulated sellers**: 12 fake seller identities (account 99) with 4 archetypes (casual/dedicated/crafter/opportunist), each with distinct pricing behavior
- **Price jitter**: ±10-20% per listing around base price
- **72-hour refresh cycle**: full wipe + re-simulation of all bot listings
- **Bot buyers**: scan player auctions every 30min, purchase probability based on price vs fair market value (0% for absurd prices, 80-95% for fair prices), 50g budget per cycle

**Config** (`mangosd.conf`): `AHBot.Enable`, `AHBot.itemcount`, `AHBot.RefreshInterval`, `AHBot.Buyer.*`
**DB table**: `world.auctionhousebot` (item, min_stack, max_stack, bid, buyout, weight, category, price_variance_pct)

## Bot Director

C++ singleton (`BotDirector`) in `vmangos/src/game/PlayerBots/BotDirector.cpp`. Runs a placement cycle every 2 minutes (configurable via `BotDirector.UpdateInterval`). Scans all online human players (account ID < 100), categorizes them by zone, and assigns dynamic bots from GUID pools.

**GUID pools** (shuffled each cycle):
- Horde: GUIDs 33-52 (20 bots)
- Alliance city: GUIDs 53-72 (20 bots)
- Alliance world: GUIDs 73-92 (20 bots)

**Capital Cities** (Stormwind, Ironforge):
- 5 always-on Alliance city bots per capital (even with no players)
- +5 extra city bots per human player in the city
- Level distribution: 40% lvl 60, 30% lvl 40-59, 30% lvl 10-39
- Passive, 80yd spawn radius

**Friendly Zones** (7 zones: Elwynn, Westfall, Loch Modan, Redridge, Darkshore, Duskwood, Wetlands):
- 5 Alliance world bots per human player
- Level = random within zone's level range (e.g. 1-10 for Elwynn)
- Passive, 150yd spawn radius

**Contested Zones** (16 zones: Hillsbrad through Silithus, including Un'Goro, Desolace, Stonetalon):
- Up to 5 Horde bots per human player — **aggressive**, 10% chance each is a lvl 60 ganker, otherwise random within zone range
- Up to 3 Alliance bots per human player — passive, 10% chance of lvl 60 "hero", otherwise random within zone range
- 200yd spawn radius

**Spawn mechanism**: After building assignments, `UpdateBotChances()` sets `chance = 30` in the `playerbot` DB table for assigned GUIDs, `chance = 1` for unassigned. `PlayerBotMgr` reads these to decide which bots log in.

**DynamicBotAI behavior**: Reads assignment from BotDirector. Wanders within assigned radius. Aggressive bots search for enemy players within 30yd and engage (melee chase or caster kite). Auto-resurrects 30s after death. Reloads assignment every 2 minutes.

**Config** (`mangosd.conf`): `BotDirector.Enable`, `BotDirector.UpdateInterval`

### Guild Bot System

Persistent guild "Sweatlords" with 20 bots (expandable to 40) managed by `GuildBotMgr` singleton:
- **Roster**: 2 tanks, 5 healers, 13 DPS across all classes, stored in `guild_bot_roster` table
- **Personality**: each bot has an archetype (enthusiastic/chill/sarcastic/quiet) in `guild_bot_roster.personality`
- **Gear persistence**: bots equip and save gear via standard `character_inventory` / `item_instance` tables
- **Spec-aware upgrades**: `guild_bot_spec_weights` table drives item comparison using stat weights per class/spec
- **Loot integration**: guild bots roll Need on upgrades during Group Loot, but always defer to human players. Never roll Greed.
- **Raid lockouts**: vanilla-authentic — guild bots get locked to raid instances like real players
- **Guild chat**: bots chat in /g on a timer (5-15min configurable), personality-flavored messages
- **Level sync**: `SyncBotLevels()` runs every 5min, sets guild bots to GM's level ±3

**Guild bot account**: 98, GUIDs 200-219

**Online presence system**: Guild bots simulate login/logout on a cosine time-of-day curve.
- Spawns as `CrowdBotAI` at IF bank front (idle presence, no combat)
- Peak: 14 bots at 8pm, trough: 3 bots at 8am (configurable, UTC-4)
- Full target count spawned immediately on first presence tick (no ramp-up)
- Presence bots appear in guild panel and participate in guild chat
- Skips raid-assembled bots (under `.guildraid` control)

**Party invite conversion**: When a player invites a guild presence bot to a group:
1. Bot auto-accepts the invite (hooked in `HandleGroupInviteOpcode`)
2. CrowdBotAI despawned, respawned as PartyBotAI with combat AI, gear, and role from roster
3. On group leave/disband, bot despawns and presence system respawns them next cycle
- Low-level guild bots (<19) get random gear; higher levels get premade templates
- `PlayerBotMgr::RemoveBotEntry()` used for clean AI swaps without session conflicts

**In-game commands** (SEC_MODERATOR):
- `.guildraid assemble` — spawn all raid-assigned guild bots
- `.guildraid disband` — dismiss all guild bots
- `.guildraid check` — show raid composition and buff coverage
- `.guildraid roster` — list all guild bots with class, spec, role, gear score
- `.guildraid botinfo <name>` — inspect a specific bot

**Config** (`mangosd.conf`): `GuildBot.Enable`, `GuildBot.AccountId`, `BotChat.GuildChat.IntervalMin/Max`, `GuildBot.Presence.*`

### Crowd Bot System

Lightweight "scenery" bots that stand around city hotspots and inns to make the world feel populated. Uses `CrowdBotAI` — a minimal `PlayerBotAI` subclass with idle emotes, micro-wander, and trade service handling.

- **Account**: 97
- **GUID ranges**: IF bank front 100-169 (50 bots), IF bank interior 170-174 (5), IF AH 175-189 (15), SW AH 125-134 (10), SW Bank 135-139 (5), Goldshire 140-144 (5), Inn bots 300-371 (72)
- **AI**: `CrowdBotAI` — 5s update tick, idle emotes every 15-60s (talk, point, wave, eat — no laugh), 5% chance of 3yd micro-wander per tick
- **Spawning**: Always-on, not BotDirector-managed. Hardcoded spawn points in `PlayerBotAI.cpp`
- **Appearance**: Preraid-bis dungeon blues (not T3), weighted 70% lvl 60 / 30% lvl 40-59, Alliance races. ~25% of lvl 60 bots sit on race-appropriate epic mounts.
- **Guild membership**: 15 bot guilds across 3 archetypes (hardcore: Oblivion, Nihilum, etc; social: Casually Late, AFK In Ironforge, etc; mix: Dawn of Azeroth, Silver Covenant, etc). ~10 members each.
- **Trade services**: CrowdBots handle `SMSG_TRADE_STATUS` via `OnPacketReceived` — accept trades for pending service requests, auto-accept after player accepts via direct `HandleAcceptTradeOpcode` call.
- **Time-of-day scaling**: Bot count scales with cosine curve — 50% of max at off-peak (8am), 100% at peak (8pm). Configurable via `RandomBot.PeakHour`, `RandomBot.OffPeakPercent`, `RandomBot.TimezoneOffset`.

**Key design constraint**: Creatures cannot render full player gear — `PLAYER_VISIBLE_ITEM` fields (armor, clothing) are Player-only in the update field system. Crowd bots must be real `Player` objects to look correct. Cost is ~5KB RAM and negligible CPU each.

**Spawn coordinate tuning**: Update coords in `PlayerBotAI.cpp` (`CITY_SPAWNS[]` or `INN_LOCATIONS[]`) and rebuild. IF bank front center: (-4919.8, -958.6, 501.5). IF bank interior: (-4891.7, -990.8, 503.9). IF AH: (-4959.9, -906.4, 503.8).

**Config** (`mangosd.conf`): `CrowdBot.Enable`, `CrowdBot.AccountId`, `RandomBot.MinBots/MaxBots`, `RandomBot.PeakHour`, `RandomBot.OffPeakPercent`, `RandomBot.TimezoneOffset`

## Dashboard API

| Endpoint | Description |
|----------|-------------|
| `GET /` | Web UI |
| `GET /guild` | Guild roster editor and raid planner |
| `GET /api/status` | Server status, uptime, player/bot counts |
| `GET /api/players` | Online players with stats |
| `GET /api/characters` | All characters (paginated) |
| `GET /api/items` | Item analytics |
| `GET /api/guilds` | Guild info |
| `GET /api/raids` | Raid analytics |
| `GET /api/guild/roster` | Guild bot roster JSON |
| `GET /api/guild/bot/<guid>` | Guild bot detail with gear |
| `POST /api/guild/bot/<guid>/spec` | Update bot spec/role |
| `POST /api/guild/raid-plan` | Save raid assignments |
| `GET /addons` | Addon listing page |
| `GET /addons/<addon_id>` | Addon detail + release downloads |
| `GET /api/addons` | List all addons (JSON) |
| `POST /api/addons` | Create new addon (admin) |
| `POST /api/addons/<id>/releases` | Upload release zip (admin) |

Player vs bot distinction: account ID < 100 = human, >= 100 = bot.

## Commits and Versioning

Uses [commitizen](https://commitizen-tools.github.io/commitizen/) conventional commit format:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`

**Scopes**: `vmangos`, `dashboard`, `discord-bot`, `scripts`, `db`

**Version bumps** (current: v0.4.x):
- `fix` commit → patch bump (+0.0.1), git tag, Discord release notes
- `feat` commit → minor bump (+0.1.0), git tag, Discord release notes
- All other types → no version bump
- Major bumps only when explicitly requested

**Releasing**:
```bash
scripts/release.sh              # auto-detect bump from commits
scripts/release.sh patch        # force patch bump
scripts/release.sh minor        # force minor bump
scripts/release.sh --dry-run    # preview without doing anything
```

The script tags, pushes to GitHub, creates a GitHub Release with notes, and posts release notes to the Discord #patch-notes channel.

## Common Commands

```bash
# Server status
pgrep mangosd && echo "World OK" || echo "World DOWN"
pgrep realmd && echo "Realm OK" || echo "Realm DOWN"

# Python services
sudo systemctl status azeroth-dashboard azeroth-discord-bot
sudo systemctl restart azeroth-dashboard

# Logs
tail -f ~/wow-private-server/vmangos/run/bin/Server.log
journalctl -u azeroth-discord-bot -f

# Database (password from .env)
mysql -u mangos -p characters

# Rebuild vmangos
cd ~/wow-private-server/vmangos/build && make -j$(nproc) && make install

# Manual DB backup
~/wow-private-server/scripts/backup-db.sh
```
