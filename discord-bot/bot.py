import discord
import asyncio
import subprocess
import json
import os
import re
import mysql.connector
from datetime import datetime, timezone

# ==================== CONFIG ====================
TOKEN = os.environ.get("DISCORD_BOT_TOKEN", "")

CHANNEL_PATCH_NOTES = 1485324341062402239
CHANNEL_PLAYER_ACTIVITY = 1485324380832665610
CHANNEL_SERVER_STATUS = 1485324622177108078

DB_CONFIG = {
    "host": "127.0.0.1",
    "user": "mangos",
    "password": "fo2IquNCzghd0qBRYetHUtQw",
    "port": 3306,
}

LEVELUP_LOG = "/home/wow/vmangos/run/bin/LevelUp.log"

# How often to check (seconds)
STATUS_CHECK_INTERVAL = 60
PLAYER_CHECK_INTERVAL = 30

# ==================== BOT ====================
intents = discord.Intents.default()
intents.message_content = True
client = discord.Client(intents=intents)

# Track state
last_server_status = None
known_online_players = {}

def is_server_running():
    """Check if mangosd process is running."""
    try:
        result = subprocess.run(["pgrep", "mangosd"], capture_output=True, text=True)
        return result.returncode == 0
    except Exception:
        return False

def get_db_connection(database):
    """Get a MySQL connection."""
    config = DB_CONFIG.copy()
    config["database"] = database
    return mysql.connector.connect(**config)

def get_online_players():
    """Query characters DB for online players."""
    try:
        conn = get_db_connection("characters")
        cursor = conn.cursor(dictionary=True)
        cursor.execute("""
            SELECT c.guid, c.name, c.level, c.race, c.class as cls, c.online, a.username
            FROM characters c
            JOIN realmd.account a ON c.account = a.id
            WHERE c.account < 100
        """)
        players = cursor.fetchall()
        cursor.close()
        conn.close()
        return players
    except Exception as e:
        print(f"DB Error: {e}")
        return []

def get_race_name(race_id):
    races = {
        1: "Human", 2: "Orc", 3: "Dwarf", 4: "Night Elf",
        5: "Undead", 6: "Tauren", 7: "Gnome", 8: "Troll"
    }
    return races.get(race_id, "Unknown")

def get_class_name(class_id):
    classes = {
        1: "Warrior", 2: "Paladin", 3: "Hunter", 4: "Rogue",
        5: "Priest", 7: "Shaman", 8: "Mage", 9: "Warlock",
        11: "Druid"
    }
    return classes.get(class_id, "Unknown")

CLASS_RACE_MAP = {
    "c1": "Warrior", "c2": "Paladin", "c3": "Hunter", "c4": "Rogue",
    "c5": "Priest", "c7": "Shaman", "c8": "Mage", "c9": "Warlock", "c11": "Druid",
    "r1": "Human", "r2": "Orc", "r3": "Dwarf", "r4": "Night Elf",
    "r5": "Undead", "r6": "Tauren", "r7": "Gnome", "r8": "Troll",
}

def get_class_emoji(class_id):
    emojis = {
        1: "⚔️", 2: "🛡️", 3: "🏹", 4: "🗡️",
        5: "✨", 7: "🌩️", 8: "🔥", 9: "😈",
        11: "🌿"
    }
    return emojis.get(class_id, "❓")

def get_class_emoji_str(class_str):
    mapping = {"c1": 1, "c2": 2, "c3": 3, "c4": 4, "c5": 5, "c7": 7, "c8": 8, "c9": 9, "c11": 11}
    return get_class_emoji(mapping.get(class_str, 0))

def get_faction_emoji(race_id):
    alliance_races = {1, 3, 4, 7}
    return "🔵" if race_id in alliance_races else "🔴"

def get_faction_emoji_str(race_str):
    alliance = {"r1", "r3", "r4", "r7"}
    return "🔵" if race_str in alliance else "🔴"

# ==================== TASKS ====================

async def server_status_loop():
    """Monitor server status and post changes."""
    global last_server_status
    await client.wait_until_ready()
    channel = client.get_channel(CHANNEL_SERVER_STATUS)
    if not channel:
        print(f"Could not find server status channel {CHANNEL_SERVER_STATUS}")
        return

    while not client.is_closed():
        try:
            running = is_server_running()
            if last_server_status is not None and running != last_server_status:
                now = datetime.now(timezone.utc)
                if running:
                    embed = discord.Embed(
                        title="🟢 Server Online",
                        description="Azeroth is back online! Connect and play.",
                        color=0x00FF00,
                        timestamp=now,
                    )
                else:
                    embed = discord.Embed(
                        title="🔴 Server Offline",
                        description="Azeroth has gone offline.",
                        color=0xFF0000,
                        timestamp=now,
                    )
                embed.set_footer(text="Azeroth Private Server")
                await channel.send(embed=embed)
            last_server_status = running
        except Exception as e:
            print(f"Status check error: {e}")
        await asyncio.sleep(STATUS_CHECK_INTERVAL)

async def levelup_log_watcher():
    """Watch LevelUp.log for instant level-up announcements."""
    await client.wait_until_ready()
    channel = client.get_channel(CHANNEL_PLAYER_ACTIVITY)
    if not channel:
        print(f"Could not find player activity channel {CHANNEL_PLAYER_ACTIVITY}")
        return

    # Seek to end of file on startup so we don't re-announce old levels
    try:
        with open(LEVELUP_LOG, "r") as f:
            f.seek(0, 2)  # Seek to end
            file_pos = f.tell()
    except FileNotFoundError:
        file_pos = 0

    # Pattern: Character Name:guid [cX rY] reaches level  Z
    pattern = re.compile(
        r"Character (\w+):\d+ \[(c\d+) (r\d+)\] reaches level\s+(\d+)"
    )

    while not client.is_closed():
        try:
            with open(LEVELUP_LOG, "r") as f:
                f.seek(file_pos)
                new_lines = f.readlines()
                file_pos = f.tell()

            for line in new_lines:
                # Skip bot level-ups
                if '<BOT>' in line:
                    continue
                match = pattern.search(line)
                if match:
                    name = match.group(1)
                    class_str = match.group(2)
                    race_str = match.group(3)
                    level = int(match.group(4))

                    cls_name = CLASS_RACE_MAP.get(class_str, "Unknown")
                    race_name = CLASS_RACE_MAP.get(race_str, "Unknown")
                    emoji = get_class_emoji_str(class_str)
                    faction = get_faction_emoji_str(race_str)

                    # Milestone levels
                    if level == 60:
                        title = f"🏆👑 {name} reached LEVEL 60! 👑🏆"
                        color = 0xFF8000
                    elif level in (10, 20, 30, 40, 50):
                        title = f"🎉 {name} hit level {level}!"
                        color = 0xFFD100
                        if level == 40:
                            title = f"🐴 {name} hit level {level}! Mount time!"
                    else:
                        title = f"⬆️ {name} reached level {level}"
                        color = 0x0070DD

                    embed = discord.Embed(
                        title=title,
                        description=f"{faction} {emoji} {race_name} {cls_name}",
                        color=color,
                        timestamp=datetime.now(timezone.utc),
                    )
                    embed.set_footer(text="Azeroth Private Server")
                    await channel.send(embed=embed)
                    print(f"Level-up announced: {name} -> {level}")

        except FileNotFoundError:
            pass
        except Exception as e:
            print(f"LevelUp log watcher error: {e}")

        await asyncio.sleep(5)  # Check every 5 seconds

async def player_activity_loop():
    """Monitor player logins and logouts."""
    global known_online_players
    await client.wait_until_ready()
    channel = client.get_channel(CHANNEL_PLAYER_ACTIVITY)
    if not channel:
        print(f"Could not find player activity channel {CHANNEL_PLAYER_ACTIVITY}")
        return

    first_run = True
    while not client.is_closed():
        try:
            players = get_online_players()
            current_online = {}
            for p in players:
                guid = p["guid"]
                if p["online"]:
                    current_online[guid] = p

            if not first_run:
                now = datetime.now(timezone.utc)
                # Check for logins
                for guid, player in current_online.items():
                    if guid not in known_online_players:
                        race = get_race_name(player["race"])
                        cls = get_class_name(player["cls"])
                        emoji = get_class_emoji(player["cls"])
                        faction = get_faction_emoji(player["race"])
                        embed = discord.Embed(
                            title=f"{faction} {player['name']} has logged in",
                            description=f"{emoji} Level {player['level']} {race} {cls}",
                            color=0x1EFF00,
                            timestamp=now,
                        )
                        embed.set_footer(text="Azeroth Private Server")
                        await channel.send(embed=embed)

                # Check for logouts
                for guid, player in known_online_players.items():
                    if guid not in current_online:
                        race = get_race_name(player["race"])
                        cls = get_class_name(player["cls"])
                        emoji = get_class_emoji(player["cls"])
                        embed = discord.Embed(
                            title=f"👋 {player['name']} has logged out",
                            description=f"{emoji} Level {player['level']} {race} {cls}",
                            color=0x9D9D9D,
                            timestamp=now,
                        )
                        embed.set_footer(text="Azeroth Private Server")
                        await channel.send(embed=embed)

            first_run = False
            known_online_players = current_online
        except Exception as e:
            print(f"Player activity error: {e}")
        await asyncio.sleep(PLAYER_CHECK_INTERVAL)

# ==================== COMMANDS ====================
@client.event
async def on_ready():
    print(f"Bot connected as {client.user}")
    channel = client.get_channel(CHANNEL_SERVER_STATUS)
    if channel:
        running = is_server_running()
        status = "🟢 Online" if running else "🔴 Offline"
        embed = discord.Embed(
            title=f"Bot Started — Server is {status}",
            description="Monitoring server status, player activity, and patch notes.",
            color=0x00FF00 if running else 0xFF0000,
            timestamp=datetime.now(timezone.utc),
        )
        embed.set_footer(text="Azeroth Private Server")
        await channel.send(embed=embed)

    # Start background tasks
    client.loop.create_task(server_status_loop())
    client.loop.create_task(player_activity_loop())
    client.loop.create_task(levelup_log_watcher())

@client.event
async def on_message(message):
    if message.author == client.user:
        return

    if message.content.strip() == "!status":
        running = is_server_running()
        players = get_online_players()
        online = [p for p in players if p["online"]]
        if running:
            desc = f"**Players online:** {len(online)}"
            if online:
                desc += "\n"
                for p in online:
                    emoji = get_class_emoji(p["cls"])
                    faction = get_faction_emoji(p["race"])
                    desc += f"\n{faction} {emoji} **{p['name']}** — Level {p['level']} {get_race_name(p['race'])} {get_class_name(p['cls'])}"
            embed = discord.Embed(
                title="🟢 Azeroth is Online",
                description=desc,
                color=0x00FF00,
                timestamp=datetime.now(timezone.utc),
            )
        else:
            embed = discord.Embed(
                title="🔴 Azeroth is Offline",
                description="The server is currently down.",
                color=0xFF0000,
                timestamp=datetime.now(timezone.utc),
            )
        embed.set_footer(text="Azeroth Private Server")
        await message.channel.send(embed=embed)

    if message.content.strip().startswith("!patchnotes"):
        content = message.content[len("!patchnotes"):].strip()
        if not content:
            await message.channel.send("Usage: `!patchnotes <your patch note text>`")
            return
        channel = client.get_channel(CHANNEL_PATCH_NOTES)
        if channel:
            embed = discord.Embed(
                title="📋 Patch Notes",
                description=content,
                color=0xFFD100,
                timestamp=datetime.now(timezone.utc),
            )
            embed.set_footer(text="Azeroth Private Server")
            await channel.send(embed=embed)
            if message.channel.id != CHANNEL_PATCH_NOTES:
                await message.channel.send("✅ Patch notes posted!")

# ==================== RUN ====================
if __name__ == "__main__":
    if not TOKEN:
        print("ERROR: Set DISCORD_BOT_TOKEN environment variable")
        exit(1)
    client.run(TOKEN)
