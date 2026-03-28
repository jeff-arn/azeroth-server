# Azeroth Server

A VMaNGOS-based World of Warcraft 1.12.1 private server with intelligent bot AI, a live web dashboard, and Discord integration.

## Features

### PartyBot AI
Class-aware bot companions that can tank, heal, and DPS in dungeons and raids. Each of the nine classes has hand-tuned behavior — threat management, crowd control, cooldown usage, and boss-specific overrides for 20+ encounters.

### Guild System
A persistent guild ("Vanguard of Azeroth") with 20 bots that gear up over time, respect raid lockouts, and can be assembled on demand with `.guildraid` commands.

### Auction House Economy
A realistic AH simulation with 12 seller personas across 4 archetypes, farmability-weighted item selection, category quotas, price jitter, and a buyer agent that purchases fairly-priced player listings.

### Bot Director
A dynamic placement service that spawns bots near active players — friendly in safe zones, mixed-faction and occasionally aggressive in contested territory. No bots when nobody's online.

### Bot Social Behavior
Bots advertise tradeskill services in trade chat and respond to whispers. Players can request portals, enchants, transmutes, and more through a conversational trade flow.

### Web Dashboard
A live dashboard showing server status, online players, character analytics, guild roster management, and raid planning.

### Discord Bot
Server status updates, player activity feeds, and automated patch notes posted to Discord.

## Architecture

```
vmangos/              C++ game server (mangosd + realmd)
bot-director/         Python — dynamic bot placement service
dashboard/            Python (Flask) — web UI and REST API
discord-bot/          Python — Discord integration
scripts/              Shell — backups, releases
vmangos-database/     Initial DB imports
```

## Quick Start

### Prerequisites
- Linux host
- MySQL 5.7+
- Python 3.8+
- CMake + GCC (to build vmangos)
- [GitHub CLI](https://cli.github.com/) (`gh`) for releases

### Setup

1. **Clone and configure**
   ```bash
   git clone https://github.com/jeff-arn/azeroth-server.git ~/wow-private-server
   cd ~/wow-private-server
   cp .env.example .env        # fill in DB_PASSWORD and DISCORD_BOT_TOKEN
   ```

2. **Build the game server**
   ```bash
   cd vmangos/build
   cmake .. -DCMAKE_BUILD_TYPE=Release
   make -j$(nproc)
   make install
   ```

3. **Configure the game server**
   ```bash
   cp vmangos/run/etc/mangosd.conf.example vmangos/run/etc/mangosd.conf
   cp vmangos/run/etc/realmd.conf.example vmangos/run/etc/realmd.conf
   # Edit both .conf files with your DB credentials
   ```

4. **Set up Python services**
   ```bash
   for dir in bot-director dashboard discord-bot; do
       python3 -m venv $dir/venv
       $dir/venv/bin/pip install -r $dir/requirements.txt
   done
   ```

5. **Import databases**
   ```bash
   # Import the world, characters, realmd, and logs schemas
   # See vmangos-database/ and vmangos/sql/ for dumps and migrations
   ```

6. **Start services**
   ```bash
   # Game server
   sudo systemctl start azeroth-realmd azeroth-mangosd

   # Supporting services
   sudo systemctl start azeroth-dashboard azeroth-discord-bot bot-director
   ```

## Configuration

All secrets are stored in `.env` (gitignored). Game server config lives in `vmangos/run/etc/*.conf` (also gitignored) — use the `.conf.example` files as templates.

Key toggles in `mangosd.conf`:

| Setting | Description |
|---------|-------------|
| `AHBot.Enable` | Auction House bot simulation |
| `GuildBot.Enable` | Persistent guild bot system |
| `BotChat.Enable` | Bot trade chat and whisper responses |
| `BotService.Enable` | Bot tradeskill services (portals, enchants, etc.) |

## API

The dashboard exposes a REST API at `http://localhost:5000`:

| Endpoint | Description |
|----------|-------------|
| `GET /api/status` | Server status, uptime, player/bot counts |
| `GET /api/players` | Online players |
| `GET /api/characters` | All characters (paginated) |
| `GET /api/guilds` | Guild info |
| `GET /api/raids` | Raid analytics |
| `GET /api/guild/roster` | Guild bot roster |

## In-Game Commands

Guild raid management (requires GM rank):

| Command | Description |
|---------|-------------|
| `.guildraid assemble` | Spawn all raid-assigned guild bots |
| `.guildraid disband` | Dismiss all guild bots |
| `.guildraid check` | Show raid composition and buff coverage |
| `.guildraid roster` | List all guild bots with class, spec, role |
| `.guildraid botinfo <name>` | Inspect a specific bot's gear and stats |

## Releasing

```bash
scripts/release.sh              # auto-detect bump from commits
scripts/release.sh patch        # force patch bump
scripts/release.sh minor        # force minor bump
scripts/release.sh --dry-run    # preview without doing anything
```

The script creates a git tag, publishes a GitHub Release, and posts patch notes to Discord.

## Backups

Database backups run daily at 4am UTC via cron with 14-day retention:

```bash
scripts/backup-db.sh            # manual backup
```

## License

Private project. Not for redistribution.
