import discord
import asyncio
import subprocess
import json
import os
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
known_player_levels = {}


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


def get_class_emoji(class_id):
    emojis = {
        1: "⚔️", 2: "🛡️", 3: "🏹", 4: "🗡️",
        5: "✨", 7: "🌩️", 8: "🔥", 9: "😈",
        11: "🌿"
    }
    return emojis.get(class_id, "❓")


def get_faction_emoji(race_id):
    alliance_races = {1, 3, 4, 7}
    return "🔵" if race_id in alliance_races else "🔴"


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


async def player_activity_loop():
    """Monitor player logins, logouts, and level-ups."""
    global known_online_players, known_player_levels
    await client.wait_until_ready()
    channel = client.get_channel(CHANNEL_PLAYER_ACTIVITY)
    if not channel:
        print(f"Could not find player activity channel {CHANNEL_PLAYER_ACTIVITY}")
        return

    # Initial population — don't announce on first check
    first_run = True

    while not client.is_closed():
        try:
            players = get_online_players()
            current_online = {}
            current_levels = {}

            for p in players:
                guid = p["guid"]
                current_levels[guid] = p["level"]

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
                            description=f"{emoji} Level {current_levels.get(guid, player['level'])} {race} {cls}",
                            color=0x9D9D9D,
                            timestamp=now,
                        )
                        embed.set_footer(text="Azeroth Private Server")
                        await channel.send(embed=embed)

                # Check for level-ups
                for guid, new_level in current_levels.items():
                    if guid in known_player_levels:
                        old_level = known_player_levels[guid]
                        if new_level > old_level:
                            player = None
                            for p in players:
                                if p["guid"] == guid:
                                    player = p
                                    break
                            if player:
                                race = get_race_name(player["race"])
                                cls = get_class_name(player["cls"])
                                emoji = get_class_emoji(player["cls"])

                                # Milestone levels get special treatment
                                if new_level in (10, 20, 30, 40, 50, 60):
                                    title = f"🎉 {player['name']} hit level {new_level}!"
                                    color = 0xFFD100  # Gold
                                    if new_level == 60:
                                        title = f"🏆👑 {player['name']} reached LEVEL 60! 👑🏆"
                                        color = 0xFF8000  # Legendary orange
                                elif new_level == 40:
                                    title = f"🐴 {player['name']} hit level {new_level}! Mount time!"
                                    color = 0xFFD100
                                else:
                                    title = f"⬆️ {player['name']} reached level {new_level}"
                                    color = 0x0070DD  # Rare blue

                                embed = discord.Embed(
                                    title=title,
                                    description=f"{emoji} {race} {cls}",
                                    color=color,
                                    timestamp=now,
                                )
                                embed.set_footer(text="Azeroth Private Server")
                                await channel.send(embed=embed)

            first_run = False
            known_online_players = current_online
            known_player_levels = current_levels

        except Exception as e:
            print(f"Player activity error: {e}")

        await asyncio.sleep(PLAYER_CHECK_INTERVAL)


# ==================== COMMANDS ====================

@client.event
async def on_ready():
    print(f"Bot connected as {client.user}")

    # Post startup status
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


@client.event
async def on_message(message):
    if message.author == client.user:
        return

    # !status command
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

    # !patchnotes command — post a patch note
    if message.content.strip().startswith("!patchnotes"):
        # Only allow from the patch notes channel or by admins
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
