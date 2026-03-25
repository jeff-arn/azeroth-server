"""Azeroth Private Server Dashboard — Flask backend."""
import subprocess
import re
import os
from flask import Flask, jsonify, render_template
import mysql.connector

app = Flask(__name__)

DB_CONFIG = {
    "host": os.environ.get("DB_HOST", "127.0.0.1"),
    "user": os.environ.get("DB_USER", "mangos"),
    "password": os.environ["DB_PASSWORD"],
    "port": int(os.environ.get("DB_PORT", "3306")),
}

LEVELUP_LOG = os.environ.get("LEVELUP_LOG", "/home/wow/wow-private-server/vmangos/run/bin/LevelUp.log")

RACES = {1: "Human", 2: "Orc", 3: "Dwarf", 4: "Night Elf", 5: "Undead", 6: "Tauren", 7: "Gnome", 8: "Troll"}
CLASSES = {1: "Warrior", 2: "Paladin", 3: "Hunter", 4: "Rogue", 5: "Priest", 7: "Shaman", 8: "Mage", 9: "Warlock", 11: "Druid"}
CLASS_COLORS = {
    1: "#C69B6D", 2: "#F48CBA", 3: "#AAD372", 4: "#FFF468",
    5: "#FFFFFF", 7: "#0070DD", 8: "#3FC7EB", 9: "#8788EE", 11: "#FF7C0A",
}
ALLIANCE_RACES = {1, 3, 4, 7}
QUALITY_NAMES = {0: "Poor", 1: "Common", 2: "Uncommon", 3: "Rare", 4: "Epic", 5: "Legendary"}
QUALITY_COLORS = {0: "#9d9d9d", 1: "#ffffff", 2: "#1eff00", 3: "#0070dd", 4: "#a335ee", 5: "#ff8000"}

ZONE_NAMES = {
    1519: "Stormwind", 1537: "Ironforge", 1657: "Darnassus",
    1637: "Orgrimmar", 1638: "Thunder Bluff", 1497: "Undercity",
    12: "Elwynn Forest", 40: "Westfall", 38: "Loch Modan", 44: "Redridge Mountains",
    10: "Duskwood", 11: "Wetlands", 148: "Darkshore", 141: "Teldrassil",
    267: "Hillsbrad Foothills", 33: "Stranglethorn Vale", 331: "Ashenvale",
    3: "Badlands", 8: "Swamp of Sorrows", 45: "Arathi Highlands",
    440: "Tanaris", 357: "Feralas", 361: "Felwood", 28: "Western Plaguelands",
    139: "Eastern Plaguelands", 46: "Burning Steppes", 51: "Searing Gorge",
    618: "Winterspring", 1377: "Silithus", 490: "Un'Goro Crater",
    405: "Desolace", 406: "Stonetalon Mountains", 15: "Dustwallow Marsh",
    400: "Thousand Needles", 36: "Alterac Mountains", 130: "Silverpine Forest",
    85: "Tirisfal Glades", 14: "Durotar", 215: "Mulgore", 17: "The Barrens",
    47: "The Hinterlands", 1: "Dun Morogh", 1581: "The Deadmines",
    718: "Wailing Caverns", 209: "Shadowfang Keep", 491: "Razorfen Kraul",
    717: "The Stockade", 1583: "Blackfathom Deeps", 721: "Gnomeregan",
    2100: "Maraudon", 1584: "Blackrock Depths", 1585: "Blackrock Spire",
    2257: "Dire Maul", 2057: "Scholomance", 2017: "Stratholme",
}

RAIDS = {
    409: {"name": "Molten Core", "phase": 1},
    249: {"name": "Onyxia's Lair", "phase": 1},
    469: {"name": "Blackwing Lair", "phase": 3},
    309: {"name": "Zul'Gurub", "phase": 4},
    509: {"name": "Ruins of Ahn'Qiraj", "phase": 5},
    531: {"name": "Temple of Ahn'Qiraj", "phase": 5},
    533: {"name": "Naxxramas", "phase": 6},
}

_zone_cache = {}


def get_db(database):
    config = DB_CONFIG.copy()
    config["database"] = database
    return mysql.connector.connect(**config)


def get_zone_name(zone_id):
    if zone_id in ZONE_NAMES:
        return ZONE_NAMES[zone_id]
    if zone_id in _zone_cache:
        return _zone_cache[zone_id]
    try:
        conn = get_db("mangos")
        cursor = conn.cursor()
        cursor.execute("SELECT name FROM area_template WHERE entry = %s", (zone_id,))
        row = cursor.fetchone()
        cursor.close()
        conn.close()
        name = row[0] if row else f"Zone {zone_id}"
        _zone_cache[zone_id] = name
        return name
    except Exception:
        return f"Zone {zone_id}"


def format_money(copper):
    gold = copper // 10000
    silver = (copper % 10000) // 100
    cop = copper % 100
    return f"{gold}g {silver}s {cop}c"


def format_uptime(seconds):
    days = seconds // 86400
    hours = (seconds % 86400) // 3600
    mins = (seconds % 3600) // 60
    if days > 0:
        return f"{days}d {hours}h {mins}m"
    return f"{hours}h {mins}m"


@app.route("/")
def index():
    return render_template("dashboard.html")


@app.route("/api/status")
def api_status():
    try:
        result = subprocess.run(["pgrep", "mangosd"], capture_output=True, text=True)
        online = result.returncode == 0
        uptime_seconds = 0
        if online:
            pid = result.stdout.strip().split("\n")[0]
            try:
                up = subprocess.run(["ps", "-o", "etimes=", "-p", pid], capture_output=True, text=True)
                uptime_seconds = int(up.stdout.strip())
            except Exception:
                pass
        players = 0
        bots = 0
        try:
            conn = get_db("characters")
            cursor = conn.cursor()
            cursor.execute("SELECT COUNT(*) FROM characters WHERE online = 1 AND account < 100")
            players = cursor.fetchone()[0]
            cursor.execute("SELECT COUNT(*) FROM characters WHERE online = 1 AND account >= 100")
            bots = cursor.fetchone()[0]
            cursor.close()
            conn.close()
        except Exception:
            pass
        return jsonify({
            "online": online,
            "uptime_seconds": uptime_seconds,
            "uptime_display": format_uptime(uptime_seconds),
            "players_online": players,
            "bots_online": bots,
        })
    except Exception as e:
        return jsonify({"online": False, "uptime_display": "0h 0m", "players_online": 0, "bots_online": 0, "error": str(e)})


@app.route("/api/players")
def api_players():
    try:
        conn = get_db("characters")
        cursor = conn.cursor(dictionary=True)
        cursor.execute("""
            SELECT c.guid, c.name, c.level, c.race, c.class as cls, c.zone, c.money,
                   c.played_time_total, a.username
            FROM characters c
            JOIN realmd.account a ON c.account = a.id
            WHERE c.account < 100 AND c.online = 1
        """)
        rows = cursor.fetchall()
        cursor.close()
        conn.close()
        players = []
        for r in rows:
            players.append({
                "name": r["name"],
                "level": r["level"],
                "race": RACES.get(r["race"], "Unknown"),
                "class": CLASSES.get(r["cls"], "Unknown"),
                "class_color": CLASS_COLORS.get(r["cls"], "#ffffff"),
                "zone": get_zone_name(r["zone"]),
                "faction": "Alliance" if r["race"] in ALLIANCE_RACES else "Horde",
                "money": format_money(r["money"]),
                "played_time": format_uptime(r["played_time_total"]),
                "account": r["username"],
            })
        return jsonify(players)
    except Exception as e:
        return jsonify([])


@app.route("/api/activity")
def api_activity():
    try:
        if not os.path.exists(LEVELUP_LOG):
            return jsonify([])
        with open(LEVELUP_LOG, "r") as f:
            lines = f.readlines()
        pattern = re.compile(
            r"(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}).*Character (\w+):\d+ \[(c\d+) (r\d+)\] reaches level\s+(\d+)"
        )
        class_map = {"c1": "Warrior", "c2": "Paladin", "c3": "Hunter", "c4": "Rogue",
                     "c5": "Priest", "c7": "Shaman", "c8": "Mage", "c9": "Warlock", "c11": "Druid"}
        class_color_map = {"c1": "#C69B6D", "c2": "#F48CBA", "c3": "#AAD372", "c4": "#FFF468",
                           "c5": "#FFFFFF", "c7": "#0070DD", "c8": "#3FC7EB", "c9": "#8788EE", "c11": "#FF7C0A"}
        events = []
        for line in reversed(lines):
            if "<BOT>" in line:
                continue
            match = pattern.search(line)
            if match:
                events.append({
                    "timestamp": match.group(1),
                    "name": match.group(2),
                    "class": class_map.get(match.group(3), "Unknown"),
                    "class_color": class_color_map.get(match.group(3), "#ffffff"),
                    "level": int(match.group(5)),
                })
            if len(events) >= 10:
                break
        return jsonify(events)
    except Exception as e:
        return jsonify([])


@app.route("/api/auctions")
def api_auctions():
    try:
        conn = get_db("characters")
        cursor = conn.cursor()
        cursor.execute("SELECT COUNT(*) FROM auction")
        total = cursor.fetchone()[0]
        cursor.execute("""
            SELECT it.Quality, COUNT(*) as count
            FROM auction a
            JOIN mangos.item_template it ON a.item_id = it.entry
            GROUP BY it.Quality
            ORDER BY it.Quality
        """)
        rows = cursor.fetchall()
        cursor.close()
        conn.close()
        by_quality = []
        for quality, count in rows:
            by_quality.append({
                "name": QUALITY_NAMES.get(quality, "Unknown"),
                "color": QUALITY_COLORS.get(quality, "#ffffff"),
                "count": count,
            })
        return jsonify({"total": total, "by_quality": by_quality})
    except Exception as e:
        return jsonify({"total": 0, "by_quality": [], "error": str(e)})


@app.route("/api/raids")
def api_raids():
    try:
        conn = get_db("mangos")
        cursor = conn.cursor()
        cursor.execute("SELECT map, COUNT(*) FROM creature_disabled GROUP BY map")
        disabled_maps = {row[0] for row in cursor.fetchall()}
        cursor.close()
        conn.close()
        raids = []
        for map_id, info in sorted(RAIDS.items(), key=lambda x: x[1]["phase"]):
            raids.append({
                "name": info["name"],
                "phase": info["phase"],
                "unlocked": map_id not in disabled_maps,
            })
        return jsonify(raids)
    except Exception as e:
        return jsonify([])


@app.route("/api/bots")
def api_bots():
    try:
        conn = get_db("characters")
        cursor = conn.cursor()
        cursor.execute("SELECT COUNT(*) FROM characters WHERE online = 1 AND account >= 100")
        total = cursor.fetchone()[0]
        cursor.execute("""
            SELECT zone, COUNT(*) as count
            FROM characters
            WHERE online = 1 AND account >= 100
            GROUP BY zone
            ORDER BY count DESC
        """)
        rows = cursor.fetchall()
        cursor.close()
        conn.close()
        by_zone = [{"zone": get_zone_name(z), "count": c} for z, c in rows]
        return jsonify({"total_online": total, "by_zone": by_zone})
    except Exception as e:
        return jsonify({"total_online": 0, "by_zone": []})


if __name__ == "__main__":
    from waitress import serve
    print("Azeroth Dashboard starting on port 8080...")
    serve(app, host="0.0.0.0", port=8080)
