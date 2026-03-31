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
├── bot-director/                  ← dynamic bot placement service
│   ├── bot_director.py
│   ├── requirements.txt
│   └── venv/                      (gitignored)
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
| Bot Director | `bot-director.service` | `bot-director/bot_director.py` |

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
- Boss Override Registry: interrupt spell IDs, spread mechanics, AoE avoidance for 20+ bosses
- DPS threat throttling and combat opener logic

### PartyBot AI Architecture

Bot AI is split into per-class files for independent iteration:
- `PartyBotAI.cpp` — core dispatch, boss overrides, shared utilities (1464 lines)
- `PartyBotAI_<Class>.cpp` — one file per class (9 files)
- `CombatBotBaseAI.h` — spell pointer structs per class
- `CombatBotBaseAI.cpp` — spell population by name, totem/heal helpers

**Class overhaul status** (tracked in GitHub issue #1):
- **Warrior**: done (v0.4 — Protection rewrite, Revenge, Challenging Shout)
- **Shaman**: done (Resto/Enhance/Elemental, context-aware totems, NS emergency)
- **Priest**: done (HP-threshold healing, Prayer of Healing, Shadow threat awareness)
- **Paladin**: done (intelligent blessing selection, Holy/Prot/Ret role-aware, Cleanse priority)
- **Hunter**: done (shot weaving, Freezing Trap CC, Feign Death threat, Aspect swapping)
- **Mage**: done (Counterspell priority, Polymorph CC maintenance, Evocation safety)
- **Warlock**: done (curse priority system, Life Tap safety, Banish CC, no Searing Pain)
- **Rogue**: done (deterministic SnD>Evisc finishers, Feint threat, Kick priority)
- **Druid**: done (Rebirth combat rez, Innervate on healers, Bear offtank, Faerie Fire uptime)

Research docs in `docs/` (gitignored): `CLASS_GROUP_BEHAVIOR_GUIDE.md`, `DUNGEON_RAID_GUIDE.md`, `bot_class_behavior_instructions.md`

### Bot Social & Services Architecture

Bot social behavior and tradeskill services are managed by two singletons:
- `BotChatManager` — trade chat advertising (rotation-based, frequency-scaled) and keyword-matched whisper responses
- `BotServiceMgr` — stateful service interactions (portal, enchant, transmute, mooncloth)

**Service flow**: player whispers bot → bot asks clarifying questions → bot says "trade me" → player initiates trade → bot auto-accepts → service delivered (portal cast, enchant applied, etc.)

**Config** (`mangosd.conf`): `BotChat.Enable`, `BotChat.TradeChat.IntervalMin/Max`, `BotService.Enable`

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

Runs every 2 minutes. Assigns bots to zones based on player locations:
- **Capital Cities** (Stormwind, Ironforge): wide level range (10-60), passive
- **Friendly Zones** (7 zones): player_level +/- 5, same faction, passive
- **Contested Zones** (13 zones): both factions, aggressive enemies, 10% chance of lvl 60
- No bots when no players are online
- Bot GUIDs: Horde 33-52, Alliance city 53-72, Alliance world 73-92

### Guild Bot System

Persistent guild "Vanguard of Azeroth" with 20 bots (expandable to 40) managed by `GuildBotMgr` singleton:
- **Roster**: 2 tanks, 5 healers, 13 DPS across all classes, stored in `guild_bot_roster` table
- **Gear persistence**: bots equip and save gear via standard `character_inventory` / `item_instance` tables
- **Spec-aware upgrades**: `guild_bot_spec_weights` table drives item comparison using stat weights per class/spec
- **Loot integration**: guild bots roll Need on upgrades during Group Loot, but always defer to human players. Never roll Greed.
- **Raid lockouts**: vanilla-authentic — guild bots get locked to raid instances like real players
- **Guild chat**: bots chat in /g on a timer (5-15min configurable) with idle banter

**Guild bot account**: 98, GUIDs 200-219

**In-game commands** (SEC_MODERATOR):
- `.guildraid assemble` — spawn all raid-assigned guild bots
- `.guildraid disband` — dismiss all guild bots
- `.guildraid check` — show raid composition and buff coverage
- `.guildraid roster` — list all guild bots with class, spec, role, gear score
- `.guildraid botinfo <name>` — inspect a specific bot

**Config** (`mangosd.conf`): `GuildBot.Enable`, `GuildBot.AccountId`, `BotChat.GuildChat.IntervalMin/Max`

### Crowd Bot System

Lightweight "scenery" bots that stand around city hotspots and inns to make the world feel populated. Uses `CrowdBotAI` — a minimal `PlayerBotAI` subclass with no combat, no spells, just idle emotes and micro-wander.

- **Account**: 97
- **GUID ranges**: City bots 100-144, Inn bots 300-371 (intentionally skips 200-219 guild bots)
- **AI**: `CrowdBotAI` — 5s update tick, idle emotes every 15-60s, 5% chance of 3yd micro-wander per tick
- **Spawning**: Always-on, not BotDirector-managed. Hardcoded spawn points in `PlayerBotAI.cpp`
- **Appearance**: Premade gear templates (weighted 70% lvl 60, 30% lvl 40-59), Alliance races
- **City hotspots** (100-144, 45 bots): Ironforge AH (15), IF Bank (10), Stormwind AH (10), SW Bank (5), Goldshire Inn (5)
- **Inn bots** (300-371, 72 bots): 3 bots at 24 major Alliance/neutral inns across both continents
- **Non-interactive**: no combat, no whisper responses, no trades

**Key design constraint**: Creatures cannot render full player gear — `PLAYER_VISIBLE_ITEM` fields (armor, clothing) are Player-only in the update field system. Crowd bots must be real `Player` objects to look correct. Cost is ~5KB RAM and negligible CPU each.

**Spawn coordinate tuning**: z-height may need adjustment for inns on upper floors or near terrain transitions. Update coords in `PlayerBotAI.cpp` (`CITY_SPAWNS[]` or `INN_LOCATIONS[]`) and rebuild.

**Config** (`mangosd.conf`): `CrowdBot.Enable`, `CrowdBot.AccountId`

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

Player vs bot distinction: account ID < 100 = human, >= 100 = bot.

## Commits and Versioning

Uses [commitizen](https://commitizen-tools.github.io/commitizen/) conventional commit format:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`

**Scopes**: `vmangos`, `dashboard`, `discord-bot`, `bot-director`, `scripts`, `db`

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
sudo systemctl status azeroth-dashboard azeroth-discord-bot bot-director
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
