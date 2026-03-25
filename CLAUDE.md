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
- Warrior AI: Revenge, Challenging Shout, Protection Warrior rewrite
- Boss Override Registry: interrupt spell IDs, spread mechanics, AoE avoidance for 20+ bosses
- DPS threat throttling and combat opener logic

## Bot Director

Runs every 2 minutes. Assigns bots to zones based on player locations:
- **Capital Cities** (Stormwind, Ironforge): wide level range (10-60), passive
- **Friendly Zones** (7 zones): player_level +/- 5, same faction, passive
- **Contested Zones** (13 zones): both factions, aggressive enemies, 10% chance of lvl 60
- No bots when no players are online
- Bot GUIDs: Horde 33-52, Alliance city 53-72, Alliance world 73-92

## Dashboard API

| Endpoint | Description |
|----------|-------------|
| `GET /` | Web UI |
| `GET /api/status` | Server status, uptime, player/bot counts |
| `GET /api/players` | Online players with stats |
| `GET /api/characters` | All characters (paginated) |
| `GET /api/items` | Item analytics |
| `GET /api/guilds` | Guild info |
| `GET /api/raids` | Raid analytics |

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

The script tags, then posts release notes to the Discord #patch-notes channel. Push tags after: `git push origin main --tags`

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
